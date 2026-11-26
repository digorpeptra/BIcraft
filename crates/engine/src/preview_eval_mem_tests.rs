//! One query's formula-engine memory under a cap (`docs/beyond-ram.md`, lot 4): answers under a
//! cap are the answers without one (a `TOPN` over a large grouping and an `ORDER BY` cut by
//! `max_rows` selected block by block), and a query over its cap is refused with an error naming
//! the part and the cap, after which the session answers as before.

use serde_json::{Value, json};

use super::storage_tests::{evaluate, same, star_with};
use crate::Session;

fn query_budget(s: &mut Session, b: Value) -> Value {
    s.execute("dax.memory", &json!({ "query_budget": b }))
        .unwrap_or_else(|e| panic!("dax.memory: {e}"))
}

fn evaluate_rows(s: &mut Session, q: &str, max_rows: u64) -> Result<Value, String> {
    s.execute(
        "dax.evaluate",
        &json!({"query": q, "max_rows": max_rows, "timeout_ms": 600_000}),
    )
    .map(|mut v| {
        if let Some(o) = v.as_object_mut() {
            o.remove("elapsed_ms");
        }
        v
    })
}

fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// `TOPN` over groupings by customer: ties, directions, several keys, `n` around the row count,
/// filters, `IGNORE`, Desktop's variable form.
fn topn_queries() -> Vec<String> {
    let sc = "SUMMARIZECOLUMNS(Ventes[ClientKey], \"CA\", [CA])";
    let ties = "SUMMARIZECOLUMNS(Ventes[ClientKey], \"M\", MAX(Ventes[Quantite]), \"CA\", [CA])";
    let order = "ORDER BY [CA] DESC, Ventes[ClientKey]";
    let mut q = vec![
        format!("EVALUATE TOPN(501, {sc}, [CA], DESC, Ventes[ClientKey], ASC) {order}"),
        format!("EVALUATE TOPN(7, {sc}, [CA], ASC, Ventes[ClientKey], DESC) ORDER BY [CA], Ventes[ClientKey] DESC"),
        format!("EVALUATE TOPN(3, {sc}, Ventes[ClientKey], DESC) ORDER BY Ventes[ClientKey]"),
        format!("EVALUATE TOPN(5, {ties}, [M]) ORDER BY [M] DESC, Ventes[ClientKey]"),
        format!("EVALUATE TOPN(5, {ties}, [M], ASC) ORDER BY [M], Ventes[ClientKey]"),
        format!("EVALUATE TOPN(9, {ties}, [M], DESC, [CA], DESC, Ventes[ClientKey], ASC) ORDER BY [M] DESC, [CA] DESC"),
        format!("EVALUATE TOPN(1000000, {sc}, [CA], DESC) {order}"),
        format!("EVALUATE TOPN(2.7, {sc}, [CA], DESC) {order}"),
        format!(
            "EVALUATE TOPN(6, SUMMARIZECOLUMNS(Ventes[ClientKey], TREATAS({{2022}}, Calendrier[Annee]), \"CA\", [CA]), \
             [CA], DESC) {order}"
        ),
        "EVALUATE TOPN(8, SUMMARIZECOLUMNS(Ventes[ClientKey], \"Q\", IGNORE(SUM(Ventes[Quantite])), \"CA\", [CA]), \
         [Q], ASC, Ventes[ClientKey], ASC) ORDER BY [Q], Ventes[ClientKey]"
            .to_string(),
        "EVALUATE TOPN(8, SUMMARIZECOLUMNS(Ventes[ClientKey], \"Q\", SUM(Ventes[Quantite]), \"B\", COUNTBLANK(Ventes[Montant])), \
         [B], ASC, Ventes[ClientKey], ASC) ORDER BY [B], Ventes[ClientKey]"
            .to_string(),
        "DEFINE VAR __DS0Core = SUMMARIZECOLUMNS(Ventes[ClientKey], \"CA\", [CA]) \
         VAR __DS0PrimaryWindowed = TOPN(501, __DS0Core, [CA], 0, Ventes[ClientKey], 1) \
         EVALUATE __DS0PrimaryWindowed ORDER BY [CA] DESC, Ventes[ClientKey]"
            .to_string(),
    ];
    // random sizes and directions over groups with many ties
    let mut seed = 11u64;
    for case in 0..24u64 {
        seed = splitmix(seed ^ case);
        let n = [1u64, 2, 10, 100, 999, 1000, 1001, 2999, 3000, 3001][(seed % 10) as usize];
        let d1 = if (seed >> 8).is_multiple_of(2) {
            "DESC"
        } else {
            "ASC"
        };
        let d2 = if (seed >> 9).is_multiple_of(2) {
            "DESC"
        } else {
            "ASC"
        };
        let item = [
            "MAX(Ventes[Quantite])",
            "[CA]",
            "COUNTROWS(Ventes)",
            "MIN(Ventes[Date])",
        ][((seed >> 12) % 4) as usize];
        q.push(format!(
            "EVALUATE TOPN({n}, SUMMARIZECOLUMNS(Ventes[ClientKey], \"K\", {item}, \"CA\", [CA]), [K], {d1}, Ventes[ClientKey], {d2}) \
             ORDER BY [K] {d1}, Ventes[ClientKey] {d2}"
        ));
    }
    q
}

#[test]
fn topn_over_a_grouping_under_small_caps_gives_the_same_rows() {
    let blocks0 = super::topn::GROUPING_BLOCKS.with(std::cell::Cell::get);
    let mut failures = Vec::new();
    let mut answered_small = 0usize;
    for (dirty, packed) in [(false, true), (true, false)] {
        let mut s = star_with(30_000, dirty, packed);
        let queries = topn_queries();
        let reference: Vec<Result<Value, String>> =
            queries.iter().map(|q| evaluate(&mut s, q, true)).collect();
        for cap in ["24K", "48K", "96K", "192K", "1M", "64M"] {
            query_budget(&mut s, json!(cap));
            s.execute("dax.clear_cache", &json!({})).unwrap();
            for (q, r) in queries.iter().zip(&reference) {
                let got = evaluate(&mut s, q, true);
                match (r, &got) {
                    (Ok(a), Ok(b)) if same(a, b) => {
                        if cap != "64M" {
                            answered_small += 1;
                        }
                    }
                    (Ok(_), Err(e)) if e.contains("query's cap") && cap != "64M" => {}
                    (Err(a), Err(b)) if a == b => {}
                    _ => failures.push(format!(
                        "cap {cap}, dirty {dirty}, packed {packed}: {q}\n  {r:?}\n  {got:?}"
                    )),
                }
            }
            let m = query_budget(&mut s, json!(cap));
            assert!(m["query"]["peak_bytes"].as_u64().unwrap() > 0, "{m}");
        }
        query_budget(&mut s, Value::Null);
        // back without a cap: the same answers
        for (q, r) in queries.iter().zip(&reference) {
            let got = evaluate(&mut s, q, true);
            assert!(matches!((r, &got), (Ok(a), Ok(b)) if same(a, b)), "{q}: {got:?}");
        }
    }
    assert!(
        failures.is_empty(),
        "{} differences:\n{}",
        failures.len(),
        failures.join("\n")
    );
    let blocks = super::topn::GROUPING_BLOCKS.with(std::cell::Cell::get) - blocks0;
    assert!(blocks > 20, "the block path ran {blocks} times");
    assert!(
        answered_small > 40,
        "{answered_small} answers under the small caps"
    );
}

/// Groupings by ranges (`docs/beyond-ram.md`): a `TOPN` or an `ORDER BY` cut by `max_rows` over a
/// grouping whose scans' states do not fit is answered a range of groups at a time with the rows
/// of the whole path: forced ranges of 1 to more than every group (blank customers, filters
/// leaving ranges empty, every kind of state: sums, counts, blanks, distinct bitmaps, ranks,
/// histograms), and the ranges the room gives under small caps.
#[test]
fn topn_over_a_grouping_by_ranges_gives_the_same_rows() {
    let ranges0 = super::topn::GROUPING_RANGES.with(std::cell::Cell::get);
    let set_len = |l: Option<usize>| super::topn::TEST_RANGE_LEN.with(|c| c.set(l));
    let mut failures = Vec::new();
    let mut extra = vec![
        "EVALUATE TOPN(5, SUMMARIZECOLUMNS(Ventes[ClientKey], \"D\", DISTINCTCOUNT(Ventes[ProduitKey]), \"CA\", [CA]), \
         [D], DESC, Ventes[ClientKey], ASC) ORDER BY [D] DESC, Ventes[ClientKey]"
            .to_string(),
        "EVALUATE TOPN(4, SUMMARIZECOLUMNS(Ventes[ClientKey], \"Med\", MEDIAN(Ventes[Quantite]), \"CA\", [CA]), \
         [Med], DESC, [CA], ASC, Ventes[ClientKey], ASC) ORDER BY [Med] DESC, [CA], Ventes[ClientKey]"
            .to_string(),
        "EVALUATE TOPN(6, SUMMARIZECOLUMNS(Ventes[ClientKey], Magasin[Region], \"CA\", [CA]), [CA], DESC, \
         Ventes[ClientKey], ASC, Magasin[Region], ASC) ORDER BY [CA] DESC, Ventes[ClientKey], Magasin[Region]"
            .to_string(),
        "EVALUATE TOPN(3, SUMMARIZECOLUMNS(Ventes[ClientKey], FILTER(ALL(Ventes[ClientKey]), Ventes[ClientKey] > 2900), \
         \"CA\", [CA]), [CA], ASC) ORDER BY [CA], Ventes[ClientKey]"
            .to_string(),
    ];
    let mut queries = topn_queries();
    queries.append(&mut extra);
    for (dirty, packed) in [(false, true), (true, false)] {
        let mut s = star_with(30_000, dirty, packed);
        let reference: Vec<Result<Value, String>> =
            queries.iter().map(|q| evaluate(&mut s, q, true)).collect();
        let orders: Vec<(&str, u64, Value)> = [
            "EVALUATE SUMMARIZECOLUMNS(Ventes[ClientKey], \"CA\", [CA]) ORDER BY [CA] DESC, Ventes[ClientKey]",
            "EVALUATE SUMMARIZECOLUMNS(Ventes[ClientKey], \"M\", MAX(Ventes[Quantite]), \"CA\", [CA]) ORDER BY [M], [CA] DESC",
        ]
        .into_iter()
        .flat_map(|q| [1u64, 10, 2999].map(|m| (q, m)))
        .map(|(q, m)| (q, m, evaluate_rows(&mut s, q, m).unwrap()))
        .collect();
        // forced ranges (the cap only turns the block path on)
        query_budget(&mut s, json!("64M"));
        for len in [Some(700usize), Some(3001), None] {
            set_len(len);
            for cap in ["64M", "96K", "48K"] {
                if len.is_some() && cap != "64M" {
                    continue;
                }
                query_budget(&mut s, json!(cap));
                s.execute("dax.clear_cache", &json!({})).unwrap();
                for (q, r) in queries.iter().zip(&reference) {
                    let got = evaluate(&mut s, q, true);
                    match (r, &got) {
                        (Ok(a), Ok(b)) if same(a, b) => {}
                        (Ok(_), Err(e)) if e.contains("query's cap") && len.is_none() && cap != "64M" => {}
                        (Err(a), Err(b)) if a == b => {}
                        _ => failures.push(format!(
                            "len {len:?} cap {cap} dirty {dirty} packed {packed}: {q}\n  {r:?}\n  {got:?}"
                        )),
                    }
                }
                for (q, m, r) in &orders {
                    match evaluate_rows(&mut s, q, *m) {
                        Ok(got) if same(r, &got) => {}
                        Err(e) if e.contains("query's cap") && len.is_none() && cap != "64M" => {}
                        got => failures.push(format!("len {len:?} cap {cap}: {q} max_rows {m}: {got:?}")),
                    }
                }
            }
        }
        set_len(None);
        query_budget(&mut s, Value::Null);
    }
    // ranges of one group each, on a smaller star (200 customers)
    let mut s = star_with(2_000, true, true);
    let small: Vec<String> = topn_queries().into_iter().take(6).collect();
    let reference: Vec<Result<Value, String>> = small.iter().map(|q| evaluate(&mut s, q, true)).collect();
    query_budget(&mut s, json!("64M"));
    set_len(Some(1));
    for (q, r) in small.iter().zip(&reference) {
        let got = evaluate(&mut s, q, true);
        if !matches!((r, &got), (Ok(a), Ok(b)) if same(a, b)) {
            failures.push(format!("len 1: {q}\n  {r:?}\n  {got:?}"));
        }
    }
    set_len(None);
    assert!(
        failures.is_empty(),
        "{} differences:\n{}",
        failures.len(),
        failures.join("\n")
    );
    let ranges = super::topn::GROUPING_RANGES.with(std::cell::Cell::get) - ranges0;
    assert!(ranges > 500, "{ranges} ranges scanned");
}

/// Under a cap below the grouped scan's states, the ranges the room gives answer what lot 4
/// refused: a `TOPN` by customer equals the uncapped answer, and only a cap too small for one
/// range of one group refuses it.
#[test]
fn a_grouping_whose_states_do_not_fit_is_answered_by_ranges() {
    let q = "EVALUATE TOPN(501, SUMMARIZECOLUMNS(Ventes[ClientKey], \"CA\", [CA]), [CA], DESC, Ventes[ClientKey], ASC) \
             ORDER BY [CA] DESC, Ventes[ClientKey]";
    // 30,000 customers: their sums take 720 KB (twice while joined)
    let mut s = star_with(300_000, false, true);
    let reference = evaluate(&mut s, q, true).unwrap();
    let ranges0 = super::topn::GROUPING_RANGES.with(std::cell::Cell::get);
    let mut answered = 0;
    for cap in ["1M", "768K", "512K", "384K"] {
        query_budget(&mut s, json!(cap));
        s.execute("dax.clear_cache", &json!({})).unwrap();
        match evaluate(&mut s, q, true) {
            Ok(got) => {
                assert!(same(&reference, &got), "cap {cap}");
                answered += 1;
            }
            Err(e) => {
                eprintln!("cap {cap}: {e}");
                assert!(e.contains("query's cap"), "cap {cap}: {e}");
            }
        }
    }
    let ranges = super::topn::GROUPING_RANGES.with(std::cell::Cell::get) - ranges0;
    assert!(
        answered >= 2 && ranges >= 4,
        "{answered} answered, {ranges} ranges"
    );
    // a few bytes: refused cleanly, then the session answers
    query_budget(&mut s, json!("2K"));
    let e = evaluate(&mut s, q, true).unwrap_err();
    assert!(e.contains("query's cap"), "{e}");
    query_budget(&mut s, Value::Null);
    assert!(same(&reference, &evaluate(&mut s, q, true).unwrap()));
}

#[test]
fn an_order_by_cut_by_max_rows_is_selected_under_a_cap_with_the_same_answer() {
    let mut s = star_with(30_000, true, true);
    let queries = [
        "EVALUATE SUMMARIZECOLUMNS(Ventes[ClientKey], \"CA\", [CA]) ORDER BY [CA] DESC, Ventes[ClientKey]",
        "EVALUATE SUMMARIZECOLUMNS(Ventes[ClientKey], \"CA\", [CA]) ORDER BY Ventes[ClientKey] DESC",
        "EVALUATE SUMMARIZECOLUMNS(Ventes[ClientKey], \"M\", MAX(Ventes[Quantite]), \"CA\", [CA]) ORDER BY [M], [CA] DESC",
        "EVALUATE SUMMARIZECOLUMNS(Ventes[ClientKey], \"M\", MAX(Ventes[Quantite])) ORDER BY [M] DESC",
        "DEFINE VAR __DS0Core = SUMMARIZECOLUMNS(Ventes[ClientKey], \"CA\", [CA]) EVALUATE __DS0Core ORDER BY [CA]",
    ];
    for max_rows in [1u64, 10, 2999, 1_000_000] {
        for q in queries {
            query_budget(&mut s, Value::Null);
            let reference = evaluate_rows(&mut s, q, max_rows).unwrap();
            for cap in ["64K", "256K", "4M"] {
                query_budget(&mut s, json!(cap));
                s.execute("dax.clear_cache", &json!({})).unwrap();
                // every short cut answers once the grouping's scan fits; every answer once its rows do
                let must = cap == "4M" || (cap == "256K" && max_rows <= 10);
                match evaluate_rows(&mut s, q, max_rows) {
                    Ok(got) => assert!(
                        same(&reference, &got),
                        "{q} max_rows {max_rows} cap {cap}:\n{reference}\n{got}"
                    ),
                    Err(e) => assert!(
                        e.contains("query's cap") && !must,
                        "{q} max_rows {max_rows} cap {cap}: {e}"
                    ),
                }
            }
        }
    }
}

#[test]
fn a_query_over_its_cap_is_refused_and_the_session_goes_on() {
    super::mem::TEST_MIN_HELD.with(|m| m.set(Some(0)));
    let mut s = star_with(20_000, false, true);
    let queries = [
        "EVALUATE FILTER(Ventes, Ventes[Quantite] > 2)",
        "EVALUATE ADDCOLUMNS(VALUES(Ventes[ClientKey]), \"x\", [CA])",
        "EVALUATE SUMMARIZECOLUMNS(Ventes[ClientKey], \"CA\", [CA]) ORDER BY Ventes[ClientKey]",
        "EVALUATE ROW(\"n\", COUNTROWS(FILTER(Ventes, Ventes[Quantite] > 2 && Ventes[Montant] > 10)))",
    ];
    let reference: Vec<Value> = queries
        .iter()
        .map(|q| evaluate_rows(&mut s, q, 1_000_000).unwrap())
        .collect();
    let m = query_budget(&mut s, json!("16K"));
    assert_eq!(m["query"]["cap"], 16 << 10);
    assert_eq!(m["query"]["cap_source"], "session");
    let mut refused = 0;
    for (q, r) in queries.iter().zip(&reference) {
        match evaluate_rows(&mut s, q, 1_000_000) {
            Ok(v) => assert!(same(r, &v), "{q}"),
            Err(e) => {
                assert!(e.contains("query's cap of 16384 bytes"), "{e}");
                assert!(e.contains(" needs "), "{e}");
                refused += 1;
            }
        }
    }
    assert!(refused >= 3, "{refused} refused");
    let m = query_budget(&mut s, json!("16K"));
    assert!(m["query"]["refused"].as_u64().unwrap() >= 3, "{m}");
    // a cap that holds them: the same answers, the peak within the cap
    let m = query_budget(&mut s, json!("64M"));
    assert_eq!(m["query"]["cap"], 64 << 20);
    for (q, r) in queries.iter().zip(&reference) {
        let v = evaluate_rows(&mut s, q, 1_000_000).unwrap();
        assert!(same(r, &v), "{q}");
        let m = query_budget(&mut s, json!("64M"));
        let peak = m["query"]["last_peak_bytes"].as_u64().unwrap();
        assert!(peak <= 64 << 20, "{m}");
    }
    // no cap: nothing is counted
    let m = query_budget(&mut s, json!(0));
    assert!(m["query"]["cap"].is_null(), "{m}");
    assert_eq!(m["query"]["cap_source"], "none");
    for (q, r) in queries.iter().zip(&reference) {
        let v = evaluate_rows(&mut s, q, 1_000_000).unwrap();
        assert!(same(r, &v), "{q}");
    }
    super::mem::TEST_MIN_HELD.with(|m| m.set(None));
}

#[test]
fn bad_query_budgets_are_refused() {
    let mut s = star_with(2_000, false, true);
    for bad in [json!(-1), json!("lots"), json!([1]), json!(1.5)] {
        assert!(
            s.execute("dax.memory", &json!({ "query_budget": bad })).is_err(),
            "{bad}"
        );
    }
    let m = query_budget(&mut s, json!("2M"));
    assert_eq!(m["query"]["cap"], 2 << 20);
    let m = query_budget(&mut s, Value::Null);
    assert_eq!(m["query"]["cap_source"], "none");
}
