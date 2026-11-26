//! `cargo xtask bench [--quick] [--real] [--filter TEXT] [--json PATH]`: performance benchmarks.
//!
//! A small harness (no criterion): each case runs once untimed as a warm-up, then up to N timed
//! runs within a time budget, and reports the median, p95 and throughput. Cases are measured
//! in-process (calling the crates directly, like the engine does) and end to end through the
//! release `bicraft` binary (process start included).
//!
//! The in-process cases need the workspace crates, which xtask only links with the `bench`
//! feature, and they are only meaningful in a release build. So `cargo xtask bench` (a debug
//! build without the feature) re-runs itself as
//! `cargo run --release -p xtask --features bench -- bench …`.
//!
//! All inputs are synthetic (see `bench/synth.rs`), generated into a scratch folder under the OS
//! temp dir and deleted at the end. `--real` adds the local-only sample at
//! `$BICRAFT_TESTDATA/sample1/report.pbix` (or `--real-file PATH`); only timings and counts are
//! printed for it.
#![cfg_attr(not(feature = "bench"), allow(dead_code))]

pub mod synth;

#[cfg(feature = "bench")]
mod inproc;
#[cfg(feature = "bench")]
mod vertipaq;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// Command-line options.
#[derive(Debug, Clone, Default)]
pub struct Opts {
    pub quick: bool,
    pub real: bool,
    pub real_file: Option<PathBuf>,
    pub filter: Option<String>,
    pub json: Option<PathBuf>,
    /// Skip `cargo build --release -p bicraft-cli` (the binary must exist).
    pub no_build: bool,
    /// Only the end-to-end (CLI and MCP) cases.
    pub cli_only: bool,
    /// Keep the scratch folder (generated inputs), e.g. to profile one case by hand.
    pub keep: bool,
}

impl Opts {
    pub fn parse(args: &[String]) -> Result<Opts, String> {
        let mut o = Opts::default();
        let mut it = args.iter();
        while let Some(a) = it.next() {
            let mut value = |name: &str| it.next().cloned().ok_or(format!("{name} needs a value"));
            match a.as_str() {
                "--quick" => o.quick = true,
                "--real" => o.real = true,
                "--real-file" => {
                    o.real = true;
                    o.real_file = Some(PathBuf::from(value("--real-file")?));
                }
                "--filter" => o.filter = Some(value("--filter")?.to_ascii_lowercase()),
                "--json" => o.json = Some(PathBuf::from(value("--json")?)),
                "--no-build" => o.no_build = true,
                "--cli-only" => o.cli_only = true,
                "--keep" => o.keep = true,
                other => return Err(format!("bench: unexpected argument `{other}`")),
            }
        }
        Ok(o)
    }
}

pub fn cmd_bench(args: &[String]) -> Result<(), String> {
    let opts = Opts::parse(args)?;
    #[cfg(feature = "bench")]
    {
        run_all(&opts)
    }
    #[cfg(not(feature = "bench"))]
    {
        let _ = opts;
        let mut c = crate::cargo();
        c.args([
            "run",
            "--release",
            "--quiet",
            "-p",
            "xtask",
            "--features",
            "bench",
            "--",
            "bench",
        ]);
        c.args(args);
        crate::run(c, "cargo run --release -p xtask --features bench -- bench")
    }
}

// ---------------------------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------------------------

/// What one run did, for throughput: `items` of `unit` and/or `bytes` processed.
#[derive(Debug, Clone, Default)]
pub struct Work {
    pub items: u64,
    pub unit: &'static str,
    pub bytes: u64,
    /// Sanity facts shown next to the numbers (e.g. `invalid=4`).
    pub note: String,
}

impl Work {
    pub fn items(items: u64, unit: &'static str) -> Work {
        Work {
            items,
            unit,
            ..Work::default()
        }
    }

    pub fn bytes(bytes: u64) -> Work {
        Work {
            bytes,
            ..Work::default()
        }
    }

    pub fn with_bytes(mut self, bytes: u64) -> Work {
        self.bytes = bytes;
        self
    }

    pub fn note(mut self, note: impl Into<String>) -> Work {
        self.note = note.into();
        self
    }
}

#[derive(Debug, Clone)]
pub struct Row {
    pub group: String,
    pub name: String,
    pub runs: usize,
    pub median: f64,
    pub p95: f64,
    pub min: f64,
    pub work: Work,
    pub error: Option<String>,
}

/// Collects the results of every case.
pub struct Bencher {
    pub quick: bool,
    filter: Option<String>,
    group: String,
    pub rows: Vec<Row>,
}

/// `q` (0..=1) quantile of sorted samples, nearest rank.
pub fn quantile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (q * sorted.len() as f64).ceil() as usize;
    sorted
        .get(rank.saturating_sub(1).min(sorted.len() - 1))
        .copied()
        .unwrap_or(0.0)
}

impl Bencher {
    pub fn new(quick: bool, filter: Option<String>) -> Self {
        Bencher {
            quick,
            filter,
            group: String::new(),
            rows: Vec::new(),
        }
    }

    pub fn group(&mut self, name: &str) {
        self.group = name.to_string();
        eprintln!("\n## {name}");
    }

    /// Whether a case (or a group of cases sharing an expensive setup) passes `--filter`.
    pub fn wants(&self, name: &str) -> bool {
        self.filter
            .as_deref()
            .is_none_or(|f| format!("{} {name}", self.group).to_ascii_lowercase().contains(f))
    }

    /// Times `f`: one warm-up run (when `max_runs > 1`), then up to `max_runs` timed runs within
    /// the time budget (at least one; the budget includes the warm-up). `--quick` divides
    /// `max_runs` by 4.
    pub fn case(&mut self, name: &str, max_runs: usize, mut f: impl FnMut() -> Result<Work, String>) {
        self.case_with_setup(name, max_runs, || Ok(()), |()| f());
    }

    /// [`Bencher::case`] with an untimed `setup` before every run (fresh copies of inputs that
    /// the timed part modifies); its result is handed to `f`.
    pub fn case_with_setup<S>(
        &mut self,
        name: &str,
        max_runs: usize,
        mut setup: impl FnMut() -> Result<S, String>,
        mut f: impl FnMut(S) -> Result<Work, String>,
    ) {
        if !self.wants(name) {
            return;
        }
        let max_runs = if self.quick {
            max_runs.div_ceil(4)
        } else {
            max_runs
        }
        .max(1);
        let budget = Duration::from_secs(if self.quick { 4 } else { 20 });
        let mut times = Vec::new();
        let mut work = Work::default();
        let mut error = None;
        let started = Instant::now();
        let mut timed = |times: &mut Vec<f64>, warm_up: bool| -> Result<Option<Work>, String> {
            let input = setup()?;
            let t = Instant::now();
            let w = f(input)?;
            let secs = t.elapsed().as_secs_f64();
            // A slow case (>= 0.5 s) is not dominated by cold caches, so its warm-up run counts
            // as a sample: halves the cost of the expensive cases.
            if !warm_up || secs >= 0.5 {
                times.push(secs);
                return Ok(Some(w));
            }
            Ok(None)
        };
        if max_runs > 1 {
            match timed(&mut times, true) {
                Ok(Some(w)) => work = w,
                Ok(None) => {}
                Err(e) => error = Some(e),
            }
        }
        while error.is_none() && times.len() < max_runs {
            if !times.is_empty() && started.elapsed() > budget {
                break;
            }
            match timed(&mut times, false) {
                Ok(Some(w)) => work = w,
                Ok(None) => {}
                Err(e) => error = Some(e),
            }
        }
        times.sort_by(f64::total_cmp);
        let row = Row {
            group: self.group.clone(),
            name: name.to_string(),
            runs: times.len(),
            median: quantile(&times, 0.5),
            p95: quantile(&times, 0.95),
            min: times.first().copied().unwrap_or(0.0),
            work,
            error,
        };
        match &row.error {
            Some(e) => eprintln!("  {name}: FAILED: {e}"),
            None => eprintln!(
                "  {name}: median {} p95 {} ({} runs) {} {}",
                fmt_secs(row.median),
                fmt_secs(row.p95),
                row.runs,
                throughput(&row),
                row.work.note
            ),
        }
        self.rows.push(row);
    }

    /// Records a case that could not run.
    pub fn skip(&mut self, name: &str, why: &str) {
        if !self.wants(name) {
            return;
        }
        eprintln!("  {name}: skipped: {why}");
        self.rows.push(Row {
            group: self.group.clone(),
            name: name.to_string(),
            runs: 0,
            median: 0.0,
            p95: 0.0,
            min: 0.0,
            work: Work::default(),
            error: Some(format!("skipped: {why}")),
        });
    }
}

pub fn fmt_secs(s: f64) -> String {
    if s >= 1.0 {
        format!("{s:.2} s")
    } else if s >= 1e-3 {
        format!("{:.1} ms", s * 1e3)
    } else {
        format!("{:.1} µs", s * 1e6)
    }
}

fn fmt_rate(per_sec: f64) -> String {
    if per_sec >= 1e6 {
        format!("{:.1} M", per_sec / 1e6)
    } else if per_sec >= 1e3 {
        format!("{:.1} k", per_sec / 1e3)
    } else {
        format!("{per_sec:.1} ")
    }
}

pub fn throughput(row: &Row) -> String {
    if row.median <= 0.0 {
        return String::new();
    }
    let mut parts = Vec::new();
    if row.work.items > 0 {
        parts.push(format!(
            "{}{}/s",
            fmt_rate(row.work.items as f64 / row.median),
            row.work.unit
        ));
    }
    if row.work.bytes > 0 {
        parts.push(format!("{:.1} MB/s", row.work.bytes as f64 / 1e6 / row.median));
    }
    parts.join(", ")
}

/// The results as Markdown tables, one per group.
pub fn markdown(rows: &[Row]) -> String {
    let mut out = String::new();
    let mut group = None;
    for r in rows {
        if group != Some(&r.group) {
            group = Some(&r.group);
            let _ = write!(
                out,
                "\n### {}\n\n| Case | Runs | Median | p95 | Throughput | Notes |\n|---|---:|---:|---:|---|---|\n",
                r.group
            );
        }
        match &r.error {
            Some(e) => {
                let _ = writeln!(out, "| {} | 0 | | | | {} |", r.name, e.replace('|', "/"));
            }
            None => {
                let _ = writeln!(
                    out,
                    "| {} | {} | {} | {} | {} | {} |",
                    r.name,
                    r.runs,
                    fmt_secs(r.median),
                    fmt_secs(r.p95),
                    throughput(r),
                    r.work.note
                );
            }
        }
    }
    out
}

fn rows_json(rows: &[Row], machine: &Value, opts: &Opts) -> Value {
    json!({
        "machine": machine,
        "quick": opts.quick,
        "results": rows.iter().map(|r| json!({
            "group": r.group, "name": r.name, "runs": r.runs,
            "median_s": r.median, "p95_s": r.p95, "min_s": r.min,
            "items": r.work.items, "unit": r.work.unit, "bytes": r.work.bytes,
            "note": r.work.note, "error": r.error,
        })).collect::<Vec<_>>(),
    })
}

// ---------------------------------------------------------------------------------------------
// Environment
// ---------------------------------------------------------------------------------------------

fn command_line(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program)
        .args(args)
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !s.is_empty()).then_some(s)
}

/// CPU, OS and toolchain, for the report header.
pub fn machine_info() -> Value {
    let cpu = if cfg!(windows) {
        command_line(
            "powershell.exe",
            &["-NoProfile", "-Command", "(Get-CimInstance Win32_Processor).Name"],
        )
        .or_else(|| std::env::var("PROCESSOR_IDENTIFIER").ok())
    } else {
        std::fs::read_to_string("/proc/cpuinfo").ok().and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("model name"))
                .and_then(|l| l.split_once(':'))
                .map(|(_, v)| v.trim().to_string())
        })
    };
    let os = if cfg!(windows) {
        command_line("cmd", &["/C", "ver"])
    } else {
        command_line("uname", &["-sr"])
    };
    let threads = std::thread::available_parallelism().map_or(0, std::num::NonZero::get);
    json!({
        "cpu": cpu.unwrap_or_else(|| "unknown".into()),
        "threads": threads,
        "os": os.unwrap_or_else(|| std::env::consts::OS.into()),
        "arch": std::env::consts::ARCH,
        "rustc": command_line("rustc", &["-V"]).unwrap_or_else(|| "unknown".into()),
    })
}

/// A scratch folder under the OS temp dir, removed on drop.
pub struct Scratch(pub PathBuf);

impl Scratch {
    pub fn new() -> Result<Scratch, String> {
        let dir = std::env::temp_dir().join(format!("bicraft-bench-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        Ok(Scratch(dir))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Recursively copies a folder.
pub fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    for entry in std::fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let target = to.join(entry.file_name());
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target).map_err(|e| format!("{}: {e}", target.display()))?;
        }
    }
    Ok(())
}

pub fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}

// ---------------------------------------------------------------------------------------------
// The CLI binary
// ---------------------------------------------------------------------------------------------

/// The release `bicraft` binary, built first unless `--no-build`.
pub fn cli_binary(opts: &Opts) -> Result<PathBuf, String> {
    if !opts.no_build {
        crate::run_cargo(&["build", "--release", "--quiet", "-p", "bicraft-cli"])?;
    }
    let meta = crate::metadata()?;
    let target = meta
        .get("target_directory")
        .and_then(Value::as_str)
        .ok_or("cargo metadata: no target_directory")?;
    let bin = Path::new(target)
        .join("release")
        .join(format!("bicraft{}", std::env::consts::EXE_SUFFIX));
    if bin.is_file() {
        Ok(bin)
    } else {
        Err(format!(
            "{} not found (build it or drop --no-build)",
            bin.display()
        ))
    }
}

/// Runs the CLI; `ok_codes` are the accepted exit codes. Returns stdout.
pub fn cli(bin: &Path, args: &[&str], ok_codes: &[i32]) -> Result<Vec<u8>, String> {
    let out = Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("spawn {}: {e}", bin.display()))?;
    let code = out.status.code().unwrap_or(-1);
    if ok_codes.contains(&code) {
        Ok(out.stdout)
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(format!(
            "bicraft {} exited with {code}: {}",
            args.first().copied().unwrap_or(""),
            err.lines().next().unwrap_or("")
        ))
    }
}

/// Speaks MCP to `bicraft mcp`: `initialize`, then `calls` (tool name, arguments) in one stdio
/// session. Returns the number of tool results that were errors.
pub fn mcp_session(bin: &Path, calls: &[(String, Value)]) -> Result<usize, String> {
    use std::io::{BufRead as _, BufReader, Write as _};
    let mut child = Command::new(bin)
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn mcp: {e}"))?;
    let mut input = String::new();
    let init = json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {
        "protocolVersion": "2025-06-18", "capabilities": {},
        "clientInfo": {"name": "bicraft-bench", "version": "0"}}});
    let _ = writeln!(input, "{init}");
    let _ = writeln!(
        input,
        "{}",
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
    );
    for (i, (name, args)) in calls.iter().enumerate() {
        let req = json!({"jsonrpc": "2.0", "id": i + 1, "method": "tools/call",
            "params": {"name": name, "arguments": args}});
        let _ = writeln!(input, "{req}");
    }
    let mut stdin = child.stdin.take().ok_or("mcp: no stdin")?;
    // Feed stdin from a thread so a full stdout pipe cannot deadlock us.
    let writer = std::thread::spawn(move || {
        let r = stdin.write_all(input.as_bytes());
        drop(stdin);
        r
    });
    let stdout = child.stdout.take().ok_or("mcp: no stdout")?;
    let mut responses = 0usize;
    let mut errors = 0usize;
    for line in BufReader::new(stdout).lines() {
        let line = line.map_err(|e| format!("mcp: {e}"))?;
        let v: Value = serde_json::from_str(&line).map_err(|e| format!("mcp: bad JSON: {e}"))?;
        if v.get("error").is_some() {
            return Err(format!("mcp: protocol error: {}", v["error"]));
        }
        if v.pointer("/result/isError") == Some(&Value::Bool(true)) {
            errors += 1;
        }
        responses += 1;
    }
    let _ = writer.join();
    let status = child.wait().map_err(|e| format!("mcp: {e}"))?;
    if responses != calls.len() + 1 {
        return Err(format!(
            "mcp: {responses} responses for {} requests ({status})",
            calls.len() + 1
        ));
    }
    Ok(errors)
}

// ---------------------------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------------------------

#[cfg(feature = "bench")]
fn run_all(opts: &Opts) -> Result<(), String> {
    if cfg!(debug_assertions) {
        eprintln!("warning: xtask is a debug build; in-process numbers are not meaningful");
    }
    let total = Instant::now();
    let bin = cli_binary(opts)?;
    let machine = machine_info();
    eprintln!("machine: {machine}");
    eprintln!("binary: {}", bin.display());
    let scratch = Scratch::new()?;
    let opts2 = opts.clone();
    let bin2 = bin.clone();
    let scratch_dir = scratch.0.clone();
    // The DAX parser and binder recurse (bounded); give them the CLI's stack.
    let rows = std::thread::Builder::new()
        .name("bench".into())
        .stack_size(256 * 1024 * 1024)
        .spawn(move || {
            let mut b = Bencher::new(opts2.quick, opts2.filter.clone());
            inproc::run(&mut b, &opts2, &bin2, &scratch_dir);
            b.rows
        })
        .map_err(|e| format!("bench thread: {e}"))?
        .join()
        .map_err(|_| "bench thread panicked".to_string())?;
    if opts.keep {
        eprintln!("kept the generated inputs in {}", scratch.0.display());
        std::mem::forget(scratch);
    } else {
        drop(scratch);
    }
    let md = markdown(&rows);
    println!(
        "# bicraft benchmarks{}\n\n- CPU: {} ({} threads)\n- OS: {} ({})\n- rustc: {}\n{md}",
        if opts.quick { " (--quick)" } else { "" },
        machine["cpu"].as_str().unwrap_or("?"),
        machine["threads"],
        machine["os"].as_str().unwrap_or("?"),
        machine["arch"].as_str().unwrap_or("?"),
        machine["rustc"].as_str().unwrap_or("?"),
    );
    if let Some(path) = &opts.json {
        let text =
            serde_json::to_string_pretty(&rows_json(&rows, &machine, opts)).map_err(|e| e.to_string())?;
        write_file(path, text.as_bytes())?;
        eprintln!("wrote {}", path.display());
    }
    let failed = rows
        .iter()
        .filter(|r| r.error.as_deref().is_some_and(|e| !e.starts_with("skipped")))
        .count();
    eprintln!(
        "\nbench: {} cases, {failed} failed, {}",
        rows.len(),
        fmt_secs(total.elapsed().as_secs_f64())
    );
    if failed > 0 {
        Err(format!("{failed} benchmark case(s) failed"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantiles_use_nearest_rank() {
        let v: Vec<f64> = (1..=20).map(f64::from).collect();
        assert_eq!(quantile(&v, 0.5), 10.0);
        assert_eq!(quantile(&v, 0.95), 19.0);
        assert_eq!(quantile(&[3.0], 0.95), 3.0);
        assert_eq!(quantile(&[], 0.5), 0.0);
    }

    #[test]
    fn options_parse() {
        let args: Vec<String> = ["--quick", "--filter", "DAX", "--real"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let o = Opts::parse(&args).unwrap();
        assert!(o.quick && o.real);
        assert_eq!(o.filter.as_deref(), Some("dax"));
        assert!(Opts::parse(&["--nope".to_string()]).is_err());
        assert!(Opts::parse(&["--json".to_string()]).is_err());
    }

    #[test]
    fn harness_measures_and_reports() {
        let mut b = Bencher::new(true, None);
        b.group("G");
        b.case("ok", 8, || {
            Ok(Work::items(1000, "x").with_bytes(2_000_000).note("n=1"))
        });
        b.case("bad", 8, || Err("boom".into()));
        b.skip("later", "no input");
        assert_eq!(b.rows.len(), 3);
        assert_eq!(b.rows[0].runs, 2);
        assert!(b.rows[1].error.is_some());
        let md = markdown(&b.rows);
        assert!(md.contains("### G") && md.contains("| ok | 2 |") && md.contains("boom"));
        assert!(
            throughput(&Row {
                median: 0.5,
                ..b.rows[0].clone()
            })
            .contains("2.0 kx/s")
        );
        let mut f = Bencher::new(true, Some("other".into()));
        f.group("G");
        f.case("ok", 1, || Ok(Work::default()));
        assert!(f.rows.is_empty());
    }

    #[test]
    fn formats_durations() {
        assert_eq!(fmt_secs(2.5), "2.50 s");
        assert_eq!(fmt_secs(0.0125), "12.5 ms");
        assert_eq!(fmt_secs(0.000_012_5), "12.5 µs");
    }
}
