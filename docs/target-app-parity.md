# BIcraft vs Power BI Desktop: parity assessment

> **Last reviewed:** 2026-11-05 · **Last updated:** 2026-11-23 · **Change:** minor (Desktop UI
> inventory of Desktop 2.159.1231.0 and its ledger, M11.28–M11.29; 2026-11-18: rows the
> lots M11.16–M11.25 closed rescored, lot status; gap review 2026-11-05; first assessment
> 2026-11-04, ROADMAP M11.7) · **Target:** Power BI Desktop as Microsoft Learn describes it
> (November 2026) · **BIcraft:** `main` at 816a4cb (gap review: ccb96e3)

How close BIcraft is to Power BI Desktop, on the two axes of [method.md](method.md) decision 5:
the **authoring** features (semantic model, Power Query, report, files) and the **report
viewer** (what Desktop's reading view shows and lets a reader do). [ROADMAP.md](../ROADMAP.md)
summarizes it, [gaps.md](gaps.md) itemizes the shortfalls, and
[parity-checklist.md](parity-checklist.md) holds every row behind the numbers. The form is the
Crafting Apps' (DeckCraft's and GridCraft's `docs/target-app-parity.md`): every number is marked
**measured** (with the command, test or doc that measures it) or **estimated**. The gap review below
is the newest full reading, the update of 2026-11-18 rescores the rows its lots closed; the
sections after them keep the method and the per-axis notes.

## Update 2026-11-23: Desktop UI inventory

ROADMAP M11.28 and M11.29. The first reading of Desktop itself rather than of Microsoft Learn:
Power BI Desktop 2.159.1231.0 (Store) read as a black box from what it renders (accessibility
trees, rendered labels, screenshots and window captures, kept locally), driven only by input
events; the method and every group are in [parity-checklist.md](parity-checklist.md) "Desktop UI
inventory". Each ribbon group, pane, menu and dialog is a row with its controls counted
(**measured**) and BIcraft's status for the group (a judgement); `cargo xtask parity` weights the
statuses by controls into the "Desktop UI controls covered" line of the [headline](#headline).
Eight authoring rows came out of it (the Power Query editor's other transforms, Output data,
the Options dialog, sparklines, Analyze, the Data format card, the editors' format and find
tools, service-bound items out of scope); the shortfalls are summarized in [gaps.md](gaps.md)
G106. The readiness estimates below were not revised.

## Update 2026-11-18

The rows the lots of M11.16–M11.25 closed were rescored against `main` at 816a4cb (198 engine
commands); the rest of the gap review's scoring stands, and no Desktop run was made. Lots 1–10,
13 and 15 of the [top 20](#top-20-missing-features-as-a-proposed-order-of-lots) landed (lot 7
without print, lot 9 without the Analytics pane), lot 17 in part (the Data view and See records,
not Show as a table); lots 11, 12, 14, 16, 18, 19 and 20 are open. New gaps found on this pass
are G56–G63 in [gaps.md](gaps.md).

| Number | 2026-11-05 | 2026-11-18 | Kind |
|---|---|---|---|
| Authoring done | 63 of 129 (49%) | **79 of 129 (61%)**; 20 partial, 30 missing | **measured**: `cargo xtask parity` |
| Authoring weighted | 68% | **78%** (P0 96%, P1 78%, P2 36%) | **measured**: same |
| Viewer, `bicraft render` | 20 of 34 (59%), weighted 74% | **22 of 33 (67%)**, weighted 80% (the bookmark row is "–" for a PNG, as drill is) | **measured**: same |
| Viewer, web viewer (page.html) | 26 of 56 (46%), weighted 61% | unchanged | **measured**: same |
| Viewer, BIcraft app | 29 of 56 (52%), weighted 65% | **38 of 56 (68%)**, weighted 81% | **measured**: same |
| Ready for real work, agent | ~65% | **~70%** | **estimated**: [Readiness](#readiness) |
| Ready for real work, person in the app | ~50% | **~70%** | **estimated**: [Readiness](#readiness) |
| Remaining effort, open lots of the top 20 | ~370–660 h (all 20) | **~170–330 Opus 5.5 agent hours** | **estimated**: the review's per-lot figures, lot 17's rest counted 5–10 h |

The two halves of the answer, as of 2026-11-18:

- **For an agent (CLI, MCP)**, the large holes are data access (Power Query editing beyond M
  text, every connector but files and Databricks, DirectQuery), field and what-if parameters,
  groups and bins, show value as, and some DAX functions (window functions, balances,
  `GROUPBY`). Bookmarks and sync slicers are done.
- **For a person in the app**, canvas editing, copy and paste, the Format visual pane with its fx
  buttons, the Selection, Bookmarks and Sync slicers panes, the text box editor, the Data view
  and the reader's drill, drillthrough, bookmarks and export are done. The holes are the Power
  Query editor and Get data's Navigator, an editable model diagram and properties pane, the
  Analytics pane, focus mode and Show as a table, view as a role and a performance analyzer.

## Gap review 2026-11-05

The user's question: what is missing for features equal to Power BI Desktop. This review rescored
every row of [parity-checklist.md](parity-checklist.md) against `main` at ccb96e3 (162 commands)
and widened the target to all of Desktop: 39 authoring rows (one out of scope) and 7 viewer rows
(the AI visuals row split in three; two out of scope) were added, among
them two new areas, **Report UI** and **Model UI**, that score Desktop's dedicated surfaces (the
Format pane, the canvas handles, the Selection and Bookmarks panes, the Model view's editing)
rather than the command behind them. No Desktop run was made; the inventory comes from the
Microsoft Learn pages in [Sources](#sources-microsoft-learn), read on 2026-11-04 and 2026-11-05
(the October 2026 "What's new" page added the Gantt chart and the dropdown list slicer), and
from the reviewer's knowledge of Desktop's documented features where no page was read again.

### Summary

| Number | 2026-11-04 (first assessment, 132b199) | 2026-11-05, same rows | 2026-11-05, widened target | Kind |
|---|---|---|---|---|
| Authoring done | 56 of 91 (62%) | 57 of 91 (63%) | **63 of 129 (49%)**; 26 partial, 40 missing | **measured**: `cargo xtask parity` (the middle column by the same scoring over the old rows) |
| Authoring weighted | 76% | 78% | **68%** (P0 87%, P1 63%, P2 31%) | **measured**: same |
| Authoring items the app reaches | 2 of 68 | 69 of 69 | **89 of 89** (since M11.3); about 40 through a dedicated surface (panes, formula bar, page tabs, File view, menus), about 49 only through the generic parameters form or the palette | **measured**: `cargo xtask parity`; the split counted by hand from the Button column |
| Viewer, `bicraft render` | 19 of 29 (66%), weighted 77% | 19 of 29, weighted 78% | **20 of 34 (59%)**, weighted 74% | **measured**: same |
| Viewer, web viewer (page.html) | 26 of 51 (51%), weighted 63% | unchanged | **26 of 56 (46%)**, weighted 61% | **measured**: same |
| Viewer, BIcraft app | 27 of 51 (53%), weighted 62% | 28 of 51 (55%), weighted 67% | **29 of 56 (52%)**, weighted 65% | **measured**: same |
| Ready for real work, agent | ~70% | ~70% | **~65%** on the widened target | **estimated**: [Readiness](#readiness) |
| Ready for real work, person in the app | ~25% | — | **~50%** | **estimated**: [Readiness](#readiness), new formula |
| Remaining effort, top 20 lots below | — | — | **~370–660 Opus 5.5 agent hours** | **estimated**: per lot below |
| Remaining effort, everything else in scope | — | — | **~400–800 h**, plus maps, custom visuals, AI visuals and DirectQuery at 60–150 h each with open feasibility questions | **estimated** |

What changed since 2026-11-04: every command has its button (M11.1–M11.3, the ribbon, menus and
palette), the Data, Build and Filters panes with drag and drop (M11.12, M11.13), the DAX formula
bar (M11.14), auto-recovery (M11.11), decomposition trees drawn and clickable in the renderer and
the app (G34), matrix expansion and per-level subtotals drawn from the saved state. The widened
target lowers the shares: the new rows are mostly what Desktop does with dedicated UI where
BIcraft has a command behind a form, and Power Query, storage and visual-family depth.

The two halves of the answer (2026-11-05; see the [update](#update-2026-11-18) for what closed
since):

- **For an agent (CLI, MCP)**, BIcraft covers the semantic model, the report's visuals,
  formatting, filters and files nearly as Desktop does. The large holes are data access
  (Power Query editing beyond M text, every connector but files and Databricks, DirectQuery),
  report-wide state (bookmarks, sync slicers, field parameters, groups) and some DAX functions.
- **For a person in the app**, the holes are the direct-manipulation surfaces: moving and
  resizing visuals with the mouse, copy and paste, the Format pane, the Selection and Bookmarks
  panes, the Power Query editor, an editable model diagram, and the reader's drill, drillthrough
  and export.

### Top 20 missing features, as a proposed order of lots

Ranked by value over effort, with what each one closes first. Value: **H** a common Desktop
workflow is blocked, **M** it is hindered, **L** a rare one. Effort in Opus 5.5 agent hours (S
3–10, M 10–30, L 30–60, XL 60–150), calibrated on [ROADMAP.md](../ROADMAP.md)'s log
(**estimated**). The Edit interactions mode, being built when this was written, landed on 2026-11-06 (M11.15).

**Status on 2026-11-18.** Done: lot 1 (M11.16), 2 (M11.16), 3 (M11.17, `query.preview`), 4
(M11.18), 5 and 8 (M11.19), 6 and 13 (M11.20), 7 (M11.21, without print), 9 (M11.22, without
the Analytics pane), 10 (M11.24, reader application 2026-11-18), 15 (M11.16 groups, M11.23,
M11.25). In part: 17 (M11.17: the Data view and See records; Show as a table not built). Open:
11, 12, 14, 16, 18, 19, 20.

| # | Lot | Value | Effort | Depends on | Feasible | Rows closed or moved |
|---|---|---|---|---|---|---|
| 1 | Copy, paste and duplicate visuals (Ctrl+C, Ctrl+V, Ctrl+D; across pages and files through a clipboard JSON of `report.extract`'s shape) | H | S (6–10) | — | yes | Visuals: copy and paste |
| 2 | Canvas direct manipulation: drag to move, eight resize handles, multi-select, align and distribute, arrow keys, snap | H | M (15–25) | `visual.set_position`, `page.arrange` exist; one batch per gesture for undo | yes | Report UI: canvas; Visuals: add, move, resize |
| 3 | Query preview: `m.preview {expression or partition, rows}` and its grid (the answer grid exists) | H | S (6–10) | the M evaluator | yes | Power Query: data preview |
| 4 | Refresh a PBIX from local sources (CSV, Excel, JSON, folders) into its `DataModel`, as for PBIP | H | M (10–20) | the writer already encodes VertiPaq | yes | Data: refresh from local files (P0) |
| 5 | Reader drill: drill mode, down, up, next level, expand all; matrix +/- expand and collapse | H | M–L (25–40) | drill state in `view.interact`; the evaluator reads active levels already | yes | Interactions: drill (P0); Matrix (P0); Drill authoring row |
| 6 | Text boxes with rich text (runs, fonts, links, dynamic values), image upload into `RegisteredResources`, button action editor | H | M (15–25) | the PBIR paragraph shape is documented in [pbir.md](formats/pbir.md) | yes | Visuals: text boxes (P0), buttons and images |
| 7 | Export to PDF (every page through the renderer) and a visual's data to CSV/XLSX | H | M (15–25) | a pure-Rust PDF writer (vector text or raster pages), `xlsx` writing | yes | Export: PDF, data, print |
| 8 | Reader drillthrough (context menu, button, back) and report page tooltips | H | M (20–30) | lot 5's menu; drillthrough filters into the target page | yes | Interactions: drillthrough, page tooltips |
| 9 | Format pane: the type's cards from the visual options catalog ([visual-options.md](formats/visual-options.md)), search, reset to default, conditional-format buttons | H | L (30–50) | `visual.options`, `visual.format` | yes | Report UI: Format pane (P0), analytics pane, conditional formatting dialog |
| 10 | Bookmarks: `bookmark.*` commands (capture data, display, current page, selected visuals; groups), the pane, apply in the reader, bookmark navigator and buttons | H | L (30–45) | the reader state of `view.interact`; bookmarks are already kept and copied | yes | Interactions: bookmarks (2 rows), bookmark navigator, Bookmarks pane |
| 11 | SQL Server connector (TDS over TLS, Windows and SQL auth, folding of the common steps), then PostgreSQL | H | L (30–60) | the connector API of [data-sources.md](formats/data-sources.md); a permissive TDS crate or a port | yes, opt-in like Databricks | Data: databases (P0, moves to P) |
| 12 | Power Query editor: queries list, applied steps from the `let` chain, preview per step, a transform ribbon writing M (filter, remove, rename, type, merge, append, group, pivot) | H | XL (60–120) | lot 3 | yes; M round trip must keep the user's text | Power Query: editor (P0) |
| 13 | Page background and wallpaper command and Format pane card | M | S (3–6) | — | yes | Pages: background |
| 14 | Performance analyzer over BIcraft's own evaluator (per-visual query and render time, copy query) | M | S–M (8–15) | the snapshot's evaluation; BIcraft's own times only (never Desktop's) | yes | Views: performance analyzer |
| 15 | Selection pane, show and hide, group and ungroup, sync slicers (commands and panes) | M | M (20–30) | groups and sync groups are kept today | yes | Report UI: selection and sync panes; Visuals: group; Filters: sync slicers |
| 16 | Field parameters and what-if parameters (create, and the evaluator switching a visual's projection) | M | M–L (20–35) | `NAMEOF` already followed; slicer selection into the projection | yes | Parameters (2 rows) |
| 17 | Data view and see records: a table's rows in the virtualized grid with sort and filter, Show as a table in the app | M | M (10–20) | the answer grid | yes | Views: data view; Reader tools: show as a table |
| 18 | Model view editing: drag cards, saved layouts, drag a column to relate, properties pane in place | M | M (15–30) | `relationship.add`, `*.set`; layouts in `diagramLayout.json` | yes | Model UI (3 rows); Relationships: layouts |
| 19 | View as a role (and none, other user) in the reader and the app | M | M (10–20) | the evaluator applying role filters | yes | Data: view as a role |
| 20 | DAX: window functions (`OFFSET`, `INDEX`, `WINDOW`, `RANK`, `ROWNUMBER`), balances, `GROUPBY`, natural joins | M | L (25–40) | the DAX oracle for the checks | yes | Data: every DAX function; prerequisite of visual calculations |

The next ones by the same ranking: show value as (M, S), groups and bins (M, M), quick measures
(M, M), point selection in matrices, scatter, waterfall and cards (M, M), keyboard focus on
visuals and screen readers (M, M), visual header in reading and editing (M, M), OLS and
perspectives commands (L, S each), visual calculations (M, L, after lot 20), storage mode and
incremental refresh policy authoring (M, M), mobile layout view (L, M), high contrast (L, M),
more UI languages (L, M each).

### Hard items and feasibility

- **DirectQuery and composite models** (XL+): every visual becomes SQL against the source; one
  query translator per source dialect, and nothing to check against without the sources. Storage
  mode authoring alone (kept today) is M.
- **Maps** (XL): map and Azure Maps tiles need tile servers (network, keys, the tiles' licences);
  BIcraft draws bundled public-domain Natural Earth country outlines instead and places country
  names and ISO codes offline (2026-11-20, G92); geocoding of other places is the hard part.
  Desktop's map visuals use Bing/Azure Maps services.
- **Custom visuals** (XL): `.pbiviz` packages are JavaScript against the visuals API and the DOM.
  The browser shell could host them in a sandboxed iframe; the desktop shell has no DOM (a
  JavaScript engine plus an SVG/DOM shim, or a web view, both large). `bicraft render` would need
  the same host. AppSource needs sign-in (out of scope).
- **AI visuals**: key influencers have an opt-in estimate (G33) not Desktop's algorithm;
  decomposition tree AI splits are re-chosen (G34). **Q&A** needs a natural-language engine over
  the linguistic schema (XL; an LLM through MCP is possible but not offline nor deterministic).
  **Smart narrative** is template text plus Q&A-style summaries (L).
- **Python and R visuals** stay out of scope: BIcraft never runs scripts (AGENTS.md).
- **Power Query step editor** (XL): the UI must write M that Desktop reads back step by step;
  BIcraft's M evaluator covers the common library ([data-sources.md](formats/data-sources.md)).
- **External tools** (L): Desktop exposes a local Analysis Services port (XMLA) that DAX Studio
  and Tabular Editor use; BIcraft offers its CLI and MCP instead. An XMLA endpoint is a large
  protocol surface and is not planned.
- **Export to PowerPoint and Analyze in Excel** are service features (out of scope); Desktop
  exports to PDF only.

### Full table by area

Every row of [parity-checklist.md](parity-checklist.md) that was partial (P) or missing (M) on
2026-11-05, with value, effort, dependencies and feasibility (**estimated**). Lot numbers refer to
the list above. Rows the lots closed since are marked D with their milestone (status of
2026-11-18); the checklist holds the current status of every row.

| Area | Feature | Status | Value | Effort | Depends on, notes |
|---|---|---|---|---|---|
| Columns | Groups and bins | M | M | M | a calculated column with Desktop's group metadata; feasible |
| Columns | Auto date/time variations | P | L | S | create the hidden date tables, not only remove them |
| Measures | Quick measures | M | M | M | DAX templates over the model; feasible |
| Measures | DAX user-defined functions | P | L | S | `function.add/set/remove` |
| Relationships | Saved diagram layouts | M | M | M | lot 18 |
| Relationships | Autodetect relationships | M | L | S | name and type matching |
| Parameters | Field parameters | M | M | M–L | lot 16 |
| Parameters | What-if parameters | M | M | S–M | lot 16 (a `GENERATESERIES` table, a measure, a slicer) |
| Security | OLS | M | L | S | commands over what is kept |
| Metadata | Perspectives | M | L | S | commands over what is kept |
| Metadata | Q&A linguistic schema and synonyms | M | L | M | kept; only useful with Q&A |
| Storage | Storage mode, composite models | M | M | M | authoring only; running is the next row |
| Storage | DirectQuery execution | M | M | XL+ | see hard items |
| Storage | Incremental refresh policy | M | L | M | kept and its partitions written already |
| Storage | User-defined aggregations | M | L | M | `alternateOf` on columns; BIcraft's evaluator would use them |
| Power Query | Power Query editor | P | H | XL | lot 12 |
| Power Query | Data preview | D | H | S | lot 3 done 2026-11-06 (M11.17, `query.preview`) |
| Power Query | Data profiling | M | L | M | after lot 3 |
| Power Query | Query dependencies view | M | L | S–M | from the M references |
| Power Query | Diagnostics, native query, folding | M | L | M | after lot 11 |
| Data | Refresh from local files | D | H | M | lot 4 done 2026-11-06 (M11.18, PBIX into its `DataModel`) |
| Data | Databases and online services | M | H | L per family | lot 11; web, OData, SharePoint next (M–L each); network only when allowed |
| Data | Privacy levels and credentials | M | L | M | with lot 11 |
| Views | TMDL view | P | M | M | `tmdl.apply {script}` (createOrReplace) and a view |
| Views | Data view | D | M | M | lot 17 done 2026-11-06 (M11.17, `table.rows`) |
| Views | Model view and properties pane | P | M | M | lot 18 |
| Views | Performance analyzer | M | M | S–M | lot 14 |
| Files | Import Power Pivot workbook | M | L | L | the xlsx's embedded model is an ABF BIcraft reads; mapping is the work |
| Pages | Page background and wallpaper | D | M | S | lot 13 done 2026-11-06 (M11.20, `page.format`) |
| Visuals | Show value as | M | M | S | projection `showAs` and the evaluator's percent forms |
| Visuals | Copy, paste, duplicate visuals | D | H | S | lot 1 done 2026-11-06 (M11.16) |
| Visuals | AI visuals | P | L | XL | see hard items |
| Visuals | Gantt chart (preview) | P | L | M | type, format objects and cards from the theme schema 2.159 (2026-11-20); its data roles are not public (G99) |
| Visuals | Text boxes with formatted text | D | H | M | lot 6 done 2026-11-06 (M11.20) |
| Visuals | Buttons, shapes, images, navigators | P | H | M | lot 6 done (M11.20) and navigators applied (2026-11-18); left Insert › Shapes, hover and press states (G52) |
| Visuals | Group and ungroup | D | M | S–M | done 2026-11-06 (M11.16) |
| Visuals | Custom visuals | M | M | XL | see hard items; import of `.pbiviz` alone is S |
| Visuals | Visual calculations | M | M | L | after lot 20 |
| Formatting | Format painter | D | M | S | `visual.copy_format`, Home › Format painter (2026-11-20) |
| Filters | Filter pane formatting and card order | M | L | S | `report.filter_pane` |
| Filters | Sync slicers | D | M | S–M | lot 15 done 2026-11-17 (M11.25; fields followed 2026-11-18) |
| Interactions | Bookmarks | D | H | L | lot 10 done 2026-11-17 (M11.24) |
| Interactions | Drill (authoring) | D | M | S | with lot 5, 2026-11-06 (M11.19, `view.keep_drill`) |
| Themes | Built-in theme gallery | M | L | S | themes of our own (Microsoft's are assets we may not ship) |
| Report | Report settings | P | L | S | `report.settings` writes the interaction defaults and the export mode (M11.15, M11.21); the rest is kept (G63) |
| Report UI | Canvas direct manipulation | D | H | M | lot 2 done 2026-11-06 (M11.16) |
| Report UI | Format pane | D | H | L | lot 9 done 2026-11-09 (M11.22) |
| Report UI | Analytics pane | P | M | M | left from lot 9: no Analytics tab in Build (G41) |
| Report UI | Conditional formatting dialog | D | M | M | the fx buttons' dialog 2026-11-18 (G54); icons, data bars, web URL, the gradient's numbers and empty values, two-condition rules and a preview 2026-11-20; rules' Percent left |
| Report UI | Selection pane | D | M | M | lot 15 done 2026-11-17 (M11.23) |
| Report UI | Bookmarks pane | D | H | M | lot 10 done 2026-11-17 (M11.24) |
| Report UI | Sync slicers pane | D | M | S | lot 15 done 2026-11-17 (M11.25) |
| Report UI | Visual header in editing | P | M | M | focus mode, sort, show as table |
| Report UI | Mobile layout view | M | L | M | a phone canvas over `visual.mobile` |
| Report UI | Customize theme dialog | P | M | M | pickers over `theme.set_*` |
| Report UI | View options | P | L | S | gridlines, lock objects, zoom slider, thumbnails |
| Model UI | Diagram editing | M | M | M | lot 18 |
| Model UI | Properties pane | P | M | M | lot 18 |
| Model UI | Manage relationships dialog | P | M | S–M | preview rows from the data |
| Model UI | Manage roles dialog | P | M | S | a role list with tables and the DAX editor |
| Model UI | DAX query view as a view | P | M | M | tabs, the formula bar's editor, update the model from `DEFINE MEASURE` |
| Accessibility | App keyboard and screen readers | P | M | M | AccessKit nodes from the hit map for visuals |
| Accessibility | High contrast | M | L | M | a high-contrast token set and report rendering |
| Localisation | App UI languages | P | L | M per language | the i18n table holds EN and FR |
| Tools | External tools (XMLA) | M | L | XL | not planned; CLI and MCP instead |
| Viewer: Visuals | Ribbon chart | P | L | S | render and app done 2026-11-20 (Ribbons card, totals, selection); page.html stacks columns |
| Viewer: Visuals | Matrix expand and collapse | D (app) | H | M | lot 5 done 2026-11-06 (M11.19); render and page.html P |
| Viewer: Visuals | Bookmark navigator | D (render, app) | M | S | lot 10 done 2026-11-18; page.html M |
| Viewer: Visuals | Maps | M | M | XL | see hard items |
| Viewer: Visuals | Decomposition tree in page.html | M (web) | L | — | page.html is being replaced by the app ([webui.md](webui.md)) |
| Viewer: Visuals | Key influencers | P | L | L | Desktop's algorithm is not documented; the estimate is BIcraft's |
| Viewer: Visuals | Q&A, smart narrative | M | L | XL | see hard items |
| Viewer: Visuals | Gantt chart | M | L | M | a placeholder until its roles are known (G99) |
| Viewer: Visuals | Custom visuals | M | M | XL | see hard items |
| Viewer: Data | Every DAX function | P | M | L | lot 20 |
| Viewer: Data | View as a role | M | M | M | lot 19 |
| Viewer: Interactions | Point selection in more visuals | M | M | M | hit regions and `select` paths per family |
| Viewer: Interactions | Report page tooltips | D (app) | M | M | lot 8 done 2026-11-06 (M11.19); page.html M |
| Viewer: Interactions | Drill | D (app) | H | M–L | lot 5 done 2026-11-06 (M11.19); page.html M |
| Viewer: Interactions | Drillthrough | D (app) | H | M | lot 8 done 2026-11-06 (M11.19); page.html M |
| Viewer: Interactions | Bookmarks | D (app) | H | L | lot 10 done 2026-11-18 (`view.interact {action: "bookmark"}`); page.html M |
| Viewer: Interactions | Q&A and web URL buttons | P (app) | L | S | web URL opens the link (with a prompt); Q&A needs Q&A |
| Viewer: Reader tools | Filters pane (web) | M | M | — | the app has it; page.html is being replaced |
| Viewer: Reader tools | Focus mode, spotlight | M | M | S | |
| Viewer: Reader tools | Show as a table | P (app) | M | M | lot 17: See records done (M11.17), Show as a table not built |
| Viewer: Reader tools | Sort from the header | P (app) | M | S | the header's More options |
| Viewer: Reader tools | Visual header | D (app) | M | M | more options and drill icons (M11.19, M11.21); page.html M |
| Viewer: Reader tools | Phone layout | M | L | M | draw `mobile.json` positions |
| Viewer: Reader tools | Keyboard and screen readers on visuals | M (app) | M | M | as the accessibility row |
| Viewer: Reader tools | High contrast | M | L | M | as the accessibility row |
| Viewer: Export | PDF, data, print | D (app: PDF, data) | H | M | lot 7 done 2026-11-06 (M11.21); print and page.html export left (G53) |

### Not assessed

- Depth inside each D row (every option of a dialog), usability, and end-to-end workflows with a
  real author: not scored, as before.
- Desktop's preview features beyond the October 2026 "What's new" page, the Desktop Options
  dialog item by item, and the full connector catalog (more than 150 connectors in Desktop's
  Get data dialog, scored as one row per family here).
- Visual fidelity of the new families (decomposition tree compared on 23 trees, G34; others
  not compared since 2026-11-04).
- No Desktop run was made, and no Desktop timing is published (AGENTS.md §1).

## Headline

The table between the markers is written by `cargo xtask parity`; `cargo xtask docs` fails when
it no longer matches the checklist.

<!-- HEADLINE -->
| Number | Value | Kind |
|---|---|---|
| **Authoring features done** | **127 of 145 (88%)**; 13 partial, 5 missing | **measured**: `cargo xtask parity` over [parity-checklist.md](parity-checklist.md) (each row's status is a judgement against the evidence below) |
| Authoring, weighted by priority | **95%** (P0 99%, P1 96%, P2 83%) | **measured**: `cargo xtask parity` over [parity-checklist.md](parity-checklist.md) (each row's status is a judgement against the evidence below) |
| Authoring items with a button in the app | **140 of 140** BIcraft has | **measured**: `cargo xtask parity` over [parity-checklist.md](parity-checklist.md) (each row's status is a judgement against the evidence below) |
| **Viewer: `bicraft render` (PNG)** | **25 of 33 (76%)** done, weighted 89% | **measured**: `cargo xtask parity` over [parity-checklist.md](parity-checklist.md) (each row's status is a judgement against the evidence below) |
| **Viewer: Web viewer (`bicraft view`, page.html)** | **28 of 56 (50%)** done, weighted 64% | **measured**: `cargo xtask parity` over [parity-checklist.md](parity-checklist.md) (each row's status is a judgement against the evidence below) |
| **Viewer: BIcraft app (egui, desktop and browser)** | **47 of 56 (84%)** done, weighted 92% | **measured**: `cargo xtask parity` over [parity-checklist.md](parity-checklist.md) (each row's status is a judgement against the evidence below) |
| **Desktop UI controls covered** | **71%** of 510 in-scope controls (223 done, 283 partial, 4 missing; 93 groups, 538 controls read, 28 out of scope) | **measured**: `cargo xtask parity` over the inventory of [parity-checklist.md](parity-checklist.md) (controls counted in Desktop as a black box; each group's status a judgement) |
| Engine commands | **299** | **measured**: `cargo xtask parity` counts the `COMMANDS` table of `crates/engine/src/commands.rs` |
<!-- /HEADLINE -->

| Number | Value | Kind |
|---|---|---|
| Render fidelity, synthetic report (27 visual families) | mean similarity **0.981**, ink **0.79** | **measured**: `cargo xtask render-compare` against captures of Power BI Desktop 2.158 ([render.md](render.md), "Results") |
| Render fidelity, a real report | mean similarity **0.959**, ink **0.45** | **measured**: same oracle ([render.md](render.md), "Results") |
| Reader interactions | **180 / 180** visuals and **12 / 12** slicer tick sets after 20 click states; **171 / 171** visuals over 19 file states | **measured**: `cargo test -p xtask interaction` against the interaction oracle ([interaction-oracle.md](interaction-oracle.md), [gaps.md](gaps.md) G29) |
| Ready for real work, for an agent (CLI, MCP) | **~70%** (2026-11-18) | **estimated**: [readiness](#readiness) below |
| Ready for real work, for a person in the app | **~70%** (2026-11-18) | **estimated**: [readiness](#readiness) below |
| Remaining effort, open lots of the top 20 | **~170–330 Opus 5.5 agent hours** (all 20: ~370–660 h) | **estimated**: [gap review](#top-20-missing-features-as-a-proposed-order-of-lots) above |

Read the authoring share as "BIcraft can do it by command", except in the Report UI and Model
UI areas, which score Desktop's dedicated surfaces. Since M11 the app reaches every command, most
of them through the generic parameters form; the Button column of the checklist says which.

## Target and how it was measured

- **Target: the feature list.** Desktop's authoring and reading features as Microsoft Learn
  documents them (pages in [Sources](#sources-microsoft-learn) below). Publishing to the service,
  sign-in, Fabric items, Copilot and service-only reader features are out of scope (rows marked
  O). No Desktop run was made for this assessment; the fidelity and interaction figures above
  come from oracles recorded earlier, black box (AGENTS.md).
- **BIcraft: the code on `main` at ccb96e3** (first assessment: 132b199; rows the later lots
  closed rescored on 816a4cb, 2026-11-18). The engine commands of
  `crates/engine/src/commands.rs` (162 at ccb96e3, 198 at 816a4cb, counted by `cargo xtask
  parity`), their parameters in
  [mcp.md](mcp.md), the renderer ([render.md](render.md)), the HTML viewer and its snapshot
  ([view-snapshot.md](view-snapshot.md)) and the egui app's parity checklist
  ([webui.md](webui.md)).
- **Scoring.** D 1, P 0.5, M 0; O and "not applicable" are left out; P0 rows weigh 3, P1 2, P2 1.
  An object BIcraft reads, keeps on save and follows through renames, but cannot create or
  change, is **M** for authoring ("kept" in the Commands column): preserving a bookmark is not
  authoring one. A row is D only when a command does what Desktop's feature does for the common
  case; depth beyond that (every option of a dialog) is not scored row by row.
- **Buttons.** "Button" is whether the egui app reaches the command, and how. Since M11.3 the
  test `every_command_has_a_button` holds every command id to a ribbon, menu or palette entry of
  `crates/ui` ([commands-ui.md](commands-ui.md)), so every row with a command says `yes`; the
  parenthesis names the surface (a pane, the formula bar, the page tabs, the File view, a menu)
  or `form` when the button opens the generic parameters form (`crates/ui/src/form.rs`). Read
  from `crates/ui/src` and [design.md](design.md) §12 on 2026-11-05. (On 2026-11-04 it was read
  from `apps/bicraft-desktop/src`: two commands besides reader clicks and reloads.)
- **Not measured.** Depth of each feature against Desktop's dialogs, usability, and any
  workflow end to end with a real author. Speed is never compared with Desktop here (its licence
  forbids publishing benchmark results,
  [interoperability.md](interoperability.md#performance-figures)).

## Authoring axis

What the checklist says, by group (the per-area table is in
[parity-checklist.md](parity-checklist.md)); updated 2026-11-18:

- **Semantic model: strong.** Tables, columns, calculated columns and tables, measures with
  dynamic format strings, hierarchies, relationships, calculation groups, RLS roles, translations,
  model settings, undo and redo, auto-recovery, renames that rewrite DAX, copying objects between
  files. Missing: field parameters, what-if parameters, groups and bins, quick measures, OLS,
  perspectives, storage modes, incremental refresh policies and user-defined aggregations (all
  kept, none authorable), autodetected relationships, user-defined functions only by import.
- **Power Query and data: weak.** M text is authored and checked (`partition.*`,
  `expression.*`, `m.validate`, the form's M editor); BIcraft's own refresh runs local file
  sources and the Databricks connector on PBIP projects and, since M11.18, on PBIX files into
  their `DataModel` (`model.refresh`); a query's first rows are previewed (`query.preview`,
  M11.17). Missing: the step editor, Get data's Navigator and an Enter data grid (G58, G59), data
  profiling, query dependencies and diagnostics, privacy levels,
  every database and online connector other than Databricks (named, never run:
  [data-sources.md](formats/data-sources.md)), DirectQuery.
- **Views: partial.** DAX query view is covered by command (`dax.evaluate`, `dax.validate_query`,
  a sortable result grid); TMDL is written in projects but there is no apply-a-script command;
  the Data view browses a table's rows with sort and filter (`table.rows`, M11.17); the model
  view is a read-only diagram; no performance analyzer.
- **Files: done.** PBIX, PBIT, PBIP and model.bim open, save and convert both ways (G1–G3 in
  [gaps.md](gaps.md)); legacy layouts are upgraded to PBIR. Power Pivot import is missing.
- **Report: strong.** Pages, visuals, field wells (the Data and Build panes with drag and drop),
  type changes, summarization, formatting by group or by any property name, conditional
  formatting, analytics lines, small multiples, sorts, visual, page and report filters (the
  Filters pane), themes, interactions, drillthrough and tooltip pages, mobile layout and report
  measures are commands; since 2026-11-06 copy and paste of visuals, grouping, page backgrounds,
  rich text boxes, images and button actions (M11.16, M11.20), since 2026-11-17 bookmarks and
  sync slicer groups (M11.24, M11.25). Missing: show value as, show items with no data (G57),
  error bars and the other analytics (G56), custom visual import, visual calculations, the Gantt
  chart, the theme gallery, most report settings (G63); buttons are partial (no Insert › Shapes,
  hover and press states not drawn).
- **Report UI and Model UI (new 2026-11-05): mostly done for the report, partial for the
  model.** The panes, the DAX formula bar, canvas editing, the Format visual pane with its fx
  buttons and the Selection, Bookmarks and Sync slicers panes are Desktop's (M11.16–M11.25); the
  Analytics pane is the generic form and the conditional formatting dialog covers gradient,
  rules and field value on some colours; the mobile layout view, an editable model diagram, the
  Model explorer (G61) and the in-place properties pane are missing.
- **Accessibility, localisation, tools: partial.** The app's chrome has an AccessKit tree, visuals
  do not; no high contrast; English and French chrome (Desktop ships 40+ languages); no external
  tools endpoint.

## Viewer axis

- **Visuals.** The common built-in families are drawn by one renderer (`bicraft-render`), used
  by `bicraft render` and by the app in both shells; the HTML viewer draws the same families (the
  renderer is a port of its drawing, and the oracles measured the renderer, not page.html).
  Decomposition trees are drawn and clickable in the renderer and the app (G34), not in
  page.html; key influencers are a placeholder with an opt-in estimate (G33). Partial: ribbon
  charts (drawn, not compared; page.html stacks columns), matrices (subtotals at every level and
  the saved expansion drawn; the reader's expand and collapse in the app only, M11.19), measures
  that use window functions, balances or `GROUPBY` (not evaluated). The bookmark navigator is
  drawn by the renderer and the app since 2026-11-18 (as text with an accent mark, not in
  Desktop's button styles), not by page.html. Missing: maps, Q&A, smart narrative, the Gantt
  chart, custom visuals.
- **Reader interactions.** Slicers, cross-filtering and cross-highlighting, synced slicers,
  edit-interactions rules, default tooltips, page navigation and slicer buttons work in the web
  viewer and the app and match the interaction oracle. Drill, drillthrough and report page
  tooltips (M11.19, documented, not measured) and bookmarks with their navigator and buttons
  (2026-11-18) work in the app, not in page.html. Missing everywhere: point selection in
  matrices, scatter, waterfall and cards, view as a role.
- **Reader tools.** The app has the Filters pane (editors change filters; a server-mode reader
  sees it read-only), a right-click Sort, the visual header's More options and drill icons, See
  records, and export to PDF and of a visual's data (M11.21). Missing everywhere: focus mode,
  spotlight, Show as a table, print, phone layout, high contrast. The web viewer's visual details
  panel (30 preview rows) is its only "show as a table"; its hit regions take keyboard focus, the
  app's do not.
- **`bicraft render`** has no reader of its own (rows marked "–"), but draws the reader state
  set through `view.interact`, renders every page to PNG, at any scale, deterministically, on
  Windows and Linux, which Desktop does not offer outside export to PDF.

## Biggest missing areas (2026-11-04)

Superseded by the [gap review](#gap-review-2026-11-05) above; kept as the first assessment's
list. Its first line (buttons, M11.1–M11.3) and the Filters pane are done.

Ordered by the share they would add (priority weight × rows), with effort in Opus 5.5 agent
hours. All **estimated**, calibrated on the work logged in [ROADMAP.md](../ROADMAP.md) (a command
with tests and docs 2–6 h, a reader feature on three surfaces 6–15 h, a subsystem 20–60 h).

| Area | Rows | Effort | Notes |
|---|---|---|---|
| Buttons for the authoring commands in the app (M11.1–M11.3) | every authoring row but two | 30–50 | placement table, palette with a parameters form, coverage test; one UI for both shells |
| Reader tools: filters pane, focus mode, spotlight, header sort, show as table, keyboard | 7 viewer rows | 25–45 | the filters pane needs page and report filter editing first |
| Drill, drillthrough, report page tooltips, more point selections | 5 viewer rows | 30–50 | drill state in `view.interact`; the evaluator already queries active hierarchy levels |
| Report-wide state: page and report filters, bookmarks, sync slicer groups, page backgrounds | 5 authoring rows, 2 viewer rows | 25–40 | bookmarks are read and copied today; a command set and the reader's apply |
| Model objects kept but not authored: field parameters, what-if, groups and bins, OLS, perspectives, storage mode, refresh policy | 8 authoring rows | 20–35 | each is a small command set over what the readers and writers already keep |
| Connectors and the Power Query step editor | 3 authoring rows | 80–150 | SQL Server first; the step editor is a UI subsystem; every connector runs only when allowed |
| Visual families: maps, AI visuals, custom visuals, bookmark navigator, matrix depth | 5 viewer rows | 70–150 | maps need tiles (network, licensing); custom visuals need a sandboxed JavaScript host |
| Export to PDF and data, print | 3 viewer rows | 15–25 | the renderer already rasterizes; PDF pages and CSV of a visual's query |

## Readiness

Written weights so the numbers can be recomputed (**estimated**):

- **For an agent (CLI, MCP):** authoring weighted parity × 0.6 + `bicraft render` weighted
  parity × 0.2 + file fidelity × 0.2. File fidelity is taken as 0.9: open, save and convert pass
  every round-trip and Desktop-acceptance check recorded in [gaps.md](gaps.md) (G1–G3, G13,
  G17, G18, G26), with Desktop releases other than the ones tested still unseen. 2026-11-05, on
  the widened target: 0.6 × 0.68 + 0.2 × 0.74 + 0.2 × 0.9 ≈ 0.74, times 0.9 because depth per
  row is not scored: **~65%** (on the first assessment's rows: 0.6 × 0.78 + 0.2 × 0.78 + 0.18 ≈
  0.80, × 0.9, still ~70%; 2026-11-04: 0.6 × 0.76 + 0.2 × 0.77 + 0.18, ~70%). 2026-11-18:
  0.6 × 0.78 + 0.2 × 0.80 + 0.2 × 0.9 ≈ 0.81, × 0.9 ≈ 0.73: **~70%**.
- **For a person in the app:** the app's viewer weighted parity × 0.4 + authoring weighted parity
  × reach × 0.6, where reach counts an item reached through a dedicated surface as 1 and one
  reached only through the generic form or the palette as 0.5. 2026-11-05: reach (40 + 0.5 × 49)
  / 89 ≈ 0.72, so 0.4 × 0.65 + 0.6 × 0.68 × 0.72 ≈ 0.55, rounded down because the weights
  under-count the canvas and Format pane holes: **~50%**. (2026-11-04, with the share of items
  that had a button instead of the reach: 0.4 × 0.62 + 0.6 × 0.03, ~25%.) 2026-11-18: the Button
  column read again (a row whose parenthesis names only `ribbon`, `form`, `palette` or `JSON`
  counts 0.5): 73 dedicated and 26 form-only of 99, reach (73 + 0.5 × 26) / 99 ≈ 0.87, so
  0.4 × 0.81 + 0.6 × 0.78 × 0.87 ≈ 0.73, rounded down because depth per row is not scored (as
  for the agent): **~70%**.

## Sources (Microsoft Learn)

Desktop's feature list, by checklist area (pages read on 2026-11-04; the last row on 2026-11-05):

| Area | Pages |
|---|---|
| Model view, tables, columns, measures | [Model view](https://learn.microsoft.com/en-us/power-bi/transform-model/desktop-modeling-view), [Calculated tables](https://learn.microsoft.com/en-us/power-bi/transform-model/desktop-calculated-tables), [Calculated columns](https://learn.microsoft.com/en-us/power-bi/transform-model/desktop-calculated-columns), [Measures](https://learn.microsoft.com/en-us/power-bi/transform-model/desktop-measures), [Quick measures](https://learn.microsoft.com/en-us/power-bi/transform-model/desktop-quick-measures), [Sort by column](https://learn.microsoft.com/en-us/power-bi/create-reports/desktop-sort-by-column), [Date tables](https://learn.microsoft.com/en-us/power-bi/transform-model/desktop-date-tables), [Auto date/time](https://learn.microsoft.com/en-us/power-bi/transform-model/desktop-auto-date-time), [Groups and bins](https://learn.microsoft.com/en-us/power-bi/create-reports/desktop-grouping-and-binning), [Dynamic format strings](https://learn.microsoft.com/en-us/power-bi/create-reports/desktop-dynamic-format-strings), [DAX user-defined functions](https://learn.microsoft.com/en-us/dax/best-practices/dax-user-defined-functions) |
| Relationships, calculation groups, parameters | [Relationships](https://learn.microsoft.com/en-us/power-bi/transform-model/desktop-create-and-manage-relationships), [Calculation groups](https://learn.microsoft.com/en-us/power-bi/transform-model/calculation-groups), [Field parameters](https://learn.microsoft.com/en-us/power-bi/create-reports/power-bi-field-parameters), [What-if parameters](https://learn.microsoft.com/en-us/power-bi/transform-model/desktop-what-if), [Query parameters](https://learn.microsoft.com/en-us/power-query/power-query-query-parameters) |
| Security and metadata | [Row-level security](https://learn.microsoft.com/en-us/fabric/security/service-admin-row-level-security), [Object-level security](https://learn.microsoft.com/en-us/fabric/security/service-admin-object-level-security), [Perspectives](https://learn.microsoft.com/en-us/analysis-services/tabular-models/perspectives-ssas-tabular), [Multiple languages](https://learn.microsoft.com/en-us/power-bi/guidance/multiple-language-locale), [Q&A tooling](https://learn.microsoft.com/en-us/power-bi/natural-language/q-and-a-tooling-intro) |
| Storage, Power Query, data | [Storage mode](https://learn.microsoft.com/en-us/power-bi/transform-model/desktop-storage-mode), [Composite models](https://learn.microsoft.com/en-us/power-bi/transform-model/desktop-composite-models), [Incremental refresh](https://learn.microsoft.com/en-us/power-bi/connect-data/incremental-refresh-overview), [Power Query UI](https://learn.microsoft.com/en-us/power-query/power-query-ui), [Data sources](https://learn.microsoft.com/en-us/power-bi/connect-data/desktop-data-sources), [Refresh](https://learn.microsoft.com/en-us/power-bi/connect-data/refresh-data) |
| Views | [DAX query view](https://learn.microsoft.com/en-us/power-bi/transform-model/dax-query-view), [TMDL view](https://learn.microsoft.com/en-us/power-bi/transform-model/desktop-tmdl-view), [Data view](https://learn.microsoft.com/en-us/power-bi/transform-model/desktop-data-view), [Performance analyzer](https://learn.microsoft.com/en-us/power-bi/create-reports/desktop-performance-analyzer) |
| Files | [Projects (PBIP)](https://learn.microsoft.com/en-us/power-bi/developer/projects/projects-overview), [Project report folder](https://learn.microsoft.com/en-us/power-bi/developer/projects/projects-report), [Project semantic model folder](https://learn.microsoft.com/en-us/power-bi/developer/projects/projects-dataset), [Enhanced report format (PBIR)](https://learn.microsoft.com/en-us/power-bi/developer/embedded/projects-enhanced-report-format), [Templates](https://learn.microsoft.com/en-us/power-bi/create-reports/desktop-templates), [Report and model separation](https://learn.microsoft.com/en-us/power-bi/guidance/report-separate-from-model) |
| Pages, visuals, formatting | [Report view](https://learn.microsoft.com/en-us/power-bi/create-reports/desktop-report-view), [Page display settings](https://learn.microsoft.com/en-us/power-bi/create-reports/power-bi-report-display-settings), [Visualization types](https://learn.microsoft.com/en-us/power-bi/visuals/power-bi-visualization-types-for-reports-and-q-and-a), [Tables](https://learn.microsoft.com/en-us/power-bi/visuals/power-bi-visualization-tables), [Matrix](https://learn.microsoft.com/en-us/power-bi/visuals/power-bi-visualization-matrix-visual), [Slicers](https://learn.microsoft.com/en-us/power-bi/visuals/power-bi-visualization-slicers), [Filled maps](https://learn.microsoft.com/en-us/power-bi/visuals/power-bi-visualization-filled-maps-choropleths), [Decomposition tree](https://learn.microsoft.com/en-us/power-bi/visuals/power-bi-visualization-decomposition-tree), [Q&A visual](https://learn.microsoft.com/en-us/power-bi/visuals/power-bi-visualization-q-and-a), [Smart narrative](https://learn.microsoft.com/en-us/power-bi/visuals/power-bi-visualization-smart-narrative), [Custom visuals](https://learn.microsoft.com/en-us/power-bi/developer/visuals/power-bi-custom-visuals), [Import a visual](https://learn.microsoft.com/en-us/power-bi/developer/visuals/import-visual), [Conditional formatting](https://learn.microsoft.com/en-us/power-bi/create-reports/desktop-conditional-table-formatting), [Analytics pane](https://learn.microsoft.com/en-us/power-bi/transform-model/desktop-analytics-pane), [Small multiples](https://learn.microsoft.com/en-us/power-bi/visuals/power-bi-visualization-small-multiples), [Buttons](https://learn.microsoft.com/en-us/power-bi/create-reports/desktop-buttons), [Visual calculations](https://learn.microsoft.com/en-us/power-bi/transform-model/desktop-visual-calculations-overview), [Accessibility](https://learn.microsoft.com/en-us/power-bi/create-reports/desktop-accessibility-creating-reports), [Themes](https://learn.microsoft.com/en-us/power-bi/create-reports/desktop-report-themes) |
| Filters, interactions, mobile | [Filters](https://learn.microsoft.com/en-us/power-bi/create-reports/power-bi-report-filter), [Filter types](https://learn.microsoft.com/en-us/power-bi/create-reports/power-bi-report-filter-types), [Filters and highlighting](https://learn.microsoft.com/en-us/power-bi/create-reports/power-bi-reports-filters-and-highlighting), [Visual interactions](https://learn.microsoft.com/en-us/power-bi/create-reports/service-reports-visual-interactions), [Bookmarks](https://learn.microsoft.com/en-us/power-bi/create-reports/desktop-bookmarks), [Drillthrough](https://learn.microsoft.com/en-us/power-bi/create-reports/desktop-drillthrough), [Report page tooltips](https://learn.microsoft.com/en-us/power-bi/create-reports/desktop-tooltips), [Mobile layout](https://learn.microsoft.com/en-us/power-bi/create-reports/power-bi-create-mobile-optimized-report-about), [Personalize visuals](https://learn.microsoft.com/en-us/power-bi/create-reports/power-bi-personalize-visuals) |
| Reading view | [Drill](https://learn.microsoft.com/en-us/power-bi/consumer/end-user-drill), [Focus mode](https://learn.microsoft.com/en-us/power-bi/consumer/end-user-focus), [Spotlight](https://learn.microsoft.com/en-us/power-bi/consumer/end-user-spotlight), [Show data](https://learn.microsoft.com/en-us/power-bi/consumer/end-user-show-data), [Change sort](https://learn.microsoft.com/en-us/power-bi/consumer/end-user-change-sort), [Export data](https://learn.microsoft.com/en-us/power-bi/visuals/power-bi-visualization-export-data), [Export](https://learn.microsoft.com/en-us/power-bi/consumer/end-user-export), [Accessibility for readers](https://learn.microsoft.com/en-us/power-bi/create-reports/desktop-accessibility-consuming-tools) |
| Latest features (gap review) | [What's new, October 2026](https://learn.microsoft.com/en-us/power-bi/fundamentals/whats-new) (Gantt chart preview, dropdown list slicer, plot area fill, auto-expanding hierarchy axes with field parameters, mobile page indicator), read 2026-11-05; the Report UI, Model UI, Power Query depth, accessibility, localisation and tools rows also draw on the reviewer's knowledge of Desktop's documented panes (Selection, Bookmarks, Sync slicers, Format, Analytics, Performance analyzer, Query dependencies, data profiling, query diagnostics, external tools), not re-read this time |

## Evidence (BIcraft)

| Area | Tests and docs |
|---|---|
| Model commands | `crates/engine/src/pbix_struct_tests.rs`, `edit_tests.rs`, `model_props_tests.rs`, `partition_tests.rs`, `roles_tests.rs`, `translations_tests.rs`, `perspectives_tests.rs`, `history_tests.rs`, `import_tests.rs`; [mcp.md](mcp.md) "Other editing commands", "Model properties" |
| DAX and queries | `crates/dax/tests/oracle.rs`, `crates/engine/tests/dax_coverage`, `dax_query_tests.rs`; [dax-oracle.md](dax-oracle.md) |
| Power Query and refresh | `mquery_tests.rs`, `refresh_tests.rs`, `sources_tests.rs`, `envprofile_tests.rs`; [formats/data-sources.md](formats/data-sources.md) |
| Files | `save_tests.rs`, `convert_tests.rs`, `crates/engine/tests/real_reports.rs` (local files only); [formats/packaging-verification.md](formats/packaging-verification.md) |
| Report commands | `report_tests.rs`, `report_apply_tests.rs`, `visual_props_tests.rs`, `visual_series_tests.rs`, `visual_conditional_tests.rs`, `visual_extras_tests.rs`, `theme_edit_tests.rs`, `layout_tests.rs`; [mcp.md](mcp.md) "Visual properties", "Declarative reports" |
| Viewer | `crates/render/src/tests.rs`, `crates/render/tests/golden.rs`, `interact_tests.rs`, `xtask/src/oracles/interactions_tests.rs`, `apps/bicraft-desktop/src/tests.rs`; [render.md](render.md), [view-snapshot.md](view-snapshot.md), [webui.md](webui.md) |
| App (buttons, panes, formula bar) | `crates/ui/src/tests/ribbon_tests.rs` (`every_command_has_a_button`), `crates/ui/src/tests/{palette_ui_tests,panes_ui_tests,filters_ui_tests,formula_ui_tests,recovery_tests}.rs`; [commands-ui.md](commands-ui.md), [design.md](design.md) §12 |

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-11-23 | minor | Desktop UI inventory (M11.28) read from Desktop 2.159.1231.0 as a black box; its control-weighted ledger in the headline (M11.29) |
| 2026-11-05 | major | Gap review: rows rescored on ccb96e3, target widened to all of Desktop (Report UI, Model UI, Power Query depth, storage, accessibility, localisation, tools), buttons read from `crates/ui`, top 20 lots, full table by area, readiness formulas revised |
| 2026-11-04 | major | First assessment (M11.7): two axes, generated checklist and headline (`cargo xtask parity`) |
