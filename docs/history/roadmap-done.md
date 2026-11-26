# Roadmap history: finished milestones and the storage engine plan

Moved unedited from `ROADMAP.md` on 2026-11-25 (only relative links were adjusted to this
folder): the item tables of milestones M0–M10 and the storage engine's next-steps plan as it
stood that day. The live roadmap is [ROADMAP.md](../../ROADMAP.md); the progress log is
[roadmap-log.md](roadmap-log.md).

## M0 Foundations ✅

| Id | Item | Status |
|---|---|---|
| M0.1 | Workspace scaffold, plus these crates: `opc` (ZIP + UTF-16LE/UTF-8), `model`, `dax` (lexer, parser, binder, ~200-function catalog), `tmsl`, `pbix`, `pbip`, `engine` | ✅ |
| M0.2 | `xtask`: `ci`, `layers`, `wsl` | ✅ |
| M0.3 | MCP server (stdio, 8 tools) and the `bicraft` CLI | ✅ |
| M0.4 | TMDL reader; PBIP projects saved as TMDL load | ✅ |
| M0.5 | PBIX packaging notes (`docs/formats/pbix-package.md`) | ✅ |

## M1 Author PBIP semantic models (write) ✅

This is the first real "compile" target. Desktop opens PBIP directly, and nothing here depends on XPress9.

| Id | Item | Status |
|---|---|---|
| M1.1 | Lossless model: keep unknown properties and objects (descriptions, lineageTag, partitions, roles, annotations…) so read → write round-trips | ✅ |
| M1.2 | TMDL writer and TMSL writer, with byte-stable output on Windows and Linux | ✅ |
| M1.3 | Model editing commands: `table.*`, `column.*`, `measure.*`, `relationship.*`. Each one validates DAX before it is applied. | ✅ |
| M1.4 | `project.new`: scaffold `.pbip`, `definition.pbism`, `.platform`, `.gitignore` per `pbix-package.md` | ✅ |
| M1.6 | Undo/redo stack; transactional `batch` (all-or-nothing) | ✅ |
| M1.5 | Round-trip tests on our own fixtures. Oracle: Desktop opens the project (Windows, manual or scripted). TMSL verified 2026-10-13: Desktop 2.158 loaded a bicraft-generated project (2 tables, M partitions, 3 measures, 1 relationship); Tabular Editor read it back from the engine with no measure errors; after a refresh in Desktop the measures return the expected values (user-confirmed). TMDL verified 2026-10-13 as well (Desktop opens a bicraft-written TMDL project). | ✅ Desktop 2.158 opens bicraft-generated PBIP projects in both TMSL and TMDL (engine read back via Tabular Editor) |

## M2 Reports (PBIR) ✅

| Id | Item | Status |
|---|---|---|
| M2.0 | Report model (`crates/report`, lossless, PBIR-shaped) and frozen pbir/layout APIs | ✅ |
| M2.1 | PBIR reader/writer (`definition/`: report, pages, visuals), validated against each file's `$schema` | ✅ real report (10 pages, 115 visuals) round-trips; 0 schema issues |
| M2.2 | Page and visual commands (`page.*`, `visual.*`, filters, themes) | ✅ |
| M2.3 | Legacy `report.json` / `Report/Layout` reader and conversion to PBIR | ✅ matches Desktop 2.158's own conversion on 8 generated legacy reports (layout oracle, G13); real legacy/PBIR pair not compared |
| M2.4 | Declarative, idempotent report building (`report.apply` / `report.check` / `report.extract`) with stable ids so a re-run does not duplicate visuals; grid and shelf placement; a "visual outside the page" check; visual property commands (title, sort, filters, formatting) | ✅ |
| M2.5 | All visual options, typed and discoverable. (a) Catalog of every formatting object/property per visual type (types, enums, ranges, defaults), built from public sources first (Microsoft's report theme JSON schema, vendored PBIR schemas), format notes to fill gaps; (b) `visual.options {visual_type}` to discover them, and `visual.format` validating every object/property/value against the catalog (today the `objects` escape hatch writes anything unchecked); (c) per-series / per-category selectors (data colors, line styles, labels per measure); (d) conditional formatting: rules, color scales (gradient), field value, icons, data bars, web URLs; (e) visual-type specifics: axes (scale, range, log, gridlines, ticks), legends, small multiples, analytics lines (constant, average, trend, forecast), tooltips/drillthrough pages, visual interactions, alt text, layer order, mobile layout; (f) theme editing (`theme.*`: data colors, text classes, visualStyles) so defaults are set once; (g) Desktop oracle for each family, plus viewer rendering of the options it can show | ✅ catalog 49 types / 14.3k properties; oracle: 14,085 properties set on every type, 14,085 kept by Desktop's save (literals and field values) |

## M3 PBIT, thin PBIX and conversions ✅ (thin PBIX not yet tested against a published model)

All conversions (PBIX → PBIP/PBIT, PBIT ↔ PBIP, PBIP TMSL ↔ TMDL, PBIP → PBIX from `cache.abf`) are reachable through `file.save_as`, `file.convert`, MCP `convert` and `bicraft convert` since G3 (2026-10-14).

| Id | Item | Status |
|---|---|---|
| M3.1 | PBIT writer: `DataModelSchema`, report, `Version`, content types, following the loader rules in `pbix-package.md` | ✅ Desktop opens generated PBITs (Settings needs QueriesSettings.Version) |
| M3.2 | Thin PBIX (report only, connected to a published dataset) | ✅ written; untested against a published model |
| M3.3 | Conversions: PBIP ↔ PBIT, PBIX → PBIP (once M4 lands); expose PBIP → PBIT in the CLI and MCP (not reachable today) | ✅ |

## M4 Read the PBIX `DataModel` ✅

| Id | Item | Status |
|---|---|---|
| M4.1 | Research: public specs (MS-XLDM) and existing open-source implementations (`docs/formats/datamodel-abf.md`) | ✅ |
| M4.2 | XPress9 format notes (`docs/formats/xpress9.md`) | ✅ |
| M4.3 | XPress9 decoder (pure Rust). Microsoft published the XPress9 C source under MIT (`microsoft/Extensible-Storage-Engine`, `dev/ese/src/_xpress9`): ported with attribution (user decision). | ✅ decoder + encoder ported (MIT, attributed); encoder reproduces Desktop's output |
| M4.4 | ABF container reader, then `metadata.sqlitedb` → `SemanticModel` | ✅ real PBIX model read end to end, identical to the PBIP reference |
| M4.5 | VertiPaq column decoding (data preview); optional | ✅ all 132 real columns decode; 40/44 match pbixray cell-for-cell (other 4: pbixray returns blanks) |

## M5 Write a full PBIX ✅

✅ First deliverable (M5.1): metadata-only edits of an existing PBIX (measures, formats, descriptions, annotations, RLS) — rewrite metadata.sqlitedb, re-pack the ABF, re-compress with XPress9. Done: ABF writer, SQLite writer, model → metadata tables, XPress9 framing writer, `file.save` on PBIX; Desktop opens edited PBIX files (added measure, format change, RLS role). Done (M5.5): `bicraft-data` (CSV with culture-aware type inference, M `#table` literals, DAX `DATATABLE`/`{…}`/`ROW`/`CALENDAR` literals). Done (M5.2): VertiPaq column encoder (byte-identical on the sample given the original dictionary order). Done (M5.3): H$/R$/U$ system tables rebuilt exactly, with Windows linguistic collation. Done (M5.4): ABF add/remove and structural metadata writer; Desktop opens a PBIX with a column removed, a table removed and a calculated table added. Done (G1, 2026-10-13): encoder + system tables + structural writer wired into `file.save` on PBIX: tables, columns and relationships added, removed or renamed, `table.import` / `table.set_data` (CSV, M, DAX literals), calculated tables and columns evaluated. Done (G2): report edits saved into PBIX/PBIT. Done (G3): PBIP → PBIX when the project has `.pbi/cache.abf`.

Done (2026-10-14): a PBIX built without an existing `DataModel` (new model, `model.bim`, PBIT, PBIP without `cache.abf`): an empty database image (`metadata.sqlitedb`, backup layout and database key as the engine writes them, `docs/formats/datamodel-fresh.md`) gets every table through the structural writer; external sources refused by name or written empty (`empty_tables`); engine-checked values; Desktop 2.158 opens the files, draws visuals with the data, refreshes a CSV table with identical values, keeps roles and hierarchies, and shows an `empty_tables` table empty. Remaining shortfalls are tracked as the G1 leftovers in `docs/gaps.md` (M connectors, DAX beyond the evaluator, relationships to value-encoded keys, policy-range partitions).

## M6 DAX validation v2 ✅

| Id | Item | Status |
|---|---|---|
| M6.0 | Shared type vocabulary (`crates/dax/src/types.rs`) and frozen `signature()` API | ✅ |
| M6.1 | Complete function catalog with parameter types and return types | ✅ 472 functions (public reference ∪ engine DMV); arity matches the engine for 464 |
| M6.2 | Type checking and context checks (row context vs filter context, scalar vs table) | ✅ |
| M6.3 | Oracle suite: compare our verdicts with the real engine (Desktop's local AS through DAX Studio / Tabular Editor) on a corpus of expressions | ✅ 583 expressions; 97.6% agreement, 0 false rejects (test floor 96%) |
| M6.5 | Circular-reference detection between measures and calculated columns | ✅ |
| M6.4 | DAX queries (`DEFINE` / `EVALUATE`) | ✅ |

## M7 Distribution and reliability ✅

| Id | Item | Status |
|---|---|---|
| M7.1 | README, LICENSE-MIT / LICENSE-APACHE, GitHub Actions CI (Windows + Linux) and release workflow building `bicraft` binaries (files prepared only; nothing is pushed or published) | ✅ |
| M7.2 | `bicraft setup`: register the MCP server with Claude Code, Claude Desktop and VS Code (dry run, backups, uninstall) | ✅ |
| M7.3 | Fuzzing of every reader (PBIX/OPC, XPress9, ABF, SQLite, VertiPaq, TMDL, TMSL, PBIR, legacy layout, DAX): stable-Rust fuzz tests + cargo-fuzz targets | ✅ |
| M7.4 | Performance benchmarks (`cargo xtask bench`) on synthetic large models/reports and the local real files; `docs/gaps.md` ranked list of known shortfalls | ✅ |

## M8 Live viewer (read-only UI) ✅

A local web page served by `bicraft` that shows, live, what is being built (not a builder: editing stays in CLI/MCP). Contract: `docs/view-snapshot.md`.

| Id | Item | Status |
|---|---|---|
| M8.1 | Structured wireframe: pages at real size, visuals at exact positions with type, title, bound fields and validation issues; model diagram (tables, measures, relationships); validation panel; live updates over SSE; `bicraft mcp --view`, `bicraft view PATH` (re-reads on disk changes) | ✅ |
| M8.2 | Rendering of the most common visuals (cards, bar/column, line, table/matrix, slicer) with the report theme and sample or real (VertiPaq) data | ✅ rendering + producer done; general DAX evaluator (CALCULATE, time intelligence, iterators: real file 113/115 visuals evaluated or field-less, 53/54 agree with Desktop); images done (wallpaper, page backgrounds, image visuals); text boxes (runs, fonts, alignment, links as styled text), slicer selections (list, dropdown, between, text search; applied to the page's other visuals), subtitles with their font, dividers: legacy layout and PBIR |
| M8.3 | Interactive viewer: slicer clicks in every mode, cross-filtering on data points with Ctrl+click and `visualInteractions`, tooltips, page navigation, Apply / Clear all slicers; reader state in the session (`view.interact`), never in the files; data recomputed by BIcraft (`POST /api/interact`, `bicraft view`) | ✅ first lot (G29 lists what is left) |

## M9 Authoring parity for agent frameworks ✅

Gaps found by a real agent framework migrating to BIcraft (2026-10-14): objects BIcraft reads and keeps but no command writes, plus read and evaluation tools agents use daily.

| Id | Item | Status |
|---|---|---|
| M9.1 | Measure and column properties: `format_string_definition` (dynamic format strings), `sort_by_column`, `is_available_in_mdx`, `is_key`, `data_category` (with Desktop's refusal rules); annotations on model/table/column/measure; `model.set` (culture, discourage implicit measures, source query culture); removing date variations / auto date tables | ✅ engine-checked; Desktop 2.158 opens an edited PBIX and PBIP with every property live; screenshot (WebView2 capture) shows the dynamic format string on a card, table and axis, sort-by-column order and WebUrl links |
| M9.2 | `relationship.set` keeping the name; calculation groups and items; date tables (`table.set data_category Time`, key column); hierarchies completed | ✅ |
| M9.3 | Detailed reads: `model.get {detail:"full"}`, `measure.get`, `column.get`, `relationship.list`; VertiPaq storage statistics / VPAX export | ✅ `model.get` (summary/full), `table.get`, `column.get`, `measure.get`, `relationship.list`; `model.storage_stats` (streamed for large backups) equal to the engine DMVs on 489 figures; `model.export_vpax` / `bicraft vpax`, imported by DAX Studio 3.6.1 (Import Metrics) with the same tables, columns and sizes as its own extractor's VPAX |
| M9.4 | `dax.evaluate` / `bicraft dax-query` over the stored data (PBIX DataModel, PBIP cache.abf); `bicraft run --file commands.json` | ✅ query oracle 64/65 agree with the engine, 0 different values (1 refused) |
| M9.5 | `model.analyze`: Microsoft's MIT best-practice rules (microsoft/Analysis-Services) embedded, plus any rule file in the same JSON format passed by the caller; no Tabular Editor dependency | ✅ |
| M9.7 | `table.set_partition` (edit a partition's M/DAX source in place, kept lineage tags, no evaluation); `m.validate` (syntax + undefined names, no evaluation); `PBI_QueryOrder` kept up to date like Desktop; Desktop's default annotations on new tables/columns (`SummarizationSetBy`, `UnderlyingDateTimeDataType`, `PBI_ResultType`, …) | ✅ `partition.set` (alias `table.set_partition`), `partition.add/remove`, `expression.add/set/rename/remove`, `m.validate`, M in `model.validate`, `PBI_QueryOrder`, default annotations; Desktop 2.158 UI first-save (and refresh + save) diff: only Desktop-only changes left |
| M9.6 | `report.render` to PNG (faithful page rendering) | ✅ |

## M10 Languages and locales ✅

Today BIcraft formats, evaluates and renders in en-US only. Power BI Desktop follows the model culture and the user's locale (e.g. fr-FR: `0,3M`, "Total par Region").

| Id | Item | Status |
|---|---|---|
| M10.1 | Locale-aware number, date and currency formatting (format strings, display units, `FORMAT` in DAX and M, `Number.ToText`/`Date.ToText`) following the model culture | ✅ new L0 crate `bicraft-locale` (59 cultures, CLDR 48 + Desktop observations); DAX `FORMAT` 99.98 % and Power Query 99.9 % equal to Desktop 2.158 ([locales.md](../formats/locales.md)); `report.render` formats with it (separators, grouping, percent, display units, date order, 12-hour clock); the viewer's page.html follows the same rules (separators from the culture, K/M units, % as the format string writes it) |
| M10.2 | Localized auto-generated visual titles and labels ("par", "et", …) in the viewer and `report.render`, matching Desktop's UI language | ✅ `ui` strings, per-visual `auto_title` and field `label` in the snapshot (en, fr, de, es, it, nl, pt, ja); `view.set_language`, `report.render {ui_language}`, `--lang`; French and English checked in Desktop 2.158 (`cargo xtask gen-uilang`), other languages unverified; the renderer still has to draw `auto_title`/`ui.strings` (m96) |
| M10.3 | Culture-dependent parsing (CSV/M type inference, `Number.FromText`, currency symbols) and collation per model culture | ✅ `Number.FromText`/`Date.FromText`/`Currency.From` and `VALUE` per culture with currency symbols (`$2.675`), CSV currency columns; collation tailorings for 18 languages, 58/59 cultures in the engine's order |
| M10.4 | Model translations (cultures, translated captions) applied in reads, viewer and render | ✅ `culture.add/remove`, `translation.set/list`, renames follow and removals drop translations; `culture` on `model.get`/`*.get`, `view.set_culture` / `report.render {culture}` (snapshot captions, labels, auto titles); TMDL/TMSL/PBIX round trips; engine returns the captions per `LocaleIdentifier`; Desktop 2.158 opens an edited PBIX (model loaded; translated UI not viewed) |

## Storage engine: next steps

Next steps, in this order:

1. **Automatic aggregations derived from the report.** ✅ 2026-10-24 on `feat/report-aggregations` (G32, not merged; what is left in `docs/gaps.md` G32).
   - *Deterministic derivation* (a script, no agent): from the PBIR, read each page's visual
     fields, slicers, visual, page and report filters, and the measures used with the columns
     their DAX reads; deduce the grain needed, then a small set of hierarchical aggregations
     (for example day × store × product → month × region × category) covering the most visuals
     for the smallest size, the size bounded by a documented rule (a maximum fraction of the
     fact, a maximum number of aggregations).
   - *Measures covered*: `SUM`, `COUNT`, `COUNTROWS`, `MIN`, `MAX`, averages by sum and count,
     and everything computed on top of them (running totals, previous year, shares, ratios,
     `RANKX`, `TOPN`); `DISTINCTCOUNT` and filters on fact columns only when the column is in the
     grain; otherwise an exact fallback to the fact.
   - *Planning*: each storage request is rewritten to the smallest aggregation that covers it
     (grain ⊇ grouping + filtered columns, aggregation available); without cover, the fact.
   - *Building*: at refresh by `model.refresh`, and lazily on first use over a `cache.abf`
     (never written); stored compressed in BIcraft's store with the same encoder; invalidated by
     the data's, the model's and the report's revisions; one invalidation test per case.
   - *Exactness*: identical to the fact to the cent, blanks, orphan keys and relationships
     included; the rewrite joins the random differential test (each query on the aggregation
     and on the fact, compared cell by cell); the Desktop oracles stay green.
   - *Honest measure*: the bench publishes two columns, "without aggregations" and "with
     aggregations", plus the aggregations' size and build time; `engine-bench check` stays on
     "without aggregations" (the bare engine's guard does not change); for the bench, the
     aggregations are derived from a test PBIR report with one page per bench query, generated
     by xtask.
   - *Real trial*: a real local report (read only, unchanged afterwards), its
     pages rendered with and without aggregations and the values compared.
   - Documented in `docs/storage-engine.md` ("Aggregations"), new gap G32 in `docs/gaps.md`.
2. **Halve BIcraft's times** (median ≤ 0.5× of today's at every size and regime, no worst case
   degraded, zero wrong value). Profile first: per bench query at 1 M and 10 M, DAX analysis,
   planning, scan, formula engine, JSON serialization (about 4 ms per 1,000 result rows today),
   thread pool, allocation. Then, kept only when measured: release profile with fat LTO and one
   codegen unit, PGO trained on the micro-bench, AVX2 / AVX-512 kernels chosen at run time
   (portable binary, musl included), a cache of analysed and planned queries keyed by text and
   model revision, precomputed join indexes (the dimension attribute's code per fact row, packed
   and lazy), one page's visuals answered in one multi-aggregate pass, sort order chosen at
   encoding for BIcraft's own data (`bicraft-data.bin`), small costs (per-query arena, thread
   count, single-thread threshold, result serialization, memory-mapped ABF).
3. **Beyond RAM** (`docs/beyond-ram.md`): lots 1–3 (memory budget, backup in a temporary file, LRU eviction of decoded columns; streaming decompression, `DataModel` part left in the file; segments on demand) ✅ merged; lot 4 (one query's memory by bytes) and lot 5 (process-wide limit `BICRAFT_MEMORY_LIMIT`) 🔄 on `feat/caps`.
4. **Fast CSV ingestion** ✅ 2026-11-04 on `perf/csv-import` (not merged): `Csv.Document(File.Contents(…))` → `Table.PromoteHeaders` → `Table.TransformColumnTypes` and `read_csv` parsed in parallel chunks and typed straight into the columns, with the evaluator's results; files streamed without the 256 MiB cap under a memory budget (`docs/formats/data-sources.md` "Fast CSV loading", `docs/benchmarks.md` "CSV ingestion"). Left: a columnar `TableData` target.

