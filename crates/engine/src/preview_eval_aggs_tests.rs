//! Report-derived aggregations (`docs/storage-engine.md`, "Aggregations"): the derivation, the
//! planner's exactness against the fact (fixed and random queries, cold and warm), and one
//! invalidation test per change that must drop them (data, measure, column, relationship,
//! report visual and filter).

use std::collections::BTreeSet;

use bicraft_vertipaq::CellValue;
use serde_json::{Value, json};

use super::super::{set_aggregation_config, work_counters};
use super::storage::aggs::{AggConfig, set_test_aggregations};
use super::storage_fuzz::{DAY0, Rng, Spec, big_spec, col, model, packed, query, spec};
use super::storage_tests::evaluate;
use crate::Session;

/// A report over the fuzz model: the shapes the fuzz queries take, as visuals.
fn add_report(s: &mut Session) {
    let run = |s: &mut Session, id: &str, p: Value| -> Value {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
    };
    run(s, "report.new", json!({}));
    run(
        s,
        "report_measure.add",
        json!({"table": "Ventes", "name": "CA catalogue",
            "expression": "SUMX(Ventes, Ventes[Quantite] * RELATED(Produit[Prix]))"}),
    );
    let visuals: &[(&str, &[&str])] = &[
        (
            "tableEx",
            &[
                "Produit[Categorie]",
                "Magasin[Region]",
                "[CA]",
                "[Qte]",
                "[Lignes]",
            ],
        ),
        (
            "tableEx",
            &["Calendrier[Annee]", "Calendrier[Mois]", "[CA YTD]", "[CA livré]"],
        ),
        (
            "tableEx",
            &["Ventes[Quantite]", "Produit[Categorie]", "[Rang]", "[Part CA]"],
        ),
        (
            "tableEx",
            &[
                "Magasin[MagasinKey]",
                "Calendrier[Annee]",
                "[Qte]",
                "Min(Ventes[Montant])",
            ],
        ),
        ("card", &["[CA catalogue]"]),
        ("tableEx", &["Magasin[Region]", "[Nb clients]"]),
    ];
    for (i, (ty, fields)) in visuals.iter().enumerate() {
        let page = format!("P{i}");
        run(s, "page.add", json!({"display_name": page}));
        let fields: Vec<Value> = fields
            .iter()
            .map(|f| json!({"role": "Values", "field": f}))
            .collect();
        run(
            s,
            "visual.add",
            json!({"page": page, "visual_type": ty, "x": 0, "y": 0, "width": 400, "height": 300, "fields": fields}),
        );
        if i == 0 {
            run(
                s,
                "visual.add",
                json!({"page": page, "visual_type": "slicer", "x": 500, "y": 0, "width": 200, "height": 300,
                    "fields": [{"role": "Values", "field": "Ventes[Quantite]"}]}),
            );
        }
    }
}

fn aggs(s: &mut Session) -> Value {
    s.execute("dax.aggregations", &json!({})).unwrap()
}

/// The query with aggregations off then on (cold, then warm), each from emptied caches.
fn both(
    s: &mut Session,
    q: &str,
) -> (
    Result<Value, String>,
    Result<Value, String>,
    Result<Value, String>,
) {
    s.execute("dax.clear_cache", &json!({})).unwrap();
    set_test_aggregations(Some(false), Some(0), true);
    let off = evaluate(s, q, true);
    s.execute("dax.clear_cache", &json!({})).unwrap();
    set_test_aggregations(Some(true), Some(0), true);
    // built now (queries never build them: a background job does, after the query)
    s.execute("dax.aggregations", &json!({"build": true})).unwrap();
    let on = evaluate(s, q, true);
    let warm = evaluate(s, q, true);
    (off, on, warm)
}

fn assert_same(s: &mut Session, q: &str, what: &str) -> Value {
    let (off, on, warm) = both(s, q);
    assert_eq!(on, off, "{what}: aggregations differ from the fact: {q}");
    assert_eq!(warm, off, "{what}: warm differs: {q}");
    on.unwrap_or_else(|e| panic!("{what}: {q}: {e}"))
}

fn hits(s: &mut Session) -> u64 {
    aggs(s)["hits"].as_u64().unwrap_or(0)
}

#[test]
fn aggregations_derive_from_visuals_filters_slicers_and_measures() {
    let mut s = model(
        &Spec {
            rows: 500,
            ..big_spec()
        },
        3,
    );
    add_report(&mut s);
    let d = crate::aggs_derive::derive(s.model.as_ref().unwrap(), s.report.as_ref().unwrap());
    let find = |field: &str| {
        d.visuals
            .iter()
            .find(|v| v.to_json().to_string().contains(field))
            .map(|v| v.to_json())
            .unwrap_or_else(|| panic!("no visual with {field}: {:?}", d.visuals))
    };
    // grouping columns, the page's slicer, measures' value columns and facts
    let v = find("'Magasin'[Region]");
    assert!(v["grain"].to_string().contains("'Ventes'[Quantite]"), "{v}"); // the slicer
    assert!(v["values"].to_string().contains("'Ventes'[Montant]"), "{v}");
    assert_eq!(v["facts"], json!(["Ventes"]));
    // time intelligence: the date column is in the grain
    let v = find("'Calendrier'[Mois]");
    assert!(v["grain"].to_string().contains("'Calendrier'[Date]"), "{v}");
    // RANKX over ALL(Produit[Categorie]); ALL(Produit) as a CALCULATE modifier adds nothing
    let v = d
        .visuals
        .iter()
        .map(|v| v.to_json())
        .find(|v| v["grain"] == json!(["'Produit'[Categorie]", "'Ventes'[Quantite]"]))
        .expect("the RANKX visual");
    assert!(v["grain"].to_string().contains("'Produit'[Categorie]"), "{v}");
    // a report measure: SUMX over the fact, RELATED column in the grain, own column a value
    let v = d
        .visuals
        .iter()
        .map(|v| v.to_json())
        .find(|v| v["grain"].to_string().contains("'Produit'[Prix]"))
        .expect("the SUMX card");
    assert!(v["values"].to_string().contains("'Ventes'[Quantite]"), "{v}");
    // DISTINCTCOUNT: its column joins the grain
    let v = find("'Ventes'[ClientKey]");
    assert!(v["values"].to_string().contains("'Ventes'[ClientKey]"), "{v}");
    // deterministic
    let again = crate::aggs_derive::derive(s.model.as_ref().unwrap(), s.report.as_ref().unwrap());
    assert_eq!(again.hash, d.hash);
}

/// Fixed queries of the bench's shapes: answered from aggregations, identical to the fact.
#[test]
fn aggregations_answer_exactly_as_the_fact() {
    for seed in [5u64, 6, 7] {
        let mut r = Rng(seed);
        let sp = spec(&mut r);
        let mut s = model(&sp, seed);
        add_report(&mut s);
        for q in [
            "EVALUATE ROW(\"c\", [CA], \"q\", [Qte], \"n\", COUNTROWS(Ventes))",
            "EVALUATE SUMMARIZECOLUMNS(Produit[Categorie], \"c\", [CA]) ORDER BY Produit[Categorie]",
            "EVALUATE SUMMARIZECOLUMNS(Magasin[Region], Produit[Categorie], \"c\", [CA], \"q\", [Qte], \"l\", [Lignes]) \
             ORDER BY Magasin[Region], Produit[Categorie]",
            "EVALUATE SUMMARIZECOLUMNS(Calendrier[Annee], Calendrier[Mois], \"y\", [CA YTD], \"l\", [CA livré]) \
             ORDER BY Calendrier[Annee], Calendrier[Mois]",
            "EVALUATE SUMMARIZECOLUMNS(Ventes[Quantite], Produit[Categorie], \"r\", [Rang], \"p\", [Part CA]) \
             ORDER BY Ventes[Quantite], Produit[Categorie]",
            "EVALUATE SUMMARIZECOLUMNS(Magasin[MagasinKey], Calendrier[Annee], \"q\", [Qte], \"m\", MIN(Ventes[Montant]), \
             \"x\", MAX(Ventes[Montant]), \"a\", AVERAGE(Ventes[Montant]), \"k\", COUNT(Ventes[Montant]), \
             \"b\", COUNTBLANK(Ventes[Montant])) ORDER BY Magasin[MagasinKey], Calendrier[Annee]",
            "EVALUATE ROW(\"x\", SUMX(Ventes, Ventes[Quantite] * RELATED(Produit[Prix])))",
            "EVALUATE SUMMARIZECOLUMNS(Magasin[Region], \"d\", [Nb clients]) ORDER BY Magasin[Region]",
            "DEFINE VAR f = TREATAS({\"C01\", \"C02\"}, Produit[Categorie]) EVALUATE SUMMARIZECOLUMNS(Magasin[Region], f, \
             \"c\", [CA]) ORDER BY Magasin[Region]",
            "EVALUATE SUMMARIZECOLUMNS(Produit[Categorie], FILTER(KEEPFILTERS(VALUES(Ventes[Quantite])), Ventes[Quantite] > 2), \
             \"c\", [CA]) ORDER BY Produit[Categorie]",
            "EVALUATE ROW(\"c\", SUMX(FILTER(Ventes, Ventes[Quantite] >= 3), Ventes[Montant]))",
            "EVALUATE ROW(\"a\", AVERAGEX(VALUES(Magasin[Region]), [CA]))",
        ] {
            assert_same(&mut s, q, &format!("seed {seed}"));
        }
        let a = aggs(&mut s);
        assert!(
            a["hits"].as_u64().unwrap_or(0) > 0,
            "no scan answered by an aggregation: {a}"
        );
        assert!(a["total_rows"].as_u64().unwrap_or(0) > 0, "{a}");
    }
    set_test_aggregations(None, None, false);
}

/// Random queries (the storage engine's fuzz grammar) with and without aggregations: identical,
/// cold and warm.
#[test]
fn aggregations_fuzz_matches_the_fact() {
    let cases: usize = bicraft_env::var("BICRAFT_FUZZ_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(120);
    let start: u64 = bicraft_env::var("BICRAFT_FUZZ_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0xA66_0001);
    let mut failures = Vec::new();
    let mut done = 0usize;
    let mut answered = 0u64;
    let mut m = 0u64;
    while done < cases && failures.len() < 5 {
        let seed = start.wrapping_add(m);
        m += 1;
        let mut r = Rng(seed);
        let sp = spec(&mut r);
        let mut s = model(&sp, seed);
        add_report(&mut s);
        for _ in 0..24usize.min(cases - done) {
            let q = query(&mut r).text();
            done += 1;
            let (off, on, warm) = both(&mut s, &q);
            if on != off || warm != off {
                failures.push(format!(
                    "seed {seed} model {sp:?}\n  {q}\n  off  {off:?}\n  on   {on:?}\n  warm {warm:?}"
                ));
                break;
            }
        }
        answered += hits(&mut s);
    }
    set_test_aggregations(None, None, false);
    println!("aggregations fuzz: {done} queries, {answered} scans answered by aggregations");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert!(
        answered as usize * 2 >= done,
        "too few scans answered by aggregations: {answered}"
    );
}

fn fact_rows(sp: &Spec, seed: u64) -> Vec<(String, super::super::store::ColData)> {
    let mut r = Rng(seed);
    let mut v: [Vec<CellValue>; 8] = Default::default();
    for _ in 0..sp.rows {
        let day = r.below(sp.days);
        v[0].push(CellValue::DateTime(DAY0 + day as f64));
        v[1].push(CellValue::DateTime(DAY0 + day as f64));
        v[2].push(CellValue::Int(1 + r.below(sp.products) as i64));
        v[3].push(CellValue::Int(1 + r.below(sp.stores) as i64));
        v[4].push(CellValue::Int(1 + r.below(sp.customers) as i64));
        v[5].push(CellValue::Int(r.below(9) as i64));
        v[6].push(CellValue::Decimal(r.below(1_000_000) as i64));
        v[7].push(CellValue::Real(r.below(1000) as f64 / 7.0));
    }
    let names = [
        "Date",
        "Livraison",
        "ProduitKey",
        "MagasinKey",
        "ClientKey",
        "Quantite",
        "Montant",
        "Poids",
    ];
    names
        .iter()
        .zip(v)
        .map(|(n, vals)| (n.to_string(), packed(col(vals), 512)))
        .collect()
}

const Q: &str = "EVALUATE SUMMARIZECOLUMNS(Produit[Categorie], Magasin[Region], \"c\", [CA], \"q\", [Qte]) \
                 ORDER BY Produit[Categorie], Magasin[Region]";

fn agg_names(s: &mut Session) -> Vec<String> {
    let a = aggs(s);
    a["facts"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|f| f["aggregations"].as_array().cloned().unwrap_or_default())
        .map(|x| format!("{} {}", x["name"], x["rows"]))
        .collect()
}

fn small() -> (Spec, Session) {
    let sp = Spec {
        rows: 3_000,
        ..big_spec()
    };
    let mut s = model(&sp, 31);
    add_report(&mut s);
    (sp, s)
}

#[test]
fn aggregations_follow_a_data_refresh() {
    let (sp, mut s) = small();
    let before = assert_same(&mut s, Q, "first");
    assert!(hits(&mut s) > 0);
    super::super::inject_table(&s, "Ventes", sp.rows as usize, fact_rows(&sp, 77));
    let after = assert_same(&mut s, Q, "after new data");
    assert_ne!(after, before, "new data: new results");
    assert!(hits(&mut s) > 0, "rebuilt over the new data");
    set_test_aggregations(None, None, false);
}

#[test]
fn aggregations_follow_a_measure_change() {
    let (_, mut s) = small();
    let before = assert_same(&mut s, Q, "first");
    s.execute(
        "measure.set",
        &json!({"name": "CA", "expression": "SUM(Ventes[Quantite])"}),
    )
    .unwrap();
    let after = assert_same(&mut s, Q, "after measure.set");
    assert_ne!(after, before);
    assert!(hits(&mut s) > 0);
    set_test_aggregations(None, None, false);
}

#[test]
fn aggregations_follow_a_column_change() {
    let (_, mut s) = small();
    assert_same(&mut s, Q, "first");
    let key = aggs(&mut s)["key"].clone();
    // a column of the model changes: the aggregations are keyed again and derived again
    s.execute(
        "column.set",
        &json!({"table": "Ventes", "name": "Montant", "format_string": "0.00", "description": "changed"}),
    )
    .unwrap();
    assert_same(&mut s, Q, "after column.set");
    let a = aggs(&mut s);
    assert_ne!(a["key"], key, "{a}");
    assert!(a["hits"].as_u64().unwrap_or(0) > 0, "{a}");
    set_test_aggregations(None, None, false);
}

#[test]
fn aggregations_follow_a_relationship_change() {
    let (_, mut s) = small();
    let before = assert_same(&mut s, Q, "first");
    s.execute(
        "relationship.set",
        &json!({"from": "Ventes[MagasinKey]", "to": "Magasin[MagasinKey]", "active": false}),
    )
    .unwrap();
    let after = assert_same(&mut s, Q, "after the relationship is made inactive");
    assert_ne!(after, before);
    s.execute(
        "relationship.set",
        &json!({"from": "Ventes[MagasinKey]", "to": "Magasin[MagasinKey]", "active": true, "cross_filter": "both"}),
    )
    .unwrap();
    assert_same(&mut s, Q, "after the relationship is made bidirectional");
    assert_same(
        &mut s,
        "EVALUATE SUMMARIZECOLUMNS(Magasin[Region], \"n\", CALCULATE(COUNTROWS(Magasin), Produit[Categorie] = \"C02\")) \
         ORDER BY Magasin[Region]",
        "a dimension counted through the fact",
    );
    set_test_aggregations(None, None, false);
}

#[test]
fn aggregations_follow_report_visual_and_filter_changes() {
    let (_, mut s) = small();
    assert_same(&mut s, Q, "first");
    let before = agg_names(&mut s);
    assert!(
        !before
            .iter()
            .any(|n| n.contains("[Mois]") && n.contains("[Region]")),
        "{before:?}"
    );
    // a new visual by month and region: the derivation changes, the aggregations follow
    s.execute("page.add", &json!({"display_name": "New"})).unwrap();
    let page = "New".to_string();
    let v = s
        .execute(
            "visual.add",
            &json!({"page": page, "visual_type": "tableEx", "x": 0, "y": 0, "width": 300, "height": 300,
            "fields": [{"role": "Values", "field": "Calendrier[Mois]"}, {"role": "Values", "field": "Magasin[Region]"},
                       {"role": "Values", "field": "[Qte]"}]}),
        )
        .unwrap();
    let q = "EVALUATE SUMMARIZECOLUMNS(Calendrier[Mois], Magasin[Region], \"q\", [Qte]) ORDER BY Calendrier[Mois], Magasin[Region]";
    assert_same(&mut s, q, "after visual.add");
    let after = agg_names(&mut s);
    assert_ne!(after, before, "a new visual: aggregations derived again");
    // a visual filter on a fact column: it joins that visual's grain
    let visual = v["visual"].as_str().unwrap().to_string();
    s.execute(
        "visual.filter",
        &json!({"page": page, "visual": visual, "field": "Ventes[Quantite]", "op": ">", "value": 2}),
    )
    .unwrap();
    let q2 = "EVALUATE SUMMARIZECOLUMNS(Calendrier[Mois], Magasin[Region], FILTER(KEEPFILTERS(VALUES(Ventes[Quantite])), \
              Ventes[Quantite] > 2), \"q\", [Qte]) ORDER BY Calendrier[Mois], Magasin[Region]";
    assert_same(&mut s, q2, "after visual.filter");
    let filtered = agg_names(&mut s);
    assert_ne!(filtered, after, "a new filter: aggregations derived again");
    set_test_aggregations(None, None, false);
}

/// Off by `dax.aggregations {enabled: false}`: nothing is answered from them.
#[test]
fn aggregations_can_be_turned_off_for_the_session() {
    let (_, mut s) = small();
    set_test_aggregations(None, Some(0), true);
    s.execute("dax.aggregations", &json!({"enabled": false})).unwrap();
    evaluate(&mut s, Q, true).unwrap();
    let a = aggs(&mut s);
    assert_eq!(a["enabled"], json!(false));
    assert_eq!(a["hits"].as_u64().unwrap_or(0), 0, "{a}");
    let built = s
        .execute("dax.aggregations", &json!({"enabled": true, "build": true}))
        .unwrap();
    assert!(built["total_rows"].as_u64().unwrap_or(0) > 0, "{built}");
    set_test_aggregations(None, None, false);
}

/// A query never builds aggregations: it scans the fact, and a background job builds them after
/// it; later queries use them, with the same results. New data while a job runs: its result is
/// dropped, never served.
#[test]
fn aggregations_build_in_the_background_off_the_query_path() {
    let (sp, mut s) = small();
    // background builds, whatever BICRAFT_AGG_SYNC says
    set_aggregation_config(
        &s,
        AggConfig {
            sync: Some(false),
            ..AggConfig::default()
        },
    );
    set_test_aggregations(Some(true), Some(0), true);
    let first = evaluate(&mut s, Q, true).unwrap();
    // what a server does once the answer is sent
    s.background_work();
    let a = aggs(&mut s);
    assert_eq!(
        a["hits"].as_u64().unwrap_or(0),
        0,
        "the first query is answered by the fact: {a}"
    );
    assert_eq!(a["jobs"].as_u64().unwrap_or(0), 1, "a job started after it: {a}");
    let mut answered = false;
    // the job works while the session is idle (no query for 200 ms)
    for _ in 0..60 {
        std::thread::sleep(std::time::Duration::from_millis(300));
        s.execute("dax.clear_cache", &json!({})).unwrap();
        let again = evaluate(&mut s, Q, true).unwrap();
        s.background_work();
        assert_eq!(again, first);
        if hits(&mut s) > 0 {
            answered = true;
            break;
        }
    }
    assert!(
        answered,
        "the background job's aggregations were never used: {}",
        aggs(&mut s)
    );
    // new data while a new job runs: the job is dropped with the old key
    super::super::inject_table(&s, "Ventes", sp.rows as usize, fact_rows(&sp, 78));
    let fresh = evaluate(&mut s, Q, true).unwrap();
    set_test_aggregations(Some(false), Some(0), true);
    s.execute("dax.clear_cache", &json!({})).unwrap();
    let off = evaluate(&mut s, Q, true).unwrap();
    assert_eq!(fresh, off);
    assert_ne!(fresh, first);
    set_test_aggregations(None, None, false);
}

/// Real-file trial (local only, ignored): `BICRAFT_AGG_TRIAL=<project.pbip>` renders every page's
/// visual data without and with aggregations (built over the project's data, whatever its size)
/// and compares them; prints counts, sizes and times, never the data.
#[test]
#[ignore]
fn aggregations_real_file_trial() {
    let Ok(path) = bicraft_env::var("BICRAFT_AGG_TRIAL") else {
        println!("BICRAFT_AGG_TRIAL is not set: skipped");
        return;
    };
    let mut s = Session::new();
    s.execute("file.open", &json!({"path": path})).unwrap();
    let limits = crate::SnapshotLimits {
        data_budget_ms: 600_000,
        data_table_rows: usize::MAX,
        ..crate::SnapshotLimits::default()
    };
    let data = |s: &mut Session| -> (Vec<Value>, f64) {
        s.execute("dax.clear_cache", &json!({})).unwrap();
        let t = std::time::Instant::now();
        let snap = s.snapshot_with(&limits);
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        let mut out = Vec::new();
        for p in snap["report"]["pages"].as_array().into_iter().flatten() {
            for v in p["visuals"].as_array().into_iter().flatten() {
                out.push(v["data"].clone());
            }
        }
        (out, ms)
    };
    set_test_aggregations(Some(false), Some(0), false);
    // the first one reads and decodes the data: not compared
    let _ = data(&mut s);
    let (off, off_ms) = data(&mut s);
    set_test_aggregations(Some(true), Some(0), false);
    let built = s.execute("dax.aggregations", &json!({"build": true})).unwrap();
    let (on, on_ms) = data(&mut s);
    let (on_warm, warm_ms) = data(&mut s);
    set_test_aggregations(None, None, false);
    let evaluated = off.iter().filter(|d| !d.is_null()).count();
    let same = off.iter().zip(&on).filter(|(a, b)| a == b).count();
    let same_warm = off.iter().zip(&on_warm).filter(|(a, b)| a == b).count();
    let a = aggs(&mut s);
    let facts = built["facts"].as_array().cloned().unwrap_or_default();
    println!(
        "visuals {} (with data {evaluated}), identical with aggregations {same} (warm {same_warm}); \
         visual requirements {}; facts {}",
        off.len(),
        built["visuals"].as_array().map_or(0, Vec::len),
        facts.len()
    );
    for f in &facts {
        let aggs: Vec<String> = f["aggregations"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|x| {
                format!(
                    "{} grain columns, {} rows, {} bytes, built {}",
                    x["grain"].as_array().map_or(0, Vec::len),
                    x["rows"],
                    x["bytes"],
                    x["built"]
                )
            })
            .collect();
        println!(
            "fact of {} rows: {} visuals, {} uncovered, plan {:.1} ms; aggregations: {aggs:?}",
            f["rows"],
            f["visuals"],
            f["uncovered"].as_array().map_or(0, Vec::len),
            f["plan_ms"].as_f64().unwrap_or(0.0)
        );
    }
    println!(
        "pages data: without {off_ms:.1} ms, with {on_ms:.1} ms (cold), {warm_ms:.1} ms (warm); build {:.1} ms; \
         scans answered by aggregations {} of {}",
        built["build_ms"].as_f64().unwrap_or(0.0),
        a["hits"],
        a["hits"].as_u64().unwrap_or(0) + a["misses"].as_u64().unwrap_or(0)
    );
    assert_eq!(same, off.len(), "visual data differ with aggregations");
}

/// Without aggregations (off, or every fact below the size floor), background work costs
/// nothing: after report edits and queries it takes no lock, synchronizes nothing and hashes no
/// model; with them, a query that wants some makes it work once.
#[test]
fn background_work_is_free_without_aggregations() {
    let (_, mut s) = small();
    set_test_aggregations(None, None, false);
    let page = |s: &mut Session, i: usize| {
        s.execute("page.add", &json!({"display_name": format!("Extra {i}")}))
            .unwrap();
        s.background_work();
    };
    for (on, min_rows) in [(false, Some(0)), (true, Some(usize::MAX))] {
        set_aggregation_config(
            &s,
            AggConfig {
                enabled: Some(on),
                min_rows,
                sync: Some(false),
            },
        );
        s.execute("dax.clear_cache", &json!({})).unwrap();
        evaluate(&mut s, Q, true).unwrap();
        s.background_work();
        let before = work_counters(&s);
        for i in 0..20 {
            page(&mut s, i + if on { 100 } else { 0 });
        }
        let after = work_counters(&s);
        assert_eq!(before.0, 0, "on {on}: background work took the lock");
        assert_eq!(after, before, "on {on}: report edits made background work");
    }
    // on, over a fact above the floor: the query wants aggregations, background work runs once
    set_aggregation_config(
        &s,
        AggConfig {
            enabled: Some(true),
            min_rows: Some(0),
            sync: Some(false),
        },
    );
    s.execute("dax.clear_cache", &json!({})).unwrap();
    evaluate(&mut s, Q, true).unwrap();
    s.background_work();
    s.background_work();
    assert_eq!(work_counters(&s).0, 1, "one query, one round of background work");
}

/// Every built aggregation: (name, id, reused).
fn built_tables(s: &mut Session) -> Vec<(String, u64, bool)> {
    aggs(s)["facts"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|f| f["aggregations"].as_array().cloned().unwrap_or_default())
        .filter_map(|x| {
            Some((
                x["name"].as_str()?.to_string(),
                x["id"].as_u64()?,
                x["reused"].as_bool().unwrap_or(false),
            ))
        })
        .collect()
}

fn build_now(s: &mut Session) -> Value {
    s.execute("dax.aggregations", &json!({"build": true})).unwrap()
}

/// A report change keeps the aggregations it still selects (same instances, not rebuilt) and
/// builds only the new ones; a removed visual's aggregation goes; a model or data change
/// rebuilds them all. Results stay those of the fact throughout.
#[test]
fn aggregations_are_kept_across_report_changes() {
    let (sp, mut s) = small();
    set_test_aggregations(Some(true), Some(0), true);
    build_now(&mut s);
    let first = built_tables(&mut s);
    assert!(!first.is_empty());
    let ids = |t: &[(String, u64, bool)]| t.iter().map(|x| x.1).collect::<BTreeSet<u64>>();
    let first_ids = ids(&first);

    // a visual by month and region: a new grain
    s.execute("page.add", &json!({"display_name": "New"})).unwrap();
    let v = s
        .execute(
            "visual.add",
            &json!({"page": "New", "visual_type": "tableEx", "x": 0, "y": 0, "width": 300, "height": 300,
            "fields": [{"role": "Values", "field": "Calendrier[Mois]"}, {"role": "Values", "field": "Magasin[Region]"},
                       {"role": "Values", "field": "[Qte]"}]}),
        )
        .unwrap();
    let report = build_now(&mut s);
    let second = built_tables(&mut s);
    let reused: Vec<_> = second.iter().filter(|x| x.2).collect();
    let fresh: Vec<_> = second.iter().filter(|x| !x.2).collect();
    assert!(!reused.is_empty(), "nothing kept: {report}");
    assert!(!fresh.is_empty(), "the new grain was not built: {report}");
    assert!(
        reused.iter().all(|x| first_ids.contains(&x.1)),
        "kept = the same instances"
    );
    assert!(fresh.iter().all(|x| !first_ids.contains(&x.1)), "new = built now");
    assert_eq!(
        report["built_tables"].as_u64(),
        Some(fresh.len() as u64),
        "{report}"
    );
    assert_eq!(
        report["reused_tables"].as_u64(),
        Some(reused.len() as u64),
        "{report}"
    );
    let q2 = "EVALUATE SUMMARIZECOLUMNS(Calendrier[Mois], Magasin[Region], \"q\", [Qte]) \
              ORDER BY Calendrier[Mois], Magasin[Region]";
    assert_same(&mut s, q2, "the new grain");
    assert_same(&mut s, Q, "a kept grain");
    // the same queries through the kept and new aggregations, warm, without a rebuild
    set_test_aggregations(Some(false), Some(0), true);
    s.execute("dax.clear_cache", &json!({})).unwrap();
    let off = evaluate(&mut s, q2, true);
    set_test_aggregations(Some(true), Some(0), true);
    let on = evaluate(&mut s, q2, true);
    assert_eq!(on, off);

    // the visual removed: the aggregations only it needed go, the others stay
    let second = built_tables(&mut s);
    let second_ids = ids(&second);
    let visual = v["visual"].as_str().unwrap().to_string();
    s.execute("visual.remove", &json!({"page": "New", "visual": visual}))
        .unwrap();
    let report = build_now(&mut s);
    let third = built_tables(&mut s);
    let names = |t: &[(String, u64, bool)]| t.iter().map(|x| x.0.clone()).collect::<BTreeSet<_>>();
    // kept: those of the second set selected again; built: only grains the second set had not
    // (the planner may merge grains differently once the visual is there)
    let second_names = names(&second);
    assert!(third.iter().any(|x| x.2), "{report}");
    assert!(
        third.iter().all(|x| if x.2 {
            second_ids.contains(&x.1)
        } else {
            !second_names.contains(&x.0)
        }),
        "{report}"
    );
    assert_eq!(names(&third), names(&first), "back to the first set: {report}");
    assert_eq!(
        report["kept_for_reuse"].as_u64(),
        Some(0),
        "the dropped ones are not kept"
    );

    // a model change: every aggregation is rebuilt
    s.execute(
        "column.set",
        &json!({"table": "Ventes", "name": "Montant", "description": "changed"}),
    )
    .unwrap();
    let report = build_now(&mut s);
    let fourth = built_tables(&mut s);
    assert_eq!(report["reused_tables"].as_u64(), Some(0), "{report}");
    assert!(fourth.iter().all(|x| !ids(&third).contains(&x.1)));

    // new data: every aggregation is rebuilt
    super::super::inject_table(&s, "Ventes", sp.rows as usize, fact_rows(&sp, 79));
    let report = build_now(&mut s);
    let fifth = built_tables(&mut s);
    assert_eq!(report["reused_tables"].as_u64(), Some(0), "{report}");
    assert!(fifth.iter().all(|x| !ids(&fourth).contains(&x.1)));
    assert_same(&mut s, Q, "after new data");
    set_test_aggregations(None, None, false);
}

/// The background job keeps the aggregations a report change still selects as well.
#[test]
fn background_jobs_keep_aggregations_across_report_changes() {
    let (_, mut s) = small();
    set_aggregation_config(
        &s,
        AggConfig {
            sync: Some(false),
            ..AggConfig::default()
        },
    );
    set_test_aggregations(Some(true), Some(0), true);
    // waits for the background job of the last query's fact, then returns the report
    let settle = |s: &mut Session| -> Value {
        for _ in 0..60 {
            s.execute("dax.clear_cache", &json!({})).unwrap();
            evaluate(s, Q, true).unwrap();
            s.background_work();
            let a = aggs(s);
            if a["jobs"].as_u64() == Some(0) && a["hits"].as_u64().unwrap_or(0) > 0 {
                return a;
            }
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
        panic!("the background job never finished: {}", aggs(s));
    };
    let first = settle(&mut s);
    assert!(first["built_tables"].as_u64().unwrap_or(0) > 0, "{first}");
    s.execute("page.add", &json!({"display_name": "New"})).unwrap();
    s.execute(
        "visual.add",
        &json!({"page": "New", "visual_type": "tableEx", "x": 0, "y": 0, "width": 300, "height": 300,
        "fields": [{"role": "Values", "field": "Calendrier[Mois]"}, {"role": "Values", "field": "Magasin[Region]"},
                   {"role": "Values", "field": "[Qte]"}]}),
    )
    .unwrap();
    s.background_work();
    let second = settle(&mut s);
    assert!(second["reused_tables"].as_u64().unwrap_or(0) > 0, "{second}");
    set_test_aggregations(None, None, false);
}

/// Two evaluations of `q` at once on the shared session, standing together at a gate (one
/// holds the main state, the other forks): their answers.
fn at_once(s: &Session, q: &str, gate: &str) -> Vec<Result<Value, String>> {
    let g = super::super::test_gate::Gate::new(gate, 2);
    let text = format!("{q} // gate:{gate}");
    let out: Vec<Result<Value, String>> = std::thread::scope(|sc| {
        let hs: Vec<_> = (0..2)
            .map(|_| {
                let text = text.as_str();
                sc.spawn(move || {
                    // the session's setting, with the size caps of small test facts lifted
                    set_test_aggregations(None, None, true);
                    s.execute_shared(
                        "dax.evaluate",
                        &json!({"query": text, "max_rows": 1_000_000, "timeout_ms": 600_000}),
                        None,
                    )
                    .map(|mut v| {
                        if let Some(o) = v.as_object_mut() {
                            o.remove("elapsed_ms");
                        }
                        v
                    })
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert!(!g.timed_out(), "the two evaluations did not run at the same time");
    super::super::test_gate::remove(&g);
    out
}

/// An evaluation running beside the session's answers from the aggregations the session built
/// (the same answers, its hits counted with the session's), and never from aggregations of
/// an older model: after a measure change both answer from the new one.
#[test]
fn forks_answer_from_the_built_aggregations() {
    let (_, mut s) = small();
    // the session's own setting: the threads below see it (the test overrides are per thread)
    set_aggregation_config(
        &s,
        AggConfig {
            enabled: Some(true),
            min_rows: Some(0),
            sync: Some(false),
        },
    );
    let reference = assert_same(&mut s, Q, "first");
    set_test_aggregations(None, None, true);
    // one evaluation alone: the hits it counts
    // (the storage engine's result cache answers a repeated query without the aggregations:
    // emptied before each measured evaluation)
    let clear = |s: &mut Session| s.execute("dax.clear_cache", &json!({})).unwrap();
    let h0 = hits(&mut s);
    clear(&mut s);
    assert_eq!(evaluate(&mut s, Q, true).unwrap(), reference);
    let h1 = hits(&mut s);
    let one = h1 - h0;
    assert!(one > 0, "the query is answered from an aggregation");
    // two at once, then one more (it takes the fork's counters over)
    let forks = s.preview.fork_counters().0;
    clear(&mut s);
    for r in at_once(&s, Q, "aggs1") {
        assert_eq!(r.unwrap(), reference);
    }
    assert_eq!(s.preview.fork_counters().0, forks + 1, "one ran on a fork");
    // the next evaluation takes the fork's counters over (at once, or from the waiting list)
    assert_eq!(evaluate(&mut s, Q, true).unwrap(), reference);
    let h2 = hits(&mut s);
    // the main state's evaluation counted `one`; the fork's too (the last one may come from the
    // result cache)
    assert!(
        h2 - h1 >= 2 * one,
        "the fork answered from the aggregations too: {} < {}",
        h2 - h1,
        2 * one
    );
    // a model change: the aggregations of the old model answer nowhere
    s.execute(
        "measure.set",
        &json!({"name": "CA", "expression": "SUM(Ventes[Quantite])"}),
    )
    .unwrap();
    set_test_aggregations(Some(false), Some(0), true);
    let fact = evaluate(&mut s, Q, true).unwrap();
    set_test_aggregations(None, None, true);
    assert_ne!(fact, reference);
    // built again for the new model, then two at once
    s.execute("dax.aggregations", &json!({"build": true})).unwrap();
    assert_eq!(evaluate(&mut s, Q, true).unwrap(), fact);
    for r in at_once(&s, Q, "aggs2") {
        assert_eq!(r.unwrap(), fact, "after the change");
    }
    set_aggregation_config(&s, AggConfig::default());
    set_test_aggregations(None, None, false);
}
