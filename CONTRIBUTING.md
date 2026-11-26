# Contributing to BIcraft

Thanks for helping. This page covers the development setup, the checks a change must pass, and
the rules that keep BIcraft safe to run on hostile files and clean to distribute.
[AGENTS.md](AGENTS.md) is the full reference: it binds coding agents and humans alike. The plan and the current milestones are in [ROADMAP.md](ROADMAP.md).

## Development setup

- **Rust:** stable, 1.95 or newer (the MSRV in `Cargo.toml`, set by egui), with `rustfmt` and `clippy`.
- **C compiler:** the SQLite tests use real SQLite as an oracle through the `rusqlite`
  dev-dependency, which compiles SQLite's bundled source (MSVC Build Tools on Windows, `cc` on
  Linux). It is a test-only dependency; BIcraft itself does not link SQLite.
- **Linux:** a change must build and pass its tests on Linux as well as on Windows. On Windows,
  use WSL (an Ubuntu distro with `cargo` installed) and run `cargo xtask wsl ci`.
- **Build output:** if your checkout lives in a synced folder (OneDrive, Dropbox…), set
  `CARGO_TARGET_DIR` to a folder outside it, so build artifacts are not synced.

```bash
cargo build                                   # the workspace
cargo build --release -p bicraft-cli         # the bicraft binary
cargo test -p bicraft-dax                    # one crate
cargo test -p bicraft-dax some_test_name     # one test
```

## Checks

```bash
cargo xtask ci        # fmt --check, layers, docs, assets, profiles, clippy -D warnings, tests, doctests, wasm check
cargo xtask ci lint   # one group: lint, test or webui (the GitHub jobs run them side by side)
cargo xtask heavy     # the nightly path: heavy tests and long fuzz loops (docs/ci.md)
cargo xtask wsl ci    # the same inside WSL Ubuntu (target dir ~/target/bicraft); runs locally on Linux
cargo xtask layers    # the layering check alone
cargo xtask docs      # no Desktop timing in committed Markdown; licenses/ up to date
cargo xtask assets    # every file under assets/, the bundled fonts and the UI's icons in ATTRIBUTION.md
cargo xtask deny      # cargo deny check (deny.toml): licences, sources, advisories; skipped if not installed
cargo xtask bench     # performance benchmarks (docs/benchmarks.md); not part of the gate
```

A change is done when `cargo xtask ci` passes on **both** Windows and Linux. GitHub Actions runs
the same gate on `windows-latest` and `ubuntu-latest` in parallel jobs, and it also builds and
tests on the MSRV (`.github/workflows/ci.yml`); `.github/workflows/nightly.yml` runs the heavy
path. Install [cargo-nextest](https://nexte.st) for a much faster test step (`cargo install
cargo-nextest --version 0.9.150 --locked`); without it the gate falls back to `cargo test`.
[docs/ci.md](docs/ci.md) explains the jobs, the caches, the build profiles and the nightly path.

Dependencies are checked by [cargo-deny](https://github.com/EmbarkStudios/cargo-deny) (`deny.toml`:
permissive licences only, crates.io only, RustSec advisories, duplicate major versions as
warnings); CI runs it in its own job, and `cargo xtask deny` runs it locally once it is installed
(`cargo install cargo-deny --version 0.20.2 --locked`). Dependabot opens grouped update pull
requests weekly for crates and GitHub Actions; actions are pinned by commit SHA with the tag in a
comment.

## Architecture rules

- **Layering.** Each crate belongs to one layer (L0 primitives → L1 model → L2 DAX → L3
  serializations → L4 containers → L5 engine → L6 MCP → apps), and it may depend only on strictly
  lower layers. Register a new crate in `xtask/src/layers.rs`. Nothing below L6 depends on
  `engine`, and nothing below L5 uses an async runtime.
- **Everything is a command.** Each user-visible operation is an `engine` command with an id, a
  params doc, `run`, and tests. The CLI and the MCP server reach every operation through these
  commands.
- **Conversion goes through `model`.** Format crates never call each other.
- **No runtime dependency on Windows or Power BI.** XPress9, ABF, VertiPaq and SQLite are pure
  Rust. Writers emit byte-identical output on Windows and Linux (watch CRLF/LF and path
  separators).

## Never crash

Every input file is treated as hostile.

- No `unwrap`, `expect`, `panic!`, `todo!`, `unimplemented!` or `unreachable!` in non-test code.
  Each crate root should deny the matching clippy lints (`#![deny(clippy::unwrap_used, …)]`,
  see `crates/opc/src/lib.rs`), with tests exempted by `clippy.toml`.
- `unsafe_code = "forbid"` across the workspace.
- Index with `get()`. Use checked arithmetic. Cap every allocation sized from input, and bound
  every recursion.
- Every crash fix lands with a regression test.
- Readers are fuzzed in two layers ([docs/fuzzing.md](docs/fuzzing.md)): stable-Rust smoke
  tests (`crates/<crate>/tests/fuzz_smoke.rs`) run with every `cargo test`, and cargo-fuzz
  targets under `fuzz/` run on demand (nightly, Linux). A new reader gets both.

## Test data

- **Committed fixtures are files we generate ourselves**, with our own code or with scripts in
  the repo. Do not commit Microsoft assets, or `.pbix` / `.pbit` / `.abf` files authored by
  anyone else.
- **Real Power BI files stay local.** Keep them outside the repository and point the tests at
  them with the `BICRAFT_TESTDATA` environment variable. Tests that need them print a note and
  skip when the variable is unset or a file is missing, so CI never depends on them.
- **Tests clean up after themselves.** Scratch files go in a `TempDir` guard from
  `testsupport/tempdir.rs` (each crate includes it with `#[path]`, so it adds no dependency).
  Dropping the guard removes the folder, even when the test panics. The file lists the few
  deliberate exceptions.
- **Never commit client or third-party files.** Do not copy them into the repo or quote their
  content in docs, tests, issues or commit messages.

## Sources and provenance

BIcraft implements file formats for interoperability. The rules, in brief (details in
[AGENTS.md](AGENTS.md#1-sources-and-provenance-absolute) and
[docs/interoperability.md](docs/interoperability.md)):

- Use public specifications and documentation (`[MS-XLDM]`, the PBIR/PBIP JSON schemas, the
  TMDL and TMSL docs), open-source code under its licence, and black-box tests against Power BI
  Desktop (opening our files, reading the files it writes) and its engine (DAX results checked
  during development; the verdicts are stored as our own data).
- **Do not analyse vendor binaries**: no disassembly, decompilation, CIL or metadata reading,
  byte scans, or reflection on non-public types of Microsoft software. Desktop is used only as a
  black box.
- Write each finding as a prose or table note under [docs/formats/](docs/formats/), citing its
  public source or the black-box observation, and implement from the note. **Never paste
  Microsoft code or documentation prose into the source.**
- Publish no Power BI Desktop benchmark result (its licence forbids disclosing benchmark
  results); `cargo xtask docs` checks the Markdown.
- Third-party code needs a compatible license, an entry in
  [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) and [ATTRIBUTION.md](ATTRIBUTION.md), and,
  when the binary embeds it, its licence file under `licenses/`. The XPress9 port is the model to
  follow: each ported file names its upstream file and commit.
- Every asset (a file under `assets/`, a bundled font, an icon) needs an
  [ATTRIBUTION.md](ATTRIBUTION.md) row: `cargo xtask assets` fails without one. A new crate needs a
  licence on the `deny.toml` allowlist.

## Commits

- Run `cargo xtask ci` before committing.
- A pull request fills the checklist of
  [`.github/pull_request_template.md`](.github/pull_request_template.md) (never crash, provenance,
  client data, server policy, buttons, docs, CI on both systems).
- Start the commit message with the milestone id from [ROADMAP.md](ROADMAP.md), followed by a
  short summary in the imperative. Examples: `M1.2: TMDL writer`, `M7.1: release workflow`.
- When an item lands, update its status in ROADMAP.md and add a line at the top of the progress
  log, `docs/history/roadmap-log.md`.

## Security

Report vulnerabilities privately, never in a public issue: see [SECURITY.md](SECURITY.md).

## License

By contributing, you agree that your contributions are dual licensed under MIT OR Apache-2.0, as
described in the [README](README.md#license), without any additional terms or conditions.
