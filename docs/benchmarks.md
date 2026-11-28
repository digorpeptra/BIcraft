# Benchmarks

`cargo xtask bench` measures BIcraft on synthetic models, reports and packages. It can also run
on the local real sample. Each case is measured in two ways:

- **in-process**: the crates are called directly, the way the engine calls them
- **end to end**: through the release `bicraft-cli` binary, process start included

The harness lives in `xtask/src/bench.rs`, the cases in `xtask/src/bench/inproc.rs`, and the
generators in `xtask/src/bench/synth.rs` and `xtask/src/bench/vertipaq.rs`.

`cargo xtask engine-bench` measures BIcraft's DAX evaluator on a star schema from 10 k to 100 M
fact rows and checks its answers against reference results: see
[the DAX engine bench](#the-dax-engine-bench). Two variants add a dimension of millions of rows
and skewed data: [big dimension and skewed data](#big-dimension-and-skewed-data-2026-10-26).

```text
cargo xtask bench [--quick] [--real] [--real-file PATH] [--filter TEXT] [--json PATH]
                  [--no-build] [--cli-only] [--keep]
```

| Option | Effect |
|---|---|
| `--quick` | Smaller sizes and a quarter of the runs. Skips the 10,000-measure, 1,000-table, 100-page and 64 MB cases. About 1–1.5 minutes of measuring on the machine below; under 2 minutes in total once the release builds are cached. |
| `--real` | Adds the local-only sample `$BICRAFT_TESTDATA/sample1/report.pbix`; `--real-file` points at another file. Only timings and counts are printed. Nothing from the file is written anywhere. |
| `--filter TEXT` | Runs only the cases whose group or name contains `TEXT` (case-insensitive). |
| `--json PATH` | Also writes the raw results (median, p95 and min seconds, items, bytes). |
| `--no-build` | Uses the existing `target/release/bicraft-cli`. Without it, the bench first runs `cargo build --release -p bicraft-cli`. |
| `--cli-only` | Runs only the end-to-end cases (CLI and MCP). |
| `--keep` | Keeps the generated inputs (under the OS temp dir), for profiling a case by hand. |

## Method

- **Builds.**
  - `cargo xtask bench` re-runs xtask as
    `cargo run --release -p xtask --features bench -- bench …`.
  - The in-process cases need the workspace crates, which xtask links only with the `bench`
    feature. Without the feature, the other xtask commands do not have to build the whole
    workspace.
  - Both binaries use the workspace release profile: thin LTO, `codegen-units = 1`, stripped.
- **Harness.**
  - Each case runs once untimed as a warm-up. A case that takes 0.5 s or more keeps its warm-up
    as a sample.
  - Then up to N timed runs, within a budget of 20 s per case (4 s with `--quick`).
  - The harness reports the median, the p95 (nearest rank) and the throughput.
  - Inputs that a case modifies, such as project copies, are prepared outside the timed part.
- **Inputs.** Every input is generated with a fixed seed into `<temp>/bicraft-bench-<pid>`, which
  is deleted at the end.

  | Input | Shape |
  |---|---|
  | Models (TMSL `model.bim`) | `N` tables × 20 typed columns × 10 measures, M partitions, a chain of relationships |
  | Measure DAX | A mix of `SUM`, `CALCULATE`/`FILTER`/`ALL`, `SUMX`, `VAR`/`DIVIDE`, `DISTINCTCOUNT`, `IF`, `REMOVEFILTERS`/`KEEPFILTERS`, `SWITCH` |
  | Measure references (`model.validate` cases) | Each measure references the previous one in chains of 100, plus two 3-measure cycles |
  | Reports | Built with `page.add`/`visual.add`: 20 visuals per page across 5 visual types, bound to columns, measures and aggregations |
  | PBIX | The `synthetic` module of `crates/pbix` around the metadata fixture, optionally with 1,000 extra measures and 8 or 32 MB of storage padding |
  | XPress9 input | ABF-like bytes: UTF-16 XML, packed small integers, repeated strings, 25% noise |
  | VertiPaq columns | Hybrid RLE + bit-packed segments of 1,048,576 rows, 1,000 distinct values |
- **Variance.** Other agents built the workspace and Power BI Desktop was running while these
  numbers were taken. Absolute numbers vary by up to 2–4× between runs on Windows (for example,
  XPress9 compression of 1 MB took 54 ms, 71 ms and 223 ms in three runs). Compare cases within
  one run, and re-run a case with `--filter` before drawing conclusions from a single number.

## Machine

| | |
|---|---|
| CPU | AMD Ryzen AI 9 365 w/ Radeon 880M (20 threads) |
| Windows | Windows 11, 10.0.26200; inputs on NTFS under `%TEMP%`; Defender on |
| WSL | Linux 6.18.54.1-microsoft-standard-WSL2 (Ubuntu); inputs on ext4 under `/tmp` |
| rustc | 1.99.0 (b940084d7 2026-09-28) |
| Date | 2026-10-13 |

## Headline numbers

The two columns are full runs (`cargo xtask bench --real`). The complete tables are at the end.

| What | Windows | WSL |
|---|---:|---:|
| Process start (`bicraft-cli --version`) | 28.6 ms | 2.3 ms |
| `dax.validate`, one expression, no model | 7.0 µs | 2.9 µs |
| `dax.validate`, one expression, 1,000-measure model | 23 µs | 11 µs |
| `dax.validate`, 10,000 expressions, 1,000-measure model | 1.16 s | 0.62 s |
| `model.validate`, 100 / 1,000 / 10,000 measures (after the G5 fix) | 11 ms / 33 ms / 0.22 s | 9 ms / 35 ms / 0.20 s |
| TMSL read, 1,000 tables (5.4 MB) | 144 ms | 76 ms |
| TMDL read from folder, 1,000 tables | 442 ms | 173 ms |
| PBIR read, 100 pages × 20 visuals (2,105 files) | 275 ms | 71 ms |
| `report.validate` with model cross-checks, 100 pages | 250 ms | 162 ms |
| `project.new` end to end (CLI, 100 tables) | 188 ms | 46 ms |
| Edit loop ×100 (`measure.add` → `file.save` → `report.validate`), one `bicraft-cli run` | 4.7 s | 2.2 s |
| The same as 100 separate `bicraft-cli run` processes | 10.2 s | 3.8 s |
| MCP: 100 `validate_dax` calls over one stdio session, process included | 25 ms | 5.6 ms |
| XPress9 decompress (64 MB; after hot spot 5, was 102 / 180 MB/s; harness, best of 5) | 1,217 MB/s | 908 MB/s |
| XPress9 compress, 1 / 16 / 64 MB (after the G6 fix; was 4.7–20 / 3.2–6.7 / 3.9–5.5 and 20 / 6.7 / 4.4 MB/s) | 70 / 295 / 200 MB/s | 56 / 178 / 235 MB/s |
| PBIX `write_model` (+1 measure), 32 MB of storage (after the G6 fix; was 11.0 s and 8.0 s) | 0.88 s | 0.75 s |
| SQLite: write / scan, 200,000 rows | 0.41 M / 2.0 M rows/s | 0.95 M / 3.6 M rows/s |
| VertiPaq decode, 8 M rows: data ids / value-encoded / hash-encoded cells | 200 / 53 / 63 M cells/s | 705 / 46 / 47 M cells/s |
| Real sample (local only): `read_model` / `read_report` / `write_model` | 14 / 13 / 65 ms | 5 / 4 / 32 ms |

Windows and WSL compared:

- **Windows overhead.** On Windows, the end-to-end cases and the cases that touch many small
  files are 3–10× slower than on WSL. The causes are process start (about 25 ms against 2–3 ms),
  NTFS, and on-access scanning:

  | Case | Windows | WSL |
  |---|---:|---:|
  | `pbir::write_dir`, 100 pages | 525 ms | 78 ms |
  | `tmdl::write_folder`, 1,000 tables | 589 ms | 95 ms |

  Pure compute is closer: DAX, TMSL and XPress9 are 1.2–2× slower on Windows. VertiPaq value
  decoding is about even.
- **Freshly copied projects on Windows.** The MCP case on a freshly copied 10-page project took
  614 ms on Windows against 28 ms on WSL. Opening a project that was not just copied took 140 ms,
  and 50 `add_measure` calls added almost nothing. The first open of new files on Windows
  appears to pay for scanning.
- **Real sample on WSL.** The CLI cases on the real sample are slower on WSL than on Windows. The
  sample is read from `/mnt/c` through the 9p bridge.
- **Batching amortizes process start.** One `bicraft-cli run` batch (or one MCP session) is about
  2× faster than 100 separate processes on both systems. It is also the only way to amortize the
  25 ms process start on Windows.

## Hot spots (what to optimize first)

The three slowest areas were profiled on WSL with `perf record -e cpu-clock` on the release
binary, built with `CARGO_PROFILE_RELEASE_STRIP=false CARGO_PROFILE_RELEASE_DEBUG=line-tables-only`.
Samples were symbolized with `addr2line`, because this `perf` build cannot resolve Rust symbols
itself. There is no sampling profiler on the Windows side.

1. **`model.validate` grew quadratically with the number of measures** ([gaps](gaps.md) G5).
   Fixed on 2026-10-13.

   | Measures | Before (Windows / WSL) | After (Windows / WSL) |
   |---|---:|---:|
   | 100 | 13 ms / 11 ms | 11 ms / 9 ms |
   | 1,000 | 0.20 s / 0.16 s | 33 ms / 35 ms |
   | 10,000 | 12.7 s / 10.8 s | 0.22 s / 0.20 s |

   84% of the samples were in `SemanticModel::measure`, a linear case-insensitive scan of every
   table's measures, which the binder and the type checker called for every measure reference.
   The type checker also re-inferred referenced measures through their chains for every object,
   because its memo lived for one expression. `model.validate` now uses one
   `bicraft_dax::Validator` per pass: a name index built once (`ModelIndex`) and a cache of the
   exact types of measures and calculated columns. What remains at 1,000 measures is mostly the
   two cycles: the ~200 measures that depend on them are still inferred per expression, 16
   measures deep each. The CLI `validate-model` on 10,000 measures went from 11 s to 0.3 s.
2. **Saving a PBIX re-compressed the whole `DataModel` at 4–20 MB/s** (G6). Fixed on
   2026-10-14.
   - `write_model` after adding one measure to a PBIX with 32 MB of storage took 8–11 s; it now
     takes 0.75–0.9 s (10–12×). Most of what remains is decompression (the read, and the
     check that the new part decodes to the written backup), at 100–190 MB/s.
   - The profile: about 66% of the samples were in `Encoder::lookup`, the LZ77 match finder,
     and within it ~60% on the loads of the hash-chain walk. The cost per byte grew with the
     input because the match finder's tables (a 2^21-entry hash table, 4 M chain links and the
     4 MiB window, about 28 MB) stop fitting in the caches once the input passes about 1 MB;
     the search itself is bounded. The exact encoder cannot walk differently without changing
     its output, so it only got cheaper match extension (8 bytes at a time) and word-sized
     slice reads, which are within the noise on this loaded machine.
   - The fix is `bicraft_xpress9::CompressionMode` (`docs/formats/xpress9.md` §6). The
     default, `Fast`, compresses each 2 MiB chunk with its own encoder, primed with the
     256 KiB that precede the chunk, on up to 8 threads, with a lookup depth of 1 (upstream's
     2^12-entry hash table); the blocks are renumbered into one session. `Balanced` is the same
     with Desktop's match search; `Exact` is the old single-encoder path (Desktop's bytes).
     Output is identical for every thread count and on Windows and Linux. Desktop 2.158 opens
     files written in every mode.

   Measured with a release harness that calls `compress_with_plan`, the old build and each
   mode run alternately on the same input (the bench's synthetic ABF-like bytes; best of 2–3
   runs; MB/s of input; the machine was loaded by parallel builds, so single runs vary 2×):

   | Input | Before (Windows / WSL) | `Exact` | `Balanced` | `Fast` (default) |
   |---|---:|---:|---:|---:|
   | 1 MB (one chunk) | 22 / 20 | 28 / 25 | 20 / 24 | 51 / 58 |
   | 16 MB | 7.4 / 9.5 | 11 / 5.6 | 44 / 34 | 288 / 146 |
   | 32 MB | 3.9 / 5.8 | 2.9 / 5.8 | 35 / 26 | 293 / 249 |
   | 64 MB | 2.8 / 4.7 | 2.9 / 5.4 | 33 / 23 | 222 / 217 |
   | 192 MB (round trip checked) | | | 36 / 49 | 345 / 281 |

   | Output size | `Exact` | `Balanced` | `Fast` |
   |---|---:|---:|---:|
   | 32 MB synthetic | 16,536 KB | 16,519 KB | 16,948 KB (+2.5%) |
   | Real sample ABF (local only, 0.94 MB) | 127 KB | 127 KB | 132 KB (+4%) |

3. **PBIR schema validation re-parsed the vendored schemas on every call** (G11). Fixed on
   2026-10-14.
   - In the edit-loop profile (300 commands in one `bicraft-cli run`), the top frames were
     `serde_json` map building and SipHash under `pbir::schema` and `pbir::validate`, then the
     PBIR and TMDL writers.
   - `validate_documents` built a new `Store` and parsed the schemas again for each call. The
     bigger cost was per document: every schema node visited looked its keywords up by name in
     the raw `serde_json` map, and every property visited built a JSON-pointer string.
   - Now each vendored schema is compiled once per process into a node arena (`$ref`s resolved to
     ids, properties sorted, patterns compiled, messages and pointers built only for issues) and
     cached per root file; parsed files are shared between roots (`docs/formats/pbir.md`
     §Validation).

   Measured with a release harness that calls `bicraft_pbir::validate_documents` directly, the
   old and new builds run alternately on the same machine (Windows, under load from parallel
   builds; median of 100 runs, 3 rounds, range of the medians):

   | Input | Before | After |
   |---|---:|---:|
   | Real sample (local only, 137 documents) | 22–38 ms | 10–13 ms |
   | Committed fixture, 2 pages (17 documents) | 2.6–5.3 ms | 0.6–0.7 ms |
   | Fixture page cloned, 11 pages (89 documents) | 13–17 ms | 2.8–3.3 ms |
   | Fixture page cloned, 101 pages (809 documents) | 108–120 ms | 37–40 ms |
   | Bench report (`cargo xtask bench --keep`), 1 page × 20 visuals (24 documents) | 3.0–3.2 ms | 0.8–1.0 ms |
   | Bench report, 10 pages (213 documents) | 13–19 ms | 6.7–7.5 ms |
   | Bench report, 100 pages (2,103 documents) | 131–157 ms | 80–88 ms |
   | First call in a process (compiles the schemas), 2 pages | 3.4–7.1 ms | 4.4–6.5 ms |

   The first call costs about 1–2 ms more than before (compiling is dearer than parsing); every
   later call in the process is 1.6–8× faster. The bench's visuals carry more query and
   formatting JSON per file than the fixture, so the per-file share (JSON parsing, `anyOf` fan-out
   over the `semanticQuery` expression kinds) is larger there. The full-results tables below are
   from before the fix; a rerun of `cargo xtask bench --filter Reports` on the loaded machine gave
   `pbir::validate_documents` 2.2 / 14.1 / 133 ms for 1 / 10 / 100 pages.

4. **Windows process start and small-file I/O** ([gaps](gaps.md) G23). Worked on 2026-10-14.

   - **Process start is the operating system's floor, not ours.** `bicraft-cli --version` (18 MB
     release binary) starts as fast as a 160 KB Rust hello-world on the same machine: medians of
     19–30 ms for both over three runs of 200 starts, alternating, with or without the 64 MiB
     worker thread (`whoami.exe`: 27 ms). `bicraft-cli` has no static initializers; the schemas and
     catalogs are already built lazily on first use. On WSL both start in 1–3 ms. The only remedy
     is not starting processes: one `bicraft-cli run` batch or an MCP session (see above).
   - **Small files: overlap the opens.** On Windows each file open and folder listing costs far
     more than the bytes (on-access scanning, NTFS), and a PBIR report keeps every visual in a
     folder of its own: the 100-page bench report is 2,105 files in 2,200 folders. Measured on
     that layout, listing the folders took ~100–140 ms and reading the files ~120 ms, one thread;
     with 8–16 threads, 20–40 ms and 20–25 ms. Changes:
     - `crates/pbir/src/fs.rs`: the report is read one folder level at a time, the folders of a
       level by up to 16 threads (each lists its folders and reads their files); documents are
       compared and written by up to 16 threads.
     - `crates/tmdl`: files are read with one open each (sizes come from the folder listing,
       not a second `metadata` open) on up to 16 threads, and parsed on several threads; files
       are compared and written on up to 16 threads; stale files are found by name, and
       `canonicalize` (an open per call) is only used for names that differ in case alone,
       instead of twice per file.
     - Fewer than 16 items run on the calling thread; a thread that cannot be started runs its
       share on the calling thread.

   Before / after, same machine and session, release builds of the tree before and after the
   change (median of 7 in-process runs, or 15 CLI runs; 100-page bench report, 1,000-table
   model):

   | Case | Windows before | Windows after | WSL before | WSL after |
   |---|---:|---:|---:|---:|
   | `bicraft-cli --version` | 24–30 ms | 20–30 ms (= hello-world) | 7.4 ms* | 7.5 ms* |
   | CLI `validate-report`, 100 pages | 608–775 ms | 377–534 ms | 378–400 ms | 317–319 ms |
   | `pbir::read_dir`, 100 pages | 619–624 ms | 152–161 ms | 74–89 ms | 50–53 ms |
   | `pbir::write_dir`, 100 pages, new folder | 2.74–2.86 s | 1.31–1.47 s | 87–91 ms | 39 ms |
   | `tmdl::write_folder`, 1,000 tables, new folder | 2.02–2.47 s | 1.35–1.88 s | 94–101 ms | 102–110 ms |
   | `tmdl::write_folder`, 1,000 tables, unchanged | 486–513 ms | 156–168 ms | 76 ms | 59–60 ms |
   | `tmdl::read_folder`, 1,000 tables | 404–424 ms | 239–296 ms | 107–136 ms | 99–110 ms |

   \* timed from a bash loop that also starts `date`; the bench's 2.3 ms is the cleaner figure.

   What remains of `validate-report` is compute (JSON parsing, schema validation, model checks),
   about the same on both systems. Creating many new files stays slow on Windows (new files are
   scanned on close): writing a 2,105-file report into an empty folder still takes over a second.
   The machine was shared with other builds during these runs; the full-results tables below are
   from before the change.

5. **Reads and saves were bound by XPress9 decompression at 100–190 MB/s.** Worked on
   2026-10-14. After G6, a PBIX save spent most of its time decoding: the read, then the check
   that the new part decodes to the written backup.
   - The profile (WSL `perf`, single-threaded decode) put the time in the per-symbol work of
     the ported streaming decoder: a bit-reader load per peek, closure-wrapped table walks, a
     `Vec::push` per literal, and a copy of every decoded slice out of the shifting history
     buffer.
   - Changes (`docs/formats/xpress9.md` §7):
     - **A fast decoder** decodes whole blocks in place. It uses a 64-bit bit buffer, a
       4,096-entry table that yields one or two literals per lookup, and 16-/8-byte match
       copies. Bounds checks go through safe slice access.
     - **Parallel decode on up to 8 threads.** Blocks depend on each other only through the
       history. A block whose history is not decoded yet records the matches that need it, and
       the calling thread replays them in order afterwards.
     - **A parallel `verify_datamodel`** replaces decompress-and-compare on the save path. Each
       block is decoded against the expected backup as its history and compared as it comes,
       without building the output.
     - **The ported decoder stays as the reference.** On any failure the decoder falls back to
       it, so bytes and errors are unchanged (asserted by the fuzz smoke tests).
     - **An unchanged backup keeps the original part.** `write_data_model` returns the original
       part when the re-packed backup is byte-identical, with no compression and no check.
   - Output was byte-identical to the old decoder on every local sample:
     - all PBIX `DataModel` parts and `cache.abf` files;
     - a 113 MB single-threaded part;
     - multithreaded parts of 578 MB and 845 MB (10 sessions each).

   Measured with a release harness that links the old crate and the new one and runs them
   alternately on the same input (best of 5, median in parentheses; MB/s of decoded output; the
   machine was loaded by other agents' builds):

   | Input | Old (Windows / WSL) | New, 1 thread | New, up to 8 threads |
   |---|---:|---:|---:|
   | Synthetic 64 MB, `Fast` part (32 chunks) | 157 (133) / 183 (164) | 383 (256) / 325 (293) | 1217 (952) / 908 (798) |
   | Synthetic 16 MB, `Exact` part (Desktop's encoding, 8 chunks) | 107 (106) / 180 (172) | 190 (187) / 285 (273) | 456 (412) / 865 (657) |
   | Real sample (local only, 0.97 MB, one block) | 648 (573) / 640 (509) | 904 (865) / 760 (641) | same path (one block) |
   | Multithreaded 578 MB part (local only, 10 sessions, session by session) | 171 / 182 | | 856 / 592 |

   | Save-path step | Old (Windows / WSL) | New |
   |---|---:|---:|
   | Verify a 64 MB part (decompress + compare → `verify_datamodel`) | 434 ms / 428 ms | 30 ms / 31 ms |
   | Compress (`Fast`) + verify, 32 MB | 258 ms / 305 ms | 109 ms / 136 ms |

   A one-block part cannot be split, so a small real sample gains only from the single-thread
   loop (1.2–1.4×). Desktop's `Exact` streams reach back up to 4 MiB, so about 40% of the <!-- perf-guard: ok (BIcraft before and after) -->
   matches in a 2 MiB block are replayed sequentially. That is why the 16 MB `Exact` case scales
   less than the `Fast` one.

Next in line are smaller items:

- `tmdl::read_documents` at 10–30 MB/s, about 3× slower than the TMSL reader on the same model.
- VertiPaq value conversion at about 50 M cells/s, against 200–700 M data ids/s. The time goes
  into building a `Vec<CellValue>` of 8 M enums.

## Full results

Full runs on 2026-10-13: Windows 394 s, WSL 309 s, 95 cases each, no failures. Throughput is
items/s and/or MB/s of input. A notes column records sanity facts such as the invalid measures
(the 6 cycle members) and the issues found. The `model.validate` and `validate-model` rows were
re-measured after the G5 fix with `cargo xtask bench --filter validate`; the full-run numbers
before the fix are in the hot spots above.

### DAX validation

| Case | Windows median (p95) | WSL median (p95) | Throughput (Windows / WSL) | Notes |
|---|---:|---:|---|---|
| dax.validate, 1 expression, no model | 7.0 µs (10.0 µs) | 2.9 µs (4.4 µs) | 142.9 kexpr/s / 346.6 kexpr/s | valid=true |
| dax.validate, 1 expression, model 100 tables / 1,000 measures | 23.4 µs (48.0 µs) | 10.9 µs (31.5 µs) | 42.7 kexpr/s / 92.2 kexpr/s | valid=true |
| dax.validate, 10,000 expressions, same model | 1.16 s (2.52 s) | 624.0 ms (722.4 ms) | 8.6 kexpr/s, 0.7 MB/s / 16.0 kexpr/s, 1.3 MB/s | invalid=625 |
| model.validate, 100 measures (chains of 100, 2 cycles) | 11.4 ms (16.4 ms) | 9.4 ms (12.3 ms) | 8.8 kmeasure/s / 10.6 kmeasure/s | invalid=6, cycles=2 |
| model.validate, 1000 measures (chains of 100, 2 cycles) | 32.6 ms (49.0 ms) | 35.2 ms (42.6 ms) | 30.7 kmeasure/s / 28.4 kmeasure/s | invalid=6, cycles=2 |
| model.validate, 10000 measures (chains of 100, 2 cycles) | 216.0 ms (275.2 ms) | 196.6 ms (230.9 ms) | 46.3 kmeasure/s / 50.9 kmeasure/s | invalid=6, cycles=2 |

### Model files (TMSL / TMDL), tables × 20 columns × 10 measures

| Case | Windows median (p95) | WSL median (p95) | Throughput (Windows / WSL) | Notes |
|---|---:|---:|---|---|
| tmsl::read, 10 tables | 966.7 µs (1.4 ms) | 556.4 µs (736.2 µs) | 10.3 ktable/s, 57 MB/s / 18.0 ktable/s, 99 MB/s | 53 KB TMSL |
| tmsl::write, 10 tables | 712.4 µs (818.5 µs) | 484.8 µs (1.8 ms) | 14.0 ktable/s, 122 MB/s / 20.6 ktable/s, 179 MB/s |  |
| tmdl::write_documents, 10 tables | 1.3 ms (1.3 ms) | 737.2 µs (1.3 ms) | 10.3 kfile/s, 42 MB/s / 17.6 kfile/s, 73 MB/s |  |
| tmdl::read_documents, 10 tables | 2.4 ms (3.0 ms) | 1.4 ms (1.6 ms) | 41.8 kmeasure/s, 22 MB/s / 74.0 kmeasure/s, 40 MB/s |  |
| tmdl::write_folder (disk), 10 tables | 5.0 ms (6.5 ms) | 1.0 ms (1.3 ms) | 2.6 kfile/s, 11 MB/s / 13.0 kfile/s, 54 MB/s |  |
| tmdl::read_folder (disk), 10 tables | 4.3 ms (4.9 ms) | 1.7 ms (2.6 ms) | 2.3 ktable/s, 13 MB/s / 5.7 ktable/s, 31 MB/s |  |
| tmsl::read, 100 tables | 14.9 ms (16.7 ms) | 6.0 ms (8.0 ms) | 6.7 ktable/s, 37 MB/s / 16.7 ktable/s, 93 MB/s | 540 KB TMSL |
| tmsl::write, 100 tables | 10.9 ms (13.0 ms) | 3.4 ms (4.4 ms) | 9.2 ktable/s, 80 MB/s / 29.0 ktable/s, 253 MB/s |  |
| tmdl::write_documents, 100 tables | 12.1 ms (14.1 ms) | 6.1 ms (6.9 ms) | 8.5 kfile/s, 45 MB/s / 16.9 kfile/s, 89 MB/s |  |
| tmdl::read_documents, 100 tables | 30.8 ms (45.6 ms) | 14.7 ms (17.3 ms) | 32.4 kmeasure/s, 18 MB/s / 67.9 kmeasure/s, 37 MB/s |  |
| tmdl::write_folder (disk), 100 tables | 41.0 ms (47.5 ms) | 10.1 ms (11.6 ms) | 2.5 kfile/s, 13 MB/s / 10.2 kfile/s, 54 MB/s |  |
| tmdl::read_folder (disk), 100 tables | 40.1 ms (51.0 ms) | 16.9 ms (25.3 ms) | 2.5 ktable/s, 13 MB/s / 5.9 ktable/s, 32 MB/s |  |
| tmsl::read, 1000 tables | 143.9 ms (169.3 ms) | 76.4 ms (87.6 ms) | 6.9 ktable/s, 39 MB/s / 13.1 ktable/s, 73 MB/s | 5450 KB TMSL |
| tmsl::write, 1000 tables | 88.8 ms (96.2 ms) | 61.0 ms (69.9 ms) | 11.3 ktable/s, 99 MB/s / 16.4 ktable/s, 143 MB/s |  |
| tmdl::write_documents, 1000 tables | 101.0 ms (108.5 ms) | 66.7 ms (71.4 ms) | 9.9 kfile/s, 54 MB/s / 15.0 kfile/s, 82 MB/s |  |
| tmdl::read_documents, 1000 tables | 516.9 ms (639.2 ms) | 171.2 ms (189.0 ms) | 19.3 kmeasure/s, 11 MB/s / 58.4 kmeasure/s, 32 MB/s |  |
| tmdl::write_folder (disk), 1000 tables | 588.5 ms (3.63 s) | 94.6 ms (104.1 ms) | 1.7 kfile/s, 9.3 MB/s / 10.6 kfile/s, 58 MB/s |  |
| tmdl::read_folder (disk), 1000 tables | 442.4 ms (491.3 ms) | 172.6 ms (176.9 ms) | 2.3 ktable/s, 12 MB/s / 5.8 ktable/s, 32 MB/s |  |

### Reports (PBIR), pages × 20 visuals, model 10 tables / 100 measures

| Case | Windows median (p95) | WSL median (p95) | Throughput (Windows / WSL) | Notes |
|---|---:|---:|---|---|
| build via page.add/visual.add commands, 1 pages | 2.0 ms (2.8 ms) | 747.8 µs (1.2 ms) | 12.1 kcmd/s / 32.1 kcmd/s |  |
| pbir::read_dir, 1 pages | 4.7 ms (6.7 ms) | 579.3 µs (736.3 µs) | 4.3 kvisual/s, 5.6 MB/s / 34.5 kvisual/s, 45 MB/s | 26 files, 25 KB |
| pbir::write_documents, 1 pages | 310.2 µs (417.6 µs) | 135.1 µs (161.9 µs) | 77.4 kfile/s, 84 MB/s / 177.6 kfile/s, 194 MB/s |  |
| pbir::write_dir (disk), 1 pages | 9.0 ms (11.6 ms) | 684.5 µs (798.2 µs) | 2.2 kvisual/s, 2.9 MB/s / 29.2 kvisual/s, 38 MB/s |  |
| pbir::validate_documents (schemas), 1 pages | 4.9 ms (5.9 ms) | 2.0 ms (2.7 ms) | 5.3 kfile/s, 5.3 MB/s / 12.8 kfile/s, 13 MB/s | issues=0 |
| report.validate (schemas + model fields), 1 pages | 7.2 ms (8.7 ms) | 3.1 ms (3.9 ms) | 2.8 kvisual/s / 6.4 kvisual/s | errors=0 |
| file.open (PBIP: TMDL + PBIR), 1 pages | 10.5 ms (12.9 ms) | 2.2 ms (2.9 ms) | 1.9 kvisual/s / 8.9 kvisual/s |  |
| file.save after a visual move (PBIR rewrite), 1 pages | 16.6 ms (21.2 ms) | 2.5 ms (3.2 ms) | 1.2 kvisual/s / 8.0 kvisual/s |  |
| CLI validate-report, 1 pages | 43.2 ms (48.7 ms) | 12.4 ms (13.8 ms) | 463 visual/s / 1.6 kvisual/s |  |
| build via page.add/visual.add commands, 10 pages | 2.9 ms (3.7 ms) | 2.7 ms (3.6 ms) | 74.3 kcmd/s / 79.0 kcmd/s |  |
| pbir::read_dir, 10 pages | 29.6 ms (37.7 ms) | 7.5 ms (14.3 ms) | 6.7 kvisual/s, 8.4 MB/s / 26.6 kvisual/s, 33 MB/s | 215 files, 244 KB |
| pbir::write_documents, 10 pages | 2.3 ms (3.8 ms) | 1.9 ms (2.1 ms) | 93.5 kfile/s, 110 MB/s / 113.3 kfile/s, 133 MB/s |  |
| pbir::write_dir (disk), 10 pages | 60.3 ms (75.9 ms) | 6.9 ms (8.2 ms) | 3.3 kvisual/s, 4.2 MB/s / 29.1 kvisual/s, 36 MB/s |  |
| pbir::validate_documents (schemas), 10 pages | 18.1 ms (22.3 ms) | 10.2 ms (13.4 ms) | 11.9 kfile/s, 14 MB/s / 21.0 kfile/s, 24 MB/s | issues=0 |
| report.validate (schemas + model fields), 10 pages | 25.9 ms (30.4 ms) | 14.6 ms (17.7 ms) | 7.7 kvisual/s / 13.7 kvisual/s | errors=0 |
| file.open (PBIP: TMDL + PBIR), 10 pages | 38.6 ms (42.3 ms) | 9.2 ms (11.4 ms) | 5.2 kvisual/s / 21.6 kvisual/s |  |
| file.save after a visual move (PBIR rewrite), 10 pages | 73.1 ms (94.9 ms) | 10.5 ms (15.4 ms) | 2.7 kvisual/s / 19.1 kvisual/s |  |
| CLI validate-report, 10 pages | 92.9 ms (104.1 ms) | 34.1 ms (41.3 ms) | 2.2 kvisual/s / 5.9 kvisual/s |  |
| build via page.add/visual.add commands, 100 pages | 25.3 ms (36.1 ms) | 20.2 ms (20.7 ms) | 83.3 kcmd/s / 104.0 kcmd/s |  |
| pbir::read_dir, 100 pages | 274.7 ms (291.7 ms) | 71.3 ms (75.1 ms) | 7.3 kvisual/s, 9.1 MB/s / 28.1 kvisual/s, 35 MB/s | 2105 files, 2432 KB |
| pbir::write_documents, 100 pages | 25.6 ms (28.9 ms) | 19.3 ms (21.6 ms) | 82.1 kfile/s, 97 MB/s / 109.1 kfile/s, 129 MB/s |  |
| pbir::write_dir (disk), 100 pages | 524.8 ms (2.24 s) | 78.1 ms (90.1 ms) | 3.8 kvisual/s, 4.7 MB/s / 25.6 kvisual/s, 32 MB/s |  |
| pbir::validate_documents (schemas), 100 pages | 159.7 ms (168.6 ms) | 88.3 ms (91.3 ms) | 13.2 kfile/s, 16 MB/s / 23.8 kfile/s, 28 MB/s | issues=0 |
| report.validate (schemas + model fields), 100 pages | 250.4 ms (272.6 ms) | 162.1 ms (174.3 ms) | 8.0 kvisual/s / 12.3 kvisual/s | errors=0 |
| file.open (PBIP: TMDL + PBIR), 100 pages | 346.7 ms (399.6 ms) | 87.7 ms (95.5 ms) | 5.8 kvisual/s / 22.8 kvisual/s |  |
| file.save after a visual move (PBIR rewrite), 100 pages | 706.6 ms (977.3 ms) | 139.9 ms (157.5 ms) | 2.8 kvisual/s / 14.3 kvisual/s |  |
| CLI validate-report, 100 pages | 685.6 ms (826.3 ms) | 292.1 ms (309.1 ms) | 2.9 kvisual/s / 6.8 kvisual/s |  |

### Process start and projects (end to end)

| Case | Windows median (p95) | WSL median (p95) | Throughput (Windows / WSL) | Notes |
|---|---:|---:|---|---|
| CLI bicraft-cli --version (process start) | 28.6 ms (36.5 ms) | 2.3 ms (3.1 ms) | 35 process/s / 435 process/s |  |
| CLI validate-dax, 1 expression, no model | 27.5 ms (43.1 ms) | 2.7 ms (3.4 ms) | 36 process/s / 374 process/s |  |
| CLI validate-model, 1000 measures | 60.4 ms (79.8 ms) | 45.8 ms (59.2 ms) | 16.5 kmeasure/s / 21.8 kmeasure/s |  |
| CLI validate-model, 10000 measures | 320.7 ms (356.5 ms) | 262.0 ms (418.2 ms) | 31.2 kmeasure/s / 38.2 kmeasure/s |  |
| project.new from model.bim, 100 tables (in-process) | 253.0 ms (275.0 ms) | 27.5 ms (39.2 ms) | 395 table/s / 3.6 ktable/s |  |
| CLI new-project --open model.bim, 100 tables | 187.9 ms (207.5 ms) | 46.4 ms (52.0 ms) | 532 table/s / 2.2 ktable/s |  |
| edit loop ×100 (measure.add → file.save → report.validate), in-process | 5.75 s (10.51 s) | 2.01 s (2.09 s) | 17 iteration/s / 50 iteration/s |  |
| edit loop ×100, one `bicraft-cli run` batch (300 commands) | 4.71 s (4.95 s) | 2.21 s (2.25 s) | 21 iteration/s / 45 iteration/s |  |
| edit loop ×100, 100 separate `bicraft-cli run` processes | 10.15 s (15.43 s) | 3.83 s (3.91 s) | 10 iteration/s / 26 iteration/s |  |

### MCP (stdio, one session)

| Case | Windows median (p95) | WSL median (p95) | Throughput (Windows / WSL) | Notes |
|---|---:|---:|---|---|
| 100 validate_dax calls, no model (process start included) | 24.9 ms (28.1 ms) | 5.6 ms (6.5 ms) | 4.0 kcall/s / 18.0 kcall/s |  |
| 100 tool calls on the 10-page project (open_file, 50 add_measure, 49 validate_dax) | 614.4 ms (3.72 s) | 27.7 ms (30.6 ms) | 163 call/s / 3.6 kcall/s |  |

### XPress9 (DataModel framing, single-threaded)

Re-measured on 2026-10-14 after the G6 fix (`cargo xtask bench --filter XPress9`; default
`CompressionMode::Fast`). The full-run numbers before the fix are in the hot spots above.
The decompress rows predate the fast decoder (hot spot 5), which makes them 4–6× faster on
multi-chunk inputs.

| Case | Windows median (p95) | WSL median (p95) | Throughput (Windows / WSL) | Notes |
|---|---:|---:|---|---|
| compress 1 MB | 15.0 ms (22.3 ms) | 18.8 ms (37.9 ms) | 70 MB/s / 56 MB/s | 528 KB out |
| decompress 1 MB | 5.6 ms (6.1 ms) | 6.1 ms (28.0 ms) | 188 MB/s / 171 MB/s | ratio 1.94 |
| compress 16 MB | 56.9 ms (110.7 ms) | 94.2 ms (178.0 ms) | 295 MB/s / 178 MB/s | 8464 KB out |
| decompress 16 MB | 145.1 ms (191.2 ms) | 90.1 ms (150.6 ms) | 116 MB/s / 186 MB/s | ratio 1.94 |
| compress 64 MB | 335.5 ms (356.1 ms) | 285.2 ms (338.0 ms) | 200 MB/s / 235 MB/s | 33873 KB out |
| decompress 64 MB | 631.2 ms (658.0 ms) | 470.0 ms (476.6 ms) | 106 MB/s / 143 MB/s | ratio 1.93 |

### PBIX (synthetic)

| Case | Windows median (p95) | WSL median (p95) | Throughput (Windows / WSL) | Notes |
|---|---:|---:|---|---|
| Package::read_bytes (ZIP), small (fixture model) | 38.9 µs (58.0 µs) | 15.8 µs (19.7 µs) | 128.5 kpart/s, 105 MB/s / 316.9 kpart/s, 259 MB/s |  |
| read_model (XPress9 + ABF + SQLite), small (fixture model) | 576.6 µs (668.8 µs) | 314.1 µs (530.7 µs) | 5.8 MB/s / 11 MB/s | 2 tables, 2 measures |
| read_report, small (fixture model) | 6.9 µs (12.3 µs) | 4.0 µs (7.0 µs) |  /  | 1 pages, 0 visuals |
| write_model (+1 measure: SQLite + ABF + XPress9), small (fixture model) | 6.0 ms (6.3 ms) | 3.9 ms (4.9 ms) | 0.6 MB/s / 0.9 MB/s | 3 KB DataModel |
| Package::to_bytes (ZIP), small (fixture model) | 60.2 µs (71.6 µs) | 30.0 µs (38.4 µs) | 68 MB/s / 136 MB/s |  |
| CLI inspect, small (fixture model) | 25.9 ms (33.6 ms) | 3.4 ms (4.2 ms) | 0.2 MB/s / 1.2 MB/s |  |
| CLI run --open … measure.add --save (in place), small (fixture model) | 42.0 ms (44.7 ms) | 13.4 ms (15.5 ms) | 0.1 MB/s / 0.3 MB/s |  |
| Package::read_bytes (ZIP), 1,000 measures + 32 MB storage | 15.8 ms (16.1 ms) | 26.3 ms (28.3 ms) | 317 part/s, 1079 MB/s / 190 part/s, 646 MB/s |  |
| read_model (XPress9 + ABF + SQLite), 1,000 measures + 32 MB storage | 314.1 ms (320.2 ms) | 198.6 ms (250.1 ms) | 54 MB/s / 86 MB/s | 2 tables, 1002 measures |
| read_report, 1,000 measures + 32 MB storage | 10.6 µs (14.1 µs) | 4.3 µs (34.1 µs) |  /  | 1 pages, 0 visuals |
| write_model (+1 measure: SQLite + ABF + XPress9), 1,000 measures + 32 MB storage | 881.0 ms (894.2 ms) | 750.4 ms (906.2 ms) | 19.8 MB/s / 23.2 MB/s | 17016 KB DataModel; re-measured 2026-10-14 after the G6 fix (was 11.02 s / 8.01 s) |
| Package::to_bytes (ZIP), 1,000 measures + 32 MB storage | 7.4 ms (9.0 ms) | 12.9 ms (16.1 ms) | 2299 MB/s / 1319 MB/s |  |
| CLI inspect, 1,000 measures + 32 MB storage | 240.3 ms (261.9 ms) | 251.2 ms (268.5 ms) | 71 MB/s / 68 MB/s |  |
| CLI run --open … measure.add --save (in place), 1,000 measures + 32 MB storage | 6.76 s (11.35 s) | 5.94 s (14.70 s) | 2.5 MB/s / 2.9 MB/s |  |

### SQLite (pure Rust)

| Case | Windows median (p95) | WSL median (p95) | Throughput (Windows / WSL) | Notes |
|---|---:|---:|---|---|
| DatabaseBuilder: 200000 rows × 5 columns | 484.3 ms (644.6 ms) | 209.6 ms (222.3 ms) | 413.0 krow/s, 29 MB/s / 954.2 krow/s, 67 MB/s |  |
| Database::open + scan 200000 rows | 101.0 ms (111.8 ms) | 56.2 ms (70.6 ms) | 2.0 Mrow/s, 139 MB/s / 3.6 Mrow/s, 249 MB/s |  |

### VertiPaq column decode (synthetic hybrid RLE + bit-packed)

| Case | Windows median (p95) | WSL median (p95) | Throughput (Windows / WSL) | Notes |
|---|---:|---:|---|---|
| data ids only, 8 M rows | 41.8 ms (65.3 ms) | 11.9 ms (14.5 ms) | 200.5 Mcell/s, 145 MB/s / 704.7 Mcell/s, 511 MB/s | 5 MB .idf |
| value-encoded column, 8 M rows | 157.7 ms (237.6 ms) | 182.1 ms (198.1 ms) | 53.2 Mcell/s, 39 MB/s / 46.1 Mcell/s, 33 MB/s |  |
| hash-encoded column (1,000-value dictionary), 8 M rows | 133.0 ms (143.8 ms) | 177.5 ms (220.8 ms) | 63.1 Mcell/s, 46 MB/s / 47.3 Mcell/s, 34 MB/s |  |

### Real file (local only; timings and counts only)

| Case | Windows median (p95) | WSL median (p95) | Throughput (Windows / WSL) | Notes |
|---|---:|---:|---|---|
| Package::read_bytes (ZIP), real sample | 2.7 ms (2.9 ms) | 2.5 ms (5.0 ms) | 53.0 kpart/s, 103 MB/s / 57.5 kpart/s, 112 MB/s |  |
| read_model (XPress9 + ABF + SQLite), real sample | 13.8 ms (15.8 ms) | 5.3 ms (6.1 ms) | 9.4 MB/s / 25 MB/s | 8 tables, 25 measures |
| read_report, real sample | 13.3 ms (16.4 ms) | 4.2 ms (4.4 ms) | 8.7 kvisual/s / 27.1 kvisual/s | 10 pages, 115 visuals |
| write_model (+1 measure: SQLite + ABF + XPress9), real sample | 65.3 ms (78.2 ms) | 32.1 ms (34.6 ms) | 2.0 MB/s / 4.1 MB/s | 127 KB DataModel |
| Package::to_bytes (ZIP), real sample | 9.4 ms (20.2 ms) | 6.8 ms (7.2 ms) | 29 MB/s / 41 MB/s |  |
| CLI inspect, real sample | 76.8 ms (79.6 ms) | 189.8 ms (196.4 ms) | 3.6 MB/s / 1.5 MB/s |  |
| CLI validate-model, real sample | 86.3 ms (98.6 ms) | 184.3 ms (197.4 ms) | 3.2 MB/s / 1.5 MB/s |  |
| CLI validate-report, real sample | 101.5 ms (119.6 ms) | 225.4 ms (256.8 ms) | 2.8 MB/s / 1.2 MB/s |  |

## Concurrency on one workspace (2026-10-28)

`cargo xtask concurrency-bench` measures `bicraft-cli serve` with several calls on one workspace
(`docs/web-deployment.md`, "Concurrency on one workspace"). It reports BIcraft figures only.

```text
cargo xtask concurrency-bench --bin target/release/bicraft-cli --data <bench-data>/r10m/bench.pbix
                              [--base-bin <main build>] [--parallel 4] [--rounds 2]
                              [--queries Q01,Q05,...] [--long QID|DAX] [--json OUT]
```

Each query of `xtask/engine-bench/queries.json` is measured against one fresh server per mode.
There are two modes: reads one at a time (`--max-concurrent-reads 1`) and reads side by side
(`--max-concurrent-reads N`). For each mode the bench measures:

- **Batches.** `N` identical `evaluate_dax_query` calls are sent at once, cold (`dax.clear_cache`
  before the batch) and warm.
- **Same answers.** Every answer of both modes is compared byte for byte with the first
  serialized one; a difference fails the run.
- **A short read behind a long one** (`--long`). It is sent 20 ms after the long one starts.
- **One query alone,** this build and `--base-bin`.
- **Peak memory** (working set, VmHWM on Linux). The fact's columns are not decoded yet: one cold
  query is compared with `N` cold identical queries at once.

All calls go over loopback HTTP and include their HTTP cost.

**Machine and data.** The machine of this page; 10 M fact rows (`bench-data/r10m`); `N` = 4; 3
rounds, medians. Timed under `bench-quiet`: no build and no other heavy process running (CPU load
1 % at the end). Final build: forks read the aggregations' snapshot and wait up to 50 ms for the
main state.

| What | Reads one at a time | Reads side by side |
|---|---:|---:|
| Short read (Q01) 20 ms after a 1.6 s query | 1,590 ms | **64 ms** |
| Short read (Q05) behind it | 1,625 ms | **87 ms** |
| Short read (Q07) behind it | 1,579 ms | **94 ms** |
| Short read (Q13) behind it | 1,588 ms | **75 ms** |
| The 1.6 s query itself | 1.60–1.64 s | 1.64–1.69 s |
| 4 × Q01 at once, cold / warm | 18.0 / 15.5 ms | 16.4 / 15.5 ms |
| 4 × Q05 at once, cold / warm | 43.5 / 33.6 ms | 37.4 / **25.5 ms** |
| 4 × Q07 at once, cold / warm | 33.3 / 14.1 ms | 39.0 / **11.6 ms** |
| 4 × Q13 at once, cold / warm | 36.6 / 23.9 ms | 30.5 / 24.7 ms |

- **What changes for users.** A short read no longer waits for a long one: ×17 to ×25 here. It
  answers in its own time plus at most 50 ms. Before forking, an evaluation waits that long for
  the main state, whose result caches answer a warm query in a few ms. The long query is
  unchanged.
- **Identical short queries at once.** These queries take 4 to 30 ms each, and one call has a
  fixed cost of about 8 ms (below). Warm batches side by side against one at a time:
  - Q05 ×1.32 and Q07 ×1.22, both answered from the report-derived aggregations, which the
    evaluations running beside the main state read too;
  - Q01 ×1.00, Q13 ×0.97 (within noise).

  Cold batches: ×0.85 to ×1.20. Q07, the ×0.85 case, ran 3 more times quiet: cold ×0.96, ×1.10 and
  ×0.91 (32.2/33.5, 32.6/29.5 and 32.2/35.2 ms one at a time / side by side), so noise; warm
  ×1.21, ×2.71 and ×1.31.
- **First run of this bench.** Before forks read the aggregations and waited 50 ms, warm Q07 and
  Q05 batches were slower side by side (×0.45 and ×0.79), and short reads behind the long one
  took 9–33 ms.
- **Answers.** Identical in both modes, for every query, every round.
- **Peak memory** (Q05, 4 cold at once against 1): 259 MiB against 254 MiB. Concurrent
  evaluations that miss the same column decode it once ("one read per column"). Before that, on
  1 M rows, four Q01 at once peaked at 80 MiB against 48 MiB for one.

**The per-call floor.** Every `tools/call` over HTTP took about 60–75 ms, even `list_commands`
on an empty workspace.

- **Cause.** The call joined its connection watcher, which polls the socket with a 50 ms read
  timeout. Discovery and `tools/list` have no watcher and took 6–12 ms.
- **Fix.** The watcher now runs on a thread of its own: the call marks it finished and goes on,
  and the watcher stops at its next poll. A mutex orders SSE keepalives before the answer. Nothing
  busy-waits.
- **Measured** with curl on loopback, median of 21 calls (this build against `main`):

| Call | `main` | This build |
|---|---:|---:|
| `tools/call list_commands`, empty workspace | 67.3–74.9 ms | **7.3–15.1 ms** |
| `tools/list` (no watcher, reference) | 7.2–12.0 ms | 5.8–10.2 ms |
| `evaluate_dax_query` alone on 10 M, warm, Q01 / Q05 / Q07 / Q13 | 58–67 / 76–80 / 66–67 / 73–74 ms | **15 / 15 / 15 / 15 ms** |

## CSV ingestion (2026-11-04)

`cargo xtask csv-bench --csv FILE [--reader m|csv] [--runs N]` (`xtask/src/csv_bench.rs`) loads
the engine bench's `Ventes.csv` (`cargo xtask engine-bench gen --rows N --out DIR`: a date, four
whole numbers, a currency; 34 MB at 1 M rows, 348 MB at 10 M, 1.78 GB at 50 M) into a
`TableData`. Each run is a fresh process, so the peak working set is the run's own.

- `m`: the query the engine bench writes (`Csv.Document` with `QuoteStyle.None`,
  `Table.PromoteHeaders`, `Table.TransformColumnTypes` in en-US), as `table.import {m}` and
  `model.refresh` run it.
- `csv`: `bicraft_data::csv::read_csv` (`table.import {csv}`) with the same column types.

Before is `main` at 9bde25a, after is the fast CSV path
([formats/data-sources.md](formats/data-sources.md), *Fast CSV loading*). The row, step, time
and file-size caps are lifted for both, separate target dirs, release builds, medians of 3 on
the machine of the engine bench below (20 threads), under the bench-quiet lock with no build
running. Both builds load the same values (checksums of the whole numbers, the currency and
the dates equal at every size).

| Rows | Reader | Before | After | Speed-up | Peak memory before → after |
|---:|---|---:|---:|---:|---:|
| 1 M | `m` | 3.90 s | 0.100 s | ×39 | 761 MB → 215 MB |
| 10 M | `m` | 46.5 s | 0.725 s | ×64 | 7.39 GB → 1.48 GB |
| 50 M | `m` | not run (refused over 256 MiB; about 37 GB) | 3.76 s (worst 4.39 s) | | 6.95 GB |
| 1 M | `csv` | 0.983 s | 0.095 s | ×10 | 367 MB → 182 MB |
| 10 M | `csv` | 11.3 s | 0.697 s | ×16 | 3.64 GB → 1.73 GB |

- **Where the time went.** The evaluator built an M value per cell (records, then a type
  conversion step), on one thread, and held several copies; `read_csv` kept a `String` per
  field until the column was typed. The fast path parses chunks on all cores and types each
  field straight into its column.
- **Where the memory goes now.** The table itself: 24 bytes a cell (`CellValue`), 1.44 GB for
  the 60 M cells at 10 M rows, plus one 64 MiB region and its chunks. `read_csv` also holds
  its input. A columnar target (typed vectors or codes and a dictionary, as the storage engine
  keeps them) would take the 10 M table to about 0.3 GB; `TableData` is the interface of every
  loader, so that is a separate change.
- 50 M with `csv` was not run: the input (1.78 GB) and the table (about 7.2 GB) did not fit the
  free memory of the machine during the run.

## PBIX refresh (2026-11-06)

`bicraft-cli refresh X.pbix --data-root DIR [--table T]` (`model.refresh` on a PBIX, M11.18) on the
engine bench's star schema (`engine-bench gen --rows N`: `Ventes` N rows from a CSV file,
`Produit` 1,000, `Magasin` 200, `Calendrier` 1,826; three relationships), its PBIX built by
BIcraft (`bicraft-cli run --file build.json`). Each run is a fresh process (wall time from process
start, peak working set sampled every 50 ms), release build, medians of 3, on the machine of the
engine bench below, with other agents' builds running (measured, but not under the bench-quiet
lock). Every run reopens its output with BIcraft's reader before replacing the file.

| Rows | Run | Time | Peak memory | Against |
|---:|---|---:|---:|---|
| 1 M | build the PBIX from the CSV files (`bicraft-cli run`) | 1.21 s | 316 MB | baseline |
| 1 M | refresh every table | 1.09 s | 353 MB | ×1.1 faster than building it |
| 1 M | refresh `Produit` only | 0.29 s | 196 MB | ×3.8 faster than every table |
| 1 M | the same data refreshed as a PBIP project (BIcraft's own snapshot) | 4.42 s | 548 MB | the PBIX refresh is ×4.1 faster |
| 10 M | build the PBIX | 12.2 s | 2.21 GB | baseline |
| 10 M | refresh every table | 10.2 s | 2.53 GB | ×1.2 faster than building it |
| 10 M | refresh `Produit` only | 0.97 s | 0.69 GB | ×10.5 faster than every table |
| 10 M | the same data refreshed as a PBIP project | 47.4 s | 5.17 GB | the PBIX refresh is ×4.6 faster, at half the memory |

- **Where the time goes** (10 M, `phases_ms`): the M load 1.5 s, the VertiPaq encoding of the
  four tables 8.2 s, writing the package and reopening it 0.35 s.
- **Where the memory goes.** The refreshed table's rows, held as `CellValue`s while they are
  encoded (60 M cells at 10 M rows), plus the encoder's data ids and the decompressed backup. A
  refresh of a dimension never decodes the fact table: 0.69 GB at 10 M is the backup and the
  package. Under `BICRAFT_MEMORY_LIMIT` these are reserved first and a refresh that does not
  fit is refused before anything is written; streaming the rows into the encoder (no full
  `CellValue` table) is the next step for larger tables.
- Not measured: a PBIX whose `DataModel` the Analysis Services engine wrote (the bench's
  Desktop-refreshed files point at CSV files outside the scratch folder, and
  `sources.rebase` refuses a PBIX with stored rows).

## The DAX engine bench

`cargo xtask engine-bench` runs 22 DAX queries, shaped like report visuals, through BIcraft's
evaluator (`dax.evaluate` over `bicraft-cli mcp`) on a star schema from 10 k to 100 M fact rows. It
records BIcraft's times and peak memory, and checks every answer cell by cell against reference
results. The storage engine it exercises is described in [storage engine](storage-engine.md),
and the memory budget and limit in [beyond RAM](beyond-ram.md).

### Model and queries

- **Model** (`xtask/src/engine_bench.rs`): a star schema, all Import. `Ventes` (the fact, N rows:
  `Date`, `ProduitKey`, `MagasinKey`, `ClientKey`, `Quantite` int64, `Montant` currency),
  `Produit` (1,000 rows: 10 categories, 50 subcategories, catalogue price), `Magasin` (200 rows,
  8 regions), `Calendrier` (2021-01-01 to 2025-12-31, marked as date table). Three active
  many-to-one single-direction relationships from the fact. `ClientKey` has N / 10 distinct
  values (high cardinality: 10 M customers at 100 M rows). Eight measures: `CA` (SUM), `Qte`,
  `Nb clients` (DISTINCTCOUNT), `CA YTD` (TOTALYTD), `CA N-1` (SAMEPERIODLASTYEAR),
  `CA Var N-1`, `Part CA` (DIVIDE over ALL), `Rang produit` (RANKX). Data: a deterministic
  generator (splitmix), uniform keys, amount = quantity × price × (1 − discount 0–30 %).
- **Queries** (`xtask/engine-bench/queries.json`, Q01 to Q22): totals, `SUMMARIZECOLUMNS` by
  category / month / store × category, `DISTINCTCOUNT` (total, by region), `CALCULATE` with a
  dimension filter, `TREATAS` filter tables as Desktop's slicers write them, YTD, last year,
  share of total with `ALL`, `RANKX` over products, `TOPN`, `FILTER` over the fact, row-by-row
  `SUMX` with `RELATED`, Desktop's `TOPN 501` table by customer, a `COUNTROWS` of a grouping by
  customer, `ROLLUPADDISSUBTOTAL`, `AVERAGEX`, a `KEEPFILTERS` filter on a fact column,
  `MIN`/`MAX`. Each has a full `ORDER BY` or one row.
- **Variants** (`engine-bench gen --variant bigdim|skew`, `xtask/src/engine_variants.rs`), 14
  queries each:
  - `bigdim` (`queries-bigdim.json`): the standard data plus a `Client` dimension of one customer
    per five sales lines (2 M customers at 10 M sales) with a unique name, a segment (6), a city
    (2,000) and its country (40), an age band (7) and a loyalty flag, related to
    `Ventes[ClientKey]`. Queries filter or group by the dimension: `TREATAS` slicers on
    attributes, `CALCULATE` with an attribute filter, a range on the key (`CALCULATE` and
    Desktop's between slicer `FILTER(KEEPFILTERS(VALUES(…)), AND(…))`), groupings by attributes,
    a `TOPN 501` table by customer name, `AVERAGEX` over the customers, customers over a
    threshold, `SUMX` with `RELATED` to the dimension, a slicer on three names, a city's detail
    table.
  - `skew` (`queries-skew.json`): Zipf-like keys with integer weights (100,000 products and the
    200 stores with weight `1/k`, customers `1/(k + 100)`), sales in date order with mostly one
    unit and no discount, an order number shared by 1–5 lines (`Ventes[NumCommande]`, 3.3 M
    distinct texts at 10 M), 1 % anonymous sales (blank customer) and the customer dimension.
    Queries: groupings over skewed keys, `TOPN 501` tables by product, order number and
    customer, distinct counts of the order numbers, the biggest and a tiny product, `RANKX` over
    the 100,000 products, a `FILTER` on rare quantities, the blank row.

### Data and reference results

`xtask/engine-bench/run.ps1` builds one tier end to end in `<data-dir>\r<N>` (Windows only):

| Step | What it does |
|---|---|
| `seed` | `engine-bench gen` writes CSV files of 10,000 sales lines and `build.json`; `bicraft-cli run` builds `seed.pbix` from them (M partitions `Csv.Document(File.Contents(...))`); then the CSV files of N lines replace them at the same paths. |
| `refresh` | Power BI Desktop opens `seed.pbix`, refreshes the model by TOM over the N-line files and saves the database with AMO `ImageSave`; `engine-bench swap-datamodel` puts that image in the package as its `DataModel` → `bench.pbix`. The VertiPaq data is then encoded, compressed and segmented by the real engine. |
| `own` | Instead of `seed` + `refresh`, without Desktop: CSV files of N lines, and `bicraft-cli run` builds `bench.pbix` with the data encoded by BIcraft itself. |
| `desktop` | `desktop.ps1` runs each query once in Desktop on `bench.pbix` and records its answers, untimed: the reference results. |
| `bicraft` | `engine-bench bicraft` runs the queries through `bicraft-cli mcp` → `bicraft.json`. |
| `compare` | Checks BIcraft's answers against the reference → `answers.md`, `answers.json`, with BIcraft's own times of the queries that agree. |

A tier's files stay in `<data-dir>`; nothing is committed. The comparison requires the same
result sets, row counts and rows in order, numbers within 1e-9 relative, and matches columns by
name (an order difference is reported). A query whose answer differs is reported as such and its
times are not counted. The bench times BIcraft only.

### Regimes and harness

- **Start**: a fresh process per query: process start, MCP `initialize`, `file.open`, then the
  query once (it decodes the columns it needs).
- **Cold**: one session with the data open, `dax.clear_cache` before each run (it empties the
  storage engine's caches and kept results, and keeps the data and its dictionaries), median of 3.
- **Warm**: one session runs every query once, then 5 more times (median; a query over 10 s is
  repeated once). Warm answers come from the storage engine's caches.
- Also recorded: `file.open`, the first `COUNTROWS(Ventes)` (XPress9 decompression + ABF parse +
  metadata index), per fact column the first and second `MAX` (decode + scan against scan), the
  server-side `elapsed_ms`, and the process' peak working set.
- **The timed client is a release build.** `cargo xtask engine-bench bicraft|rounds` from a debug
  xtask re-runs itself through `cargo run --release`: a debug client parsing a 2,000-row answer
  (Q05) took longer than the query. The wall time includes sending, answering and parsing every
  row.
- **Interleaved rounds.** `engine-bench rounds --rounds 3 --modes without,with` measures round 1
  (without, then with report-derived aggregations), then round 2, and so on, so a load change
  weighs on both modes alike. Each round stays in `bicraft[-aggs].roundN.json`; they are merged
  into `bicraft.json` / `bicraft-aggs.json`: per query, the median of each timing over the rounds,
  every round's times under `rounds`, the highest peak memory. A query whose results differ
  between two rounds no longer counts as the same.
- **`engine-bench check --data <data-dir>`** reads every size folder and fails when a query's
  results differ from the reference. It is run at each milestone.

### Running it

```powershell
# outside a synced folder; the Desktop steps take a lock so that one Desktop runs at a time
$env:CARGO_TARGET_DIR = '<target-dir>'
cargo build --release -p bicraft-cli -p xtask
# one tier end to end: seed, refresh in Desktop + ImageSave, reference results, BIcraft, compare
powershell -ExecutionPolicy Bypass -File xtask\engine-bench\run.ps1 -Rows 1000000 -Data <data-dir> -Target <target-dir>
#   -Steps desktop,bicraft,compare reuses the tier's bench.pbix; -Only Q01,Q05 runs a subset
#   -Variant bigdim|skew: the variants (r<N>-<variant>)
#   -Steps own,bicraft: data encoded by BIcraft, no Desktop
<target-dir>\release\xtask.exe engine-bench rounds --bin <target-dir>\release\bicraft-cli.exe --pbix <data-dir>\r1m\bench.pbix --dir <data-dir>\r1m
<target-dir>\release\xtask.exe engine-bench check --data <data-dir>
```

`cargo xtask engine-bench` with no step prints every step and its options. The variants build
their data with BIcraft only:

```powershell
powershell -ExecutionPolicy Bypass -File xtask\engine-bench\run.ps1 -Rows 10000000 -Variant bigdim -Steps own,bicraft -Data <data-dir> -Target <target-dir>
```

The fact of a variant is written in one file and one partition per 4 M lines (BIcraft's M read
files of at most 256 MB before the fast CSV path; the split is kept so the data stays the same).

**Machine.** AMD Ryzen AI 9 365 (10 cores, 20 threads), 31 GB RAM (free memory between 2.6 and
15.6 GB: other workloads held up to 12.7 GB), Windows 11 10.0.26200, Defender on, data on NTFS
outside any synced folder, rustc 1.99.0, release builds. Milestones run on a quiet machine (no
build running, CPU ≤ 12 % at the start); other sections say when the machine was shared.

### Milestone 2 (2026-10-25)

`main` 2bc2aed: storage engine, worker pool, grouped-sum kernel, result path without copies,
faster open of a large PBIX. Release client, 3 interleaved rounds (without, then with
aggregations), quiet machine, **on battery** (BIcraft's absolute times are slightly pessimistic,
see the bisect below). The 100 M `bench.pbix` was regenerated once, measured, then deleted. All 22
results were identical to the reference in every round, tier and mode.

Median over the 22 queries of each query's median:

| Rows | Mode | `file.open` | Start | Cold | Warm | Peak memory |
|---|---|---:|---:|---:|---:|---:|
| 1 M | without | 28 ms | 64 ms | 1.7 ms | 0.4 ms | |
| 1 M | with | | 60 ms | 0.6 ms | 0.4 ms | |
| 10 M | without | 68 ms | 120 ms | 3.9 ms | 0.4 ms | |
| 10 M | with | | 117 ms | 0.6 ms | 0.5 ms | |
| 100 M | without | 0.44 s | 0.61 s | 23.8 ms | 0.4 ms | 5.40 GB |
| 100 M | with | | 0.59 s | 1.8 ms | 0.5 ms | 6.90 GB |

The two heaviest queries, Q17 (table by customer, `TOPN 501` over one group per customer) and
Q05 (store × category, 2,000 rows), in ms:

| Rows | Query | Cold without | Cold with | Warm without | Warm with |
|---|---|---:|---:|---:|---:|
| 1 M | Q17 | 22.3 | 11.2 | 10.9 | 9.6 |
| 1 M | Q05 | 8.1 | 4.9 | 5.9 | 5.2 |
| 10 M | Q17 | 174 | 102 | 96 | 91 |
| 10 M | Q05 | 11.3 | 5.3 | 5.7 | 4.9 |
| 100 M | Q17 | 3,322 | 1,003 | 980 | 880 |
| 100 M | Q05 | 39.2 | 7.9 | 5.6 | 5.9 |

- **Start at 100 M** (process + `initialize` + `file.open` + the query) is 0.49–0.85 s per query,
  except Q17 (3.7 s), Q07 and Q18 (1.2 s each). At the first milestone (`main` 4505911, on AC) it
  was 1.96 s median, with `file.open` at 1.13 s.
- **Peak memory at 100 M** (working set, highest of the 3 rounds): 5.40 GB without aggregations,
  6.90 GB with them (first milestone: 5.48 / 7.01 GB).
- **Most warm answers take 0.3–0.5 ms.**
- **Bisect.** Some cold times looked slower than at the first milestone (Q09–Q11, Q16, Q17,
  Q19–Q21: 0.3–1.5 ms at 10 M, 3–9 ms at 100 M, Q17 3,080 → 3,322 ms). Four binaries (4e7ee73,
  the kernel merge 629c7a0, the results branch 2c3b5ff, 2bc2aed) ran 3 interleaved rounds each at
  10 M. Only one regression came from code: Q16 ~15 % slower since the kernel merge (cold 3.7 →
  4.1–4.5 ms; fixed below). Q05 (28 → 16 ms, the results branch) and Q03 (5.5 → 4.6 ms, the
  kernel) got faster. The rest of the difference came from running on battery.
- The first launch of a freshly built executable is slow once (the operating system scans a new
  binary: 0.68 s for Q01 in round 1, 0.06 s in rounds 2 and 3).

**Q16 fix, on AC** (`feat/q16`, `docs/storage-engine.md` "Columns in one run"). The kernel merge
copied and walked the 1,034 row counts of each block for a column in one RLE run, and counted
rows in a separate pass. The deltas are now taken only when they cost fewer steps than adding
per row, and a lone run column adds and counts in one loop. `kernel_micro` at 10 M rows: Q16-like
15.1 → 12.1 ms on 1 thread, 1.71 → 1.53 ms on 20 threads; Q03 and Q04 unchanged. Cold on the
real files after the fix, without aggregations (milestone 2 in brackets), ms:

| Query | 1 M | 10 M |
|---|---:|---:|
| Q16 | 1.0 (1.3) | 2.4 (3.5) |
| Q03 | 1.1 (1.7) | 3.3 (3.7) |
| Q04 | 1.4 (1.7) | 3.4 (4.1) |
| Q05 | 7.0 (8.1) | 9.8 (11.3) |
| Q17 | 26.1 (22.3) | 157 (174) |

With aggregations, Q16 answers in 0.5 ms cold at both tiers; warm, Q16 takes 0.4 ms and Q17
9.5 / 84.5 ms. Apart from Q16, the gain comes from running on AC. At 1 M, Q17 cold is noisy
(rounds of 20–31 ms).

### Scaling from 10 k to 100 M rows (2026-10-24)

The first measurement with the storage engine (`dff9d4d`), one session per tier, data encoded by
the engine:

| Fact rows | Answered / refused | `file.open` | Peak memory | Refused before the storage engine |
|---:|---|---:|---:|---:|
| 10 k | 22 / 0 | 13.3 ms | 0.02 GB | 1 |
| 100 k | 22 / 0 | 26.8 ms | 0.03 GB | 1 |
| 1 M | 22 / 0 | 52.2 ms | 0.09 GB | 3 |
| 10 M | 22 / 0 | 228 ms | 0.66 GB | 5 |
| 50 M | 22 / 0 | 939 ms | 3.33 GB | 8 |
| 100 M | 22 / 0 | 1.81 s | 6.61 GB | 22 |

- **Start**: 6–29 ms per query at 10 k rows, 0.7–1.7 s at 100 M (5.3 s for Q18 and 7.3 s for
  Q17, one row per customer).
- **Warm**: the storage engine's caches (grouped scans, lookup tables, kept `SUMMARIZECOLUMNS`
  tables and iterator results, distinct values) answer most repeated queries in under 1 ms.
- **Memory**: the fact columns stay in their VertiPaq segments (bit-packed and RLE); most of the
  peak at 100 M is the formula engine's one row per customer (Q17, Q18).
- **Slowest cold scans at 100 M** were those through the per-row path: Q02 43 ms (a
  hash-encoded column read through a table per code), Q15 37 ms (a filtered scan), Q04 77 ms and
  Q19 124 ms (60 and 48 groups through a lookup table). The kernels and the worker pool below
  addressed them.
- **Limits**: 2 × 10^11 row visits per query, 50 M grouping combinations, 2 × 10^9 rows per table,
  an 8 GiB decompressed backup. Before the storage engine they were 2 × 10^10 row visits, 2 M
  groupings, 50 M rows per table and 1 GiB, and every query was refused at 100 M.

### With and without report-derived aggregations (2026-10-24)

Aggregations derived from a bench report with one page per query (`docs/storage-engine.md`,
"Aggregations"; `cargo xtask engine-bench report` lists its commands, `gen` writes them into
`build.json`, and `engine-bench bicraft --aggregations` applies them in memory to an older
`bench.pbix`). "Without" applies the same report with the aggregations off (`--report`); "with"
builds them in the foreground after the data load (`dax.aggregations {"build": true}`, timed
apart) and keeps them through `dax.clear_cache`, like a precomputed aggregation table. In a
session that does not ask for them, a background job builds them while the session is idle, so
no query waits for them. 22/22 identical at every size, in both modes.

| Rows | Aggregations | Rows (% of the fact) | Memory | Build | Peak memory without / with | Scans answered |
|---|---|---|---|---|---|---|
| 1 M | 8 | 165,226 (16.5 %) | 15.6 MB | 0.4 s | 0.09 / 0.12 GB | 86 of 94 |
| 10 M | 8 | 1,527,986 (15.3 %) | 139 MB | 2.2 s | 0.60 / 0.77 GB | 86 of 94 |
| 50 M | 8 | 9,098,772 (18.2 %) | 847 MB | 18.7 s | 2.82 / 3.91 GB | 86 of 94 |
| 100 M | 8 | 14,098,532 (14.1 %) | 1.31 GB | 42.6 s | 5.55 / 7.37 GB | 86 of 94 |

- The aggregations: year × category × region, product, day, category × store, product ×
  quantity, year × category × store, year × product × region, customer (at 1 M). The eight scans
  left to the fact are distinct counts of customers under a filter or grouping on another table
  (Q07, Q18), whose grain would be close to the size of the fact.
- The cold median is about 2 times as fast with them at 1 M, 4 times at 10 M and 9 times at 50 M
  and 100 M. Start and warm do not change (warm is answered by the scan cache either way).
- `cargo xtask engine-bench aggs-summary --data <data-dir>` prints the table for the size folders
  that have `bicraft-aggs.json`.

### Big dimension and skewed data (2026-10-26)

`feat/bigdim`, 10 M sales, data built by BIcraft (`-Steps own,bicraft`), release client, 3
interleaved rounds before / after, median per query (before = `main` 6d693dc, after =
`feat/bigdim` 68ecd57; on AC, other builds running at the same time; a query over 60 s is
refused). Every answer is identical before and after where both answer, in every round, and the
variants' queries equal the reference evaluator in process at 200,000 sales (all but S12, which
the reference refuses at its work cap).

`bigdim` (2 M customers), ms:

| Query | Start: before → after | Cold: before → after | Warm: before → after |
|---|---:|---:|---:|
| B01 Segment slicer (TREATAS) on a bar by category | 1,779 → 364 | 21.9 → 17.3 | 0.85 → 0.57 |
| B02 `CALCULATE` with a filter on an attribute | 1,770 → 349 | 56.3 → 44.8 | 0.72 → 0.40 |
| B03 By country, with `DISTINCTCOUNT` | 1,878 → 444 | 181 → 149 | 0.80 → 0.48 |
| B04 Cities of one country | 1,783 → 402 | 67.8 → 42.8 | 1.80 → 1.47 |
| B05 Table by customer name (`TOPN 501`) | 2,661 → 1,131 | 303 → 208 | 90.4 → 74.5 |
| B06 Between slicer on the key (Desktop's form) | 4,389 → 932 | 2,017 → 68.7 | 1,907 → 25.1 |
| B07 `CALCULATE` with a range on the key | 3,892 → 895 | 1,517 → 37.2 | 1,502 → 24.6 |
| B08 Segment × age band, year slicer | 1,844 → 450 | 115 → 77.7 | 1.91 → 1.45 |
| B09 `AVERAGEX` over the customers by segment | 4,331 → 1,346 | 1,743 → 453 | 0.53 → 0.46 |
| B10 Customers over a threshold | 8,248 → 1,352 | 5,312 → 444 | 5,135 → 332 |
| B11 `SUMX` with `RELATED` to the customers | refused → 395 | refused → 55.8 | refused → 0.36 |
| B12 Slicer on three customer names | 3,222 → 1,781 | 24.4 → 18.8 | 10.0 → 6.59 |
| B13 Active customers by segment in 2025 | 1,701 → 372 | 91.3 → 59.6 | 0.90 → 0.64 |
| B14 A city's customer table (`TOPN 501`) | 3,022 → 1,512 | 773 → 539 | 71.3 → 43.0 |

`skew` (100,000 products, 1 M customers, 3.3 M order numbers), ms:

| Query | Start: before → after | Cold: before → after | Warm: before → after |
|---|---:|---:|---:|
| S01 By category (skewed products) | 337 → 271 | 5.90 → 5.93 | 0.53 → 0.50 |
| S02 Table by product (`TOPN 501`) | 410 → 373 | 61.5 → 65.1 | 6.42 → 7.46 |
| S03 Distinct orders and customers | 1,538 → 1,715 | 0.69 → 0.67 | 0.40 → 0.35 |
| S04 By month: sales, distinct orders | 1,595 → 1,734 | 34.4 → 37.9 | 0.70 → 0.49 |
| S05 The biggest and a tiny product | 462 → 377 | 96.6 → 42.2 | 53.6 → 5.72 |
| S06 Slicer on the three biggest products | 352 → 322 | 7.91 → 7.15 | 0.88 → 0.81 |
| S07 Table by order number (`TOPN 501`) | 1,808 → 2,058 | 261 → 300 | 162 → 187 |
| S08 Store × category in 2024 | 321 → 290 | 10.7 → 9.71 | 5.40 → 7.23 |
| S09 `SUMX` with `RELATED` over 100,000 prices | 279 → 269 | 11.7 → 11.8 | 0.44 → 0.42 |
| S10 Table by customer with distinct orders (`TOPN 501`) | refused → 2,591 | refused → 467 | refused → 61.8 |
| S11 `FILTER` on rare quantities | 305 → 329 | 78.1 → 72.1 | 84.1 → 72.0 |
| S12 `RANKX` over 100,000 products (`TOPN 501`) | 11,266 → 764 | 11,209 → 472 | 5.83 → 5.65 |
| S13 By segment, anonymous sales included | 692 → 341 | 25.5 → 24.0 | 0.53 → 0.44 |
| S14 Share of total by year × category | 292 → 265 | 5.64 → 5.35 | 0.70 → 0.53 |

- The worst cold query of `bigdim` goes from 5.3 s (B10; B11 refused) to 0.54 s (B14), of `skew`
  from 11.2 s (S12; S10 refused) to 0.47 s. Peak memory of the `skew` session 3.24 → 1.24 GB (the
  distinct counts of many groups no longer try one bitmap per group).
- **Start** on `bigdim` goes from 1.7–4.4 s to 0.35–1.8 s: the first query built the relationship
  map of 2 M keys by hashing them (now a table indexed by key for whole-number keys in a dense
  range), and a column's sort ranks compared two copies of the values per comparison (a `MAX` of
  `Ventes[ClientKey]` took 2.4 s, now 0.39 s; on the standard 10 M file's 1 M customers
  0.97 → 0.18 s). `S03`, `S04` and `S07` start at ~1.7 s: reading the 3.3 M order numbers'
  dictionary (1.5 s), unchanged.
- Within noise (±15 % here): S02, S03, S04, S07, S08 warm; nothing in the changes touches them.
- What remains slowest: the `TOPN 501` tables by customer name (B05, B14: groupings by a 2 M-value
  column, 0.2–0.5 s cold) and the threshold over 2 M customers (B10, 0.44 s: a table of 2 M values
  and one grouped scan by customer).
- **The standard bench, same build**: 22/22 identical in every round. In an A/B against `main` on
  the same files, Q13 (`RANKX` over the products) at 10 M goes 13.9 → 8.6 ms cold (one scan of the
  products per cell no longer), and warm answers 0.4–0.5 → 0.3–0.4 ms (validation and evaluator
  setup). Start is unchanged within noise.

Exactness tests (`crates/engine/src/preview_eval_bigdim_tests.rs`): the variants' queries against the
reference evaluator, cold and warm. The reference path is quadratic for some of them (S12's
`RANKX` over every product: products² × sales row visits; context transition over 70,000
customer keys), so a debug build (`cargo xtask ci`) checks smaller data: 6,000 sales and 200
products for `skew`, the tables of values over 3,000 customers (`RELATED` stays at 70,000). The
full sizes run with `cargo test --release -p bicraft-engine bigdim_tests` (9 s for the module
after the release build, 2026-10-29; a debug build took 260-440 s per test).

In process (`variant_micro`, 4 M sales) and on a large model (`query_overhead_micro`: 200 extra
tables, 2,000 measures; a card query 4.27 → 0.56 ms, a constant 1.33 → 0.22 ms), see
[storage engine](storage-engine.md), "Big dimensions, skewed data and per-query costs".

### Engine changes, before and after (2026-10-24 to 2026-10-25)

Each change was measured as an A/B: the base build and the change, 3 interleaved rounds per tier
(base, change, base, …), median per query over the rounds, 22/22 identical in every round.

**Opening a large PBIX** (`feat/open` 7506c1a against `main` 629c7a0, quiet machine;
`docs/storage-engine.md` "Opening a large PBIX"). The `DataModel` is decompressed once instead
of twice before the first answer, the stored part is read on parallel positional reads, XPress9
decodes on one thread per core, and a column's segments and first-seen ids are built in parallel.

| Rows | `file.open` before → after | Start median before → after |
|---|---:|---:|
| 1 M | 32 → 27 ms | 89 → 60 ms |
| 10 M | 142 → 65 ms | 277 → 113 ms |

Every query's start is faster: at 10 M 95–145 ms instead of 257–323 ms, Q17 (one row per
customer) 288 instead of 472 ms; at 1 M 51–87 ms instead of 78–110 ms. Peak memory is unchanged
(10 M: 0.57 GB, 0.61 GB before). Projected to 100 M: `file.open` ~1.26 → ~0.58 s, start ~1.96 →
~0.75 s (milestone 2 then measured 0.44 s and 0.61 s).

**Result path without copies** (`feat/results` against `main` 629c7a0, quiet machine). Q05 warm
(2,000 rows) spent most of its time copying and allocating around the result. Tool replies are now
written straight from the result (`ToolResult::response_line`, byte-identical to the old
serialization, golden-tested), result objects are moved instead of copied by `json!`, `ORDER BY`
keys that name a result column are read in place, and text order compares without allocating.
Inside the server, Q05 warm at 1 M went from ~5.0 ms (call 3.5 ms, reply line 1.1 ms, drop
0.4 ms) to ~3.1 ms (call 1.8 ms: `ORDER BY` 0.45 ms, rows 1.0 ms); the rest of the wall time is
the transfer and the client's parse of the ~430 KB line. Wall times, ms:

| Query | Tier | Cold before → after | Warm before → after |
|---|---|---:|---:|
| Q05 (2,000 rows) | 1 M | 14.0 → 6.8 | 12.9 → 5.0 |
| Q05 | 10 M | 16.8 → 10.1 | 12.0 → 4.9 |
| Q05 with aggregations | 1 M / 10 M | 11.4 → 4.9 / 10.3 → 5.1 | 11.6 → 4.2 / 10.5 → 4.3 |
| Q13 (1,000 rows) | 1 M | 11.8 → 8.8 | 4.3 → 2.1 |
| Q13 | 10 M | 15.4 → 11.4 | 4.6 → 2.5 |
| Q17 (501 rows) | 1 M | 20.5 → 25.4 ¹ | 9.8 → 9.1 |
| Q17 | 10 M | 154 → 157 | 86.1 → 84.4 |

¹ Noise: its three cold rounds were 25.4, 20.2 and 30.8 ms. Five more interleaved rounds at 1 M
gave Q17 cold medians of 23.6 (before) and 20.7 ms (after). Q17 is bound by its scan and `TOPN`,
which this change does not touch.

**Grouped-sum kernel** (`feat/kernel` against `main` 4505911; machine shared, ±7 % between sets
of rounds; `docs/storage-engine.md` "Grouped-sum kernel"). Integer sums by data id go through an
interleaved kernel: four accumulator copies for few groups, rows counted once per block for all
the columns, a dummy slot for filtered-out rows, a column in one run added from per-slot row
deltas, padded worker accumulators. Grouped scans, cold (median of 5 interleaved rounds, ms
before → after, and end-to-end rows/s of the query after):

| Query | 1 M | 10 M | 10 M rows/s |
|---|---|---|---|
| Q03 (CA by category) | 1.4 → 1.2 | 4.1 → 3.5 | 2.9 G (was 2.4 G) |
| Q04 (CA, Qte by month) | 1.6 → 1.5 | 4.8 → 4.2 | 2.4 G (was 2.1 G) |
| Q05 (CA by store × category) | 16.7 → 17.8 | 20.4 → 20.6 | 0.49 G (formula engine: `ORDER BY`, rows) |
| Q16 (SUMX quantity × related price) | 1.1 → 1.3 | 3.0 → 3.1 | |
| Q19 (CA by region × year) | 1.8 → 1.9 | 4.9 → 5.0 | |

Projected to 100 M (the scan time grows linearly with the rows): Q04 38 → ~33 ms; Q05's scan gains
~3 %, the rest is the formula engine. What would still move the 100 M grouped scans: an
x86-64-v3 build (unpacking, +10–20 %; no `unsafe` dispatch), fusing the two-key map into the
kernel (Q05, Q19), and for Q05 the formula engine.

**Worker pool** (`feat/pool` against `main` b5b9b7a, quiet machine; `docs/storage-engine.md`
"Worker pool"). The scans run on a persistent worker pool instead of starting their threads each
time, and a worker is woken per 64 Ki rows (128 Ki in WSL 2, where waking a worker is slow)
instead of a thread started per 512 Ki rows. Cold, Windows, ms before → after: at 1 M Q03
3.3 → 1.5, Q04 3.9 → 2.1, Q08 2.8 → 1.4, Q10 8.1 → 3.6, Q12 3.1 → 1.4, Q14 3.4 → 1.6, Q18
4.8 → 2.5, Q19 4.1 → 2.1; at 10 M Q04 7.0 → 5.4, Q11 10.7 → 7.8, Q14 6.5 → 5.0, Q16 4.7 → 3.1,
Q21 4.4 → 2.9. No query is slower on Windows. In WSL 2 (Linux release builds, `bench.pbix` copied
into the VM) the only one is Q15 at 1 M (1.7 → 2.0 ms); it was 2.6 ms with 64 Ki rows per worker,
which is why the rows per worker follow the measured wake-up cost. Start does not change (with 5
fresh processes per query at 1 M, the 22 start times sum to 931 → 909 ms), nor does warm.

**`TOPN` over a grouping without building every row** (`feat/topn` against
`feat/report-aggregations` b5b9b7a; `docs/storage-engine.md` "Requests"). Typed first `TOPN` key
and candidate bound, only the kept rows built, lazy query variables for Desktop's `__DS0Core`,
groups of one column listed without counting. Q17 (`TOPN(501, SUMMARIZECOLUMNS(Ventes[ClientKey],
"CA", [CA]), …)`), median of 3 rounds:

| Q17 | Base | `feat/topn` |
|---|---:|---:|
| 1 M cold | 65.3 ms | 29.6 ms |
| 1 M warm | 23.9 ms | 13.7 ms |
| 1 M start | 120 ms | 71 ms |
| 10 M cold | 535 ms | 169 ms |
| 10 M warm | 251 ms | 90 ms |
| 10 M start | 645 ms | 322 ms |

The other 21 queries do not change beyond round-to-round noise (sub-millisecond differences).

Build note: two source trees sharing one `CARGO_TARGET_DIR` can leave the second tree's crates
"fresh" (cargo's dep-info paths are relative to the workspace root and compared by time), so an
A/B "after" binary is rebuilt after touching the changed files and checked by hash.

## Data view paging (2026-11-06)

`table.rows` (ROADMAP M11.17, docs/mcp.md "Data view reads") against the plain path an app
without paging would take: every row of the table read into JSON (as `dax.evaluate` of the whole
table returns it), sorted with a comparator, then the page sliced. **Measured** with
`cargo test --release -p bicraft-engine --lib table_rows_bench -- --ignored --nocapture`
(`crates/engine/src/preview_rows_tests.rs`), one run, on the machine above (AMD Ryzen AI 9 365,
build slot shared with other jobs): a synthetic table of 10,000,000 rows and 4 columns (whole
number, text with 26 values and blanks, fixed decimal with blanks, date), its codes kept
compressed as a stored model's are (segments of 4,096 rows, bit-packed and RLE). The sorted page
is checked equal to the plain path's.

| Operation | `table.rows` | Plain path | Speedup |
|---|---|---|---|
| First page (100 rows) | 4.2 ms | 11,782 ms | ×2,800 |
| A page in the middle | 0.2 ms | 11,782 ms | ×53,000 |
| Sorted on the decimal column, cold (orders all 10 M rows) | 525 ms | 48,422 ms | ×92 |
| Another page of that sort (the order kept) | 0.7 ms | 48,422 ms | ×74,000 |
| Filtered on two text values (1.46 M rows) and sorted on the text | 128 ms | 48,422 ms (sort only) | ×380 |

The cold sort is two sequential passes over the compressed codes (a counting sort over the
column's ranks) plus the order itself, 40 MB reserved against the memory limit and kept for the
next pages; the plain path holds the whole table as JSON (several GB at 10 M rows).
