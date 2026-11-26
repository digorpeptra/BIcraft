//! Stable-Rust fuzz smoke tests of the engine's own file readers (M11.36): `.vpax`, `.pbids`,
//! M text, `.pqt`, the CDM `model.json` and a Dataflow's `queryMetadata.json` with its
//! `mashup.pq` (`bicraft_engine::readers`), and the `import_readers` cargo-fuzz target's
//! framing (docs/fuzzing.md). Seeds are the synthetic fixtures under
//! `tests/fixtures/import-sources/` and ZIPs built here.

use bicraft_engine::readers::{Kind, read_bytes};

fn fixture(name: &str) -> Vec<u8> {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/import-sources")
        .join(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn zip(parts: &[(&str, &[u8])]) -> Vec<u8> {
    let mut pkg = bicraft_opc::Package::default();
    for (n, d) in parts {
        pkg.put(n, d.to_vec(), false);
    }
    pkg.to_bytes().unwrap()
}

fn vpax_seed() -> Vec<u8> {
    let dax = br#"{"$id":"1","ModelName":"M","Tables":{"$id":"2","$values":[{"$id":"3","TableName":"T","RowsCount":2,"Columns":{"$id":"4","$values":[{"$id":"5","ColumnName":"A","DataType":"Int64","TotalSize":10},{"$id":"6","ColumnName":"B","DataType":"String"}]},"Measures":{"$id":"7","$values":[{"$id":"8","MeasureName":"N","MeasureExpression":"COUNTROWS(T)"}]}}]},"Relationships":{"$id":"9","$values":[{"$id":"10","FromColumn":{"$ref":"5"},"ToColumn":{"$ref":"6"}}]}}"#;
    zip(&[("DaxModel.json", dax)])
}

fn pqt_seed() -> Vec<u8> {
    zip(&[
        ("MashupDocument.pq", &fixture("queries.pq")),
        (
            "MashupMetadata.json",
            br#"{"queriesMetadata":{"Customers":{"loadEnabled":true}}}"#,
        ),
    ])
}

/// Every reader on one input (the framing of the `import_readers` cargo-fuzz target: the
/// first frame picks the reader, the last one is a Dataflow's M document).
fn exercise(data: &[u8]) {
    let f = smoke::frames(data, 3);
    let (pick, body, doc) = (f[0], f[1], f[2]);
    let kind = match pick.first().copied().unwrap_or(0) % 6 {
        0 => Kind::Vpax,
        1 => Kind::Pbids,
        2 => Kind::M,
        3 => Kind::Pqt,
        4 => Kind::Cdm,
        _ => Kind::Dataflow,
    };
    if let Ok(imp) = read_bytes(kind, "fuzz", body, Some(doc)) {
        // what is read is a model the TMSL writer and reader accept
        let text = bicraft_tmsl::write(&imp.model);
        assert!(
            bicraft_tmsl::read(&text).is_ok(),
            "{kind:?}: the model does not round-trip"
        );
    }
}

#[test]
fn import_readers() {
    let pick = |k: u8, body: &[u8], doc: &[u8]| smoke::frame(&[&[k], body, doc]);
    let mashup = fixture("dataflow/mashup.pq");
    let seeds = vec![
        pick(0, &vpax_seed(), b""),
        pick(1, &fixture("sales.pbids"), b""),
        pick(2, &fixture("queries.pq"), b""),
        pick(2, &fixture("single.m"), b""),
        pick(3, &pqt_seed(), b""),
        pick(4, &fixture("model.json"), b""),
        pick(5, &fixture("dataflow/queryMetadata.json"), &mashup),
    ];
    smoke::run("import_readers", &seeds, smoke::JSON_DICT, 3000, exercise);
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
