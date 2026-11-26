//! Shared helpers for the integration tests: the deterministic input generators (mirroring
//! the C driver that produced `tests/vectors/`, see the crate README) and the manifest.
#![allow(dead_code, clippy::indexing_slicing)]

use std::path::PathBuf;

/// splitmix64, as in the C driver.
pub struct SplitMix(pub u64);

impl SplitMix {
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

const WORDS: [&str; 12] = [
    "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta", "iota", "kappa", "lambda", "mu",
];

fn put(b: &mut Vec<u8>, cap: usize, v: u8) {
    if b.len() < cap {
        b.push(v);
    }
}

fn gen_text(rng: &mut SplitMix, b: &mut Vec<u8>, size: usize, cap: usize) {
    while b.len() < size {
        let w = WORDS[(rng.next_u64() % 12) as usize];
        for &c in w.as_bytes() {
            if b.len() >= size {
                break;
            }
            put(b, cap, c);
        }
        let sep = if rng.next_u64().is_multiple_of(8) {
            b'\n'
        } else {
            b' '
        };
        put(b, cap, sep);
    }
}

/// Regenerate a test input (same algorithm as `generate()` in the C driver).
pub fn generate(gen_name: &str, size: usize, seed: u64) -> Vec<u8> {
    let mut rng = SplitMix(seed);
    let mut b: Vec<u8> = Vec::with_capacity(size);
    match gen_name {
        "zeros" => b.resize(size, 0),
        "random" => {
            while b.len() < size {
                let v = (rng.next_u64() >> 56) as u8;
                put(&mut b, size, v);
            }
        }
        "text" => gen_text(&mut rng, &mut b, size, size),
        "struct" => {
            let mut id: u32 = 0;
            while b.len() < size {
                let cat = (rng.next_u64() % 2) as u8;
                let amount = (rng.next_u64() % 16) as u32;
                let mut rec = [0u8; 16];
                rec[..4].copy_from_slice(&id.to_le_bytes());
                rec[4] = cat;
                rec[8..12].copy_from_slice(&amount.to_le_bytes());
                for v in rec {
                    put(&mut b, size, v);
                }
                id = (id + 1) & 255;
            }
        }
        "mixed" => {
            let mut last_off: u64 = 1;
            while b.len() < size {
                match rng.next_u64() % 4 {
                    0 => {
                        let n = 1 + rng.next_u64() % 64;
                        for _ in 0..n {
                            let v = (rng.next_u64() >> 56) as u8;
                            put(&mut b, size, v);
                        }
                    }
                    1 => {
                        let lim = (b.len() + 16 + (rng.next_u64() % 497) as usize).min(size);
                        gen_text(&mut rng, &mut b, lim, size);
                    }
                    2 => {
                        if b.is_empty() {
                            continue;
                        }
                        let mut off = if !rng.next_u64().is_multiple_of(2) {
                            last_off
                        } else {
                            1 + rng.next_u64() % (b.len().min(70000) as u64)
                        };
                        if off > b.len() as u64 {
                            off = b.len() as u64;
                        }
                        last_off = off;
                        let n = 3 + rng.next_u64() % 300;
                        for _ in 0..n {
                            let v = b[b.len() - off as usize];
                            put(&mut b, size, v);
                        }
                    }
                    _ => {
                        let v = (rng.next_u64() >> 56) as u8;
                        let n = 1 + rng.next_u64() % 2000;
                        for _ in 0..n {
                            put(&mut b, size, v);
                        }
                    }
                }
            }
        }
        other => panic!("unknown generator {other}"),
    }
    b
}

/// One line of `tests/vectors/manifest.txt`.
#[derive(Debug, Clone)]
pub struct Vector {
    pub name: String,
    pub kind: String,
    pub gen_name: String,
    pub size: usize,
    pub seed: u64,
    pub max_window: u32,
    pub params: bicraft_xpress9::EncoderParams,
    pub chunk: usize,
    /// prefix threads, prefix chunks per thread, main threads, main chunks per thread
    pub groups: [u64; 4],
}

pub fn vectors_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("vectors")
}

pub fn manifest() -> Vec<Vector> {
    let text = std::fs::read_to_string(vectors_dir().join("manifest.txt")).expect("manifest");
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            let n = |i: usize| -> u64 { f[i].parse().expect("number") };
            Vector {
                name: f[0].to_string(),
                kind: f[1].to_string(),
                gen_name: f[2].to_string(),
                size: n(3) as usize,
                seed: n(4),
                max_window: n(5) as u32,
                params: bicraft_xpress9::EncoderParams {
                    window_size_log2: n(6) as u32,
                    mtf_entry_count: n(7) as u32,
                    lookup_depth: n(8) as u32,
                    optimization_level: n(9) as u32,
                    ptr_min_match_length: n(10) as u32,
                    mtf_min_match_length: n(11) as u32,
                    max_stream_length: n(12) as u32,
                },
                chunk: n(13) as usize,
                groups: [n(14), n(15), n(16), n(17)],
            }
        })
        .collect()
}

pub fn read_vector(v: &Vector) -> Vec<u8> {
    std::fs::read(vectors_dir().join(format!("{}.x9", v.name))).expect("vector file")
}

/// The synthetic banner used by the C driver (not a Microsoft string).
pub fn synthetic_banner(multithreaded: bool) -> Vec<u8> {
    let text = if multithreaded {
        "BIcraft synthetic test files: multithreaded XPress9"
    } else {
        "BIcraft synthetic test files: single-threaded XPress9"
    };
    let mut out = vec![0u8; 102];
    for (i, c) in text.bytes().take(50).enumerate() {
        out[2 * i] = c;
    }
    out
}

/// Re-create a vector's compressed bytes with the Rust encoder, the same way the C driver did.
pub fn rust_build(v: &Vector, input: &[u8]) -> Vec<u8> {
    use bicraft_xpress9::{DEFAULT_SESSION_SEED, Encoder};
    let mut enc = Encoder::new(v.max_window).expect("encoder");
    let chunks = |chunk: usize| -> Vec<&[u8]> { input.chunks(chunk.max(1)).collect() };
    match v.kind.as_str() {
        "stream" => {
            enc.start_session(&v.params, DEFAULT_SESSION_SEED)
                .expect("session");
            if v.chunk == 0 {
                enc.compress(input, true).expect("compress")
            } else {
                let mut out = Vec::new();
                for c in chunks(v.chunk) {
                    out.extend(enc.compress(c, true).expect("compress"));
                }
                out
            }
        }
        "dm_single" => {
            let mut out = synthetic_banner(false);
            enc.start_session(&v.params, DEFAULT_SESSION_SEED)
                .expect("session");
            for c in chunks(v.chunk) {
                let body = enc.compress(c, true).expect("compress");
                out.extend((c.len() as u32).to_le_bytes());
                out.extend((body.len() as u32).to_le_bytes());
                out.extend(body);
            }
            out
        }
        "dm_multi" => {
            let mut out = synthetic_banner(true);
            let [pt, pp, mt, mp] = v.groups;
            for x in [mp, pp, pt, mt, v.chunk as u64] {
                out.extend(x.to_le_bytes());
            }
            let all = chunks(v.chunk);
            let mut it = all.into_iter();
            for (threads, per) in [(pt, pp), (mt, mp)] {
                for _ in 0..threads {
                    enc.start_session(&v.params, DEFAULT_SESSION_SEED)
                        .expect("session");
                    for _ in 0..per {
                        let c = it.next().expect("chunk");
                        let body = enc.compress(c, true).expect("compress");
                        out.extend((c.len() as u32).to_le_bytes());
                        out.extend((body.len() as u32).to_le_bytes());
                        out.extend(body);
                    }
                }
            }
            out
        }
        other => panic!("unknown kind {other}"),
    }
}
