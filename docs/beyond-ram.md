# Beyond RAM

Goal: BIcraft opens and queries models whose data does not fit in memory, never returns a value
different from Desktop's, and never crashes. Today the 100 M-row bench model peaks at 5.4 GB
(6.9 GB with report-derived aggregations) on a 31 GB machine whose free memory varies between
2.6 and 15 GB. This note lists where the memory goes,
what can be dropped, the options, the staged plan, and what lots 1 to 5, the groupings by ranges and the re-read work do
(`docs/storage-engine.md` describes the engine itself).

## Where memory goes today

Per session, for data read from a VertiPaq backup (PBIX `DataModel`, PBIP `cache.abf`):

| # | What | Size | Lifetime |
|---|---|---|---|
| 1 | The package: every PBIX part in memory, the compressed `DataModel` included (parts over 4 GiB stay in the file, read by range; under a budget the `DataModel` stays in the file, lot 2) | the file (10 M: 91 MB) | the open file (saves write it back) |
| 2 | The decompressed backup (`VertipaqStore`), held whole | 10 M: 109 MB; 100 M: about 1.1 GB | from the first query to the next data change |
| 3 | Decoded columns (`State.columns`): the segments' words copied out of the backup (`Packed`, about the compressed size), the data id → code table (4 bytes per id), the dictionary (`Vec<CellValue>`: 32 bytes per distinct value plus text) | 10 M: 6 fact columns about 100 MB; `ClientKey` (1 M distinct) alone about 60 MB | until the next data change |
| 3a | `Codes::plain`: one code per row, unpacked once when an evaluator path needs it | 4 bytes per row (10 M: 40 MB per column) | with the column |
| 3b | Per-column derived tables: dictionary tables (value index, numbers, ranks, Currency and whole-number values) and row tables (rows per code, first rows) | 40–90 bytes per distinct value | with the column; `dax.clear_cache` drops the row tables |
| 4 | Relationship indexes (`RelIndexes`): a code map per relationship end; each entry holds its two columns | 4 bytes per code of the many side's key | kept by warm queries and `dax.clear_cache` |
| 5 | Storage-engine caches (`Store`): grouped scans, lookup tables, `ALL` tables, kept `SUMMARIZECOLUMNS` tables, iterator results, `RANKX` values | at most 2 GiB of grouped scans (emptied beyond), 40 M kept cells | between queries, until `dax.clear_cache` or a change |
| 6 | Report-derived aggregations: pre-grouped copies of a fact; each holds its grain and value columns | at most 1 GiB per fact (100 M: 1.31 GB in all) | kept by `dax.clear_cache` |
| 7 | Viewer results (JSON per visual) | at most 20,000 results | between snapshots |
| 8 | One query's working set: the columns it reads (held by the evaluation), formula-engine tables (Q17 / Q18: one row per customer), grouped scans' states and routed rows, the answer's JSON | bounded by row visits (2 × 10^11) and combinations (50 M); by bytes under a query cap (lot 4) or the process' limit (lot 5) | the query |

At 100 M rows the peak is (1) about 0.9 GB, (2) about 1.1 GB, (3) about 1 GB of segments plus
the dictionaries of the 10 M customers, and (8) the formula engine's tables of Q17 and Q18
(the largest single item, `docs/storage-engine.md` "Beyond memory").

## What can be dropped

| Kind | Items | How it comes back | Cost to bring back (10 M) |
|---|---|---|---|
| **Recomputed** | storage caches (5), viewer results (7), row tables (3b), `Codes::plain` (3a), relationship indexes (4), aggregations (6) | the next query computes them again, as after `dax.clear_cache` | a few ms per grouped scan; aggregations: the background job (2.2 s) |
| **Re-read** | decoded columns (3) | decoded again from the backup. Decoding is deterministic: the same backup gives the same first-seen codes, so every cache keyed by column and code (5, 6) stays valid. Caches keyed by the column's identity (4) are rebuilt | 14 ms (`Montant`), 34 ms (`ClientKey`) |
| **Re-read from disk** | the backup (2) | from a temporary file, by positional reads; or decompressed again from the package part | a few positional reads per column (page cache: memory speed) |
| **Left in the file** (lot 2) | the package's `DataModel` part (1), under a budget | read from the package file when a save, a convert or a reopened store needs it | one sequential read (a save or convert reads it whole, as before) |

## Options

- **Lazy column decode.** Already the case: a column is decoded the first time a query reads it,
  and only the columns a query reads are decoded.
- **Evicting decoded columns under a budget** (LRU by bytes). A column a query holds is
  **pinned** by that reference (`Arc` count above the cache's own): the evaluator keeps each
  column it reads for the whole evaluation, so a column is never dropped under a running scan,
  and memory is freed only when nobody uses it. A re-read column gives the same codes, so cached
  results stay valid; caches that recognize a column by its address are purged with it, so they
  can neither serve stale data nor keep the evicted column alive.
- **Operating on compressed segments.** Already the case (`bicraft-scan` scans VertiPaq
  segments a block at a time). The leftover is `Codes::plain` (4 bytes per row), used by a few
  evaluator paths: it is dropped first, without dropping the column, when the column is not in
  use.
- **The backup on disk.** Memory-mapping it would be the obvious route, but every mmap crate
  (`memmap2`, `mmap-rs`) needs `unsafe` at the call site (the file may change under the map),
  and the workspace forbids `unsafe`. Positional reads (`FileExt::seek_read` on Windows,
  `read_exact_at` on Unix, both safe `std` APIs, already used by `bicraft-opc` to read large
  parts) read exactly a column's files: its `.idf` segments, `.idfmeta` and dictionary, a few MB
  at a time. Columns are copied out of the backup when decoded anyway, so a column read from the
  file costs one extra copy of its compressed bytes, and the OS page cache keeps hot files at
  memory speed without counting against the process.
- **Streaming decompression to the file.** `file.open` decompresses the whole `DataModel` in
  memory to read `metadata.sqlitedb`. A multi-threaded `DataModel` is a list of independent
  chunks that can be decoded a few at a time and appended to the file; a single-threaded one is
  one XPress9 session whose matches reach 4 MiB back, so it decodes through a sliding window.
  Then neither the backup nor its compressed part needs to be whole in memory (lot 2, with the
  package part left in the file).
- **Segments read on demand.** A column's segments become (file, offset, length, statistics)
  records; a scan reads the segments its predicates do not skip and keeps a bounded LRU of
  segments. That lifts the last per-column bound (one column must fit in the budget) but changes
  how kernels borrow columns (`Col<'_>` borrows the column for the scan): lot 3.
- **Spilling intermediate results.** The scan kernels' state is already bounded (512 MiB per
  grouped scan, radix partitions). The formula engine's tables of one row per group (`TOPN` over
  10 M customers) are the largest consumer at 100 M: a columnar result (one vector per column
  instead of one per row) and streaming when `max_rows` cuts the answer would bound them (lot 4).
- **What Analysis Services does** (public documentation only): VertiPaq is an in-memory engine.
  The server's `VertiPaqPagingPolicy` lets column data be paged to disk through the system's
  paging file when memory runs short (`VertiPaqMemoryLimit` sets where paging starts), and Power BI
  Premium's large semantic model format loads columns on demand on first use and evicts them under
  memory pressure. Lot 1 is the same idea at the level of whole columns, with an explicit budget
  instead of the operating system's paging.

## Plan

1. **Lot 1 (done, below): a memory budget.** `BICRAFT_MEMORY_BUDGET`, or `dax.memory {budget}`
   per session; default unlimited (nothing changes). Under a budget: the backup goes to a
   temporary file and its columns are read by positional reads; decoded columns are kept within
   the budget (LRU, pinned while a query uses them); storage caches are capped and dropped first;
   a column that cannot fit is refused with a clear error before it is read.
2. **Lot 2 (done, below): nothing whole at open.** Streaming XPress9 decompression from the
   package file to the temporary file; the `DataModel` part left in the file under a budget (as
   parts over 4 GiB already are); `metadata.sqlitedb` read from the file.
3. **Lot 3 (done, below): segments on demand.** Columns over half the budget scanned segment by
   segment from the file through a bounded cache of segments. Left: aggregations still hold their
   columns (pinned while they exist), and a column's values (its dictionary) must fit.
4. **Lot 4 (done, below): one query's memory by bytes.** A per-query cap (the budget by default);
   big allocations reserved before they are made, large tables counted while they live; `TOPN`
   over a large grouping and `ORDER BY` cut by `max_rows` block by block through a bounded heap;
   radix scans routed by windows of rows; what still does not fit refused with a clear error.
5. **Lot 5 (done, below): a guaranteed ceiling.** `BICRAFT_MEMORY_LIMIT`: one pool for the process,
   every big allocation reserved against it first; eviction (own session, then the others), then
   a clean refusal; fair shares between running queries; background builds yield.
6. **Groupings by ranges (done, below).** A `TOPN` or a cut `ORDER BY` over a grouping whose scan
   states do not fit goes a range of groups at a time, one bounded scan per range, through the
   same bounded heap; refused only when one group's range cannot fit.

## Lot 1: the memory budget

### Setting

- `BICRAFT_MEMORY_BUDGET`: bytes, with an optional `K`, `M`, `G` or `T` suffix (binary: `300M` is
  300 MiB); `0`, `unlimited` or unset: no budget. Read once per process through `bicraft-env`.
  An unreadable value is ignored (no budget) rather than refusing to start.
- `dax.memory {budget?}`: sets the budget for the session (a number of bytes, a string with a
  suffix, or `null` for the process default) and reports what the session holds (below).
- `BICRAFT_SPILL_DIR`: where the backup's temporary file goes (default: the system's temporary
  folder). On Linux, a `/tmp` mounted as `tmpfs` lives in memory: point this at a disk folder.

### What it bounds

**Counted** memory is what the session keeps between queries: decoded columns (segments, id
table, dictionary, `Codes::plain`, derived tables) and the storage engine's caches (grouped
scans and kept tables). After each column read and at the end of each evaluation, while counted
memory exceeds the budget, in this order:

1. the storage engine's caches are dropped (recomputed by the next query, as after
   `dax.clear_cache`) when they hold more than an eighth of the budget; a smaller cache is dropped
   last (see "Re-reads below the working set");
2. columns not used by a running evaluation drop their `Codes::plain` copy;
3. columns not in use are evicted (least recently used first until lot 5; see "Re-reads below the
   working set" for the order now), with the relationship-index entries that recognize them by
   address (rebuilt on next use).

Not counted: the package (1), the running query's own tables (8), aggregations' own tables
(capped instead at a quarter of the budget per fact, at most 1 GiB), and the relationship
indexes and viewer results (small, and purged or count-limited).

**A column that cannot fit** is refused with a clear error naming it, its size and the budget,
before any of it is read (its size estimated from its files in the backup: segments plus
dictionary), or after decoding when its decoded size is over the budget. The query fails cleanly;
the session keeps working. **One query's working set** (the columns it reads together) may exceed
the budget: they are all pinned while it runs and released at its end. The budget therefore
bounds what is kept and each column a query reads, not the peak of one query (lots 3 and 4).

**The backup under a budget** is written to a temporary file (`bicraft-backup-<pid>-<n>.abf`)
once the first query over it is answered: by the session's background work (a thread started
after the answer is sent, as for aggregation jobs), else at the start of the next evaluation, so
the first answer does not wait for the write (at 10 M rows writing 109 MB took about 120 ms on
Windows, which doubled the start time when done before the answer). Reads go on while the file is
written; only the switch takes a lock, and every holder of the store (an aggregation job's
included) switches with it. Columns are then read from the file by positional reads. The file is
removed with the last store holding it: on Windows it is opened delete-on-close (the system
removes it even after a crash), on Unix it is unlinked as soon as it is written. A file that
cannot be written leaves the backup in memory (still correct; `dax.memory` reports the error).

Not under the budget: tables loaded from partitions (M queries, DAX literal tables) and BIcraft's
own data snapshot, which have no backup to read a column from again; they stay in memory as
before.

### What it does not change

- Results: eviction only drops what is computed again from the same data, and a re-read column
  gives the same codes; caches keyed by address are purged with it (tests below).
- The default: without a budget no file is written, nothing is evicted, and the only new work is
  one comparison per column read and per evaluation.

### Tests

`crates/engine/src/memory_budget_tests.rs`, on a star of our own (6,000-row fact, 50-row
dimension, a relationship) saved as a PBIX by BIcraft:

- `answers_under_a_tiny_budget_are_the_answers_without_one`: nine queries (sums, distinct counts,
  groupings through the relationship, `CALCULATE` filters, `TOPN`, `SUMX` with `RELATED`,
  `FILTER`) under a budget of 1.25 × the largest column, three rounds with `dax.clear_cache`
  between them, equal to a session without a budget; columns were evicted, the kept columns stay
  under the budget, the backup is in its file, moved by background work after the first answer.
- `an_evicted_column_is_not_kept_alive_and_comes_back_the_same`: weak references to every column
  read; a one-byte budget evicts them all and none survives (the relationship indexes let go of
  them), then the same answers under a large budget.
- `a_column_over_the_budget_is_refused_and_the_session_goes_on`: a 1 KiB budget gives an error
  naming the column and the budget; without a budget the same session answers.
- `a_spilled_backup_reads_the_same_columns_and_its_file_goes_with_it`: every column read from the
  file equals the one read from memory (values and codes); clones share the file; the folder is
  empty once the last store is dropped.
- `bad_budgets_are_refused`, and the parser's unit test (`300M`, `2g`, `64KiB`, `0`,
  `unlimited`, overflow, garbage).

### Measured (2026-10-25, Windows, release, engine bench files)

**Same answers.** The 22 bench queries run twice in one session (`bicraft-cli run`, 44 answers)
under each budget, compared with a session without one: **0 differences** at every budget.

| Rows | Budget | Evictions | Kept at the end (columns + caches) | Largest single query's working set | Backup |
|---|---|---:|---:|---:|---|
| 1 M | none | 0 | 23.5 MB | | 16.4 MB in memory |
| 1 M | 16 MiB | 67 (and 23 unpacked copies, 4 cache drops) | 14.7 MB | 21.5 MB | file |
| 1 M | 8 MiB | | | | refused: `Ventes[Montant]` needs 8.4 MB |
| 10 M | none | 0 | 146 MB (121 MB of columns) | | 109 MB in memory |
| 10 M | 300 or 150 MiB | 0 (it fits) | 146 MB | | file |
| 10 M | 80 MiB | 73 | 65 MB | 121 MB | file |
| 10 M | 60 MiB | 117 | 52 MB | 114 MB | file |

**Time and process peak** (`engine-bench bicraft` on the 10 M file, one round each, 22/22 identical
to Desktop):

| Budget | Peak working set | Start (median) |
|---|---:|---|
| none | 526 MB | 0.114 s |
| 300 MiB | 424 MB | 0.125 s |
| 80 MiB | 390 MB | 0.121 s |

- The peak drops by the backup (−100 MB). Under 80 MiB, Q17 (the table by customer) re-reads
  `ClientKey` on each warm run (96 → 238 ms); the other warm and cold times are within noise.
- The rest of the peak under a budget: the package's `DataModel` part (91 MB) and the whole
  backup decompressed by `file.open` until it moves to its file (109 MB) — lot 2 — and one
  query's working set (121 MB, Q17: `ClientKey` and its tables) — lots 3 and 4.
- Before the move was deferred to after the answer, the start median under a budget was 0.236 s.

### Left after lot 1

- `file.open` decompressed the whole `DataModel` and the package held the part: lot 2.
- A column larger than the budget was refused: lot 3.

## Lot 2: nothing whole at open

### What changes under a budget

- **The `DataModel` stays in the package file.** `file.open` reads the package with
  `Package::open_deferring(path, limits, ["DataModel"])` (`bicraft-opc`): the part is recorded
  like a part over 4 GiB (offset, sizes, CRC-32) and its bytes are not read. `Part::bytes()` reads
  it when a save, a convert or `table.import {dax}` needs the whole part (CRC checked; a file that
  changed under the package is an error, never other bytes), so saves and converts work as before
  (`write_model` now accepts a deferred part up to the 4 GiB part cap). Saves copy it from the file
  as they already copied parts over 4 GiB.
- **The backup is decompressed as a stream to its temporary file**
  (`bicraft_xpress9::decompress_datamodel_into`), then the model and the column index are read
  from the file by positional reads (`bicraft_pbix::backup_index_at`: header page, virtual
  directory, BackupLog, `metadata.sqlitedb`). The store opened this way is handed to the first query
  (`seed_store`), whose backup is already in its file: nothing moves after the answer. A store
  opened later (after a save, a budget set after the open) streams the same way; a PBIP
  `cache.abf` too.
- **The decoder runs in three stages at once.** A thread reads the part 8 MiB of compressed bytes
  ahead; the workers decode jobs of about 1 MiB from one piece into the next (no pause between
  pieces: a piece's plan carries the XPress9 session state, signature, block numbering and first
  byte, to the next, `dec_fast::plan_from`), at most 2 × threads jobs ahead of the calling thread,
  which finalises them in order against the 4 MiB window; three threads write the finished jobs
  at their offsets (positional writes, so their order does not matter). Memory held: two pieces,
  the jobs in flight, the window and 8 queued jobs, whatever the part's size (at 10 M about
  60 MB, against 91 MB of part and 109 MB of backup before). The bytes and errors are the whole-part
  decoder's: when the fast path cannot vouch for a piece, the part is read again from the start by
  the reference streaming decoder, which overwrites the same offsets.
- **The temporary file** is created delete-on-close and `FILE_ATTRIBUTE_TEMPORARY` on Windows (the
  cache manager keeps it in memory rather than writing it out when it can), unlinked at once on
  Unix, and removed on every error path (the file is owned by the store being built: an error
  drops it).
- Without a budget nothing changes: the whole part in memory, the parallel decode, the backup
  handed to the first query.

### Tests

`crates/xpress9/src/stream.rs`: the streamed decode equals the whole-part decoder for the Exact
and Fast single-threaded layouts and the multithreaded layout, at 1 and 4 threads, pieces from 1
byte to 64 MiB; same errors for truncated and corrupted parts; limits refused from the declared
sizes; a sink error stops the decode and is returned. `crates/opc`: parts deferred by choice read
back and write the same archive; a changed file is an error. `crates/engine/src/memory_budget_tests.rs`:

- `a_budget_at_open_streams_the_backup_to_its_file_and_answers_the_same`: budget set before
  `file.open`, the part deferred, the nine answers equal twice, backup in its file, a model edit
  saved (the part read from the file) and reopened with the same answers, the spill folder empty
  once the session is dropped.
- `a_streamed_backup_reads_the_same_columns_and_errors_leave_no_file`: every column equal to the
  in-memory store's; corrupted, truncated, garbage and over-the-limit parts and a missing folder
  are errors that leave no file.
- `a_broken_datamodel_under_a_budget_fails_cleanly`: a corrupted `DataModel` opened under a budget
  reports a model error as without one, queries fail, no file left.
- `bench_queries_under_budgets_equal_the_unlimited_answers` (ignored, local bench data): the 22
  bench queries twice per session under each budget, set before `file.open`, against a session
  without a budget.

### Measured (2026-10-25, Windows, release, engine bench files)

**Same answers:** 22 queries × 2 per session at 10 M under 300 / 150 / 80 / 60 MiB: 0 differences.

| Rows | Budget | Peak working set | `file.open` (fresh) |
|---|---|---:|---:|
| 10 M | none | 520 MiB | 0.055 s |
| 10 M | 300 MiB | 333 MiB (lot 1: 404 MiB) | 0.105 s |
| 10 M | 80 MiB | 306 MiB (lot 1: 372 MiB) | 0.103 s |
| 1 M | none | 75 MiB | 0.025 s |
| 1 M | 16 MiB | 47 MiB | 0.035 s |

- 22/22 identical to Desktop in every run.
- The default path is unchanged (start within noise).
- Under a budget the start pays the streaming (0.105 s against 0.055 s at 10 M): the stream decodes
  at about 80 ms for 109 MB (null sink), the whole-part decoder at 55–60 ms; the rest is writing
  the file. Lot 1 deferred its write after the first answer but held part and backup; lot 2
  trades about 50 ms of start for about 70 MiB less peak.
- Under 16 MiB at 1 M and 80 MiB at 10 M, Q17 re-reads `ClientKey` on every warm run (warm 38 ms
  at 1 M): the budget is below that query's working set.

## Lot 3: segments on demand

### What changes under a budget

- **Columns read segment by segment.** A column is read segment by segment when its files
  (segments and dictionary) are over half the budget, and also when it was read whole and its
  decoded size (segments, id table, values) turns out to be over the budget. The reader is
  `VertipaqStore::column_lazy` in `crates/engine/src/preview_store_lazy.rs`. Such a column keeps:
  - its values;
  - its data id → code table;
  - for each segment, its rows, its exact id range and its statistics (`bicraft_scan::SegStats`).

  The segments' data is read from the backup when a scan reaches them.
  `bicraft_vertipaq::idf::locate_idf` finds where each segment's blocks are without reading the
  column: it reads the size words of the `.idf` by positional reads and makes the same checks as
  `split_idf`.
- **First read of such a column.** Its segments are read once, in batches of at most half the
  budget (several segments at a time, on the pool). Each batch gives the segments' statistics and
  their first-seen data ids; a bitmap of the ids already met carries from one batch to the next.
  Then the batch is dropped.
  - The codes are the ones the column gets when read whole (same first-seen order).
  - So every cache keyed by column and code stays valid.
- **Scans read segments through the session's segment cache** (`bicraft_scan::lazy`: one `Lazy`
  per column, one `SegCache` per session).
  - `Packed` reads a segment's data through a single accessor: from the column, or through
    `Lazy::get`. So every kernel and every evaluator path works unchanged.
  - Each thread remembers its last segment without keeping it alive (a scan reads a segment's
    rows in a row).
  - The cache keeps at most half the budget, least recently used out first (scan-resistant
    since "Re-reads below the working set").
  - A segment a scan holds stays alive through its `Arc`: eviction drops only the cache's
    reference.
  - Each column read on demand has an identity of its own. A column read again gets a new
    identity, so no segment of an earlier read is ever served. A dropped or evicted column removes
    its segments from the cache.
- **Budget accounting.**
  - The segment cache's bytes (at most half the budget) are set apart (since "Re-reads below
    the working set": only up to an eighth of the budget; above that it keeps what the columns
    leave).
  - The decoded columns share the rest, LRU as in lot 1.
  - Storage caches are still dropped first.
  - The segment cache is no longer dropped: it is shrunk to what the columns leave.
- **Never a wrong answer from a failed read.** A segment that cannot be read is counted
  (`load_failures`) and reads as blank ids. `dax.evaluate` and the viewer compare the count before
  and after each evaluation. If it moved, the evaluation fails with "a column segment could not be
  read from the backup's temporary file", and the storage caches are dropped.
- **Clear refusals, never an allocation that cannot fit.**
  - A dictionary file over the budget is refused before it is read.
  - A segment over half the budget is refused before any segment is read: `column 'T'[C]: a
    segment needs 2.5 MB, over what the memory budget leaves for one segment (2.0 MB; ...)`.
  - Values and id table over the budget are refused after reading.
  - The error is not kept: the same session answers under a larger budget.
- **Reporting.** `dax.memory` reports:
  - `lazy_columns`, `lazy_reads`;
  - `segment_cache {bytes, cap, segments, loads, hits, evictions}`;
  - `segment_load_failures`.

### Tests

`crates/scan/src/lazy.rs`:
- A column read on demand gives the same results as the column in memory, for several segment
  sizes: codes, ids, counts, runs, id sums, and grouped filtered scans on 4 threads.
- The cache keeps to its bound, and evicted segments come back the same.
- A dropped column's segments leave the cache; a column read again reads its segments again.
- A failed read is counted, reads as blank, and is not kept.

`crates/engine/src/memory_budget_tests.rs`:

- `a_column_read_segment_by_segment_gives_the_same_codes`: every column read segment by segment,
  from memory and from the file, has the same values, codes and blank flag as when read whole. A
  segment over the room left is refused before it is read.
- `answers_with_columns_read_segment_by_segment_are_the_same`:
  - seven queries run three rounds, with `dax.clear_cache` between rounds, under a budget where
    the largest column is read segment by segment;
  - the answers equal those without a budget;
  - a one-byte budget then evicts every column, and the segment cache empties;
  - after that, the same answers again.
- `a_budget_below_one_segment_is_a_clear_error`, with a 64-byte budget:
  - every query is refused with a message naming the budget;
  - nothing is kept;
  - the session answers once the budget is lifted;
  - the spill folder is empty.
- The bench differential (above), at budgets down to below one segment.

### Measured (2026-10-25, Windows, release, engine bench files)

**Same answers.** 22 queries × 2 per session, against a session without a budget:

| Rows | Budget | Differences | Refused (of 44) | Time for the 44 answers | Columns read segment by segment |
|---|---|---:|---:|---:|---:|
| 1 M | 16 MiB | 0 | 0 | 0.21 s (none: 0.12 s) | 0 |
| 1 M | 8 / 6 MiB | 0 | 0 (lot 1 at 8 MiB: refused) | 1.3 s | `Montant` |
| 1 M | 4 MiB | 0 | 40: a 2.5 MB segment over the 2 MiB room | | |
| 10 M | 300 / 80 MiB | 0 | 0 | 0.5 / 1.0 s (none: 0.49 s) | 0 |
| 10 M | 50 MiB | 0 | 0 (lot 1: `ClientKey`, 54.7 MB decoded, refused) | 4.0 s | 1 |
| 10 M | 30 MiB | 0 | 0 | 4.7 s | 2 |
| 10 M | 16 MiB | 0 | 8 (Q06, Q07, Q17, Q18: `ClientKey`'s 1 M values and id table need 26.7 MB) | 10.3 s | 3 |

- No answer differs at any budget.
- What does not fit is refused with a clear error, never with an out-of-memory.
- The largest decoded column at 10 M is `ClientKey`, 54.7 MB with its tables. Every budget down
  to 30 MiB answers all 22 queries.

**Time and process peak** (`engine-bench bicraft`, one round each, 22/22 identical to Desktop):

| Rows | Budget | Peak working set |
|---|---|---:|
| 10 M | none | 527 MiB |
| 10 M | 300 MiB | 331 MiB |
| 10 M | 80 MiB | 303 MiB |
| 10 M | 50 MiB | 268 MiB |
| 10 M | 30 MiB | 260 MiB |
| 1 M | none | 75 MiB |
| 1 M | 16 MiB | 47 MiB |
| 1 M | 8 MiB | 71 MiB |

- The default path is unchanged.
- **Below the working set, answers stay correct but are much slower.**
  - At 50 and 30 MiB at 10 M, the queries over `ProduitKey` and `Montant` take 120–160 ms, warm
    and cold, against a few ms without a budget.
  - The reason: the columns these queries need together do not fit, so each query reads one of
    them again in a full pass (every segment read, first-seen ids, values decoded).
  - The peak still holds one query's working set (Q17's table by customer) and the transient
    decoded segments of the scan's threads.
  - At 1 M under 8 MiB, `Montant`'s values (6 MB) leave little room for the other columns.
  - These budgets trade time for memory. Above the working set (80 MiB at 10 M, 16 MiB at 1 M),
    nothing is slower than with lot 2.

### Extrapolation to 100 M rows (not regenerated for this work)

At 100 M rows:
- the backup is about 1.1 GB and the part about 0.9 GB;
- `ClientKey` has 10 M distinct values;
- fact segments hold 1 M rows: about 2.5 MB each for the 20-bit columns, 100 segments per column.

Expected behaviour:
- **Open under a budget.** Streamed at the 10 M rate (about 1.1 GB/s), so about 1 s for 1.1 GB. It
  holds about 60 MB instead of part + backup (2 GB). Without a budget: about 0.6 s, as now.
- **Peak under a budget.**
  - Lot 1 measured 5.4 GB at 100 M without a budget.
  - Lot 2 removes the part and the backup (about 2 GB).
  - The decoded columns follow the budget.
  - What remains is one query's working set: Q17 and Q18's formula-engine tables by customer
    (10 M rows, the largest item, lot 4), plus `ClientKey`'s values and id table, about 270 MB
    (10 M × 27 bytes), which must fit.
- **Budgets.**
  - About 1–2 GB should answer the 22 queries at the speed of an unbounded session (above the
    working set, like 80 MiB at 10 M).
  - About 300–500 MB should still answer them all, much more slowly (like 30 MiB at 10 M).
  - Under about 280 MB, `ClientKey`'s values do not fit: Q06, Q07, Q17 and Q18 are refused
    cleanly.
  - Under 5 MB (two segments), every fact column is refused.
- To be measured at the next milestone, once the 100 M data is regenerated.

### Left after lot 3

- One query's formula-engine tables were bounded by row visits, not by bytes (lot 4, below).
- A column's values (its dictionary) must fit in memory. Possible fixes:
  - a high-cardinality column could keep its values on disk (sorted dictionaries read by range);
  - an affine column could skip them.
- Aggregations still hold their columns (pinned while they exist).
- Under a budget below the working set, two changes would cut the re-reads (the first done in
  "Re-reads below the working set", the second in part):
  - a scan-resistant segment cache (under LRU, repeated scans larger than the cache get no hits);
  - keeping the results of the first pass over a read-on-demand column.

## Lot 4: one query's memory

### Setting

- `dax.memory {query_budget}` (bytes, a string with a suffix, `0` for no cap, `null` for the
  process default) or `BICRAFT_QUERY_BUDGET`; **by default the memory budget** (no budget: no
  cap, nothing counted). `dax.memory` reports `query {cap, cap_source, last_peak_bytes,
  peak_bytes, refused}`.
- So under a budget B the session keeps at most about B between queries, and one query's own
  working memory is held to B as well.

### What it counts

One evaluation's formula-engine memory (`crates/engine/src/preview_eval_mem.rs`), by bytes:

- **Reserved before it is allocated**, where the size is known first:
  - a grouping's combinations (4 bytes per code, now one flat vector instead of a vector per
    combination: a grouping by customer held 10 M small vectors at 100 M rows);
  - the value vectors of a `SUMMARIZECOLUMNS` and its rows, by blocks of 4,096 before they are
    built;
  - each grouped scan's states: the workers' sinks, `sum_by_id`'s per-worker accumulators, the
    ordered float sums' per-morsel states, and the radix path's routed rows and partitions;
  - row masks, the visible rows of a table, the rows of a table materialized as values;
  - `ORDER BY` keys and order, and the answer's rows (kept until the query ends);
  - `TOPN`'s heap, its blocks and its candidates (below).
- **Counted when built**: every table an expression returns, when over 256 KiB, is held by the
  meter through a weak reference (by address, once) and released when its last reference goes.
  Nothing big is allocated between building it and counting it.
- A grouped scan's result kept in the storage cache is charged to the query until it ends, then
  counted with what the session keeps.

Over the cap the query fails with an error naming the part and the cap, before the allocation:
`the query needs more memory than its cap: the grouped scan of a Sum (999961 groups) needs
84.8 MB with 3.8 MB already held by its other tables, over the query's cap of 80.0 MB
(dax.memory query_budget, BICRAFT_QUERY_BUDGET, or the memory budget)`. The charges are dropped
with the evaluation, so nothing stays reserved, and the session answers the next query.

### Strategies that keep the answer exact in less memory

- **`TOPN` over a large grouping, block by block** (`grouping_top_blocks`). When every group's
  values would not fit in what the cap leaves (or when computing them is refused there), the
  grouped scans are computed once and held, then the groups go by blocks of at most a quarter of
  the room: each block's values are read from the scans and offered to a bounded heap of the
  best `n` keys (all keys, in their directions). A group after the heap's worst is never in the
  result (the bound only gets stricter), so only candidates are kept, pruned as they grow. The
  final selection is the whole path's: every key, ties by position, the rows tying with the
  last kept. Memory: the heap and the candidates (about `n` rows) plus one block, instead of
  one value per group and item.
- **`ORDER BY` cut by `max_rows`.** Under a cap, `EVALUATE SUMMARIZECOLUMNS(...) ORDER BY ...`
  whose keys are its group-by columns or items is answered by the same selection with the
  position breaking ties (exactly `max_rows` rows), and `row_count` the whole grouping's.
  Skipped when a column's type would be read from the rows left out.
- **Radix scans by windows of rows** (`bicraft_scan::scan_groups_within`). A high-cardinality
  grouped scan routes (group, payload) pairs to partitions before aggregating them: 8 to 16
  bytes per selected row, 1.6 GB at 100 M rows. Under a cap, the rows go by windows of whole
  morsels (three quarters of what the cap leaves beside the states); the partitions' states carry
  from one window to the next. The sinks are order-insensitive, so the result is the same.
- **Fewer scan threads** when their states would not fit (`Engine::scan_threads`).

A grouped scan holds one state per group (24 bytes per customer for a sum, and the join of its
partitions doubles it for a moment). Under lot 4 alone a grouping whose states did not fit was
refused; a `TOPN` or a cut `ORDER BY` over it now goes by ranges of groups ("Groupings by
ranges", below).

### Tests

- `preview_eval_mem.rs`: charges released when dropped, refusals name the part and the cap,
  held tables counted once while they live, small tables not counted, a table over the cap
  refused, no cap no counting; under a pool's limit: charges in the pool, refusals saying so, the
  session asked for what is missing, background charges in their class without a ticket.
- `preview_eval_mem_tests.rs` (the bench star in memory, 30,000 rows, 3,000 customers, dirty and
  clean data, compressed and plain columns):
  - `topn_over_a_grouping_under_small_caps_gives_the_same_rows`: 36 `TOPN` shapes (ties,
    directions, several keys, `n` around the row count, filters, `IGNORE`, Desktop's variable
    form, 24 random ones) under caps of 24 K to 64 M: every answer equals the uncapped one or is
    a clean refusal; the block path ran; the same answers once the cap is lifted.
  - `an_order_by_cut_by_max_rows_is_selected_under_a_cap_with_the_same_answer`: five shapes,
    `max_rows` 1 to 1 M, three caps: same rows, `row_count` and `truncated`.
  - `a_query_over_its_cap_is_refused_and_the_session_goes_on`: refusals naming the cap, the
    counter, then the same answers under a larger cap and without one.
  - `bad_query_budgets_are_refused`.
- `crates/scan/src/tests.rs`: the windowed radix path equals the one-thread scan for sums,
  first rows and counts at windows from one morsel to unbounded.
- The bench differentials (below).

### Measured: same answers (2026-10-25, Windows, release, engine bench files)

The 22 bench queries twice in one session under each budget (the query cap following it),
against a session without one (`bench_queries_under_budgets_equal_the_unlimited_answers`):

| Rows | Budget = query cap | Differences | Refused (of 44) | 44 answers (none: 0.34 s at 10 M, 0.11 s at 1 M) |
|---|---|---:|---:|---:|
| 10 M | 300 MiB | 0 | 0 | 0.39 s |
| 10 M | 80 MiB | 0 | 0 | 0.97 s |
| 10 M | 50 / 30 MiB | 0 | 2: Q17's grouped scan (1 M customers) needs 46.8 MB | 2.3 / 4.3 s |
| 1 M | 16 MiB | 0 | 0 | 0.20 s |
| 1 M | 8 / 6 MiB | 0 | 0 | 1.3 s |

- No answer differs. The query cap refuses only Q17 below about 55 MiB at 10 M: its sum by
  customer holds 24 bytes per customer, twice while the partitions are joined (lot 3 answered
  there by going over the budget). The `TOPN` above it goes block by block.
- Before the windowed radix path, Q17's scan needed 198 MB at 10 M (its routed rows: 16 bytes per
  sales row) and was refused at 80 MiB.

## Lot 5: a guaranteed ceiling

### Setting and semantics

- `BICRAFT_MEMORY_LIMIT` (bytes, same suffixes; unset, `0` or `unlimited`: no limit, the default,
  nothing counted, nothing changes). One limit for the process (`bicraft_mem::global`); a host
  embedding BIcraft may set it with `bicraft_mem::set_limit`.
- **Under a limit there is always a budget**: the session's (or `BICRAFT_MEMORY_BUDGET`), never
  more than the limit, and half of it by default. Everything lots 1 to 4 do under a budget
  follows (backup in a temporary file, LRU columns, segments on demand, the query cap).
- **What is counted** (crate `bicraft-mem`, a foundation crate every layer may use): every big
  allocation reserves its bytes in the pool before it is made, in four classes:

  | Class | What | When |
  |---|---|---|
  | kept | decoded columns (reserved from their file size before they are read, then their decoded size, or the column is refused), the segment cache, the storage caches, the aggregations' tables | while the session keeps them |
  | query | everything lot 4 counts: groupings, value vectors, scan states and routed rows, masks, materialized tables, sort keys, answer rows, a scan's result until the query ends | while the query runs |
  | background | an aggregation job's columns and scans | while it builds |
  | io | the streamed decompression's buffers (pieces, jobs, window: fewer threads and smaller pieces when they would not fit, then the lean reference decoder) | while the stored data is decompressed |

- **Admission.** A reservation that does not fit asks the caller's own session to free what it
  keeps (storage caches, least recently used columns no query holds, segments), then every
  other session (`bicraft_mem::Evictor`, which never waits for a busy session), then is refused:
  the query fails with an error naming what needed the memory and the limit (`... needs 24.0 MB,
  over the memory limit of 150.0 MB (131.2 MB already reserved; BICRAFT_MEMORY_LIMIT)`). Nothing
  stays reserved by a refused query; its session goes on.
- **Fair shares.** Each running query (one `QueryTicket` per evaluation) may hold what the limit
  leaves after the kept data, the background builds and the buffers, divided by the queries
  running at that moment; alone, all of it. A query over its share is refused (`over this query's
  share of the memory limit (... with 2 queries running)`), after the evictions.
- **Background work yields.** Aggregation jobs wait while any query of the pool runs, their
  reservations are admitted only under three quarters of the limit, and never get memory by
  eviction: a refused build is dropped (its queries use the fact).
- **Counted after the fact** (bounded by construction, never more than what was reserved for
  them): a column's decoded size above its file size is reserved right after decoding (or the
  column goes); a scan's result is moved from the query to the session at the end of the query.
- **Not counted: the fixed base.** The program and its thread stacks, the model's metadata and
  the session's own structures, the package's parts other than the `DataModel` (left in the
  file under a budget), BIcraft's own data snapshot and literal tables, the viewer's results,
  relationship indexes, tables under 256 KiB, a scan worker's transient segment read from the
  file (one per worker), the allocator's overhead and fragmentation, and the MCP transport's copy
  of an answer (itself bounded by the reserved answer rows).
- **Recommended pod limit** = `BICRAFT_MEMORY_LIMIT` + the base margin measured below.
- `dax.memory` reports `limit {limit, reserved_bytes, peak_reserved_bytes, by_class, refused,
  evictions, running_queries, session_reserved_bytes, process_peak_bytes}` (`process_peak_bytes`:
  Linux's `VmHWM`; elsewhere `null`, measured from outside, as `engine-bench` does).

### Tests

- `crates/mem/src/lib.rs`: no limit, nothing counted; reservations counted until dropped,
  refusals change nothing; forced counts block further reservations until released; eviction
  local first, then the registered evictors, never for background work; fair shares between
  queries (alone, everything left; two, half each); eight threads reserving at once never pass
  the limit; the limit lifted and set again; the process peak read on Linux.
- `crates/engine/src/memory_limit_tests.rs`, sessions with a pool of their own (the process'
  limit would reach the other tests):
  - `answers_under_a_memory_limit_are_the_answers_without_one`: the nine star queries three
    rounds under a limit whose half holds the largest column: same answers, columns evicted, the
    pool's peak within the limit, nothing refused, no query memory left, everything released
    with the session, the spill folder empty.
  - `a_query_over_the_limit_is_refused_cleanly_and_the_session_goes_on`: a limit below one
    column: every query refused with a clear error, counters, no query memory left, then the
    same answers without a limit and under a workable one.
  - `two_sessions_share_one_limit`: two sessions on two threads, four rounds each, one pool:
    same answers, peak within the limit, nothing left, no file left.
  - `one_session_frees_what_it_keeps_for_another`: A keeps every column; under a limit just over
    them, B's columns evict A's; A answers again afterwards.
  - `background_work_waits_while_a_query_runs`, and background reservations under 3/4 of the
    limit.
  - `the_decompression_buffers_are_reserved_and_a_refusal_leaves_no_file`.
  - `bench_queries_under_limits_equal_the_unlimited_answers` (ignored, local bench data): the 22
    bench queries twice per session under each limit against a session without one; peak
    reservation within the limit, nothing left reserved.

### Measured (2026-10-25, Windows, release, engine bench files)

**Same answers, within the limit.** The 22 queries twice in one session whose pool has each
limit, against a session without one (`bench_queries_under_limits_equal_the_unlimited_answers`):

| Rows | Limit | Differences | Refused (of 44) | Peak reserved | Left reserved | 44 answers |
|---|---|---:|---:|---:|---:|---:|
| 10 M | 600 MB | 0 | 0 | 360 MB | 0 | 0.41 s |
| 10 M | 300 MB | 0 | 0 | 252 MB | 0 | 0.39 s |
| 10 M | 200 MB | 0 | 0 | 191 MB | 0 | 0.91 s |
| 10 M | 150 MB | 0 | 2 (Q17: its scan's 46.8 MB beside 115 MB kept and pinned) | 130 MB | 0 | 0.71 s |
| 1 M | 64 / 32 MB | 0 | 0 | 27 / 24 MB | 0 | 0.11 / 0.19 s |
| 1 M | 24 MB | 0 | 0 | 21 MB | 0 | 0.31 s |
| 1 M | 16 MB | 0 | 0 | 15.6 MB | 0 | 2.45 s |

- The pool's peak never passes the limit; nothing stays reserved after a session.
- Under 12 MB at 1 M, `ClientKey` (5.2 MB decoded) and `Montant` are refused cleanly; under
  8 MB a fact segment (2.5 MB) no longer fits (lot 3's rule).

**The process against the limit** (`engine-bench bicraft`, `BICRAFT_MEMORY_LIMIT` in the
environment of `bicraft-cli mcp`, aggregations off; peak working set of the warm session; 22/22
identical to Desktop in every run):

| Rows | Limit | Peak working set | Over the limit |
|---|---|---:|---:|
| 10 M | none | 483 MiB | |
| 10 M | 600 MB | 291 MiB | |
| 10 M | 300 MB | 229 MiB | |
| 10 M | 200 MB | 210 MiB | +20 MB |
| 10 M | 150 MB | 159 MiB | +17 MB |
| 1 M | none | 71 MiB | |
| 1 M | 64 MB | 46 MiB | |
| 1 M | 32 MB | 44 MiB | +14 MB |
| 1 M | 24 MB | 43 MiB | +21 MB |
| 1 M | 16 MB | 71 MiB | +58 MB |

- **The fixed base** is 15–25 MB at these sizes (the process, the model, the transport, the
  allocator). At the smallest limit (16 MB at 1 M) the peak is the unlimited one: the first read
  of a column segment by segment holds batches beside what is counted (lot 3 measured the same
  71 MiB at an 8 MiB budget). **Recommended pod limit: `BICRAFT_MEMORY_LIMIT` + 64 MiB**, and a
  limit of at least a few times the largest column.
- **Default path unchanged**: without a limit, start, cold and warm are within run-to-run noise of
  lot 3 at 1 M and 10 M.
- Under a limit the start pays the streamed open of lot 2. At 300 MB and above the cold and warm
  times are those of an unlimited session. At 200 MB at 10 M the high-cardinality queries (Q06,
  Q07, Q17, Q18) read `ClientKey` again on each run (Q17 is the slowest warm); at 24 MB and below
  at 1 M every query reads its columns again: such limits trade time for memory, never answers.
- Under a tight limit the stored data decompresses through the lean reference decoder (about
  10 MB of buffers) instead of the parallel one (about 30 MB for one thread, 90 MB for 20).

### Extrapolation to 100 M rows (not regenerated for this work)

At 100 M rows (10 M customers; lot 3's estimates for the open and the columns):

- **Q17** (a `TOPN` by customer) needs, under a cap or a limit: the flat combinations (40 MB,
  against 400–560 MB of small vectors before), its grouped scan's state (24 bytes per customer,
  about 240 MB, 480 MB while its partitions are joined; the routed rows held to a window), the
  `TOPN`'s heap and blocks (a few MB), and `ClientKey`'s values and id table (about 270 MB,
  kept). About 0.8 GB in all, against more than 2 GB of formula-engine tables and routed rows
  without a cap.
- **Q18** counts the customers without building them (unchanged).
- **A limit of about 1.6 GB** (a budget of 0.8 GB for what is kept, the same for the query) should
  answer all 22 queries; below about 1.2 GB Q17 is refused cleanly, the other queries answering,
  more slowly as the limit shrinks (columns read again).
- The fixed base (measured at 10 M below) grows with the model's metadata, not with the rows.
- To be measured at the next milestone, once the 100 M data is regenerated.

### Left

- A grouped scan holds one state per group: a grouping whose states do not fit was refused.
  `TOPN` and cut `ORDER BY` now go by ranges of groups ("Groupings by ranges", below).
- A column's values (its dictionary) must fit (lot 3).
- Not counted (the base): see "Setting and semantics". The viewer's results cache (at most 20,000
  results) and BIcraft's own data snapshot are the largest of them on big models.
- Under Windows the process peak is measured from outside (`engine-bench`); in-process only on
  Linux (`VmHWM`).

## Groupings by ranges

### What changes

Under a query cap or a memory limit, a `TOPN` over a `SUMMARIZECOLUMNS` and an `ORDER BY` cut by
`max_rows` go block by block (lot 4). Their grouped scans were still computed whole: one state per
group (a sum: 24 bytes per customer, twice while the partitions are joined). So Q17 at 10 M
(1 M customers, about 47 MB of states) was refused below about 55 MiB of cap or a 170 MB limit.
Now, when that whole scan is refused:

- **The groups are cut into ranges** of consecutive group numbers (the number the whole scan gives
  a group from its codes, the first grouping column varying slowest). A range holds as many groups
  as fit in half of what the query may still use, beside the candidates the heap may keep, at every
  item's state plus one scan's (twice) per group.
- **One bounded grouped scan per range and item** (`bicraft_scan::scan_groups_range`,
  `scan_ordered_range`): the rows of other groups are skipped, and the sink holds the range's groups
  only. The workers' states are held to a quarter of the room. When per-worker sinks would not fit,
  the range's scan is radix-partitioned (one shared state per partition, rows routed by windows)
  rather than run on one thread.
- **Then every combination whose group falls in the range** is offered to the same bounded heap
  (`TopSel`), by its position among the combinations. A combination of codes no column holds (no
  group: every value blank) goes with the first range, as the whole path reads it.
- **A range whose scan or states are refused is halved**, down to one group. Only a range of one
  group that does not fit is refused, with the usual error naming the part and the cap or limit.
- Nothing changes when the states fit. The default unlimited path compiles to the same code: the
  range check is a generic folded away. Other consumers of a grouping, which need every group's
  value at once, are unchanged.

### Why the answers are the same

- **Each group's value is the whole scan's.**
  - A group's state only depends on its own rows.
  - The kernel is chosen from the whole grouping, not from the range, so a range never switches
    kernel (integer sums by data id or by code, histograms, ordered double sums). The histogram and
    bitmap size checks are made on the whole grouping, without allocating.
  - Integer sums, counts, ranks, bitmaps and histograms do not depend on the order of the rows.
  - Double sums go through the same morsels (64 Ki rows, whatever the threads or the range). Each
    morsel's sink holds the group's partial sum over the same rows, and the sinks are merged in row
    order: the same operations on the same values, so the same bits (unit test below).
- **Every group is in exactly one range**, and every combination is offered once.
- **The selection does not depend on the order of the offers.**
  - A row is dropped only when it comes after the heap's worst, which is never better than the
    final `take`-th.
  - Positions keep the combinations' order. The candidates are sorted by position before the final
    selection (every key, ties by position).
  - The number of kept groups is the sum over the ranges.

### Cost

One scan of the fact per range, plus one pass over the combinations per range. Under a budget where
the columns are read segment by segment, each range's scan reads every segment again. The ranges'
results are not kept between queries (they are partial): the warm run computes them again.

### Tests

- `crates/scan/src/tests.rs`, `grouped_scans_by_ranges_give_each_group_its_whole_state`:
  - sums, first rows, counts and double sums (compared bit for bit) of a two-column key with up to
    1.5 M groups;
  - plain and compressed columns, filtered or not, 1 and 4 threads;
  - ranges of 1 group to more than every group, a range past the end (empty);
  - workers' states held to one byte (radix) or unbounded;
  - each group equals the whole scan's.
- `crates/engine/src/preview_eval_mem_tests.rs`:
  - `topn_over_a_grouping_by_ranges_gives_the_same_rows`:
    - the 36 `TOPN` shapes of lot 4, plus distinct counts, medians (histograms), a two-column
      grouping across a relationship, a filter leaving most ranges empty, and `ORDER BY` cut at 1,
      10 and 2,999 rows;
    - dirty (blank customers) and clean data, compressed and plain columns;
    - forced ranges of 700 and 3,001 groups, ranges from the room under 96 K and 48 K caps, and
      ranges of one group on a 200-customer star;
    - every answer equals the uncapped one (under the caps: or is a clean refusal).
  - `a_grouping_whose_states_do_not_fit_is_answered_by_ranges`: 30,000 customers under caps of
    1 M down to 384 K are answered by ranges with the uncapped rows; under 2 K the query is refused
    cleanly, then the session answers.
- The bench differentials (below).

### Measured (2026-10-26, Windows, release, on AC, engine bench files)

**Same answers.** The 22 queries twice per session at 10 M, against a session without a budget or
limit (`bench_queries_under_budgets_equal_the_unlimited_answers`,
`bench_queries_under_limits_equal_the_unlimited_answers`):

| Budget / limit | Differences | Refused (of 44) | Lots 4 and 5 | 44 answers | Pool peak |
|---|---:|---:|---|---:|---:|
| 80 MiB budget | 0 | 0 | 0 refused | 1.04 s | |
| 50 MiB budget | 0 | 0 | 2 refused (Q17) | 4.15 s | |
| 30 MiB budget | 0 | 0 | 2 refused (Q17) | 6.41 s | |
| 200 MB limit | 0 | 0 | 0 refused | 0.97 s | 191 MB |
| 150 MB limit | 0 | 0 | 2 refused (Q17) | 1.13 s | 145 MB |
| 120 MB limit | 0 | 0 | | 1.53 s | 118 MB |

The pool's `refused` counter shows 2 at every limit: the whole scan's reservation, refused before
the ranges answer.

**Time and process peak** (`engine-bench bicraft`, one round, `BICRAFT_MEMORY_BUDGET` or
`BICRAFT_MEMORY_LIMIT` in the server's environment, aggregations off):

| Setting | Peak working set | Q17 cold / warm |
|---|---:|---|
| none | 491 MiB | 104 ms / 31 ms |
| 80 MiB | 192 MiB | 213 ms / 220 ms |
| 50 MiB | 193 MiB | 707 ms / 705 ms (was refused) |
| 30 MiB | 190 MiB | 995 ms / 1.01 s (was refused) |
| 200 MB | 210 MiB | 136 ms / 134 ms |
| 150 MB | 157 MiB | 259 ms / 235 ms (was refused) |
| 120 MB | 142 MiB | refused: `Ventes[Montant]` (25.4 MB) over the limit beside 97.9 MB reserved |

- Every answer is identical to Desktop's in every run. At 120 MB, 21 of 22 answer.
- **Default path**: unchanged at 1 M and 10 M. In an interleaved A/B against `main` (3 rounds,
  10 M), per query the cold runs overlap (Q17 98–104 ms on both), so the difference is noise.
- Below the working set (50 and 30 MiB), every query over `ClientKey` and the fact's big columns is
  much slower, as in lot 3: each range's scan reads the columns' segments again through the
  segment cache. Q17 now answers in 0.7–1.0 s instead of being refused.
- At 120 MB in the server process, Q17 is refused by lot 5's column admission, not by its grouping.
  `Montant` is read whole (it is under half the budget), and its 25.4 MB do not fit beside the
  97.9 MB already reserved. The in-process differential, with less reserved beside it, answers.
  Reading such a column segment by segment when its whole read is refused would lift this (left).

### Left

- Other consumers of a grouping whose states do not fit (a `SUMMARIZECOLUMNS` returned whole, an
  iterator over it) still compute it whole. Their answer holds every group anyway; ranges would only
  save the join's second copy.
- Items keyed differently (an item over another fact that not every grouping column reaches) do not
  share one group numbering. Such a `TOPN` is still refused when its states do not fit.
- A column refused whole under a limit now falls back to segments on demand ("Re-reads below the working set").

## Re-reads below the working set

Below a query's working set, lots 3 and 5 stayed correct but were much slower, because columns
were read again from the spill file on every query. At 10 M rows under 30–50 MiB, queries took
about 130 ms; under a 200 MB limit, Q17 read `ClientKey` again on
every warm run. The traces (`bench_queries_under_budgets_equal_the_unlimited_answers` with
`BICRAFT_BUDGET_TRACE=1`: per run, its time, columns read, evictions, segment loads and hits) show
four causes:

- **Segments read once per morsel.** A scan over several columns read segment by segment holds
  about two segments of each column at a time. With more columns than the cache held, a segment was
  evicted while the other workers still needed it, then read again for the next morsel: 100 to 200
  segment loads per query, for 10 segments per column.
- **Least recently used flooding, at both levels.** A scan repeated over more segments than the
  cache holds evicts each segment just before the next pass needs it, so it gets no hits. The same
  happens with columns: a query run again, whose columns do not all fit, evicted mid-query the very
  columns it read next, and so read all of them again on every run.
- **Storage caches dropped first.** Their few kilobytes went at every enforcement, so a warm run
  could not be answered from them and read its columns again.
- **Segments ahead of columns.** The segment cache kept up to half the budget ahead of the
  decoded columns. Evicting a column means reading it again whole, which costs far more than
  reading one segment again.

### What changes

**The segment cache** (`crates/scan/src/lazy.rs`) works as in 2Q / segmented LRU:

- *protected* part:
  - at most three quarters of the cache;
  - a segment enters it when it is used again while kept, or read again soon after it was evicted
    (a short history of evicted segments);
  - a protected segment leaves only when it is stale (unused for two passes over every column
    reading through the cache) or when the bound shrinks.
- *probation* part: every other segment, least recently used out first.
- The eviction order is: probation before protected, segments no scan holds before held ones,
  stale before fresh.
  - So a new segment never displaces a protected segment that is still in use.
  - A repeated scan over more segments than fit finds the protected share on every pass (at least
    three quarters of the cache); the rest streams through probation.
  - A one-time scan of another column flushes nothing.
  - When the working set changes, the stale entries give way.
- **One read per segment.** A segment being read is read once: the other threads that need it
  wait for that read (`OnceLock`). A segment evicted while a scan still holds it is handed out
  again rather than read again (a weak reference per recent segment).
- **Each thread remembers its last four segments** instead of one: a scan over several columns
  alternates between them.
- **The column's first pass is reused.** The first pass of a column read segment by segment
  (statistics, first-seen ids) offers its segments to the cache. They are kept while they fit
  without evicting anything, so the scan that follows starts with them.
- `dax.memory`'s `segment_cache` adds `protected_bytes` and `shared` (segments handed out without
  a read).

**Columns** (`crates/engine/src/preview_memory.rs`, `Memory::eviction_order`). The order in which
columns are evicted, first to last:

1. columns the running (or last) evaluation did not use;
2. fewest evaluations using them (each evaluation counts once; halved every 64 evaluations, and
   kept when a column is evicted, so a column read again keeps its history);
3. among equal counts, most recently used first.

This order fixes three cases:

- A query run again finds the columns it read last time.
- A query whose columns do not all fit reads again only the column it reads last, instead of all
  of them. Within one evaluation, most recently used first is the optimal order for a loop.
- A one-off query does not push out the usual working set.

`dax.memory` adds `column_reads` (stored columns read under the budget, re-reads included).

**Storage caches.** A storage cache under an eighth of the budget is now dropped last, after the
columns not in use; a larger one is still dropped first.

**Segments and columns** (`State::enforce_core`, `memory::segment_floor`). The segment cache is
set apart up to an eighth of the budget. Above that, up to half the budget, it keeps what the
columns leave and is shrunk when the columns need the room.

**A column refused whole under the limit is read segment by segment** (`Ctx::column`). The case:
at a 120 MB limit, `Ventes[Montant]` (25 MB of files) could not be admitted beside 98 MB already
reserved, so Q17 was refused. Now, when the reservation of a whole read (its files) is refused,
or the reservation of its decoded size is refused:

- the column is read segment by segment instead, admitted for its dictionary file and then for
  its values and id table;
- the first pass reads its batches in what the limit leaves (at least one segment);
- it is refused only when even that does not fit, or when a segment is over what the budget leaves
  for one segment.

**Unchanged:**

- Without a budget or a limit, nothing here runs. A column's use is counted only under a budget.
- Every reservation and bound is the same as before: the cache's bytes stay within its cap and its
  reservation, columns within the budget, and the pool within the limit.
- Answers never change. A column read again is a new identity, so the cache cannot serve a segment
  of an earlier read. A dropped column removes its segments, its history and its pending reads.

### Tests

`crates/scan/src/lazy.rs`:
- `a_repeated_scan_larger_than_the_cache_finds_its_protected_share`: 20 segments, a cache of 8.
  The first pass reads 20 segments; from the third pass on, at most 15 (plain LRU: 20). The cache
  stays bounded, and the protected part stays within it.
- `a_single_scan_of_another_column_does_not_flush_the_protected_segments`.
- `stale_protected_segments_give_way_to_a_new_working_set`.
- `threads_reading_the_same_segments_read_each_once`: 8 threads over 20 segments, 20 reads.
- `a_shrunk_cache_keeps_to_its_bound_and_a_dropped_column_leaves_nothing`.
- `segments_offered_by_the_first_pass_are_kept_while_they_fit`: nothing is evicted, and the next
  pass finds them.
- The lot 3 tests (same codes and scans, bound, a dropped column's segments forgotten, a column
  read again reads its segments again, failed reads) are unchanged.

`crates/engine/src/memory_budget_tests.rs`:
- `a_query_repeated_over_more_columns_than_fit_reads_again_only_one`:
  - three columns under a budget that holds two and a half, five runs;
  - the first run reads 3 columns, later runs at most 1 (least recently used first: 3 every run);
  - same answers, kept columns within the budget, and a small storage cache kept.
- `a_one_off_query_does_not_evict_the_usual_columns`.

`crates/engine/src/memory_limit_tests.rs`:
- `a_column_refused_whole_under_the_limit_is_read_segment_by_segment`:
  - a pool whose free room is under the column's files and decoded size;
  - the query answers, with the column read segment by segment, nothing refused, the peak within
    the limit;
  - with 16 bytes left, a clean refusal, then the same answer once the memory is back.
- The bench differentials take `BICRAFT_BUDGET_REPEAT=k` (each query `k` times in a row, as
  the bench's warm runs).

### Measured (2026-10-26, Windows, release, engine bench files)

**Same answers.** The 22 queries, each run 3 times in a row, two rounds per session, against a
session without a budget or limit: **0 differences**.

| Rows | Budget / limit | Before (lot 5) | Now | Columns read | Segment loads |
|---|---|---:|---:|---:|---:|
| 10 M | 80 MiB | 2.24 s | 2.04 s | 84 → 85 | |
| 10 M | 50 MiB | 5.20 s | 0.84 s | 218 → 64 | 5,337 → 88 |
| 10 M | 30 MiB | 8.69 s | 1.83 s | 250 → 80 | 10,102 → 447 |
| 1 M | 16 / 8 MiB | | 0.49 / 1.25 s | 83 / 222 | 0 / 28 |
| 10 M | 200 / 150 / 120 MB limit | | 1.68 / 2.62 / 2.60 s | | |

- **Before.** The "Before" column is lot 5's code, with aggregations on. With aggregations on, a first
  version of this change took 0.95 s and 2.47 s at 50 and 30 MiB; the "Now" column is with aggregations off,
  as the engine bench runs.
- **Errors.** The 6 errors at 50 and 30 MiB are lot 4's query cap: Q17's grouped scan (46.8 MB)
  over a 50 or 30 MiB cap, as before.
- **Limits.** The pool's peak stays within every limit (197.7, 146.3 and 119.7 MB) and nothing
  stays reserved. At 120 MB Q17 now answers: 6 whole reads refused by the limit, each read
  segment by segment instead.

**Engine bench** (`engine-bench bicraft`, one round; 22/22 identical to Desktop in every run;
peak working set of the warm session; the 50 and 30 MiB runs with `BICRAFT_QUERY_BUDGET=0`, as
lot 3 measured them before the query cap):

| Rows | Budget / limit | Peak |
|---|---|---:|
| 10 M | none | 490 MiB |
| 10 M | 80 MiB | 193 MiB |
| 10 M | 50 MiB | 229 MiB |
| 10 M | 30 MiB | 217 MiB |
| 10 M | 200 MB limit | 208 MiB |
| 10 M | 150 MB limit | 160 MiB |
| 10 M | 120 MB limit | 142 MiB (Q17 refused before, answered now) |
| 1 M | none | 71 MiB |
| 1 M | 16 MiB | 41 MiB |
| 1 M | 8 MiB | 42 MiB |

- **Default path.** Without a budget nothing changes: the default runs are within run-to-run
  noise at 1 M and 10 M.
- **Tight budgets.** Under 50 and 30 MiB at 10 M, the warm runs are answered from the kept storage
  cache and columns, so the warm worst case no longer reads its columns again. Cold runs (a fresh
  session per query) still read their columns segment by segment once; the scan finds the first
  pass's segments.
- **Under a 200 MB limit** Q17 keeps `ClientKey` between runs.
- **Still slow:**
  - Q17 warm at 1 M under 16 and 8 MiB (35 and 50 ms). Its working set (`ClientKey` and
    `Montant`) is over the budget, so one column is read again on each run.
  - At 1 M under 8 MiB, the cold runs (a fresh session reads `Montant` segment by segment:
    21–28 ms).
  - At 10 M under 30 MiB, Q19 and Q04 cold (44 and 21 ms).

### Left

- A whole column read again (`ClientKey` at 10 M, 50 ms) costs the same as its first read. Keeping
  the dictionary (or the first pass's first-seen ids) when the column is evicted would make the
  re-read cheaper, at the cost of counting that kept memory.
- A query whose working set is over the budget still reads one column again on each run.
- Cold runs under budgets below a column (1 M at 8 MiB) pay the first pass segment by segment.
