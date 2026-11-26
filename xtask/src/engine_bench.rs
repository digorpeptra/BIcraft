//! `cargo xtask engine-bench`: bicraft's DAX evaluator on a synthetic star schema, its timings
//! and memory, and its answers checked against reference answers recorded with the Analysis
//! Services engine on the same data (docs/benchmarks.md, section "The DAX engine bench").
//!
//! - `gen --rows N --out DIR`: a synthetic star schema as CSV files (`Ventes` N rows, `Produit`
//!   1 000, `Magasin` 200, `Calendrier` 5 years) and `DIR/build.json`, the `bicraft run` commands
//!   that build `DIR/bench.pbix` from them (M partitions over the CSV files, data computed by
//!   bicraft; Desktop opens it with the data and never refreshes);
//! - `bicraft --bin EXE --pbix FILE --out FILE`: every query of `xtask/engine-bench/queries.json`
//!   through `bicraft mcp` (one session: first run, then warm repetitions; plus one fresh
//!   session per query for the first-run figure), client-side wall time and `elapsed_ms`, open
//!   time and the process' peak memory;
//! - `compare --desktop FILE --bicraft FILE [--markdown FILE]`: results checked cell by cell.
//!
//! The reference answers come from `xtask/engine-bench/desktop.ps1` (ADOMD.NET, untimed).

use std::fmt::Write as _;
use std::io::{BufRead as _, BufReader, BufWriter, Write as _};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::Instant;

use bicraft_locale::calendar;
use serde_json::{Value, json};

use crate::engine_variants::Variant;

pub const USAGE: &str = "\
usage: cargo xtask engine-bench <step> ARGS   (docs/benchmarks.md)
  gen --rows N --out DIR [--customers C] [--variant standard|bigdim|skew] [--parts K]
                                              CSV files + DIR/build.json (bicraft run --file);
                                              bigdim: + a Client dimension of rows/5 customers;
                                              skew: Zipf-like keys, date order, order numbers
                                              (src/engine_variants.rs, queries-<variant>.json)
                                              variants: the fact in K files of consecutive lines,
                                              one partition each (default: one per 4 M lines)
  bicraft --bin EXE --pbix FILE --out FILE [--warm 5] [--fresh 1] [--only Q01,Q02]
           [--timeout-ms 600000] [--aggregations] [--report] [--variant V]
                                              bicraft's timings (MCP session); without --aggregations
                                              they are off (BICRAFT_AGGREGATIONS=0); with it, the bench
                                              report (one page per query) is applied in memory and the
                                              aggregations derived from it are built (size, build time)
                                              (--report: the bench report applied, aggregations off: the
                                              comparable without)
  rounds --bin EXE --pbix FILE --dir DIR [--rounds 3] [--modes without,with] [--warm 5]
           [--fresh 1] [--only ...] [--timeout-ms 600000] [--variant V]
                                              interleaved rounds (round 1: without, with; round 2:
                                              ...), DIR/bicraft[-aggs].roundN.json, merged into
                                              DIR/bicraft.json and DIR/bicraft-aggs.json (median per
                                              query over the rounds, every round kept for the worst);
                                              `without` = bench report applied, aggregations off
  merge --out FILE ROUND.json...              merge rounds measured apart (same rule)
  aggs-summary --data DIR                    bicraft's times without and with aggregations per size,
                                              from DIR/r*/{desktop,bicraft,bicraft-aggs}.json
  report                                      the bench report's commands (JSON)
  compare --desktop FILE --bicraft FILE [--markdown FILE] [--json FILE]
  summary --data DIR [--bicraft bicraft.json] one line per size from DIR/r*/{desktop,bicraft}.json
  check --data DIR [--file bicraft.json]     fails when a query of a size folder does not return
                                              the reference answers (DIR/r*/desktop.json)
The timed client (`bicraft`, `rounds`) always runs as a release build: from a debug xtask it
re-runs itself through `cargo run --release` (a debug client's JSON parsing would weigh on
BIcraft's wall times).
  swap-datamodel --pbix IN --image FILE --out OUT
                                              OUT = IN with its DataModel part replaced by an
                                              engine image (AMO ImageSave, desktop.ps1 -ImageOut)
  queries [--variant V]                       list the queries
";

const QUERIES: &str = include_str!("../engine-bench/queries.json");

/// The model's measures: (name, DAX, format string).
pub const MEASURES: &[(&str, &str, &str)] = &[
    ("CA", "SUM(Ventes[Montant])", "#,0.00"),
    ("Qte", "SUM(Ventes[Quantite])", "#,0"),
    ("Nb clients", "DISTINCTCOUNT(Ventes[ClientKey])", "#,0"),
    ("CA YTD", "TOTALYTD([CA], Calendrier[Date])", "#,0.00"),
    (
        "CA N-1",
        "CALCULATE([CA], SAMEPERIODLASTYEAR(Calendrier[Date]))",
        "#,0.00",
    ),
    ("CA Var N-1", "DIVIDE([CA] - [CA N-1], [CA N-1])", "0.0 %"),
    ("Part CA", "DIVIDE([CA], CALCULATE([CA], ALL(Produit)))", "0.0 %"),
    (
        "Rang produit",
        "IF(HASONEVALUE(Produit[Produit]), RANKX(ALL(Produit[Produit]), [CA]))",
        "0",
    ),
];

pub const PRODUCTS: u64 = 1_000;
pub const STORES: u64 = 200;
pub const CATEGORIES: u64 = 10;
pub const SUBCATEGORIES: u64 = 50;
pub const REGIONS: u64 = 8;
/// Calendrier: 2021-01-01 .. 2025-12-31.
pub const FIRST_YEAR: i64 = 2021;
pub const YEARS: i64 = 5;

pub fn cmd(args: &[String]) -> Result<(), String> {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    // the timed client is optimized code
    if cfg!(debug_assertions) && matches!(a.first().copied(), Some("bicraft" | "rounds")) {
        let mut c = crate::cargo();
        c.args(["run", "--release", "--quiet", "-p", "xtask", "--", "engine-bench"]);
        c.args(args);
        return crate::run(c, "cargo run --release -p xtask -- engine-bench (release client)");
    }
    match a.first().copied() {
        Some("gen") => {
            let rows = num(&a, "--rows")?.ok_or("gen: --rows is required")?;
            let out = opt(&a, "--out").ok_or("gen: --out is required")?;
            let variant = variant_of(&a)?;
            let customers = num(&a, "--customers")?.unwrap_or_else(|| variant.default_customers(rows));
            if variant == Variant::Standard {
                generate(Path::new(out), rows, customers)
            } else {
                let parts =
                    num(&a, "--parts")?.unwrap_or_else(|| crate::engine_variants::default_parts(rows));
                crate::engine_variants::generate(Path::new(out), variant, rows, customers, parts)
            }
        }
        Some("bicraft") => {
            let bin = opt(&a, "--bin").ok_or("bicraft: --bin is required")?;
            let pbix = opt(&a, "--pbix").ok_or("bicraft: --pbix is required")?;
            let out = opt(&a, "--out").ok_or("bicraft: --out is required")?;
            let warm = num(&a, "--warm")?.unwrap_or(5);
            let fresh = num(&a, "--fresh")?.unwrap_or(1);
            let timeout = num(&a, "--timeout-ms")?.unwrap_or(600_000);
            let only: Vec<String> = opt(&a, "--only")
                .map(|s| s.split(',').map(str::to_string).collect())
                .unwrap_or_default();
            let aggs = a.contains(&"--aggregations");
            let report = aggs || a.contains(&"--report");
            let variant = variant_of(&a)?;
            if variant != Variant::Standard && report {
                return Err(
                    "bicraft: --aggregations and --report apply the standard bench report (no --variant)"
                        .into(),
                );
            }
            let r = measure_bicraft(
                Path::new(bin),
                Path::new(pbix),
                warm,
                fresh,
                timeout,
                &only,
                aggs,
                report,
                variant,
            )?;
            write(
                Path::new(out),
                &serde_json::to_string_pretty(&r).map_err(|e| e.to_string())?,
            )
        }
        Some("compare") => {
            let d = read_json(Path::new(
                opt(&a, "--desktop").ok_or("compare: --desktop is required")?,
            ))?;
            let p = read_json(Path::new(
                opt(&a, "--bicraft").ok_or("compare: --bicraft is required")?,
            ))?;
            let (md, summary) = compare(&d, &p);
            print!("{md}");
            if let Some(f) = opt(&a, "--markdown") {
                write(Path::new(f), &md)?;
            }
            if let Some(f) = opt(&a, "--json") {
                write(
                    Path::new(f),
                    &serde_json::to_string_pretty(&summary).map_err(|e| e.to_string())?,
                )?;
            }
            Ok(())
        }
        Some("rounds") => {
            let bin = opt(&a, "--bin").ok_or("rounds: --bin is required")?;
            let pbix = opt(&a, "--pbix").ok_or("rounds: --pbix is required")?;
            let dir = opt(&a, "--dir").ok_or("rounds: --dir is required")?;
            let variant = variant_of(&a)?;
            let default_modes = if variant == Variant::Standard {
                "without,with"
            } else {
                "without"
            };
            let modes: Vec<&str> = opt(&a, "--modes").unwrap_or(default_modes).split(',').collect();
            if variant != Variant::Standard && modes.contains(&"with") {
                return Err(
                    "rounds: the `with` mode builds the standard bench report's aggregations (no --variant)"
                        .into(),
                );
            }
            if let Some(m) = modes.iter().find(|m| !matches!(**m, "without" | "with")) {
                return Err(format!("rounds: unknown mode `{m}` (without, with)"));
            }
            let plan = RoundPlan {
                bin: Path::new(bin),
                pbix: Path::new(pbix),
                dir: Path::new(dir),
                rounds: num(&a, "--rounds")?.unwrap_or(3).max(1),
                modes: &modes,
                warm: num(&a, "--warm")?.unwrap_or(5),
                fresh: num(&a, "--fresh")?.unwrap_or(1),
                timeout: num(&a, "--timeout-ms")?.unwrap_or(600_000),
                only: opt(&a, "--only")
                    .map(|s| s.split(',').map(str::to_string).collect())
                    .unwrap_or_default(),
                variant,
            };
            run_rounds(&plan)
        }
        Some("merge") => {
            let out = opt(&a, "--out").ok_or("merge: --out is required")?;
            let mut files = Vec::new();
            let mut skip = false;
            for x in a.iter().skip(1) {
                if skip {
                    skip = false;
                } else if *x == "--out" {
                    skip = true;
                } else {
                    files.push(read_json(Path::new(x))?);
                }
            }
            let merged = merge_rounds(&files)?;
            write(
                Path::new(out),
                &serde_json::to_string_pretty(&merged).map_err(|e| e.to_string())?,
            )
        }
        Some("check") => {
            let data = opt(&a, "--data").ok_or("check: --data is required")?;
            let file = opt(&a, "--file").unwrap_or("bicraft.json");
            let (report, bad) = check(Path::new(data), file)?;
            print!("{report}");
            if bad > 0 {
                return Err(format!("{bad} answer(s) differ from the reference"));
            }
            Ok(())
        }
        Some("summary") => {
            let data = opt(&a, "--data").ok_or("summary: --data is required")?;
            let file = opt(&a, "--bicraft").unwrap_or("bicraft.json");
            print!("{}", summary_of(Path::new(data), file)?);
            Ok(())
        }
        Some("swap-datamodel") => {
            let pbix = opt(&a, "--pbix").ok_or("swap-datamodel: --pbix is required")?;
            let image = opt(&a, "--image").ok_or("swap-datamodel: --image is required")?;
            let out = opt(&a, "--out").ok_or("swap-datamodel: --out is required")?;
            swap_datamodel(Path::new(pbix), Path::new(image), Path::new(out))
        }
        Some("aggs-summary") => {
            let data = opt(&a, "--data").ok_or("aggs-summary: --data is required")?;
            print!("{}", aggs_summary(Path::new(data))?);
            Ok(())
        }
        Some("report") => {
            println!(
                "{}",
                serde_json::to_string_pretty(&Value::Array(report_commands())).map_err(|e| e.to_string())?
            );
            Ok(())
        }
        Some("queries") => {
            for q in queries_of(variant_of(&a)?)? {
                println!("{}  {}", str_of(&q, "id"), str_of(&q, "title"));
            }
            Ok(())
        }
        _ => Err(USAGE.to_string()),
    }
}

fn opt<'a>(a: &[&'a str], name: &str) -> Option<&'a str> {
    let i = a.iter().position(|x| *x == name)?;
    a.get(i + 1).copied()
}

fn num(a: &[&str], name: &str) -> Result<Option<u64>, String> {
    opt(a, name)
        .map(|v| {
            v.replace('_', "")
                .parse::<u64>()
                .map_err(|e| format!("{name} {v}: {e}"))
        })
        .transpose()
}

pub(crate) fn write(path: &Path, text: &str) -> Result<(), String> {
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

fn read_json(path: &Path) -> Result<Value, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    serde_json::from_slice(bytes).map_err(|e| format!("{}: bad JSON: {e}", path.display()))
}

fn str_of<'v>(v: &'v Value, k: &str) -> &'v str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

/// The versioned queries (`xtask/engine-bench/queries.json`).
/// The `--variant` option (standard by default).
fn variant_of(a: &[&str]) -> Result<Variant, String> {
    opt(a, "--variant").map_or(Ok(Variant::Standard), Variant::parse)
}

/// A variant's queries (`queries.json` for the standard data).
pub fn queries_of(variant: Variant) -> Result<Vec<Value>, String> {
    match variant.queries_text() {
        Some(t) => crate::engine_variants::parse_queries(t, variant.name()),
        None => queries(),
    }
}

pub fn queries() -> Result<Vec<Value>, String> {
    let v: Value = serde_json::from_str(QUERIES).map_err(|e| format!("queries.json: {e}"))?;
    v.get("queries")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| "queries.json: no `queries` array".to_string())
}

/// One customer for ten sales lines, at least 1 000.
pub fn default_customers(rows: u64) -> u64 {
    (rows / 10).max(1_000)
}

// ---------------------------------------------------------------------------------------------
// Data
// ---------------------------------------------------------------------------------------------

pub(crate) fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// Days of the calendar.
pub fn calendar_days() -> i64 {
    calendar::days_from_civil(FIRST_YEAR + YEARS, 1, 1) - calendar::days_from_civil(FIRST_YEAR, 1, 1)
}

/// (year, month, day) of day `d` (0 = 2021-01-01).
pub fn ymd(d: i64) -> (i64, i64, i64) {
    calendar::civil_from_days(calendar::days_from_civil(FIRST_YEAR, 1, 1).saturating_add(d))
}

/// Catalogue price of a product, in cents (5.00 .. 504.99).
pub fn price_cents(p: u64) -> u64 {
    500 + splitmix(p ^ 0xC0FFEE) % 50_000
}

/// One sales line: (day, product, store, customer, quantity, amount in cents).
pub fn sale(k: u64, customers: u64, days: u64) -> (u64, u64, u64, u64, u64, u64) {
    let r = splitmix(k);
    let r2 = splitmix(r);
    let day = r % days;
    let product = 1 + (r >> 20) % PRODUCTS;
    let store = 1 + (r >> 40) % STORES;
    let customer = 1 + r2 % customers;
    let qty = 1 + (r2 >> 40) % 10;
    // discount 0..30 %
    let discount = (r2 >> 50) % 31;
    let amount = qty * price_cents(product) * (100 - discount) / 100;
    (day, product, store, customer, qty, amount)
}

pub(crate) fn cents(c: u64) -> String {
    format!("{}.{:02}", c / 100, c % 100)
}

pub(crate) fn csv_file(
    path: &Path,
    header: &str,
    rows: u64,
    mut line: impl FnMut(u64, &mut String),
) -> Result<(), String> {
    let f = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut w = BufWriter::with_capacity(1 << 20, f);
    let mut s = String::with_capacity(128);
    let err = |e: std::io::Error| format!("{}: {e}", path.display());
    writeln!(w, "{header}").map_err(err)?;
    for k in 1..=rows {
        s.clear();
        line(k, &mut s);
        s.push('\n');
        w.write_all(s.as_bytes()).map_err(err)?;
    }
    w.flush().map_err(err)
}

/// `out` canonical, without the `\\?\` prefix: the path goes into M partitions Desktop keeps.
pub(crate) fn plain_dir(out: &Path) -> Result<std::path::PathBuf, String> {
    let out = std::fs::canonicalize(out).map_err(|e| format!("{}: {e}", out.display()))?;
    Ok(match out.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(s) => std::path::PathBuf::from(s),
        None => out,
    })
}

fn generate(out: &Path, rows: u64, customers: u64) -> Result<(), String> {
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let out = plain_dir(out)?;
    let t0 = Instant::now();
    csv_file(
        &out.join("Produit.csv"),
        "ProduitKey,Produit,Categorie,SousCategorie,PrixCatalogue",
        PRODUCTS,
        |k, s| {
            let sub = 1 + splitmix(k ^ 0xAB) % SUBCATEGORIES;
            let cat = 1 + (sub - 1) % CATEGORIES;
            let _ = write!(
                s,
                "{k},P{k:04},Cat{cat:02},Sous{sub:02},{}",
                cents(price_cents(k))
            );
        },
    )?;
    csv_file(
        &out.join("Magasin.csv"),
        "MagasinKey,Magasin,Region",
        STORES,
        |k, s| {
            let region = 1 + splitmix(k ^ 0x5EED) % REGIONS;
            let _ = write!(s, "{k},M{k:03},Region{region}");
        },
    )?;
    let days = calendar_days();
    csv_file(
        &out.join("Calendrier.csv"),
        "Date,Annee,MoisNum,AnneeMois",
        days as u64,
        |k, s| {
            let (y, m, d) = ymd(k as i64 - 1);
            let _ = write!(s, "{y:04}-{m:02}-{d:02},{y},{m},{}", y * 100 + m);
        },
    )?;
    csv_file(
        &out.join("Ventes.csv"),
        "Date,ProduitKey,MagasinKey,ClientKey,Quantite,Montant",
        rows,
        |k, s| {
            let (day, p, st, c, q, a) = sale(k, customers, days as u64);
            let (y, m, d) = ymd(day as i64);
            let _ = write!(s, "{y:04}-{m:02}-{d:02},{p},{st},{c},{q},{}", cents(a));
        },
    )?;
    let pbix = out.join("bench.pbix");
    let build = build_commands(&out, &pbix);
    write(
        &out.join("build.json"),
        &serde_json::to_string_pretty(&build).map_err(|e| e.to_string())?,
    )?;
    let meta = json!({"rows": rows, "customers": customers, "products": PRODUCTS, "stores": STORES,
        "days": days, "generated_s": t0.elapsed().as_secs_f64()});
    write(
        &out.join("meta.json"),
        &serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?,
    )?;
    eprintln!(
        "{} rows, {customers} customers -> {} ({:.1} s); next: bicraft run --file {}",
        rows,
        out.display(),
        t0.elapsed().as_secs_f64(),
        out.join("build.json").display()
    );
    Ok(())
}

pub(crate) fn m_csv(path: &Path, types: &[(&str, &str)]) -> String {
    let p = path.display().to_string().replace('"', "\"\"");
    let t: Vec<String> = types.iter().map(|(c, ty)| format!("{{\"{c}\", {ty}}}")).collect();
    format!(
        "let\n    Source = Csv.Document(File.Contents(\"{p}\"), [Delimiter = \",\", Encoding = 65001, QuoteStyle = QuoteStyle.None]),\n    Headers = Table.PromoteHeaders(Source, [PromoteAllScalars = true]),\n    Typed = Table.TransformColumnTypes(Headers, {{{}}}, \"en-US\")\nin\n    Typed",
        t.join(", ")
    )
}

/// The `bicraft run --file` commands that build `pbix` from the CSV files in `dir`.
pub fn build_commands(dir: &Path, pbix: &Path) -> Value {
    crate::engine_variants::build_commands(dir, pbix, Variant::Standard)
}

/// Report measures of the bench report (the queries' expressions that are not model measures).
pub const REPORT_MEASURES: &[(&str, &str)] = &[
    ("Lignes", "COUNTROWS(Ventes)"),
    (
        "CA gros paniers",
        "SUMX(FILTER(Ventes, Ventes[Quantite] >= 8), Ventes[Montant])",
    ),
    (
        "CA catalogue",
        "SUMX(Ventes, Ventes[Quantite] * RELATED(Produit[PrixCatalogue]))",
    ),
    (
        "Clients actifs 2024",
        "COUNTROWS(CALCULATETABLE(SUMMARIZECOLUMNS(Ventes[ClientKey], \"CA\", [CA]), Calendrier[Annee] = 2024))",
    ),
    ("CA moyen par magasin", "AVERAGEX(VALUES(Magasin[Magasin]), [CA])"),
];

/// One page of the bench report: (query, visual type, fields, slicers, visual filters as
/// `visual.filter` parameters without page and visual).
type ReportPage = (
    &'static str,
    &'static str,
    &'static [&'static str],
    &'static [&'static str],
    Value,
);

fn report_pages() -> Vec<ReportPage> {
    vec![
        ("Q01", "card", &["[CA]"], &[], json!([])),
        (
            "Q02",
            "multiRowCard",
            &["[CA]", "[Qte]", "[Lignes]"],
            &[],
            json!([]),
        ),
        ("Q03", "tableEx", &["Produit[Categorie]", "[CA]"], &[], json!([])),
        (
            "Q04",
            "tableEx",
            &["Calendrier[AnneeMois]", "[CA]", "[Qte]"],
            &[],
            json!([]),
        ),
        (
            "Q05",
            "tableEx",
            &["Magasin[Magasin]", "Produit[Categorie]", "[CA]"],
            &[],
            json!([]),
        ),
        ("Q06", "card", &["[Nb clients]"], &[], json!([])),
        (
            "Q07",
            "tableEx",
            &["Magasin[Region]", "[Nb clients]"],
            &[],
            json!([]),
        ),
        (
            "Q08",
            "card",
            &["[CA]"],
            &[],
            json!([{"field": "Produit[Categorie]", "in": ["Cat03"]}]),
        ),
        (
            "Q09",
            "tableEx",
            &["Magasin[Region]", "[CA]", "[Qte]"],
            &["Produit[Categorie]", "Calendrier[Annee]"],
            json!([]),
        ),
        (
            "Q10",
            "tableEx",
            &["Calendrier[AnneeMois]", "[CA]", "[CA YTD]"],
            &["Calendrier[Annee]"],
            json!([]),
        ),
        (
            "Q11",
            "tableEx",
            &["Calendrier[Annee]", "[CA]", "[CA N-1]", "[CA Var N-1]"],
            &[],
            json!([]),
        ),
        (
            "Q12",
            "tableEx",
            &["Produit[Categorie]", "[CA]", "[Part CA]"],
            &[],
            json!([]),
        ),
        (
            "Q13",
            "tableEx",
            &["Produit[Produit]", "[CA]", "[Rang produit]"],
            &[],
            json!([]),
        ),
        (
            "Q14",
            "tableEx",
            &["Produit[Produit]", "[CA]"],
            &[],
            json!([{"field": "Produit[Produit]", "top": 10, "by": "[CA]"}]),
        ),
        ("Q15", "card", &["[CA gros paniers]"], &[], json!([])),
        ("Q16", "card", &["[CA catalogue]"], &[], json!([])),
        ("Q17", "tableEx", &["Ventes[ClientKey]", "[CA]"], &[], json!([])),
        ("Q18", "card", &["[Clients actifs 2024]"], &[], json!([])),
        (
            "Q19",
            "tableEx",
            &["Magasin[Region]", "Calendrier[Annee]", "[CA]"],
            &[],
            json!([]),
        ),
        ("Q20", "card", &["[CA moyen par magasin]"], &[], json!([])),
        (
            "Q21",
            "tableEx",
            &["Produit[SousCategorie]", "[CA]"],
            &[],
            json!([{"field": "Ventes[Quantite]", "op": ">", "value": 5}]),
        ),
        (
            "Q22",
            "multiRowCard",
            &["Min(Ventes[Date])", "Max(Ventes[Date])"],
            &[],
            json!([]),
        ),
    ]
}

/// The bench report as `bicraft` commands: one page per bench query (named by its id), its
/// visual with the query's fields, the query's filters as slicers or visual filters, and the
/// report measures the queries' own expressions need. The aggregations of the "with
/// aggregations" measures are derived from it.
pub fn report_commands() -> Vec<Value> {
    let mut c = Vec::new();
    for (name, dax) in REPORT_MEASURES {
        c.push(json!({"command": "report_measure.add", "params": {"table": "Ventes", "name": name, "expression": dax}}));
    }
    for (id, ty, fields, slicers, filters) in report_pages() {
        c.push(json!({"command": "page.add", "params": {"display_name": id}}));
        let fields: Vec<Value> = fields
            .iter()
            .map(|f| json!({"role": "Values", "field": f}))
            .collect();
        let visual = format!("{id}-visual");
        c.push(
            json!({"command": "visual.add", "params": {"page": id, "visual_type": ty, "x": 20, "y": 20,
            "width": 600, "height": 400, "fields": fields, "key": visual, "force": true}}),
        );
        for (k, s) in slicers.iter().enumerate() {
            c.push(
                json!({"command": "visual.add", "params": {"page": id, "visual_type": "slicer",
                "x": 640, "y": 20 + 120 * k, "width": 200, "height": 100,
                "fields": [{"role": "Values", "field": s}], "key": format!("{id}-slicer-{k}")}}),
            );
        }
        for f in filters.as_array().into_iter().flatten() {
            let mut p = f.clone();
            if let Some(o) = p.as_object_mut() {
                o.insert("page".into(), json!(id));
                o.insert("visual".into(), json!(visual));
            }
            c.push(json!({"command": "visual.filter", "params": p}));
        }
    }
    c
}

/// The answer guard: every query of every size folder of `data` (`desktop.json`, the reference
/// answers, against the measures `file`) must return the reference results. Returns the report
/// and the number of queries whose answers differ.
pub fn check(data: &Path, file: &str) -> Result<(String, usize), String> {
    let mut md = String::new();
    let mut bad = 0usize;
    for (rows, dir) in size_dirs(data, file)? {
        let d = read_json(&dir.join("desktop.json"))?;
        let p = read_json(&measures(&dir, file).unwrap_or_default())?;
        let (_, s) = compare(&d, &p);
        let empty = Vec::new();
        for row in s["rows"].as_array().unwrap_or(&empty) {
            let id = row["id"].as_str().unwrap_or("?");
            let verdict = row["verdict"].as_str().unwrap_or("");
            if !is_same(verdict) {
                bad += 1;
                let _ = writeln!(md, "{rows} {id}: {verdict}");
            }
        }
        let _ = writeln!(md, "{rows} rows: {} of {} the same", s["agree"], s["queries"]);
    }
    Ok((md, bad))
}

/// The size folders of `data` that hold reference answers and the measures `file`, by their
/// fact's row count, smallest first.
fn size_dirs(data: &Path, file: &str) -> Result<Vec<(u64, std::path::PathBuf)>, String> {
    let mut dirs: Vec<(u64, std::path::PathBuf)> = std::fs::read_dir(data)
        .map_err(|e| format!("{}: {e}", data.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.join("desktop.json").is_file() && measures(p, file).is_some())
        .filter_map(|p| {
            let rows = read_json(&p.join("desktop.json")).ok()?.get("rows")?.as_u64()?;
            Some((rows, p))
        })
        .collect();
    dirs.sort();
    Ok(dirs)
}

/// A verdict of `compare` that counts as the same answer.
fn is_same(verdict: &str) -> bool {
    verdict == "same" || verdict.starts_with("same values")
}

/// The measures file `file` of a size folder, when it exists.
fn measures(dir: &Path, file: &str) -> Option<std::path::PathBuf> {
    let p = dir.join(file);
    p.is_file().then_some(p)
}

/// The synthesis table over every size folder of `data` (`r10k`, `r1m`, ...), smallest first, of
/// the bicraft measures in `file` of each folder (`bicraft-base.json`: an
/// earlier run kept for a before / after table): agreement with the reference answers and
/// BIcraft's own medians.
pub fn summary_of(data: &Path, file: &str) -> Result<String, String> {
    let mut md = String::new();
    let _ = writeln!(
        md,
        "| Rows | Same / refused | Start median | Cold median | Warm median | bicraft open | bicraft peak |"
    );
    let _ = writeln!(md, "|---:|---|---:|---:|---:|---:|---:|");
    for (rows, dir) in size_dirs(data, file)? {
        let d = read_json(&dir.join("desktop.json"))?;
        let p = read_json(&measures(&dir, file).unwrap_or_default())?;
        let (_, s) = compare(&d, &p);
        let refused = p
            .get("queries")
            .and_then(Value::as_array)
            .map_or(0, |q| q.iter().filter(|x| x.get("error").is_some()).count());
        let gb = |v: Option<f64>| v.map_or("–".to_string(), |b| format!("{:.2} GB", b / 1e9));
        let num = |v: &Value, k: &str| v.get(k).and_then(Value::as_f64);
        let p_open = match (
            num(&p, "open_s"),
            p.pointer("/load/wall_s").and_then(Value::as_f64),
        ) {
            (Some(o), Some(l)) if p.pointer("/load/error").is_none_or(Value::is_null) => Some(o + l),
            _ => None,
        };
        let g = |k: &str| s.get(k).and_then(Value::as_f64);
        let _ = writeln!(
            md,
            "| {rows} | {} / {refused} | {} | {} | {} | {} | {} |",
            s["agree"],
            f(g("start_median_s")),
            f(g("cold_median_s")),
            f(g("warm_median_s")),
            f(p_open),
            gb(p.pointer("/memory/peak_working_set").and_then(Value::as_f64))
        );
    }
    Ok(md)
}

/// Copies the package `pbix` to `out`, every part as it is stored except `DataModel`, which
/// becomes the bytes of `image` (stored, as Desktop writes it).
pub fn swap_datamodel(pbix: &Path, image: &Path, out: &Path) -> Result<(), String> {
    fn err(p: &Path) -> impl Fn(std::io::Error) -> String + '_ {
        move |e| format!("{}: {e}", p.display())
    }
    let input = std::fs::File::open(pbix).map_err(err(pbix))?;
    let mut zin = zip::ZipArchive::new(input).map_err(|e| format!("{}: {e}", pbix.display()))?;
    let output = std::fs::File::create(out).map_err(err(out))?;
    let mut zout = zip::ZipWriter::new(BufWriter::new(output));
    let mut swapped = false;
    for i in 0..zin.len() {
        let entry = zin.by_index_raw(i).map_err(|e| e.to_string())?;
        if entry.name().map_err(|e| e.to_string())? == "DataModel" {
            drop(entry);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored)
                .large_file(true);
            zout.start_file("DataModel", opts).map_err(|e| e.to_string())?;
            let mut img = std::fs::File::open(image).map_err(err(image))?;
            std::io::copy(&mut img, &mut zout).map_err(err(image))?;
            swapped = true;
        } else {
            zout.raw_copy_file(entry).map_err(|e| e.to_string())?;
        }
    }
    zout.finish().map_err(|e| e.to_string())?;
    if swapped {
        Ok(())
    } else {
        Err(format!("{}: no DataModel part", pbix.display()))
    }
}

// ---------------------------------------------------------------------------------------------
// bicraft (MCP session)
// ---------------------------------------------------------------------------------------------

struct Mcp {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next: u64,
}

impl Mcp {
    /// `aggs`: report-derived aggregations on (else `BICRAFT_AGGREGATIONS=0`).
    fn start(bin: &Path, aggs: bool) -> Result<Self, String> {
        let mut child = Command::new(bin)
            .arg("mcp")
            .env("BICRAFT_AGGREGATIONS", if aggs { "1" } else { "0" })
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("spawn {} mcp: {e}", bin.display()))?;
        let stdin = child.stdin.take().ok_or("mcp: no stdin")?;
        let stdout = BufReader::new(child.stdout.take().ok_or("mcp: no stdout")?);
        let mut m = Mcp {
            child,
            stdin,
            stdout,
            next: 0,
        };
        m.request(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {},
            "clientInfo": {"name": "bicraft-engine-bench", "version": "0"}}),
        )?;
        let n = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        writeln!(m.stdin, "{n}").map_err(|e| format!("mcp: {e}"))?;
        Ok(m)
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.next += 1;
        let id = self.next;
        let req = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(self.stdin, "{req}").map_err(|e| format!("mcp: {e}"))?;
        self.stdin.flush().map_err(|e| format!("mcp: {e}"))?;
        let mut line = String::new();
        loop {
            line.clear();
            let n = self
                .stdout
                .read_line(&mut line)
                .map_err(|e| format!("mcp: {e}"))?;
            if n == 0 {
                return Err("mcp: the server closed its output".into());
            }
            let mut v: Value = serde_json::from_str(&line).map_err(|e| format!("mcp: bad JSON: {e}"))?;
            if v.get("id") == Some(&json!(id)) {
                if let Some(e) = v.get("error") {
                    return Err(format!("mcp: protocol error: {e}"));
                }
                // moved out, not copied: the client's own work stays out of the timings
                return Ok(v.get_mut("result").map_or(Value::Null, Value::take));
            }
        }
    }

    /// `run_command`: (wall seconds, structured result or error text).
    fn command(&mut self, command: &str, params: Value) -> Result<(f64, Result<Value, String>), String> {
        let t = Instant::now();
        let mut r = self.request(
            "tools/call",
            json!({"name": "run_command", "arguments": {"command": command, "params": params}}),
        )?;
        let structured = r.get_mut("structuredContent").map_or(Value::Null, Value::take);
        let wall = t.elapsed().as_secs_f64();
        if r.get("isError") == Some(&Value::Bool(true)) {
            let text = r
                .pointer("/content/0/text")
                .and_then(Value::as_str)
                .unwrap_or("error")
                .to_string();
            return Ok((wall, Err(text)));
        }
        Ok((wall, Ok(structured)))
    }

    /// Peak working set and peak private bytes of the server process (Windows), in bytes.
    fn peak_memory(&self) -> Value {
        let pid = self.child.id();
        if !cfg!(windows) {
            return Value::Null;
        }
        let ps =
            format!("$p = Get-Process -Id {pid}; \"$($p.PeakWorkingSet64) $($p.PeakPagedMemorySize64)\"");
        let Ok(out) = Command::new("powershell")
            .args(["-NoProfile", "-Command", &ps])
            .output()
        else {
            return Value::Null;
        };
        let text = String::from_utf8_lossy(&out.stdout);
        let n: Vec<u64> = text.split_whitespace().filter_map(|x| x.parse().ok()).collect();
        match n.as_slice() {
            [ws, private] => json!({"peak_working_set": ws, "peak_private": private}),
            _ => Value::Null,
        }
    }

    fn close(mut self) {
        drop(self.stdin);
        let _ = self.child.wait();
    }
}

fn median(v: &[f64]) -> Option<f64> {
    let mut v: Vec<f64> = v.iter().copied().filter(|x| x.is_finite()).collect();
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    let n = v.len();
    Some(if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    })
}

fn eval_params(dax: &str, timeout: u64) -> Value {
    json!({"query": dax, "max_rows": 1_000_000, "timeout_ms": timeout})
}

fn elapsed_s(r: &Value) -> Option<f64> {
    r.get("elapsed_ms").and_then(Value::as_f64).map(|ms| ms / 1000.0)
}

/// Results in the shape of `desktop.ps1`: `[{columns: [{name, type}], rows: [[cells]]}]`.
fn result_sets(r: &Value) -> Value {
    let sets = r
        .get("results")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Value::Array(
        sets.iter()
            .map(|s| {
                let cols = s
                    .get("columns")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let names: Vec<String> = cols.iter().map(|c| str_of(c, "name").to_string()).collect();
                let rows: Vec<Value> = s
                    .get("rows")
                    .and_then(Value::as_array)
                    .map(|rs| {
                        rs.iter()
                            .map(|row| {
                                Value::Array(
                                    names
                                        .iter()
                                        .map(|n| row.get(n).cloned().unwrap_or(Value::Null))
                                        .collect(),
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                json!({"columns": cols, "rows": rows, "row_count": s.get("row_count")})
            })
            .collect(),
    )
}

/// One first run in a fresh session: (query wall seconds, `elapsed_ms` in seconds, error, start
/// seconds = process start and MCP `initialize` + `file.open` + the query).
type FreshRun = (f64, Option<f64>, Option<String>, f64);

#[allow(clippy::too_many_arguments)]
fn measure_bicraft(
    bin: &Path,
    pbix: &Path,
    warm: u64,
    fresh: u64,
    timeout: u64,
    only: &[String],
    aggs: bool,
    report: bool,
    variant: Variant,
) -> Result<Value, String> {
    let qs: Vec<Value> = queries_of(variant)?
        .into_iter()
        .filter(|q| only.is_empty() || only.iter().any(|o| o == str_of(q, "id")))
        .collect();
    let path = pbix.display().to_string();
    let mut per_query = Vec::new();

    // one fresh session per query: the first run of that query on a newly opened file
    let mut fresh_runs: Vec<Vec<FreshRun>> = vec![Vec::new(); qs.len()];
    let mut opens = Vec::new();
    let mut spawns = Vec::new();
    for _ in 0..fresh {
        for (i, q) in qs.iter().enumerate() {
            // start (launch to first answer, then the query): the process, MCP
            // `initialize`, `file.open` and the query; the bench report's in-memory edits between
            // them are not counted (a file saved with its report needs none)
            let t0 = Instant::now();
            let mut m = Mcp::start(bin, aggs)?;
            let spawn_s = t0.elapsed().as_secs_f64();
            spawns.push(spawn_s);
            let (open_s, opened) = m.command("file.open", json!({"path": path}))?;
            opened.map_err(|e| format!("file.open: {e}"))?;
            opens.push(open_s);
            if report {
                apply_report(&mut m)?;
            }
            let (wall, r) = m.command("dax.evaluate", eval_params(str_of(q, "dax"), timeout))?;
            let start = spawn_s + open_s + wall;
            let entry = match &r {
                Ok(v) => (wall, elapsed_s(v), None, start),
                Err(e) => (wall, None, Some(e.clone()), start),
            };
            if let Some(slot) = fresh_runs.get_mut(i) {
                slot.push(entry);
            }
            m.close();
            eprintln!("fresh {} {:.3} s", str_of(q, "id"), wall);
        }
    }

    // one session: open, first run of every query in order, then warm repetitions
    let mut m = Mcp::start(bin, aggs)?;
    let (open_s, opened) = m.command("file.open", json!({"path": path}))?;
    opened.map_err(|e| format!("file.open: {e}"))?;
    if report {
        apply_report(&mut m)?;
    }
    // the data load: the first query decodes the columns it needs; a trivial query on the fact
    // table's row count measures the backup's decompression and indexing alone
    let (load_s, loaded) = m.command(
        "dax.evaluate",
        eval_params("EVALUATE ROW(\"n\", COUNTROWS(Ventes))", timeout),
    )?;
    let load = json!({"wall_s": load_s, "elapsed_s": loaded.as_ref().ok().and_then(elapsed_s),
        "error": loaded.as_ref().err()});
    // the aggregations derived from the bench report, built now: their size and build time
    let aggregations = if aggs {
        let (build_s, built) = m.command("dax.aggregations", json!({"build": true}))?;
        let mut v = built.map_err(|e| format!("dax.aggregations: {e}"))?;
        if let Some(o) = v.as_object_mut() {
            o.insert("build_wall_s".into(), json!(build_s));
            o.remove("visuals");
        }
        v
    } else {
        Value::Null
    };
    for (i, q) in qs.iter().enumerate() {
        let id = str_of(q, "id");
        let (first_wall, first) = m.command("dax.evaluate", eval_params(str_of(q, "dax"), timeout))?;
        let mut entry = json!({"id": id, "title": str_of(q, "title")});
        match &first {
            Ok(v) => {
                entry["first_s"] = json!(first_wall);
                entry["first_elapsed_s"] = json!(elapsed_s(v));
                entry["results"] = result_sets(v);
                let mut walls = Vec::new();
                let mut els = Vec::new();
                // a query over 10 s is repeated once only (the bench must stay usable at scale)
                let reps = if first_wall > 10.0 { warm.min(1) } else { warm };
                for _ in 0..reps {
                    let (w, r) = m.command("dax.evaluate", eval_params(str_of(q, "dax"), timeout))?;
                    walls.push(w);
                    if let Ok(v) = r {
                        els.extend(elapsed_s(&v));
                    }
                }
                entry["warm_s"] = json!(median(&walls));
                entry["warm_elapsed_s"] = json!(median(&els));
                entry["warm_runs_s"] = json!(walls);
                // comparable cold: data open and decoded, the caches emptied before each run
                // (`dax.clear_cache`); median of 3
                let mut colds = Vec::new();
                let mut cold_els = Vec::new();
                for _ in 0..if first_wall > 10.0 { 1 } else { 3 } {
                    let (_, c) = m.command("dax.clear_cache", json!({}))?;
                    c.map_err(|e| format!("dax.clear_cache: {e}"))?;
                    let (w, r) = m.command("dax.evaluate", eval_params(str_of(q, "dax"), timeout))?;
                    colds.push(w);
                    if let Ok(v) = r {
                        cold_els.extend(elapsed_s(&v));
                    }
                }
                entry["cold_s"] = json!(median(&colds));
                entry["cold_elapsed_s"] = json!(median(&cold_els));
                entry["cold_runs_s"] = json!(colds);
            }
            Err(e) => {
                entry["first_s"] = json!(first_wall);
                entry["error"] = json!(e);
            }
        }
        if let Some(f) = fresh_runs.get(i) {
            entry["fresh_s"] = json!(median(&f.iter().map(|x| x.0).collect::<Vec<_>>()));
            entry["fresh_elapsed_s"] = json!(median(&f.iter().filter_map(|x| x.1).collect::<Vec<_>>()));
            entry["fresh_start_s"] = json!(median(&f.iter().map(|x| x.3).collect::<Vec<_>>()));
            if let Some(e) = f.iter().find_map(|x| x.2.clone()) {
                entry["fresh_error"] = json!(e);
            }
        }
        eprintln!("{id} first {first_wall:.3} s warm {}", entry["warm_s"]);
        per_query.push(entry);
    }
    let used = if aggs {
        m.command("dax.aggregations", json!({}))?
            .1
            .ok()
            .map(|v| json!({"hits": v["hits"], "misses": v["misses"]}))
    } else {
        None
    };
    let memory = m.peak_memory();
    m.close();
    let profile = if aggs {
        Value::Null
    } else {
        profile_load(bin, &path, timeout, variant.fact_columns())?
    };
    let size = std::fs::metadata(pbix).map(|x| x.len()).unwrap_or(0);
    Ok(json!({
        "engine": "bicraft",
        "binary": bin.display().to_string(),
        "pbix": path,
        "pbix_bytes": size,
        "open_s": open_s,
        "fresh_open_s": median(&opens),
        "fresh_spawn_s": median(&spawns),
        "client": if cfg!(debug_assertions) { "debug" } else { "release" },
        "load": load,
        "memory": memory,
        "profile": profile,
        "warm_runs": warm,
        "aggregations": aggregations,
        "aggregations_used": used,
        "queries": per_query,
    }))
}

/// Where the first run's time goes, in a fresh session: opening the package, the backup
/// (XPress9 decompression, ABF and metadata index: a row count needs no column), then per fact
/// column the first touch (decode + scan) against a second touch (scan only).
fn profile_load(bin: &Path, path: &str, timeout: u64, fact_columns: &[&str]) -> Result<Value, String> {
    let mut m = Mcp::start(bin, false)?;
    let (open_s, opened) = m.command("file.open", json!({"path": path}))?;
    opened.map_err(|e| format!("file.open: {e}"))?;
    let (backup_s, b) = m.command(
        "dax.evaluate",
        eval_params("EVALUATE ROW(\"n\", COUNTROWS(Ventes))", timeout),
    )?;
    let mut cols = Vec::new();
    for c in fact_columns {
        let q = format!("EVALUATE ROW(\"x\", MAX(Ventes[{c}]))");
        let (first, r1) = m.command("dax.evaluate", eval_params(&q, timeout))?;
        let (second, r2) = m.command("dax.evaluate", eval_params(&q, timeout))?;
        cols.push(json!({"column": c, "first_s": first, "second_s": second,
            "error": r1.err().or(r2.err())}));
    }
    let memory = m.peak_memory();
    m.close();
    Ok(
        json!({"open_s": open_s, "backup_s": backup_s, "backup_error": b.err(), "columns": cols, "memory": memory}),
    )
}

/// Applies the bench report to the open file in memory (never saved), unless it already has it
/// (files generated since the report joined `build.json`).
fn apply_report(m: &mut Mcp) -> Result<(), String> {
    let (_, summary) = m.command("report.summary", json!({"visuals": false}))?;
    let has = summary
        .ok()
        .and_then(|v| v.get("pages").and_then(Value::as_array).cloned())
        .is_some_and(|p| p.iter().any(|x| str_of(x, "display_name") == "Q01"));
    if has {
        return Ok(());
    }
    for c in report_commands() {
        let id = str_of(&c, "command").to_string();
        let (_, r) = m.command(&id, c.get("params").cloned().unwrap_or(Value::Null))?;
        r.map_err(|e| format!("{id}: {e}"))?;
    }
    Ok(())
}

/// What `rounds` measures.
struct RoundPlan<'a> {
    bin: &'a Path,
    pbix: &'a Path,
    dir: &'a Path,
    rounds: u64,
    modes: &'a [&'a str],
    warm: u64,
    fresh: u64,
    timeout: u64,
    only: Vec<String>,
    variant: Variant,
}

/// The measures file of a mode: `bicraft` (without aggregations) or `bicraft-aggs`.
fn mode_stem(mode: &str) -> &'static str {
    if mode == "with" { "bicraft-aggs" } else { "bicraft" }
}

/// Interleaved rounds: round 1 measures every mode, then round 2, and so on, so that a load
/// change on the machine weighs on every mode alike. Each round is kept
/// (`DIR/<stem>.roundN.json`), then each mode's rounds are merged into `DIR/<stem>.json`.
fn run_rounds(plan: &RoundPlan) -> Result<(), String> {
    std::fs::create_dir_all(plan.dir).map_err(|e| format!("{}: {e}", plan.dir.display()))?;
    let reference = read_json(&plan.dir.join("desktop.json")).ok();
    let mut measured: Vec<Vec<Value>> = vec![Vec::new(); plan.modes.len()];
    for round in 1..=plan.rounds {
        for (k, mode) in plan.modes.iter().enumerate() {
            let t = Instant::now();
            let aggs = *mode == "with";
            let v = measure_bicraft(
                plan.bin,
                plan.pbix,
                plan.warm,
                plan.fresh,
                plan.timeout,
                &plan.only,
                aggs,
                plan.variant == Variant::Standard,
                plan.variant,
            )?;
            let file = plan.dir.join(format!("{}.round{round}.json", mode_stem(mode)));
            write(
                &file,
                &serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?,
            )?;
            let line = reference.as_ref().map_or(String::new(), |d| {
                let (md, _) = compare(d, &v);
                md.lines().last().unwrap_or("").to_string()
            });
            eprintln!(
                "[round {round}/{} {mode}] {:.1} s; {line}",
                plan.rounds,
                t.elapsed().as_secs_f64()
            );
            if let Some(slot) = measured.get_mut(k) {
                slot.push(v);
            }
        }
    }
    for (mode, rounds) in plan.modes.iter().zip(&measured) {
        let merged = merge_rounds(rounds)?;
        let file = plan.dir.join(format!("{}.json", mode_stem(mode)));
        write(
            &file,
            &serde_json::to_string_pretty(&merged).map_err(|e| e.to_string())?,
        )?;
        if let Some(d) = &reference {
            let (md, s) = compare(d, &merged);
            let stem = mode_stem(mode).replacen("bicraft", "answers", 1);
            write(&plan.dir.join(format!("{stem}.md")), &md)?;
            write(
                &plan.dir.join(format!("{stem}.json")),
                &serde_json::to_string_pretty(&s).map_err(|e| e.to_string())?,
            )?;
            eprintln!(
                "[{mode}, {} rounds] {}",
                rounds.len(),
                md.lines().last().unwrap_or("")
            );
        }
    }
    Ok(())
}

/// Per-query timings merged over rounds: the median per field; `rounds` keeps every round's
/// start, cold and warm (for the slowest round); the peak memory is the highest. Results must be
/// the same in every round (`round_mismatch` otherwise). A query refused in any round keeps
/// that refusal.
pub fn merge_rounds(rounds: &[Value]) -> Result<Value, String> {
    let first = rounds.first().ok_or("merge: no rounds")?;
    if !rounds.iter().all(Value::is_object) {
        return Err("merge: a round is not a measures object".into());
    }
    let mut out = first.clone();
    let num_at = |v: &Value, ptr: &str| v.pointer(ptr).and_then(Value::as_f64);
    let med_of = |ptr: &str| -> Option<f64> {
        median(&rounds.iter().filter_map(|v| num_at(v, ptr)).collect::<Vec<_>>())
    };
    for k in ["open_s", "fresh_open_s", "fresh_spawn_s"] {
        if let Some(o) = out.as_object_mut() {
            o.insert(k.into(), json!(med_of(&format!("/{k}"))));
        }
    }
    if out.get("load").is_some_and(Value::is_object) {
        out["load"]["wall_s"] = json!(med_of("/load/wall_s"));
        out["load"]["elapsed_s"] = json!(med_of("/load/elapsed_s"));
    }
    for k in ["peak_working_set", "peak_private"] {
        let peak = rounds
            .iter()
            .filter_map(|v| v.pointer(&format!("/memory/{k}")).and_then(Value::as_u64))
            .max();
        if let (Some(p), Some(m)) = (peak, out.get_mut("memory").and_then(Value::as_object_mut)) {
            m.insert(k.into(), json!(p));
        }
    }
    if let Some(o) = out.as_object_mut() {
        o.insert("rounds".into(), json!(rounds.len()));
    }
    let empty = Vec::new();
    let qs = out
        .get_mut("queries")
        .and_then(Value::as_array_mut)
        .ok_or("merge: no queries")?;
    for q in qs.iter_mut().filter(|q| q.is_object()) {
        let id = str_of(q, "id").to_string();
        let same: Vec<&Value> = rounds
            .iter()
            .filter_map(|v| {
                v.get("queries")
                    .and_then(Value::as_array)
                    .unwrap_or(&empty)
                    .iter()
                    .find(|x| str_of(x, "id") == id)
            })
            .collect();
        for k in [
            "first_s",
            "first_elapsed_s",
            "warm_s",
            "warm_elapsed_s",
            "cold_s",
            "cold_elapsed_s",
            "fresh_s",
            "fresh_elapsed_s",
            "fresh_start_s",
        ] {
            let vals: Vec<f64> = same
                .iter()
                .filter_map(|x| x.get(k).and_then(Value::as_f64))
                .collect();
            if !vals.is_empty() {
                q[k] = json!(median(&vals));
            }
        }
        let per = |k: &str| -> Value {
            json!(
                same.iter()
                    .filter_map(|x| x.get(k).and_then(Value::as_f64))
                    .collect::<Vec<_>>()
            )
        };
        let start_k = if same.iter().all(|x| x.get("fresh_start_s").is_some()) {
            "fresh_start_s"
        } else {
            "fresh_s"
        };
        q["rounds"] = json!({"start": per(start_k), "cold": per("cold_s"), "warm": per("warm_s")});
        if start_k == "fresh_s" {
            // older measures: the start of each round is completed as `compare` does
            let starts: Vec<f64> = rounds
                .iter()
                .filter_map(|v| {
                    let x = v
                        .get("queries")?
                        .as_array()?
                        .iter()
                        .find(|x| str_of(x, "id") == id)?;
                    p_start_of(v, x)
                })
                .collect();
            q["rounds"]["start"] = json!(starts);
        }
        if let Some(e) = same.iter().find_map(|x| x.get("error").cloned()) {
            q["error"] = e;
        }
        let base = q.get("results").cloned();
        for (n, x) in same.iter().enumerate().skip(1) {
            let mismatch = match (&base, x.get("results")) {
                (Some(a), Some(b)) => diff_results(a, b),
                (None, None) => None,
                _ => Some("results missing in a round".into()),
            };
            if let Some(m) = mismatch {
                q["round_mismatch"] = json!(format!("round {}: {m}", n + 1));
                break;
            }
        }
    }
    Ok(out)
}

/// BIcraft's medians without and with aggregations, per size folder of `data` (`bicraft.json` and
/// `bicraft-aggs.json`, each checked against the reference answers of `desktop.json`), and the
/// aggregations' size and build time.
pub fn aggs_summary(data: &Path) -> Result<String, String> {
    let mut md = String::new();
    let _ = writeln!(
        md,
        "| Rows | Same (without / with) | Cold median: without / with | Warm median: without / with | Start median with | Aggregations | Rows (% of fact) | Memory | Build | Scans answered |"
    );
    let _ = writeln!(md, "|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|");
    for (rows, dir) in size_dirs(data, "bicraft-aggs.json")? {
        let d = read_json(&dir.join("desktop.json"))?;
        let with = read_json(&dir.join("bicraft-aggs.json"))?;
        let (_, sw) = compare(&d, &with);
        let without = measures(&dir, "bicraft.json")
            .map(|p| read_json(&p))
            .transpose()?
            .map(|p| compare(&d, &p).1)
            .unwrap_or(Value::Null);
        let g = |s: &Value, k: &str| s.get(k).and_then(Value::as_f64);
        let a = &with["aggregations"];
        let facts = a["facts"].as_array().cloned().unwrap_or_default();
        let count: usize = facts
            .iter()
            .map(|f| f["aggregations"].as_array().map_or(0, Vec::len))
            .sum();
        let agg_rows = a["total_rows"].as_f64().unwrap_or(0.0);
        let _ = writeln!(
            md,
            "| {rows} | {} / {} | {} / {} | {} / {} | {} | {count} | {:.0} ({:.2} %) | {:.1} MB | {} | {} of {} |",
            without["agree"],
            sw["agree"],
            f(g(&without, "cold_median_s")),
            f(g(&sw, "cold_median_s")),
            f(g(&without, "warm_median_s")),
            f(g(&sw, "warm_median_s")),
            f(g(&sw, "start_median_s")),
            agg_rows,
            100.0 * agg_rows / rows.max(1) as f64,
            a["total_bytes"].as_f64().unwrap_or(0.0) / 1e6,
            f(a["build_wall_s"].as_f64()),
            with["aggregations_used"]["hits"],
            with["aggregations_used"]["hits"].as_u64().unwrap_or(0)
                + with["aggregations_used"]["misses"].as_u64().unwrap_or(0),
        );
    }
    Ok(md)
}

// ---------------------------------------------------------------------------------------------
// Comparison
// ---------------------------------------------------------------------------------------------

fn cell_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Number(x), Value::Number(y)) => {
            let (x, y) = (x.as_f64().unwrap_or(f64::NAN), y.as_f64().unwrap_or(f64::NAN));
            x == y || (x - y).abs() <= 1e-9 * x.abs().max(y.abs()).max(1.0)
        }
        // a whole-valued double the engine returns as a number, bicraft as text or vice versa
        (Value::String(x), Value::String(y)) => {
            x == y || x.trim_end_matches("T00:00:00") == y.trim_end_matches("T00:00:00")
        }
        (Value::Bool(x), Value::Bool(y)) => x == y,
        _ => false,
    }
}

/// Column names of every result set differ only in order (`Some(description)`).
pub fn column_order(d: &Value, p: &Value) -> Option<String> {
    let (ds, ps) = (d.as_array()?, p.as_array()?);
    for (a, b) in ds.iter().zip(ps) {
        let n = |s: &Value| -> Vec<String> {
            s.get("columns")
                .and_then(Value::as_array)
                .map(|c| c.iter().map(|x| str_of(x, "name").to_string()).collect())
                .unwrap_or_default()
        };
        let (na, nb) = (n(a), n(b));
        if !na.is_empty() && na != nb {
            return Some(format!("column order: engine {na:?}, bicraft {nb:?}"));
        }
    }
    None
}

/// `None` when the results agree, else the first difference.
pub fn diff_results(d: &Value, p: &Value) -> Option<String> {
    let (Some(ds), Some(ps)) = (d.as_array(), p.as_array()) else {
        return Some("missing results".into());
    };
    if ds.len() != ps.len() {
        return Some(format!("{} result sets vs {}", ds.len(), ps.len()));
    }
    for (k, (a, b)) in ds.iter().zip(ps).enumerate() {
        let ra = a
            .get("rows")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut rb = b
            .get("rows")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        // same columns in another order: compared by name (the order is reported by `column_order`)
        let names = |s: &Value| -> Vec<String> {
            s.get("columns")
                .and_then(Value::as_array)
                .map(|c| c.iter().map(|x| str_of(x, "name").to_string()).collect())
                .unwrap_or_default()
        };
        let (na, nb) = (names(a), names(b));
        if !na.is_empty() && na != nb {
            let perm: Option<Vec<usize>> = na.iter().map(|n| nb.iter().position(|m| m == n)).collect();
            match perm {
                Some(p) if nb.len() == na.len() => {
                    rb = rb
                        .iter()
                        .map(|row| {
                            Value::Array(
                                p.iter()
                                    .map(|i| row.get(*i).cloned().unwrap_or(Value::Null))
                                    .collect(),
                            )
                        })
                        .collect();
                }
                _ => return Some(format!("set {k}: columns {na:?} vs {nb:?}")),
            }
        }
        if ra.len() != rb.len() {
            return Some(format!("set {k}: {} rows vs {}", ra.len(), rb.len()));
        }
        for (i, (x, y)) in ra.iter().zip(&rb).enumerate() {
            let (xs, ys) = (
                x.as_array().cloned().unwrap_or_default(),
                y.as_array().cloned().unwrap_or_default(),
            );
            if xs.len() != ys.len() || xs.iter().zip(&ys).any(|(u, v)| !cell_eq(u, v)) {
                return Some(format!("set {k} row {i}: {x} vs {y}"));
            }
        }
    }
    None
}

fn f(v: Option<f64>) -> String {
    match v {
        Some(s) if s < 1.0 => format!("{:.1} ms", s * 1000.0),
        Some(s) => format!("{s:.2} s"),
        None => "–".into(),
    }
}

/// BIcraft's answers `p` checked against the reference answers `d`, query by query, with
/// BIcraft's own times (start, cold, warm) of the queries whose answers are the same.
pub fn compare(d: &Value, p: &Value) -> (String, Value) {
    let empty = Vec::new();
    let dq = d.get("queries").and_then(Value::as_array).unwrap_or(&empty);
    let pq = p.get("queries").and_then(Value::as_array).unwrap_or(&empty);
    let mut md = String::new();
    let _ = writeln!(
        md,
        "| Query | bicraft start | bicraft cold | bicraft warm | Result |"
    );
    let _ = writeln!(md, "|---|---:|---:|---:|---|");
    let (mut cold, mut warm, mut start) = (Vec::new(), Vec::new(), Vec::new());
    let mut rows = Vec::new();
    for q in pq {
        let id = str_of(q, "id");
        let dd = dq.iter().find(|x| str_of(x, "id") == id);
        let p_start = p_start_of(p, q);
        // without a comparable cold run (older measures): the first run of the session
        let p_cold = q
            .get("cold_s")
            .and_then(Value::as_f64)
            .or_else(|| q.get("first_s").and_then(Value::as_f64));
        let p_warm = q.get("warm_s").and_then(Value::as_f64);
        let verdict = if let Some(e) = q.get("round_mismatch").and_then(Value::as_str) {
            format!(
                "DIFFERENT between rounds: {}",
                e.chars().take(160).collect::<String>()
            )
        } else if let Some(e) = q.get("error").and_then(Value::as_str) {
            format!("refused by bicraft: {}", e.chars().take(140).collect::<String>())
        } else if dd.and_then(|x| x.get("error")).is_some() {
            "reference error".to_string()
        } else {
            match (dd.and_then(|x| x.get("results")), q.get("results")) {
                (Some(a), Some(b)) => match diff_results(a, b) {
                    Some(e) => format!("DIFFERENT: {}", e.chars().take(160).collect::<String>()),
                    None => match column_order(a, b) {
                        Some(o) => format!("same values, {o}"),
                        None => "same".into(),
                    },
                },
                _ => "not compared".into(),
            }
        };
        let ok = is_same(&verdict);
        if ok {
            start.extend(p_start);
            cold.extend(p_cold);
            warm.extend(p_warm);
        }
        let shown = |x: Option<f64>| if ok { f(x) } else { "–".into() };
        let _ = writeln!(
            md,
            "| {id} {} | {} | {} | {} | {verdict} |",
            str_of(q, "title"),
            shown(p_start),
            shown(p_cold),
            shown(p_warm)
        );
        rows.push(
            json!({"id": id, "bicraft_start_s": p_start, "bicraft_cold_s": p_cold,
            "bicraft_warm_s": p_warm, "verdict": verdict}),
        );
    }
    let agree = rows
        .iter()
        .filter(|x| x["verdict"].as_str().is_some_and(is_same))
        .count();
    let summary = json!({
        "agree": agree,
        "queries": rows.len(),
        "start_median_s": median(&start),
        "cold_median_s": median(&cold),
        "warm_median_s": median(&warm),
        "rounds": p.get("rounds").and_then(Value::as_u64).unwrap_or(1),
        "rows": rows,
    });
    let _ = writeln!(
        md,
        "\n{agree} of {} queries return the reference answers; bicraft medians over them: start {}, cold {}, warm {}",
        summary["queries"],
        f(median(&start)),
        f(median(&cold)),
        f(median(&warm))
    );
    (md, summary)
}

/// BIcraft's start for query `q` of the measures `p`: process start + `file.open` + the query in a
/// fresh session (`fresh_start_s`). Older measures timed the query alone (`fresh_s`): their
/// session's `file.open` (`fresh_open_s`, else `open_s`) is added (the process start, ~30 ms, is
/// missing).
fn p_start_of(p: &Value, q: &Value) -> Option<f64> {
    if let Some(s) = q.get("fresh_start_s").and_then(Value::as_f64) {
        return Some(s);
    }
    let query = q
        .get("fresh_s")
        .and_then(Value::as_f64)
        .or_else(|| q.get("first_s").and_then(Value::as_f64))?;
    let open = p
        .get("fresh_open_s")
        .and_then(Value::as_f64)
        .or_else(|| p.get("open_s").and_then(Value::as_f64))
        .unwrap_or(0.0);
    Some(open + query)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries_parse_and_ids_are_unique() {
        let qs = queries().unwrap();
        assert!(qs.len() >= 20);
        let mut ids: Vec<&str> = qs.iter().map(|q| str_of(q, "id")).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), qs.len());
        assert!(qs.iter().all(|q| str_of(q, "dax").contains("EVALUATE")));
    }

    #[test]
    fn calendar_is_five_years() {
        assert_eq!(calendar_days(), 1826);
        assert_eq!(ymd(0), (2021, 1, 1));
        assert_eq!(ymd(59), (2021, 3, 1));
        assert_eq!(ymd(1826 - 1), (2025, 12, 31));
        assert_eq!(ymd(365 * 3 + 59), (2024, 2, 29));
        // year boundaries (G37)
        assert_eq!(ymd(364), (2021, 12, 31));
        assert_eq!(ymd(365), (2022, 1, 1));
        assert_eq!(ymd(365 * 3 + 60), (2024, 3, 1));
        assert_eq!(ymd(365 * 3 + 365), (2024, 12, 31));
        assert_eq!(ymd(365 * 3 + 366), (2025, 1, 1));
    }

    #[test]
    fn sales_are_deterministic_and_in_range() {
        let a = sale(42, 1_000, 1826);
        assert_eq!(a, sale(42, 1_000, 1826));
        for k in 1..2_000 {
            let (d, p, s, c, q, amount) = sale(k, 1_000, 1826);
            assert!(d < 1826 && (1..=PRODUCTS).contains(&p) && (1..=STORES).contains(&s));
            assert!((1..=1_000).contains(&c) && (1..=10).contains(&q));
            assert!(amount <= q * price_cents(p));
        }
    }

    #[test]
    fn build_commands_end_with_the_save() {
        let c = build_commands(Path::new("C:/x"), Path::new("C:/x/bench.pbix"));
        let a = c.as_array().unwrap();
        assert_eq!(a[0]["command"], "model.new");
        assert_eq!(a.last().unwrap()["command"], "file.save_as");
        assert_eq!(
            a.iter().filter(|x| x["command"] == "measure.add").count(),
            MEASURES.len()
        );
    }

    #[test]
    fn compare_flags_differences() {
        let d = json!({"queries": [{"id": "Q01", "cold_s": 0.01, "warm_s": 0.002,
            "results": [{"rows": [[1.0, "2024-01-01T00:00:00"]]}]}]});
        let same = json!({"queries": [{"id": "Q01", "title": "t", "first_s": 0.02, "warm_s": 0.004,
            "results": [{"rows": [[1.000_000_000_000_1, "2024-01-01T00:00:00"]]}]}]});
        let (_, s) = compare(&d, &same);
        assert_eq!(s["agree"], 1);
        assert_eq!(s["warm_median_s"], 0.004);
        let other = json!({"queries": [{"id": "Q01", "title": "t", "first_s": 0.02, "warm_s": 0.004,
            "results": [{"rows": [[2.0, "2024-01-01T00:00:00"]]}]}]});
        let (md, s) = compare(&d, &other);
        assert_eq!(s["agree"], 0);
        assert!(md.contains("DIFFERENT"));
    }

    /// The bench report applies to the bench model and every page derives what it needs.
    #[test]
    fn the_bench_report_applies_and_derives_requirements() {
        let mut s = bicraft_engine::Session::new();
        let mut run = |id: &str, p: Value| {
            s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"));
        };
        run("model.new", json!({"name": "Bench", "culture": "en-US"}));
        let tables: [(&str, &[(&str, &str)]); 4] = [
            (
                "Produit",
                &[
                    ("ProduitKey", "int64"),
                    ("Produit", "string"),
                    ("Categorie", "string"),
                    ("SousCategorie", "string"),
                    ("PrixCatalogue", "decimal"),
                ],
            ),
            (
                "Magasin",
                &[
                    ("MagasinKey", "int64"),
                    ("Magasin", "string"),
                    ("Region", "string"),
                ],
            ),
            (
                "Calendrier",
                &[
                    ("Date", "dateTime"),
                    ("Annee", "int64"),
                    ("MoisNum", "int64"),
                    ("AnneeMois", "int64"),
                ],
            ),
            (
                "Ventes",
                &[
                    ("Date", "dateTime"),
                    ("ProduitKey", "int64"),
                    ("MagasinKey", "int64"),
                    ("ClientKey", "int64"),
                    ("Quantite", "int64"),
                    ("Montant", "decimal"),
                ],
            ),
        ];
        for (t, cols) in tables {
            run(
                "table.add",
                json!({"name": t, "partition": {"source_type": "m", "expression": "#table({}, {})"}}),
            );
            for (c, ty) in cols {
                run("column.add", json!({"table": t, "name": c, "data_type": ty}));
            }
        }
        run(
            "table.set",
            json!({"name": "Calendrier", "data_category": "Time", "key_column": "Date"}),
        );
        for (from, to) in [
            ("Ventes[Date]", "Calendrier[Date]"),
            ("Ventes[ProduitKey]", "Produit[ProduitKey]"),
            ("Ventes[MagasinKey]", "Magasin[MagasinKey]"),
        ] {
            run("relationship.add", json!({"from": from, "to": to}));
        }
        for (name, dax, _) in MEASURES {
            run(
                "measure.add",
                json!({"table": "Ventes", "name": name, "expression": dax}),
            );
        }
        run("report.new", json!({}));
        for c in report_commands() {
            run(str_of(&c, "command"), c["params"].clone());
        }
        let a = s.execute("dax.aggregations", &json!({})).unwrap();
        let visuals = a["visuals"].as_array().cloned().unwrap_or_default();
        // one visual per query page (the slicers too, which read no measure, are left out)
        assert_eq!(visuals.len(), queries().unwrap().len(), "{a}");
        let page_of = |id: &str| {
            visuals
                .iter()
                .find(|v| v["grain"].to_string().contains(id) || v["values"].to_string().contains(id))
                .cloned()
        };
        // Q21: the visual filter on a fact column joins the grain; Q10: the slicer and TOTALYTD's dates
        let q21 = page_of("'Produit'[SousCategorie]").unwrap();
        assert!(q21["grain"].to_string().contains("'Ventes'[Quantite]"), "{q21}");
        let q10 = visuals
            .iter()
            .find(|v| {
                v["grain"].to_string().contains("'Calendrier'[Date]")
                    && v["grain"].to_string().contains("AnneeMois")
            })
            .unwrap();
        assert!(q10["grain"].to_string().contains("'Calendrier'[Annee]"), "{q10}");
    }

    fn round(cold: f64, warm: f64, start: f64, cell: f64, peak: u64) -> Value {
        json!({"open_s": 0.5, "fresh_open_s": 0.4, "memory": {"peak_working_set": peak},
            "queries": [{"id": "Q01", "title": "t", "cold_s": cold, "warm_s": warm,
            "fresh_s": start - 0.4, "fresh_start_s": start, "results": [{"rows": [[cell]]}]}]})
    }

    /// Start counts BIcraft's own open: new measures carry it, older ones get `fresh_open_s` added.
    #[test]
    fn start_includes_the_open() {
        let new = json!({"fresh_open_s": 1.0, "queries": []});
        let q = json!({"fresh_s": 0.2, "fresh_start_s": 1.25});
        assert_eq!(p_start_of(&new, &q), Some(1.25));
        let old = json!({"fresh_open_s": 1.0, "open_s": 3.0});
        assert_eq!(p_start_of(&old, &json!({"fresh_s": 0.25})), Some(1.25));
        let older = json!({"open_s": 3.0});
        assert_eq!(p_start_of(&older, &json!({"first_s": 0.5})), Some(3.5));
    }

    #[test]
    fn rounds_merge_to_medians_and_keep_the_slowest() {
        let rounds = [
            round(0.010, 0.004, 1.0, 1.0, 100),
            round(0.030, 0.002, 3.0, 1.0, 300),
            round(0.020, 0.003, 2.0, 1.0, 200),
        ];
        let m = merge_rounds(&rounds).unwrap();
        assert_eq!(m["rounds"], 3);
        assert_eq!(m["memory"]["peak_working_set"], 300);
        let q = &m["queries"][0];
        assert_eq!(q["cold_s"], 0.020);
        assert_eq!(q["warm_s"], 0.003);
        assert_eq!(q["fresh_start_s"], 2.0);
        assert_eq!(q["rounds"]["cold"], json!([0.010, 0.030, 0.020]));
        assert!(q.get("round_mismatch").is_none());
        let d = json!({"queries": [{"id": "Q01", "results": [{"rows": [[1.0]]}]}]});
        let (_, s) = compare(&d, &m);
        assert_eq!(s["agree"], 1);
        // a round with another value: the query no longer counts as the same
        let bad = [round(0.01, 0.004, 1.0, 1.0, 1), round(0.01, 0.004, 1.0, 2.0, 1)];
        let m = merge_rounds(&bad).unwrap();
        assert!(
            m["queries"][0]["round_mismatch"]
                .as_str()
                .unwrap()
                .starts_with("round 2")
        );
        let (_, s) = compare(&d, &m);
        assert_eq!(s["agree"], 0);
        assert!(merge_rounds(&[]).is_err());
        assert!(merge_rounds(&[json!([1, 2])]).is_err());
    }

    /// `check` fails on a size folder whose answers differ from the reference, not on times.
    #[test]
    fn check_counts_answers_that_differ() {
        let dir = std::env::temp_dir().join(format!("engine-bench-check-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let size = dir.join("r1m");
        std::fs::create_dir_all(&size).unwrap();
        let d = json!({"rows": 1_000_000, "queries": [{"id": "Q01", "results": [{"rows": [[1.0]]}]}]});
        std::fs::write(size.join("desktop.json"), d.to_string()).unwrap();
        let slow = merge_rounds(&[round(30.0, 9.0, 60.0, 1.0, 1)]).unwrap();
        std::fs::write(size.join("bicraft.json"), slow.to_string()).unwrap();
        let (report, bad) = check(&dir, "bicraft.json").unwrap();
        assert_eq!(bad, 0, "{report}");
        let wrong = merge_rounds(&[round(0.01, 0.001, 1.0, 2.0, 1)]).unwrap();
        std::fs::write(size.join("bicraft.json"), wrong.to_string()).unwrap();
        let (report, bad) = check(&dir, "bicraft.json").unwrap();
        assert_eq!(bad, 1);
        assert!(report.contains("Q01: DIFFERENT"), "{report}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
