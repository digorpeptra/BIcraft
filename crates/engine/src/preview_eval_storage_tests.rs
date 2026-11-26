//! The storage engine's micro-bench and exactness tests (`docs/storage-engine.md`): the star
//! schema of the engine bench (`xtask/src/engine_bench.rs`, same generator) built in memory,
//! columns put straight into the preview cache (no file, no CSV, no Desktop), and the bench's
//! queries plus edge cases run with the storage engine and with the reference evaluator (the
//! per-cell path, [`super::set_storage_engine`] off), which must agree.
//!
//! - `cargo test -p bicraft-engine storage_` : exactness at 3 k rows (clean and dirty data);
//! - `cargo test --release -p bicraft-engine storage_micro -- --ignored --nocapture` : time per
//!   query (first run, comparable cold: `dax.clear_cache` before each run, warm) and rows per second at `BICRAFT_MICRO_ROWS` rows (default 1 M), storage engine on, and
//!   off up to `BICRAFT_MICRO_REF_ROWS` (default 100 k); `BICRAFT_SCAN_THREADS` sets the threads,
//!   `BICRAFT_MICRO_ONLY=Q05,Q13` runs some of the queries, `BICRAFT_MICRO_QUERY` others (`;;`
//!   between queries).

use std::time::Instant;

use bicraft_vertipaq::CellValue;
use serde_json::{Value, json};

use super::super::store::ColData;
use crate::Session;

const PRODUCTS: u64 = 1_000;
const STORES: u64 = 200;
const CATEGORIES: u64 = 10;
const SUBCATEGORIES: u64 = 50;
const REGIONS: u64 = 8;
const FIRST_YEAR: i64 = 2021;
const YEARS: i64 = 5;
/// Serial of 2021-01-01.
const DAY0: f64 = 44_197.0;

const MEASURES: &[(&str, &str)] = &[
    ("CA", "SUM(Ventes[Montant])"),
    ("Qte", "SUM(Ventes[Quantite])"),
    ("Nb clients", "DISTINCTCOUNT(Ventes[ClientKey])"),
    ("CA YTD", "TOTALYTD([CA], Calendrier[Date])"),
    ("CA N-1", "CALCULATE([CA], SAMEPERIODLASTYEAR(Calendrier[Date]))"),
    ("CA Var N-1", "DIVIDE([CA] - [CA N-1], [CA N-1])"),
    ("Part CA", "DIVIDE([CA], CALCULATE([CA], ALL(Produit)))"),
    (
        "Rang produit",
        "IF(HASONEVALUE(Produit[Produit]), RANKX(ALL(Produit[Produit]), [CA]))",
    ),
];

fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days() -> i64 {
    (FIRST_YEAR..FIRST_YEAR + YEARS)
        .map(|y| if is_leap(y) { 366 } else { 365 })
        .sum()
}

fn price_cents(p: u64) -> u64 {
    500 + splitmix(p ^ 0xC0FFEE) % 50_000
}

fn text(s: String) -> CellValue {
    CellValue::Text(s)
}

fn int(n: u64) -> CellValue {
    CellValue::Int(n as i64)
}

fn col(values: Vec<CellValue>) -> ColData {
    ColData::from_stored(values).unwrap()
}

/// The bench's star schema with `rows` sales; `dirty`: some sales with an unknown product, a
/// blank amount, customer or quantity.
/// The sales columns are kept compressed (`packed`, as a stored model's are) or in memory.
pub(super) fn star_with(rows: u64, dirty: bool, packed: bool) -> Session {
    star_opts(
        rows,
        dirty,
        packed,
        bicraft_env::var("BICRAFT_MICRO_SORT").is_ok(),
    )
}

/// [`star_with`], the sales of each million rows sorted by quantity then store when `sorted`.
pub(super) fn star_opts(rows: u64, dirty: bool, packed: bool, sorted: bool) -> Session {
    let fact = |c: ColData| -> ColData {
        if packed {
            ColData {
                // sorted as VertiPaq sorts, encoded as it encodes: segments of a million rows,
                // value encoding for many distinct numbers only
                codes: if sorted {
                    super::super::store::Codes::pack_with(&c, 1 << 20, 10_000)
                } else {
                    super::super::store::Codes::pack(&c, 65_536)
                },
                ..c
            }
        } else {
            c
        }
    };
    let mut s = Session::new();
    let mut steps: Vec<(&str, Value)> = vec![("model.new", json!({"name": "Bench", "culture": "en-US"}))];
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
        steps.push((
            "table.add",
            json!({"name": t, "partition": {"source_type": "m", "expression": "#table({}, {})"}}),
        ));
        for (c, ty) in cols {
            steps.push(("column.add", json!({"table": t, "name": c, "data_type": ty})));
        }
    }
    steps.push((
        "table.set",
        json!({"name": "Calendrier", "data_category": "Time", "key_column": "Date"}),
    ));
    for (from, to) in [
        ("Ventes[Date]", "Calendrier[Date]"),
        ("Ventes[ProduitKey]", "Produit[ProduitKey]"),
        ("Ventes[MagasinKey]", "Magasin[MagasinKey]"),
    ] {
        steps.push(("relationship.add", json!({"from": from, "to": to})));
    }
    for (name, dax) in MEASURES {
        steps.push((
            "measure.add",
            json!({"table": "Ventes", "name": name, "expression": dax}),
        ));
    }
    for (id, p) in steps {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"));
    }

    let mut p: [Vec<CellValue>; 5] = Default::default();
    for k in 1..=PRODUCTS {
        let sub = 1 + splitmix(k ^ 0xAB) % SUBCATEGORIES;
        let cat = 1 + (sub - 1) % CATEGORIES;
        p[0].push(int(k));
        p[1].push(text(format!("P{k:04}")));
        p[2].push(text(format!("Cat{cat:02}")));
        p[3].push(text(format!("Sous{sub:02}")));
        p[4].push(CellValue::Decimal(price_cents(k) as i64 * 100));
    }
    let [a, b, c, d, e] = p;
    super::super::inject_table(
        &s,
        "Produit",
        PRODUCTS as usize,
        vec![
            ("ProduitKey".into(), col(a)),
            ("Produit".into(), col(b)),
            ("Categorie".into(), col(c)),
            ("SousCategorie".into(), col(d)),
            ("PrixCatalogue".into(), col(e)),
        ],
    );
    let mut m: [Vec<CellValue>; 3] = Default::default();
    for k in 1..=STORES {
        let region = 1 + splitmix(k ^ 0x5EED) % REGIONS;
        m[0].push(int(k));
        m[1].push(text(format!("M{k:03}")));
        m[2].push(text(format!("Region{region}")));
    }
    let [a, b, c] = m;
    super::super::inject_table(
        &s,
        "Magasin",
        STORES as usize,
        vec![
            ("MagasinKey".into(), col(a)),
            ("Magasin".into(), col(b)),
            ("Region".into(), col(c)),
        ],
    );
    let n_days = days();
    let mut cal: [Vec<CellValue>; 4] = Default::default();
    for d in 0..n_days {
        let (y, mo, _) = super::ymd(DAY0 as i64 + d);
        cal[0].push(CellValue::DateTime(DAY0 + d as f64));
        cal[1].push(CellValue::Int(y));
        cal[2].push(CellValue::Int(mo));
        cal[3].push(CellValue::Int(y * 100 + mo));
    }
    let [a, b, c, d] = cal;
    super::super::inject_table(
        &s,
        "Calendrier",
        n_days as usize,
        vec![
            ("Date".into(), col(a)),
            ("Annee".into(), col(b)),
            ("MoisNum".into(), col(c)),
            ("AnneeMois".into(), col(d)),
        ],
    );
    let customers = (rows / 10).max(1);
    let mut v: [Vec<CellValue>; 6] = Default::default();
    for k in 1..=rows {
        let r = splitmix(k);
        let r2 = splitmix(r);
        let day = r % n_days as u64;
        let mut product = 1 + (r >> 20) % PRODUCTS;
        let store = 1 + (r >> 40) % STORES;
        let customer = 1 + r2 % customers;
        let qty = 1 + (r2 >> 40) % 10;
        let discount = (r2 >> 50) % 31;
        let amount = qty * price_cents(product) * (100 - discount) / 100;
        let mut amount = CellValue::Decimal(amount as i64 * 100);
        let mut customer = int(customer);
        let mut quantity = int(qty);
        if dirty {
            match k % 97 {
                0 => product = 5_000 + k % 3,
                1 => amount = CellValue::Null,
                2 => customer = CellValue::Null,
                3 => quantity = CellValue::Null,
                _ => {}
            }
        }
        v[0].push(CellValue::DateTime(DAY0 + day as f64));
        v[1].push(int(product));
        v[2].push(int(store));
        v[3].push(customer);
        v[4].push(quantity);
        v[5].push(amount);
    }
    // `BICRAFT_MICRO_SORT`: the rows of each million sorted by quantity then store, as
    // VertiPaq orders a segment (long runs, RLE)
    if sorted {
        let key = |c: &CellValue| match c {
            CellValue::Int(x) => *x,
            _ => i64::MIN,
        };
        let n = v[0].len();
        let mut perm: Vec<usize> = (0..n).collect();
        for chunk in perm.chunks_mut(1 << 20) {
            chunk.sort_by_key(|i| (key(&v[4][*i]), key(&v[2][*i])));
        }
        for c in v.iter_mut() {
            let old = std::mem::take(c);
            *c = perm.iter().map(|i| old[*i].clone()).collect();
        }
    }
    let [a, b, c, d, e, f] = v;
    super::super::inject_table(
        &s,
        "Ventes",
        rows as usize,
        vec![
            ("Date".into(), fact(col(a))),
            ("ProduitKey".into(), fact(col(b))),
            ("MagasinKey".into(), fact(col(c))),
            ("ClientKey".into(), fact(col(d))),
            ("Quantite".into(), fact(col(e))),
            ("Montant".into(), fact(col(f))),
        ],
    );
    s
}

/// The 22 queries of the engine bench.
pub(super) fn bench_queries() -> Vec<(String, String)> {
    let q: Value = serde_json::from_str(include_str!("../../../xtask/engine-bench/queries.json")).unwrap();
    q["queries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|q| {
            (
                q["id"].as_str().unwrap().to_string(),
                q["dax"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

/// Edge cases of the storage engine's decompositions (blank rows, several filters on a column,
/// fact columns as groups, ALL inside a grouping, every aggregation).
pub(super) fn edge_queries() -> Vec<(String, String)> {
    [
        "EVALUATE SUMMARIZECOLUMNS(Produit[Categorie], Magasin[Region], \"N\", COUNTROWS(Ventes), \
         \"D\", DISTINCTCOUNT(Ventes[ClientKey]), \"DN\", DISTINCTCOUNTNOBLANK(Ventes[ClientKey]), \
         \"Mi\", MIN(Ventes[Montant]), \"Ma\", MAX(Ventes[Date]), \"Av\", AVERAGE(Ventes[Quantite]), \
         \"AvC\", AVERAGE(Ventes[Montant]), \"C\", COUNT(Ventes[Montant]), \"CB\", COUNTBLANK(Ventes[Montant])) \
         ORDER BY Produit[Categorie], Magasin[Region]",
        "EVALUATE SUMMARIZECOLUMNS(Produit[Categorie], Produit[SousCategorie], \"x\", \
         CALCULATE([CA], ALL(Produit[Categorie])), \"y\", [Part CA]) ORDER BY Produit[Categorie], Produit[SousCategorie]",
        "EVALUATE SUMMARIZECOLUMNS(Calendrier[Annee], TREATAS({2022, 2023}, Calendrier[Annee]), \"CA\", [CA], \
         \"Med\", MEDIAN(Ventes[Quantite]), \"Sd\", STDEV.S(Ventes[Quantite])) ORDER BY Calendrier[Annee]",
        "EVALUATE ROW(\"a\", CALCULATE([CA], Ventes[Quantite] = 3), \"b\", CALCULATE([CA], Ventes[Quantite] = 3, \
         Ventes[Quantite] = 4), \"c\", CALCULATE([Nb clients], Produit[Categorie] = \"Cat03\", Magasin[Region] = \"Region2\"))",
        "EVALUATE SUMMARIZECOLUMNS(Ventes[Quantite], \"CA\", [CA], \"Cl\", [Nb clients]) ORDER BY Ventes[Quantite]",
        "EVALUATE ROW(\"s\", SUMX(VALUES(Magasin[Region]), [CA]), \"m\", MAXX(VALUES(Produit[Categorie]), [Qte]))",
        "EVALUATE SUMMARIZECOLUMNS(Magasin[Region], \"k\", CALCULATE([CA], KEEPFILTERS(Magasin[Region] IN \
         {\"Region1\", \"Region3\"}))) ORDER BY Magasin[Region]",
        "EVALUATE SUMMARIZECOLUMNS(Produit[Categorie], \"CA\", [CA], \"Lines\", COUNTROWS(Ventes)) ORDER BY Produit[Categorie]",
        "EVALUATE ADDCOLUMNS(VALUES(Calendrier[Annee]), \"CA\", [CA], \"YTD\", CALCULATE([CA YTD], \
         Calendrier[MoisNum] = 6)) ORDER BY Calendrier[Annee]",
        "EVALUATE ROW(\"n\", COUNTROWS(FILTER(Ventes, Ventes[Quantite] > 8)), \"f\", COUNTROWS(Ventes), \
         \"v\", COUNTROWS(VALUES(Ventes[ClientKey])))",
        "EVALUATE ROW(\"s\", SUMX(FILTER(Ventes, Ventes[Quantite] <= 3), Ventes[Montant]), \
         \"a\", AVERAGEX(Ventes, Ventes[Montant]), \"q\", SUMX(Ventes, Ventes[Quantite]), \
         \"aq\", AVERAGEX(FILTER(Ventes, Ventes[Quantite] > 2), Ventes[Quantite]), \
         \"mi\", MINX(Ventes, Ventes[Montant]), \"ma\", MAXX(FILTER(Ventes, Ventes[Quantite] = 4), Ventes[Quantite]), \
         \"c\", COUNTX(Ventes, Ventes[ClientKey]), \"cf\", COUNTX(FILTER(Ventes, Ventes[Quantite] = 9), Ventes[Montant]))",
        "EVALUATE SUMMARIZECOLUMNS(Produit[Categorie], \"s\", SUMX(FILTER(Ventes, Ventes[Quantite] >= 5), Ventes[Montant]), \
         \"q\", SUMX(Ventes, Ventes[Quantite] * 2)) ORDER BY Produit[Categorie]",
        "EVALUATE ROW(\"a\", COUNTROWS(SUMMARIZECOLUMNS(Ventes[Quantite], \"m\", SUM(Ventes[Montant]))), \
         \"b\", COUNTROWS(CALCULATETABLE(SUMMARIZECOLUMNS(Ventes[ClientKey], \"CA\", [CA]), Produit[Categorie] = \"Cat03\")), \
         \"c\", COUNTROWS(SUMMARIZECOLUMNS(Ventes[Montant], \"q\", SUM(Ventes[Quantite]))), \
         \"d\", COUNTROWS(CALCULATETABLE(SUMMARIZECOLUMNS(Ventes[ClientKey], \"n\", COUNTROWS(Ventes)), Ventes[Quantite] > 99)))",
        "EVALUATE SUMMARIZECOLUMNS(Produit[Categorie], Produit[Produit], TREATAS({\"Cat01\", \"Cat02\"}, Produit[Categorie]), \
         \"r\", RANKX(ALL(Produit[Produit]), [CA]), \"s\", RANKX(ALL(Produit[Produit]), SUM(Ventes[Montant])), \
         \"d\", RANKX(ALL(Produit[Produit]), [Qte], , ASC, DENSE), \"t\", RANKX(ALL(Produit), CALCULATE([CA])), \
         \"c\", RANKX(ALL(Produit[Categorie]), [CA])) ORDER BY Produit[Categorie], Produit[Produit]",
        "EVALUATE SUMMARIZECOLUMNS(Produit[Categorie], \"a\", SUMX(Ventes, Ventes[Quantite] * RELATED(Produit[PrixCatalogue])), \
         \"b\", SUMX(Ventes, RELATED(Produit[PrixCatalogue]) * Ventes[Quantite]), \
         \"c\", SUMX(Ventes, Ventes[Montant] * RELATED(Produit[PrixCatalogue])), \
         \"d\", CALCULATE(SUMX(Ventes, Ventes[Quantite] * RELATED(Magasin[MagasinKey])), Ventes[Quantite] > 4)) \
         ORDER BY Produit[Categorie]",
    ]
    .iter()
    .enumerate()
    .map(|(i, q)| (format!("E{:02}", i + 1), q.to_string()))
    .collect()
}

pub(super) fn evaluate(s: &mut Session, q: &str, storage: bool) -> Result<Value, String> {
    super::set_storage_engine(storage);
    // no wall-clock budget: a correctness test must not fail because the machine is busy
    let r = super::query::without_time_budget(|| {
        s.execute(
            "dax.evaluate",
            &json!({"query": q, "max_rows": 1_000_000, "timeout_ms": 600_000}),
        )
    });
    super::set_storage_engine(true);
    r.map(|mut v| {
        if let Some(o) = v.as_object_mut() {
            o.remove("elapsed_ms");
        }
        v
    })
}

pub(super) fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            let (x, y) = (x.as_f64().unwrap_or(f64::NAN), y.as_f64().unwrap_or(f64::NAN));
            x == y || (x - y).abs() <= 1e-12 * x.abs().max(y.abs())
        }
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(a, b)| same(a, b)),
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| same(v, w)))
        }
        _ => a == b,
    }
}

fn check(rows: u64, dirty: bool) {
    check_with(rows, dirty, true);
    check_with(rows, dirty, false);
}

fn check_with(rows: u64, dirty: bool, packed: bool) {
    check_on(star_with(rows, dirty, packed), packed);
}

fn check_on(mut s: Session, packed: bool) {
    let mut failures = Vec::new();
    for (id, q) in bench_queries().into_iter().chain(edge_queries()) {
        let reference = evaluate(&mut s, &q, false);
        let got = evaluate(&mut s, &q, true);
        match (&reference, &got) {
            (Ok(r), Ok(g)) if same(r, g) => {}
            // the reference refuses what it cannot do in its budget; the storage engine may answer
            (Err(_), Ok(_)) => {}
            (Err(a), Err(b)) if a == b => {}
            _ => failures.push(format!("{id}: reference {reference:?}\n  storage {got:?}")),
        }
    }
    assert!(
        failures.is_empty(),
        "packed {packed}: {} differences:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn storage_engine_matches_the_reference_evaluator() {
    check(3_000, false);
}

#[test]
fn storage_engine_matches_the_reference_on_dirty_data() {
    check(3_000, true);
}

/// Sorted sales (long runs of one quantity, RLE segments as VertiPaq writes them): the paths
/// that read runs (counts per run, first row per code, dead blocks) agree with the reference.
#[test]
fn storage_engine_matches_the_reference_on_sorted_runs() {
    check_on(star_opts(3_000, true, true, true), true);
    check_on(star_opts(3_000, false, true, true), true);
}

/// A smoke test of speed for the CI: every bench query on 20 k compressed rows answers, as the
/// reference does, and in a bound wide enough for any machine and a debug build (a gross
/// regression, such as a scan per cell, goes over it).
#[test]
fn storage_smoke_bench_queries_answer_in_bounds() {
    let mut s = star_with(20_000, true, true);
    for (id, q) in bench_queries() {
        let start = Instant::now();
        let got = evaluate(&mut s, &q, true).unwrap_or_else(|e| panic!("{id}: {e}"));
        let ms = start.elapsed().as_millis();
        assert!(ms < 20_000, "{id}: {ms} ms");
        let warm = Instant::now();
        assert_eq!(evaluate(&mut s, &q, true).unwrap(), got, "{id}: warm");
        assert!(
            warm.elapsed().as_millis() < 5_000,
            "{id}: warm {} ms",
            warm.elapsed().as_millis()
        );
    }
}

pub(super) fn env_rows(key: &str, default: u64) -> u64 {
    bicraft_env::var(key)
        .ok()
        .and_then(|v| v.replace('_', "").parse().ok())
        .unwrap_or(default)
}

fn median_ms(s: &mut Session, q: &str, storage: bool, reps: usize) -> Result<f64, String> {
    median_ms_with(s, q, storage, reps, false)
}

/// The median time of `reps` runs; `cold`: the caches emptied before each (`dax.clear_cache`).
pub(super) fn median_ms_with(
    s: &mut Session,
    q: &str,
    storage: bool,
    reps: usize,
    cold: bool,
) -> Result<f64, String> {
    let mut t = Vec::new();
    for _ in 0..reps {
        if cold {
            s.execute("dax.clear_cache", &json!({}))?;
        }
        let start = Instant::now();
        evaluate(s, q, storage)?;
        t.push(start.elapsed().as_secs_f64() * 1e3);
    }
    t.sort_by(f64::total_cmp);
    // `BICRAFT_MICRO_BEST=1`: the best run (steadier on a busy machine), else the median
    if bicraft_env::var("BICRAFT_MICRO_BEST").is_ok() {
        return Ok(t.first().copied().unwrap_or(0.0));
    }
    Ok(t.get(t.len() / 2).copied().unwrap_or(0.0))
}

/// Time per bench query (first run, then the median of 3), storage engine on and, on small
/// data, off.
#[test]
#[ignore]
fn storage_micro() {
    let rows = env_rows("BICRAFT_MICRO_ROWS", 1_000_000);
    let ref_rows = env_rows("BICRAFT_MICRO_REF_ROWS", 100_000);
    let only = bicraft_env::var("BICRAFT_MICRO_ONLY").unwrap_or_default();
    let packed = bicraft_env::var("BICRAFT_MICRO_PLAIN").is_err();
    let t = Instant::now();
    let mut s = star_with(rows, false, packed);
    println!(
        "{rows} rows built in {:.2} s, {} scan threads, sales columns {}",
        t.elapsed().as_secs_f64(),
        bicraft_scan::configured_threads(),
        if packed {
            "packed"
        } else {
            "in memory (BICRAFT_MICRO_PLAIN)"
        }
    );
    println!("query  first ms    cold ms    warm ms   M rows/s (cold)   reference ms");
    let custom: Vec<(String, String)> = bicraft_env::var("BICRAFT_MICRO_QUERY")
        .map(|q| {
            q.split(";;")
                .enumerate()
                .map(|(i, q)| (format!("C{:02}", i + 1), q.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let all: Vec<(String, String)> = if custom.is_empty() {
        bench_queries().into_iter().chain(edge_queries()).collect()
    } else {
        custom
    };
    for (id, q) in all {
        if !only.is_empty() && !only.split(',').any(|o| o == id) {
            continue;
        }
        let start = Instant::now();
        let first = evaluate(&mut s, &q, true).map(|_| start.elapsed().as_secs_f64() * 1e3);
        let reps = env_rows("BICRAFT_MICRO_REPS", 3) as usize;
        let cold = median_ms_with(&mut s, &q, true, reps, true);
        let warm = median_ms(&mut s, &q, true, 3);
        let reference = if rows <= ref_rows {
            median_ms(&mut s, &q, false, 1).map_or_else(|e| format!("refused: {e}"), |m| format!("{m:.2}"))
        } else {
            "-".into()
        };
        let unpacked = super::super::store::UNPACKED.swap(0, std::sync::atomic::Ordering::Relaxed);
        let reference = if unpacked > 0 {
            format!("{reference}   (unpacked {unpacked} rows)")
        } else {
            reference
        };
        match (first, cold, warm) {
            (Ok(f), Ok(c), Ok(w)) => println!(
                "{id}   {f:>9.2} {c:>10.2} {w:>10.2} {:>12.0}      {reference}",
                rows as f64 / c.max(1e-3) / 1e3
            ),
            (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => println!("{id}   refused: {e}"),
        }
    }
}

#[test]
fn storage_engine_keeps_no_result_across_redefined_measures() {
    let mut s = star_with(2_000, false, true);
    let q = |body: &str| {
        format!(
            "DEFINE MEASURE Ventes[X] = {body} EVALUATE SUMMARIZECOLUMNS(Produit[Categorie], \"X\", [X]) \
             ORDER BY Produit[Categorie]"
        )
    };
    for body in [
        "SUM(Ventes[Montant])",
        "SUM(Ventes[Quantite])",
        "SUM(Ventes[Montant])",
    ] {
        let got = evaluate(&mut s, &q(body), true).unwrap();
        let want = evaluate(&mut s, &q(body), false).unwrap();
        assert!(same(&got, &want), "{body}: {got} / {want}");
    }
}
