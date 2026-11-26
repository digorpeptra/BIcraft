# Continuous integration: the gate and the nightly path

`cargo xtask ci` is the gate a change must pass on Windows and on Linux (`cargo xtask wsl ci`
from Windows). GitHub Actions runs the same steps in parallel jobs (`.github/workflows/ci.yml`).
What the gate leaves out to stay near ten minutes runs nightly (`cargo xtask heavy`,
`.github/workflows/nightly.yml`). Neither runs `cargo xtask webcheck`, the web viewer's
screenshot regression check with Craft Anything: a local development aid, run on demand and
skipped without the tool ([webcheck.md](webcheck.md)). The corpus test (`bicraft-engine`,
`corpus`) runs in the gate but checks files only when `BICRAFT_CORPUS` names the pinned corpus
folder; `cargo xtask corpus verify|fetch` checks or downloads it ([corpus.md](corpus.md)).

## The gate

```text
cargo xtask ci [lint] [test] [webui] [--no-nextest]
```

| Group | Steps |
|---|---|
| `lint` | `cargo fmt --check`, `layers`, `docs`, `assets`, `profiles`, `cargo clippy --workspace --all-targets --all-features -D warnings` |
| `test` | the workspace tests except the heavy ones, then the doctests |
| `webui` | `cargo check` of the browser app for wasm32 (skipped with a note without the target) |

Without a group, all three run, cheap steps first; the run stops at the first failure and prints
each step's duration. `profiles` fails when a profile the tests use turns off overflow checks or
debug assertions (below).

The tests run with [cargo-nextest](https://nexte.st) when it is installed, and with
`cargo test` otherwise (or with `--no-nextest`). nextest runs each test in its own process, in
parallel across all test binaries, so one slow binary no longer holds the others back. It does not
run doctests: the gate runs them with `cargo test --doc` after. Install the version CI uses:

```bash
cargo install cargo-nextest --version 0.9.150 --locked
```

`.config/nextest.toml` holds the profiles: `default` (local runs), `ci` (the gate: no fail-fast,
failures repeated at the end, a JUnit report in `target/nextest/ci/junit.xml`, a test killed
after five minutes, the five engine differential tests below after ten) and `heavy` (the
nightly path, twenty minutes).

The Linux-side target dir is shared by every checkout that runs `cargo xtask wsl ci`, and cargo
records a workspace's sources relative to its root, so a test binary built from another checkout
can be reused unchanged; its `env!("CARGO_MANIFEST_DIR")` then names the other checkout. On
2026-11-24 the xpress9 tests read `tests/vectors/manifest.txt` from a deleted merge checkout and
failed with `NotFound` (a run of the crate alone, which rebuilt it, passed). Run the WSL gate from
the main checkout only, or give another checkout its own `BICRAFT_WSL_TARGET_DIR`, and clear the
Linux-side target dir if such a failure appears. `bicraft-view`'s
`the_agent_needs_its_token_and_cannot_be_a_web_page` once failed with a broken pipe and passed on a
rerun (2026-11-23).

## Build profiles

Most of the test time is spent in the storage engine, the DAX evaluator, the scan kernels and
the codecs, which are many times slower unoptimised: the engine's differential tests took over
20 minutes each at `opt-level = 0`, about 90 s at 1. The root `Cargo.toml` builds dev and test
code with `opt-level = 1` for the workspace and `opt-level = 2` without debug info for
dependencies (compiled once, then cached), and keeps line tables (`debug = "line-tables-only"`)
so panics and backtraces still name the file and line. The crates above the engine whose tests
are not compute-bound (`bicraft-cli`, `-mcp`, `-view`, `-controller`, `-auth`, `-http`,
`-desktop`, `-web`) stay at `opt-level = 0`, where their build costs more than their tests.
Optimising only the compute crates (engine, scan, DAX, codecs) is not enough: the reference
evaluator's hot code also runs in the model and data crates, and those tests then took over five
minutes. `opt-level = 2` for the engine as well brought its tests from 100 s to 62 s, for a
slower build of the largest crate; not taken. Full debug info for a debugger:
`CARGO_PROFILE_DEV_DEBUG=true`. The GitHub Windows test job builds with `CARGO_PROFILE_DEV_DEBUG=0` (no PDBs to write).

**Debug assertions and overflow checks stay on.** The never-crash rules (AGENTS.md §3) rely on
the tests panicking on an arithmetic overflow and on `debug_assert!`. `cargo xtask ci`'s
`profiles` step reads the root `Cargo.toml` and fails if the `dev` or `test` profile, or one of
their package overrides, sets `overflow-checks` or `debug-assertions` to anything but `true`, or
makes them inherit another profile; a unit test of xtask checks at run time that an overflow
panics in a test build.

## The nightly path

```text
cargo xtask heavy [--list | --filter-out]
```

`cargo xtask heavy` needs cargo-nextest. It runs:

1. **The heavy tests**, left out of the gate (`xtask/src/heavy.rs`, `HEAVY`). A test goes there
   only when it is a bigger variant of a behaviour that a gate test still covers. One today:
   `tests::speed_ui_tests::a_million_row_report_stays_responsive` (`bicraft-ui`), the UI speed
   scenarios over a 1,000,000-row report, about 20 s; the gate runs the same scenarios over
   100,000 rows (`a_large_report_stays_responsive`, about 5 s). Both run alone under nextest
   (`threads-required = "num-cpus"`); [ui-perf.md](ui-perf.md) has their budgets and numbers.
   Measured on 2026-11-04, only five tests were genuinely heavy: the engine's differential
   tests on 3,000- and 30,000-row models (`storage_tests::storage_engine_matches_the_reference_*`,
   `mem_tests::topn_over_a_grouping_{by_ranges,under_small_caps}_gives_the_same_rows`), over 20
   minutes each unoptimised. No smaller gate test covers their paths (sorted runs, dirty data,
   groupings answered by ranges under small caps), so moving them would drop coverage; with the
   build profiles above they take about 90 s each and run side by side, so they stay. Shrinking
   their models for the gate and keeping the full size nightly needs a size knob in the engine
   tests, left to the engine's owners. Every other test runs in under 20 s.
2. **Long runs** (`LONG_RUNS`): gate tests whose loops take their size from the environment, run
   again with about ten times the gate's iterations. The gate keeps running each at its usual
   size.

| Package | Tests | Nightly setting (gate default) |
|---|---|---|
| `bicraft-abf` | `fuzz_smoke` | `BICRAFT_FUZZ_SMOKE_ITERS=150000` (15000) |
| `bicraft-dax` | `fuzz_smoke` | `BICRAFT_FUZZ_SMOKE_ITERS=400000` (40000 and 30000) |
| `bicraft-layout` | `fuzz_smoke` | `BICRAFT_FUZZ_SMOKE_ITERS=6000` (600) |
| `bicraft-mcp` | `fuzz_smoke` | `BICRAFT_FUZZ_SMOKE_ITERS=20000` (2000) |
| `bicraft-metadata` | `fuzz_smoke` | `BICRAFT_FUZZ_SMOKE_ITERS=60000` (6000) |
| `bicraft-opc` | `fuzz_smoke` | `BICRAFT_FUZZ_SMOKE_ITERS=500000` (50000 and 30000) |
| `bicraft-pbir` | `fuzz_smoke` | `BICRAFT_FUZZ_SMOKE_ITERS=6000` (600) |
| `bicraft-pbix` | `fuzz_smoke` | `BICRAFT_FUZZ_SMOKE_ITERS=30000` (3000) |
| `bicraft-sqlite` | `fuzz_smoke` | `BICRAFT_FUZZ_SMOKE_ITERS=150000` (15000) |
| `bicraft-tmdl` | `fuzz_smoke` | `BICRAFT_FUZZ_SMOKE_ITERS=100000` (10000) |
| `bicraft-tmsl` | `fuzz_smoke` | `BICRAFT_FUZZ_SMOKE_ITERS=200000` (20000) |
| `bicraft-vertipaq` | `fuzz_smoke` | `BICRAFT_FUZZ_SMOKE_ITERS=500000` (50000) |
| `bicraft-xpress9` | `fuzz_smoke` | `BICRAFT_FUZZ_SMOKE_ITERS=100000` (10000 and 8000) |
| `bicraft-engine` | `storage_fuzz::storage_fuzz_matches_the_reference_evaluator`, `aggs_tests::aggregations_fuzz_matches_the_fact` | `BICRAFT_FUZZ_CASES=2400` (240 and 120) |
| `bicraft-scan` | `kernel_tests::kernel_matches_a_naive_loop_on_random_shapes` | `BICRAFT_KERNEL_CASES=640` (64) |

`--list` prints both lists; `--filter-out` prints the nextest filter of the gate's tests (every
test but the heavy ones), which the MSRV job uses. A failing long run prints its seed or saves
its input as the gate's run does (docs/fuzzing.md); replay it with the same variables.
The long runs share the gate's build (`--workspace` with a package filter), so after the
gate only the runs themselves take time: about four minutes on Linux (205 s of tests plus a few
seconds per run).

First run (2026-11-04, Linux): every long run passes except `bicraft-tmdl`'s `tmdl_read` at
`BICRAFT_FUZZ_SMOKE_ITERS=100000`: input #42697 (deterministic: the generator is seeded by the
target name) gives a model whose TMDL, once written, does not read back (`expected an object
type or property name`), a writer round-trip bug the gate's 10,000 inputs do not reach. Replay:
`BICRAFT_FUZZ_SMOKE_ITERS=100000 cargo test -p bicraft-tmdl --test fuzz_smoke`. Left to the TMDL
work; the nightly job stays red until it is fixed.

The nightly workflow also runs the whole gate on the latest stable Rust, so a new release's
lints show up there before CI's pinned toolchain is bumped. It runs at 02:17 UTC on main and on
demand (Actions > Nightly > Run workflow).

## GitHub jobs

`.github/workflows/ci.yml`, on pushes to main and on pull requests:

| Job | Runs |
|---|---|
| lint | `cargo xtask ci lint`, then a check that `Cargo.lock` did not change |
| test (ubuntu-latest, windows-latest) | `cargo xtask ci test` (nextest), JUnit report uploaded |
| webui | `cargo xtask ci webui` with the wasm32 target |
| msrv | the gate's tests and the doctests on the `rust-version` of the root `Cargo.toml` (1.95), whole workspace |
| deny | `cargo deny --all-features --locked check` over `deny.toml` (prebuilt cargo-deny from `taiki-e/install-action`, no Rust build) |

- **Toolchain.** The lint, test and webui jobs use the toolchain pinned in `RUST_TOOLCHAIN`. A
  new Rust release therefore does not empty the caches; bump it deliberately (the nightly
  latest-stable job shows what a bump brings).
- **Caches** (`Swatinem/rust-cache`, compiled dependencies). One entry per job and OS, keyed on
  the toolchain, `Cargo.lock`, the member manifests, the `CARGO_*` environment and a hash of the
  root `Cargo.toml`'s `[profile.*]` sections. Only pushes to main save; pull requests restore
  main's entries and never write, so the cache budget keeps main's. `CACHE_VERSION` starts
  every entry afresh.
- **Concurrency.** A new push to a pull request cancels its running checks; runs on main always
  finish, since they save the caches.
- **Pinned actions.** Every `uses:` names a commit SHA, with its tag in a comment
  (`actions/checkout@<sha> # v7.0.1`); a moved tag cannot change what runs. Dependabot
  (`.github/dependabot.yml`) proposes the new SHA and tag weekly, all actions in one pull request.

## Assets and dependencies

- **`cargo xtask assets`** (in `lint`): every file under `assets/`, `docs/images/` (the README's
  images) and `crates/render/geo/`, every font under `crates/ui/fonts/` and
  `crates/render/fonts/`, and the UI's icons (`crates/ui/src/icons.rs`, drawn in code) have a
  table row in `ATTRIBUTION.md` naming them in backticks: the path, a folder
  (`crates/render/fonts/`), a glob (`dir/*.png`, `dir/**`), or a bare file name in a row that
  names its folder. A folder or glob wider than the checked folder (`docs/` cited as a source)
  covers nothing in it. It fails listing the files without a row, and the rows naming a
  checked file that is gone.
- **`cargo xtask deny`** runs `cargo deny --all-features check` when cargo-deny is installed
  (`cargo install cargo-deny --version 0.20.2 --locked`), else prints a note and succeeds; the
  `deny` job runs it on every push. `deny.toml`: permissive licences only, each used today;
  per-crate exceptions with their reasons (`colored`, MPL-2.0, a test-only dependency of
  `egui_kittest`; the OFL and Ubuntu font licences of the embedded fonts); crates.io the only
  source; RustSec vulnerabilities fail, unmaintained crates fail for direct dependencies; no
  advisory is ignored (an ignore must say why it does not reach BIcraft, and goes with the
  upgrade: zip 9, skrifa and harfrust removed the last six, docs/gaps.md G38); duplicate major
  versions warn.
  An advisory published after a commit fails the job on the next run, with no change in the
  tree.
- **Dependabot**: weekly, Mondays; crates grouped (the egui stack together, other minor and
  patch bumps together, each major alone; three open pull requests at most), actions in one
  group (two at most).

## Timings

Measured on 2026-11-04 on the maintainer's laptop (shared with other builds, `CARGO_BUILD_JOBS=2`,
cold target directory), before and after this setup. GitHub timings follow the first runs on
main.

Cold `cargo xtask ci`, per step (minutes:seconds). "After" on Linux reused the dependencies of
an earlier run (as a GitHub job restoring its cache does); every workspace crate was rebuilt.

| Step | Windows before | Windows after | Linux (WSL) before | Linux (WSL) after |
|---|---|---|---|---|
| fmt | 0:11 | 0:11 | 0:39 | 0:24 |
| clippy | 4:06 | 6:29 (machine busier: clippy does not depend on the profiles) | 4:42 | 1:30 (check artifacts reused) |
| test build | 13:42 | 28:43 (dependencies cold, at opt-level 2) | 9:40 | 11:55 |
| test run | 30:13 (`cargo test`) | 2:08 (nextest) | 26:32 (`cargo test`) | 1:38 (nextest) |
| doctests | in the test run | 0:16 | in the test run | 0:22 |
| layers, docs, profiles | 1:38 (built xtask) | 0:02 | 1:31 (built xtask) | 0:06 |
| webui | 1:57 | 3:36 | skipped (no wasm32 target) | skipped |
| **total** | **~52 min** | **41:24** | **~43 min** | **15:55** |

- **Test run.** The CPU time of all tests fell from 14,200 s to 873 s on Linux (8,300 s plus
  five 20-minute timeouts to 1,313 s on Windows): 2,300 tests, all passing under nextest on both
  systems. The five engine differential tests named above take 80 to 115 s each and set the
  wall time of the run; every other test takes under 25 s.
- **Test build, dependencies warm** (what a GitHub run with a restored cache compiles), Windows:
  18:03 at `CARGO_BUILD_JOBS=2`, then `cargo xtask ci lint` 3:07.
- **Previous slowest tests** (Linux, `opt-level = 0`): the five engine differential tests over
  20 minutes each; `an_order_by_cut_by_max_rows_is_selected_under_a_cap_with_the_same_answer`
  193 s, `storage_fuzz_matches_the_reference_evaluator` 112 s, the scan kernel's
  `grouped_scans_by_ranges_give_each_group_its_whole_state` 100 s, the engine's big-dimension
  variants 24 to 85 s; now 12 s, 10 s, 18 s and 4 s or less.

### The engine's critical path

With the tests down to two minutes, the build is the gate's cost, and the engine sets it.
`bicraft-engine` is compiled twice: as the library, and again as its own unit-test binary
(`bicraft_engine lib (test)`), because its in-crate `#[cfg(test)]` tests compile the whole
library a second time under `cfg(test)`. Both start together once the dependencies are built,
and with two build jobs they share the machine; the crates above the engine and the engine's
integration tests wait for the library.

On 2026-11-05 the engine's tests that need only its public API moved to integration test
binaries (below). Measured on Windows, `CARGO_BUILD_JOBS=2`, `CARGO_INCREMENTAL=0` (as the
GitHub jobs build), dependencies warm, after touching `crates/engine/src/lib.rs`, with the
machine shared with other builds:

| `cargo build --tests -p bicraft-engine` | Before | After |
|---|---|---|
| `bicraft_engine lib (test)` | 561 s, 811 tests | 450 s, 476 tests |
| `bicraft_engine` library | 420 s | 336 s |
| integration tests after the library | 15 s (5 binaries) | 110 s (9 binaries; `model` 38 s, `report` 36 s, `files` 16 s, `commands` 6 s) |
| **wall time** (critical path: `lib (test)`) | **561 s** | **450 s, ×1.25** |

The whole gate gains nothing at two build jobs. Same machine and settings, the engine's sources
changed (everything below it warm), each side run twice:

| Windows, `CARGO_BUILD_JOBS=2` | Before | After |
|---|---|---|
| `cargo build --tests --workspace` | 10:33, 9:35 | 10:20, 10:50 |
| compile time of the engine's units, summed | 909 s, 877 s | 984 s, 941 s |
| `cargo xtask ci test` (test build, then 2,572 tests) | 11:36 | 12:23 |
| of which the nextest run (2,572 tests, 25 skipped) | 102 s | 99 s |

In the workspace build the critical path is the library, then what waits for it: the crates
above the engine, `bicraft_ui lib (test)` (80-105 s), `xtask bin (test)` (80-100 s), and now the
engine's integration binaries (about 100 s instead of 15 s), all on the same two jobs.

The second compile cannot go away while tests stay inside the crate: one in-crate test is
enough to build the library again under `cfg(test)`. And what moves out is compiled again in the
integration binaries, at a higher cost than inside `lib (test)` (the generic code each binary
instantiates from serde_json and the engine's types is compiled and linked once per binary): the
19,500 lines moved cost about 100 s there, against 47 to 111 s saved in `lib (test)`. The engine
on its own builds faster, because its two chains (`lib (test)`, and the library then the
integration tests) now end together; the gate does not, because its second chain already had
other work to wait for. One binary instead of four took 92 s instead of 104 s, within the noise
of a shared machine, so the four binaries by area stay. Removing `lib (test)` entirely
(`[lib] test = false`) would need every remaining in-crate test moved or rewritten against a
`#[doc(hidden)] pub` test surface of the engine's internals (the store, the evaluator's
switches, `refs`, `visual_props`); not done, the internals stay private. More build jobs help as
well: the measurements above use two, the GitHub runners have four cores, where the work
waiting for the library runs side by side.

### Where engine tests go

`crates/engine/tests/` holds the engine's integration tests. A new engine test goes there when
it drives the engine only through its public API (`Session::execute`, the public fields of
`Session`, `COMMANDS`, the re-exported types), in the binary of its area:

| Binary | Area |
|---|---|
| `tests/model/` | model edits, history, import, model readers and properties, partitions, perspectives, translations, Power Query, data sources, storage statistics, DAX queries |
| `tests/report/` | report commands, checks and repairs, bound values, usage, themes, interactions, filters, the snapshot |
| `tests/files/` | saving, converting, diffing, new packages, the PBIP data cache, opening in Desktop, auto-recovery |
| `tests/commands/` | the command table, its UI placement, the server-mode roles |
| `tests/<name>.rs` | a test that needs a process of its own (`hostile_params.rs` installs a panic hook) or local data (`real_reports.rs`) |

Each binary's `main.rs` mounts the files under the module path they had inside the crate
(`#[path = "."] mod snapshot { #[path = "snapshot_tests.rs"] mod tests; }`; the `#[path = "."]`
keeps the files next to `main.rs`, as a path through a missing directory does not resolve on
Linux), so a test keeps its name when it moves, and the nextest filters (`.config/nextest.toml`,
`xtask/src/heavy.rs`) match by name within `package(bicraft-engine)` whatever the binary. Add a
file to an existing binary rather than a new binary: each binary links the library again.
Helpers stay in each file; `testsupport/tempdir.rs` is shared through `#[path]`.

A test stays inside the crate (`src/*_tests.rs` or a `mod tests` block) only when it needs a
private item: the store and the evaluator internals (`preview_eval_*_tests.rs`,
`preview_tests.rs`, `memory_*_tests.rs`, `concurrency_tests.rs`), a `#[cfg(test)]` hook of the
library (the fake Databricks endpoint of `refresh_tests.rs`, the evaluator's switches), or a
private function (`visual_props::apply_props`, `refs`, the `citest`, `sources`, `sandbox` and
`pbix_struct` internals). Where only some tests of a file need one, the file is split: the
in-crate part keeps the same file name and module path and says which items it needs
(`src/save_tests.rs` and `tests/files/save_tests.rs`, for instance).
