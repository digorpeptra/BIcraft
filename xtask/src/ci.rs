//! `cargo xtask ci`: the gate (docs/ci.md), and `cargo xtask heavy`, its nightly complement.
//!
//! The gate runs in groups so the GitHub jobs can run them side by side:
//!
//! - `lint`: fmt --check, layers, docs, assets, profiles, clippy -D warnings;
//! - `test`: the tests, with cargo-nextest when it is installed (`cargo test` otherwise), then
//!   the doctests; the heavy tests of [`crate::heavy`] are left out;
//! - `webui`: the wasm32 check of the browser app.
//!
//! `cargo xtask ci` alone runs all three. Every step prints its duration in the summary.

use std::time::{Duration, Instant};

use crate::{cargo, heavy, run};

/// The cargo-nextest version CI installs (`.config/nextest.toml` recommends the same).
pub const NEXTEST_VERSION: &str = "0.9.150";

type StepFn = fn(&Options) -> Result<(), String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Group {
    Lint,
    Test,
    Webui,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Options {
    lint: bool,
    test: bool,
    webui: bool,
    /// Force `cargo test` even when cargo-nextest is installed.
    no_nextest: bool,
}

const STEPS: &[(&str, Group, StepFn)] = &[
    ("fmt", Group::Lint, |_| {
        crate::run_cargo(&["fmt", "--all", "--", "--check"])
    }),
    ("layers", Group::Lint, |_| crate::cmd_layers()),
    ("docs", Group::Lint, |_| crate::docs_check::cmd()),
    ("assets", Group::Lint, |_| crate::assets::cmd()),
    ("profiles", Group::Lint, |_| crate::profiles::check()),
    ("clippy", Group::Lint, |_| {
        crate::run_cargo(&[
            "clippy",
            "--workspace",
            "--all-targets",
            "--all-features",
            "--",
            "-D",
            "warnings",
        ])
    }),
    ("test", Group::Test, |o| test(o.no_nextest)),
    ("doctests", Group::Test, |o| doctests(o.no_nextest)),
    ("webui", Group::Webui, |_| crate::webui::check()),
];

pub fn parse(args: &[String]) -> Result<Options, String> {
    let mut o = Options::default();
    for a in args {
        match a.as_str() {
            "lint" => o.lint = true,
            "test" => o.test = true,
            "webui" => o.webui = true,
            "all" => (o.lint, o.test, o.webui) = (true, true, true),
            "--no-nextest" => o.no_nextest = true,
            other => {
                return Err(format!(
                    "ci: unknown argument `{other}` (expected lint, test, webui, all or --no-nextest)"
                ));
            }
        }
    }
    if !(o.lint || o.test || o.webui) {
        (o.lint, o.test, o.webui) = (true, true, true);
    }
    Ok(o)
}

fn selected(o: &Options, g: Group) -> bool {
    match g {
        Group::Lint => o.lint,
        Group::Test => o.test,
        Group::Webui => o.webui,
    }
}

pub fn cmd(args: &[String]) -> Result<(), String> {
    let o = parse(args)?;
    let steps: Vec<_> = STEPS.iter().filter(|(_, g, _)| selected(&o, *g)).collect();
    let total = Instant::now();
    let mut done: Vec<(&str, Duration)> = Vec::new();
    for (name, _, f) in &steps {
        eprintln!("\n=== ci: {name} ===");
        let start = Instant::now();
        let result = f(&o);
        let took = start.elapsed();
        if let Err(e) = result {
            println!("\nCI summary:");
            for (d, t) in &done {
                println!("  ok    {d:<9} {}", secs(*t));
            }
            println!("  FAIL  {name:<9} {}: {e}", secs(took));
            for (n, _, _) in steps.iter().skip(done.len() + 1) {
                println!("  skip  {n}");
            }
            return Err(format!("ci failed at `{name}`"));
        }
        done.push((name, took));
    }
    println!("\nCI summary:");
    for (d, t) in &done {
        println!("  ok    {d:<9} {}", secs(*t));
    }
    println!("all {} steps passed in {}", done.len(), secs(total.elapsed()));
    Ok(())
}

fn secs(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 60 {
        format!("{}m{:02}s", s / 60, s % 60)
    } else {
        format!("{:.1}s", d.as_secs_f64())
    }
}

/// Whether `cargo nextest` runs here.
pub fn nextest_installed() -> bool {
    cargo()
        .args(["nextest", "--version"])
        .output()
        .is_ok_and(|o| o.status.success())
}

fn use_nextest(no_nextest: bool) -> bool {
    if no_nextest {
        return false;
    }
    if nextest_installed() {
        return true;
    }
    eprintln!(
        "note: cargo-nextest is not installed (cargo install cargo-nextest --version {NEXTEST_VERSION} --locked); \
         running the tests with cargo test"
    );
    false
}

/// The workspace tests without the heavy ones.
fn test(no_nextest: bool) -> Result<(), String> {
    if use_nextest(no_nextest) {
        let filter = heavy::exclude_filter();
        let mut c = cargo();
        c.args([
            "nextest",
            "run",
            "--workspace",
            "--profile",
            "ci",
            "--no-tests",
            "warn",
            "-E",
            &filter,
        ]);
        run(c, "cargo nextest run --workspace --profile ci -E <not heavy>")
    } else {
        let mut args: Vec<String> = ["test", "--workspace", "--", "--quiet"]
            .map(String::from)
            .to_vec();
        args.extend(heavy::skip_args());
        let mut c = cargo();
        c.args(&args);
        run(c, "cargo test --workspace -- --skip <heavy>")
    }
}

/// nextest does not run doctests; `cargo test` (the fallback) already did.
fn doctests(no_nextest: bool) -> Result<(), String> {
    if !use_nextest(no_nextest) {
        eprintln!("doctests: run by cargo test above");
        return Ok(());
    }
    crate::run_cargo(&["test", "--workspace", "--doc", "--", "--quiet"])
}

/// `cargo xtask heavy [--list | --filter-out]`: the heavy tests only (docs/ci.md, nightly
/// workflow). `--list` prints them; `--filter-out` prints the nextest filter of every other
/// test (the MSRV job of ci.yml runs nextest itself).
pub fn cmd_heavy(args: &[String]) -> Result<(), String> {
    if args.iter().any(|a| a == "--filter-out") {
        println!("{}", heavy::exclude_filter());
        return Ok(());
    }
    if args.iter().any(|a| a == "--list") {
        for t in heavy::HEAVY {
            println!(
                "{}  {}\n    heavy: {}\n    gate: {}",
                t.package, t.test, t.why, t.covered_by
            );
        }
        for r in heavy::LONG_RUNS {
            let env: Vec<String> = r.env.iter().map(|(k, v)| format!("{k}={v}")).collect();
            println!("{}  {}  ({})", r.package, r.filter, env.join(" "));
        }
        return Ok(());
    }
    if let Some(a) = args.first() {
        return Err(format!(
            "heavy: unknown argument `{a}` (expected --list or --filter-out)"
        ));
    }
    if !nextest_installed() {
        return Err(format!(
            "heavy: needs cargo-nextest (cargo install cargo-nextest --version {NEXTEST_VERSION} --locked)"
        ));
    }
    let mut failed = Vec::new();
    if !heavy::HEAVY.is_empty() {
        let mut c = cargo();
        c.args(["nextest", "run", "--workspace", "--profile", "heavy", "-E"]);
        c.arg(heavy::include_filter());
        if let Err(e) = run(c, "cargo nextest run --workspace --profile heavy -E <heavy>") {
            failed.push(e);
        }
    }
    for r in heavy::LONG_RUNS {
        let mut c = cargo();
        c.envs(r.env.iter().copied());
        // `--workspace` with a package filter, not `-p`: the build is the gate's (same features),
        // so it is compiled once for all the runs.
        let filter = format!("package({}) & ({})", r.package, r.filter);
        c.args([
            "nextest",
            "run",
            "--workspace",
            "--profile",
            "heavy",
            "-E",
            &filter,
        ]);
        let env: Vec<String> = r.env.iter().map(|(k, v)| format!("{k}={v}")).collect();
        let what = format!("{} cargo nextest run --workspace -E '{filter}'", env.join(" "));
        if let Err(e) = run(c, &what) {
            failed.push(e);
        }
    }
    if failed.is_empty() {
        println!(
            "heavy: {} heavy test(s) and {} long run(s) passed",
            heavy::HEAVY.len(),
            heavy::LONG_RUNS.len()
        );
        Ok(())
    } else {
        Err(format!(
            "heavy: {} run(s) failed:\n  {}",
            failed.len(),
            failed.join("\n  ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn no_group_means_all() {
        let o = parse(&[]).unwrap();
        assert!(o.lint && o.test && o.webui && !o.no_nextest);
        assert_eq!(parse(&args(&["all"])).unwrap(), o);
    }

    #[test]
    fn groups_select_their_steps() {
        let o = parse(&args(&["lint"])).unwrap();
        let names: Vec<_> = STEPS
            .iter()
            .filter(|(_, g, _)| selected(&o, *g))
            .map(|(n, _, _)| *n)
            .collect();
        assert_eq!(names, ["fmt", "layers", "docs", "assets", "profiles", "clippy"]);
        let o = parse(&args(&["test", "--no-nextest"])).unwrap();
        assert!(o.test && !o.lint && !o.webui && o.no_nextest);
    }

    #[test]
    fn unknown_arguments_are_errors() {
        assert!(parse(&args(&["--heavy"])).is_err());
        assert!(parse(&args(&["tests"])).is_err());
    }

    #[test]
    fn durations_read_well() {
        assert_eq!(secs(Duration::from_millis(1500)), "1.5s");
        assert_eq!(secs(Duration::from_secs(125)), "2m05s");
    }
}
