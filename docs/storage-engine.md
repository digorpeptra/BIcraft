# Storage engine

BIcraft evaluates DAX in two layers, as Analysis Services does: a **formula engine** (the DAX
evaluator, `crates/engine/src/preview_eval*.rs`: filter contexts, `CALCULATE`, context
transition, time intelligence, table functions) and a **storage engine** that answers its
"scan, filter, group, aggregate" requests over the compressed VertiPaq columns. Before it, each
aggregation scanned its whole table in its own filter context, so a `SUMMARIZECOLUMNS` cost
cells × rows; now one grouped scan answers every cell. Results are identical to the per-cell
path (the reference), which stays in the code and runs whenever a request cannot be split exactly.

## Design decisions

- **Our own engine, not DuckDB or DataFusion at its core.** Both were considered and refused:
  they bring C++ (DuckDB) or a large dependency tree (DataFusion, Arrow) where the project keeps
  pure, safe Rust without C/C++; they would hold a second copy of the data in their own format
  (Arrow or DuckDB columns) next to VertiPaq's, where scanning the VertiPaq segments as they are
  costs no conversion and no extra memory; and DAX's semantics (blank rows of relationships,
  context transition, Currency arithmetic, first-seen orders, auto-exist) do not map onto SQL
  without a translation layer that would itself need the oracles. The kernels needed (filtered,
  grouped scans over dictionary codes) are small enough to own and to test against naive loops.
- **Formula engine over a storage engine, on compressed segments.** As in Analysis Services: the
  DAX evaluator keeps every semantic decision and asks for scans; the storage engine answers
  them over the segments VertiPaq wrote (bit-packed and RLE, read a block at a time, segment
  skipping), never over one code per row. A request it cannot split exactly goes to the per-cell
  reference path, which stays in the code as the oracle of the storage engine.
- **Three regimes, measured honestly.** *Start*: a fresh process that opens the file and
  answers. *Cold*: the data open, every cache emptied before each run (`dax.clear_cache`, the
  counterpart of Analysis Services' `ClearCache`), median of 3.
  *Warm*: the same query again. A statistic precomputed when a column is read and kept by
  `dax.clear_cache` (id ranges, per-segment sums of ids, the dictionary's tables) is a
  precomputation, published as such, never presented as a faster scan.
- **Caches never serve a stale result.** Every cache is keyed by the model's hash and the data's
  generation; results kept by expression text also by a fingerprint of every measure and by the
  filter context's content, and only for pure expressions (no clock, user, random value or
  `ALLSELECTED`). Each cache has an invalidation test (model edit, new data, `dax.clear_cache`,
  another session, a redefined measure, a moved clock, another outer selection), and the random
  differential test runs every query cold and warm. Three stale or wrong results were found that
  way and fixed: text-keyed caches that ignored a redefined measure, an expression cloned for
  analysis whose columns resolved by a reused address, and iterator results kept across a change
  of the clock.
- **The worst case first.** The bar is the worst case, not the median: every bench query, at
  every size from 10 k to 100 M rows and in each regime, answered, with zero value different from
  the reference results. `cargo xtask engine-bench check` fails otherwise; it is run at each
  milestone.

## Data: columns kept compressed

A column ([`ColData`](../crates/engine/src/preview_store.rs)) maps each row to a code into its
table of distinct values (code 0 is blank, codes numbered in the order the rows first meet the
values). A column read from a VertiPaq backup (PBIX `DataModel`, PBIP `cache.abf`) is **not
unpacked**: it keeps its segments as the engine wrote them ([`Packed`](../crates/scan/src/packed.rs)):

- bit-packed data ids (`XMRENoSplit<N>`: `64 / N` ids per word, a base id per segment),
- hybrid RLE segments (runs of one id, and runs of bit-packed ids in the sub-segment),
- consecutive ids (`XM123`),

plus one table from data id to code. Opening a column copies its segments' words (the compressed
size, one segment per task on the worker pool) and makes one pass over the ids to number the
distinct values in first-seen order: the segments are cut into one chunk of consecutive segments
per thread, each chunk lists its ids in first-seen order with their bitmap, then each chunk's
bitmap is replaced by the union of the chunks before it (one pass, in order) and the chunks drop,
on several threads, the ids an earlier chunk has; RLE runs count one step per run. It
also records each segment's exact id range, for segment skipping, and its sum of data ids with
its rows of blank ids (small materialized aggregates, Moerkotte, VLDB 1998): a sum without filter
or grouping of a value-encoded column is read from them, one step per segment. These statistics
are computed when the column is read, belong to its encoding as its id ranges do, and are not a
cache: `dax.clear_cache` keeps them. No array of one code per row is
built: the kernels unpack a block of 2,048 rows at a time into their own buffers. A column that
an evaluator path needs one code per row for is unpacked once, on first use (`Codes::plain`); the
micro-bench reports when that happens. Columns of literal tables and BIcraft's own data snapshot
are in memory (one code per row).

What queries derive from a column is kept with the column (`Derived`), shared by every query
over the same data:

- tables of the dictionary: value index, sort ranks, per-code numbers, Currency values and
  whole-number values, count of non-blank values, and the kinds of its values (some Currency,
  all Currency, all whole numbers);
- tables of the rows: rows per code of the whole column, and the first row of each code (read
  from the top of the column until every code that some row holds is met: a few blocks for a
  column of few values).

`dax.clear_cache` drops the tables of the rows and keeps the tables of the dictionary, which are
part of a column as Analysis Services keeps it in memory (its `ClearCache` does not drop
dictionaries either).

## Opening a large PBIX

A fresh process answers its first visual after `file.open` (ZIP read, XPress9 decompression of
the `DataModel`, ABF and `metadata.sqlitedb` parse) and the first query (the store's column index,
then each column the query reads). Measured at 10 M rows (91 MB part, 109 MB backup, 20 threads,
Windows), before → after (2026-10-25, `feat/open`):

| Step | Before | After | How |
|---|---:|---:|---|
| ZIP read of the stored `DataModel` | 48–53 ms | 11 ms | one allocation of the declared size instead of a doubling series; a stored part of 8 MiB or more read by positional reads on up to 8 threads (`bicraft-opc` `parread.rs`), each thread taking the CRC-32 of its piece, the pieces' CRCs combined and checked against the entry's |
| XPress9 decompression | 77–83 ms | 37–46 ms | one decoder thread per core instead of at most 8 (`docs/formats/xpress9.md` §7); output from a zeroed allocation (pages faulted in by the workers, not zeroed first by the calling thread) |
| Model (`metadata.sqlitedb`) | 3 ms | 3 ms | |
| First query: decompression again | 80 ms | 0 | `file.open` hands the backup it decompressed to the session's store ([`PreviewCache::seed_backup`](../crates/engine/src/preview.rs)): the first query indexes it instead of decompressing the part a second time |
| First query: one column (`Montant`, 174 k distinct) | 20 ms | 14 ms | segments' words copied on the worker pool; first-seen ids joined by chunk bitmaps (above) |
| First query: `ClientKey` (1 M distinct) | 48 ms | 34 ms | same |

- **Which backup the store may take.** The handed-over backup is kept with the identity of the part
  it was decompressed from (sampled fingerprint, length and address of its bytes, which stay put
  while the package holds them) and is used once, only when the session's `DataModel` part is
  still that one; a new fingerprint (a save that rewrote the part, another file) drops it, and the
  store decompresses the part as before. Tests: `open_seed_tests.rs` (taken over by the first
  query, same answers without it, a backup recorded for other bytes is ignored, another file
  opened). A PBIT, a deferred part (over 4 GiB) or a backup read as a stream hands nothing over.
- **Memory.** Unchanged at the first query (the store holds the backup either way). Between
  `file.open` and the first query the backup is held once, where before it was freed and
  decompressed again.
- **Not done.** Decoding only the columns a query needs: a single-threaded `DataModel` is one
  XPress9 session whose matches reach up to 4 MiB back, so a byte range cannot be decoded without
  what precedes it in general. Overlapping the ZIP read with decompression (≤ 11 ms at 10 M).
  The single-thread decoder runs at ~310 MB/s; it is now most of `file.open`.

## Kernels: `bicraft-scan` (L0)

[`crates/scan`](../crates/scan/src/lib.rs) is a safe-Rust crate of scan kernels, whose only dependency is `rayon-core` for its worker
pool (see "Worker pool" below):

- `Col`: a column's codes, in memory or `Packed`;
- `Selection`: the rows visible in a filter context, as a conjunction of `allowed[code[row]]`
  predicates (one bitmap per column, over its dictionary) and row masks;
- `Key`: dense group numbers from the codes of the grouping columns, each through an optional
  lookup table (a fact's foreign-key code → a dimension column's code);
- `Sink`: per-group state fed one selected row at a time: row counts, blank / non-blank counts,
  exact `i128` sums of whole or Currency values, Neumaier-compensated double sums, min / max sort
  ranks, distinct-code bitmaps, (group × code) histograms, first row per group, row counts with
  a first row (`Tally`);
- `scan` (over a selection), `scan_groups` (radix-partitioned for high cardinalities),
  `scan_list` (over a list of rows), `filter_list`, `mask`, `rows`; `Packed::counts_parallel`
  (rows per code of a whole column, one step per RLE run); `sum_ids` (per group, the sum of a
  compressed column's data ids: a value-encoded column's values are an affine function of its
  ids, checked on its dictionary when it is read, so its sums need no table per row).

Each scan first builds a **plan**: for a compressed column, the operation's per-code table
(allowed codes, group lookup) is composed with the column's id → code table once, so the inner
loop reads the unpacked data ids and looks them up directly. Bit-packed ids are unpacked a whole
word at a time by a loop specialized for each VertiPaq width. A block of a predicate column that
lies in a segment whose exact id range holds no allowed id is skipped without unpacking it
(**segment skipping**: a date filter on a fact loaded by date reads only the segments of those
dates), or in a zone of 64 Ki rows of it whose ids hold none
(zone maps, "Column-store techniques"); a block where no row is left is not unpacked further.

Execution is **morsel-driven** (Leis et al., SIGMOD 2014): rows are cut into 64 Ki-row morsels
taken by the workers from a shared counter; each worker filters 2,048 rows at a time into a
selection vector (vectorized execution, Kersten et al., VLDB 2018) and feeds its own sink; sinks
are merged at the end. It works on the compressed representation directly (Abadi et al., SIGMOD
2006). Threads: `BICRAFT_SCAN_THREADS`, else the machine's parallelism, fewer below 64 Ki rows
(one morsel; more where waking a worker is slow, see "Worker pool") per thread, none below
128 Ki rows, or when the sinks would exceed 1 GiB together. A
grouping whose state per worker would exceed 8 MiB (one group per customer) is
**radix-partitioned** by group: the workers route each selected row's (group, value) to
partitions of 256 KiB of state, then each partition is aggregated by one worker in cache and the
partitions are joined in order.

### Worker pool

Every parallel loop of the storage engine runs on a persistent worker pool
([`crates/scan/src/pool.rs`](../crates/scan/src/pool.rs), `rayon-core`). That covers the scans
(`morsels`, both phases of `scan_groups`, `mask`, `rows`, `filter_list`), `Packed::new` and
`Packed::counts_parallel`, and the engine's parallel column decoding (`decode_parallel`,
`first_seen_ids`). Before the pool, each of them started and joined its own threads with
`std::thread::scope`, which costs 1.1–1.5 ms per 20-thread scan on Windows (0.1 ms for one
thread). That cost is why a thread was only started per 512 Ki rows, so every scan under
~1 M rows ran on one thread. Waking the pool's parked workers costs 3–13 µs
(`pool_start_cost`; two Windows runs, before and after the fan-out below):

| Tasks | Scoped threads | Pool |
|---:|---:|---:|
| 1 | 115–129 µs | 0.0–0.1 µs |
| 4 | 293–334 µs | 2.9–4.2 µs |
| 10 | 619–797 µs | 4.3–4.6 µs |
| 20 | 1,140–1,539 µs | 5.6–12.7 µs |

- **One module.** Nothing else names `rayon-core`. The kernels call `pool::run` / `map` / `each`
  (run task *i* for *i* in 0..n, the caller running task 0), so the pool can be swapped without
  touching a kernel. `rayon-core` keeps its `unsafe` inside; our crates stay
  `forbid(unsafe_code)`.
- **Fan-out.** A loop of n tasks hands half of them to one worker, which hands half of those to
  the next, and so on, while each thread runs its first task. Starting the loop then costs about
  log2(n) wake-ups in a row instead of n. That matters where a wake-up is slow: in a WSL 2 VM
  (~70 µs per wake-up, against a few µs on Windows) 20 tasks start in 1.0 ms instead of 1.7 ms
  (scoped threads: 2.0 ms). On Windows the cost goes from 9 to 6 µs.
- **Lifetime.** Created on first use, once per process: one worker less than the scan threads
  (`BICRAFT_SCAN_THREADS`, else the available parallelism), because the calling thread works too.
  `set_threads` (tests, the background job) still caps the tasks of a scan.
- **Background job.** The aggregation build's thread (`pool::set_background`) sends its loops to a
  small pool of its own, sized for `BICRAFT_AGG_THREADS` (a quarter of the threads). A query's
  scans never queue behind the job's. `pool_foreground_latency_under_background_load`: a
  1 M-row grouped scan takes 1.07 ms alone, 1.06 ms while a loop of 32 M-row scans runs on the
  background pool (median of 60; 1.05 / 1.05 ms in a second run).
- **Panics.** Each task is caught. A failed task makes a scan return `None` as before, and the
  kernels that return rows or counts (`mask`, `rows`, `filter_list`, `Packed::new`,
  `counts_parallel`) redo the whole work on the calling thread rather than answer with a part
  missing. Before, a failed chunk was silently dropped there. The pool's workers survive
  (`pool_task_panic_is_a_failure_not_a_crash`).
- **Thresholds**, re-tuned with the kernels' micro-bench at 50 k – 2 M rows, 20 threads, sweeping
  the rows per worker from 512 Ki down to 16 Ki (`BICRAFT_SCAN_ROWS_PER_THREAD`,
  `BICRAFT_SCAN_PARALLEL_MIN_ROWS` override them for such sweeps). One worker per 64 Ki rows (one
  morsel) is the knee on Windows. At 1 M rows the filtered grouped sum drops from 6.2 to 1.2 ms,
  the compressed one from 7.7 to 1.1 ms and `sum_by_id` from 2.7 to 0.5 ms. Going below one
  morsel gains nothing, and under 128 Ki rows (two morsels) a scan stays on its thread as before.
- **Rows per worker follow the wake-up cost.** In WSL 2 a wake-up round trip takes ~90 µs,
  against ~1 µs on Windows, and there 128 Ki rows per worker beat 64 Ki. In the engine bench at
  1 M, Q15 / Q20 took 2.6 / 1.8 ms at 64 Ki, 1.9 / 1.1 at 128 Ki and 1.6 / 1.5 before the pool.
  The first scan big enough to ask therefore times a few two-task loops
  (`pool::wake_round_trip_ns`). A worker then gets at least that many rows as the round trip
  lasts in nanoseconds (the cheapest scans read about a row per ns), as a power of two between
  64 Ki and 512 Ki: 64 Ki on Windows, 128 Ki in WSL 2. The cheapest loops (`mask`, `rows`:
  ~2 G rows/s on one thread) take at least 256 Ki rows per worker: in WSL 2, at 64 Ki, waking
  15 workers cost more than `mask`'s whole single-threaded pass over 1 M rows (1.2 against
  0.4 ms). Morsels did not change (64 Ki rows), so `scan_ordered`'s reduction order, and the
  bits of double sums, are the same as before.

### Grouped-sum kernel (`sum_by_id_multi`)

The integer sums by data id (`SUM` / `AVERAGE` of value-encoded columns and of compressed
columns with a small dictionary, several columns of one table in one pass) are the cold
storage-engine time of grouped visuals. Q03, Q04 and Q05 of the bench do almost nothing else.
A block of 2,048 rows whose rows have their own groups goes through an **interleaved kernel**:

- **Four copies of the accumulators** (row `i` of a block adds to copy `i % 4`, when there are
  at most 127 groups). Rows of one group in a row then add to different copies, so an add does
  not wait for the previous one through memory (store-to-load forwarding). With more groups,
  consecutive rows seldom share one, and four copies would only fill the cache: four copies
  made 511 groups ~10 % slower.
- **Rows counted once.** A column's count is its rows minus its blanks. The rows per group are
  counted once per block, in the loop of the first column whose ids hold no blank there, and
  shared by every column. Each column then only counts its blanks, and only where a blank can
  occur. Whether one can occur comes from the segment statistics. They are exact because they
  are computed from the data when the column is read: rows of ids 0–2 and the largest id
  (`Packed::ids_within`). For a value table, its only ids without a value must be the blank ids.
- **No branch for a filter.** In a block a filter thins, a dropped row goes to a dummy slot past
  the groups, which is never read, instead of a branch per row.
- **Groups looked up in the loop.** With one grouping column, its data ids are mapped to groups
  inside the accumulation loop rather than being written out first.
- **Columns in one run.** A column lying in one RLE run over the block can add its value times the
  block's rows of each slot, using the row counts before and after the block, instead of one add per
  row. That costs a copy of the counts plus one pass over every accumulator element per such
  column. So it is chosen only when it takes fewer element steps than the adds per row
  (`defer_runs`). Otherwise the run column adds its value on each row. If no other column counts
  the block's rows, it counts them in the same loop (`lanes_const_rows`), in one pass. Q04 (a run
  column next to a counting one, 36 groups, 180 elements) takes the deltas. Q16 (one run column,
  1,001 groups, 1,034 elements) keeps the fused pass. Always taking the deltas up to 2,048
  elements made Q16 ~15 % slower cold (milestone 2 bisect).
- **Padding.** Each worker's accumulators have 16 unused elements on each side. Without them,
  small vectors allocated by different workers shared cache lines, and their adds bounced the
  lines between cores (false sharing). This is about half of the gain at 20 threads.

Blocks of one group, and blocks in runs of the key, keep their previous loops. Only integer
sums go through this kernel: u64 sums of ids, below 2^32 rows; i64 when the largest value times
the rows stays under 2^62; i128 otherwise. Integer addition does not depend on its order, so the
copies, blocks and workers may add in any order, and the result is exact and the same on any
number of threads. Double sums (`FloatSums` through `scan_ordered`) do not use the kernel, and
their reduction order is unchanged. Tests: `kernel_matches_a_naive_loop_on_random_shapes` (random
segment sizes and encodings, blanks in some segments only, value tables with and without other
invalid ids, one and two keys, a key in runs, few and many groups, with and without filters,
1 and 4 threads; `BICRAFT_KERNEL_CASES` for more), `ids_within_reads_the_statistics_exactly`,
`kernel_edges`, plus the earlier `sums_by_id_match_a_naive_loop_on_runs`.

**Measured** (`kernel_micro`, 10 M rows in 2^20-row segments, best run over at least 15 runs and
0.3 s per case, A/B against `main` 4505911 in interleaved runs, Windows, 20 threads):

| Shape | Before | After |
|---|---:|---:|
| Q03: one column, 10-bit key through a lookup to 10 groups | 4.6–4.8 G rows/s | 5.4–5.6 G rows/s (+15–20 %) |
| Q04: two columns (one in RLE runs), 36 groups | 3.4–3.5 | 3.9–4.0 (+13–16 %) |
| Q05: two keys, 510 groups | 3.4–3.5 | 3.5–3.6 (+2–4 %) |
| Filtered (~67 %), 10 groups | 4.3–4.5 | 3.9–4.2 (−5 to 0 %) |
| Hash-encoded values through a table (60,000 ids) | 3.8–3.9 | 4.6–5.1 (+17–35 %) |

On one thread: Q03 510 → 615 M rows/s (+20 %), Q04 355 → 395 (+12 %), the table case
410 → 560 (+35 %), Q05 and the filtered case unchanged. The prototype of idea #4 in
`docs/perf-ideas.md` (branch `exp/ideas`) reported ×2 against `sum_by_id`, which does not hold.
Its baseline was timed first, before the pool's workers and the parked cores had woken up. With
a warm-up and a minimum time per case, the same prototype is ~25 % faster than `main` at
20 threads, and this kernel gets most of that.

**x86-64-v3 (AVX2).** Choosing AVX2 code at run time (`#[target_feature]` with
`is_x86_feature_detected!`) needs `unsafe`, which the workspace forbids, so it is not done. A
second build profile (`RUSTFLAGS="-C target-cpu=x86-64-v3"`, CPUs since 2013–2015) is the safe
route. It is not shipped. Measured on the `exp/ideas` prototypes: the scatter-add does not
vectorize, so it brings ~10–20 % on the scans, mostly from unpacking.

## Requests: `preview_eval_storage.rs`

- **Predicates** (`pred`): the evaluator's masks for the scanned table become predicates: its
  own filters, and the filters of the tables that reach it through active relationships,
  propagated over codes (`propagate_codes`) rather than materialized per row. The other tables'
  masks are still the evaluator's (they are dimensions).
- **Aggregations** (`aggregate_storage`): a filter context splits into a **base** and **grouping
  filters**: single-code filters (a cell, a context transition, `T[c] = v`) and small code sets
  (up to 8,192 codes: time intelligence, slicers) on columns that map to the scanned table's rows:
  its own columns, or a dimension column reached by **exactly one** many-to-one path in the
  context's relationship state (the traversal mirrors `mask_from`; duplicate keys on the one side
  refuse the split). A filter that does not reach the scanned table is dropped (it changes
  nothing). One grouped scan under the base computes the aggregation for every combination of the
  grouping columns' codes; it is cached in the evaluation (key: aggregation, column, base filters
  by content hash, relationship overrides, grouping columns), and each request reads one group,
  or folds the groups of its code sets (sums, counts, min / max, bitmap unions, histogram sums).
  Exactness: the filters are a conjunction, and through a many-to-one path a fact row is visible
  under `code ∈ S` exactly when its related row's code is in `S` (unmatched rows: the blank row,
  code 0). Aggregations reproduce `aggregate` exactly: Currency and whole-number sums are exact
  integers (whole numbers only when every sum stays exact in a double); real-number sums and
  `MEDIAN` / `STDEV` / `VAR` use per-group histograms (the reference's own arithmetic) when they
  fit in 4 Mi slots, else real-number sums are compensated (within a few ulps of the reference,
  whose own order is arbitrary) and the others take the reference path.
- **Whole-column aggregations** (no filter, no grouping): sums, counts, min / max and distinct
  counts by one parallel scan; the aggregations that need every value (`MEDIAN`, `STDEV`, `VAR`,
  real-number sums) from the rows per code of the whole column, counted once (RLE runs one step
  each) and kept with the column, which then answer any aggregation of it in O(distinct values).
- **Direct paths**: a `SUMMARIZECOLUMNS` item, an `ADDCOLUMNS` item or an iterator body with
  context transition (`SUMX(VALUES(T[c]), [M])`, `RANKX(ALL(T[c]), [M])`) that is one aggregation
  (`AGG(T[c])`, `COUNTROWS(T)`, or a measure whose expression is one, without calculation groups)
  is computed for all its combinations or rows from one grouped scan, without a filter context per
  cell. `RANKX` and `ALL` tables are memoized in the evaluation.
- **Row iterators over a fact** (`FILTER(T, …)`, `SUMX(T, …)`, `AVERAGEX`, `MINX`, `MAXX`,
  `COUNTX`): when the expression reads only columns of `T` (and `RELATED` columns, through their
  foreign key), literals and pure scalar functions, it is evaluated once per distinct combination
  of those columns' codes (in a row that has it) and weighted by its rows: `SUMX(Ventes,
  Ventes[Quantite] * RELATED(Produit[PrixCatalogue]))` is 10 × 1,000 evaluations instead of N.
  `SUMX(T, …)` and `SUMX(FILTER(T, p), …)` over a model table (with `p` reading at most one
  column) never list the rows: the visible rows stay a selection, `p` is evaluated once per code
  of its column and joins the selection as an allowed-code bitmap, then one grouped scan counts
  the rows of each combination of the body's columns.
  The predicate is read on the first row of each code of its column (`first_rows`), visible or
  not, since only the visible rows are kept afterwards; an error there falls back to the visible
  codes alone.
- **Several sums of one table** (`prefetch_sums`): the direct items of a `SUMMARIZECOLUMNS` that
  sum compressed columns of the same table under the same filters and grouping are computed in
  one pass (`sum_by_id_multi`: the selection and the groups once per block).
- **Counted groupings** (`count_grouping`): `COUNTROWS(SUMMARIZECOLUMNS(T[g], "x", SUM(T[c])))`
  (or `COUNTROWS(T)` as the item, a measure that is one of them, under `CALCULATETABLE` or not)
  is the number of distinct values of `T[g]` over the visible rows whose `T[c]` is not blank: one
  distinct-count scan, the grouping never built.
- **`RANKX(ALL(T[c]…), body)`** (`rank_values`), when the body transitions the row context (a
  measure, `CALCULATE`): the values ranked among are computed once, sorted, and every row's rank
  is a binary search. The context transition replaces the filters on the ranked columns, so they
  are left out of the key, and the table serves every row of the grouping.
- **Runs**: a block whose predicate column lies in one RLE run of one id is judged on that id
  (kept or dropped whole), a block whose grouping columns lie in runs has one group, and the sum
  kernels then take their loop without mask or groups (`Packed::run_through`). A grouping by one
  column whose block lies in runs (a store sorted within the segment) sums each run in a tight
  loop and adds it to the run's group (`Packed::runs_in`, `Plan::group_runs`), and a block of a
  value column in one run adds its value times the selected rows. A whole-column sum of a column
  mostly made of runs reads its rows per code (one step per run). Sums of values read through a
  table (hash-encoded columns) accumulate in an `i64` when the sum over every row of the table
  cannot exceed 2^62, else in an `i128`.
- **Groupings** (`combos_storage`), **visible rows**, **distinct values**, `ALL`: one parallel
  scan each, in the reference's order (first seen first).
- **`TOPN`** (`preview_eval_topn.rs`) reads its order keys from the table's columns when they are
  columns. The first key is read once into a typed array (exact Currency units, doubles for
  numbers and dates, booleans; text falls back to generic values); its `n`-th value bounds the
  candidates (a row after it on the first key is after the `n`-th row on every key), and only
  those few are compared on every key. Ties at the boundary are all kept (more than `n` rows),
  in sorted order, ties by position. A key of 256 Ki rows or more is read and bounded by shares
  on several threads: each share keeps its `n` best values in a buffer of at most `2n` whose
  worst kept value rejects the others ("Column-store techniques").
- **`TOPN(n, SUMMARIZECOLUMNS(…), keys…)`** (Desktop's table visuals) when the grouping has only
  group-by columns, filter tables and items that are one aggregation each, and every key is an
  item (`[name]`) or a group-by column: the groups and the items' values come from the grouped
  scans as `SUMMARIZECOLUMNS` computes them, the groups whose items are all blank are dropped,
  the selection runs on them, and **only the kept rows are built** (the grouping's table is not
  built or kept). When the grouping's table is already kept (another query built it), the
  generic path reads it. A query variable bound to a `SUMMARIZECOLUMNS` (Desktop's `VAR
  __DS0Core = SUMMARIZECOLUMNS(…)`) is evaluated on first use, at the query's top level, so a
  `TOPN` over it takes the same path; any other use evaluates it once.
- **One group-by column without filter** (`combos_storage`): a compressed column without a blank
  row holds every code `1..` (codes are numbered from the distinct values its rows meet), so its
  groups are listed without counting the rows per code.

### Grouped row iterators (`preview_eval_iterx.rs`, 2026-11-04)

The DataFusion test oracle (`exp/datafusion`, `docs/experiments/datafusion.md` on that branch,
use 4) found one slow shape: an iterator written as an item of a grouping,
`SUMMARIZECOLUMNS(Produit[Produit], "X", SUMX(Ventes, Ventes[Quantite] *
RELATED(Produit[PrixCatalogue])))`, took seconds at 10 M rows where the same `SUMX` without the
grouping (Q16) took milliseconds. The value was right. An item that is not one aggregation was
evaluated cell by cell. Each cell's iterator took the row-iterator path above, which costs one
scan of the visible rows, so 1,000 products cost 1,000 scans.

Now such an item (`SUMX`, `AVERAGEX`, `MINX`, `MAXX`, `COUNTX`, `COUNTAX` over a model table or
`FILTER(T, p)` with `p` reading one column, inline, in a measure, or under `CALCULATE` with
boolean filters `T[c] op constant` on tables that hold no grouping column) is computed for every
cell by one grouped scan:

- **The key.** It holds the grouping columns (their lookup tables from the fact's columns, as the
  grouped aggregations use them) and the columns the body reads. Parts that read the same fact
  column are **fused** into that column's own codes when it has fewer codes than their product.
  `Produit[Produit]` and `RELATED(Produit[PrixCatalogue])` both read `Ventes[ProduitKey]`, so the
  key has 1,000 groups instead of 1,000 × 1,000. Every part's code is then a lookup from the
  foreign key's code: no hashing per row.
- **Exact products** (`grouped_products`). `SUMX`, `AVERAGEX` and `COUNTX` of a product of columns
  of the table and `RELATED` columns holding whole numbers or Currency, with at most one Currency
  factor (Currency × Currency is a double). It applies when n × the product of the factors'
  largest absolute values stays under 2^52 units, so every product and partial sum is exact.
  One factor of the table is summed by the grouped sum kernel (cached as any grouped scan, see
  "Caches between queries"); with no such factor, the rows are counted. The other factors are
  read once per group, and each cell adds its groups' sums times their factors in `i128`. The
  reference multiplies and adds the same integers exactly (doubles under 2^53, or Currency
  units), so the values are the same: Currency stays Currency, AVERAGEX of Currency rounds half
  away from zero, a blank factor (a blank value, or a missing related row) makes the product
  blank and it is skipped, and a cell with no product is blank. Past the bound, the next path
  runs. Overflows are then reported by the cells one by one, as the reference reports them.
- **Any other row expression** (`grouped_weighted`). One tally gives the rows and the first row
  of each (cell, body combination). The body is evaluated once per combination met. Each cell
  aggregates its combinations, weighted by their rows, in the order of their first rows,
  exactly as the per-cell path does (`weighted`). The results are the same bits as before, which
  the tests check.
- **Not taken** (the cells one by one, unchanged): a relationship override in the context
  (`USERELATIONSHIP`, `CROSSFILTER`), a calculation group, a `CALCULATE` filter of another form or
  on a grouping column's table, a key over 2^22 groups or more than 2^22 cells, and any
  evaluation error, so that the error the reference reports is the one returned.

Measured with the micro-bench (`storage_micro`, the bench's star schema, sales columns
compressed in segments of 65,536 rows, 20 threads, release build), as A/B runs of the build
before and after this change, two interleaved rounds under `bench-quiet`. Cold = `dax.clear_cache`
before each of 3 runs, median; the speed-up is before / after. Warm answers were already kept
results (under 0.6 ms before and after).

| Query (cold) | 1 M before | 1 M after | Speed-up | 10 M before | 10 M after | Speed-up |
|---|---:|---:|---:|---:|---:|---:|
| O10 `SUMX(Ventes, Ventes[Quantite] * RELATED(Produit[PrixCatalogue]))` by product | 619 ms | 1.4 ms | ×440 | 3,237 ms | 4.2 ms | ×780 |
| `AVERAGEX` of the same and `COUNTX(Ventes, Ventes[Quantite])` by category × region | 193 ms | 3.3 ms | ×59 | 638 ms | 11.0 ms | ×58 |
| a measure that is the same `SUMX`, by month | 44 ms | 6.0 ms | ×7.3 | 226 ms | 14.3 ms | ×16 |
| `MAXX` of the same product, by product (weighted path) | 769 ms | 12.2 ms | ×63 | 3,586 ms | 14.9 ms | ×241 |
| `SUMX(FILTER(Ventes, Ventes[Quantite] >= 5), …)` by region, two categories | 7.2 ms | 2.8 ms | ×2.5 | 41.5 ms | 6.7 ms | ×6.2 |
| a double body (`… * 1.1 * RELATED(…)`), by product (weighted path) | 780 ms | 12.6 ms | ×62 | 3,624 ms | 16.0 ms | ×227 |
| E12 (`SUMX(FILTER(…))` and `SUMX(Ventes, Ventes[Quantite] * 2)` by category) | 12.6 ms | 2.0 ms | ×6.5 | 63.8 ms | 6.9 ms | ×9.3 |

The 22 bench queries and the other edge cases did not change: every cold median is within the
noise of the two rounds (±25 % at these sub-10 ms times), at 1 M and at 10 M, worst case
included (Q17 at 10 M: 101 ms before, 105 ms after). E15 is still slow (7 s at 10 M): one of its
items is `Ventes[Montant] * RELATED(Produit[PrixCatalogue])`, a Currency × Currency product
(a double), and the amount has too many distinct values for the tally, so its cells are still
computed one by one. 100 M rows were not measured: the micro-bench builds its data in memory as
values (about 20 GB at 100 M).

Tests (`preview_eval_iterx_tests.rs`): random queries over a random snowflake
(`grouped_iterators_match_the_reference_evaluator`: unknown and blank keys, blank values, Currency,
whole numbers and doubles, a chain of many-to-one relationships, filters on the fact and on the
dimensions, `FILTER` sources, `CALCULATE` filters, measures, `USERELATIONSHIP` over an inactive
relationship, text bodies that fail; `BICRAFT_FUZZ_SEED`, `BICRAFT_FUZZ_CASES`). Each query is run
by the reference evaluator, then by the storage engine without this path, then with it, cold and
warm. With the path, the answers are the same bits as without it, and equal to the reference
(doubles within 1e-9, since compensated sums differ from a sum row by row). Other tests check
that each shape takes its path and that overrides and filters on a grouping column's table do
not (`grouped_iterator_shapes_take_their_paths`), that Currency values near the top of the range
overflow as the reference does (`grouped_iterators_overflow_as_the_reference`), and that new
dimension data, `dax.clear_cache` and a relationship made inactive give fresh results
(`grouped_iterators_follow_new_data_and_model_edits`). No new cache was added: the path reads the
grouped scans and lookup tables the session already keeps.

## Caches between queries

The grouped scans (aggregation, column, base filters by content, relationship overrides,
grouping columns), the lookup tables of many-to-one paths and the `ALL` tables only depend on the
model and its data: the session keeps them from one evaluation to the next (`dax.evaluate`, the
viewer's visuals), as Analysis Services' storage engine keeps its cache, keyed by the model's
hash and the data's generation. A query that defines its own columns (`DEFINE COLUMN`) does not
share its results.

Results kept **by expression text** (iterators such as `SUMX`, `SUMMARIZECOLUMNS` tables of pure
expressions, counted groupings, the values `RANKX` ranks among) carry in their key a fingerprint
of every measure of the evaluation (a query may redefine one), the filter context by content and
the relationship overrides, and are kept only when the expression and the measures it uses are
**pure**: no clock (`TODAY`, `NOW`…), no random value, no user, no `ALLSELECTED`, no calculation
item function. An impure expression is recomputed at each query.

`dax.clear_cache` empties every one of these caches (not the relationship indexes below) and the per-column tables of the rows (see
"Data"), keeping the data open, its columns read and their dictionary tables: the next query runs
as Analysis Services does after `ClearCache` (what the bench calls *cold*).

### Relationship indexes and per-model setup (`preview_eval_rels.rs`)

A relationship's code map gives, for each code of one end, the code of the other end that holds
the same key. Building it hashes every key of the dimension, so it costs O(dimension rows). Until
2026-10-24 every query rebuilt it, warm queries included, together with the blank-row flag (does
some key on the many side have no row on the one side?). That cost does not show on the bench's
small dimensions, but on a 2 M-row customer table it dominated every warm query. Analysis
Services builds its relationship indexes when it processes the data, and `ClearCache` keeps
them. The session now does the same:

- **What is kept.** The code map of each relationship end, whether some non-blank key of it is
  unmatched, and whether a column holds the blank code. That last flag is also what a table's
  unique-column test reads, which used to unpack and scan a whole column on every query.
- **How long.** They are built on first use and kept by warm queries. `dax.clear_cache` keeps
  them too, as it keeps the decoded columns, so they count as a precomputation in the cold
  regime, exactly as Desktop's relationship indexes do.
- **Invalidation, at two levels.** First, the whole set is dropped when the model hash or the
  data generation changes: a model edit, a relationship edit (activation, ends, cardinality), a
  culture change, a refresh or another source. Second, each entry holds the two decoded columns
  it was built from, checked by identity (the entry keeps them alive, so their address can never
  be reused by other data), plus the culture whose text identity matches the keys. Data that the
  session does not share, such as a calculation group's rows or a query's own columns, never
  matches an entry. The set is capped at 4,096 entries.
- **Measures.** What an evaluation reads from the text of every measure is kept per model hash:
  the measures fingerprint in the keys of text-keyed results, and whether some measure uses
  `ALLSELECTED`. This holds only when the evaluation runs on the session's own model. A query
  with `DEFINE MEASURE` computes these facts from its own model.
- **No copy of the model.** `dax.evaluate` copies the model only when the query defines
  measures. Otherwise it evaluates on the session's model directly.

Tests (`preview_eval_rels_tests.rs`) compare each case against a fresh session holding the same
model and data, and against the reference evaluator:

- the indexes are built once and kept by warm queries and by `dax.clear_cache`;
- they are rebuilt after a measure edit and after a culture change;
- new data in the fact only, the dimension only, or both, including a blank row that appears or
  disappears, never gets a stale result;
- a relationship that is deactivated, moved to another column or made many-to-many is followed;
- the measure facts follow a measure added with `ALLSELECTED` and then removed.

Measured on 2026-10-25. The micro-bench is `rel_micro`, release build, with a 2 M-row customer
dimension and 4 M sales. Every run is a new query (a new selection), and runs with the indexes
kept alternate with runs where they are rebuilt each time. Warm medians:

| Query | Rebuilt per query | Kept | Speed-up |
|---|---:|---:|---:|
| `SUMMARIZECOLUMNS(Client[Segment], "CA", [CA])` | 1,025 ms | 0.23 ms | ×4,400 |
| `CALCULATE([CA], Client[ClientKey] <= n)` (n differs each time) | 1,462 ms | 501 ms | ×2.9 |
| `CALCULATE([CA], Client[Segment] = …)`, `VALUES`, `TREATAS` | 0.03–3.4 ms | 0.03–3.4 ms | ×1.0 (already answered from kept results) |

The 501 ms left on the range filter were the filter itself: a table of the 2 M key values, the
predicate per row, then every kept value looked up again by key. It is now compiled and read from
codes (see "Big dimensions, skewed data and per-query costs" below).

The engine bench (3 interleaved rounds against the previous build) is unchanged within noise at
1 M and 10 M: its dimensions are small, and cold times gain a little. All 22 queries give the
same results as Desktop. These figures use a release build of the bench client. With a debug
client, Q05 at 1 M (2,000 rows returned) takes ~21 ms of wall time, of which 5–6 ms is on the
server; most of the rest is the debug client parsing the JSON.

### Big dimensions, skewed data and per-query costs (`preview_eval_pred.rs`, 2026-10-26)

The bench's dimensions are small and its keys uniform, which hid several costs that grow with a
dimension's rows or a model's size. Two bench variants expose them (`engine-bench gen --variant
bigdim|skew`, [benchmarks](benchmarks.md#big-dimension-and-skewed-data-2026-10-26)): a
`Client` dimension of millions of customers with attributes, and Zipf-like keys with a
high-cardinality text column in the fact. What was found and changed, each exact by
construction and checked against the reference evaluator:

- **`RELATED` searched the dimension.** The generic path found the row of the related key by a
  linear search of the dimension's key column, per call: `SUMX(Ventes, IF(RELATED(Client[Fidelite])
  = "Oui", Ventes[Montant]))` hit the work cap at 1 M sales and 200 k customers. It now reads the
  first row of the key's code from a table of the first row of every code of the column
  (`ColData::rows_of_codes`, one pass; a table of the rows: `dax.clear_cache` drops it, new data
  replaces it), the row the search found.
- **Iterators keyed by the related values.** A row iterator evaluated once per combination of the
  codes it reads used the foreign key's codes for a `RELATED` column (one group per customer). A
  foreign key read only through `RELATED` is now keyed by the codes of the related columns,
  through the many-to-one lookup table of the grouped scans (`body_parts`): three loyalty values
  instead of 2 M customers. `SUMX(T, IF(cond, T[a]))` with `a` Currency or whole numbers and
  `cond` reading few values goes further (`conditional_sum`): one grouped sum of `a` by the
  values `cond` reads, `cond` evaluated once per combination met, the sums where it holds added
  (the reference adds `a` on the same rows; the sums are exact).
- **Filters over one column's values.** `CALCULATE([CA], Client[ClientKey] <= n)` is
  `FILTER(ALL(Client[ClientKey]), …)`; Desktop's between slicer writes `VAR __DS0FilterTable =
  FILTER(KEEPFILTERS(VALUES(Client[ClientKey])), AND(… >= a, … <= b))`. The reference built a table
  of 2 M values, evaluated the predicate per row through a row context, then looked every kept
  value up again by its key to build the filter. Now a predicate made of the column, constants
  (pure expressions without columns, evaluated once), comparisons, `&&` / `||`, `AND` / `OR` /
  `NOT`, `IN { constants }`, and measures that are one aggregation (all values at once by one
  grouped scan, as each row's context transition gives them) is compiled into a small tree
  evaluated per value with the evaluator's own comparison and truth functions, on the worker pool
  (`filter_values`). As a `CALCULATE` or `SUMMARIZECOLUMNS` filter it goes straight from codes
  (`filter_codes_arg`): each code of the domain (`ALL`: every code, `VALUES` / `DISTINCT`: the
  visible codes) is kept when the predicate holds for its value; no table is built and no value
  is looked up by key. This equals the reference's filter whenever two codes never share a key,
  which is checked on the column's value index (otherwise the reference runs). A query variable
  bound to such a `FILTER` is left unevaluated until used (DAX variables are lazy); used as a
  filter, it is read from codes in the context it is defined in. Any evaluation error falls back
  to the reference, which reports it (or not, when it stops earlier) as before.
- **Tables of values remember their codes.** `VALUES(c)`, `ALL(c)` and a `FILTER` of them are
  built from codes; for a column whose values all have their own key, the evaluation records
  those codes with the table (by the address of its rows, kept alive, for the evaluation only;
  the `ALL` tables kept between queries keep theirs), and turning the table into a filter or a
  context transition over it reads them instead of looking each value up by key.
- **One code of a column visible.** `HASONEVALUE(Produit[Produit])` or `VALUES` in each cell of
  a grouping by that column scanned the table once per cell (the cell's filter is one code): with
  100,000 products, 100,000 scans. When every filter of the context is one code of the column
  itself, the answer is that code when some row holds it (`values_of_one_code`), without a scan.
- **Implied filters of a context transition over a key.** `AVERAGEX(VALUES(Client[ClientKey]),
  [CA])` in each segment's cell ran one grouped scan per segment. When the iterated column has
  one row per value and every iterated value's row passes the context's filters on its table,
  those filters select that one row either way: they are dropped from each row's context
  (`drop_implied_filters`), and one grouped scan serves every cell.
- **Distinct counts of many groups.** A grouped distinct count kept one bitmap of the column's
  codes per group: 100,000 customers × 3.3 M order numbers would need 41 GB, so it fell to the
  per-cell path (120 s at 1 M sales). Beyond 64 MiB of bitmaps, the groups' distinct codes are
  kept as sorted lists built from the (group, code) pairs met (`scan::DistinctPairs`,
  deduplicated while the scan runs, appended by range for radix-partitioned scans), whose size
  follows the rows, not groups × codes; counts, unions of groups and coarser groupings read them
  as they read the bitmaps.
- **The first query on a big dimension.** It built the relationship map of 2 M keys by hashing
  every key of both ends (~1.3 s at 10 M sales): whole-number keys within ±2^53 in a dense range
  (surrogate keys) now use a table indexed by key (`build_ints`, the same map as the hash: a test
  compares them), and a fresh process answers its first visual on the `bigdim` variant in
  0.35–0.45 s instead of 1.7 s. A column's sort ranks (what `MIN`, `MAX`, `TOPN` and orders read)
  sorted the dictionary with two copies of the values, and their lower-cased text, per comparison:
  the sort keys are now computed once and sorted with ties by position (the stable sort's order):
  `MAX(Ventes[ClientKey])` over 2 M keys 2.4 s → 0.39 s, over the standard 10 M file's 1 M keys
  0.97 s → 0.18 s.
- **Per query, on a large model** (200 extra tables of 20 columns, 2,000 measures,
  `query_overhead_micro`): the query was validated against a copy of the model holding the
  query's result and variable tables (2.6 ms for the copy alone), then parsed a second time,
  and the evaluator's registry of tables, columns and relationships was rebuilt (0.77 ms). Now a <!-- perf-guard: ok (BIcraft's own timings) -->
  query without `DEFINE TABLE`, `COLUMN` or `MEASURE` (Desktop's visual queries) is validated
  against the model with those tables beside it (`Names::Overlay` in `bicraft-dax`: the same
  diagnostics as against the copy, checked by a differential test), the parse comes back with
  the validation (`validate_query_parsed`), and the registry is kept by the session for its own
  model, by model hash (any edit builds it again; a query with its own measures builds its own;
  calculation groups, which hold parsed expressions, are still read per query).

  | On that model | Before | After |
  |---|---:|---:|
  | Validation of a card query | 2.59 ms | 0.07 ms |
  | Evaluator setup (`Engine::new`) | 0.77 ms | 0.12 ms |
  | `dax.evaluate` of `ROW("x", 1)` | 1.33 ms | 0.22 ms |
  | `dax.evaluate` of a card over a chain of 20 measures | 4.27 ms | 0.56 ms |

In-process timings (`variant_micro`, release with the fast profile, 4 M sales, 800,000 customers
or 100,000 skewed products, cold = `dax.clear_cache` before each run, median of 3; the machine was
shared, so ±20 %):

| Query | Before (cold) | After (cold) |
|---|---:|---:|
| B11 `SUMX` with `RELATED` to the customers | 6,394 ms (after the `RELATED` fix; refused before) | 48 ms |
| B07 `CALCULATE` with a range on the customer key | 458 ms | 21 ms <!-- perf-guard: ok (BIcraft before and after) --> |
| B06 Desktop's between slicer (variable) | 617 ms | 40 ms <!-- perf-guard: ok (BIcraft before and after; B06 is a query name) --> |
| B10 customers over a threshold (`FILTER(VALUES(key), [CA] > x)`) | 544 ms | 157 ms |
| B09 average per customer by segment | 639 ms | 231 ms |
| S10 table by customer with a distinct count of orders | 120,550 ms at 1 M | 239 ms |
| S12 `RANKX` over 100,000 products, `TOPN 501` | 10,802 ms at 1 M | 691 ms |

Tests (`preview_eval_bigdim_tests.rs`): every query of both variants, cold and warm, against the
reference evaluator on small data (`variant_*_matches_the_reference_evaluator`); the compiled
filters with every operator, blanks, measures, outer filters, lazy variables, conditional sums,
and the errors the reference reports or does not (`variant_compiled_filters_match_the_reference`);
distinct counts forced to sorted lists (`variant_sparse_distinct_counts_match_the_reference`,
and a kernel test against the bitmaps in `bicraft-scan`); `RELATED` over a dimension of 70,000
customers and the tables of values with codes after new dimension data that renumbers the codes
(`variant_related_over_a_big_dimension_follows_new_data`); the registry after a relationship made
inactive, then bidirectional, and a new column, against a fresh session
(`registry_follows_model_edits`); in `bicraft-dax`, validation with and without the copy of the
model on queries with variables, `ORDER BY`, `START AT`, query tables and measures, and errors
(`queries_validate_the_same_without_a_copy_of_the_model`).


## Aggregations

Automatic aggregations derived from the report (G32): small pre-grouped copies of a fact table,
chosen from what the report's visuals ask, that answer the storage engine's grouped scans in
place of the fact when they can do so exactly. Like Power BI's user-defined aggregations, but
derived, built and checked by BIcraft itself, with no table added to the model and nothing
written to any file.

### Derivation (`crates/engine/src/aggs_derive.rs`)

A deterministic function of the report and the model (no data, no randomness, no agent). For
each visual of each page:

- **Grouping columns**: the visual's bound columns (any role).
- **Filter columns**: the columns of the report's, the page's and the visual's filters (with or
  without a condition: a filter card can be set later), and of every slicer of the same page.
- **Measures**: the bound measures (model and report measures) and implicit aggregations, with
  their dependency closure (measures referenced by measures, to a depth of 32). In a measure's
  DAX:
  - a column under an aggregation function (`SUM`, `AVERAGE`, `MIN`, `MAX`, `COUNT`, `COUNTA`,
    `COUNTBLANK`, `DISTINCTCOUNT`, …) is a **value column** of its table, which becomes a
    **fact**; `COUNTROWS(T)` makes `T` a fact; `DISTINCTCOUNT(T[c])` also makes `c` a grain
    column (a distinct count is only derivable when the column is in the grain);
  - a row iterator over a table (`SUMX(T, …)`, `AVERAGEX`, `MINX`, `MAXX`, `COUNTX`,
    `FILTER(T, …)`) makes `T` a fact; its own columns read in the body are value columns, columns
    of other tables (`RELATED`) and its `FILTER` predicate's columns are grain columns;
  - a column read anywhere else (a `CALCULATE` filter argument, `VALUES(c)` iterated by
    `SUMX` / `AVERAGEX` / `RANKX`, `ALL(c)` as an iterated table, `TREATAS`) is a grain column; a
    column only removed (`ALL`, `REMOVEFILTERS`, `ALLEXCEPT`, `ALLSELECTED` as `CALCULATE`
    modifiers) is not;
  - the date column of a time-intelligence function (`TOTALYTD`, `DATESYTD`,
    `SAMEPERIODLASTYEAR`, `DATEADD`, `PARALLELPERIOD`, `PREVIOUS…`, `NEXT…`) is a grain column:
    the date sets those functions produce are aligned on days.

A visual's **requirement** for a fact `T` is its grain columns that reach `T` (a column of `T`,
or of a table at the one end of a chain of many-to-one relationships from `T`), plus its value
columns of `T`. A column whose filter does not reach `T` is dropped (it changes nothing); a
column reached by several paths leaves the visual uncovered for `T`.

In the engine, each grain column of a dimension is expressed on the fact column it is reached
through (the foreign key): the **class** of a code of that fact column is the combination of
the codes of the grain columns reached through it (`Ventes[ProduitKey]` → (`Categorie`,
`SousCategorie`)); a grain column of `T` itself, or the key the relationship points to, makes
the class the code itself. An **aggregation** is a grain (for each grain fact column, its class
map) plus, per row (one non-empty combination of classes): the fact's rows, and for each value
column its non-blank count, its exact sum (Currency and whole-number columns whose sums the
storage engine computes exactly) and its lowest and highest value.

### Selection: the size rule

- Facts under **100,000 rows** get no aggregation (a scan is already sub-millisecond).
- A candidate's size is **estimated** before it is built: `P · (1 − e^(−N/P))` rows, `P` the
  product of its class counts and `N` the fact's rows (the expected number of distinct
  combinations of independent uniform keys); its exact size is known once built.
- **Each aggregation: at most N / 10 rows** (at least ten times smaller than the fact), and at
  most 2^24 combinations of classes (it is built by one dense grouped scan per statistic).
- **All aggregations of a fact: at most N / 4 rows together, and at most 8 of them.**
- Candidates: every visual's grain, the union of every pair of them and the union of all.
  Greedy choice (Harinarayan, Rajaraman and Ullman, SIGMOD 1996): at each step the candidate of
  largest benefit, the benefit being the sum over the visuals it covers of the rows saved (rows
  of the smallest chosen aggregation covering the visual, or of the fact, minus its own); ties by
  size, then by name; until the budget, 8 aggregations, or no candidate saves rows.
- An aggregation covers a visual when it has every grain fact column of the visual with the same
  grain columns or more (or the fact column itself).

The result is a small hierarchy: a coarse aggregation is built from the smallest finer one
already built (its rows folded), the finest from the fact.

### Planning (`crates/engine/src/preview_eval_aggs.rs`)

Every grouped scan of the storage engine (`compute_with`: the cached aggregations behind
`SUMMARIZECOLUMNS` cells, iterators with context transition, `RANKX`, `SUMX(FILTER(T, p), c)`,
`SUMX(T, a * RELATED(b))`, prefetched sums) first asks the aggregations of its fact. A scan is
**covered** by an aggregation when:

1. it selects rows only by allowed-code bitmaps over fact columns (no row mask), each over a grain
   fact column and **constant on each class** (all the codes of a class allowed, or none);
2. each grouping column maps from a grain fact column through a code table that is **constant on
   each class**;
3. the aggregation is derivable: `COUNTROWS`, `COUNT`, `COUNTBLANK`, `MIN`, `MAX`, `SUM` and
   `AVERAGE` (exact sum and count: not real-number columns); `DISTINCTCOUNT` only of a grain fact
   column whose class is its code; never `MEDIAN`, `STDEV`, `VAR`;
4. its columns are the columns the aggregation was built from (same model column, same
   dictionary, same data generation).

The smallest covering aggregation answers; otherwise the fact is scanned, as before. The check is
made on the request's own code tables, so it holds whatever the filters, the relationships
activated (`USERELATIONSHIP`, directions) or the time-intelligence date sets: a set of dates is
covered by a month grain exactly when it is a union of whole months. Exactness: the rows a
covered request selects, per group, are unions of aggregation rows, and counts, exact integer
sums, minimums and maximums of a union are those of its parts; a distinct count of a grain column
whose class is its code counts the classes present. The answer is the entry the scan would have
produced (same sums, counts, ranks, bitmaps), so everything above it (folding of code sets,
totals, caches) is unchanged. A whole-column sum without filter or grouping keeps the segments'
statistics (faster).

### Building and keeping

- **Never on a query's path.** A query never plans nor builds aggregations: the first storage
  request over a fact of at least 100,000 rows that has none notes the fact, the query is answered
  from the fact as without aggregations, and when its evaluation ends a **background job** starts:
  on its own thread, over the same decoded data (the backup and columns already open are shared,
  the columns it decodes are handed to the session), it derives the plan and builds every
  aggregation, the finest first, with a quarter of the machine's threads (`BICRAFT_AGG_THREADS`,
  on a background worker pool of their own, so a query's scans never wait for a worker behind the
  job's)
  and at most 1 GiB of aggregations per fact, and only while the session is idle (no evaluation
  in flight and none ended in the last 200 ms; checked before each of its scans, so a query that
  arrives waits for at most one scan of the job). Queries keep scanning the fact until the job is
  done; the next query then installs its result (a non-blocking check) and uses it. A job is
  cancelled as soon as its key is out of date (new data, model or report: its result is dropped,
  never installed). After `model.refresh`, the first query over the new data starts the job.
  `dax.aggregations {"build": true}` plans and builds them at once in the foreground (the bench
  and the tests).
- Data: those the session reads (PBIX `DataModel`, PBIP `cache.abf`, BIcraft's own snapshot).
- **Never written**: not into `cache.abf`, the PBIX, the project or `.pbi/bicraft-data.bin` (its
  format has no versioned section to carry them, and a new version would make older snapshots
  unreadable). They live in the session's memory, as the decoded columns do.
- **Keyed by** the data's generation, the model's hash and the hash of the report's derived
  requirements: new data (a refresh, a new `cache.abf`), any model edit (a measure, a column, a
  relationship) drops them all, and they are derived again. A **report edit** that changes what
  visuals need (a field, a filter, a slicer, a page) keeps the model and data: the requirements
  are derived again and the selection re-run, and every built aggregation whose (fact, grain,
  aggregated columns) is selected again is **kept** (the same table, not rebuilt; the pool lives
  in the state of one model hash and data generation, so only an exact match is reused); only the
  new ones are built (by the background job, or at once by `dax.aggregations {"build": true}`),
  and those no longer selected are dropped. At 10 M rows, adding one visual with a new grain to
  the bench report: 5 of 8 kept, 3 built, 0.75 s instead of 1.55 s (median of 3; 0.57 s of it is
  the re-run selection, which recomputes its class maps). `dax.aggregations` reports
  `built_tables`, `reused_tables`, `rebuild_ms` and, per aggregation, its `id` and `reused`. The columns are recognized by model column and
  dictionary size: the same model and data generation decode the same codes (decoding is
  deterministic), so an aggregation built by a job serves the columns a query decoded itself.
- `dax.clear_cache` keeps them: they are data, like Power BI's aggregation tables, and the bench
  publishes them as a precomputation, with their size and build time.
- `dax.aggregations` reports the derivation (requirements per visual, the grains chosen, sizes,
  build time, requests answered) and turns them off or on for the session (`{"enabled": false}`);
  `BICRAFT_AGGREGATIONS=0` turns them off by default.
- **Free without aggregations.** `BICRAFT_AGGREGATIONS`, `BICRAFT_AGG_MIN_ROWS` and
  `BICRAFT_AGG_SYNC` are read once per process (the session's settings, which tests set, come
  first); each evaluation resolves the smallest fact with aggregations once (`usize::MAX` when
  off), and a storage request over a smaller fact returns before any other work.
  `Session::background_work` takes no lock and synchronizes nothing unless a flag says it has work
  (a query wanted aggregations, `dax.aggregations` ran, or the session changed while it has
  aggregations); the model hash is computed lazily (when a query or a job needs it), streamed into
  the hasher. Measured on 1 M rows, fresh sessions (the first query of a newly opened file, 10
  sessions per query, 3 rounds): with and without the bench report applied first (56 report
  commands), before and after, the four cells are within run-to-run noise (about ±5 %); the
  synchronization the report commands triggered costs about 0.1 ms in all (`background_work_is_free_without_aggregations`
  counts locks, synchronizations and model hashes).
### Measured

On the engine bench (`docs/benchmarks.md`, "With and without report-derived aggregations"), with
a bench report of one page per query: 8 aggregations per size, 14–18 % of the fact's rows; the
cold median is about twice as fast with them at 1 M and about nine times as fast at 100 M, start
and warm unchanged, 22/22 identical. Tests: `aggregations_*` in `preview_eval_aggs_tests.rs` (derivation,
exactness on the bench's shapes, random differential test against the fact cold and warm, one
invalidation test per change, the background job, on / off, aggregations kept across report
changes in the foreground and by the job) and the real-file trial
(`BICRAFT_AGG_TRIAL=<project.pbip>`, ignored by default). `BICRAFT_AGG_SYNC=1` (with
`BICRAFT_AGG_MIN_ROWS=0`) builds them in the foreground on first use, to run the oracles and the
other tests through them.

## Column-store techniques (2026-11-19)

Five well-known column-store techniques were tried on BIcraft's kernels. Each was measured before and after on
`groups_micro` (`cargo test --release -p bicraft-scan groups_micro -- --ignored --nocapture`,
10 M rows, 1 M customers, 20 threads) and kept only when measurably faster. **Measured** on a
shared machine (other builds running, CPU near 100 %): medians of interleaved A/B rounds of the
same test binaries; run-to-run noise is about ±15 %. The 100 M tier and the full engine bench
were not run under that load.

- **Radix buffers reused (kept).** A radix-partitioned
  scan (`scan_groups_in`) routed every selected row at once into per-partition vectors grown by
  doubling: up to 8 bytes per row (800 MB at 100 M rows), faulted in and copied as they grew. It
  now routes by windows of at most 128 MiB (about 8 Mi rows; `ROUTED_DEFAULT_BYTES`), each
  partition's vector first sized for an even share of a worker's rows (plus a quarter), and the
  buffers of one window are cleared and reused by the next. The result does not change (the
  sinks are order-insensitive, as with the windows of `max_routed`), and the routed memory of a
  scan is bounded whatever the rows. Measured, integer sums by 1 M customers at 10 M rows:
  107 → 66 ms (median of 3 rounds; ×1.6 faster).
- **Parallel merge of the workers' states (dropped).** Tried: below the 8 MiB radix limit, the workers' states
  merged by ranges of groups on the pool, each range folded in worker order (the same bits for
  double sums, checked by a test over every sink). Measured, counts by 100 k groups: 9.8 →
  10.0 ms; by 900 k groups: 90 → 110 ms (medians of 3 interleaved rounds, within the noise, no
  gain). The merge is not where the time goes: `threads_for` already gives a large per-worker
  state few workers (two at 900 k groups over 10 M rows), so there are few states to merge.
  Giving those scans every worker once the merge is parallel was slower too (900 k groups:
  48 → 60 ms, 3 rounds): 20 states of 7 MiB do not stay in cache. Reverted.
- **Skipping without the 65,536-id cap, and zone maps (kept).** Segment skipping tested "no allowed id in
  the segment's id range" by walking the range, so it was off for ranges of 65,536 ids or more
  (a key column such as `ClientKey`). A plan now packs the allowed-id table into bits with the
  allowed ids before each word (a prefix sum): the test is constant-time whatever the range.
  And `Packed::new` computes, in the pass that already reads each segment for its statistics, a
  **zone map**: the lowest and highest data id of each 64 Ki rows of a segment (`ZONE_ROWS`, one
  morsel); a block in a zone that no allowed id reaches is skipped like a dead segment (columns
  read on demand have no zone map: segment skipping only). Results are unchanged (a zone is
  skipped only when none of its ids is allowed; the zone bounds are checked against the rows
  in `zone_maps_skip_exactly_whatever_the_id_range`). Measured, 10 M rows, 3 rounds: one store
  of a fact sorted by quantity then store in each 1 Mi-row segment, 1.30 → 1.08 ms (×1.2
  faster); a range of 1 % of the customers on a fact in customer order (segments of 100 k ids),
  2.08 → 1.42 ms (×1.5 faster); the two-predicate case unchanged (3.6 ms). Nothing changes on
  data whose ids are spread over every zone, which is the standard bench (rows in random order;
  the bench was not rerun).
- **Predicate order and early exit (kept).** The most selective condition is read first, and the other columns only for the rows
  left.
  `Plan::new` now orders the predicates by the fraction of the column's codes they allow (fewest
  first; the exact rows per code are not kept, since `clear_cache` drops them), and
  `Plan::block` first makes the checks that read no codes (dead segment or zone, a block
  inside one RLE run), then unpacks the remaining predicates one at a time and stops at the
  first one after which no row of the block is left. Results are unchanged
  (`predicates_in_any_order_select_the_same_rows`). Measured, 10 M rows, 3 interleaved rounds:
  a 90 % predicate then a 10 % one on columns in runs, rows sorted, 2.07 → 0.88 ms (median,
  ×2.3 faster; rounds 2.07/2.75/1.28 → 1.14/0.70/0.88 ms); the unsorted cases (counts by
  store, with a mask) within the noise (5.2 → 6.0 ms and 5.5 → 5.2 ms).
- **Parallel top-k with a threshold (kept).** Each thread keeps the best rows seen so
  far and skips a row worse than the worst of them. Profiled first: Q17 warm (`TOPN 501` over 1 M customers,
  10 M rows) spent about 15 ms of 40 ms in the selection: reading the first key into a typed
  array (8 ms) and the candidate bound, a copy of the million values and one selection over
  them (8 ms); the grouped scan took most of the rest. Now a key of at least 256 Ki rows is
  read by shares on the scan threads (`TopKey::read`, one share per thread, at least 128 Ki
  rows each), and `bound` keeps the `n` best values of each share in a buffer of at most
  `2n` (the `n`-th of the buffer rejects the others with one comparison), takes the `n`-th
  of what the shares kept, then lists each share's positions not after it, in position order;
  a worker that fails has its share computed again on the calling thread. Results are
  unchanged (`topn_bound_on_threads_matches_one_full_selection` against one full selection on
  1 to 64 threads, `topn_long_key_read_by_shares_is_the_whole_key`, and the existing TOPN
  tests). Measured, 10 M rows, 3 interleaved rounds of 3 warm runs: the candidate bound 8.2 →
  2.4 ms (×3.4 faster), the key read 7.9 → 6.3 ms, the selection 15.2 → 9.1 ms (×1.7 faster),
  Q17 warm 44.1 → 35.2 ms (median, ×1.25 faster; rounds 44.1/55.0/39.0 → 33.3/44.2/35.2 ms).
  The 100 M tier was not run (the machine was shared).

## Tests

- `storage_engine_matches_*`: the 22 bench queries and the edge cases (E01–E14) against the
  reference evaluator, on clean and dirty data (unknown keys, blanks), compressed and in memory,
  and on sales sorted as VertiPaq sorts a segment (long runs, RLE).
- `storage_fuzz_matches_the_reference_evaluator`: random queries (shapes of the bench, filters,
  `CALCULATE` modifiers, `USERELATIONSHIP`, a bidirectional relationship, orphan keys, negative
  and blank values, rollups, counted groupings), each run cold and warm against the reference;
  deterministic by seed (`BICRAFT_FUZZ_SEED`, `BICRAFT_FUZZ_CASES` for more cases; a failure
  prints the seed and the query shrunk to its smallest failing form).
- `storage_results_are_the_same_bits_on_any_number_of_threads`: 1, 2, 8 and 20 threads, double
  sums included (fixed reduction order).
- `pool_*` (scan crate): results in task order, a panicking task reported as a failure while the
  pool keeps working, nested loops, and the background job's loops kept off the foreground
  workers (checked by worker thread name).
- `storage_caches_follow_the_model_and_the_data`, `storage_caches_follow_the_clock_and_allselected`,
  `storage_engine_keeps_no_result_across_redefined_measures`: a model edit, new data,
  `dax.clear_cache`, another session, a redefined measure, a moved clock and another outer
  selection under `ALLSELECTED` all give fresh results.
- `storage_currency_sums_are_exact_and_overflows_fail_as_the_reference`, and the scan crate's
  encoding edges (blocks and segments boundaries, runs across blocks, mixed encodings, negative
  bases, blanks, empty segments).

What they found: text-keyed caches that ignored a redefined measure; an expression cloned for
analysis whose columns resolved by a reused address (the wrong column); iterator results kept
across a change of the clock.

## Micro-bench

The development loop runs in seconds on synthetic data built in memory, without Desktop, CSV
or files:

- kernels alone, rows per second and thread scaling:
  `cargo test --release -p bicraft-scan micro -- --ignored --nocapture` (`BICRAFT_MICRO_ROWS`,
  default 1 M; `BICRAFT_SCAN_THREADS`, `BICRAFT_MICRO_THREADS`, and the threshold overrides
  `BICRAFT_SCAN_ROWS_PER_THREAD` / `BICRAFT_SCAN_PARALLEL_MIN_ROWS`);
- the grouped-sum kernel on the bench's shapes (Q03, Q04, Q05, filtered, a value table), 1 and
  all threads: `cargo test --release -p bicraft-scan kernel_micro -- --ignored --nocapture`
  (`BICRAFT_MICRO_ROWS`, default 10 M; warm-up and at least 0.3 s per case, or the first cases
  are timed on parked cores);
- the worker pool: wake cost against scoped threads, and a foreground scan's latency under a
  background loop: `cargo test --release -p bicraft-scan pool_ -- --ignored --nocapture`;
- the engine bench's star schema (same generator as `xtask/src/engine_bench.rs`), columns put
  straight into the preview cache, the sales columns kept compressed as VertiPaq segments of
  65,536 rows (alternately bit-packed and hybrid RLE; `BICRAFT_MICRO_PLAIN=1`: in memory):
  exactness of the storage engine against the reference evaluator on the 22 bench queries plus
  edge cases, clean and dirty data (unknown keys, blanks), compressed and in memory:
  `cargo test -p bicraft-engine storage_engine`; time per query, first run, *cold* (caches
  emptied by `dax.clear_cache` before each of 3 runs, median) and *warm* (median of 3), with the
  rows of compressed columns an evaluator path had to unpack:
  `cargo test --release -p bicraft-engine storage_micro -- --ignored --nocapture`
  (`BICRAFT_MICRO_ROWS`, `BICRAFT_MICRO_REF_ROWS` for the reference's timings,
  `BICRAFT_MICRO_ONLY=Q05,Q13`, `BICRAFT_MICRO_QUERY` for other queries, `;;` between them;
  `BICRAFT_MICRO_SORT=1` sorts the sales of each million rows by quantity then store, as
  VertiPaq orders a segment, which gives the long runs and RLE segments of a real file).
- the bench variants (a big customer dimension, skewed keys), every column compressed, with the
  reference's answer compared on small data:
  `cargo test --release -p bicraft-engine variant_micro -- --ignored --nocapture`
  (`BICRAFT_MICRO_VARIANT=bigdim|skew`, `BICRAFT_MICRO_ROWS`, `BICRAFT_MICRO_DIM_ROWS`,
  `BICRAFT_MICRO_PRODUCTS`, `BICRAFT_MICRO_REF_ROWS`, `BICRAFT_MICRO_ONLY`, `BICRAFT_MICRO_QUERY`);
- what a query costs before it reads data on a large model (validation, evaluator setup):
  `cargo test --release -p bicraft-engine query_overhead_micro -- --ignored --nocapture`
  (`BICRAFT_MICRO_TABLES`, `BICRAFT_MICRO_MEASURES`).

The full engine bench on real files (`docs/benchmarks.md`) runs only at milestones.

## Beyond memory

Designed and staged in [`docs/beyond-ram.md`](beyond-ram.md). Lot 1 is a **memory budget**
(`BICRAFT_MEMORY_BUDGET`, or `dax.memory {budget}` per session; default unlimited, nothing
changes): under it the decompressed backup moves to a temporary file read by positional reads,
decoded columns are kept within the budget (least recently used first, never one a query is
using), the storage engine's caches are capped at a quarter of it and dropped first, and a column
that cannot fit is refused with a clear error. Lot 2: under a budget the `DataModel` part stays in
the package file and is decompressed as a stream (read-ahead, workers across pieces, parallel
positional writes) straight to the temporary file, from which the model and the column index are
read; nothing whole is held at open. Lot 3: a column over half the budget (or over the budget once
decoded) is read segment by segment: `Packed` reads a segment through one accessor, from the column
or from a `bicraft_scan::lazy::Lazy` that reads it from the backup through the session's bounded
segment cache, so every kernel works unchanged and the codes are those of the column read whole.

Lot 4 bounds **one query's memory by bytes** (`dax.memory {query_budget}`, `BICRAFT_QUERY_BUDGET`,
by default the memory budget): a meter per evaluation (`preview_eval_mem.rs`) reserves the big
allocations before they are made (groupings, value vectors, scan states, masks, materialized
tables, sort keys, answer rows) and counts every large table an expression returns while it lives;
over the cap the query is refused with an error naming the part. Exact strategies keep answers
within it: groupings are flat (4 bytes per code); `TOPN` over a large grouping (and an `ORDER BY`
cut by `max_rows`) goes block by block through a bounded heap of the best keys; radix-partitioned
scans route their rows by windows (`scan_groups_within`); scans use fewer threads when their
states would not fit. Lot 5 is a **process-wide limit** (`BICRAFT_MEMORY_LIMIT`, crate
`bicraft-mem`): kept columns and caches, query memory, background builds and decompression
buffers all reserve against one pool before allocating; a reservation that does not fit evicts
first (the session's own caches and columns, then other sessions'), then is refused cleanly;
running queries share the limit fairly; aggregation builds yield. Default: no budget, no limit,
nothing counted.

## BIcraft's own data snapshot

`model.refresh` (Databricks and other sources BIcraft refreshes itself, `crates/engine/src/refresh.rs`)
writes `.pbi/bicraft-data.bin`: rows as values (`bicraft_data::store`), read back into columns
of one code per row. To run the same engine on it, the snapshot should store each column as
BIcraft's VertiPaq encoder writes it (`crates/vertipaq/src/encode`, already used to write PBIX
data): dictionary or value encoding, bit-packed and RLE segments with their statistics, then
read it into `Packed` columns exactly as a backup is. Not implemented yet: the snapshot keeps its
row format, and its columns use the in-memory path.

## Results

On the engine bench (`docs/benchmarks.md`, "Scaling from 10 k to 100 M rows"), 2026-10-24 evening (commits up to `7912181` for 50 M and 100 M, `23952fd` for the other sizes):
22/22 queries return Desktop's results at every size from 10 k to 100 M rows, and none is
refused (before the storage engine: up to 8 refused at 50 M, all 22 at 100 M). The cold gains of
Q01 and Q02 come from the segments' statistics (a precomputation read with the column, see
"Data"), not from scanning faster. Peak memory at 100 M is 6.6 GB, most of it the formula
engine's one row per customer (Q17, Q18). Later measurements, with absolute times per tier, are
in `docs/benchmarks.md`.

## Limits

| Limit | Value |
|---|---|
| Grouping combinations of one `SUMMARIZECOLUMNS` (`dax.evaluate`) | 50 M |
| Row visits of one query (`dax.evaluate`; a scan counts its rows once) | 2 × 10^11 |
| Groups of one grouped scan | 2^26, and 512 MiB of state |
| Histogram slots | 4 Mi |
| Distinct counts of one grouped scan kept as bitmaps (beyond: sorted lists of the codes met) | 64 MiB |
| Code sets folded as grouping filters | 8,192 codes per column, 16,384 groups per request |
| Cached aggregations (kept between queries) | 2 GiB (emptied beyond) |
| Segment skipping | any id range; zones of 64 Ki rows (zone maps), not for segments read on demand |
| Decompressed backup in memory | 8 GiB |
| Table rows | 2 × 10^9 (32-bit codes and row numbers) |
| One query's formula-engine memory | `query_budget` (default: the memory budget; none without one) |
| Process memory (reserved allocations) | `BICRAFT_MEMORY_LIMIT` (default: none) |
