# UI speed tests

The engine has its benches ([benchmarks.md](benchmarks.md)); these tests time the window
itself. They follow the Crafting Apps' method ([method.md](method.md)): DeckCraft drives its
egui window with egui_kittest, without a GPU or a window on screen, and asserts generous
bounds that catch a regression of an order of magnitude without failing on a busy machine.

## What DeckCraft does

Read from the public repository (`storytold/deckcraft`, `main`, 2026-11-19):

- **UI soak** (`crates/ui-egui/src/ui_soak_tests.rs`): seeded random clicks, drags, keys,
  typing and scrolling over the sample deck, with invariants every ten actions (documents
  valid, bounded undo, journal, thumbnail and texture caches, no caught panic) and a **mean
  frame under 250 ms**, "generous for slow CI machines". 150 actions in the gate, a long run
  behind `#[ignore]` and an environment variable.
- **Huge documents** (`crates/ui-egui/src/huge_deck_tests.rs`): structural rather than timed.
  Over 800 to 2,000 slides, the outline, thumbnail pane and sorter lay out and render only what
  is in view, and a settled pane renders nothing more (counted renders, not wall time).
- **`cargo xtask bench`** (`xtask/src/bench.rs`): the release CLI times open, render, save and
  export, compared with a saved baseline JSON (a step 1.5 times slower is flagged). It measures
  no frame rate (its docs/gaps.md says so).
- Its tests build with `opt-level = 1` for the workspace and 2 for dependencies, as BIcraft's.

## What BIcraft measures

`crates/ui/src/tests/speed_ui_tests.rs` opens a generated report in the window as the desktop
runs it: the engine session on a worker thread, as `apps/bicraft-desktop/src/local.rs` does
(snapshot published after each change and again while visuals wait for data), and the pages on
the render thread. The model is `Sales(Id, Product, Region, Amount, Qty)` made by M, a
calculated column and three measures; the report has 8 pages of 10 visuals (three cards, a
slicer, a table, a matrix, column, line, pie and bar charts). The window is 1600 × 960 at 60
frames a second.

A **frame** is one egui pass plus the tessellation of its shapes into meshes (what the CPU does
before the GPU; no GPU work is timed). A **latency** is the wall time from the input to its
result on screen. Scenarios:

| Scenario | Measured | Budget (gate / nightly) |
|---|---|---|
| Opening | first frame; first snapshot (since 2026-11-25 the pages without their data); first page drawn with all its data | 30 s / 120 s to the page |
| Idle, Report view | 120 frames; and the window asks for no further frame | median 250 ms, max 2 s |
| Page tabs | click to the new page drawn with its data (P2, P5, P1) | 10 s / 60 s |
| Drag a visual | 60 pointer moves of the drag; release to the moved visual drawn | frames as above; 10 s / 60 s |
| Resize a visual | 60 moves of its bottom-right handle; release to the visual drawn at its new size; the frames whose content is drawn at another size than its outline | frames as above; 10 s / 60 s; no such frame |
| Format visual | click to the pane read (`visual.props` answered); the frames after | as above |
| DAX formula bar | 19 characters typed, two frames each | frames as above |
| Data view | switch to the first rows; 60 wheel events over the whole table; the rows scrolled to read | as above |
| Transform data | click to the Power Query editor's preview | 10 s / 60 s |

The budgets are bounds, not targets: the gate's are 10 to 40 times the medians below. The
frame bounds were first set to 50 ms and 1 s; with `cargo test` running the crate's 330 tests
on all cores, the Format pane's median frame reached 72 ms, so they are 250 ms (DeckCraft's
figure) and 2 s. Under nextest, `.config/nextest.toml` gives these tests every thread
(`threads-required = "num-cpus"`), so no other test runs beside them.

- **Gate:** `tests::speed_ui_tests::a_large_report_stays_responsive`, 100,000 rows, about 5 s.
- **Nightly:** `tests::speed_ui_tests::a_million_row_report_stays_responsive`, 1,000,000 rows,
  about 20 s, in `xtask/src/heavy.rs` ([ci.md](ci.md)).

`cargo test -p bicraft-ui --lib speed_ui_tests -- --nocapture --test-threads 1` prints the
table of every measure.

## Measured

**Measured** on 2026-11-19: Windows 11, AMD Ryzen AI 9 365 (20 threads), 31 GB, rustc 1.99.0,
egui_kittest 0.36.2, the test profile (workspace `opt-level = 1`, dependencies 2), the tests
alone (`--test-threads 1`) on a machine shared with other agents' builds. Ranges over three
runs (gate) and two runs (nightly); they vary by up to 3 times with the load.

| Measure | 100,000 rows | 1,000,000 rows |
|---|---|---|
| First frame | 0.7–0.8 ms | 0.6–0.8 ms |
| First page drawn with its data | 0.82–1.11 s | 7.9–11.8 s |
| Idle frame, median (max) | 1.5–6.1 ms (10.6–18.2 ms) | 2.0–5.7 ms (5.2–20.0 ms) |
| Page tab, click to drawn (median of 3) | 135–214 ms | 118–304 ms |
| Drag frame, median (max) | 2.0–9.5 ms (3.6–23.8 ms) | 1.8–4.7 ms (6.0–9.3 ms) |
| Drag release to drawn | 113–221 ms | 121–176 ms |
| Format visual, click to read | 10–28 ms | 138–212 ms |
| Formula bar frame while typing, median (max) | 2.2–7.3 ms (3.1–16.4 ms) | 2.3–4.3 ms (3.6–13.3 ms) |
| Data view, switch to first rows | 8–18 ms | 11–17 ms |
| Data view scroll frame, median (max) | 1.1–3.2 ms (2.0–7.0 ms) | 1.1–2.6 ms (2.4–10.5 ms) |
| Data view, scrolled rows read | 18–42 ms | 39–59 ms |
| Transform data, click to preview | 0.55–0.99 s | 5.7–9.1 s |
| Power Query editor frame, median (max) | 3.4–8.6 ms (14.8–32.7 ms) | 3.9–6.5 ms (16.6–31.4 ms) |

Reading the table:

- Every frame stays under 33 ms (30 frames a second) at both sizes; frames do not grow with the
  row count, since the grids lay out only the rows in view.
- What grows with the rows is engine work off the window's thread: opening (the M query that
  generates the table, evaluated in full) and the Power Query preview, which evaluates the
  query at its step (5.7–9.1 s at a million rows). The window keeps drawing meanwhile.
- The idle window settles: after a few frames it requests no repaint.

## Opening a file from disk

`apps/bicraft-desktop/src/tests/open_speed_tests.rs` opens a `.pbix` and a `.pbip` that BIcraft
wrote itself (`Sales` of N rows made by M, a calculated column, three measures; page 1 with two
cards, a slicer, a table, a matrix, a column chart, a line chart and a gauge, page 2 with a pie
and a bar chart; the project's `.pbi/cache.abf` from the package's data) through the window's
real path: `App::open_path`, the local source of `src/local.rs` on its worker thread, the render
thread. It also takes the file apart on the test's thread, stage by stage: reading it, unzipping
it, decompressing the `DataModel` (XPress9), reading its backup's metadata (ABF, SQLite),
`file.open` in full, the snapshots until every visual has its data (with each visual's
evaluation time from the Performance analyzer), and rendering page 1. The gate opens 100,000
rows, under 60 s. `BICRAFT_OPEN_BENCH_ROWS=1000000,10000000` times larger files, generated once
into `BICRAFT_OPEN_BENCH_DIR` when it is set; the files under `BICRAFT_TESTDATA` are timed too
when it is set, printed by number only.

```text
cargo test -p bicraft-desktop --lib open_speed -- --nocapture --test-threads 1
```

## Measured on 2026-11-25: opening and resizing

**Measured** on the maintainers' laptop (the machine above), Windows 11, rustc 1.99.0, in a
**release-like** profile: `cargo test --release` with `CARGO_PROFILE_RELEASE_LTO=false` and
`CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16` (opt-level 3 without the release build's thin LTO), the
tests alone, two runs each (one run for "before"); other builds may have been running. The
local test-data folder was not set, so no real file was timed. BIcraft's own numbers only.

Opening a file from disk, until page 1 is drawn:

| File | Before: page 1 drawn (with its data) | After: page 1 drawn (no data yet) | After: page 1 drawn with its data |
|---|---|---|---|
| 100,000 rows, `.pbix` (1.1 MB) | 123 ms | 33–34 ms | 92–95 ms |
| 100,000 rows, `.pbip` | 122 ms | 31–35 ms | 97–103 ms |
| 1,000,000 rows, `.pbix` (12 MB) | 591 ms | 54–72 ms | 324–339 ms |
| 1,000,000 rows, `.pbip` | 566 ms | 37–40 ms | 309–313 ms |
| 10,000,000 rows, `.pbix` (128 MB) | 133 ms | 126–127 ms | no data (below) |
| 10,000,000 rows, `.pbip` | 155 ms | 26–28 ms | no data (below) |

Stages of the 1,000,000-row `.pbix`, before → after: reading the file 4.7 ms, unzipping it
3.4 ms, decompressing its `DataModel` (12 → 29 MB) 18 ms, the metadata 1.4 ms, `file.open` in
full 22 ms; the first snapshot, which runs every visual's query, **486 → 213 ms** (each visual
32–81 ms → 10–44 ms); a snapshot afterwards, every result cached, 5 ms; rendering page 1 at
scale 1, 62–74 ms. At 10,000,000 rows, decompressing the 128 MB `DataModel` (384 MB of backup)
takes 124–145 ms and `file.open` 110–117 ms.

Resizing a visual (the column chart of the 100,000- and 1,000,000-row reports above):

| Measure | Before | After |
|---|---|---|
| Frames of the drag, median (max) | 1.7–3.5 ms (4.6–6.2 ms) | 1.9–2.6 ms (5.2–7.0 ms) |
| Frames drawing the content at another size than the outline | 58 of 60, then 13–15 after the release | 0 of 60, 0 after the release |
| Release to the visual drawn at its new size | 61–65 ms | 61–75 ms |

What changed:

- **Resize.** While the handle moves, each resized visual's content is drawn from the page image
  on screen, scaled into its outline, over its old place filled with the colour behind it; after
  the release it stays so until the page rendered at the new size is on screen
  (`crates/ui/src/canvas.rs`, `previews`). Before, the image kept the old size until the engine
  and the renderer answered (one frame lagging the outline per pointer move, about 60 ms after
  the release). The engine still renders the page once, at the end.
- **Opening.** The local source publishes the opened file's pages without their data first (a
  snapshot that runs no query), then the snapshot that runs them (`apps/bicraft-desktop/src/local.rs`,
  `publish_opened`): the report is drawn while the queries run, instead of a blank page.
- **A cold visual's query.** Its grouping numbered each of a table's rows through a hash map
  (one entry per row, about 30 ms a visual at 1,000,000 rows); a table of slots does it when the
  grouping columns have at most 2^20 key combinations (`crates/engine/src/preview.rs`,
  `table_groups`). The results are the same.

What remains slow, and why:

- **Tables over 2,000,000 rows showed no data in the window**, and **the first snapshot's
  queries** went through the viewer's own row-by-row evaluator: both addressed the same day, see
  "Visuals through the storage engine" below.
- **Rendering a page** takes 60–75 ms at scale 1 for 8 visuals; it runs after every edit, on the
  render thread.
- **The generated report of the UI test** opens in 0.7 s at 100,000 rows and 6.3 s at 1,000,000
  in the release-like profile: its table is an M query evaluated in full at open (no stored
  data), not a file's stored rows.

## Measured on 2026-11-25: visuals through the storage engine (G70)

**Measured** on the same laptop, Windows 11, in the same **release-like** profile as above
(`cargo test --release`, `CARGO_PROFILE_RELEASE_LTO=false`, `CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16`),
`open_speed_tests` with `BICRAFT_OPEN_BENCH_ROWS=100000,1000000,10000000` on the same generated
files, two runs before and two after; other builds were running on the machine. BIcraft's own
numbers only.

| File | First snapshot, before | First snapshot, after | Page 1 drawn with its data, before | After |
|---|---|---|---|---|
| 100,000 rows, `.pbix` | 70–82 ms | 21–61 ms | 85–97 ms | 65–90 ms |
| 100,000 rows, `.pbip` | 32–33 ms | 19–20 ms | 91–123 ms | 65–100 ms |
| 1,000,000 rows, `.pbix` | 204–235 ms | 36–42 ms | 304 ms | 105–134 ms |
| 1,000,000 rows, `.pbip` | 229–231 ms | 53–64 ms | 282–312 ms | 103–138 ms |
| 10,000,000 rows, `.pbix` | 5 ms, **0 of 8 visuals** (an error each) | 54–60 ms, 8 of 8 | no data | 221–263 ms |
| 10,000,000 rows, `.pbip` | 139–168 ms, **0 of 8 visuals** | 188–216 ms, 8 of 8 | no data | 265–270 ms |

Each visual of page 1 at 1,000,000 rows took 7–48 ms before, 0.1–29 ms after. The `.pbip`'s
first visual includes reading its `.pbi/cache.abf` (about 130–160 ms at 10,000,000 rows, before
and after).

What changed (`crates/engine/src/preview.rs`, `grid`): a visual with a value field is evaluated by
the general evaluator first, whose aggregations the storage engine answers with one grouped scan
over the columns' codes for every cell (as `dax.evaluate`), cached between the visuals of a
snapshot; the row-by-row evaluator runs only when the general evaluator cannot (more than
20,000 groups, the work it allows), and only it keeps the 2,000,000-row cap
(`SnapshotLimits::data_table_rows`). A visual of columns only keeps the row-by-row evaluator
first. Results are the same, real-number sums to their last digits (`preview_grid_tests.rs`);
the memory is that of the storage engine, under the session's memory limit
([beyond-ram.md](beyond-ram.md)); the window's memory was not measured.

## Not covered

See [gaps.md](gaps.md) G70: release-build numbers, the GPU and the OS event loop, the browser shell,
the Model view's diagram on a large model, memory, a saved baseline to compare runs, a soak of
random input as DeckCraft's, Linux numbers.
