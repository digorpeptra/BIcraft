//! `TOPN`'s fast paths (`preview_eval_topn.rs`) against references: the typed first key and
//! the candidate bound against a full stable sort, and the grouping path (storage engine on)
//! against the generic evaluator (off) on ties, blanks, directions, several keys, `n` above the
//! row count, `n` = 0, text keys, filters and Desktop's `VAR __DS0Core` form.

use std::cmp::Ordering;

use serde_json::Value;

use super::V;
use super::storage_tests::{evaluate, same, star_with};
use super::tables::sort_cmp;
use super::topn::{TopKey, bound_with, top_rows};

fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// `TOPN`'s rows by definition: a stable sort on every key, the `take` first, then every row
/// tying with the last of them.
fn naive(rows: &[Vec<V>], take: usize, desc: &[bool]) -> Vec<usize> {
    let cmp = |a: usize, b: usize| -> Ordering {
        for (k, d) in desc.iter().enumerate() {
            let o = sort_cmp(&rows[a][k], &rows[b][k]);
            let o = if *d { o.reverse() } else { o };
            if o != Ordering::Equal {
                return o;
            }
        }
        Ordering::Equal
    };
    let mut order: Vec<usize> = (0..rows.len()).collect();
    order.sort_by(|a, b| cmp(*a, *b));
    if take == 0 || order.is_empty() {
        return Vec::new();
    }
    let last = order[take.min(order.len()) - 1];
    order
        .into_iter()
        .filter(|i| cmp(*i, last) != Ordering::Greater)
        .collect()
}

/// The fast selection as `top_n_table` runs it: the typed first key when it reads, its
/// candidates, then the exact selection.
fn fast(rows: &[Vec<V>], take: usize, desc: &[bool]) -> Vec<usize> {
    let len = rows.len();
    let take = take.min(len);
    let full = |a: usize, b: usize| -> Ordering {
        for (k, d) in desc.iter().enumerate() {
            let o = sort_cmp(&rows[a][k], &rows[b][k]);
            let o = if *d { o.reverse() } else { o };
            if o != Ordering::Equal {
                return o;
            }
        }
        Ordering::Equal
    };
    match take
        .checked_sub(1)
        .and_then(|_| TopKey::read(len, |i| &rows[i][0]))
    {
        Some(k0) => {
            let cands = k0.candidates(take, desc[0]);
            let cmp = |a: usize, b: usize| -> Ordering {
                let o = k0.cmp_rows(a, b);
                let o = if desc[0] { o.reverse() } else { o };
                o.then_with(|| {
                    for (k, d) in desc.iter().enumerate().skip(1) {
                        let o = sort_cmp(&rows[a][k], &rows[b][k]);
                        let o = if *d { o.reverse() } else { o };
                        if o != Ordering::Equal {
                            return o;
                        }
                    }
                    Ordering::Equal
                })
            };
            top_rows(cands, take, &cmp)
        }
        None => top_rows((0..len).collect(), take, &full),
    }
}

fn value(kind: u64, r: u64) -> V {
    // few distinct values, so ties are common; blanks in every kind
    let x = r % 7;
    if x == 0 {
        return V::Blank;
    }
    match kind {
        0 => V::Num(x as f64 - 3.5),
        1 => V::Cur((x as i64 - 3) * 12_345),
        2 => V::Bool(x.is_multiple_of(2)),
        3 => V::Text(["b", "A", "a", "c", "B", "d"][(x as usize) % 6].to_string()),
        4 => V::Date(45_000.0 + x as f64),
        // numbers and Currency mixed
        _ => {
            if x.is_multiple_of(2) {
                V::Num(x as f64 / 2.0)
            } else {
                V::Cur(x as i64 * 5_000)
            }
        }
    }
}

#[test]
fn topn_fast_selection_matches_a_full_sort() {
    let mut seed = 7u64;
    for case in 0..600u64 {
        seed = splitmix(seed ^ case);
        let len = (seed % 40) as usize;
        let nkeys = 1 + (seed >> 8) as usize % 3;
        let kinds: Vec<u64> = (0..nkeys).map(|k| splitmix(seed ^ (k as u64 + 11)) % 6).collect();
        let desc: Vec<bool> = (0..nkeys)
            .map(|k| splitmix(seed ^ (k as u64 + 99)).is_multiple_of(2))
            .collect();
        let rows: Vec<Vec<V>> = (0..len)
            .map(|i| {
                kinds
                    .iter()
                    .enumerate()
                    .map(|(k, kind)| value(*kind, splitmix(seed ^ ((i * 31 + k) as u64))))
                    .collect()
            })
            .collect();
        for take in [0usize, 1, 2, 3, 5, len.saturating_sub(1), len, len + 3] {
            assert_eq!(
                fast(&rows, take, &desc),
                naive(&rows, take, &desc),
                "case {case}, kinds {kinds:?}, desc {desc:?}, take {take}, rows {rows:?}"
            );
        }
    }
}

/// The candidate bound on any number of threads (top-k buffers with a threshold, then a filter
/// by share) gives the positions a full selection gives: ties at the bound, few or many
/// distinct values, `take` around a share's length, both directions.
#[test]
fn topn_bound_on_threads_matches_one_full_selection() {
    for (case, (len, distinct)) in [
        (1usize, 1u64),
        (2, 2),
        (7, 3),
        (100, 5),
        (1000, 1000),
        (5003, 40),
        (20_000, 7),
    ]
    .into_iter()
    .enumerate()
    {
        let v: Vec<(u8, i64)> = (0..len)
            .map(|i| {
                let r = splitmix(i as u64 ^ (case as u64) << 32);
                (u8::from(!r.is_multiple_of(11)), (r >> 8) as i64 % distinct as i64)
            })
            .collect();
        for desc in [false, true] {
            let ord = |x: &(u8, i64), y: &(u8, i64)| if desc { y.cmp(x) } else { x.cmp(y) };
            for take in [
                0usize,
                1,
                2,
                3,
                17,
                len / 3,
                len / 2,
                len.saturating_sub(2),
                len.saturating_sub(1),
                len,
                len + 1,
            ] {
                let want = take
                    .checked_sub(1)
                    .filter(|at| *at < len.saturating_sub(1))
                    .map(|at| {
                        let mut s = v.clone();
                        s.select_nth_unstable_by(at, ord);
                        let thr = s[at];
                        (0..len)
                            .filter(|i| ord(&v[*i], &thr) != Ordering::Greater)
                            .collect::<Vec<_>>()
                    });
                for threads in [1usize, 2, 3, 8, 64] {
                    let got = bound_with(&v, take, desc, &|x: &(u8, i64), y: &(u8, i64)| x.cmp(y), threads);
                    assert_eq!(
                        got, want,
                        "len {len}, distinct {distinct}, desc {desc}, take {take}, threads {threads}"
                    );
                }
            }
        }
    }
}

/// A long first key is read by shares on several threads: the key of every row in order, and
/// `None` when only a late share holds text or NaN, or booleans in one share meet numbers in
/// another.
#[test]
fn topn_long_key_read_by_shares_is_the_whole_key() {
    let len = 600_000usize;
    let nums: Vec<V> = (0..len)
        .map(|i| {
            if i.is_multiple_of(13) {
                V::Blank
            } else {
                V::Num(i as f64 / 7.0)
            }
        })
        .collect();
    match TopKey::read(len, |i| &nums[i]) {
        Some(TopKey::Float(v)) => {
            assert_eq!(v.len(), len);
            for (i, x) in v.iter().enumerate() {
                let want = if i.is_multiple_of(13) {
                    (0, 0.0)
                } else {
                    (1, i as f64 / 7.0)
                };
                assert_eq!(*x, want, "row {i}");
            }
        }
        _ => panic!("numbers read as a float key"),
    }
    let curs: Vec<V> = (0..len).map(|i| V::Cur(i as i64 * 3 - 5)).collect();
    match TopKey::read(len, |i| &curs[i]) {
        Some(TopKey::Int(v)) => assert!(v.iter().enumerate().all(|(i, x)| *x == (1, i as i64 * 3 - 5))),
        _ => panic!("Currency read as an exact key"),
    }
    for (last, what) in [(V::Text("x".into()), "text"), (V::Num(f64::NAN), "NaN")] {
        let mut v = nums.clone();
        v[len - 1] = last;
        assert!(TopKey::read(len, |i| &v[i]).is_none(), "{what} in the last share");
    }
    let mut mixed: Vec<V> = (0..len).map(|i| V::Bool(i.is_multiple_of(2))).collect();
    mixed[len - 1] = V::Num(1.0);
    assert!(TopKey::read(len, |i| &mixed[i]).is_none(), "booleans and numbers");
}

/// Currency compares exactly on the typed key, even where doubles would merge two values.
#[test]
fn topn_typed_currency_key_is_exact() {
    let big = 9_007_199_254_740_993i64; // 2^53 + 1
    let rows = vec![vec![V::Cur(big - 1)], vec![V::Cur(big)], vec![V::Blank]];
    assert_eq!(fast(&rows, 1, &[true]), vec![1]);
    assert_eq!(fast(&rows, 1, &[false]), vec![2]);
    assert_eq!(fast(&rows, 2, &[false]), vec![2, 0]);
}

/// TOPN shapes over a grouping, Desktop's included.
fn topn_queries() -> Vec<String> {
    let sc = "SUMMARIZECOLUMNS(Ventes[ClientKey], \"CA\", [CA])";
    let ties = "SUMMARIZECOLUMNS(Ventes[ClientKey], \"M\", MAX(Ventes[Quantite]), \"CA\", [CA])";
    let order = "ORDER BY [CA] DESC, Ventes[ClientKey]";
    vec![
        format!("EVALUATE TOPN(501, {sc}, [CA], DESC, Ventes[ClientKey], ASC) {order}"),
        format!("EVALUATE TOPN(7, {sc}, [CA], ASC, Ventes[ClientKey], DESC) ORDER BY [CA], Ventes[ClientKey] DESC"),
        format!("EVALUATE TOPN(7, {sc}, [CA]) {order}"),
        format!("EVALUATE TOPN(7, {sc}, [CA], 1) ORDER BY [CA], Ventes[ClientKey]"),
        format!("EVALUATE TOPN(3, {sc}, Ventes[ClientKey], DESC) ORDER BY Ventes[ClientKey]"),
        format!("EVALUATE TOPN(3, {sc}, Ventes[ClientKey], ASC) ORDER BY Ventes[ClientKey]"),
        // ties at the boundary: every customer with the top quantity
        format!("EVALUATE TOPN(5, {ties}, [M]) ORDER BY [M] DESC, Ventes[ClientKey]"),
        format!("EVALUATE TOPN(5, {ties}, [M], ASC) ORDER BY [M], Ventes[ClientKey]"),
        format!("EVALUATE TOPN(5, {ties}, [M], DESC, [CA], ASC) ORDER BY [M] DESC, [CA]"),
        format!("EVALUATE TOPN(9, {ties}, [M], DESC, [CA], DESC, Ventes[ClientKey], ASC) ORDER BY [M] DESC, [CA] DESC"),
        // n above the row count, n = 0, fractional and negative n
        format!("EVALUATE TOPN(1000000, {sc}, [CA], DESC) {order}"),
        format!("EVALUATE TOPN(0, {sc}, [CA], DESC) {order}"),
        format!("EVALUATE TOPN(2.7, {sc}, [CA], DESC) {order}"),
        format!("EVALUATE TOPN(-1, {sc}, [CA], DESC) {order}"),
        // text keys (group-by columns), several groups
        "EVALUATE TOPN(4, SUMMARIZECOLUMNS(Produit[Produit], \"CA\", [CA]), Produit[Produit], DESC) \
         ORDER BY Produit[Produit]"
            .to_string(),
        "EVALUATE TOPN(4, SUMMARIZECOLUMNS(Produit[Categorie], Magasin[Region], \"CA\", [CA], \"Q\", [Qte]), \
         Magasin[Region], ASC, [Q], DESC) ORDER BY Magasin[Region], [Q] DESC"
            .to_string(),
        "EVALUATE TOPN(3, SUMMARIZECOLUMNS(Produit[Categorie], \"CA\", [CA]), [CA], DESC) ORDER BY [CA] DESC".to_string(),
        // filters: a filter table, CALCULATETABLE around it, an empty result
        format!(
            "EVALUATE TOPN(6, SUMMARIZECOLUMNS(Ventes[ClientKey], TREATAS({{2022}}, Calendrier[Annee]), \"CA\", [CA]), \
             [CA], DESC) {order}"
        ),
        format!("EVALUATE CALCULATETABLE(TOPN(6, {sc}, [CA], DESC), Produit[Categorie] = \"Cat03\") {order}"),
        format!("EVALUATE TOPN(6, CALCULATETABLE({sc}, Ventes[Quantite] > 99), [CA], DESC) {order}"),
        // blanks: an item blank for some groups, IGNORE, the blank customer of dirty data
        "EVALUATE TOPN(8, SUMMARIZECOLUMNS(Ventes[ClientKey], \"Q\", SUM(Ventes[Quantite]), \"B\", COUNTBLANK(Ventes[Montant])), \
         [B], ASC, Ventes[ClientKey], ASC) ORDER BY [B], Ventes[ClientKey]"
            .to_string(),
        "EVALUATE TOPN(8, SUMMARIZECOLUMNS(Ventes[ClientKey], \"Q\", IGNORE(SUM(Ventes[Quantite])), \"CA\", [CA]), \
         [Q], ASC, Ventes[ClientKey], ASC) ORDER BY [Q], Ventes[ClientKey]"
            .to_string(),
        // Desktop's visual form: the grouping in a variable
        "DEFINE VAR __DS0Core = SUMMARIZECOLUMNS(Ventes[ClientKey], \"CA\", [CA]) \
         VAR __DS0PrimaryWindowed = TOPN(501, __DS0Core, [CA], 0, Ventes[ClientKey], 1) \
         EVALUATE __DS0PrimaryWindowed ORDER BY [CA] DESC, Ventes[ClientKey]"
            .to_string(),
        "DEFINE VAR __DS0Core = SUMMARIZECOLUMNS(Ventes[ClientKey], \"CA\", [CA]) \
         EVALUATE TOPN(5, __DS0Core, [CA], 0) ORDER BY [CA] DESC \
         EVALUATE ROW(\"n\", COUNTROWS(__DS0Core)) \
         EVALUATE TOPN(5, __DS0Core, [CA], 1) ORDER BY [CA]"
            .to_string(),
        "DEFINE VAR __DS0Core = SUMMARIZECOLUMNS(Produit[Categorie], \"CA\", [CA]) \
         EVALUATE ADDCOLUMNS(__DS0Core, \"x\", [CA] * 2) ORDER BY Produit[Categorie]"
            .to_string(),
        // shapes the grouping path leaves to the generic one
        format!("EVALUATE TOPN(5, {sc}, [CA] * -1, DESC) ORDER BY [CA]"),
        "EVALUATE TOPN(5, SUMMARIZECOLUMNS(ROLLUPADDISSUBTOTAL(Produit[Categorie], \"T\"), \"CA\", [CA]), [CA], DESC) \
         ORDER BY [CA] DESC"
            .to_string(),
    ]
}

fn rows_of(v: &Value) -> usize {
    v["results"]
        .as_array()
        .and_then(|r| r.first())
        .and_then(|r| r["rows"].as_array())
        .map_or(0, Vec::len)
}

#[test]
fn topn_over_a_grouping_matches_the_generic_evaluator() {
    let n0 = super::topn::GROUPING_TOPS.with(std::cell::Cell::get);
    for dirty in [false, true] {
        for packed in [true, false] {
            let mut s = star_with(3_000, dirty, packed);
            let mut failures = Vec::new();
            for q in topn_queries() {
                let reference = evaluate(&mut s, &q, false);
                // twice: the second run reads the caches the first filled
                for run in 0..2 {
                    let got = evaluate(&mut s, &q, true);
                    match (&reference, &got) {
                        (Ok(r), Ok(g)) if same(r, g) => {}
                        (Err(a), Err(b)) if a == b => {}
                        _ => failures.push(format!(
                            "{q} (run {run}):\n  reference {reference:?}\n  storage {got:?}"
                        )),
                    }
                }
            }
            assert!(
                failures.is_empty(),
                "dirty {dirty}, packed {packed}: {} differences:\n{}",
                failures.len(),
                failures.join("\n")
            );
        }
    }
    // most shapes took the grouping path (4 data sets, 2 runs each)
    let n = super::topn::GROUPING_TOPS.with(std::cell::Cell::get) - n0;
    assert!(n >= 4 * 2 * 15, "{n}");
}

/// The variable form gives the inline form's rows, and ties at the boundary keep more than `n`
/// rows.
#[test]
fn topn_variable_form_and_ties() {
    let mut s = star_with(3_000, true, true);
    let fused = || super::topn::GROUPING_TOPS.with(std::cell::Cell::get);
    let n0 = fused();
    let inline = evaluate(
        &mut s,
        "EVALUATE TOPN(501, SUMMARIZECOLUMNS(Ventes[ClientKey], \"CA\", [CA]), [CA], 0, Ventes[ClientKey], 1) \
         ORDER BY [CA] DESC, Ventes[ClientKey]",
        true,
    )
    .unwrap();
    let var = evaluate(
        &mut s,
        "DEFINE VAR __DS0Core = SUMMARIZECOLUMNS(Ventes[ClientKey], \"CA\", [CA]) \
         EVALUATE TOPN(501, __DS0Core, [CA], 0, Ventes[ClientKey], 1) ORDER BY [CA] DESC, Ventes[ClientKey]",
        true,
    )
    .unwrap();
    assert!(same(&inline, &var), "{inline}\n{var}");
    // both forms took the grouping path
    assert_eq!(fused(), n0 + 2);
    // 300 customers and the blank one of dirty data
    assert_eq!(rows_of(&inline), 301);
    let ties = evaluate(
        &mut s,
        "EVALUATE TOPN(2, SUMMARIZECOLUMNS(Ventes[ClientKey], \"M\", MAX(Ventes[Quantite])), [M]) ORDER BY [M] DESC",
        true,
    )
    .unwrap();
    assert!(rows_of(&ties) > 2, "{ties}");
}
