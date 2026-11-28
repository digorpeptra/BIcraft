//! `cargo xtask concurrency-bench`: parallel reads on one workspace of `bicraft-cli serve`
//! (`docs/web-deployment.md`, "Concurrency on one workspace"; `docs/benchmarks.md`).
//!
//! On a PBIX of the engine bench (`engine-bench gen`, e.g. 10 M rows), for each query: N
//! identical `evaluate_dax_query` calls sent at once to one workspace, with the reads
//! serialized (`--max-concurrent-reads 1`) and side by side (`--max-concurrent-reads N`), cold
//! (`dax.clear_cache` before each batch) and warm; the wall time of each batch, the answers of
//! both modes compared byte for byte, and one query alone (single-query latency, optionally
//! against a base binary). Peak memory of the server: one cold query against N at once.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::deploy::client::{self, Endpoint};

const USAGE: &str = "\
usage: cargo xtask concurrency-bench --bin BICRAFT --data FILE.pbix [--base-bin BICRAFT]
                                     [--parallel N] [--rounds R] [--queries Q01,Q03,...]
                                     [--queries-file FILE] [--json OUT]

  --bin         the bicraft-cli binary to measure (a release build)
  --base-bin    another build (e.g. main) for the single-query latency A/B
  --data        an engine-bench PBIX (xtask/engine-bench; regenerate with engine-bench gen)
  --parallel    identical queries sent at once (default 4)
  --rounds      batches per query and mode (default 3; the median is reported)
  --queries     query ids of the queries file (default Q01,Q03,Q04,Q05,Q06,Q07)
  --long        a query id (or DAX text) run alone, then while it runs each query once (serialized
                and side by side): what a short read waits for behind a long one
";

struct Opts {
    bin: PathBuf,
    base: Option<PathBuf>,
    data: PathBuf,
    parallel: usize,
    rounds: usize,
    ids: Vec<String>,
    queries_file: PathBuf,
    json: Option<PathBuf>,
    /// A long query: the short ones' latency while it runs (`--long`).
    long: Option<String>,
}

fn parse(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts {
        bin: PathBuf::new(),
        base: None,
        data: PathBuf::new(),
        parallel: 4,
        rounds: 3,
        ids: ["Q01", "Q03", "Q04", "Q05", "Q06", "Q07"]
            .map(String::from)
            .to_vec(),
        queries_file: crate::root()
            .join("xtask")
            .join("engine-bench")
            .join("queries.json"),
        json: None,
        long: None,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut v = || {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{a} needs a value\n{USAGE}"))
        };
        match a.as_str() {
            "--bin" => o.bin = PathBuf::from(v()?),
            "--base-bin" => o.base = Some(PathBuf::from(v()?)),
            "--data" => o.data = PathBuf::from(v()?),
            "--parallel" => o.parallel = v()?.parse().map_err(|e| format!("--parallel: {e}"))?,
            "--rounds" => o.rounds = v()?.parse().map_err(|e| format!("--rounds: {e}"))?,
            "--queries" => o.ids = v()?.split(',').map(str::to_owned).collect(),
            "--queries-file" => o.queries_file = PathBuf::from(v()?),
            "--json" => o.json = Some(PathBuf::from(v()?)),
            "--long" => o.long = Some(v()?),
            "-h" | "--help" => return Err(USAGE.into()),
            other => return Err(format!("unknown argument {other}\n{USAGE}")),
        }
    }
    if o.bin.as_os_str().is_empty() || o.data.as_os_str().is_empty() {
        return Err(USAGE.into());
    }
    o.parallel = o.parallel.clamp(1, 64);
    o.rounds = o.rounds.max(1);
    Ok(o)
}

/// A running `bicraft-cli serve` on a temporary root holding one workspace with the data file.
struct Server {
    child: Child,
    ep: Endpoint,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start(bin: &Path, root: &Path, reads: usize) -> Result<Server, String> {
    let mut c = Command::new(bin);
    c.args(["serve", "--root"])
        .arg(root)
        .args(["--listen", "127.0.0.1:0", "--queue", "64"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    // 0: the binary's default (a base build may not know the flag)
    if reads > 0 {
        c.args(["--max-concurrent-reads", &reads.to_string()]);
    }
    for (k, _) in std::env::vars_os() {
        let k = k.to_string_lossy().into_owned();
        if k.starts_with("BICRAFT_") {
            c.env_remove(k);
        }
    }
    let mut child = c.spawn().map_err(|e| format!("{}: {e}", bin.display()))?;
    let stderr = child.stderr.take().ok_or("no stderr")?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if let Some(i) = line.find("http://") {
                let addr = line
                    .get(i + 7..)
                    .unwrap_or_default()
                    .split('/')
                    .next()
                    .unwrap_or_default();
                let _ = tx.send(addr.to_owned());
            }
        }
    });
    let addr = rx
        .recv_timeout(Duration::from_secs(30))
        .map_err(|_| "bicraft-cli serve printed no endpoint".to_owned())?;
    let ep = Endpoint {
        addr: addr.parse().map_err(|e| format!("{addr}: {e}"))?,
        host: addr.clone(),
        token: String::new(),
        workspace: "bench".into(),
    };
    Ok(Server { child, ep })
}

/// A tool call without credentials (a loopback server has no authentication).
fn call(ep: &Endpoint, tool: &str, args: Value) -> Result<Value, String> {
    let r = client::mcp(
        ep,
        "tools/call",
        json!({"name": tool, "arguments": args}),
        false,
        false,
    )?;
    if r.status != 200 {
        return Err(format!("{tool}: HTTP {} {}", r.status, r.text()));
    }
    let m = r.message()?;
    let res = m.get("result").cloned().ok_or_else(|| format!("{tool}: {m}"))?;
    if res.get("isError") == Some(&Value::Bool(true)) {
        return Err(format!(
            "{tool}: {}",
            res.get("structuredContent").cloned().unwrap_or_default()
        ));
    }
    Ok(res.get("structuredContent").cloned().unwrap_or_default())
}

/// The answer of a query without its timing.
fn answer(ep: &Endpoint, q: &str) -> Result<Value, String> {
    let mut v = call(
        ep,
        "evaluate_dax_query",
        json!({"query": q, "timeout_ms": 600_000}),
    )?;
    if let Some(o) = v.as_object_mut() {
        o.remove("elapsed_ms");
    }
    Ok(v)
}

/// `n` identical queries at once: the batch's wall time and the answers.
fn batch(ep: &Endpoint, q: &str, n: usize) -> Result<(Duration, Vec<Value>), String> {
    let t = Instant::now();
    let answers: Vec<Result<Value, String>> = std::thread::scope(|s| {
        let hs: Vec<_> = (0..n).map(|_| s.spawn(|| answer(ep, q))).collect();
        hs.into_iter()
            .map(|h| h.join().unwrap_or_else(|_| Err("panicked".into())))
            .collect()
    });
    let took = t.elapsed();
    Ok((took, answers.into_iter().collect::<Result<_, _>>()?))
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v.get(v.len() / 2).copied().unwrap_or(0.0)
}

/// The server's peak memory (working set on Windows, VmHWM on Linux), in MiB.
fn peak_mib(child: &Child) -> Option<f64> {
    let pid = child.id();
    if cfg!(windows) {
        let out = Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!("(Get-Process -Id {pid}).PeakWorkingSet64"),
            ])
            .output()
            .ok()?;
        String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse::<f64>()
            .ok()
            .map(|b| b / 1048576.0)
    } else {
        let s = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
        s.lines()
            .find_map(|l| l.strip_prefix("VmHWM:"))
            .and_then(|v| v.trim().trim_end_matches("kB").trim().parse::<f64>().ok())
            .map(|k| k / 1024.0)
    }
}

fn open(ep: &Endpoint, file: &str) -> Result<(), String> {
    call(ep, "open_file", json!({"path": file})).map(|_| ())
}

pub fn cmd(args: &[String]) -> Result<(), String> {
    let o = parse(args)?;
    let text =
        std::fs::read_to_string(&o.queries_file).map_err(|e| format!("{}: {e}", o.queries_file.display()))?;
    let all: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let queries: Vec<(String, String)> = o
        .ids
        .iter()
        .map(|id| {
            all["queries"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|q| q["id"] == id.as_str())
                .and_then(|q| q["dax"].as_str())
                .map(|d| (id.clone(), d.to_owned()))
                .ok_or_else(|| format!("no query {id}"))
        })
        .collect::<Result<_, _>>()?;
    // the workspace: a hard link to the data file (a copy when links are not possible)
    let root = std::env::temp_dir().join(format!("bicraft-conc-bench-{}", std::process::id()));
    let ws = root.join("bench");
    std::fs::create_dir_all(&ws).map_err(|e| e.to_string())?;
    let file = "bench.pbix";
    if std::fs::hard_link(&o.data, ws.join(file)).is_err() {
        std::fs::copy(&o.data, ws.join(file)).map_err(|e| format!("{}: {e}", o.data.display()))?;
    }
    let out = run(&o, &root, file, &queries);
    let _ = std::fs::remove_dir_all(&root);
    let report = out?;
    if let Some(p) = &o.json {
        std::fs::write(p, serde_json::to_string_pretty(&report).unwrap_or_default())
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn run(o: &Opts, root: &Path, file: &str, queries: &[(String, String)]) -> Result<Value, String> {
    let n = o.parallel;
    let mut rows = Vec::new();
    // 1. serialized and parallel batches, cold and warm, answers compared
    let mut reference: Vec<Value> = Vec::new();
    for (mode, reads) in [("serialized", 1usize), ("parallel", n)] {
        let s = start(&o.bin, root, reads)?;
        open(&s.ep, file)?;
        for (i, (id, q)) in queries.iter().enumerate() {
            let mut cold = Vec::new();
            let mut warm = Vec::new();
            for _ in 0..o.rounds {
                call(&s.ep, "run_command", json!({"command": "dax.clear_cache"}))?;
                let (t, a) = batch(&s.ep, q, n)?;
                cold.push(t.as_secs_f64());
                let (t2, b) = batch(&s.ep, q, n)?;
                warm.push(t2.as_secs_f64());
                for x in a.iter().chain(&b) {
                    match reference.get(i) {
                        Some(r) if r != x => {
                            return Err(format!(
                                "{id} ({mode}): an answer differs from the serialized one"
                            ));
                        }
                        Some(_) => {}
                        None => reference.push(x.clone()),
                    }
                }
            }
            let (c, w) = (median(cold), median(warm));
            eprintln!(
                "concurrency-bench: {id} {mode:>10} x{n}: cold batch {:>8.1} ms ({:.2} q/s), warm batch {:>8.1} ms ({:.2} q/s)",
                c * 1000.0,
                n as f64 / c,
                w * 1000.0,
                n as f64 / w
            );
            rows.push(json!({"query": id, "mode": mode, "n": n, "cold_s": c, "warm_s": w}));
        }
    }
    // 1b. a short read while a long one runs
    let mut behind = Vec::new();
    if let Some(l) = &o.long {
        let long_q = queries_text(&o.queries_file, l)?;
        for (mode, reads) in [("serialized", 1usize), ("parallel", n)] {
            let s = start(&o.bin, root, reads)?;
            open(&s.ep, file)?;
            for (_, q) in queries {
                answer(&s.ep, q)?;
            }
            answer(&s.ep, &long_q)?;
            let t = Instant::now();
            answer(&s.ep, &long_q)?;
            let alone = t.elapsed().as_secs_f64();
            for (id, q) in queries {
                let (short, long_took) = std::thread::scope(|sc| -> Result<(f64, f64), String> {
                    let t0 = Instant::now();
                    let (ep, lq) = (&s.ep, long_q.as_str());
                    let h = sc.spawn(move || answer(ep, lq).map(|_| t0.elapsed().as_secs_f64()));
                    std::thread::sleep(Duration::from_millis(20));
                    let t = Instant::now();
                    answer(&s.ep, q)?;
                    let short = t.elapsed().as_secs_f64();
                    let long_took = h.join().map_err(|_| "panicked".to_owned())??;
                    Ok((short, long_took))
                })?;
                eprintln!(
                    "concurrency-bench: {id} {mode:>10} behind {l} ({:.0} ms alone): short read {:>8.1} ms, long read {:>8.1} ms",
                    alone * 1000.0,
                    short * 1000.0,
                    long_took * 1000.0
                );
                behind.push(json!({"query": id, "mode": mode, "long": l, "long_alone_s": alone, "short_s": short, "long_s": long_took}));
            }
        }
    }
    // 2. one query alone (warm and cold), this binary and the base one
    let mut single = Vec::new();
    for (label, bin) in std::iter::once(("new", &o.bin)).chain(o.base.as_ref().map(|b| ("base", b))) {
        let s = start(bin, root, if label == "base" { 0 } else { n })?;
        open(&s.ep, file)?;
        for (id, q) in queries {
            let mut warm = Vec::new();
            let mut cold = Vec::new();
            answer(&s.ep, q)?;
            for _ in 0..o.rounds.max(5) {
                let t = Instant::now();
                answer(&s.ep, q)?;
                warm.push(t.elapsed().as_secs_f64());
            }
            for _ in 0..o.rounds {
                call(&s.ep, "run_command", json!({"command": "dax.clear_cache"}))?;
                let t = Instant::now();
                answer(&s.ep, q)?;
                cold.push(t.elapsed().as_secs_f64());
            }
            let (w, c) = (median(warm), median(cold));
            eprintln!(
                "concurrency-bench: {id} single {label:>4}: warm {:>8.2} ms, cold {:>8.1} ms",
                w * 1000.0,
                c * 1000.0
            );
            single.push(json!({"query": id, "bin": label, "warm_s": w, "cold_s": c}));
        }
    }
    // 3. peak memory: one cold query, then N cold identical queries at once, each on a fresh server
    let mut memory = Vec::new();
    if let Some((id, q)) = queries.first() {
        for k in [1, n] {
            let s = start(&o.bin, root, n)?;
            open(&s.ep, file)?;
            // the backup opened (a small dimension read): the queries below decode the fact's
            // columns, each fork on its own when they run at once
            answer(&s.ep, "EVALUATE ROW(\"r\", COUNTROWS(VALUES(Magasin[Region])))")?;
            let before = peak_mib(&s.child);
            batch(&s.ep, q, k)?;
            let after = peak_mib(&s.child);
            eprintln!(
                "concurrency-bench: {id} peak memory, {k} cold quer{} at once: {:.0} MiB (after open {:.0} MiB)",
                if k == 1 { "y" } else { "ies" },
                after.unwrap_or(0.0),
                before.unwrap_or(0.0)
            );
            memory.push(json!({"query": id, "at_once": k, "peak_mib": after, "after_open_mib": before}));
        }
    }
    let _ = std::io::stderr().flush();
    Ok(json!({"parallel": n, "batches": rows, "behind": behind, "single": single, "memory": memory}))
}

/// The DAX of query `id` of the queries file (or `id` itself when it is DAX text).
fn queries_text(file: &Path, id: &str) -> Result<String, String> {
    if id.trim_start().to_ascii_uppercase().starts_with("EVALUATE") || id.contains(' ') {
        return Ok(id.to_owned());
    }
    let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let all: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    all["queries"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|q| q["id"] == id)
        .and_then(|q| q["dax"].as_str())
        .map(str::to_owned)
        .ok_or_else(|| format!("no query {id}"))
}
