# Known gaps: open entries

The full entry of every open gap, moved unedited from `gaps.md` on 2026-11-25 and kept up to date
since. The one-line summary of each is in [gaps.md](gaps.md); closed entries are in
[history/gaps-done.md](history/gaps-done.md).

## P1

### G8. Distribution (M7.1 workflows not run)

- **Status.** `bicraft-cli setup` is done (M7.2): it registers the MCP server with Claude Code,
  Claude Desktop, VS Code, Cursor and Codex, with a dry run, backups, uninstall and
  `setup doctor` (`docs/mcp.md` §"Setup"). The CI (`.github/workflows/ci.yml`: Windows, Ubuntu,
  MSRV) and tag-triggered release (`release.yml`) workflows are prepared. By project policy
  nothing is pushed or published, so they have not run yet. On 2026-11-27 the binaries were
  renamed as in DeckCraft (the desktop app `bicraft`, the CLI `bicraft-cli`), both archives of
  `cargo xtask dist` carry both programs, the Linux archive is a glibc build with the app's desktop
  integration, and `release.yml` runs `cargo xtask dist` per OS ([releasing.md](releasing.md)), for
  the first release, v0.1.0.
- **What is left.**
  - The CI and release workflows have never run on GitHub.
  - No binaries are published.
- **Why it matters.** Today a user needs a Rust toolchain to try the tool.
- **Where.** The `.github/` workflows, `apps/bicraft-cli`.
- **Effort.** S–M.
- **Depends on.** The decision to publish the repository.

### G10. Large-model storage layouts have not been seen — fixed

- **Status.** Fixed on 2026-10-14 by the large-model oracle
  ([`vertipaq-large.md`](formats/vertipaq-large.md),
  `crates/vertipaq/tests/large.rs`, data local only): synthetic models of 1M, 5M and 20M rows,
  200 tables, multi-segment and multi-partition columns, 5M distinct strings, Huffman pages,
  every data type with blanks, relationships and hierarchies, processed by a local engine instance.
  - Reader: all 968 user columns match the engine's DAX aggregates; two reader bugs fixed
    (`.hidx` bins spilling to the overflow list, 1-bit Huffman codes).
  - Encoder and system tables: byte-identical with the engine on every file but one RLE
    allocation (3 392/3 394 column files, 1 888/1 890 `H$`, 202 `R$`, 12 `U$`); rules fixed for
    `bookmark_bits`, `.hidx` growth, string page size and compression, value-encoded doubles,
    statistics.
  - Restore: images with every hash column re-encoded and every system table rebuilt load
    into the engine (AMO `ImageLoad`) and answer every query identically.
  - **Incremental refresh and rare layouts** (second round, `vertipaq-large.md` §5; cases
    `incr`, `hybrid`, `rare`, small, data local only): a table with a refresh policy and 8
    policy-range partitions (one DirectQuery in hybrid mode), and columns of blanks, one-value
    and 100 000-character strings, int64 near ±2^62, scaled doubles, int64 sweeps. Every
    column decodes like the engine; BIcraft's metadata-only save (measure added) and
    structural save (rename, column removed, table + relationship added, new columns on the
    8-partition table) load into the engine with the policy, partitions and every answer
    intact; re-encoded images of all three load too.
  - Fixed: columns of blanks have no `.dictionary` (reader, `bicraft-pbix`, preview, encoder);
    1-bit Huffman pages decode to 8 characters per byte (reader bound); empty partitions and
    negative double magnitudes (encoder); the int64 hash-versus-value choice (at most 20 rows
    per distinct value, ids below 2^31: 117/117 engine columns); new columns on
    multi-partition tables (structural writer); the RLE statistics are sums over segments and
    partitions.
  - **Left (minor, no load impact):** `General` class and ids 0/1 (never produced),
    `DictionaryStorage.Size` beyond the fitted formulas, the RLE allocation of re-sorted
    segments, the exact int64 threshold (20–22 rows per value), replacing the data of a
    multi-partition table (refused), skipping `H$` for new columns of hybrid tables (the
    engine materializes none; it loads ours).
- **What (original).** One local sample does not cover these:
  - **Segments and partitions.** Multi-segment and multi-partition columns. Only segment 0 has been
    seen, so the `.idf` naming for later segments, the default segment size and blank packing are
    unknown (`vertipaq.md` open questions, `abf.md` "Naming of multi-segment `.idf` files").
  - **Strings.** Huffman-compressed string pages: `decode_bits` is unknown.
  - **Rare encodings.**
    - Some `XMRENoSplit` widths and the `General` compression class.
    - Data ids 0 and 1 in user columns.
    - Inconsistent `DistinctStates` statistics.
  - **Pbixray cross-check.** 4 of 44 real columns could not be cross-checked against pbixray.
  - **Incremental refresh.** Policy-range partitions are mapped (G16); their data layout is unobserved.
- **Why it matters.**
  - Reading data, and writing it once G1 lands, may fail on exactly the models where users need a
    tool most.
  - Model metadata is not affected.
- **Where.** `crates/vertipaq`, `crates/abf`, `docs/formats/vertipaq.md` §"Open questions".
- **Effort.** S for what is left.
- **Depends on.** Nothing.
- **Storage mode authoring (2026-11-19).** `table.set_storage_mode` writes Import, DirectQuery
  and Dual (composite models) with Desktop's rules, and the Model view shows and changes them
  ([model-objects.md](formats/model-objects.md), "Storage mode"). Left:
  - **No DirectQuery execution.** BIcraft evaluates imported data only: a DirectQuery table has
    no rows, and its visuals are `unavailable` with "not evaluated: DirectQuery" (Dual tables
    use their imported rows). Running a visual's query against the source (SQL generation,
    folding) is not built.
  - One DirectQuery source group: limited relationships between two DirectQuery sources are not
    detected; Desktop's "minimum set" of Dual tables is approximated by every DirectQuery table
    on the one side; an Import table a PBIX stores cannot change mode (edit a PBIP).
  - Nothing opened in Desktop.
- **Incremental refresh authoring (2026-11-19).** `table.set_refresh_policy` and
  `table.remove_refresh_policy` write a table's `refreshPolicy`, checked against the RangeStart and
  RangeEnd parameters and the query's filter step, from a table's menu › Incremental refresh
  ([model-objects.md](formats/model-objects.md), "Incremental refresh policy"). Left:
  - BIcraft does not apply the policy (no range partitions are created; the service does it).
  - The filter step check finds references to RangeStart, RangeEnd and `Table.SelectRows`, not
    which bound has the equal sign; the polling expression is BIcraft's own text.
  - Nothing opened in Desktop or published.

## P2

### G12. Thin PBIX not yet opened against a published model (M3.2)

- **Not pursued, by the maintainers' decision (2026-11-24).** Opening a thin PBIX signed in,
  against a model published in the Power BI service, needs a Power BI account, a published
  dataset and a sign-in BIcraft never makes; it is accepted as a known limitation. BIcraft
  reads and edits a thin report's report, and reports its live connection: `file.open` and
  `file.inspect` give `live_connection` (`type`, `data_source`, `catalog`, read from the
  `Connections` part; the rest of the connection string is never shown), the snapshot's
  `source.live_connection` too, and the model error and the Model view say that the model is
  published in the Power BI service, that BIcraft does not sign in, and that this is not pursued
  (tests: `a_thin_reports_live_connection_is_read_without_the_rest_of_its_string` in
  `crates/pbix`, `a_thin_pbix_says_its_model_is_published_and_not_opened` in `crates/engine`,
  `a_thin_report_says_its_model_is_published` in `crates/ui`).
- **Seen in Desktop (2026-11-23, Power BI Desktop 2.159.1231.0, Store edition, not signed
  in).** A thin PBIX BIcraft wrote (`write_thin_pbix_documents`, one page with a card on a
  measure; `Connections` = `pbiServiceXmlaStyleLive`, `Data Source=powerbi://…/myorg/<workspace>;
  Initial Catalog=<model>`) shows an error dialog ("object reference not set to an instance of
  an object") and Desktop then opens a blank untitled report: no page of the file, no visual.
  The same package with only `Connections` changed to `analysisServicesDatabaseLive` and an
  unreachable `localhost` server gets Desktop's "cannot connect" dialog (Retry, Edit, Cancel)
  instead, so the parts are read and the failure is tied to the service connection while
  signed out. Whether a signed-in Desktop binds the minimal connection string is still open.
- **What.** These parts of a thin PBIX are unverified (`pbit.md` open questions):
  - the minimal connection string (`Data Source=…;Initial Catalog=…`)
  - percent-escaped part names
  - the mashup package version `2.130.0.0`
- **Why it matters.** The file may not open, and the first user to try it finds out.
- **Where.** `crates/pbixwriter`, `docs/formats/pbit.md`.
- **Effort.** S.
- **Depends on.** A published dataset to connect to.

### G14. Report validation does not check everything

- **Done.** Data roles are checked per built-in visual type (role names, kinds, field counts and
  accepted combinations from black-box tests against Desktop 2.158, `docs/formats/visual-roles-bb.md`)
  in `visual.add`, `visual.bind`, `report.apply` and `report.validate`. Report-level measures
  (`reportExtensions.json` / legacy `modelExtensions`) are read, their DAX validated against the
  model, bindable by name, and edited with `report_measure.*`.
- **Done (2026-10-14).** `report.validate` also checks (`crates/engine/src/report_checks.rs`,
  `docs/formats/pbir.md` "Report validation against Desktop"):
  - every filter container (report, page, visual, bookmark filter states): the filter `type`
    against its field (relative dates need a date column; categorical types on measures) and the
    shape of its conditions (`TopN` subquery, relative dates, value lists), text operators on
    non-text columns, literals of the wrong type for their column;
  - `Hierarchy`, `HierarchyLevel` and `PropertyVariationSource` references in visuals, filters and
    bookmarks against the model's hierarchies, levels and date variations;
  - bookmark targets: active page, section pages, visuals and groups, captured visual types,
    target visuals.

  Calibrated with Desktop 2.158 through UI Automation on generated projects: what makes a visual
  show an error (relative date on a non-date, wrong literal type, text operator on a number,
  missing hierarchy or level) is an error; filter types Desktop runs anyway are warnings; Desktop
  opens and applies bookmarks whatever they point to, so bookmark findings stay warnings.
- **Done (2026-10-15).** Condition `kind`s (scatter chart, bubble map, Azure Maps: with details,
  X/Y must be summarized) and the "hierarchically related" rule of multi-field slicers and list
  slicers (Desktop's relationship-chain search over active relationships) are errors; a scorecard
  with a field is an error; Q&A and AI narratives visuals warn (fields ignored, Q&A retired in
  February 2027). Checked in Desktop 2.158 on generated PBIX files (`docs/formats/visual-roles-bb.md`,
  "Kinds and relationships"), plus: relative-date filters on a
  measure break the visual (error, as encoded), `VisualTopN` with an item count or a plain list runs
  (as encoded), a generated `reportExtensions.json` opens and its measure shows. No new error or
  warning on the 32 local reports (incl. the render-oracle reports).
- **Done (2026-11-24).** `report.validate` also checks the column a field takes from a date
  variation's date table (`Column` over a `PropertyVariationSource`, an auto date/time level
  bound without its hierarchy): a column the date table lacks is an error (a warning in
  bookmarks), next to the existing check of the variation's name
  (`date_variation_columns_are_resolved`). Legacy-layout report measures: `report_measure.add`
  and `.set` write the shape Desktop writes in `config.modelExtensions` (LO-report:
  `dataType` as its `PrimitiveType` code from `microsoft/powerbi-visuals-utils-typeutils`,
  `errorMessage`, `hidden`, `formulaOverride`, `formatInformation`), and `report.validate` warns
  on a `dataType` that is no known code and on a document a PBIR save would leave out
  (`legacy_layouts_take_and_check_report_measures`).
- **What is left.**
  - Legacy report measures: a legacy layout without `modelExtensions` still refuses
    `report_measure.add` (the first extension's name and place were not observed);
    `description` and `display_folder` are refused there (their legacy names are only inferred);
    `formatInformation.format` is written `General` as observed, its meaning unknown; a legacy
    layout edited this way was not opened in Desktop.
  - Not tried in Desktop (no Desktop run on 2026-11-24): date variations
    (`PropertyVariationSource`) with a wrong name or a wrong date-table column (errors, as the
    other unresolved fields); maps with a wrong kind (map visuals are disabled on the test
    machine; verdict from the capabilities).
  - Seen while testing, not checked by `report.validate`: the same aggregation in two roles of a
    scatter chart (`queryRef` suffixed `1`) makes Desktop fail the query; a one-to-one relationship
    with single-direction cross filtering stops Desktop opening the file.
- **Where.** `crates/visuals/src/roles_bb.rs`, `crates/engine/src/roles.rs`,
  `crates/engine/src/report_measures.rs`, `crates/engine/src/report_checks.rs`.
- **Effort.** S.
- **Depends on.** A model with auto date/time tables for the variation case.

### G16. `metadata.sqlitedb` objects that are not mapped — mostly fixed

- **Done.** Every object TOM exposes is read and, where no stored data depends on it, written
  through one declarative mapping (`crates/metadata/src/spec.rs`): translations, linguistic
  metadata, perspectives, calculation-group selection expressions, refresh policies and
  policy-range partitions, calendars, related column details, alternateOf, data coverage
  definitions, data sources, query / entity partition sources, functions, and annotations,
  extended and changed properties on every object. Checked on Desktop's engine in both
  directions (engine-written files read equal to the engine's TMSL; files written by BIcraft
  loaded by the engine and scripted back equal); fixtures in `crates/metadata/tests/engine`,
  status table in `metadata-sqlitedb.md` §"Nested objects".
- **Done 2026-10-14.**
  - Compatibility level 1000000 on the same engine (case `preview`): sets and perspective sets
    (the engine needs sets enabled), `bindingInfoCollection`,
    `analyticsAIMetadata`, `excludedArtifacts` on model / table / expression,
    `metadataAccessPolicy`, the Direct Lake indexing behaviors; read and add / change / remove
    written and loaded back by the engine. The earlier "no TMSL shape" verdict came from a wrong
    collection name (`bindingInfos`) and a too-low level. `metadata-sqlitedb.md`
    §"Compatibility level 1000000".
  - Encrypted data-source secrets: opaque, listed by `encrypted_secrets`, kept byte for byte by
    every write; the engine still decrypts them after loading a file BIcraft rewrote. Adding a
    data source stays refused (the engine requires an encrypted credential or connection
    string). §"Encrypted data-source secrets".
  - Direct Lake: Desktop's engine refuses `mode: directLake` at every level ("not available in
    this product") but accepts the same entity partition in DirectQuery mode; the row shape,
    the framing tables (`DeltaTableMetadataStorage`, `ParquetFileStorage`,
    `DeltaTableColumnStorage`) and the `parquet` source code are documented from that and the
    published TOM, and Direct Lake rows read and survive metadata edits (synthetic test).
    §"Direct Lake".
  - Removing a table now also removes its sets (regression test); BIcraft's TMDL round-trips
    the new kinds (`excludedArtifact`, `analyticsAIMetadata`).
- **What is left.**
  - A Fabric-written Direct Lake file, to observe filled framing rows and confirm the
    `parquet` source's `location` column (inferred, unobserved).
  - Not TOM objects at all, so not mapped: partition `MAttributes`.
  - TMDL spelling of the level-1000000 objects: the public TOM serializer (17.0.87.18) does not
    know them, so BIcraft's spelling follows TOM's naming rules without a reference.
  - Writes not confirmed on the engine: Azure AD role members (an offline engine cannot load
    them), removing functions / query groups / data sources, adding or removing refresh policies.
  - Unknown enum codes are dropped; compatibility level and database name are left empty; measure
    `DataType` is written as variant for non-literal expressions.
- **Why it matters.** Models that use these features lose information when read, or show it
  incompletely to agents.
- **Where.** `crates/metadata`.
- **Effort.** S for what is left.
- **Depends on.** A Fabric sample for filled Direct Lake framing rows.

### G17. Writer behaviour Desktop has not confirmed — mostly fixed

- **Status.** Mostly fixed on 2026-10-15, with variants of a PBIX BIcraft built, loaded by the
  engine of Desktop 2.158.1304.0 (`ImageLoad`) and opened in Desktop (cases
  `variants` / `chunks` / `markers`; `xpress9.md` §6.2):
  - single-threaded framing: accepted at every size tried; a bare ABF also opens;
  - session signature: any constant loads (0, all ones, ours); a change mid-stream, a wrong
    header CRC or restarted block indices are refused;
  - chunk size: at most 2 MiB (2 MiB + 1 byte is refused, by Desktop too). **Fixed:** the writer
    refused nothing up to 4 GiB and took up to 64 MiB from a sample; it now refuses more than
    2 MiB;
  - other windows and MTF settings load (the writer keeps Desktop's);
  - ABF `ErrorCode` true without markers fails: the flag is honoured; Power BI never sets it;
  - restore checks: `DBPROPERTIES` markers and row times moved to 1970 or 2174 load;
  - `DictionaryStorage.Size`: **fixed**, derived from 702 engine dictionaries and three sweeps
    of our own; long, real and string dictionaries follow the engine's in-memory hash table
    (`vertipaq.md`, "Dictionary size"), exact without hash collisions (the engine's 64-bit and
    string hashes are not identified, so their 12- or 8-byte collision entries are left out).

  Left: other Desktop releases (schema drift; only 2.158 is installed here), Excel CRC markers
  and `ApplyCompression` (no sample), the two hash functions.

- **What.** These have not been checked against Desktop:
  - **XPress9 framing.**
    - Whether a single-threaded `DataModel` stream is accepted at any size. The writer only
      emits that form. Accepted so far up to the local samples' 0.9 MiB of ABF, including
      parts of 15 chunks whose blocks were compressed independently (`CompressionMode::Fast`,
      fresh MTF state per block, renumbered indices; checked 2026-10-14). A local engine instance's
      AMO `ImageLoad` also accepted single-threaded images of 578 MB and 845 MB of ABF, where
      its own `ImageSave` writes the multithreaded form (`vertipaq-large.md`, 2026-10-14).
    - The content-derived session seed (any constant signature loads, `xpress9.md` §6.2).
    - The 2 MiB chunk size, which is inferred.
  - **ABF CRC markers.** Excel backups are unsupported. `ApplyCompression` (xpress8) is
    unsupported.
  - **`metadata.sqlitedb` schema drift.** Only Desktop 2.158 has been seen.
  - **Restore checks.** Whether a restore checks timestamps, storage names or the `DBPROPERTIES`
    markers. Seen with `ImageLoad`: replaced files keep their timestamps and storage names and
    load; stale `DictionaryStorage.Size` values load; `H$` tables are consistency-checked,
    `R$` indexes are not.

  Sources: `xpress9.md`, `abf.md`, `datamodel-abf.md`, `metadata-sqlitedb.md`.
- **Why it matters.** A file that opens with 2.158 may not open with other Desktop releases or
  other model sizes.
- **Where.** `crates/xpress9`, `crates/abf`, `crates/metadata`.
- **Effort.** Varies. Most items need a Desktop session per release.
- **Depends on.** The Desktop oracle (Windows only).

### G19. Gaps in the DAX type vocabulary — mostly fixed

- **Status.** Mostly fixed on 2026-10-13, with additive changes to `types.rs`; `signature()` is
  unchanged.
  - `Param::group` carries the engine's `REPEATGROUP` for `SUMMARIZECOLUMNS`, `SUMMARIZE`,
    `GROUPBY` and `ADDMISSINGITEMS`.
  - `Signature::min_args` carries their engine minimum. This removes 3 of the 8 arity deviations.
    The 5 left are justified probe artifacts and documented choices: `TRUE`, `FALSE`, `NOT`,
    `USERNAME`, `RANK`.
  - New kinds: `Dates`, `Calendar`, `MeasureRef`, `TableOrColumn`, `Relation`, `OrderBy`,
    `PartitionBy`, `MatchBy`, `KeywordOrScalar`.
  - The type checker binds the multi-section functions' arguments by shape and checks the order
    of their sections (oracle category `sections`).
  - Filter modifiers, 2026-10-14. The type checker marks `USERELATIONSHIP`, `CROSSFILTER`,
    `REMOVEFILTERS`, `ALLCROSSFILTERED`, `KEEPFILTERS`, `NONVISUAL` and `ALL()`/`ALLSELECTED()`
    without arguments as modifiers, with an internal marker. Their serialized result type and
    `signature()` are unchanged. It checks where each one stands, following engine rules probed
    with 100 corpus entries (category `modifiers`):
    - `misplaced-filter-modifier` errors, for example `COUNTROWS(KEEPFILTERS(…))`,
      `SUMX(REMOVEFILTERS(T), …)`, or `USERELATIONSHIP` outside a `CALCULATE` filter
    - `runtime-misplaced-filter-modifier` warnings where a measure's definition is accepted but
      evaluation fails, for example `COUNTROWS(REMOVEFILTERS(T))`
    - `ALL` and `ALLSELECTED` with arguments stay table functions
  - Signature corpus, 2026-11-23 (M11.40, [dax-oracle.md](dax-oracle.md) "Signature corpus"):
    the argument counts of all 472 catalogued functions and 3807 generated signature probes,
    recorded on the engine. 467 functions have the engine's counts (the 5 deviations above);
    3750 probes agree. The type checker now refuses constants for keyword, window and
    measure-reference parameters, table expressions for table references, constant filters,
    visual-calculation functions outside visual calculations, `IGNORE`-like functions outside
    their parent, calendar-only arguments without a calendar, and wrong row widths in
    `CONTAINSROW`/`TREATAS`.
  - Still open: `SAMPLEAXISWITHLOCALMINMAX`'s second section, and the 33 probe deviations of
    `PROBE_DEVIATIONS` in `crates/dax/tests/oracle/signatures.rs` (they cover 91 probes, 57 of which disagree): mostly
    `ADDMISSINGITEMS` and `SAMPLEAXISWITHLOCALMINMAX` section shapes, `INFO.*` functions this
    engine edition lacks or whose restrictions it checks against its own names, `TOPNSKIP`,
    `ISAFTER`/`ISONORAFTER` with 40 arguments, `EXTERNALMEASURE`, `TOCSV`/`TOJSON` given text.
    The M catalogue is recorded against Power BI Desktop's M engine (G108).

  The rest of this entry is the state before the fix.
- **What.**
  - **Flattened multi-section functions.** There is no repeat-group id per parameter, so these
    are flattened: `SUMMARIZECOLUMNS`, `GROUPBY`, `SUMMARIZE`, `ADDMISSINGITEMS`,
    `SAMPLEAXISWITHLOCALMINMAX`.
  - **Missing kinds.** There is no kind for:
    - keyword-or-scalar sort orders
    - `dates` arguments
    - calendar references
    - measure references
    - "table or column" (`DISTINCT`)
    - filter modifiers
  - **Arity.** 8 functions deviate deliberately from the engine.

  Source: `dax-functions.md` §"Gaps in the type vocabulary".
- **Why it matters.** Argument checks are weaker for these functions.
- **Where.** `crates/dax/src/types.rs` (frozen API), `crates/dax/src/catalog`.
- **Effort.** M.
- **Depends on.** Agreement to change the frozen `types.rs` API.

### G21. `bicraft-data` evaluates literals only (mostly fixed 2026-10-14)

- **Done.** `crates/data/src/m/` holds a sandboxed Power Query evaluator. The supported surface
  and the Desktop comparison are in [formats/data-sources.md](formats/data-sources.md), *Power
  Query (M)*.
  - **Language and library:** `let`, `each`, `try`, records and lists, Desktop's "Enter data",
    about 340 `Table.*`/`List.*`/`Text.*`/`Number.*`/`Date.*` functions, and `#duration` columns.
  - **Files:** `Csv.Document`, `Json.Document` and `Excel.Workbook` over `File.Contents`, under an
    explicit file root only.
  - **Caps:** depth, steps, time, rows and bytes.
  - **Options:** `SourceOptions` and `load_partition_opts` carry the file root, culture,
    shared queries and parameters, and the error mode.
  - **Desktop agreement:** all 36 Desktop-refreshed test tables are identical, plus 52 decimal
    probes and a typed load table (2026-10-14).
  - **Numbers:** M numbers are doubles or exact .NET-style decimals, as in Desktop: literals and
    number text with more than 15 digits stay decimal; `Number.Round` keeps the scale on ties
    (`2.675` → `2.680`); `Precision.Decimal`; decimal formats; decimals load exactly into
    whole-number and currency columns (`crates/data/src/m/decimal.rs`).
  - **Engine (2026-10-14):** `table.import {m}`, structural PBIX saves and the viewer run M
    queries with the model's culture and the queries/parameters they name; files only under an
    opt-in `data_root` (`crates/engine/src/datasrc.rs`). Text is stored as Analysis Services
    stores it (`TextStorage`: XML-invalid controls and trailing blanks dropped on import, one
    spelling per equal value, en-US = ASCII case only); a PBIX written this way matched Desktop's
    own refresh value for value (en-US, fr-FR).
  - **Folders (2026-10-14):** `Folder.Files` and `Folder.Contents` under an allow-list of roots
    (`SourceOptions::file_roots`), file contents read only when used, links never followed out
    of the roots; identical to Desktop on a synthetic tree (Desktop round 4), dates aside (UTC).
  - **External connectors (2026-10-14):** `Sql.Database`, `Web.Contents`, `OData.Feed`,
    `Python.Execute` and the other database, web and script connectors are never run:
    `Error::ExternalSource` ("external source, not evaluated"); the query is kept as written.
  - **Databricks and refresh (2026-10-23):** a connector interface in `bicraft-data`
    (`connect.rs`: lazy navigation and remote tables, folding of `Table.SelectColumns`,
    `RemoveColumns`, `FirstN` and `SelectRows` conjunctions in exact M semantics, everything else
    local) and the Databricks connector (`crates/databricks`: SQL Statement Execution API,
    parameterized SQL, polling, chunks, optional external links, retries, types), run only when
    the caller allows it: `model.refresh {allow_connectors: ["databricks"]}`, CLI `bicraft
    refresh`, MCP `refresh_model`. The rows, calculated tables and columns go to bicraft's own
    snapshot `.pbi/bicraft-data.bin` (`cache.abf` is never written), read by `dax.evaluate`,
    `report.render` and the viewer while it is newer than `cache.abf`. Token from
    `BICRAFT_DATABRICKS_TOKEN` (or a masked prompt), never stored or shown. 11 end-to-end tests
    against a local fake of the API, 5 engine tests, 11 evaluator tests
    ([formats/data-sources.md](formats/data-sources.md), *Connectors*).
  - **DAX (2026-10-14):** calculated tables over the model's data are evaluated by the engine's
    DAX evaluator (the layer rules keep it out of `bicraft-data`): PBIX saves, and now
    `table.import {dax}` / `table.set_data {dax}`. 66 calculated-table probes (`SUMMARIZECOLUMNS`,
    `ADDCOLUMNS`, `FILTER`, `SELECTCOLUMNS`, `GENERATESERIES`, `CALENDARAUTO`, `UNION`,
    `DISTINCT`/`VALUES`, `TOPN`, text and `FORMAT` functions, …) match the engine of Desktop 2.158
    in column names, data types, source columns and rows
    (`crates/engine/tests/calc_tables/`). Fixed on the way: context transition inside `CALCULATE`
    and nested row contexts, currency result types, blank text results, `UNION` source columns,
    mixed table constructors, `CALENDARAUTO` without dates; 31 text, conversion and date-part
    functions added, `FORMAT` among them.
- **Left.**
  - **M:**
    - right and full outer joins
    - two-argument comparers
    - error messages are English (`Number.FromText` with currency symbols such as `$2.675` and
      the per-culture conversions were added in M10.1/M10.3, [locales.md](formats/locales.md))
    - `Folder.*` dates are read in UTC (Desktop: local time); `Kind`/`Content Type` checked for
      `.csv`, `.txt`, `.json` and no extension only
  - **DAX:** `SEARCH`/`CONTAINSSTRING` wildcards (`FORMAT` follows the model culture since
    M10.1); whatever else the evaluator does not support is refused with the reason.
  - **Connectors:** a first run against a real Databricks workspace (the user's, with their
    token); Arrow results; OAuth U2M; other connectors (SQL Server, Snowflake, …); comparing the
    Databricks type mapping with Desktop's connector; proxies.
- **Where.** `crates/data`, `crates/databricks`, `crates/engine/src/refresh.rs`,
  `crates/engine/src/preview_eval*.rs`, `crates/engine/src/pbix_struct.rs`.
- **Effort.** S for what is left.
- **Depends on.** G1.

### G22. Patent and OSP coverage

- **What.** ~~The `[MS-XLDM]` Open Specification Promise and the MIT-licensed XPress9 source,
  which has no patent grant, have not been reviewed for coverage.~~ **Researched** 2026-10-14 by
  the maintainers (private notes, not legal advice).
  - `[MS-XLDM]` and ECMA-376 (OPC) are on the OSP list, but the OSP covers only the parts that
    conform to the spec.
  - XPress9 is in no open spec and its MIT license grants no patents. No Microsoft patent on it
    was found; a candidate list is marked unverified.
- **Still open.**
  - Read the Patent and Program Map entries.
  - Have counsel review the XPress9 patent candidates and the MIT implied-license question.
  - Work through the maintainers' checklist.
- **Why it matters.** This is a project risk for redistribution, not an engineering task.
- **Where.** Legal review.
- **Effort.** Not an engineering estimate.
- **Depends on.** Nothing.

### G119. BIcraft visuals' format options against the standard visuals (2026-11-25)

- **Audit.** [formats/bicraft-visuals-format-gaps.md](formats/bicraft-visuals-format-gaps.md)
  compares the Format pane of each of BIcraft's 14 visuals with the closest standard visual:
  on `main` none offered a font family, bold / italic / underline, display units or decimal
  places; only the org chart had a text colour; axis labels and titles had no option outside the
  Spec visual's JSON.
- **Done (Spec 1.5.0.0).** The shared runtime's text cards (`runtime/textsettings.ts`,
  `crate::visual_runtime::TEXT_PROPS`): Data labels, X axis (labels and title), Y axis (labels,
  title, display units, decimals) and Legend, each with the font row (family, size, bold, italic,
  underline), a colour and a show switch, with Power BI's property names; the Spec visual takes
  them over its spec's new `text` key ([formats/spec-visual.md](formats/spec-visual.md) "Text").
  Every bound measure is drawn as its own series when the spec draws one plain series
  ("Series"), else the visual says how many measures it leaves out. Numbers follow the report's
  language ("Numbers in the report's language"). Every other visual's version was raised for the
  runtime it bundles; their own cards are unchanged.
- **Left.** The text cards on the 13 other visuals (each needs its layout to take the sizes and
  colours; the note lists the most used per visual); legend position; axis range and gridline
  options in the pane (the Spec visual has them in JSON only); the text family is not measured
  (widths are estimated from character counts); the Format pane's new cards not yet opened in
  Desktop (the canvas was: 2.160.1151.0 draws two measures as two series, the label size and
  display units set through `labels` and French numbers, [formats/spec-visual.md](formats/spec-visual.md)
  "Seen in Desktop").
- **Effort.** M (per visual: S to M).

## P3

### G29. Viewer interactivity — first lot done, aligned on the interaction oracle

- **Status.** Done on 2026-10-23 (M8.3, branch `feat/viewer-interactive`): `bicraft-cli view` is a
  report reader. Slicer clicks (list, dropdown, tiles, list and button slicers, range, relative
  date, text), synced slicers, cross-filtering and highlighting on data points (bars, columns,
  combo columns, line points, pie and donut slices, funnel stages, treemap tiles, table rows) with
  Ctrl+click and the page's `visualInteractions`, tooltips, page navigation (tabs,
  `pageNavigator`, `PageNavigation` and `Back` buttons), "Apply all slicers" (pending selections)
  and "Clear all slicers". The state lives in the session (`view.interact`, `view.interactions`),
  never in the files; the data are recomputed by BIcraft's evaluator ([view-snapshot.md](view-snapshot.md),
  "Interactions").
- **Checked** against the Desktop 2.159 interaction oracle ([interaction-oracle.md](interaction-oracle.md)):
  the 20 click states played on `view.interact` give 180 / 180 visuals and 12 / 12 slicer tick
  sets identical to Desktop (highlighted values included); the 19 file states 171 / 171
  (`cargo test -p xtask interaction`, floors 180, 12 and 171). The oracle corrected the first
  lot's assumptions: the default interaction is a highlight on visuals that can highlight
  (the types of the highlight probe, interaction-oracle.md), a filter elsewhere (`DataFilter`, `HighlightFilter`, `NoFilter` as
  Desktop names them); a click on a synced slicer reaches its copies; a selected value the other
  filters leave out stays listed and ticked.
- **What is left.**
  - A highlight is computed by a second evaluation of the visual, joined on the grouping values:
    Desktop's query (`IGNORE(CALCULATE(m, KEEPFILTERS(<selection>)))`) waits for `IGNORE` in the
    DAX evaluator (branch `feat/dax-coverage`).
  - Drawing not compared with Desktop's: the pie highlight (share of the slice's area), ribbon
    charts, line series of combos and Q&A visuals (decomposition trees: G34).
  - A selection of several points over columns of several tables filters each table by its own
    values: exact for one point or one table, a superset otherwise (a filter condition reads one
    table).
  - No point selection in matrices, scatter and waterfall charts, gauges, KPIs, cards and
    small-multiple panels (ribbon charts select their segments since 2026-11-20). Drill, matrix expand and collapse, drillthrough and report-page
    tooltips are done since 2026-11-06 (G51); bookmarks are applied by the readers since
    2026-11-18 (G55).
  - Hierarchy slicers select on their first column; `strictSingleSelect` / `singleSelect` are
    not in the oracle corpus (kept from the format pane's names).
  - `mcp --view` stays read-only (its idle hook sees the session read-only).
  - Edit interactions (2026-11-06, G47): the rules are edited in the app's mode, and the report
    setting `defaultFilterActionIsDataFilter` is honoured (no oracle state records it yet; the
    viewer's rule: a selection filters the visuals that could highlight unless their rule says
    `HighlightFilter`, which now highlights explicitly).
  - BIcraft's ten visuals (2026-11-24): done in the app and the viewer, both directions. Their
    layouts record which rows each mark stands for, as their TypeScript runtime does
    (`bicraft_render::own_hits`): a click sends `select {point, points?}` (a histogram bin, a
    Sankey node or a Venn set selects every point it covers), Ctrl adds, the same click or the
    background clears, the points left out are drawn at 40 %, hovering shows the visual's
    tooltip rows. A selection elsewhere highlights the nine with a measure field since
    2026-11-24 (G114, `supportsHighlight`); slicers filter them.
    The app walks a visual's points from the keyboard (Ctrl+Right, arrows, Enter or Space,
    Ctrl+Space, Escape; `crates/ui/src/point_keys.rs`, for every visual with data points). A
    data-point selection is captured in bookmarks as the container's `highlight` (public
    bookmark schema 2.1.0) and restored by `bookmark.apply` and the readers. The viewer's page
    draws them from the publisher's `drawing` (view-snapshot.md "BIcraft visuals"). Since
    2026-11-24 See records on a mark of several points lists the rows of every point (the hit's
    `points` reach `visual.records`, a `UNION` of one `CALCULATETABLE` per point), and the org
    chart's collapse buttons are hit regions (`view.interact` `tree_toggle`: the app saves it
    with `visual.toggle_node`, a reader keeps it in the view state). Left: the `highlight` shape is not yet
    compared with a bookmark Desktop writes; a mark selects at most 1,000 points; the viewer's
    image is redrawn by the publisher at each snapshot (64 visuals at most) and its hover is
    not drawn there.
- **Where.** `crates/engine/src/interact.rs`, `crates/view` (`POST /api/interact`, page),
  `apps/bicraft-cli/src/view_cmd.rs`, `xtask/src/oracles/interactions_tests.rs`.
- **Effort.** M for what is left.
- **Depends on.** `IGNORE` in the DAX evaluator, for the one-query highlight.

### G30. DAX evaluator coverage — mostly fixed

- **Status.** Mostly fixed on 2026-10-23 (branch `feat/dax-coverage`). See
  `docs/formats/dax-functions.md` §"Evaluator coverage" for the functions added, the behavior
  taken from the engine, what is still refused and the known differences; `docs/dax-oracle.md`
  §"Function coverage" for the corpus (501 queries recorded on Desktop 2.159).
- **What.** Functions outside the evaluator are refused (`not supported`), so a measure using
  them could not be evaluated without Desktop (an evaluation harness would count them as "not evaluated").
  Calculation groups were not applied, dynamic format strings not evaluated, `UTCNOW()` was
  `NOW()`.
- **Lot 20 (2026-11-18).** `OFFSET`, `INDEX`, `WINDOW`, `RANK`, `ROWNUMBER` (`ORDERBY`,
  `PARTITIONBY`, `MATCHBY`, blanks, ties), `GROUPBY` with `CURRENTGROUP()`, `NATURALINNERJOIN`,
  `NATURALLEFTOUTERJOIN` and the month, quarter and year opening and closing balances are evaluated
  (`crates/engine/src/preview_eval_window.rs`); the 15 coverage-oracle entries for them agree, the
  other tests' values are documented, not oracle-checked; several choices are unconfirmed (several
  current rows, tie-breaking, blank joins, the opening balance's day:
  [formats/dax-functions.md](formats/dax-functions.md) "Evaluator coverage"); a window over a
  large relation in every cell is O(n²) within the work cap.
- **2026-11-20.** `ADDMISSINGITEMS`, `SUBSTITUTEWITHINDEX`, `SAMPLE` (with an order key) and
  `TOPNSKIP` are evaluated (`crates/engine/src/preview_eval_pick.rs`, rules in
  [formats/dax-functions.md](formats/dax-functions.md) "Evaluator coverage"); their four
  coverage-oracle entries agree, the other tests' values are documented, not oracle-checked.
  Unconfirmed choices: `SAMPLE`'s default order (ascending), where `SUBSTITUTEWITHINDEX` puts the
  index column (the first common column's place) and its error without a common column,
  `TOPNSKIP`'s tie rule (only identical rows come along).
- **2026-11-25** (branch `feat/dax-leftovers`). `ADDMISSINGITEMS` with `ROLLUPISSUBTOTAL` (and
  `ROLLUPGROUP`), the `ties` argument of `TOPNSKIP`, `OPENINGBALANCEWEEK`/`CLOSINGBALANCEWEEK`
  and the month, quarter and year balances over a model calendar (`preview_eval_calendar.rs`),
  wildcards in `SEARCH`/`CONTAINSSTRING`; the visual-calculation forms of the window functions
  are an error outside a visual calculation, as the reference says; without
  `utc_offset_minutes`, `NOW()`/`UTCNOW()` follow the process's local clock (the machine's time
  zone in the desktop app and the CLI). Rules in [formats/dax-functions.md](formats/dax-functions.md)
  "Added on 2026-11-25". **Not oracle-checked**: no engine oracle was available
  (`BICRAFT_ORACLES_DIR` unset); unconfirmed choices: the `TOPNSKIP` ties reading, the order of
  the rows `ADDMISSINGITEMS` adds and its missing total row, a lone `~` in a wildcard pattern.
- **2026-11-25, calendars** (branch `feat/dax-calendars`). Calendar-based time intelligence over
  a model calendar (`preview_eval_calendar.rs`): `TOTALWTD`/`MTD`/`QTD`/`YTD`, `DATESWTD`/`MTD`/
  `QTD`/`YTD`, `STARTOF*`/`ENDOF*` (week to year), `FIRSTDATE`/`LASTDATE`, `PREVIOUS*`/`NEXT*`
  (day to year), `PARALLELPERIOD`, `DATEADD` with `extension` and `truncation`,
  `SAMEPERIODLASTYEAR`, `DATESINPERIOD` with `endbehavior`, `DATESBETWEEN`; tagged columns'
  sort-by columns cleared (balances too), time-related columns kept by `DATEADD` and
  `SAMEPERIODLASTYEAR`; the filter arguments of `ROLLUPISSUBTOTAL` and several
  `ROLLUPISSUBTOTAL` in one `ADDMISSINGITEMS`; calculated tables and columns on the local
  clock, with `model.refresh`'s `utc_offset_minutes`. Rules in
  [formats/dax-functions.md](formats/dax-functions.md) "Calendar-based time intelligence".
  **Not oracle-checked** (`BICRAFT_ORACLES_DIR` unset); unconfirmed choices listed there (the
  page's text and its example disagree on time-related columns of another calendar; the reading
  of the `ROLLUPISSUBTOTAL` level filters; `ENDALIGNED` for a run of several days).
- **2026-11-25, `FORMAT` and nesting** (branch `fix/g30-format-calendar`). `FORMAT` reads a
  letter that is none of the reference's date, time or numeric characters as a literal, so a
  scaling comma before an unquoted suffix divides by 1000 (`"0,K"` of 1234 is `1K`); a nested
  calendar function (`PREVIOUSDAY(PREVIOUSMONTH(Cal))`) is an error naming the nesting, as the
  Power BI page's considerations say it is not supported. Rules in
  [formats/dax-functions.md](formats/dax-functions.md). **Not oracle-checked**
  (`BICRAFT_ORACLES_DIR` unset); the engine's error text for the nesting is not known.
- **Left.** `SAMPLE` without an order key (random, refused); oracle verdicts for the 2026-11-25
  cases (the leftovers, the calendars, the unquoted `FORMAT` suffix and the nesting error),
  which need Microsoft's engine.
  `USER*`/`CUSTOMDATA`/`RAND*` stay refused by design.
- **Effort.** M.
- **Depends on.** Nothing.

### G31. DAX evaluation speed on large models — fixed for the bench, next steps open

- **Status.** On 2026-10-24 (branch `feat/storage-engine`): a storage engine under the DAX
  evaluator (`docs/storage-engine.md`), checked on the engine bench (`docs/benchmarks.md`):
  22/22 queries identical to Desktop 2.159 from 10 k to 100 M rows, none refused, in each
  regime.
- **What.** The evaluator scanned the whole fact for each cell (cells × rows), refused work over
  20 billion row visits or 2 million groupings, decoded every column to one code per row, and
  refused backups over 1 GiB with a misleading message ("file access is disabled").
- **Left.** The grouped scans at 100 M (cold Q04, Q05) are the bare engine's slowest. Very high-cardinality
  groupings (one row per customer, Q17) build one row per group in the formula engine (time and
  memory). Result serialization costs about 4 ms per 1,000 rows. Data beyond RAM (decompressed
  backup held whole). BIcraft's own data snapshot (`model.refresh`) is still in row form.
  Next steps, in order (`ROADMAP.md`, "Storage engine: next steps"): automatic aggregations
  derived from the report (to become G32), halving BIcraft's times (profiling first), beyond RAM.
- **2026-11-19, column-store techniques** (`docs/storage-engine.md`, "Column-store techniques"):
  reused radix buffers, segment skipping for any id range with zone maps, predicate order with
  an early exit, and a parallel top-k for `TOPN` landed (measured at 10 M rows: ×1.2 to ×2.3
  on the cases they target, Q17 warm ×1.25); a parallel merge by group ranges measured no gain
  and was dropped. Not yet measured at 100 M or on Linux; columns read on demand have no zone
  map.
- **Effort.** M (what is left).
- **Depends on.** Nothing.

### G32. Report-derived aggregations — done for the bench, some cases left to the fact

- **Status.** On 2026-10-24 (branch `feat/report-aggregations`, `docs/storage-engine.md`
  "Aggregations"): the report's visuals, filters (all levels), slicers and measures (with their
  dependency closure) give each visual's grain; a size rule (facts of 100,000 rows or more; each
  aggregation at most a tenth of the fact, all of them at most a quarter, at most 8) and a greedy
  choice by rows saved pick a small hierarchy of aggregations; every grouped scan of the storage
  engine is answered by the smallest one that covers it exactly (bitmaps and grouping code tables
  constant on its classes, a derivable aggregation), else by the fact. They are built in memory
  only (never written), by a background job that starts after the answer is sent and works while
  the session is idle, and dropped with any change of data, model or report. Tests: derivation,
  exactness on the bench's shapes, a random differential test against the fact (4,000 queries,
  cold and warm), one invalidation test per change (data, measure, column, relationship, report
  visual and filter), the background job (first query from the fact, later ones from the
  aggregations, a stale job dropped), and the oracles with aggregations forced
  (`BICRAFT_AGG_SYNC=1 BICRAFT_AGG_MIN_ROWS=0`: interaction oracle 438/438, 171/171, 180/180).
- **Measured** (`docs/benchmarks.md`, "With and without report-derived aggregations"): the cold
  median is about 2 times as fast with them at 1 M, 4 times at 10 M and 9 times at 50 M and
  100 M; start and warm unchanged (within run-to-run noise); aggregations 14–18 % of the fact's
  rows. A real project (local only, small
  fact, aggregations forced): every visual's data identical with and without.
- **Left.** Distinct counts under a filter or grouping on another table (Q07, Q18) stay on the
  fact (their grain is close to the fact's size). The background job stops at 1 GiB of
  aggregations per fact while `dax.aggregations {build}` (the bench) builds them all (1.3 GB at
  100 M). Aggregations are rebuilt by each new session (not stored: the snapshot format has no
  versioned section). `model.refresh` does not build them itself: the first query over the new
  data starts the job. Real-number sums, `MEDIAN`, `STDEV`, `VAR` are never answered from them.
  Without aggregations, the grouped scans at 100 M (Q04 cold) stay the bare engine's slowest.
- **Effort.** S–M (what is left).
- **Depends on.** G31.

### G33. Key influencers visual — placeholder and opt-in estimate (2026-11-04)

- **What.** Power BI computes `keyDriversVisual` with its own models, which are not public and
  are not reproduced. BIcraft draws a placeholder laid out like Desktop's visual by default. On
  request (`key_influencers: "estimate"`) it draws its own badged estimate: Welch t-tests, slopes,
  equal-count bins and a regression tree ([key-influencers.md](key-influencers.md)).
- **What is left.** Measures in Explain by. Expand by for a categorical target. Counts and
  sorting by count. The drill into a selected influencer. The toggle button in the desktop and
  web apps (the snapshot and `view.interact` carry it). A pixel-level comparison of the layout with
  Desktop (the layout copies one capture: formats/key-influencers-bb.md).
- **Effort.** S–M.
### G34. Decomposition trees — first lot done

- **Status.** On 2026-11-04 (branch `feat/decomposition-tree`): the snapshot evaluates a
  `decompositionTreeVisual` level by level from its saved expansion state (pinned levels, the
  selected path, "High value" / "Low value" levels chosen again for the current path, relative
  AI mode, sorting, blanks, filters and slicers; `data.decomp`, docs/view-snapshot.md), the
  renderer draws it with Desktop 2.159's geometry and options (docs/render.md, "Decomposition
  tree"), and the hit map selects (cross-filter), moves the path, expands (popup) and removes
  levels through `view.interact` (`decomp_path`, `decomp_expand`, `decomp_collapse`; reader
  state only). Checked against the decomposition tree oracle (23 trees recorded in Desktop,
  `cargo test -p xtask decomp`).
- **Second lot.** On 2026-11-26 (branch `feat/g34-decomp-tree`, docs/render.md "Scrolling and
  Collapse"): the egui UI's "+" opens an "Expand by" popup ("High value", "Low value", the fields
  left) that sends `decomp_expand`; the "+", the header crosses and the scroll arrows are
  labelled buttons over the page, reached with Tab (the popup's first item focused, Escape
  closes it). The scroll arrows are clickable: `view.interact` `decomp_scroll {level, start}`
  moves a level's window by one window, `decomp_scroll {shift}` scrolls a tree wider than the
  visual by one level (arrows on the cut sides; BIcraft's own steps, Desktop's not recorded);
  `tree.defaultClickAction: toggle` collapses the tree under an intermediate path member, as the
  public theme schema describes it.
- **Left.** The live viewer's `page.html` still shows a labelled box: not planned, page.html is
  being retired for the egui UI (docs/webui.md), which draws the tree. Not observed in Desktop,
  so not reproduced: the window of a long level with no selected member follows an empirical
  rule (nine cases), Desktop's scroll steps, negative bars, `axisStart`/`axisEnd`, conditional
  bar colours and tooltip fields; a highlighted tree's blank first-level member shares the
  root's highlight key.
- **Effort.** S (what is left).

### G36. Browser document tabs as parallel views — done

- **Status.** On 2026-11-05 (branch `feat/webui-parallel-views`): the browser's document tabs no
  longer take turns in the workspace's one session. Each workspace file's tab is a **document
  view** on the server (`POST …/view/api/session {"file"}`, [mcp.md](mcp.md), "Document views"):
  a private session of its own on that file, with its own snapshot, event stream, interactions
  and MCP endpoint (`/w/<id>/mcp/view/<view>`), so tabs are edited side by side as on the
  desktop. Saves follow the editor rules (`expected_revision` per view) plus a per-file check
  (`file_conflict`: a save over a revision the session never read writes nothing), the
  workspace's own session included. Caps per user (8) and per workspace (32), the memory check,
  idle expiry (30 min, four times as long with unsaved edits) and readers' read-only views; a
  refusal closes the tab with the server's message in the tab strip. Existing MCP clients of
  `/w/<id>/mcp` are unchanged.
- **Deployment.** On 2026-11-05 (branch `chore/deploy-followups`): the `bicraft` chart's
  `viewer.documentViews.perUser` / `.perWorkspace` / `.idleSeconds` and the controller's
  `workspaces.pod.maxViewsPerUser` / `maxViews` / `viewIdleSeconds` set the three flags (0 keeps
  the server's defaults); `/metrics` has `bicraft_document_views_open{mode}`,
  `bicraft_document_views_max` and, with the workspace label,
  `bicraft_workspace_document_views_open` ([kubernetes.md](kubernetes.md), "Observability").
- **Page close.** On 2026-11-26 (branch `fix/g36-tab-close`): a page that goes (its browser tab
  or window closed, a reload, another page) closes the views its tabs hold on `pagehide`, as
  `keepalive` requests (`POST …/api/close?v=` with each view's token), so the server drops them
  at once instead of after `--view-idle`; their unsaved edits stay in the recovery journal
  (`--recovery-dir`, G40) and are offered when the file is opened again. The idle expiry stays
  the fallback: a page kept in the back-forward cache keeps its views (it may come back), and a
  release the browser never sends (a crash, a lost network) waits for it.
- **Left.** An upload over a file a view edits is refused (409) rather than merged.
- **Effort.** S (what is left).

### G40. Auto-recovery and autosave — done

- **Status.** On 2026-11-05 (branch `feat/auto-recovery`, [recovery.md](recovery.md)): every
  edit since the last save is journaled (one JSON line per command, its GUIDs recorded so a replay
  is exact; a full recovery copy for commands with outside inputs, an undo past the save and past
  the 4 MiB cap) in the per-user recovery folder (desktop) or the server's `--recovery-dir`.
  Opening the file again offers the edits (the prompt, File > Home, `recovery.list`),
  `recovery.apply` replays them through the normal commands and stops at the first failure; a
  file changed since is recoverable from a recovery copy only. An expired browser view, a dropped
  session and a server restart lose nothing. Autosave (opt-in) saves PBIP projects every N minutes
  or after N seconds without an edit, through `file.save` and its checks; PBIX and PBIT are not
  offered (repacking cost).
- **Never-saved models, view settings, every tab.** On 2026-11-26 (branch `feat/g40-recovery`,
  [recovery.md](recovery.md)): a model never saved (`model.new`, an import read into a new model)
  is journaled under a key of its own, its first edit as a recovery copy; `recovery.list {all}`
  and the start screen list it (`base: "untitled"`, the model's name), and `recovery.apply {id}`
  recovers it in any session without unsaved edits (File > Home: "New model, never saved",
  Recover in the open tab). `project.new` and `file.save_as` end its journal. The view settings
  (visual text language, captions' culture, as-of date, the reader's page) are journaled (header
  and `view` lines) and put back after the edits (`view: {applied, errors}`). Autosave runs for
  every open document tab, each on its own clock.
- **Left.** On the desktop, recovering a never-saved model needs an open document (it replaces
  that tab's content, as New model does; the window cannot start an empty tab); the window's zoom
  and panes are window preferences, not per-file view state, so they are not journaled; a
  recovery copy keeps the model and the report, not the session's `data_root` or cache file; the
  desktop's listing skips journals locked by a running instance but cannot say which instance
  holds them.
- **Deployment.** On 2026-11-05 (branch `chore/deploy-followups`): the `bicraft` chart sets
  `--recovery-dir /var/lib/bicraft/recovery` on a claim of its own (`recovery.*`, on by default,
  1 GiB, kept on uninstall, refused inside the workspace root) and `recovery.allowAutosave`; the
  controller gives each workspace a claim `bws-<id>-recovery` with the data claim's reclaim
  policy (`workspaces.recovery.*`, `status.recoveryClaim`), so an idle scale to zero keeps
  unsaved edits; `/metrics` has `bicraft_recovery_journals` and `bicraft_recovery_journal_bytes`
  ([recovery.md](recovery.md), "In Kubernetes").
- **Effort.** S (what is left).

### G38. Dependencies and repository hygiene left after M11.5 (2026-11-05)

- **Status.** M11.5 landed `SECURITY.md`, the PR template, Dependabot, `deny.toml` with its CI
  job and `cargo xtask deny`, `cargo xtask assets` in the gate and SHA-pinned actions
  ([ci.md](ci.md) "Assets and dependencies").
- **Left.**
  - Done 2026-11-05 (`deps/zip-9`): `zip` 9.4 (feature `deflate-flate2` only; zopfli and
    `arbitrary` leave the tree) replaces 2.4, so RUSTSEC-2026-0367, -0368, -0369 and -0429 no
    longer apply; `crates/opc/src/extract.rs` still extracts entry by entry and now also refuses
    what `zip` reports as a symlink. The renderer reads fonts with `skrifa` and shapes with
    `harfrust` (the versions epaint uses) instead of `ttf-parser` and `rustybuzz`, which leave
    the tree (RUSTSEC-2026-0206, -0192); the UI's font tests read cmaps with skrifa. `deny.toml`
    ignores no advisory. Checked: the 34 PBIX/PBIT fixtures of `BICRAFT_TESTDATA` give the same
    bytes before and after when opened and written (as they are and through `write_model`); the
    `cargo xtask dist` zip is identical (zip 9 needs `System::Unix` for its Unix modes);
    `crates/opc/tests/golden_bytes.rs` pins the writer's bytes (CRC and length taken under zip 2)
    and reads them back from memory, from a file on several threads and deferred; the render
    goldens and 4 967 text measures over bundled and system faces are bit-identical.
  - Done 2026-11-26 (`chore/g38-hygiene`): `colored` (MPL-2.0) leaves the tree with `dify`,
    `open` and the rest of `egui_kittest`'s `snapshot` feature, which the desktop's `gpu-tests`
    no longer enable: the screenshot tests only call `Harness::render` (the `wgpu` feature) and
    save PNGs with tiny-skia. `deny.toml` drops its MPL-2.0 exception; no crate needs it.
  - Done 2026-11-26: GitHub issue forms under `.github/ISSUE_TEMPLATE/` (bug report, feature
    request; blank issues off), with security reports sent to the private advisory of
    [SECURITY.md](../SECURITY.md) and a no-client-data checkbox.
  - Workspace crates are not `publish = false`, so cargo-deny counts their versionless path
    dependencies as wildcards; `wildcards = "allow"` until the maintainers decide on publishing.
  - Done 2026-11-26 (`fix/g38-queued-gauge`): `bicraft_tool_calls_queued` counts a tool call
    only while it waits at its workspace's session gate (`crates/mcp/src/http/rwgate.rs`), raised
    and lowered under the gate's mutex after a write has announced itself; a call that gets the
    lock at once is not counted ([kubernetes.md](kubernetes.md) metrics table). The HTTP test
    `a_write_waits_for_reads_and_later_reads_see_it` runs once, without retried rounds.
- **Effort.** S each.

### G39. UI control channel (M11.6) — done, on `bicraft-cli serve` too

- **Status.** On 2026-11-05 (branch `feat/m11-control`): agents drive the running app and take
  screenshots through an opt-in channel, the same methods on the desktop
  (`bicraft --control PORT`: JSON lines on loopback, a random token per launch in an
  owner-only file) and in the browser (`bicraft-cli view --control`: the server relays requests to
  the app page over an event stream). Requests run between frames through the code a click runs
  (`crates/ui/src/control.rs`); screenshots are egui's screenshot command. `bicraft-cli ui` is the
  client ([desktop.md](desktop.md), [webui.md](webui.md)).
- **Serve relay.** On 2026-11-05 (branch `feat/control-serve`): `bicraft-cli serve --viewer-control`
  relays the same methods to each workspace's app pages (`crates/mcp/src/http/viewer.rs` on the
  shared `bicraft_view::control::Relay`), behind the server's authentication: an editor
  credential plus the control token (`X-Bicraft-Control`), browsers refused; a request reaches
  only the caller's own pages, by default the last connected, by page view, or by document view
  (its file's tab first); `GET …/api/control` lists the caller's views; the server's caps apply.
  `bicraft-cli ui --serve | --url` is the client ([web-deployment.md](web-deployment.md), "UI
  control channel"). The Windows token file now has an explicit owner-only access list
  (`icacls`, before the token is written: the workspace forbids `unsafe`, so no Win32 call).
  Headless Firefox drove both `bicraft-cli view --control` and `bicraft-cli serve --viewer-control`
  through `bicraft-cli ui` (`apps/bicraft-cli/tests/browser_control.rs`, ignored by default:
  [webui.md](webui.md)), screenshots included.
- **Headless render.** On 2026-11-26 (branch `feat/g39-headless-render`):
  `bicraft FILE --render OUT.png [--size WxH] [--scale N] [--set JSON]` draws the whole
  window with no window, GPU or display. The app runs offscreen (`crates/ui/src/offscreen.rs`)
  and is driven by the channel's own `ui.wait`, `ui.set` and `ui.screenshot`; the screenshot is
  egui's triangles filled on the CPU, the same bytes on every run (tested,
  `apps/bicraft-desktop/src/tests/headless_tests.rs`, no `gpu-tests`; [desktop.md](desktop.md),
  "Headless render"). Not compared pixel by pixel with a wgpu screenshot.
- **Left.** No synthetic pointer or key
  input on purpose: requests name commands, tabs, pages and visuals; a click on a slicer item or
  a data point goes through `command view.interact`. The headless render is the desktop
  binary's (`bicraft-cli ui` has no `render` of its own; the browser app has none). The browser test needs Firefox and an embedded bundle, so it is not in CI.
  An agent addresses a document view by the id `server.views` lists; the window's own
  `ui.inspect` does not show view ids.
- **Deployment.** On 2026-11-05 (branch `chore/deploy-followups`): the chart's
  `viewer.control.*` and the controller's `workspaces.pod.viewerControl` pass `--viewer-control`
  with the token from a Secret (`BICRAFT_VIEWER_CONTROL_TOKEN`; the chart refuses it without the
  Secret or the viewer); the gateway forwards `X-Bicraft-Control` and the per-view routes as they
  are (tested); `/metrics` counts the relay's requests and errors
  (`bicraft_control_requests_total`, `bicraft_control_request_errors_total`). Not run in a cluster
  in this lot (`deploy controller-smoke` does not drive the channel).
- **Effort.** Done.

### G42. Report metadata as fields (last refresh, INFO functions) — done, not measured in Desktop

- **Status.** On 2026-11-05 (branch `feat/refresh-metadata-fields`): the DAX evaluator runs
  `INFO.TABLES`, `INFO.PARTITIONS`, `INFO.MODEL`, `INFO.COLUMNS`, `INFO.MEASURES`,
  `INFO.RELATIONSHIPS` and the four `INFO.VIEW.*` with the documented columns, the refresh times
  of the stored data (`RefreshedTime`, `ModifiedTime`, `StructureModifiedTime`; blank without
  data) and restrictions; the validator and the evaluator refuse the query-only ones outside DAX
  queries (`info-query-only`), as Microsoft's reference says Power BI does. The optional
  `table.add_refresh_info` (Model › Calculations › Refresh info table) adds a standard import
  table refreshed with the data (`DateTimeZone.UtcNow()`, `DateTime.LocalNow()`) and a
  `Last refresh` measure, the Desktop-compatible way to show the last refresh on a card or a
  table, since `INFO.PARTITIONS` is not allowed in a measure
  ([formats/dax-functions.md](formats/dax-functions.md), "INFO functions"; [mcp.md](mcp.md),
  "Refresh info table").
- **Local clock.** On 2026-11-05 (branch `chore/deploy-followups`): `DateTime.LocalNow()` and
  `DateTimeZone.LocalNow()` (the Refresh info table's local column) read the machine's time zone
  on the desktop and in the CLI, at each evaluation, as Desktop does; `bicraft-cli serve` keeps UTC, as
  the service, unless `--local-offset` or `TZ` says otherwise
  ([formats/data-sources.md](formats/data-sources.md), "Clock").
- **Left.** Where the INFO functions run and their columns are **documented, not measured**: the
  probes and the checklist for Desktop are in `tools/info-oracle/`. The `INFO.VIEW.*` text values
  (`DataType` names, `StorageMode`, the `Relationship` shape) and IDs differ from the engine's by
  design or are unverified; the other 62 `INFO.*` functions are validated but not evaluated; a
  report-level measure (a visual's extension measure) is not checked against the query-only rule
  by the evaluator; the Refresh info table's round trip was not opened in Desktop.
- **Effort.** S (the measurements).
### G43. Report text glyphs — mostly fixed

- **Status.** On 2026-11-05 (branch `fix/render-glyph-fallback`): a measure named `CA Δ N-1` drew
  as `CA   N-1` on the app's pages (bundled fonts only) and in `bicraft-cli render` without a system
  font that has Greek: Selawik is Latin only and the character was skipped. The renderer's chain
  now ends with OFL glyph subsets of Noto Sans (Regular, Bold), Noto Sans Mono (box drawing and
  block elements), Noto Sans Symbols 2, Noto Sans Math, Noto Sans Symbols and Noto Emoji
  (`crates/render/fonts/SOURCE.txt`, [render.md](render.md) "Bundled fallback faces"); the UI's
  chrome uses the same faces. A character no face has is drawn as a box and listed once per
  render (`missing_glyphs` and a warning of `report.render`). Tests: every character of the
  fixtures and a list of common report characters have a glyph
  (`crates/render/tests/glyph_coverage.rs`).
- **Left.** **CJK** (Chinese, Japanese, Korean) is not bundled: a usable face is 15–20 MB, more
  than the whole browser bundle. Such text uses the system's fonts (Windows has them) or a face
  given with `--font-dir` / `BICRAFT_FONT_DIRS` (Noto Sans CJK, OFL) and otherwise draws as boxes,
  listed; the app's pages (bundled fonts only, desktop and browser) always draw boxes. An
  optional CJK download for the app is open. Emoji are monochrome (Desktop shows colour emoji),
  and only a list of common ones is bundled (`EMOJI` in `xtask/src/font_subset.rs`);
  the symbol faces have one weight (bold text gets regular arrows and shapes); the scripts of
  South and South-East Asia, Arabic and Hebrew are not bundled either (system fonts only). The
  rare characters of the bundled blocks left out are listed at the end of `SOURCE.txt`.
- **Effort.** M (an optional CJK face for the app).

### G47. Editing interactions by hand (M11.15) — done

- **Status.** On 2026-11-06 (branch `feat/ui-edit-interactions`): before, `page.interactions` was
  reachable only through the generic parameters form. Both shells now have Desktop's Edit
  interactions mode (`crates/ui/src/interactions.rs`, docs/design.md "Edit interactions"): the
  Visual tab's button is a toggle (Escape leaves; readers cannot enter); the selected visual is the
  source and every other visual with fields shows Filter, Highlight and None at its top-right
  corner, only the modes the pair can take (from the snapshot's `interactivity`, the engine's
  table of the types that highlight), the effective mode marked, the default lighter with a "Default: …"
  tooltip; a click runs `page.interactions` with one rule (the default removes it), one undo
  step. Data points stay clickable (live preview); leaving clears the selection. New command
  `report.settings` (`default_interaction`, `drill_filters_other_visuals` in `report.json`
  `settings`), on the Visual tab and the palette; the viewer follows the cross-filter default,
  and an explicit `HighlightFilter` now highlights under it.
- **Groups and keyboard (2026-11-26, branch `feat/g47-interactions`).** A visual group is a source
  and a target of `page.interactions`: the rule stands for every pair between the visuals inside
  it (nested groups too), each written as its own `visualInteractions` entry, a later rule of the
  same call winning for a pair; a group and one of its own visuals have no pair (refused), as has
  an empty group. No Desktop file with a group in `visualInteractions` has been seen, so BIcraft
  never writes a group's name there (unconfirmed whether Desktop would). In Edit interactions a
  selected group is the source: its visuals take no icons, the others show the modes any of them
  can take, nothing is marked when they differ (tooltip "the group's visuals differ here"), and a
  click writes the rule for all of them (the shared default removes them). The icons are reached
  with Tab and run with Enter or Space; targets with the same title are named "Title (name)".
- **Left.** Desktop draws its icons over the visual header; ours sit inside the visual's top-right
  corner and can cover a title there. A group is not a target in the mode itself (its visuals
  are, one by one; `page.interactions` takes it). Since M11.19 (G51) the reader's drill follows `drill_filters_other_visuals`, and the
  per-visual "Apply drill down filters to" (`drillFilterOtherVisuals`) is `visual.drill_filter`. The
  report setting has no state in the interaction oracle corpus (its behaviour follows Microsoft
  Learn's description; a recording would check it).
- **Effort.** S.

### G45. Editing DAX in a formula bar (M11.14) — done

- **Status.** On 2026-11-05 (branch `feat/ui-formula-bar`): before, measures were edited only
  through the generic parameters form (`measure.set` on the Measure tab, `measure.add` on Home);
  the Data pane had no Edit. Both shells now have Desktop's formula bar under the ribbon
  (`crates/ui/src/formula.rs`, `dax_text.rs`): the selected measure, calculated column or
  calculated table as `Name = <DAX>` with the DAX colours, one line or expanded; Enter commits
  (`measure.set` with the rename in one step, `column.set`, `partition.set` then `table.rename`,
  `measure.add` for New measure), Esc cancels, Ctrl+Z restores; autocomplete over the engine's
  function list (new read command `dax.functions`), the model's names and the keywords; the
  signature of the call being typed; the live check (`dax.validate`, which now takes `kind`) with
  the error underlined and its message under the bar; Edit in the Data pane's menu and a double
  click in the diagram; readers read-only.
- **Second lot (2026-11-25, branch `feat/data-view-formula-bar`).** Invalid DAX is committed as
  Desktop commits it: the bar sends `force`, the object keeps its new expression and the error
  stays under the bar ("Saved with an error: …"). A click away from the bar, or another object
  picked while editing, commits the edit (a click on the bar's own buttons does not). **Format
  DAX** (the bar's button) runs `dax.format {style: "formula"}`: BIcraft's own layout following
  the DAX formatting conventions SQLBI describes publicly (functions and keywords in upper case,
  `SUM ( T[C] )`, one argument per line when a call does not fit, `RETURN`'s expression one level
  in); the text changes on the bar, not committed. The autocomplete shows the selected function's
  signature and BIcraft's one-sentence description of it in English or French (472 functions,
  `crates/dax/src/catalog/descriptions.rs`, written for BIcraft in its own words, not taken from
  Microsoft's reference; `dax.functions` `description`, `description_fr`). Tests:
  `the_formula_style_follows_the_public_conventions`,
  `every_function_has_a_description_in_both_languages`, `the_live_check_underlines_the_error`,
  `a_click_elsewhere_commits_the_edit`, `format_dax_lays_the_formula_out`,
  `the_completion_describes_the_selected_function`.
- **Third lot (2026-11-25, branch `feat/g45-formula-find`).** A calculated table's new
  expression re-derives its columns (`partition.set`, as Desktop does): the expression is
  evaluated over the session's data (`dax.evaluate`, one row kept); a column of the same source
  column (a field parameter's `[Value1]`) or name keeps its properties (lineage tag,
  description, format; its type and source column follow the expression), new ones are added as inferred calculated-table columns, data columns the
  expression no longer gives are removed with their relationships, sort-by settings and
  hierarchy levels (`columns: {added, removed, retyped}`); calculated columns stay; when the
  expression cannot be evaluated the columns are kept, with a note. `partition.set` takes
  `new_name` (and `update_report`, `update_dax`): the bar's expression and rename of a
  calculated table are one command, one undo step. The bar marks the bracket next to the cursor
  and its partner (background and line), opens find and replace with Ctrl+H or its search
  button (matches marked, Match case, Enter / Shift+Enter, Replace, Replace all; view state, the
  commit stays the bar's command), and underlines the error with a wavy line in the theme's
  error colour. Tests: `a_calculated_tables_new_expression_gives_its_columns`,
  `a_new_expression_and_name_are_one_undo_step`, `a_calculated_column_and_a_calculated_table`,
  `find_and_replace_in_the_bar`, `the_bracket_pair_at_the_cursor_is_marked`,
  `the_error_underline_is_wavy`, `the_bracket_at_the_cursor_finds_its_partner`,
  `find_ignores_ascii_case_unless_asked`.
- **Left.** The columns' order and the names of duplicate result columns after a re-derivation
  are BIcraft's (kept columns stay in place, new ones after the data columns; not compared with
  what Desktop writes), and visuals bound to a removed column are left as they are; Desktop's
  exact error wording under the bar is not reproduced (BIcraft shows its own diagnostic); the
  DAX query view and the dialogs' DAX boxes keep a straight underline; find does not scroll a
  long expression to the match; no folding or format-on-commit; Format DAX
  is BIcraft's formula style, not a copy of any other formatter's output, and has no options in
  the UI (width 100, four spaces); the descriptions of a few rarely used or undocumented functions
  (INFO.*, NATURALJOINUSAGE, SAMPLE* functions) stay generic; the autocomplete has no fuzzy
  matching beyond "contains"; the Measure contextual tab shows only in the Model view (the bar's gear opens the
  measure's properties form in the Report view); a 10,000-line expression takes about 56 ms a frame while typing in the
  optimized test build (`a_large_expression_stays_responsive`, accessibility tree included), not
  yet the 16 ms of a smooth frame.
- **Effort.** S–M.

### G41. Editing a visual's fields by hand (M11.12, M11.13) — done

- **Status.** On 2026-11-05 (branch `feat/ui-fields-panes`): the Report view's Data and Build
  panes in both shells (`crates/ui/src/panes.rs`): the field list (search, hidden toggle,
  display folders, hierarchies and levels), the visual's type picker and wells, drag and drop of
  fields to wells and onto visuals, reorder and move between wells, aggregation of a column,
  remove by × or by dragging out, the field's box; every edit an engine command (new:
  `visual.move_field`, `visual.set_aggregation`, `visual.set_type`), readers read-only.
- **Second lot (M11.13, 2026-11-05, `feat/ui-filters-pane`).** The Filters pane (filters on this
  visual, this page, all pages; basic with the engine's values, advanced, top N; clear, remove;
  a field dropped on a section adds an empty card), new commands `page.filter`,
  `page.clear_filters`, `report.filter`, `report.clear_filters`, `column.values`; the snapshot
  lists every level's cards. "Rename for this visual" (`visual.rename_field`, the projection's
  `displayName`, a double click on a well's field). A whole hierarchy dragged to a well or a
  visual (`visual.bind_hierarchy`, one `HierarchyLevel` projection per level, rendered as its
  columns). A field's box with nothing selected, or a drop on an empty part of the page, makes a
  new visual of Desktop's type.
- **Third lot (M11.13, 2026-11-18).** Relative date and relative time filtering on date columns
  (last / this / next N days to calendar years, minutes, hours, include today; PBIR
  `RelativeDate` / `RelativeTime` cards, `relative` param of the three `*.filter` commands);
  lock and hide icons on each card (`isLockedInViewMode` / `isHiddenInViewMode`, one command
  each), readers leave hidden cards out and see the lock. `page.filter` and `report.filter`
  refuse Top N (Desktop offers it on visual filters only). The relative shapes beyond "last N
  including today" are inferred, not yet checked against files Desktop writes.
- **Filters pane formatting (2026-11-19).** `page.format_filter_pane {page, card: pane |
  available | applied, …}` writes the page's `objects.outspacePane` and `objects.filterCard`
  (selector id `Available` / `Applied`) from the public page and theme schemas; the theme's
  `outspacePane` and `filterCard` are the defaults (snapshot `filter_pane_format`); the Format
  tab with nothing selected shows Desktop's Filter pane and Filter cards cards
  (`crates/ui/src/filter_pane_ui.rs`); the Filters pane draws the background, border, font and
  icon colour, title, header, search and text sizes, input boxes, the Apply colour and both card
  states, for an author and a reader. Left: the pane's `width` is written and read back but the
  app keeps its own pane width; the font family maps to the app's four Selawik weights (no other
  face is drawn); the `selector.id` of a page `filterCard` entry is the theme's `$id`, inferred,
  not compared with a file Desktop saved; the HTML viewer has no Filters pane.
- **Filters pane card order (2026-11-19).** `filter.order` writes a level's `filterSortOrder`
  (A to Z, Z to A, custom) and, in custom order, every card's `ordinal` (public
  `filterConfiguration` schema); the snapshot lists the cards in that order (`filter_sort`). In
  the Filters pane an author drags a card's header within its section (a line shows where it
  goes) and each section has a sort list; a reader can do neither. Left: an absent
  `filterSortOrder` is read as A to Z from Microsoft Learn, not compared with a file Desktop
  saved; a visual's bound fields without a card are sorted in by name (in custom order they come
  last) and cannot be dragged until they have a card; cards are not dragged between sections.
- **Reader filters (2026-11-26, `feat/g41-reader-filters`).** A reader (the `reader` role on a
  server, the browser viewer) changes a card the author neither locked nor hid, as in Power BI's
  reading view: `view.interact` `filter {page?, visual?, field, condition}` and `filter_clear`
  (`crates/engine/src/interact_reader.rs`, `filter_edit::reader_change`) keep the changed
  `filterConfig` as the reader's view state (the override an applied bookmark sets), so the
  visuals and the Filters pane show it while the report, the revision, the dirty flags and the
  saved files do not change; `reset` drops it, each server viewer has their own. Refused: a
  locked or hidden card, a field without a card (a visual's bound field counts as one), the
  view-mode flags, and a change of the filter type unless the report's "Allow users to change
  filter types" is on. In the Filters pane the reader's clicks, Apply filter and eraser send
  these actions; a locked card's controls are greyed with the reason; a reader has no ×. Checks:
  engine `interact::filters_tests` (4), kittest `reader_filters_ui_tests` (1), seen red first.
  Left: the reader's filters are not kept between sessions (Power BI's persistent filters).
- **Long names (2026-11-26, `fix/card-units-assets`).** A Filters pane card's field name and
  condition, and a Build pane well's field, are cut with an ellipsis where they do not fit (a
  long name was clipped mid-letter, "Revenue Targe", beside a selected visual's card icons) and
  shown whole on hover; the accessibility label keeps the whole name. Check: kittest
  `a_long_field_name_is_elided_on_its_card_and_shown_whole_on_hover`, seen red first.
- **Left.** Well captions are BIcraft's words for the common types, the role name for
  the others; the Filters pane has no "Require single selection" (relative date and time filters and the lock
  and hide icons landed on 2026-11-18, above; Top N stays visual-only, as in Desktop); a filter BIcraft does not read is shown as
  kept, not edited; a whole hierarchy dropped on an empty part of the page makes no visual; the
  empty card's `type` (`Categorical` / `Advanced`) is inferred, not checked against a card Desktop
  adds itself; the new visual's size (400 × 300) is BIcraft's.
- **Effort.** M (done).

### G46. Desktop features missing for parity — reviewed 2026-11-05, rescored 2026-11-18

- **Status.** Open. The gap review of [target-app-parity.md](target-app-parity.md) ("Gap review
  2026-11-05") rescored [parity-checklist.md](parity-checklist.md) on `main` at ccb96e3 and
  widened it to all of Desktop: authoring 63 of 129 rows done (49%, weighted 68%; on the
  2026-11-04 rows 57 of 91, 63%, weighted 78%), viewer done share `bicraft-cli render` 59%, web viewer
  46%, app 52% (weighted 74%, 61%, 65%). Every command has a button since M11, about half of them
  only through the generic parameters form.
- **Rescored 2026-11-18** on `main` at 816a4cb, after the lots M11.16–M11.25 (measured by `cargo
  xtask parity`, each row a judgement): authoring 79 of 129 rows done (61%, weighted 78%),
  `bicraft-cli render` 22 of 33 (67%, weighted 80%), web viewer 26 of 56 (46%, weighted 61%), app 38
  of 56 (68%, weighted 81%). Lots 1–10, 13 and 15 of the review landed (lot 7 without print, lot
  9 without the Analytics pane); lot 17 (the Data view, See records, Show as a table, M11.19).
- **Focus mode, Show as a table and Sort axis (M11.19)** are reader view state (`view.interact`
  focus / show_table, `interaction.focus`); Sort axis sends `visual.set_sort`, and for a reader
  `view.interact` `sort` (the reader's view only, dropped by `reset`, carried by bookmarks as
  `singleVisual.orderBy`). Spotlight (2026-11-19) is view state too (`view.interact` `spotlight`,
  the renderer dims the other visuals; Escape or a click outside ends it; bookmarks capture and
  apply `singleVisual.display.mode: "spotlight"`). Left: the header shows for the selected visual
  only (not on hover); no horizontal / vertical table layout switch; the table shows the snapshot
  rows only (no scroll, rows not clickable); single-field sort with no Remove sort and no
  Shift+click multi-column sort; the dim amount (15 % opacity) and the bookmark shapes
  (`singleVisual` without `visualType`) not compared with Power BI; focus mode (`maximize`) not
  captured in bookmarks; `bookmark.apply` (the editing one) ignores `orderBy` and spotlight; none
  of it in the HTML viewer (`crates/view`).
- **What is left**, as the review's order of lots (the SQL Server connector, lot 11, landed: G68; the Model explorer tree and layout tabs, lot 18, landed: G61; the DAX window functions, balances and `GROUPBY` landed with lot 20; groups and bins landed: G75; quick measures landed: G76): show value as, more point selections, keyboard
  access to visuals, visual calculations, storage mode, high contrast, more UI languages, and the
  rows found since: G56 (error bars and the other analytics), G57 (show items with no data), G58
  (Get data and Navigator), G59 (Enter data grid), G60 (Optimize ribbon), G61 (Model explorer),
  G62 (Analyze insights), G63 (report settings). Hard ones with open feasibility questions:
  DirectQuery, maps (tiles and geocoding), custom visuals (a JavaScript host), Q&A and the other
  AI visuals.
- **Not assessed.** Depth within done rows, usability, the full connector catalog and Desktop's
  Options dialog item by item; no Desktop run.
- **Where.** `crates/ui`, `crates/engine`, `crates/render`, `crates/data`.
- **Effort.** XL: the top 20 lots were ~370–660 Opus 5.5 agent hours (estimated); the open ones
  ~170–330 h by the review's per-lot figures, lot 17's rest counted 5–10 h (estimated), the rest ~400–800 h.
- **Depends on.** Per lot, in the review's table.

### G51. Reader drill, drillthrough and report-page tooltips (M11.19) — done, documented not measured

- **Status.** Done on 2026-11-06 (branch `feat/viewer-drill`): items 5 and 8 of the gap review
  ([target-app-parity.md](target-app-parity.md)).
  - **Engine** (`crates/engine/src/interact_drill.rs`, `view.interact`): `drill_down` on a point,
    `drill_up`, `drill_next` (go to the next level), `drill_expand` (expand all down one level),
    `drill_mode` (a click then drills); a matrix's `matrix_expand` / `matrix_collapse` on a member,
    an entire level or all; `drillthrough` (the target's drillthrough filters from the point, "Keep
    all filters" carrying the source visual's page filters, own filters and slicer selections, the
    Back history) and Drillthrough buttons on the page's selection; `tooltip` (the visual's
    tooltip page, set or "Auto", under the hovered point and the visual's filters). The state is
    the reader's, applied with Desktop's saved fields (`active` flags, `howCreated: "Drill"`
    filters, `expansionStates`, filled drillthrough filters), so the evaluator reads it as a saved
    file; a visual's `drillFilterOtherVisuals` ("Apply drill down filters to", `visual.drill_filter`), else the report's "Drilling filters other visuals" (`report.settings`), makes a drill filter the page; `view.keep_drill` writes the
    state into the report as Desktop saves it. A visual's stale query (a field bound after the
    query was written) is read rebuilt for its drill level.
  - **Renderer**: an axis expanded down a level groups by every level with concatenated labels;
    a matrix's +/- icons (`rowHeaders.showExpandCollapseButtons`) and row-header menu regions.
  - **App** (both shells, `crates/ui/src/drill.rs`): the header icons on hover, the right-click
    entries (Drill down, Drill up, Show next level, Expand to next level, Expand / Collapse >
    Selection, Entire level, All, Drill through > page, Keep drill state), report-page tooltips
    drawn small under the pointer.
- **Checked.** Engine tests (`crates/engine/tests/report/interact_drill_tests.rs`): drilled rows
  equal `dax.evaluate` of the same filter context, drill filters on other visuals only when
  turned on, `view.keep_drill` round trip, matrix expansion states, drillthrough with and without
  "Keep all filters", the Back button, a Drillthrough button, cross-report refused, tooltip pages
  under the point and "Auto"; render tests (`tables_tests.rs`) and the golden `drill.png`; kittest
  (`crates/ui/src/tests/drill_ui_tests.rs`). Desktop's behaviour comes from Microsoft Learn
  (drill mode, drillthrough, report tooltips, read 2026-11-06): **not measured** in Desktop.
- **What is left.**
  - Record drill, drillthrough and tooltip states with the interaction oracle
    ([interaction-oracle.md](interaction-oracle.md): `drive.mjs` steps for the header icons and
    the context menu) and compare, as G29 did for clicks.
  - page.html (the web viewer) has none of it.
  - Cross-report drillthrough (a service feature) is refused; summarized drillthrough fields
    (`asAggregation`) are not filled; the header's ⇓ on a selected point with drill mode off
    (Desktop expands only that point) expands every point; a matrix drills its Rows from the
    canvas (Columns through `role: "Columns"` only); the drill icons are drawn shapes, not
    Desktop's icon font; `drillFilterOtherVisuals` is read on the drilled visual, as Desktop's
    "Apply drill down filters to" sets it (the schema's text names the target).
  - Desktop saves the drill state with the file; BIcraft only on `view.keep_drill`.
- **Effort.** S–M for what is left.
- **Depends on.** The interaction oracle's driver, for the measurement.

### G48. Canvas editing (M11.16) — done

- **Status.** On 2026-11-06 (branch `feat/ui-canvas-editing`): before, a click selected one
  visual and positions changed only through forms (`visual.set_position`, `visual.place`,
  `page.arrange`). Both shells now edit the canvas as Desktop does (`crates/ui/src/canvas.rs`):
  click, Ctrl+click, a rubber band and Ctrl+A select one or several visuals; a drag moves them,
  eight handles resize them, edges and centres snap to the other visuals and the page with smart
  guides (and to an 8 px grid with View › Snap to grid), arrows nudge 1 px (Shift 10 px), Delete
  removes the selection; the Visual tab's Arrange group aligns, distributes, orders, groups and
  ungroups; Ctrl+C, Ctrl+V, Ctrl+D and the menus copy, paste and duplicate across pages and open
  documents through the OS or browser clipboard with an in-app fallback. New engine commands
  `visual.copy` (read-only), `visual.paste`, `visual.duplicate`, `visual.move`, `visual.align`,
  `visual.group`, `visual.ungroup`; `visual.remove {visuals}` and `visual.order {visuals, to}`
  take a set. Each gesture is one command, so one undo step.
- **Since 2026-11-19.** A click on a grouped visual selects the outermost group, a click inside
  the selected group its next level down to the member (Ctrl+click toggles that level); Shift
  while resizing keeps the aspect ratio of the box taken; the canvas menu's Paste puts the
  clip's top-left corner at the right-clicked point (Ctrl+V and Home › Paste keep the copied
  place); `visual.cut` (Ctrl+X, the Visual menu) copies and removes in one undo step;
  `visual.remove` also removes a group whose members are all removed. The Selection pane landed
  on 2026-11-17 (M11.23, `crates/ui/src/selection_pane.rs`).
- **Since 2026-11-20.** View › Page options as Microsoft Learn describes them ("Apply page size
  and settings in a Power BI report", "Use gridlines and snap-to-grid"): Gridlines (the 8 px grid
  shown on the author's canvas), Snap to grid (moves and resizes) and Lock objects (no drag,
  resize or arrow nudge; clicks and rubber bands still select) are independent window
  preferences, since the published report schema has no property for them. View › Page view
  (Fit to page, Fit to width, Actual size) runs `page.set_display_option`, the page's
  `displayOption`; the status bar's zoom (fit buttons, Zoom out, a 10–400 % slider, Zoom in, the
  percentage) is a view setting of the window over it. The Learn pages describe no page
  thumbnails in Desktop, whose pages are tabs below the canvas (the app's page tabs).
- **Since 2026-11-27** (branch `feat/g48-canvas`, the parts that need no Desktop). View › Smart
  guides (on by default, a window preference like Gridlines) turns off snapping to the other
  visuals' and the page's edges and centres and the dashed guides; Snap to grid and Alt work as
  before. Gridlines is drawn by the renderer over the page background and image and under the
  visuals (`RenderOptions::grid`, the page image's key), no longer painted over the page image;
  the phone canvas keeps its drag-time grid overlay. `visual.copy` puts the report images its
  visuals show in the clip (`images: [{name, data}]`, base64, at most 4 MB, those left out named
  in `warnings`) and `visual.paste` registers them in the target (checked as `image.add` checks a
  file; the same name and bytes reused, another image of that name gets a file of its own), so
  an image visual pasted into another file keeps its picture. `visual.remove` refits a group to
  the members it keeps.
- **Left** (each needs a Desktop observation). Lock objects is not compared with Desktop: Learn
  says the objects cannot be moved or resized, so BIcraft still deletes, cuts and pastes.
  Bookmarks and sync groups naming the source visuals are not copied (which bookmark or sync
  group a pasted copy should join is not documented, so it is left open). Snapping uses
  BIcraft's own threshold (6 screen points) and grid (8 px), not measured in Desktop; whether
  Desktop names its guides switch Smart guides and what it turns off is not observed either.
- **Where.** `crates/ui/src/canvas.rs`, `crates/engine/src/canvas_edit.rs`.
- **Effort.** S.

### G50. PBIX refresh from local sources (M11.18) — done, opened in Desktop up to 10 M rows a table

- **Status.** On 2026-11-06 (branch `feat/pbix-refresh`): before, `model.refresh` refreshed
  PBIP projects only (into `.pbi/bicraft-data.bin`) and a PBIX kept its stored rows until
  Desktop refreshed it. Now `model.refresh` on an open PBIX
  (`crates/engine/src/refresh_pbix.rs`) runs its import partitions' M in BIcraft's evaluator
  (the same connector and `data_root` rules), evaluates its calculated tables and columns, and
  writes the new rows into its `DataModel` with the structural writer
  (`DataModelEditor::write_refreshed`): partitions Ready with the refresh time,
  `LASTCUBEDATACHANGE` raised. `tables` refreshes those and what reads them (calculated tables
  again, calculated columns of other tables over their stored rows); the other tables keep their
  storage files byte for byte. The file is written to a temporary sibling, reopened and checked
  (structure, row counts), then renamed; errors leave it as it was. `force` (unsaved edits),
  `backup`, read-only roles, the server's per-file check (`file_conflict`). Home › Refresh, F5
  and a table's "Refresh data" (Data pane, model diagram) in both shells, the answer window with
  a spinner, a summary line and the per-table grid. On the way: a replaced table with calculated
  columns and a new row count failed to save (the evaluator read the stored row count).
- **Measured** ([benchmarks.md](benchmarks.md), *PBIX refresh*): 10 M fact rows refreshed in
  10.2 s (×1.2 faster than building the PBIX, ×4.6 faster than the same data refreshed as a PBIP
  project), one dimension in 0.97 s. <!-- perf-guard: ok (BIcraft against BIcraft) -->
- **Seen in Desktop (2.160.1151.0, 2026-11-25).** Refreshed PBIX files whose tables hold up to
  1,048,576 rows open with the new data (rows, a measure over a calculated column through
  `RELATED`, a new key on the one side, a column chart and a table equal to `bicraft-cli dax-query`),
  `--table` and an unchanged refresh too, and Desktop's own Refresh then succeeds and saves.
  A table of more rows was then refused on open by Desktop's consistency check (fixed below).
- **Desktop open (2.160.1151.0, 2026-11-25, branch `fix/g50-multisegment`).** Desktop refused
  any BIcraft table over 1,048,576 rows, refreshed or new, with its consistency check (DBCC, on
  the data segments). Found by comparing BIcraft's 10 M-row PBIX with
  Desktop's refresh of it and swapping one value: a partition of several 2^20-row segments needs
  `SegmentMapStorage.Type` 1, BIcraft wrote 3 (the one-segment value). The writer now takes the
  type from the segment count (test
  `a_partition_of_several_segments_has_segment_map_type_1`); Desktop then opened PBIX files of
  1,048,577, 2,097,153 and 10,000,000 rows and a `bicraft-cli refresh` from 1,000 to 1,200,000 rows,
  with a card and a table of count and sum measures equal to the expected values
  ([formats/vertipaq.md](formats/vertipaq.md) §11, "Segment map type of a multi-segment
  partition").
- **Left.** The maintainers' checklist in
  [formats/datamodel-fresh.md](formats/datamodel-fresh.md) (*Refresh of an existing PBIX*: RLS
  through View as, Excel sources) is not fully run; a
  PBIX whose data the Analysis Services engine wrote was not refreshed in the bench; the rows of
  a refreshed table are held in memory while they are encoded (2.5 GB at 10 M rows, bounded by
  `BICRAFT_MEMORY_LIMIT` through refusal, not streamed), at most 16,777,216 rows a table;
  incremental-refresh policy partitions, DirectQuery and database connectors other than
  Databricks are not refreshed; `sources.rebase` still refuses a PBIX with stored rows (it could
  now offer a refresh); no per-table progress while it runs (a spinner).
- **Effort.** M (done); streaming the encoder M.
### G53. Export to PDF and of a visual's data (M11.21) — done, PDF/UA and PDF/A identified; validator run left

- **Status.** Done 2026-11-06 on `feat/export-pdf-data`. `report.export_pdf` writes report
  pages to one PDF (all pages but hidden ones, the current page, a range, pages by name), one PDF
  page per report page at its size; shapes and charts as vector paths, text as real text with
  embedded TrueType subsets and a ToUnicode map (selectable, searchable), images and canvases
  too large to record as images ([render.md](render.md), "PDF"). Our own PDF writer and font
  subsetter (no new crate); a structural reader (`bicraft_render::pdf_read`) checks the files.
  `visual.export_data` writes a visual's summarized or underlying data to CSV (30,000 rows) or
  XLSX (150,000 rows, a minimal SpreadsheetML workbook through `bicraft-opc`), under its filters
  and the reader's slicers; `report.settings {export_data}` sets and the snapshot shows the report's
  `exportDataMode`, which the export honours. The app: File › Export › Export to PDF with the
  page choice, the Export data setting, the selected visual's More options (⋯) and right-click
  Export data dialog; the browser downloads the file the export wrote in the workspace.
  On 2026-11-19 the PDF got an outline (a bookmark per page that goes to it) and a tagged
  structure (`Document`, a `Sect` per page and per visual titled as it, a `P` per line of text,
  a `Figure` with the visual's alt text, `/Artifact` for the rest, `/Lang`, the parent tree;
  the snapshot now carries a visual's `alt_text`), and `report.print` with File › Export ›
  Print (the page choice of Export to PDF): the PDF written to the temporary folder and opened
  with the OS's PDF application on the desktop, printed from a hidden frame with the browser's
  print in the browser ([render.md](render.md), "PDF", "Print").
  On 2026-11-27 (`feat/g53-pdf`), the parts that need no Desktop: `report.export_pdf` takes
  `pdf_ua` (default on: an XMP metadata stream with `dc:title` and `pdfuaid:part` 1) and `pdf_a`
  (default off: `pdfaid` 2B and a `GTS_PDFA1` output intent with an sRGB ICC profile BIcraft
  builds from IEC 61966-2-1's parameters, `crates/render/src/icc.rs`); the trailer has an `ID`.
  Table visuals are `Table` / `TR` / `TH` (`Scope Column`) / `TD` structures, empty cells kept;
  a chart without alt text is a `Figure` labelled with its title, its labels still paragraphs;
  the visuals' sections are in tab order, then top to bottom and left to right; every glyph has
  a `ToUnicode` entry (U+FFFD for a ligature). CFF faces are embedded whole (`FontFile3`
  `OpenType`, `CIDFontType0`, up to 16 MB). The reader checks what it can see
  (`PdfInfo::ua_issues`, `pdfa_issues`). Export to PDF in the app has an Archival PDF
  (PDF/A-2b) box. `visual.export_data {kind: "current_layout"}` (XLSX only; the Export data
  dialog's first choice, EN and FR) writes a matrix with column fields as a cross table (header
  rows per column field, labels merged over their values) and other visuals as the summarized
  table, within 150,000 cells of values, as Microsoft Learn's "Export data from
  visualizations" describes the option. Tests (each seen red with its change undone): render
  `a_table_visual_is_tagged_as_a_table_of_header_and_data_cells`,
  `a_chart_without_alt_text_is_a_figure_labelled_with_its_title_and_keeps_its_text`,
  `the_reading_order_is_the_tab_order_then_top_to_bottom_and_left_to_right`,
  `the_default_file_is_identified_as_pdf_ua_and_passes_our_checks`,
  `pdf_a_2b_has_metadata_an_srgb_output_intent_and_an_id`,
  `a_cff_face_is_embedded_whole_as_opentype` (a CFF face built in the test), `icc::tests`;
  engine `current_layout_writes_a_matrix_as_a_cross_table`,
  `pdf_ua_by_default_and_pdf_a_on_request`; kittest `export_pdf_*` (the archival box).
- **What is left.**
  - A run of a PDF/UA and PDF/A validator (PAC, veraPDF: none installed here) on the files;
    until then the identification is BIcraft's claim, checked by its own reader only. A
    matrix is not tagged as a table (its spanning headers would need `ColSpan`); CFF faces are
    embedded whole, not subset. Print has no dialog of its own (page setup, printer): the OS's
    PDF application or the browser prints, and the browser's print from a hidden frame is not
    checked in every browser (the file is downloaded when it fails to load).
  - Faces whose `fsType` forbids embedding are drawn as outlines with invisible Helvetica text
    over them (searchable, not selectable as the face).
  - Underlying data groups the aggregated columns (a row per distinct combination of the columns
    shown) instead of Desktop's flattened row-level export, which adds the other non-hidden
    columns of the tables of the visual's measures and related tables (Microsoft Learn
    describes it; not implemented: it needs the rows of those tables through the evaluator);
    the "Applied filters" header row of Desktop's Excel files is not written; values are raw
    (numbers, ISO dates in CSV) rather than with their format strings; the current layout of a
    table is the summarized table, and its matrix has no subtotals.
  - Export from the web viewer (`page.html`) and Desktop's per-visual "Export data" disable
    switch are not there; a black-box comparison with a PDF and a CSV written by Desktop is to
    do.
- **Where.** `crates/render/src/{canvas,pdf,pdf_read,ttf_subset}.rs`,
  `crates/engine/src/{export_pdf,export_data}.rs`, `crates/ui/src/{backstage,export_dialog,menus}.rs`,
  `apps/bicraft-web/src/web.rs`.

### G54. Format visual pane in the sidebar (M11.22) — done, Desktop's shapes of the newer fx unchecked

- **Status.** Done 2026-11-09 on `feat/format-pane` (gap review item 9). Before, a visual was
  formatted from the contextual Visual ribbon tab (Title, Format visual, Axes, Series format,
  Conditional formatting, Tooltip page, Image buttons), each opening the generic parameters
  form, and the title's text was a JSON code box (the form drew `title`, declared JSON, as a code
  area with a `{ }` hint; the title only showed inside the visual picker's dropdown). Now the
  Build pane has Desktop's Build visual and Format visual sub-tabs (`crates/ui/src/format_pane.rs`):
  Visual and General cards, a search box, header switches, Reset to default (one command per
  card), the values read with `visual.props`; the ribbon keeps one Format button that opens the
  tab, the other formatting commands moved to the palette. `visual.set_title`'s form is a free
  text field holding the visual's title plus one field per property (the command takes them
  flat); title and subtitle take `font_family` and `underline` (subtitle `bold` and `italic`).
- **What is left.**
  - On 2026-11-18 the **fx buttons** landed (`crates/ui/src/conditional_dialog.rs`): beside a
    chart's Data colours › Colour (`data_color`), a table's or matrix's Values › Background and
    Text colour (`background`, `font_color`, with the series chosen) and a card's Callout value ›
    Colour (`font_color`), marked when `visual.props` lists a `conditional` there. The dialog
    offers Gradient (lowest, optional middle, highest colours), Rules (=, >, >=, <, <=, between,
    a colour each) and Field value, based on a measure or a summarized column; OK is one
    `visual.conditional`, Remove sends `rule: null`; it reads the visual's format back. Tests:
    `the_fx_*` in `format_ui_tests.rs` (seen red without the button, the send and the marker);
    captures `format-*-fx`, `format-*-fx-rules`, `format-*-fx-on`.
  - On 2026-11-19 **fx on more properties**: the General cards' Title and Subtitle text colour,
    Effects › Background and Visual border colour (Gradient, Rules, Field value), the Title and
    Subtitle text (Field value only: a title from a text measure or column, Microsoft Learn
    "Expression-based titles"), a chart's Data labels › Colour and a card's Category label ›
    Colour. `visual.conditional` names them `title_color`, `subtitle_color`,
    `visual_background`, `border_color`, `title_text`, `subtitle_text` (and `title_background`,
    with no row in the pane), `label_color`, `category_color`. The container's targets are one
    value for the whole visual, written in `visualContainerObjects.<object>` in the entry
    without selector, beside its plain properties (no series, no `apply_to`; an
    `{object, property}` target naming a container object the type has no visual object of that
    name goes there too); the label colours take the data colour's wildcard selector. The viewer
    draws them: the producer evaluates the whole-visual rules once in the visual's filter context
    without its grouping (`data.visual_styles`; a gradient whose stops have no value gives its
    lowest colour, since there is one value) and the label colours per point (`cell_styles`
    `label`, `cat`); column, bar and line charts' data labels and the card's category label use
    them. Tests: engine `container_targets_write_visual_container_objects`,
    `data_label_and_category_label_colours`, `container_and_label_conditional_formats_are_evaluated`,
    the M2.5 PBIR-in-PBIX and legacy PBIX save round trips; kittests
    `the_general_cards_fx_buttons_format_the_container`, `the_title_text_fx_takes_a_field`,
    `the_label_colours_have_their_fx`. Captures `format-fx-*`.
  - On 2026-11-20 the **dialog was completed** against Microsoft Learn ("Apply conditional
    table formatting in Power BI"): a table's and matrix's **Cell elements** card (Data bars,
    Icons, Web URL, each an fx button) opens the data bars dialog (Show bar only, minimum and
    maximum as the lowest / highest value or a number, positive and negative bar colours, axis
    colour, bar direction), the icons dialog (Rules: an icon list per rule with every icon of
    the sets drawn as the tables draw them; Icon layout left / right of data or icon only; Icon
    alignment; Style, an icon set filling three rules) and the web URL dialog (the field). The
    colour dialogs gained the gradient's Minimum / Center / Maximum as the lowest / middle /
    highest value or a number (a diverging scale with the middle colour), "How should we format
    empty values?" (don't format, as zero, a colour), Apply to (values only, values and totals,
    totals only) on tables and matrices, rules with a second condition ("If value >= 0 and < 50"),
    "is blank", Reverse colour (icon) order. A **preview** under the controls draws five sample
    values with the format, as the viewer evaluates it (RGB interpolation, first rule met, bars
    from the axis); its accessible name lists the samples and their colours. The viewer now
    draws data bars' axis line, right to left and Show bar only (`cell_styles` `bar.axis`,
    `axis_color`, `only`; before, they were written and read back but not drawn). Tests: engine
    `data_bars_axis_direction_and_bar_only_are_evaluated`,
    `dialog_conditional_formats_round_trip_through_pbip_and_pbix` (table and matrix, PBIP, PBIR
    PBIX, legacy PBIX); render `data_bars_draw_their_axis_and_show_bar_only`; the dialog's
    unit tests (rules, preview colours, read back); kittests
    `the_cell_elements_open_the_data_bars_icons_and_web_url_dialogs`,
    `the_data_bars_dialog_applies_its_settings`,
    `the_icons_dialog_applies_a_style_layout_and_alignment`, `the_web_url_dialog_applies_a_field`,
    `the_gradient_dialog_takes_numbers_blanks_and_totals`,
    `the_rules_take_a_second_condition_and_reverse_their_colours` (seen red with the card, the
    stops' numbers, Apply to and Reverse taken out); captures `condfmt-*`.
  - On 2026-11-25 **fx on axes, legend, gauges and KPIs, and Percent rules**
    (`feat/fx-bookmark-drill`): the X- and Y-axis Colour and Title colour, the Legend colour,
    a gauge's Target colour, Target label and Callout value colours, a KPI's Indicator colour,
    Goal and Distance colours and Colour coding's Good, Neutral and Bad colours have their fx
    button. `visual.conditional` names them `category_axis_color`, `category_title_color`,
    `value_axis_color`, `value_title_color`, `legend_color`, `target_color`,
    `target_label_color`, `callout_color`, `indicator_color`, `goal_color`, `distance_color`,
    `good_color`, `neutral_color`, `bad_color` (or their `{object, property}`); each is one value
    for the visual, written in the object's entry without selector ([pbir.md](formats/pbir.md)
    "Conditional formats of the container and of labels"), and refused on a type without that
    property. Rules take **Percent** bounds (`percent: true`, 0 to 100; the dialog's "Rule n
    type" Number / Percent), written as `Min + (Max - Min) * p/100` of the rule's field
    (BIcraft's shape from the semantic query schema). The producer evaluates them
    (`data.visual_styles`; a percent of the visual's lowest to highest value) and the renderer
    draws the axis label and title colours, the legend colour, the gauge's target, target
    label and callout and the KPI's indicator, goal, distance, good and bad colours. Tests:
    engine `whole_visual_targets_write_their_object_without_selector`,
    `rules_take_percent_bounds`, `whole_visual_colours_and_percent_rules_are_evaluated`; render
    `whole_visual_conditional_colours_are_drawn`; kittests
    `the_axes_and_legend_colours_have_their_fx`, `the_gauge_and_kpi_colours_have_their_fx`,
    `the_rules_take_percent_bounds`.
  - On 2026-11-27 **gradient Percent stops and the KPI's neutral colour** (`feat/g54-format`):
    a gradient's Minimum, Center and Maximum take a Percent (`{color, value, percent: true}`,
    0 to 100; the dialog's stop type Lowest / Number / Percent), the stop's `value` written as
    the Percent rules' `Min + (Max - Min) * p/100` expression over the gradient's input (an
    inference: no public shape) and read back only in that shape; the producer places the stop
    at that percent of the visual's lowest to highest value, the dialog's preview too. A KPI
    whose indicator equals its goal is drawn (value, trend area, status mark) in Colour coding's
    Neutral colour (`status.neutralColor`, or its fx) when one is set, else in the Good colour
    as before. Tests: engine `gradient_stops_take_percents`, `gradient_percent_stops_are_evaluated`,
    `preview::format::tests::gradients_and_rules`, the `dialog_conditional_formats_round_trip`
    PBIP / PBIR PBIX / legacy PBIX round trip; render
    `the_kpi_at_its_goal_is_drawn_in_the_neutral_colour`; ui
    `the_gradient_takes_percent_stops` (each seen red with its change undone).
  - Left: these whole-visual and percent shapes (rules' bounds, gradient stops) are BIcraft's
    reading of the schemas, **not compared with a file Desktop formatted** (black-box oracle to
    run); the KPI's neutral colour when none is set (Desktop's default not observed: BIcraft
    keeps Good) and Colour coding's `direction` (Low is good; BIcraft reads every KPI as High is
    good), neither compared with Desktop; the gradient's Percentile stops (no public shape); the legacy JavaScript viewer (`page.html`)
    does not draw the new targets; the new card's and button slicer's elements (Microsoft
    Learn's list; the engine takes them as `{object, property}`), icons by Field value (a custom icon from a URL), icon rules
    with an "else", the swatch (and the title's text box) still shown under an active format,
    title background drawn by the viewer (no title background is drawn at all yet), the
    pie, donut, treemap and area charts' per-point data label colours. The PBIR shapes of the
    container's and labels' formats follow the published schemas and Microsoft Learn; they were
    not compared with a file Desktop formats (black-box oracle to run).
  - On 2026-11-18 the Data colours card gained Desktop's **colour per series or value** under
    its own rows: one row per value field when a chart has several (`Y`, `Y2`), per legend value
    (`Series`), per category value for pies, donuts (`Category`) and treemaps (`Group`), and per
    category value when Show all (`dataPoint.showAllDataPoints`) is on. The catalog gives Show
    all to the line, area, funnel, scatter and map types only, so column and bar charts offer no
    per-category colours (Desktop 2.159 was not opened to compare). Values come from
    `column.values` (the first 100, a note when cut); each swatch writes `visual.series {target,
    color}` (`{field, value}` for a value), one undo step; the colour read back is the snapshot's
    `format.series_colors`; a row without its own colour shows the page's palette for its place
    (`bicraft_render::data_colors`, the same the renderer draws with, which also feeds the colour
    popups). Reset to default clears the card's own properties, not these per-series colours
    (the palette's Series format clears them). Tests: `the_data_colours_card_colours_each_value`
    (seen red without the rows), `the_data_colours_rows_follow_the_fields`; captures
    `format-*-colours` in `format_pane_screenshots`.
  - On 2026-11-17 the Visual group's cards were **generated from the visual catalog**
    (`bicraft_visuals`): each card is an object (`ObjSpec`) and its rows are the properties the
    catalog gives that object **for the selected type**, so the set of cards follows the type
    exactly (a pie no axis, a table no legend, a gauge a Gauge axis and a Target, a card a
    Callout value); a card's title is the catalog's object name, normalised and translated; each
    row writes one object property through `visual.format {objects}` (colour properties as
    `{color}`, numeric enumerations as numbers); a test
    (`the_visual_cards_follow_each_type`) checks every card's switch and rows name a property the
    catalog gives that type. `visual.axis` left the tab's commands (the cards write
    `visual.format`).
  - On 2026-11-17 the cards also **read their values back**: `visual.props` puts every plain
    object (no selector, no state) of the visual into `format.objects` as
    `{object, property, value}`, the value in the `visual.format` escape hatch's shapes (bool,
    number, text, `{color}`, `{literal}`, `{expr}`, `{solid}`); the values `format.objects`
    cannot write back (arrays such as a paragraph list) are left out. The pane finds a card's
    value there, so a switch, a list, a numeric list, a style button, a swatch and Reset to
    default all read what they wrote; a kittest
    (`the_generated_cards_controls_commit_their_object`) clicks each control kind of a column
    chart's generated cards and checks both the `visual.format` sent and the value read back.
  - Visual cards exist for every type of the Visualizations pane's gallery except the script
    visuals and RDL (and the non-data types: buttons, shapes, text boxes), which have General
    only and point to Format visual's form, where any catalog property can be written by name;
    a test (`every_gallery_type_has_visual_cards`) keeps the gallery and the cards in step.
    `image` keeps its own static card. The cards no longer carry every property of their object:
    the fx buttons and the per-series and per-category colours are still left (see above).
  - Not in the cards: General › Properties' size and position (the canvas does it), the
    divider and spacing of the title area, the header icons one by one, the tooltip's own text
    formatting, the analytics pane (`visual.analytics`, palette); the theme's colour grid has no
    tints and shades row; defaults shown when neither the visual nor the theme sets a value are
    BIcraft's reading of Desktop's, not measured.
  - Not compared with a file Desktop formats the same way (black-box oracle to run).
- **Where.** `crates/ui/src/{format_pane,panes,ribbon,control}.rs`,
  `crates/engine/src/visual_props.rs`, `crates/ui/src/tests/format_ui_tests.rs`.

### G49. Data view, query preview and See records (M11.17) — done

- **Status.** On 2026-11-06 (branch `feat/ui-data-view`): before, a table's rows were seen only
  through `dax.evaluate` in the answer window, a query's result only by loading it into a table
  (`table.import {m}`), and a data point's rows not at all. Both shells now have Desktop's Data
  view (`crates/ui/src/data_view.rs`): the Data pane's tables and queries, a virtualized grid
  paged from the new read `table.rows` (sort and filter as view state, from each header's menu;
  type tags; calculated columns computed by the DAX evaluator when the data does not hold them;
  cell selection, Ctrl+C as tab-separated text; the row count in the status bar), the Column tools
  on the Table tab (name, data type, format, summarization, data category, sort by column through
  `column.set`; New column and New measure on the formula bar, `column.add` taking the
  expression's type when `data_type` is left out), a query preview (`query.preview`, BIcraft's M
  evaluator, connectors only when allowed, refused to readers and without the server's network)
  and See records from a visual's right-click menu (`visual.records`, the point's values through
  the relationships). The three reads run shared and are skipped by recovery.
- **Filters (2026-11-18).** See records applies the report, page and visual filters and the
  slicers (`crates/engine/src/preview_records.rs`, as DAX `FILTER(ALL(..))` and
  `CALCULATETABLE(TOPN(..))` arguments); a condition that cannot filter rows (a measure, several
  tables) is listed in `not_applied`.
- **Measure filters, several tables, cross-highlight, text and number filters (2026-11-25,
  branch `feat/data-view-formula-bar`).** A filter on a measure or an aggregation keeps the
  visual's points it keeps (`CALCULATETABLE(FILTER(VALUES(<grouping column>) | SUMMARIZE(<records'
  table>, <grouping columns>), <condition>), <the column filters>)`, the measure computed under
  the other filters); a condition over several tables filters the records' rows with the other
  tables' columns through `RELATED` (when they are on the one side of active relationships); a
  visual cross-highlighted by another visual's selection shows the rows of its highlighted part
  (`cross_highlight` names the source; BIcraft's choice, not compared with Desktop yet). The Data
  view's header menu has Text, Number and Date filters (Equals, Begins with, Ends with, Contains,
  Greater than, Between…, Is blank, Custom filter…) and the Custom filter window (two tests, And
  or Or), sent as `table.rows` conditions (new ops `ends_with`, `not_ends_with`, and `any` for
  Or). Tests: `see_records_applies_the_filters_and_the_slicers`,
  `see_records_applies_a_condition_over_several_tables`,
  `a_measure_filter_on_a_card_is_named_not_applied`, `see_records_follows_a_cross_highlight`,
  `table_rows_reads_a_literal_table_with_a_calculated_column`,
  `a_headers_menu_has_text_and_number_filters`.
- **Cards, unreachable grouping columns, the records' table, sorting (2026-11-27, branch
  `feat/g49-filters`).** A measure filter on a card (no grouping column the records' table
  reaches) is computed once under the other filters, as the visual evaluator filters its one
  row: when it fails the card is blank and the records are empty (`FILTER(ALL(<a column>),
  FALSE())` in `query`), else every row is kept. Grouping columns the records' table cannot reach
  through `RELATED` (a table with no relationship, a table on the many side) no longer make the
  query fail (`SUMMARIZE` refused them): they do not filter the records' rows, and a point of the
  reached columns is kept when one of its combinations with their values meets the condition
  (`NOT(ISEMPTY(FILTER(VALUES(<their columns>), <condition>)))`, the measure computed per
  combination). The records' table is now the table the first measure's expression reads (the
  one `RELATED` reaches the others from when it reads several, its home table on a tie, the
  measures it calls followed when it reads no column), so a measure kept in another table than
  its columns' shows its columns' rows; when the measure has a detail rows expression, the
  records are that expression's rows under the point and the filters (`detail_rows: true`). A
  preview's and the records' header menus sort the rows read (view state, no request, Clear
  sort back to the order read). Tests:
  `a_measure_filter_on_a_card_keeps_or_drops_its_records`,
  `a_measure_filter_applies_to_columns_related_cannot_reach`,
  `see_records_reads_the_table_of_the_measures_columns`,
  `see_records_shows_a_measures_detail_rows` (engine),
  `see_records_rows_sort_from_a_headers_menu` (UI). Unobserved in Desktop, BIcraft's choices:
  the tie rule of the records' table when a measure reads several tables, what Desktop's See
  records shows for a card whose measure filter fails (BIcraft: no rows), whether Desktop puts a
  detail rows expression's rows under the visual's filters as BIcraft does, and whether Desktop's
  records window sorts.
- **Left.** A measure condition that also reads a column the visual does not group by, and a
  condition over several tables `RELATED` cannot reach (many-to-many, a filter through a
  bidirectional relationship), are still named in `not_applied`; grouping columns `RELATED`
  cannot reach are taken not to filter the records' table, which is wrong when they do through a
  bidirectional or many-to-many relationship; on a visual grouped only by such columns the check
  asks whether any of their values meets the condition, not the clicked point's. The records'
  table follows the first measure, not the measure of the clicked mark (the hit map's point
  carries grouping columns only). The preview and the records are read once (up to 10,000 rows):
  a sort orders the rows read, not the whole answer; the grid has no column statistics (Desktop
  shows none in the Data view either, but Power Query's profile is G46's row); the filter menu lists the
  first 1,000 distinct values (a search narrows them); its Text, Number and Date filters are
  sections of the menu, not Desktop's submenus, and their test names are BIcraft's own, not
  checked against Desktop's menu; sorting or filtering on a calculated column the data does not hold
  computes it over at most 2,000,000 rows; Format offers a fixed list of format strings (the
  Column tools have no custom format box or decimal places spinner); New table and Mark as date
  table stay on the Table and Model tabs' forms (Manage relationships has its dialog since
  2026-11-20, G61).
- **Effort.** M (done).

### G52. Rich content: text boxes, images, buttons, page format (M11.20) — done

- **Status.** On 2026-11-06 (branch `feat/ui-rich-content`): before, a text box's text, an
  image, a button's action and a page's background could only be written through `visual.format
  {objects}` JSON or kept from Desktop's files; no image could be stored. Now: seven commands
  (`textbox.add`, `textbox.set`, `image.add`, `image.set`, `button.add`, `visual.set_action`,
  `page.format`, all journaled as lines) write what Desktop writes
  ([formats/pbir.md](formats/pbir.md), "Text boxes, images, buttons and page format"), with an
  **Insert** ribbon tab, the text box editor in place on the canvas with its toolbar, the Buttons,
  Image, Action and Page format windows, and Ctrl+click on buttons in editing (a reader's plain
  click). Image bytes are checked by content, capped at 16 MB and decoded before they are stored;
  SVG references outside the bytes are no longer followed by the renderer (it used to read a file
  an SVG named, `svg_references_outside_the_bytes_are_not_read`). The viewer acts on web URL
  buttons; text boxes draw bulleted and numbered paragraphs.
- **Left.**
  - Lists are written as `listType` on the paragraph, a key BIcraft chose: no Desktop file with a
    list has been read (the public theme schema's `paragraphs` has no list key). Desktop is
    expected to ignore it and show plain paragraphs; a black-box check with the layout oracle is
    the next step.
  - Bookmark actions are applied since 2026-11-18 (`view.interact` `button` → `Bookmark`,
    `crates/engine/src/interact.rs`, G55); Q&A buttons are refused ("Q&A is not available in
    BIcraft"; Q&A is out of scope, and no small step short of a Q&A engine is open).
  - Button states drawn 2026-11-20 ([formats/buttons.md](formats/buttons.md),
    `crates/render/src/states.rs`, tests `buttons_tests.rs`, `buttons_ui_tests.rs`): the app tells
    the renderer where the pointer is and draws the page again over a button, shape or navigator
    button that has an On hover or On press look; a drillthrough button is disabled without one
    selected point (its disabled look, else dimmed to 45 %, BIcraft's choice), takes no click and
    shows its enabled / disabled tooltip; conditional (fx) destinations and tooltips
    (`visual.set_action {destination_field, tooltip_field}`) are evaluated by the snapshot and
    followed by the readers; the Format pane's "Apply settings to: State" writes the state picked.
    Fill images, custom icons, glow and shadow per state and the button's tile shape drawn
    2026-11-27 ([formats/buttons.md](formats/buttons.md) "Shape, glow and shadow of an action
    button"; snapshot `button.shape`, `button.show_glow`…, `fill_image`, `icon_image` per state;
    the Format pane's Shape, Glow and Shadow cards for buttons; render tests
    `action_buttons_draw_their_state_shadow_glow_and_shape`,
    `action_buttons_draw_their_state_fill_image_and_custom_icon`, seen red). Left: the Data
    function action (Fabric user data functions: out of scope); `page.html` (the legacy JS
    viewer, slated for removal) draws no state at all, not the disabled one either; exports have
    no pointer, so they draw the default and disabled states only; a custom icon placed above,
    below or custom, Icon size and the icon margins are not drawn (left or right of the text, at
    the built-in icon's size); a fill image is not cut to a tile shape; no command writes fill
    images or custom icons (kept from files); the fx `expr`, the 45 % dim, the effect defaults
    and the image placement not checked against Desktop.
  - Insert › Shapes landed on 2026-11-19 (`shape.add`, the 23 `tileShape` values of the theme
    schema's `visual-shape`; the Format pane's Shape, Rotation, Fill and Outline cards; the
    renderer draws each tile, `crates/render/src/shapes.rs`). Not checked against Desktop: the
    written records follow the schemas and docs/formats/pbir.md, the drawn geometry (arrow
    proportions, the adjustment percentages, a turned shape scaled down to stay in its box) is
    BIcraft's own; the Format pane offers Rounded corners for every tile (the other adjustment
    properties of the theme schema are read but have no control); text inside a shape is drawn
    since 2026-11-20 (upright, `rotation.textAngle` not applied); the legacy JS viewer still draws
    every shape as a rectangle.
  - The editor lays the text out with the app's bundled faces (the page uses the run's family),
    one alignment for the whole box only when every paragraph shares it, list markers as an
    indent; Desktop's editor has its own font list and paragraph spacing.
  - Images: BMP is not read; WebP is stored with a warning (Desktop's dialog does not offer it);
    the browser takes images from the workspace's files (no upload from the page itself).
  - Desktop's insertion positions are not reproduced (new items are centred on the page).
- **Effort.** S–M for what is left (lots 8 and 10, on which it waited, landed).

### G55. Bookmarks and Sync slicers panes (M11.24, M11.25) — done, Desktop's drill-in-bookmark shape and button effects unchecked

- **Status.** On 2026-11-17 (`main`, unmerged): Desktop's last two report panes. **Bookmarks**
  (`crates/engine/src/bookmarks.rs`, `crates/ui/src/bookmarks_pane.rs`): the report's bookmarks in
  their saved order with groups foldable; Add / View / Update / Rename / Delete / Group / Ungroup,
  the in-group moves and Desktop's Data / Display / Current page options (written as the PBIR
  `suppressData` / `suppressDisplay` / `suppressActiveSection` flags; Current page off by default).
  A bookmark is captured as Desktop's PBIR `bookmark` document (`explorationState` with the active
  page, the hidden visuals' `display.mode` and the filters as `{"byName": {...}}`), kept in
  `report.bookmarks`, its order and groups in `definition/bookmarks/bookmarks.json`
  (`bookmarksMetadata` 1.0.0, `report.resources`); `bookmark.apply` restores the captured states
  within the options in one undo step. **Sync slicers**
  (`crates/engine/src/slicer_sync.rs`, `crates/ui/src/sync_pane.rs`): the report's slicers page by
  page, a group name, the Fields / Filters options and Synchronize / Unsync, writing each
  slicer's `visual.syncGroup` (`groupName`, `fieldChanges`, `filterChanges`) kept in
  `visual_extra` and written back by the PBIR writer; `slicer.sync_list` lists the groups. Both
  panes are closed until their strip is clicked (`panes.bookmarks`, `panes.sync`).
- **Left.**
  - Reader bookmarks done 2026-11-18 (`crates/engine/src/interact_bookmark.rs`, the renderer's
    `bookmark_nav_body`, `crates/ui/src/tests/navigator_ui_tests.rs`): `view.interact {action:
    "bookmark"}`, Bookmark buttons and the bookmark navigator apply a bookmark to the view only
    (page, per-visual visibility and filter overrides within Data / Display / Current page;
    `applyOnlyToTargetVisuals`; `reset` clears them; the report, its revision and dirty flags never
    change). The bookmark navigator is drawn (its group or every bookmark, the applied one
    highlighted); the page navigator leaves out hidden and tooltip pages unless its options show
    them and follows its orientation; in the editor both act on Ctrl+click, as buttons do.
    `allowDeselectionBookmark` / `deselectionBookmark` / `hideDeselectedBookmark` applied
    2026-11-20 (a click on the selected button launches the deselection bookmark or sends
    `view.interact {action: "bookmark_deselect"}`). Not applied: Desktop's captured drill states
    and cross-highlights, a page navigator's per-page `showPage`.
    The Bookmarks pane's View still writes the report (a reader could go through
    `view.interact`); `page.html` does not draw the bookmark navigator.
    Cross-highlights in bookmarks landed on 2026-11-24 (G29, `interact_highlight.rs`). **Drill
    states** since 2026-11-25 (`crates/engine/src/interact_drill.rs` "Bookmarks",
    `bookmarks.rs`): `bookmark.capture` / `bookmark.update` write a drilled visual's active
    levels (`singleVisual.activeProjections`), a matrix's expansion (`expansionStates`) and
    the drilled values (`filters.byExpr`, `howCreated` 2), and `bookmark.apply` and
    `view.interact {action: "bookmark"}` restore the view's drill, resetting it on a bookmark
    without one ([pbir.md](formats/pbir.md) "Bookmarks"). Test
    `a_bookmark_captures_and_restores_the_drill`. The drill mode since 2026-11-27, as a
    formatting-object update of BIcraft's own (`singleVisual.objects.merge.bicraftDrill`, valid
    against the bookmark schema, which has no property for it; [pbir.md](formats/pbir.md)
    "Bookmarks"), restored on and off by `bookmark.apply` and `view.interact` (test
    `a_bookmark_round_trips_the_drill_mode`, seen red). Left: the shape is read from the public
    schemas, not from a file Desktop saved; the drill mode's place is BIcraft's, not seen in a
    Desktop file.
  - Slicer selections done 2026-11-19 (`crates/engine/src/interact_bookmark.rs`,
    `bookmarks_slicer_tests.rs`, kittest `a_bookmark_captures_and_restores_a_slicer_selection`):
    `bookmark.capture` / `bookmark.update` write each slicer's selection as the session shows it
    (a reader's click included) as `singleVisual.objects` (`merge.general` filter, `merge.data`
    range properties, `remove` of `general.filter` when nothing is selected;
    [pbir.md](formats/pbir.md) "Bookmarks"); `bookmark.apply` writes it back into the slicer and
    drops the reader's own selection of it; `view.interact {action: "bookmark"}` makes it the
    reader's applied selection; both within the Data option. Kept by PBIR, PBIX and the legacy
    layout. Left: the empty-slicer shape
    (`remove`) is inferred from the schema, not seen in a Desktop file; text-slicer searches
    (`selfFilter`) are not captured.
  - Navigator styles done 2026-11-19 (`crates/render/src/bodies.rs`, `snapshot_visual.rs`,
    [render.md](render.md)): per-state `fill`, `outline`, `text`, `accentBar` (the
    `default` and `selected` states the report theme schema documents) and `layout.cellPadding`.
    Hover and press states drawn per navigator button since 2026-11-20 (press over hover over
    selected over default). Shapes, glow and shadow since 2026-11-25 (`feat/fx-bookmark-drill`):
    the navigator's `shape` object (`tileShape` and its corner adjustments, no selector) gives
    every button the shape visual's outline, filled and outlined along it; the per-state `glow`
    and `shadow` objects (report theme schema 2.159) reach `navigator.styles` and the renderer
    draws a glow (three widening strokes, the accent colour and 50 % transparency unless set)
    and a shadow (the outline moved by distance and angle, 3 px at 45° and 70 % transparency
    unless set) under each button; the Format visual pane has Shape, Shape glow and Shape shadow cards for
    both navigators (EN/FR). Tests: engine `navigator_shape_glow_and_shadow_reach_the_snapshot`,
    render `navigator_buttons_draw_their_shape_shadow_and_glow`, kittest
    `a_navigators_shape_glow_and_shadow_cards_format_it`. Left: these drawing defaults are
    BIcraft's (no public source gives Desktop's), inner shadows are drawn as outer ones (the
    action button's glow, shadow and shape are drawn the same way since 2026-11-27, G52), text alignment, theme `visualStyles`
    for navigators, Desktop's own default look; not compared with Desktop 2.159.
  - Sync groups are applied by the readers (checked 2026-11-18; the propagation existed since
    M8.3, this entry was stale): a click on a slicer item reaches every slicer of its group on
    every page, in the app and in the browser viewer, so another page shows the synced slicer
    ticked and its visuals filtered (`filterChanges`; oracle rule 6: joining a group copies no
    selection). Fields on (`fieldChanges`, M11.25, 2026-11-18): a field bound, removed or moved
    on a slicer of the group goes to the group in the same command, one undo step
    (`crates/engine/src/slicer_sync.rs`, tests `slicer_sync_view_tests.rs`,
    `sync_ui_tests.rs`, seen red then green); a group name typed in another case joins the
    existing group. Left: a slicer joining a group with Fields on is not aligned on the others'
    fields until the next change, and `fieldChanges` is inferred from the option's name, not
    checked against Desktop (the interaction oracle does not cover it).
  - No chapter or personal bookmark (service only: out of scope).
  - The three panes were laid out from `docs/design.md` §5 and the product's documented behaviour,
    not from a screenshot: the strips, pane widths, row heights, labels and their order have not been
    compared visually with Power BI Desktop 2.159 in this batch. Only the engine, the kittest checks
    and the CI gate are evidence here.
- **Effort.** S for what is left (a Desktop-saved drilled bookmark, drill mode and button effects
  to compare: each needs Desktop); the reader's bookmarks and sync groups landed on
  2026-11-18, slicer selections in bookmarks and navigator styles on 2026-11-19, drill states
  in bookmarks and navigator shapes, glow and shadow on 2026-11-25.

### G56. Analytics beyond reference lines (2026-11-18)

- **Done (2026-11-18).** The Visualizations pane has Desktop's Analytics tab
  (`crates/ui/src/analytics_pane.rs`): a card per analytics kind the visual type allows, Add,
  options and Remove, each gesture one `visual.analytics` (one undo step), no tab for readers.
  `visual.analytics` takes `kind: "error_bars"` (`objects.error` per series: measure or
  aggregation bounds, `relative`, bar, marker, line-chart band), `ratio_line` (`ratioLine`) and
  `symmetry_shading` (`plotAreaShading`); the snapshot gives `data.error_bars`. The renderer draws
  error bars on clustered columns and bars and on lines, the line band, the scatter's constant
  and statistic lines, its ratio line and its symmetry shading.
- **Error bars per series, stacked and combo (2026-11-27, branch `feat/g56-errorbars`).** With a
  legend every legend series gets its bars at its own points, in its colour when the bars match
  the series' colour, else the error-bar formatting (the legend's value holds the one entry,
  `crates/render/src/data.rs`). On stacked and 100% columns and bars each series' bars sit at the
  top of its segment, each bound as far from that top as from the series' value: the `error`
  object has no property that chooses the stack's top or the segment's, so this is BIcraft's
  reading ([formats/pbir.md](formats/pbir.md) "Chart specifics"). Both combo charts draw them on
  the columns (clustered or stacked) and on the line. Render tests
  (`crates/render/src/analytics_tests.rs`) check each series' caps at its own bounds and bar in
  its colour, no cap inside a stacked segment where its unshifted bounds would be, and both
  parts of a combo; each failed before the change.
- **Left.** The `errorRange` shape, the error bars' defaults and the stacked placement are
  BIcraft's reading (no public source; "relative = value + bound" from community notes), not
  checked against a file Desktop saved or Desktop's drawing. No error bars on stacked areas or on
  a combo's line with a legend; no constant X line on a categorical axis.
- **Anomaly detection (2026-11-19).** `visual.analytics` takes `kind: "anomaly_detection"`
  (`objects.anomalyDetection`, the theme schema's properties, `selector.metadata` the series),
  refused where Microsoft Learn says Power BI has none (not a line chart, a legend, several or
  secondary values, beside a forecast or a min, max, average, median or percentile line). The
  Analytics tab has a Find anomalies card. The preview gives `data.anomalies` and the renderer
  draws the expected range and the markers: BIcraft's own estimate (a moving median and the
  residuals' median absolute deviation, `crates/render/src/anomaly.rs`), labelled as such in the
  card, the render note and the data, never Power BI's SR-CNN result. Left: the sensitivity's
  `transform` and the selector are BIcraft's reading, not checked against a file Desktop saved;
  the time-series axis is not required.
- **Anomaly explanations (2026-11-27, branch `feat/g56-errorbars`).** `visual.explain_anomaly
  {page, visual, point?, compare?, fields?, top?}` (`crates/engine/src/visual_analyze.rs`,
  [analyze-point.md](analyze-point.md#explain-an-anomaly)): BIcraft's own explanation, labelled
  as such. By default each candidate field's values go through the same detection as the line and
  the field is scored by how much of the deviations from their expected values one value carries
  (`visual.analyze`'s change score, with the expected value for the previous point); `compare:
  "previous"` is `visual.analyze`'s change. The app: an anomaly's right-click › Explain the
  anomaly (EN, FR), shown only on a flagged point. Checked: engine tests on a synthetic star (the
  planted region ranks first, ahead of the channel and the related table's colour; ordinary
  points refused) and a kittest (the entry only on the anomaly, its point sent). Left: no
  comparison with Desktop's explanations, Explain by's fields are not kept in the report, and a
  left click on the marker only selects the point.
- **Effort.** S–M.

### G57. "Show items with no data" (2026-11-18)

- **Done (M11, 2026-11-18).** `visual.show_items_no_data` sets the role's `showAll`; the preview
  lists the rows of the category's table that pass the filters, with empty values.
  `visual.show_value_as` writes a percentage of the grand, column or row total as the
  projection's `Arithmetic`/`ScopedEval` (public `semanticQuery` schema); the preview divides by
  the visual evaluated at that scope. Both are in the Build pane's well item menu.
- **Checked in Desktop (2026-11-23, 2.159.1231.0 Store).** A PBIP BIcraft wrote opens: the
  matrix's percent of column total and the chart's percent of grand total show the right
  percentages and Desktop names them as its own quick calculations; a table's `showAll` lists
  the category with no data. A chart with both `showAll` and a percent of the grand total is
  drawn without the empty categories and flagged (the two are not compatible in Desktop): the
  preview now leaves them out too and `visual.show_items_no_data` / `visual.show_value_as` warn
  ([formats/pbir.md](formats/pbir.md) "Show items with no data").
- **Left.** The two PBIR forms are not compared with a file Desktop saved itself (operand order,
  the scope Desktop writes for a column total, its `queryRef`, the `0.00%` format); `showAll`
  beside a percent of a column or row total not tried in Desktop; the subtotal levels' empty
  members and the hierarchy levels in the Scope (below) not opened in Desktop. Other Show value
  as kinds of Desktop's menu, if any, are not modelled.
- **Done (2026-11-27, branch `feat/g57-subtotals`).** A matrix's subtotal levels list the members
  with no data of their show-all fields (`preview_matrix::levels`), so a collapsed matrix shows
  an empty category as its leaf rows do. A percent of a column or row total over hierarchy-level
  rows or columns is accepted and writes every level of the role in the `ScopedEval` Scope as
  its projection's `HierarchyLevel` expression (the schema says the scope holds columns; a level
  groups by its column; inferred), read back as the same percentage from PBIP and PBIX (legacy
  `prototypeQuery`). Per-level subtotals (`subTotals` `perRowLevel` / `perColumnLevel` with
  `selector.metadata`) were already read, drawn and written (formats/pbir.md "Matrix expansion
  and per-level subtotals"). Tests: engine `a_matrix_subtotal_level_lists_its_items_with_no_data`
  and `a_percentage_over_hierarchy_level_rows_writes_the_levels_in_its_scope`, each seen red
  without its change.
- **Effort.** S.

### G58. Get data and the Navigator (2026-11-18)

- **Transform data (2026-11-18).** The Navigator's Transform data and Home › Transform data now
  open the Power Query editor (lot 12, G67).

- **Done (2026-11-18).** Home › Get data opens a source gallery (Excel workbook, Text/CSV, JSON,
  Folder, Enter data, Blank query); the Navigator lists a source's items (`source.navigate`:
  sheets, tables and defined names, CSV with a guessed separator and encoding, JSON lists and
  records, a folder's files and a combined CSV item) with check boxes, a search and a preview
  (`query.preview`); Load sends one `source.load` (one undo step, all or nothing). M shapes:
  `crates/data/src/m/navigate.rs`, [formats/data-sources.md](formats/data-sources.md).
  `source.navigate` is a read open to readers, its paths resolved in the workspace like
  `query.preview {data_root}` (`roles_tests.rs` `UNGATED_READ_PATHS`).
- **Connectors in the gallery (lot 11).** SQL Server database (G68), Web, OData feed, SharePoint
  folder and SharePoint list (G69).
- **Left.** No other database or online connector in the gallery; the Navigator's M (step names, `Csv.Document` without
  `Columns`, BIcraft's own folder combine) not compared with what Desktop writes; xlsx is tested
  in `crates/data` only.
- **Effort.** M (done).

### G60. Optimize ribbon (2026-11-18)

- **Done (2026-11-18).** Microsoft Learn's "Optimize ribbon in Power BI Desktop" and
  "DirectQuery optimization scenarios with the Optimize ribbon" (`crates/engine/src/optimize.rs`):
  - `report.pause_visuals {paused?}` writes `report.json` `settings.pauseQueries` (saved: Learn
    says a paused report stays paused when reopened). While paused, a visual whose data is not in
    the result cache is not evaluated: it keeps the data it last showed with `pending: true`, or
    gets the `pending` status (`preview::report_data`, `Pause`).
  - `report.refresh_visuals` (read-only): the next view evaluates every visual once, the report
    staying paused; the Performance analyzer's `perf.refresh` asks the same.
  - `report.query_reduction {preset?, disable_cross_highlighting?, slicers?, filters?}` writes
    `slowDataSourceSettings` (public schema `ExplorationSlowDataSourceSettings`): Query
    reduction = cross-highlighting off by default, an Apply button on each slicer, a single Apply
    button on the Filters pane; Interactivity removes them all. The viewer follows the first: a
    selection reaches only the visuals an Edit interactions rule names.
  - UI (`crates/ui/src/optimize_ui.rs`): Analyze › Optimize (Pause visuals toggle, Refresh all
    visuals, Optimization presets menu with Customize… opening File › Options and settings at its
    Query reduction section) and the banner "Visual queries are paused" with Refresh and Resume
    visual queries. Tests: engine `optimize_tests` (4), kittests `optimize_ui_tests` (4).
- **Done (2026-11-27, branch `feat/g60-optimize`).** From the same Learn pages, described in
  BIcraft's words:
  - An **Optimize** ribbon tab after View (`Tab::Optimize`, engine and UI): Pause visuals
    (Pause visuals, Refresh all visuals), Optimization presets, and the Performance analyzer
    button (the same pane as Analyze › Performance). The commands left Analyze › Optimize.
  - `visual.refresh {page, visual}` (read-only): the next view evaluates that visual once, the
    others staying paused (`PreviewCache::request_refresh_one`, `Pause::only`). UI: each waiting
    visual (`data.pending` or `status: "pending"`) shows a strip at its top while paused, with a
    pause icon, the words "Pending changes" and a focusable Refresh visual button; the visual's
    right-click menu has Refresh visual while paused.
  - While paused, measure-bound formatting values (`preview::report_bound`: titles, subtitles,
    button text and fill, text box runs, fx destinations) are not evaluated: the visual keeps the
    values it last showed, marked `pending`. `view.interact` refuses a report-page `tooltip`, and
    the app shows no tooltip on a waiting visual.
  - Query reduction's Apply buttons are honoured: with `isSlicerSelectionsButtonEnabled` every
    slicer's changes wait (`slicer.pending`, `slicer.apply_button` in the snapshot) until
    `view.interact` `slicer_apply {page, visual}` (the slicer's Apply button at its bottom right);
    with `isFilterSelectionsButtonEnabled` a basic Filters pane card's clicks wait in the pane,
    marked "Not applied", until its Apply filter; with `isApplyAllButtonEnabled` the pane's single
    Apply all runs every waiting edit.
  - Tests: engine `optimize_tests` (4 new), kittests `optimize_ui_tests` (6 new), each seen
    failing with its change undone.
- **What is left.** Nothing compared with Desktop (no Desktop run). A pure formatting change
  (a colour) changes the visual's cache key, so a paused visual is marked pending though its data
  did not need a query (Desktop applies many formatting options at once); per-card Apply covers
  basic cards only (advanced, Top N and relative cards already wait for their Apply filter); the
  filters' waiting edits are view state of the window, not of the session, so automation applies
  a filter by running its command; the status bar does not repeat "Visual queries are paused"; a
  legacy layout keeps the options under `config.slowDataSourceSettings`, a name inferred from the
  PBIR one, not seen in a Desktop file; the paused visuals' last data is a second copy in memory
  (bounded by the snapshot's limits).
- **Effort.** S.

### G61. Model explorer and calculation groups in the Model view (2026-11-18)

- **What.** Calculation groups and items are edited through the generic forms
  (`calculation_group.*`, `calculation_item.*`). Desktop's Model explorer tree (the semantic
  model's relationships, roles, cultures, perspectives, calculation groups, shared expressions)
  and the ordering of items by drag are not there.
- **Lot 18 part 1 (2026-11-18).** The diagram is editable: drag a card (`diagram.move`, one
  undo step, saved in the PBIP `diagramLayout.json` or the PBIX `DiagramLayout` part, see
  [formats/diagram-layout.md](formats/diagram-layout.md)), drop a column on another table's column
  (`relationship.add`), double-click a relationship (`relationship.set` form), Delete
  (`relationship.remove`). The Properties pane (`crates/ui/src/props_pane.rs`) edits the selected
  table, column, measure or relationship through their set and rename commands.
- **Autodetect relationships (2026-11-19).** `relationship.autodetect`
  (`crates/engine/src/relationship_autodetect.rs`): same column name and data type in two
  unrelated tables, a unique side read from the data (many-to-one single, or one-to-one both),
  one per pair of tables, none that makes paths ambiguous; a preview, then `apply` (one undo
  step). The Model explorer's Relationships menu › Autodetect relationships lists them to tick
  (`crates/ui/src/autodetect.rs`).
  Unchecked against Desktop: its name matching beyond equal names, which match wins in a pair,
  whether an ambiguous candidate is made inactive instead of left out
  ([formats/model-objects.md](formats/model-objects.md)).
- **Manage relationships (2026-11-20).** Modeling › Manage relationships opens Desktop's list
  (`crates/ui/src/relationships_dialog.rs`: active box, from, to, cardinality, cross filter; New,
  Autodetect, Edit, Delete) instead of a form and a grid; the Create / Edit window picks a table
  per end with a preview of its first five rows (`table.rows`), the column by a click on the
  preview's header, cardinality (four choices), cross filter direction, Make this relationship
  active, Assume referential integrity, Apply security filter in both directions. Each change runs
  `relationship.add` / `relationship.set` with `check: true` (`crates/engine/src/relationship_review.rs`):
  the errors and warnings show at once and a change of columns takes the suggested cardinality,
  direction and activity. `relationship.add` now refuses a "one" side with duplicates or blanks
  and no unique side (unless many-to-many), and adds a relationship inactive (with a note) when
  another is active between the two tables or paths would become ambiguous; `relationship.set`
  moves an end (`new_from` / `new_to`, PBIX storage rebuilt under the same name); both take
  `assume_referential_integrity` (DirectQuery or Dual tables) and `security_filtering`.
  Left, unchecked against Desktop: its exact messages and their order, whether it suggests
  many-to-many with single or both directions, whether it greys or unticks "Make this
  relationship active" when it cannot be active, its many-to-many cross filter default, whether
  Dual tables allow referential integrity; Delete removes without the confirmation Desktop asks
  (it can be undone); the review reads up to 5,000,000 values per column each time a column or
  option changes (no cache; a check is no undo step, journal line or new revision); the review's
  messages are the engine's, in English in the French window too; the diagram's drag and drop
  still creates many-to-one without the window.
- **Perspectives (2026-11-19).** `perspective.add`, `perspective.set` and `perspective.remove`
  (`crates/engine/src/perspective_edit.rs`) write a perspective's tables, columns, measures and
  hierarchies one by one; Manage perspectives (the Model explorer's Perspectives menu,
  `crates/ui/src/perspective_dialog.rs`) ticks them; the Model view's Perspective list filters
  the diagram (view state). Unchecked against Desktop: how its own diagram filters by
  perspective; an `includeAll` a file brought is rewritten as the members it stands for once
  the perspective is saved from the dialog.
- **Multi-selection and synonyms (2026-11-20).** `model.set_properties`
  (`crates/engine/src/set_properties.rs`) sets the same properties on several tables, columns
  and measures in one undo step; synonyms go into the model culture's Q&A schema
  (`crates/engine/src/synonyms.rs`). Ctrl+click in the diagram, Ctrl or Shift+click in the Data
  pane; the Properties pane shows what the selection shares, a differing value as "Multiple
  values" (`props_pane.rs` `multi`). Tests: engine `set_properties_tests` (6), kittests
  `model_multi_tests` (3). Unchecked against Desktop: Shift+click, the Data pane selection, the
  properties offered for a mixed selection, the synonym states written
  ([formats/model-objects.md](formats/model-objects.md)).
- **The explorer's other objects (2026-11-23).** A click on a calculation item, a hierarchy, a
  role, a culture, a shared expression or a data source selects it (`Sel::Item`) and the
  Properties pane edits it through its commands instead of opening a form: a calculation item's
  name, description, expression and format string expression (`calculation_item.set`), a
  hierarchy's name, description, display folder and hidden (`hierarchy.set`; levels listed), a
  role's name (`role.rename`), description and model permission (the new `role.set`), a shared
  expression's name, description, query group and M text (`expression.rename`,
  `expression.set`; kind Query or Parameter shown). A culture (model culture, linguistic schema)
  and a data source (type, provider, protocol; no connection string or credential in the
  snapshot) are read-only. The tree lists hierarchies with their levels under their table and a
  Data sources node (TMSL `dataSources`); a search box keeps the rows whose name holds its text.
  In a layout tab other than the one of all tables, a table's menu adds its related tables (one
  `diagram.move`) or removes it from the layout (the new `diagram.remove_tables`). The snapshot
  gives `calculation_item_details`, `expression_details`, `culture_details`, `data_sources`,
  role descriptions and hierarchy descriptions. Tests: engine `role_set_writes_…`,
  `tables_leave_a_layout_tab_…`, `the_snapshot_details_…`, kittests `model_explorer_ui_tests`
  (6). Unchecked against Desktop: which properties its pane offers for these objects, whether it
  lists data sources in the Model explorer, how its search matches, where Add related tables
  places the cards.
- **Left.** Key column, row label and featured table in the Properties pane (their storage was
  not observed in a file Desktop saved); a culture cannot be renamed; Remove from layout and Add
  related tables are in the Data pane's table menu, not the diagram card's; the written node
  height (300) and whether `nodeLineageTag` and `size` are required are unchecked against
  Desktop; a PBIX save that only moves cards still rewrites the DataModel metadata.
- **Effort.** M.

### G62. Analyze insights on a data point (2026-11-18)

- **What.** "Explain the increase / decrease" and "Find where the distribution is different" on
  a data point. Whether Desktop 2.159 still offers them was not re-read.
- **Done (2026-11-27, branch `feat/g62-analyze`).** `visual.analyze {page, visual, point, kind?,
  fields?, top?}` (`crates/engine/src/visual_analyze.rs`, [analyze-point.md](analyze-point.md)):
  BIcraft's own ranking, never presented as Desktop's. `change` (default on a date or number
  axis) compares the point with the previous point of its axis and scores each column by the
  share of the members' changes its top member carries; `distribution` (default on a
  categorical axis) scores each column by the total variation distance between its shares under
  the point and the mean of its shares under the other points. Grouped queries through the
  preview evaluator, so filters, slicers and relationships apply; bounded (40 fields, 100
  values each, 20,000 rows a query, 60 s), the rest listed in `skipped`; unsupported points
  are errors. Read-only, shared, allowed to readers and in server mode. UI: a data point's
  right-click › Analyze this point (EN, FR), the answer window's grid. Tests: engine
  `analyze_point_tests` (3: the planted field ranks first for an increase, a decrease and a
  different distribution, through a related table; refusals), kittest
  `analyze_runs_on_the_right_clicked_point`.
- **What is left.** The answer is a list, not a visual added to the page. Only the visual's
  first measure; measures, pairs of fields and binned numeric columns are not candidates. No
  comparison with Desktop's results.
- **Effort.** S for what is left.

### G63. Report settings left (2026-11-18)

- **Done (2026-11-18).** `report.settings` (`crates/engine/src/report_settings.rs`) also writes
  `persistent_filters` (`isPersistentUserStateDisabled`), `personalize_visuals`
  (`allowInlineExploration`), `cross_report_drillthrough` (`useCrossReportDrillthrough`),
  `allow_change_filter_types` (`allowChangeFilterTypes`) and `filter_pane_visible`
  (`objects.outspacePane` `visible`), in PBIR and in a legacy layout
  (`docs/formats/pbir.md`, "Report settings"); the snapshot's `report.settings` shows them. UI:
  File › Options and settings (`crates/ui/src/options_page.rs`) and the Filters pane's eye; a
  reader's window has no Filters pane when the report hides it. Tests: engine
  `report_settings_tests` (5: written, undone, PBIP, PBIX, legacy layout), kittests
  `report_settings_ui_tests` (3).
- **What is left.** The service-side settings are written and kept, but BIcraft's viewer does
  not act on them (personalizing visuals, saved reader state, drillthrough from another report,
  the reader's filter type menu). The defaults of an absent key (personalize, cross-report
  drillthrough and change filter types off) are inferred, not compared with a file Desktop
  saved. "Enable search for Filters pane" (`disableFilterPaneSearch`) and the other `settings`
  keys have no parameter.
- **Effort.** S.

### G64. Field and what-if parameters left (2026-11-18)

- **Done.** `parameter.what_if` makes the `GENERATESERIES` table, its column, the
  `<name> Value` measure and a single-value slicer in one undo step; `parameter.fields` makes the
  tuple table with `NAMEOF`, its three columns and `ParameterMetadata`, and a slicer; the preview
  shows the fields the parameter's selection chooses (`crates/engine/src/parameters.rs`,
  `preview_field_params.rs`); Model › New parameter opens the two dialogs. Note:
  [formats/parameters.md](formats/parameters.md).
- **Checked in Desktop (2026-11-23, 2.159.1231.0 Store).** The model shapes match the field and
  what-if parameters Desktop made on the same model (`ParameterMetadata`, the group-by and sort
  of the label column, `GENERATESERIES`, `SELECTEDVALUE`); Desktop draws BIcraft's what-if
  slicers as its own (a value box and a slider) and BIcraft's field parameter slicer lists the
  labels ([formats/parameters.md](formats/parameters.md) "Desktop answers").
- **Done (2026-11-23).** A visual bound to a field parameter's label column is written as Desktop
  saves it: the parameter's fields as the role's projections (labels as `displayName`) and the
  role's `fieldParameters` entry, kept in step with the model after every edit
  (`field_param_queries.rs`); a `fieldParameters` range reads back as one binding of the label
  column (`bicraft_report::query::derive`), so the preview follows the slicer. Desktop
  2.159.1231.0 shows the fields and follows its slicer ([formats/parameters.md](formats/parameters.md)
  "Desktop answers, second session").
- **Done (2026-11-27, no Desktop needed).** The legacy layout writer writes a visual bound to a
  field parameter as the parameter's fields (one `Select` item and projection each, the labels as
  `columnProperties.displayName`) instead of refusing the save; the `fieldParameters` range is
  not written (no legacy key for it seen in a Desktop file), and a legacy report opened or read
  back after a save binds such a run of projections to the parameter again
  (`field_param_queries::adopt_legacy`). A slicer on a what-if column (`'Single'` mode, or no
  mode as Desktop writes its own) is drawn as a single-value slider at the selected value or the
  parameter's default; in the app the track is an accessible slider (drag, click, arrows) that
  sends `view.interact` `slicer_set`, so the measure follows (`crates/ui/src/value_slider.rs`;
  [formats/parameters.md](formats/parameters.md)).
- **Left.** Desktop's own legacy form of a field parameter visual is *not seen in a Desktop
  file*: a legacy PBIX BIcraft wrote was not opened in Desktop, which may show the fields as
  plain columns. The what-if slider's value box is not an input, its sizes are the range
  slicer's, and the HTML viewer (`page.html`) still draws a list. A file opened is rewritten only
  once edited. Small differences kept: no `formatString: 0` on the what-if measure and order
  column, `summarizeBy: none` (Desktop: `sum`) on the order column, the `'Single'` mode Desktop
  does not write for its own slicer. A field parameter's hierarchies (Desktop's auto-expanding
  axes) are not modelled: the model keeps hierarchies only as raw TOM JSON in a table's `extra`,
  and no Desktop-written field parameter that lists one, or its auto-expanding axis, has been
  observed, so there is no shape to implement without Desktop. No
  dedicated test of a model edit that succeeds while the slicer fails.
- **Effort.** S–M.

### G65. View as roles left (2026-11-18)

- **Done.** `view.interact {action: "view_as", roles, user?}` is view state (snapshot
  `interaction.view_as`, cleared by reset; one per server reader). Every evaluation (visuals,
  slicers, cards, `dax.evaluate`, `table.rows`, `visual.records`) reads the secured tables'
  rows filtered before any query, so `ALL` and `REMOVEFILTERS` cannot leave them; the filters
  follow active relationships (one to many always, many to one when both directions), several
  roles are united, an error in a role filter fails closed; `USERPRINCIPALNAME()` and
  `USERNAME()` return the other user (`crates/engine/src/preview_rls.rs`,
  `preview_eval_rls.rs`). Model › View as opens Desktop's dialog; a banner shows the roles with
  Stop viewing (`crates/ui/src/view_as.rs`). The server applies no model role to its readers
  today, so View as can only narrow what a reader sees; if the server maps callers to roles
  later, View as must intersect with them, never unite.
- **Object-level security (2026-11-19).** `role.set_permission` hides or shows a table or a
  column for a role (`metadataPermission`, refused on a table that would break a relationship
  chain; `crates/engine/src/role_permission.rs`); View as makes what every chosen role hides a
  missing object, and refuses row filters and object security from different roles; Manage
  roles (the Model explorer's Roles menu, `crates/ui/src/roles_dialog.rs`) edits filters and
  object security; TMDL, TMSL and PBIX keep them. Left: a relationship whose key column is
  secured fails under View as (Learn says it still works when its table is not secured); the
  union over several roles and View as's messages are not compared with Desktop
  ([formats/model-objects.md](formats/model-objects.md)).
- **Manage security roles (2026-11-20).** The dialog has Desktop's layout per Learn: roles (New,
  Rename `role.rename`, Delete), Select tables, Filter data in the default editor (rows of
  column / condition / value with And / Or, `crates/ui/src/role_filter.rs`) or the DAX editor
  (`crates/ui/src/dax_box.rs`: the formula bar's colours and completion), Check
  (`role.check_filter`, the errors and warnings with an icon and words, nothing saved), Revert
  and Save. Left: the default editor's conditions and the DAX it writes, its And / Or grouping
  and the messages are not compared with Desktop; an unsaved draft is dropped when another role
  or table is chosen; no Duplicate role; the DAX editor has no signature help (the formula bar's).
- **Left.** A role filter that reads another secured table is evaluated without that table's
  security; one-to-one relationships and the blank row are not checked against the engine
  oracle; the secured state copies the filtered columns in memory (not measured on large
  volumes); `USERPRINCIPALNAME()` outside View as has no signed-in user; the browser app was not
  run live; DAX semantics come from the public documentation.
- **Effort.** S–M.

### G66. Performance analyzer left (2026-11-18)

- **Done.** `perf.start`, `perf.stop`, `perf.clear`, `perf.refresh`, `perf.results`,
  `perf.render` and `perf.export` record BIcraft's own times on a monotonic clock (evaluation per
  visual, render measured around the renderer's draw, the copy-query `SUMMARIZECOLUMNS` text,
  which runs in `dax.evaluate`); editors only, no revision; the pane is Analyze › Performance
  analyzer (`crates/ui/src/perf_pane.rs`). Never Desktop's times (AGENTS.md §1).
- **Desktop-free parts (2026-11-27).** Copy query writes every filter the viewer applies: top N
  filters as `TOPN` tables ranked under the column filters, measure and aggregation filters as a
  `FILTER` over the rows under the same filter tables (`KEEPFILTERS`), beside the report, page
  and visual filters, slicer selections, cross-filters and relative dates it already wrote; in
  `dax.evaluate` it returns the visual's rows under each (`perf_tests.rs`,
  `the_copied_query_returns_the_visuals_rows_under_every_filter`); View as applies to it as to
  any evaluation of the session. Render times without Refresh visuals: while a recording runs
  the snapshot's `perf.unrendered` names the visuals whose latest event has no render time, and
  the app sends the times of the first drawing of that snapshot (`perf.render`). The browser
  build times its draws with `web-time` (`performance.now()`; already in the graph through
  eframe).
- **Left.** The pane was not run in a live browser (wasm32 check only): that needs
  `cargo xtask webui` to build the page, `bicraft-cli view FILE --ui egui` and a browser driven
  through the control channel (`bicraft-cli ui --view …`) to start a recording, interact and read
  `render_ms` back. The copy-query text is BIcraft's, not compared with Desktop's; a condition
  mixing columns and measures is not applied by BIcraft and not written; when the evaluator
  falls back without its filters (an unsupported filter) the text still holds them; a
  cross-highlighted visual records its full and highlighted evaluations as two events; a
  snapshot whose drawing is skipped (a newer one drawn first) leaves its visuals without a
  render time.
- **Effort.** S.

### G67. Power Query editor left (2026-11-18)

- **Done.** Home › Transform data and the Navigator's Transform data open the editor
  (`crates/ui/src/pq_editor.rs`): the queries list, the applied steps from the `let` chain
  (select, rename with references rewired, delete, insert after), a 100-row preview per step
  (`query.preview {step}`), the editable formula bar and a transform ribbon writing M (18
  transforms: filter, remove or keep columns, rename, change type, promote headers, sort, remove
  duplicates, replace values, group by, merge, expand, append, custom column, split, unpivot,
  pivot, free M). The user's text, comments included, is kept for untouched steps
  (`crates/data/src/m/steps.rs`). Close & Apply is `query.apply`: one undo step, all or nothing,
  shared queries first, the tables that reach a changed query reloaded.
- **Data profiling (2026-11-19).** `query.profile` (`crates/engine/src/query_profile.rs`): each
  column's valid, error and empty shares (error values counted by the evaluator,
  `bicraft_data::m::load_m_counting_errors`), distinct and unique counts and most frequent
  values, and one column's statistics and value distribution, over the top 1000 rows or the
  entire data set (Microsoft Learn, "Using the data profiling tools"). The editor's View tab:
  Column quality and Column distribution under each header, Column profile in a bottom pane,
  the entire data set switch and the status line. Left: the statistics per type and the share
  rounding were not compared with Desktop's; "empty" counts null and empty text; no histogram
  by value range for numbers or dates (the most frequent values only); no Group by on the value
  distribution, no right-click actions on its bars; which error rows became blanks is not kept
  (the profile's errors are counts).
- **Query dependencies (2026-11-19).** `query.dependencies` (`crates/engine/src/query_deps.rs`):
  the model's queries or the editor's drafts, the data sources they call, their references and
  the model tables they load into, in layers (Microsoft Learn, "Query dependencies"); View ›
  Query dependencies draws it with zoom in, zoom out, fit and drag (`crates/ui/src/pq_deps.rs`).
  Left: the layout is BIcraft's own (layers, no crossing reduction), not compared with
  Desktop's; no selection highlighting of a node's chain, no layout direction choice; a source is
  labelled by its function and literal arguments (a computed path is shown as `…`).
- **M completion, hover and signature help (2026-11-23, M11.35).** The Advanced editor and the
  formula bar complete M, show the signature of the call being typed and say what the name
  under the pointer is (`crates/ui/src/m_box.rs`), from BIcraft's catalogue of the functions its
  evaluator runs (`crates/mlang`; `m.complete`, `m.signature`, `m.hover` for agents). Left: the
  descriptions are English only (the kind words are translated); functions the evaluator does
  not run (connectors, documented-only names) are not suggested; parameter types and return
  types are not shown; columns come from the preview of the step shown, not from the step the
  cursor is in; no colours in the M editors; not compared with Desktop's IntelliSense.
- **Left.** No Advanced editor over the whole query from the editor; no type or filter menus on
  the column headers; Expand asks for typed column names; no step settings gear; no query
  management (new, duplicate, reference, rename, groups); a merge preview does
  not see the other queries' drafts; Cancel does not confirm; step names are written in English;
- **Transform and Add column tabs (2026-11-23).** `query.apply_step` gains 21 kinds
  (`crates/data/src/m/transform_more.rs`, read by `crates/engine/src/query_transforms.rs`):
  Keep and Remove rows (top, bottom, range, alternate, blank, errors), Replace errors, Fill
  down and up, Transpose, Reverse rows, Use headers as first row, Move (reorder), Duplicate
  column, Index column, Merge columns (in place or added), Conditional column, and `column_op`:
  Format, Extract, Standard, Rounding, Date and Time items in place (`Table.TransformColumns`)
  or as a new column (`Table.AddColumn`), from the Microsoft Learn M function reference. The
  editor's Transform and Add column tabs and Home's Keep rows and Remove rows menus
  (`crates/ui/src/pq_tabs.rs`) ask their values in the window and run that command. Filters
  write `#date` / `#datetime` literals (`{"date": …}`, `{"datetime": …}`); `query.preview`
  gives a date/time column's M type (`date_kind`), so a date column's filter menu lists its
  values. The step settings gear: `query.steps` reads each step's `{kind, …}` back from its M
  (`crates/data/src/m/transform_parse.rs`, the text parsed, never evaluated) for the
  transforms that ask values; Step settings reopens the form filled, OK writes the step anew
  over the step it read (`query.edit_step {settings}`). `query.preview` and `query.profile`
  take the editor's drafts of the other queries (`queries`), which a merge or a reference
  reads. Left: the step and new column names (`Uppercased Text`, `Inserted Year`, `Merged`…),
  the M of Merge columns in place (every column typed as text first) and of the row
  reductions were not compared with Desktop's; Statistics, Parse, Extract by delimiters with
  ranges, Split by number of characters, positions or case transitions, Column from examples,
  Detect data type, Count rows and Group by's advanced form with several aggregations are not
  offered (Advanced editor or Custom column); a conditional column's output is a value (not
  another column); the gear covers the transforms that ask values (not Merge queries' column
  name, Group by with several aggregations, a filter on a date value, Custom column, which
  stays M); a deleted query's name is still read from the model by a preview.
- **Number, date, time and duration menus; Export results (2026-11-23, G106).** `column_op`
  gains 41 operations: Standard (integer-divide, modulo, percentage, percent of), Scientific
  (power, square and cube as powers, square root, exponent, base-10 and natural logarithms,
  factorial), Trigonometry (sine, cosine, tangent and their inverses), Information (is even, is
  odd, sign), Date (age, start and end of quarter, week and day, days in month, week of month),
  Time (time only, start and end of hour), Duration (days, hours, minutes, seconds, total
  years, days, hours, minutes, seconds) and Extract › Text between delimiters, with the M
  reference's functions (`Number.Factorial` and `Number.Sin` … `Number.Atan` added to the
  evaluator); the gear reads integer-divide, modulo, power and between delimiters back. The
  Transform and Add column tabs gain Scientific, Trigonometry, Information and Duration menus.
  `query.export {out}` writes every row of a query to a CSV file (Home › Export results,
  Desktop's Output data). Left: Age reads the refresh clock (a preview without one shows
  errors); Start and End of day or hour are typed `datetime` even over a time column; total
  years is total days / 365 as Desktop's menu describes it, not compared with its M; the
  Standard menu over several columns of Add column (`[A] + [B]`) is not written; the step and
  column names (`Calculated Power`, `Inserted Calculated Sign`) were not compared with
  Desktop's; Export results writes only CSV and the whole query (not the step shown).
- **Left.** Expand asks for typed column names; no query groups; Cancel does not confirm; step
  names are written in English;
  a table with several partitions is not reloaded (only its partition changes, with a warning).
  Folding indicators: BIcraft's own (the query up to each step: folded, partially folded, not
  folded, unknown; [formats/data-sources.md](formats/data-sources.md), *Folding indicators*),
  not compared with Desktop's per-step indicators; each step is probed anew (navigation listings
  are read again per step); View native query is read-only and shows what BIcraft would send.
  Query diagnostics: one step (or each step in turn) at a time, BIcraft's own times; no recorded
  diagnostics session over a refresh, no diagnostics tables as Desktop writes them.
  Not compared with what Desktop writes: the `Changed Type1` naming, the generated M's spacing,
  `[PromoteAllScalars=true]`, rewiring after an insertion mid-chain.
- **Effort.** M.

### G68. SQL Server connector left (2026-11-18)

- **Done.** `crates/sqlserver` (pure Rust, synchronous, std::net and rustls; TLS required):
  PRELOGIN, LOGIN7 with SQL authentication, SQL batches, the common column types, error tokens as
  M `DataSource.Error`, Azure routing, named instances through the SQL Server Browser; bounded
  reads, a fuzz target (`tds_tokens`). M: `Sql.Database` (navigation table of tables and views),
  `Sql.Databases`, `[Query=…]` as a native query; selecting columns, rows, `Table.FirstN` and
  (SQL Server only, non-text columns) `Table.Sort` fold into one `SELECT`. `source.credentials`
  keeps the user and password in session memory only (refused in server mode); Get data › SQL
  Server database and its Navigator; opt-in through `allow_connectors: ["sqlserver"]`. Note:
  [formats/tds.md](formats/tds.md).
- **Left.** Never run against a real SQL Server or Azure SQL (the opt-in test
  `BICRAFT_SQLSERVER_TEST_SERVER` was not run); no Windows or Azure AD authentication; the
  Navigator's M shape not compared with Desktop's; `Sql.Databases` is not in the gallery; datetime
  values to the millisecond; the options in `IGNORED_OPTIONS` have no effect; no privacy levels;
  web, OData and SharePoint: G69. (RUSTSEC-2026-0543, chrono unmaintained, which predated this
  lot, went with M11.43: chrono was replaced by jiff.)
- **Effort.** M.

### G69. Web, OData and SharePoint connectors left (2026-11-19, 2026-11-20)

- **Done.** `crates/webdata` (pure Rust, synchronous: ureq with rustls, roxmltree for CSDL):
  `Web.Contents` with `RelativePath`, `Query`, `Headers`, `Timeout`, `Content` (POST),
  `ManualStatusHandling`, `IsRetry`; status errors as catchable `DataSource.Error`; bounded
  bodies, deadlines, a response cache per connector. `OData.Feed` (JSON, OData v4; v3 JSON light
  names read): the service document as a navigation table (`Name`, `Data`, `Signature`, `Kind`),
  `$metadata` for the columns and types, an entity-set URL as a table; columns, whole-number,
  date, logical and null filters, sorts and row limits fold into `$select`, `$filter`,
  `$orderby`, `$top`; next links followed on the feed's origin. Get data › Web (a JSON document
  or delimited text at a URL) and OData feed, with their Navigator; opt-in through
  `allow_connectors: ["web"]` and `["odata"]`. Fuzz target `odata_payloads` and its smoke test.
  Note: [formats/odata.md](formats/odata.md).
- **Done (2026-11-20).** `SharePoint.Files`, `SharePoint.Contents` and `SharePoint.Tables` over
  the SharePoint REST API (`_api/web`, `odata=nometadata`, paged, same origin only), file
  `Content` and nested folders and list items read only when used; Learn's options checked.
  Credentials per URL prefix for Web, OData and SharePoint (`source.credentials {kind: "web"}`:
  anonymous, Basic, Web API key as `ApiKeyName` or a header, bearer token), in session memory
  only, scoped on redirects (followed by BIcraft, `Authorization` dropped across origins). Privacy
  levels None, Public, Organizational, Private recorded per data source (`source.privacy`) and
  listed with the credentials (`source.settings`). UI: Get data › SharePoint folder and
  SharePoint list, an Authentication choice in the URL forms, Home › Data source settings (Edit
  Permissions, Clear Permissions). Note: [formats/sharepoint.md](formats/sharepoint.md).
- **Left.** Never run against a real service or SharePoint site (an in-process test server
  only); no OAuth sign-in (an organizational account's access token must be supplied) and no
  Windows authentication; **the Data Privacy Firewall is not enforced** (BIcraft behaves as
  Desktop with *Ignore the Privacy Levels*: Learn describes the levels and the firewall's
  partitions, but not enough to reproduce which combinations Desktop refuses) and the levels are
  kept in the session only (Learn does not say where Desktop stores them); no `Web.Page` /
  `Web.BrowserContents` (HTML tables), no `Xml.Tables` (the evaluator has no XML reader), no
  binary metadata (`Value.Metadata` of a response); OData Atom/XML payloads (v2/v3 services
  without JSON), `$expand` and navigation columns, complex and collection columns,
  `Edm.Duration`, singletons and function imports are not read; text comparisons are not folded
  (the service's collation is unknown); SharePoint `ApiVersion = 14` uses the version 15
  endpoints, `DisableAppendNoteColumns` has no effect, list columns that are records (lookups,
  people) cannot be loaded into a model table (select or expand them first), nothing folds into a
  SharePoint request; the Navigator's M shapes (`Signature` and `Kind` columns, step names,
  `Csv.Document` without `Columns`, the SharePoint list step) not compared with a file Desktop
  saved.
- **Effort.** M (OAuth sign-in: M; the firewall: L, needs black-box observations first).

### G70. UI speed tests left (2026-11-19)

- **Done.** `crates/ui/src/tests/speed_ui_tests.rs` ([ui-perf.md](ui-perf.md)): the window over a
  generated report (8 pages of 10 visuals; 100,000 rows in the gate, 1,000,000 nightly), the
  engine on a worker thread and pages on the render thread as in the desktop; frame times and
  input-to-screen latencies of opening, idle, page tabs, dragging a visual, the Format pane, the
  formula bar, the Data view's scroll and the Power Query preview, asserted against generous
  budgets; an idle window requests no repaint.
- **Done (2026-11-25).** Resizing a visual (the content follows its outline on every frame) and
  opening a `.pbix` / `.pbip` from disk through the desktop's open path, stage by stage
  (`apps/bicraft-desktop/src/tests/open_speed_tests.rs`), measured in a release-like profile.
- **Left.** Numbers in the release profile itself (thin LTO; 2026-11-25's are release-like); the GPU's
  part of a frame (wgpu, texture uploads) and the OS event loop's latency (kittest has neither);
  the browser shell (wasm32, no clock in the tests); the Model view's diagram on a model of
  hundreds of tables; memory (RSS) of the window; a saved baseline compared run to run (as
  DeckCraft's `cargo xtask bench` does for its engine); a soak of seeded random input with a
  frame budget (DeckCraft's `ui_soak_tests.rs`); numbers on Linux. The Power Query editor's
  preview at 1,000,000 rows takes 5.7–9.1 s (the step evaluated in full, off the window's
  thread); a preview limited to its first rows would be faster. A visual the general evaluator
  cannot evaluate (more than 20,000 groups, more work than it allows) falls back to the row-by-row
  evaluator, which still reads at most 2,000,000 rows a table (`data_table_rows`: the visual is
  then `unsupported`, its reason naming the cap); so does a visual of columns only of several
  tables (an `error` saying so). View as (row-level security) still reads at most 2,000,000 rows
  a table.
- **Done (2026-11-25).** The viewer's visuals through the storage engine
  ([ui-perf.md](ui-perf.md) "Visuals through the storage engine"): the general evaluator first,
  the row-by-row evaluator as the fallback; the 10,000,000-row files draw every visual (none
  before), the first snapshot at 1,000,000 rows 204–235 → 36–64 ms (release-like); tests in
  `crates/engine/src/preview_grid_tests.rs`. A top N filter beside a condition on the grouped
  table was dropped by the row-by-row evaluator (every row shown); fixed.
- **Effort.** S–M.

### G71. Visual calculations left (2026-11-19)

- **Done.** `visual.calculation.add`, `visual.calculation.set` (expression, rename, `hidden` on
  any field) and `visual.calculation.remove`, stored as PBIR `NativeVisualCalculation`
  projections and `hidden` flags that PBIP and PBIX round-trip; a binder of their own in
  `crates/dax/src/visual_calc.rs` (fields by name, axes, resets, cycles); evaluation over the
  visual matrix with axes, resets, the window shortcuts, `COLLAPSE`/`EXPAND` and their `ALL`
  forms, `ISATLEVEL`, `RANGE`, aggregations and scalar functions; tables, matrices and charts
  show them and drop hidden fields; the edit window (More options › New calculation) with the
  templates, the formula bar, the Hide switches and the visual matrix. Note:
  [formats/visual-calculations.md](formats/visual-calculations.md). 2026-11-23: `LOOKUP`
  (`EXPLICIT`) and `LOOKUPWITHTOTALS` over every level of the visual matrix, `OFFSET`, `INDEX`
  and `WINDOW` (as tables of `CALCULATE` and the `X` functions), `RANK` (`SKIP`, `DENSE`) and
  `ROWNUMBER`, `ORDERBY` of fields or expressions, `PARTITIONBY`, `ROWPAGES` and its
  combinations as walked axes (small multiples partition otherwise), and the axes ordered by the
  model's sort-by column; visits and positions capped, errors as blank cells or reasons
  (`preview_viscalc_tests.rs`: `lookup_and_window_functions_*`,
  `rowpages_walks_the_small_multiples`, `the_axis_follows_the_sort_by_column`,
  `window_arguments_out_of_reach_give_reasons`). 2026-11-24: `MATCHBY` of the visual's fields
  (they must single out the current cell in its partition, else an error in that cell) and
  `LOOKUP … INFERRED` (a field on a supplied field's axis with one value among the matching cells
  is inferred, not taken from the current cell) (`matchby_and_lookup_inferred_follow_the_matrix`).
  2026-11-27: the small multiples as `ROWPAGES` levels: the visual matrix has levels without the
  small-multiple fields, so `COLLAPSE`/`COLLAPSEALL`/`EXPAND` along `ROWPAGES` (or to a
  small-multiple field), `ISATLEVEL` of one and `LOOKUPWITHTOTALS` without a value for each small
  multiple are evaluated (each was a reason); walks along `ROWS` restart on each multiple
  (`rowpages_navigates_across_the_small_multiples`, `rowpages_walks_the_small_multiples`). The edit
  window previews what is typed: `visual.calculation.add`/`set {preview: true}` evaluate the visual
  with the calculation and change nothing (no undo step, no revision); 400 ms after the last
  change the window sends it and shows the data in its matrix, or the error under the formula bar
  (`a_preview_evaluates_the_calculation_without_changing_the_report`, kittest
  `typing_previews_the_calculation_without_applying_it`).
- **Left.** **Not checked against the engine**: no Desktop file and no DAX oracle was compared,
  every expected value (those of 2026-11-23, 2026-11-24 and 2026-11-27 too) is computed by hand from the
  Learn pages of the functions; the check needs the maintainers' development oracles or Desktop
  (outside this repository). `COLLAPSE`/`EXPAND` over several axes (`ROWS ROWPAGES`) take
  levels off the last-named axis first, as for `ROWS COLUMNS` (BIcraft's reading). `INFERRED` reads functional dependence from the visual matrix's
  values along the supplied fields' axes, not from the model (Learn does not say how the engine
  finds it; a field on another axis is never inferred). Not evaluated (the column stays empty
  with its reason): `MATCHBY` of anything but the visual's fields, a model table as a window
  function's relation, other DAX functions. `RANK`/`ROWNUMBER` are blank
  on a total of the walked axes (Learn: blank for total rows; not checked at subtotals); the
  sort-by order is not applied to `ORDERBY` of a field; the axis order ignores the visual's own
  sort (by value); rows beyond the visual's row limit are not in the matrix. The `queryRef` written for a calculation is its name (Desktop's choice is not
  documented). Hiding a matrix's row or column grouping field is drawn by dropping its column
  only (the matrix's levels are not rebuilt without it; not compared with Desktop). The edit
  window has no field-list drag into the calculation; its live preview is not compared with
  Desktop's edit mode.
- **Effort.** M (LOOKUP and the window functions over the matrix: M).

### G72. Mobile layout view left (2026-11-19)

- **Done.** View › Mobile layout: the phone canvas (portrait, Desktop's 324 × 3600 since 2026-11-24, below; first 323 per Microsoft
  Learn, at least 568 high), the Page visuals pane (drag or double-click a visual onto the
  phone), the canvas's gestures on the phone (move, the eight handles, Shift, arrows, the
  grid, 18 × 20 since 2026-11-24), Delete, the X and the menu to take a visual off, the Format pane writing the
  phone's own formatting (`visual.mobile_format`, `objects` and `visualContainerObjects` of
  `mobile.json`; `singleVisual.mobileObjects` / `mobileVcObjects` in a legacy layout) and
  *Clear mobile formatting*; `visual.mobile` takes a list of visuals in one step; the snapshot
  carries each visual's phone place and format; an option reads the phone layout in a narrow
  window ([desktop.md](desktop.md) "Mobile layout"). PBIR and PBIX round trips tested.
- **Checked in Desktop (2026-11-23, Power BI Desktop 2.159.1231.0, Store edition).** A project
  with a phone layout BIcraft wrote (`visual.mobile`, `visual.mobile_format`) opens, and Desktop's
  save writes each `mobile.json` back with the same content (CRLF line endings). View › Mobile
  layout (input events, the page's DOM read through the DevTools port) shows the card, the chart
  and the table at their `mobile.json` `x`, `y`, `width` and `height`, 1:1 in CSS pixels; the
  chart's title override from `mobile.json` `visualContainerObjects` shows on the phone while the
  desktop page keeps its default title. Desktop's phone canvas is 324 CSS pixels wide, scrolls to
  3600 high with 559 visible in that window, and its grid cells are 18 wide by 20 high.
- **Fixed (2026-11-24, maintainers' decision): the phone canvas is Desktop's.** It was 323
  wide (Learn's figure), at least 568 high, with an 8-point grid; it now takes the figures
  observed in Desktop 2.159.1231.0 above: 324 wide, 3600 high while the layout is edited (more
  only when visuals go further), a grid of cells 18 wide by 20 high, which the gestures snap
  to (move, resize, a drop from Page visuals, the next row of a double-click) and which
  `visual.mobile {snap: true}` applies for agents (x and width on the columns, y and height on
  the rows, at least one cell, kept inside the 324). The figures live in one place,
  `bicraft_render::phone`, read by the engine and the app. Not observed, so BIcraft's own
  choice and still open: how Desktop rounds a position onto its grid (BIcraft takes the nearest
  line), whether its canvas grows past 3600, whether it keeps a visual inside the width, and the
  559 visible pixels (that window's height, not a canvas figure; BIcraft fits about 568 points
  in the view when it picks the zoom). Positions already in a `mobile.json` are never moved by
  the change: files keep their bytes on a round trip. The narrow-window reading keeps its own
  height (at least 568, as tall as its visuals). Tests: `mobile_layout_snaps_to_desktops_phone_grid`
  (engine), `positions_go_to_the_nearest_grid_line_inside_the_phone` (render) and
  `mobile_ui_tests` (324 wide, 3600 high, drops and drags on the 18 × 20 grid), seen red before
  the change.
- **Done (2026-11-27): Auto-create, Remove all, copy, paste and duplicate on the phone.**
  `visual.mobile_auto {page}` lays out the page's shown visuals by BIcraft's own rule, not
  Desktop's: reading order of the desktop layout, stacked full width with one grid row between
  rows, the ratio kept between 120 and 480 high, two cards or slicers next in that order sharing
  a row at half width (60 to 240 high); mobile formatting kept, hidden visuals taken off.
  `visual.mobile_remove_all {page}` empties the phone layout. Both are View › Mobile layout
  buttons and canvas menu items, one undo step each. `visual.copy`'s clip carries the copied
  visuals' phone documents (`mobile`); `visual.paste` and `visual.duplicate` take `mobile:
  true` (the phone canvas's Ctrl+V, Ctrl+D and menu): the copy is a new visual of the same page
  that also goes on the phone canvas, at the copied phone place moved one grid cell while taken,
  with the copied mobile formatting ([desktop.md](desktop.md) "Mobile layout"). Tests:
  `mobile_auto_create_stacks_the_shown_visuals_inside_the_phone`,
  `mobile_remove_all_empties_the_layout_in_one_step`,
  `mobile_paste_and_duplicate_keep_the_visual_on_its_page_and_phone` (engine),
  `auto_create_and_remove_all_lay_out_and_empty_the_phone_in_one_step_each` and
  `copy_paste_and_duplicate_on_the_phone_keep_the_visual_on_its_page` (`mobile_ui_tests`), each
  seen red with the layout, removal or phone flag broken.
- **Left.** Desktop's grid rounding, whether its canvas grows past 3600 and whether it keeps a
  visual inside the width are unobserved (above); so are Desktop's own Auto-create result (BIcraft's
  rule is its own) and what its eraser does to mobile formatting. No group on the phone canvas:
  the format notes do not say how a visual group appears in a mobile layout, `visual.mobile`
  refuses a group and the phone canvas draws every visual alone (Ctrl+G says so); no cut there
  (it would take the visual off the page). The override semantics
  of the other mobile `objects` (per selector, property by property: BIcraft's reading of Learn)
  are unchecked. No filter of
  the Page visuals pane by appearance, no shown/hidden toggle from the pane; the fx dialog and
  the image's scaling stay the visual's own on the phone; the phone's `tabOrder` is not written; the narrow-window
  option goes by the report area's width only (no device or orientation detection).
- **Effort.** S–M.

### G73. Themes and high contrast left (2026-11-19)

- **Done (2026-11-19 to 2026-11-20).** The View › Themes gallery of BIcraft's eight own themes,
  Customize current theme, the high-contrast UI theme and the view-only high-contrast report
  palette; the summary is in [history](history/gaps-done.md#summary-as-of-2026-11-25).
- **Done (2026-11-27): high contrast outside the window.** `report.render {high_contrast:
  true}` and `bicraft-cli render --high-contrast` draw the pages with the app's high-contrast
  palette (View › High-contrast colours; the report file does not change; [render.md](render.md)
  "High contrast"), so MCP and the CLI have it too. The desktop reads Windows contrast themes
  once at start (the per-user high-contrast flags, `windows-registry`'s safe API,
  `bicraft_desktop::contrast`): when they are on and no theme is stored, the window starts in
  High contrast with the pages in high contrast, as the browser does with `forced-colors`
  ([design.md](design.md) §8). Tests: `high_contrast_draws_the_high_contrast_palette` (engine:
  another picture, every text run at 7:1 or more on the black page) and
  `the_on_bit_turns_high_contrast_on` (desktop).
- **Left.** The built-in themes have not been opened in Desktop. The report palette is BIcraft's
  colour mapping (close hues share a colour, images unchanged), not compared with Power BI's
  high-contrast rendering. The desktop does not follow a contrast change while it runs, and on
  Linux it has no detection (winit and eframe expose light and dark only; a desktop
  environment's contrast preference, such as the XDG portal's, is not read). `report.export_pdf`
  has no high-contrast option. The HTML viewer (`bicraft-cli view`, page.html) has none either: it
  draws the visuals in its own script with colours chosen in dozens of places, so the palette
  would mean porting the mapping and the text-backdrop rule there, for a page slated for removal
  ([webui.md](webui.md)); the egui app in the browser (`bicraft-cli view --ui egui`) has the theme
  and follows the system's high contrast.
- **Effort.** S.

### G74. Web viewer regression checks left (2026-11-19)

- **Done.** `cargo xtask webcheck record|check` ([webcheck.md](webcheck.md)): `bicraft-cli view` on a
  completed copy of our synthetic `visuals` fixture, seven walkthroughs of the HTML viewer
  (opening, page tabs, a slicer, a data point, a drill, a bookmark, the page's own tabs) in Craft
  Anything's headless browser, screenshots compared with baselines kept outside the repository,
  a pass/fail table and diff images. Not part of `cargo xtask ci`; skipped without the tool.
- **Left.** The egui app (`bicraft-cli view --ui egui`, `/app/`): it needs the wasm bundle embedded,
  and its controls are drawn on a canvas, so Craft Anything's states capture finds nothing to
  click; its gestures would go through the control channel (`--control`) instead. Real clicks
  on the page's visuals: `web_walkthrough_record` reads the page (Deep mode), which is never
  used, so slicer, point, drill and bookmark gestures are sent as the page sends them
  (`POST /api/interact`) and only the click handlers of the tabs are exercised
  (`web_page_states_capture`, which found one tab to open in the proof run). No export
  walkthrough: `bicraft-cli view` has no export (the download link is `bicraft-cli serve`'s). Baselines
  depend on the machine (the browser's language and colour scheme come from the system; the
  screenshot is the browser's own window size, 750 x 485 in the proof run, not the 1280 x 800
  asked). No run on Linux yet.

### G75. Groups and bins left (2026-11-19)

- **Done.** `column.group` adds a list group (`SWITCH(TRUE, …)` with `(Blank)` and `Other`, or
  the ungrouped values kept) or bins (numbers by size or into a number of bins, dates by years,
  months, days, hours, minutes or seconds) as a calculated column with the `GroupingMetadata` extended property, in one
  undo step; `replace` edits a group; the snapshot reads a group's definition back (`group`).
  The evaluator now computes calculated columns that the data does not hold (literal
  partitions, or added after the data was stored), so groups work in queries and visuals before
  any save. The Groups dialog opens from the Data pane's column menu (New group), Column tools ›
  Data groups and the Model view's Table tab. Round trips through PBIP (TMDL) and PBIX tested.
  Note: [formats/groups.md](formats/groups.md).
- **Checked in Desktop (2026-11-23, 2.159.1231.0 Store).** Desktop computes and lists the groups
  and bins BIcraft writes. Its Edit groups opened a BIcraft list group without its groups: it
  reads them from the `GroupingDesignState` annotation, whose shape Desktop wrote for its own
  group; `column.group` now writes it for list groups ([formats/groups.md](formats/groups.md)
  "Desktop answers"). Desktop's SWITCH and `GroupingMetadata` match BIcraft's.
- **Done (2026-11-23, later).** Bins take the form Desktop 2.159.1231.0 saved for bins made in
  its own dialog: its DAX (numbers toward zero, a number of bins computed from the column's
  minimum and maximum so it follows a refresh, months restarting each year, days from day 1),
  its `binningMetadata` unit codes (4 year, 6 month, 8 day, 9 hour, 10 minute, 11 second; none
  for a number of bins), the format strings and a `GroupingDesignState` with a `BinItem`
  (`Floor` with the public `TimeUnit`, or `Discretize`); dates bin by hours, minutes and seconds
  too. Desktop's Edit groups then opened BIcraft's number, count, month and hour bins and its list
  group with their definitions, without error ([formats/groups.md](formats/groups.md)
  "Re-check"). The evaluator stores numbers as dates in a date/time calculated column.
- **Done (2026-11-27), without Desktop.** A blank listed in a list group is an explicit member:
  the design state lists the `semanticQuery` null literal in the group's `In` values and leaves
  out the `(Blank)` placeholder; it is written, saved to PBIP and read back. A number of bins on
  dates (`bin_count` on a date column) is written as a bin size in the date form, the data's span
  in the unit divided by the count and rounded up (without a unit, the largest the span holds
  that many times). Group data (`visual.group_points`, the menu of a data point) makes a list
  group of the category values of the points selected in a visual (Ctrl+click), else of the
  point right-clicked, and adds to an existing group column of the same column. All three are
  BIcraft's own forms, **not compared with Desktop** (not seen in a Desktop file)
  ([formats/groups.md](formats/groups.md) "BIcraft's own forms").
- **Left.** Check the three forms above in Desktop (whether its Edit groups opens a group with a
  blank member; whether Desktop offers a number of bins on dates and in which form). Group data
  does not add the group to the visual's Legend as Desktop does (Microsoft Learn). A date bin
  made from a count keeps its size, it does not follow a refresh. Number, date and boolean values
  in a list group's design state follow the public literal forms, not a file Desktop saved. A
  structural PBIX save of a day or time bin (numbers in a date/time column) is not tested. The
  Data pane's menu says New group on a group column too (the dialog opens as Edit groups).
- **Effort.** S.

### G76. Quick measures left (2026-11-19)

- **Done.** `measure.quick {kind, params, table?, name?}` writes an ordinary measure from
  BIcraft's own DAX template for 25 calculations (aggregates per category, filtered value and
  its differences, year-, quarter- and month-to-date, year-over-year, quarter-over-quarter and
  month-over-month change, rolling average, running total, totals for a category filtered or
  not, addition, subtraction, multiplication, division, percentage difference, star rating,
  concatenated list), validated by the DAX binder, in one undo step. The Quick measures dialog
  opens from Home › Quick measure and the Data pane's New quick measure (a table's menu, or a
  field's, which starts as the base value); a filter's values are ticked from the column's
  values. Evaluator tests on a small synthetic model with a date table. Note:
  [formats/quick-measures.md](formats/quick-measures.md).
- **Done (2026-11-24).** *Sales from new customers* (base, customer, date: the customers with
  nothing bought before the context's first date, whatever the other filters) and *Correlation
  coefficient* (Pearson's over a category, both values not blank), as Learn names them (Filters,
  Mathematical operations); `year_end_month` gives year-to-date a fiscal year end (`"6/30"`);
  time intelligence on a column with auto date/time walks its date table's dates
  (`T[C].[Date]`), and on a column that is neither that nor a marked date table's the result
  warns. Evaluator tests: `sales_from_new_customers`, `correlation_coefficient`,
  `fiscal_year_to_date`, `time_intelligence_on_a_fact_date_column`.
- **Left.** The rolling average counted on a calendar of every date when the date column has
  gaps. The templates are BIcraft's own and are not compared with the DAX or the results of a
  quick measure Desktop wrote (Learn gives neither the inputs nor the DAX of the two new ones).
  **Not relaxed, by design:** on a date column with gaps and neither auto date/time nor a marked
  date table, time intelligence still sees only the dates present: a template cannot tell a
  day's context from a month's or a year's when only the dates holding rows are known, so
  shifting them (`DATEADD`) misses dates without rows and snapping to whole periods would change
  results at finer grains; the fix is in the model (a date table holding every date, marked).
- **Effort.** S.

### G80. Custom visuals left (2026-11-19)

- **Done.** `visual.import_custom {path | data}` reads a `.pbiviz` as hostile input (32 MB at
  most, 256 entries, capped decompressed sizes and expansion ratio, plain names only, the guid
  checked; [formats/pbiviz.md](formats/pbiviz.md)) and stores it as Desktop stores an embedded
  visual: `CustomVisuals/<guid>/package.json` and `resources/<guid>.pbiviz.json` with a
  `CustomVisual` resource package in `report.json`; the same guid replaces it.
  `visual.remove_custom {name, force?}` removes it (refused while visuals use it, unless
  `force`). `report.custom_visuals` lists it; the snapshot carries `report.custom_visuals` and a
  visual's `custom_visual` with the package's icon; the Visualizations pane shows a cell per
  custom visual with its icon (a click adds or switches to that type) and Desktop's "…" menu
  (Import a visual from a file, Remove a custom visual), Insert › Import a visual too; the
  desktop reads the file through its dialog, the browser through a file upload. PBIP and PBIX
  round trips tested; a fuzz smoke test and a cargo-fuzz target (`pbiviz_read`) cover the reader.
- **Done (2026-11-20, M11.30): custom visual projects without Node.** `visual.scaffold` writes a
  project (`pbiviz.json`, `capabilities.json` with the data roles and format options of its form,
  a TypeScript visual with `getFormattingModel`, CSS, an icon we draw); `visual.build` compiles its
  TypeScript and bundles it with swc (`crates/tsbuild`, Apache-2.0 crates from crates.io) with a
  module registering the class under the guid; `visual.pack` writes the `.pbiviz`;
  `bicraft-cli visual new|build|pack`; Insert › Visuals › New visual project, Compile a visual, Package a
  visual. A packaged visual imports back through `visual.import_custom` and survives PBIP and PBIX
  round trips (engine tests); [formats/pbiviz.md](formats/pbiviz.md) "Visual projects".
- **Seen in Desktop (2026-11-23).** A `visual.pack` package embedded by `visual.import_custom`
  in a PBIX is drawn by Desktop 2.159.1231.0 with its engine's data, now that `visual.build`
  names the bundle after the guid (`var <guid> = …`, `default` = the plugin). Before that fix,
  the visual was registered but stayed empty. WebAssembly runs in the sandbox (M11.31).
  [formats/pbiviz.md](formats/pbiviz.md) "Seen in Desktop".
- **Left (projects).** Desktop's own import dialog given a `visual.pack` package, and its format
  pane cards, are not checked. The build strips types without checking them
  (no TypeScript type checker in Rust); npm packages are refused (no `node_modules`, the API
  package only for its types), as are `.tsx` (JSX), LESS styles, import cycles and dynamic
  `import()`; no source maps, minification, `stringResources` or `externalJS`; `package.json`
  has no `build` field. swc's crates are pinned to the newest that built with the workspace MSRV
  then (1.90); the browser app does not build them (the engine is native only).
- **Out of scope: AppSource (Get more visuals).** Desktop fetches AppSource visuals from
  Microsoft's store: it needs the network, a signed-in Microsoft service, its terms and its
  catalog. BIcraft never connects to Microsoft services at run time (AGENTS.md §4), so it does
  not build it. A report's `publicCustomVisuals` references are listed and kept, never fetched;
  a visual of that type is drawn as a labelled placeholder.
- **Left.** **A custom visual's JavaScript is never run**: its visuals are drawn as a placeholder
  with the package's icon and name, not as the visual (a sandboxed JavaScript host would be an
  XL project of its own, docs/target-app-parity.md). Its data roles are not offered as wells in
  the Build pane (the visual takes no fields there). A legacy layout (`Report/Layout`) is
  refused: its resource packages are not written. The browser uploads at most 5 MB (the bytes
  travel in base64 in one MCP request, whose body is capped at 8 MB). Not checked in Desktop:
  Desktop's own import dialog (whether it asks before replacing a visual of the same guid), a
  package whose `.pbiviz.json` is not named `<guid>.pbiviz.json`, a BIcraft-imported package
  opened in Desktop (only the layout of §11 of pbix-package.md, built by hand, was).
- **Effort.** — (AppSource: out of scope; the data roles' wells: S; the Desktop check of a
  packaged visual: S; npm packages and LESS: L).

### G81. DAX query view left (2026-11-20)

- **Done.** The app's DAX query view (`crates/ui/src/dax_query.rs`), from the status bar's view
  icons and Analyze › Run DAX query, as Microsoft Learn's "DAX query view" page describes
  Desktop's: query tabs along the bottom (new, rename by a double click or the tab's menu,
  duplicate, close; the last one closed leaves an empty tab), kept per document as view state;
  the DAX editor with the formula bar's colours; Run, Ctrl+Enter and F5 run the tab's query
  through `dax.evaluate`; the results grid (sortable, Copy as TSV) with the row count, a picker
  when the query has several EVALUATEs, and errors inline with an icon and the word Error. Each
  `DEFINE MEASURE Table[Name] = …` gets a lens, "Update model: Add new measure" or "Update model:
  Overwrite measure", and the tool bar's "Update model with changes" applies them all, through
  the new command `dax.apply_define {query, measures?, force?}` (`crates/engine/src/dax_define.rs`:
  the query parsed by `bicraft-dax`, the measures added or their expression replaced in one undo
  step, validated with every change made; an unknown table, a measure of another table, a query
  that does not parse or invalid DAX change nothing). The ribbon's Run DAX query and Update model
  open the view and act on its tab. PBIP (TMDL) and PBIX round trips tested.
- **Done (2026-11-23).** The tabs are the open file's: `file.open` reads a PBIP project's
  `<Name>.SemanticModel/DAXQueries/*.dax` and `.pbi/daxQueries.json` (tab order, tab shown) as
  Microsoft Learn's "Power BI Desktop project semantic model folder" describes them
  ([formats/dax-queries.md](formats/dax-queries.md); `crates/pbip/src/dax_queries.rs`); the view
  loads them from the snapshot's `dax_queries` and hands them back with the new command
  `dax.set_queries` 1.5 s after a change and before every save (`dirty.queries`, the tab's
  unsaved dot); `file.save` writes them (text bytes as typed, closed tabs' files removed,
  byte-identical on Windows and Linux), `file.save_as` / `project.new` into the new project
  (`crates/engine/src/dax_queries.rs`). Format query: `dax.format` (`crates/dax/src/format.rs`,
  our own layout; only white space changes, checked token for token and idempotent), a tool bar
  button and the command palette. Cancel: `dax.cancel` moves the session's cancel epoch, which
  the evaluator checks with its time budget, so every `dax.evaluate` started before it stops
  ("The query was cancelled."); read-only, so a server runs it beside the query; the desktop
  app's source also raises the session's cancel flag as the command is sent. Squiggles: the text
  is checked with `dax.validate_query` 0.35 s after typing stops, the first error underlined and
  told above the results (line, column, message), as the formula bar does.
- **Done (2026-11-27).** The view's editing tools (`crates/ui/src/dax_query_tools.rs`), pure
  view state like typing: **Comment** (the tool bar's button, Ctrl+/) puts `// ` before the
  lines the selection touches at their common indent, or removes the line comments (`//` or
  `--`) when every non-blank one has one, the selection kept on the lines; **Find and replace**
  (the tool bar's button, Ctrl+F, Ctrl+H for the replace field): a row above the editor with the
  match count, previous and next (Shift+Enter, Enter), Match case, Whole word, Replace, Replace
  all, Esc to close, the matches marked in the editor (background and underline) and the current
  one scrolled into view; a **Data pane** at the right of the view (the tool bar's Data pane
  button hides it; a search box): the model's tables with their columns and measures, hidden
  ones dimmed, Right / Left unroll and roll a table; a click or Enter on a column or a measure
  inserts `'Table'[Column]` or `[Measure]` at the editor's cursor; the right-click menu
  (Shift+F10) of a table, a column or a measure writes a **quick query** into a new tab and runs
  it with `dax.evaluate`: `TOPN(100, 'Table')`, `DISTINCT('Table'[Column]) ORDER BY` it,
  `ROW("Measure", [Measure])`, or inserts the reference. The shortcuts are in Shift+?'s list
  (none clashes: the formula bar's Ctrl+H is not shown in this view, the registry declares
  neither). EN/FR; the controls are named in the accessibility tree. Tests:
  `dax_query_ui_tests` (comment, find and replace, Data pane, quick queries, French) and
  `dax_query_tools::tests`, each seen failing with its feature disabled.
- **Done (2026-11-27, later).** **Define quick queries**: the new read-only command
  `dax.define_query {table?, measure?, references?}` (`crates/engine/src/dax_define.rs`) writes
  a `DEFINE` block, one `MEASURE 'Table'[Name] = expression` per measure as the model has it,
  and `EVALUATE ROW("Name", [Name], …)` of the measures asked for; with `references`, every
  measure they reference, transitively, after the binder's dependency graph
  (`bicraft_dax::deps`), each before the measures that use it (an iterative walk, a cycle walked
  once). The Data pane's menu of a table has **Define all measures**, a measure's **Define with
  references**; the query goes to a new tab and runs (the palette's entry opens the view; from
  the control channel with `table` or `measure` it does the same). `dax.evaluate` now accepts a
  query measure whose text is the model's own (nothing to replace), so these queries evaluate
  where a model measure uses one of them. The Data pane lists **hierarchies** (Right / Left
  unroll them; a level inserts its column) and a **calculation group's items** (an item inserts
  `'Group'[Column] = "Item"`, the group's first visible column). **Drag**: every field row
  (column, measure, level, item) drags into the editor, a caret shows where it lands and the
  reference is inserted at the drop point (a click or Enter still inserts at the cursor). The
  **lenses** sit on their own line above each DEFINE MEASURE, from MEASURE's column: the line's
  first blank is laid out taller (`dax_query_tools::lift`), so the text drops below the room
  the lens takes; a line without a blank (`DEFINE MEASURE…` at column 0 with no space) keeps
  its lens at the right end. EN/FR. Tests: engine
  `define_query_writes_a_tables_measures_or_a_measure_with_its_references` (the query validates,
  evaluates and applies back unchanged) and `define_query_with_references_walks_a_cycle_once`;
  kittests `define_quick_queries_write_the_measures_with_their_references_and_run`,
  `the_data_pane_lists_hierarchies_and_calculation_items`,
  `a_field_dragged_into_the_editor_lands_where_it_is_dropped`,
  `a_lens_sits_on_its_own_line_above_its_measure`; unit test
  `a_lifted_line_keeps_its_text_and_drops_below_the_room_for_its_lens`.
- **Left.** A PBIX or PBIT keeps the tabs for the session only (a save warns): no public
  description of where a package holds them, so it needs a Desktop observation (save a PBIX with
  query tabs, read its parts) before BIcraft writes anything there; a thin report's
  `<Name>.Report/DAXQueries/` is not read; from the desktop app connected to a remote server,
  Cancel waits behind the running query (that source sends one request at a time; closing the
  request is what stops a server query there); the Data pane has no Model tab; a query that
  redefines a model measure with another text, where another model measure uses it, is still
  refused by the evaluator; Ctrl+F not checked in a browser, which may keep it for its own find;
  the text cursor on a lens line is as tall as the line and its lens. The Define queries' text is
  BIcraft's own, not compared with Desktop's. Desktop's own tab-name rules are not known.
- **Checked in Desktop (2026-11-23, 2.159.1231.0 Store).** A project BIcraft wrote opens with its
  tabs in order and the default tab shown; Desktop's save keeps the unchanged `.dax` files' bytes
  and rewrites `daxQueries.json` in the same shape with CRLF line endings
  ([formats/dax-queries.md](formats/dax-queries.md) "Desktop answers").
- **Effort.** S (the Model tab), M (PBIX storage, after a Desktop observation).

### G84. App keyboard navigation left (2026-11-20)

- **Done.** `crates/ui/src/keyboard.rs`, after Microsoft Learn's "Keyboard shortcuts in Power BI
  Desktop": Ctrl+F6 / Ctrl+Shift+F6 move between the window's sections (ribbon rows, formula bar,
  the view, each side pane, page tabs, status bar), F6 activates the Selection pane, Tab and
  Shift+Tab wrap inside a section, Right / Left unroll and roll the Data pane's tables, folders and
  hierarchies; every window is a modal dialog for the keyboard and the pointer (Tab trapped,
  Escape closes, the focus returns to the opener; first control focused when opened from the
  keyboard); a 2 px focus ring in the `focus` token (4.5:1 on every surface, light, dark and high
  contrast); the accessible names of every control in the views and the Manage relationships,
  Manage security roles and Customize theme dialogs, checked by a walk of the AccessKit tree
  (the Data and Filters panes' search boxes were unnamed). Since 2026-11-23: Alt (alone) or F10
  key tips on the ribbon, letters generated from the registry per tab and language, unique and
  prefix-free within a level, each running its button's command id (`crates/ui/src/key_tips.rs`);
  Shift+? lists the registry's shortcuts and the navigation keys, searchable, EN/FR
  (`crates/ui/src/shortcuts_list.rs`); Shift+F10 opens the focused control's right-click menu (the
  selected visual's on the report page), first item focused, focus back on close; the Model
  view's Properties and Data panes are sections of their own. Since 2026-11-27
  ([desktop.md](desktop.md) "Keyboard navigation and screen readers"): the Menu key opens the
  same menus as Shift+F10 (egui 0.37 reads it as `Key::Menu`, no shell mapping needed); the
  canvas's visuals take egui's focus one by one (`crates/ui/src/visual_focus.rs`), Tab and
  Shift+Tab in the report's tab order then reading order, the focused visual selected so the
  arrows move it, Ctrl+Right enters its points, Delete removes it, Escape returns to the page,
  each a figure in the AccessKit tree named after its title, selected, with its place in the
  order and read-only when the session cannot move it; Alt+Shift+1 collapses every table of the
  Data pane and Alt+Shift+9 expands them all (Desktop's keys; the Report and Model views' Data
  panes, listed by Shift+?); the status bar's message is a polite live region that names the
  section Ctrl+F6 or F6 moves to and says when the tables were collapsed or expanded.
- **Left.** Alt+Shift+1 / 9 do nothing in the DAX query view's Data pane (its own tree).
  Shift+F10 and the Menu key open nothing on a
  control without a right-click menu (the Selection pane's rows have none). Key tips are
  painted, not in the AccessKit tree. The name test covers the views and three dialogs, not
  every dialog. Not run with Narrator, NVDA or JAWS (only the AccessKit tree egui builds was
  read, including the live region); F6, Ctrl+F6, Alt, F10, the Menu key and Alt+Shift+digits
  not checked in a browser, which may keep them, nor in the running desktop window (whether
  Windows' Alt / F10 window-menu handling comes first, whether Alt+Shift switches the input
  language, whether winit delivers the Menu key on this keyboard).
- **Effort.** S (screen-reader and browser runs).

### G92. Map visuals left (2026-11-20)

- **Done.** [formats/maps.md](formats/maps.md): the four map types take their wells and format
  cards; `shape_map.set_map` stores a custom TopoJSON or GeoJSON map as a `ShapeMap` item of
  `RegisteredResources` (Format › Map settings › Add a map type in the app); the renderer
  (`crates/render/src/maps.rs`, PNG, PDF and the app) draws bubbles at their latitude and
  longitude on a Web Mercator base map of Natural Earth's 1:110m country outlines (public domain,
  bundled, 41 KB; drawing only) and a shape map's custom areas in its projection, coloured by
  Legend or Color saturation with a legend and tooltips (Zoom › Auto zoom fits the areas with
  data); a click on a bubble or an area selects it and cross-filters the page. Locations that
  are country names (English, French, long, formal) or ISO alpha-2 / alpha-3 codes are placed
  offline: bubbles at the country, or a filled map's countries filled. `shape_map.set_map
  {builtin: "world_countries"}` (Format › Map settings › World countries (Natural Earth)) writes
  Natural Earth's countries into the report as an ordinary custom TopoJSON `ShapeMap` resource.
  2026-11-23, Power BI Desktop 2.159.1231.0: a shape map with the World countries map opens,
  draws the matched countries and saves the `shape` object and the TopoJSON back unchanged
  (the property shape `shape.map.geoJson` and the file are confirmed); Desktop fills every
  matched area with one colour when only Color saturation is bound, so BIcraft no longer draws a
  gradient (regression test `shape_maps_colour_matched_areas_alike_and_select_them`).
- **Fixed (2026-11-24, by the maintainers' decision: an open basemap instead of Bing).** Maps,
  filled maps and Azure Maps draw on a free, open vector basemap
  ([formats/maps.md](formats/maps.md) "The open basemap's detailed layers"): under 90° of
  longitude, Natural Earth's 1:50m countries and lakes; under 70°, also the admin-1 boundaries
  of the nine countries Natural Earth splits at that scale and city names (never over a data
  bubble), all in theme colours, light and dark, marked "Made with Natural Earth"
  (`crates/render/geo/ne_50m_basemap.bin`, 329,555 bytes, public domain, made by
  `cargo xtask geo-bundle --basemap` from SHA-256-checked sources). Locations naming one of
  Natural Earth's 1,251 populated places (optionally `, country` or `, state`) are placed
  offline. Golden pages `maps.png` and `maps-dark.png` (bubbles on five continents, cities,
  a filled map), tests `geo` (+2), `maps_tests` (+1, Lake Geneva drawn as water: seen red at
  1:110m). Same renderer for PNG, PDF, the app and the browser app.
- **Left.** Places other than countries and Natural Earth's populated places (smaller towns,
  regions, postcodes) without latitude and longitude need geocoding by Bing or Azure Maps (a
  placeholder says so); a city name shared by several places takes the most populous unless
  qualified; a filled map's colour saturation by conditional formatting (Fill colours fx) is not
  applied; filled maps fill countries only (no states or provinces); Desktop's built-in shape
  maps (USA, Canada, …) are Microsoft's map data and are not bundled; maps from a URL are not
  fetched; no online map tiles (an opt-in tile server was left out: a synchronous renderer below
  the network layers, no synchronous fetch in the browser, PDF embedding third-party rasters,
  and OpenStreetMap's tile policy needing a cache and per-server opt-in; formats/maps.md);
  the detail stops at 1:50m (no streets, no 1:10m coasts). page.html draws a labelled box (web viewer). A custom
  map in a legacy-layout PBIX is refused on save: the legacy item type code of `ShapeMap` is not
  observed. The shading Colors › Color saturation sets (what Desktop writes, how it shades) is
  not drawn. Map and filled map were not compared with Desktop: its security option turning
  them off was left unchanged (it sends locations to Bing). Data labels on maps are BIcraft's
  reading; heat map, clustering, map styles and the other zoom options are kept but not drawn.
- **Effort.** M (page.html, a Desktop check of the property shape and the legacy code with the
  packaging oracle); L (geocoding would need the online service: out of scope offline).

### G99. Gantt visual (2026-11-20)

- **Done.** [formats/gantt.md](formats/gantt.md): the published report theme schema 2.159
  (vendored, replacing 2.157) adds `ganttChart`; BIcraft accepts it as a built-in type
  (`visual.add`, `report.apply`, themes), checks `visual.format` against its objects (`bar`,
  `milestone`, `progress`, `row`, `axis`, the column objects) and gives the Format pane Bars,
  Milestones, Progress, Rows and Axis cards. Files keep a Gantt visual byte for byte (PBIP and
  PBIR PBIX round trips tested). 2026-11-23: a wells test against Desktop 2.159.1231.0
  (gantt-A/B/C/R, docs/formats/visual-roles-bb.md) read the roles `Task` (grouping, 1),
  `StartDate` and `EndDate` (a date column summarized as earliest / latest, or a date measure;
  1 each), `Progress` (measure, 1), `Category` (grouping, up to 3), `Columns` and `Tooltips`
  (measures); no other well takes a field before `Task`. `crates/visuals/src/roles_bb.rs` holds
  them: the Build pane shows the wells, the gallery offers the Gantt, `visual.bind` checks them.
- **Done (2026-11-23, drawing).** The engine evaluates a Gantt one row per task (earliest start
  first without a saved sort, at most 1,000 rows); `format.gantt`, `gantt_rows` and
  `gantt_widths` carry its formatting, per-member collapse and column widths;
  `crates/render/src/gantt.rs` draws the task list and detail columns, category rows (70/80/90 %
  tints, chevron, plus/minus or caret, collapsed state read), bars, milestones (four shapes),
  progress, an auto-scaled (days, months, years) or two-level time axis with gridlines and
  shaded week days, top, bottom or both, inverted or not; a click selects a task or a category
  for cross-filtering, hover shows its fields. Tests:
  `crates/engine/tests/report/gantt_tests.rs`, `crates/render/src/gantt_tests.rs`, the `gantt`
  golden page.
- **Done (2026-11-27, reading).** A category's expand/collapse icon toggles it in the reader's
  view (`view.interact` `gantt_toggle`, listed last in `format.gantt_rows`; the report not
  changed, `reset` drops it). Rows that do not fit scroll (a scroll bar on the right of the time
  line) and the time line zooms (1 to 512 times, auto-scale picking the unit again, a window bar
  under the rows): both are the app's view state (`RenderOptions::gantt`, never in the file or
  the snapshot). The app lays labelled controls over them (`crates/ui/src/gantt_ui.rs`):
  Expand / Collapse buttons, the scroll bar (arrows, Page Up / Down, Home, End, track click,
  drag), the time line's zoom slider (+, -, left and right arrows, Home), the window bar, the
  wheel, horizontal scroll and Ctrl+wheel. Tests: render `gantt_tests` (scrolling, zoom, the
  icon's action), engine `the_readers_expand_collapse_hides_and_shows_a_categorys_tasks_in_the_view_only`,
  kittest `gantt_ui_tests` (4), each seen failing without its change.
- **Left.** No today line. Sizes, colours and label formats are BIcraft's choices, not compared
  with Desktop's drawing; Desktop's zoom slider (`zoomShow`, `zoomMin`, `zoomMax`) is not read;
  how Desktop saves a per-member collapse and `shadingDays` in PBIR was not observed, so the
  reader's collapse is not saved. page.html does not draw the Gantt. The date-only restriction of
  Start and End is not checked (BIcraft's role kinds have no data type). The Format pane does not
  offer the schema's enumerations (milestone shapes, date units) as choices.
- **Effort.** S (a Desktop comparison: drawing, zoom slider, saved collapse).

### G97. DAX user-defined functions left (2026-11-20)

- **Done.** [formats/dax-udf.md](formats/dax-udf.md): `function.add`, `function.set`,
  `function.rename` and `function.remove` (`crates/engine/src/udf.rs`), each one undo step;
  definitions parsed and validated by `bicraft_dax::udf` as `DEFINE FUNCTION` against the model
  (names, types, subtypes, pass modes, defaults, recursion); call sites checked by the binder
  (arity, scalar or table arguments); measures and `dax.evaluate` queries (with `DEFINE
  FUNCTION`) that call one are evaluated by inline expansion (val arguments bound once, expr
  arguments substituted); compatibility level raised to 1702; TMDL, TMSL and PBIX round trips
  tested. The Model explorer lists Functions with New, Edit (the DAX editor form), Rename,
  Delete and Script TMDL.
- **Left.** Report-level measures do not follow `function.rename`; a function whose body stops
  validating after a model change is not listed by `model.validate`; the DAX query view's
  Update model does not save a `DEFINE FUNCTION` (use `function.add` or the TMDL view); no
  Quick queries on a function; the argument-kind checks cover scalar versus table only (not
  `ColumnRef`/`MeasureRef` targets). Not checked in Desktop.
- **Effort.** S (rename in report measures, `model.validate`, `dax.apply_define`), M (the
  reference kinds).

### G98. TMDL view left (2026-11-20)

- **Done.** [formats/tmdl-view.md](formats/tmdl-view.md): `tmdl.script {objects}` and
  `tmdl.apply {script}` (`crates/tmdl/src/script.rs`, `crates/engine/src/tmdl_script.rs`):
  createOrReplace of tables, members under `ref table`, functions, expressions, relationships,
  roles, perspectives, cultures, data sources, query groups and the whole model, one undo step,
  DAX validated before anything changes, errors as `line L, column C`. The app's TMDL view
  (`crates/ui/src/tmdl_view.rs`): script tabs, Apply (Ctrl+Enter), Script model, Copy, Go to the
  error; Script TMDL on the Data pane's and the Model explorer's menus fills the empty tab or
  opens a new one. PBIP and PBIX saves of an applied script tested.
- **Left.** No Preview (the diff before Apply), Problems pane or syntax colours; the script tabs
  are not saved in a PBIP project's `TMDLScripts` folder; no compatibility-level upgrade prompt
  (functions raise the level silently); `kpi`, calculation items and other nested objects
  outside a `ref table` member list cannot be applied alone; a rename through a script is not
  detected (the old object stays). Not checked in Desktop.
- **Effort.** S (save the tabs, the Problems pane), M (Preview diff, syntax colours).

### G100. Excel Power Pivot import left (2026-11-20)

- **Done.** [formats/powerpivot.md](formats/powerpivot.md) (M11.33): `file.open` of an `.xlsx` /
  `.xlsm` reads `xl/model/item.data` (bare or XPress9-wrapped ABF, `ErrorCode` CRC markers,
  `ApplyCompression` Plain LZ77 per file with `crates/xpress8`), the ASSL metadata of
  compatibility levels 1100/1103 (tables, columns, calculated columns, hierarchies,
  relationships, measures) and every column's values with the VertiPaq decoder, and gives a
  new Power BI model held in memory, saved with `file.save_as` / `file.convert` as PBIX, PBIP,
  PBIT or model.bim (`file.save` never writes the workbook). The Open dialog offers Excel
  workbooks.
- **Done 2026-11-23.** A workbook Excel 16.0.20625 saved from our own synthetic worksheet tables
  (three tables, two relationships, three measures with formats and a description; built
  through Excel's COM object model by `crates/powerpivot/tests/fixtures/excel/make-model.ps1`,
  its computer name scrubbed by the `scrub_workbook` example) is a fixture with its
  `SOURCE.txt`. Reading it found and fixed: the ABF reader named `<db>.0.db.xml` and the
  measure group / partition files `.xml` (a file whose name starts with its group folder's
  name was taken as inside it), which hid the compatibility level and the database name;
  measure format strings, descriptions, display folders and hidden flags are now read from the
  MDX script's `CalculationProperties`; the CRC end marker has a final XOR (CRC-32/BZIP2), so
  the synthetic writer's markers were wrong (fixture re-blessed). BIcraft's DAX results on the
  imported model, and after a save as PBIX, equal Excel's engine on the same queries.
- **Left.** Calculated columns, hierarchies, hidden columns and measures, display folders and
  KPIs are checked on synthetic workbooks only: Excel's object model cannot create them (the
  Power Pivot window can, on the user's screen). Not supported: backups holding
  `metadata.sqlitedb` (none seen: Excel 16.0.20625 still writes ASSL 1103), the workbook's
  connections (each table's M keeps the source query as a comment and raises an error on
  refresh; a worksheet-linked table has no query at all), KPIs, the model's culture,
  perspectives, and the sheet content. The CRC markers are dropped, not verified. BIcraft's
  synthetic workbook opens in Excel with an empty data model (no `x15:dataModel` extension nor
  connections), which is fine for a reader fixture. The parity row stays P for the connections
  and the objects not yet seen in an Excel-saved file.
- **Effort.** S for a second Excel-saved fixture (calculated column, hierarchy, hidden objects,
  through the Power Pivot window); M for connections and `metadata.sqlitedb`.
### G103. More readers left (2026-11-20)

- **Done.** [formats/import-sources.md](formats/import-sources.md): `file.open` reads a `.vpax`
  (its `Model.bim`, else `DaxModel.json`, plus the VertiPaq Analyzer statistics in the result),
  a `.pbids` (one query per connection), a `.pq`/`.m` section document or single expression, a
  `.pqt` template, a Dataflow Gen2 definition (`mashup.pq` with `queryMetadata.json`) and a CDM
  `model.json` (entities, typed columns, relationships) as a new, unsaved model;
  `objects.import` takes the same files as sources. Every reader is capped (file size, part
  sizes, objects, nesting) and fuzzed (`import_readers`); XML parts are never parsed.
- **Left.** Read from public descriptions only: no file exported by the service, Desktop or
  VertiPaq Analyzer was read, so the `.pqt` metadata part name and a real Dataflow export are
  unverified. Tables read from a query have no columns until the query runs (`table.import`
  with `replace: true`, or a refresh in Desktop); there is no Navigator after a `.pbids`; its
  dataflow and other protocols are refused. A VPAX opens editable, not read-only, and its
  statistics are not in `model.storage_stats`. The app's kittest
  `open_a_pbids_file_gives_a_new_model` still passes with the reader switched off (it only finds
  the file name in the snapshot): it should check the source's table.
- **Effort.** S per kind once a real export is at hand.

### G104. Redact secrets left (2026-11-20)

- **Done.** `file.redact {path, out, data?}`, `bicraft-cli redact` and File › Export › Redact a copy
  for sharing write a PBIX, PBIT or PBIP copy whose connection strings, credentials, servers,
  databases, paths and URLs (also URLs inside report text) are stable placeholders, and with
  `data` the stored text values too. The copy is reopened and every written byte (each part,
  the decompressed DataModel, every project file; UTF-8 and UTF-16LE) is scanned: a copy where
  a collected value survives is deleted and the command fails.
- **Left.** Navigation literals that are not source arguments (`Source{[Name = "db"]}`) are
  replaced only when the same value was collected elsewhere; with `data`, numbers and dates are
  kept, calculated tables keep their stored text and M literal data (Enter data) is not
  replaced; paths inside a sentence are not detected (URLs are); a PBIP copy drops its `.pbi`
  cache. Not tried on a Desktop-saved file.
- **Effort.** S.

### G105. Spec visual left (2026-11-23)

- **Done.** `visual.set_spec {page, visual, spec}` checks a declarative chart spec (marks, encodings
  from the `category` and `values` roles, band, linear and time scales, axes, legend; capped,
  every error with its place), embeds the BIcraft Spec custom visual (its TypeScript compiled by
  `bicraft_tsbuild`) and stores the spec in the visual's `spec.json` property;
  `visual.spec_template` gives starting specs; `visual.scaffold {template: "spec"}` writes the
  visual's sources. `report.render`, PDF export and the app draw the spec with the renderer's own
  layout, which golden tests hold equal to the TypeScript's (run with Node where it is installed).
- **Seen in Desktop.** Power BI Desktop 2.159.1231.0 drew version 1.0.0.0 like `report.render`
  (a combo and a `Date` category on a time scale, 2026-11-23); 1.0.1.0 fixes the legend overlap and
  the cut tick label seen there (reopened: fixed), and the format pane shows the spec's text area.
- **Spec edited in Desktop (2026-11-23, same version).** The line's colour changed in the Format
  pane's Spec (JSON) text area (input events): the visual redraws with the new colour; Desktop's
  save stores the spec in `visual.json` as the same literal form BIcraft writes (the diff is line
  endings only), and BIcraft renders the saved project with the new colour. That save also wrote
  the `CustomVisualMetadata` item's `path` as its `name`: BIcraft had named the item by the guid,
  so the path became `<guid>` and the visual was not registered when the file was reopened.
  `visual.import_custom` now names the item `<guid>.pbiviz.json` like its path, and the Spec
  visual is reinstalled when its item has the old name; a project written that way kept the path
  through Desktop's save (`imports_lists_snapshots_and_round_trips`).
- **Default title (2026-11-23).** Desktop draws its default title ("Total et Target par K" in a
  French Desktop) above the Spec visual, in the same place and style as a built-in chart's, and
  the drawing below it. The renderer now keeps the container's title for the Spec visual (other
  undrawn custom visuals still show a labelled box without one):
  `the_spec_visual_has_the_default_title_above_its_drawing`.
- **Left.** A report written before the fix with another custom visual keeps the guid-named
  item until it is imported again. The runtime's tooltips, selection and cross-filtering,
  Ctrl+click, background clear, context menu and keyboard focus were seen in Desktop 2.159.1231.0
  on 2026-11-24 ([community-visuals.md](community-visuals.md) Runtime, "Seen in Desktop"; the
  focus is kept after a selection since 1.1.2.0); high contrast and the Spec visual's landing
  page were not checked there. Its roles read Catégorie and Valeurs in that Desktop's French Build
  pane, its Modern, Bold and Minimal looks (the spec's `style`) draw as `report.render` draws
  them, and a bookmark restores a selection in it (2026-11-24, G114). No
  stacked bars, sorting, small multiples or second y axis; the live viewer's HTML page draws it
  from the renderer's image since 2026-11-24 (G29); the two layouts and the runtime are run only where Node is on the PATH.
- **Effort.** S–M.

### G109. Chart pack, part B left (2026-11-23)

- **Done.** Four of BIcraft's custom visuals ([formats/chart-pack.md](formats/chart-pack.md)):
  BIcraft Org Chart, Venn, Marimekko and Calendar Heatmap, each with its data roles, format pane,
  tooltips, selection ids, context menu, keyboard focus, high contrast and landing page from the
  M12.2 runtime; `chart_pack::install` embeds a package once; the snapshot's `chart_pack` lets
  `report.render`, PDF export and the app draw the same primitives as the TypeScript (21 golden
  layouts; each packaged visual run under Node with a stand-in host and document).
- **Seen in Desktop (2.159.1231.0, 2026-11-24).** The four draw as `report.render` draws them,
  with tooltips, selection and cross-filtering both ways, the context menu, the landing page, the
  French format pane, and the org chart's collapse saved in `tree.toggled` and read back by
  BIcraft ([community-visuals.md](community-visuals.md) Runtime, "Seen in Desktop"). Fixed then:
  the Venn circle selected one row (now every row of its set, 1.0.1.0), the renderer titled the
  org chart and calendar "Total" and the Venn diagram not at all (now Desktop's titles).
- **Layout options (2026-11-24, 1.0.2.0).** Both drawers, held equal by 9 more golden cases:
  the org chart's compact tidy tree (subtrees placed by their contours, `tree.compact`, on by
  default; off: leaves in equal slots), the area-proportional two-set Venn diagram
  (`sets.proportional`), the Marimekko's widest-first columns and width axis (`columns.sort`,
  `columns.widthAxis`), the calendar's months-with-data range and Sunday-first weeks
  (`range.months`, `range.sundayFirst`), each in the format pane (EN, FR).
- **1.0.2.0 in Desktop (2.159.1231.0, 2026-11-24).** A PBIX `bicraft` wrote with every new
  option on (the org chart compact, a two-set Venn proportional, the Marimekko sorted with its
  width axis, the calendar on months with data and Sunday-first; with the four part A visuals of
  G110 set to cumulative, vertical, horizontal and slope) opened without a message; each of the
  eight drew the same shapes as `bicraft-cli render` (Desktop's text in French: janv., lun.), and
  the format pane showed each new option as a toggle, on (French: Arbre compact; Surfaces selon
  les nombres (deux ensembles); Les plus larges d'abord, Axe des largeurs; Seulement les mois
  avec des données, Semaines commençant le dimanche; Cumulé; Vertical; Horizontal; Graphique de
  pente). No mismatch found.
- **Orientation, series colours, collapse buttons (2026-11-24, org chart and Marimekko
  1.3.3.0).** Both drawers, 3 more golden cases and unit tests seen failing first: the org chart
  drawn left to right (`tree.leftToRight`, "Left to right" / "De gauche à droite" in the format
  pane, `displayNameKey` `Object_tree_leftToRight`): levels across, slots down, connectors from
  the parent's right edge, the button on the box's right edge; the Marimekko's first six series
  take `colors.series1`–`series6` ("Series 1" / "Série 1" …) over the look's palette colour,
  which they keep otherwise (the theme's or BIcraft's, as the other visuals since 1.1.0.0). The
  org chart's collapse buttons are hit regions in BIcraft's app and viewer (G29): the app saves
  the click as the TypeScript does (`visual.toggle_node`, undoable), a reader keeps it in the
  view state (`view.interact` `tree_toggle`).
- **Left (2026-11-25).** High contrast in Desktop. 1.3.3.0 was seen in Desktop 2.160.1151.0 on
  2026-11-25: left to right, the series colours and the collapse saved in `tree.toggled`, as
  `bicraft-cli render` draws them ([formats/chart-pack.md](formats/chart-pack.md) "Seen in
  Desktop"). The data roles, in English in a French Desktop
  before `stringResources` were packaged, were seen in French in Desktop 2.159.1231.0 on
  2026-11-24 (1.1.0.0; Id, Id du parent, Étiquette, Valeur; Élément, Ensemble; Catégorie, Série,
  Valeur; Date, Valeur). Layout limits: the org chart
  has no images or drag, and its compaction packs to the left (no even spread of
  small subtrees between large ones); the Venn diagram is area-proportional for two sets only and
  draws three sets at most; the Marimekko's series past the sixth have no colour option; the
  calendar has no week range or fiscal year. The live viewer's HTML page draws them from the renderer's image since
  2026-11-24 (G29).
- **Effort.** M.

### G106. Sparklines and Data format (2026-11-23)

- **Done.** The Format pane's General › Data format card and `visual.set_data_format` write a
  field's format for one visual as the projection's `format` (public PBIR schema
  `RoleProjection.format`; a PBIX legacy layout's `columnProperties.formatString`); the preview's
  data column takes it and the renderer draws the values with it (scientific and General through
  the VB `FORMAT` rules). `visual.add_sparkline`, `visual.edit_sparkline` (Insert › Sparklines,
  a value field's menu in the Build pane, the palette) write a `SparklineData` projection and an
  `objects.sparklines` entry selected by its queryRef; the preview gives each row of a table or
  matrix its points and bicraft_render draws them as a line or columns with markers
  ([formats/sparklines.md](formats/sparklines.md)).
- **Checked in Desktop 2.159.1231.0 (2026-11-23).** BIcraft's sparklines (PBIX and PBIP, line
  and column, date and text X axis, markers) draw in Desktop and its Edit sparkline dialog and
  Sparklines card read them; a sparkline added with Desktop's own dialog has BIcraft's
  `queryRef` and selector. Fixed to match what Desktop saves: no `PointsPerSparkline`,
  `ApplyCalculationGroupTo: "Point"` by default (the Add dialog's choice), `markers` as a double
  literal with Desktop's bits (1 every point, 2 highest, 4 lowest, 8 first, 16 last, 32
  negative, 128 isolated; none written marks isolated points, drawn so too). The Data format
  card now writes Desktop's strings (`G`, `0.00E+000`, `d`/`D`/`G`/`T`/`t` for named dates, the
  three-section `$` and `€` currencies; percentage thousands off by default); every BIcraft
  format draws in Desktop with the same values, a negative value under a negative section is
  drawn by the VB rules (`($5.00)`). Formats Desktop's card shows as Custom: `0.00%` (its
  percentage takes the UI locale's `0.00 %` form), other currencies, and `G` on a date (yet
  Desktop's own choice). Details in [formats/sparklines.md](formats/sparklines.md) "Seen in
  Desktop".
- **Total row (fixed 2026-11-24).** The preview gives a table's and a matrix's Total row the
  sparkline's points in the total's filter context, and each matrix subtotal level the points
  per member of the fields it keeps; bicraft_render draws them in the table's Total row and the
  matrix's totals (`the_total_row_and_subtotals_have_their_sparklines`,
  `the_total_row_draws_its_sparkline`). The same test found that bicraft_render drew no
  sparkline column in a matrix at all (it kept only numeric value columns); it now draws it in
  every row.
- **Matrix subtotals in Desktop (2026-11-24, Desktop 2.159.1231.0).** A matrix with Dept › Name
  rows, Tier columns and a line sparkline of Total by Day, expanded one level: Desktop draws a
  sparkline in every leaf row, in each Dept subtotal row, in the column subtotal (the Total
  column group) and in the grand total, as BIcraft does. Its rendered DOM showed that a line
  places each point by its X value on one X range for the whole visual (the earliest to the
  latest Day of every cell; 4 px from the left, 6 px from the right of the cell) and its own Y
  range per cell (4 px from the top and bottom; one point at mid-height), so a row whose days fall
  in May and June draws in the right part of its cell. BIcraft spread every row's points over
  the whole cell; fixed: bicraft_render now places a line's points on the column's X range over
  its rows, totals and subtotals when every X is a number or a date
  (`sparkline_points_share_the_visuals_x_range`).
- **Left.** The X axis is sorted by value (a
  sort-by column such as a month number is not followed); when a row has more than 52 points the
  first 52 are kept (Desktop's choice is not documented); a text X axis and a column
  sparkline still spread their points evenly over the cell (Desktop not looked at for either);
  no shared Y scale across rows, no
  tooltip on a point; `ApplyCalculationGroupTo` is written, not evaluated; the Data format card
  offers six currency symbols and seven date formats (any other through Custom).
- **Effort.** S.

### G111. BIcraft visuals built in left (2026-11-23)

- **Done.** `crates/engine/src/builtin_visuals.rs` registers BIcraft's nine visuals (the Spec visual and
  the first pack's parts A and B); the Visualizations pane lists them after the built-in types whether the report embeds
  them or not; `visual.add` and `visual.set_type` take their name or guid and embed the package on
  first use (a missing or older build replaced, a newer one kept); `report.custom_visuals`
  reports them; `file.save` and `file.save_as` drop the package of one the edited report no longer
  uses, and a PBIP save now removes the folder of a dropped custom visual.
- **Fixed 2026-11-24.** `report.apply` takes their names and guids as `visual.add` does (the
  guid stored, their own roles checked, each package embedded on first use before the plan is
  applied, `installed`); a Spec visual added by `visual.add`, `visual.set_type` or `report.apply`
  without a spec starts with `visual.spec_template`'s `combo` for two values or more, else `bar`
  (`report_apply_takes_bicraft_visual_names`, `a_new_spec_visual_starts_with_a_template_spec`).
  The template follows the number of values only, not the category's data type.
- **Left.** Not checked in Desktop: a report holding a package kept at a newer
  version. Seen in Desktop 2.159.1231.0 (2026-11-24): a PBIX whose three unused BIcraft packages
  were dropped on save opens without a message and its Visualizations pane lists only the embedded
  ones; that PBIX still held the dropped packages' `Report/CustomVisuals/` files (Desktop's own
  save keeps such files too), and the PBIX writer now removes a `Report/CustomVisuals/` part the
  report no longer lists (`a_dropped_custom_visual_leaves_no_parts`).
- **Icons, 2026-11-24.** Each of the ten visuals has its own drawing in BIcraft's icon style
  (`crates/visuals/src/icons.rs`), painted by the pane in the theme's ink and carried by its
  package as a 20 × 20 PNG (docs/community-visuals.md "Icons"). Seen in Power BI Desktop
  2.159.1231.0 (2026-11-24), light and dark: the ten icons are distinct in the Visualizations
  pane. Left: Desktop draws the PNG at its own size as a 20 × 20 CSS-pixel background, so on a
  scaled screen it is upscaled and soft, and the slate ink reads fainter than the built-in
  types' icons; a larger PNG would be cropped.
- **Fixed 2026-11-25 (darker, bolder ink, the maintainers' choice).** The package PNG keeps 20 ×
  20 and draws its ink in `#4a5058` (7.5:1 on Desktop's light pane, 2.3:1 on its dark one, where
  Desktop's own icons keep a darker ink), strokes 1.3 times the UI's rule (about 1.5 px) and
  fuller tones (ink `1 - (1 - a)³`, accent `1 - (1 - a)²`); the gallery is unchanged
  (`package_icons_have_dark_bold_ink`). Every visual's package version was raised for the new
  icon bytes (histogram, bullet chart, box plot 1.3.1.0, dumbbell 1.3.3.0, org chart and
  Marimekko 1.3.4.0, Venn 1.3.3.0, calendar 1.3.2.0, Sankey 1.3.5.0, Scatter+ 1.0.2.0, Line /
  Area+ 1.0.3.0, Waterfall+ 1.0.2.0, Gauge / KPI+ 1.0.1.0, Spec 1.4.1.0). Seen in Power BI
  Desktop 2.160.1151.0 (2026-11-25, dark theme, 150 % screen) on a PBIX `bicraft` wrote with the
  14 visuals: the Visualizations pane shows the new icons at about the weight of its own icons'
  ink and fills. Left: the light theme not seen in Desktop on this build (checked on the
  rasterized PNGs over its light pane colour); the PNG stays soft once Desktop upscales it.
- **Effort.** S.

### G112. BIcraft visuals outside BIcraft left (2026-11-24)

- **Done.** `visual.export_bicraft_visual {name, out}` writes the package BIcraft embeds for one
  of its nine visuals, a list or `all` (`<guid>.<version>.pbiviz` files in a folder); `bicraft
  visual export` and Insert › Visuals › Export a BIcraft visual run it; every exported file is read
  back by `visual.import_custom` as the current package and its capabilities validate against the
  API schema; `cargo xtask dist` writes the nine with the release binary under `visuals/` with a
  README naming their TypeScript sources, and its smoke test checks the folder.
- **Imported in Desktop (2026-11-24, Power BI Desktop 2.159.1231.0).** On a PBIX `bicraft`
  wrote without any BIcraft visual, the Visualizations pane "…" › Import a visual from a file
  showed Desktop's warning about custom visuals (Import, Cancel, "do not show again"), then the
  Windows Open dialog filtered on `*.pbiviz`; `bicraftHistogram….1.0.2.0.pbiviz` and
  `bicraftVenn….1.0.2.0.pbiviz` from `bicraft-cli visual export --all` each gave "Import succeeded"
  and an icon in the pane. Added to the page, the histogram showed its landing page, then drew
  Score's bins (Score dropped on Values; ticking a numeric column put its Sum in Frequency, as
  Desktop does with a measure role), People in Frequency; the Venn took S2 Id and Skill (ticked)
  and drew DAX 6, both 3, SQL 3. Saved from Desktop, the PBIX holds both packages at 1.0.2.0
  (`report.custom_visuals`: registered, no issue) and `bicraft-cli render` draws them alike. The
  warning and the success message are an embedded browser control without accessibility
  content: they were answered with posted mouse messages, the Open dialog with window messages.
  The Build pane's wells show the role names in English (Values, Frequency, Item, Set) while
  the landing page is French, as for the embedded packages (G109, G110).
- **Left.** Uploaded as an organizational visual in the Power BI service (needs a tenant
  administrator); `cargo xtask dist`
  with the new step not yet run end to end (release build, WSL). Out of scope by the maintainers'
  decision of 2026-11-23: any Microsoft Marketplace (AppSource) or Microsoft Store submission or
  certification.
- **Effort.** S.

### G113. BIcraft Sankey left (2026-11-24)

- **Done.** BIcraft Sankey (`bicraftSankey`, 1.0.0.0; [formats/chart-pack.md](formats/chart-pack.md)
  "Sankey"), added to the first pack at the user's request: source, destination and weight;
  nodes in columns by longest path, heights by flow, barycentre ordering, Bézier ribbons, cycles
  left out with a note, a format pane, role and option names (`displayNameKey`, `stringResources`) and date nodes (their column's format string) in English and French, the runtime's interactions, the
  Rust twin in `report.render`, PDF and the app (7 golden cases, the TypeScript compared under
  Node), registered for the Visualizations pane, `visual.add`, `report.apply` and
  `visual.export_bicraft_visual`. Since 1.2.0.0 (2026-11-24, the look's second pass): a link that
  skips columns runs through a waypoint in each column between its ends, the waypoints sorted
  with the nodes by the barycentre sweeps, so it passes beside the nodes, never over them (under
  every preset); each node's links keep their own slots of its height, ordered by the other end;
  since 1.3.4.0 (2026-11-25) the first twelve nodes seen take a colour option each
  (`colors.node1`–`node12`, the Node colors card, swatches named after the nodes in Power BI,
  EN/FR with `displayNameKey`) and the links coloured by a node follow it (golden case
  `sankey_node_colors`, `sankey_node_colour_options_colour_nodes_and_their_links` seen failing
  first);
  in a dense column a label closer than its size to a larger node's label is left out; the last
  two columns' labels share their gap only where they face each other, so a narrow visual keeps
  them whole. A regression test checks every Sankey case for shared slots and ribbons over nodes
  in Rust and TypeScript.
- **Left.** One level of links only: no multi-level paths (several source and destination pairs
  or a path column), no node or link drag, node colours only for the first twelve nodes seen (by
  place, not by name: a node seen in another place after a data change takes that place's
  colour) and no colour rule or legend; 1.3.4.0 not yet opened in Desktop;
  nodes are capped at 100 and links at 1,000; a label left out in a dense column is only in the
  tooltip; a cycle's left-out link is only counted, not drawn back; high contrast is not yet seen
  in Desktop.
- **Seen in Desktop (2.159.1231.0, 2026-11-24, the waypoint routing).** Links skipping one and
  two columns under Modern, Bold and Minimal, and visuals 220, 180 and 160 px wide or high: drawn
  as `bicraft-cli render` draws them. Fixed: Desktop drew the labels at 11 px under Modern where the
  renderer drew 12 px (and so left out a label Desktop kept), because the packaged Sankey,
  Marimekko, org chart and Venn read an unset `fontSize` as a fixed default; they now take the
  look's text size, the format pane shows it (pack 1.3.2.0, `the_packaged_visuals_run_under_node`,
  seen failing first). 1.3.2.0 not yet opened in Desktop.
- **Seen in Desktop (2.159.1231.0, 2026-11-24, before the waypoint routing).** Energy flows with a
  link skipping a column, a cycle page and date nodes: drawn as `bicraft-cli render` drew them then
  (the skipping ribbon behind the middle node, the French cycle note, midnight dates as
  `05/01/2026 00:00:00` like Desktop's table); node and ribbon tooltips; click, background click
  and cross-filtering both ways; the data point menu; Enter and Escape on a focused ribbon; the
  format pane, roles (Source, Destination, Poids) and the Couleur selon drop-down in French,
  `links.colorBy` saved by Desktop and drawn alike by `bicraft-cli render`
  ([formats/chart-pack.md](formats/chart-pack.md) "Seen in Desktop"). Fixed: Ctrl+click on a node
  sharing rows with the selection toggled the shared rows off; the runtime now adds only the ids
  not yet selected (`multiIds`), seen right in Desktop.
- **Effort.** M (multi-level).
### G114. BIcraft's look left (2026-11-24)

- **Done.** One look layer for BIcraft's ten visuals (docs/formats/visual-look.md): theme
  colours or BIcraft's palette, Modern / Minimal / Bold presets and their switches, decorated
  marks in the renderer (PNG and PDF) and in Power BI (SVG), the Style card, the entry animation
  and the hover dimming; Minimal reproduces the earlier drawings (the earlier golden files pass
  under it), 15 golden cases cover the presets and the theme. Second pass (2026-11-24, chart
  visuals and pack 1.2.0.0, Spec 1.3.0.0): a vivid eight-hue BIcraft palette (validated for
  colour-blind separation in light and dark), used by Modern and Bold also on Power BI's stock
  themes; dark report backgrounds get dark-surface text, grid and axis tokens (4.5:1 or more,
  tested); a card behind the plot (Style option, on in Modern and Bold) and value chips (on in
  Bold); larger tick, name and value text; the calendar heatmap fills its frame with month
  separators, weekday names and a stepped legend; the Spec visual has the Style card in its
  format pane (its settings over the spec's `style`), and its combo lines markers. Minimal draws
  as before except two deliberate changes listed in `look_minimal_tests.rs` (Sankey routing,
  BIcraft's palette when asked for); 9 more golden cases.
- **Seen in Desktop (2.159.1231.0, 2026-11-24, first pass).** The ten under Modern, Bold and
  Minimal and a custom report theme draw as `bicraft-cli render` draws them in the theme's colours;
  the entry animation runs on a data update, hovering dims the other points; the Style card and
  its drop-downs are in French and Desktop's save of a preset is drawn alike by `bicraft-cli render`
  ([formats/visual-look.md](formats/visual-look.md) "Seen in Desktop"). Fixed: a custom theme
  without a base theme keeps Desktop's classic titles (grey, regular), which the renderer drew
  dark and bold (the resolved theme's `classic`).
- **Interactions (2026-11-24, same Desktop).** With a column chart, a table, a card and two
  slicers: the ten's selections highlight the column chart and filter the others, slicers,
  page and report filters and drillthrough filter the ten, Edit interactions rules are honoured,
  a bookmark restores a Spec selection ([community-visuals.md](community-visuals.md)
  "Interactions with built-in visuals").
- **Highlights (2026-11-24, chart visuals and pack 1.3.0.0, Spec 1.4.0.0).** The nine with a
  measure role declare `supportsHighlight` (Microsoft Learn, "Highlight data points in Power BI
  visuals": the categorical values carry `highlights` beside `values`); the Venn diagram, without
  a value column, does not. The Spec visual's bars, the bullet's value bar, the Marimekko's
  segments, the calendar's days and the Sankey's ribbons draw the highlighted part over the full
  mark at 40 %; the histogram's bins, the box plot's boxes, the dumbbell's rows, the org chart's
  nodes and Sankey nodes without a highlighted link are dimmed
  ([formats/bicraft-visuals.md](formats/bicraft-visuals.md) "Highlights"). The renderer draws the
  same from the snapshot's `data.highlights` (twin layouts, held equal by 16 new golden cases under
  Node), and BIcraft's interactions highlight them (`interact::can_highlight`) instead of
  filtering them when a field is in a measure role; slicers still filter.
- **Seen in Desktop (2.159.1231.0, 2026-11-24, highlights and second pass).** Edit interactions
  offers Filter, Highlight and None on the nine, Highlight by default (Filter and None on the
  Venn); a column chart bar or a table row highlights the nine as `bicraft-cli render` draws the same
  selection and filters the Venn. A visual of the nine with no field in its measure roles gets
  all its rows and no `highlights` and stays unchanged: fixed, BIcraft filtered it and now leaves
  it as it is (`a_bicraft_visual_without_a_measure_field_is_highlighted_unchanged`, seen failing
  first). The card, value chips, Bold labels and the Spec visual's Style card (French, Fond en
  carte off saved and drawn alike) draw as `bicraft-cli render` draws them; inside their frames the
  dark-surface tokens too ([formats/visual-look.md](formats/visual-look.md) "Seen in Desktop").
- **Left.** Under a dark custom theme (`#1F1F1F` background, no base theme) Desktop keeps the
  page canvas and the visual containers white with grey titles, where `bicraft-cli render` paints
  page and containers in the theme's background (the built-in renderer's page and container
  backgrounds, not BIcraft's visuals). The org chart dims a node's card, not its texts, and the
  dimming is hard to see in Desktop and the render alike. The Spec combo template's legend shows
  its own field names ("First measure", "Second measure"). Not yet seen in Desktop: high
  contrast, reduced motion. The Spec visual's value-range colours keep their earlier default. Night tokens follow
  the theme's background only, not a page background set on its own. Seen beside them in the
  same run, for the built-in renderer: a column chart on a dateTime column is drawn by Desktop on
  a continuous time axis, and now by `report.render` too (2026-11-24, [render.md](render.md)
  "Continuous date axes"; its tick texts, eight-label limit and label room measured in Desktop
  later that day and followed; left there: Desktop's first-and-last labels on a narrow daily
  chart and its line value axes not starting at zero), with numeric categories, the axis
  `start` / `end` and the live HTML viewer following the same rules (render.md "Continuous
  numeric axes, axis start and end"; not compared with Desktop: numeric ticks, how Desktop saves
  a date `start`, whether a ribbon chart can be continuous); a classic slicer on a dateTime
  column without a mode is now a Between range in the snapshot, as in Desktop.
- **Effort.** S (the dark page and container backgrounds; high contrast and reduced motion in a
  Desktop session).

### G117. BIcraft Waterfall+ left (2026-11-24)

- **Done.** BIcraft Waterfall+ (`bicraftWaterfallPlus`, 1.0.0.0;
  [formats/waterfall-gauge.md](formats/waterfall-gauge.md)): vertical or horizontal steps from a
  running total, subtotals after each group, a flag column or every N steps, a total, colours by
  sign, step or a rule, up to eight measures as the steps, a target marker with the variance in
  the header, value and percent labels, connectors, BIcraft's look and Style card, highlights,
  format pane in English and French, the runtime's interactions, the Rust twin in
  `report.render`, PDF and the app, registered for the Visualizations pane, `visual.add` and
  `visual.export_bicraft_visual`.
- **Seen in Desktop (2.160.1151.0, 2026-11-25)** as `bicraft-cli render` draws it, with tooltips,
  selection both ways, highlights, the French format pane and options saved by Desktop read back
  ([formats/waterfall-gauge.md](formats/waterfall-gauge.md) "Seen in Desktop").
- **Done (1.0.1.0, 2026-11-25).** The value axis groups its digits by the culture in both drawers
  (`1 000` in French, a no-break space, as the tooltips write them), and a fall's label in the
  horizontal layout goes left of its bar only inside the plot, else right of the bar, else it is
  left out (golden cases `wf_thousands_fr`, `wf_small_fall_horizontal`; the test
  `waterfall_axis_follows_the_culture_and_fall_labels_clear_the_names` seen failing on 1.0.0.0).
- **Left.** 1.0.1.0 not yet opened in Desktop; the axis ticks are not written with the measure's
  format string (no currency, no display units); high contrast not checked in Desktop. Several measures are summed over the rows, so a
  non-additive measure (a ratio, a distinct count) gives a wrong step; there is no breakdown by
  category within a measure and no category × measure grid; with several measures a step selects
  nothing (a custom visual cannot select a measure); subtotals, the total and the target have no
  rows of their own and select nothing; no drill-down; a label that does not fit is left out, not
  moved; at most 200 bars and 1,000 rows; number formats are held equal between the runtime and
  the renderer only on the tested format strings.
- **Effort.** M (breakdown).

### G118. BIcraft Gauge / KPI+ left (2026-11-24)

- **Done.** BIcraft Gauge / KPI+ (`bicraftGaugeKpi`, 1.0.0.0;
  [formats/waterfall-gauge.md](formats/waterfall-gauge.md)): a radial arc, a linear bullet or a
  KPI card; colour bands from up to three thresholds (roles or the format pane), a target, the
  minimum and maximum from roles, the format pane or a rounded automatic maximum; the variance
  against the target or the previous value, good and bad in colour, sign and words, reversible
  with `lowerIsBetter`; a sparkline from an optional trend field; the value in its field's
  format; BIcraft's look, highlights, format pane in English and French, the runtime's
  interactions and the Rust twin, registered like the others.
- **Seen in Desktop (2.160.1151.0, 2026-11-25)**: arc, bullet and KPI card as `bicraft-cli render`
  draws them, with tooltips, selection, highlights, the French format pane and the style and
  preset saved by Desktop read back ([formats/waterfall-gauge.md](formats/waterfall-gauge.md)
  "Seen in Desktop").
- **Left.** High contrast not checked in Desktop. One value per visual (no small multiples or
  per-category gauges); the arc draws no tick per threshold and no needle; the sparkline has no
  axis, no target line and shows only the latest 60 dots; four bands at most; the thresholds are
  absolute values, not percentages of the target; number formats as for G117.
- **Effort.** M (small multiples).
