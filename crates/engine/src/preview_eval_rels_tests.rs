//! Relationship indexes kept by the session (`preview_eval_rels.rs`): built once per (model,
//! data), kept by warm queries and `dax.clear_cache`, never serving a stale result after a model
//! edit, a relationship edit or new data (each checked against a fresh session holding the same
//! model and data), plus a micro-bench on a big dimension:
//!
//! `cargo test --release -p bicraft-engine rel_micro -- --ignored --nocapture`
//! (`BICRAFT_MICRO_DIM_ROWS`, default 2 M customers; `BICRAFT_MICRO_ROWS`, default 4 M sales).

use std::time::Instant;

use bicraft_vertipaq::CellValue;
use serde_json::{Value, json};

use super::super::store::ColData;
use super::storage_tests::{evaluate, same};
use crate::Session;

fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

fn col(values: Vec<CellValue>) -> ColData {
    ColData::from_stored(values).unwrap()
}

/// What the test session holds.
#[derive(Clone)]
struct Spec {
    customers: u64,
    sales: u64,
    /// Customers missing from the dimension (their sales go to the blank row).
    missing: u64,
    /// The data's seed (new data: another seed).
    seed: u64,
    /// The relationship's many side: `ClientKey` or `ClientKey2`.
    from: &'static str,
    active: bool,
    /// The relationship is many-to-many.
    many_many: bool,
}

impl Default for Spec {
    fn default() -> Self {
        Spec {
            customers: 500,
            sales: 5_000,
            missing: 20,
            seed: 1,
            from: "ClientKey",
            active: true,
            many_many: false,
        }
    }
}

fn model(spec: &Spec) -> Session {
    let mut s = Session::new();
    let mut steps: Vec<(&str, Value)> = vec![("model.new", json!({"name": "Rel", "culture": "en-US"}))];
    let tables: [(&str, &[(&str, &str)]); 2] = [
        ("Client", &[("ClientKey", "int64"), ("Segment", "string")]),
        (
            "Ventes",
            &[
                ("ClientKey", "int64"),
                ("ClientKey2", "int64"),
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
    let mut rel =
        json!({"from": format!("Ventes[{}]", spec.from), "to": "Client[ClientKey]", "active": spec.active});
    if spec.many_many {
        rel["from_cardinality"] = json!("many");
        rel["to_cardinality"] = json!("many");
        rel["cross_filter"] = json!("single");
    }
    steps.push(("relationship.add", rel));
    steps.push((
        "measure.add",
        json!({"table": "Ventes", "name": "CA", "expression": "SUM(Ventes[Montant])"}),
    ));
    for (id, p) in steps {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"));
    }
    s
}

fn inject(s: &Session, spec: &Spec) {
    inject_client(s, spec);
    inject_sales(s, spec);
}

fn inject_client(s: &Session, spec: &Spec) {
    let mut keys = Vec::new();
    let mut segs = Vec::new();
    for k in 1..=spec.customers {
        keys.push(CellValue::Int(k as i64));
        segs.push(CellValue::Text(format!("S{}", splitmix(k ^ spec.seed) % 7)));
    }
    super::super::inject_table(
        s,
        "Client",
        spec.customers as usize,
        vec![("ClientKey".into(), col(keys)), ("Segment".into(), col(segs))],
    );
}

fn inject_sales(s: &Session, spec: &Spec) {
    let span = spec.customers + spec.missing;
    let (mut k1, mut k2, mut amt) = (Vec::new(), Vec::new(), Vec::new());
    for k in 1..=spec.sales {
        let r = splitmix(k ^ (spec.seed << 32));
        k1.push(CellValue::Int((1 + r % span) as i64));
        k2.push(CellValue::Int((1 + (r >> 24) % spec.customers) as i64));
        amt.push(CellValue::Decimal(((r >> 40) % 10_000) as i64 * 100));
    }
    super::super::inject_table(
        s,
        "Ventes",
        spec.sales as usize,
        vec![
            ("ClientKey".into(), col(k1)),
            ("ClientKey2".into(), col(k2)),
            ("Montant".into(), col(amt)),
        ],
    );
}

fn session(spec: &Spec) -> Session {
    let s = model(spec);
    inject(&s, spec);
    s
}

const QUERIES: &[&str] = &[
    "EVALUATE SUMMARIZECOLUMNS(Client[Segment], \"CA\", [CA]) ORDER BY Client[Segment]",
    "EVALUATE ROW(\"n\", COUNTROWS(VALUES(Client[Segment])), \"k\", COUNTROWS(VALUES(Client[ClientKey])))",
    "EVALUATE ROW(\"c\", CALCULATE([CA], Client[Segment] = \"S1\"), \"t\", CALCULATE([CA], ALL(Client)))",
    "EVALUATE ROW(\"r\", COUNTROWS(FILTER(Ventes, RELATED(Client[Segment]) = \"S2\")))",
    "EVALUATE ROW(\"x\", SUMX(Client, [CA]))",
];

/// Every query's result or error, storage engine on (and checked against the reference
/// evaluator).
fn results(s: &mut Session) -> Vec<Value> {
    QUERIES
        .iter()
        .map(|q| {
            let on = evaluate(s, q, true).unwrap_or_else(|e| json!({ "error": e }));
            let off = evaluate(s, q, false).unwrap_or_else(|e| json!({ "error": e }));
            assert!(same(&on, &off), "{q}: storage {on} vs reference {off}");
            on
        })
        .collect()
}

fn builds(s: &Session) -> u64 {
    s.preview.lock().rel_indexes.builds
}

fn assert_same(a: &[Value], b: &[Value], what: &str) {
    for ((q, x), y) in QUERIES.iter().zip(a).zip(b) {
        assert!(same(x, y), "{what}: {q}: {x} vs {y}");
    }
}

/// The results of a fresh session holding `spec` (nothing kept from any earlier query).
fn fresh(spec: &Spec) -> Vec<Value> {
    results(&mut session(spec))
}

#[test]
fn rel_indexes_are_built_once_and_kept_by_warm_queries_and_clear_cache() {
    let spec = Spec::default();
    let mut s = session(&spec);
    let first = results(&mut s);
    let n = builds(&s);
    assert!(n > 0, "the queries use the relationship");
    let warm = results(&mut s);
    assert_same(&first, &warm, "warm");
    assert_eq!(builds(&s), n, "warm queries rebuild no relationship index");
    s.execute("dax.clear_cache", &json!({})).unwrap();
    let cold = results(&mut s);
    assert_same(&first, &cold, "after dax.clear_cache");
    assert_eq!(builds(&s), n, "dax.clear_cache keeps the relationship indexes");
    // blank row: sales of missing customers
    let v = evaluate(&mut s, QUERIES[1], true).unwrap();
    assert!(
        v.to_string().contains("\"[n]\":8"),
        "7 segments plus the blank row: {v}"
    );
}

#[test]
fn rel_indexes_are_dropped_by_a_model_edit() {
    let spec = Spec::default();
    let mut s = session(&spec);
    let before = results(&mut s);
    let n = builds(&s);
    s.execute(
        "measure.add",
        &json!({"table": "Ventes", "name": "Autre", "expression": "COUNTROWS(Ventes)"}),
    )
    .unwrap();
    let after = results(&mut s);
    assert_same(&before, &after, "after a measure edit");
    assert!(builds(&s) > n, "a model edit rebuilds the relationship indexes");
}

#[test]
fn rel_indexes_never_serve_old_data() {
    let spec = Spec::default();
    let mut s = session(&spec);
    let _ = results(&mut s);
    let n = builds(&s);
    // new data in both tables: another seed, more customers missing
    let new = Spec {
        seed: 7,
        missing: 60,
        customers: 400,
        ..spec.clone()
    };
    inject(&s, &new);
    let after = results(&mut s);
    assert!(builds(&s) > n, "new data rebuilds the relationship indexes");
    assert_same(&after, &fresh(&new), "after new data");
    // new data in the dimension only: now holds every key of the sales (no blank row)
    let full = Spec {
        customers: 460,
        ..new.clone()
    };
    inject_client(&s, &full);
    let mut f = model(&full);
    inject_client(&f, &full);
    inject_sales(&f, &new);
    assert_same(&results(&mut s), &results(&mut f), "after new dimension data");
    let v = evaluate(&mut s, QUERIES[1], true).unwrap();
    assert!(v.to_string().contains("\"[n]\":7"), "no blank row any more: {v}");
    // new data in the fact only: keys beyond the dimension again (a blank row)
    let more = Spec {
        customers: 460,
        missing: 100,
        seed: 9,
        ..new
    };
    inject_sales(&s, &more);
    let mut f = model(&more);
    inject_client(&f, &full);
    inject_sales(&f, &more);
    assert_same(&results(&mut s), &results(&mut f), "after new fact data");
    let v = evaluate(&mut s, QUERIES[1], true).unwrap();
    assert!(v.to_string().contains("\"[n]\":8"), "a blank row again: {v}");
}

#[test]
fn rel_indexes_follow_relationship_edits() {
    let spec = Spec::default();
    let mut s = session(&spec);
    let _ = results(&mut s);
    // deactivated: the dimension filters no sale
    s.execute(
        "relationship.set",
        &json!({"from": "Ventes[ClientKey]", "to": "Client[ClientKey]", "active": false}),
    )
    .unwrap();
    let inactive = Spec {
        active: false,
        ..spec.clone()
    };
    let r: Vec<Value> = QUERIES
        .iter()
        .filter(|q| !q.contains("RELATED"))
        .map(|q| evaluate(&mut s, q, true).unwrap())
        .collect();
    let mut f = session(&inactive);
    let e: Vec<Value> = QUERIES
        .iter()
        .filter(|q| !q.contains("RELATED"))
        .map(|q| evaluate(&mut f, q, true).unwrap())
        .collect();
    for (x, y) in r.iter().zip(&e) {
        assert!(same(x, y), "inactive relationship: {x} vs {y}");
    }
    // moved to another column of the fact
    s.execute(
        "relationship.remove",
        &json!({"from": "Ventes[ClientKey]", "to": "Client[ClientKey]"}),
    )
    .unwrap();
    s.execute(
        "relationship.add",
        &json!({"from": "Ventes[ClientKey2]", "to": "Client[ClientKey]"}),
    )
    .unwrap();
    let moved = Spec {
        from: "ClientKey2",
        ..spec.clone()
    };
    assert_same(&results(&mut s), &fresh(&moved), "relationship on another column");
    // many-to-many: no blank row from it
    s.execute(
        "relationship.remove",
        &json!({"from": "Ventes[ClientKey2]", "to": "Client[ClientKey]"}),
    )
    .unwrap();
    s.execute(
        "relationship.add",
        &json!({"from": "Ventes[ClientKey]", "to": "Client[ClientKey]", "from_cardinality": "many",
                "to_cardinality": "many", "cross_filter": "single"}),
    )
    .unwrap();
    let mm = Spec {
        many_many: true,
        ..spec
    };
    assert_same(&results(&mut s), &fresh(&mm), "many-to-many relationship");
}

#[test]
fn rel_indexes_after_a_culture_change() {
    let spec = Spec::default();
    let mut s = session(&spec);
    let _ = results(&mut s);
    let n = builds(&s);
    s.execute("model.set", &json!({"culture": "fr-FR"})).unwrap();
    // (the partitions depend on the culture: their data is loaded again)
    inject(&s, &spec);
    let _ = results(&mut s);
    assert!(
        builds(&s) > n,
        "another culture rebuilds the relationship indexes"
    );
}

fn env_rows(key: &str, default: u64) -> u64 {
    bicraft_env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Median time of `reps` runs of `q` with `{i}` replaced by a number never used before (each
/// run is a new query: no result kept by text answers it, as when a visual's selection changes).
fn median_ms(s: &mut Session, q: &str, reps: usize, next: &mut u64) -> f64 {
    let mut t: Vec<f64> = (0..reps)
        .map(|_| {
            *next += 1;
            let text = q
                .replace("{i}", &next.to_string())
                .replace("{s}", &(*next % 7).to_string());
            let at = Instant::now();
            evaluate(s, &text, true).unwrap_or_else(|e| panic!("{text}: {e}"));
            at.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    t.sort_by(f64::total_cmp);
    t.get(t.len() / 2).copied().unwrap_or(f64::NAN)
}

/// Warm time per query on a big dimension, with the session's relationship indexes and without
/// them (rebuilt by every query, as before they were kept). Every run is a new query (a new
/// selection), as visuals ask after a slicer change.
#[test]
#[ignore = "micro-bench: cargo test --release -p bicraft-engine rel_micro -- --ignored --nocapture"]
fn rel_micro() {
    let spec = Spec {
        customers: env_rows("BICRAFT_MICRO_DIM_ROWS", 2_000_000),
        sales: env_rows("BICRAFT_MICRO_ROWS", 4_000_000),
        missing: 1_000,
        ..Spec::default()
    };
    let at = Instant::now();
    let mut s = session(&spec);
    println!(
        "{} customers, {} sales, built in {:.1} s",
        spec.customers,
        spec.sales,
        at.elapsed().as_secs_f64()
    );
    let queries = [
        "EVALUATE SUMMARIZECOLUMNS(Client[Segment], \"CA\", [CA], \"i\", {i}) ORDER BY Client[Segment]",
        "EVALUATE ROW(\"c\", CALCULATE([CA], Client[Segment] = \"S{s}\"), \"i\", {i})",
        "EVALUATE ROW(\"n\", COUNTROWS(VALUES(Client[Segment])), \"i\", {i})",
        "EVALUATE SUMMARIZECOLUMNS(Client[Segment], TREATAS({\"S{s}\"}, Client[Segment]), \"CA\", [CA], \"i\", {i})",
        "EVALUATE ROW(\"c\", CALCULATE([CA], Client[ClientKey] <= {i}000), \"i\", {i})",
    ];
    let mut next = 0u64;
    println!("query | warm kept (ms) | warm rebuilt (ms) | speed-up");
    for (i, q) in queries.iter().enumerate() {
        // every selection seen once in both modes, then the two modes interleaved
        median_ms(&mut s, q, 14, &mut next);
        super::rels::set_keep(false);
        median_ms(&mut s, q, 14, &mut next);
        super::rels::set_keep(true);
        let (mut k, mut r) = (Vec::new(), Vec::new());
        for _ in 0..9 {
            k.push(median_ms(&mut s, q, 1, &mut next));
            super::rels::set_keep(false);
            r.push(median_ms(&mut s, q, 1, &mut next));
            super::rels::set_keep(true);
        }
        k.sort_by(f64::total_cmp);
        r.sort_by(f64::total_cmp);
        let (kept, rebuilt) = (k[4], r[4]);
        println!("M{} | {kept:.2} | {rebuilt:.2} | x{:.1}", i + 1, rebuilt / kept);
    }
}

/// What the session keeps from the measures' text (fingerprint, `ALLSELECTED` used) follows a
/// measure edit: a measure added with `ALLSELECTED` gets the shadow filter context.
#[test]
fn measure_facts_follow_measure_edits() {
    let spec = Spec::default();
    let mut s = session(&spec);
    let _ = results(&mut s);
    assert_eq!(s.preview.lock().measure_facts.map(|f| f.allselected), Some(false));
    let add = json!({"table": "Ventes", "name": "Part",
                     "expression": "DIVIDE([CA], CALCULATE([CA], ALLSELECTED(Client[Segment])))"});
    s.execute("measure.add", &add).unwrap();
    let q =
        "EVALUATE FILTER(SUMMARIZECOLUMNS(Client[Segment], \"P\", [Part]), [P] > 0) ORDER BY Client[Segment]";
    let mut f = session(&spec);
    f.execute("measure.add", &add).unwrap();
    let a = evaluate(&mut s, q, true).unwrap();
    let b = evaluate(&mut f, q, true).unwrap();
    assert!(same(&a, &b), "{a} vs {b}");
    let facts = |s: &Session| s.preview.lock().measure_facts.map(|f| f.allselected);
    assert_eq!(facts(&s), Some(true), "the added measure uses ALLSELECTED");
    // a query measure of the same text: the query's own model, not the session's facts
    let d = "DEFINE MEASURE Ventes[Q] = SUM(Ventes[Montant]) EVALUATE ROW(\"q\", [Q], \"c\", [CA])";
    let v = evaluate(&mut s, d, true).unwrap();
    assert!(v.to_string().contains("\"[q]\""), "{v}");
    s.execute("measure.remove", &json!({"name": "Part"})).unwrap();
    f.execute("measure.remove", &json!({"name": "Part"})).unwrap();
    assert_same(&results(&mut s), &results(&mut f), "after a measure removal");
    assert_eq!(facts(&s), Some(false), "ALLSELECTED no longer used");
}
