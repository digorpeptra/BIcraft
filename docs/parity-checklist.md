# Power BI Desktop feature checklist

> **Generated:** 2026-11-05 (summary by `cargo xtask parity`; rows scored against `main` at
> ccb96e3, gap review; rows touched since by each lot, last rescored 2026-11-18 on `main` at 816a4cb) · **Target:** Power BI Desktop as Microsoft Learn describes it (November 2026); since
> 2026-11-23 also as Desktop 2.159.1231.0 (Store) shows it, read as a black box
> ([Desktop UI inventory](#desktop-ui-inventory), M11.28)

The row-by-row checklist behind the breadth numbers of
[target-app-parity.md](target-app-parity.md). Two axes, scored separately:

- **Authoring**: what a report author does in Desktop (model, Power Query, report, files). Status
  is BIcraft's, through its engine commands (CLI, MCP, `batch`); **Button** says whether the
  BIcraft app (one egui UI, desktop and browser) reaches it and how. Since M11 every command has
  a button ([ROADMAP.md](../ROADMAP.md) M11.3), most of them through the generic parameters form.
- **Viewer**: what Desktop's reading view shows and lets a reader do, for three surfaces:
  **render** = `bicraft-cli render` / `report.render` (PNG), **web** = the HTML viewer of
  `bicraft-cli view` (`crates/view/src/page.html`), **app** = the BIcraft egui app (desktop and
  browser, [webui.md](webui.md)).

Status: **D** done, **P** partial, **M** missing, **O** out of scope (left out of every share),
**–** not applicable to that surface (a PNG has no clicks; also left out). Priority **P0** is a
common workflow, **P1** a frequent one, **P2** a less common one; weights 3 / 2 / 1. "Kept"
means BIcraft reads the object, keeps it byte for byte on save and follows renames, but has no
command that creates or changes it: that scores **M** for authoring.

Do not edit between the summary markers: change the rows, then run `cargo xtask parity`.
`cargo xtask docs` (part of `cargo xtask ci`) fails when the summary or the headline of
target-app-parity.md is stale.

<!-- SUMMARY -->
**Authoring: 127 of 145 items done (88%), weighted parity 95%** (P0 99%, P1 96%, P2 84%); 14 partial, 4 missing, 9 out of scope. 141 of the 141 items BIcraft has (done or partial) have a button in the app.

| Area | Items | Done | Partial | Missing | Parity |
|---|---|---|---|---|---|
| Tables | 6 | 6 | 0 | 0 | 100% |
| Columns | 6 | 5 | 1 | 0 | 96% |
| Measures | 4 | 4 | 0 | 0 | 100% |
| Hierarchies | 1 | 1 | 0 | 0 | 100% |
| Relationships | 3 | 3 | 0 | 0 | 100% |
| Calculation groups | 1 | 1 | 0 | 0 | 100% |
| Parameters | 3 | 3 | 0 | 0 | 100% |
| Security | 2 | 2 | 0 | 0 | 100% |
| Metadata | 4 | 3 | 1 | 0 | 88% |
| Storage | 4 | 2 | 0 | 2 | 50% |
| Power Query | 9 | 8 | 1 | 0 | 94% |
| Data | 5 | 3 | 2 | 0 | 80% |
| Views | 6 | 6 | 0 | 0 | 100% |
| Editing | 4 | 4 | 0 | 0 | 100% |
| Files | 9 | 7 | 2 | 0 | 92% |
| Pages | 6 | 6 | 0 | 0 | 100% |
| Visuals | 22 | 17 | 5 | 0 | 94% |
| Formatting | 11 | 11 | 0 | 0 | 100% |
| Filters | 6 | 6 | 0 | 0 | 100% |
| Interactions | 3 | 3 | 0 | 0 | 100% |
| Themes | 3 | 3 | 0 | 0 | 100% |
| Report | 2 | 2 | 0 | 0 | 100% |
| Report UI | 14 | 14 | 0 | 0 | 100% |
| Model UI | 7 | 5 | 1 | 1 | 88% |
| Accessibility | 2 | 2 | 0 | 0 | 100% |
| Localisation | 1 | 0 | 1 | 0 | 50% |
| Tools | 1 | 0 | 0 | 1 | 0% |

Open P0 authoring items: Refresh from databases and online services (SQL Server, SharePoint, web, OData…).

**Viewer** (Desktop's reading view):

| Surface | Items | Done | Partial | Missing | Done share | Weighted parity |
|---|---|---|---|---|---|---|
| `bicraft-cli render` (PNG) | 33 | 25 | 4 | 4 | 76% | 89% (P0 100%, P1 85%, P2 55%) |
| Web viewer (`bicraft-cli view`, page.html) | 56 | 28 | 5 | 23 | 50% | 64% (P0 90%, P1 41%, P2 25%) |
| BIcraft app (egui, desktop and browser) | 56 | 47 | 6 | 3 | 84% | 92% (P0 100%, P1 86%, P2 79%) |

| Area | render | web | app |
|---|---|---|---|
| Visuals | 90% | 78% | 90% |
| Page | 100% | 89% | 100% |
| Data | 90% | 90% | 90% |
| Interactions | 100% | 57% | 91% |
| Navigation | – | 100% | 100% |
| Reader tools | 0% | 12% | 94% |
| Export | 100% | 0% | 100% |

Open P0 viewer items in the app: none.

**Desktop UI inventory** (93 groups, 538 controls read in Desktop, 28 of them out of scope): of the 510 in-scope controls, 223 sit in groups BIcraft has done, 283 partial, 4 missing; **coverage 71%** (done 1, partial 0.5, weighted by controls).

| Surface | Groups | Controls | Done | Partial | Missing | Out of scope | Coverage |
|---|---|---|---|---|---|---|---|
| Report ribbon | 29 | 96 | 46 | 28 | 1 | 21 | 80% |
| Visual tabs | 7 | 18 | 18 | 0 | 0 | 0 | 100% |
| Table view | 5 | 54 | 33 | 21 | 0 | 0 | 81% |
| Model view | 4 | 38 | 10 | 25 | 3 | 0 | 59% |
| DAX query view | 2 | 16 | 6 | 10 | 0 | 0 | 69% |
| TMDL view | 2 | 14 | 7 | 7 | 0 | 0 | 75% |
| Chrome | 4 | 21 | 19 | 0 | 0 | 2 | 100% |
| File view | 8 | 32 | 6 | 23 | 0 | 3 | 60% |
| Panes | 7 | 82 | 37 | 45 | 0 | 0 | 73% |
| Menus | 3 | 29 | 0 | 29 | 0 | 0 | 50% |
| Dialogs | 4 | 44 | 11 | 33 | 0 | 0 | 62% |
| Power Query | 18 | 94 | 30 | 62 | 0 | 2 | 66% |
<!-- /SUMMARY -->

## Authoring rows

**Button** names how the app reaches the command since M11 (every command has its button,
`every_command_has_a_button`; [commands-ui.md](commands-ui.md)): `ribbon`, `menu` (right-click),
`palette`, a `pane` (Data, Build, Filters), the `formula bar`, the `page tabs`; `form` means the
button opens the generic parameters form (`crates/ui/src/form.rs`), not a dialog or pane shaped
like Desktop's. `no`: nothing to reach (no command). Rows in the **Report UI** and **Model UI**
areas score Desktop's dedicated surfaces themselves (a pane, a mode, direct manipulation): there a
command reached only through a form is **P**.

| Area | Feature | Priority | Status | Commands | Button |
|---|---|---|---|---|---|
| Tables | Add, rename, remove a table (relationships, roles and DAX follow) | P0 | D | `table.add`, `table.rename`, `table.remove` | yes (ribbon, menu, form) |
| Tables | Enter data or import a table from CSV, an M query or a DAX literal | P0 | D | `table.import`, `table.set_data` | yes (ribbon Get data, form) |
| Tables | Get data: source gallery and Navigator (Excel, Text/CSV, JSON, Folder, SQL Server database, Web, OData feed; preview, check items, load) | P0 | D | `source.navigate`, `source.load`, `query.preview` | yes (Home › Get data, Navigator) |
| Tables | Enter data and Edit data as an editable typed grid | P1 | D | `table.enter_data`, `table.entered_data` | yes (Home › Enter data, Data pane menu Edit data) |
| Tables | Calculated tables | P1 | D | `table.import {dax}`, `partition.set` | yes (formula bar) |
| Tables | Table properties: description, hidden, mark as date table | P1 | D | `table.set` | yes (ribbon, form; Data pane Hide) |
| Columns | Calculated columns | P0 | D | `column.add {expression}`, `column.set` | yes (formula bar, ribbon) |
| Columns | Rename and remove columns (DAX follows) | P0 | D | `column.rename`, `column.remove` | yes (menu, Data pane) |
| Columns | Data type, format string, summarize by, display folder, hidden, description | P0 | D | `column.set` | yes (ribbon, form) |
| Columns | Sort by column, data category, key column | P1 | D | `column.set {sort_by_column, data_category, is_key}` | yes (ribbon, form) |
| Columns | Groups and bins | P2 | D | `column.group` (list groups with Other and (Blank), numeric bins by size or count, date bins by years, months, days, hours, minutes or seconds, or into a number of bins; Group data on selected data points (`visual.group_points`); a calculated column with `GroupingMetadata` and `GroupingDesignState` in the form Desktop saves; Edit groups with `replace`; G75) | yes (Data pane menu New group, Column tools › Data groups, Groups dialog, a data point's Group data) |
| Columns | Auto date/time variations | P2 | P | `column.remove_variation` (removal only) | yes (ribbon, form) |
| Measures | Add, edit, rename, move, remove measures (DAX checked, references follow) | P0 | D | `measure.add`, `measure.set`, `measure.rename`, `measure.move`, `measure.remove` | yes (formula bar, ribbon, menu) |
| Measures | Static and dynamic format strings | P1 | D | `measure.set {format_string, format_string_definition}` | yes (ribbon, form) |
| Measures | Quick measures | P2 | D | `measure.quick` (27 calculations: per category, filtered value and differences, sales from new customers, YTD (fiscal year end too)/QTD/MTD, YoY/QoQ/MoM, rolling average, running total, totals for a category, arithmetic, correlation coefficient, star rating, concatenated list; BIcraft's own DAX templates, G76) | yes (Home › Quick measure, Data pane menu New quick measure, Quick measures dialog) |
| Measures | DAX user-defined functions | P2 | D | `function.add`, `function.set` (expression, or parameters with type, subtype and pass mode plus a body; description, hidden), `function.rename` (calls follow), `function.remove` (refused while called); definitions validated as `DEFINE FUNCTION`, call sites checked by the binder (arity, scalar or table arguments); measures and queries that call one are evaluated (val eager, expr lazy); TMDL, TMSL and PBIX round trips; `objects.import {functions}` ([formats/dax-udf.md](formats/dax-udf.md), G97) | yes (Model explorer Functions node: New, Edit with the DAX editor form, Rename, Delete, Script TMDL)|
| Hierarchies | User hierarchies and levels | P1 | D | `hierarchy.add`, `hierarchy.set`, `hierarchy.remove` | yes (ribbon, menu, form) |
| Relationships | Create, edit, delete (cardinality, cross filter, active, security filter) | P0 | D | `relationship.add`, `relationship.set`, `relationship.remove`, `relationship.list` | yes (ribbon, menu, form) |
| Relationships | Saved model diagram layouts | P2 | D | `diagram.move`, `diagram.add`, `diagram.rename`, `diagram.remove` (layout tabs saved in `diagramLayout.json` / the PBIX `DiagramLayout` part; a table rename follows; no "add related tables", no removal of one table from a layout) | yes (drag a card, layout tabs and their menu) |
| Relationships | Autodetect relationships | P2 | D | `relationship.autodetect` (same name and type, a unique side in the data, no ambiguous path; a preview, then `apply` of the ticked ones in one undo step) | yes (Model explorer, Relationships menu › Autodetect relationships, dialog) |
| Calculation groups | Groups, items, precedence, selection and format string expressions | P1 | D | `calculation_group.add`, `calculation_group.set`, `calculation_item.add`, `calculation_item.set` | yes (ribbon, menu, form) |
| Parameters | Field parameters | P1 | D | `parameter.fields` (tuple table with `NAMEOF`, `ParameterMetadata`, a slicer; the preview shows the chosen fields; renames follow `NAMEOF`) | yes (Model ribbon New parameter, dialog) |
| Parameters | What-if (numeric range) parameters | P2 | D | `parameter.what_if` (`GENERATESERIES` table, `SELECTEDVALUE` measure, a single-value slicer) | yes (Model ribbon New parameter, dialog) |
| Parameters | Power Query parameters and query groups | P1 | D | `expression.add`, `expression.set`, `env.apply` | yes (ribbon, form) |
| Security | Roles with DAX row filters (RLS) | P0 | D | `role.add`, `role.set_filter`, `role.remove` | yes (ribbon, menu, form) |
| Security | Object-level security (OLS) | P2 | D | `role.set_permission` (a table or a column hidden from a role, refused when it would break a relationship chain); View as treats hidden objects as missing; TMDL, TMSL and PBIX round trips | yes (Model explorer, Roles menu › Manage roles, dialog) |
| Metadata | Perspectives | P2 | D | `perspective.add`, `perspective.set`, `perspective.remove` (tables, columns, measures and hierarchies; renames and removals follow); the Model view filters its diagram to one; TMDL, TMSL and PBIX round trips | yes (Model explorer, Perspectives menu › Manage perspectives, dialog; Model view Perspective list) |
| Metadata | Translations: cultures, captions, descriptions, display folders | P2 | D | `culture.add`, `culture.remove`, `translation.set`, `translation.list` | yes (ribbon, menu, form) |
| Metadata | Q&A linguistic schema and synonyms | P2 | P | synonyms of tables, columns and measures (`model.set_properties` `synonyms`, the Properties pane's Synonyms); the rest of the schema (phrasings, relationships, conditions) kept | yes (Properties pane) |
| Metadata | Model settings: culture, discourage implicit measures | P2 | D | `model.set` | yes (ribbon, form) |
| Storage | Storage mode (Import, DirectQuery, Dual), composite models | P1 | D | `table.set_storage_mode` (every partition's `mode`; Desktop's rules: calculated tables Import only, a connector needed, limited relationships and Dual on a fact table warned, `dual_related` sets the DirectQuery dimension tables to Dual); the snapshot's `storage_mode`; TMDL, TMSL and PBIX round trips; DirectQuery tables are not evaluated (next row) | yes (Model view › Properties › Advanced › Storage mode, with the switch-to-Import dialog; a mode badge on each card of a composite model) |
| Storage | DirectQuery execution: visuals query the source live | P1 | M | — (BIcraft evaluates imported data only) | no |
| Storage | Incremental refresh policy | P2 | D | `table.set_refresh_policy` (archive and refresh periods, only complete periods, real time as a hybrid policy, detect data changes; checks the RangeStart/RangeEnd Date/Time parameters, the Import mode and the filter step, and says what is missing), `table.remove_refresh_policy`; the snapshot's `refresh_policy`; TMDL, TMSL and PBIX round trips; the service applies it (kept and read; its partitions are written) | yes (a table's menu › Incremental refresh in the Data pane and on the model diagram: the dialog with the switch, the periods, the optional settings and the review sentences) |
| Storage | User-defined aggregations (Manage aggregations, `alternateOf`) | P2 | M | — (BIcraft's own derived aggregations, `dax.aggregations`, are not Desktop's feature) | no |
| Power Query | Partitions and queries as M text | P0 | D | `partition.set`, `partition.add`, `partition.remove`, `m.validate` | yes (ribbon, form) |
| Power Query | Power Query editor: applied steps, transformations, preview | P0 | D | `query.list`, `query.steps`, `query.apply_step`, `query.edit_step`, `query.remove_step`, `query.new`, `query.rename`, `query.apply` (Close & Apply with renames and deletions, one undo step), `query.preview {step}`; 19 transforms writing M (the filter menu's value list as `filter_values`), the user's text kept for untouched steps | yes (Transform data window: queries list with new, duplicate, reference, rename and delete, applied steps, preview with column type and filter menus, formula bar, transform ribbon; closing with changes asks) |
| Power Query | Advanced editor: edit a query's M with a syntax check | P1 | D | `expression.set`, `partition.set`, `m.validate`; in the Power Query editor `query.steps` checks the whole query's draft; `m.format` lays the M out (comments kept, idempotent, optionally applied in one undo step); `m.complete`, `m.signature`, `m.hover`: completion, signature help and hover from BIcraft's catalogue of the functions its evaluator runs | yes (ribbon, form with an M editor; the Power Query editor's Advanced editor and formula bar, each with a Format button, completion as you type or with Ctrl+Space, the signature of the call being typed and what the name under the pointer is) |
| Power Query | Data preview of a query's result | P1 | D | `query.preview {name or expression, limit}`: the first rows through BIcraft's M evaluator, nothing loaded; connectors only when allowed | yes (Home › Preview query, the Data view's query entries, the grid) |
| Power Query | Data profiling: column quality, distribution, profile | P2 | D | `query.profile {step, column?, full?}`: valid, error and empty shares, distinct and unique counts, the most frequent values, a column's statistics and value distribution, over the top 1000 rows or the entire data set | yes (Power Query editor › View: Column quality and Column distribution under each header, Column profile in a bottom pane, the top 1000 rows / entire data set switch and status line) |
| Power Query | Query dependencies view | P2 | D | `query.dependencies {queries?}`: the queries, the sources they read, the queries they reference and the model tables they load into, in layers | yes (Power Query editor › View › Query dependencies: the diagram with zoom in, zoom out, fit and drag) |
| Power Query | Query diagnostics, view native query, folding indicators | P2 | D | `query.folding` (each step: folded, partially folded, not folded, unknown; Databricks, SQL Server, OData), `query.native` (the SQL or OData URL BIcraft would send), `query.diagnose` (BIcraft's own time per step, rows, the source requests with their native text); no diagnostics session over a whole refresh, no saved diagnostics queries | yes (Power Query editor: an icon and a word per applied step; a step's context menu has View native query, greyed out with why, and Diagnose step) |
| Power Query | The editor's other ribbon transforms (seen in Desktop 2.159.1231.0): transpose, reverse rows, count rows, detect data type, fill, move, convert to list; the text, number and date and time column transforms; index, conditional and duplicate columns, column from examples, invoke a custom function; keep or remove rows by position, range or errors; combine files | P1 | P | `query.apply_step`: the 19 first kinds plus keep and remove rows (top, bottom, range, alternate, blank, errors), replace errors, fill, transpose, reverse rows, use headers as first row, move, duplicate, index and conditional columns, merge columns, and `column_op`: Format, Extract (between delimiters included), Standard, Scientific, Trigonometry, Rounding, Information, Date, Time and Duration, in place or as a new column (`bicraft_data::m::transform_more`). Left: count rows, detect data type, convert to list, Parse, the Statistics menus, column from examples, invoke a custom function, combine files, the Add column menus over several columns | yes (Power Query editor Home, Transform and Add column tabs; Advanced editor) |
| Power Query | Export a query's results to a file (the editor's Home › Output data, seen in Desktop 2.159.1231.0) | P2 | D | `query.export {out}`: every row of the query (or of one step) to a CSV file; only CSV | yes (Power Query editor › Home › Export results, ribbon, form) |
| Data | Refresh from local files (CSV, Excel, JSON, folders) | P0 | D | `model.refresh` (PBIP projects into BIcraft's own data snapshot; a PBIX into its `DataModel` since M11.18, G50, not yet opened in Desktop), `table.import`, `table.set_data` | yes (ribbon Refresh, F5) |
| Data | Refresh from databases and online services (SQL Server, SharePoint, web, OData…) | P0 | P | SQL Server: `Sql.Database` / `Sql.Databases` (TDS 7.4 over TLS, SQL authentication, folding of columns, rows, top and sort; opt-in `allow_connectors: ["sqlserver"]`); Web: `Web.Contents` (options, POST, anonymous; JSON and text read by `Json.Document` / `Csv.Document`, opt-in `["web"]`); OData: `OData.Feed` (v4 JSON, `$metadata` types, folding into `$select`, `$filter`, `$orderby`, `$top`, paging, opt-in `["odata"]`); SharePoint: `SharePoint.Files` / `SharePoint.Contents` / `SharePoint.Tables` over the REST API with a bearer token (opt-in `["sharepoint"]`); credentials per URL for web sources (anonymous, Basic, Web API key, bearer); Databricks; tested only against in-process test servers, no OAuth sign-in, no `Web.Page` | yes (Get data › SQL Server database, Web, OData feed, SharePoint folder, SharePoint list, Navigator) |
| Data | Databricks connector | P2 | D | `model.refresh {allow_connectors: ["databricks"]}` | yes (ribbon, form) |
| Data | Data source settings: paths, servers, parameters per environment | P1 | D | `sources.list`, `sources.rebase`, `env.apply`, `env.check` | yes (ribbon, form) |
| Data | Privacy levels and stored credentials per source | P2 | P | `source.credentials` (SQL Server login; web sources: anonymous, Basic, Web API key, bearer token per URL prefix; session memory only, never in a file or log); `source.privacy` records None / Public / Organizational / Private per source and `source.settings` lists sources, credential kinds and levels; the privacy firewall is not enforced and levels are not saved in the file; Databricks reads its token from the environment | yes (Home › Data source settings, Edit Permissions, Clear Permissions; Get data's Authentication) |
| Views | DAX query view: write, check and run queries | P1 | D | `dax.validate_query`, `dax.evaluate`, `dax.apply_define` | yes (DAX query view, ribbon, form, result grid) |
| Views | TMDL view: script objects, edit, apply | P2 | D | `tmdl.script {objects}` (createOrReplace, members under `ref table`), `tmdl.apply {script}` (objects replaced with all their descendants, one undo step; a parse, `ref` or DAX error changes nothing and says `line L, column C`); no Preview diff, Problems pane or `TMDLScripts` folder ([formats/tmdl-view.md](formats/tmdl-view.md), G98) | yes (TMDL view: script tabs, Apply, Script model, Copy, Go to the error; Script TMDL on the Data pane and Model explorer menus; the palette) |
| Views | Data view: browse a table's rows | P1 | D | `table.rows` (paged, sort and filter as view state, calculated columns computed); Column tools through `column.set` | yes (Data view: virtualized grid, header menus, Column tools, copy as TSV) |
| Views | Model view and properties pane | P1 | D | `model.get`, `diagram.move`, `relationship.*`, `table.set`, `column.set`, `measure.set` (the diagram is editable, the Properties pane edits the selected object) | yes (Model view, Properties pane) |
| Views | Optimize ribbon (Pause visuals, Refresh visuals, optimization presets) | P2 | D | `report.pause_visuals` (`settings.pauseQueries`), `report.refresh_visuals`, `visual.refresh`, `report.query_reduction` (`slowDataSourceSettings`; default cross-highlighting off, a slicer's own Apply button and the Filters pane's Apply buttons honoured) (G60) | yes (Optimize tab, paused banner, a visual's pending-changes strip, File › Options › Query reduction) |
| Views | Performance analyzer | P2 | D | `perf.start`, `perf.stop`, `perf.clear`, `perf.refresh`, `perf.results`, `perf.render`, `perf.export` (BIcraft's own times: evaluation, render, copy query) | yes (Analyze › Performance analyzer pane) |
| Editing | Undo and redo | P0 | D | `edit.undo`, `edit.redo`, `edit.history` | yes (ribbon, Ctrl+Z, Ctrl+Y) |
| Editing | DAX errors and warnings while editing | P0 | D | `dax.validate`, `model.validate` | yes (formula bar live check, Analyze) |
| Editing | Copy objects between files (measures, tables, pages, visuals, bookmarks, theme) | P1 | D | `objects.import` | yes (ribbon Import, form) |
| Editing | Auto-recovery of unsaved work | P1 | D | `recovery.list`, `recovery.apply`, `recovery.discard` | yes (prompt, File view, palette) |
| Files | Open PBIX, PBIT, PBIP and model.bim | P0 | D | `file.open` | yes (File view, Ctrl+O) |
| Files | Open a Power BI data source file (.pbids) | P2 | D | `file.open` (also `.vpax`, `.pq`/`.m`, `.pqt`, Dataflow Gen2, CDM `model.json`: a new model; `objects.import` sources; no Navigator, G103) | yes (File view, Open dialog's Other sources) |
| Files | New blank file | P0 | D | `model.new`, `report.new`, `project.new` | yes (File view, Ctrl+N) |
| Files | Save a PBIX with model, report and data | P0 | D | `file.save`, `file.save_as` | yes (File view, Ctrl+S) |
| Files | Save as a template (PBIT) | P1 | D | `file.save_as`, `file.convert` | yes (File view) |
| Files | Save as a project (PBIP: TMDL or TMSL, PBIR) | P1 | D | `file.save_as`, `file.convert`, `project.new` | yes (File view) |
| Files | Upgrade a legacy report layout to PBIR | P2 | D | `file.save_as`, `file.convert` | yes (File view) |
| Files | Import an Excel Power Pivot workbook | P2 | P | `file.open` (`.xlsx`, `.xlsm`; then `file.save_as`, `file.convert`; checked on a workbook Excel 16.0 saved; connections, calculated columns and hierarchies from Excel not yet, G100) | yes (File view, Ctrl+O) |
| Files | Options dialog: the global and current-file settings (data load, regional settings, privacy, auto recovery, semantic model settings, query reduction, report settings) | P1 | P | `report.settings` (the Report settings section), `report.query_reduction`, `model.set` (culture, implicit measures); the other sections have no page | yes (File › Options and settings: Report settings only) |
| Pages | Add, duplicate, remove, rename, reorder pages; set the opening page | P0 | D | `page.add`, `page.remove`, `page.rename`, `page.move`, `page.set_active`; duplicate is `report.extract` then `report.apply` | yes (page tabs) |
| Pages | Hide a page; page size and display option | P1 | D | `page.add {width, height, display_option}`, `page.set_type {hidden, width, height}` | yes (page tabs, form) |
| Pages | Page background and wallpaper | P1 | D | `page.format {background_*, wallpaper_*, size, vertical_alignment}` (colour, transparency, image, fit) | yes (Report › Page format window) |
| Pages | Drillthrough pages (fields, keep all filters, cross-report, back button) | P1 | D | `page.set_type {type: "drillthrough"}` | yes (page tabs, form) |
| Pages | Report page tooltips | P1 | D | `page.set_type {type: "tooltip"}`, `visual.tooltip_page` | yes (page tabs, ribbon, form) |
| Pages | Mobile (phone) layout | P2 | D | `visual.mobile` | yes (ribbon, form) |
| Visuals | Add, move, resize, remove visuals | P0 | D | `visual.add`, `visual.set_position`, `visual.move`, `visual.remove` | yes (drag, handles, arrows, Delete, ribbon, form) |
| Visuals | Bind fields to data roles (role rules checked per type), a new visual from a field | P0 | D | `visual.bind`, `visual.unbind`, `visual.move_field`, `visual.bind_hierarchy` | yes (Data and Build panes, drag and drop) |
| Visuals | Change a visual's type, keeping its fields | P0 | D | `visual.set_type` | yes (Build pane) |
| Visuals | Summarize a field; rename a field for this visual | P1 | D | `visual.set_aggregation`, `visual.rename_field` | yes (Build pane) |
| Visuals | Show value as: percent of grand total, of column, of row | P1 | D | `visual.show_value_as` (PBIR Arithmetic/ScopedEval; percentages from evaluated totals) | yes (Build pane well menu) |
| Visuals | Show items with no data | P1 | D | `visual.show_items_no_data` (role showAll) | yes (Build pane well menu) |
| Visuals | Copy, cut, paste and duplicate visuals (Ctrl+C, Ctrl+X, Ctrl+V), within and across pages | P0 | D | `visual.copy`, `visual.cut`, `visual.paste` (`x`, `y` for the menu's paste at the pointer), `visual.duplicate` (across pages and files; missing fields kept as broken references) | yes (Ctrl+C, Ctrl+X, Ctrl+V, Ctrl+D, menus, Home Paste) |
| Visuals | Charts: bar, column, line, area, combo, ribbon, waterfall, funnel, scatter, pie, donut, treemap | P0 | D | `visual.add {visual_type}` | yes (Build pane type picker) |
| Visuals | Cards, KPI, gauge, table, matrix | P0 | D | `visual.add {visual_type}` | yes (Build pane type picker) |
| Visuals | Slicers: list, dropdown, tiles, range, relative date, button, text | P0 | D | `visual.add {visual_type}`, `visual.format` | yes (Build pane type picker) |
| Visuals | Maps: map, filled map, shape map, Azure Maps | P2 | D | `visual.add {visual_type}` (roles checked; wells and format cards, docs/formats/maps.md), `shape_map.set_map` (a custom TopoJSON or GeoJSON map stored as a `ShapeMap` resource, or `builtin: world_countries`: Natural Earth's countries written as an ordinary custom TopoJSON map; PBIP and PBIR PBIX; a legacy-layout PBIX refuses it on save) | yes (Build pane type picker; Format › Map settings › Add a map type, World countries (Natural Earth)) |
| Visuals | AI visuals: key influencers, decomposition tree, Q&A, smart narrative | P2 | P | `visual.add` (roles checked for the first two only) | yes (Build pane type picker) |
| Visuals | Gantt chart (core visual, preview since October 2026) | P2 | P | `visual.add {visual_type: "ganttChart"}` and `visual.format` on its objects (`bar`, `milestone`, `progress`, `axis`, `row`, columns), the type and objects from the public report theme schema 2.159; its seven wells (`Task`, `StartDate`, `EndDate`, `Progress`, `Category`, `Columns`, `Tooltips`) from a wells test against Desktop 2.159.1231.0; drawn as a placeholder ([formats/gantt.md](formats/gantt.md)) | yes (gallery, Build pane wells, Format visual cards) |
| Visuals | Text boxes with formatted text | P0 | D | `textbox.add`, `textbox.set` (runs: font, size, bold, italic, underline, colour, links; alignment, lists; dynamic values) | yes (Insert › Text box, editor in place with its toolbar) |
| Visuals | Buttons, shapes, images, navigators | P1 | D | `button.add` (Desktop's Buttons list, navigators), `visual.set_action` (every action type but Data function; conditional fx destination and tooltip `destination_field` / `tooltip_field`; drillthrough enabled / disabled tooltips), Default / On hover / On press / Disabled / Selected states through `visual.format {state}` drawn by the readers under the pointer, drillthrough buttons disabled without a selected point, `image.add` / `image.set` (stored as a registered resource); page and bookmark navigators applied by the readers, Allow deselection (`view.interact {action: "bookmark_deselect"}`); `shape.add` (23 tile shapes), fill, outline, rotation and text through `visual.format`; [formats/buttons.md](formats/buttons.md) | yes (Insert › Buttons, Shapes, Image; Visual › Action with fx and the button or shape style; Format pane cards with Apply settings to: State) |
| Visuals | Group and ungroup visuals | P2 | D | `visual.group`, `visual.ungroup` (groups move and scale their members) | yes (Visual tab Arrange) |
| Visuals | Layer order and alignment | P1 | D | `visual.order`, `visual.align`, `page.arrange`, `visual.place` | yes (Visual tab Arrange menus, form) |
| Visuals | Custom visuals (.pbiviz import) | P1 | D | `visual.import_custom {path or data}` (a .pbiviz read as hostile input, stored as Desktop stores an embedded visual), `visual.remove_custom`, `report.custom_visuals`; never run: drawn as a placeholder with its icon and name (G80) | yes (Visualizations pane gallery cell with its icon, … › Import a visual from a file, Remove a custom visual; Insert › Import a visual) |
| Visuals | Custom visual projects (pbiviz new / package) | P2 | D | `visual.scaffold` (a project with the data roles and format options chosen), `visual.build` (its TypeScript compiled and bundled with swc, no Node), `visual.pack` (the `.pbiviz` that `visual.import_custom` reads), `bicraft-cli visual new`, `build` and `pack` (M11.30); `visual.export_bicraft_visual` and `bicraft-cli visual export` write BIcraft's own visuals as `.pbiviz` files (M12.5); a packaged visual not yet opened in Desktop | yes (Insert › Visuals › New visual project, Compile a visual, Package a visual, Export a BIcraft visual) |
| Visuals | AppSource visuals (Get more visuals) | P1 | O | — (Microsoft's online store: network and a signed-in service, G80) | – |
| Visuals | Visual calculations | P2 | P | `visual.calculation.add`, `visual.calculation.set` (expression, rename, hide any field), `visual.calculation.remove` (PBIR `NativeVisualCalculation`; axes, resets, window shortcuts, COLLAPSE/EXPAND, ISATLEVEL, RANGE evaluated; LOOKUP and the window functions not, G71) | yes (More options › New calculation: edit window with templates, formula bar, Hide switches, visual matrix) |
| Visuals | Python and R visuals | P2 | O | — (BIcraft never runs scripts) | – |
| Visuals | Sparklines in table and matrix cells (Insert › Add a sparkline) | P2 | P | `visual.add_sparkline` (Y axis, summarization, X axis, line or column, colour, width, markers; at most 5 per visual), `visual.edit_sparkline`, removed by `visual.unbind`: a `SparklineData` projection and an `objects.sparklines` entry (public schemas), evaluated per row (at most 52 points) and drawn in the cell by bicraft_render ([formats/sparklines.md](formats/sparklines.md)); the queryRef spelling and marker bits are BIcraft's, not checked against Desktop (G106) | yes (Insert › Sparklines › Add a sparkline; a value field's menu in the Build pane › Add a sparkline…, a sparkline's › Edit sparkline…; Format › Sparklines card) |
| Visuals | Analyze a data point: explain an increase or decrease, find where a distribution differs (the visual's context menu, seen in Desktop 2.159.1231.0) | P2 | P | `visual.analyze {page, visual, point, kind?}`: BIcraft's own ranking of the model's columns (the change against the previous point of a date or number axis; the distribution under the point against the other points), not Desktop's algorithm and not compared with it; a list, not a visual (G62, [analyze-point.md](analyze-point.md)) | yes (a data point's right-click › Analyze this point, the answer window's list) |
| Formatting | Title, subtitle, background, border, shadow, padding, header | P0 | D | `visual.set_title`, `visual.format` | yes (Format visual General cards, palette form) |
| Formatting | Axes, legend, data labels, gridlines, display units | P0 | D | `visual.axis`, `visual.format` | yes (Format visual cards, palette form) |
| Formatting | Series and category colours, line styles | P0 | D | `visual.series` | yes (Format visual Data colours card, palette form) |
| Formatting | Any format pane property by name, checked against the type's catalog | P1 | D | `visual.format {objects}`, `visual.options` | yes (ribbon, form, JSON) |
| Formatting | Conditional formatting: scales, rules, field value, icons, data bars, web URL | P1 | D | `visual.conditional` | yes (Format visual fx buttons, palette form) |
| Formatting | Analytics lines: constant, average, min, max, median, percentile, trend, forecast, error bars, anomaly detection (BIcraft's estimate), ratio line, symmetry shading | P2 | D | `visual.analytics`; `visual.explain_anomaly` (an anomaly's explanations, BIcraft's own ranking) | yes (Analytics tab, ribbon, form; an anomaly's right-click › Explain the anomaly) |
| Formatting | Small multiples | P2 | D | `visual.small_multiples` | yes (ribbon, form) |
| Formatting | Sort a visual | P1 | D | `visual.set_sort` | yes (menu, form) |
| Formatting | Format painter | P2 | D | `visual.copy_format {page, from, to}` (one undo step; what the December 2015 Desktop post lists: title but its text, data labels but units and decimals, axes but start and end, legend but its text, colours of the same field, borders, backgrounds, slicers; options the target lacks dropped; fields, filters, size and position kept; [formats/format-painter.md](formats/format-painter.md); not compared with a Desktop run) | yes (Home › Format painter toggle: the next click on a visual applies, Esc cancels; no double-click lock, which Microsoft does not document) |
| Formatting | Accessibility: alt text and tab order | P1 | D | `visual.format {alt_text}`, `visual.order` | yes (ribbon, form) |
| Formatting | The Format pane's Data format card (General tab, seen in Desktop 2.159.1231.0) | P2 | D | `visual.set_data_format` (auto, general, whole, decimal, percentage, currency, scientific, date, custom; decimals, thousands separator, currency symbol): the projection's visual-scoped `format` (public PBIR schema; PBIX legacy layout `columnProperties.formatString`), the model unchanged; the values drawn with it ([formats/sparklines.md](formats/sparklines.md)) | yes (Format › General › Data format: Apply settings to, Format, Decimal places, Thousands separator, Currency symbol, Date format, Format code, Apply) |
| Filters | Visual-level filters: basic, advanced, top N, hidden, locked | P0 | D | `visual.filter`, `visual.clear_filters` | yes (Filters pane) |
| Filters | Page-level and report-level filters | P0 | D | `page.filter`, `page.clear_filters`, `report.filter`, `report.clear_filters` (columns; basic, advanced, relative; Top N refused, visual-only as in Desktop) | yes (Filters pane) |
| Filters | Relative date and relative time filtering | P1 | D | `relative` param of `visual.filter`, `page.filter`, `report.filter` (PBIR RelativeDate / RelativeTime) | yes (Filters pane) |
| Filters | Lock and hide filter cards | P1 | D | `*.filter {locked, hidden}` (`isLockedInViewMode` / `isHiddenInViewMode`); readers honour them | yes (card icons) |
| Filters | Filter pane formatting and card order | P2 | D | `page.format_filter_pane {card}` (the pane, available or applied cards: `objects.outspacePane`, `objects.filterCard`; theme defaults; PBIR and legacy layout); `filter.order {sort, field, to}` (`filterSortOrder`, `ordinal`: A to Z, Z to A, custom) | yes (Format tab, nothing selected: Filter pane, Filter cards; Filters pane: drag a card, each section's sort list) |
| Filters | Sync slicers across pages | P1 | D | `slicer.set_sync` (`syncGroup` per slicer), `slicer.sync_list`; the viewers apply them, fields too (`fieldChanges`, 2026-11-18) | yes (Sync slicers pane) |
| Interactions | Edit interactions (filter, highlight, none) | P1 | D | `page.interactions` | yes (Visual tab Edit interactions mode, icons on the canvas) |
| Interactions | Bookmarks: create, update, group, bookmark navigator | P1 | D | `bookmark.capture`, `bookmark.update`, `bookmark.group`, `bookmark.ungroup`, `bookmark.rename_group`, `bookmark.set`, `bookmark.remove`, `bookmark.list`, `bookmark.apply`; applied by the readers, the bookmark navigator and Bookmark buttons (`view.interact {action: "bookmark"}`, 2026-11-18); slicer selections captured and restored, navigators drawn in their per-state styles (2026-11-19); drill states not captured (G55) | yes (Bookmarks pane) |
| Interactions | Drill: hierarchies on axes, saved expand state | P2 | D | `visual.bind`, `visual.bind_hierarchy` (hierarchy levels); `view.keep_drill` writes the reader's drill and expansion as Desktop saves them, `visual.expand` edits a matrix's expansion directly (documented, not measured) | yes (Build pane, Visual menu) |
| Themes | Custom theme JSON: install, export, remove | P1 | D | `theme.set`, `theme.import`, `theme.export`, `theme.remove`, `theme.get` | yes (ribbon, form) |
| Themes | Customize the theme: colours, text classes, visual styles | P1 | D | `theme.set_colors`, `theme.set_text_classes`, `theme.set_visual_style`, `theme.customize` (all in one step) | yes (ribbon, form) |
| Themes | Built-in theme gallery | P2 | D | `theme.set {builtin}`: BIcraft's own eight themes (contrast-checked; not Power BI's), installed as a custom theme; Browse for themes (`theme.import`), Customize current theme (the Customize theme dialog, `theme.customize`), Save current theme | yes (View › Themes gallery with swatches) |
| Report | Report-level measures | P2 | D | `report_measure.add`, `report_measure.set`, `report_measure.remove`, `report_measure.list` | yes (ribbon, form) |
| Report | Report settings (persistent filters, personalization, cross-report drillthrough) | P2 | D | `report.settings {default_interaction, drill_filters_other_visuals, export_data, persistent_filters, personalize_visuals, cross_report_drillthrough, filter_pane_visible, allow_change_filter_types}` (PBIR and legacy layout); the service-side ones are written, not acted on by BIcraft's viewer (G63) | yes (File › Options and settings; the Filters pane's eye; Visual tab Interaction settings) |
| Report UI | Data, Build and Filters panes with drag and drop | P0 | D | the commands of the Visuals and Filters rows | yes (panes) |
| Report UI | DAX formula bar: colours, autocomplete, signature help, live check | P0 | D | `measure.set`, `column.set`, `measure.add`, `dax.validate`, `dax.functions` | yes (formula bar) |
| Report UI | Canvas: drag to move, resize handles, multi-select, align and distribute | P0 | D | `visual.move`, `visual.align`, `visual.order`; smart guides, grid snap, arrow nudges, rubber band, Ctrl+A | yes (canvas, Visual tab Arrange) |
| Report UI | Format pane: cards per visual type, search, reset to default | P0 | D | `visual.set_title`, `visual.format`, `visual.axis`, `visual.tooltip_page`, `image.set`, `visual.props` (the Build pane's Format visual tab, M11.22); other types and any property through Format visual's form | yes (Build pane Format visual, ribbon Format) |
| Report UI | Analytics pane | P2 | D | `visual.analytics` (the Visualizations pane's Analytics tab, G56) | yes (Analytics tab) |
| Report UI | Conditional formatting dialog (gradient, rules, field value, preview) | P1 | D | `visual.conditional`: the fx buttons' dialog on data colours, table and matrix values and a card's callout (2026-11-18, G54), titles, subtitles, background, border, data labels and a card's category label, a title's text from a field (2026-11-19); 2026-11-20: a table's or matrix's Cell elements card opens the data bars, icons and web URL dialogs; the gradient's numbers, middle colour (diverging) and empty values, Apply to (values, totals), rules with a second condition, Reverse colour order, icon sets, layout and alignment, data bars' colours, axis, direction and Show bar only, and a live preview; rules' Percent bounds and the gradient's Percent and Percentile stops are not offered (no public shape) | yes (Format visual fx buttons, Cell elements, palette, form) |
| Report UI | Edit interactions mode: filter, highlight and none icons on the other visuals | P1 | D | `page.interactions` (one rule per icon click), `report.settings` (cross-filter by default) | yes (Visual tab toggle, icons on the canvas) |
| Report UI | Selection pane: layers, show and hide, rename, tab order | P1 | D | `visual.set_hidden`, `visual.rename_group`, `visual.tab_order`, `visual.order`, `visual.group`, `visual.ungroup` | yes (Selection pane, Visual tab Arrange) |
| Report UI | Bookmarks pane | P1 | D | `bookmark.capture`, `bookmark.update`, `bookmark.set`, `bookmark.remove`, `bookmark.group`, `bookmark.ungroup`, `bookmark.rename_group`, `bookmark.apply` | yes (Bookmarks pane) |
| Report UI | Sync slicers pane | P1 | D | `slicer.set_sync`, `slicer.sync_list` | yes (Sync slicers pane) |
| Report UI | Visual header in editing: more options, focus mode, sort, show as table | P1 | D | More options (⋯) and right-click menu: `visual.export_data`, Delete, Sort axis (`visual.set_sort`), Focus mode and Show as a table (`view.interact` focus / show_table); the Focus icon in the header | yes (header, menu, Esc) |
| Report UI | Mobile layout view (phone canvas, drag visuals onto it) | P2 | D | `visual.mobile`, `visual.mobile_format`, `visual.props` `layout` | yes (View › Mobile layout, Page visuals pane, drag, handles, Delete, X, Format pane; G72) |
| Report UI | Customize theme dialog | P1 | D | `theme.customize` (one undo step): the dialog of Microsoft Learn's Customize current theme, read from the snapshot's theme: Name and colors (name, theme colours with More colours, sentiment, divergent; Advanced: structural colours), Text (General, Title, Cards and KPIs, Tab headers: family, size, colour), Visuals (background, border, header, tooltips), Page (wallpaper, page background), Filter pane (pane, available and applied cards); not compared with Desktop's dialog run (G73) | yes (View › Themes › Customize current theme, ribbon, palette) |
| Report UI | View options: gridlines, snap to grid, lock objects, zoom slider, page thumbnails | P2 | D | `page.set_display_option` (View › Page view: Fit to page, Fit to width, Actual size, saved as the page's `displayOption`, PBIR and legacy layout); Gridlines, Snap to grid (moves and resizes) and Lock objects (no move, resize or nudge; selection and menus kept) as window preferences, independent, as Microsoft Learn describes them (the report schema has no property for them); the status bar's zoom: Fit page, Fit width, Actual size, Zoom out, the slider (10–400 %), Zoom in, the percentage; page thumbnails: none, Learn describes none in Desktop, whose pages are tabs below the canvas (the page tabs); Smart guides (on by default) turns snapping to the other visuals and the page, and the guides, off; the grid is drawn under the visuals | yes (View › Page view menu, View › Gridlines, Snap to grid, Smart guides, Lock objects; status bar) |
| Model UI | Diagram: drag cards, drag a column onto another to create a relationship | P1 | D | `diagram.move`, `relationship.add`; double-click `relationship.set`, Delete `relationship.remove` | yes (Model view) |
| Model UI | Model explorer: the semantic model's tree (calculation groups and items, cultures, measures, perspectives, relationships, roles, shared expressions, tables), its menus, calculation items ordered by drag | P2 | P | `calculation_item.set {ordinal}`, `calculation_group.*`, `role.*`, `culture.*`, `expression.*`, `relationship.*`, renames and removals; perspectives listed only, no hierarchies, functions or data sources nodes, no search | yes (Model view, Data / Model pane) |
| Model UI | Properties pane: edit one or several selected objects in place | P1 | D | `model.set_properties` (Ctrl+click in the diagram, Ctrl or Shift+click in the Data pane; the shared properties, mixed values blank, one change on all in one undo step: description, display folder, is hidden, format, data category, summarize by, sort by column, storage mode), synonyms of one object (Q&A schema), one object or relationship through `table.set`, `column.set`, `measure.set`, `relationship.set` and renames; left: key column, row label, featured table | yes (Properties pane) |
| Model UI | Manage relationships dialog with a data preview | P1 | D | `relationship.list` (the list: active box, from, to, cardinality, cross filter), `relationship.add` / `relationship.set` with `check: true` (the review as columns are picked: suggested cardinality and direction from the data, duplicates on the one side, no unique side, many-to-many warning, made inactive when ambiguous), `new_from` / `new_to`, `assume_referential_integrity`, `security_filtering`; `table.rows` for the previews; `relationship.remove`, `relationship.autodetect` | yes (Modeling › Manage relationships, New, Edit, Delete, Autodetect; a double click on a line) |
| Model UI | Manage roles dialog: tables, DAX filter editor, check | P1 | D | Manage security roles (2026-11-20): roles (New, Rename `role.rename`, Delete), Select tables (filtered and hidden marked), Filter data in the default editor (column / condition / value rows, And / Or, written as DAX) or the DAX editor (colours, completion), Check `role.check_filter` (errors and warnings listed, nothing saved), Revert, Save `role.set_filter`; Hide table and columns (`role.set_permission`) | yes (Model explorer, Roles menu › Manage roles, dialog) |
| Model UI | DAX query view as a view: query tabs, editor, results grid, update the model from DEFINE | P1 | D | the DAX query view (`crates/ui/src/dax_query.rs`): query tabs per document (new, rename, duplicate, close; view state), the DAX editor with the formula bar's colours, Run / Ctrl+Enter / F5 through `dax.evaluate`, the results grid with its row count and errors inline; `dax.apply_define` (Update model: Add new measure / Overwrite measure per DEFINE MEASURE, Update model with changes for all, one undo step); the Data pane with insert-on-click and quick queries, Comment (Ctrl+/), Find and replace (`crates/ui/src/dax_query_tools.rs`); Define all measures and Define with references (`dax.define_query`), hierarchies and calculation items, fields dragged into the editor, lenses on their own line; left in G81 (queries not saved into a PBIX, no Model tab) | yes (status bar view switcher, Analyze › Run DAX query and Update model, the tool bar's Update model with changes, the lenses) |
| Model UI | Editing tools of the DAX query and TMDL editors: format, comment and uncomment, find and replace (their ribbon's Formatting and Editing groups) | P2 | M | — (the command palette exists; the two editors have no format, comment or find and replace) | no |
| Accessibility | App keyboard navigation and screen readers (chrome, panes, dialogs) | P1 | D | view state (`crates/ui/src/keyboard.rs`, 2026-11-20): Ctrl+F6 / Ctrl+Shift+F6 between the window's sections (ribbon, formula bar, view, each pane, page tabs, status bar), F6 to the Selection pane, Tab / Shift+Tab inside a section, arrows between controls, Right / Left to unroll and roll the Data pane's tables, Enter / Space; dialogs modal (Tab trapped, Escape closes, the focus returns to the opener); a 2 px focus ring at 4.5:1 in every theme; every control of the views and dialogs named in the AccessKit tree (EN, FR); left in G84 (Alt key tips, Shift+?, Shift+F10, not run with a real screen reader); visuals on the canvas: the Reader tools row | yes (shell) |
| Accessibility | High-contrast mode in the app and for reports | P2 | D | view state, no file change: the high-contrast UI theme (7:1 text tokens; the browser follows the system's `forced-colors` / `prefers-contrast`, the desktop does not detect Windows contrast themes) and report pages drawn with a high-contrast palette | yes (Home › High contrast, View › High-contrast colours) |
| Localisation | App UI languages (Desktop ships 40+) | P2 | P | English and French chrome; report text in 59 cultures (M10) | yes (shell) |
| Tools | External tools: launch DAX Studio or Tabular Editor on the open model (local XMLA port) | P2 | M | — (BIcraft exposes CLI and MCP, not an XMLA endpoint) | no |
| Out of scope | Publish to the Power BI service, workspaces, apps, sharing | P0 | O | — (excluded by the task) | – |
| Out of scope | Thin reports on a published semantic model (live connection) | P2 | O | — (needs the service) | – |
| Out of scope | Sign-in, Fabric items, Direct Lake editing, dataflows, datamarts | P2 | O | — (needs the service) | – |
| Out of scope | Scheduled refresh, gateways, deployment pipelines, Git integration | P2 | O | — (service features) | – |
| Out of scope | Copilot and Skills for Fabric | P2 | O | — | – |
| Out of scope | Sensitivity labels | P2 | O | — (needs the service) | – |
| Out of scope | Service-bound items seen in Desktop 2.159.1231.0: metrics (goals), paginated report, Power Apps, Power Automate and ArcGIS visuals; OneLake catalog and Dataverse sources; Copilot's Summarize and Prepare data for AI; Q&A verified answers; the online theme gallery | P2 | O | — (need the service or a sign-in) | – |

## Viewer rows

The app's drill, matrix expand and collapse, drillthrough and report-page tooltip rows (2026-11-06,
M11.19, G51) follow Microsoft Learn's pages: **documented, not measured** in Desktop.

| Area | Feature | Priority | render | web | app |
|---|---|---|---|---|---|
| Visuals | Cards: card, new card, multi-row card | P0 | D | D | D |
| Visuals | KPI and gauge | P0 | D | D | D |
| Visuals | Bar and column charts: clustered, stacked, 100 % | P0 | D | D | D |
| Visuals | Line, area, stacked area, line-and-column combos | P0 | D | D | D |
| Visuals | Pie, donut, treemap, funnel, waterfall | P0 | D | D | D |
| Visuals | Scatter and bubble charts | P1 | D | D | D |
| Visuals | Ribbon chart | P2 | D | P | D |
| Visuals | Table: totals, data bars, icons, URLs, column styles | P0 | D | D | D |
| Visuals | Matrix: subtotals at every level, expand and collapse | P0 | D | D | D |
| Visuals | Slicers, every kind | P0 | D | D | D |
| Visuals | Text boxes, shapes, buttons, images, page navigator | P0 | D | D | D |
| Visuals | Bookmark navigator | P2 | D | M | D |
| Visuals | Maps: map, filled map, shape map, Azure Maps | P1 | P | M | P |
| Visuals | Decomposition tree | P2 | D | M | D |
| Visuals | Key influencers | P2 | P | M | P |
| Visuals | Q&A and smart narrative | P2 | M | M | M |
| Visuals | Gantt chart (preview) | P2 | M | M | M |
| Visuals | Custom visuals (.pbiviz) | P1 | P | M | P |
| Visuals | Python and R visuals | P2 | O | O | O |
| Visuals | Power Apps, Power Automate, paginated report and metrics visuals (service) | P2 | O | O | O |
| Visuals | Small multiples | P2 | D | D | D |
| Visuals | Reference and trend lines, log axes, axis bounds, data labels, legends | P1 | D | D | D |
| Visuals | Conditional formatting: colours, icons, data bars | P1 | D | D | D |
| Page | Theme, page background, wallpaper, visual containers | P0 | D | D | D |
| Page | Numbers and dates in the culture, display units, format strings | P0 | D | D | D |
| Page | Model translations and the language of generated text | P2 | D | D | D |
| Page | Fit to page, fit to width, zoom | P1 | D | P | D |
| Data | Visuals show the model's data (stored data or BIcraft's evaluator) | P0 | D | D | D |
| Data | Every DAX function a measure uses is evaluated (window functions, balances, `GROUPBY` and natural joins since lot 20; `ADDMISSINGITEMS`, `SUBSTITUTEWITHINDEX`, `SAMPLE`, `TOPNSKIP` since 2026-11-20; the `*WEEK` balances, visual-calculation forms, `ADDMISSINGITEMS` subtotals, `TOPNSKIP` ties and `SEARCH` wildcards are not) | P1 | P | P | P |
| Data | Report, page and visual filters applied | P0 | D | D | D |
| Data | View as a role (test RLS) | P1 | D | D | D |
| Interactions | Slicer selection: click, Ctrl+click, select all, range, date and text popups | P0 | – | D | D |
| Interactions | Cross-filter and cross-highlight from bars, columns, lines, pies, funnels, treemaps, table rows | P0 | – | D | D |
| Interactions | Point selection in matrices, scatter, waterfall, ribbon, gauges, KPIs, cards, small multiples | P1 | – | M | M |
| Interactions | Edit-interactions rules followed (filter, highlight, none) | P1 | – | D | D |
| Interactions | Synced slicers | P1 | – | D | D |
| Interactions | Reader state drawn: dimmed points, highlights, selections | P1 | D | D | D |
| Interactions | Apply all and Clear all slicers buttons | P2 | – | D | D |
| Interactions | Default tooltips | P0 | – | D | D |
| Interactions | Report page tooltips | P1 | – | M | D |
| Interactions | Drill down, drill up, expand to the next level | P0 | – | M | D |
| Interactions | Drillthrough (context menu, button, back) | P1 | – | M | D |
| Interactions | Bookmarks: apply, navigator, bookmark buttons | P1 | – | M | D |
| Interactions | Other button actions: Q&A, web URL | P2 | – | M | P |
| Navigation | Page tabs, hidden pages left out | P0 | – | D | D |
| Navigation | Page navigator, page and back buttons | P0 | – | D | D |
| Reader tools | Filters pane: see and change report, page and visual filters | P0 | – | M | D |
| Reader tools | Focus mode | P1 | – | M | D |
| Reader tools | Spotlight | P2 | – | M | D |
| Reader tools | Show as a table, see records | P1 | – | P | D |
| Reader tools | Sort a visual from its header | P1 | – | M | D |
| Reader tools | Visual header: more options, drill icons | P1 | – | M | D |
| Reader tools | Mobile (phone) layout | P2 | M | M | D |
| Reader tools | Keyboard navigation and screen readers on visuals | P1 | – | P | P |
| Reader tools | High-contrast rendering | P2 | M | M | D |
| Export | Export to PDF | P1 | D | M | D |
| Export | Export a visual's data (CSV, Excel) | P1 | – | M | D |
| Export | Print | P2 | – | M | D |
| Out of scope | Export to PowerPoint, Analyze in Excel (service) | P2 | O | O | O |
| Out of scope | Comments, subscriptions, alerts, personal bookmarks (service) | P2 | O | O | O |
| Out of scope | Automatic page refresh (DirectQuery) | P2 | O | O | O |
| Out of scope | Personalize visuals (service) | P2 | O | O | O |

## Desktop UI inventory

ROADMAP M11.28 and M11.29. Power BI Desktop **2.159.1231.0** (Microsoft Store edition, French UI)
read as a black box on **2026-11-23**, on a synthetic one-table report written by `bicraft-cli run`:
the web views driven with input events only (DevTools `Input.dispatchMouseEvent` and
`dispatchKeyEvent`) and read from what they render (the accessibility tree, the rendered
elements' labels and text, screenshots); the native windows (Options, Get data, the Power Query
editor) read from window captures and the Windows accessibility tree and driven with mouse
messages. No script, network payload or binary of Desktop was read (AGENTS.md §1). The captures
stay outside the repository (§2); only this list is kept, in our own words.

One row per ribbon group, pane, menu or dialog. **Controls** is the number of labelled controls
counted in that group (**measured**; a split button counts once, a gallery's items each).
**Status** is BIcraft's for the group as a whole, a judgement against the rows above: **D** every
control has a done row and a button, **P** some do, **M** none, **O** out of scope (service,
sign-in, links to Microsoft's sites). Not reached: the Manage relationships dialog (disabled with
a single table), the contextual tabs of other visual types, the Format pane cards of visual types
other than the clustered column chart, the inside of each Options section, the Get data list past
its first page, the Power Query editor's Help tab and the right end of its Home tab.

| Surface | Group and its controls | Controls | Status | Checklist row |
|---|---|---|---|---|
| Report ribbon | Home › Clipboard: paste, cut, copy, format painter | 4 | D | Copy, cut, paste; Format painter |
| Report ribbon | Home › Data: get data, Excel workbook, OneLake catalog, SQL Server, enter data, Dataverse, recent sources | 7 | P | Get data; Enter data; OneLake and Dataverse out of scope |
| Report ribbon | Home › Queries: transform data (editor, data source settings), refresh | 2 | D | Power Query editor; Refresh |
| Report ribbon | Home › Insert: new visual, text box, more visuals | 3 | D | Add visuals; Text boxes; Custom visuals |
| Report ribbon | Home › Calculations: new visual calculation, new measure, quick measure | 3 | P | Visual calculations (P); Measures; Quick measures |
| Report ribbon | Home › Sensitivity, Share, Copilot: sensitivity label, publish, prepare data for AI, Copilot | 4 | O | Out of scope |
| Report ribbon | Insert › Pages: new page | 1 | D | Pages |
| Report ribbon | Insert › Visuals: new visual, more visuals, visual type menu | 3 | D | Add visuals |
| Report ribbon | Insert › AI visuals: key influencers, decomposition tree, narrative | 3 | P | AI visuals (P) |
| Report ribbon | Insert › Power Platform: paginated report, Power Apps, Power Automate | 3 | O | Out of scope |
| Report ribbon | Insert › Elements: text box, buttons, shapes, image | 4 | D | Text boxes; Buttons, shapes, images |
| Report ribbon | Insert › Sparklines: add a sparkline | 1 | P | Sparklines (P) |
| Report ribbon | Modeling › Relationships: manage relationships | 1 | D | Manage relationships dialog |
| Report ribbon | Modeling › Calculations: new visual calculation, new calculation group, new measure, quick measure, new column, new table | 6 | P | Visual calculations (P); the rest done |
| Report ribbon | Modeling › Calendars: mark as date table | 1 | D | Table properties |
| Report ribbon | Modeling › Page refresh: change detection | 1 | O | Automatic page refresh (DirectQuery) |
| Report ribbon | Modeling › Parameters: new parameter (numeric range, fields) | 1 | D | Field and what-if parameters |
| Report ribbon | Modeling › Security: manage roles, view as | 2 | D | Roles; View as a role |
| Report ribbon | Modeling › Q&A: Q&A setup, language, linguistic schema | 3 | P | Q&A linguistic schema (P) |
| Report ribbon | Design › Themes: current theme, reset to default, four gallery themes, themes menu, online theme gallery, customize, save current theme, import theme | 11 | D | Themes rows (the online gallery out of scope) |
| Report ribbon | Design › Customize page: page size, page background, wallpaper | 3 | D | Page size; Page background |
| Report ribbon | Design › Help: how to make a theme (link) | 1 | O | Link |
| Report ribbon | View › Scale to fit: page view | 1 | D | View options |
| Report ribbon | View › Mobile: mobile layout | 1 | D | Mobile layout view |
| Report ribbon | View › Page options: gridlines, snap to grid, lock objects | 3 | D | View options |
| Report ribbon | View › Show panes: filters, bookmarks, selection, performance analyzer, sync slicers | 5 | D | Panes rows; Performance analyzer |
| Report ribbon | Optimize: pause visuals, refresh visuals, optimization presets, performance analyzer, apply-all-slicers button | 5 | P | Optimize ribbon (G60: the Apply buttons are not shown) |
| Report ribbon | Help: about, guided learning, training videos, documentation, support, blog, community, developers, submit an idea, external tools, samples, consulting services | 12 | O | Links |
| Report ribbon | External tools: the registered tools (one installed here) | 1 | M | External tools |
| Visual tabs | Format › Interactions: edit interactions, drill filters apply to | 2 | D | Edit interactions; Report settings |
| Visual tabs | Format › Arrange: bring forward, send backward, layer menu, selection, align, group | 6 | D | Layer order and alignment; Group |
| Visual tabs | Data and drill › Show: visual table, data point table | 2 | D | Show as a table, see records |
| Visual tabs | Data and drill › Interactions: drill filters apply to | 1 | D | Report settings |
| Visual tabs | Data and drill › Drill actions: drill to next level, expand next level, drill up, drill down, drillthrough | 5 | D | Drill; Drillthrough |
| Visual tabs | Data and drill › Data groups: groups | 1 | D | Groups and bins |
| Visual tabs | Data and drill › AI: find anomalies | 1 | D | Analytics lines (anomaly detection, BIcraft's estimate) |
| Table view | Home ribbon: clipboard (3), data (7), queries (2), relationships, calculations (4), security (2), sensitivity, publish | 21 | P | As the report view's Home and Modeling groups |
| Table view | Table tools: name, manage relationships, new measure, quick measure, new column, new table, mark as date table | 7 | D | Tables; Measures; Columns |
| Table view | Column tools: name, data type, format, currency, decimal places, percent, thousands separator, summarization, data category, sort by column, data groups, manage relationships, new column | 13 | D | Columns rows |
| Table view | Measure tools: name, home table, format, currency, decimal places, percent, thousands separator, data category, new measure, quick measure | 10 | D | Measures rows |
| Table view | Grid: rows with column filters and sort, formula bar, table status line | 3 | D | Data view |
| Model view | Home ribbon: clipboard (3), data (7), queries (2), relationships, calculations (measure, column, table, calculation group), parameters, security (2), Q&A (3), sensitivity, publish | 25 | P | As the report view's Home and Modeling groups |
| Model view | Diagram: table cards with columns and measures, layout tabs (all tables, new layout), zoom out, zoom in, reset layout, fit to page | 7 | D | Diagram; Saved layouts |
| Model view | Properties pane with no selection: database in card headers, related fields when a card is collapsed, pin related fields | 3 | M | Model view and properties pane |
| Model view | Data pane: Tables and Model tabs, search | 3 | D | Model explorer |
| DAX query view | Ribbon: cut, copy, paste, format, comment, uncomment, find, replace, command palette, Copilot | 10 | P | Editors' tools (M); Copilot out of scope |
| DAX query view | Editor: run, query tabs, new query, results pane, Data pane with Tables and Model tabs, zoom | 6 | D | DAX query view |
| TMDL view | Ribbon: cut, copy, paste, format, find, replace, command palette | 7 | P | Editors' tools (M) |
| TMDL view | Editor: apply, preview the changes, problems pane, output pane, script tabs, new script, semantic model tree (cultures, expressions, functions, calculation groups, measures, perspectives, relationships, roles, tables) | 7 | D | TMDL view |
| Chrome | View switcher: report, table, model, DAX query, TMDL | 5 | D | Views |
| Chrome | Title bar: save, undo, redo, settings, light and dark theme | 5 | D | Undo and redo; Save |
| Chrome | Title bar: share, sign in | 2 | O | Out of scope |
| Chrome | Page tabs and status bar: page tabs, new page, previous and next pages, desktop and mobile layout, zoom out, zoom in, zoom level, fit to page | 9 | D | Pages; View options; Mobile layout |
| File view | Home: blank report, recommended, recent files, shared with me, keyword filter, filter menu | 6 | P | New blank file; shared files out of scope |
| File view | Open, save, save as | 3 | D | Files rows |
| File view | Share, publish, sign in | 3 | O | Out of scope |
| File view | Get data list: Excel workbook, Power BI semantic models, dataflows, SQL Server, Analysis Services, Text/CSV, web, OData feed, blank query, template apps, more | 11 | P | Get data; Refresh from databases (P) |
| File view | Import: Power BI template, visual from a file, visual from AppSource, Excel workbook contents | 4 | P | Open; Custom visuals; AppSource out of scope; Power Pivot import (P) |
| File view | Export: Power BI template, PDF | 2 | D | Save as a template; Export to PDF |
| File view | Options and settings: options, data source settings | 2 | P | Options dialog (P); Data source settings |
| File view | About | 1 | D | Licences page |
| Panes | Visualizations › Build: 43 built-in visual types, all visual types, get more visuals | 45 | P | Visuals rows (AI visuals, Gantt P; service visuals out of scope) |
| Panes | Visualizations › wells of a column chart: X axis, Y axis, legend, small multiples, tooltips, drillthrough fields, cross-report, keep all filters | 8 | D | Bind fields; Drillthrough pages |
| Panes | Format › Visual cards of a column chart: X axis, Y axis, legend, small multiples, gridlines, zoom slider, columns, data labels, plot area background | 9 | D | Format pane |
| Panes | Format › General cards: properties, title, effects, data format, header icons, tooltips, alt text | 7 | D | Format pane; Data format card |
| Panes | Analytics: constant line, min, max, average, median, percentile, error bars | 7 | D | Analytics pane |
| Panes | Filters: search, on this visual, on this page, on all pages | 4 | D | Filters rows |
| Panes | Data: search, table tree | 2 | D | Data pane |
| Menus | A visual's context menu: copy, show as table, analyze, group, summarize, new visual calculation, set up a verified answer | 7 | P | Analyze (P, BIcraft's own ranking); Summarize and verified answers out of scope |
| Menus | A visual's More options: export data, show as table, remove, spotlight, sort by, new visual calculation, set up a verified answer | 7 | P | Visual header; verified answers out of scope |
| Menus | A table's menu in the Data pane: select, new measure, new column, new quick measure, edit query, incremental refresh, manage aggregations, rename, delete from model, hide, mark as date table, view hidden, show all, collapse all, expand all | 15 | P | User-defined aggregations (M); the rest done |
| Dialogs | Options › Global: data load, Power Query editor, DirectQuery, R scripting, Python scripting, security, privacy, regional settings, updates, usage data, diagnostics, preview features, save and recover, report settings, Copilot | 15 | P | Options dialog (P) |
| Dialogs | Options › Current file: data load, regional settings, privacy, auto recovery, semantic model settings, query reduction, report settings | 7 | P | Options dialog (P) |
| Dialogs | Get data: search, eight categories (all, file, database, Microsoft Fabric, Power Platform, Azure, online services, other), certified connectors, template apps | 11 | P | Get data; Refresh from databases (P) |
| Dialogs | Manage security roles: roles, new, tables, rules, switch to the DAX editor, clear, delete, group, ungroup, save, close | 11 | D | Manage roles dialog |
| Power Query | Home › Close and New query: close and apply, new source, recent sources, enter data | 4 | D | Power Query editor; Get data |
| Power Query | Home › Data sources and Parameters: data source settings, manage parameters | 2 | D | Data source settings; Power Query parameters |
| Power Query | Home › Output data: export the query's results | 1 | D | Export a query's results |
| Power Query | Home › Query: refresh preview, properties, advanced editor, manage | 4 | D | Power Query editor; Advanced editor |
| Power Query | Home › Manage columns, Reduce rows, Sort: choose columns, remove columns, keep rows, remove rows, sort ascending, sort descending | 6 | D | Editor's other transforms (keep and remove rows) |
| Power Query | Home › Transform: split column, group by, data type, use first row as headers, replace values | 5 | D | Power Query editor |
| Power Query | Home › Combine: merge queries, append queries, combine files | 3 | P | Editor's other transforms (combine files) |
| Power Query | Transform › Table: group by, use first row as headers, transpose, reverse rows, count rows | 5 | P | Editor's other transforms |
| Power Query | Transform › Any column: data type, detect data type, rename, replace values, fill, pivot, unpivot, move, convert to list | 9 | P | Editor's other transforms |
| Power Query | Transform › Text column: split column, format, merge columns, extract, parse | 5 | P | Editor's other transforms (Parse left) |
| Power Query | Transform › Number column: statistics, standard, scientific, trigonometry, rounding, information | 6 | P | Editor's other transforms (Statistics left) |
| Power Query | Transform › Date and time column: date, time, duration | 3 | D | Editor's other transforms |
| Power Query | Transform › Scripts: run an R script, run a Python script | 2 | O | Python and R (BIcraft never runs scripts) |
| Power Query | Add column › General: column from examples, custom column, invoke a custom function, conditional column, index column, duplicate column | 6 | P | Editor's other transforms (column from examples, invoke a custom function left) |
| Power Query | Add column › From text, from number, from date and time: format, merge, extract, parse; statistics, standard, scientific, trigonometry, rounding, information; date, time, duration | 13 | P | Editor's other transforms (Parse, Statistics and the menus over several columns left) |
| Power Query | View: query settings, formula bar, monospaced, show whitespace, column quality, column distribution, column profile, always allow parameters, go to column, advanced editor, query dependencies | 11 | P | Data profiling; Query dependencies; Advanced editor |
| Power Query | Tools: diagnose step, start diagnostics, stop diagnostics, diagnostic options | 4 | P | Query diagnostics |
| Power Query | Panes: queries list, formula bar, data grid with column type icons, query settings (name, all properties, applied steps), status bar | 5 | D | Power Query editor |


## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-11-23 | minor | G106: the Data format card done (`visual.set_data_format`, the projection's visual-scoped `format`), sparklines partial (`visual.add_sparkline`, `visual.edit_sparkline`, evaluated per row and drawn in the cell; the queryRef spelling and marker bits not checked against Desktop); the Insert › Sparklines and General cards inventory rows follow |
| 2026-11-23 | major | M11.28, M11.29: Desktop 2.159.1231.0 (Store) read as a black box; the Desktop UI inventory section (one row per ribbon group, pane, menu and dialog, with its controls counted) and its ledger in the summary; 8 authoring rows added (Power Query transforms and Output data, the Options dialog, sparklines, Analyze, the Data format card, the editors' tools, service-bound items out of scope) |
| 2026-11-20 | minor | Maps (G92), real geography: bundled Natural Earth country outlines under map, filled map and Azure Maps drawings, country names and ISO codes placed offline (filled maps fill them), the built-in World countries (Natural Earth) shape map written as an ordinary custom TopoJSON map; the viewer row stays partial (other places need geocoding, no tiles, web not done) |
| 2026-11-20 | minor | Maps (G92): the viewer row partial (render and app: bubbles at latitude and longitude drawn offline, a shape map's custom map with saturation, legend, tooltips and cross-filtering; place names need geocoding and built-in shape maps are Microsoft's: placeholders; web not done); `shape_map.set_map` on the authoring row |
| 2026-11-19 | minor | Custom visuals (G80): .pbiviz import and removal done; AppSource split into its own row, out of scope; the viewer row partial (render and app: a placeholder with the package's icon) |
| 2026-11-18 | minor | Lot 18 part 2 (G61): saved diagram layouts done (layout tabs); one Model UI row added (Model explorer, partial) |
| 2026-11-05 | major | Gap review (target-app-parity.md "Gap review 2026-11-05"): rows rescored against `main` at ccb96e3 (buttons since M11, Filters pane, formula bar, decomposition tree, matrix expansion); 39 authoring rows (one out of scope) and 7 viewer rows (two out of scope; AI visuals split in three) added (Report UI, Model UI, Power Query depth, storage, accessibility, localisation, tools, service exclusions) |
| 2026-11-04 | major | First checklist (M11.7): 97 authoring rows and 55 viewer rows scored against `main` at 132b199 |
