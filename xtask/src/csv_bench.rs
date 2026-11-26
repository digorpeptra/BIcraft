//! `cargo xtask csv-bench --csv FILE [--reader m|csv] [--runs N]`: time and peak memory of
//! BIcraft's CSV ingestion (`docs/benchmarks.md`, *CSV ingestion*).
//!
//! The input is the engine bench's `Ventes.csv` (`cargo xtask engine-bench gen --rows N --out
//! DIR`; 6 columns: a date, four whole numbers, a currency). Readers: `m` runs the query the
//! engine bench writes (`Csv.Document` + `Table.PromoteHeaders` + `Table.TransformColumnTypes`,
//! what `table.import {m}` and `model.refresh` run) and `csv` runs `bicraft_data::csv::read_csv`
//! (`table.import {csv}`) with the same types. The row, step, time and size caps are lifted
//! so the path itself is measured.
//!
//! Each run is a fresh process (`csv-bench one …`), so its peak memory is its own: the peak
//! working set on Windows, `VmHWM` on Linux. The loaded table's checksum (sums of the whole
//! numbers, currency and dates, blank count) is printed so two builds can be compared.
//!
//! The in-process load needs `bicraft-data`, linked with the `bench` feature: `cargo xtask
//! csv-bench …` re-runs itself as `cargo run --release -p xtask --features bench -- csv-bench …`.

use std::process::Command;

use serde_json::{Value, json};

#[cfg(feature = "bench")]
const TYPES: &[(&str, &str)] = &[
    ("Date", "type date"),
    ("ProduitKey", "Int64.Type"),
    ("MagasinKey", "Int64.Type"),
    ("ClientKey", "Int64.Type"),
    ("Quantite", "Int64.Type"),
    ("Montant", "Currency.Type"),
];

fn opt<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

pub fn cmd(args: &[String]) -> Result<(), String> {
    if args.first().is_some_and(|a| a == "one") {
        return one(args.get(1..).unwrap_or(&[]));
    }
    if !cfg!(feature = "bench") {
        let mut c = crate::cargo();
        c.args([
            "run",
            "--release",
            "-p",
            "xtask",
            "--features",
            "bench",
            "--",
            "csv-bench",
        ]);
        c.args(args);
        return crate::run(c, "cargo run --release -p xtask --features bench -- csv-bench");
    }
    let csv = opt(args, "--csv").ok_or("--csv FILE is required")?;
    let reader = opt(args, "--reader").unwrap_or("m");
    let runs: usize = opt(args, "--runs")
        .map(str::parse)
        .transpose()
        .map_err(|e| format!("--runs: {e}"))?
        .unwrap_or(3);
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut results = Vec::new();
    for r in 0..runs {
        let out = Command::new(&exe)
            .args(["csv-bench", "one", "--csv", csv, "--reader", reader])
            .output()
            .map_err(|e| e.to_string())?;
        let text = String::from_utf8_lossy(&out.stdout);
        let v: Value = serde_json::from_str(text.trim())
            .map_err(|e| format!("run {r}: {e}: {text} {}", String::from_utf8_lossy(&out.stderr)))?;
        eprintln!("run {}: {v}", r + 1);
        results.push(v);
    }
    let mut secs: Vec<f64> = results.iter().filter_map(|v| v["load_s"].as_f64()).collect();
    secs.sort_by(f64::total_cmp);
    let peaks: Vec<f64> = results.iter().filter_map(|v| v["peak_mb"].as_f64()).collect();
    let summary = json!({
        "reader": reader,
        "file": csv,
        "runs": results.len(),
        "median_s": secs.get(secs.len() / 2),
        "worst_s": secs.last(),
        "peak_mb_max": peaks.iter().copied().fold(0.0, f64::max),
        "checksum": results.first().map(|v| v["checksum"].clone()),
        "rows": results.first().map(|v| v["rows"].clone()),
    });
    println!("{summary}");
    Ok(())
}

#[cfg(not(feature = "bench"))]
fn one(_args: &[String]) -> Result<(), String> {
    Err("csv-bench one needs the bench feature".into())
}

#[cfg(feature = "bench")]
fn one(args: &[String]) -> Result<(), String> {
    use bicraft_data::{CellValue, DataType, Limits, SourceOptions};
    let path = std::path::Path::new(opt(args, "--csv").ok_or("--csv FILE is required")?);
    let reader = opt(args, "--reader").unwrap_or("m");
    let limits = Limits {
        max_bytes: usize::MAX / 2,
        max_rows: usize::MAX / 2,
        max_steps: u64::MAX / 2,
        max_millis: 0,
        ..Limits::default()
    };
    let t0 = std::time::Instant::now();
    let data = match reader {
        "m" => {
            let dir = path.parent().ok_or("the CSV file has no folder")?;
            let expr = crate::engine_bench::m_csv(path, TYPES);
            let opts = SourceOptions {
                limits,
                file_roots: vec![dir.to_path_buf()],
                culture: Some("en-US".into()),
                ..SourceOptions::default()
            };
            bicraft_data::m::load_m_with(&expr, &opts).map_err(|e| e.to_string())?
        }
        "csv" => {
            let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
            let mut o = bicraft_data::csv::CsvOptions::new();
            o.delimiter = Some(',');
            o.limits = limits;
            o.column_types = TYPES
                .iter()
                .map(|(n, t)| {
                    let t = match *t {
                        "type date" => DataType::DateTime,
                        "Currency.Type" => DataType::Decimal,
                        _ => DataType::Int64,
                    };
                    (n.to_string(), t)
                })
                .collect();
            bicraft_data::csv::read_csv(&bytes, &o).map_err(|e| e.to_string())?
        }
        r => return Err(format!("unknown reader `{r}` (m or csv)")),
    };
    let load_s = t0.elapsed().as_secs_f64();
    let (mut ints, mut dec, mut dates, mut blanks) = (0i128, 0i128, 0f64, 0u64);
    for c in data.columns() {
        for v in &c.values {
            match v {
                CellValue::Int(i) => ints += i128::from(*i),
                CellValue::Decimal(d) => dec += i128::from(*d),
                CellValue::DateTime(d) => dates += d,
                CellValue::Null => blanks += 1,
                _ => {}
            }
        }
    }
    let out = json!({
        "reader": reader,
        "rows": data.row_count(),
        "load_s": load_s,
        "peak_mb": peak_mb(),
        "checksum": {"ints": ints.to_string(), "currency_x10000": dec.to_string(), "dates": dates, "blanks": blanks},
    });
    println!("{out}");
    Ok(())
}

/// This process' peak memory in MiB: the peak working set on Windows, `VmHWM` on Linux.
#[cfg(feature = "bench")]
fn peak_mb() -> Option<f64> {
    if cfg!(windows) {
        let pid = std::process::id();
        let out = Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!("(Get-Process -Id {pid}).PeakWorkingSet64"),
            ])
            .output()
            .ok()?;
        let bytes: f64 = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
        Some(bytes / f64::from(1u32 << 20))
    } else {
        let s = std::fs::read_to_string("/proc/self/status").ok()?;
        let kb: f64 = s
            .lines()
            .find_map(|l| l.strip_prefix("VmHWM:"))?
            .trim()
            .trim_end_matches("kB")
            .trim()
            .parse()
            .ok()?;
        Some(kb / 1024.0)
    }
}
