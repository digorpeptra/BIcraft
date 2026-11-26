//! Fuzz smoke test (M7.3): mutated `.idfmeta`, `.idf`, `.dictionary` and `.hidx` files must give
//! `Err` or values, never a panic, a hang or an unbounded allocation. Mirrors the `vertipaq_*`
//! cargo-fuzz targets (docs/fuzzing.md). Seeds are built here from the layouts in
//! docs/formats/vertipaq.md.

use bicraft_vertipaq::bitpack;
use bicraft_vertipaq::huffman::{BitWriter, HuffmanCode};
use bicraft_vertipaq::idfmeta::{DataObject, RleFacet, SegmentDataObject, SegmentStats, SubSegmentMeta};
use bicraft_vertipaq::{
    ColumnEncoding, ColumnMeta, Compression, InnerCompression, Limits, LogicalType, SegmentMeta, StorageType,
    decode_column_with_limits, decode_data_ids, parse_dictionary, parse_hash_index, parse_idfmeta,
};

/// Row cap for fuzzed columns: the default (2^26) lets a few bytes of RLE describe gigabytes of
/// cells, which is bounded but too slow for a fuzz loop.
const LIMITS: Limits = Limits {
    max_rows: 1 << 16,
    ..Limits::DEFAULT
};

fn stats(min: u32, max: u32, rows: u64) -> SegmentStats {
    SegmentStats {
        distinct_states: 0,
        min_data_id: min,
        max_data_id: max,
        original_min_segment_data_id: 2,
        rle_sort_order: -1,
        row_count: rows,
        has_nulls: 0,
        rle_runs: 0,
        others_rle_runs: 0,
    }
}

/// One hybrid RLE + bit-packed segment: `runs` are (data id, count), `None` = bit-packed run.
fn hybrid(runs: &[(Option<u32>, u32)], packed: &[u32], width: u8) -> (Vec<u8>, Vec<u8>) {
    let base = packed.iter().copied().min().unwrap_or(3);
    let mut idf = Vec::new();
    let mut entries = Vec::new();
    let mut taken = 0u32;
    let mut rows = 0u64;
    for &(id, count) in runs {
        let v = id.unwrap_or_else(|| {
            let v = u32::MAX - taken;
            taken += count;
            v
        });
        entries.push((v, count));
        rows += u64::from(count);
    }
    let alloc = entries.len().max(16);
    idf.extend_from_slice(&(alloc as u64).to_le_bytes());
    for k in 0..alloc {
        let (v, c) = entries.get(k).copied().unwrap_or((0, 0));
        idf.extend_from_slice(&v.to_le_bytes());
        idf.extend_from_slice(&c.to_le_bytes());
    }
    let rel: Vec<u32> = packed.iter().map(|v| v - base).collect();
    let words = bitpack::pack(&rel, width).unwrap();
    let sub_at = idf.len() as u64;
    idf.extend_from_slice(&((words.len() / 8) as u64).to_le_bytes());
    idf.extend_from_slice(&words);
    let compression = Compression::HybridRle(InnerCompression::NoSplit(width));
    let meta = ColumnMeta {
        segments: vec![SegmentMeta {
            records: rows,
            base_id: 1,
            compression_class: compression.class_id(),
            sub_compression_class: compression.sub_class_id(),
            rle: Some(RleFacet {
                bookmark_bits: 4,
                storage_alloc_size: 2 * alloc as u64,
                storage_used_size: 2 * entries.len() as u64,
                segment_needs_resizing: 0,
            }),
            first_run_value: runs.first().and_then(|r| r.0).unwrap_or(base),
            stats: stats(3, 20, rows),
            sub_segment: Some(SubSegmentMeta {
                records: packed.len() as u64,
                base_id: 0,
            }),
        }],
        data_objects: Some(vec![SegmentDataObject {
            primary: DataObject {
                offset: 0,
                words: alloc as u64,
            },
            sub: Some(DataObject {
                offset: sub_at,
                words: (words.len() / 8) as u64,
            }),
        }]),
    };
    (idf, meta.to_bytes())
}

/// Two plain NoSplit<32> segments (the system-table layout).
fn plain() -> (Vec<u8>, Vec<u8>) {
    let seg = |records: u64| SegmentMeta {
        records,
        base_id: 0,
        compression_class: 703_062,
        sub_compression_class: 1,
        rle: None,
        first_run_value: 0,
        stats: stats(2, 2, 0),
        sub_segment: None,
    };
    let mut idf = Vec::new();
    let mut objects = Vec::new();
    for vals in [(3..9).collect::<Vec<u32>>(), vec![0, 1, 2]] {
        let words = bitpack::pack(&vals, 32).unwrap();
        objects.push(SegmentDataObject {
            primary: DataObject {
                offset: idf.len() as u64,
                words: (words.len() / 8) as u64,
            },
            sub: None,
        });
        idf.extend_from_slice(&((words.len() / 8) as u64).to_le_bytes());
        idf.extend(words);
    }
    let meta = ColumnMeta {
        segments: vec![seg(6), seg(3)],
        data_objects: Some(objects),
    };
    (idf, meta.to_bytes())
}

fn columns() -> Vec<(Vec<u8>, Vec<u8>)> {
    vec![
        hybrid(
            &[(Some(5), 70), (None, 4), (Some(3), 64), (None, 3)],
            &[3, 4, 6, 9, 7, 7, 3],
            3,
        ),
        hybrid(&[(None, 4)], &[3, 4, 5, 10], 4),
        hybrid(&[(Some(3), 4)], &[], 1),
        plain(),
    ]
}

fn hash_header(algorithm: i32, entry: u32, locals: u32) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&algorithm.to_le_bytes());
    v.extend_from_slice(&entry.to_le_bytes());
    v.extend_from_slice(&64u32.to_le_bytes());
    v.extend_from_slice(&locals.to_le_bytes());
    v.extend_from_slice(&u64::MAX.to_le_bytes());
    v
}

fn long_dictionary(values: &[i64], element_size: u32) -> Vec<u8> {
    let mut d = 0i32.to_le_bytes().to_vec();
    d.extend(hash_header(-1, 8, 6));
    d.extend_from_slice(&(values.len() as u64).to_le_bytes());
    d.extend_from_slice(&element_size.to_le_bytes());
    for &v in values {
        if element_size == 4 {
            d.extend_from_slice(&(v as i32).to_le_bytes());
        } else {
            d.extend_from_slice(&v.to_le_bytes());
        }
    }
    d
}

fn real_dictionary(values: &[f64]) -> Vec<u8> {
    let mut d = 1i32.to_le_bytes().to_vec();
    d.extend(hash_header(-1, 16, 3));
    d.extend_from_slice(&(values.len() as u64).to_le_bytes());
    d.extend_from_slice(&8u32.to_le_bytes());
    for &v in values {
        d.extend_from_slice(&v.to_le_bytes());
    }
    d
}

/// A string dictionary with one uncompressed page and one Huffman page (single character set
/// when `charset` is given, else UTF-16 bytes with an all-8-bit code).
fn string_dictionary(plain: &[&str], huffman: &[&str], charset: Option<u8>) -> Vec<u8> {
    let mut body = Vec::new();
    let mut handles = Vec::new();
    let mut longest = 0u64;
    for s in plain.iter().chain(huffman) {
        longest = longest.max(s.encode_utf16().count() as u64);
    }
    // Page 0: uncompressed.
    body.extend_from_slice(&0u64.to_le_bytes());
    body.push(0);
    body.extend_from_slice(&0u64.to_le_bytes());
    body.extend_from_slice(&(plain.len() as u64).to_le_bytes());
    body.push(0);
    body.extend_from_slice(&[0xDD, 0xCC, 0xBB, 0xAA]);
    let mut chars: Vec<u16> = Vec::new();
    for s in plain {
        handles.push((chars.len() as u32, 0u32));
        chars.extend(s.encode_utf16());
        chars.push(0);
    }
    body.extend_from_slice(&0u64.to_le_bytes());
    body.extend_from_slice(&(chars.len() as u64).to_le_bytes());
    body.extend_from_slice(&(chars.len() as u64 * 2).to_le_bytes());
    for c in chars {
        body.extend_from_slice(&c.to_le_bytes());
    }
    body.extend_from_slice(&[0xCD, 0xAB, 0xCD, 0xAB]);
    // Page 1: Huffman, every byte value 8 bits long.
    body.extend_from_slice(&0u64.to_le_bytes());
    body.push(0);
    body.extend_from_slice(&(plain.len() as u64).to_le_bytes());
    body.extend_from_slice(&(huffman.len() as u64).to_le_bytes());
    let code = HuffmanCode::from_lengths(&[8; 256]).unwrap();
    let mut w = BitWriter::new();
    for s in huffman {
        handles.push((w.bit_len() as u32, 1));
        let bytes: Vec<u8> = match charset {
            Some(_) => s.encode_utf16().map(|u| u as u8).collect(),
            None => s.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        };
        for b in bytes {
            let (c, l) = code.codeword(b).unwrap();
            w.push(c, l);
        }
    }
    let total_bits = w.bit_len() as u32;
    let buf = w.into_bytes();
    body.push(1);
    body.extend_from_slice(&[0xDD, 0xCC, 0xBB, 0xAA]);
    body.extend_from_slice(&total_bits.to_le_bytes());
    let set = if charset.is_some() {
        bicraft_vertipaq::dictionary::CHARSET_SINGLE
    } else {
        bicraft_vertipaq::dictionary::CHARSET_MULTIPLE
    };
    body.extend_from_slice(&set.to_le_bytes());
    body.extend_from_slice(&(buf.len() as u64).to_le_bytes());
    if let Some(cs) = charset {
        body.push(cs);
    }
    body.extend_from_slice(&12u32.to_le_bytes());
    body.extend_from_slice(&code.encode_array());
    body.extend_from_slice(&(buf.len() as u64).to_le_bytes());
    body.extend_from_slice(&buf);
    body.extend_from_slice(&[0xCD, 0xAB, 0xCD, 0xAB]);

    let mut d = 2i32.to_le_bytes().to_vec();
    d.extend(hash_header(0, 8, 6));
    d.extend_from_slice(&((plain.len() + huffman.len()) as u64).to_le_bytes());
    d.push(1);
    d.extend_from_slice(&longest.to_le_bytes());
    d.extend_from_slice(&2u64.to_le_bytes());
    d.extend(body);
    d.extend_from_slice(&(handles.len() as u64).to_le_bytes());
    d.extend_from_slice(&8u32.to_le_bytes());
    for (o, p) in handles {
        d.extend_from_slice(&o.to_le_bytes());
        d.extend_from_slice(&p.to_le_bytes());
    }
    d
}

fn dictionaries() -> Vec<Vec<u8>> {
    vec![
        long_dictionary(&[10, -20, 30, i64::MAX], 8),
        long_dictionary(&[1, 2, 3], 4),
        real_dictionary(&[0.5, -1.25, 1e300]),
        string_dictionary(&["North", "South", ""], &["East", "West"], None),
        string_dictionary(&["a"], &["abc", "zz"], Some(0)),
    ]
}

fn hidx(entries: &[(u32, u32)], bins: usize, locals: u32, with_stats: bool) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&(-1i32).to_le_bytes());
    v.extend_from_slice(&8u32.to_le_bytes());
    v.extend_from_slice(&(16 + 8 * locals).to_le_bytes());
    v.extend_from_slice(&locals.to_le_bytes());
    v.extend_from_slice(&(bins as u64).to_le_bytes());
    v.extend_from_slice(&(entries.len() as i64).to_le_bytes());
    v.extend_from_slice(&((bins as i64) - 1).to_le_bytes());
    v.push(u8::from(with_stats));
    if with_stats {
        for x in [entries.len() as u64, bins as u64, 0, 0, u64::from(locals), 0, 2] {
            v.extend_from_slice(&x.to_le_bytes());
        }
        for x in [4u32, 1, 2] {
            v.extend_from_slice(&x.to_le_bytes());
        }
    }
    // A bin's count covers all its entries; those beyond its local ones go to the overflow
    // list, bin by bin (the engine's layout).
    let mut table: Vec<Vec<(u32, u32)>> = vec![Vec::new(); bins];
    for &(h, k) in entries {
        table[(h as usize) % bins].push((h, k));
    }
    let overflow: Vec<(u32, u32)> = table
        .iter()
        .flat_map(|b| b.iter().skip(locals as usize).copied())
        .collect();
    for b in &table {
        v.extend_from_slice(&0u64.to_le_bytes());
        v.extend_from_slice(&(b.len() as u32).to_le_bytes());
        for k in 0..locals as usize {
            let (h, key) = b.get(k).copied().unwrap_or((0, 0));
            v.extend_from_slice(&h.to_le_bytes());
            v.extend_from_slice(&key.to_le_bytes());
        }
        v.extend_from_slice(&0u32.to_le_bytes());
    }
    v.extend_from_slice(&(overflow.len() as u64).to_le_bytes());
    for (h, k) in overflow {
        v.extend_from_slice(&h.to_le_bytes());
        v.extend_from_slice(&k.to_le_bytes());
    }
    v
}

/// The encoding a `vertipaq_column` input asks for (its first byte).
fn encoding(selector: u8) -> ColumnEncoding {
    let e = match selector % 8 {
        0 => ColumnEncoding::hash(StorageType::Long),
        1 => ColumnEncoding::hash(StorageType::Real),
        2 => ColumnEncoding::hash(StorageType::String),
        3 => ColumnEncoding::value(-3, 1.0, StorageType::Long),
        4 => ColumnEncoding::value(-2, 4.0, StorageType::Real),
        5 => ColumnEncoding::raw(),
        6 => ColumnEncoding::value(-2, 0.0001, StorageType::Long),
        _ => ColumnEncoding::value(i64::MAX - 5, 0.1, StorageType::Long),
    };
    let codes = [0, 2, 6, 8, 9, 10, 11, 17];
    e.with_logical_type(LogicalType::from_code(codes[usize::from(selector >> 5)]))
}

const BIN_DICT: &[&[u8]] = &[
    b"\xff\xff\xff\xff",
    b"\x00\x00\x00\x00\x00\x00\x00\x00",
    b"\xff\xff\xff\xff\xff\xff\xff\x7f",
    b"\xdd\xcc\xbb\xaa",
    b"\xcd\xab\xcd\xab",
];

#[test]
fn vertipaq_idfmeta() {
    let seeds: Vec<Vec<u8>> = columns().into_iter().map(|(_, meta)| meta).collect();
    smoke::run("vertipaq_idfmeta", &seeds, BIN_DICT, 50000, |data| {
        if let Ok(meta) = parse_idfmeta(data) {
            let _ = meta.to_bytes();
        }
    });
}

#[test]
fn vertipaq_idf() {
    let seeds: Vec<Vec<u8>> = columns()
        .iter()
        .map(|(idf, meta)| smoke::frame(&[meta, idf]))
        .collect();
    smoke::run("vertipaq_idf", &seeds, BIN_DICT, 50000, |data| {
        let f = smoke::frames(data, 2);
        if let Ok(meta) = parse_idfmeta(f[0]) {
            let _ = bicraft_vertipaq::split_idf(f[1], &meta);
            if let Ok(ids) = decode_data_ids(f[1], &meta, &LIMITS) {
                assert!(ids.len() as u64 <= LIMITS.max_rows);
            }
        }
    });
}

#[test]
fn vertipaq_dictionary() {
    smoke::run("vertipaq_dictionary", &dictionaries(), BIN_DICT, 50000, |data| {
        let _ = parse_dictionary(data);
    });
}

#[test]
fn vertipaq_hidx() {
    let entries: Vec<(u32, u32)> = (0..20).map(|i| (i * 16 + 1, i + 3)).collect();
    let seeds = vec![
        hidx(&entries, 16, 6, true),
        hidx(&entries[..3], 4, 2, false),
        hidx(&entries, 2, 1, true),
    ];
    smoke::run("vertipaq_hidx", &seeds, BIN_DICT, 50000, |data| {
        let _ = parse_hash_index(data);
    });
}

#[test]
fn vertipaq_column() {
    let mut seeds = Vec::new();
    let dicts = dictionaries();
    for (i, (idf, meta)) in columns().iter().enumerate() {
        for (s, dict) in [
            (0u8, &dicts[0]),
            (3, &dicts[1]),
            (1, &dicts[2]),
            (2, &dicts[3]),
            (5, &dicts[4]),
        ] {
            seeds.push(smoke::frame(&[&[s + (i as u8) * 32], meta, dict, idf]));
        }
    }
    smoke::run("vertipaq_column", &seeds, BIN_DICT, 50000, |data| {
        let f = smoke::frames(data, 4);
        let selector = f[0].first().copied().unwrap_or(0);
        let dict = (!f[2].is_empty()).then_some(f[2]);
        if let Ok(cells) = decode_column_with_limits(f[3], f[1], dict, encoding(selector), &LIMITS) {
            assert!(cells.len() as u64 <= LIMITS.max_rows);
        }
    });
}

// ------------------------------------------------------------------------------------------------
// Deterministic mutation engine. The same block is copied into every crate's `tests/fuzz_smoke.rs`
// (crates cannot share test code without a new crate); keep the copies identical. See
// docs/fuzzing.md.

#[allow(dead_code)]
mod smoke {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    /// Slowest acceptable single input in a debug build: catches pathological slowness.
    const SLOW_INPUT: Duration = Duration::from_secs(3);
    /// No progress for this long is a hang: the watchdog saves the input and aborts the process.
    const HANG: Duration = Duration::from_secs(60);

    /// xorshift64: tiny, deterministic, good enough to pick mutations.
    pub struct Rng(u64);

    impl Rng {
        pub fn new(seed: u64) -> Self {
            Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
        }

        pub fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }

        pub fn below(&mut self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                (self.next() % n as u64) as usize
            }
        }
    }

    const INTERESTING: [u64; 16] = [
        0,
        1,
        2,
        0x7f,
        0x80,
        0xff,
        0x100,
        0x7fff,
        0x8000,
        0xffff,
        0x1_0000,
        0x7fff_ffff,
        0x8000_0000,
        0xffff_ffff,
        0x7fff_ffff_ffff_ffff,
        u64::MAX,
    ];

    /// Applies 1–6 random mutations: bit flips, byte sets, truncation, insertion, deletion,
    /// interesting integers, splices and crossovers with other seeds, chunk copies and
    /// dictionary tokens.
    pub fn mutate(rng: &mut Rng, seeds: &[Vec<u8>], dict: &[&[u8]], data: &mut Vec<u8>, max_len: usize) {
        for _ in 0..1 + rng.below(6) {
            let len = data.len();
            match rng.below(11) {
                0 if len > 0 => {
                    let i = rng.below(len);
                    data[i] ^= 1 << rng.below(8);
                }
                1 if len > 0 => {
                    let i = rng.below(len);
                    data[i] = rng.next() as u8;
                }
                2 => data.truncate(rng.below(len + 1)),
                3 => {
                    let at = rng.below(len + 1);
                    let n = 1 + rng.below(16);
                    let bytes: Vec<u8> = (0..n).map(|_| rng.next() as u8).collect();
                    data.splice(at..at, bytes);
                }
                4 if len > 0 => {
                    let a = rng.below(len);
                    let b = (a + 1 + rng.below(64)).min(len);
                    data.drain(a..b);
                }
                5 if len > 0 => {
                    let bytes = INTERESTING[rng.below(INTERESTING.len())].to_le_bytes();
                    let width = [1usize, 2, 4, 8][rng.below(4)];
                    let at = rng.below(len);
                    for (k, b) in bytes.iter().take(width).enumerate() {
                        if let Some(slot) = data.get_mut(at + k) {
                            *slot = *b;
                        }
                    }
                }
                6 if !seeds.is_empty() => {
                    // Splice: overwrite a range with a slice of another seed.
                    let other = &seeds[rng.below(seeds.len())];
                    let from = rng.below(other.len() + 1);
                    let n = rng.below(other.len() - from + 1).min(256);
                    let at = rng.below(len + 1);
                    let end = (at + rng.below(256)).min(len);
                    data.splice(at..end, other[from..from + n].iter().copied());
                }
                7 if !seeds.is_empty() => {
                    // Crossover: our head, another seed's tail.
                    let other = &seeds[rng.below(seeds.len())];
                    data.truncate(rng.below(len + 1));
                    let from = rng.below(other.len() + 1);
                    data.extend_from_slice(&other[from..]);
                }
                8 if len > 1 => {
                    let a = rng.below(len);
                    let n = (1 + rng.below(64)).min(len - a);
                    let chunk = data[a..a + n].to_vec();
                    let at = rng.below(len + 1);
                    data.splice(at..at, chunk);
                }
                9 if !dict.is_empty() => {
                    let token = dict[rng.below(dict.len())];
                    let at = rng.below(len + 1);
                    data.splice(at..at, token.iter().copied());
                }
                10 if len > 0 => {
                    let i = rng.below(len);
                    data[i] = data[i].wrapping_add(rng.below(5) as u8).wrapping_sub(2);
                }
                _ => {}
            }
        }
        data.truncate(max_len);
    }

    struct Watch {
        progress: AtomicU64,
        done: AtomicBool,
        input: Mutex<Vec<u8>>,
    }

    fn fnv(s: &str) -> u64 {
        s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
        })
    }

    fn save(target: &str, kind: &str, input: &[u8]) -> PathBuf {
        let path = std::env::temp_dir().join(format!("bicraft-fuzz-smoke-{target}-{kind}.bin"));
        let _ = std::fs::write(&path, input);
        path
    }

    /// `BICRAFT_FUZZ_SEED_DIR=<dir>` writes the seeds to `<dir>/<target>/` (how the cargo-fuzz
    /// corpus under `fuzz/corpus/` is generated).
    fn dump_seeds(target: &str, seeds: &[Vec<u8>]) {
        let Some(dir) = bicraft_env::var_os("BICRAFT_FUZZ_SEED_DIR") else {
            return;
        };
        let dir = PathBuf::from(dir).join(target);
        std::fs::create_dir_all(&dir).unwrap();
        for (i, s) in seeds.iter().enumerate() {
            std::fs::write(dir.join(format!("seed-{i:03}")), s).unwrap();
        }
    }

    fn watchdog(target: &str, watch: &Watch) {
        let mut last = u64::MAX;
        let mut since = Instant::now();
        while !watch.done.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(50));
            let now = watch.progress.load(Ordering::Relaxed);
            if now != last {
                last = now;
                since = Instant::now();
            } else if since.elapsed() > HANG {
                let input = watch.input.lock().map(|g| g.clone()).unwrap_or_default();
                let path = save(target, "hang", &input);
                eprintln!(
                    "{target}: no progress for {HANG:?}; input saved to {}",
                    path.display()
                );
                std::process::exit(1);
            }
        }
    }

    /// Runs every seed, then `iters` mutated inputs, through `exercise`. Fails on a panic or on
    /// an input slower than [`SLOW_INPUT`]; the offending input is saved under the temp dir.
    /// `BICRAFT_FUZZ_SMOKE_ITERS` overrides `iters`.
    pub fn run(target: &str, seeds: &[Vec<u8>], dict: &[&[u8]], iters: usize, exercise: impl Fn(&[u8])) {
        assert!(!seeds.is_empty(), "{target}: no seeds");
        dump_seeds(target, seeds);
        // `BICRAFT_FUZZ_REPLAY=<file>` runs one saved input (a reproducer) and nothing else.
        if let Some(path) = bicraft_env::var_os("BICRAFT_FUZZ_REPLAY") {
            exercise(&std::fs::read(path).unwrap());
            return;
        }
        let iters = bicraft_env::var("BICRAFT_FUZZ_SMOKE_ITERS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(iters);
        let max_len = seeds.iter().map(Vec::len).max().unwrap_or(0) * 2 + 64;
        let watch = Arc::new(Watch {
            progress: AtomicU64::new(0),
            done: AtomicBool::new(false),
            input: Mutex::new(Vec::new()),
        });
        let dog = {
            let watch = Arc::clone(&watch);
            let target = target.to_string();
            std::thread::spawn(move || watchdog(&target, &watch))
        };
        let mut rng = Rng::new(fnv(target));
        let start = Instant::now();
        let mut slowest = (Duration::ZERO, Vec::new());
        for i in 0..seeds.len() + iters {
            let input = match seeds.get(i) {
                Some(seed) => seed.clone(),
                None => {
                    let mut data = seeds[rng.below(seeds.len())].clone();
                    mutate(&mut rng, seeds, dict, &mut data, max_len);
                    data
                }
            };
            if let Ok(mut slot) = watch.input.lock() {
                slot.clone_from(&input);
            }
            let t = Instant::now();
            let outcome = catch_unwind(AssertUnwindSafe(|| exercise(&input)));
            let took = t.elapsed();
            watch.progress.fetch_add(1, Ordering::Relaxed);
            if outcome.is_err() {
                watch.done.store(true, Ordering::Relaxed);
                let path = save(target, "crash", &input);
                panic!(
                    "{target}: input #{i} panicked; reproducer saved to {}",
                    path.display()
                );
            }
            if took > slowest.0 {
                slowest = (took, input);
            }
        }
        watch.done.store(true, Ordering::Relaxed);
        let _ = dog.join();
        eprintln!(
            "{target}: {} inputs in {:.2?}, slowest {:.2?}",
            seeds.len() + iters,
            start.elapsed(),
            slowest.0
        );
        if slowest.0 > SLOW_INPUT {
            let path = save(target, "slow", &slowest.1);
            panic!(
                "{target}: one input took {:.2?} (budget {SLOW_INPUT:?}); saved to {}",
                slowest.0,
                path.display()
            );
        }
    }

    /// Splits `data` into `n` frames; all but the last carry a u32 LE length prefix (clamped to
    /// what is left). The fuzz targets use the same framing.
    pub fn frames(data: &[u8], n: usize) -> Vec<&[u8]> {
        let mut out = Vec::with_capacity(n);
        let mut rest = data;
        for _ in 1..n {
            let (len, tail) = match rest.split_first_chunk::<4>() {
                Some((l, t)) => (u32::from_le_bytes(*l) as usize, t),
                None => (0, &[][..]),
            };
            let len = len.min(tail.len());
            out.push(&tail[..len]);
            rest = &tail[len..];
        }
        out.push(rest);
        out
    }

    /// The inverse of [`frames`].
    pub fn frame(parts: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        for (i, p) in parts.iter().enumerate() {
            if i + 1 < parts.len() {
                out.extend_from_slice(&(p.len() as u32).to_le_bytes());
            }
            out.extend_from_slice(p);
        }
        out
    }

    /// Documents as `[u16 LE name length][name][u32 LE data length][data]`, repeated (lengths
    /// clamped, at most 4096 documents). The fuzz targets use the same encoding.
    pub fn docs(mut data: &[u8]) -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        while !data.is_empty() && out.len() < 4096 {
            let (n, rest) = match data.split_first_chunk::<2>() {
                Some((l, t)) => (usize::from(u16::from_le_bytes(*l)), t),
                None => (0, &[][..]),
            };
            let n = n.min(rest.len());
            let name = String::from_utf8_lossy(&rest[..n]).into_owned();
            let rest = &rest[n..];
            let (m, rest) = match rest.split_first_chunk::<4>() {
                Some((l, t)) => (u32::from_le_bytes(*l) as usize, t),
                None => (0, &[][..]),
            };
            let m = m.min(rest.len());
            out.push((name, rest[..m].to_vec()));
            data = &rest[m..];
        }
        out
    }

    /// The inverse of [`docs`].
    pub fn pack_docs(docs: &[(String, Vec<u8>)]) -> Vec<u8> {
        let mut out = Vec::new();
        for (name, data) in docs {
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(data);
        }
        out
    }

    /// Reads every file under `dir` as `(path relative to dir with '/' separators, bytes)`,
    /// sorted by path.
    pub fn read_tree(dir: &std::path::Path) -> Vec<(String, Vec<u8>)> {
        fn walk(root: &std::path::Path, dir: &std::path::Path, out: &mut Vec<(String, Vec<u8>)>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(root, &path, out);
                } else {
                    let rel = path
                        .strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/");
                    out.push((rel, std::fs::read(&path).unwrap()));
                }
            }
        }
        let mut out = Vec::new();
        walk(dir, dir, &mut out);
        out.sort();
        out
    }

    /// JSON-ish tokens for text formats.
    pub const JSON_DICT: &[&[u8]] = &[
        b"{",
        b"}",
        b"[",
        b"]",
        b"\"",
        b":",
        b",",
        b"null",
        b"true",
        b"-1",
        b"1e999",
        b"0.5",
        b"\\u0000",
        b"\\\"",
        b"{\"\":",
        b"[[[[",
        b"\"name\"",
        b"\"config\"",
        b"\"expression\"",
    ];
}
