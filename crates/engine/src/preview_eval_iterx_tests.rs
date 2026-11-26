//! Row iterators as the items of a grouping (`preview_eval_iterx.rs`): random queries over a
//! random snowflake (unknown and blank keys, blank values, Currency and whole numbers, a chain of
//! many-to-one relationships, an inactive relationship), each run with the reference evaluator
//! and with the storage engine, cold and warm, which must agree; the paths taken are counted so
//! that a silent fall back to the cells one by one shows.

use bicraft_vertipaq::CellValue;
use serde_json::{Value, json};

use super::super::store::{Codes, ColData};
use super::storage::iterx::iterx_hits;
use super::storage_tests::{evaluate, same};
use crate::Session;

fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = splitmix(self.0);
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[self.below(xs.len() as u64) as usize]
    }
}

fn col(values: Vec<CellValue>) -> ColData {
    ColData::from_stored(values).unwrap()
}

fn packed(values: Vec<CellValue>, seg: usize) -> ColData {
    let c = col(values);
    ColData {
        codes: Codes::pack(&c, seg),
        ..c
    }
}

/// `F` (`rows` rows) → `D` (products, with unknown keys) → `G` (groups, a chain), `F` → `S`
/// (stores), and an inactive `F[Alt]` → `S[SKey]`. `big`: Currency values near the top of the
/// range (overflows).
fn snowflake(seed: u64, rows: u64, big: bool) -> Session {
    let mut r = Rng(seed);
    let mut s = Session::new();
    let mut steps: Vec<(&str, Value)> = vec![("model.new", json!({"name": "Snow", "culture": "en-US"}))];
    let tables: [(&str, &[(&str, &str)]); 4] = [
        ("G", &[("GKey", "int64"), ("Taux", "int64"), ("GName", "string")]),
        (
            "D",
            &[
                ("DKey", "int64"),
                ("DName", "string"),
                ("GKey", "int64"),
                ("Prix", "decimal"),
                ("Poids", "int64"),
            ],
        ),
        ("S", &[("SKey", "int64"), ("Region", "string")]),
        (
            "F",
            &[
                ("DKey", "int64"),
                ("SKey", "int64"),
                ("Alt", "int64"),
                ("Qty", "int64"),
                ("Amt", "decimal"),
                ("Real", "double"),
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
    for (from, to, active) in [
        ("F[DKey]", "D[DKey]", true),
        ("D[GKey]", "G[GKey]", true),
        ("F[SKey]", "S[SKey]", true),
        ("F[Alt]", "S[SKey]", false),
    ] {
        steps.push((
            "relationship.add",
            json!({"from": from, "to": to, "active": active}),
        ));
    }
    for (id, p) in steps {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"));
    }
    let groups = 2 + r.below(5);
    let products = 5 + r.below(60);
    let stores = 2 + r.below(8);
    let mut g: [Vec<CellValue>; 3] = Default::default();
    for k in 1..=groups {
        g[0].push(CellValue::Int(k as i64));
        g[1].push(if r.below(5) == 0 {
            CellValue::Null
        } else {
            CellValue::Int(r.below(7) as i64 - 2)
        });
        g[2].push(CellValue::Text(format!("G{}", r.below(3))));
    }
    let mut d: [Vec<CellValue>; 5] = Default::default();
    for k in 1..=products {
        d[0].push(CellValue::Int(k as i64));
        d[1].push(CellValue::Text(format!("P{}", r.below(products))));
        // some products in no group, or an unknown one
        d[2].push(match r.below(10) {
            0 => CellValue::Null,
            1 => CellValue::Int(99),
            _ => CellValue::Int(1 + r.below(groups) as i64),
        });
        d[3].push(match r.below(8) {
            0 => CellValue::Null,
            _ if big => CellValue::Decimal(9_000_000_000_000_000_000 - r.below(1000) as i64),
            _ => CellValue::Decimal(r.below(2_000_000) as i64 - 500_000),
        });
        d[4].push(if r.below(6) == 0 {
            CellValue::Null
        } else {
            CellValue::Int(r.below(20) as i64 - 5)
        });
    }
    let mut sv: [Vec<CellValue>; 2] = Default::default();
    for k in 1..=stores {
        sv[0].push(CellValue::Int(k as i64));
        sv[1].push(CellValue::Text(format!("R{}", r.below(3))));
    }
    let seg = [64usize, 1000, 65_536][r.below(3) as usize];
    let mut f: [Vec<CellValue>; 6] = Default::default();
    for _ in 0..rows {
        f[0].push(match r.below(20) {
            0 => CellValue::Null,
            1 => CellValue::Int(1000 + r.below(3) as i64),
            _ => CellValue::Int(1 + r.below(products) as i64),
        });
        f[1].push(CellValue::Int(1 + r.below(stores) as i64));
        f[2].push(CellValue::Int(1 + r.below(stores + 1) as i64));
        f[3].push(if r.below(12) == 0 {
            CellValue::Null
        } else {
            CellValue::Int(r.below(12) as i64 - 2)
        });
        f[4].push(match r.below(10) {
            0 => CellValue::Null,
            _ if big => CellValue::Decimal(9_000_000_000_000_000_000 - r.below(1000) as i64),
            _ => CellValue::Decimal(r.below(500) as i64 * 100 - 10_000),
        });
        f[5].push(if r.below(9) == 0 {
            CellValue::Null
        } else {
            CellValue::Real((r.below(1000) as f64) / 8.0 - 20.0)
        });
    }
    let [a, b, c] = g;
    super::super::inject_table(
        &s,
        "G",
        groups as usize,
        vec![
            ("GKey".into(), col(a)),
            ("Taux".into(), col(b)),
            ("GName".into(), col(c)),
        ],
    );
    let [a, b, c, dd, e] = d;
    super::super::inject_table(
        &s,
        "D",
        products as usize,
        vec![
            ("DKey".into(), col(a)),
            ("DName".into(), col(b)),
            ("GKey".into(), col(c)),
            ("Prix".into(), col(dd)),
            ("Poids".into(), col(e)),
        ],
    );
    let [a, b] = sv;
    super::super::inject_table(
        &s,
        "S",
        stores as usize,
        vec![("SKey".into(), col(a)), ("Region".into(), col(b))],
    );
    let [a, b, c, dd, e, ff] = f;
    super::super::inject_table(
        &s,
        "F",
        rows as usize,
        vec![
            ("DKey".into(), packed(a, seg)),
            ("SKey".into(), packed(b, seg)),
            ("Alt".into(), packed(c, seg)),
            ("Qty".into(), packed(dd, seg)),
            ("Amt".into(), packed(e, seg)),
            ("Real".into(), packed(ff, seg)),
        ],
    );
    s
}

const GROUPS: &[&str] = &[
    "D[DName]",
    "D[DKey]",
    "G[GName]",
    "G[Taux]",
    "S[Region]",
    "F[Qty]",
    "F[SKey]",
];

const BODIES: &[&str] = &[
    "F[Qty] * RELATED(D[Prix])",
    "RELATED(D[Prix]) * F[Qty]",
    "F[Qty] * RELATED(G[Taux])",
    "F[Amt] * RELATED(D[Poids])",
    "F[Qty] * RELATED(D[Poids]) * RELATED(G[Taux])",
    "RELATED(D[Prix]) * RELATED(G[Taux])",
    "RELATED(D[Prix])",
    "F[Amt]",
    "F[Qty]",
    "F[Qty] * F[Qty]",
    "F[Amt] * F[Qty]",
    "F[Real] * RELATED(D[Poids])",
    "F[Qty] * 1.5 + RELATED(D[Poids])",
    "IF(RELATED(G[GName]) = \"G1\", F[Amt])",
    "F[Amt] * RELATED(D[Prix])",
    "RELATED(S[Region]) & F[Qty]",
    "DIVIDE(F[Qty], RELATED(D[Poids]))",
];

const ITERS: &[&str] = &["SUMX", "AVERAGEX", "MINX", "MAXX", "COUNTX"];

const FILTERS: &[&str] = &[
    "TREATAS({\"R0\", \"R2\"}, S[Region])",
    "FILTER(ALL(G[GName]), G[GName] <> \"G2\")",
    "KEEPFILTERS(FILTER(VALUES(F[Qty]), F[Qty] > 2))",
    "TREATAS({1, 2, 3}, D[DKey])",
];

const CALC_FILTERS: &[&str] = &[
    "F[Qty] > 3",
    "D[Poids] < 4",
    "S[Region] = \"R1\"",
    "G[Taux] >= 0",
    "F[Amt] > 0",
    "-1 < F[Qty]",
];

fn define(defs: &[String]) -> String {
    if defs.is_empty() {
        String::new()
    } else {
        format!("DEFINE {} ", defs.join(" "))
    }
}

fn random_query(r: &mut Rng) -> String {
    let mut args: Vec<String> = Vec::new();
    let mut defs: Vec<String> = Vec::new();
    let ng = r.below(3);
    let mut used = Vec::new();
    for _ in 0..ng {
        let g = r.pick(GROUPS);
        if !used.contains(&g) {
            used.push(g);
            args.push(g.to_string());
        }
    }
    if r.below(3) == 0 {
        args.push(r.pick(FILTERS).to_string());
    }
    let items = 1 + r.below(2);
    for i in 0..items {
        let f = r.pick(ITERS);
        let body = r.pick(BODIES);
        let src = if r.below(4) == 0 {
            format!("FILTER(F, F[Qty] >= {})", r.below(6))
        } else {
            "F".to_string()
        };
        let mut item = format!("{f}({src}, {body})");
        if r.below(4) == 0 {
            item = format!("CALCULATE({item}, {})", r.pick(CALC_FILTERS));
        }
        if r.below(5) == 0 {
            defs.push(format!("MEASURE F[M{i}] = {item}"));
            item = format!("[M{i}]");
        }
        args.push(format!("\"v{i}\", {item}"));
    }
    let mut q = format!("SUMMARIZECOLUMNS({})", args.join(", "));
    match r.below(6) {
        0 => q = format!("CALCULATETABLE({q}, USERELATIONSHIP(F[Alt], S[SKey]))"),
        1 => q = format!("CALCULATETABLE({q}, D[Poids] > 0)"),
        _ => {}
    }
    let order: Vec<&str> = used.clone();
    if order.is_empty() {
        format!("{}EVALUATE {q}", define(&defs))
    } else {
        format!("{}EVALUATE {q} ORDER BY {}", define(&defs), order.join(", "))
    }
}

/// Equal values; doubles within a few ulps of the terms they add (the reference adds row by row,
/// the storage engine's double sums are compensated: a sum that cancels to `-1.8e-17` in the
/// reference is 0 there, with or without this path).
fn close(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            let (x, y) = (x.as_f64().unwrap_or(f64::NAN), y.as_f64().unwrap_or(f64::NAN));
            same(a, b) || (x - y).abs() <= 1e-9
        }
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(a, b)| close(a, b)),
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| close(v, w)))
        }
        _ => a == b,
    }
}

/// The reference's answer, then the storage engine's cold and warm: all the same (errors too),
/// and the storage engine's answer without this path (the cells one by one): the same exactly.
fn check(s: &mut Session, q: &str) {
    let want = evaluate(s, q, false);
    super::storage::iterx::set_iterx_off(true);
    s.execute("dax.clear_cache", &json!({})).unwrap();
    let before = evaluate(s, q, true);
    super::storage::iterx::set_iterx_off(false);
    s.execute("dax.clear_cache", &json!({})).unwrap();
    let cold = evaluate(s, q, true);
    let warm = evaluate(s, q, true);
    match (&before, &cold) {
        (Ok(a), Ok(b)) => assert!(same(a, b), "cells one by one / grouped: {q}\n{a}\n{b}"),
        (Err(_), Err(_)) => {}
        (a, b) => panic!("cells one by one / grouped: {q}\n{a:?}\n{b:?}"),
    }
    for (what, got) in [("cold", &cold), ("warm", &warm)] {
        match (&want, got) {
            (Ok(a), Ok(b)) => {
                if !close(a, b) {
                    let rows = |v: &Value| v["results"][0]["rows"].as_array().cloned().unwrap_or_default();
                    let (ra, rb) = (rows(a), rows(b));
                    let i = ra.iter().zip(&rb).position(|(x, y)| !close(x, y));
                    panic!(
                        "{what} {q}\nrows {} / {}, first difference: reference {:?}, storage {:?}",
                        ra.len(),
                        rb.len(),
                        i.and_then(|i| ra.get(i)),
                        i.and_then(|i| rb.get(i)),
                    );
                }
            }
            (Err(a), Err(b)) => assert_eq!(a, b, "{what} {q}"),
            (a, b) => panic!("{what} {q}\nreference {a:?}\nstorage {b:?}"),
        }
    }
}

#[test]
fn grouped_iterators_match_the_reference_evaluator() {
    let seed = bicraft_env::var("BICRAFT_FUZZ_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0x5u64);
    let cases: u64 = bicraft_env::var("BICRAFT_FUZZ_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(160);
    let before = iterx_hits();
    let mut r = Rng(seed);
    let mut s = snowflake(seed, 2_000, false);
    for i in 0..cases {
        if i % 40 == 39 {
            s = snowflake(r.next(), 500 + r.below(3_000), false);
        }
        let q = random_query(&mut r);
        check(&mut s, &q);
    }
    let after = iterx_hits();
    assert!(
        after.0 > before.0 && after.1 > before.1,
        "both grouped paths taken: {before:?} -> {after:?}"
    );
}

/// The shapes the module was written for, each checked to take its path.
#[test]
fn grouped_iterator_shapes_take_their_paths() {
    let mut s = snowflake(7, 3_000, false);
    let products = [
        "EVALUATE SUMMARIZECOLUMNS(D[DName], \"x\", SUMX(F, F[Qty] * RELATED(D[Prix]))) ORDER BY D[DName]",
        "EVALUATE SUMMARIZECOLUMNS(G[GName], S[Region], \"a\", AVERAGEX(F, F[Qty] * RELATED(D[Prix])), \
         \"c\", COUNTX(F, RELATED(D[Poids]) * F[Qty])) ORDER BY G[GName], S[Region]",
        "EVALUATE SUMMARIZECOLUMNS(D[DKey], \"x\", SUMX(F, F[Qty] * RELATED(D[Poids]) * RELATED(G[Taux]))) ORDER BY D[DKey]",
        "EVALUATE SUMMARIZECOLUMNS(S[Region], TREATAS({\"G0\"}, G[GName]), \"x\", \
         SUMX(FILTER(F, F[Qty] > 3), F[Amt] * RELATED(D[Poids]))) ORDER BY S[Region]",
        "DEFINE MEASURE F[M] = SUMX(F, F[Qty] * RELATED(D[Prix])) \
         EVALUATE SUMMARIZECOLUMNS(D[DName], \"m\", [M]) ORDER BY D[DName]",
        "EVALUATE SUMMARIZECOLUMNS(G[Taux], \"x\", SUMX(F, RELATED(D[Prix]))) ORDER BY G[Taux]",
        "EVALUATE SUMMARIZECOLUMNS(D[DName], \"x\", CALCULATE(SUMX(F, F[Qty] * RELATED(D[Prix])), F[Qty] > 3, S[Region] = \"R1\")) ORDER BY D[DName]",
    ];
    for q in products {
        let before = iterx_hits();
        check(&mut s, q);
        assert!(iterx_hits().0 > before.0, "exact products path: {q}");
    }
    let weighted = [
        "EVALUATE SUMMARIZECOLUMNS(D[DName], \"x\", MAXX(F, F[Qty] * RELATED(D[Prix]))) ORDER BY D[DName]",
        "EVALUATE SUMMARIZECOLUMNS(S[Region], \"x\", SUMX(F, F[Real] * RELATED(D[Poids])), \
         \"y\", MINX(F, F[Qty] * 1.5 + RELATED(D[Poids]))) ORDER BY S[Region]",
        "EVALUATE SUMMARIZECOLUMNS(G[GName], \"x\", SUMX(F, IF(RELATED(G[GName]) = \"G1\", F[Amt]))) ORDER BY G[GName]",
    ];
    for q in weighted {
        let before = iterx_hits();
        check(&mut s, q);
        assert!(iterx_hits().1 > before.1, "weighted path: {q}");
    }
    // a relationship override: the cells one by one
    let q = "EVALUATE CALCULATETABLE(SUMMARIZECOLUMNS(S[Region], \"x\", SUMX(F, F[Qty] * RELATED(D[Prix]))), \
             USERELATIONSHIP(F[Alt], S[SKey])) ORDER BY S[Region]";
    let before = iterx_hits();
    check(&mut s, q);
    assert_eq!(iterx_hits(), before, "USERELATIONSHIP falls back");
    // a CALCULATE filter on a grouping column's table: the cells one by one
    let q = "EVALUATE SUMMARIZECOLUMNS(D[DName], \"x\", CALCULATE(SUMX(F, F[Qty] * RELATED(D[Prix])), D[Poids] > 2)) ORDER BY D[DName]";
    let before = iterx_hits();
    check(&mut s, q);
    assert_eq!(
        iterx_hits(),
        before,
        "a filter on the grouping's table falls back"
    );
}

/// Currency products and sums near the top of the range: the reference's overflow errors (or
/// values) exactly, through the fall back.
#[test]
fn grouped_iterators_overflow_as_the_reference() {
    for seed in 0..4 {
        let mut s = snowflake(seed, 400, true);
        for q in [
            "EVALUATE SUMMARIZECOLUMNS(D[DName], \"x\", SUMX(F, F[Qty] * RELATED(D[Prix]))) ORDER BY D[DName]",
            "EVALUATE SUMMARIZECOLUMNS(S[Region], \"x\", SUMX(F, F[Amt])) ORDER BY S[Region]",
            "EVALUATE SUMMARIZECOLUMNS(S[Region], \"x\", AVERAGEX(F, F[Amt] * RELATED(D[Poids]))) ORDER BY S[Region]",
            "EVALUATE SUMMARIZECOLUMNS(G[GName], \"x\", SUMX(F, RELATED(D[Prix]))) ORDER BY G[GName]",
        ] {
            check(&mut s, q);
        }
    }
}

/// The grouped scans the path reads are cached: new data, a model edit and `dax.clear_cache`
/// The grouped scans the path reads are cached: new data in the dimension, `dax.clear_cache`
/// and a relationship edit all give fresh results.
#[test]
fn grouped_iterators_follow_new_data_and_model_edits() {
    let q =
        "EVALUATE SUMMARIZECOLUMNS(D[DName], \"x\", SUMX(F, F[Qty] * RELATED(D[Prix]))) ORDER BY D[DName]";
    let mut s = snowflake(11, 2_000, false);
    check(&mut s, q);
    let first = evaluate(&mut s, q, true).unwrap();
    // new products and prices in the same session (cached scans and lookup tables kept so far)
    let reprice = |s: &Session, step: i64| {
        let keys = 1..=200i64;
        super::super::inject_table(
            s,
            "D",
            200,
            vec![
                ("DKey".into(), col(keys.clone().map(CellValue::Int).collect())),
                (
                    "DName".into(),
                    col(keys
                        .clone()
                        .map(|k| CellValue::Text(format!("N{}", k % 7)))
                        .collect()),
                ),
                (
                    "GKey".into(),
                    col(keys.clone().map(|_| CellValue::Int(1)).collect()),
                ),
                (
                    "Prix".into(),
                    col(keys.clone().map(|k| CellValue::Decimal(k * step)).collect()),
                ),
                ("Poids".into(), col(keys.map(|_| CellValue::Int(1)).collect())),
            ],
        );
    };
    reprice(&s, 100);
    let second = evaluate(&mut s, q, true).unwrap();
    check(&mut s, q);
    assert!(!same(&first, &second), "new data, new answer");
    reprice(&s, 300);
    let third = evaluate(&mut s, q, true).unwrap();
    check(&mut s, q);
    assert!(!same(&second, &third), "new prices, new answer");
    // a cleared cache, then a relationship made inactive: RELATED no longer resolves
    s.execute("dax.clear_cache", &json!({})).unwrap();
    assert!(same(&evaluate(&mut s, q, true).unwrap(), &third));
    s.execute(
        "relationship.set",
        &json!({"from": "F[DKey]", "to": "D[DKey]", "active": false}),
    )
    .unwrap();
    check(&mut s, q);
}
