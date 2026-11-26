//! Kernel tests against naive loops, and the kernels' micro-bench (`docs/storage-engine.md`):
//! `cargo test --release -p bicraft-scan micro -- --ignored --nocapture`.

#![allow(clippy::indexing_slicing, clippy::needless_range_loop)]
use std::time::Instant;

use super::*;

fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// `n` rows: (fk code 0..=1000, value code 0..=50, high-cardinality code 0..n/10).
fn data(n: usize) -> (Vec<u32>, Vec<u32>, Vec<u32>) {
    let mut fk = Vec::with_capacity(n);
    let mut val = Vec::with_capacity(n);
    let mut hi = Vec::with_capacity(n);
    for i in 0..n as u64 {
        let r = splitmix(i);
        fk.push((r % 1001) as u32);
        val.push(((r >> 20) % 51) as u32);
        hi.push(((r >> 40) % (n as u64 / 10).max(1)) as u32);
    }
    (fk, val, hi)
}

/// `codes` kept as VertiPaq-like segments of `seg` rows: data id = code + 3 (blank: 2), every
/// other segment bit-packed, the others hybrid RLE (runs of 4 equal codes and longer, the rest
/// bit-packed in the sub-segment).
pub(super) fn pack(codes: &[u32], seg: usize) -> Packed {
    let id = |c: u32| if c == 0 { 2 } else { c + 3 };
    let width = |max: u32| {
        let bits = 32 - max.leading_zeros();
        [1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 16, 21, 32]
            .into_iter()
            .find(|w| u32::from(*w) >= bits.max(1))
            .unwrap_or(32)
    };
    let bits = |vals: &[u32], base: u32| {
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
        let data = if k % 2 == 0 {
            SegData::Plain(bits(&ids, 2))
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
            SegData::Rle {
                runs,
                sub: bits(&sub, 2),
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

#[test]
fn packed_columns_read_as_their_codes() {
    // runs and noise
    let codes: Vec<u32> = (0..10_000u32)
        .map(|i| {
            if (i / 50) % 3 == 0 {
                (i / 50) % 7
            } else {
                splitmix(u64::from(i)) as u32 % 40
            }
        })
        .collect();
    for seg in [1, 7, 100, 4096, 20_000] {
        let p = pack(&codes, seg);
        assert_eq!(p.rows(), codes.len());
        for (r, c) in codes.iter().enumerate() {
            assert_eq!(p.get(r), *c, "seg {seg} row {r}");
        }
        for start in [0usize, 1, 49, 2047, 5000, 9990] {
            let mut buf = Vec::new();
            let len = 3000.min(codes.len() - start);
            assert_eq!(
                Col::Packed(&p).block(start, len, &mut buf),
                &codes[start..start + len]
            );
        }
        let mut want = vec![0u64; 41];
        for c in &codes {
            want[*c as usize] += 1;
        }
        assert_eq!(p.counts(41), want);
    }
}

#[test]
fn grouped_scans_match_naive_loops() {
    for n in [0usize, 1, 5_000, 300_000] {
        for packed in [false, true] {
            grouped(n, packed);
        }
    }
}

fn grouped(n: usize, packed: bool) {
    {
        let (fk, val, hi) = data(n);
        let (pfk, pval, phi) = (pack(&fk, 70_000), pack(&val, 65_536), pack(&hi, 100_003));
        let (cfk, cval, chi) = if packed {
            (Col::Packed(&pfk), Col::Packed(&pval), Col::Packed(&phi))
        } else {
            (Col::Plain(&fk), Col::Plain(&val), Col::Plain(&hi))
        };
        // lut: fk code → one of 10 categories (0: blank)
        let lut: Vec<u32> = (0..1001u32)
            .map(|c| if c == 0 { 0 } else { 1 + c % 10 })
            .collect();
        let allowed: Vec<bool> = (0..51).map(|c| c % 3 != 0).collect();
        let mask: Vec<bool> = (0..n).map(|i| i % 7 != 0).collect();
        let sel = Selection {
            rows: n,
            preds: vec![Pred {
                codes: cval,
                allowed: &allowed,
            }],
            masks: vec![&mask],
            none: false,
        };
        let key = Key::new(vec![KeyPart {
            codes: cfk,
            lut: Some(&lut),
            card: 11,
        }])
        .unwrap();
        let per_code: Vec<Option<i64>> = (0..51).map(|c| (c > 0).then_some(c * 3)).collect();
        for threads in [1, 4] {
            let counts = scan(&sel, &key, Some(cval), threads, || Counts::new(11)).unwrap();
            let sums = scan(&sel, &key, Some(cval), threads, || IntSums::new(11, &per_code)).unwrap();
            let distinct = scan(&sel, &key, Some(cval), threads, || {
                Distinct::new(11, 51, 1 << 20).unwrap()
            })
            .unwrap();
            let mut c = vec![0u64; 11];
            let mut s = vec![0i128; 11];
            let mut d = vec![std::collections::HashSet::new(); 11];
            for i in 0..n {
                if !allowed[val[i] as usize] || !mask[i] {
                    continue;
                }
                let g = lut[fk[i] as usize] as usize;
                c[g] += 1;
                if let Some(x) = per_code[val[i] as usize] {
                    s[g] += i128::from(x);
                }
                d[g].insert(val[i]);
            }
            assert_eq!(counts.0, c, "n {n} threads {threads}");
            assert_eq!(sums.sums, s);
            for g in 0..11 {
                assert_eq!(distinct.count(g).0, d[g].len() as u64);
            }
            let m = super::mask(&sel, threads);
            let naive: Vec<bool> = (0..n).map(|i| allowed[val[i] as usize] && mask[i]).collect();
            assert_eq!(m, naive);
        }
        // two parts, high cardinality
        let key2 = Key::new(vec![
            KeyPart {
                codes: cval,
                lut: None,
                card: 51,
            },
            KeyPart {
                codes: chi,
                lut: None,
                card: (n / 10).max(1),
            },
        ])
        .unwrap();
        let groups = key2.groups();
        let got = scan(&Selection::all(n), &key2, None, 4, || Counts::new(groups)).unwrap();
        let mut want = vec![0u64; groups];
        for i in 0..n {
            want[key2.id(&[val[i], hi[i]]).unwrap()] += 1;
        }
        assert_eq!(got.0, want);
        // the radix-partitioned path (1.5 M groups at 300 k rows) gives the same state
        let per_code2: Vec<Option<i64>> = (0..51).map(|c| Some(c * 7 - 20)).collect();
        let a = scan_groups(&Selection::all(n), &key2, Some(cval), 24, |len| {
            IntSums::new(len, &per_code2)
        })
        .unwrap();
        let b = scan(&Selection::all(n), &key2, Some(cval), 1, || {
            IntSums::new(groups, &per_code2)
        })
        .unwrap();
        assert_eq!(a.sums, b.sums);
        assert_eq!(a.counts, b.counts);
        let a = scan_groups(&Selection::all(n), &key2, None, 4, FirstRow::new).unwrap();
        let b = scan(&Selection::all(n), &key2, None, 1, || FirstRow::new(groups)).unwrap();
        assert_eq!(a.0, b.0);
        // routed by windows of rows (lot 4): the same states, whatever the window
        for max_routed in [1usize, 300_000, 3_000_000, usize::MAX] {
            let a = crate::scan_groups_within(&Selection::all(n), &key2, Some(cval), 24, max_routed, |len| {
                IntSums::new(len, &per_code2)
            })
            .unwrap();
            let b = scan(&Selection::all(n), &key2, Some(cval), 1, || {
                IntSums::new(groups, &per_code2)
            })
            .unwrap();
            assert_eq!((a.sums, a.counts), (b.sums, b.counts), "max_routed {max_routed}");
            let a = crate::scan_groups_within(&Selection::all(n), &key2, None, 4, max_routed, FirstRow::new)
                .unwrap();
            let b = scan(&Selection::all(n), &key2, None, 1, || FirstRow::new(groups)).unwrap();
            assert_eq!(a.0, b.0, "max_routed {max_routed}");
            let a = crate::scan_groups_within(&Selection::all(n), &key2, None, 8, max_routed, Counts::new)
                .unwrap();
            assert_eq!(a.0, want, "max_routed {max_routed}");
            assert!(crate::scan_groups_bytes(n, groups, 24, max_routed) >= groups * 24);
        }
        // a list of rows, close together then scattered
        let list: Vec<u32> = (0..n as u32)
            .filter(|r| r % 3 != 0)
            .chain((0..n as u32).rev().step_by(97))
            .collect();
        let a = scan_list(&list, &key2, Some(cval), 4, || IntSums::new(groups, &per_code2)).unwrap();
        let mut want = vec![0i128; groups];
        for r in &list {
            let r = *r as usize;
            want[key2.id(&[val[r], hi[r]]).unwrap()] += i128::from(per_code2[val[r] as usize].unwrap());
        }
        assert_eq!(a.sums, want);
    }
}

/// Groupings by ranges (`docs/beyond-ram.md`): the groups of a key scanned a range at a time
/// give each group the state of the whole scan, for every range size (one range, many, ranges
/// of one group, empty ranges past the end), filtered or not, on 1 and 4 threads, radix or not,
/// with the workers' states held to a few bytes; double sums are the same bits.
#[test]
fn grouped_scans_by_ranges_give_each_group_its_whole_state() {
    for n in [0usize, 1, 5_000, 300_000] {
        for packed in [false, true] {
            let (_, val, hi) = data(n);
            let (pval, phi) = (pack(&val, 65_536), pack(&hi, 100_003));
            let (cval, chi) = if packed {
                (Col::Packed(&pval), Col::Packed(&phi))
            } else {
                (Col::Plain(&val), Col::Plain(&hi))
            };
            let allowed: Vec<bool> = (0..51).map(|c| c % 3 != 0).collect();
            let mask: Vec<bool> = (0..n).map(|i| i % 7 != 0).collect();
            let filtered = Selection {
                rows: n,
                preds: vec![Pred {
                    codes: cval,
                    allowed: &allowed,
                }],
                masks: vec![&mask],
                none: false,
            };
            let key = Key::new(vec![
                KeyPart {
                    codes: cval,
                    lut: None,
                    card: 51,
                },
                KeyPart {
                    codes: chi,
                    lut: None,
                    card: (n / 10).max(1),
                },
            ])
            .unwrap();
            let groups = key.groups();
            let ints: Vec<Option<i64>> = (0..51).map(|c| (c % 5 != 0).then_some(c * 7 - 20)).collect();
            let floats: Vec<Option<f64>> = (0..51)
                .map(|c| (c % 5 != 0).then_some(f64::from(c) * 0.1 + 1e9 * f64::from(c % 3)))
                .collect();
            for sel in [&Selection::all(n), &filtered] {
                for threads in [1usize, 4] {
                    set_threads(Some(threads));
                    let whole =
                        scan_groups(sel, &key, Some(cval), 24, |len| IntSums::new(len, &ints)).unwrap();
                    let first = scan_groups(sel, &key, None, 4, FirstRow::new).unwrap();
                    let counts = scan_groups(sel, &key, None, 8, Counts::new).unwrap();
                    let fl = scan_ordered(sel, &key, Some(cval), threads, || FloatSums::new(groups, &floats))
                        .unwrap();
                    for len in [1usize, 7, 1000, 65_536, groups.div_ceil(3), groups, groups + 5] {
                        if groups.div_ceil(len) > 64 {
                            continue;
                        }
                        let (mut sums, mut cnt, mut rows, mut c2) =
                            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
                        let (mut fs, mut fc) = (Vec::new(), Vec::new());
                        let mut lo = 0usize;
                        // one range past the end: empty
                        while lo <= groups {
                            let max_states = if len % 2 == 0 { 1 } else { usize::MAX };
                            let r = scan_groups_range(
                                sel,
                                &key,
                                Some(cval),
                                24,
                                1 << 20,
                                lo,
                                len,
                                max_states,
                                |l| IntSums::new(l, &ints),
                            )
                            .unwrap();
                            let want_len = len.min(groups.saturating_sub(lo));
                            assert_eq!(r.sums.len(), want_len);
                            sums.extend(r.sums);
                            cnt.extend(r.counts);
                            let f = scan_groups_range(
                                sel,
                                &key,
                                None,
                                4,
                                usize::MAX,
                                lo,
                                len,
                                max_states,
                                FirstRow::new,
                            )
                            .unwrap();
                            rows.extend(f.0);
                            let c =
                                scan_groups_range(sel, &key, None, 8, 1, lo, len, max_states, Counts::new)
                                    .unwrap();
                            c2.extend(c.0);
                            let o = scan_ordered_range(sel, &key, Some(cval), threads, lo, len, || {
                                FloatSums::new(want_len, &floats)
                            })
                            .unwrap();
                            fs.extend((0..want_len).map(|g| o.total(g).to_bits()));
                            fc.extend(o.counts);
                            lo += len;
                        }
                        let what = format!("n {n} packed {packed} threads {threads} len {len}");
                        assert_eq!((&sums, &cnt), (&whole.sums, &whole.counts), "{what}");
                        assert_eq!(rows, first.0, "{what}");
                        assert_eq!(c2, counts.0, "{what}");
                        let want: Vec<u64> = (0..groups).map(|g| fl.total(g).to_bits()).collect();
                        assert_eq!(fs, want, "{what}");
                        assert_eq!(fc, fl.counts, "{what}");
                    }
                    // a range starting past the end keeps nothing
                    let r = scan_groups_range(sel, &key, None, 8, 1, groups + 3, 10, 1, Counts::new).unwrap();
                    assert!(r.0.is_empty());
                    assert!(scan_groups_range_bytes(n, 10, 24, 1, 1) <= (1 << 20) + 10 * 24 * 2);
                    set_threads(None);
                }
            }
        }
    }
}

#[test]
fn float_sums_are_compensated() {
    let per_code = vec![None, Some(1e16), Some(1.0), Some(-1e16)];
    let codes: Vec<u32> = vec![1, 2, 2, 2, 3, 0];
    let r = scan(
        &Selection::all(codes.len()),
        &Key::single(),
        Some(Col::Plain(&codes)),
        1,
        || FloatSums::new(1, &per_code),
    )
    .unwrap();
    assert_eq!(r.total(0), 3.0);
    assert_eq!(r.counts, vec![5]);
}

#[test]
fn keys_refuse_too_many_groups() {
    let c = [0u32];
    let p = KeyPart {
        codes: Col::Plain(&c),
        lut: None,
        card: 1 << 20,
    };
    assert!(Key::new(vec![p, p]).is_none());
    assert_eq!(Key::new(vec![p]).unwrap().id(&[(1 << 20) + 1]), None);
}

/// Throughput per kernel and thread count at 1 M rows (`BICRAFT_MICRO_ROWS` to change it).
#[test]
#[ignore]
fn micro() {
    let n: usize = bicraft_env::var("BICRAFT_MICRO_ROWS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1_000_000);
    let (fk, val, hi) = data(n);
    let lut: Vec<u32> = (0..1001u32).map(|c| c % 10).collect();
    let allowed: Vec<bool> = (0..51).map(|c| c % 3 != 0).collect();
    let per_code: Vec<Option<i64>> = (0..51).map(|c| Some(c * 3)).collect();
    let max = configured_threads();
    let mut threads: Vec<usize> = bicraft_env::var("BICRAFT_MICRO_THREADS")
        .ok()
        .map(|v| v.split(',').filter_map(|t| t.parse().ok()).collect())
        .unwrap_or_else(|| vec![1, 2, 4, 8, max]);
    threads.retain(|t| *t <= max);
    threads.dedup();
    let time = |f: &mut dyn FnMut()| {
        f();
        let t = Instant::now();
        let reps = 5;
        for _ in 0..reps {
            f();
        }
        t.elapsed().as_secs_f64() / reps as f64
    };
    println!("rows {n}");
    let (pfk, pval) = (pack(&fk, 1 << 20), pack(&val, 1 << 20));
    for t in threads {
        let filtered_p = Selection {
            rows: n,
            preds: vec![Pred {
                codes: Col::Packed(&pval),
                allowed: &allowed,
            }],
            masks: Vec::new(),
            none: false,
        };
        let by_cat_p = Key::new(vec![KeyPart {
            codes: Col::Packed(&pfk),
            lut: Some(&lut),
            card: 10,
        }])
        .unwrap();
        let cap = |sink_bytes: usize| t.min(threads_for(n, sink_bytes));
        let filtered = Selection {
            rows: n,
            preds: vec![Pred {
                codes: Col::Plain(&val),
                allowed: &allowed,
            }],
            masks: Vec::new(),
            none: false,
        };
        let by_cat = Key::new(vec![KeyPart {
            codes: Col::Plain(&fk),
            lut: Some(&lut),
            card: 10,
        }])
        .unwrap();
        let by_hi = Key::new(vec![KeyPart {
            codes: Col::Plain(&hi),
            lut: None,
            card: n / 10,
        }])
        .unwrap();
        let all = Selection::all(n);
        let cases: Vec<(&str, f64)> = vec![
            (
                "sum, no filter",
                time(&mut || {
                    scan(&all, &Key::single(), Some(Col::Plain(&val)), cap(24), || {
                        IntSums::new(1, &per_code)
                    });
                }),
            ),
            (
                "sum, filtered, by category (lut)",
                time(&mut || {
                    scan(&filtered, &by_cat, Some(Col::Plain(&val)), cap(240), || {
                        IntSums::new(10, &per_code)
                    });
                }),
            ),
            (
                "sum by customer (n/10 groups)",
                time(&mut || {
                    scan(&all, &by_hi, Some(Col::Plain(&val)), cap(n / 10 * 24), || {
                        IntSums::new(n / 10, &per_code)
                    });
                }),
            ),
            (
                "sum by customer, scan_groups",
                time(&mut || {
                    scan_groups(&all, &by_hi, Some(Col::Plain(&val)), 24, |len| {
                        IntSums::new(len, &per_code)
                    });
                }),
            ),
            (
                "distinct customers by category",
                time(&mut || {
                    scan(&all, &by_cat, Some(Col::Plain(&hi)), cap(n / 8), || {
                        Distinct::new(10, n / 10, 1 << 30).unwrap_or(Distinct {
                            card: 1,
                            words: 1,
                            bits: Vec::new(),
                        })
                    });
                }),
            ),
            (
                "packed: sum, no filter",
                time(&mut || {
                    scan(&all, &Key::single(), Some(Col::Packed(&pval)), cap(24), || {
                        IntSums::new(1, &per_code)
                    });
                }),
            ),
            (
                "packed: sum, filtered, by category",
                time(&mut || {
                    scan(&filtered_p, &by_cat_p, Some(Col::Packed(&pval)), cap(240), || {
                        IntSums::new(10, &per_code)
                    });
                }),
            ),
            (
                "packed: sum_by_id by category",
                time(&mut || {
                    sum_by_id(
                        &all,
                        &by_cat_p,
                        &pval,
                        IdValues::Affine { first: 3 },
                        t.min(threads_for(n, 240)),
                    );
                }),
            ),
            (
                "packed: sum_by_id, no group",
                time(&mut || {
                    sum_by_id(
                        &all,
                        &Key::single(),
                        &pval,
                        IdValues::Affine { first: 3 },
                        t.min(threads_for(n, 24)),
                    );
                }),
            ),
            (
                "packed: counts per code",
                time(&mut || {
                    pval.counts_parallel(51, t);
                }),
            ),
            (
                "mask, filtered",
                time(&mut || {
                    super::mask(&filtered, cap(0));
                }),
            ),
        ];
        for (name, s) in cases {
            println!(
                "threads {t:>2}  {name:<34} {:>8.2} ms  {:>7.0} M rows/s",
                s * 1e3,
                n as f64 / s / 1e6
            );
        }
    }
}

#[test]
fn segment_statistics_skip_exactly() {
    // sorted codes: most segments hold no allowed code and are skipped whole
    let n = 400_000usize;
    let codes: Vec<u32> = (0..n).map(|i| 1 + (i / 1000) as u32).collect();
    let card = 1 + n / 1000;
    let p = pack(&codes, 4096);
    let mut allowed = vec![false; card + 1];
    for c in [5usize, 6, 250, 399] {
        allowed[c] = true;
    }
    for col in [Col::Plain(&codes), Col::Packed(&p)] {
        let sel = Selection {
            rows: n,
            preds: vec![Pred {
                codes: col,
                allowed: &allowed,
            }],
            masks: Vec::new(),
            none: false,
        };
        let got = scan(&sel, &Key::single(), Some(col), 4, || Counts::new(1)).unwrap();
        assert_eq!(got.0, vec![4000]);
        let rows = super::rows(&sel, 4);
        let want: Vec<u32> = (0..n as u32)
            .filter(|r| allowed[codes[*r as usize] as usize])
            .collect();
        assert_eq!(rows, want);
    }
}

/// Segments whose id range spans more than 65,536 ids, and zones of 64 Ki rows inside a segment
/// of 1 Mi rows (plain and RLE): what the statistics and zone maps skip holds no selected row,
/// the zones' bounds are exact, and the rows and counts are those of a naive filter.
#[test]
fn zone_maps_skip_exactly_whatever_the_id_range() {
    let n = 3 * (1 << 20) + 12_345;
    let seg = 1 << 20;
    // ids rise along the rows (a fact loaded by key), with some noise inside each zone
    let codes: Vec<u32> = (0..n)
        .map(|i| 1 + (i / 4) as u32 + if i % 97 == 0 { 3 } else { 0 })
        .collect();
    let card = codes.iter().copied().max().unwrap() as usize + 1;
    let ranges = [
        (10usize, 20usize),
        (300_000, 300_010),
        (700_000, 701_000),
        (card - 5, card),
    ];
    for rle in [false, true] {
        let p = if rle {
            crate::kernel_tests::pack_rle(&codes, seg)
        } else {
            pack(&codes, seg)
        };
        // the zone maps bound the ids of their rows exactly
        for si in 0..p.segments().len() {
            let (_, first, rows) = p.segment_at(si * seg).unwrap();
            for (z, (lo, hi)) in p.zones(si).iter().enumerate() {
                let a = first + z * crate::packed::ZONE_ROWS;
                let b = (a + crate::packed::ZONE_ROWS).min(first + rows);
                let ids: Vec<u32> = (a..b).map(|r| p.get_id(r)).collect();
                assert_eq!(
                    (*lo, *hi),
                    (*ids.iter().min().unwrap(), *ids.iter().max().unwrap())
                );
            }
        }
        for (lo, hi) in ranges {
            let allowed: Vec<bool> = (0..=card).map(|c| (lo..hi).contains(&c)).collect();
            let sel = Selection {
                rows: n,
                preds: vec![Pred {
                    codes: Col::Packed(&p),
                    allowed: &allowed,
                }],
                masks: Vec::new(),
                none: false,
            };
            let want: Vec<u32> = (0..n as u32)
                .filter(|r| allowed[codes[*r as usize] as usize])
                .collect();
            assert_eq!(super::rows(&sel, 4), want, "rle {rle} {lo}..{hi}");
            let got = scan(&sel, &Key::single(), None, 4, || Counts::new(1)).unwrap();
            assert_eq!(got.0, vec![want.len() as u64]);
        }
    }
}

/// Predicates in any order (the plan puts the most selective first and stops reading a block
/// once no row is left): the rows of a naive conjunction, with runs, a mask and plain columns.
#[test]
fn predicates_in_any_order_select_the_same_rows() {
    let n = 300_000usize;
    // a column in long runs, one that varies on every row, one in memory
    let runs: Vec<u32> = (0..n).map(|i| 1 + ((i / 5000) % 7) as u32).collect();
    let noise: Vec<u32> = (0..n).map(|i| 1 + (splitmix(i as u64) % 50) as u32).collect();
    let plain: Vec<u32> = (0..n).map(|i| 1 + (splitmix(i as u64 ^ 9) % 4) as u32).collect();
    let mask: Vec<bool> = (0..n).map(|i| i % 11 != 0).collect();
    let (pr, pn) = (
        crate::kernel_tests::pack_rle(&runs, 1 << 16),
        pack(&noise, 1 << 16),
    );
    let a_runs: Vec<bool> = (0..9).map(|c| c == 3).collect();
    let a_noise: Vec<bool> = (0..52).map(|c| c % 5 != 0).collect();
    let a_plain: Vec<bool> = (0..6).map(|c| c != 2).collect();
    let preds = [
        Pred {
            codes: Col::Packed(&pr),
            allowed: &a_runs,
        },
        Pred {
            codes: Col::Packed(&pn),
            allowed: &a_noise,
        },
        Pred {
            codes: Col::Plain(&plain),
            allowed: &a_plain,
        },
    ];
    let want: Vec<u32> = (0..n as u32)
        .filter(|r| {
            let r = *r as usize;
            a_runs[runs[r] as usize] && a_noise[noise[r] as usize] && a_plain[plain[r] as usize] && mask[r]
        })
        .collect();
    for order in [[0, 1, 2], [1, 2, 0], [2, 1, 0], [1, 0, 2]] {
        let sel = Selection {
            rows: n,
            preds: order.iter().map(|i| preds[*i]).collect(),
            masks: vec![&mask[..]],
            none: false,
        };
        assert_eq!(super::rows(&sel, 4), want, "{order:?}");
        let got = scan(&sel, &Key::single(), None, 4, || Counts::new(1)).unwrap();
        assert_eq!(got.0, vec![want.len() as u64], "{order:?}");
    }
}

/// Encoding edges: block (2,048) and segment boundaries, RLE runs across blocks, segments of
/// different encodings in one column, an empty segment, consecutive ids, an all-blank column.
#[test]
fn packed_encoding_edges_read_as_their_codes() {
    // ids: 2 blank, codes = id - 2 for ids >= 3 (lut), plus ids that map to the blank code
    let bits = |ids: &[u32], base: u32, w: u8| {
        let per = 64 / usize::from(w);
        let words = ids
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
    let mut want: Vec<u32> = Vec::new();
    // 1. bit-packed, 2,047 rows (a block boundary inside the next segment)
    let a: Vec<u32> = (0..2047u32).map(|i| 3 + i % 37).collect();
    segs.push(Segment {
        rows: a.len(),
        data: SegData::Plain(bits(&a, 2, 6)),
        min_id: 3,
        max_id: 39,
    });
    want.extend(a.iter().map(|i| i - 2));
    // 2. empty segment
    segs.push(Segment {
        rows: 0,
        data: SegData::Plain(bits(&[], 2, 1)),
        min_id: 3,
        max_id: 3,
    });
    // 3. hybrid RLE: a run of 5,000 across two block boundaries, packed runs, a blank run
    let sub_ids: Vec<u32> = (0..300u32).map(|i| 3 + i % 11).collect();
    let runs = vec![
        Run {
            start: 0,
            len: 5000,
            id: 7,
            packed: false,
        },
        Run {
            start: 5000,
            len: 300,
            id: 0,
            packed: true,
        },
        Run {
            start: 5300,
            len: 2100,
            id: 2,
            packed: false,
        },
        Run {
            start: 7400,
            len: 10,
            id: 41,
            packed: false,
        },
    ];
    segs.push(Segment {
        rows: 7410,
        data: SegData::Rle {
            runs,
            sub: bits(&sub_ids, 2, 4),
        },
        min_id: 2,
        max_id: 41,
    });
    want.extend(std::iter::repeat_n(5, 5000));
    want.extend(sub_ids.iter().map(|i| i - 2));
    want.extend(std::iter::repeat_n(0, 2100));
    want.extend(std::iter::repeat_n(39, 10));
    // 4. consecutive ids (XM123)
    segs.push(Segment {
        rows: 30,
        data: SegData::Plain(Ids::Seq { first: 3 }),
        min_id: 3,
        max_id: 32,
    });
    want.extend((3..33u32).map(|i| i - 2));
    // 5. bit-packed at 32 bits with a large base
    let e: Vec<u32> = (0..2049u32).map(|i| 1_000 + i % 3).collect();
    segs.push(Segment {
        rows: e.len(),
        data: SegData::Plain(bits(&e, 1_000, 32)),
        min_id: 1_000,
        max_id: 1_002,
    });
    want.extend(e.iter().map(|i| i - 958));
    let mut lut = vec![0u32; 1_003];
    for id in 3..=41u32 {
        lut[id as usize] = id - 2;
    }
    for id in 1_000..=1_002u32 {
        lut[id as usize] = id - 958;
    }
    let p = Packed::new(segs, lut);
    assert_eq!(p.rows(), want.len());
    for (r, c) in want.iter().enumerate() {
        assert_eq!(p.get(r), *c, "row {r}");
    }
    for start in [0usize, 1, 2046, 2047, 2048, 4095, 7000, want.len() - 5] {
        let len = 4100.min(want.len() - start);
        let mut buf = Vec::new();
        assert_eq!(
            Col::Packed(&p).block(start, len, &mut buf),
            &want[start..start + len],
            "block at {start}"
        );
    }
    let card = 45;
    let mut counts = vec![0u64; card];
    for c in &want {
        counts[*c as usize] += 1;
    }
    assert_eq!(p.counts(card), counts);
    assert_eq!(p.counts_parallel(card, 4), counts);
    // a filtered, grouped scan over it equals the same scan over the plain codes
    let allowed: Vec<bool> = (0..card).map(|c| c % 2 == 1 || c == 0).collect();
    let per_code: Vec<Option<i64>> = (0..card as i64).map(|c| (c > 0).then_some(c * 10 - 7)).collect();
    for col in [Col::Plain(&want), Col::Packed(&p)] {
        let sel = Selection {
            rows: want.len(),
            preds: vec![Pred {
                codes: col,
                allowed: &allowed,
            }],
            masks: Vec::new(),
            none: false,
        };
        let key = Key::new(vec![KeyPart {
            codes: col,
            lut: None,
            card,
        }])
        .unwrap();
        let got = scan(&sel, &key, Some(col), 3, || IntSums::new(card, &per_code)).unwrap();
        let mut sums = vec![0i128; card];
        for c in &want {
            if allowed[*c as usize]
                && let Some(x) = per_code[*c as usize]
            {
                sums[*c as usize] += i128::from(x);
            }
        }
        assert_eq!(got.sums, sums);
    }
    // an all-blank column
    let blank = Packed::new(
        vec![Segment {
            rows: 5000,
            data: SegData::Rle {
                runs: vec![Run {
                    start: 0,
                    len: 5000,
                    id: 2,
                    packed: false,
                }],
                sub: Ids::Seq { first: 3 },
            },
            min_id: 2,
            max_id: 2,
        }],
        vec![0, 0, 0],
    );
    assert_eq!(blank.counts(1), vec![5000]);
    let mut buf = Vec::new();
    assert!(
        Col::Packed(&blank)
            .block(4000, 1000, &mut buf)
            .iter()
            .all(|c| *c == 0)
    );
}

/// `sum_by_id_multi` (and `sum_by_id`) against a naive loop, on columns of long runs, short runs
/// and noise kept as alternately bit-packed and RLE segments: blocks inside one run (one id, one
/// group, all kept or all dropped), groups by runs, by rows and by two columns, value-encoded
/// and table values (small and wide, negative, blank ids), with and without filters.
#[test]
fn sums_by_id_match_a_naive_loop_on_runs() {
    let n = 50_000usize;
    let q: Vec<u32> = (0..n).map(|i| 1 + ((i / 700) % 10) as u32).collect();
    let st: Vec<u32> = (0..n).map(|i| 1 + ((i / 37) % 200) as u32).collect();
    let v: Vec<u32> = (0..n).map(|i| (splitmix(i as u64) % 51) as u32).collect();
    let w: Vec<u32> = (0..n).map(|i| ((i / 3000) % 7) as u32).collect();
    let (pq, pst, pv, pw) = (pack(&q, 8192), pack(&st, 8192), pack(&v, 8192), pack(&w, 8192));
    let mask: Vec<bool> = (0..n)
        .map(|i| !splitmix(i as u64 ^ 7).is_multiple_of(5))
        .collect();
    let allowed_q: Vec<bool> = (0..11).map(|c| c >= 6).collect();
    let q_lut: Vec<u32> = (0..11).map(|c| c % 4).collect();
    // id = code + 3, blank id 2
    let id_of = |c: u32| if c == 0 { 2 } else { c + 3 };
    let max_id = 60usize;
    let small: Vec<i64> = (0..max_id as i64).map(|i| i * 7 - 100).collect();
    let wide: Vec<i64> = (0..max_id as i64).map(|i| (i - 30) * (1 << 50)).collect();
    let valid: Vec<bool> = (0..max_id).map(|i| i % 9 != 4 && i != 2).collect();
    let cols: [(&Packed, &[u32]); 2] = [(&pv, &v), (&pw, &w)];
    for (sel_name, preds, masks) in [
        ("all", false, false),
        ("pred on runs", true, false),
        ("pred and mask", true, true),
    ] {
        let sel = Selection {
            rows: n,
            preds: if preds {
                vec![Pred {
                    codes: Col::Packed(&pq),
                    allowed: &allowed_q,
                }]
            } else {
                Vec::new()
            },
            masks: if masks { vec![&mask[..]] } else { Vec::new() },
            none: false,
        };
        let selected = |i: usize| (!preds || allowed_q[q[i] as usize]) && (!masks || mask[i]);
        // (name, key, group of row i)
        type Group<'k> = Box<dyn Fn(usize) -> usize + 'k>;
        let keys: Vec<(&str, Key<'_>, Group<'_>)> = vec![
            ("single", Key::single(), Box::new(|_| 0)),
            (
                "stores (runs)",
                Key::new(vec![KeyPart {
                    codes: Col::Packed(&pst),
                    lut: None,
                    card: 201,
                }])
                .unwrap(),
                Box::new(|i| st[i] as usize),
            ),
            (
                "quantity through a table (long runs)",
                Key::new(vec![KeyPart {
                    codes: Col::Packed(&pq),
                    lut: Some(&q_lut),
                    card: 4,
                }])
                .unwrap(),
                Box::new(|i| q_lut[q[i] as usize] as usize),
            ),
            (
                "values (rows)",
                Key::new(vec![KeyPart {
                    codes: Col::Packed(&pv),
                    lut: None,
                    card: 51,
                }])
                .unwrap(),
                Box::new(|i| v[i] as usize),
            ),
            (
                "quantity x stores",
                Key::new(vec![
                    KeyPart {
                        codes: Col::Packed(&pq),
                        lut: None,
                        card: 11,
                    },
                    KeyPart {
                        codes: Col::Packed(&pst),
                        lut: None,
                        card: 201,
                    },
                ])
                .unwrap(),
                Box::new(|i| q[i] as usize * 201 + st[i] as usize),
            ),
        ];
        for (key_name, key, group) in &keys {
            let groups = key.groups();
            for threads in [1, 4] {
                let spec: Vec<(&Packed, IdValues<'_>)> = vec![
                    (cols[0].0, IdValues::Affine { first: 3 }),
                    (cols[1].0, IdValues::Affine { first: 3 }),
                    (
                        cols[0].0,
                        IdValues::Table {
                            values: &small,
                            valid: &valid,
                        },
                    ),
                    (
                        cols[1].0,
                        IdValues::Table {
                            values: &wide,
                            valid: &valid,
                        },
                    ),
                ];
                let got = sum_by_id_multi(&sel, key, &spec, threads).unwrap();
                for (k, (_, vals)) in spec.iter().enumerate() {
                    let codes = if k % 2 == 0 { &v } else { &w };
                    let mut sums = vec![0i128; groups];
                    let mut counts = vec![0u64; groups];
                    for i in 0..n {
                        if !selected(i) {
                            continue;
                        }
                        let id = id_of(codes[i]);
                        let g = group(i);
                        match vals {
                            IdValues::Affine { first } => {
                                if id >= *first {
                                    sums[g] += i128::from(id);
                                    counts[g] += 1;
                                }
                            }
                            IdValues::Table { values, valid } => {
                                if valid[id as usize] {
                                    sums[g] += i128::from(values[id as usize]);
                                    counts[g] += 1;
                                }
                            }
                        }
                    }
                    assert_eq!(
                        got[k],
                        (sums, counts),
                        "{sel_name}, {key_name}, column {k}, {threads} threads"
                    );
                }
                let one = sum_by_id(&sel, key, spec[0].0, spec[0].1, threads).unwrap();
                assert_eq!(one, got[0], "sum_by_id: {sel_name}, {key_name}");
            }
        }
    }
}

/// Distinct codes as sorted pairs ([`DistinctPairs`] → [`SparseDistinct`]) count as the
/// bitmaps ([`Distinct`]): per group, unions, groups appended by range, coarsened groups, and
/// the deduplications in the middle of a long scan.
#[test]
fn sparse_distinct_counts_as_the_bitmaps() {
    let (groups, card) = (300usize, 5_000usize);
    let mut dense = Distinct::new(groups, card, usize::MAX).unwrap();
    let mut a = DistinctPairs::new(groups, card);
    let mut b = DistinctPairs::new(groups, card);
    for i in 0..3_000_000u64 {
        let r = splitmix(i);
        let g = (r % groups as u64) as usize;
        // skewed values, the blank code and a value past the codes (clamped as the bitmaps do)
        let v = match r >> 60 {
            0 => 0,
            1 => card as u32 + 7,
            _ => ((r >> 20) % (1 + (r >> 40) % card as u64)) as u32,
        };
        dense.add(g, v, 0);
        if i % 2 == 0 {
            a.add(g, v, 0)
        } else {
            b.add(g, v, 0)
        }
    }
    a.merge(b);
    let sparse = a.finish();
    assert_eq!(sparse.groups(), groups);
    for g in 0..groups {
        assert_eq!(sparse.count(g), dense.count(g), "group {g}");
    }
    for set in [vec![], vec![3], vec![1, 7, 299], (0..groups).collect::<Vec<_>>()] {
        assert_eq!(sparse.count_union(&set), dense.count_union(&set), "{set:?}");
    }
    // by range: the second sink's groups follow the first's
    let mut lo = DistinctPairs::new(2, 10);
    lo.add(1, 4, 0);
    let mut hi = DistinctPairs::new(3, 10);
    hi.add(0, 4, 0);
    hi.add(2, 9, 0);
    lo.append(hi);
    let s = lo.finish();
    assert_eq!(s.groups(), 5);
    assert_eq!(
        (s.group(1), s.group(2), s.group(4), s.group(0)),
        (&[4][..], &[4][..], &[9][..], &[][..])
    );
    // coarsened: groups g and g + 150 together
    let map: Vec<usize> = (0..groups).map(|g| g % 150).collect();
    let coarse = sparse.coarsen(&map, 150).unwrap();
    for g in 0..150 {
        assert_eq!(coarse.count(g), dense.count_union(&[g, g + 150]));
    }
    assert!(sparse.coarsen(&map, 10).is_none());
}
