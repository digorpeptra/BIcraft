# Known gaps: closed entries and history

Moved from `docs/gaps.md` on 2026-11-25 (relative links adjusted to this folder): the introduction and summary table as they stood
that day, then the entries of the closed gaps. Open gaps: [gaps.md](../gaps.md).

## Introduction as of 2026-11-25


This is a ranked list of what BIcraft cannot do yet, does slowly, or has not verified. It was
collected on 2026-10-13 (M7.4) from these sources:

- `ROADMAP.md`: ⬜ and 🔄 items
- the "Open questions" and "Known gaps" sections of `docs/formats/*.md`
- `docs/dax-oracle.md`: disagreements with the engine
- the crate docs: the module headers of `crates/engine`, `crates/pbix`, `crates/data` and
  `crates/metadata`, and `crates/xpress9/README.md`
- `docs/mcp.md`
- the benchmark hot spots in [benchmarks.md](../benchmarks.md)

Several milestones were moving while this list was written (M2.4, M5, M7.x, M8), so re-check an
item against `ROADMAP.md` before you start on it.

**Priority** is the impact on users:

| Priority | Meaning |
|---|---|
| **P0** | Blocks a common workflow. |
| **P1** | Hurts a common workflow, or blocks a less common one. |
| **P2** | Hurts a less common workflow, or is a correctness risk that has not been observed yet. |
| **P3** | Nice to have. |

**Effort**: **S** is about a day or less, **M** is a few days, **L** is a week or more.

## Summary as of 2026-11-25

| # | Gap | Priority | Area | Effort | Depends on |
|---|---|---|---|---|---|
| G1 | ~~Structural edits of a PBIX (tables, columns, relationships, data) are not reachable from the CLI or MCP~~ **Fixed** 2026-10-13 | P0 | engine, pbix | L | — |
| G2 | ~~Report edits are not saved into a PBIX or PBIT~~ **Fixed** 2026-10-13 | P0 | engine, pbix, layout | M | — |
| G3 | ~~Conversions PBIX → PBIP and PBIP ↔ PBIT are not reachable~~ **Fixed** 2026-10-14 | P1 | engine, pbixwriter | M | — |
| G4 | ~~DAX validator false accepts (48 of 583 in the last recorded run)~~ **Fixed** 2026-10-13: 916/917 agree, 1 false accept, 0 false rejects | P1 | dax | M | — |
| G5 | ~~`model.validate` and DAX binding are quadratic in the number of measures~~ **Fixed** 2026-10-13 | P1 | model, dax | S | — |
| G6 | ~~Saving a PBIX re-compresses the whole `DataModel`; the XPress9 encoder runs at 5–25 MB/s~~ **Fixed** 2026-10-14: parallel primed chunks, 200–300 MB/s, 32 MB save 11 s → 0.9 s | P1 | xpress9, pbix | M–L | — |
| G7 | ~~No undo or redo, and `batch` is not transactional~~ (done, M1.6) | P1 | engine, mcp | M | — |
| G8 | No published binaries; CI and release workflows have never run (prepared only, nothing is published by policy); ~~`bicraft setup`~~ done (M7.2) | P1 | repo, cli | S–M | publishing decision |
| G9 | ~~Reader fuzzing is in progress~~ **Done** 2026-10-13 (M7.3): smoke tests in CI + cargo-fuzz targets, findings F1–F4 fixed | P1 | all readers | M | — |
| G10 | VertiPaq and ABF layouts of large models — fixed (engine oracle up to 20M rows; incremental refresh, hybrid tables and rare layouts read, kept and written); storage mode and incremental refresh policies authored (2026-11-19), DirectQuery not executed (visuals say "not evaluated: DirectQuery") | P1 | vertipaq, abf | — | — |
| G11 | ~~PBIR schemas are re-parsed on every validation~~ **Fixed** 2026-10-14: compiled once per process, `validate_documents` 1.6–8× faster | P2 | pbir | S | — <!-- perf-guard: ok (BIcraft before and after) --> |
| G12 | Thin PBIX opened in Desktop 2.159.1231.0 (2026-11-23), signed out: the package is read (a live connection to an unreachable Analysis Services server gets Desktop's "cannot connect" dialog), but the `pbiServiceXmlaStyleLive` `powerbi://` connection BIcraft writes fails with a null-reference error and Desktop opens a blank untitled report; the open signed in, against a published model, is **not pursued, by the maintainers' decision (2026-11-24)**: a known limitation; BIcraft reads and edits a thin report's report and reports its live connection (`file.open` `live_connection`, the model error and the Model view say so) | P3 | pbixwriter, engine | — (not pursued) | — |
| G13 | ~~Legacy layout → PBIR is not checked against Desktop's own conversion~~ **Fixed** 2026-10-15: Desktop 2.158 oracle on 8 generated reports, 0 differences after the fixes; left: phone formatting source, visual-object descriptor filtering, image GC, report-model gaps | P3 | layout, report | S | — |
| G14 | Report validation leftovers: date variation names and their date-table columns are checked, and legacy-layout report measures are added, edited and checked (2026-11-24), but neither was tried in Desktop; a scatter chart with the same aggregation in two roles and a one-to-one single-direction relationship are not checked; filters, hierarchies, bookmarks, data roles (incl. condition kinds, related slicer fields, Q&A/AI/scorecard) and report measures are checked (Desktop-calibrated) | P3 | engine | S | — |
| G15 | ~~Renames and removals do not rewrite DAX~~ **Fixed** 2026-10-14 | P2 | engine, dax | M | — |
| G16 | ~~`metadata.sqlitedb` objects that are not mapped~~ **Mostly fixed** 2026-10-14: sets, binding infos, analytics AI metadata, excluded artifacts, level-1000000 properties engine-confirmed; secrets kept opaque byte for byte; Direct Lake mapped from TOM (Desktop's engine refuses it) | P2 | metadata | S | Fabric sample |
| G17 | ~~Writer behaviour Desktop has not confirmed (XPress9 framing and seed, CRC markers, schema drift between releases)~~ **Mostly fixed** 2026-10-15: framing, seed, chunk limit (2 MiB, now enforced), `ErrorCode`, restore checks settled on Desktop 2.158 and its engine; `DictionaryStorage.Size` written as the engine computes it (hash collisions excepted); left: other Desktop releases, Excel CRC markers | P2 | xpress9, abf, metadata, vertipaq | varies | Other Desktop releases |
| G18 | ~~Open PBIR/PBIP questions about what Desktop accepts; the JSON Schema validator implements only a subset~~ **Fixed** 2026-10-14: questions settled with Desktop 2.158, draft-07 validator with a regex engine | P2 | pbir, pbip | S each | — |
| G19 | ~~Gaps in the DAX type vocabulary (flattened multi-section functions; 8 arity deviations)~~ **Mostly fixed** 2026-10-13: repeat groups, new kinds, 5 justified deviations left; filter modifiers typed and placed 2026-10-14 | P2 | dax | M | — |
| G20 | ~~Runtime-only DAX errors (22 in the corpus) are not reported, not even as warnings~~ **Fixed** 2026-10-13: 109 of 110 flagged (`runtime-*` warnings, 2026-10-14), no false warning | P2 | dax | S–M | — |
| G21 | ~~`bicraft-data` evaluates literals only~~ **Mostly fixed** 2026-10-14: a sandboxed M evaluator checked against Desktop (36 of 36 test tables identical), folder sources, DAX calculated tables over model data checked against the engine (66 of 66 probes); 2026-10-23: **Databricks connector** run only when allowed (SQL Statement Execution API, query folding of selections, simple filters and limits) and `model.refresh` into BIcraft's own data snapshot, tested against a local fake of the API, not yet against a workspace; other connectors still reported and never run; M right/full joins and a few conversions remain | P2 | data, databricks, engine | M | — |
| G22 | Patent and OSP coverage of MS-XLDM and the MIT XPress9 code: **researched** 2026-10-14 (maintainers' private notes); counsel review and Patent Map check still open | P2 | legal | — | — |
| G23 | ~~Process start and small-file I/O are 5–10× slower on Windows than on Linux~~ **Done** 2026-10-14: I/O 2–4× faster; process start is the OS floor | P3 | cli, pbir, tmdl | S | — |
| G24 | ~~Edge cases in the TMDL writer and reader~~ **Fixed** 2026-10-14: TOM ↔ BIcraft TMDL round trip verified against TOM (development oracles) | P3 | tmdl | S | — |
| G25 | Unsupported SQLite features (none are used by `metadata.sqlitedb`) — WAL, freelist trunks, auto-vacuum, UTF-16, every page size **checked** 2026-10-14 (101-database SQLite differential, no fix needed) | P3 | sqlite | S–M | — |
| G26 | ~~Open questions about the PBIX package (content types, Zip64, custom visuals, Copilot parts)~~ **Fixed** 2026-10-14: settled on Desktop 2.158; Zip64 writer, `report.custom_visuals`, Copilot containers (layout, encoding, carried on convert, `inspect`), packages over 4 GiB (deferred parts, streamed model read); only the engine's own > 4 GiB output unseen | P3 | opc, pbix | S each | — |
| G27 | ~~In-flight UX milestones (M2.4, M8.1, M8.2)~~ **Done** 2026-10-14 | P3 | report, view | M–L | — |
| G28 | ~~Stale or missing documentation~~ **Fixed** 2026-10-14 | P3 | docs | S | — |
| G29 | Viewer interactivity (M8.3, 2026-10-23): slicers, cross-filtering and highlighting, Ctrl+click, `visualInteractions`, synced slicers, tooltips, navigation, Apply / Clear all slicers **done** in `bicraft view`, aligned on the Desktop 2.159 interaction oracle (click states 180/180 visuals, 12/12 slicer ticks; file states 171/171); left: highlight by two evaluations (no `IGNORE` yet), multi-table multi-point superset, no points in matrices / scatter / waterfall / small multiples (drill, drillthrough and report-page tooltips: G51, 2026-11-06; bookmarks applied by the readers since 2026-11-18: G55), `mcp --view` read-only; since 2026-11-24 BIcraft's ten visuals interact in the app and the viewer (a click, Ctrl+click and the keyboard select a mark's rows and filter the page, other visuals' selections and slicers filter them, a bookmark keeps their selection as the container's `highlight`, the viewer draws them from the renderer's image and hit map); since 2026-11-24 See records of a mark that stands for several points lists the rows of every point (`visual.records {points}`) and the org chart's collapse buttons are clickable (saved by `visual.toggle_node` in the app, the reader's view state in the viewer, `view.interact` `tree_toggle`); left there: the `highlight` written is the public schema's shape, not yet compared with Desktop's | P2 | engine, view | M | `IGNORE` in the DAX evaluator |
| G30 | DAX evaluator coverage: **mostly fixed** 2026-10-23 (`feat/dax-coverage`): ~90 functions added, calculation groups, dynamic format strings, local/UTC clock, `IGNORE`, checked on Desktop 2.159 (`tests/dax_coverage`: 0 wrong value); window functions, balances, `GROUPBY` and natural joins evaluated since 2026-11-18 (lot 20), `ADDMISSINGITEMS`, `SUBSTITUTEWITHINDEX`, `SAMPLE`, `TOPNSKIP` since 2026-11-20; left: the `ADDMISSINGITEMS` subtotal form, `TOPNSKIP` ties, the `*WEEK` balances, visual-calculation forms, wildcards, time zone without `utc_offset_minutes` | P1 | engine | M | — |
| G31 | DAX evaluation speed on large models: **fixed for the bench** 2026-10-24 (`feat/storage-engine`): storage engine under the evaluator (`docs/storage-engine.md`), 22/22 bench queries identical to Desktop from 10 k to 100 M rows in each regime, none refused; left: grouped scans at 100 M, one row per group for very high cardinalities, result serialization, data beyond RAM; next: halving the times (2026-11-19: column-store techniques, ×1.2 to ×2.3 measured at 10 M on the cases they target), beyond RAM (aggregations: G32) | P1 | engine | M | — |
| G32 | Report-derived aggregations: **done for the bench** 2026-10-24 (`feat/report-aggregations`, `docs/storage-engine.md` "Aggregations"): derived from the report's visuals, filters, slicers and measures, chosen under a size rule, built by a background job off the query path, used only when exactly covering; bench cold median about twice as fast with them at 1 M and about nine times as fast at 100 M, start and warm unchanged, 22/22 identical; left: distinct counts under other filters (Q07, Q18), memory budget vs foreground build at 100 M, not kept across sessions, no eager build at `model.refresh`; a report edit keeps the aggregations still selected (10 M, one visual added: 0.75 s instead of 1.55 s), the re-run selection is most of that | P1 | engine | M | G31 |
| G34 | Decomposition trees: **first lot done** 2026-11-04 (`feat/decomposition-tree`): evaluated level by level from the saved state (AI splits re-chosen, sorting, blanks, filters), drawn with Desktop 2.159's geometry, selection and path changes in the viewer; checked on 23 trees recorded in Desktop; left: page.html box, the "+" popup in the egui UI, scroll arrows, a few options | P2 | engine, render | S | — |
| G35 | ~~Every command has its button (M11)~~ **Fixed** 2026-11-05 (`feat/m11-ribbon`, `feat/m11-palette`, `feat/ui-forms-polish`): the registry's tabs, groups and buttons with the session's enabled state, contextual tabs, menus, shortcuts, the command palette (and the title row's search button), the parameters form of any command with a picker for every object kind and the commands' defaults, answers as sortable grids and key/value trees with a JSON toggle, the browser's keyboard (F5 kept, New on Alt+N) | P2 | ui | S | — |
| G36 | Browser document tabs as parallel views: **done** 2026-11-05 (`feat/webui-parallel-views`): each workspace file's tab is a document view on the server with a session of its own (snapshot, events, commands, saves), per-file save check, caps per user and per workspace, idle expiry, memory check, readers' read-only views; Helm and controller values and `/metrics` gauges since `chore/deploy-followups`; left: a tab closed by the browser (not the window) keeps its view until the idle time (its unsaved edits stay in the recovery journal, on by default in the charts) | P2 | mcp, ui, web | S | — |
| G37 | Calendar math through a date crate (M11.9): **done** 2026-11-05 (`feat/m11-chrono`), on `jiff` instead of the unmaintained `chrono` since 2026-11-23 (M11.43): DAX, M, `bicraft-locale` and the readers and writers go through `bicraft_locale::calendar` (jiff), held to the old answers by a differential golden test; left: the timestamps of `bicraft-controller`, of the MCP HTTP request log and file listing and of the UI's Backstage keep their own civil-from-days copies, and the `engine-bench` data generator its own calendar | P3 | controller, mcp, ui | S | — |
| G38 | Dependencies left behind by `cargo deny` (M11.5, 2026-11-05): `zip` 9 and skrifa / harfrust landed (no advisory ignored); left `colored` (MPL-2.0, test-only); no issue forms; the HTTP test's queued gauge counts a call before it reaches the lock | P3 | repo, opc, render | S–M | — |
| G40 | Auto-recovery and autosave: **done** 2026-11-05 (`feat/auto-recovery`): a journal of the edits since the last save (desktop: per-user folder; server: `--recovery-dir`), offered back when the file is opened again (prompt, File view, status bar), recovery copies for outside inputs, opt-in autosave of PBIP projects; the charts set `--recovery-dir` on a claim of their own and `/metrics` counts the journals (`chore/deploy-followups`); left: a new model never saved is not journaled, view settings are not recovered, autosave saves the active tab only | P2 | engine, mcp, ui | S | — |
| G42 | Report metadata as fields: **done** 2026-11-05 (`feat/refresh-metadata-fields`): `INFO.*` and `INFO.VIEW.*` evaluated with the refresh times, query-only ones refused outside queries, an optional Refresh info table with a `Last refresh` measure; `DateTime.LocalNow()` is the machine's local time on the desktop and in the CLI, UTC on a server unless `--local-offset` (`chore/deploy-followups`); left: measuring the documented rules and columns in Desktop (`tools/info-oracle`) | P2 | dax, engine | S | — |
| G43 | Report text glyphs: **mostly fixed** 2026-11-05 (`fix/render-glyph-fallback`): bundled Noto fallback faces after Selawik (Greek, Cyrillic, currency, superscripts, arrows, math, shapes, box drawing, common emoji); a missing character is drawn as a box and listed by `report.render`; left: CJK not bundled (system fonts or `--font-dir`), emoji monochrome, no per-weight symbols | P2 | render | M (CJK) | — |
| G41 | Editing a visual's fields by hand: **done** 2026-11-05 (`feat/ui-fields-panes`, `feat/ui-filters-pane`): Data, Build and Filters panes, drag and drop, new visual from a field, whole-hierarchy drag, per-visual rename; the Analytics tab landed with G56 (2026-11-18); relative date and time filters and the cards' lock and hide icons landed (M11.13, 2026-11-18), Top N stays visual-only as in Desktop; Filters pane formatting (`page.format_filter_pane`, 2026-11-19; the pane's width not drawn) and card order (`filter.order`, 2026-11-19: drag within a section, A to Z, Z to A, custom); left: readers cannot change unlocked filters for their session, the inferred relative shapes are not checked against Desktop files | P3 | ui, engine | M | — |
| G45 | Editing DAX in a formula bar (M11.14): **done** 2026-11-05 (`feat/ui-formula-bar`): Desktop's formula bar for measures, calculated columns and calculated tables (colours, expand, Enter / Esc, undo, New measure, rename by the name part, autocomplete, signature help, live check, Edit from the Data pane and the diagram, readers read-only); left: calculated tables keep their columns, no commit of invalid DAX, no commit on focus loss, no Format DAX, no function descriptions | P2 | ui, engine | S–M | — |
| G46 | Desktop features missing for parity (gap review 2026-11-05, [target-app-parity.md](../target-app-parity.md); rows rescored 2026-11-18, measured by `cargo xtask parity`): authoring 79 of 129 rows done (61%, weighted 78%), app viewer 38 of 56 (68%, weighted 81%); lots 1–10, 13 and 15 of the review landed (M11.16–M11.25; lot 7 without print, lot 9, its Analytics pane with G56), lot 17 in part; biggest holes now: OAuth sign-in and the privacy firewall (SQL Server, Web, OData and SharePoint landed, G68, G69), the Power Query editor's remaining tools (G67) | P1 | ui, engine, render, data | XL (open lots of the top 20 ~170–330 h, estimated) | the lots of the review |
| G47 | Editing interactions by hand (M11.15): **done** 2026-11-06 (`feat/ui-edit-interactions`): Desktop's Edit interactions mode (Visual tab toggle, Filter / Highlight / None icons on every target with only the valid modes, default marked, one rule per click, Esc, live preview, readers kept out), `report.settings` (cross-filter by default, honoured by the viewer; drilling filters other visuals, stored); left: the icons sit inside the visual's corner (Desktop: over the header), groups are neither sources nor targets, no oracle state for the report setting (reader drill since G51; "Apply drill down filters to" is `visual.drill_filter`) | P2 | ui, engine | S | — |
| G51 | Reader drill, matrix expand and collapse, drillthrough and report-page tooltips (M11.19): **done** 2026-11-06 (`feat/viewer-drill`) in the engine (`view.interact` drill_*, matrix_*, drillthrough, tooltip; `view.keep_drill`), the renderer (concatenated labels, matrix +/-, row-header menus) and the app (header icons, right-click entries, Back, tooltip pages); documented, not measured in Desktop; the authoring side since 2026-11-20: `visual.expand` (+/-, Expand and Collapse › Selection, Entire level, All, the header steps) writes the matrix's `expansionStates` while editing, undoable, round-tripping through PBIP and PBIX, and page.html draws the +/- buttons; left: page.html's drill header icons, cross-report drillthrough, summarized drillthrough fields, the Desktop oracle | P1 | engine, render, ui | S for what is left | the interaction oracle's driver for drill clicks |
| G48 | Canvas editing (M11.16): **done** 2026-11-06 (`feat/ui-canvas-editing`): selection of several visuals (Ctrl+click, rubber band, Ctrl+A), drag to move, eight resize handles, smart guides and Grid snap, arrow nudges, align and distribute, layer order of a set, group and ungroup, copy, paste and duplicate across pages and documents (OS clipboard, browser clipboard, in-app fallback), one command per gesture; since 2026-11-19 group-first clicks, Shift resize keeping the aspect ratio, the menu's Paste at the pointer, Cut (`visual.cut`, Ctrl+X) and no empty group after removals; since 2026-11-20 View › Page options (Gridlines, Snap to grid, Lock objects, independent window preferences), View › Page view (`page.set_display_option`, saved with the page) and the status bar's zoom (fit buttons, slider, zoom in and out); left: no Show smart guides switch (the guides are always on, Alt moves freely), the grid is drawn over the page image (not behind the visuals), Lock objects not compared with Desktop (Learn: no move or resize; BIcraft still deletes and pastes), images of pasted image visuals are not carried to another file, bookmarks and sync groups naming the copied visuals are not copied, snapping thresholds not measured; the Selection pane landed with M11.23 | P2 | ui, engine | S | — |
| G50 | PBIX refresh from local sources (M11.18): **done** 2026-11-06 (`feat/pbix-refresh`): `model.refresh` on an open PBIX runs its import partitions in BIcraft's evaluator and writes the new rows into its `DataModel` (refresh state as Desktop's), selected tables and what reads them, the other tables byte for byte, a verified temporary file renamed atomically, memory reserved against the limit; left: Desktop-open check (maintainers' checklist), rows held in memory per table (not streamed), 16.7 M rows per table | P0 | engine, pbix, ui | M (done) | — |
| G52 | Rich content (M11.20): **done** 2026-11-06 (`feat/ui-rich-content`): Insert › Text box with the editor in place (fonts, B/I/U, colour, alignment, lists, links, + Value), Insert › Image stored as a registered resource (dialog on the desktop, workspace files in the browser; bytes checked and decoded, SVG sandboxed), Insert › Buttons, the action editor with button states, Ctrl+click in editing, page background, wallpaper, size and alignment; left: the list key is BIcraft's (not seen in a Desktop file), Q&A actions are written but refused by the viewer (bookmark actions applied since 2026-11-18, G55), button states drawn since 2026-11-20 (fill images, custom icons, glow and shadow per state not drawn), shapes (Insert › Shapes, 2026-11-19) and their text not checked against Desktop, the editor lays out with the bundled faces and one alignment per box | P2 | ui, engine, render | S–M | — |
| G54 | Format visual pane (M11.22, gap review item 9): **done** 2026-11-09 (`feat/format-pane`): the Build pane's Build visual / Format visual sub-tabs, Visual and General cards (title as free text, subtitle, effects, header icons, tooltips, properties; axes, legend, data labels, data colours, card, table, slicer, image cards), search, switches, Reset to default, one command per edit; formatting left the Visual ribbon tab (a Format button opens the tab); the title form's JSON box fixed (free text); per-series and per-value colours and the fx buttons (gradient, rules, field value) done 2026-11-18; fx on titles, subtitles, backgrounds, borders, data and category labels done 2026-11-19; the dialog's data bars, icons, web URL, gradient numbers, empty values, two-condition rules and preview done 2026-11-20; left: fx on axes, legend, gauges and KPIs, rules' Percent | P2 | ui, engine | S | — |
| G55 | Bookmarks and Sync slicers panes (M11.24, M11.25): **done** 2026-11-17: the Bookmarks pane (capture, update, set, remove, group, ungroup, rename a group, move, apply; Data / Display / Current page as Desktop's suppress flags; Desktop's PBIR `bookmark` documents and `bookmarks.json` in `report.resources`) and the Sync slicers pane (`slicer.set_sync` writing `visual.syncGroup`, `slicer.sync_list`); reader bookmarks and navigators **done** 2026-11-18 (`view.interact {action: "bookmark"}`, Bookmark buttons, the bookmark navigator drawn; the page navigator leaves out hidden and tooltip pages); slicer selections captured and restored, navigator styles drawn (2026-11-19); Allow deselection and navigator hover / press states (2026-11-20, [formats/buttons.md](../formats/buttons.md)); left: drill states and cross-highlights in bookmarks, navigator shapes, glow and shadow; sync groups applied by the readers (selections and fields, 2026-11-18) | P2 | ui, engine, view | S | — |
| G33 | Key influencers: a placeholder laid out like Desktop's by default, BIcraft's own badged estimate on request (`view.interact {action: "key_influencers"}`, `report.render {key_influencers}`); left: measures in Explain by, Expand by for a categorical target, counts, the drill into an influencer, the toggle in the app (the snapshot and `view.interact` carry it, no button), a layout comparison | P3 | render, engine, ui | S–M | — |
| G39 | UI control channel (M11.6): **done** 2026-11-05 on the desktop, `bicraft view --control` and `bicraft serve --viewer-control`; left: no synthetic input on purpose, no headless `ui.render`, the Firefox test not in CI | P3 | ui, mcp, view | — | — |
| G49 | Data view, query preview and See records (M11.17): **done** 2026-11-06 (`table.rows`, `query.preview`, `visual.records`; the report, page and visual filters and the slicers applied since 2026-11-18); left: measure-level and multi-table filters listed in `not_applied`, no cross-highlight from other visuals, the first measure's home table; no text or number filters in the header menus | P2 | ui, engine | S–M | — |
| G56 | Analytics: **mostly done** 2026-11-18 (the Visualizations pane's Analytics tab, error bars as `objects.error`, the scatter's ratio line and symmetry shading, all drawn); anomaly detection landed 2026-11-19 (`objects.anomalyDetection`, a Find anomalies card, BIcraft's estimate drawn and in `data.anomalies`); left: error bars with a legend, on stacked or combo columns, anomaly explanations, the `errorRange` and sensitivity shapes unchecked against a Desktop file | P3 | engine, render | S–M | — |
| G53 | Export (M11.21): **done** 2026-11-06 (`report.export_pdf` vector PDF with embedded subsets, `visual.export_data` CSV/XLSX, `report.settings {export_data}`); print (`report.print`), a PDF outline and tags 2026-11-19; left: PDF/A and PDF/UA, CFF faces embedded, Desktop's flattened underlying data and "current layout" export, export from page.html, a black-box comparison | P2 | render, engine, ui | S–M | — |
| G57 | Show items with no data and Show value as are done (M11, `visual.show_items_no_data`, `visual.show_value_as`); Desktop 2.159.1231.0 opens both as BIcraft writes them (2026-11-23) and turns Show items with no data off beside a percent of the grand total (BIcraft now does too, with a warning); left: not compared with a file Desktop saved itself, the matrix subtotal levels list no items with no data, hierarchy-level rows are left out of the written Scope | P2 | report, engine | S | — |
| G58 | Get data and Navigator: **done** 2026-11-18 (source gallery Excel, Text/CSV, JSON, Folder, SQL Server database, Web, OData feed; Navigator with check boxes and preview, `source.navigate`, `source.load` one undo step); left: no other database or online connectors in the gallery, the Navigator's M shapes not compared with Desktop's | P2 | data, ui | S | — |
| G59 | Enter data as an editable grid: **done** 2026-11-18 (`table.enter_data`, `table.entered_data`, typed columns, TSV paste, Edit data from the Data pane); checked in Desktop 2.159.1231.0 (2026-11-23): a table BIcraft wrote loads, an empty text cell `""` and a `null` one both show blank, as two rows; Desktop's Edit data (Create Table dialog) failed on rows holding `null`, and Desktop writes an empty cell as `""`, so BIcraft now writes `""` too (fixed, tested); with that, the dialog opens with every row, and an edit saved by Desktop changes only the payload (plus a `PBI_NavigationStepName` annotation); left: a real Ctrl+V | P3 | ui | S | — |
| G60 | Optimize ribbon (2026-11-18): Pause visuals, Refresh visuals and the optimization presets are commands with buttons (Analyze › Optimize, a banner while paused, File › Options › Query reduction); left: no Optimize tab of its own, no per-visual Refresh in a pending visual's banner, tooltips and formatting not blocked while paused, the slicer and filter Apply buttons are written but BIcraft's slicers and Filters pane do not show them, the legacy key `slowDataSourceSettings` inferred, nothing compared with a Desktop file | P3 | ui, engine | S | — |
| G61 | Model view left (lot 18: the editable diagram and the Properties pane, then the Data / Model pane with the Model explorer, calculation items ordered by drag and the layout tabs `diagram.add` / `diagram.rename` / `diagram.remove`, 2026-11-18; calculation items, hierarchies, roles, cultures, shared expressions and data sources in the Properties pane, the tree's hierarchies and data sources, its search, Add related tables and Remove from layout (`role.set`, `diagram.remove_tables`) done 2026-11-23, their Desktop behaviour unchecked); which diagram Desktop treats as the layout of all tables (BIcraft: ordinal 0), whether it can be deleted and whether a tab switch writes `selectedDiagram` were not checked against Desktop (docs/formats/diagram-layout.md, "Unchecked"); multi-selection and synonyms done 2026-11-20 (`model.set_properties`; Shift+click, Data pane selection and the mixed display are not compared with Desktop, synonym states inferred from Learn); also left: key column, row label and featured table in the pane | P2 | ui, engine | S–M | — |
| G62 | Analyze insights on a data point ("Explain the increase / decrease", "Find where the distribution is different"): nothing in BIcraft; Desktop 2.159.1231.0 still has Analyze in a visual's context menu (seen 2026-11-23, M11.28; the submenu was not opened) | P3 | engine, ui | M | — |
| G63 | Report settings (2026-11-18): `report.settings` authors persistent filters, personalize visuals, cross-report drillthrough, the Filters pane's visibility to readers and "Allow users to change filter types" (File › Options and settings, the pane's eye); left: the service-side settings are written but BIcraft's viewer does not act on them (personalizing, saved reader state, drillthrough from another report, the reader's filter type menu); the absent-key defaults are inferred, not compared with a Desktop file; "Enable search for Filters pane" and the other `settings` keys have no parameter | P3 | engine, ui | S | — |
| G64 | Field and what-if parameters (`parameter.fields`, `parameter.what_if`, 2026-11-18): the model shapes match Desktop's own (Desktop 2.159.1231.0, 2026-11-23); a visual bound to a field parameter carries the role's `fieldParameters` and the parameter's fields as Desktop saves them, and Desktop shows the fields (2026-11-23); the PBIX legacy layout writer does not write them; the what-if slicer draws as a list, not a slider; hierarchies chosen by a field parameter are not modelled | P2 | engine, render | S–M | — |
| G65 | View as roles (`view.interact view_as`, 2026-11-18): a role filter reading another secured table is evaluated without that table's security; one-to-one relationships and the blank row are not checked against the engine; the secured state copies the filtered columns in memory (no large-volume measure); no signed-in user outside View as; the browser app not run live | P2 | engine | S–M | — |
| G66 | Performance analyzer (`perf.*`, 2026-11-18): no render time in the browser (no wasm32 clock); render times are collected after Refresh visuals only, not for view events; the copy-query DAX leaves out measure filters, Top N and the filters BIcraft ignores; the pane was not run in a live browser | P3 | ui, engine | S | — |
| G67 | Power Query editor (lot 12, 2026-11-18); since 2026-11-18 (part 2) the Advanced editor over the whole query (checked by `query.steps`, a text that does not parse is not taken), the column headers' type menu and filter menu (sorts, text or number filters, a value list with check boxes from the first 1000 rows writing `filter_values`), new blank queries, duplicates and references (`query.new`), renames with the drafts that read the query following (`query.rename`) and deletions after a question, committed by Close & Apply in the same undo step (`query.apply {renamed, removed}`), and a question before changes are discarded; since 2026-11-19 a folding indicator (icon and word) on each applied step (`query.folding`) and View native query and Diagnose step on a step's context menu (`query.native`, `query.diagnose`: BIcraft's own time, rows and source requests), the editor passing the connectors the query calls (`query.steps {connectors}`) to its previews; since 2026-11-23 the Transform and Add column tabs and Home's Keep rows and Remove rows menus (38 column operations, row reduction, fill, replace errors, transpose, reverse, move, index, duplicate, merge and conditional columns; `query.apply_step` kinds of `crates/data/src/m/transform_more.rs`), the step settings gear (`query.steps {settings}`, `query.edit_step {settings}`), `#date` / `#datetime` literals in filters and the value list of date columns (`query.preview` `date_kind`), and previews reading the other queries' drafts (`query.preview {queries}`); since 2026-11-23 (part 2) the Scientific, Trigonometry, Information and Duration menus, the rest of Standard, Date and Time, Text between delimiters (79 column operations) and Home › Export results (`query.export`, CSV). Left: the Statistics and Parse menus, count rows, detect data type, convert to list, column from examples, invoke function; the folding indicators are BIcraft's (the whole query up to each step, its own evaluator), not compared with Desktop's; each step is probed anew (a navigation listing is read again per step); Expand asks for typed column names, no query groups, no Enable load (a new query that is not a table fails at Close & Apply, a blank one included), data profiling landed (`query.profile`, View tab; Desktop's statistics per type not compared), the query dependencies view landed (`query.dependencies`; its layout not compared with Desktop's), step names always in English (docs/formats/locales.md records no localized step names, so none are written), a multi-partition table is not reloaded; generated M (the names `Query1`, `Sales (2)`, the value-list `or` / `and` shapes) not compared with Desktop's | P2 | data, ui | M | — |
| G68 | SQL Server connector (lot 11, 2026-11-18): tested only against a scripted fake TDS server (never a real SQL Server or Azure SQL); no Windows or Azure AD authentication; the Navigator's M shape (`schema.item` names, `Source{[Schema,Item]}[Data]`, lowercase GUIDs) not compared with Desktop's; `Sql.Databases` not in the gallery; datetime to the millisecond; options in `IGNORED_OPTIONS` have no effect; privacy levels recorded but not enforced, credentials per URL for web sources (G69) | P1 | data, sqlserver | M | — |
| G69 | Web, OData and SharePoint connectors (lot 11, 2026-11-19/20): tested only against an in-process test server (never a real service or SharePoint site); credentials Basic, Web API key and a supplied bearer token per URL prefix (no OAuth sign-in, no Windows); privacy levels recorded and shown, the Data Privacy Firewall not enforced, levels not saved in a file; no `Web.Page` (HTML tables), no `Xml.Tables`, no binary metadata; OData JSON v4 only (no Atom/XML), no navigation, complex or collection columns, no singletons or functions; text filters not folded; SharePoint version 14 endpoints not used, list record columns (lookups, people) not loadable into a model table; the Navigator's M shapes not compared with Desktop's | P1 | data, webdata, engine | M | — |
| G70 | UI speed tests (2026-11-19, [ui-perf.md](../ui-perf.md)): frames and latencies of the test profile only (no release build), CPU side only (no GPU, no OS event loop), the desktop shell only (not the browser); the Model view's diagram, memory, a saved baseline and a random-input soak are not measured; the Power Query preview takes 5.7–9.1 s at 1,000,000 rows | P2 | ui | S–M | — |
| G71 | Visual calculations (`visual.calculation.*`, 2026-11-19): **not checked against the engine** (values computed by hand from the Learn pages); `LOOKUP`, `LOOKUPWITHTOTALS`, `OFFSET`, `INDEX`, `WINDOW`, `RANK`, `ROWNUMBER`, `ORDERBY`/`PARTITIONBY`, `ROWPAGES` walks and sort-by axis order **done** 2026-11-23; `MATCHBY` and `LOOKUP … INFERRED` **done** 2026-11-24 (INFERRED read from the visual matrix's values); left: navigation along `ROWPAGES`; the `queryRef` written is BIcraft's choice; no live preview in the edit window | P2 | engine, dax, ui | M | — |
| G72 | Mobile layout view (2026-11-19): Desktop 2.159.1231.0 opens a project with a phone layout and saves `mobile.json` unchanged apart from CRLF line endings, and its phone canvas shows each visual at its `mobile.json` position and size, 1:1 in CSS pixels, with a title set by `visual.mobile_format` on the phone only (2026-11-23); BIcraft's phone canvas is **aligned on Desktop's (fixed 2026-11-24, maintainers' decision)**: 324 wide, 3600 high while edited, an 18×20 grid (`visual.mobile {snap: true}`, the canvas's gestures); how Desktop rounds onto its grid and whether it grows past 3600 unobserved; override semantics of the other mobile `objects` unchecked; no Auto-create, no remove-all, no appearance filter in Page visuals; fx dialog and image scaling not per phone; no copy, paste or group on the phone canvas | P2 | ui, engine, pbir | S–M | — |
| G73 | Themes and high contrast (2026-11-19): the View › Themes gallery of BIcraft's eight own themes (`theme.set {builtin}`), Customize current theme as the Customize theme dialog (2026-11-20: name and colours, text, visuals, page and Filters pane, Apply as one `theme.customize`; not compared with a Desktop run of the dialog, and it offers the settings listed in Microsoft Learn's page, not every property of the theme schema), the high-contrast UI theme and the view-only high-contrast report palette are **done**; left: the built-in themes not opened in Desktop; the desktop does not detect Windows contrast themes (eframe exposes light and dark only; Home › High contrast instead); the report palette is BIcraft's colour mapping (close hues share a colour, images unchanged), not compared with Power BI's high-contrast rendering; `bicraft render` and the HTML viewer have no high-contrast option; Filters pane formatting and card order **done** 2026-11-19 (`page.format_filter_pane`, `filter.order`, G41) | P2 | ui, render, visuals | S | — |
| G74 | Web viewer regression checks (`cargo xtask webcheck`, M11.27, 2026-11-19, [webcheck.md](../webcheck.md)): the HTML viewer only (the egui app at `/app/` is a canvas Craft Anything cannot click, and needs the wasm bundle embedded); gestures sent as the page's `POST /api/interact` (Craft Anything's scripted clicks need Deep mode, never used), only the tabs clicked in the page; no export walkthrough (`bicraft view` has none); baselines per machine (language, colour scheme, window size); Windows only so far | P3 | xtask, view | S | — |
| G75 | Groups and bins (`column.group`, 2026-11-19): list groups and bins (numbers by size or count, dates by years to seconds) are written in the form Desktop 2.159.1231.0 saves, `GroupingDesignState` included, and its Edit groups opens each (2026-11-23); no design state for a list group listing a blank (Desktop's dialog has no such group); no number of bins on dates; no grouping of data points selected on a visual | P2 | engine, ui | S | — |
| G76 | Quick measures (`measure.quick`, 2026-11-19): BIcraft's own DAX templates, not compared with the DAX or results of a quick measure Desktop wrote; Sales from new customers, Correlation coefficient and a fiscal year end for year-to-date **done** 2026-11-24; time intelligence walks an auto date/time table's dates, and on a column with neither that nor a marked date table it warns (no template can find a period's bounds from a column with gaps) | P2 | engine, ui | S | — |
| G80 | Custom visuals (2026-11-19, [formats/pbiviz.md](../formats/pbiviz.md)): `.pbiviz` import (`visual.import_custom`), removal (`visual.remove_custom`), the gallery cells with their icons and the page placeholder are **done**; projects (2026-11-20, M11.30): `visual.scaffold`, `visual.build` (swc, no Node) and `visual.pack` **done**, a packaged visual embedded by BIcraft **drawn by Desktop 2.159.1231.0** with its data and WebAssembly running in its sandbox (2026-11-23), Desktop's own import dialog not tried, no npm packages, LESS, `.tsx` or type checking; **AppSource (Get more visuals) is out of scope**: it needs the network and Microsoft's store (a signed-in service, its terms and its catalog), which BIcraft never reaches; a custom visual's JavaScript is never run (a labelled placeholder with its icon instead; the BIcraft Spec visual is drawn from its spec, G105); legacy layouts refused; the browser uploads at most 5 MB; nothing checked in Desktop's own import dialog | P2 | engine, ui | — (AppSource: out of scope) | — |
| G81 | DAX query view (2026-11-20, 2026-11-23): query tabs, the DAX editor, Run (Ctrl+Enter, F5), the results grid, Update model (`dax.apply_define`), the tabs saved in a PBIP project's `DAXQueries` folder (`dax.set_queries`, [formats/dax-queries.md](../formats/dax-queries.md)), Format query (`dax.format`), Cancel (`dax.cancel`) and squiggles while typing are **done**; left: a PBIX/PBIT keeps the tabs for the session only (no public description of its parts), no Data pane or quick queries, no Comment or Find and replace, Cancel waits for its turn over a remote server from the desktop app; checked in Desktop 2.159.1231.0 (2026-11-23): tabs, default tab and save shape | P2 | ui, engine | S | — |
| G84 | App keyboard navigation (2026-11-20, [desktop.md](../desktop.md) "Keyboard navigation"): Ctrl+F6 sections, F6 to the Selection pane, Tab inside a section, modal dialogs that give the focus back, the focus ring, named controls, Alt / F10 key tips on the ribbon, Shift+? (shortcuts list), Shift+F10 context menus and the Model view's panes as sections are **done**; left: the Menu key (egui reads no event for it), keyboard focus on the canvas's visuals, Alt+Shift+1 / 9; not run with Narrator, NVDA or JAWS, F6, Ctrl+F6, Alt and F10 not checked in a browser or against Windows' window-menu handling | P3 | ui | S–M | — |
| G92 | Map visuals (2026-11-20, [formats/maps.md](../formats/maps.md)): bubbles at latitude and longitude or at the countries or major cities a location names, filled maps of countries and shape maps (custom or the built-in World countries (Natural Earth) map, written as an ordinary custom TopoJSON map) drawn offline with cross-filtering on an open basemap of Natural Earth data (2026-11-24, by the maintainers' decision: 1:50m countries, lakes, admin-1 boundaries and city names for regions); **left**: geocoding of places other than countries and Natural Earth's 1,251 populated places (needs Bing / Azure Maps), Desktop's built-in shape maps (Microsoft's map data), filled-map colour saturation by conditional formatting, online map tiles (not done, see the entry), maps from a URL, page.html, a custom map in a legacy PBIX, Color saturation shading set in Colors, map and filled map compared with Desktop (switched off in its security options) | P2 | render, engine, view | M–L | online map service |
| G99 | Gantt visual (2026-11-20, [formats/gantt.md](../formats/gantt.md)): `ganttChart` and its formatting objects (report theme schema 2.159) are catalogued, inserted and formatted; its seven wells come from a wells test against Desktop 2.159.1231.0; evaluated one row per task (by start, at most 1,000) and drawn (2026-11-23): task list with detail columns, collapsible category rows, bars, milestones, progress, the auto-scaled or two-level date axis, gridlines, shaded days, tooltips and cross-filtering; **left**: no scrolling, zoom axis or click to expand/collapse; not compared with Desktop's drawing; how Desktop saves a per-member collapse and `shadingDays` not observed | P3 | render | S | Desktop's Gantt drawing |
| G100 | Excel Power Pivot import (2026-11-20, [formats/powerpivot.md](../formats/powerpivot.md)): `file.open` of `.xlsx`/`.xlsm` imports the data model (tables, columns, calculated columns, hierarchies, relationships, measures with format strings, descriptions and display folders, values) into a new Power BI model saved as PBIX/PBIP; a workbook Excel 16.0.20625 saved from our synthetic tables is a fixture, read with Excel's own DAX results (2026-11-23); **not pursued** (2026-11-23, maintainers' decision: the import is kept as it is, Power Pivot is not developed further; out of scope): calculated columns, hierarchies, hidden objects and KPIs checked only on synthetic workbooks (Excel's object model cannot create them); `metadata.sqlitedb` backups (none seen), connections (the M keeps the source as a comment and errors on refresh), culture and perspectives not read | P2 | powerpivot, engine | S–M | the Power Pivot window (user's screen) |
| G101 | Calibri stand-in (2026-11-20, M11.38, [render.md](../render.md) "Calibri"): text in Calibri takes the bundled Carlito subsets (Calibri's advance widths) when Calibri is not installed; **left**: Greek, Cyrillic and Vietnamese text in Calibri falls through to Noto Sans (other widths); no kerning in the subsets; Calibri Light takes the regular face; other Office fonts (Cambria → Caladea…) have no bundled stand-in; widths not measured against Calibri drawn by Desktop | P3 | render | S | — |
| G102 | Broken report references (2026-11-20, M11.39, [mcp.md](../mcp.md) "Report rules"): `model.analyze` lists stale fields, queryRefs, selectors, interactions and field parameters with their place and a fix: `report.rebind` to the suggested field of the same kind, else `visual.unbind`, `*.clear_filters` or `report.remove_reference` (drillthrough and tooltip fields, formatting, bookmarks), `visual.fix_query_refs` and `parameter.fix_field` (2026-11-23); **left**: no fix for other properties without a near name, hierarchy levels, stale selectors and dangling interactions; a re-bind that changes the table of a reference written through a shared query alias is refused; a bookmark filter card kept in `filters.byName` is removed whole (2026-11-23); checked in Desktop 2.159.1231.0 on 2026-11-23: a synthetic report repaired by the five fixes (0 findings) opens with every visual drawn and no error mark, the drillthrough page left without fields shows as an ordinary page and the bookmark whose filter card was removed applies without error; a measure bound with another home table is accepted (as `model.usage` resolves it); report-level measures' DAX is checked by `report.validate`, not here; not compared with what Desktop flags when it opens such a report | P2 | engine, ui | M | — |
| G103 | More readers (M11.36, 2026-11-20, [formats/import-sources.md](../formats/import-sources.md)): `.vpax`, `.pbids`, `.pq`/`.m`, `.pqt`, Dataflow Gen2 and the CDM `model.json` open as a new model and are `objects.import` sources; **left**: read from public descriptions only, no file the service or Desktop exported was read (the `.pqt` metadata part name and a real Dataflow export are unverified); no Navigator after a `.pbids` (tables have no columns until their query runs: `table.import` with `replace: true`); `.pbids` dataflow and other protocols refused; a VPAX opens editable (not read-only) and its statistics are only in the `file.open` result, not in `model.storage_stats` | P2 | engine | S | a real export of each kind |
| G104 | Redact secrets (M11.37, 2026-11-20): `file.redact`, `bicraft redact`, File › Export › Redact a copy for sharing; **left**: navigation literals that are not source arguments (`Source{[Name = "db"]}`) are redacted only when the same value was collected elsewhere; with `data`, numbers and dates are kept, calculated tables keep their stored text, M literal data (Enter data) is not replaced; a PBIP copy drops its `.pbi` cache; not tried on a Desktop-saved PBIX (the self-check refuses a copy where a collected value survives, e.g. in a DataMashup part BIcraft does not rewrite) | P2 | engine | S | a Desktop-saved file |
| G105 | Spec visual (M11.32, 2026-11-23, [formats/spec-visual.md](../formats/spec-visual.md)): `visual.set_spec`, `visual.spec_template`, `visual.scaffold {template: spec}`, the BIcraft Spec custom visual (TypeScript built by BIcraft's bundler) and the renderer's drawing of the same layout are **done**; drawn in Power BI Desktop 2.159.1231.0 like `report.render` (combo and a `Date` category on a time scale, 2026-11-23; 1.0.1.0 reopened with the layout fixes, the format pane shows the spec; the spec edited in its Spec (JSON) text area redraws and is saved as the same PBIR literal, which BIcraft reads back; Desktop's save writes a `CustomVisualMetadata` item's name as its path, so BIcraft now names the item after its `.pbiviz.json` file, fixed and tested); the renderer draws the default title Desktop shows above the visual (2026-11-23); since 2026-11-24 (1.1.3.0, 3 more golden cases) its data roles and format pane are named in English and French (`displayNameKey`, `stringResources`, also in `visual.scaffold {template: spec}`) and a date category is written with its format string in labels, the legend, band axes and time ticks of a day or more; **left**: 1.1.3.0's French names and dates not yet seen in Desktop; reports BIcraft wrote before the fix keep a guid-named item until the visual is imported again (the Spec visual is reinstalled by the next `visual.set_spec`); tooltips, selection, cross-filtering, the context menu and keyboard focus of BIcraft's visual runtime (M12.2) seen in Desktop 2.159.1231.0 on 2026-11-24 (the focus kept after a selection since 1.1.2.0); high contrast and the Spec visual's landing page not checked there; no stacked bars, sorting, small multiples or a second y axis; the TypeScript and Rust layouts are compared only where Node is installed; the live viewer's HTML page (`crates/view`) draws it from the renderer's image since 2026-11-24 (G29) | P2 | render, engine | S–M | — |
| G106 | Desktop UI inventory (M11.28, 2026-11-23, Desktop 2.159.1231.0, [parity-checklist.md](../parity-checklist.md) "Desktop UI inventory"): what the black-box reading found missing or partial, as groups of controls rather than one entry per button: the Power Query editor's Statistics and Parse menus, count rows, detect data type, convert to list, column from examples, invoke function and combine files (its number, date, time, duration and text transforms, index, conditional and duplicate columns, keep and remove rows and Output data landed by 2026-11-23, G67); Analyze on a data point (G62); format, comment and find and replace in the DAX query and TMDL editors; the Model view's card display options; the Options dialog beyond Report settings; **not reached**: the Manage relationships dialog, other visual types' contextual tabs and Format cards, the inside of each Options section, the full connector list (sparklines and the Data format card done 2026-11-23 and checked in Desktop 2.159.1231.0 the same day, the Total row's sparkline 2026-11-24, matrix subtotals' sparklines checked in Desktop and their points placed on the visual's X range the same day: "G106. Sparklines and Data format" below) | P2 | ui, engine | L | — |
| G107 | Pinned corpus (M11.8, 2026-11-23, [corpus.md](../corpus.md)): hosting **decided by the maintainers (2026-11-24)**: the assets of release `corpus-v1` of the project repository (`digorpeptra/BIcraft` since 2026-11-25) (`base_url` set; `cargo xtask corpus fetch` reads it through the GitHub API with `GITHUB_TOKEN`, `GH_TOKEN` or `gh auth token`; `cargo xtask corpus stage` prepares the upload); **left**: the upload itself (a maintainer's, after review), then a `fetch` from a clean folder; the PBIT entry is now Desktop's own export (Power BI Desktop 2.159.1231.0, File › Export › Power BI template, by keyboard, 2026-11-23: the export adds `SecurityBindings` and keeps the description in `Metadata` `FileDescription`; the corpus test and `cargo xtask corpus verify` pass on it); no entry yet with a refreshed PBIP cache, DirectQuery, field parameters, bookmarks or a legacy-layout report | P3 | tests | S | maintainers' hosting decision |
| G108 | M signatures (M11.40, 2026-11-23, [dax-oracle.md](../dax-oracle.md) "M signatures"): 414 of 416 catalogue functions (409 at first, 7 added by the Power Query editor's number menus and re-probed) have the counts, parameter names and types Desktop 2.159.1231.0 reports; **left**: `Error.Record`'s `parameters` and `errorCode` and `Splitter.SplitTextByDelimiter`'s `csvStyle` are not evaluated (listed deviations); parameter types are recorded and catalogued but the evaluator does not check arguments against them, and signature help and hover still show names only; 539 library functions of `#shared` are not in BIcraft (`library_only`); `file.save_as {empty_tables}` refused a table whose unsupported source (`#shared`) is reached only through a function called by `List.Transform` (the probe query read `#shared` first to get past it) | P3 | M | S | none |
| G109 | Chart pack, part B (M12.3, 2026-11-23, [formats/chart-pack.md](../formats/chart-pack.md)): BIcraft Org Chart, Venn, Marimekko and Calendar Heatmap (TypeScript on the M12.2 runtime, packaged as `.pbiviz`, drawn by the renderer with twin layouts held by 21 golden cases) are **done**; seen in Power BI Desktop 2.159.1231.0 on 2026-11-24 (drawing, default title, tooltips, selection and cross-filtering, context menu, landing page, French format pane, the org chart's collapse saved in `tree.toggled` and read back; the Venn circle now selects every row of its set, 1.0.1.0; [community-visuals.md](../community-visuals.md) Runtime "Seen in Desktop"); since 2026-11-24 (1.0.2.0, both layouts, 9 more golden cases) the org chart's tree is compacted by its subtrees' contours (`tree.compact`, on by default), two Venn sets can be drawn area-proportional (`sets.proportional`), the Marimekko can draw its widest column first and a width axis (`columns.sort`, `columns.widthAxis`), the calendar can draw only the months with data and start weeks on Sunday (`range.months`, `range.sundayFirst`), all four drawn by Desktop 2.159.1231.0 as `bicraft render` draws them and shown in its format pane (2026-11-24); since 2026-11-24 (1.0.3.0) their data roles and format-pane names carry `displayNameKey`s with English and French `stringResources`, and date cells are written with their column's format string (from the dataView on the TypeScript side, from the model in the renderer, both through the same VB-style date rules, 3 more golden cases); seen in Desktop 2.159.1231.0 on 2026-11-24 (1.1.0.0): the roles in French (Id, Id du parent, Étiquette, Valeur; Élément, Ensemble; Catégorie, Série, Valeur; Date, Valeur) and General Date at midnight written `05/01/2026 00:00:00` as Desktop's own table writes it; since 2026-11-24 (org chart and Marimekko 1.3.3.0, 3 more golden cases) the org chart can be drawn left to right (`tree.leftToRight`, format pane EN/FR) and its collapse buttons are clickable in BIcraft (`visual.toggle_node` in the app, the reader's view state in the viewer, G29), and the Marimekko's first six series take colours of their own (`colors.series1`–`series6`) over the look's palette; **left** (2026-11-24): 1.3.3.0 not yet seen in Desktop; names in English and French only; high contrast not checked in Desktop; the org chart has no drag or images, and its compaction packs subtrees to the left (small subtrees between two large ones are not spread evenly); three sets are never area-proportional and four or more are not drawn; the calendar has no week range or fiscal year, and a year block of `range.months` still spans the years between the first and last month; the layouts are compared with the TypeScript only where Node is installed; the live viewer's HTML page (`crates/view`) draws them from the renderer's image since 2026-11-24, with their interactions in the app and the viewer (G29) | P2 | render, engine | M | — |
| G110 | BIcraft chart visuals, first pack part A (M12.3, 2026-11-23, [formats/bicraft-visuals.md](../formats/bicraft-visuals.md)): the histogram, bullet chart, box plot and dumbbell / slope chart, `visual.set_bicraft_visual` (Visual › Type › BIcraft chart), their packages built by BIcraft's bundler and the renderer's same layouts (17 golden cases, the TypeScript compared under Node) are **done**; opened in Power BI Desktop 2.159.1231.0 (2026-11-23, a PBIX written by the tests) the four visuals draw their data with the options saved by BIcraft; their interactions seen there on 2026-11-24 (tooltips, selection, Ctrl+click, cross-filtering both ways, context menu, keyboard, landing page, format pane options saved by Desktop and read back; a box now selects all its rows and the renderer draws the default title, 1.0.1.0); since 2026-11-24 (1.0.2.0, both layouts, 6 more golden cases) the box plot can be horizontal (`box.horizontal`), the bullet chart vertical (`style.vertical`), the histogram cumulative (`bins.cumulative`), and the slope chart moves its labels apart (11 px at least, kept above the axes' bottom; since 2026-11-24 (dumbbell 1.3.1.0, 2 more golden cases) the dumbbell's labels too where their spans meet, and a label moved from its dot gets a thin leader line), all four drawn by Desktop 2.159.1231.0 as `bicraft render` draws them and shown in its format pane (2026-11-24); since 2026-11-24 (1.0.3.0, 5 more golden cases) their data roles and format-pane names are localized (`displayNameKey`, English and French `stringResources`) and date categories are written with their column's format string in labels, axes and tooltips, by a port of BIcraft's VB-style date formatting checked against the Rust over 47 formats, 8 dates and both locales; seen in Desktop 2.159.1231.0 on 2026-11-24 (1.1.0.0): the roles in French (Valeurs, Fréquence; Catégorie, Valeur, Cible, Plages, Maximum; Catégorie, Échantillon, Valeur; Catégorie, Début, Fin) and General Date at midnight written `05/01/2026 00:00:00` as Desktop's own table writes it; **left** (2026-11-24): names in English and French only, and dates in other locales fall back to English; high contrast not checked in Desktop; no notches or violin, no per-row bullet scales, no density histogram, histograms of at most 200 bins and 30,000 distinct values; the dumbbell's labels moved apart and leader lines not yet seen in Desktop; high contrast is tested only through the shared runtime under Node; the TypeScript and Rust layouts are compared only where Node is installed; the live viewer's HTML page (`crates/view`) draws them from the renderer's image since 2026-11-24, with their interactions in the app and the viewer (G29) | P2 | render, engine | M | — |
| G111 | BIcraft visuals built in (M12.4, 2026-11-23, [community-visuals.md](../community-visuals.md) "Built in by default"): the registry, the gallery cells (desktop and browser), `visual.add` / `visual.set_type` by name with the package embedded on first use, `report.custom_visuals` `bicraft` and the save dropping unused packages are **done**, and since 2026-11-24 `report.apply` by name and a starting spec for a new Spec visual, and each visual's own icon (pane and package); **left**: the icons, seen in Desktop's pane on 2026-11-24 (light and dark, ten distinct), look soft and faint there (a 20 × 20 PNG upscaled on a scaled screen, slate ink), Desktop not checked on a report whose package was kept at a newer version (a report whose packages were dropped opens cleanly in Desktop 2.159.1231.0, 2026-11-24; the PBIX writer now also removes the dropped packages' `Report/CustomVisuals/` files, which it had left behind) | P3 | engine | S | — |
| G112 | BIcraft visuals outside BIcraft (M12.5, 2026-11-24, [community-visuals.md](../community-visuals.md) "Outside BIcraft"): `visual.export_bicraft_visual`, `bicraft visual export` and Insert › Visuals › Export a BIcraft visual write their `.pbiviz`, read back by `visual.import_custom`, and `cargo xtask dist` ships the nine under `visuals/` (**done**; the exported histogram and Venn imported through Desktop 2.159.1231.0's own Import a visual from a file dialog, added to a page with fields and saved, 2026-11-24); **left**: not published to the Power BI service nor uploaded there as an organizational visual (expected to draw when the tenant allows SDK visuals, and to be left out of PowerPoint and PDF exports and email subscriptions, which Microsoft supports for certified visuals only; [community-visuals.md](../community-visuals.md) "Outside BIcraft"), not tried on Power BI Report Server; the `dist` step not yet run end to end (release build and WSL); no Marketplace (AppSource) or Microsoft Store submission, by decision | P3 | engine, xtask | S | a Desktop session; a Power BI tenant admin for the organizational upload |
| G113 | BIcraft Sankey (M12.3 addition, 2026-11-24, [formats/chart-pack.md](../formats/chart-pack.md) "Sankey"): the tenth BIcraft visual, one level of weighted links in columns by longest path, cycles left out with a note, format pane EN/FR, runtime interactions, Rust twin (**done**), waypoints for links that skip columns, own ribbon slots, label collisions and narrow-width labels (**done**, 2026-11-24); seen in Power BI Desktop 2.159.1231.0 on 2026-11-24 before the waypoints (drawing, tooltips, selection both ways, menu, keys, French format pane and `links.colorBy`; Ctrl+click on shared rows fixed); the waypoint routing seen in the same Desktop later that day, drawn as `bicraft render` draws it; the labels' unset size now the look's in Power BI too (pack 1.3.2.0, Desktop drew 11 px where the renderer drew 12); **left**: multi-level paths, per-node colours, high contrast not yet seen in Desktop | P3 | engine, render | M | — |
| G114 | BIcraft's look left (M12.3 modern look, 2026-11-24, [formats/visual-look.md](../formats/visual-look.md)): presets, theme colours, decorations, Style card, motion (**done**), second pass: vivid palette, dark-background tokens, card, value chips, calendar filling its frame, Spec Style card (**done**, 2026-11-24); first pass seen in Power BI Desktop 2.159.1231.0 on 2026-11-24 with the interactions with built-in visuals (a custom theme without a base theme now keeps the classic titles); highlight support (**done** in BIcraft, 2026-11-24: nine of the ten declare `supportsHighlight` and draw highlighted parts or dim, chart visuals and pack 1.3.0.0, Spec 1.4.0.0); highlights and the second pass seen in Desktop 2.159.1231.0 on 2026-11-24 (Edit interactions offers Highlight, drawings as `bicraft render`; a visual without a measure field now stays unchanged, as in Desktop); **left**: under a dark theme Desktop keeps the page and containers white where `bicraft render` paints them dark, the Spec combo template's legend names, high contrast and reduced motion not seen in Desktop; value-range colours of the Spec visual and the calendar's explicit colours stay as set | P3 | engine, render | S | a Desktop session |
| G115 | BIcraft Scatter+ (M12.3 addition, 2026-11-24, [formats/chart-pack.md](../formats/chart-pack.md) "Scatter+"): every point up to 10,000 with a note past it, hexagonal density view above a threshold, marginal histograms or box strips, filled and bordered markers with size and colour roles, trend line with R², quadrants at averages or values with labels and shares, labels on the top or the selected points, format pane EN/FR, highlights, interactions, Rust twin (**done**); **left**: not seen in Power BI Desktop; BIcraft's renderer draws no labels in the `selected` mode (its input carries no selection yet); one trend line over all points (none per legend group); density cells coloured by count only, in the first colour; rows past 10,000 are left out, not drawn as density; points with the same details value are one point (aggregated by the data view); the cap note groups digits the English way in French too | P3 | engine, render | M | a Desktop session |
| G116 | BIcraft Line / Area+ (M12.3 addition, 2026-11-24, [formats/chart-pack.md](../formats/chart-pack.md) "Line / Area+"): band between two measures coloured by which is above, threshold shading, area formatted apart, labels on all, the last or the extreme points, line colours by series, emphasis or rule, a continuous date axis (twin of the renderer's time axis), a dashed forecast stretch from a date or today, series names at line ends, format pane EN/FR, highlights, interactions, Rust twin (**done**); **left**: not seen in Power BI Desktop; one rule (no list of rules) and one threshold; no stacked areas; per-series colours only from the look's palette (no colour per series in the format pane); the date axis labels use estimated text widths; with more than 3,000 points a click selects a whole series | P3 | engine, render | M | a Desktop session |
| G117 | BIcraft Waterfall+ (M12.3 addition, 2026-11-24, [formats/waterfall-gauge.md](../formats/waterfall-gauge.md)): horizontal or vertical, subtotals by group, flag or every N, colours by sign, step or rule, several measures, a target with its variance, labels and connectors, look, highlights, format pane EN/FR, interactions, Rust twin (**done**); **left**: not seen in Desktop, several measures summed (non-additive measures wrong), no breakdown, multi-measure steps and totals select nothing | P3 | engine, render | S | a Desktop session |
| G118 | BIcraft Gauge / KPI+ (M12.3 addition, 2026-11-24, [formats/waterfall-gauge.md](../formats/waterfall-gauge.md)): arc, bullet and KPI card, free bands from roles or the format pane, a target and three thresholds, variance vs target or previous, sparkline from a trend field, the field's format, look, highlights, format pane EN/FR, interactions, Rust twin (**done**); **left**: not seen in Desktop, one value per visual, no threshold ticks or needle on the arc, no sparkline axis | P3 | engine, render | S | a Desktop session |

## P0: blocks a common workflow

### G1. Structural PBIX edits are not reachable from the CLI or MCP — fixed

- **Status.** Fixed on 2026-10-13. `file.save` / `file.save_as` on a PBIX write structural
  edits with their VertiPaq data (`pbix-package.md` §8, "Structural saves"):
  - tables, columns and relationships added or removed, tables and columns renamed (same
    lineage tag, new name: object rows renamed, storage untouched), tables given new data
    (replaced in two stages, relationships rebuilt);
  - new commands `table.import` (CSV file or text with culture-aware type inference, an M
    `#table` literal, or a DAX `DATATABLE`/`{…}`/`ROW`/`CALENDAR` literal; CSV is stored as an M
    `#table` literal partition, so the table stays refreshable) and `table.set_data`;
  - calculated tables (DAX literals, else the viewer's DAX evaluator; columns inferred from
    the result) and calculated columns (evaluated row by row, in dependency passes);
  - `bicraft_pbix::structural::DataModelEditor` (encoder → `H$`/`R$` → storage rows → ABF →
    XPress9), `bicraft_metadata::write_structural_sqlitedb_renamed_at`;
  - metadata-only saves keep the fast path; report-only saves keep `DataModel` untouched.
  Desktop 2.158 opened every case tried and returned the expected values through DAX queries:
  an imported CSV table with a relationship to an existing table, a measure and a card; a
  removed calculated column and a removed import column; a calculated table (`FILTER` over a
  table, including a calculated column added in the same save), a `DATATABLE` table and two
  calculated columns (values identical to Desktop's own computation); a renamed table and a
  renamed relationship column; replaced data; a 200 000-row import. A full refresh of the
  imported table in Desktop succeeded with unchanged values. Timings (release, local sample):
  metadata-only or calculated-column save 0.2 s; 200 000-row CSV import with a relationship
  3.8 s.
- **Leftovers done (2026-10-14).** Engine-checked with the structural check of the development oracles (the
  engine `ImageLoad`s each save and answers like its own deployment; `metadata-sqlitedb.md`,
  "Engine check of the structural writer"):
  - a data column added to an existing import table: its `m` partitions run again and the
    values are matched to the stored rows through the stored columns (any stored row order);
    refused, with `table.set_data` as the way out, when the query no longer returns the stored
    rows;
  - many-to-many (no stored index, blanks and duplicates on both sides), one-to-one (two
    indexes) and both-direction relationships;
  - user hierarchies: `hierarchy.add` / `hierarchy.remove`, written with their `U$` node table
    (identical to the engine's), on new, replaced and stored tables;
  - tables of several `m` partitions written and replaced per partition (one dictionary per
    column, row numbers continuing, `PartitionStorage.DataObjectId` = position);
  - `column.remove` on a calculated table rewrites its expression with `SELECTCOLUMNS` (the
    engine would bring the column back otherwise);
  - a replaced table keeps its perspective memberships (new measures are now written before
    perspectives and translations, so they can name them).
  Desktop 2.158 opened two saves of the local sample made through the commands (new tables with
  many-to-many, one-to-one and both-direction relationships, a hierarchy, a table of two `m`
  partitions, a calculated table; then a data column added from the query, one partition given
  new data, a calculated-table column removed, a hierarchy on the two-partition table) and
  answered every DAX and MDX query as the engine oracle did.
- **PBIX from scratch (2026-10-14, M5).** A new model, a `model.bim`, a PBIT or a PBIP without
  `.pbi/cache.abf` saved as `.pbix` gets an empty database image (fresh `metadata.sqlitedb`,
  engine backup layout, database key file) and every table through this structural writer
  (`docs/formats/datamodel-fresh.md`); a table whose source cannot run is refused by name or,
  with `empty_tables`, written without rows (tables without rows now take the engine's form: no
  dictionary file, no `H$` table). Engine oracle: expected values everywhere; Desktop 2.158
  opens the files, draws visuals with the data, refreshes a CSV table with identical values and
  keeps roles and hierarchies.
- **Left.**
  - `m` partitions run in BIcraft's M evaluator (G21: literals, Enter data, transforms, local
    files under `data_root`; no connector is run); other sources are refused with the reason.
  - Calculated tables are evaluated by the engine's DAX evaluator, including `ADDCOLUMNS`,
    `SUMMARIZE(COLUMNS)`, `SELECTCOLUMNS`, `GENERATESERIES`, `CALENDARAUTO`, set functions and
    text/`FORMAT` functions (66 probes match the engine, 2026-10-14, G21); anything else is
    refused with the reason.
  - A relationship whose "one" end (or a one-to-one end) is a stored value-encoded column is
    refused (the column would have to be re-encoded as hash; the engine does that on
    refresh). Incremental-refresh policy-range partitions are not re-evaluated (a table with
    them cannot be given new data per partition); `table.set_data` gives any table one
    partition. Renaming inferred columns is supported since 2026-10-14 (G15).
- **Where.** `crates/engine/src/pbix_struct.rs`, `crates/engine/src/pbix_new.rs`, `crates/engine/src/preview_eval_struct.rs`,
  `crates/engine/src/hierarchy.rs`, `crates/pbix/src/structural.rs`,
  `crates/metadata/src/structural/` (`add.rs`, `hierarchy.rs`, `rename.rs`),
  the structural check; from scratch: `crates/{abf,metadata,pbix}/src/fresh.rs`, the fresh-model check.

### G2. Report edits are not saved into a PBIX or PBIT — fixed

- **Status.** Fixed on 2026-10-13. `file.save` / `file.save_as` on a PBIX or PBIT write the
  edited report back in the form the package holds (`bicraft_pbix::write_report`):
  - PBIR parts: changed documents rewritten (compact JSON, as Desktop writes them in a PBIX),
    new pages and visuals added, removed ones deleted, untouched documents byte-identical;
  - legacy `Report/Layout`: patched in place by the new `bicraft_layout::write_layout`
    (untouched sections and visuals keep their text; edits the layout cannot carry are refused);
  - `SecurityBindings` is dropped when report parts change: Desktop rejects a package whose
    report parts no longer match it (checked, `pbix-package.md` §9);
  - an unedited model keeps its compressed `DataModel` (no XPress9 round trip on a report-only
    save).
  Desktop 2.158 opened every case tried (identity, moved visual, added visual with fields and
  title, page added and removed, `report.apply`, active page, report + measure, legacy layout
  edits, PBIT).
- **Left.** Converting a legacy report to PBIR inside a package is not offered (never done
  silently); legacy edits with no mapping yet (theme, bookmarks, page bindings, report settings)
  fail the save with the list.

## P1

### G3. Conversions are not reachable (M3.3) — fixed

- **Status.** Fixed on 2026-10-14 (`crates/engine/src/convert.rs`). `file.save_as` infers the
  target from `format` or the extension and converts; `file.convert {from, to}` (MCP `convert`,
  CLI `bicraft convert IN OUT`) does the same without touching the open file:
  - PBIX → PBIP (TMDL/TMSL, PBIR or legacy report, `.pbi/cache.abf` from the `DataModel` so
    Desktop opens it with data), PBIX → PBIT, PBIT ↔ PBIP, PBIP TMSL ↔ TMDL, PBIP → PBIX when the
    project has `cache.abf` (refused with the reason otherwise, and for PBIT → PBIX);
  - a legacy report stays legacy unless `report_format: "pbir"`; legacy-only keys are translated
    (phone layouts → `mobile.json`, bookmarks metadata) or dropped with a warning naming them, on
    every PBIR write (`file.save`, `project.new`, conversions) and in `report.validate`;
  - the model diagram and `StaticResources/` / `CustomVisuals/` are carried; dropped parts are
    listed in `warnings`; MAX_PATH guard on project targets.
  Desktop 2.158 opened all 13 conversions tried with the full model (`pbix-package.md` §10).
  Found on the way and fixed: models from a PBIX carried `isAvailableInMDX` (refused by Desktop
  in TMSL/TMDL; since 2026-10-14 the metadata reader produces TOM's `isAvailableInMdx`, the
  structural writers read either spelling, and the engine's write-time rename is gone), and the
  TMDL writer put `valueFilterBehavior` after `dataAccessOptions`.
- **Left.** A PBIR report cannot
  be turned into a legacy layout. A project whose model differs structurally from its
  `cache.abf` cannot become a PBIX until Desktop refreshes the cache.

### G4. DAX validator false accepts — fixed

- **Status.** Fixed on 2026-10-13. The 14 false accepts left in the 631-entry corpus are gone, and
  the corpus grew to 917 entries, all recorded with the engine: 916 agree, 1 false accept
  (`NONVISUAL` on a filter that covers non-group-by columns), 0 false rejects. The oracle test now
  asserts ≥ 99% agreement and no false reject. The new rules are listed in `docs/dax-oracle.md`
  §"Results":
  - unknown functions are errors against a model; reserved words
  - `WEEK` intervals and `dates` arguments
  - `GROUPBY`/`CURRENTGROUP`
  - window relations and clauses
  - natural joins and calculated-table column names
  - `CALENDARAUTO`
  - query cycles
  - comparisons of incompatible types in calculated columns, tables and queries
  - multi-section argument order

  2026-10-14: `SUM(Typed[N])` was a false accept found in Desktop. The reserved-word list only
  had DAX function names and keywords; the engine also reserves its MDX/DMX keywords (`Typed`,
  `Type`, `Status`, `Name`, `Key`, `ID`, `Version`…), 568 words in all, and most of them refuse a
  bare table reference too. The validator now errors in every place the engine does, with a hint
  to quote the table, and the writers (`dax.rename`, engine messages) quote these names.
  24 `keyword-names` oracle entries agree.

  The rest of this entry is the state before the fix.
- **What.** The last recorded oracle run is in `docs/dax-oracle.md` "Results". On a corpus of 583
  expressions:
  - definitions: 91.8% agreement (535 of 583), 48 false accepts, 0 false rejects
  - including evaluation: 88.0% agreement, 70 false accepts

  `ROADMAP.md` (M6.3) and `crates/dax/tests/oracle.rs` give a later figure: 97.6% agreement,
  0 false rejects, and a test floor of 96%. `dax-oracle.md` has not been updated to match (G28).
  The 48 recorded false accepts, by theme:

  | Theme | Count |
  |---|---|
  | Argument kinds and keyword values (`COUNTROWS(column)`, `MAX(table)`, `DATEADD` interval, `GROUPBY` aggregation, `USERELATIONSHIP`, …) | 8 |
  | `RELATED` in the wrong direction, or to an unrelated table | 7 |
  | Table shape (tuple width, `UNION` width, `IN` width) | 6 |
  | Duplicate column names from table functions | 5 |
  | Variables (redefinition, function name as variable name, `var[column]`) | 4 |
  | Unknown functions reported only as warnings | 3 |
  | CALCULATE boolean filter without a column | 3 |
  | Window functions (`PARTITIONBY`/`ORDERBY` placement) | 3 |
  | Other (empty argument, unquoted `Date[Date]`, `CALENDARAUTO` without a date column) | 3 |
  | Row context (column not in a one-column `VALUES`; bare column as a calculated table) | 2 |
  | Table-constructor column names; `SUMMARIZE` by an unrelated column | 2 |
  | Cycles (context transition in calculated columns; between query measures) | 2 |

  By role, the weakest is calculated tables: 80.0% agreement.
- **Why it matters.** A false accept lets an invalid expression through. The user only finds out
  when Desktop rejects the file.
- **Where.**
  - `crates/dax/src/typecheck.rs`
  - `crates/dax/src/binder.rs`
  - `crates/dax/src/query.rs`
  - work list: `docs/dax-oracle.md` §"Work list"
- **Effort.** M overall. Most themes are S on their own.
- **Depends on.** Nothing.

### G5. Validation cost grows quadratically with the number of measures (benchmark hot spot #1) — fixed

- **Status.** Fixed on 2026-10-13. `model.validate` on 100 / 1,000 / 10,000 measures now takes
  11 / 33 / 216 ms on Windows and 9 / 35 / 197 ms on WSL, and `validate-model` on the
  10,000-measure model takes 0.3 s end to end ([benchmarks](../benchmarks.md)).
  - `bicraft_dax::ModelIndex` hashes every table, column and measure name once (upper-cased,
    first match wins, as the scans did). The binder, the type checker and `deps::dependencies`
    look names up through it; `binder::measure_index` is gone.
  - `bicraft_dax::Validator` holds the index and a type cache for one borrowed, immutable model.
    The cache keeps only *exact* types: those inferred without reaching a cycle or the
    16-object nesting budget, so they do not depend on which object asked first. Long chains
    are typed bottom-up by retrying from the objects where the budget ran out. Objects whose
    dependencies reach a cycle are still inferred per expression, as before.
  - `model.validate` (and so `Session::snapshot`) and the `broken` check of renames and removals
    use one `Validator` per model. Cycle diagnostics are matched to objects through a map.
  - `Context`, `validate` and `validate_as` are unchanged and still scan: building an index for
    one expression costs more than it saves.
  - Diagnostics are identical. Result types are identical except at the end of chains deeper
    than 16 measures, where `validate_as` gives up and reports a variant while the validator
    gives the exact type. `crates/dax/src/tests_validator.rs` (generated models with chains,
    cycles, duplicate names and hostile nesting, validated in both orders), the oracle corpus
    (`validator_matches_per_expression_validation`) and the engine test
    `whole_model_pass_matches_per_object_validation` compare both paths.
  - Left as is: `dax.validate` against a large model still scans per expression (about 15–25 µs
    against a 1,000-measure model); a session-level `Validator` would need invalidation on edits.
- **What (before the fix).** `model.validate` timings:

  | Measures | Time |
  |---|---|
  | 100 | 11–15 ms |
  | 1,000 | 0.16–0.27 s |
  | 10,000 | 10.8–12.7 s |

  The CLI shows the same: `validate-model` on the 10,000-measure model takes 11 s. A `perf` profile on WSL puts
  about 84% of the time in `SemanticModel::measure`, which is a linear, case-insensitive scan over
  every table's measures (`crates/model/src/lib.rs`). The binder and type checker call it for
  every measure reference. The type checker also re-infers each referenced measure through its
  dependency chain for every object it validates, because its memo lives for one `validate_as`
  call (`crates/dax/src/typecheck.rs`, `infer_object`).
- **Why it matters.**
  - Enterprise models with 1,000–5,000 measures are common.
  - Every `measure.add`/`set` validates against the model, so editing one of those models gets
    slower too.
  - `dax.validate` against a 1,000-measure model runs at about 10,000 expressions/s, against
    about 200,000/s without a model.
- **Where.**
  - `crates/model/src/lib.rs` (`measure`, `table`)
  - `crates/dax/src/binder.rs`
  - `crates/dax/src/typecheck.rs`
  - `crates/engine/src/commands.rs` (`model_validate`)
- **Effort.** S. Build a case-insensitive name index once per validation pass and keep it in
  `Context`. Share the type memo across the objects of one `model.validate`.
- **Depends on.** Nothing. `Context` is public API, so add a field with a default.

### G6. Saving a PBIX re-compresses the whole `DataModel` (benchmark hot spot #2) — fixed

- **Status.** Fixed on 2026-10-14. `compress_datamodel` now defaults to
  `CompressionMode::Fast`: each 2 MiB chunk gets its own encoder, primed with the 256 KiB before
  it, lookup depth 1, run on up to 8 threads; the blocks are renumbered into one session, so the
  layout is still Desktop's single-threaded one. `Balanced` (Desktop's match search, parallel)
  and `Exact` (the old path, Desktop's bytes) remain. Output does not depend on the thread count
  or the platform. Compression went from 4–20 MB/s to 200–300 MB/s (≥ 1 MB inputs), and the
  32 MB-storage `write_model` from 11.0 s / 8.0 s to 0.88 s / 0.75 s (Windows / WSL); the part is <!-- perf-guard: ok (BIcraft before and after) -->
  2.5–4% larger than with `Exact`. Desktop 2.158 opens files written in each mode, with 1 to 15
  chunks (`docs/benchmarks.md` hot spot 2, `docs/formats/xpress9.md` §6). The decompression that
  then dominated the save (read, then verify) was made 4–6× faster on 2026-10-14: a fast
  decoder on up to 8 threads, and a parallel `verify_datamodel` (hot spot 5, `xpress9.md` §7).
  Option 3 below (reuse unchanged chunks) still applies.
- **Cause found.** The match finder's tables (about 28 MB) leave the caches past about 1 MB of
  input; the search is bounded, so the slowdown was cache misses, not an algorithmic blow-up.

- **What.** `pbix::write_model` decompresses the ABF, rewrites `metadata.sqlitedb`, re-packs, and
  re-compresses every byte with XPress9.

  | Case | Time |
  |---|---|
  | One added measure on a synthetic PBIX with 8 MB of storage | 1.4–1.6 s |
  | One added measure on a synthetic PBIX with 32 MB of storage | 8–11 s |
  | The 32 MB case through `bicraft run … --save` | 6–7 s |

  The XPress9 encoder runs at:

  | Input | Throughput |
  |---|---|
  | 1 MB synthetic | about 20 MB/s |
  | 16 MB synthetic | 5–7 MB/s |
  | 64 MB synthetic | 4–5.5 MB/s |

  The cost per byte grows with the input size.

  The README figure, 55–120 MB/s, was measured on 4 MiB inputs. In the `perf` profile, 66% of the
  save is in `Encoder::lookup`, the LZ77 match finder. Decompression runs at 140–200 MB/s.
- **Why it matters.** Real `DataModel` parts of 100–500 MB would take tens of seconds to minutes
  per save, which makes the edit loop on a real PBIX slow.
- **Where.**
  - `crates/xpress9/src/enc_lz77.rs`
  - `crates/xpress9/src/datamodel.rs` (`compress_with_plan`)
  - `crates/pbix/src/lib.rs` (`write_data_model`)
- **Effort.** M–L. The options are:
  1. Find out why throughput drops on larger inputs (hash chains, window handling).
  2. Write Desktop's multithreaded framing, so chunks can be compressed in parallel.
  3. Copy the compressed chunks that come before the first changed byte. This needs the encoder
     state at a chunk boundary.
- **Depends on.** For option 2, G17: which framings Desktop accepts.

### G7. No undo or redo; `batch` is not transactional (M1.6)

- **What.** A failed step in `batch` (MCP) or `bicraft run` leaves the earlier steps applied, and
  there is no undo stack (`docs/mcp.md`).
- **Why it matters.** Agents make mistakes. Without a rollback, the only way back is to reopen the
  file and lose every unsaved edit.
- **Where.** `crates/engine/src/lib.rs` (`Session::batch`), `crates/mcp/src/tools.rs`.
- **Effort.** M. Snapshot-based undo is simple, but a session can hold a multi-GB package, so the
  model and report need cheap snapshots or inverse commands.
- **Depends on.** Nothing.
- **Status: done (M1.6).** `edit.undo` / `edit.redo` / `edit.history` keep a bounded history of
  shared snapshots (last 100 steps or ~256 MB; `crates/engine/src/history.rs`), and `batch` /
  `bicraft run` are atomic by default (rollback of model, report, flags, history and a replaced
  open file; files written by saves stay). Not covered: undoing file writes.

### G9. Reader fuzzing (M7.3) — done

- **Status.** Done on 2026-10-13 ([fuzzing.md](../fuzzing.md)). Every reader below has a stable-Rust
  smoke test (`crates/<crate>/tests/fuzz_smoke.rs`, run by `cargo xtask ci` on Windows and Linux)
  and a cargo-fuzz target under `fuzz/` (nightly, Linux), on the same input encodings; the MCP
  request handler has one too. Campaigns of about 105M executions found no crash; findings F1
  (TMDL fence round trip) and F2–F4 (allocation caps) are fixed with regression tests. F5 (TMDL
  writer emitting JSON keys as property lines, found by a 100 000-input smoke run) was fixed on
  2026-11-04.
- **What (original).** The fuzz targets for every hostile-input reader were 🔄:
  - PBIX/OPC
  - XPress9
  - ABF
  - SQLite
  - VertiPaq
  - TMDL
  - TMSL
  - PBIR
  - legacy layout
  - DAX
- **Why it matters.** "Never crash" is a project rule, and users open files from anywhere.
- **Where.** `fuzz/` and per-crate fuzz tests.
- **Effort.** M.
- **Depends on.** Nothing.

## P2

### G11. PBIR schemas are re-parsed on every validation (benchmark hot spot #3) — fixed

- **Status.** Fixed on 2026-10-14. Each vendored schema is compiled once per process into a node
  arena (`$ref`s as ids, sorted properties, compiled patterns, messages and pointers built only
  for issues), cached per root file. `validate_documents`: real sample 22–38 → 10–13 ms,
  bench report 100 pages 131–157 → 80–88 ms; the first call in a process costs 1–2 ms more
  (`benchmarks.md` hot spot 3).

- **What.** Each call to `validate_documents`, and so each `report.validate`, builds a new schema
  `Store`. That store parses the vendored JSON Schemas again (`crates/pbir/src/validate.rs`,
  `crates/pbir/src/schema.rs` `Store::load`). In the edit-loop profile, most samples are in
  `serde_json` map building and SipHash.

  | Case | Time |
  |---|---|
  | `report.validate` on a 1-page report (26 files) | 3–7 ms |
  | `report.validate` on a 10-page report | 15–26 ms |
  | `report.validate` on a 100-page report | 160–250 ms |
  | Edit loop (`measure.add` → `file.save` → `report.validate`), per iteration | 22–57 ms |
- **Why it matters.** Agents validate after each edit, so this cost lands in every iteration of the
  edit loop.
- **Where.** `crates/pbir/src/schema.rs`, `crates/pbir/src/validate.rs`.
- **Effort.** S. Parse once into a `OnceLock<Store>`, and use a cheaper hasher for the schema maps.
- **Depends on.** Nothing.

### G13. Legacy layout → PBIR fidelity

- **Done (2026-10-15).** Desktop 2.158's own legacy → PBIR conversion is the oracle
  (`legacy-layout-bb.md`): `cargo xtask layout-oracle` generates 8 legacy reports of our own
  (visual types, formatting and conditional formatting, sorts, filters of every level and type,
  page settings and bindings, bookmarks, groups, phone layouts, themes, settings, report
  measures, a custom visual), `tools/layout-oracle/run.ps1` has Desktop upgrade each one on save
  (UI Automation, no setting changed), and `crates/engine/tests/layout_oracle.rs` (local only)
  compares `file.convert` with Desktop's `definition/` file by file. 51 differences (besides
  `$schema` versions) found and fixed, now 0; Desktop's rules are in `legacy-layout-bb.md`
  (theme version table, settings defaults, object descriptors, group flattening without the
  stylable header, drill `active` flags, cached display names, `reportExtensions.json`, …).
- **What is left.**
  - Phone formatting: Desktop carries neither inline `mobileObjects` nor our guess of
    `mobileState.json` into `mobile.json`; its real legacy source is Unknown.
  - Visual and container formatting objects are not filtered by Desktop's descriptors; Desktop's
    garbage collection of unreferenced image resources is not reproduced (§11.9).
  - `filterExpressionMetadata` and multi-column cached display names stay unmapped.
  - The report model has no typed place for bookmarks metadata, `mobile.json` or
    `reportExtensions.json` (they ride in `Report.resources` / `legacy:` keys), and no
    `Position::angle`.
  - No real (non-synthetic) legacy/PBIR pair has been compared.

  Sources: `legacy-layout-bb.md`, `pbir.md` model notes.
- **Why it matters.** Converted reports may lose details that Desktop would keep.
- **Where.** `crates/layout-bb`, `crates/report`.
- **Effort.** M.
- **Depends on.** A real legacy/PBIR pair (local only).

### G15. Renames and removals do not rewrite DAX — fixed

- **Status.** Fixed on 2026-10-14.
  - `table.rename`, `column.set` / `column.rename` and `measure.set` / `measure.rename` rewrite
    every DAX expression that references the object (`bicraft_dax::rename`,
    `crates/engine/src/dax_rename.rs`): measures, calculated columns and tables (field
    parameters' `NAMEOF`), row filters, KPI expressions, dynamic format strings, detail rows,
    calculation items and selection expressions, report-level measures (and their entity). Only
    resolved references change; strings, comments and the rest of the text keep their bytes.
    Result: `dax_rewritten`, `dax_not_rewritten` (does not parse, or a variable would capture the
    new table name); `update_dax: false` opts out.
  - `table.remove`, `column.remove` and `measure.remove` are refused while DAX uses the object,
    with the list; `force: true` removes it and leaves the expressions (`used_by`).
  - M expressions are not rewritten (Desktop does not either). Visual calculations name fields
    by `nativeQueryRef`, which renames keep.
  - Checked with Desktop 2.158: a PBIP with measures, calculated columns and two RLS roles,
    renamed by BIcraft (a measure, two columns, two tables), opens and every measure, calculated
    column and row filter evaluates to the original's values. Desktop resolves
    `'Product'` to a variable named `Product` when one is in scope (so quoted names are not
    rewritten there; the validator applies the same rule since 2026-10-14, checked with 26 oracle
    entries, `docs/dax-oracle.md`).
  - Compared with Tabular Editor 2.29's formula fix-up (the stand-in for Desktop's own rename),
    on the same renames against the same model: identical text for every measure, calculated
    column and row filter, except that the fix-up renamed a variable shadowing the old table name
    (`VAR Product = 3 RETURN Product + COUNTROWS('Product')` became
    `VAR 'Item' = 3 RETURN 'Item' + COUNTROWS('Item')`, which does not parse); BIcraft leaves it.
    Both quote a reserved new name (`Item[x]` → `'Item'[x]`).
  - Since 2026-10-14 renames also follow (`crates/engine/src/rename_refs.rs`, result
    `metadata_updated`): perspective members, culture translations, linguistic metadata
    bindings, calculated table columns' `sourceColumn`, column properties naming a table or column
    (variations, `alternateOf`, grouping columns) and calendars. User-defined function bodies are
    rewritten like any DAX (parameters shadow tables). `measure.move` rewrites
    `OldTable[Measure]` qualifiers (model and report-level DAX) and moves the measure's
    perspective, translation and linguistic entries. A PBIX metadata save keeps a renamed
    measure's row (matched by lineage tag, or as the only renamed measure of its table), so its
    perspective and translation rows follow. Removals drop the perspective members they leave
    behind (`crates/engine/src/perspectives.rs`, result `perspective_entries_dropped`).
  - Since 2026-10-14 (later):
    - `NAMEOF` is evaluated (`crates/engine/src/preview_eval_nameof.rs`), spelled as the engine
      prints it (64 `nameof-*` entries recorded with Desktop 2.158's engine in
      `crates/engine/tests/table_functions/expected.json`: `'Table'[Column]`, measures qualified
      by their home table, components and escaping, the refused forms). A structural PBIX save
      after renaming a measure or column a field parameter names re-evaluates the parameter
      table (the replaced table keeps its group-by column, `ParameterMetadata` and sort-by).
    - Calculated-table columns with an inferred name follow their source column's rename, as the
      engine's own rename (TOM `RequestRename`) does: `Amount` → `Net`, `Sales_Amount` → `Sales_Net`
      for an unnamed `SELECTCOLUMNS` column (and on a table rename), transitively through
      calculated tables built on them, with the DAX that references them (`inferred_renamed` in
      the result, `crates/engine/src/rename_inferred.rs`). Renaming an inferred column by hand
      makes its name explicit. Saved into a PBIX as well (inferred names stay inferred, source
      columns follow); a PBIX built by the engine, renamed this way by BIcraft, ImageLoads into
      the engine (DBCC passes) with the same names and values as the engine's own rename.
    - Linguistic metadata entity keys generated from the names (`sales.amount`) are renamed with
      their bindings, and relationship roles targeting them follow. The engine does not touch
      linguistic metadata (checked), and Desktop's own rewrite was not observed: keys follow the
      schema convention (lower case, white space as `_`); other keys stay.
- **Left.**
  - Desktop's linguistic schema rewrite on rename is assumed, not observed (generated terms are
    left as they are).

### G18. PBIR and PBIP acceptance questions — fixed

- **Status.** Fixed on 2026-10-14, checked with Power BI Desktop 2.158.1304.0 on projects BIcraft
  generated (opened, saved through the Save button, files compared):
  - page folder name ≠ `name`: opens; on save the folder keeps its name;
  - a page missing from `pageOrder`: opens (listed last); on save Desktop adds it to
    `pageOrder`. `validate_documents` no longer reports it;
  - empty `themeCollection`: opens; saved unchanged (`{}`);
  - `.pbip`, `definition.pbism` and `definition.pbir` are rewritten on save in the V1 form, without
    `$schema` (CRLF, as every file Desktop saves);
  - `definition.pbir` declaring `definitionProperties` 1.0.0 with `version` 4.0: opens;
  - Desktop writes U+2028 as `
` and non-ASCII text raw (UTF-8); we write both raw and
    Desktop reads that.
  - The validator now covers every keyword the vendored PBIR and theme schemas use, and the rest
    of draft-07 except `$id` base changes; patterns run on a small ECMA-262 engine (coverage
    table in `pbir.md` §Validation). Unsupported pattern syntax still passes, but no vendored
    pattern uses it.

- **What.** Whether Desktop:
  - requires page and visual folder names to equal `name`
  - tolerates pages that are missing from `pageOrder`
  - accepts an empty `themeCollection`
  - rewrites `.pbip`/`.pbism` back to V1 without `$schema`

  Also open:
  - U+2028 and U+2029 are written raw.
  - The precedence of PBIP `version` over `$schema` is unknown.
  - The JSON Schema validator implements a subset; unknown regex patterns pass.

  Sources: `pbir.md`, `pbip.md`, `pbix-package.md`.
- **Why it matters.** Without these answers, a report BIcraft accepts may be one Desktop
  rejects.
- **Where.** `crates/pbir`, `crates/pbip`.
- **Effort.** S each.
- **Depends on.** The Desktop oracle.

### G20. Runtime-only DAX errors are not reported — fixed

- **Status.** Fixed on 2026-10-13. Run-time errors that can be told statically are reported as
  warnings, with codes starting with `runtime-`:
  - comparisons of incompatible types
  - text that does not convert
  - several rows used as a value
  - literal arguments out of a function's domain
  - aggregations of text
  - non-date `dates` columns
  - non-boolean row filters
  - (2026-10-14) a `SUMMARIZECOLUMNS` expression that is a column's `VALUES`/`DISTINCT`/`ALL`
    when the group-by columns do not determine that column
  - (2026-10-14) a filter modifier a measure's definition accepts but evaluation refuses

  They never make an expression invalid. There are two exceptions. Incompatible comparisons are
  errors in calculated columns, calculated tables and `EVALUATE` statements, where the engine
  rejects them at definition. Misplaced filter modifiers are errors in `EVALUATE` statements. The
  corpus now has 110 such entries and 109 are flagged. No `runtime-*` warning fires on an entry
  the engine evaluates; the oracle test asserts it. Details in `docs/dax-oracle.md` §"Results".

  The rest of this entry is the state before the fix.
- **What.** In 22 corpus entries, the engine accepts the definition but evaluation fails:

  | Kind | Entries |
  |---|---|
  | Types | 12 |
  | Implicit coercion | 4 |
  | Time intelligence | 2 |
  | RLS | 2 |
  | Table constructor | 1 |
  | Window | 1 |

  Examples: implicit conversions, a multi-value `VALUES` used as a scalar, a non-boolean RLS
  filter. BIcraft could report these as warnings (`dax-oracle.md`).
- **Why it matters.** Users find these errors only when a visual fails to render.
- **Where.** `crates/dax/src/typecheck.rs`.
- **Effort.** S–M.
- **Depends on.** G4, the same corpus.

## P3

### G23. Windows overhead

- **What.** On the same machine, Windows is much slower than WSL for process start and small-file
  I/O:

  | Case | Windows | WSL |
  |---|---|---|
  | Process start | 23–29 ms | 2–3 ms |
  | `pbir::write_dir`, 100 pages (2,105 files) | 525 ms | 78 ms |
  | `tmdl::write_folder`, 1,000 tables | 589 ms | 95 ms |

  The likely causes are NTFS and on-access scanning, not our code. On Windows, a `bicraft run`
  batch or an MCP session amortizes the start cost: 100 separate processes take about 2× as long
  as one batch.
- **Why it matters.** Short CLI calls feel slower on Windows, the main target platform.
- **Status.** Done on 2026-10-14 ([benchmarks](../benchmarks.md) hot spot 4). Process start is
  Windows' floor: a Rust hello-world starts as slowly as `bicraft --version`, so batching (`run`,
  MCP) stays the remedy. Small-file I/O now overlaps the opens on up to 16 threads and avoids
  extra opens (`metadata` before `read`, `canonicalize` per file): reading the 100-page report
  went from ~620 to ~155 ms, writing it from ~2.8 to ~1.4 s, rewriting an unchanged 1,000-table
  TMDL folder from ~500 to ~160 ms. Creating thousands of new files is still slow on Windows.
- **Where.** `crates/pbir/src/fs.rs`, `crates/tmdl/src/folder.rs`, `crates/tmdl/src/par.rs`.
- **Effort.** S.
- **Depends on.** Nothing.

### G24. TMDL edge cases

- **What.** From `tmdl.md` §"Known gaps":
  - Names with line breaks cannot be written.
  - Control characters are written, but Desktop rejects them.
  - Unknown nested objects become property blocks, which Desktop rejects.
  - TMSL string arrays in `extra` round-trip as a single string.
  - The indent width is inferred wrongly in one rare layout.
- **Status.** Fixed on 2026-10-14. The TOM oracle (development oracles, outside this repository) diffs BIcraft's TMDL with the TOM
  serializer of Desktop 2.158 both ways (`docs/formats/tmdl.md` §"TOM oracle"): BIcraft now
  rewrites TOM's TMDL byte for byte, reads it into the TMSL TOM's reader gives, and TOM reads
  everything BIcraft writes. The diff found and fixed about twenty more cases (data sources,
  role members, ordinals, dates, implied partitions and column types, quoting, property orders,
  the object kinds of TOM 17). Names with control characters or line breaks are refused by
  `write_folder` (`Error::UnwritableName`); the other three items are TOM's own behavior.
- **Where.** `crates/tmdl`.
- **Effort.** S.
- **Depends on.** Nothing.

### G25. SQLite features that are not supported

- **Checked on 2026-10-14.** A differential campaign against SQLite 3.49.1 (101 generated
  databases: page sizes 512 to 65536, UTF-8 / UTF-16le / UTF-16be, freelists with trunk pages,
  auto-vacuum full and incremental, checkpointed WAL files, payloads at every overflow
  threshold) found no reader or writer bug: every row matched SQLite and every identity
  `rewrite` passed `integrity_check` (`sqlite.md`, "Tests"). Two fixtures keep the cases the
  committed set lacked (`freelist_512_utf16be`, `wal_8192_incremental`). Frames still in a
  `-wal` side file are not read (the reader takes one file image), by design.
- **What remains.** From `sqlite.md`:
  - `WITHOUT ROWID` and virtual tables
  - VIRTUAL generated columns and non-literal defaults (read as Null)
  - expression and partial indexes
  - collations other than BINARY, NOCASE and RTRIM
  - files of 1 GiB or more
  - `rewrite` drops the freelist

  None of these occur in `metadata.sqlitedb`.
- **Where.** `crates/sqlite`.
- **Effort.** S–M.
- **Depends on.** Nothing.

### G26. PBIX package questions — fixed

- **Status.** Settled on 2026-10-14 against Desktop 2.158 and .NET's public
  `System.IO.Packaging` API (`pbix-package.md` §11):
  - `[Content_Types].xml`: Desktop's exact shape (BIcraft writes it); a part no entry covers is
    silently hidden, duplicates and schema violations make the file unreadable. `check_package`
    reports all of these.
  - Zip64, data descriptors and any entry order are accepted (forced Zip64 on a small PBIX
    opens). `bicraft-opc` has its own writer laid out like Desktop's files, with Zip64 where a
    value needs it (`WriteOptions::force_zip64` for tests); reading handles Zip64 and data
    descriptors under the size caps.
  - UTF-16 BOM: fatal before `Version` and `Settings`, tolerated before `DataModelSchema`.
  - `.zip` and `.tmp` open, like `.pbix` and `.pbit`.
  - Custom visuals: `CustomVisuals/<guid>/package.json` + `resources/<guid>.pbiviz.json`, item
    paths relative to `resources/` (checked with a visual of our own). They are read into the
    report in both forms, kept byte-identical on save, validated, and listed by the new
    `report.custom_visuals` command.
  - `/DataMashup`: absent from the 2.158-saved samples (V3 models).
  - Copilot containers (`pbix-package.md` §2, "Copilot containers"): `Copilot/` holds
    `version.json`, `settings.json`, `schema.json`, `examplePrompts.json`,
    `Instructions/{version.json,instructions.md}` and `VerifiedAnswers/` (`definitions/*.json`,
    `ResourcePackages/`); schemas and limits from Microsoft's public Copilot JSON schemas. Desktop checks:
    package text is UTF-16LE (UTF-8 refused), the container is parsed whatever `/Version` says,
    the 15,000-character instructions limit is enforced, root `VerifiedAnswers/` next to `Copilot/` is refused, and a project's
    `SemanticModel/Copilot/` is read only with `definition.pbism` 6.0 (ignored with 4.2). BIcraft
    keeps the parts on save, carries them PBIX ⇄ PBIP / PBIT (UTF-16LE ⇄ UTF-8, `/Version` 1.36,
    `definition.pbism` 6.0; a converted PBIX opened in Desktop) and `file.inspect` reports
    `details.copilot` (counts only).
  - Packages over 4 GiB (§11, "Packages over 4 GiB"): parts over 4 GiB stay in the file
    (`Part::deferred`) and are copied raw on save (streamed to a temporary file, then
    re-pointed); the model of such a `DataModel` is read by streaming XPress9 and keeping only the
    ABF header, directory, BackupLog and `metadata.sqlitedb`. Tested with sparse 5 GiB archives
    and backups (no 4 GiB in memory or on disk). Model edits into such a file are refused with a
    clear error; report edits work.
- **What remains.**
  - what Desktop's engine writes for a `DataModel` over 4 GiB (not producible on this machine)
  - model edits and conversions of a `DataModel` over 4 GiB (would need a streamed ABF rewrite)
  - a Desktop-written verified answer definition (its JSON shape)
- **Where.** `crates/opc` (`deferred.rs`, `zipwrite.rs`), `crates/xpress9/src/stream.rs`,
  `crates/abf` (streamed index), `crates/pbix` (`large.rs`, `copilot_files`), `crates/pbip`
  (`read_copilot_files` / `write_copilot_files`), `crates/pbixwriter`, `crates/engine`
  (`copilot.rs`, `save.rs`, `custom_visuals.rs`).
- **Effort.** S each.
- **Depends on.** Nothing.

### G27. In-flight UX milestones — done

- **Status.**
  - M2.4 is done (2026-10-13): `report.apply`, `report.check` and `report.extract`, with stable
    ids, shelf and grid layout and visual property commands (`docs/mcp.md` §"Declarative
    reports").
  - M8.1 is done (2026-10-13): the live viewer (`bicraft view PATH`, `bicraft mcp --view`).
  - M8.2 is done (2026-10-14): the common visuals are rendered with the report theme and with
    data from VertiPaq or BIcraft's DAX evaluator; filters, slicer selections, text boxes,
    subtitles, dividers and images show.
- **What is left.** Minor: text-box runs bound to a measure, titles set by a measure, deeper
  levels of hierarchy slicers.
- **Where.** `crates/engine` (snapshot, preview evaluator), `crates/view`.
- **Effort.** S for what is left.
- **Depends on.** Nothing.

### G28. Stale or missing documentation — fixed

- **Status.** Fixed on 2026-10-14. Each item below was checked against the code:
  - `dax-oracle.md` "Results" now gives the 1142-entry run and the 99% floor (updated with G4).
  - `abf.md` documents add and remove.
  - `metadata-sqlitedb.md` and `pbix-package.md` record that Desktop opens metadata-only saves
    (M5.1), and `pbix-package.md` §8 no longer says `file.save_as` writes a package only from
    the same kind of file.
  - `docs/mcp.md` describes PBIX/PBIT saves with structural and report edits, and conversions.
  - `docs/formats/data-sources.md` exists.
  - The VertiPaq "Encoder" (`vertipaq.md` §11) and metadata "Structural writing" sections exist.
  - `ROADMAP.md` M1.5 records the TMDL check; the M3 and M5 texts reflect G1, G2 and G3; the
    `docs/gaps.md` row is ✅.
  - `README.md`: status table, known limits, quick start (`convert`, `view`) and MCP tool list.
  - `CONTRIBUTING.md`: `cargo xtask bench` and the fuzzing layers.
- **What (original).**
  - `dax-oracle.md` "Results" (91.8%, 90% floor) predates the 97.6% figure and 96% floor in
    `ROADMAP.md` and `crates/dax/tests/oracle.rs`.
  - `abf.md` says there is no add or remove, but the ABF writer now has both.
  - `metadata-sqlitedb.md` and `pbix-package.md` still list Desktop acceptance of metadata-only
    saves as open. M5.1 settled it.
  - `docs/mcp.md` describes PBIX/PBIT save as unavailable.
  - `crates/data` points to a missing `docs/formats/data-sources.md`.
  - The VertiPaq "Encoder" and metadata "Structural writing" sections are referenced but missing.
  - `ROADMAP.md` M1.5 text still says the TMDL writer is in progress.
- **Effort.** S.
- **Depends on.** Nothing.

### G35. Every command has its button (M11) — fixed

- **Status.** On 2026-11-05 (branch `feat/m11-ribbon`): the app's ribbon is the engine's command
  registry (`crates/ui/src/ribbon.rs` over `registry_table.rs`, a generated copy of
  `bicraft_engine::COMMANDS` the browser compiles too): File (the Backstage, which lists the
  File tab's commands), Home, Report, Model, Data, Analyze, View and the contextual Visual, Table
  and Measure tabs; every ribbon-placed command is a button of its group with its EN/FR label,
  icon and tooltip, greyed with the session's reason (the snapshot's `commands.disabled`, the
  reader role on top). Menu-placed commands are in the right-click menus of a visual, a page tab
  and the model diagram's objects; the registry's shortcuts work. On 2026-11-05 (branch
  `feat/m11-palette`, M11.2): the command palette (Ctrl+Shift+P) lists every command of the same
  registry with the same enabled state, and `form::open(app, id, prefill)` builds any command's
  parameters form (the ribbon, the menus and the palette use it). `every_command_has_a_button`
  (M11.3) reads the palette's own entries and fails on a command no table reaches or a button
  without a command. On 2026-11-05 (branch `feat/ui-forms-polish`): answers are grids (sortable
  columns, Copy as TSV, only the visible rows laid out) and key/value trees with a JSON toggle
  (`crates/ui/src/answer.rs`), in the form and the answer window; every object kind has a picker
  (the snapshot lists hierarchies, partitions, calculation items, shared queries, roles,
  cultures and report measures by name; environment profiles come from the new read command
  `env.list` through the document's source), and the model diagram's menus list the objects
  their commands act on; optional values show the command's default (`Param::default` in the
  engine, on every optional boolean with a fixed one); the title row has the search button; in
  the browser F5 is kept from the browser and New is Alt+N (browsers keep Ctrl+N), shown there
  only.
- **Left.** On `bicraft view` in the browser (no command endpoint) the profile picker stays free
  text, as every command there.
- **Effort.** —

### G37. Calendar math through a date crate (M11.9, `jiff` since M11.43) — done

- **Status.** On 2026-11-05 (branch `feat/m11-chrono`): every civil-date conversion, leap year,
  month length, month shift, month end, weekday and ISO week of DAX (`crates/engine`), M
  (`crates/data`), `bicraft-locale`'s .NET and VB patterns, the PBIX, metadata and Databricks
  readers and writers, the renderer's date axes and the engine's timestamps goes through
  `bicraft_locale::calendar`, built on `chrono` (docs/formats/dax-functions.md, "Calendar").
  On 2026-11-23 (M11.43) the module moved from `chrono` (RUSTSEC-2026-0543, unmaintained) to
  `jiff`, with the same answers (the golden test unchanged).
  `crates/engine/tests/calendar_golden.rs` holds the port to the answers recorded from the
  hand-written code before the move (4 134 lines, all unchanged).
- **Left.** Nothing. On 2026-11-25 (branch `chore/g37-calendar-code`) the last copies went: the
  RFC 3339 timestamps of `crates/controller/src/spec.rs`, of the MCP HTTP request log
  (`crates/mcp/src/http/reqlog.rs`) and file listing (`crates/mcp/src/http/files.rs`), the
  Backstage dates (`crates/ui/src/backstage.rs`) and the `engine-bench` data generator's calendar
  (`xtask/src/engine_bench.rs`) call `bicraft_locale::calendar`, each held to its old strings by
  tests run on the old code first (year ends, 2000-02-29, 2024-02-29, 2100-02-28 and 2100-03-01).
- **Effort.** —

### G59. Enter data as an editable grid (2026-11-18) — done

- **Done (2026-11-18).** Home › Enter data opens a typed grid (add, rename, remove columns and
  rows, a type per column, TSV paste, first row as headers) that writes Desktop's Enter data
  query (`Binary.Decompress` and `Table.TransformColumnTypes`, values in the model's query
  culture) through `table.enter_data`, one undo step; Edit data (Data pane and diagram table
  menu, tables with `entered_data: true`) reads it back with `table.entered_data` and replaces it.
- **Checked in Desktop (2026-11-23, Power BI Desktop 2.159.1231.0, Store edition).** A project
  whose Enter data table BIcraft wrote (a text column with a `""` cell and a `null` cell) opens,
  refreshes and loads the table; both cells show blank, as two separate rows in a table visual.
- **Edit data in Desktop (2026-11-23, same version).** On rows holding `null`, Desktop's Edit
  (the Create Table dialog of the Source step) failed and did not open; Desktop's own payload
  writes every empty cell as `""`. BIcraft now writes `""` for every empty cell (a null text
  cell becomes an empty text; typed columns read `""` as null in the type step, as before):
  regression test `entered_rows_hold_empty_texts_not_nulls_as_desktop_writes_them`. A table
  written that way opens in the dialog with all its rows and columns; one cell edited (20 to 21)
  and saved, the TMDL differs only by the new payload, whose rows differ only in that cell, and
  an added `annotation PBI_NavigationStepName = Navigation`.
- **Ctrl+V (2026-11-26, branch `feat/g59-paste`).** Both shells turn Ctrl+V into one egui
  `Paste` event holding the clipboard's text with `\r\n` made `\n` (egui-winit 0.36.2 on the
  desktop, eframe 0.36.2's page `paste` listener in the browser), so the grid reads that event
  in both. Cells (a tab or a line break) fill the grid from the selected cell, growing its rows
  and columns; any paste while no text box has the focus does too. A single value goes into the
  focused text box without the line break a spreadsheet's copy of one cell ends with (it used to
  leave a trailing space). While the grid is open the Report canvas no longer reads the paste
  (it showed "The clipboard holds no BIcraft visuals."). Kittest
  `ctrl_v_fills_the_grid_from_the_selected_cell_and_grows_it`, seen failing on both points first.
- **Left.** Nothing. The automated tests feed the shells' `Paste` event; a Ctrl+V from the OS
  clipboard in the running app is a manual check.
- **Effort.** —
