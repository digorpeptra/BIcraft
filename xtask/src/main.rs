//! Workspace tooling: `cargo xtask <command>`.
//!
//! Pure Rust (std + serde_json). External tools (`cargo`, `wsl.exe`) are
//! invoked through `std::process::Command`. Child processes inherit the
//! environment, so `CARGO_TARGET_DIR` set by the caller applies to them too.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod assets;
mod bench;
mod ci;
mod commands_ui;
mod conc_bench;
mod corpus;
mod csv_bench;
mod deny;
mod deploy;
mod dist;
mod docs_check;
mod engine_bench;
mod engine_variants;
mod font_subset;
mod geo_bundle;
mod heavy;
mod layers;
mod oracles;
mod parity;
mod pkg_oracle;
mod profiles;
mod rcmp;
mod schema_watch;
mod uilang;
mod webcheck;
mod webui;
mod wsl;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  ci [lint] [test] [webui] [--no-nextest]
                  the gate (docs/ci.md): lint = fmt --check, layers, docs, assets,
                  profiles, clippy -D warnings; test = the tests without the heavy ones
                  (cargo-nextest when installed, else cargo test) and the doctests; webui =
                  wasm32 check.
                  No group runs all three; stops at the first failure
  heavy [--list|--filter-out]
                  the heavy tests only, with their long loops (nightly; docs/ci.md)
  assets          every file under assets/, the bundled fonts and the UI's icons have a row in
                  ATTRIBUTION.md (part of ci lint)
  deny [ARGS]     cargo deny check (licences, sources, advisories, bans; deny.toml) when
                  cargo-deny is installed, else a note; ARGS go to `cargo deny check`
  corpus verify|fetch [--dir DIR] [--url BASE] | stage --out DIR [--dir DIR]
                  check the corpus files (BICRAFT_CORPUS) against corpus/manifest.json, or
                  download them first (the GitHub release, authenticated by GITHUB_TOKEN or
                  `gh auth token`; network; docs/corpus.md); stage copies them under their
                  release asset names and prints the upload command (never runs it)
  layers          enforce the crate dependency layering (AGENTS.md layer table)
  docs            committed Markdown checks: no Power BI Desktop timing or speed-up
                  (docs/interoperability.md); parity summaries up to date
  commands-ui [--check]
                  write docs/commands-ui.md, the placement table of every engine command (ribbon
                  tab and group, menu or palette, label, icon, shortcut, needs) from commands.rs
  parity [--check]
                  recompute the parity summary of docs/parity-checklist.md and the headline of
                  docs/target-app-parity.md from the checklist rows (--check: fail if stale)
  test            cargo test --workspace
  webui [--out DIR] [--wasm-opt]
                  the browser app bundle (apps/bicraft-web for wasm32 + wasm-bindgen; docs/webui.md)
  webui check     cargo check of the browser app for wasm32 (skipped without the target)
  webcheck record|check [--dir DIR] [--bicraft PATH] [--only WALK] [--tolerance N] [--max-diff F]
                  web viewer walkthroughs and screenshot comparisons with Craft Anything, a
                  local development aid; skipped without it; not part of ci (docs/webcheck.md)
  bench [--quick] [--real] [--real-file PATH] [--filter TEXT] [--json PATH]
                  [--no-build] [--cli-only] [--keep]
                  performance benchmarks, in-process and through the release CLI
                  (re-runs xtask in release with --features bench; docs/benchmarks.md)
  engine-bench gen|bicraft|rounds|merge|compare|check|queries ARGS
                  bicraft's DAX evaluator on generated data and queries; results checked
                  against reference answers (xtask/engine-bench; docs/benchmarks.md)
  csv-bench --csv FILE [--reader m|csv] [--runs N]
                  time and peak memory of the CSV ingestion over the engine bench's Ventes.csv
                  (re-runs xtask in release with --features bench; docs/benchmarks.md)
  concurrency-bench --bin BICRAFT --data FILE.pbix [--base-bin B] [--parallel N] [--rounds R]
                  N identical queries at once on one workspace of `bicraft-cli serve`, reads
                  serialized against side by side, answers compared (docs/benchmarks.md)
  render-compare DESKTOP_DIR OURS_DIR LAYOUT.json [--json] [--diff DIR]
                  [--min-sim X] [--floors FILE]
                  per-visual scores of `bicraft-cli render` against Power BI Desktop
                  captures (tools/render-oracle; docs/render.md)
  oracle-gen synth|grids|parts|slicers|expand|kpis|states|interactions ARGS
                  synthetic reports of the render oracle (tools/render-oracle/README.md)
  font-subset IN.ttf OUT.ttf CHARS
                  a glyph subset of a TrueType font (the UI's symbol face; docs/design.md)
  geo-bundle SRC.geojson | --basemap DIR
                  the country outlines of the offline maps, or the 1:50m layers of the open
                  basemap, from Natural Earth (docs/formats/maps.md)
  dax-corpus [--check]
                  write (or check) crates/dax/tests/oracle/corpus.json (docs/dax-oracle.md)
  metadata-edit EDIT IN.json OUT.json
                  one TMSL edit of the metadata oracle (docs/formats/metadata-sqlitedb.md)
  storage-compare CASE.dmv.json CASE.stats.json [--verbose]
                  model.storage_stats against the engine storage DMVs (docs/formats/vertipaq-stats.md)
  tmdl-model [LEVEL] [--out FILE]
                  the synthetic TMSL model of the TMDL oracle (docs/formats/tmdl.md)
  oracle-gen uilang OUT.pbix COMMANDS.json LAYOUT.json [CULTURE]
                  write the UI-language oracle report's `bicraft-cli run` commands
                  (auto titles, aggregation names, (Blank), Total, empty visuals, formats)
  layout-oracle gen|install DIR
                  synthetic legacy reports for the legacy -> PBIR oracle (G13;
                  docs/formats/legacy-layout-bb.md, tools/layout-oracle)
  pkg-oracle observe PATH [--json] | mutate BASE.pbit BASE.pbix PROJECT_DIR OUT_DIR
                  structure of a PBIX/PBIT/PBIP (never content), or one variant per
                  packaging fact for Desktop (tools/packaging-oracle;
                  docs/formats/packaging-verification.md)
  schema-watch [--json] [--no-diff]
                  compare the $schema versions bicraft writes and vendors with the ones
                  Microsoft publishes (network; exit 0 ok, 1 action needed, 2 failure;
                  docs/schema-watch.md)
  deploy check|smoke|kind-smoke|controller-smoke [ARGS]
                  Kubernetes packaging of `bicraft-cli serve`: static checks of deploy/, a
                  container-free smoke test, the kind/Helm flow, the workspace controller in
                  kind (docs/kubernetes.md)
  dist [--out DIR] [--only windows|linux] [--no-build] [--no-smoke]
                  release archives (zip, tar.gz, SHA256SUMS) of the bicraft-cli binary in a
                  folder outside the repo (default ~/bicraft-dist); Linux built in WSL;
                  smoke-tested from the extracted archives (docs/releasing.md)
  wsl [ci|test|layers|heavy] [ARGS]
                  run that xtask command (default: ci) inside WSL Ubuntu with a
                  Linux-side target dir (~/target/bicraft, or $BICRAFT_WSL_TARGET_DIR);
                  on Linux, run it locally
  help            show this message
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "schema-watch") {
        return schema_watch::cmd(args.get(1..).unwrap_or(&[]));
    }
    let result = match args.first().map(String::as_str) {
        Some("assets") => assets::cmd(),
        Some("bench") => bench::cmd_bench(args.get(1..).unwrap_or(&[])),
        Some("ci") => ci::cmd(args.get(1..).unwrap_or(&[])),
        Some("heavy") => ci::cmd_heavy(args.get(1..).unwrap_or(&[])),
        Some("engine-bench") => engine_bench::cmd(args.get(1..).unwrap_or(&[])),
        Some("concurrency-bench") => conc_bench::cmd(args.get(1..).unwrap_or(&[])),
        Some("corpus") => corpus::cmd(args.get(1..).unwrap_or(&[])),
        Some("csv-bench") => csv_bench::cmd(args.get(1..).unwrap_or(&[])),
        Some("font-subset") => font_subset::cmd(args.get(1..).unwrap_or(&[])),
        Some("geo-bundle") => geo_bundle::cmd(args.get(1..).unwrap_or(&[])),
        Some("layers") => cmd_layers(),
        Some("layout-oracle") => oracles::cmd_layout_oracle(args.get(1..).unwrap_or(&[])),
        Some("deny") => deny::cmd(args.get(1..).unwrap_or(&[])),
        Some("deploy") => deploy::cmd(args.get(1..).unwrap_or(&[])),
        Some("dist") => dist::cmd(args.get(1..).unwrap_or(&[])),
        Some("docs") => docs_check::cmd(),
        Some("dax-corpus") => oracles::cmd_dax_corpus(args.get(1..).unwrap_or(&[])),
        Some("metadata-edit") => oracles::cmd_metadata_edit(args.get(1..).unwrap_or(&[])),
        Some("oracle-gen") if args.get(1).is_some_and(|a| a == "uilang") => {
            uilang::cmd(args.get(2..).unwrap_or(&[]))
        }
        Some("oracle-gen") => oracles::cmd_gen(args.get(1..).unwrap_or(&[])),
        Some("parity") => parity::cmd(args.get(1..).unwrap_or(&[])),
        Some("commands-ui") => commands_ui::cmd(args.get(1..).unwrap_or(&[])),
        Some("pkg-oracle") => pkg_oracle::cmd(args.get(1..).unwrap_or(&[])),
        Some("render-compare") => rcmp::cmd(args.get(1..).unwrap_or(&[])),
        Some("storage-compare") => oracles::cmd_storage_compare(args.get(1..).unwrap_or(&[])),
        Some("test") => cmd_test(),
        Some("tmdl-model") => oracles::cmd_tmdl_model(args.get(1..).unwrap_or(&[])),
        Some("webcheck") => webcheck::cmd(args.get(1..).unwrap_or(&[])),
        Some("webui") => webui::cmd(args.get(1..).unwrap_or(&[])),
        Some("wsl") => cmd_wsl(args.get(1..).unwrap_or(&[])),
        Some("-h" | "--help" | "help") | None => {
            print!("{USAGE}");
            Ok(())
        }
        Some(other) => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Workspace root (parent of the xtask crate).
fn root() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest.parent().unwrap_or(manifest).to_path_buf()
}

fn cargo() -> Command {
    let mut c = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    c.current_dir(root());
    c
}

fn run(mut cmd: Command, what: &str) -> Result<(), String> {
    eprintln!("$ {what}");
    let status = cmd
        .status()
        .map_err(|e| format!("{what}: failed to spawn: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{what}: exited with {status}"))
    }
}

fn run_cargo(args: &[&str]) -> Result<(), String> {
    let mut c = cargo();
    c.args(args);
    run(c, &format!("cargo {}", args.join(" ")))
}

fn metadata() -> Result<serde_json::Value, String> {
    let out = cargo()
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "cargo metadata failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata: bad JSON: {e}"))
}

fn cmd_layers() -> Result<(), String> {
    let crates = layers::from_metadata(&metadata()?)?;
    println!("Dependency layering (AGENTS.md layer table)\n");
    println!("{:<20} {:<14} workspace deps", "crate", "layer");
    for c in &crates {
        let ws: Vec<String> = c
            .deps
            .iter()
            .filter(|d| d.workspace)
            .map(|d| {
                let k = match d.kind {
                    layers::DepKind::Normal => "",
                    layers::DepKind::Dev => " (dev)",
                    layers::DepKind::Build => " (build)",
                };
                format!("{}{k}", d.name.strip_prefix("bicraft-").unwrap_or(&d.name))
            })
            .collect();
        println!(
            "{:<20} {:<14} {}",
            c.name,
            layers::describe(layers::classify(&c.name)),
            ws.join(", ")
        );
    }
    let violations = layers::check(&crates);
    println!();
    if violations.is_empty() {
        println!("OK: {} crates, no layering violations.", crates.len());
        Ok(())
    } else {
        println!("{} violation(s):", violations.len());
        for v in &violations {
            println!("  - {v}");
        }
        Err(format!("{} layering violation(s)", violations.len()))
    }
}

fn cmd_test() -> Result<(), String> {
    run_cargo(&["test", "--workspace"])
}

fn cmd_wsl(args: &[String]) -> Result<(), String> {
    let sub = args.first().map(String::as_str).unwrap_or("ci");
    let rest = args.get(1..).unwrap_or(&[]);
    if !matches!(sub, "ci" | "test" | "layers" | "heavy") {
        return Err(format!(
            "wsl: unsupported command `{sub}` (expected ci, test, layers or heavy)"
        ));
    }
    if !cfg!(windows) {
        return match sub {
            "ci" => ci::cmd(rest),
            "heavy" => ci::cmd_heavy(rest),
            "test" => cmd_test(),
            _ => cmd_layers(),
        };
    }
    let repo = wsl::to_wsl_path(&root().to_string_lossy())?;
    let mut all = vec![sub.to_owned()];
    all.extend(rest.iter().cloned());
    let script = wsl::script_args(&repo, &all);
    eprintln!("$ wsl.exe -d Ubuntu -- bash -lc {script}");
    let mut c = Command::new("wsl.exe");
    // `--exec` hands argv to bash directly (no extra shell re-parsing).
    c.args(["-d", "Ubuntu", "--exec", "bash", "-lc", &script]);
    // A Windows-side target dir means nothing inside WSL; the script sets its own.
    c.env_remove("CARGO_TARGET_DIR");
    run(c, &format!("wsl ({})", all.join(" ")))
}
