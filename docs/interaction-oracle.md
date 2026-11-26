# Interaction oracle (Power BI Desktop)

What does Power BI Desktop display, visual by visual, for a given interaction state: slicer
selections of every mode, slicers that filter each other, a click (or Ctrl+click) on a bar, a
column, a pie slice, a line point or a table row, the page's `visualInteractions`, one-way and
two-way relationships, an inactive relationship, `CALCULATE` / `ALL` / `KEEPFILTERS` /
`ALLSELECTED` under a selection, slicers synced across pages, the Clear and Apply filters
buttons? The interaction oracle answers with a corpus recorded in Desktop: for each state, the
DAX query each visual sent to the engine, the engine's result, and what the visual drew. It is the
judge of the interactive viewer (`crates/view`), and its queries already test BIcraft's DAX
evaluator.

| File | What it is |
|---|---|
| `xtask/src/oracles/interactions.rs` | The scenarios (one page each), the synthetic model, the report generator: `cargo xtask oracle-gen interactions DIR [patch]` |
| `xtask/src/oracles/interactions_collect.rs` | `cargo xtask oracle-gen interactions DIR collect [OUT.json]`: recordings → corpus |
| `xtask/src/oracles/interactions_tests.rs` | The tests on the corpus (run on Linux, no Desktop) |
| `xtask/interaction-oracle/record.ps1` | One recording, Windows only: Desktop, trace, driver, replay |
| `xtask/interaction-oracle/launch.ps1` | Starts Desktop on the report with WebView2's DevTools port, finds its engine port (the port file, `msmdsrv.port.txt`, that the engine writes in its workspace folder) |
| `xtask/interaction-oracle/trace.ps1` | AMO server trace (QueryBegin / QueryEnd) of that engine, one JSON line per event |
| `xtask/interaction-oracle/drive.mjs` | Plays the steps through the DevTools protocol and reads every visual (Node 22+) |
| `xtask/interaction-oracle/replay.ps1` | Runs every traced query again on the same engine (ADOMD) and keeps the results |
| `testsupport/interaction_oracle/corpus.json` | The corpus (generated, committed) |
| `testsupport/interaction_oracle/model.bim` | The model it was recorded on (generated, committed) |

Current corpus: recorded 2026-11-25 with Power BI Desktop **2.160.1151.0** (Microsoft Store), UI
fr-FR, in one recording: **39 states** (19 from the files alone, 20 reached by clicks) on 28 scenario pages,
**388 visual readings** (9 or 10 visuals per page), **438 distinct DAX queries** with their
results.

## Regenerating

```powershell
$d = "$env:TEMP\oracle-reports\interactions"   # any folder outside the repository
cargo xtask oracle-gen interactions $d                   # model.bim, cmds1.json, cmds2.json, plan.json
bicraft run --file $d\cmds1.json                        # -> $d\proj\Interactions.pbip
cargo xtask oracle-gen interactions $d patch             # selections, interactions, sync groups, page filters
bicraft run --file $d\cmds2.json                        # -> $d\interactions.pbix (data computed by bicraft)
powershell -ExecutionPolicy Bypass -File xtask\interaction-oracle\record.ps1 -Dir $d
cargo xtask oracle-gen interactions $d collect           # -> testsupport\interaction_oracle\
cargo test -p xtask interaction -- --nocapture
```

`record.ps1` takes the Desktop lock `desktop` in `$env:BICRAFT_LOCKS_DIR` (default
`%LOCALAPPDATA%\bicraft\locks`), and waits while it or the priority lock
(`$env:BICRAFT_PRIORITY_LOCK`, default `priority`) exists, opens **its own** Desktop instance on the generated
PBIX, and closes only that instance (by its process id) at the end, whatever happens. `-Only
I26,I27` records some scenarios only, `-Raw raw2` writes that recording to another folder:
`collect` merges every `DIR\raw*` folder in name order, a later recording replacing the states it
repeats (the current corpus is one recording, `raw`; the driver times out every call instead
of hanging).

Desktop's own text never reaches the repository: engine error messages stay out of `results`
(an error is `"error": true`), and the corpus keeps no UI string (the "Select all" item becomes
`{"select_all": true}`, a blank card `{"card": null}`, slicer header texts are dropped). The DAX
queries are the text Desktop generated for our report on our model.

## Method

**Report.** `oracle-gen interactions` writes a star schema of DAX calculated tables (no data
source): `Product` (6 rows: Product, Category, Color), `Store` (4: Store, Region), `Date`
(2025-01-01 – 2026-12-31: Year, Month, Quarter; date table), `Sales` (240 rows from
`GENERATESERIES`; each product sold in two stores only), `Tag` (one row per page, no
relationship). Relationships: `Sales → Product` one-way, `Sales ↔ Store` **both directions**,
`Sales → Date` on `Date`, `Sales → Date` on `ShipDate` **inactive**. Measures: `Total Amount`,
`Units`, `Lines`, `All Products` (`ALL(Product)`), `Red Keep` (`KEEPFILTERS(Color = "Red")`),
`Red Replace` (`Color = "Red"`), `Share Selected` (`ALLSELECTED(Product)`), `Shipped Amount`
(`USERELATIONSHIP` on ShipDate), `Store Count`, `Product Count`.

Every scenario page carries the same probes: three cards (`Total Amount`, `Store Count`,
`Product Count`), a bar chart (Category), a column chart (Store), a pie (Color), two tables
(Product × the CALCULATE measures; Product × Share Selected, Shipped Amount) and a line chart
(Month), plus the scenario's slicers and buttons in the left column. The page I00 (no visual) is
the active page, so every scenario page is first queried while the trace runs.

**One page per scenario, a page filter per page.** Each page filters `Tag[Page]` on its own id.
The table has no relationship, so the filter changes no value, but it makes every query text of
a page unique: Desktop answers a query it already sent from its client cache, without asking the
engine (seen on the first trial: a second page with the same visuals sent nothing), and a page
filter is the only neutral way to keep each page's queries visible in the trace.

**Applying a state.** File states are written into the PBIR by the `patch` step: a slicer's
`objects.general.filter` (and `data.mode`, numeric bounds, relative-date settings), the page's
`visualInteractions`, a visual's `syncGroup`, `isHidden`, a button's `visualLink`. Click states are
played by `drive.mjs` in Desktop's report view, a WebView2 page reached through its DevTools port
(the same channel as `tools/render-oracle/capture.ps1`): page tabs are clicked, data points
clicked with `Input.dispatchMouseEvent` (Ctrl through the event's modifiers; buttons are
Ctrl+clicked, as Desktop's editing view requires). No window focus or screen is involved, and no
UI Automation was needed.

**Reading a state.** Three sources, kept together per visual:
1. **Engine trace.** `trace.ps1` records every query the engine of that Desktop instance
   receives. A query is given to the visual of the page shown at that moment whose fields (its
   columns and measures outside the filter-table variables) are the query's. The query results
   are then obtained by running each distinct query again on the same engine (`replay.ps1`): the
   queries are self-contained (filters are in their text, relative dates resolved to literal
   dates), so the result is what Desktop received. A visual that sends nothing in a step keeps
   its previous query; when its display matches an earlier query of its own instead (a step back
   to an earlier state, served from Desktop's cache), that one is taken (`query_from: "cache"`).
2. **What the visual drew**, read from the page's DOM only (geometry, fill opacity, text, ARIA
   attributes; no script object of the page). Bar and column charts: each mark's accessible name
   is its value, the axis tick at the same `aria-posinset` its category, `aria-selected` its
   selection and a fill opacity below 1 its dimming; the highlighted value is the mark's value
   times the length of the highlight overlay drawn over it divided by the mark's length. Pie
   slices: category and value from the accessible name; the highlighted share is the square of
   the highlight overlay's radius over the slice's (the overlay's area over the slice's, which
   agrees with the query values). Line charts draw one focusable point: it is focused and moved
   along the line with the arrow keys, and each point's accessible name gives its category and
   value. Tables: the cell texts; slicers: the items and their selected state, or the inputs;
   cards: the number shown in their accessible name.
3. **Which visual types highlight**: the highlight probe below (`highlight_types.json`).

`display_matches_query` says whether every number (or item) the visual drew is in its query's
result: 364 readings agree, 8 more through the cache rule, 1 does not (a rule below: I27.file),
15 have nothing comparable (range, relative and text slicers, the hidden slicer, the button
slicer).

## Corpus format (`corpus.json`, version 1)

```json
{
  "version": 1,
  "generator": "cargo xtask oracle-gen interactions (xtask/src/oracles/interactions.rs)",
  "model": "model.bim",
  "desktop": "2.160.1151.0", "ui_culture": "fr-FR", "recorded": "2026-11-25T17:01:44+01:00",
  "recordings": 1,
  "page_filter": "every page filters the disconnected table Tag[Page] on its own id",
  "pages": [ {
    "page": "I13", "title": "slicer nofilter",
    "visuals": [ { "name": "i13_sl", "key": "sl", "type": "slicer", "x": 10, "y": 10, "width": 250, "height": 225,
                   "fields": [["Values", "Product[Category]"]],
                   "selection": { "Version": 2, "From": […], "Where": […] },     // the file's general.filter
                   "mode": "'Dropdown'", "sync_group": "syncCategory", "hidden": true,
                   "action": "'ClearAllSlicers'" } ],                            // buttons
    "interactions": [ { "source": "i13_sl", "target": "i13_amt", "type": "NoFilter" } ]
  } ],
  "states": [ {
    "id": "I19.gear_ctrl", "page": "I19", "shown_page": "I19",
    "note": "Ctrl+click a second bar: both selected",
    "actions": [ { "do": "click|item|button|page", "visual": "i19_bar", "point": "Gear", "item": "…",
                   "page": "I16", "ctrl": true, "ok": true } ],                  // in order, after the previous step
    "visuals": [ {
      "name": "i19_col", "type": "clusteredColumnChart",
      "query": "q230", "query_from": "trace|cache|none", "fresh": true,          // fresh: sent during this step
      "display_matches_query": true,
      "display": {
        "points": [ { "cat": "Lyon", "value": 6171, "highlight": 4553, "dimmed": true, "selected": true,
                      "original": 6171, "ratio": 0.31 } ],                       // charts
        "rows": [ { "cells": ["Product", "Total Amount"], "selected": false } ], // tables, header first
        "items": [ { "text": "Toys", "selected": true }, { "select_all": true, "selected": false } ],
        "inputs": ["2", "4"],                                                    // range / relative inputs
        "card": "14,25K"                                                         // or null when blank
      } } ]
  } ],
  "queries": [ { "id": "q230", "dax": "DEFINE …", "fields": ["Sales[Total Amount]", "Store[Store]"],
                 "results": [ { "columns": [ { "name": "Store[Store]", "type": "string" } ],
                                "rows": [ { "Store[Store]": "Lille", "[Total_Amount]": 3088 } ] } ],
                 "error": true } ]                                               // instead of results
}
```

A step's state is cumulative: the actions of a scenario's earlier steps still apply. A `page`
action leaves the scenario page; that step is read on the other page (`shown_page`). Values are
the engine's (blank = `null`, dates ISO); a visual's data is the **last** result set of its query
(line charts send a row count first), without the `[IsGrandTotalRowTotal]` total row.

## Using the corpus

`cargo test -p xtask interaction -- --nocapture` (Linux too):

- `observed_queries_match_the_dax_evaluator`: each query through `dax.evaluate` on the model
  (data computed by BIcraft), compared like the M9.4 query oracle. **438 / 438 agree, 0 refused,
  0 different** on the 2026-11-25 recording. The test fails on any different value and below 438.
- `file_states_match_the_viewer_data`: the 19 file states, through the existing snapshot
  (`Session::snapshot_with`, `preview_filters.rs`): **171 / 171 non-slicer visuals agree** with
  Desktop's rows (floor 171).
- `click_states_match_the_interactive_viewer`: the 20 click states, played in order on one
  session with `view.interact` (a scenario starts with `reset`; `click` → `select` with the clicked
  value typed like the visual's first column, `item` → `slicer_toggle`, `button`, `page`, Ctrl
  kept), each visual compared like the file states, a highlighted visual's rows followed by their
  `data.highlights` values (Desktop's highlight column), slicers by the items they show ticked.
  A visual Desktop redrew from its cache with no highlight (rule 13, `fresh: false`) is compared on
  its drawn (full) values. **180 / 180 visuals, 12 / 12 slicer tick sets** on the 2026-11-25
  recording (floors 180 and 12).

## Interaction rules established

Each rule cites the corpus states that show it (`I19.toys` = state, `q216` = query), and its
source: **F** the files applied, **T** the engine trace (queries and results), **D** what the
visual drew (DOM), **P** the highlight probe (`highlight_types.json`).

### Slicers

1. **A slicer selection filters every other visual of the page through a filter table in the
   visual's query** (T): `In` → `TREATAS({"Toys"}, 'Product'[Category])` (I02.file, q013);
   several values of one slicer are OR-ed in the same `TREATAS` (I03.file); `Not(In)` (select
   all, then untick) → `FILTER(KEEPFILTERS(VALUES(col)), NOT(col IN {"Toys"}))` (I14.file, q136);
   a range → `FILTER(KEEPFILTERS(VALUES(col)), AND(col >= 2, col <= 4))` (I06.file, I15.file q148);
   a text slicer's Contains → `FILTER(KEEPFILTERS(VALUES(col)), SEARCH("a", col, 1, 0) >= 1)`, so
   without case (I09.file, q080). Two slicers are two filter tables: AND (I27.file; I28.applied:
   Toys and Red leave nothing, every visual blank). Page filters come as further filter tables.
2. **A relative-date slicer filters only through its `general.filter`** (F, T, D): with only its
   `data` settings (`Relative`, last 6 months) Desktop shows the range ("26/05/2026 –
   25/11/2026" on 2026-11-25) but filters nothing (I07.file: 21 684, the total); with the filter
   (`Between(DateSpan(DateAdd(DateAdd(Now, 1, day), -6, month), day), DateSpan(Now, day))`) the
   query carries **literal dates resolved by the client**: `'Date'[Date] >= DATE(2026, 5, 26) &&
   < DATE(2026, 11, 26)` (I08.file, q072: 5 926). Today is included, the bounds are half-open.
3. **A slicer lists its items with the other slicers' and the page's filters, never its own**
   (T): `CALCULATETABLE(VALUES(col), KEEPFILTERS(<each other filter table>))`, `TOPN(101, …)`
   (I27.file q396, q387; I14.file q135 lists all three categories under its own `Not In`). The filters
   reach the item column through the model like any filter context: same table (I10.file: under
   Toys, the product slicer lists Ball and Kite only), two-way relationship (I11.file: under Kite,
   the store slicer lists Lille and Paris, q106), but not up a one-way relationship (I11.file:
   all 12 months, q101; I12.file: under Paris, all 6 products).
4. **A selected value missing from a slicer's filtered items is still listed, selected** (T, D):
   I27.file, the colour slicer holds Red and Blue under Category = Toys; its query returns Blue
   and Green (q387), it shows Blue, Green and Red with Red and Blue ticked — the only reading that
   differs from its query. Its filter still applies (no Toys product is red: the result is the
   Toys ∩ Blue one).
5. **A hidden slicer filters like a visible one** (F, T): I16.file (hidden synced copy holding
   Toys) gives the Toys values.
6. **Sync groups are not reconciled when the report opens; a UI selection is** (F, T, D): a copy
   whose file holds no filter is unfiltered although the other copies hold Toys (I17.file: totals,
   nothing ticked); after Gear is clicked on I18, the hidden copy (I18.on_I16) and the copy
   without filter (I18.on_I17, now showing Gear ticked) both filter on Gear.
7. **`visualInteractions` from a slicer: `NoFilter` removes the slicer's filter table from the
   target's query; `DataFilter` is the default filter** (F, T): I13.file, the card and the bar
   chart keep the totals (q123 has only the page filter), the pie and the other visuals are
   filtered on Toys.
8. **Clicking slicer items: a plain click replaces the selection, Ctrl+click adds to it** (D, T):
   I26.toys → Toys; I26.gear_ctrl → Toys and Gear (OR); I26.wheels_plain → Wheels only. The
   slicer itself is not re-queried (its item query stays q350).

### Clicks on visuals

9. **The default interaction depends on the target's type: highlight when the visual supports
   it, filter otherwise** (D, T, P): after a click on a bar (I19.toys) the column chart and the pie
   highlight; the cards, both tables and the line chart filter. In the highlight probe (below)
   Desktop 2.160 highlights bar / column (clustered, stacked, 100 %), combo, ribbon, pie, donut,
   funnel, treemap and decomposition tree targets, and filters line, area (also stacked and
   100 %), scatter, waterfall, card, multi-row card, new card, KPI, gauge, table and matrix
   targets. Maps sent no query (unsettled); Q&A was not probed. BIcraft filters both.
10. **A filter from a click is the same filter table as a slicer's** (T): `TREATAS({"Toys"},
    'Product'[Category])` (I19.toys q216, q221); Ctrl+click puts both values in it (I19.gear_ctrl,
    I21.blue_ctrl); a table row filters on its row field (I22.ball: `TREATAS({"Ball"},
    'Product'[Product])`, q297); a line point on its category (I23.m3: `TREATAS({3}, 'Date'[Month])`).
11. **A highlight is one query with two values per category** (T, D): `SUMMARIZECOLUMNS(cat,
    <slicer and page filter tables>, "Total_Amount", [Total Amount], "SalesTotal_Amount",
    IGNORE(CALCULATE([Total Amount], KEEPFILTERS(__H0FilterTable))))` with `__H0FilterTable` the
    selection's `TREATAS` (q223, q222). The first value is the full bar, drawn dimmed (opacity
    0.4); the second the highlighted part, drawn opaque over it (I19.toys col: Lille 3 088 full,
    1 491 highlighted); a category with no highlighted value keeps only its dimmed bar (I22.ball
    bar: Gear). A pie keeps its slices' sizes (the full values) and draws each slice's highlighted
    share, `highlight / full` (I19.toys: Blue 0.524 = 3 748 / 7 150, Red 0).
12. **The clicked visual is not queried again** (T, D): it keeps its data, the selected points
    opaque and `selected`, the others dimmed (I19.toys bar; I20.paris col; I21.red pie; I23.m3
    line; I22.ball table: no dimming in tables).
13. **Selection gestures** (D, T): Ctrl+click adds a point (I19.gear_ctrl, I21.blue_ctrl); a plain
    click on one of several selected points keeps only that one (I19.gear_plain); a plain click on
    the only selected point clears the selection — Desktop then shows the page's first data again
    from its cache, without a query (I19.gear_again: every visual back to the totals, nothing
    fresh).
14. **`visualInteractions` from a chart** (F, T, D): `NoFilter` → the target is neither queried nor
    dimmed (I24.toys col); `DataFilter` on a highlight-capable visual → a filter instead of a
    highlight (I24.toys pie: only Blue and Green, q330); `HighlightFilter` on a visual that cannot
    highlight (table, line) → a filter (I24.toys tbl q325, line).
15. **A click combines with the slicers** (T, D): the slicer's filter table stays in every query,
    the clicked visual's own included; the highlight is computed inside it (I25.toys: Paris and
    Lyon selected, bar shows Gear 4 887 / Toys 3 991 / Wheels 3 660, column chart highlights 1 750
    and 2 241, q342).

### Buttons

16. **Clear all slicers (`ClearAllSlicers`) clears every slicer of the page and nothing else**
    (D, T): I27.cleared, after a click on the Toys bar and the button, both slicers are empty and
    the visuals are filtered or highlighted by the bar selection alone (cards 7 480, column chart
    highlighted).
17. **With an Apply all slicers button (`ApplyAllSlicers`) on the page, slicer clicks wait for
    the button, even on an Import model** (D, T): I28.pending, Toys and Red ticked, no query sent,
    every visual still at the totals; I28.applied, after the button, both apply at once (AND:
    every visual blank).

### Measures and relationships under a selection

18. **Two-way relationships carry a dimension's selection to the other dimensions, one-way ones
    do not** (T): under Kite, `Store Count` is 2 (I11.file); under Paris (slicer I12.file or
    click I20.paris), `Product Count` stays 6.
19. **Table rows: a product row is shown when any measure of the row is not blank** (T, D): under
    Paris (I12.file, I20.paris) the first table lists all six products, blank `Total Amount` where
    the product is not sold in Paris, because `All Products` is not blank; the second table
    (Share Selected, Shipped Amount) lists only the three sold there.
20. **`CALCULATE` modifiers see the selection as a filter context** (T): `ALL(Product)` returns the
    total of the other filters (21 684 under Toys, I02.file; 6 367 under Paris, I12.file);
    `KEEPFILTERS(Color = "Red")` and `Color = "Red"` give blank under Toys (no red toy) and the red
    totals under Color = Red (I04.file); `ALLSELECTED(Product)` is the visual's outer filters, so
    the shares add up to 1 under a slicer (I02.file 0.50 / 0.50) and under a click (I22.ball: 1.00);
    `USERELATIONSHIP` follows the ship date under a month range (I15.file: Shipped Amount total
    7 598, Total Amount 7 412).

### Query shapes (T)

21. Cards: `SUMMARIZECOLUMNS(<filter tables>, "Total_Amount", IGNORE([Total Amount]))`, no
    group-by. Bars and columns: `TOPN(1001, …)`; pies `TOPN(1002, …)`; tables
    `ROLLUPADDISSUBTOTAL(col, "IsGrandTotalRowTotal")` and `TOPN(502, …)` ordered total first;
    line charts a row count `ROW("DS0IntersectionCount", …)` then
    `SAMPLEAXISWITHLOCALMINMAX(3500, …, 350, , ALPHABETICAL, ASC, 105000, 60)`; slicer items
    `TOPN(101, CALCULATETABLE(VALUES(col), KEEPFILTERS(…)))`. Measure columns are named after the
    measure with spaces replaced by `_`; the highlight column prefixes the table name
    (`SalesTotal_Amount`).

## Highlight probe

Which built-in visual types Desktop highlights, rather than filters, when a data point of another
visual is selected. `xtask/interaction-oracle/highlight_probe.mjs gen` writes, on the oracle's
model, one page with a source bar chart (`Product[Category]`) and one target per type (29
types), each target with a measure of its own (`HL_<type>` = `[Total Amount]`) so that its query
can be told apart in the engine trace; `record.ps1` plays a click on Toys in the source and
traces the queries; `highlight_probe.mjs verdicts` classifies each target: **highlight** when its
query computes its measure a second time under `KEEPFILTERS` of the selection's filter table
(rule 11), **filter** when its query filters on the selection's `TREATAS`, **unknown** when it
sent no query. The drawn marks agree: the highlighted bar, column, combo, ribbon and pie targets
show dimmed marks with a highlight overlay, the filtered ones neither.

Run **HL-2026-11-25**: Power BI Desktop 2.160.1151.0 (Store), UI fr-FR, recorded
2026-11-25T17:14+01:00; results in `testsupport/interaction_oracle/highlight_types.json` (the raw
recording is kept locally by the maintainers).

- Highlight: `clusteredColumnChart`, `columnChart`, `hundredPercentStackedColumnChart`,
  `clusteredBarChart`, `barChart`, `hundredPercentStackedBarChart`,
  `lineClusteredColumnComboChart`, `lineStackedColumnComboChart`, `ribbonChart`, `funnel`,
  `pieChart`, `donutChart`, `treemap`, `decompositionTreeVisual`.
- Filter: `lineChart`, `areaChart`, `stackedAreaChart`, `hundredPercentStackedAreaChart`,
  `waterfallChart`, `scatterChart`, `card`, `multiRowCard`, `cardVisual`, `kpi`, `gauge`,
  `tableEx`, `pivotTable`.
- Unknown: `map`, `filledMap` (no query: the maps did not load). Not probed: `qnaVisual`,
  `azureMap`, `shapeMap` and other types.

The table of the types that highlight in `crates/engine/src/interact.rs` is exactly the
highlight list (`interact_tests::highlight_types_are_the_ones_desktop_highlighted`); a type the
probe did not settle is filtered.

## Limits

- One Desktop build (2.160.1151.0, Store), French UI, one machine; relative dates resolve against
  the recording day (2026-11-25), so I08 is only valid for its literal-date queries.
- Click positions come from the drawn marks (bars, columns, slices, line points, table cells):
  other visual types (scatter, maps, matrices, the new button slicer) are not clicked, and the
  button slicer's (`advancedSlicerVisual`) items are not read (its filter does apply: I05.file).
- No state separates `KEEPFILTERS` from a plain `CALCULATE` filter on the same column (a
  Color = Blue selection would); no drill, no bookmarks, no report-level filters, no
  `defaultFilterActionIsDataFilter` report setting, no DirectQuery model (rule 17 is on Import).
- The DOM readings are a cross-check (numbers in the queries' results); card texts are the
  formatted, abbreviated numbers ("21,68K").
- Queries are attributed to visuals by their fields: two visuals with exactly the same fields on
  one page would be indistinguishable (the scenarios avoid it).
- The recording opens a **PBIX built by
  BIcraft** from the generated PBIP, not the PBIP itself, so Desktop starts with the data
  (calculated tables computed) and never writes into the project; the PBIP stays the versioned
  source of the report.
