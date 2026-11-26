# Fuzzing (M7.3)

Input files are hostile. The project rule is **never crash**: every reader must return an `Err` or a value for
every input, with no panic, no hang and no unbounded allocation. Two layers check this:

| Layer | Where | Toolchain | When |
|---|---|---|---|
| Smoke tests | `crates/<crate>/tests/fuzz_smoke.rs` | stable, Windows + Linux | every `cargo test` / `cargo xtask ci`; ten times the inputs nightly (`cargo xtask heavy`, docs/ci.md) |
| cargo-fuzz targets | `fuzz/` (libFuzzer, coverage-guided) | nightly, Linux (WSL) | on demand (campaigns) |

Both layers run the **same logic on the same input encodings**, so a reproducer found by one replays in the
other.

## Targets

| Target | Crate | What it calls | Input encoding |
|---|---|---|---|
| `opc_package` | opc | `Package::read_bytes`, `decode_text` on every part, `to_bytes` | ZIP bytes |
| `opc_text` | opc | `decode_text`, then `encode_text` → `decode_text` must give the same text | raw bytes |
| `pbix_package` | pbix | `Package::read_bytes`, `inventory`, `read_model`, `read_report` | ZIP bytes |
| `pbix_parts` | pbix | the same on a package built from the parts (skips the ZIP CRC check, so mutations reach the parts) | documents |
| `pbiviz_read` | pbix | `pbiviz::read_pbiviz` (a custom visual package, `visual.import_custom`); a package that reads keeps only `package.json` and `resources/…` | ZIP bytes |
| `import_readers` | engine | `readers::read_bytes` (`file.open` of `.vpax`, `.pbids`, M text, `.pqt`, CDM `model.json`, a Dataflow's `queryMetadata.json` with its `mashup.pq`, M11.36); what reads round-trips through TMSL | three frames: the reader, the file, the M document |
| `xpress9_decompress` | xpress9 | `decompress` (output capped at 4 MiB) | u32 LE expected length, then the stream |
| `xpress9_datamodel` | xpress9 | `inspect_datamodel`, `datamodel::detect_layout`, `decompress_datamodel` (cap 16 MiB) | `DataModel` part |
| `xpress8_decompress` | xpress8 | `decompress_chunked` (cap 1 MiB) and `decompress_block` (first byte × 64 bytes) | chunked file |
| `powerpivot_workbook` | powerpivot | `read_workbook` and `read_backup` with small caps (4 MiB files, 1 MiB XML, 65,536 cells) | `.xlsx` package or `item.data` backup |
| `abf_parse` | abf | `Abf::parse`, every accessor, `to_writer().finish()`, which must parse again with the same file count | ABF bytes |
| `sqlite_database` | sqlite | `Database::from_bytes`, `column_names` and every row of every table (and `sqlite_master`) | SQLite image |
| `vertipaq_idfmeta` | vertipaq | `parse_idfmeta`, `to_bytes` | `.idfmeta` |
| `vertipaq_idf` | vertipaq | `parse_idfmeta`, `split_idf`, `decode_data_ids` (max 2^16 rows) | frames `[idfmeta, idf]` |
| `vertipaq_dictionary` | vertipaq | `parse_dictionary` | `.dictionary` |
| `vertipaq_hidx` | vertipaq | `parse_hash_index` | `.hidx` |
| `vertipaq_column` | vertipaq | `decode_column_with_limits` (max 2^16 rows) under 8 encodings × 8 logical types | frames `[selector, idfmeta, dictionary, idf]` |
| `metadata_sqlitedb` | metadata | `read_sqlitedb` | SQLite image |
| `metadata_rows` | metadata | `build_model` then `apply_model_at` on the fixture rows edited by a byte script (6-byte ops: set int/text/real/blob/null, remove or duplicate a row) | script |
| `tmsl_read` | tmsl | `read`; a model that reads must `write` and read back | JSON (lossy UTF-8) |
| `tmdl_read` | tmdl | `parse` per file, `read_documents`; a model that reads must `write_documents` and read back | documents |
| `pbir_read` | pbir | `validate_documents`, `read_documents`, `check_names`; a report that reads must write and read back | documents |
| `layout_read` | layout | `read_layout`, `read_layout_with_summary` | JSON (lossy UTF-8) |
| `dax_expression` | dax | `parse_expression`, `validate` (no model), `validate_as` against a model, kind chosen by input length | text (lossy UTF-8) |
| `dax_query` | dax | `parse_query`, `validate_query` with and without a model | text (lossy UTF-8) |
| `mcp_handle` | mcp | `handle` on a fresh `Server`, one JSON-RPC message per line (≤ 64); every reply must be JSON | lines |
| `tds_tokens` | sqlserver | `Tokens::next_token` until the end or an error (rows decoded against their `COLMETADATA`) | a TDS token stream |
| `odata_payloads` | webdata | `csdl::parse` (`$metadata`), the URL checks, `odata::date` / `time` / `base64`, `odata::page_rows` against a column of every type | text or bytes |

Encodings (implemented in `fuzz/src/lib.rs` and in each smoke test's `smoke` module):

- **frames(n)**: the first `n − 1` frames are `[u32 LE length][bytes]` (length clamped to what is left); the
  last frame is the rest.
- **documents**: repeated `[u16 LE name length][name][u32 LE data length][data]` (clamped, at most 4096).
  Names are paths (`tables/Sales.tmdl`, `definition/report.json`, `Report/Layout`, …).

Model contexts: the `dax_*` fuzz targets bind against `crates/dax/tests/oracle/model.bim` (read with
`bicraft-tmsl`); the dax smoke test cannot depend on tmsl (layering) and builds a similar model by hand.

**MCP safety**: `mcp_handle` drops, before they reach the server, every message in which any decoded JSON key
or string contains (ASCII case-insensitive) `open`, `inspect`, `save`, `project`, `file`, `path` or `dir`. That
removes `open_file`, `inspect_file`, `save`, `save_as`, `new_project`, the `file.*` / `project.*` commands and
any path argument, so fuzzing never reads or writes files. The smoke test `fs_messages_are_filtered` checks
the filter.

## Smoke tests (stable Rust)

Each `fuzz_smoke.rs` runs every seed, then N mutated inputs, through the target's logic. The mutation engine
is ~150 lines copied in each file (crates cannot share test code without a new crate; keep the copies
identical): a xorshift64 PRNG seeded from the target name, and 1–6 mutations per input (bit flip, random
byte, truncation, insertion, range deletion, interesting integers at widths 1/2/4/8, splice and crossover with
another seed, chunk duplication, dictionary tokens, small increments). Deterministic: the same inputs on every
run and platform.

Each run fails when an input **panics** (the input is saved to `<temp>/bicraft-fuzz-smoke-<target>-crash.bin`),
takes longer than **3 s** in a debug build (`-slow.bin`), or makes no progress for **60 s** (a watchdog saves
`-hang.bin` and aborts the process).

| Crate | Targets | Inputs per run | Debug time (Windows) |
|---|---|---|---|
| opc | opc_package, opc_text | 30 000 + 50 000 | 0.5 s |
| xpress9 | xpress9_decompress, xpress9_datamodel | 10 000 + 8 000 | 4 s |
| abf | abf_parse | 15 000 | 5.5 s |
| sqlite | sqlite_database | 15 000 | 2.6 s |
| vertipaq | 5 targets | 5 × 50 000 | 0.5 s |
| metadata | metadata_sqlitedb, metadata_rows | 6 000 + 6 000 | 4.5 s |
| tmsl | tmsl_read | 20 000 | 3 s |
| tmdl | tmdl_read | 10 000 | 6–9 s |
| pbir | pbir_read | 600 | 5.5 s |
| layout | layout_read | 10 000 | 6–8 s |
| dax | dax_expression, dax_query | 40 000 + 30 000 | 1.7 s |
| mcp | mcp_handle (+ filter check) | 2 000 | 4–9 s |
| pbix | pbix_package, pbix_parts, pbiviz_read | 3 000 + 3 000 + 3 000 | 3 s (pbiviz_read: 0.05 s) |
| engine | import_readers | 3 000 | 0.3 s |
| sqlserver | tds_tokens | 20 000 | 0.1 s |
| webdata | odata_payloads | 20 000 | 1 s (estimated) |

Environment variables:

- `BICRAFT_FUZZ_SMOKE_ITERS=<n>`: mutated inputs per target (overrides the default), e.g. a long local run.
- `BICRAFT_FUZZ_REPLAY=<absolute path>`: run one saved input (a smoke reproducer or a cargo-fuzz artifact) through the
  target and nothing else (tests run in the crate directory, so use an absolute path); with a test filter, e.g.
  `BICRAFT_FUZZ_REPLAY=$PWD/crash.bin cargo test -p bicraft-tmdl --test fuzz_smoke tmdl_read`.
- `BICRAFT_FUZZ_SEED_DIR=<dir>`: write each target's seeds to `<dir>/<target>/seed-NNN`. This is how
  `fuzz/corpus/` is generated:
  ```bash
  BICRAFT_FUZZ_SEED_DIR="$PWD/fuzz/corpus" BICRAFT_FUZZ_SMOKE_ITERS=0 cargo test --test fuzz_smoke \
    -p bicraft-opc -p bicraft-xpress9 -p bicraft-abf -p bicraft-sqlite -p bicraft-vertipaq \
    -p bicraft-metadata -p bicraft-tmsl -p bicraft-tmdl -p bicraft-pbir -p bicraft-layout \
    -p bicraft-dax -p bicraft-mcp -p bicraft-pbix -p bicraft-sqlserver -p bicraft-webdata
  ```

## Seeds

Only our own committed material: the crates' fixtures (`crates/sqlite/tests/fixtures/*.db`,
`crates/metadata/tests/fixtures/synthetic_metadata.sqlitedb`, `crates/tmsl/tests/fixtures/sales.bim`,
`crates/tmdl/tests/fixtures/*`, `crates/pbir/tests/fixtures/sales.Report`,
`crates/layout-bb/tests/fixtures/shop_layout.json`, `crates/xpress9/tests/vectors/*.x9`, the DAX oracle corpus),
plus files built in the smoke tests (synthetic ABF backups, VertiPaq column files from the layouts in
`docs/formats/vertipaq.md`, `bicraft_pbix::synthetic::pbix`, ZIP packages, MCP sessions). **Never put files
from `$BICRAFT_TESTDATA` (client material) in `fuzz/corpus` or `fuzz/artifacts`.**

`fuzz/corpus/<target>/` holds the seeds (1.8 MB). The corpus libFuzzer grows during a campaign goes to a
separate directory outside the repo (see below), never into `fuzz/corpus`.

## cargo-fuzz campaigns (WSL)

One-time setup inside WSL Ubuntu:

```bash
rustup toolchain install nightly --profile minimal
cargo install cargo-fuzz --locked
```

`fuzz/` is its own workspace (excluded from the root one), so `cargo xtask ci` never builds it. Build outside
the checkout (on a Linux-side target dir):

```bash
cd fuzz   # from the repository root
cargo +nightly fuzz build --target-dir ~/target/bicraft-fuzz        # all 22 targets, ~2.5 min
cargo +nightly fuzz list
```

Run one target (the first corpus directory receives new inputs; the second is the read-only seed set):

```bash
mkdir -p ~/fuzzwork/corpus/tmdl_read ~/fuzzwork/artifacts/tmdl_read
cargo +nightly fuzz run --target-dir ~/target/bicraft-fuzz tmdl_read \
  ~/fuzzwork/corpus/tmdl_read fuzz/corpus/tmdl_read -- \
  -max_total_time=300 -rss_limit_mb=1536 -timeout=10 \
  -artifact_prefix=$HOME/fuzzwork/artifacts/tmdl_read/ -print_final_stats=1
```

- `-timeout=10`: an input slower than 10 s is a finding (`timeout-*`).
- `-rss_limit_mb=1536`: peak RSS above 1.5 GB (or one allocation above it) is a finding (`oom-*`). The
  binaries run under AddressSanitizer; a quiet target sits around 400–600 MB.
- Run several targets in parallel (one process each, `xargs -P`); 11 at a time fit in 20 cores / 16 GB.

A crash, timeout or OOM writes `crash-*`, `timeout-*` or `oom-*` into the artifact prefix. Minimize it with
`cargo +nightly fuzz tmin <target> <file>`, copy the minimized file to `fuzz/artifacts/<target>/` with a
descriptive name, list it below, and add a regression test in the crate with the fix.

## Campaign results

2026-10-13, WSL Ubuntu, 20 cores, nightly + cargo-fuzz 0.13.2 (libFuzzer, ASan), every target 300 s,
11 targets in parallel. "cov" is libFuzzer's edge coverage, "ft" its feature count.

| Target | Executions | exec/s | cov | ft | Peak RSS | Crashes / timeouts / OOMs |
|---|---:|---:|---:|---:|---:|---|
| opc_package | 1 701 013 | 5 651 | 2 503 | 9 735 | 592 MB | 0 / 0 / 0 |
| opc_text | 29 111 401 | 96 715 | 104 | 238 | 582 MB | 0 / 0 / 0 |
| pbix_package | 239 978 | 797 | 8 649 | 17 648 | 504 MB | 0 / 0 / 0 |
| pbix_parts | 132 525 | 440 | 9 077 | 24 333 | 517 MB | 0 / 0 / 0 |
| xpress9_decompress | 448 679 | 1 490 | 520 | 1 665 | 391 MB | 0 / 0 / 0 |
| xpress9_datamodel | 861 219 | 2 861 | 684 | 2 344 | 479 MB | 0 / 0 / 0 |
| abf_parse | 430 058 | 1 428 | 1 843 | 6 038 | 449 MB | 0 / 0 / 0 |
| sqlite_database | 492 811 | 1 637 | 2 237 | 8 512 | 498 MB | 0 / 0 / 0 |
| vertipaq_idfmeta | 24 970 405 | 82 958 | 387 | 714 | 468 MB | 0 / 0 / 0 |
| vertipaq_idf | 15 587 870 | 51 786 | 503 | 1 035 | 467 MB | 0 / 0 / 0 |
| vertipaq_dictionary | 5 881 481 | 19 539 | 1 092 | 3 179 | 391 MB | 0 / 0 / 0 |
| vertipaq_hidx | 15 325 043 | 50 913 | 366 | 755 | 487 MB | 0 / 0 / 0 |
| vertipaq_column | 7 732 296 | 25 688 | 1 433 | 3 876 | 419 MB | 0 / 0 / 0 |
| metadata_sqlitedb | 149 121 | 495 | 3 547 | 10 527 | 495 MB | 0 / 0 / 0 |
| metadata_rows | 72 615 | 241 | 2 545 | 7 942 | 479 MB | 0 / 0 / 0 |
| tmsl_read | 518 626 | 1 723 | 2 278 | 8 792 | 469 MB | 0 / 0 / 0 |
| tmdl_read | 109 074 | 362 | 5 717 | 25 945 | 488 MB | 0 / 0 / 0 (round-trip check then skipped for inputs with ```; F1, since fixed) |
| pbir_read | 6 552 | 21 | 3 931 | 14 411 | 496 MB | 0 / 0 / 0 |
| layout_read | 117 420 | 390 | 2 901 | 8 459 | 472 MB | 0 / 0 / 0 |
| dax_expression | 712 997 | 2 368 | 3 829 | 13 124 | 500 MB | 0 / 0 / 0 |
| dax_query | 676 601 | 2 247 | 5 573 | 15 407 | 492 MB | 0 / 0 / 0 |
| mcp_handle | 20 406 | 67 | 7 008 | 22 922 | 526 MB | 0 / 0 / 0 |

No panic, timeout (> 10 s) or OOM (> 1.5 GB) in about 105 million executions. No slow unit was reported.
`pbir_read` (~48 ms per input under ASan, mostly `validate_documents`) and `mcp_handle` are the slowest targets
and deserve longer campaigns.

## Findings

| Id | Target | Kind | Reproducer | Location | Status |
|---|---|---|---|---|---|
| F1 | tmdl_read | round trip (not a crash) | `fuzz/artifacts/tmdl_read/roundtrip-fence-in-expression` | `crates/tmdl/src/write.rs`, `Writer::expression` / `needs_fence` | fixed (M7.3) |
| F2 | (probe) vertipaq | memory amplification within the cap | see below | `crates/vertipaq/src/lib.rs`, `DEFAULT_MAX_ROWS` | fixed (M7.3) |
| F3 | (probe) xpress9 / pbix | memory amplification within the cap | see below | `crates/pbix/src/lib.rs`, `MAX_DATA_MODEL_BYTES` | fixed (M7.3) |
| F4 | (code reading) opc | no aggregate cap | — | `crates/opc/src/package.rs`, `Package::read` | fixed (M7.3) |
| F5 | tmdl_read (smoke, 100 000 inputs) | round trip (not a crash) | smoke input #42697 (`BICRAFT_FUZZ_SMOKE_ITERS=100000`) | `crates/tmdl/src/write.rs`, `Emitter::prop` / `classify` | fixed (2026-11-04) |

**F1: TMDL writer emits a fence the reader cannot close.** The reader accepts an indented multi-line
expression containing a line that is just ```` ``` ````. When the writer fences that expression (because it
also has leading/trailing whitespace or an empty last line), the inner ```` ``` ```` line closes the fence
early and the output fails to read: `tables/T.tmdl:9: expected an object type or property name, found
```````. Minimal model: table `T`, column `C`, property `e` = `"a\n```\nb"`. Fix idea: never fence an expression
that contains a ```` ``` ```` line (or choose a representation the reader round-trips). The smoke test and the
fuzz target skip the round-trip check for inputs containing ```` ``` ```` (`has_fence`); remove that skip with
the fix. To replay: `BICRAFT_FUZZ_REPLAY=$PWD/fuzz/artifacts/tmdl_read/roundtrip-fence-in-expression cargo test -p bicraft-tmdl --test fuzz_smoke`.

*Fixed in M7.3 (fuzz findings).* TMDL has no escape inside a fence, so the writer never fences a body
with a line that is ```` ``` ```` apart from whitespace (`closes_fence`); it writes the indented form,
which our reader reads with leading and inner whitespace intact. Only whitespace and empty lines at
the very end of such a body are lost (the indented form trims them). The `has_fence` skips are gone
from `crates/tmdl/tests/fuzz_smoke.rs` and `fuzz/src/lib.rs`; regression test
`expression_with_a_fence_line_round_trips` (`crates/tmdl/tests/round_trip.rs`).

**F2: VertiPaq row cap allows ~4 000 000× amplification.** A 144-byte `.idf` plus a 264-byte `.idfmeta` (one
RLE run of 2^26 rows) makes `decode_column` with default `Limits` return 67 108 864 cells: 24 B each (1.6 GB)
plus 256 MB of data ids, in 0.65 s (release). Bounded, but enough to exhaust a small machine from a tiny
file. The fuzz targets use `max_rows = 2^16` for that reason. Suggestion: callers derive `Limits::max_rows`
from the partition/table row count in `metadata.sqlitedb`, or check runs against `SegmentStats::row_count`
and the segment's `records`, and/or lower the default.

*Fixed in M7.3 (fuzz findings).* RLE legitimately describes any number of rows in 8 bytes (a constant
column), so the input size cannot bound the output; the limits do, before allocation. `Limits` gains
`max_bytes` (default `DEFAULT_MAX_BYTES` = 1 GiB: 4 B per data id, plus one `CellValue` per cell and the
bytes of every text cell copied from the dictionary), and `DEFAULT_MAX_ROWS` drops from 2^26 to 2^24
(16 Desktop segments). `decode_data_ids` no longer reserves the declared row count up front; a hybrid
segment's RLE entries must cover its `records` before anything is reserved for them. The engine preview
keeps passing its own `max_rows`. Regression tests `rle_bomb_is_refused_by_default_limits`,
`byte_budget_bounds_ids_cells_and_text`, `claimed_rows_need_rle_entries`
(`crates/vertipaq/tests/synthetic.rs`).

**F3: `DataModel` decompression allows ~43 000× amplification.** `compress_datamodel` turns 256 MiB of
zeros into a 6 249-byte part; `pbix::read_model` decompresses with `MAX_DATA_MODEL_BYTES` = 4 GiB, so a
~100 KB `DataModel` part can make it allocate 4 GiB. Suggestion: cap relative to the part size (Power BI's
real ratio is far below 1 000×) or make the limit configurable from the engine.

*Fixed in M7.3 (fuzz findings).* `pbix::DataModelLimits` (used by `read_model` / `write_model` and their
`_with` variants) accepts `min(max_bytes, max(part size × max_ratio, min_allowance))`: by default
4 GiB, 1 000× and 64 MiB (Desktop's parts expand about 7×). The sizes declared by the chunk envelopes are <!-- perf-guard: ok (a size ratio) -->
checked first, so a lying part fails with `Error::DataModelTooLarge` before any decoding;
`xpress9::decompress_datamodel` also sums the declared sizes against its `limit` up front. Regression
tests `data_model_expansion_is_capped_relative_to_the_part` (`crates/pbix/tests/write_model.rs`) and
`declared_sizes_are_checked_before_decoding` (`crates/xpress9/tests/datamodel_writer.rs`).

**F4: no total cap across ZIP parts.** `MAX_PART_BYTES` (4 GiB) applies per part; a package with many
deflate-bomb parts (deflate reaches ~1 000×, overlapping entries more) can exceed memory in total.
Suggestion: a total decompressed-bytes budget in `Package::read`.

*Fixed in M7.3 (fuzz findings).* `opc::ReadLimits { max_part_bytes, max_total_bytes }` (defaults
`MAX_PART_BYTES` = 4 GiB, `MAX_PACKAGE_BYTES` = 8 GiB) with `Package::read_with` / `read_bytes_with` /
`open_with`; `read`/`read_bytes`/`open` use the defaults. Each part is checked against its declared size
and then read through a bound of `min(part cap, budget left)`, so understated sizes cannot get past
either cap. Errors: `PartTooLarge { part, limit }`, `PackageTooLarge { limit }`. Regression tests
`total_decompressed_size_is_capped` and `lying_sizes_are_caught_while_reading`
(`crates/opc/src/package.rs`).

**F5: TMDL writer emits JSON keys as property lines.** Found by a 100 000-input smoke run
(`BICRAFT_FUZZ_SMOKE_ITERS=100000 cargo test -p bicraft-tmdl --test fuzz_smoke`, input #42697; the
gate's default 10 000 does not reach it). An extended property's JSON value (`= {...}`) can carry
any key; under an unknown block of the model it went through the nested property writer, which
wrote each key as a bare TMDL line (`a b: 1`, or U+FFFD followed by ` = [true,null]`). The reader takes a
property name as a run of letters, digits and `_`, so the output did not read back: `model.tmdl:31:
expected an object type or property name`. Minimal model: `model Model` / `block` /
`extendedProperty Ext =` with the JSON `{"a b": 1}`.

*Fixed 2026-11-04.* A nested object, or a collection of objects, with a key that is not a TMDL
keyword (or whose singular keyword would be empty, as for a key `s`) is written as one
`key: <compact JSON>` line and reads back as that text, which writes the same line again. A
`translations` block is checked the same way, through its nested collections. An object's own
unknown key that is no keyword (only TMSL can bring one) is dropped rather than written
unreadable. The reader now takes `ref = x` and `ref: x` as a property named `ref` instead of a
broken reference. Regression test `json_keys_that_are_not_keywords_read_back`
(`crates/tmdl/tests/round_trip.rs`).

## Where things live

| Path | What |
|---|---|
| `fuzz/Cargo.toml` | the cargo-fuzz crate (`[workspace] members = ["."]`, excluded from the root workspace) |
| `fuzz/src/lib.rs` | what each target does, input encodings, MCP filter, metadata script |
| `fuzz/fuzz_targets/<target>.rs` | one-line libFuzzer entry points |
| `fuzz/corpus/<target>/` | seeds (regenerate with `BICRAFT_FUZZ_SEED_DIR`) |
| `fuzz/artifacts/<target>/` | minimized reproducers of findings |
| `crates/*/tests/fuzz_smoke.rs` | the stable smoke tests |
