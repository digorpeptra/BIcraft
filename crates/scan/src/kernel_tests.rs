//! The grouped-sum kernel (`sum_by_id_multi`): randomized differential tests against a naive
//! loop, and its micro-bench on the shapes of the engine bench's grouped scans (Q03, Q04, Q05,
//! a filtered one): `cargo test --release -p bicraft-scan kernel_micro -- --ignored --nocapture`.

#![allow(clippy::indexing_slicing, clippy::needless_range_loop)]
use std::time::Instant;

use super::*;

fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// How [`pack_as`] stores a segment.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Layout {
    /// Bit-packed.
    Plain,
    /// Hybrid RLE (runs of 4 equal ids and longer, the rest bit-packed).
    Rle,
    /// Alternately bit-packed and hybrid RLE.
    Mixed,
}

/// `codes` as VertiPaq-like segments of `seg` rows: data id = code + 3 (blank, code 0: id 2).
fn pack_as(codes: &[u32], seg: usize, layout: Layout) -> Packed {
    let id = |c: u32| if c == 0 { 2 } else { c + 3 };
    let width = |max: u32| {
        let bits = 32 - max.leading_zeros();
        [1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 16, 21, 32]
            .into_iter()
            .find(|w| u32::from(*w) >= bits.max(1))
            .unwrap_or(32)
    };
    let bits = |vals: &[u32]| {
        let base = vals.iter().copied().min().unwrap_or(0);
        let w = width(vals.iter().map(|v| v - base).max().unwrap_or(0));
        let per = 64 / usize::from(w);
        let words = vals
            .chunks(per)
            .map(|ch| {
                ch.iter().enumerate().fold(0u64, |acc, (i, v)| {
                    acc | (u64::from(v - base) << (i * usize::from(w)))
                })
            })
            .collect();
        Ids::Bits {
            width: w,
            base,
            words,
        }
    };
    let mut segs = Vec::new();
    for (k, chunk) in codes.chunks(seg.max(1)).enumerate() {
        let ids: Vec<u32> = chunk.iter().map(|c| id(*c)).collect();
        let (min_id, max_id) = (*ids.iter().min().unwrap(), *ids.iter().max().unwrap());
        let rle = match layout {
            Layout::Plain => false,
            Layout::Rle => true,
            Layout::Mixed => k % 2 == 1,
        };
        let data = if !rle {
            SegData::Plain(bits(&ids))
        } else {
            let mut runs: Vec<Run> = Vec::new();
            let mut sub = Vec::new();
            let mut i = 0;
            while i < ids.len() {
                let mut j = i;
                while j < ids.len() && ids[j] == ids[i] {
                    j += 1;
                }
                if j - i >= 4 {
                    runs.push(Run {
                        start: i as u32,
                        len: (j - i) as u32,
                        id: ids[i],
                        packed: false,
                    });
                } else {
                    match runs.last_mut() {
                        Some(r) if r.packed && r.start + r.len == i as u32 => r.len += (j - i) as u32,
                        _ => runs.push(Run {
                            start: i as u32,
                            len: (j - i) as u32,
                            id: sub.len() as u32,
                            packed: true,
                        }),
                    }
                    sub.extend_from_slice(&ids[i..j]);
                }
                i = j;
            }
            if sub.is_empty() {
                sub.push(0);
            }
            SegData::Rle {
                runs,
                sub: bits(&sub),
            }
        };
        segs.push(Segment {
            rows: chunk.len(),
            data,
            min_id,
            max_id,
        });
    }
    let max = codes.iter().copied().max().unwrap_or(0);
    let mut lut = vec![0u32; (max + 4) as usize];
    for c in 1..=max {
        lut[(c + 3) as usize] = c;
    }
    Packed::new(segs, lut)
}

/// `codes` as hybrid RLE segments of `seg` rows (for the other test modules).
pub(crate) fn pack_rle(codes: &[u32], seg: usize) -> Packed {
    pack_as(codes, seg, Layout::Rle)
}

/// Per group, the naive sum and count of `codes` (`vals` read by data id) over the selected rows.
fn naive(
    n: usize,
    selected: &dyn Fn(usize) -> bool,
    group: &dyn Fn(usize) -> usize,
    groups: usize,
    codes: &[u32],
    vals: IdValues<'_>,
) -> GroupSums {
    let mut sums = vec![0i128; groups];
    let mut counts = vec![0u64; groups];
    for i in 0..n {
        if !selected(i) {
            continue;
        }
        let id = if codes[i] == 0 { 2 } else { codes[i] + 3 };
        let g = group(i);
        match vals {
            IdValues::Affine { first } => {
                if id >= first {
                    sums[g] += i128::from(id);
                    counts[g] += 1;
                }
            }
            IdValues::Table { values, valid } => {
                if valid.get(id as usize).copied().unwrap_or(false) {
                    sums[g] += i128::from(values.get(id as usize).copied().unwrap_or(0));
                    counts[g] += 1;
                }
            }
        }
    }
    (sums, counts)
}

/// The interleaved kernel against a naive loop over random shapes: segment sizes that do and do
/// not align with blocks, bit-packed / RLE / mixed segments, value columns with and without blank
/// ids (whole segments without any, so the count-once path runs, and segments with a few), runs
/// of one id across blocks, value tables whose invalid ids are only the blank ids (count-once) or
/// also others, one key / two keys / a key in runs / a key with few or many groups (one or four
/// accumulator copies), with and without a filter (dropped rows to the dummy slot), 1 and 4
/// threads. Deterministic by seed (`BICRAFT_KERNEL_CASES` for more cases).
#[test]
fn kernel_matches_a_naive_loop_on_random_shapes() {
    let cases: u64 = bicraft_env::var("BICRAFT_KERNEL_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(64);
    for case in 0..cases {
        let r = |k: u64| splitmix(case.wrapping_mul(1_000_003).wrapping_add(k));
        let n = 3_000 + (r(1) % 40_000) as usize;
        let seg = [2048usize, 4096, 5000, 8192, 65536][(r(2) % 5) as usize];
        let layout = [Layout::Plain, Layout::Rle, Layout::Mixed][(r(3) % 3) as usize];
        // groups: few (copies) or many (one copy)
        let card = [3usize, 10, 51, 5000][(r(4) % 4) as usize];
        let key_codes: Vec<u32> = (0..n)
            .map(|i| 1 + (splitmix(i as u64 ^ r(5)) % card as u64) as u32)
            .collect();
        // a column in long runs (a second key, or a value)
        let runs: Vec<u32> = (0..n)
            .map(|i| 1 + ((i / (500 + (r(6) % 4000) as usize)) % 7) as u32)
            .collect();
        // values: blanks in some segments only, or nowhere
        let blanks_every = [0u64, 7, 1000][(r(7) % 3) as usize];
        let dirty_seg = (r(8) % 3) as usize;
        let vals: Vec<u32> = (0..n)
            .map(|i| {
                let x = splitmix(i as u64 ^ r(9));
                if blanks_every > 0 && (i / seg) % 3 == dirty_seg && x.is_multiple_of(blanks_every) {
                    0
                } else {
                    1 + (x % 200) as u32
                }
            })
            .collect();
        let pk = pack_as(&key_codes, seg, layout);
        let pr = pack_as(&runs, seg, layout);
        let pv = pack_as(&vals, seg, layout);
        let max_id = 210usize;
        let small: Vec<i64> = (0..max_id as i64).map(|i| i * 13 - 900).collect();
        let wide: Vec<i64> = (0..max_id as i64).map(|i| (i - 100) * (1 << 52)).collect();
        // invalid ids: the blank ids only, or others too
        let clean: Vec<bool> = (0..max_id).map(|i| i >= 3).collect();
        let holes: Vec<bool> = (0..max_id).map(|i| i >= 3 && i % 11 != 5).collect();
        let lut: Vec<u32> = (0..=card as u32).map(|c| c % 4).collect();
        let key_shape = r(10) % 4;
        type Group<'k> = Box<dyn Fn(usize) -> usize + 'k>;
        let (key, group): (Key<'_>, Group<'_>) = match key_shape {
            0 => (
                Key::new(vec![KeyPart {
                    codes: Col::Packed(&pk),
                    lut: None,
                    card: card + 1,
                }])
                .unwrap(),
                Box::new(|i| key_codes[i] as usize),
            ),
            1 => (
                Key::new(vec![KeyPart {
                    codes: Col::Packed(&pk),
                    lut: Some(&lut),
                    card: 4,
                }])
                .unwrap(),
                Box::new(|i| lut[key_codes[i] as usize] as usize),
            ),
            2 => (
                Key::new(vec![
                    KeyPart {
                        codes: Col::Packed(&pr),
                        lut: None,
                        card: 8,
                    },
                    KeyPart {
                        codes: Col::Packed(&pk),
                        lut: Some(&lut),
                        card: 4,
                    },
                ])
                .unwrap(),
                Box::new(|i| runs[i] as usize * 4 + lut[key_codes[i] as usize] as usize),
            ),
            _ => (
                Key::new(vec![KeyPart {
                    codes: Col::Packed(&pr),
                    lut: None,
                    card: 8,
                }])
                .unwrap(),
                Box::new(|i| runs[i] as usize),
            ),
        };
        let allowed: Vec<bool> = (0..=card).map(|c| c % 3 != 1).collect();
        let mask: Vec<bool> = (0..n)
            .map(|i| !splitmix(i as u64 ^ r(11)).is_multiple_of(4))
            .collect();
        let filter = r(12) % 3;
        let sel = Selection {
            rows: n,
            preds: if filter >= 1 {
                vec![Pred {
                    codes: Col::Packed(&pk),
                    allowed: &allowed,
                }]
            } else {
                Vec::new()
            },
            masks: if filter == 2 { vec![&mask[..]] } else { Vec::new() },
            none: false,
        };
        let selected = |i: usize| (filter < 1 || allowed[key_codes[i] as usize]) && (filter < 2 || mask[i]);
        let spec: Vec<(&Packed, &[u32], IdValues<'_>)> = vec![
            (&pv, &vals, IdValues::Affine { first: 3 }),
            (&pr, &runs, IdValues::Affine { first: 3 }),
            (
                &pv,
                &vals,
                IdValues::Table {
                    values: &small,
                    valid: &clean,
                },
            ),
            (
                &pv,
                &vals,
                IdValues::Table {
                    values: &wide,
                    valid: &holes,
                },
            ),
            (
                &pr,
                &runs,
                IdValues::Table {
                    values: &small,
                    valid: &holes,
                },
            ),
        ];
        let cols: Vec<(&Packed, IdValues<'_>)> = spec.iter().map(|(p, _, v)| (*p, *v)).collect();
        let groups = key.groups();
        let want: Vec<GroupSums> = spec
            .iter()
            .map(|(_, codes, v)| naive(n, &selected, &group, groups, codes, *v))
            .collect();
        for threads in [1, 4] {
            let got = sum_by_id_multi(&sel, &key, &cols, threads).unwrap();
            for (k, (g, w)) in got.iter().zip(&want).enumerate() {
                assert_eq!(
                    g, w,
                    "case {case}: n {n}, seg {seg}, {layout:?}, card {card}, key {key_shape}, filter {filter}, \
                     blanks {blanks_every}, column {k}, {threads} threads"
                );
            }
        }
    }
}

/// Columns in long runs on their own (Q16: a SUMX of a quantity in RLE runs by a foreign key of
/// 1,001 groups): the run column that counts the rows in its own loop (no other column does),
/// two run columns (deltas or adds per row), a run column next to a bit-packed one, runs of
/// blanks, runs ending inside blocks, few and many groups, with and without a filter, 1 and 4
/// threads, against the naive loop.
#[test]
fn run_columns_alone_match_a_naive_loop() {
    for case in 0..24u64 {
        let r = |k: u64| splitmix(case.wrapping_mul(7_919).wrapping_add(k));
        let n = 20_000 + (r(1) % 30_000) as usize;
        let seg = [2048usize, 5000, 65536][(r(2) % 3) as usize];
        let card = [5usize, 1001][(case % 2) as usize];
        let key_codes: Vec<u32> = (0..n)
            .map(|i| 1 + (splitmix(i as u64 ^ r(3)) % card as u64) as u32)
            .collect();
        let run_len = 1500 + (r(4) % 5000) as usize;
        // runs of 1..=9, every fourth run blank when the case says so
        let blank_runs = case % 3 == 0;
        let qty: Vec<u32> = (0..n)
            .map(|i| {
                let k = i / run_len;
                if blank_runs && k % 4 == 3 {
                    0
                } else {
                    1 + (k % 9) as u32
                }
            })
            .collect();
        let qty2: Vec<u32> = (0..n)
            .map(|i| 1 + ((i / (run_len / 2 + 700)) % 5) as u32)
            .collect();
        let other: Vec<u32> = (0..n)
            .map(|i| 1 + (splitmix(i as u64 ^ r(5)) % 300) as u32)
            .collect();
        let (pk, pq, pq2, po) = (
            pack_as(&key_codes, seg, Layout::Plain),
            pack_as(&qty, seg, Layout::Rle),
            pack_as(&qty2, seg, Layout::Rle),
            pack_as(&other, seg, Layout::Plain),
        );
        let key = Key::new(vec![KeyPart {
            codes: Col::Packed(&pk),
            lut: None,
            card: card + 1,
        }])
        .unwrap();
        let groups = key.groups();
        let group = |i: usize| key_codes[i] as usize;
        let values: Vec<i64> = (0..320i64).map(|i| i * 7 - 50).collect();
        let valid: Vec<bool> = (0..320usize).map(|i| i >= 3 && i % 13 != 6).collect();
        let affine = IdValues::Affine { first: 3 };
        let table = IdValues::Table {
            values: &values,
            valid: &valid,
        };
        let allowed: Vec<bool> = (0..=card).map(|c| c % 3 != 1).collect();
        let filtered = case % 4 >= 2;
        let sel = Selection {
            rows: n,
            preds: if filtered {
                vec![Pred {
                    codes: Col::Packed(&pk),
                    allowed: &allowed,
                }]
            } else {
                Vec::new()
            },
            masks: Vec::new(),
            none: false,
        };
        let selected = |i: usize| !filtered || allowed[key_codes[i] as usize];
        let sets: Vec<Vec<(&Packed, &[u32], IdValues<'_>)>> = vec![
            vec![(&pq, &qty, affine)],
            vec![(&pq, &qty, table)],
            vec![(&pq, &qty, affine), (&pq2, &qty2, affine)],
            vec![(&pq, &qty, affine), (&pq2, &qty2, table), (&pq, &qty, table)],
            vec![(&pq, &qty, affine), (&po, &other, affine)],
            vec![(&po, &other, table), (&pq, &qty, affine)],
        ];
        for spec in &sets {
            let cols: Vec<(&Packed, IdValues<'_>)> = spec.iter().map(|(p, _, v)| (*p, *v)).collect();
            for threads in [1, 4] {
                let got = sum_by_id_multi(&sel, &key, &cols, threads).unwrap();
                for (k, (_, codes, v)) in spec.iter().enumerate() {
                    let want = naive(n, &selected, &group, groups, codes, *v);
                    assert_eq!(
                        got[k],
                        want,
                        "case {case}: n {n}, seg {seg}, card {card}, filtered {filtered}, column {k} of {}, \
                         {threads} threads",
                        spec.len()
                    );
                }
            }
        }
    }
}

/// When columns in one run take deltas: never for one run column by 1,001 groups whose rows no
/// other column counts (Q16: one pass that adds and counts), and for a run column next to a
/// counting one by 36 groups (Q04); never when the accumulators outnumber the rows.
#[test]
fn run_deltas_only_where_they_save_steps() {
    let q16 = Lanes::new(1001).unwrap().len();
    let q04 = Lanes::new(36).unwrap().len();
    assert!(!defer_runs(q16, BLOCK, 1, false));
    assert!(!defer_runs(q16, BLOCK, 1, true));
    assert!(!defer_runs(q16, BLOCK, 2, false));
    assert!(defer_runs(q04, BLOCK, 1, true));
    assert!(!defer_runs(q04, BLOCK, 1, false));
    assert!(defer_runs(q04, BLOCK, 2, false));
    assert!(!defer_runs(q04, BLOCK, 0, true));
    assert!(!defer_runs(q04, 100, 1, true));
    assert!(!defer_runs(Lanes::new(5000).unwrap().len(), BLOCK, 3, true));
    assert!(!defer_runs(usize::MAX, usize::MAX, usize::MAX, true));
}

/// Segment statistics: the ids from `first` on and below a limit, exactly (blank ids anywhere in
/// a segment, its largest id, blocks across two segments, past the end).
#[test]
fn ids_within_reads_the_statistics_exactly() {
    let mut codes: Vec<u32> = (0..10_000).map(|i| 1 + (i % 50) as u32).collect();
    codes[7_000] = 0; // one blank in the second segment (rows 4096..8192)
    codes[9_999] = 80; // the largest id in the last segment
    for layout in [Layout::Plain, Layout::Rle, Layout::Mixed] {
        let p = pack_as(&codes, 4096, layout);
        assert!(p.ids_within(0, 4096, 3, 54), "{layout:?}");
        assert!(!p.ids_within(0, 4096, 3, 53), "{layout:?}: id 53 is used");
        assert!(
            !p.ids_within(4000, 4200, 3, usize::MAX),
            "{layout:?}: a blank in the next segment"
        );
        assert!(
            p.ids_within(4000, 4200, 2, usize::MAX),
            "{layout:?}: blank id 2 allowed"
        );
        assert!(p.ids_within(8192, 9000, 3, 84), "{layout:?}");
        assert!(
            !p.ids_within(8192, 9000, 3, 83),
            "{layout:?}: id 83 in the segment"
        );
        assert!(
            !p.ids_within(9000, 20_000, 3, usize::MAX),
            "{layout:?}: past the end"
        );
        assert!(!p.ids_within(0, 100, 4, usize::MAX), "{layout:?}: first above 3");
    }
}

/// A key whose lookup sends codes past its groups, a value column past its table, an empty
/// selection, one group with a filter: no panic, and the naive loop's answer.
#[test]
fn kernel_edges() {
    let n = 9_000usize;
    let key_codes: Vec<u32> = (0..n).map(|i| (i % 13) as u32).collect();
    let vals: Vec<u32> = (0..n).map(|i| 1 + (i % 300) as u32).collect();
    let (pk, pv) = (
        pack_as(&key_codes, 4096, Layout::Plain),
        pack_as(&vals, 4096, Layout::Mixed),
    );
    // a value table shorter than the ids used: ids past it count as blanks
    let values: Vec<i64> = (0..100).collect();
    let valid: Vec<bool> = (0..100).map(|i| i >= 3).collect();
    let mask: Vec<bool> = (0..n).map(|i| i % 3 == 0).collect();
    let sel = Selection {
        rows: n,
        preds: Vec::new(),
        masks: vec![&mask[..]],
        none: false,
    };
    for key in [
        Key::single(),
        Key::new(vec![KeyPart {
            codes: Col::Packed(&pk),
            lut: None,
            card: 5,
        }])
        .unwrap(),
    ] {
        let groups = key.groups();
        let group = |i: usize| {
            if groups == 1 {
                0
            } else {
                (key_codes[i] as usize).min(4)
            }
        };
        for v in [
            IdValues::Affine { first: 3 },
            IdValues::Table {
                values: &values,
                valid: &valid,
            },
        ] {
            let got = sum_by_id_multi(&sel, &key, &[(&pv, v)], 4).unwrap();
            assert_eq!(got[0], naive(n, &|i| mask[i], &group, groups, &vals, v));
            let none = Selection {
                none: true,
                ..sel.clone()
            };
            let got = sum_by_id_multi(&none, &key, &[(&pv, v)], 4).unwrap();
            assert_eq!(got[0], (vec![0; groups], vec![0; groups]));
        }
    }
}

/// Throughput of the grouped-sum kernel on the engine bench's shapes, 1 and all threads, best of
/// 5 (`BICRAFT_MICRO_ROWS`, default 10 M; 2^20-row segments like VertiPaq's):
/// - Q03: one value-encoded column (21-bit ids) by a 10-bit foreign key through a lookup to
///   10 groups;
/// - Q04: two columns (one value-encoded, one in long RLE runs) by a foreign key to 36 groups;
/// - Q05: one column by two keys (51 × 10 groups);
/// - Q16-like: a column in long RLE runs by a foreign key of 1,001 groups (a SUMX of quantity
///   times a product attribute);
/// - Q08-like: Q03 filtered on a column at ~67 % selectivity;
/// - table: Q03 with a hash-encoded column (60,000 values) read through a table (Currency-like
///   sums in an i64).
#[test]
#[ignore]
fn kernel_micro() {
    let n: usize = bicraft_env::var("BICRAFT_MICRO_ROWS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10_000_000);
    let seg = 1 << 20;
    let t0 = Instant::now();
    let fk: Vec<u32> = (0..n).map(|i| 1 + (splitmix(i as u64) % 1000) as u32).collect();
    let date: Vec<u32> = (0..n)
        .map(|i| 1 + (splitmix(i as u64 ^ 0x51) % 1096) as u32)
        .collect();
    let store: Vec<u32> = (0..n)
        .map(|i| 1 + (splitmix(i as u64 ^ 0x77) % 50) as u32)
        .collect();
    let amount: Vec<u32> = (0..n)
        .map(|i| 1 + ((splitmix(i as u64 ^ 0x9E37) >> 7) % ((1 << 21) - 8)) as u32)
        .collect();
    let qty: Vec<u32> = (0..n).map(|i| 1 + ((i / 3000) % 10) as u32).collect();
    // a hash-encoded amount: 60,000 distinct values read through a table
    let price: Vec<u32> = (0..n)
        .map(|i| 1 + (splitmix(i as u64 ^ 0x3C) % 60_000) as u32)
        .collect();
    let (pfk, pdate, pstore, pamount, pqty, pprice) = (
        pack_as(&fk, seg, Layout::Plain),
        pack_as(&date, seg, Layout::Plain),
        pack_as(&store, seg, Layout::Plain),
        pack_as(&amount, seg, Layout::Plain),
        pack_as(&qty, seg, Layout::Rle),
        pack_as(&price, seg, Layout::Plain),
    );
    drop((fk, date, store, amount, qty, price));
    println!("rows {n}, built in {:?}", t0.elapsed());
    let cat_lut: Vec<u32> = (0..1001u32).map(|c| c % 10).collect();
    let month_lut: Vec<u32> = (0..1097u32).map(|c| (c / 31) % 36).collect();
    let allowed_qty: Vec<bool> = (0..11).map(|c| c % 3 != 0).collect();
    let values: Vec<i64> = (0..60_004i64).map(|i| i * 100 - 7).collect();
    let valid: Vec<bool> = (0..60_004usize).map(|i| i >= 3).collect();
    let by_cat = Key::new(vec![KeyPart {
        codes: Col::Packed(&pfk),
        lut: Some(&cat_lut),
        card: 10,
    }])
    .unwrap();
    let by_product = Key::new(vec![KeyPart {
        codes: Col::Packed(&pfk),
        lut: None,
        card: 1001,
    }])
    .unwrap();
    let by_month = Key::new(vec![KeyPart {
        codes: Col::Packed(&pdate),
        lut: Some(&month_lut),
        card: 36,
    }])
    .unwrap();
    let by_store_cat = Key::new(vec![
        KeyPart {
            codes: Col::Packed(&pstore),
            lut: None,
            card: 51,
        },
        KeyPart {
            codes: Col::Packed(&pfk),
            lut: Some(&cat_lut),
            card: 10,
        },
    ])
    .unwrap();
    let all = Selection::all(n);
    let filtered = Selection {
        rows: n,
        preds: vec![Pred {
            codes: Col::Packed(&pqty),
            allowed: &allowed_qty,
        }],
        masks: Vec::new(),
        none: false,
    };
    let affine = IdValues::Affine { first: 3 };
    let table = IdValues::Table {
        values: &values,
        valid: &valid,
    };
    type Case<'c> = (
        &'c str,
        &'c Selection<'c>,
        &'c Key<'c>,
        Vec<(&'c Packed, IdValues<'c>)>,
    );
    let cases: Vec<Case<'_>> = vec![
        ("Q03 sum by category", &all, &by_cat, vec![(&pamount, affine)]),
        (
            "Q04 2 sums by month (1 in runs)",
            &all,
            &by_month,
            vec![(&pamount, affine), (&pqty, affine)],
        ),
        (
            "Q05 sum by store x category",
            &all,
            &by_store_cat,
            vec![(&pamount, affine)],
        ),
        (
            "Q08-like filtered, by category",
            &filtered,
            &by_cat,
            vec![(&pamount, affine)],
        ),
        ("table values by category", &all, &by_cat, vec![(&pprice, table)]),
        (
            "Q16-like run column by product",
            &all,
            &by_product,
            vec![(&pqty, affine)],
        ),
    ];
    let max = configured_threads();
    let time = |f: &mut dyn FnMut()| {
        f();
        let mut best = f64::MAX;
        // at least 15 runs and 0.3 s: the cores parked by a single-threaded case wake up
        let start = Instant::now();
        let mut runs = 0;
        while runs < 15 || start.elapsed().as_secs_f64() < 0.3 {
            let t = Instant::now();
            f();
            best = best.min(t.elapsed().as_secs_f64());
            runs += 1;
        }
        best
    };
    // warm-up: the pool's workers and the clock
    for (_, sel, key, cols) in &cases {
        for _ in 0..5 {
            let _ = sum_by_id_multi(sel, key, cols, max);
        }
    }
    for t in [1usize, max] {
        for (name, sel, key, cols) in &cases {
            let s = time(&mut || {
                let _ = sum_by_id_multi(sel, key, cols, t);
            });
            println!(
                "threads {t:>2}  {name:<34} {:>8.2} ms  {:>7.0} M rows/s",
                s * 1e3,
                n as f64 / s / 1e6
            );
        }
    }
}

/// Grouped scans through [`scan_groups`], [`scan`] and [`mask`] on the shapes the
/// column-store techniques target (`docs/storage-engine.md`, "Column-store techniques"),
/// median and best of the runs (at least 7 and 1 s per case), all threads
/// (`BICRAFT_MICRO_ROWS`, default 10 M; `BICRAFT_MICRO_ONLY=radix,merge,preds,skip` for a
/// subset):
/// - radix: one group per customer (rows / 10 groups, as Q17), first row and integer sums;
/// - merge: per-worker states of 100 k and 900 k groups (counts, under the 8 MiB radix limit);
/// - preds: two predicates, the less selective first (a 90 % one, then a 10 % one);
/// - skip: a filter on one store of a fact sorted by quantity then store within each 1 Mi-row
///   segment (as VertiPaq orders a segment), and a filter on a customer range (ids > 65,536).
#[test]
#[ignore]
fn groups_micro() {
    let n: usize = bicraft_env::var("BICRAFT_MICRO_ROWS")
        .ok()
        .and_then(|v| v.replace('_', "").parse().ok())
        .unwrap_or(10_000_000);
    let only = bicraft_env::var("BICRAFT_MICRO_ONLY").unwrap_or_default();
    let want = |k: &str| only.is_empty() || only.split(',').any(|o| o == k);
    let seg = 1 << 20;
    let customers = (n / 10).max(1);
    let t0 = Instant::now();
    let cust: Vec<u32> = (0..n)
        .map(|i| 1 + (splitmix(i as u64 ^ 0xC0) % customers as u64) as u32)
        .collect();
    let amount: Vec<u32> = (0..n)
        .map(|i| 1 + (splitmix(i as u64 ^ 0x9E37) % 5000) as u32)
        .collect();
    let store: Vec<u32> = (0..n)
        .map(|i| 1 + (splitmix(i as u64 ^ 0x77) % 200) as u32)
        .collect();
    let qty: Vec<u32> = (0..n)
        .map(|i| 1 + (splitmix(i as u64 ^ 0x51) % 10) as u32)
        .collect();
    // sorted by quantity then store within each segment
    let mut sorted: Vec<(u32, u32)> = qty.iter().copied().zip(store.iter().copied()).collect();
    for ch in sorted.chunks_mut(seg) {
        ch.sort_unstable();
    }
    let s_store: Vec<u32> = sorted.iter().map(|p| p.1).collect();
    let s_qty: Vec<u32> = sorted.iter().map(|p| p.0).collect();
    drop(sorted);
    // customers in row order (a fact loaded by customer)
    let s_cust: Vec<u32> = (0..n).map(|i| 1 + (i / 10) as u32).collect();
    // the columns live as long as the test: leaked, so the keys borrow them for 'static
    let leak = |p: Packed| -> &'static Packed { Box::leak(Box::new(p)) };
    let pcust = leak(pack_as(&cust, seg, Layout::Plain));
    let pamount = leak(pack_as(&amount, seg, Layout::Plain));
    let pstore = leak(pack_as(&store, seg, Layout::Plain));
    let pqty = leak(pack_as(&qty, seg, Layout::Plain));
    let psstore = leak(pack_as(&s_store, seg, Layout::Mixed));
    let pscust = leak(pack_as(&s_cust, seg, Layout::Plain));
    let psqty = leak(pack_as(&s_qty, seg, Layout::Rle));
    // the stores of the sorted rows, bit-packed (no runs)
    let psstore_bits = leak(pack_as(&s_store, seg, Layout::Plain));
    drop((cust, amount, store, qty, s_store, s_cust, s_qty));
    println!(
        "rows {n}, {customers} customers, built in {:?}, {} threads",
        t0.elapsed(),
        configured_threads()
    );
    let per_code: Vec<Option<i64>> = (0..5004i64).map(|v| (v > 0).then_some(v * 3)).collect();
    let time = |name: &str, f: &mut dyn FnMut() -> usize| {
        let check = f();
        let mut ts = Vec::new();
        let start = Instant::now();
        while ts.len() < 7 || start.elapsed().as_secs_f64() < 1.0 {
            let t = Instant::now();
            std::hint::black_box(f());
            ts.push(t.elapsed().as_secs_f64() * 1e3);
        }
        ts.sort_by(f64::total_cmp);
        println!(
            "{name:<44} median {:>9.2} ms  best {:>9.2} ms  ({} runs, check {check})",
            ts[ts.len() / 2],
            ts[0],
            ts.len()
        );
    };
    let all = Selection::all(n);
    let key_of = |codes: &'static Packed, card: usize| {
        Key::new(vec![KeyPart {
            codes: Col::Packed(codes),
            lut: None,
            card,
        }])
        .unwrap()
    };
    if want("radix") {
        let by_cust = key_of(pcust, customers + 1);
        time("radix: first row by customer", &mut || {
            let f = scan_groups(&all, &by_cust, None, 4, FirstRow::new).unwrap();
            f.0.iter().filter(|r| **r != u32::MAX).count()
        });
        time("radix: integer sums by customer", &mut || {
            let s = scan_groups(&all, &by_cust, Some(Col::Packed(pamount)), 24, |g| {
                IntSums::new(g, &per_code)
            })
            .unwrap();
            s.counts.iter().filter(|c| **c > 0).count()
        });
    }
    if want("merge") {
        for g in [100_000usize, 900_000] {
            // the customer codes folded to `g` groups
            let lut: Vec<u32> = (0..=customers as u32 + 1).map(|c| c % g as u32).collect();
            let lut: &'static [u32] = Box::leak(lut.into_boxed_slice());
            let key = Key::new(vec![KeyPart {
                codes: Col::Packed(pcust),
                lut: Some(lut),
                card: g,
            }])
            .unwrap();
            time(&format!("merge: counts by {g} groups"), &mut || {
                let c = scan_groups(&all, &key, None, 8, Counts::new).unwrap();
                c.0.iter().filter(|x| **x > 0).count()
            });
        }
    }
    if want("preds") {
        let wide: Vec<bool> = (0..202).map(|c| c % 10 != 0).collect();
        let narrow: Vec<bool> = (0..12).map(|c| c == 4).collect();
        let sel = Selection {
            rows: n,
            preds: vec![
                Pred {
                    codes: Col::Packed(pstore),
                    allowed: &wide,
                },
                Pred {
                    codes: Col::Packed(pqty),
                    allowed: &narrow,
                },
            ],
            masks: Vec::new(),
            none: false,
        };
        let by_store = key_of(pstore, 201);
        time("preds: 90 % then 10 %, counts by store", &mut || {
            let c = scan(&sel, &by_store, None, configured_threads(), || Counts::new(201)).unwrap();
            c.0.iter().sum::<u64>() as usize
        });
        time("preds: 90 % then 10 %, mask", &mut || {
            mask(&sel, configured_threads()).iter().filter(|x| **x).count()
        });
        // the same predicates on sorted segments: the quantity lies in long runs
        let sel = Selection {
            rows: n,
            preds: vec![
                Pred {
                    codes: Col::Packed(psstore_bits),
                    allowed: &wide,
                },
                Pred {
                    codes: Col::Packed(psqty),
                    allowed: &narrow,
                },
            ],
            masks: Vec::new(),
            none: false,
        };
        time("preds: 90 % then 10 % (in runs), sorted, count", &mut || {
            let c = scan(&sel, &Key::single(), None, configured_threads(), || {
                Counts::new(1)
            })
            .unwrap();
            c.0[0] as usize
        });
    }
    if want("skip") {
        let one: Vec<bool> = (0..202).map(|c| c == 17).collect();
        let sel = Selection {
            rows: n,
            preds: vec![Pred {
                codes: Col::Packed(psstore),
                allowed: &one,
            }],
            masks: Vec::new(),
            none: false,
        };
        time("skip: one store, sorted segments, count", &mut || {
            let c = scan(&sel, &Key::single(), None, configured_threads(), || {
                Counts::new(1)
            })
            .unwrap();
            c.0[0] as usize
        });
        // a range of 1 % of the customers in the middle
        let (lo, hi) = (customers / 2, customers / 2 + customers / 100);
        let range: Vec<bool> = (0..=customers + 1).map(|c| (lo..hi).contains(&c)).collect();
        let sel = Selection {
            rows: n,
            preds: vec![Pred {
                codes: Col::Packed(pscust),
                allowed: &range,
            }],
            masks: Vec::new(),
            none: false,
        };
        time("skip: customer range, ordered fact, count", &mut || {
            let c = scan(&sel, &Key::single(), None, configured_threads(), || {
                Counts::new(1)
            })
            .unwrap();
            c.0[0] as usize
        });
    }
}
