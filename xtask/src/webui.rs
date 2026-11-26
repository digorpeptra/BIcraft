//! `cargo xtask webui`: builds the browser app (`apps/bicraft-web`, docs/webui.md) and
//! `cargo xtask webui check`, the wasm32 check that `cargo xtask ci` runs.
//!
//! The bundle is the wasm-bindgen output of `bicraft-web` for wasm32-unknown-unknown:
//! `bicraft_web.js` (the module glue) and `bicraft_web_bg.wasm`. Building bicraft with
//! `BICRAFT_WEBUI_DIR` set to the output folder embeds it (`crates/view/build.rs`).
//!
//! Tools, Rust only: the `wasm32-unknown-unknown` target (`rustup target add`) and the
//! wasm-bindgen CLI of exactly the version in Cargo.lock (`cargo install wasm-bindgen-cli
//! --version <v> --locked --root DIR`), found as `BICRAFT_WASM_BINDGEN`, in
//! `BICRAFT_WEBUI_TOOLS/bin` or on `PATH`. `wasm-opt` (binaryen) is used when `--wasm-opt` is
//! given and it is on `PATH`.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{cargo, root, run};

/// Size budget of the bundle, gzip-compressed (what a browser downloads once, then revalidates).
pub const BUDGET_GZIP_BYTES: u64 = 8 * 1024 * 1024;

const TARGET: &str = "wasm32-unknown-unknown";
const PROFILE: &str = "webui";

pub fn cmd(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("check") => check(),
        _ => build(args),
    }
}

/// Whether rustup lists the wasm32 target (always true without rustup: let cargo say).
fn target_installed() -> bool {
    match Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
    {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .any(|l| l.trim() == TARGET),
        _ => true,
    }
}

/// `cargo check` of the browser app for wasm32 (skipped, with a note, without the target).
pub fn check() -> Result<(), String> {
    if !target_installed() {
        eprintln!(
            "note: the {TARGET} target is not installed (rustup target add {TARGET}); skipping the browser app check"
        );
        return Ok(());
    }
    let mut c = cargo();
    c.args(["check", "-p", "bicraft-web", "--target", TARGET, "--bins"]);
    run(c, &format!("cargo check -p bicraft-web --target {TARGET}"))
}

/// The version of `name` in Cargo.lock.
pub fn locked_version(lock: &str, name: &str) -> Option<String> {
    let mut lines = lock.lines();
    while let Some(l) = lines.next() {
        if l.trim() == format!("name = \"{name}\"") {
            let v = lines
                .next()?
                .trim()
                .strip_prefix("version = \"")?
                .strip_suffix('"')?;
            return Some(v.to_string());
        }
    }
    None
}

/// The wasm-bindgen CLI to run, checked against `want`.
fn wasm_bindgen(want: &str) -> Result<PathBuf, String> {
    let exe = if cfg!(windows) {
        "wasm-bindgen.exe"
    } else {
        "wasm-bindgen"
    };
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(p) = bicraft_env::var_os("BICRAFT_WASM_BINDGEN") {
        candidates.push(PathBuf::from(p));
    }
    if let Some(d) = bicraft_env::var_os("BICRAFT_WEBUI_TOOLS") {
        candidates.push(PathBuf::from(d).join("bin").join(exe));
    }
    candidates.push(PathBuf::from("wasm-bindgen"));
    let install = format!(
        "cargo install wasm-bindgen-cli --version {want} --locked --root <DIR>, then set BICRAFT_WEBUI_TOOLS=<DIR>"
    );
    let mut seen = Vec::new();
    for c in candidates {
        let Ok(out) = Command::new(&c).arg("--version").output() else {
            continue;
        };
        let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if text.split_whitespace().nth(1) == Some(want) {
            return Ok(c);
        }
        seen.push(format!("{} ({text})", c.display()));
    }
    if seen.is_empty() {
        Err(format!("wasm-bindgen {want} not found: {install}"))
    } else {
        Err(format!(
            "wasm-bindgen must be {want} (Cargo.lock), found {}: {install}",
            seen.join(", ")
        ))
    }
}

/// The workspace's target folder (`cargo metadata`: honours `CARGO_TARGET_DIR` and config).
fn target_dir() -> PathBuf {
    crate::metadata()
        .ok()
        .and_then(|m| m.get("target_directory")?.as_str().map(PathBuf::from))
        .or_else(|| std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from))
        .unwrap_or_else(|| root().join("target"))
}

fn gzip_len(bytes: &[u8]) -> u64 {
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    if gz.write_all(bytes).is_err() {
        return 0;
    }
    gz.finish().map(|v| v.len() as u64).unwrap_or(0)
}

fn mb(n: u64) -> String {
    format!("{:.2} MB", n as f64 / 1_048_576.0)
}

/// `cargo xtask webui [--out DIR] [--wasm-opt]`.
fn build(args: &[String]) -> Result<(), String> {
    let mut out: Option<PathBuf> = None;
    let mut opt = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" => out = Some(PathBuf::from(it.next().ok_or("--out needs a folder")?)),
            "--wasm-opt" => opt = true,
            other => return Err(format!("webui: unknown argument `{other}`")),
        }
    }
    let out = bundle(out, opt)?;
    println!(
        "\nTo embed it, build bicraft with\n  BICRAFT_WEBUI_DIR={}\n(then `bicraft view FILE --ui egui`, or /w/<id>/view/app/ on `bicraft serve`)",
        out.display()
    );
    Ok(())
}

/// Builds the bundle into `out` (default `<target>/webui`) and returns that folder (absolute);
/// also used by `cargo xtask dist`, whose binaries always embed it.
pub fn bundle(out: Option<PathBuf>, opt: bool) -> Result<PathBuf, String> {
    if !target_installed() {
        return Err(format!(
            "the {TARGET} target is missing: rustup target add {TARGET}"
        ));
    }
    let lock = std::fs::read_to_string(root().join("Cargo.lock")).map_err(|e| format!("Cargo.lock: {e}"))?;
    let want = locked_version(&lock, "wasm-bindgen").ok_or("no wasm-bindgen in Cargo.lock")?;
    let bindgen = wasm_bindgen(&want)?;
    let mut c = cargo();
    c.args([
        "build",
        "-p",
        "bicraft-web",
        "--bin",
        "bicraft-web",
        "--target",
        TARGET,
        "--profile",
        PROFILE,
    ]);
    run(
        c,
        &format!("cargo build -p bicraft-web --target {TARGET} --profile {PROFILE}"),
    )?;
    let wasm = target_dir().join(TARGET).join(PROFILE).join("bicraft-web.wasm");
    if !wasm.is_file() {
        return Err(format!("{}: not built", wasm.display()));
    }
    let out = out.unwrap_or_else(|| target_dir().join("webui"));
    let out = std::path::absolute(&out).unwrap_or(out);
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut c = Command::new(&bindgen);
    c.args([
        "--target",
        "web",
        "--no-typescript",
        "--out-name",
        "bicraft_web",
        "--out-dir",
    ])
    .arg(&out)
    .arg(&wasm);
    run(c, "wasm-bindgen --target web")?;
    let bg = out.join("bicraft_web_bg.wasm");
    if opt {
        let mut c = Command::new("wasm-opt");
        c.args([
            "-Oz",
            "--enable-bulk-memory",
            "--enable-nontrapping-float-to-int",
            "-o",
        ])
        .arg(&bg)
        .arg(&bg);
        if let Err(e) = run(c, "wasm-opt -Oz") {
            eprintln!("note: wasm-opt failed or is missing ({e}); keeping the unoptimized module");
        }
    }
    report(&out)?;
    Ok(out)
}

/// Whether the binary `bin` embeds the browser app (`bicraft build-info`).
pub fn binary_embeds(bin: &Path) -> Result<bool, String> {
    let o = Command::new(bin)
        .arg("build-info")
        .output()
        .map_err(|e| format!("{}: {e}", bin.display()))?;
    if !o.status.success() {
        return Err(format!("{} build-info failed", bin.display()));
    }
    let v: serde_json::Value =
        serde_json::from_slice(&o.stdout).map_err(|e| format!("{} build-info: {e}", bin.display()))?;
    Ok(v.pointer("/webui/embedded").and_then(serde_json::Value::as_bool) == Some(true))
}

/// Prints the sizes against the budget.
fn report(out: &Path) -> Result<(), String> {
    let mut raw = 0u64;
    let mut gz = 0u64;
    println!("\nbrowser app bundle in {}:", out.display());
    for f in ["bicraft_web.js", "bicraft_web_bg.wasm"] {
        let bytes = std::fs::read(out.join(f)).map_err(|e| format!("{f}: {e}"))?;
        let g = gzip_len(&bytes);
        println!("  {f:<22} {:>10}   gzip {:>10}", mb(bytes.len() as u64), mb(g));
        raw += bytes.len() as u64;
        gz += g;
    }
    println!(
        "  total                  {:>10}   gzip {:>10}   (budget {} gzip{})",
        mb(raw),
        mb(gz),
        mb(BUDGET_GZIP_BYTES),
        if gz > BUDGET_GZIP_BYTES { ": OVER" } else { "" }
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_versions() {
        let lock = "[[package]]\nname = \"wasm-bindgen\"\nversion = \"0.2.129\"\nsource = \"x\"\n\n[[package]]\nname = \"wasm-bindgen-futures\"\nversion = \"0.4.79\"\n";
        assert_eq!(locked_version(lock, "wasm-bindgen").as_deref(), Some("0.2.129"));
        assert_eq!(
            locked_version(lock, "wasm-bindgen-futures").as_deref(),
            Some("0.4.79")
        );
        assert_eq!(locked_version(lock, "nope"), None);
        // the real lock pins the crate the app uses
        let real = std::fs::read_to_string(root().join("Cargo.lock")).unwrap();
        assert!(locked_version(&real, "wasm-bindgen").is_some());
    }
}
