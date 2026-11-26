//! The engine bench's variants (`xtask/src/engine_variants.rs`, docs/benchmarks.md "Big dimension
//! and skewed data") built in memory with the same generators, every column kept compressed as a
//! stored model's: their queries (`xtask/engine-bench/queries-{bigdim,skew}.json`) with the
//! storage engine, cold and warm, against the reference evaluator.
//!
//! - `cargo test -p bicraft-engine variant_` : exactness on small data (smaller still in a debug
//!   build, see `sized`; `cargo test --release -p bicraft-engine bigdim_tests` runs the full sizes:
//!   9 s for the module after a release build of the engine, 2026-10-29);
//! - `cargo test --release -p bicraft-engine variant_micro -- --ignored --nocapture` : time per
//!   query (first run, cold: `dax.clear_cache` before each run, warm) on
//!   `BICRAFT_MICRO_VARIANT` (`bigdim`, default, or `skew`) with `BICRAFT_MICRO_ROWS` sales
//!   (default 4 M) and `BICRAFT_MICRO_DIM_ROWS` customers (default: rows / 5 for `bigdim`,
//!   rows / 10 for `skew`), and the reference's answer compared up to `BICRAFT_MICRO_REF_ROWS`
//!   (default 200 k); `BICRAFT_MICRO_ONLY=B05,B07` runs some of the queries,
//!   `BICRAFT_MICRO_QUERY` others (`;;` between them).

use std::time::Instant;

use bicraft_vertipaq::CellValue;
use serde_json::{Value, json};

use super::super::store::{Codes, ColData};
use super::storage_tests::{env_rows, evaluate, median_ms_with, same};
use crate::Session;

const PRODUCTS: u64 = 1_000;
const STORES: u64 = 200;
const CATEGORIES: u64 = 10;
const SUBCATEGORIES: u64 = 50;
const REGIONS: u64 = 8;
const CITIES: u64 = 2_000;
const COUNTRIES: u64 = 40;
const SEGMENTS: u64 = 6;
const AGE_BANDS: u64 = 7;
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
    ("Nb commandes", "DISTINCTCOUNT(Ventes[NumCommande])"),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Variant {
    BigDim,
    Skew,
}

fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days() -> u64 {
    (FIRST_YEAR..FIRST_YEAR + YEARS)
        .map(|y| if is_leap(y) { 366 } else { 365 })
        .sum()
}

fn price_cents(p: u64) -> u64 {
    500 + splitmix(p ^ 0xC0FFEE) % 50_000
}

fn int(n: u64) -> CellValue {
    CellValue::Int(n as i64)
}

fn text(s: String) -> CellValue {
    CellValue::Text(s)
}

/// A column as a stored model keeps it: compressed segments.
fn packed(values: Vec<CellValue>) -> ColData {
    let c = ColData::from_stored(values).unwrap();
    ColData {
        codes: Codes::pack(&c, 65_536),
        ..c
    }
}

/// Integer Zipf-like weights `W / (k + offset)` (`engine_variants::Zipf`).
struct Zipf(Vec<u64>);

impl Zipf {
    fn new(n: u64, offset: u64) -> Zipf {
        let mut cum = Vec::new();
        let mut total = 0u64;
        for k in 1..=n {
            total = total.saturating_add((1u64 << 40) / (k + offset).max(1));
            cum.push(total);
        }
        Zipf(cum)
    }

    fn draw(&self, r: u64) -> u64 {
        let total = self.0.last().copied().unwrap_or(0).max(1);
        let x = r % total;
        self.0.partition_point(|&c| c <= x) as u64 + 1
    }
}

/// The variant's model (no data yet).
fn model(variant: Variant) -> Session {
    let mut s = Session::new();
    let mut steps: Vec<(&str, Value)> = vec![("model.new", json!({"name": "Bench", "culture": "en-US"}))];
    let mut ventes: Vec<(&str, &str)> = vec![
        ("Date", "dateTime"),
        ("ProduitKey", "int64"),
        ("MagasinKey", "int64"),
        ("ClientKey", "int64"),
        ("Quantite", "int64"),
        ("Montant", "decimal"),
    ];
    if variant == Variant::Skew {
        ventes.insert(4, ("NumCommande", "string"));
    }
    let tables: Vec<(&str, Vec<(&str, &str)>)> = vec![
        (
            "Produit",
            vec![
                ("ProduitKey", "int64"),
                ("Produit", "string"),
                ("Categorie", "string"),
                ("SousCategorie", "string"),
                ("PrixCatalogue", "decimal"),
            ],
        ),
        (
            "Magasin",
            vec![
                ("MagasinKey", "int64"),
                ("Magasin", "string"),
                ("Region", "string"),
            ],
        ),
        (
            "Calendrier",
            vec![
                ("Date", "dateTime"),
                ("Annee", "int64"),
                ("MoisNum", "int64"),
                ("AnneeMois", "int64"),
            ],
        ),
        (
            "Client",
            vec![
                ("ClientKey", "int64"),
                ("Client", "string"),
                ("Segment", "string"),
                ("Ville", "string"),
                ("Pays", "string"),
                ("TrancheAge", "string"),
                ("Fidelite", "string"),
            ],
        ),
        ("Ventes", ventes),
    ];
    for (t, cols) in &tables {
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
        ("Ventes[ClientKey]", "Client[ClientKey]"),
    ] {
        steps.push(("relationship.add", json!({"from": from, "to": to})));
    }
    for (name, dax) in MEASURES {
        if *name == "Nb commandes" && variant != Variant::Skew {
            continue;
        }
        steps.push((
            "measure.add",
            json!({"table": "Ventes", "name": name, "expression": dax}),
        ));
    }
    for (id, p) in steps {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"));
    }
    s
}

fn inject(s: &Session, table: &str, rows: usize, cols: Vec<(&str, Vec<CellValue>)>) {
    let cols = cols
        .into_iter()
        .map(|(n, v)| (n.to_string(), packed(v)))
        .collect();
    super::super::inject_table(s, table, rows, cols);
}

/// The variant with `rows` sales, `customers` customers and (skew) `products` products.
pub(super) fn variant(variant: Variant, rows: u64, customers: u64, products: u64) -> Session {
    let s = model(variant);
    fill(&s, variant, rows, customers, products);
    s
}

/// Puts the variant's data in a session holding its model (and possibly more).
fn fill(s: &Session, variant: Variant, rows: u64, customers: u64, products: u64) {
    let products = if variant == Variant::Skew {
        products
    } else {
        PRODUCTS
    };
    let mut p: [Vec<CellValue>; 5] = Default::default();
    for k in 1..=products {
        let sub = 1 + splitmix(k ^ 0xAB) % SUBCATEGORIES;
        let cat = 1 + (sub - 1) % CATEGORIES;
        p[0].push(int(k));
        p[1].push(text(if variant == Variant::Skew {
            format!("P{k:06}")
        } else {
            format!("P{k:04}")
        }));
        p[2].push(text(format!("Cat{cat:02}")));
        p[3].push(text(format!("Sous{sub:02}")));
        p[4].push(CellValue::Decimal(price_cents(k) as i64 * 100));
    }
    let [a, b, c, d, e] = p;
    inject(
        s,
        "Produit",
        products as usize,
        vec![
            ("ProduitKey", a),
            ("Produit", b),
            ("Categorie", c),
            ("SousCategorie", d),
            ("PrixCatalogue", e),
        ],
    );
    let mut m: [Vec<CellValue>; 3] = Default::default();
    for k in 1..=STORES {
        m[0].push(int(k));
        m[1].push(text(format!("M{k:03}")));
        m[2].push(text(format!("Region{}", 1 + splitmix(k ^ 0x5EED) % REGIONS)));
    }
    let [a, b, c] = m;
    inject(
        s,
        "Magasin",
        STORES as usize,
        vec![("MagasinKey", a), ("Magasin", b), ("Region", c)],
    );
    let n_days = days();
    let mut cal: [Vec<CellValue>; 4] = Default::default();
    for d in 0..n_days {
        let (y, mo, _) = super::ymd(DAY0 as i64 + d as i64);
        cal[0].push(CellValue::DateTime(DAY0 + d as f64));
        cal[1].push(CellValue::Int(y));
        cal[2].push(CellValue::Int(mo));
        cal[3].push(CellValue::Int(y * 100 + mo));
    }
    let [a, b, c, d] = cal;
    inject(
        s,
        "Calendrier",
        n_days as usize,
        vec![("Date", a), ("Annee", b), ("MoisNum", c), ("AnneeMois", d)],
    );
    let mut cl: [Vec<CellValue>; 7] = Default::default();
    for k in 1..=customers {
        let r = splitmix(k ^ 0x00C1_1E17);
        let city = 1 + (r >> 8) % CITIES;
        cl[0].push(int(k));
        cl[1].push(text(format!("C{k:07}")));
        cl[2].push(text(format!("Segment{}", 1 + r % SEGMENTS)));
        cl[3].push(text(format!("Ville{city:04}")));
        cl[4].push(text(format!("Pays{:02}", 1 + (city - 1) % COUNTRIES)));
        cl[5].push(text(format!("A{}", 1 + (r >> 24) % AGE_BANDS)));
        cl[6].push(text(if (r >> 40) % 10 < 3 { "Oui" } else { "Non" }.to_string()));
    }
    let [a, b, c, d, e, f, g] = cl;
    inject(
        s,
        "Client",
        customers as usize,
        vec![
            ("ClientKey", a),
            ("Client", b),
            ("Segment", c),
            ("Ville", d),
            ("Pays", e),
            ("TrancheAge", f),
            ("Fidelite", g),
        ],
    );
    let mut v: [Vec<CellValue>; 7] = Default::default();
    match variant {
        Variant::BigDim => {
            for k in 1..=rows {
                let r = splitmix(k);
                let r2 = splitmix(r);
                let product = 1 + (r >> 20) % PRODUCTS;
                let qty = 1 + (r2 >> 40) % 10;
                let amount = qty * price_cents(product) * (100 - (r2 >> 50) % 31) / 100;
                v[0].push(CellValue::DateTime(DAY0 + (r % n_days) as f64));
                v[1].push(int(product));
                v[2].push(int(1 + (r >> 40) % STORES));
                v[3].push(int(1 + r2 % customers));
                v[4].push(int(qty));
                v[5].push(CellValue::Decimal(amount as i64 * 100));
            }
        }
        Variant::Skew => {
            let (zp, zs, zc) = (
                Zipf::new(products, 0),
                Zipf::new(STORES, 0),
                Zipf::new(customers, 100),
            );
            let (mut order, mut left, mut store, mut customer) = (0u64, 0u64, 1u64, None);
            for k in 1..=rows {
                let r = splitmix(k ^ 0x5EE0);
                let r2 = splitmix(r);
                if left == 0 {
                    let o = splitmix(r2 ^ 0x0DE5);
                    order += 1;
                    left = 1 + o % 5;
                    store = zs.draw(splitmix(o));
                    customer = (!(o >> 32).is_multiple_of(100)).then(|| zc.draw(splitmix(o ^ 0xC0)));
                }
                left -= 1;
                let day = ((k - 1) * n_days / rows.max(1)).min(n_days - 1);
                let product = zp.draw(r);
                let qty = if (r2 >> 8) % 10 < 7 { 1 } else { 2 + (r2 >> 16) % 9 };
                let discount = if (r2 >> 24) % 10 < 6 {
                    0
                } else {
                    5 + (r2 >> 32) % 26
                };
                let amount = qty * price_cents(product) * (100 - discount) / 100;
                v[0].push(CellValue::DateTime(DAY0 + day as f64));
                v[1].push(int(product));
                v[2].push(int(store));
                v[3].push(customer.map_or(CellValue::Null, int));
                v[6].push(text(format!("CMD{order:08}")));
                v[4].push(int(qty));
                v[5].push(CellValue::Decimal(amount as i64 * 100));
            }
        }
    }
    let [a, b, c, d, e, f, g] = v;
    let mut cols = vec![
        ("Date", a),
        ("ProduitKey", b),
        ("MagasinKey", c),
        ("ClientKey", d),
        ("Quantite", e),
        ("Montant", f),
    ];
    if variant == Variant::Skew {
        cols.push(("NumCommande", g));
    }
    inject(s, "Ventes", rows as usize, cols);
}

/// A variant's queries (`xtask/engine-bench/queries-<variant>.json`).
pub(super) fn queries(variant: Variant) -> Vec<(String, String)> {
    let text = match variant {
        Variant::BigDim => include_str!("../../../xtask/engine-bench/queries-bigdim.json"),
        Variant::Skew => include_str!("../../../xtask/engine-bench/queries-skew.json"),
    };
    let q: Value = serde_json::from_str(text).unwrap();
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

/// Every query of `v`, cold and warm with the storage engine, equals the reference evaluator.
fn check(v: Variant, rows: u64, customers: u64, products: u64) {
    let mut s = variant(v, rows, customers, products);
    let mut answered = 0;
    let all = queries(v).len();
    for (id, q) in queries(v) {
        let want = evaluate(&mut s, &q, false).unwrap_or_else(|e| panic!("{id} reference: {e}"));
        s.execute("dax.clear_cache", &json!({})).unwrap();
        let cold = evaluate(&mut s, &q, true).unwrap_or_else(|e| panic!("{id}: {e}"));
        let warm = evaluate(&mut s, &q, true).unwrap_or_else(|e| panic!("{id} warm: {e}"));
        assert!(same(&cold, &want), "{id} cold: {cold}\nreference: {want}");
        assert!(same(&warm, &want), "{id} warm: {warm}\nreference: {want}");
        if want["results"][0]["rows"]
            .as_array()
            .is_some_and(|r| !r.is_empty())
        {
            answered += 1;
        }
    }
    // on small data a filter may select nothing (a city without customers), not most
    assert!(answered + 2 >= all, "{answered} of {all} queries return rows");
}

#[test]
fn variant_bigdim_matches_the_reference_evaluator() {
    check(Variant::BigDim, 12_000, 3_000, 0);
}

/// `full` in a release build, `debug` in a debug one. The reference evaluator is quadratic where
/// these tests exercise the storage engine: a context transition visits every row of the tables
/// it filters, so `RANKX(ALL(Produit[Produit]), [CA])` costs products² × sales row visits (S12:
/// 270 s in a debug build at 600 products and 12,000 sales) and an iteration with context
/// transition over the keys of a dimension, keys × (keys + sales). Debug builds (`cargo xtask
/// ci`) use smaller data; `cargo test --release -p bicraft-engine variant_` runs the full sizes.
fn sized(full: u64, debug: u64) -> u64 {
    if cfg!(debug_assertions) { debug } else { full }
}

/// The skewed variant's sizes (sales, customers, products). Debug: S12's TOPN 501 keeps every
/// one of the 200 products (S07 and S10 still cut theirs).
fn skew_sizes() -> (u64, u64, u64) {
    (sized(12_000, 6_000), 1_500, sized(600, 200))
}

#[test]
fn variant_skew_matches_the_reference_evaluator() {
    let (rows, customers, products) = skew_sizes();
    check(Variant::Skew, rows, customers, products);
}

/// Time per variant query: first run, cold (median of 3), warm (median of 3), and the
/// reference's answer compared on small data.
#[test]
#[ignore = "micro-bench: cargo test --release -p bicraft-engine variant_micro -- --ignored --nocapture"]
fn variant_micro() {
    let v = match bicraft_env::var("BICRAFT_MICRO_VARIANT").as_deref() {
        Ok("skew") => Variant::Skew,
        _ => Variant::BigDim,
    };
    let rows = env_rows("BICRAFT_MICRO_ROWS", 4_000_000);
    let default_dim = if v == Variant::BigDim { rows / 5 } else { rows / 10 };
    let customers = env_rows("BICRAFT_MICRO_DIM_ROWS", default_dim.max(1_000));
    let products = env_rows("BICRAFT_MICRO_PRODUCTS", 100_000);
    let ref_rows = env_rows("BICRAFT_MICRO_REF_ROWS", 200_000);
    let only = bicraft_env::var("BICRAFT_MICRO_ONLY").unwrap_or_default();
    let t = Instant::now();
    let mut s = variant(v, rows, customers, products);
    println!(
        "{v:?}: {rows} sales, {customers} customers, built in {:.1} s",
        t.elapsed().as_secs_f64()
    );
    println!("query  first ms    cold ms    warm ms   reference");
    let custom: Vec<(String, String)> = bicraft_env::var("BICRAFT_MICRO_QUERY")
        .map(|q| {
            q.split(";;")
                .enumerate()
                .map(|(i, q)| (format!("C{:02}", i + 1), q.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let all = if custom.is_empty() { queries(v) } else { custom };
    for (id, q) in all {
        if !only.is_empty() && !only.split(',').any(|o| o == id) {
            continue;
        }
        let start = Instant::now();
        let first = evaluate(&mut s, &q, true);
        let first_ms = start.elapsed().as_secs_f64() * 1e3;
        let cold = median_ms_with(&mut s, &q, true, 3, true);
        let warm = median_ms_with(&mut s, &q, true, 3, false);
        let reference = if rows <= ref_rows {
            match (&first, evaluate(&mut s, &q, false)) {
                (Ok(a), Ok(b)) if same(a, &b) => "same".to_string(),
                (Ok(_), Ok(_)) => "DIFFERENT".to_string(),
                (_, Err(e)) => format!("reference refused: {e}"),
                (Err(_), Ok(_)) => "refused, the reference answers".to_string(),
            }
        } else {
            "-".into()
        };
        match (first, cold, warm) {
            (Ok(_), Ok(c), Ok(w)) => println!("{id}   {first_ms:>9.2} {c:>10.2} {w:>10.2}   {reference}"),
            (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => println!("{id}   refused: {e}"),
        }
    }
}

/// `RELATED` to a dimension of more codes than the capped first-row table holds: the row of
/// each key comes from the column's table of first rows (`ColData::rows_of_codes`), which new
/// dimension data replaces (the old table is never read for the new rows).
#[test]
fn variant_related_over_a_big_dimension_follows_new_data() {
    let queries = [
        "EVALUATE ROW(\"f\", SUMX(Ventes, IF(RELATED(Client[Fidelite]) = \"Oui\", Ventes[Montant])))",
        "EVALUATE ROW(\"n\", COUNTROWS(FILTER(Ventes, RELATED(Client[Segment]) = \"Segment2\")), \
         \"m\", MAXX(Ventes, RELATED(Client[Client])))",
        // tables of values that remember their codes (`ALL` kept between queries, a `FILTER` of
        // them, `VALUES`): new data renumbers the codes
        "EVALUATE ROW(\"a\", CALCULATE([CA], FILTER(ALL(Client[ClientKey]), Client[ClientKey] <= 900 && [CA] > 0)), \n         \"v\", COUNTROWS(FILTER(VALUES(Client[ClientKey]), [Qte] > 5)), \n         \"s\", SUMX(FILTER(VALUES(Client[Segment]), [CA] > 0), [Qte]))",
    ];
    let [related @ .., values] = queries;
    if cfg!(debug_assertions) {
        // the tables of values iterate with context transition over every key: keys × (keys +
        // sales) row visits in the reference (230 s per run at 70,000 keys in a debug build), so
        // a debug build checks them over 3,000 keys (`sized`)
        follows_new_data(70_000, &related, 2);
        follows_new_data(3_000, &[values], 0);
    } else {
        follows_new_data(70_000, &queries, 2);
    }
}

/// Each of `queries` equals the reference (cold and warm) before and after new data for the
/// `customers` customers of the big-dimension variant; the first `changed` answers change.
fn follows_new_data(customers: u64, queries: &[&str], changed: usize) {
    let mut s = variant(Variant::BigDim, 8_000, customers, 0);
    let run = |s: &mut Session| -> Vec<Value> {
        queries
            .iter()
            .map(|q| {
                let want = evaluate(s, q, false).unwrap();
                s.execute("dax.clear_cache", &json!({})).unwrap();
                let got = evaluate(s, q, true).unwrap();
                assert!(same(&got, &want), "{q}: {got} / {want}");
                let warm = evaluate(s, q, true).unwrap();
                assert!(same(&warm, &want), "{q} warm: {warm} / {want}");
                want
            })
            .collect()
    };
    let before = run(&mut s);
    // new dimension data: every customer's loyalty and segment change, keys in another order
    let mut cl: [Vec<CellValue>; 7] = Default::default();
    for k in (1..=customers).rev() {
        cl[0].push(int(k));
        cl[1].push(text(format!("X{k:07}")));
        cl[2].push(text(format!("Segment{}", 1 + k % SEGMENTS)));
        cl[3].push(text("Ville0001".into()));
        cl[4].push(text("Pays01".into()));
        cl[5].push(text("A1".into()));
        cl[6].push(text(if k % 2 == 0 { "Oui" } else { "Non" }.to_string()));
    }
    let [a, b, c, d, e, f, g] = cl;
    inject(
        &s,
        "Client",
        customers as usize,
        vec![
            ("ClientKey", a),
            ("Client", b),
            ("Segment", c),
            ("Ville", d),
            ("Pays", e),
            ("TrancheAge", f),
            ("Fidelite", g),
        ],
    );
    let after = run(&mut s);
    // the RELATED answers change with the dimension (the third reads keys and sales only)
    assert!(
        before.iter().zip(&after).take(changed).all(|(a, b)| !same(a, b)),
        "{before:?} / {after:?}"
    );
}

/// Compiled predicates over a column's values (`preview_eval_pred.rs`): every operator, blanks,
/// measures, outer filters, and the errors the reference reports (or not), against the reference.
#[test]
fn variant_compiled_filters_match_the_reference() {
    let mut s = variant(Variant::BigDim, 10_000, 3_000, 0);
    let queries = [
        "EVALUATE ROW(\"a\", CALCULATE([CA], Client[ClientKey] > 100 && Client[ClientKey] <= 2000), \
         \"b\", CALCULATE([CA], NOT(Client[ClientKey] IN {1, 2, 3})), \
         \"c\", CALCULATE([CA], Client[ClientKey] = BLANK()), \"d\", CALCULATE([CA], Client[ClientKey] <> BLANK()), \
         \"e\", CALCULATE([CA], Client[ClientKey] == 7 || Client[ClientKey] >= 2999), \
         \"f\", CALCULATE([CA], OR(Client[ClientKey] < -1, NOT Client[ClientKey] > 10)))",
        "EVALUATE ROW(\"n\", COUNTROWS(FILTER(VALUES(Client[Client]), Client[Client] >= \"C0001000\")), \
         \"m\", COUNTROWS(FILTER(VALUES(Client[ClientKey]), [CA] > 5000 || [Qte] < 3)), \
         \"z\", COUNTROWS(FILTER(ALL(Client[ClientKey]), ISBLANK([CA]))), \
         \"w\", COUNTROWS(FILTER(ALL(Client[ClientKey]), [CA] = BLANK())), \
         \"v\", COUNTROWS(FILTER(VALUES(Client[ClientKey]), AND([Nb clients] = 1, Client[ClientKey] < 500))))",
        "EVALUATE SUMMARIZECOLUMNS(Client[Segment], \"x\", CALCULATE([CA], FILTER(KEEPFILTERS(VALUES(Client[ClientKey])), \
         Client[ClientKey] <= 1500)), \"y\", CALCULATE([CA], Client[ClientKey] IN {5, 50, 500, 5000})) ORDER BY Client[Segment]",
        "DEFINE VAR f = FILTER(KEEPFILTERS(VALUES(Client[ClientKey])), Client[ClientKey] <= 900) \n         VAR g = FILTER(ALL(Client[ClientKey]), Client[ClientKey] > 2500) \n         EVALUATE SUMMARIZECOLUMNS(Client[Segment], f, \"CA\", [CA], \"n\", COUNTROWS(f), \"g\", CALCULATE([CA], g)) \n         ORDER BY Client[Segment]",
        // context transition over a key: every row visible (the segment filter is implied) or not
        "EVALUATE SUMMARIZECOLUMNS(Client[Segment], \"a\", AVERAGEX(ALL(Client[ClientKey]), [CA]), \n         \"v\", AVERAGEX(VALUES(Client[ClientKey]), [CA]), \"m\", MAXX(VALUES(Client[ClientKey]), [Qte])) \n         ORDER BY Client[Segment]",
        "DEFINE VAR k = 1200 EVALUATE ROW(\"v\", CALCULATE([CA], Client[ClientKey] < k), \
         \"t\", CALCULATE([CA], Client[ClientKey] < DATE(1900, 1, 1)))",
        // conditional sums (`SUMX(T, IF(cond, T[a]))`)
        "EVALUATE SUMMARIZECOLUMNS(Produit[Categorie], \"s\", SUMX(Ventes, IF(RELATED(Client[Segment]) = \"Segment2\", Ventes[Montant])), \n         \"q\", SUMX(Ventes, IF(Ventes[Quantite] > 3, Ventes[Quantite], BLANK())), \n         \"f\", SUMX(FILTER(Ventes, Ventes[Quantite] < 9), IF(RELATED(Client[Fidelite]) = \"Oui\" && Ventes[Quantite] > 2, Ventes[Montant])), \n         \"n\", SUMX(Ventes, IF(RELATED(Client[Pays]) = \"Nowhere\", Ventes[Montant]))) ORDER BY Produit[Categorie]",
        "EVALUATE ROW(\"e\", SUMX(Ventes, IF(RELATED(Client[Client]) > 3, Ventes[Montant])))",
        // errors: text against a number, a constant that fails
        "EVALUATE ROW(\"e\", COUNTROWS(FILTER(VALUES(Client[Client]), Client[Client] > 5)))",
        "EVALUATE ROW(\"e\", CALCULATE([CA], Client[ClientKey] > 1 / 0 + \"x\"))",
        // never evaluated on a row: no error
        "EVALUATE ROW(\"n\", COUNTROWS(FILTER(VALUES(Client[ClientKey]), Client[ClientKey] < 0 && Client[ClientKey] > \"x\")))",
    ];
    for q in queries {
        let want = evaluate(&mut s, q, false);
        s.execute("dax.clear_cache", &json!({})).unwrap();
        let got = evaluate(&mut s, q, true);
        match (&got, &want) {
            (Ok(a), Ok(b)) => assert!(same(a, b), "{q}: {a} / {b}"),
            (Err(a), Err(b)) => assert_eq!(a, b, "{q}"),
            _ => panic!("{q}: {got:?} / {want:?}"),
        }
    }
}

/// Distinct counts kept as sorted lists per group (forced here; by default only when a bitmap per
/// group would be too large): single groups, folded code sets, coarsened groupings, blank codes.
#[test]
fn variant_sparse_distinct_counts_match_the_reference() {
    let (rows, customers, products) = skew_sizes();
    let mut s = variant(Variant::Skew, rows, customers, products);
    let queries: Vec<String> = queries(Variant::Skew)
        .into_iter()
        .map(|(_, q)| q)
        .chain([
            "EVALUATE SUMMARIZECOLUMNS(Client[Segment], Calendrier[Annee], \"d\", [Nb commandes], \
             \"c\", [Nb clients], \"nb\", DISTINCTCOUNTNOBLANK(Ventes[ClientKey])) ORDER BY Client[Segment], Calendrier[Annee]"
                .to_string(),
            "EVALUATE SUMMARIZECOLUMNS(Client[Segment], \"d\", [Nb commandes]) ORDER BY Client[Segment]".to_string(),
            "EVALUATE ROW(\"a\", CALCULATE([Nb clients], TREATAS({2022, 2023}, Calendrier[Annee])), \
             \"b\", CALCULATE([Nb commandes], Produit[Categorie] IN {\"Cat01\", \"Cat04\"}))"
                .to_string(),
        ])
        .collect();
    for q in &queries {
        let want = evaluate(&mut s, q, false).unwrap();
        super::storage::set_dense_distinct_limit(0);
        s.execute("dax.clear_cache", &json!({})).unwrap();
        let got = evaluate(&mut s, q, true);
        let warm = evaluate(&mut s, q, true);
        super::storage::set_dense_distinct_limit(64 << 20);
        let (got, warm) = (got.unwrap(), warm.unwrap());
        assert!(same(&got, &want), "{q}: {got} / {want}");
        assert!(same(&warm, &want), "{q} warm: {warm} / {want}");
    }
    s.execute("dax.clear_cache", &json!({})).unwrap();
}

/// What a query costs before it reads any data, on a large model: `BICRAFT_MICRO_TABLES` extra
/// tables of 20 columns (default 200) and `BICRAFT_MICRO_MEASURES` measures in chains of 20
/// (default 2,000); the validation alone, then the whole `dax.evaluate` of a card.
#[test]
#[ignore = "micro-bench: cargo test --release -p bicraft-engine query_overhead_micro -- --ignored --nocapture"]
fn query_overhead_micro() {
    let tables = env_rows("BICRAFT_MICRO_TABLES", 200);
    let measures = env_rows("BICRAFT_MICRO_MEASURES", 2_000);
    let mut s = model(Variant::BigDim);
    for t in 0..tables {
        let name = format!("T{t:03}");
        s.execute(
            "table.add",
            &json!({"name": name, "partition": {"source_type": "m", "expression": "#table({}, {})"}}),
        )
        .unwrap();
        for c in 0..20 {
            s.execute(
                "column.add",
                &json!({"table": name, "name": format!("C{c:02}"), "data_type": "int64"}),
            )
            .unwrap();
        }
    }
    for m in 0..measures {
        let expr = if m % 20 == 0 {
            "SUM(Ventes[Montant]) * 2".to_string()
        } else {
            format!(
                "[M{:04}] + CALCULATE([CA], Client[Segment] = \"Segment1\")",
                m - 1
            )
        };
        s.execute(
            "measure.add",
            &json!({"table": "Ventes", "name": format!("M{m:04}"), "expression": expr}),
        )
        .unwrap();
    }
    // the data last: a model edit drops what was put in the session
    fill(&s, Variant::BigDim, 20_000, 2_000, 0);
    let q = "EVALUATE ROW(\"x\", [M0019])";
    let _ = evaluate(&mut s, q, true).unwrap();
    let model = s.model.clone().unwrap();
    let time = |f: &mut dyn FnMut()| {
        let mut t: Vec<f64> = (0..50)
            .map(|_| {
                let at = Instant::now();
                f();
                at.elapsed().as_secs_f64() * 1e3
            })
            .collect();
        t.sort_by(f64::total_cmp);
        t[t.len() / 2]
    };
    let ctx = bicraft_dax::Context {
        model: Some(&model),
        table: None,
    };
    let validate = time(&mut || {
        let _ = bicraft_dax::validate_query_parsed(q, &ctx);
    });
    let clone = time(&mut || {
        let _ = model.clone();
    });
    let total = time(&mut || {
        let _ = evaluate(&mut s, q, true).unwrap();
    });
    let constant = time(&mut || {
        let _ = evaluate(&mut s, "EVALUATE ROW(\"x\", 1)", true).unwrap();
    });
    println!(
        "{tables} extra tables, {measures} measures: validate {validate:.3} ms (model clone {clone:.3} ms), dax.evaluate {total:.3} ms, of a constant {constant:.3} ms"
    );
}

/// The model's shape is kept between evaluations (`Registry`, by model hash): a relationship made
/// inactive or bidirectional and a new column are seen by the next query, which answers as a
/// fresh session holding the same model and data.
#[test]
fn registry_follows_model_edits() {
    let customers = 1_500u64;
    // the customers again, with one more column
    let with_extra = |s: &Session| {
        let mut cl: [Vec<CellValue>; 8] = Default::default();
        for k in 1..=customers {
            let r = splitmix(k ^ 0x00C1_1E17);
            let city = 1 + (r >> 8) % CITIES;
            cl[0].push(int(k));
            cl[1].push(text(format!("C{k:07}")));
            cl[2].push(text(format!("Segment{}", 1 + r % SEGMENTS)));
            cl[3].push(text(format!("Ville{city:04}")));
            cl[4].push(text(format!("Pays{:02}", 1 + (city - 1) % COUNTRIES)));
            cl[5].push(text(format!("A{}", 1 + (r >> 24) % AGE_BANDS)));
            cl[6].push(text(if (r >> 40) % 10 < 3 { "Oui" } else { "Non" }.to_string()));
            cl[7].push(int(k % 3));
        }
        let [a, b, c, d, e, f, g, h] = cl;
        inject(
            s,
            "Client",
            customers as usize,
            vec![
                ("ClientKey", a),
                ("Client", b),
                ("Segment", c),
                ("Ville", d),
                ("Pays", e),
                ("TrancheAge", f),
                ("Fidelite", g),
                ("Extra", h),
            ],
        );
    };
    let q = "EVALUATE SUMMARIZECOLUMNS(Client[Segment], \"CA\", [CA], \"n\", COUNTROWS(Client)) ORDER BY Client[Segment]";
    let q_extra = "EVALUATE SUMMARIZECOLUMNS(Client[Extra], \"CA\", [CA]) ORDER BY Client[Extra]";
    let mut s = variant(Variant::BigDim, 6_000, customers, 0);
    let first = evaluate(&mut s, q, true).unwrap();
    assert!(evaluate(&mut s, q_extra, true).is_err(), "no column Extra yet");
    let edits: Vec<(&str, Value)> = vec![
        (
            "relationship.set",
            json!({"from": "Ventes[ClientKey]", "to": "Client[ClientKey]", "active": false}),
        ),
        (
            "relationship.set",
            json!({"from": "Ventes[ClientKey]", "to": "Client[ClientKey]", "active": true, "cross_filter": "both"}),
        ),
        (
            "column.add",
            json!({"table": "Client", "name": "Extra", "data_type": "int64"}),
        ),
    ];
    let mut applied: Vec<(&str, Value)> = Vec::new();
    let mut answers = vec![first];
    for (id, p) in edits {
        s.execute(id, &p).unwrap();
        applied.push((id, p));
        let mut fresh = model(Variant::BigDim);
        for (id, p) in &applied {
            fresh.execute(id, p).unwrap();
        }
        // an edit drops the data put in the session: the same data again
        for x in [&fresh, &s] {
            fill(x, Variant::BigDim, 6_000, customers, 0);
            if id == "column.add" {
                with_extra(x);
            }
        }
        for q in [q, q_extra] {
            let got = evaluate(&mut s, q, true);
            let want = evaluate(&mut fresh, q, true);
            match (&got, &want) {
                (Ok(a), Ok(b)) => assert!(same(a, b), "{id}: {a} / {b}"),
                (Err(a), Err(b)) => assert_eq!(a, b, "{id}"),
                _ => panic!("{id} {q}: {got:?} / {want:?}"),
            }
        }
        answers.push(evaluate(&mut s, q, true).unwrap());
    }
    // the new column is read; the inactive relationship changed the answer
    assert!(evaluate(&mut s, q_extra, true).is_ok());
    assert!(!same(&answers[0], &answers[1]), "{answers:?}");
}

/// How each fact column of a bench PBIX (`BICRAFT_VARIANT_PBIX`) is read: kept compressed or
/// decoded in full, and the time of its first read.
#[test]
#[ignore = "local data: BICRAFT_VARIANT_PBIX=…/bench.pbix cargo test --release -p bicraft-engine variant_columns -- --ignored --nocapture"]
fn variant_columns_read() {
    let Ok(pbix) = bicraft_env::var("BICRAFT_VARIANT_PBIX") else {
        eprintln!("BICRAFT_VARIANT_PBIX is not set: skipped");
        return;
    };
    let mut s = Session::new();
    s.execute("file.open", &json!({"path": pbix})).unwrap();
    for c in [
        "Date",
        "ProduitKey",
        "MagasinKey",
        "ClientKey",
        "Quantite",
        "Montant",
        "NumCommande",
    ] {
        let at = Instant::now();
        let r = evaluate(&mut s, &format!("EVALUATE ROW(\"x\", COUNT(Ventes[{c}]))"), true);
        let ms = at.elapsed().as_secs_f64() * 1e3;
        let at = Instant::now();
        let _ = evaluate(&mut s, &format!("EVALUATE ROW(\"x\", MAX(Ventes[{c}]))"), true);
        let max_ms = at.elapsed().as_secs_f64() * 1e3;
        let st = s.preview.lock();
        let how = match st.columns.get(&("ventes".to_string(), c.to_lowercase())) {
            Some(Ok(d)) if d.codes.col_packed().is_some() => format!("packed, {} values", d.values.len()),
            Some(Ok(d)) => format!("decoded in full, {} values", d.values.len()),
            Some(Err(e)) => format!("error {e:?}"),
            None => "not read".into(),
        };
        println!(
            "{c}: read and count {ms:.1} ms, then MAX {max_ms:.1} ms, {how}, {}",
            r.is_ok()
        );
    }
}
