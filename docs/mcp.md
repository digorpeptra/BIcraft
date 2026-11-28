# MCP server

`bicraft-cli mcp` runs a Model Context Protocol server on stdio (newline-delimited JSON-RPC 2.0,
protocol revisions `2025-06-18`, `2025-03-26` and `2024-11-05`). It drives one in-process engine
session: the file or model an agent opens, with its report, stays open across tool calls until
another one is opened. Logs go to stderr; stdout carries only the protocol.

```json
{
  "mcpServers": {
    "bicraft": { "command": "bicraft-cli", "args": ["mcp"] }
  }
}
```

Use the full path to the binary if `bicraft-cli` is not on `PATH`
(`target/release/bicraft-cli` after `cargo build --release -p bicraft-cli`). `bicraft-cli setup`
writes this entry for you.

## Setup

`bicraft-cli setup` registers the server with agent clients. It writes the
**absolute path of the running binary** with `args: ["mcp"]`, so run it from the binary you will
keep (for example after `cargo install --path apps/bicraft-cli`), not from a `target/` build.

```sh
bicraft-cli setup --client claude_code --client vscode --dry-run   # show the plan, write nothing
bicraft-cli setup --client claude_code --client vscode --yes   # apply it
bicraft-cli setup --all-detected --yes --json                  # every detected client
bicraft-cli setup --uninstall --yes                            # remove the bicraft entries
bicraft-cli setup doctor                                       # handshake + registration check
```

| `--client` | Configuration file (user scope) | Project scope (`--scope project`) | Entry |
|---|---|---|---|
| `claude_code` | `~/.claude.json` (`$CLAUDE_CONFIG_DIR/.claude.json`) | `.mcp.json` | `mcpServers.bicraft` with `type: "stdio"` ([docs](https://code.claude.com/docs/en/mcp)) |
| `claude_desktop` | `%APPDATA%\Claude\claude_desktop_config.json`; `~/Library/Application Support/Claude/…` (macOS); `$XDG_CONFIG_HOME/Claude/…` (Linux) | none | `mcpServers.bicraft` ([docs](https://modelcontextprotocol.io/docs/develop/connect-local-servers)) |
| `vscode` | `Code/User/mcp.json` under `%APPDATA%`, `~/Library/Application Support` or `$XDG_CONFIG_HOME` | `.vscode/mcp.json` | `servers.bicraft` with `type: "stdio"` ([docs](https://code.visualstudio.com/docs/copilot/customization/mcp-servers)) |
| `cursor` | `~/.cursor/mcp.json` | `.cursor/mcp.json` | `mcpServers.bicraft` with `type: "stdio"` ([docs](https://cursor.com/docs/context/mcp)) |
| `codex` | `$CODEX_HOME/config.toml` (default `~/.codex/config.toml`) | `.codex/config.toml` (trusted projects) | `[mcp_servers.bicraft]` `command`, `args` ([docs](https://developers.openai.com/codex/mcp)) |

On Windows, Claude Desktop's MSIX build reads a package-private copy,
`%LOCALAPPDATA%\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude\claude_desktop_config.json`,
once it has saved one; setup uses that file when it exists, the `%APPDATA%` file otherwise. Quit
Claude Desktop fully before applying (a running app can rewrite its configuration). A process
that Claude Desktop started (a terminal in its Code tab) has its `%APPDATA%` writes redirected into
the package; setup detects that view and refuses to write there; run it from an ordinary terminal.

**Selection.** `--client ID` (repeatable) or `--all-detected` (every client whose configuration
file or directory exists). Without them, setup asks in a terminal; otherwise (or with `--yes`) it
only refreshes clients where BIcraft is already registered. Non-interactive `--uninstall`
without `--client` removes every registration it finds.

**What a run does.**

- **Plan first.** Each selected client gets an action: `add`, `update` (another binary path or
  arguments), `unchanged`, `remove`, `none` (nothing to remove) or `invalid`. Setup writes only with
  `--yes` or a `y` at the terminal prompt; `--dry-run` never writes.
- **Malformed files block the run.** Invalid JSON (comments and trailing commas included), a
  top level or `mcpServers`/`servers`/`bicraft` that is not an object, invalid TOML, or a
  Codex entry written with dotted keys or an inline table: the file is reported, never rewritten,
  and nothing is written for any client.
- **Writes are careful.** The old file is copied to `<file>.bicraft.bak` (replacing an older
  backup), the new one is written to a temporary file and renamed over it, then read back and
  checked. A file that changed since the plan is not written. A symbolic link is followed.
- **Only the BIcraft entry changes.** JSON keeps key order, indentation, line endings, BOM and
  final newline; other keys of the entry (`env`…) are kept. Codex's TOML is edited as text: a new
  table is appended, an update replaces only the `command` and `args` values, an uninstall removes
  `[mcp_servers.bicraft]` and its sub-tables; comments and other tables stay byte for byte.
- **Idempotent.** A second run reports `unchanged` and touches nothing.
- Restart the clients listed in `restart` to load the change.

**Doctor.** `bicraft-cli setup doctor` starts `<binary> mcp`, sends `initialize` and `tools/list`,
and reports the server version, protocol and tool count, then each client's `registration`:
`aligned`, `stale` (another command; `registered_command_exists` says if that binary still exists),
`absent` or `invalid`. It is healthy when the handshake works and no client is stale; with
`--client`, those clients must also be `aligned`. Doctor never writes.

**Sandbox.** `--config-root DIR` (or `BICRAFT_SETUP_HOME=DIR`) resolves every path under `DIR`
instead of the home directory (`DIR/.claude.json`, `DIR/AppData/Roaming/…`, `DIR/.config/…`,
`DIR/Library/Application Support/…`) and ignores `APPDATA`, `XDG_CONFIG_HOME`, `CLAUDE_CONFIG_DIR`
and `CODEX_HOME`. `--platform windows|linux|macos` (sandbox only) picks the layout. The tests use
it; it is also a safe way to preview a run.

**JSON output** (`--json`, one document on stdout):

```json
{
  "status": "planned",
  "operation": "install",
  "scope": "user",
  "dry_run": true,
  "server": {"name": "bicraft", "command": "/home/me/.cargo/bin/bicraft-cli", "args": ["mcp"]},
  "config_root": null,
  "platform": "linux",
  "clients": [
    {"client": "cursor", "name": "Cursor", "path": "/home/me/.cursor/mcp.json", "detected": true,
     "exists": true, "selected": true, "registration": "stale",
     "registered_command": "/old/bicraft", "action": "update", "error": null, "backup": null,
     "result": null}
  ],
  "restart": [],
  "warnings": [],
  "exit_code": 0
}
```

`status` is one of `planned`, `applied`, `unchanged`, `nothing_selected`, `cancelled`,
`needs_confirmation`, `blocked` or `failed`; unselected clients have `action: "skip"`; `result`
is `written` or `failed` after an apply. Doctor's document has `healthy`, `handshake` (`ok`,
`server_name`, `server_version`, `protocol_version`, `tools`, `error`), `clients`, `problems` and
`warnings`.

| Exit code | `setup` | `setup doctor` |
|---|---|---|
| 0 | planned, applied, unchanged, cancelled, nothing selected | healthy |
| 1 | blocked (malformed configuration), needs confirmation (no `--yes`, no terminal) | not healthy |
| 2 | usage error, or a write failed (`failed`) | usage error |

### Agent skills

The repository's [skills/](../skills/) folder holds task guides for coding agents in the
`SKILL.md` format of the Crafting Apps (frontmatter `name` and `description`, with optional
`references/`). Each guide uses only the commands below.

| Skill | What it guides |
|---|---|
| [bicraft-visual-authoring](../skills/bicraft-visual-authoring/SKILL.md) | From "make me a visual that…" to a packaged Power BI visual: a built-in BIcraft visual (`visual.add`), a Spec visual (`visual.set_spec`) or a TypeScript one (`visual.scaffold` → `visual.pack` → `visual.import_custom`), then `report.render`, Power BI Desktop and `visual.export_bicraft_visual` |

`bicraft-cli mcp` does not serve skills: the server declares only the `tools` capability, with no
MCP prompts or resources, and `bicraft-cli setup` registers the server without copying skills.
Copy a skill's folder into the agent's skills directory (for Claude Code, `.claude/skills/` in a
project or `~/.claude/skills/`).

## Tools

Every tool runs an engine command, so the MCP server, `bicraft-cli run` and the tests always behave
the same. The convenience tools map 1:1 to the most common commands; `run_command` and `batch`
reach all of them.

| Tool | Command | What it does |
|---|---|---|
| `list_commands` | | Every command (id, label, params doc, and its button: `place`, `shortcut`, `needs`, `form`, `enabled`, `disabled_reason`; see *Buttons, shortcuts and forms*), optionally filtered by a substring |
| `run_command` | any | Run one command: `{command, params}` |
| `batch` | any | Run several commands in order; stops at the first error and reports its index and the results so far. `atomic` (default `true`): a failure rolls the whole batch back (see "Undo, redo and atomic batches") |
| `open_file` | `file.open` | Open a `.pbix`/`.pbit`, a `.pbip` file or project folder, or a `model.bim`, or import an Excel workbook's Power Pivot data model (`.xlsx`/`.xlsm`: a new Power BI model held in memory, `imported` in the result, saved with `file.save_as`/`file.convert`, never over the workbook; docs/formats/powerpivot.md); `data_root?` (see *Power Query sources*); `cache_abf?` (PBIP: an explicit data cache, see *Explicit data cache*); or read a `.vpax`, `.pbids`, `.pq`/`.m`, `.pqt`, Dataflow Gen2 or CDM `model.json` into a new unsaved model (M11.36, [formats/import-sources.md](formats/import-sources.md)) |
| `inspect_file` | `file.inspect` | Kind, parts and formats of the open file, whether its model and report are readable (`model_error`, `report_error`), page and visual counts, `dirty` / `report_dirty` (unsaved edits), and for a thin report its `live_connection` (`type`, `data_source`, `catalog`; its model is published in the Power BI service and not opened, a known limitation: docs/gaps.md G12) |
| `model_summary` | `model.summary` | Tables, columns, measures, relationships and roles |
| `validate_dax` | `dax.validate` | Parse a DAX expression and bind it against the open model (syntax and arity only without a model); `kind` (measure, calculated_column, calculated_table) adds that object's context rules |
| (`run_command`) | `dax.functions` | The DAX functions with their signatures (`DIVIDE(numerator, denominator[, alternate_result])`) and BIcraft's one-sentence description of each (`description`, `description_fr`: written for BIcraft in its own words), and the keywords; `query` filters by name |
| `validate_model` | `model.validate` | Validate every measure and calculated column |
| `model_usage` | `model.usage` | Where an object is used: DAX (direct and transitive), RLS/OLS, relationships, hierarchies, sort-by, M, field parameters, perspectives, translations, and the report (visual roles, filters, drillthrough, tooltips, bookmarks, formatting, sort); the dependency graph (see "Where objects are used, and what is unused") |
| `model_unused` | `model.unused` | Unused measures, columns, calculated tables and M expressions with the reason, and what only model rules keep; `reports` unions thin reports |
| `remove_unused` | `model.remove_unused` | Remove them (all or `objects`) in one undoable edit; `dry_run` previews |
| (`run_command`) | `model.analyze` | Best-practice rules: Microsoft's BPA rules plus your rule files (see "Best-practice analysis") |
| `new_model` | `model.new` | Start an empty model in memory |
| `add_table` | `table.add` | Add a table, with an M or DAX (calculated) partition |
| `add_column` | `column.add` | Add a data column or a calculated column |
| `add_measure` | `measure.add` | Add a measure |
| `set_measure` | `measure.set` | Change, rename or reformat a measure |
| `add_relationship` | `relationship.add` | Relate `Table[Column]` (many side) to `Table[Column]` (one side); `name?` (unique) instead of a new GUID. One-to-one cross filters in both directions (its default; `cross_filter: "single"` is refused: Desktop does not open such a file). `security_filtering?`, `assume_referential_integrity?` (DirectQuery or Dual tables); without `active` it is added inactive when another is active between the two tables or paths would become ambiguous; a "one" side with duplicates or blanks, or no unique side (unless many-to-many), is refused; `check: true` answers the review (errors, warnings, suggested cardinality and direction from the data) and changes nothing |
| `save` | `file.save` | Save the open file: a PBIP project (the report too when it was edited, as PBIR), a PBIX/PBIT (the edited model and/or report, written back in place), or `model.bim` |
| `save_as` | `file.save_as` | Save, or convert, to a new PBIP project (`out/Name.pbip`, with the report), a `.pbit`, a `.pbix` (from an open PBIX, or a PBIP with `.pbi/cache.abf`), or `model.bim` (model only); the target becomes the open file. See "Conversions" |
| `convert` | `file.convert` | Convert a file without opening it (`{from, to, format?, report_format?, cache?, overwrite?, strict?}`); the open file is not touched |
| (`run_command`) | `file.redact` | `{path, out, data?, overwrite?}`: a copy of a PBIX, PBIT or PBIP for a bug report, its connection strings, credentials, server and database names, file paths and URLs (with `data`, the stored text values) replaced by stable placeholders; every written byte checked, a survivor deletes the copy; returns counts and placeholders, never the original values; also `bicraft-cli redact IN OUT [--data]` ([formats/import-sources.md](formats/import-sources.md)) |
| `diff_files` | `file.diff` | Semantic diff of two files of any format (`{a, b, scope?, ignore_formatting?, ignore_positions?, ignore_whitespace?, format?: "json", "text" or "markdown"}`): model and report objects added, removed, changed, renamed or moved, DAX compared after normalisation; metadata only; read-only ([diff.md](diff.md)) |
| `import_objects` | `objects.import` | Copy measures, tables, calculated columns, calculation groups, functions, M expressions, roles, pages, visuals, bookmarks or the theme from another file into the open one, with what they use; `on_conflict`: fail, skip, replace or rename; one undoable edit; `dry_run` previews (see "Importing objects from another file") |
| (`run_command`) | `file.textconv` | `{file}`: the canonical text form of one file (sorted objects, one property per line), as `bicraft-cli diff --textconv` gives git ([diff.md](diff.md)) |
| (`run_command`) | `file.open_in_desktop` | Open the saved file (the `.pbix`/`.pbit`, or the project's `.pbip`) with the OS default application, normally Power BI Desktop. `{dry_run?}`; never a path from the caller; returns `{path, opened, unsaved, warning?}` (unsaved edits are not in the file: save first). Read-only. Also the live viewer's button |
| `new_project` | `project.new` | Create `<dir>/<name>.pbip` from the open model and report (or empty ones) and open it |
| `report_summary` | `report.summary` | Pages (name, display name, size, active) and their visuals (type, position, bound fields); visual type counts; distinct fields |
| `add_page` | `page.add` | Add a page: `{display_name, width?, height?, display_option?, index?}` |
| `add_visual` | `visual.add` | Add a visual: `{page, visual_type, x, y, width, height, z?, fields?: [{role, field}], force?}` |
| `bind_field` | `visual.bind` | Bind a field to a data role: `{page, visual, role, field, index?, force?}` |
| `validate_report` | `report.validate` | PBIR schema checks, structure, and every field of visuals and filters against the model; a field projected under its queryRef and again under that queryRef plus a digit suffix (`Sum(T.C)` and `Sum(T.C)1`, saved by BIcraft before 2026-10-15; Desktop fails the visual's query) is an error that `report.repair` fixes |
| `validate_dax_query` | `dax.validate_query` | Parse a DAX query (`DEFINE` MEASURE/VAR/TABLE/COLUMN/FUNCTION, `EVALUATE`, `ORDER BY`, `START AT`) and bind it against the open model without changing it; also `bicraft-cli validate-dax-query` |
| `render_page` | `report.render` | Draw a report page in pure Rust (theme, evaluated data, slicer selections, images, formatting) and return it as an MCP **image** content block (`{type: "image", mimeType: "image/png", data}`; `structuredContent.pages[].image` then says which block); `out` writes PNG files instead. Per-visual `status` (what was drawn): `drawn`, `preview` (made-up values: the visual has no data), `placeholder` (a type that is not drawn), `estimate` (a key influencers visual drawn from BIcraft's own estimate, `key_influencers: "estimate"`), `blank` (no field bound), `skipped`; per-visual `data` (what the evaluated dataset holds, language-independent): `ok`, `no_rows`, `blank_values`, `partial_blank`, `not_bound`, `not_evaluated`, with `rows` / `value_cells` / `blank_cells`; `pages[].visuals_without_data`; `fail_on_empty: true` makes visuals without data an error. See [render.md](render.md) "Data states" |
| `evaluate_dax_query` | `dax.evaluate` | Run a DAX query over the open file's data and return the rows, like DAX Studio: `{query, max_rows?, timeout_ms?, allow_cache_abf?}` (see *Running DAX queries*); also `bicraft-cli dax-query` |
| `refresh_model` | `model.refresh` | Refresh the open PBIX file's or PBIP project's import tables with BIcraft, external connectors only when allowed (Databricks): `{tables?, allow_connectors?: ["databricks"], external_links?, data_root?, force?, backup?}`; a PBIX gets the new rows in its `DataModel` (as Desktop's Refresh writes them), a PBIP project in `.pbi/bicraft-data.bin` (see *Refreshing data with BIcraft*); also `bicraft-cli refresh` |
| `run_tests` | `project.test` | Run a project's CI quality gates from its declarative test file (`bicraft.tests.yaml` next to it, or `tests`: a file or the tests inline): DAX assertions over its data by BIcraft's own engine, model rules (validation, BPA, naming, required properties, forbidden functions, expression length), report rules (report.validate by category, visuals per page, alt text, page size, theme, navigation to hidden pages). `{path?, tests?, format?: json|text|junit|sarif, fail_on?: error|warning, base_dir?}`; read-only (`path` opens in a private session). Returns `{passed, exit_code, summary, suites}`, plus `output` (JUnit XML, SARIF 2.1.0, text). See [ci-tests.md](ci-tests.md); also `bicraft-cli test` |
| `list_sources` | `sources.list` | The data sources of the model's M (files, folders, databases, Databricks, web) with each argument's role, form (literal, parameter, step, computed) and span, plus the parameters and how the sources use them; read-only. See [sources-env.md](sources-env.md); also `bicraft-cli sources` |
| `rebase_sources` | `sources.rebase` | Move file paths from one root to another (Windows, UNC, POSIX; optionally relative to a text parameter, added when missing) and map servers, databases, Databricks hosts and HTTP paths, URL prefixes. `{from?, to?, param?, servers?, databases?, hosts?, http_paths?, urls?, dry_run?}`; literals only, one edit. Also `bicraft-cli sources rebase` |
| `apply_env` | `env.apply` | Apply a profile of `bicraft.env.yaml`: typed parameter values and the profile's mappings, in one edit. `{profile, env_file?, dry_run?}`. Also `bicraft-cli env apply` |
| `check_env` | `env.check` | What a profile does not cover and what it would change; `ok` for CI. `{profile, env_file?}`; read-only. Also `bicraft-cli env check` |

Other editing commands, through `run_command` or `batch`:

| Command | Params |
|---|---|
| `table.import` | `{name, csv? (file path or text), m? (a Power Query query), dax? (DATATABLE/{…}/ROW/CALENDAR literal), culture?, delimiter?, types?: {column: type}, replace?, hidden?, description?, data_root?}`: a table with data (CSV is stored as an M `#table` literal partition, `m` is kept as written, `dax` makes a calculated table); `replace` gives an existing table new data. On a PBIX, `file.save` writes the data into the `DataModel`. See *Power Query sources* below |
| `table.set_data` | `{name, csv?\|m?\|dax?, culture?, delimiter?, types?, data_root?}`: new data for an existing table (its relationships are rebuilt on a PBIX save) |
| `table.remove` | `{name, force?}`: also removes the table's relationships and row filters; refused while other DAX uses the table (see *Editing rules*) |
| `table.rename` | `{name, new_name, update_dax?}`: relationships, roles and every DAX reference follow |
| `table.set` | `{name, description?, hidden?, data_category?, key_column?}`: `data_category: "Time"` with `key_column` marks a date table (the column gets `isKey`); refused unless the column is a date/time column whose values are unique, not blank, one per day without gaps and at one time of day (checked when the session has the data, `data_checked`); leaving `Time` clears the key |
| `table.set_storage_mode` | `{table, mode: import\|directQuery\|dual, dual_related?}`: writes every partition's `mode`. Refused for a calculated table, a table with a refresh policy, a non-M partition, a query with no connector, and an Import table a PBIX stores; warns about limited relationships (Import with DirectQuery) and Dual on the many side; `dual_related` (with `import`) sets the DirectQuery tables on the one side of its relationships to Dual. Returns `previous_mode`, `set_to_dual`, `dual_candidates`, `limited_relationships`. DirectQuery tables are not evaluated (visuals: "not evaluated: DirectQuery") |
| `table.set_refresh_policy` | `{table, store: {n, unit}, refresh: {n, unit}, detect_changes?, only_complete_periods?, realtime?}` (units `day\|month\|quarter\|year`): writes the table's `refreshPolicy` (archive and refresh periods, `incrementalPeriodsOffset -1`, `mode: hybrid` for real time, a polling expression on the change detection column, the query as `sourceExpression`). Needs the `RangeStart` and `RangeEnd` Date/Time parameters, an Import table and a filter step on both; the error lists what is missing. Raises the compatibility level to 1550 (1565 for real time). The policy applies in the service; the partition is kept |
| `table.remove_refresh_policy` | `{table}`: removes the table's `refreshPolicy` |
| `column.set` | `{table, name, new_name?, data_type?, source_column?, expression?, format_string?, display_folder?, summarize_by?, hidden?, description?, force?, update_dax?}` |
| `column.rename` | `{table, name, new_name, update_dax?}`: `column.set` with only a new name |
| `column.set` / `column.add` properties | `sort_by_column?: "Column"\|null`, `is_available_in_mdx?`, `is_key?`, `data_category?` (see *Model properties*) |
| `measure.add` / `measure.set` | `format_string_definition?: string\|null`: the dynamic format string (see *Model properties*) |
| `model.set` | `{culture?, discourage_implicit_measures?: bool, source_query_culture?: string\|null}` |
| `model.set_annotation` | `{name, value: string\|null}`; also `table.set_annotation {table, name, value}`, `column.set_annotation {table, column, name, value}`, `measure.set_annotation {measure, table?, name, value}`. `null` removes; returns `previous` |
| `column.remove_variation` | `{table, name, variation?, force?}`: removes a column's date variations (all by default), their relationships and the auto date tables (`LocalDateTable_…`) no other variation uses |
| `column.remove` | `{table, name, force?}`: also removes the relationships that use it; on a calculated table the expression is rewritten to select the other columns (`expression_rewritten`) |
| `measure.rename` | `{name, new_name, table?, update_dax?}`: `measure.set` with only a new name |
| `measure.remove` | `{name, force?}` |
| `measure.move` | `{name, to_table, force?, update_dax?}`: `Table[Measure]` references follow |
| `dax.apply_define` | `{query, measures?: [name], force?}`: the DAX query view's Update model. Each `DEFINE MEASURE Table[Name] = …` of the query (or those named) is added to Table, or replaces the expression of Table's measure of that name; one undo step, validated with every change made. Returns `{changes: [{table, measure, action: added\|updated\|unchanged}], added, updated, unchanged, changed}`; an unknown table, a measure of another table or a query that does not parse is an error |
| `dax.format` | `{query, max_width? (20 to 1000, default 100), indent? (0 to 16, default 4), style? (`query`, default, or `formula`)}`: the DAX query view's Format query, for a query (`DEFINE`/`EVALUATE`) or an expression; `style: formula` is the formula bar's Format DAX, BIcraft's style following the DAX formatting conventions SQLBI describes publicly (function names and keywords in upper case, a space before each `(` and inside non-empty brackets, `SUM ( T[C] )`, the expression after `RETURN` on its own line one level in). Except that upper case, only white space changes (tokens, comments, names and texts stay as written, checked); `DEFINE`, `EVALUATE`, `ORDER BY` and `START AT` start a line, each definition one level in, a call or `{…}` that does not fit or holds `VAR` or a line comment gets one argument per line, `VAR` and `RETURN` start lines. Idempotent. Returns `{query, changed}`; text that does not parse is an error |
| `dax.define_query` | `{table?, measure?, references? (default false)}`: the DAX query view's Define all measures and Define with references. `table`: every measure of that table; `measure`: that measure; with `references`, every measure they reference too, transitively (the binder's dependency graph), each before the measures that use it. Returns `{query, measures, evaluated}`: `DEFINE` with one `MEASURE 'Table'[Name] = expression` per measure as the model has it, then `EVALUATE ROW("Name", [Name], …)` of the measures asked for. Read-only |
| `dax.cancel` | `{}`: stops every `dax.evaluate` of the session that started before it (they fail with `cancelled by the client`); read-only, so a server runs it beside the query. Returns `{running}`. The desktop app also raises the session's cancel flag as it sends it, so a local query stops before the command's turn |
| `dax.queries` | `{}`: the DAX query view's tabs of the open file, `{tabs: [{name, text}], default_tab, saved_in: project\|session, dirty, generation, error}`; a PBIP project's are read by `file.open` from `<Name>.SemanticModel/DAXQueries/` ([formats/dax-queries.md](formats/dax-queries.md)) |
| `dax.set_queries` | `{tabs: [{name, text?}], default_tab?}`: replaces the tabs (names are file names: valid on Windows and Linux, unique ignoring case); an unsaved change of its own (`dirty.queries` of the snapshot, no undo step) that `file.save` writes into a PBIP project (one `.dax` file per tab, `.pbi/daxQueries.json`), `file.save_as` and `project.new` into the new project. A PBIX, PBIT or model.bim keeps them for the session (a save warns). The snapshot's `dax_queries` holds the same tabs and a `generation` that changes with every `file.open` |
| `relationship.remove` | `{name}` or `{from, to}` |
| `relationship.set` | `{name}` or `{from, to}`, then `new_name?` (unique, case-insensitive; renames it in place, column variations that name it follow, nothing else names relationships: DAX uses their columns, reports do not refer to them; on a PBIX the row is renamed), `active?`, `cross_filter?: single\|both`, `from_cardinality?`, `to_cardinality?: one\|many`, `security_filtering?: single\|both\|none`: changes the relationship in place (same name); refused for a second active relationship between two tables, a new ambiguous filter path, one-to-many (swap the ends), security filtering both without cross filtering both, a one-to-one with `cross_filter: "single"` (a one-to-one left single-direction, e.g. from an older file, is made both with a note), and a new "one" side with duplicates or blanks (when the data is known). Also `new_from?` / `new_to?` (move an end; a new "one" end is checked against the data), `assume_referential_integrity?`, and `check: true` (the review without a change, as `relationship.add`). On a PBIX, a cardinality or an end change rebuilds its storage under the same name |
| `relationship.autodetect` | `{apply?, only?: [{from, to}]}`: what Manage relationships › Autodetect finds (same column name and data type in two unrelated tables, a unique non-blank side in the data, one per pair of tables, no ambiguous path; `docs/formats/model-objects.md`). Without `apply`, answers `{candidates, skipped: [{from, to, code, reason}]}` and changes nothing; with `apply: true`, adds them (or those `only` names) in one undo step |
| `diagram.move` | `{table, x, y}` or `{tables: [{table, x, y}]}`, then `diagram?` (a layout tab's name; default the selected one): puts each table's card at that top-left corner of the model diagram, adding its node when the diagram has none (docs/formats/diagram-layout.md); the model becomes dirty and `file.save` writes the layout back (PBIX `DiagramLayout`, PBIP `diagramLayout.json`; a `model.bim` has no place for it, with a warning). One undo step. The Model view's drag of a card runs it, with the cards the file had no place for yet at their current places |
| `diagram.add` | `{name, tables?: [table]}`: a new layout tab after the others (unique name ignoring case, at most 100 layouts), its tables placed in rows of four, made the selected one. One undo step |
| `diagram.rename` | `{name, new_name}`: `selectedDiagram` and `defaultDiagram` follow. One undo step |
| `diagram.remove` | `{name}`: refused for the layout of all tables (ordinal 0, else the first); a selection naming it goes to that layout. One undo step. `table.rename` renames the tables' nodes in every layout |
| `diagram.remove_tables` | `{tables, diagram?}`: takes the tables' cards out of a layout tab (default the selected one), refused for the layout of all tables; the tables stay in the model. One undo step |
| `hierarchy.add` | `{table, name, levels: [column \| {name?, column}], hidden?, description?, display_folder?}`: a user hierarchy, top level first; on a PBIX, `file.save` writes its node table |
| `hierarchy.remove` | `{table, name}` |
| `hierarchy.set` | `{table, name, new_name?, levels?: [column \| {name?, column}], rename_levels?: {old: new}, hidden?, description?, display_folder?}`: kept levels keep their properties; a rename follows perspectives, translations and date variations |
| `calculation_group.add` | `{name, precedence?, column? (default Name), description?, hidden?}`: a calculation group table (item column, hidden `Ordinal`, `calculationGroup` partition); sets `discourageImplicitMeasures` as Desktop does; precedences must differ |
| `calculation_group.set` | `{name, precedence?, description?, multiple_or_empty_selection_expression?, no_selection_expression?, force?}` (`null` clears) |
| `calculation_group.remove` | `{name, force?}` |
| `calculation_item.add` | `{group, name, expression, ordinal?, format_string_expression?, description?, force?}`: DAX validated (`SELECTEDMEASURE()` and friends); `ordinal` inserts and moves later items down |
| `calculation_item.set` | `{group, name, new_name?, expression?, ordinal? (moves; ordinals become 0..n), format_string_expression?, description?, force?}` |
| `calculation_item.remove` | `{group, name}`. On a PBIX, `file.save` rewrites the group table's rows (one per item) when items are added, removed, renamed or moved |
| `role.add` | `{name, model_permission?}` (default `read`) |
| `role.set_filter` | `{role, table, filter_expression}`: `null` clears the filter |
| `role.remove` | `{name}` |
| `role.rename` | `{name, new_name}`: unique, no comma; View as follows |
| `role.set` | `{name, description?: string \| null, model_permission?}`: at least one; the Properties pane of a role. One undo step |
| `role.check_filter` | `{role?, table, filter_expression}`: the DAX checked as a row filter of `table`, nothing changed; `{valid, empty, errors, warnings, diagnostics}` |
| `role.set_permission` | `{role, table, column?, metadata_permission: "none" \| "read" \| null}`: object-level security of a table or a column (`null` or `"default"` removes it); refused on a table related to two or more other tables; View as treats what every chosen role hides as missing (`docs/formats/model-objects.md`) |
| `perspective.add` | `{name, description?, tables?: [table \| {table, all?, columns?, measures?, hierarchies?}]}`: a perspective with those members (a table name alone: every member), written one by one (`docs/formats/model-objects.md`) |
| `perspective.set` | `{name, new_name?, description?, tables?}`: the `tables` given replace the members (`null` empties it); needs one of the three |
| `perspective.remove` | `{name}` |
| `model.set_properties` | `{objects: [{table} \| {table, column} \| {table, measure}], props: {description?, hidden?, synonyms?, display_folder?, format_string?, data_category?, summarize_by?, sort_by_column?, storage_mode?}, force?}`: the same properties on every object through its own setter, all or nothing, one undo step; a property that does not apply to one of them is refused (`docs/formats/model-objects.md`) |
| `report.new` | `{}`: a blank one-page report for a session without one (after `new_model`) |
| `report.apply` | `{spec, prune?: true \| "visuals" \| "all", dry_run?, force?}`: makes the report match a declarative spec; re-applying is a no-op (see *Declarative reports*); a `type` may name one of BIcraft's visuals, embedded on first use (`installed`) |
| `report.check` | `{spec, prune?, force?}`: read-only; the changes `report.apply` would make, and every error |
| `report.repair` | `{page?, visual? (needs page)}`: repairs what `report.validate` reports as fixable. Today: a visual whose query projects one field under two queryRefs (e.g. a scatter chart's Y and Size as `Sum(Sales.Qty)` and `Sum(Sales.Qty)1`) gets the first queryRef (and `nativeQueryRef`) on every projection of the field, and `metadata` selectors naming the duplicate follow; a legacy layout then selects the field once on save. Only a digit suffix of the first queryRef counts: Desktop itself gives one field other distinct queryRefs in some visuals (`T.M` and `T.M (…)`), which stay; projections of different fields are never touched, whatever their names. Re-binding the visual (`visual.bind`, `report.apply` `fields`) repairs it too. Returns `{count, fixed: [{page, visual, kind: "duplicate_query_ref", kept, replaced}]}` |
| `report.rebind` | `{from: "Table[Name]", to: field}`: `from` is a column or measure the model lacks (refused when it resolves); every reference of `to`'s kind to it (visual roles and sorts, filters, drillthrough and tooltip bindings, formatting, bookmarks, other properties) is pointed at `to`, with the stored queryRefs, selectors, bookmark states and phone layout that name it, as `column.rename` with `update_report` rewrites the report. Refused, changing nothing, when nothing refers to `from`, or when the table changes for a reference written through a query alias (`From`). One undo step. Returns `{from, to, kind, affected_bindings, affected_binding_count, other_references, other_reference_count, query_refs_updated?}` |
| `report.remove_reference` | `{field, place, page?, visual?, bookmark?}`: removes what names `field` (it may be missing from the model) at one place. `place: "drillthrough"` with `page`: the page's drillthrough (or tooltip) filter cards on the field and the `pageBinding` parameters bound to them or naming it; `"formatting"` with `page` and `visual`, `page`, or neither (the report): each formatted property naming the field in `objects` / `visualContainerObjects` (an entry whose selector names it goes whole; an entry, object or formatting key left empty goes); `"bookmark"` with `bookmark`: each entry of the bookmark's lists (filters, sorts, formatting) naming it. Refused, changing nothing, when nothing there names the field or a reference would remain. Returns `{field, place, at, removed}` |
| `visual.fix_query_refs` | `{page, visual}`: each projection whose `queryRef` names another field (`REPORT_STALE_QUERYREF`) takes the field's own (`Sales.Total`, `Sum(Sales.Amount)`), unless another projection of the visual already has it; selectors, expansion states, the visual's state in bookmarks and its phone layout follow. Refused when there is none to rename. Returns `{renamed: [{role, field, from, to}], kept, references_updated}` |
| `report.extract` | `{page?}`: read-only; the report as a spec that `report.apply` accepts with no change |
| `report.render` | `{page?, out?, scale?, format?: "png", font_dirs?, system_fonts?, badges?, ui_language?, culture?, format_culture?, as_of?, text_dump?, lcd_text?, high_contrast?, fail_on_empty?, key_influencers?}`: read-only; pages to PNG without Power BI Desktop (every page without `page`; `out` a `.png` for one page, a folder or `name.png` → `name-<n>.png` for several; without `out` the PNGs come back as base64, which the MCP tools turn into image blocks). Fonts: the system's (Segoe UI…), `font_dirs`, `BICRAFT_FONT_DIRS`, then the bundled Selawik. `ui_language` (e.g. `fr-FR`) sets the language of the generated text for this call only (see `view.set_language`); `format_culture` the culture of numbers and dates (default the model's); `culture` only the model translation whose captions name fields (a culture without translations, or the model's own, draws the stored names and adds `warnings`; only a malformed name is refused); `as_of` the date `TODAY()`/`NOW()` return. `text_dump: true` also lists every drawn text run (string, font, px size, weight, colour, box in report pixels; with `out`, a `.json` beside each PNG) for the render oracle's text check. `lcd_text: true` draws upright text with subpixel (ClearType-like) antialiasing, as the app draws its pages on screens under 175 % (default grayscale; [render.md](render.md) "Subpixel text"). `high_contrast: true` draws the pages with the app's high-contrast palette (View › High-contrast colours: black page, white text, bright series colours; [render.md](render.md) "High contrast"); the file does not change. Result: `pages[]` with `visuals[]` = `{name, type, status, note, data, rows?, value_cells?, blank_cells?}` (`status` = what was drawn; `data` = what the evaluated dataset holds: `ok`, `no_rows`, `blank_values`, `partial_blank`, `not_bound`, `not_evaluated`; counts only with a dataset) and `visuals_without_data` (visuals whose `data` is `no_rows`, `blank_values` or `not_evaluated`); `fail_on_empty: true` returns an error naming them instead (see [render.md](render.md) "Data states"). `key_influencers: "estimate"` draws key influencers visuals from BIcraft's own estimate, badged, instead of the placeholder ([key-influencers.md](key-influencers.md)). Also `bicraft-cli render FILE [--page P] [--out F] [--lang TAG] [--culture NAME] [--format-culture NAME] [--as-of DATE] [--text-json] [--lcd-text] [--high-contrast] [--fail-on-empty] [--key-influencers estimate]` (exit 1 when a visual has no data, after writing). See [render.md](render.md) |
| `report.export_pdf` | `{out, pages?: "all"\|"current"\|"2-4"\|name\|index\|[…], hidden?, title?, scale?, font_dirs?, system_fonts?, ui_language?, format_culture?, as_of?, pdf_ua?, pdf_a?}`: report pages to one PDF file, as Desktop's Export to PDF: one PDF page per report page at its size (0.75 pt per report pixel), drawn as `report.render` draws it, shapes as vector paths, text as real text with embedded font subsets and a ToUnicode map (selectable, searchable), a bookmark per page and a tagged structure (a section per page and visual in reading order, paragraphs, a figure with the visual's alt text or a chart's title, table visuals as tables of header and data cells); `pdf_ua` (default true) identifies the file as PDF/UA-1 in its XMP metadata, `pdf_a` (default false) as PDF/A-2b with an sRGB output intent; a CFF face (`.otf`) is embedded whole; a face whose licence (`fsType`) forbids embedding is drawn as outlines with invisible searchable text. Every page not hidden by default; `hidden: true` adds hidden ones to `"all"` and ranges; pages named one by one are exported even when hidden. `out` is required and never empty (server mode: a workspace path). Result: `{path, bytes, title, pages: [{index, page, display_name, width_pt, height_pt, visuals, raster_parts}], fonts: [{name, embedded, glyphs}], conformance: {pdf_ua, pdf_a: "2b"\|null}, skipped_hidden, warnings?}`. Writes a file only: no revision, no undo step. Also `bicraft-cli export-pdf FILE [--out F.pdf] [--pages P] [--hidden]`. See [render.md](render.md) "PDF" |
| `report.print` | `{pages?, hidden?, title?, out?, dry_run?, ui_language?}`: the pages as `report.export_pdf` writes them, to `out` (default `bicraft-print/<title>.pdf` in the temporary folder; required in server mode), then handed to the OS's default application for `.pdf` files to print (`explorer.exe`, `open` or `xdg-open`, no shell); nothing is opened in server mode or with `dry_run: true`. Result: `report.export_pdf`'s and `{opened}`. See [render.md](render.md) "Print" |
| `visual.export_data` | `{visual, page?, out, format?: "csv"\|"xlsx", kind?: "summarized"\|"underlying"\|"current_layout"}`: a visual's data to a file, as its More options › Export data. Summarized (default): the visual's fields in their order, aggregated as the visual aggregates them, every hierarchy level, under its filters, slicers and cross-filters. Underlying: aggregated columns grouped instead (a row per distinct combination), measures on each row; only when the report allows it. Current layout (XLSX only): the summarized data in the visual's shape, a matrix with column fields as a cross table (a header row per column field, each label merged over its values, then the row fields' and values' names, a row per row key), other visuals as the summarized table; at most 150,000 cells of values, no subtotals. At most 30,000 rows to CSV and 150,000 to XLSX (Power BI's limits; past them the first rows, `truncated: true` and a warning). CSV: UTF-8 with a BOM, CRLF, RFC 4180 quoting, a `'` before text starting with `=`, `+`, `-`, `@`. XLSX: one sheet `Data`, numbers as numbers, dates as dates. Refused when the report's `exportDataMode` forbids it. Result: `{path, bytes, format, kind, page, visual, columns, rows, total_rows, limit, truncated, export_data_mode, warnings?}` |
| `report.custom_visuals` | `{}`: read-only; embedded custom visuals (`CustomVisuals/<guid>/` files with their `report.json` package: name, version, API version, data roles, files, issues), AppSource (`publicCustomVisuals`) and organization visuals, each with the visuals that use it; `bicraft`: BIcraft's own visuals (name, guid, version, `embedded`, `embedded_version`, `current`, roles, `used_by`), which `visual.add` and `visual.set_type` take by name and embed on first use, and whose unused packages a save drops (docs/community-visuals.md "Built in by default") |
| `visual.import_custom` | `{path \| data}`: imports a `.pbiviz` (at most 32 MB, read as hostile input: capped sizes and entries, ZIP-bomb guard, plain names only) into `CustomVisuals/<guid>/` with its `CustomVisual` resource package; the same guid replaces it; PBIR reports only. Its JavaScript is never run: the visual is drawn as a placeholder with its icon and name (docs/formats/pbiviz.md) |
| `visual.remove_custom` | `{name, force?}`: removes a custom visual's package, files and AppSource reference; refused while visuals use it unless `force` (they stay as placeholders) |
| `visual.scaffold` | `{folder, name, display_name?, guid?, description?, author?, data_roles?, format_options?, overwrite?, template?}`: writes a custom visual project (`template: spec`: the BIcraft Spec visual's own sources, docs/formats/spec-visual.md) (`pbiviz.json`, `capabilities.json` with the roles `name:Grouping\|Measure\|GroupingOrMeasure` and format options `object.property:bool\|color\|numeric\|text` chosen, `src/visual.ts`, `style/visual.css`, an icon); no session change (docs/formats/pbiviz.md "Visual projects") |
| `visual.build` | `{folder, out?}`: compiles the project's TypeScript and bundles it into one script with swc, no Node (`dist/visual.js`); npm imports, import cycles, LESS and `.tsx` are refused; types are not checked |
| `visual.pack` | `{folder, out?}`: builds the project and writes `dist/<guid>.<version>.pbiviz`, which `visual.import_custom` reads (`bicraft-cli visual new\|build\|pack` on the command line) |
| `visual.export_bicraft_visual` | `{name, out}`: writes the `.pbiviz` of one of BIcraft's fourteen visuals (`name`: its name or guid, a list, or `all`; `out`: the `.pbiviz` file for one visual, else a folder of `<guid>.<version>.pbiviz` files), the package BIcraft embeds, for Desktop's Import a visual from a file and for organizational visuals; needs no open file and leaves the session as it is (`bicraft-cli visual export NAME... \| --all [--out F\|DIR]`) |
| `visual.set_spec` | `{page, visual, spec}`: draws the visual from a declarative chart spec (an object or its JSON text, at most 32 KB): marks `bar`, `line`, `point`, `area`, `text`, `rule`, `rect` whose `x`, `y`, `x2`, `y2`, `color`, `size` and `label` read `category` and `value` / `value.N` (the visual's measures in order), band, linear and time scales, axes, gridlines, palette, legend; every error names its place. Embeds the BIcraft Spec custom visual (built from BIcraft's TypeScript by its own bundler) when missing or older, turns the visual into one (columns to `category`, measures to `values`) and stores the spec in its `spec.json` property: Power BI draws it with the visual's script, `report.render`, PDF export and the app with the same layout (docs/formats/spec-visual.md) |
| `visual.spec_template` | `{kind?}`: read-only; a starting spec (`bar`, `line`, `area`, `point`, `scatter`, `combo`, `text`, `rule`) with the roles and fields a spec reads |
| `visual.set_bicraft_visual` | `{page, visual, kind, options?}`: turns the visual into one of BIcraft's own chart visuals, `histogram` (roles `values`, `frequency`), `bullet` (`category`, `value`, `target`, `bands` up to 4, `maximum`), `box_plot` (`category`, `sample`, `value`: quartiles, whiskers at 1.5 IQR, outliers) or `dumbbell` (`category`, `start`, `end`; `style.slope` draws a slope chart); embeds its package (built from BIcraft's TypeScript by its own bundler) when missing or older, moves columns to its grouping roles and measures to its measure roles (`dropped`: the rest) and sets its checked format options (`{bins: {count: 20}}`, null removes one). Power BI draws it with the visual's script, `report.render`, PDF export and the app with the same layout (docs/formats/bicraft-visuals.md) |
| `page.remove` | `{page}`: also removes its visuals; the last page cannot be removed |
| `page.rename` | `{page, display_name}`: the display name; the page name (folder) never changes |
| `page.move` | `{page, index}`: 0-based |
| `page.set_active` | `{page}`: the page the report opens on |
| `visual.set_position` | `{page, visual, x?, y?, z?, width?, height?}` |
| `visual.place` | `{page, visual, col, row, col_span?, row_span?, grid?: {columns?, rows?, margin?, gutter?}}`: puts a visual on a grid cell (see *Layout*) |
| `page.arrange` | `{page, layout?, visuals?: [name \| {visual, place?}], dry_run?}`: lays out a page on shelves or a grid (see *Layout*) |
| `visual.unbind` | `{page, visual, role, field?}`: without `field`, empties the role |
| `visual.remove` | `{page, visual}`: a group goes with its members |
| `visual.set_title` | `{page, visual, title}`: `"text"`, `{text?, show?, font_size?, color?, alignment?, bold?, italic?}`, or `null` (see "Visual properties") |
| `visual.set_sort` | `{page, visual, field, direction?: "asc"\|"desc"}`: `field: null` removes the sort |
| `visual.filter` | `{page, visual, field, in \| not_in \| op + value \| conditions + logic? \| top + by + direction?, hidden?, locked?}`: adds the filter, or replaces the one on that field |
| `visual.clear_filters` | `{page, visual, field?}`: every visual-level filter, or those on one field |
| `visual.format` | `{page, visual, background?, border?, padding?, shadow?, header?, alt_text?, subtitle?, data_labels?, legend?, x_axis?, y_axis?, objects?}` |
| `visual.props` | `{page, visual}`: read-only; the visual's `props` (title, sort, filters, format, series, conditional) |
| `visual.copy_format` | `{page, from, to: name \| [names], to_page?}`: the format painter, in one undo step: each target gets the source's formatting objects its type has, but the title and subtitle text, data label display units and decimals, axis start and end, the legend's title text, its fields, filters, size, position, alt text and action; colours and labels set for one field's values only when the target binds that field ([formats/format-painter.md](formats/format-painter.md)). Returns `{page, from, changed, changes: [{visual, objects}]}` |
| `visual.series` | `{page, visual, target, color?, labels?, line?, objects?, clear?}` or `{page, visual, series?, colors?}`: per-series / per-category formatting (see "Series and conditional formatting") |
| `visual.conditional` | `{page, visual, target?, series?, apply_to?, rule}` (`rule: null` removes) or `{page, visual, conditional: [...]}` |
| `visual.set_data_format` | `{page, visual, role, field? \| index?, type?: auto\|general\|whole\|decimal\|percentage\|currency\|scientific\|date\|custom, format?, decimals?, thousands?, currency_symbol?, date_format?}`: Format pane › General › Data format, a format string for this visual only, written as the projection's `format` (the model is not changed; `auto` or `format: null` removes it); the preview's data column takes it as `format_string` ([formats/sparklines.md](formats/sparklines.md)) |
| `visual.add_sparkline` | `{page, visual, y, summarization?, x, chart_type?: line\|column, color?, width?, markers?: [highest, lowest, first, last, all], marker_color?, marker_shape?, marker_size?, apply_calculation_group_to?: point\|sparkline}`: a sparkline in a table or matrix (at most 5), a `SparklineData` projection in Values and an `objects.sparklines` entry selected by its queryRef; the preview gives each row `{"sparkline": [[x, y], …]}` (at most 52 points) ([formats/sparklines.md](formats/sparklines.md)) |
| `visual.edit_sparkline` | `{page, visual, index?, y?, summarization?, x?, …look}`: changes a sparkline's axes or look, keeping its queryRef; `visual.unbind` removes it |
| `visual.options` | `{visual_type?, object?}`: read-only; the formatting catalog (see "Visual formatting catalog") |
| `visual.axis` | `{page, visual, axis: "x"\|"y"\|"y2", show?, title?, title_text?, min?, max?, start_at_zero?, scale?: "linear"\|"log", gridlines?, display_units?, precision?, font_size?, color?, invert?, type?, clear?}` (see "Chart specifics") |
| `visual.analytics` | `{page, visual, lines: [...] \| null}`: the complete list of constant / average / min / max / median / percentile / trend / forecast lines, error bars, the scatter's ratio line and symmetry shading |
| `visual.small_multiples` | `{page, visual, field?, rows?, columns?, title?}`; `{field: null}` alone removes them |
| `visual.tooltip_page` | `{page, visual, tooltip_page: page \| "default" \| "none"}` |
| `page.set_type` | `{page, type: "report"\|"tooltip"\|"drillthrough", fields?, keep_all_filters?, cross_report?, back_button?, width?, height?, hidden?}` |
| `page.interactions` | `{page, rules: [{source, target, type: "filter"\|"highlight"\|"none"\|"default"}], clear?}` |
| `report.settings` | `{default_interaction?: "highlight"\|"filter", drill_filters_other_visuals?, export_data?: "AllowSummarized"\|"AllowSummarizedAndUnderlying"\|"None", persistent_filters?, personalize_visuals?, cross_report_drillthrough?, filter_pane_visible?, allow_change_filter_types?}`: Desktop's Report settings of the current file (`docs/formats/pbir.md` "Report settings"): the report's interaction defaults, persistent filters, personalize visuals, cross-report drillthrough, the Filters pane's visibility to readers, whether readers may change filter types, and its Export data setting (`settings.exportDataMode`: what readers may export; a report without one allows summarized data, as Desktop's new reports; `visual.export_data` honours it, the snapshot shows it as `report.export_data_mode` and `report.settings.export_data_mode`); no parameters reads them |
| `report.pause_visuals` | `{paused?: bool (default true)}`: Optimize › Pause visuals (`settings.pauseQueries`, saved): paused visuals keep their last data, marked `pending`, until `report.refresh_visuals` or a resume |
| `report.refresh_visuals` | `{}`: Optimize › Refresh visuals: the next view evaluates every visual once; read-only |
| `visual.refresh` | `{page, visual}`: a paused visual's own Refresh: the next view evaluates that visual once, the others staying paused; read-only |
| `report.query_reduction` | `{preset?: "query_reduction"\|"interactivity", disable_cross_highlighting?, slicers?: "instantly"\|"apply_button", filters?: "instantly"\|"apply_buttons"\|"single_button"}`: Optimize › Optimization presets (`slowDataSourceSettings`); the viewer honours the default interactions being off |
| `visual.drill_filter` | `{page, visual, apply_to: "page"\|"visual"\|"default"}`: Desktop's "Apply drill down filters to" (`drillFilterOtherVisuals`); `default` follows `report.settings` |
| `view.keep_drill` | `{page?, visual?}`: writes the reader's drill and matrix expansion into the report as Desktop saves them (`active` flags, `howCreated: Drill` filters, `expansionStates`) |
| `visual.expand` | `{page, visual, role?, path? \| level? \| all? \| step?, collapse?}`: expands or collapses an authored matrix (Selection, Entire level, All, or the header steps `expand_all`, `next`, `up`) and writes its `active` flags and `expansionStates` as Desktop saves them; undoable |
| `visual.order` | `{page, visuals: [front to back]}` or `{page, visual, to: "front"\|"back"\|"forward"\|"backward"}` |
| `visual.mobile` | `{page, visual, x, y, width, height, snap?}` or `{page, visual, layout: null}` (`snap: true`: on the phone grid of Power BI Desktop, 18 × 20 cells inside its 324 width) |
| `theme.get` | `{}`: read-only; the custom theme (file, JSON, schema `issues`), the base theme reference, and the `resolved` colors and fonts the viewer uses (see "Report themes") |
| `theme.set` | `{theme, force?}`: install a custom theme (a theme JSON object or its text), replacing the current one |
| `theme.import` | `{path, force?}`: the same from a theme JSON file (at most 4 MB; BOM and CRLF accepted) |
| `theme.export` | `{path, overwrite?}`: read-only; writes the custom theme file (else the embedded base theme) |
| `theme.remove` | `{}`: back to the base theme; the theme file and its resource item go too |
| `theme.set_colors` | `{data_colors?, background?, foreground?, table_accent?, good?, neutral?, bad?, maximum?, center?, minimum?, force?}` |
| `theme.set_text_classes` | `{callout?, title?, header?, label?, largeTitle?, …: {font_face?, font_size?, color?, font_weight?} \| null, force?}`: theme text classes |
| `theme.set_visual_style` | `{visual_type ("*" = every visual), object, property, value, state?, force?}`: a `visualStyles` default |
| `theme.customize` | `{name?, data_colors?, background?, …, minimum?, text_classes?: {label?, title?, callout?, header?, …}, styles?: [{visual_type, object, property, value, state?}], force?}`: the Customize theme dialog's Apply: the name, colours, text classes and visual styles in one edit (one undo step), each part checked as the three commands above check it |
| `edit.undo` | `{steps? (default 1)}`: undo the last edits; returns `{undone: [{label, commands, at_ms}], can_undo, can_redo, dirty, report_dirty}` |
| `edit.redo` | `{steps? (default 1)}`: redo what `edit.undo` undid; any new edit clears it |
| `edit.history` | `{}`: read-only; `{undo, redo}` (next step first), `can_undo`, `can_redo`, `bytes` (estimated snapshot memory) and `limits` |
| `model.get` | `{detail?: "summary" \| "full", culture?}`: read-only; the model with every property it holds (see *Model reads*; `culture`: see *Translations*) |
| `table.get` / `column.get` / `measure.get` | `{name}` / `{table, name}` / `{name, table?}`, each with `culture?`: read-only; one object with every property |
| `culture.add` / `culture.remove` | `{name}`: add an empty culture (e.g. `fr-FR`) / remove one with its translations (see *Translations*) |
| `translation.set` | `{culture, object, property: "caption" \| "description" \| "display_folder", value \| null}`: set or remove one translated property |
| `translation.list` | `{culture?}`: read-only; the cultures and every translated property |
| `view.set_culture` | `{culture \| null}`: captions in that culture in the viewer snapshot and `report.render` (which also takes `culture` for one call) |
| `view.set_language` | `{ui_language}`: the UI language of the text Power BI Desktop generates (auto titles "Total by Region" / "Total par Region", implicit-measure names "Sum of X", "(Blank)", "Total", slicer "All" / "Multiple selections") in the viewer snapshot and `report.render`; a BCP 47 tag, phrase tables for en, fr, de, es, it, nl, pt, ja, other languages use English; default en-US. Desktop uses its own UI language, not the model culture. See [view-snapshot.md](view-snapshot.md) *UI language* |
| `relationship.list` | `{table?}`: read-only; every relationship with cross filtering, cardinalities, activity and security filtering |
| `model.storage_stats` | `{table?, columns?, segments?, row_number?}`: read-only; VertiPaq sizes from the stored data (see *Storage statistics and VPAX*) |
| `model.export_vpax` | `{path, overwrite?, model_bim?}`: writes a VPAX file for VertiPaq Analyzer / DAX Studio; also `bicraft-cli vpax PATH OUT.vpax` |

`table.rename`, `column.set`, `column.rename`, `measure.set`, `measure.rename` and `measure.move` also
take `update_report: true` (see below).

`bicraft-cli describe ID` prints any command's parameters; `list_commands` returns the same text.

## Buttons, shortcuts and forms

Every command declares where its button lives in the app (ROADMAP M11.1, AGENTS.md §9); the full
table is [commands-ui.md](commands-ui.md), generated by `cargo xtask commands-ui`. Each entry of
`list_commands` (and `bicraft-cli commands`, `bicraft-cli describe ID`) carries, next to `id`, `label` and
`params`:

- `place`: `{kind: "ribbon", tab, contextual, group, label, icon}`, `{kind: "menu", menu: page |
  visual | model_object, label, icon}` or `{kind: "palette", label, icon}`. Tabs: `file` (the
  backstage), `home`, `report`, `model`, `data`, `analyze`, `view`, and the contextual `visual`,
  `table`, `measure`.
- `shortcut`: a key chord such as `"Ctrl+S"` or `"F5"`, or null; unique across commands
  (Ctrl+Shift+P stays the palette's).
- `needs`: what the session must hold before the button is enabled: `file`, `saved_report_file`,
  `project`, `model`, `report`, `no_report`, `data`, `undo`, `redo`.
- `form`: the parameters as a schema the palette builds its form from: `[{name, kind, required,
  object?, values?}]`, `kind` one of `text`, `name` (an existing object; `object` says which:
  `table`, `column`, `measure`, `page`, `visual`, `data_role`, …), `field` (`Table[Column]`,
  `[Measure]`), `dax`, `m`, `code` (DAX or M), `path`, `paths`, `path_or_text`, `bool`,
  `integer`, `number`, `enum` (`values`), `color`, `list`, `json`. Path parameters are exactly
  those the server policy resolves.
- With a session (the MCP `list_commands` tool): `enabled` and, when false, `disabled_reason`,
  from the needs, the server policy and the caller's role (`Session::command_enabled`, no side
  effects). Without one (`bicraft-cli commands`) both are absent.

## Explicit data cache (`cache_abf`)

A PBIP project's data normally comes from its own `<Name>.SemanticModel/.pbi/cache.abf`. To let a
throwaway copy of a project (without `.pbi`) read the original's cache without copying or linking
it, name the file explicitly:

- `file.open {path, cache_abf}` sets it for the session (another `file.open` resets it);
  `file.set_cache_abf {cache_abf: path|null}` changes it later (null: the project's own again) and
  returns `{cache_abf, project_cache_abf}`; `file.inspect` shows it.
- `dax.evaluate`, `report.render`, `model.storage_stats` and `model.export_vpax` also take a
  `cache_abf` parameter for that one command (MCP `evaluate_dax_query` and `render_page` too).
- CLI: `--cache-abf CACHE.abf` on `dax-query`, `render`, `vpax` and `view`. (`analyze` reads no
  data, so it has none.)
- The explicit file wins over the discovered one. It must exist and be readable: a missing file is
  an error, never a silent fallback, also when it disappears after it was set. It is opened
  read-only and never written or copied: `file.save` writes only the model and report, and
  `file.save_as` refuses a project whose cache would land on it. A `file.save_as` to a new file,
  `project.new` and `model.new` drop it. Only a PBIP takes one (a PBIX keeps its data inside).
- `model.storage_stats` reports `source: "cache_abf"`; `dax.evaluate` reports `data_source:
  "cache.abf"`.

## Refreshing data with BIcraft (`model.refresh`)

`model.refresh {tables?, allow_connectors?, external_links?, data_root?, force?, backup?}` (MCP
`refresh_model`, CLI `bicraft-cli refresh PATH [--allow-connector databricks]... [--table T]...
[--external-links] [--data-root DIR] [--backup]`; the ribbon's Home › Refresh and F5, a table's
right-click "Refresh data" in the Data pane and the model diagram) refreshes an open **PBIX
file's** or **PBIP project's** import tables without Power BI Desktop. The sources run the same
way for both; where the rows go differs (*On a PBIX* below):

- M partitions run in BIcraft's Power Query evaluator; `DateTimeZone.UtcNow()` is the refresh
  time; files only under `data_root`.
- **Connectors only when allowed.** `allow_connectors: ["databricks"]` lets bicraft-cli run
  `DatabricksMultiCloud.Catalogs` / `Databricks.Catalogs` through the Databricks SQL Statement
  Execution API of the SQL warehouse in the query's HTTP path, on the host the query names and no
  other (`external_links: true` also lets large results come from presigned cloud-storage links,
  without the token). Without it a query that reads an external source fails as before, and the
  error says to pass `allow_connectors`. Column selections, simple row filters and row limits
  fold into the SQL; the rest runs locally.
- **Token.** Read from `BICRAFT_DATABRICKS_TOKEN` in the environment of the process (the MCP
  server's, the CLI's); the CLI asks for it, masked, when the variable is unset on an interactive
  console. No parameter or option takes a token. It is read when the first request is made and
  never written, logged or returned.
- Calculated tables and columns are then evaluated by BIcraft's DAX evaluator (PBIP:
  `not_computed` lists the ones it cannot evaluate; PBIX: nothing is written then).
- **Where the rows go (PBIP).** `<Name>.SemanticModel/.pbi/bicraft-data.bin`, BIcraft's own format,
  written only when every import table loads (atomically). `.pbi/cache.abf` is never read,
  written or copied. Add `**/.pbi/bicraft-data.bin` to `.gitignore` (projects BIcraft creates
  have it; the result warns otherwise): the file holds the source's rows.
- **Who reads them.** `dax.evaluate` (`data_source: "bicraft-data"`), `report.render` and the
  viewer read the snapshot while it is newer than the project's `cache.abf` and no explicit
  `cache_abf` is set; a table whose query or columns changed since is stale and not read.
- **Result.** `{path, refreshed_at, bytes, tables: [{table, origin: "m"|"dax", rows, columns, ms,
  statements: [{host, sql, rows, ms}]}], kept, skipped, not_computed: [{table, column, reason}],
  connectors, elapsed_ms, warnings}`.

### On a PBIX

The new rows are written into the file's `DataModel`, as Power BI Desktop's Refresh writes them,
so Desktop (and every other reader) opens the file with the new data:

- **What is refreshed.** Without `tables`: every import table (`m` partitions, import mode) and
  every calculated table. With `tables`: those (Desktop's "Refresh data" on a table), plus what
  reads them, found through the model's DAX dependencies: calculated tables that read them are
  evaluated again (`origin: "dax"`), and a table whose calculated columns read them keeps its
  stored rows and gets those columns recomputed (`origin: "stored"`; its source is not run). A
  calculated column that reads refreshed data in a table BIcraft cannot refresh (DirectQuery, a
  policy range) is listed in `stale`. Every other table keeps its storage files byte for byte;
  the relationships that touch a refreshed table get new indexes (dual tables are refreshed as
  import ones). DirectQuery,
  incremental-refresh policy and other partition kinds are `skipped` with the reason.
- **What is written.** The structural writer replaces each refreshed table (VertiPaq encoding,
  `H$`/`R$` system tables, the same name, lineage tags, measures, hierarchies, relationships,
  roles and perspectives); its partitions get `State` 1 (Ready) and `RefreshedTime` =
  `ModifiedTime` = the refresh time, and `LASTCUBEDATACHANGE` is raised to it, as Desktop does
  ([formats/metadata-sqlitedb.md](formats/metadata-sqlitedb.md), *Refresh state*). The model's
  definition is unchanged.
- **Never a corrupt file.** Nothing is written unless every table loads and every relationship
  still holds (a "one" side with a duplicate or a blank is refused, as Desktop refuses it). The
  package goes to a `<name>.bicraft-tmp` sibling, is reopened with BIcraft's reader (the model's
  structure compared with the refreshed one, every refreshed table's row count checked), then
  renamed over the file; any error removes the temporary file and leaves the file as it was.
  `backup: true` (`--backup`) keeps the previous file as `<name>.bak`.
- **Unsaved edits.** Refused while the model has unsaved edits; `force: true` refreshes the file's
  saved model and keeps the edits unsaved (a later `file.save` writes them over the refreshed
  file). A PBIT holds no data and a thin report has no `DataModel`: both are refused.
- **Who may.** An editor (a read-only role is refused, as for any edit); in server mode the file
  is the workspace's and the per-file check of `file.save` applies: a refresh over a file another
  session saved or refreshed since this one read it is refused with `file_conflict`, and a
  refresh is a new revision of the file for the other sessions ("Document views").
- **Memory.** The `DataModel` part, its decompressed backup and each refreshed table's rows (as
  they load) are reserved against the process' memory limit (`BICRAFT_MEMORY_LIMIT`,
  [beyond-ram.md](beyond-ram.md)): what does not fit is refused before anything is written. The
  other tables' data is never decoded. A table is held in memory column by column while it is
  encoded (not streamed to disk), at most 16,777,216 rows per table.
- **Result.** `{path, format: "pbix", refreshed_at, tables: [{table, origin: "m"|"dax"|"stored",
  rows, columns, partitions, ms}], kept, skipped: [{table, reason}], stale: [{table, column,
  reason}], connectors, statements, bytes, previous_bytes, backup, verified, phases_ms: {load,
  encode, write_verify}, elapsed_ms, warnings}`. The app shows it in the answer window: a line
  (tables refreshed, kept, time) and the per-table grid, with a spinner while it runs.

```bash
bicraft-cli refresh sales.pbix --data-root ./data             # every table
bicraft-cli refresh sales.pbix --data-root ./data --table Product  # one table and what reads it
```

```bash
export BICRAFT_DATABRICKS_TOKEN=…   # set by the user, never in a file
bicraft-cli refresh projects/perso/perso.pbip --allow-connector databricks
bicraft-cli dax-query projects/perso/perso.pbip "EVALUATE ROW(\"Dépenses\", [Dépenses])"
```

Details: [formats/data-sources.md](formats/data-sources.md), *Connectors* and *Refresh by
BIcraft*.

## Running DAX queries

`dax.evaluate {query, max_rows?, timeout_ms?, allow_cache_abf?, cache_abf?}` (MCP `evaluate_dax_query`, CLI
`bicraft-cli dax-query PATH (QUERY | --file QUERY.dax) [--max-rows N] [--timeout-ms N]
[--no-cache-abf]`) runs a DAX query over the open file's data with bicraft's own evaluator (the
one behind the viewer and calculated tables; no Power BI Desktop, also on Linux). Read-only.

- **Data**: a PBIX's `DataModel`; a PBIP's BIcraft data snapshot (`model.refresh`) while it is
  newer than `.pbi/cache.abf` and no explicit `cache_abf` is set, else its `.pbi/cache.abf`;
  otherwise each table's partitions (M
  queries in BIcraft's Power Query evaluator, files only under `data_root`; DAX literal tables).
  `allow_cache_abf` (default `true`) lets a caller whose rules forbid reading `cache.abf` refuse
  it: with `false` only the partitions are used, and a table that has no other source fails with a
  message saying `cache.abf` was not read. `data_source` in the result says which was used
  (`DataModel`, `bicraft-data`, `cache.abf` or `partitions`). Decoded columns are cached with the viewer's until
  the file changes.
- **Query**: `[DEFINE …] EVALUATE … [ORDER BY …] [START AT …]`, several `EVALUATE`s allowed. The
  query is validated against the model first (as `dax.validate_query`); an invalid one is refused
  with its diagnostics. `DEFINE MEASURE` adds (or overrides) a measure for this query only,
  `COLUMN` computes a column row by row, `VAR` and `TABLE` are evaluated once and seen by the
  `EVALUATE`s (not by measures). `DEFINE FUNCTION` defines (or overrides) a user-defined function
  for this query, and calls of the model's functions are evaluated ("DAX user-defined functions"
  below); `@parameters` are refused. The model is not changed.
- **Result**: `{results: [{columns: [{name, type}], rows: [{name: value}], row_count, truncated}],
  data_source, elapsed_ms}`, one entry per `EVALUATE`. Model columns are named `Table[Column]`,
  other columns `[Name]`, as the engine's clients show them. Types are TMSL names (`int64`,
  `double`, `decimal`, `dateTime`, `string`, `boolean`). Blank is `null`, dates are ISO text
  (`2024-02-29T00:00:00`), currency is a number, infinities are `"Infinity"`/`"-Infinity"`.
  `row_count` counts every row (after `START AT`); `rows` stops at `max_rows`.
- **Never a wrong number**: a function or construct the evaluator does not handle is an error
  naming it (`not supported by bicraft's DAX evaluator: PERCENTILE.INC() is not supported`).
  Evaluation errors the engine also raises (text to number, a table where one value is expected)
  are errors too.
- **Caps**: `max_rows` (default 10,000, at most 1,000,000 per `EVALUATE`), `timeout_ms` (default
  30,000, at most 600,000), 20 billion row visits, 2 million grouping combinations per
  `SUMMARIZECOLUMNS`, 2 million rows per table function, 20 million result cells, 1 MB of query
  text.

Agreement with Analysis Services on a corpus of queries is measured in `docs/dax-oracle.md`.

### DAX user-defined functions

[formats/dax-udf.md](formats/dax-udf.md). Each is one undo step and needs a model with tables;
the compatibility level is raised to 1702.

- **`function.add {name, expression? | body? + parameters?, description?, hidden?, force?}`**:
  `expression` is the whole `(a : SCALAR NUMERIC, b = 0) => body`; or `body` with `parameters:
  [{name, type?, subtype?, mode?: val|expr, default?}]`. The definition is validated as `DEFINE
  FUNCTION name = …` against the model (invalid DAX is refused unless `force`); names follow
  Learn's rules (letters, digits, underscores, periods for namespaces, not a DAX function);
  recursion is refused. Returns `{function: {name, expression, parameters, body}}`.
- **`function.set {name, …}`**: the same keys; a `body` alone keeps the parameters, `parameters`
  alone keep the body; `description: null` clears it. The model's DAX that calls the function and
  no longer validates is listed in `broken`.
- **`function.rename {name, new_name}`**: calls in measures, calculated columns and tables, row
  filters, calculation items and other functions follow (`calls_rewritten`).
- **`function.remove {name, force?}`**: refused while DAX calls it (`used_by`), unless `force`.
- Calls are checked by `dax.validate` and every measure edit: too few or too many arguments
  (`wrong-argument-count`), a table for a scalar parameter or a scalar for a table one
  (`argument-type`). Measures and queries that call a function are evaluated.

### TMDL scripts (the TMDL view)

[formats/tmdl-view.md](formats/tmdl-view.md).

- **`tmdl.script {objects?: [{kind, name, table?}]}`** (read-only): a `createOrReplace` script of
  model objects (`model`, `table`, `column`, `measure`, `hierarchy`, `partition`, `function`,
  `expression`, `relationship`, `role`, `perspective`, `culture`); a table's members go under
  `ref table`. Without objects, the whole model. Returns `{script, count}`.
- **`tmdl.apply {script, force?}`**: applies a `createOrReplace` script in one undo step. Each
  declared object is added, or replaces the model's object of that kind and name with all its
  descendants; members under `ref table T` replace or add T's. A script that does not parse, an
  unknown `ref` table, an object it cannot apply or DAX that does not validate (unless `force`)
  changes nothing, and the error says `line L, column C: …` in the script. Returns `{applied:
  [{kind, name, table?, action: added|replaced}], warnings, diagnostics}`.

## Data view reads (ROADMAP M11.17)

Three read-only commands behind the app's Data view ([desktop.md](desktop.md) "Data view"),
also useful to an agent. Each runs on a shared session (`bicraft-cli serve` runs several at once) and
is skipped by auto-recovery (`replay: Skip`); readers may run them, except `query.preview` with
`allow_connectors`.

- **`table.rows {table, offset?, limit?, columns?, sort?, filter?}`**: one page of a table's
  rows (`limit` 1–10,000, default 100) from the same data `dax.evaluate` reads (the PBIX
  `DataModel`, a PBIP's data, the partitions). `sort` is one column (`{column, descending?}`),
  `filter` a list of conditions that must all hold: `{column, values: [...]}` keeps those values
  (`in`), `{column, op, value}` compares (`=`, `<>`, `>`, `>=`, `<`, `<=`, `contains`,
  `not_contains`, `starts_with`, `not_starts_with`, `ends_with`, `not_ends_with`, `not_in`,
  `blank`, `not_blank`; text without regard to case, dates as ISO text), and `{column, any:
  [{op, value}]}` (1 to 8 tests) keeps a value that passes one of them: the Data view's Text,
  Number and Date filters send their Custom filter's two tests as two conditions (And) or one
  `any` (Or). Both are view state: the model is not changed. Returns
  `{table, available, columns: [{name, data_type, kind: data|calculated, format_string,
  summarize_by, data_category, sort_by_column, hidden, available, computed?, reason?}], rows:
  [[...]], offset, limit, row_count, total_rows, sort, filtered, order_cached, elapsed_ms}`
  (`available: false` with a `reason` when the table has no data). How it stays fast on tens of
  millions of rows: a page without a sort or filter reads `limit` codes per column; a sort ranks
  the column's distinct values once and orders the rows with a counting sort (stable: equal
  values keep the table's order; blank first ascending, last descending); a filter is evaluated
  once per distinct value, then the rows are read in blocks of 65,536 codes; the order is kept in
  the session (four of them) until the data or the model changes, so the next pages are lookups
  (`order_cached: true`). The order and the block buffers are reserved against the process'
  memory limit (`BICRAFT_MEMORY_LIMIT`) first, and the session drops them before its columns when
  memory is short. A calculated column the data does not hold is computed by the DAX evaluator
  for the page's rows only; sorting or filtering on it computes it over the table (up to 2,000,000
  rows, else refused with the reason) and keeps it. Measured by `table_rows_bench`
  ([benchmarks.md](benchmarks.md), "Data view paging").
- **`query.preview {name | expression, partition?, limit?, data_root?, allow_connectors?,
  timeout_ms?}`**: the first rows (default 1,000, at most 10,000) of a Power Query query, as the
  Power Query editor previews it, without loading anything: a shared expression (`name`), a
  table's M partition (`name` of the table, `partition` to pick one), or M text. Evaluated by
  BIcraft's M evaluator as `Table.FirstN(query, limit + 1)` (a source that folds a row limit reads
  no more), with the model's other queries, parameters and culture; files only under `data_root`
  (else the session's). External connectors run only when named in `allow_connectors`, as for
  `model.refresh`: such a run counts as writing for the server's roles (a reader is refused) and
  runs exclusively, and a server refuses it unless it allows network access
  (`BICRAFT_SERVER_NETWORK`). Returns `{name, source: query|table|expression, columns: [{name,
  data_type}], rows, row_count, truncated, connectors, elapsed_ms}`; a query that is not a table
  (a parameter) is an error saying so.
- **`visual.records {page, visual, point?, points?, limit?}`**: See records of a data point (the visual's
  right-click menu in the app). `point` is what the renderer's hit map gives a click, as
  `view.interact` `select` takes it (`[{c, v}]`); a mark that stands for several points (a
  histogram bin, a Venn set of BIcraft's visuals) also gives `points` (at most 1,000), and the rows are
  then those of every point (`EVALUATE UNION(CALCULATETABLE(…), …)`, one per point; the answer's
  `points` counts them); the rows are those of the table the visual
  summarizes (the table its first measure's expression reads, the one `RELATED` reaches the
  others from when it reads several, following the measures it calls when it reads none; else
  its home table; else its first aggregated or grouping column's table) under the point's
  values, through the model's relationships, computed by the DAX evaluator
  (`EVALUATE CALCULATETABLE('Sales', 'Product'[Category] = "Toys")`, in `query`). When that
  measure has a detail rows expression, its rows are those of the expression instead
  (`CALCULATETABLE(<the expression>, …)`, `detail_rows: true`; one that fails falls back to the
  table, named in `not_applied`).
  The report's, the page's and the visual's filters and the page's slicer selections (the
  reader's view state included) are added as filter arguments (`FILTER(ALL(col), cond)`, a top N
  as `TOPN` under the other filters; listed in `filters`, `filters_applied: "point and
  filters"`). A filter on a measure or an aggregation keeps the points it keeps:
  `CALCULATETABLE(FILTER(VALUES(<grouping column>), [Total] > 50), <the column filters>)`
  (`SUMMARIZE` of the records' table by the grouping columns when there are several), so the
  measure is computed under the other filters as the visual computes it. Grouping columns the
  records' table cannot reach through `RELATED` (a table with no relationship, the many side)
  do not filter its rows: a point is kept when one of its combinations with their values meets
  the condition (`FILTER(VALUES('Product'[Category]), NOT(ISEMPTY(FILTER(VALUES('Region'[Name]),
  [Total] > 75))))`). On a card (no grouping column the records' table reaches) the condition is
  computed once, under the other filters: when it fails the card is blank and the records are
  empty (`FILTER(ALL(<a column>), FALSE())` in `query`). A condition over
  columns of several tables (a hierarchy slicer's tuples, an `Or` across tables) filters the
  records' rows, the other tables' columns through `RELATED` (`FILTER('Sales',
  RELATED('Product'[Category]) = "Games" || 'Sales'[Amount] > 35)`) when they are on the one side
  of active relationships from it. When another visual's selection cross-highlights the visual,
  the rows are those of the highlighted part (the selection is added as a filter, as the
  highlight is computed) and `cross_highlight` names the source; a cross-filter applies as a
  filter. What cannot be applied (a measure condition that also reads a column the visual does
  not group by, a condition over several tables `RELATED` cannot reach) is named in
  `not_applied`. Returns `{page, visual, table, detail_rows, point, query, columns, rows,
  row_count, truncated, filters_applied, filters, not_applied, cross_highlight}`.
  Following the highlight is BIcraft's choice, not yet compared with Desktop's See records on a
  cross-highlighted visual.
- **`visual.analyze {page, visual, point, kind?, fields?, top?}`**: Analyze a data point (the
  visual's right-click menu in the app, *Analyze this point*): BIcraft's own ranking of the model's
  columns, not Power BI's result ([analyze-point.md](analyze-point.md)). `kind: "change"` (the
  default on a date or number axis) compares the point with the previous point of its axis and
  scores each field by how much of the change one of its values carries; `kind:
  "distribution"` (the default on a categorical axis) scores each field by the total variation
  distance between its shares under the point and the mean of its shares under the other
  points. The visual's filters, slicers and the model's relationships apply. Returns `{kind,
  measure, point, change | value, candidates: [{field, score, member, …}], skipped: [{field,
  reason}], estimate}`; a point that cannot be analyzed (the first point of a change, a change on
  a categorical axis, no measure) is an error.
- **`visual.explain_anomaly {page, visual, point?, compare?, fields?, top?}`**: Explain an
  anomaly that a line chart's Find anomalies flags (an anomaly's right-click menu in the app,
  *Explain the anomaly*): BIcraft's own explanation of its own anomaly estimate, not Power BI's
  ([analyze-point.md](analyze-point.md#explain-an-anomaly)). Without `point`, the anomaly furthest
  outside its expected range. `compare: "expected"` (the default) runs each field's values
  through the same detection as the line and scores the field by how much of the members'
  deviations from their expected values one of its values carries; `compare: "previous"` is
  `visual.analyze`'s change from the previous point. Returns `{compare, measure, anomaly: {at,
  value, expected, low, high, deviation, direction}, anomalies, candidates, skipped, estimate}`;
  a visual without anomaly detection, a point that is not an anomaly or a line without one is an
  error.

`column.add` also takes a calculated column without `data_type`: the type is the expression's
(the DAX checker's result type; text when it depends on the rows), as Desktop's New column
infers it.

## Model reads

`model.summary` stays the compact listing (names, data types, measures, relationships, row
filters). The detailed reads return every property the model holds, in one stable snake_case
shape (`bicraft-cli describe model.get` lists every key):

- `model.get {detail: "summary"}` (default): model properties (`culture`, `default_mode`,
  `discourage_implicit_measures`, `source_query_culture`, ...), `counts` of every kind of object,
  and one line per table (`kind`: `data`, `calculated` or `calculation_group`, column / measure /
  hierarchy counts, partitions with mode and source type).
- `model.get {detail: "full"}`: the same model properties plus every table as `table.get`
  returns it, every relationship as `relationship.list` does, roles (members, table permissions
  with row filters, metadata and column permissions), shared expressions (M queries and
  parameters), perspectives, cultures, data sources, query groups and functions (these five as
  the TMSL objects the file holds), model annotations, and the database properties.
- `table.get {name}`: `columns`, `measures`, `hierarchies` (levels with their columns),
  `partitions` (`mode`, `source_type`: `m`, `calculated`, `query`, `entity`, `policyRange`, ...,
  `expression`: the M or DAX text, `data_view`, `query_group`, `data_source`, other `source`
  properties), `calculation_group` (`precedence`, selection expressions and `items` with their
  ordinal, expression and dynamic format string), `refresh_policy`, `data_category`, `hidden`,
  `is_private`, lineage tags.
- `column.get {table, name}`: `kind` (`data`, `calculated`, `calculated_table_column`),
  `data_type`, `source_column`, `expression`, `format_string`, `display_folder`, `summarize_by`,
  `sort_by_column`, `data_category`, `description`, `hidden`, `is_key`, `is_unique`,
  `is_nullable`, `is_available_in_mdx`, `is_name_inferred`, `is_data_type_inferred`,
  `encoding_hint`, `lineage_tag`, `source_lineage_tag`, `alternate_of`, `variations`, and the
  `relationships` that use it.
- `measure.get {name, table?}`: `expression`, `format_string`, `format_string_definition` (the
  dynamic format string's DAX), `display_folder`, `description`, `hidden`, `data_type`,
  `detail_rows_definition`, `kpi`, lineage tags.
- `relationship.list {table?}`: `from` / `to` (`Table[Column]`, and the four parts),
  `is_active`, `cross_filter` (`single`, `both`, `automatic`) with the TMSL
  `cross_filtering_behavior`, `from_cardinality` / `to_cardinality`,
  `security_filtering_behavior`, `rely_on_referential_integrity`, `join_on_date_behavior`.
- **Refresh state.** When the open file has stored data (a PBIX `DataModel` or a PBIP
  `.pbi/cache.abf`), `table.get` and `model.get {detail: "full"}` add read-only `refresh` blocks
  read from the backup's `metadata.sqlitedb`: each partition gets `refresh: {state, state_code,
  refreshed_time, modified_time, error_message}` (`state`: the TOM name, `Ready`, `NoData`,
  `CalculationNeeded`, `SemanticError`, `EvaluationError`, `DependencyError`, `Incomplete`,
  `ForceCalculationNeeded`; `error_message` as the engine stores it, an XML list of HRESULTs and
  arguments), each table `refresh: {last_refresh, modified_time, structure_modified_time}`, and
  the model `refresh: {source, last_refresh (latest partition refresh), last_data_change,
  last_schema_change, modified_time, structure_modified_time, partition_count,
  partitions_not_ready: [{table, partition, state}]}`. Times are ISO-8601 UTC
  (`2026-10-14T09:30:00.123Z`); `null` means never refreshed. Objects are matched by name, so a
  partition added since the last save has no block; a PBIT, a `model.bim`, a project without
  `cache.abf` or a new model has none. `model.storage_stats` reports the same values on its
  partitions and as `refresh`. BIcraft's own saves write `Ready` and the save time for the
  partitions whose data they materialize and keep the stored values otherwise (details:
  `docs/formats/metadata-sqlitedb.md`, "Refresh state").
- **The same state in DAX.** `dax.evaluate` runs `INFO.PARTITIONS()` (`RefreshedTime`, `State`),
  `INFO.TABLES()`, `INFO.MODEL()`, `INFO.COLUMNS()` (`ModifiedTime`, `StructureModifiedTime`),
  `INFO.MEASURES()`, `INFO.RELATIONSHIPS()` and the four `INFO.VIEW.*`, with the values above as
  DAX date/times (UTC; blank when never refreshed). Only a query may call the `INFO.*` functions
  other than `INFO.VIEW.*`, as in Power BI: a measure, calculated column or table that uses one
  is refused (`info-query-only`). Columns and values: `docs/formats/dax-functions.md`, "INFO
  functions".
- **Refresh info table.** To show the last refresh in a visual, `table.add_refresh_info
  {name? = "Refresh info", measure? = true, measure_name? = "Last refresh", hidden? = false}`
  (Model tab, Calculations, "Refresh info table") adds an ordinary import table: one row from a
  Power Query query, `#table(type table [#"Last refresh (UTC)" = datetime, #"Last refresh
  (local)" = datetime], {{DateTimeZone.RemoveZone(DateTimeZone.UtcNow()), DateTime.LocalNow()}})`,
  which Power BI Desktop, the service and `model.refresh` evaluate at each refresh, and a measure
  `MAX('Refresh info'[Last refresh (UTC)])` formatted `General Date` (each viewer's culture shows
  it its own way). It is the author's choice: nothing adds it by default. In the service
  `DateTime.LocalNow()` is the service's clock (UTC), so the measure reads the UTC column. The
  table and the measure are plain TMDL/TMSL and PBIX objects, with nothing specific to BIcraft;
  bind the measure or the columns like any field (`visual.add`, the field pickers). Refused when
  the table or the measure name is taken; one undo step.
- **What-if and field parameters.** Model tab, Parameters, "New parameter" (Numeric range,
  Fields), as Desktop's Modeling › New parameter. `parameter.what_if {name, min, max, increment,
  default?, data_type?, add_slicer?, page?}` adds the calculated table
  `GENERATESERIES(min, max, increment)`, the measure `[<name> Value]` (`SELECTEDVALUE` with the
  default) and a single-value slicer; `parameter.fields {name, fields: [field | {field, label?}],
  add_slicer?, page?}` adds the table of `("Label", NAMEOF('T'[c]), order)` tuples with its three
  columns and the `ParameterMetadata` and group-by properties, and a slicer of its labels. A
  visual bound to the field parameter shows the fields its slicer chooses (all of them when none
  is). One undo step each; the slicer goes on the active page unless `page` is given, and only
  when the session has a report. `parameter.fix_field {name, field, to?}` points the `NAMEOF`
  that names `field` (as written there, `'T'[c]` or `[Measure]`; it may be missing from the
  model) at `to`, keeping its label and order, or without `to` removes its row (refused for the
  only one); the new expression is validated, one undo step. Shapes: `docs/formats/parameters.md`.
- **Groups and bins.** The Data pane's column menu (New group), Column tools › Data groups, as
  Desktop's New group. `column.group {table, column, name?, kind?, groups?, other?, bin_size?,
  bin_count?, unit?, replace?}` adds a calculated column `<column> (groups)` (a `SWITCH(TRUE, …)`
  of the listed values, `(Blank)`, `Other`) or `<column> (bins)` (numbers by size or into a
  number of bins, dates by years, months, days, hours, minutes or seconds) with the
  `GroupingMetadata` extended property and the `GroupingDesignState` annotation Desktop reads;
  `replace` rewrites a group column (Edit groups), whose definition the snapshot gives as the
  column's `group`. One undo step. `visual.group_points {page, visual, points?, point?, name?,
  group?, other?}` (Group data, the menu of a data point) groups the category values of the points
  selected in the visual (Ctrl+click), else of the point right-clicked, through `column.group`.
  Shapes: `docs/formats/groups.md`.
- **Quick measures.** Home › Quick measure and the Data pane's New quick measure (on a table, or
  on a field, which starts as the base value), as Desktop's Quick measures.
  `measure.quick {kind, params, table?, name?}` writes an ordinary measure from BIcraft's own DAX
  template (aggregates per category, filtered values and their differences, sales from new
  customers, YTD (with a fiscal year end)/QTD/MTD, YoY/QoQ/MoM change, rolling average, running
  total, totals for a category, arithmetic, correlation coefficient, star rating, concatenated
  list), validated by the DAX binder. Time intelligence on a column with auto date/time walks its
  date table's every date (`T[C].[Date]`); on a column that is neither that nor a marked date
  table's, the result warns that only the dates the column holds are seen. One undo step. Kinds,
  parameters and templates: `docs/formats/quick-measures.md`.

Rules of the shape: every key is present; an unset property is `null` and an unset boolean is its
TMSL default (`is_available_in_mdx` and `is_nullable` true, the others false); multi-line TMSL
text is joined with `\n`; every object has `annotations` (`[{name, value}]`) and `extra`, the
properties BIcraft has no key for, by their TMSL name, so nothing the file holds is hidden.

### Translations

A model's cultures carry translated captions, descriptions and display folders of the model,
tables, columns, measures, hierarchies, levels, perspectives, roles and shared expressions
(TMSL `cultures[].translations`, TMDL `cultures/<name>.tmdl`, PBIX `ObjectTranslation` rows;
see [formats/tmdl.md](formats/tmdl.md) and [formats/metadata-sqlitedb.md](formats/metadata-sqlitedb.md)).
Linguistic metadata (the Q&A schema) is kept as it is.

- `culture.add {name}` adds an empty culture; `culture.remove {name}` removes it with its
  translations (and its linguistic schema; a note says so for the model's own culture).
- `translation.set {culture, object, property, value}`: `object` is `{}` (the model), `{table}`,
  `{table, column}`, `{table, measure}` (or `{measure}` alone), `{table, hierarchy}`,
  `{table, hierarchy, level}`, `{perspective}`, `{role}` or `{expression}`; names match without
  regard to case. `property` is `caption`, `description` or `display_folder` (columns, measures
  and hierarchies only). `value: null` (or `""`) removes the translation; entries left empty go
  too. The result says whether anything `changed`.
- `translation.list {culture?}`: `cultures` (`name`, `model_culture`, number of `translations`,
  `linguistic_metadata`) and `translations: [{culture, object, property, value}]`, with `object`
  in the shape `translation.set` takes.
- Renames (`table.rename`, `column.rename`, `measure.rename`, `measure.move`, `hierarchy.set`
  names and levels, `expression.rename`) keep translations attached (`metadata_updated`). An edit
  that removes an object drops its translations and lists them in `translations_dropped`. Every
  culture command is one undo step.
- Perspectives follow the same rules: renames keep their members attached (`metadata_updated`),
  and any edit that removes a table, column, measure, hierarchy or calculation group (directly,
  in a `batch` or through `report.apply`) drops its perspective entries (a removed table's whole
  perspective table) and lists them in `perspective_entries_dropped: [{perspective, objects}]`.
  Undo brings them back.
- Reads in a culture: `model.get`, `table.get`, `column.get` and `measure.get` take `culture`.
  Each object then gains `caption` (its translated caption, else its name) and `translation:
  {caption, description, display_folder}` (`null` where there is none), and shows the translated
  `description` and `display_folder` where there is one; `name` stays the DAX name. The model's
  own culture works without a `cultures` entry (names only); any other unknown culture is an
  error.
- Viewer and render: `view.set_culture {culture}` (or `report.render {culture}` for one call;
  there a culture without translations renders the stored names with a warning instead of
  failing) adds `display_culture` to the snapshot and a `caption` on translated tables, columns,
  measures and visual `fields`; renderers label a field with its `caption` when it has one.
  The UI phrases around them ("by", "Total") follow `view.set_language`.

## Storage statistics and VPAX

`model.storage_stats {table?, columns? (true), segments? (false), row_number? (false)}` reads the
VertiPaq storage of the open file's stored data (a PBIX `DataModel` or a PBIP `.pbi/cache.abf`;
a PBIT, a `model.bim` or a new model has none and is refused). Only `metadata.sqlitedb` and the
`.idfmeta` segment maps are read, never the column data; a backup too large to decompress in
memory is streamed (`streamed: true`). Per table: `rows`, `total_size` and `percent_of_model`,
`dictionary_size`, `data_size`, `hierarchy_size` (attribute hierarchies, `H$`),
`user_hierarchies_size` (`U$`), `relationships_size` (`R$` kept with the table), partitions
(rows, segments, and the refresh state: `state`, `refreshed_time`, ... as under *Model reads*)
and columns (`data_type`, `encoding`: `hash` / `value`, `cardinality`,
`selectivity`, sizes, segment count; with `segments: true`, each segment's rows, used size,
compression and bits). `relationships` lists each relationship's index size. Sizes are bytes, as
the engine's `DISCOVER_STORAGE_TABLE_COLUMN_SEGMENTS` / `DISCOVER_STORAGE_TABLE_COLUMNS` report
them; how each figure is derived and how it compares with the engine is in
`docs/formats/vertipaq-stats.md`.

`model.export_vpax {path, overwrite?, model_bim? (true)}` (CLI `bicraft-cli vpax PATH OUT.vpax
[--overwrite] [--no-model-bim]`) writes a VPAX file, the open format of VertiPaq Analyzer read by
DAX Studio, Tabular Editor and Bravo: `DaxModel.json` (the model and these statistics) and
`Model.bim` (the model as TMSL).

## Best-practice analysis

`model.analyze {rules?, default_rules? (true), rule_ids?, severity_min?, max_findings? (10000)}`
(CLI `bicraft-cli analyze PATH [--rules FILE]... [--no-default-rules] [--rule-id ID]...
[--severity-min 1-5|info|warning|error] [--max-findings N]`; over MCP through `run_command`)
checks the open model against best-practice rules in the BPA rule-file format: a JSON array of
`{ID, Name, Category, Description, Severity, Scope, Expression, FixExpression?,
CompatibilityLevel?}` whose `Expression` is a Dynamic LINQ predicate over the model's objects.

- **Default rules**: Microsoft's 71 best-practice rules (`BestPracticeRules/BPARules.json` of
  microsoft/Analysis-Services, MIT), embedded unmodified. `default_rules: false` leaves them out.
- **Your rules**: `rules` is a rule-file path, a list of paths, or the rules inline (an array, or
  `{"rules": [...]}`). Agent frameworks pass their own rule files this way.
- **Result**: `summary` (rule counts by status, findings by severity), `findings` (`rule_id`,
  `rule_name`, `category`, `severity` `info`/`warning`/`error` and `severity_level`, `object_type`,
  `object` in DAX syntax, `table`), `findings_omitted` beyond `max_findings`, `rules` (one entry
  per rule run: `source`, `status`, `objects`, `findings`, `reason`, `fix_expression`) and
  `invalid_rules` (file entries that are not rules).
- **Status per rule**: `evaluated`; `partial` (some objects could not be evaluated, so findings on
  them may be missing); `not_evaluated` (the expression or every scope is unsupported, with the
  reason); `skipped` (the rule's `CompatibilityLevel` is above the model's). An unsupported
  property, method or regular expression never counts as "no violation".
- `FixExpression` is reported, never applied: make the fix with the editing commands.
- Rules reading VertiPaq statistics through annotations (`Vertipaq_Cardinality`,
  `Vertipaq_RowCount`, ...) only fire when the model carries those annotations.

The CLI exits with 1 when a rule of severity 3 or more fires. The supported expression subset is
described in `docs/formats/bpa.md`.

**Report rules** (M11.39). With a report open (a `.pbip`, a `.pbix`, or a report added to the
session), `model.analyze` also runs bicraft's own report rules, category `Report references`,
source `bicraft report rules` (`report_rules: false`, CLI `--no-report-rules`, leaves them out;
`rule_ids` and `severity_min` select them like the others). They find what a rename or a removal
in the model leaves behind, through the same walk of the report as `model.usage`:

| Rule | Severity | Finds |
|---|---|---|
| `REPORT_BROKEN_VISUAL_FIELD` | error | a visual's field (any role) or sort on a table, column, measure or hierarchy level the model lacks |
| `REPORT_BROKEN_FILTER` | error | a visual, page or report filter on such a field (its field or its condition) |
| `REPORT_BROKEN_DRILLTHROUGH` | error | a drillthrough or tooltip page bound to such a field |
| `REPORT_BROKEN_FORMATTING` | error | conditional formatting (or a title from a field) using such a field |
| `REPORT_BROKEN_FIELD_PARAMETER` | error | a field parameter whose `NAMEOF` names an object the model lacks |
| `REPORT_BROKEN_BOOKMARK` | warning | a bookmark's state (filters, selection) naming such a field |
| `REPORT_BROKEN_PROPERTY` | warning | any other report, page or visual property naming such a field |
| `REPORT_STALE_QUERYREF` | warning | a projection whose `queryRef` names another field than the one it projects (case, an aggregation switch and a numeric duplicate suffix are not flagged) |
| `REPORT_STALE_SELECTOR` | warning | formatting keyed (`selector.metadata`) to a queryRef no projection of the visual has: Desktop ignores it |
| `REPORT_DANGLING_INTERACTION` | warning | a page's `visualInteractions` naming a visual that is not on the page |

Each finding adds `location` (`kind` as in `model.usage`, `page` and `page_name`, `visual`,
`visual_type`, `role`, `bookmark`, `query_ref`, `object`, `field_parameter`, whichever apply),
`message` (where, what is missing, and "did you mean" when the model has a near name),
`suggestion` (that name: a close name on the same table, the same name on another table, or a
name it starts or ends with) and, when one command repairs it without touching anything else,
`fix: {command, params}` (one undo step each):

- a missing field with a suggestion of the same kind (a column for a column, a measure for a
  measure): `report.rebind {from, to}`, which re-points every reference the report makes to it
  (so one fix clears its other findings too);
- else the reference is removed: `visual.unbind` (the field by its position in its role),
  `visual.clear_filters`, `page.clear_filters` or `report.clear_filters` (the card on the
  field), `report.remove_reference` with `place` `drillthrough` (a drillthrough or tooltip
  page's field), `formatting` (conditional formatting of a visual, a page or the report) or
  `bookmark` (the bookmark's entries naming it), offered only when it removes every reference
  there (tried on a copy);
- `REPORT_STALE_QUERYREF`: `visual.fix_query_refs {page, visual}`, unless the field's own
  queryRef is already used in the visual;
- `REPORT_BROKEN_FIELD_PARAMETER`: `parameter.fix_field {name, field, to}` to the suggestion, or
  without `to` (the row goes) when the parameter lists other fields.

Other properties (`REPORT_BROKEN_PROPERTY` without a suggestion), hierarchy levels, selectors and
interactions are reported only: rename back (`column.rename` / `measure.rename` with
`update_report`) or edit them. Each fix leaves a report `report.validate` accepts (checked in
`crates/engine/tests/report/report_lint_tests.rs`). The rule list was compared with DashCraft's lint rules for ideas
(`crates/pbi-core/src/lint/RULES.md` at commit 0958fd82, MIT OR Apache-2.0); no text or code was
taken. In the app, Analyze › Best practices lists the report findings first, each with Go to
visual (or page), which shows the page and selects the visual, and Fix, which runs the fix and
the analysis again.

## Where objects are used, and what is unused

`model.usage`, `model.unused` and `model.remove_unused` (MCP `model_usage`, `model_unused`,
`remove_unused`; CLI `bicraft-cli usage` and `bicraft-cli unused`) answer "where is this field used?" and
"what can go?" from the files alone, on any platform. They reuse the dependency logic of the
rename and remove commands (every DAX expression resolved by the same binder) and add everything
else that uses a model object.

**What counts as a usage.** Each usage is `{kind, ...}`:

| Source | Kinds |
|---|---|
| DAX in the model (an edge of the dependency graph) | the expression's kind: `measure`, `calculated column`, `calculated table` (field parameters through `NAMEOF`), `kpi target`/`status`/`trend`, `format string expression` (dynamic format strings), `detail rows expression`, `calculation item`, `calculation item format string`, selection expressions; `calculation group (measure named in a string)` for `SELECTEDMEASURENAME() = "…"` |
| Model structure | `relationship` (`active` says whether it is), `row filter` (RLS), `object-level security`, `hierarchy` (level), `sort by`, `variation` and `auto date/time table`, `key column`, `default label`, `aggregation` (`alternateOf`), `group by column`, `calculation group`, `user-defined function`, `M query` / `M expression` (queries naming other queries) |
| Metadata | `perspective`, `translation`, `linguistic schema` (Q&A) |
| Report (`report` is `open` or the extra report's file name) | `visual` (page, visual, visual type, role), `visual filter`, `page filter`, `report filter`, `drillthrough`, `tooltip page`, `bookmark`, `formatting` (conditional formatting and other field-driven properties), `sort`, `report measure` (report-level measures), `visual property` / `page property` / `report property` (anything else kept verbatim) |
| You | `allow-list` (`allow` patterns) |

A column or measure also needs its table, and a calculated table that names a whole table needs
all of its columns. An expression that does not parse is matched by name (`[Name]`, `'Table'`):
it can only keep more objects, never fewer (a warning says how many).

**`model.usage {object?, kind?, reports?, graph?}`.** With `object` (`Table[Column]`, `[Measure]`,
`'Table'`, an M expression name; `kind` disambiguates): `used`, `used_by_report`, `direct` (its own
usages and the expressions that reference it), `indirect` (the usages of everything that depends on
it, each with `through`, the chain of objects), `dependants` (transitive, with `depth`),
`depends_on`, `depends_on_all` and `graph {nodes, edges: [[from, to, kind]]}` (`from` needs `to`).
Without `object`: every object with `used`, `used_by_report` and its direct usages, plus the whole
graph (`graph: false` leaves it out; `kind` filters).

**`model.unused {reports?, allow?, kept?, keep_metadata?}`.** An object is used when it has a usage or a used
object needs it, transitively: a measure only an unused measure references is unused, and `why`
says so (`used only by unused objects: measure [Old Margin]`, `referenced nowhere`). Lists
`unused.measures`, `unused.columns`, `unused.tables` (calculated tables only, with what they
contain; their members are not listed again) and `unused.expressions` (shared M expressions no
table's query names, directly or through used expressions; M parameters, `IsParameterQuery`,
are kept since data sources, dynamic M parameters and refresh policies read them), `counts`, and `kept`: the objects no report uses
that a model rule keeps, with the `rules` and the first usage (`detail`) or the kept object that
needs them (`needed_by`). Conservative by design: relationship keys (inactive relationships too),
sort-by targets, RLS and OLS, hierarchy levels, key and default-label columns, aggregation
columns, calculation groups (and the measures they name), auto date/time tables, user-defined
functions, perspectives, translations, the Q&A linguistic schema and report-level measures keep
what they use. `allow` takes names or `*`/`?` patterns (`[KPI*]`, `Sales[Key]`, `'Scratch'`,
case-insensitive); patterns matching nothing are reported.

`keep_metadata: false` (CLI `--ignore-metadata`) stops perspectives, translations and the Q&A
linguistic schema from keeping objects: they name objects without using them, and a fully
translated model would otherwise list nothing. The default stays conservative. With it,
`model.remove_unused` drops the perspective and translation entries of what it removes.

**Several reports.** A semantic model is often shared by thin reports. The open file's report
is read, and `reports: [paths]` adds others (`.pbix`/`.pbit`, `.pbip`, a project folder or a
`<Name>.Report` folder; in server mode the paths follow the workspace rules); usages are the
union. The first warning always says how many reports were read and that reports not given may
still use what is listed. Without any report, report usage is unknown and the warning says so.

**`model.remove_unused {objects?, reports?, allow?, keep_metadata?, dry_run?, force?}`.** Removes everything
`model.unused` lists, or the `objects` named (each must still be unused under the same reports
and allow-list, else nothing is removed). It runs the mutations of `table.remove`,
`column.remove` and `measure.remove` as one model edit: one `edit.undo` step, all or nothing,
perspective and translation entries dropped with the objects, and `broken` lists any DAX that no
longer validates. Removing a subset that leaves an unused object behind which uses a removed one
is refused (it would break) unless `force`; so is removing an expression another expression left
behind names, and removing expressions from a PBIX with data (as `expression.remove`). `dry_run` returns the same result and leaves the
session untouched.

```text
$ bicraft-cli unused Sales.pbip
{"unused": {"measures": [{"kind": "measure", "table": "Sales", "name": "Old", "why": "used only by unused objects: measure [Older]"}, ...],
 "kept": [{"kind": "column", "table": "Product", "name": "SortOrder", "rules": ["sort by"], "detail": {...}}, ...],
 "warnings": ["report usage covers 1 report(s); a semantic model can serve other reports ..."]}
```

Performance: one pass parses each DAX expression once; on a generated model with 5,000 measures
and 200 visuals `model.unused` takes about 47 ms (release build, median of 5; `model.usage` of one object 33 ms, of every object 59 ms, a `remove_unused` dry run of 3,408 objects 156 ms).

## Importing objects from another file

`objects.import` (MCP `import_objects`, CLI `bicraft-cli import SOURCE TARGET`) copies objects from
another file into the open one. The source can be any file `file.open` reads (PBIX, PBIT, PBIP,
model.bim); it is opened read-only in a private session and never saved. The whole import is one
command, so one `edit.undo` step, and nothing changes when it is refused.

**What can be selected.** Each parameter takes a list of names or `*` / `?` patterns
(case-insensitive); a selector that matches nothing is an error.

| Parameter | Objects |
|---|---|
| `measures` | `Name`, `[Name]`, `Table[Name]`, `Sales[*]` |
| `tables` | Tables with their columns, measures, hierarchies, partitions (M or DAX), calculated tables |
| `columns` | Calculated columns `Table[Column]`, added to the target's table of that name |
| `calculation_groups` | Calculation group tables |
| `functions` | User-defined DAX functions |
| `expressions` | Shared M expressions and parameters (not into a PBIX with data, as `expression.add`) |
| `roles` | Security roles (permissions on tables the target lacks are dropped with a warning) |
| `pages` | Report pages with their visuals (name or display name) |
| `visuals` | Single visuals, `"Page/visual"` or `{page, visual}`, added to `target_page`; a group comes with its members and a member with its groups |
| `bookmarks` | Bookmarks (name or display name) |
| `theme` | `true`: the source's custom theme (installed like `theme.set`) |

`relationships` (default `true`) adds the source's relationships that touch an imported table
when both columns exist in the result and the target has no relationship between the same
columns. A relationship that would make a second active path between two tables is imported
inactive, with a warning; one whose name is taken gets a new GUID.

**Dependencies.** With `include_dependencies` (default `true`) the import also takes what the
selection uses and the target lacks, by name: measures a measure references, the tables of the
columns it uses (a data column only comes with its table), calculated columns, user-defined
functions it calls, the shared M expressions and M tables a query references, the local date
tables of auto date/time columns, and the measures and columns the fields of imported visuals,
filters and bookmarks use. `dependencies` lists each one with `required_by` and `from: "source"`
(imported) or `from: "target"` (the target's object of that name is used). The closure stops
on cycles. Measures and columns of an imported table come with it.

**Conflicts.** An object conflicts when the target has one of the same name (tables and shared
expressions share one name space; measure names are unique in the model and differ from the
columns of their table; pages and bookmarks by display name; the theme when the target has a
custom theme). `on_conflict`:

- `fail` (default): nothing is imported; `conflicts` lists every clash.
- `skip`: the target keeps its object; imported objects that use the name use the target's.
- `replace`: the target's object is removed and the source's takes its place (a table keeps its
  position; target relationships whose columns are gone are removed, with a warning).
- `rename`: the source object is renamed `Name (imported)` (`rename_suffix`, then `… 2`, `… 3`)
  in the private copy with the rename commands, so every DAX expression and visual binding of
  the imported set follows the new name, as with `table.rename` / `measure.rename`.

Dependencies follow the same rule only when they are imported; one the target already has is
not a conflict. `into_table` puts the imported measures into that target table; `Table[Measure]`
references follow. Page, visual and bookmark ids that the target already uses get new ids, and
bookmarks follow their pages. Lineage tags the target already has are regenerated
(`lineage_tags_regenerated`). Registered images the imported pages and visuals show are copied;
one whose name the target uses for another file gets a new name (`logo_2.png`).

**Checks.** `broken` lists the DAX expressions that would not validate in the result (imported
or the target's own, as `measure.remove` reports them) and `broken_fields` the field references
of imported pages, visuals and bookmarks that do not resolve in the target model (a report-level
measure must exist in the target report). Either refuses the import unless `allow_broken`, which
imports anyway and turns them into warnings. `dry_run: true` returns the same plan with
`would_apply` and `errors` and changes nothing.

Result: `{objects: [{kind, name, table?, action: added|replaced|renamed|skipped, reason:
selected|dependency|with its table, required_by?, source_name?, into_table?}], dependencies,
relationships, conflicts, lineage_tags_regenerated, broken, broken_fields, warnings, applied,
would_apply?, errors?}`.

Not copied: the source's perspectives and translations, custom visual packages, report-level
measures, and M references to a table renamed by `rename` (M is not rewritten, as with
`table.rename`).

**Applying a template report.** Import its pages and its theme into the report to restyle:

```text
$ bicraft-cli import Template.pbip Sales.pbip --page "*" --theme --on-conflict rename
added     page Overview
added     measure Sales[Margin] (used by page `Overview`)
added     theme Corporate
saved Sales.pbip
```

**CLI.** `bicraft-cli import SOURCE TARGET` opens TARGET, runs the import and saves TARGET with
`file.save` (a PBIX keeps `<name>.bak` unless `--no-backup`); `--dry-run` writes nothing. Flags:
`--measure`, `--table`, `--column`, `--calculation-group`, `--function`, `--expression`,
`--role`, `--page`, `--visual PAGE/VISUAL`, `--bookmark` (each repeatable), `--target-page`,
`--theme`, `--on-conflict`, `--rename-suffix`, `--into-table`, `--no-dependencies`,
`--no-relationships`, `--allow-broken`, `--json` (the full result instead of one line per
object). Exit 0 on success, 1 when the import is refused (or a dry run would be), 2 for usage
errors and unreadable files.

In server mode `source` is resolved under the workspace root like every other path.

## Editing rules

- **Names** are matched case-insensitively, like DAX. Table names are unique in the model, column
  names in their table, and measure names in the whole model; a measure cannot share its name with a
  column of its table.
- **DAX is validated before an edit is applied**: measures, calculated columns, calculated tables
  and row filters are bound against the model *as it will be after the edit*. Invalid DAX rejects
  the edit and the error lists each diagnostic (`line:column: message [code]`). Pass
  `"force": true` to apply it anyway; the diagnostics then come back in `warnings`.
- **Renames rewrite DAX, like Power BI Desktop.** `table.rename` and the renames of `column.set` /
  `column.rename` and `measure.set` / `measure.rename` rewrite every DAX expression that references
  the object: measures, calculated columns, calculated tables (field parameters' `NAMEOF` too), row
  filters, KPI target/status/trend expressions, dynamic format strings, detail rows expressions,
  calculation items and selection expressions, the bodies of user-defined functions (`functions`,
  whose parameters shadow tables like variables), and report-level measures (whose entity follows
  a table rename). `measure.move` rewrites the `Table` of `Table[Measure]` references the same way. Only real references change, resolved like the validator does (row contexts,
  variables, columns of the same name in other tables); strings, comments, whitespace and the
  casing of everything else keep their bytes. A renamed table keeps the quoting of each reference
  (`'Sales'[x]` → `'Fact'[x]`, `Sales[x]` → `Fact[x]`) and is quoted where it must be (spaces,
  non-ASCII, a reserved word such as `Date` or `Item`, or a variable of that name in scope).
  `dax_rewritten` lists the rewritten expressions `{kind, table, name, measure?, references}`;
  `dax_not_rewritten` those that mention the old name but do not parse (left as they are).
  `"update_dax": false` opts out. M (Power Query) expressions are never rewritten: Desktop does not
  rewrite them either. Visual calculations name the visual's fields by `nativeQueryRef`, which a
  rename keeps, so they need no rewrite.
- **Renames follow name-keyed metadata.** Whatever `update_dax` says, renames and `measure.move`
  also update perspectives (table, column and measure members), culture translations, linguistic
  metadata bindings (`ConceptualEntity` / `ConceptualProperty`), calculated table columns'
  `sourceColumn` (`Sales[Amount]` for a column of `= Sales`), column properties that name a table
  or column (`variations`, `alternateOf`, grouping columns) and calendars; `metadata_updated`
  lists them (`{kind, name | culture | table, …}`). A moved measure's perspective and translation
  entries move to the new table's entry.
- **Removals are refused while DAX uses the object.** `table.remove`, `column.remove` and
  `measure.remove` fail with the list of expressions that reference it (for a table: its columns and
  measures too; the table's own expressions go with it), including report-level measures. With
  `"force": true` the object is removed, the expressions are left as they are, and the result lists
  them in `used_by`.
- **`broken`**: renames, removals and moves return the expressions that validated before the edit
  and no longer do, with their diagnostics. Fix them with `set_measure` / `column.set`.
- **`*.set` tools** change only the properties they are given. A string property set to `null` or
  `""` is cleared.
- **Unknown parameters are errors**, so a misspelt property is reported instead of ignored.
- Every result has `warnings` (strings) and `diagnostics` (DAX diagnostics per validated
  expression).
- Edits only change the in-memory model. `inspect_file` reports `dirty: true` until a save.
- New tables, columns and measures get a `lineageTag`; relationships get a GUID name. New
  tables, columns and shared expressions also get the annotations and default format strings
  Power BI Desktop gives the objects it creates (`SummarizationSetBy`, `PBI_FormatHint`,
  `UnderlyingDateTimeDataType`, `PBI_ResultType`, …; list in
  [formats/tmdl.md](formats/tmdl.md)). A `dateTime` column's defaults (`Long Date` +
  `UnderlyingDateTimeDataType = Date`, `Long Time` + `Time`, `General Date`) follow the type
  the table's M query declares for its source column, read from the query text (typed
  `#table`, `Table.TransformColumnTypes`, typed `Table.AddColumn`). `*.set` commands never add
  them, except that `column.set` changing a data column's `data_type` swaps a default format
  string for the new type's and moves `UnderlyingDateTimeDataType` along, as Desktop does.

### Model properties

Properties Power BI Desktop keeps on measures, columns and the model (M9.1). They are written to
TMSL (`model.bim`, PBIT), TMDL and the PBIX metadata; each command is one undo step and
all-or-nothing (a `measure.add` whose format string is refused adds no measure).

- **Dynamic format strings.** `format_string_definition` on `measure.add` / `measure.set` is a DAX
  expression returning the format string (`IF([Total] > 1000, "#,0,K", "#,0")`), validated like a
  measure of the same table (`force` applies invalid DAX). It is stored as TMSL
  `formatStringDefinition.expression`. The engine refuses a measure with both a static and a
  dynamic format string, so, as TOM, Tabular Editor and Desktop do, setting the dynamic one
  clears the static `format_string` (a warning names it) and keeps no copy. `null` removes the
  dynamic one and leaves the measure with no format string, unless `format_string` is passed in
  the same call (`{"format_string_definition": null, "format_string": "0.0"}`). `format_string`
  alone on a measure that keeps its dynamic format string is refused. Existing annotations of the
  measure are left as they are.
- **Sort by column.** `sort_by_column` names a column of the same table (`null` clears it). Refused
  when the sort column has more than one value for one value of the column (texts compare
  case-insensitively, like the engine), checked on the stored data (PBIX `DataModel`, PBIP
  `cache.abf`) or the table's literal partitions; without data (calculated columns, connectors)
  it is a warning. Also refused: sorting a column by itself, a sort cycle, a sort column calculated
  from the column, and a sort column without attribute hierarchy. `force` applies the data
  refusal anyway.
- **`is_available_in_mdx: false`** (no attribute hierarchy) is refused, as the engine refuses it,
  while the column is a sort-by column or a hierarchy level; a sorted column, a relationship key
  or a variation's default column only warn (the engine accepts them). On a PBIX it is written
  for new columns; changing it on a column already stored in the `DataModel` is refused at save
  (the attribute hierarchy is part of the VertiPaq data), so save such an edit as a PBIP or PBIT.
  **`is_key: true`**: one key column per table, with unique non-blank values when there is data;
  on a PBIX the row-number column stops being the key (the engine refuses two).
- **`data_category`**: `Address`, `Place`, `City`, `County`, `StateOrProvince`, `PostalCode`,
  `Country`, `Continent`, `Latitude`, `Longitude`, `WebUrl`, `ImageUrl`, `Barcode` (case and spaces
  ignored, `Uncategorized` or `null` clears); other values are kept with a warning.
- **Annotations** (`*.set_annotation`): free-form name/value pairs. Names starting with `PBI_` or
  `__PBI_` are Desktop's own and come with a warning.
- **`model.set`**: `culture` (e.g. `fr-FR`), `discourage_implicit_measures`,
  `source_query_culture` (`null` clears). A PBIX with data refuses a culture change on save (its
  stored text is collated for the old culture).
- **Date variations.** Desktop's auto date/time gives each date column a variation that points
  through a relationship to a hidden `LocalDateTable_<guid>`. `column.remove_variation` removes a
  column's variations, their relationships and the auto date tables no other variation uses,
  like turning auto date/time off for that column; refused while DAX uses such a table unless
  `force`. `table.remove` of a table that variations point to removes those variations first
  (`removed_variations`): a variation without its relationship does not load. `column.remove` of
  a date column also removes its auto date table (`removed_tables`) unless DAX uses it. Visuals
  bound to a removed date hierarchy show up in `report.validate`.

### Power Query sources

BIcraft runs a table's M query itself (no Desktop, no gateway) when it needs the table's rows:
`table.import {m}`, a PBIX save of a new or replaced import table, and the viewer's preview of a
table that has no stored data. The evaluator and what it supports are described in
[formats/data-sources.md](formats/data-sources.md); connectors (`Sql.Database`, `Web.Contents`, …)
are refused with the reason, except the Databricks and SQL Server ones where a command takes
`allow_connectors`.

- **Culture and shared queries.** The query runs with the model's `sourceQueryCulture` (else
  `culture`), and can name the model's other queries and parameters (`expressions`) and other
  tables' queries.
- **Files: `data_root`, opt-in.** `File.Contents` (for `Csv.Document`, `Json.Document`,
  `Excel.Workbook`) reads only under the session's data root, and there is none by default. Pass
  `data_root: "<folder>"` to `file.open`, `file.save`, `file.save_as`, `table.import` or
  `table.set_data` (or `bicraft-cli view PATH --data-root DIR`). It stays set for the session;
  `file.open` without it resets it, and `null` clears it. A query that needs files without a root
  fails with an error that says so. `inspect_file` shows the current root.
- **Large CSV files.** `Csv.Document(File.Contents(…))` followed by `Table.PromoteHeaders` and
  `Table.TransformColumnTypes` (the query Desktop writes for a CSV file) is streamed and parsed
  on all cores, with the evaluator's results: no 256 MiB file cap, only a memory budget that
  refuses a load cleanly ([formats/data-sources.md](formats/data-sources.md), *Fast CSV
  loading*). `table.import {csv}` keeps its 256 MiB cap, because its data is written into the
  model as an M `#table` literal; load a larger file with `m`.
- **Text as Analysis Services stores it.** Imported text loses the control characters XML does
  not allow and its trailing spaces, tabs and line breaks; values equal for the engine are stored
  once, spelled as the first one (en-US: ASCII case variants; other cultures: case variants per
  the collation). The structural save and the viewer's DAX results follow these rules, so
  `DISTINCTCOUNT` over `a`, `A ` is 1, as in Desktop.

### Get data

What Home › Get data and Home › Enter data do in the app, as commands (`run_command`):

- **`source.navigate {path, kind?, delimiter?, data_root?, detect_types?}`** (read): lists the
  items of a local Excel workbook, text/CSV file, JSON file or folder (`kind`: `excel`, `csv`,
  `json`, `folder`, else from the extension) with the M query of each:
  `{kind, path, data_root, items: [{name, kind, m, hidden, error?}], truncated}`. The query
  reads files only under `data_root`: the one given, else the session's root when the path is
  under it, else the file's folder (the folder itself for `folder`). In server mode the paths
  resolve under the workspace, as `file.open`'s do. Preview an item with
  `query.preview {expression: m, data_root}`.
- **`source.navigate {kind: "sqlserver", server, database, query?}`** (read without `server`
  only, so refused to readers): the user tables and views of a SQL Server database
  (`name` `schema.item`, `kind` `Table` or `View`), or one item `Query1` for a SQL statement,
  each with its `Sql.Database` query, and `allow_connectors: ["sqlserver"]` to pass to
  `query.preview` and `source.load`. Needs network access (refused in server mode without it).
  [formats/tds.md](formats/tds.md) describes the connector.
- **`source.navigate {kind: "web", url, delimiter?, detect_types?}`** and **`{kind: "odata",
  url}`** (refused to readers): a JSON document or delimited text at an http(s) URL, read once
  through `Web.Contents` (an HTML page is refused), or the entity sets of an OData feed, each with
  its `OData.Feed(url, null, [Implementation="2.0"]){[Name, Signature="table"]}[Data]` query;
  the answer's `allow_connectors` (`["web"]` or `["odata"]`) goes to `query.preview`,
  `source.load` and `model.refresh`. Uses the credential stored for the URL, if any; needs network
  access. [formats/odata.md](formats/odata.md) describes both connectors.
- **`source.navigate {kind: "sharepoint_folder", url}`** and **`{kind: "sharepoint_list", url}`**
  (refused to readers): the files of a SharePoint site as a folder (`SharePoint.Files(url,
  [ApiVersion = 15])`, `Combine` and `Load` items), or its lists that are not hidden, each with
  its `SharePoint.Tables(url, [Implementation = "2.0", ViewMode = "All"])` query; the answer's
  `allow_connectors` is `["sharepoint"]`. Needs a bearer token for the site
  (`source.credentials`) and network access. [formats/sharepoint.md](formats/sharepoint.md).
- **`source.credentials {server, user, password?, authentication?, clear?}`**: a SQL Server
  login for the session, kept in memory only (never in a file, the recovery journal, an answer
  or a log), else `BICRAFT_SQLSERVER_USER` / `BICRAFT_SQLSERVER_PASSWORD`. `clear: true` forgets
  it; `authentication: "windows"` is refused. Denied in server mode.
- **`source.credentials {kind?: "web", url, authentication, user?, password?, key?, header?,
  token?, clear?}`**: a credential for the http(s) sources under `url` (Web, OData feed,
  SharePoint; the longest stored prefix wins; `kind` defaults to `web` when `url`, `key`,
  `header` or `token` is given), in session memory only. `authentication`: `anonymous`,
  `basic` (`user`, `password`), `key` (Web API: the query parameter named by `ApiKeyName`, or
  the header `header`), `bearer` or `organizational` (`token`, an access token obtained
  elsewhere); `windows` is refused. Never followed to another origin on a redirect.
- **`source.privacy {kind, source, level?, clear?}`**: records a data source's privacy level
  (`none`, `public`, `organizational`, `private`) for the session; `clear: true` forgets it. The
  levels are shown, not enforced (no privacy firewall).
- **`source.settings {}`** (read): the model's data sources (literal URL, server and path
  arguments of its connector calls) and the stored ones:
  `{sources: [{kind, source, key, functions, credential, privacy}], firewall: false}`;
  `credential` is the kind stored (`basic`, `key`, `bearer`, …), never the secret.
  [formats/sharepoint.md](formats/sharepoint.md) *Privacy levels*.
- **`source.load {tables: [{name, m}], data_root?, allow_connectors?}`**: imports 1 to 100 items
  as import tables in one undo step; if one fails, none is kept and the error names it. With
  `allow_connectors`, a connector's table gets its columns from the source (no rows: run
  `model.refresh` with the same list for them).
- **`table.enter_data {name, columns, rows, culture?, replace?, hidden?, description?}`**: a
  typed-in table, as Desktop's Enter data writes it ([formats/data-sources.md](formats/data-sources.md),
  *Desktop's "Enter data"*). `columns` are names or `{name, type}` (`text`, `whole`, `number`,
  `fixed`, `date`, `datetime`, `logical`; no type: detected); `rows` are lists of texts, numbers,
  booleans or `null`, texts read in `culture` (default: the model's query culture).
  `replace: true` rewrites an existing table (Edit data). The answer gives the types and culture
  used.
- **`table.entered_data {name}`** (read): reads a typed-in table back as
  `{name, columns: [{name, type}], rows, culture, shape}`, `shape` being `enter_data` or
  `table_literal`; a table loaded from another query is refused.

### Power Query editor

What Home › Transform data does in the app (ROADMAP M11 lot 12), as commands. The step
commands take the query's M (`expression`, the editor's draft) or a query of the model
(`name`, a shared query or a table's M partition) and return the new M: they change nothing in
the model, as the editor's steps stay in the editor until Close & Apply.

- **`query.list {}`** (read): the model's queries, `{queries: [{name, source: table|query,
  expression, partition, partitions, hidden, group, parameter}], data_root}`.
- **`query.steps {name? | expression?, partition?}`** (read): the applied steps, the variables
  of the query's top-level `let` with their M as written (`steps: [{name, expression,
  function}]`), the `in` expression (`result`) and the step it names (`result_step`). A query
  that is not a `let` is one step `Source`; the first step added wraps it as
  `let Source = … in Source`.
- **`query.preview {…, step}`**: the query as it is at that step (the steps up to it, then
  `in <step>`). Nested values (a merge's table column) show as `[Table]`, `[Record]`, `[List]`.
  `queries: [{name, expression}]` passes the editor's drafts of the other queries, read in place
  of the model's (a merge or a reference sees them as the editor has them; `query.profile` takes
  them too). A date/time column says its M type (`date_kind`: `date`, `datetime`…), which the
  filter menu's value list writes as `#date` or `#datetime` literals.
- **`query.apply_step {name? | expression?, after?, step: {kind, …}, step_name?}`**: one step
  after `after` (default the last step), written as Power Query writes it, named as the editor
  names it (`Filtered Rows`, a second `Changed Type` being `Changed Type1`). Kinds:
  `filter_rows`, `remove_columns`, `keep_columns`, `rename_columns`, `change_type`,
  `promote_headers`, `sort`, `remove_duplicates`, `replace_values`, `group_by`, `merge`,
  `expand`, `append`, `add_column`, `split_column`, `unpivot`, `pivot`, `custom` (Insert step
  after), and `filter_values {column, values, exclude?}` (a column filter menu's value list:
  `each ([C] = a or [C] = b)`, or `each ([C] <> a and …)` with `exclude`). Rows:
  `remove_top_rows`, `remove_bottom_rows`, `keep_top_rows`, `keep_bottom_rows` `{count}`,
  `keep_range {first?, count}`, `remove_alternate_rows {first?, remove?, keep?}`,
  `remove_blank_rows`, `remove_errors` / `keep_errors {columns?}`, `replace_errors {columns,
  value}`, `fill_down` / `fill_up {columns}`, `reverse_rows`, `transpose`, `demote_headers`.
  Columns: `reorder_columns {columns}`, `duplicate_column {column, name?}`, `add_index {name?,
  start?, step?}`, `merge_columns {columns, delimiter?, name?, add?}`, `conditional_column
  {name?, rules: [{column, op, value, output}], otherwise?}` (`each if … then … else if … else
  …`), and `column_op {op, columns, add?, name?, text?, end?, count?, start?, number?}`: the
  Transform and Add column menus' Format (`lower`, `upper`, `proper`, `trim`, `clean`,
  `prefix`, `suffix`), Extract (`length`, `first_chars`, `last_chars`, `text_range`,
  `before_delimiter`, `after_delimiter`, `between_delimiters {text, end}`), Date (`age`,
  `year`, `month`, `day`, `quarter`, `month_name`, `day_name`, `day_of_week`, `week_of_year`,
  `week_of_month`, `day_of_year`, `days_in_month`, `start_of_*` / `end_of_*` of `month`,
  `year`, `quarter`, `week`, `day`, `date_only`), Time (`time_only`, `hour`, `minute`,
  `second`, `start_of_hour`, `end_of_hour`), Duration (`duration_days`, `duration_hours`,
  `duration_minutes`, `duration_seconds`, `total_years` (total days / 365), `total_days`,
  `total_hours`, `total_minutes`, `total_seconds`), Standard (`add`, `subtract`, `multiply`,
  `divide`, `integer_divide`, `modulo`, `percentage`, `percent_of`, each with `number`),
  Scientific (`abs`, `power {number}`, `square_root`, `exponent`, `log10`, `ln`, `factorial`),
  Trigonometry (`sin`, `cos`, `tan`, `asin`, `acos`, `atan`), Rounding (`round`, `round_up`,
  `round_down`) and Information (`is_even`, `is_odd`, `sign`) items, in place
  (`Table.TransformColumns`) or as a new column (`add: true`, `Table.AddColumn` from the first
  column), with the `Text.*`, `Number.*`, `Date.*`, `Time.*` and `Duration.*` functions of the
  M reference (`age`: `Date.From(DateTime.LocalNow()) - _`, which needs the refresh clock).
  Start and End of day or hour are typed `datetime` whatever the column's type; the Statistics
  menus (a column's sum, average…) and the Add column menus over several columns are not
  written. A value is text, a number,
  true/false, null, `{"date": "YYYY-MM-DD"}` or `{"datetime": "YYYY-MM-DDTHH:MM:SS"}`. The
  step that read `after` next (or the result) reads the new step.
- **`query.edit_step {…, step, formula?, settings?, new_name?}`**: the formula bar (a leading
  `=` is dropped), the step settings gear (`settings`: the `{kind, …}` of `query.apply_step`,
  written anew over the step it read) or a rename; the references to a renamed step follow.
  `query.steps` gives each step's `settings` when its M reads back as one of the transforms that
  ask values (filter rows, replace values, group by, merge, split, keep and remove rows, replace
  errors, index, merge columns, conditional column, the column operations with a value), else
  null; such a step is edited as M.
- **`query.remove_step {…, step}`**: what read the step reads the step before it; the first step
  goes only when no step reads it.
- **`query.apply {queries: [{name, expression}], data_root?, refresh?}`** (Close & Apply): writes
  the changed queries back in one undo step, all or nothing. A table loaded by one M partition is
  reloaded by BIcraft's M evaluator (its columns follow the query's; its partition keeps its name,
  mode and properties), and so is each table whose query reads a changed shared query; a new name
  becomes an import table, as `source.load` makes. `refresh: false` writes the text only, as
  `partition.set` and `expression.set` do. `renamed: [{name, new_name}]` renames model queries
  or tables first (the M of the other queries follows); `removed: [name]` deletes them last,
  refused while another M query still reads one.
- **`query.new {kind?: blank|duplicate|reference, of?, expression?, names?}`** (read): a new
  query for the editor's drafts, named as the editor names it (`Query1`; `Sales (2)` for a
  duplicate or a reference of `Sales`), its M `let Source = "" in Source` (blank), the draft
  (duplicate) or `let Source = Sales in Source` (reference). Nothing changes until
  `query.apply` loads it.
- **`query.rename {name, new_name, queries: [{name, expression}]}`** (read): a query renamed in
  the drafts; returns the drafts whose references follow. Nothing changes until
  `query.apply {renamed}`.
- **`query.folding {name? | expression?, partition?, data_root?, allow_connectors?,
  timeout_ms?}`** (read; writing for the roles with `allow_connectors`, as `query.preview`): the
  folding indicator of each applied step, for the query that ends at that step: `folded` (it goes
  to the source as one request), `partial` (the source runs part of it, BIcraft the rest over the
  rows it returns), `not_folded` (no source that folds) or `unknown` (not evaluated, with
  `reason`), with `connector` and `native_available`. No remote table's rows are read; navigation
  listings and column lists may be. `connectors` (also in every `query.steps` answer) names the
  `allow_connectors` families the query calls, itself or through the model's queries.
- **`query.native {…, step?}`** (View native query): `{native, kind: sql|url, folding,
  connector}` for a folded step, the text BIcraft would send (nothing is sent; a Databricks
  statement lists its parameters as `-- :p0 = value (TYPE)` comments), else `native: null` with
  `reason`. See [formats/data-sources.md](formats/data-sources.md), *Folding indicators*.
- **`query.diagnose {…, step? | all_steps?}`** (Diagnose step): the query at that step (default
  the result; `all_steps`: each step in turn) evaluated as a refresh does, with BIcraft's own
  `elapsed_ms` (the steps before it included), `rows`, `columns`, `error`, and `requests`:
  each request sent to a source (`list` of a navigation node, `query` of a table) with its native
  text, rows and BIcraft's own time. Never Desktop's figures.
- **`query.profile {name? | expression?, partition?, step?, column?, full?}`** (read, as
  `query.preview` with `allow_connectors`): the editor's data profiling, as Microsoft Learn's
  "Using the data profiling tools" describes View › Column quality, Column distribution and
  Column profile. Over the first 1000 rows ("Column profiling based on top 1000 rows"), or the
  whole result with `full: true` ("based on the entire data set"). Each column has `count`,
  `valid`, `errors`, `empty` (null or empty text) and their shares, `distinct`, `unique` (values
  seen once) and `distribution` (its 30 most frequent values with their counts); `column` names
  a column whose `statistics` (count, errors, empty, distinct, unique, min, max; for numbers
  average, sample standard deviation, zero, even, odd; for text empty_string) and 100 most
  frequent `values` come back too. Error values are counted per column by BIcraft's evaluator
  (they load as blanks elsewhere); which statistics Desktop shows for each type was not compared.
- **`query.export {name? | expression?, partition?, step?, queries?, out}`** (writes `out`, as
  `query.preview` otherwise): every row of the query (or of the query at `step`) to a CSV file,
  as the editor's Home › Output data exports a query's results (seen in Desktop 2.159.1231.0;
  BIcraft's button is Home › Export results). Evaluated as `query.preview` is, without its row
  limit (the evaluator's row cap applies); the file is written as `visual.export_data` writes
  CSV: UTF-8 with a byte order mark, a header row, CRLF line ends, text that starts with `=`,
  `+`, `-` or `@` prefixed with `'`. Returns `{name, step, source, path, bytes, columns, rows,
  elapsed_ms}`. Only CSV: the file formats Desktop offers there were not compared.
- **`query.dependencies {queries?: [{name, expression, load?}]}`** (read): the graph of View ›
  Query dependencies (Microsoft Learn, "Query dependencies"), over the model's queries (each
  table's partition and each shared expression) or the editor's drafts. `nodes` have an `id`,
  a `kind` (`source`: a data source call and its literal arguments; `query`; `parameter`;
  `table`: a query loaded into the model; `model_table`), a `name` and a `layer` (sources at 0,
  each query after the queries it reads, model tables last); `edges` go from what is read to what
  reads it (`source`, `reference`, `load`); `unparsed` lists queries whose M does not parse
  (drawn without edges). A reference cycle keeps both edges and does not loop (the layering
  stops after a bounded number of passes). The diagram's layout is
  BIcraft's own, not compared with Desktop's.

Only the steps edited change: every other byte of the query (comments, line breaks,
indentation) is kept, so a query Desktop wrote reads back step by step. The function shapes
follow the public Power Query M function reference; the English step names follow queries saved
by Desktop (a Desktop in another language names its steps in that language; BIcraft always writes
the English names).

### Editing queries without running them (M9.7)

For projects whose tables load from sources BIcraft never runs (Databricks, SQL, …), M is
checked statically and never evaluated (`run_command` for each):

- **`m.validate {expression, shared_expressions?, names?}`**: syntax errors with line and column
  (`m-syntax`), names that are not a step of the query, a query or parameter of the open model
  (unless `shared_expressions: false`), a name in `names`, nor a library function
  (`m-undefined-name`, error), and dotted names BIcraft does not know (`m-unknown-function`,
  warning only). Also returns the steps, the result type when the text tells it and the library
  functions used (`evaluated` by BIcraft, `documented`, `connector`). `model.validate` reports
  the same for every M partition and shared expression under `m` (its `valid` includes them),
  and under `relationships` (`{valid, checked_count, invalid: [{name, from, to, message}]}`, also
  in `valid`) the relationships Power BI Desktop does not open: a one-to-one relationship with
  single-direction cross filtering (Desktop 2.158 refuses the PBIX or PBIP; repair it with
  `relationship.set {cross_filter: "both"}`).
- **`m.format {expression | name, partition?, apply?, max_width?, indent?, tabs?}`** (M11.35):
  lays the M out (one `let` step per line, the result after `in`, groups kept on one line when
  they fit in `max_width`, default 100, else one item per line; `indent` spaces, default 4, at
  most 16, or tabs). Only white space changes: tokens, comments, texts and quoted identifiers stay
  as written, and the result is checked to parse to the same tree (else the text is refused).
  Idempotent. Returns `{name, source: query|table|expression, expression, changed, applied}`.
  Without `apply` nothing changes (no undo step, no new revision; readers may run it); with
  `apply: true` the named query or table partition takes the formatted text through
  `expression.set` or `partition.set`, one undo step, nothing when it is already formatted.
  Details in [formats/data-sources.md](formats/data-sources.md) *Formatting*.
- **`m.complete {expression, cursor? | line?, column?, table?, columns?, queries?, steps?,
  explicit?}`** (M11.35): what may come at the cursor (characters from the start, or `line`
  and `column` from 1; default the end of the text). Returns `{replace: {start, end}, items:
  [{label, insert, kind, detail}]}`: `kind` is `function`, `constant`, `keyword`, `step` (the
  text's own `let` steps and `steps`), `query` (the open model's queries and `queries`, quoted
  as `#"…"` when they must be) or `column` (inside `[ ]`: `columns` and the columns of the model
  table `table`, inserted with their `]`); `detail` is a function's signature or a constant's or
  keyword's description. Names that start with what is typed come first, then those with a
  dotted part that does (`SelectR` finds `Table.SelectRows`), then those that contain it; at
  most 200. Without `explicit` the list opens only on a name being typed or inside `[ ]`; never
  in a comment or a text. The functions are BIcraft's catalogue of those its evaluator runs
  (`bicraft-mlang`, our own one-line descriptions), not every function of the M library.
- **`m.signature {expression, cursor? | line?, column?}`**: the innermost call of a library
  function around the cursor (a call of a step or another function is looked through):
  `{function: {name, signature, description, params: [{name, optional}]} | null, argument,
  active}`, where `argument` counts the commas typed at that call's level (those in lists,
  records and texts do not count) and `active` is the parameter it fills (null past the last).
- **`m.hover {expression, cursor? | line?, column?, table?, columns?, queries?, steps?}`**:
  the name on or just before the cursor, `{hover: {name, kind, range, signature, params,
  description} | null}` with the kinds of `m.complete`. The three commands change nothing,
  evaluate nothing and are open to readers.
- **`partition.set {table, partition?, expression, kind?, mode?}`** (alias
  `table.set_partition`): replaces a partition's source in place. The table, its columns,
  lineage tags, relationships, measures and the partition name stay; the result has
  `previous_expression`. M must pass `m.validate` (errors refuse unless `force`), DAX binds as a
  calculated table. Changing an M table into a calculated one (or back) is refused while it has
  data columns. `partition.add` / `partition.remove` manage tables with several partitions.
- **Shared expressions**: `expression.add` (gets a lineage tag and Desktop's `PBI_ResultType`),
  `expression.set`, `expression.rename` (the M queries that use the name follow, quoted where
  needed: `m_rewritten`) and `expression.remove` (refused while M uses it, unless `force`).
  Query names are unique across tables and shared expressions.
- **`PBI_QueryOrder`**, Desktop's list of queries (model annotation), follows every command
  like Desktop: a new M table or shared expression is appended, a removed one dropped, a renamed
  one renamed in place (a case-only rename too). A table that becomes a query later (its
  partition switched to M by `partition.set`) is appended at the end, after the expressions, and
  dropped when it stops being one; extra M partitions of a query table change nothing. A model without the annotation gets it with its first query; a model
  that has queries but no annotation, or an annotation that is not a JSON list, is left alone.
- **On a PBIX with data** the stored rows came from the old query, and Desktop never stores a
  model whose query and data differ. `file.save` therefore reloads a table whose partitions
  changed (BIcraft's Power Query and DAX evaluators), and `partition.set` refuses a source that
  reaches a connector, directly or through another query, with the advice to save the file as a
  PBIP, edit it there and refresh in Desktop. For the same reason `expression.set` is refused
  when a loaded table uses the expression, and `expression.add/rename/remove` are refused on a
  PBIX with data. PBIP, PBIT and model.bim are edited fully.

### Data sources and environments

The sources of the M queries can be listed and moved without running them: `sources.list`,
`sources.rebase` (path roots, optionally relative to a `RootFolder`-style text parameter;
servers, databases, Databricks hosts, URLs) and the profiles of `bicraft.env.yaml` (`env.apply`,
`env.check`). Only literals are rewritten, the rest of the M stays byte for byte, and each rewrite is
one edit with a `dry_run` preview. See [sources-env.md](sources-env.md).

## Undo, redo and atomic batches

- **Every command that changes the model or the report is one undo step**: `edit.undo {steps?}`
  and `edit.redo {steps?}` move through them, `edit.history` lists them (label, command ids,
  time). Read-only commands, failed commands and commands that change nothing add no step. Undo
  and redo bump `revision`, so the live viewer follows.
- **What undo restores**: the in-memory model and report, their read errors, and the `dirty` /
  `report_dirty` flags. The flags are recomputed against the last open or save: undoing past a
  save makes the session dirty again; redoing back to the saved state makes it clean.
- **What undo does not touch**: files. `save`, `save_as` and `new_project` are not undone, and
  the open file stays the one they wrote. `open_file` and `new_model` clear the history.
- **Bounds**: the last 100 steps or about 256 MB of snapshots, whichever comes first; the oldest
  steps go first. Snapshots are shared between steps (a report edit does not copy the model), so a
  step usually costs one copy of the part it changed.
- **`batch` is atomic by default.** If a command fails, the session goes back to its state before
  the batch: model, report, flags, history, and the open file when a step opened another file or
  started a new model. The result says which command failed (`failed_index`, `failed_command`,
  `rolled_back: true`) and carries the results of the steps before it. Files written by save
  steps inside the batch stay written; the session is then marked dirty. A committed batch is a
  single undo step. `edit.undo` / `edit.redo` cannot run inside an atomic batch. Pass
  `"atomic": false` to keep the steps before a failure applied (each its own undo step).
  `bicraft-cli run` behaves the same (also with `--file commands.json`); `--no-atomic` turns it off.

## Auto-recovery (`recovery.*`)

A host that turns auto-recovery on (the desktop app, `bicraft-cli serve --recovery-dir`) journals
every edit since the last save in a per-user folder, never next to the file
([recovery.md](recovery.md)). `recovery.list {all?}` shows this session's journal and the edits
other sessions left for the open file (`offers`: id, edits, date, `base` same, changed or
missing, `recoverable`, `why`); `recovery.apply {id?}` replays one through the normal commands
(stopping at the first failure, `{applied, total, failed}`); `recovery.discard {id?, all?,
current?}` drops them. Each command's `replay` (exact, copy, skip, base) says how it is journaled;
commands with outside inputs write a full recovery copy instead of a line. `bicraft-cli mcp` and the
CLI keep no journal.

## Report rules

- **The report comes with the file.** `open_file` reads it next to the model: PBIR
  (`definition/`) or the legacy `report.json` of a PBIP, PBIR parts or `Report/Layout` of a
  PBIX/PBIT. A report that cannot be read is reported in `report_error`; the model stays usable.
  After `new_model`, `run_command report.new` starts a blank report.
- **Pages** are found by name or display name (case-insensitive; a display name shared by two
  pages is ambiguous). New pages and visuals get 20-hex-digit names, like Desktop's. Display names
  are unique.
- **Fields** are written `Table[Column]`, `[Measure]` (or `Table[Measure]`), or an aggregated
  column `Function(Table[Column])` with `Sum`, `Avg`, `Count` (distinct count), `Min`, `Max`,
  `CountNonNull`, `Median`, `StandardDeviation` or `Variance`. Table names use DAX quoting
  (`'Order Lines'[Qty]`). They are resolved against the model, which fixes their spelling; an
  unknown table, column or measure is rejected unless `force: true`. Without a model,
  `Table[Name]` is bound as a column, unchecked.
- **Roles** are the visual's data roles: `Values` (card, table), `Category`, `Y`, `Series` (the
  legend), `Tooltips`, `Rows` (small multiples) for charts, `Rows` / `Columns` / `Values` (matrix),
  `Values` (slicer)… `visual.options {visual_type}` lists each built-in type's roles with their
  kind and limits (`data_roles`, `role_conditions`, `required_roles`; see
  `docs/formats/visual-roles-bb.md`). `visual.add`, `visual.bind` and `report.apply` check them: an
  unknown role (with a "did you mean", Legend → Series…), a measure in a column-only role
  (Category, Series, Rows…) and too many fields (a card's second value, a second legend field, two
  Y measures with a legend) are errors unless `force: true`; a role spelled with another case is
  rewritten to Desktop's; a visual missing what it needs to draw (no Category or Y on a chart) gets
  a warning. `visual.bind` only judges what the new field adds. Custom visuals, Q&A, scorecards
  and AI narratives are not checked.
- **Report-level measures** (`reportExtensions.json`, or a legacy layout's `modelExtensions`):
  `report_measure.list`, `report_measure.add {table, name, expression, format_string?, data_type?,
  description?, display_folder?, hidden?, force?}`, `report_measure.set {name, table?, expression?,
  …}` (`null` clears a property) and `report_measure.remove {name, table?}`. Their DAX is validated
  against the model with the other report measures in scope (invalid DAX is refused unless
  `force`); `dataType` defaults to the expression's type. Fields `[Name]` / `Table[Name]` resolve
  to them like model measures. On a legacy layout that has report measures, `add` writes the shape
  Desktop writes there (`dataType` as its numeric code, `formatInformation`), `set` changes
  `expression`, `hidden`, `data_type` and `format_string`; `description` and `display_folder` are
  refused (their legacy form was not observed), and a legacy layout without report measures refuses
  `add` (convert to PBIR first). Not opened in Desktop after such an edit (2026-11-24).
- **`validate_report`** returns errors for schema problems, duplicate page or visual names, a
  missing active page, and every field of a visual, filter or formatting rule that the model does
  not have (bookmarks give warnings). Filters are checked against their `type`: a relative date
  filter on a column that is not a date, a text operator on a column that is not text and a
  literal of the wrong type for its column are errors (Desktop shows an error in the visual); a
  `type` that does not match the shape of its conditions, or a measure filtered by a categorical
  filter or on a page, is a warning. Hierarchies, hierarchy levels and date variations used by
  visuals and filters must exist in the model (errors), and so must the column of a date
  variation's date table a field names (`Column` over a `PropertyVariationSource`). Bookmarks are checked for their active
  page, the pages, visuals and groups they hold state for, captured visual types, target visuals
  and their filters; all bookmark findings are warnings, because Desktop opens and applies them
  anyway (`docs/formats/pbir.md`, "Report validation against Desktop"). Data roles are checked per visual type (above), including the field kinds some combinations need (a scatter chart or map with details takes summarized X/Y, not raw columns) and, with the model, that the fields of a multi-field slicer or list slicer are hierarchically related (one table, or tables linked by one chain of active relationships); both are errors, as Desktop shows an error in the visual (`docs/formats/visual-roles-bb.md`). A scorecard holding a field is an error. Report-level
  measures are validated: their DAX, their table, name clashes with the model, and references to
  report measures that do not exist are errors; in a legacy layout, a `dataType` that is no known
  numeric code, or a `modelExtensions` shape that a PBIR save would leave out, is a warning. Warnings (counted in `warning_count`) flag visuals that stick out
  of their page (saying by how many pixels on which edge) and pairs of visible visuals that overlap
  (`{page, visual, other}`). Overlaps of 1 px or less, hidden visuals, a visual and its own group,
  and a shape or image that wholly holds the other visual (a background) are not reported.
  `valid` is `error_count == 0`.
- **Model edits keep the report honest.** `table.rename`, `table.remove`, `column.set` (rename),
  `column.remove`, `measure.set` (rename), `measure.remove` and `measure.move` return `report`:
  `affected_bindings` (page, visual, role, field), `other_references` (filters and formatting, by
  location) and their counts. Renames and moves rewrite them when given `update_report: true`;
  removals never touch the report (rebind with `bind_field` / `visual.unbind`). A rewrite also
  renames the `queryRef` Desktop derives from a field's name (`Sales.Total` → `Sales.Revenue`,
  `Sum(Sales.Amount)` → `Sum(Sales.Price)`) and every place a visual names it: the selectors of
  series colors and labels, conditional formats and analytics lines, `SelectRef` values,
  expansion states, the phone layout (`mobile.json`) and the visual's state in bookmarks, so the
  formatting follows the field (`report.query_refs_updated` counts them). A `queryRef` that is
  not Desktop's name for its field, or whose new name another field of the visual already has,
  is kept, and so are the references to it; `nativeQueryRef` (the name visual calculations use)
  is never changed.
- **Saving.** `save` writes the model and, if it was edited, the report of a PBIP project as PBIR.
  A legacy `report.json` project is converted: `definition/` is written, `definition.pbir` is
  raised to version 4.0 (3.0 without `.platform`), and `report.json` is removed only after that
  succeeded. `save_as` / `new_project` write the open report into the new project. A
  `model.bim` holds no report (`save` warns).
- **Saving a PBIX/PBIT.** `save` (and `save_as` to a `.pbix`/`.pbit` of the same kind) writes
  only what has unsaved edits, and every other part keeps its bytes:
  - the **model** when it was edited (PBIX: metadata edits, and structural edits with their
    data: tables, columns and relationships added, removed or renamed, `table.import` /
    `table.set_data`, calculated tables and columns evaluated; see `pbix-package.md` §8. PBIT:
    the TMSL text). An unedited model keeps its compressed `DataModel` as is, so a report-only
    save does not re-compress it;
  - the **report** when it was edited, **in the form the file holds**: PBIR parts under
    `Report/definition/` (changed documents rewritten, new pages and visuals added, removed ones
    deleted; untouched documents keep their bytes) or the legacy `Report/Layout` (patched in
    place: untouched pages and visuals keep their text). A legacy report is never converted to
    PBIR. An edit the legacy layout cannot carry (theme, bookmarks, page bindings, report
    settings) fails the save with the list, and nothing is written.
  - `SecurityBindings` is removed whenever a report part changes: Desktop refuses a file whose
    report parts no longer match it, opens the file without it, and writes a new one on its next
    save (`security_bindings_removed`, plus a warning).
  The result has `model_saved`, `report_saved`, `report_format` (`pbir` / `legacy`),
  `report_parts` (`changed`, `added`, `removed` counts) and `security_bindings_removed`. The file
  is replaced atomically; `save` keeps the previous one as `<name>.bak` unless `backup: false`.
- **Conversions** (`save_as` to another kind of file, or `convert {from, to}`; CLI: `bicraft
  convert IN OUT [--format F] [--report-format keep|pbir] [--no-cache] [--empty-tables]
  [--data-root DIR] [--overwrite] [--strict]`).
  Everything goes through the model and report; the target format comes from `format` or the
  extension (`.pbix`, `.pbit`, `.bim`, anything else a PBIP project):

  | From \ to | PBIP (TMDL / TMSL) | PBIT | PBIX |
  |---|---|---|---|
  | PBIX | model + report + `.pbi/cache.abf` (the `DataModel`, with metadata edits written in) | model schema + report | (same kind: `save`) |
  | PBIT | model + report, no cache | (same kind) | built from scratch (below) |
  | PBIP | the other model format; cache copied | model schema + report | with `.pbi/cache.abf`, it becomes the `DataModel`; otherwise built from scratch |
  | `model.bim`, new model | model + blank report | model schema + blank report | built from scratch |

  - **Built from scratch** (`built_from_scratch: true`; `docs/formats/datamodel-fresh.md`): an
    empty database image gets every table as a structural save adds it: M queries run in
    BIcraft's Power Query evaluator (files only under `data_root`), calculated tables and
    columns in its DAX evaluator. A table whose source BIcraft cannot run (database, web and
    script connectors) is refused with its name; `empty_tables: true` writes it without rows,
    keeping its query, for Desktop to load on refresh (listed in `empty_tables`).

  - The report keeps its form: a legacy report stays legacy (`report.json` in a PBIP,
    `Report/Layout` in a package, with its unsaved edits patched in) unless `report_format:
    "pbir"`, which converts it (also between two files of the same kind) and counts the
    legacy-only properties it drops in `warnings`. A PBIR report cannot become legacy.
  - `cache` (PBIP targets): default on when the source has data; `false` skips it; `true` fails
    when there is no data. Without a cache Desktop opens the project without data until refresh.
  - The model diagram and the report's `StaticResources/` and `CustomVisuals/` files are carried
    over. `SecurityBindings` and parts with no counterpart in the target (e.g. `DataMashup`,
    `Connections`) are named in `warnings`.
  - A PBIP from a project is refused when its model differs from the cache in ways the data cannot
    follow (Desktop must refresh and save first). Long paths: same `strict` / `long_paths` rules as
    `new_project`. Existing `.pbix`/`.pbit`/`.bim` targets need `overwrite`; projects are never
    replaced.
  - The result: `saved`, `format`, `converted_from`, `report_format`, `report_converted_to_pbir`,
    `model` (tables, measures, relationships), `report` (pages, visuals), `cache` (PBIP) or `data`
    (PBIX), `warnings`. The written file is read back before the session switches to it.

## Layout

`page.arrange` and `visual.place` compute positions so an agent never does pixel arithmetic.
Results are whole pixels and deterministic; a layout that cannot fit is an error that names the
visual and says how much room is missing, and nothing changes.

**Page layout** (`layout` of `page.arrange`):

| Value | Meaning |
|---|---|
| absent, `null`, `"shelf"` | shelf packing, 16 px margin and gutter |
| `{"shelf": {"margin"?, "gutter"?}}` | shelf packing with these spacings (px, rounded) |
| `"grid"` | a 12-column, 8-row grid, 16 px margin and gutter |
| `{"grid": {"columns"?, "rows"?, "margin"?, "gutter"?}}` | 1 to 100 columns and rows |

The *content area* is the page minus the margin on each side. Grid cells split it evenly, with the
gutter between them.

**Visual placement** (`place` of a `visuals` entry; `null` or absent means automatic):

| Shape | Layout | Meaning |
|---|---|---|
| `null`, `{}` | both | default size for the visual type (below) |
| `{"width"?, "height"?}` | shelf | pixels, or `"NN%"` of the content area *gutters included*: `"50%"` + `"50%"` fill one shelf |
| `{"col", "row", "col_span"?, "row_span"?}` | grid | 0-based cell, spans default to 1 |
| `{"col_span"?, "row_span"?}` | grid | flows into the first free area, row by row, after the explicit cells |
| `{"x", "y", "width", "height"}` | both | a fixed rectangle on the page, kept as given |

Default sizes (shelf share of the content area / grid span on a 12 x 8 grid, scaled to other
grids): cards, KPIs and slicers 25% x 20% / 3 x 2; text boxes 100% x 10% / 12 x 1; buttons and
navigators 1/6 x 10% / 2 x 1; images and shapes 1/6 x 20% / 2 x 2; every other visual 50% x 40% /
6 x 3. On a 1280 x 720 page, a row of four cards and four charts fill the page.

Shelf packing goes left to right and starts a new shelf under the tallest visual of the previous
one. In a grid, cells outside the grid and overlapping cells are errors.

- Without `visuals`, `page.arrange` lays out every visible top-level visual in reading order
  (top to bottom, then left to right). With `visuals`, only those, in that order; the others keep
  their position. A visual inside a group cannot be listed: arrange its group, and the members
  move and scale with it.
- `dry_run: true` returns the positions without changing anything.
- Both commands keep `z` and the tab order, and return the overlaps left on the page in `warnings`.

```json
{"page": "Overview", "layout": {"grid": {"columns": 12, "rows": 8}},
 "visuals": ["card1", "card2",
             {"visual": "chart1", "place": {"col": 0, "row": 2, "col_span": 12, "row_span": 6}}]}
```

## Visual properties

A visual's title, sort, visual-level filters and formatting form one `props` object.
`visual.props` reads it; `visual.set_title`, `visual.set_sort`, `visual.filter`,
`visual.clear_filters` and `visual.format` each change one part (and `report.apply` takes the
whole object per visual). Each returns `{page, visual, changed, changes, props}`: `changes` lists
what was written, and is empty (and the report stays clean) when the visual already had those
values. Values are compared by meaning, so applying the same props twice, or the `props` a visual
returns, changes nothing. Every value is checked before anything is written. Properties you do not
name are left as they are.

| Key | Shape | Written to |
|---|---|---|
| `title` | `"text"` (text, shown), `{text?, show?, font_size?, color?, alignment?: left\|center\|right, bold?, italic?}`, `null` (back to the default) | `visualContainerObjects.title` |
| `sort` | `{field, direction?: asc\|desc}`, a list of them, or `null`. The field must be bound to the visual or exist in the model. Default direction: `desc` for measures and aggregations, `asc` for columns | `query.sortDefinition` |
| `filters` | the complete list of visual-level filters (see below); `[]` removes them | `filterConfig.filters` |
| `format` | the groups below, plus `objects` | `visualContainerObjects`, `objects` |
| `series`, `colors` | per-series / per-category formatting (below) | `objects` entries with a selector |
| `conditional` | conditional formats (below) | `objects` entries with a selector |

**Filters.** Fields are written as elsewhere (`Product[Color]`, `[Total]`, `Sum(Sales[Amount])`).

| Filter | Shape | PBIR `type` |
|---|---|---|
| basic | `{field, in: [values]}` or `{field, not_in: [values]}`, columns only, up to 1,000 values | `Categorical` |
| advanced | `{field, op, value}`, or `{field, conditions: [{op, value}, {op, value}], logic?: and\|or}`. `op`: `=`, `<>`, `>`, `>=`, `<`, `<=`, `contains`, `not_contains`, `starts_with`, `not_starts_with`. `= null` means "is blank" and `<> null` "is not blank". Works on columns, measures and aggregations | `Advanced` |
| top N | `{field, top: N, by: "[Measure]" or "Sum(T[C])", direction?: top\|bottom}`, field is a column | `TopN` |
| verbatim | `{raw: {...}}`: a PBIR filter object kept as given and matched by its `name` (relative dates, tuples…). `visual.props` returns filters it cannot express this way | as given |

Any filter also takes `hidden` and `locked` (hidden or locked in view mode). With a model, values
are typed by the column: text `'Red'`, int64 `5L`, double `100D`, decimal `2.5M`, dates
`datetime'2024-01-31T00:00:00'` (written `"2024-01-31"` or `"2024-01-31T08:00:00"`); values for
measures are doubles. A filter on a field that already has one is updated in place and keeps its
name. A filter left out of the list is removed. A filter left out that sits on an automatic filter
card (a bound field, `howCreated: Auto`) loses only its condition, and the card stays. New filters
get a 20-hex-digit name and `howCreated: User`, as in Desktop.

**Format groups.** Setting a group to `null` removes its formatting. Setting one property to `null`
removes that property. Colors are `#RGB`, `#RRGGBB` or `#RRGGBBAA`, and transparency is 0–100.

| Group | Shape | PBIR object |
|---|---|---|
| `background` | `bool` \| `{show?, color?, transparency?}` | container `background` |
| `border` | `bool` \| `{show?, color?, radius?, width?}` | container `border` |
| `padding` | number (all sides) \| `{top?, bottom?, left?, right?}` | container `padding` |
| `shadow` | `bool` \| `{show?, color?, transparency?}` | container `dropShadow` |
| `header` | `bool` \| `{show?}` (the visual header icons) | container `visualHeader` |
| `alt_text` | string | container `general.altText` |
| `subtitle` | like `title` (without bold and italic) | container `subTitle` |
| `data_labels` | `bool` \| `{show?, font_size?, color?}` | `labels` |
| `legend` | `bool` \| `{show?, position?: Top\|Bottom\|Left\|Right\|TopCenter\|BottomCenter\|LeftCenter\|RightCenter\|TopRight\|BottomRight, font_size?}` | `legend` |
| `x_axis`, `y_axis` | `bool` \| `{show?, title?: bool, font_size?, color?}` | `categoryAxis`, `valueAxis` |
| `objects` | `[{object, property, value, selector?, state?, target?: visual\|container}]` | any |

`objects` is the escape hatch. A value can be:

- a boolean, written `true`
- a number, written `12D`
- a string, written `'text'`
- `{color: "#hex"}`
- `{literal: "3L"}`: a literal kept exactly as written
- a PBIR value (`{expr: …}` or `{solid: …}`), kept verbatim
- `null`, which removes the property

The target defaults to the container when the object and property exist in
`visualContainerObjects`; misspelt container properties are refused. A `selector` addresses the
entry with that exact selector. Without one, the entry without a selector is used, except for
stateful objects (below).

**State selectors.** The new card (`cardVisual`), the new slicers (`listSlicer`,
`advancedSlicerVisual`), buttons (`actionButton`), navigators (`pageNavigator`,
`bookmarkNavigator`) and shapes (`shape`) draw most of their formatting from a *state* entry,
`"selector": {"id": "default"}` (or `hover`, `press`, `selected`, `disabled`…). Power BI Desktop
ignores those properties in the entry without selector: an `accentBar` color written there is
silently not drawn. So an `objects` item for a stateful object goes to the `default` state entry;
`state: "hover"` (any state the object takes) targets another one, and `state: "none"` the entry
without selector. Setting `show` in the `default` state also writes it to the entry without
selector, where Desktop reads the on/off toggle of button and shape fills and texts (Desktop's own
files carry both). `state` is refused on objects without states (a card's `overFlow`, an
`advancedSlicerVisual`'s `layout`, a shape's `shape`, every `textSlicer` and `basicShape` object: Desktop reads
those without selector), on container objects, and together with `selector`. `visual.props` and
`report.extract` list the stateful objects' formatting as `format.objects` items (`state` given
when it is not `default`), so re-applying them changes nothing. `report.validate` warns about
formatting Desktop ignores because of its selector (it does not rewrite it): properties of
stateful objects other than `show` found only in the entry without selector (files written
before), a button fill / text or shape text without `show` in the entry without selector, a card
entry whose selector combines `id` with `metadata`, and properties of objects Desktop reads
without selector (a card's `overFlow`, a shape's `shape` and `rotation`, …) found only in a
`default` state entry. The list of stateful objects per visual type is
in `docs/formats/visual-options.md`, "State selectors".

### Series and conditional formatting

`series`, `colors` and `conditional` format one series (a measure of a chart, a column of a
table) or one category value. `visual.series` and `visual.conditional` set them, and
`visual.format` and `report.apply` take them too. They are patches: what they do not name stays.
`visual.props` returns them, and re-applying that changes nothing.

A **target** says what is formatted:

- `"[Total]"`, `"Sales[Amount]"`, `"Sum(Sales[Amount])"`: a field bound to the visual. It is
  written as Desktop does, `"selector": {"metadata": "Sales.Total"}` (the projection's
  `queryRef`).
- `"Product[Name]=Bikes"`: one value of a column, for one bar or one legend item. It is written
  `{"data": [{"scopeId": {"Comparison": {"ComparisonKind": 0, "Left": <column>, "Right":
  <literal>}}}]}`, and the value is typed by the column (`'Bikes'`, `3L`, `2.5D`, dates).
  `{field, value}` is the same with a JSON value: a number, `null` for blank, or text.

```json
{"colors": {"[Total]": "#118DFF", "Product[Name]=Bikes": "#E66C37"},
 "series": [{"target": "[Margin]", "line": {"width": 3, "style": "dashed"},
             "labels": {"show": true, "color": "#444444"}}]}
```

| `series` item key | Shape | PBIR object |
|---|---|---|
| `target` | as above (required) | the entry's `selector` |
| `color` | `"#hex"` \| `null` | `dataPoint.fill` |
| `labels` | `bool` \| `{show?, color?, font_size?, position?, display_units?, precision?}` \| `null` | `labels` |
| `line` | `{show?, width?, style?: solid\|dashed\|dotted, show_marker?, marker_shape?, marker_size?}` \| `null` | `lineStyles` |
| `objects` | `[{object, property, value}]`, values as in `format.objects` | any |
| `clear` | `true` removes every entry with this target first | |

`colors` is a short way to write `color`. Values are checked against the catalog. The object must
take selectors (`visual.options` shows `selectors`). Entries with other selectors (`id`, roles)
are left alone.

Power BI Desktop 2.158 behaves like this:

- A bar chart with one series takes its bar color from the default color, the `dataPoint` entry
  without a selector, and ignores a per-measure (`metadata`) color. So on a column, bar
  (clustered, stacked, 100%), funnel, ribbon or combo chart whose only field in `Y` is the target,
  with nothing in the legend (`Series`) or the combo's line values (`Y2`), a target's `color` /
  `colors` entry is written as that default entry. A per-measure color already there is kept
  (`visual.props` lists it under the series' `objects`); `null` removes both. Line and area charts
  do the opposite (they use the per-measure color and ignore the default one), so there the
  per-measure selector is written as usual. `visual.props` gives the default color back as the
  series' `color`, so re-applying it changes nothing.
- Per-value colors (`Product[Name]=Bikes`) apply to a chart with one series; with several measures
  Desktop colors by measure and ignores them. Per-measure colors on charts with several measures
  work.
- Per-series `labels` change the labels' color and size once data labels are on
  (`format.data_labels: true`). A series' `labels.show: true` alone does not turn them on.

A **`conditional`** item is `{target?, series?, apply_to?, rule}`:

- `target`: on tables and matrices `background`, `font_color`, `icon`, `data_bars` or `web_url`
  (`values.backColor`, `values.fontColor`, `values.icon`, `columnFormatting.dataBars`,
  `values.webURL`). On cards it is `font_color` (`labels.color`) or `category_color`
  (`categoryLabels.color`), and on charts `data_color` (`dataPoint.fill`) or `label_color`
  (`labels.color`, the data labels). Every visual also takes the container's targets, one value
  for the whole visual written in `visualContainerObjects` (the object's entry without selector):
  `title_text`, `subtitle_text` (a field value: the title from a text measure), `title_color`,
  `title_background`, `subtitle_color`, `visual_background` and `border_color`; they take no
  `series` and no `apply_to`. Any other target is written `{object, property}`, and the catalog
  must mark that property `conditional`. When `target` is left out, it follows the rule (icons,
  data bars, web URL), or else defaults to `background`, `font_color` or `data_color`.
- `series`: the bound column or measure that gets the format. It is written as the selector's
  `metadata`. Tables need it, but it defaults to the rule's field when that field is bound.
- `apply_to`: `values` (the default), `values_and_totals` or `totals`. It is written
  `{"data": [{"dataViewWildcard": {"matchingOption": 1|0|2}}], "metadata": …}`. Data bars use
  `{"metadata": …}` alone.
- `rule: null` removes the conditional format.

| Rule | Shape | Written as |
|---|---|---|
| `gradient` | `{field, min, mid?, max, empty?}`. A stop is `"#hex"` (lowest, middle or highest value), `{color, value}` (a number) or `{color, value, percent: true}` (a percent, 0 to 100, of the range from the lowest to the highest value). `empty` sets blank values: `zero`, `none` or `"#hex"` | `FillRule` `linearGradient2` / `linearGradient3`, `nullColoringStrategy` |
| `rules` | `{field, cases: [{op, value, color}], else?}`. `op` is one of `=`, `>`, `>=`, `<`, `<=`, or `between` (with `values: [low, high]`, both included). `{gte?, gt?, lte?, lt?, eq?, color}` joins two bounds. `= null` means blank | `Conditional` `Cases` (`Comparison`, `And`), `DefaultValue` |
| `field_value` | `{field, as?: color\|value}`. A column is read through `Min`, which is Desktop's "First" | the field expression (`{solid: {color: {expr}}}` or `{expr}`) |
| `icons` | `{field, cases: [{op, value, icon}] \| preset + thresholds: [low, high], else?, position?: before\|after\|only, align?: top\|middle\|bottom}`. The presets are `circles`, `traffic`, `triangles`, `symbols`, `flags`, `stars` and `arrows`. Icon names are Desktop's (`CircleHigh`, `TrafficLow`, `ColoredArrowUp`…) | `{kind: "Icon", value: {expr: Conditional}, layout, verticalAlignment}` |
| `data_bars` | `{positive?, negative?, axis?, reverse?, show_bar_only?, min?, max?}`. The colors default to `#118DFF`, `#D64550` and `#000000` | `{positiveColor, negativeColor, axisColor, reverseDirection, hideText, minValue?, maxValue?}` |
| `web_url` | `{field}` | `values.webURL = {expr: field}` |

Gradient, rules and icons need a number: a measure or an aggregation such as `Sum(T[C])`. Rule
values on measures are written as doubles (`1000D`). `visual.props` returns only the conditional
formats it can express this way. Percent-based rules and theme colors stay in the file untouched.
They also stay in place when other formats are re-applied.

```json
{"conditional": [
  {"target": "background", "series": "[Total]",
   "rule": {"gradient": {"field": "[Total]", "min": "#FFF59D", "max": "#1B5E20"}}},
  {"target": "font_color", "series": "[Margin]",
   "rule": {"rules": {"field": "[Margin]", "cases": [{"op": ">=", "value": 300, "color": "#00C853"},
                                                     {"op": "<", "value": 300, "color": "#D50000"}]}}},
  {"series": "[Cost]", "rule": {"icons": {"field": "[Cost]", "preset": "traffic", "thresholds": [300, 700]}}},
  {"series": "[Margin]", "rule": {"data_bars": {"positive": "#42A5F5"}}},
  {"series": "Sales[Region]", "target": "background", "rule": {"field_value": {"field": "Sales[Color]"}}}
]}
```

### Visual formatting catalog

`visual.options` lists what can be formatted on each built-in visual type, from Microsoft's
published report theme schema (version 2.157), aligned with Power BI Desktop 2.158's own visual
capabilities (see `docs/formats/visual-options.md`). Text sizes
must be at least 8 (the schema's maximum of 60 is not enforced: Desktop renders larger ones).

- `{}`: the 49 visual types, each with its data roles and object count, and the container objects
  (`title`, `background`, `border`, `dropShadow`…) every type has.
- `{visual_type}`: the type's objects, each with `target` (`visual` for `objects`, `container`
  for `visualContainerObjects`), `selectors` (whether entries may be per series / category /
  measure / state), and its properties: `{name, type, min?, max?, values?, conditional?}` with
  `type` one of `bool`, `number`, `integer` (written `3L`), `text`, `color`, `enum`, `font`,
  `image`, `field`, `other` (anything is kept). `conditional: true` marks properties that take
  conditional formatting (field value, rules, gradients): every literal-typed property, since
  Desktop keeps a field value on all of them.
- `{visual_type, object}`: that object (or both, when a visual and a container object share the
  name, like `general`), with display names and descriptions.

**Every value `visual.format`, `visual.set_title` and `report.apply` write is checked** against
the catalog when the visual type is built in:

- an unknown object or property is refused with a hint: `` `clusteredColumnChart` has no visual
  object `legnd`; did you mean `legend`? ``;
- a bad value is refused with what is expected: `` `transparency` expects a number from 0 to 100;
  got 120D ``, `` `position` expects one of 'Top', 'TopCenter', …; got 'bottom' (did you mean
  'Bottom'?) ``, a whole number for `integer`, a `{color}` for colors, a `'#RRGGBB'` or
  `ThemeDataColor` color;
- field and rule expressions are accepted only on `conditional` properties.

Values a visual already has are not re-checked, so re-applying a visual's own `props` always
works, and `null` (removal) is always allowed. Custom visual types are not checked: the edit is
applied and `warnings` says so.

### Chart specifics

Axes, analytics lines, small multiples and report-page tooltips are visual properties: commands
`visual.axis`, `visual.analytics`, `visual.small_multiples`, `visual.tooltip_page`, and the
`props` keys `axes`, `analytics`, `small_multiples`, `tooltip_page` (`report.apply`,
`report.extract`, `visual.props`). Re-running any of them changes nothing.

- **`axes`**: `{x?, y?, y2?}`. `x` is `categoryAxis`, `y` `valueAxis`, `y2` the secondary axis
  (`valueAxis.sec*` on combo charts, `y2Axis` on line and area charts). Each axis takes `show`,
  `title` (bool, or the title text), `title_text`, `min` / `max` (`start` / `end`),
  `start_at_zero: true` (`min: 0`), `scale: linear|log` (`logAxisScale`), `gridlines` (bool or
  `{show, color, width, style}`; not on y2), `display_units`
  (`auto|none|thousands|millions|billions|trillions`), `precision` (0-15), `font_size`, `color`,
  `invert` (not on y2), `type: categorical|continuous` (x only: `axisType`; trend and forecast
  lines need a continuous axis). `null` removes one setting; an axis set to `null` (or `clear: true`) loses
  them all. Desktop has no label-angle or tick-mark setting: `label_angle` / `ticks` are refused.
- **`analytics`**: the complete list of lines (`null` or `[]` removes them all), in the order
  constant, statistical, trend, forecast:
  - `{kind: "constant", value, axis?: "x"|"y"}`: a `y1AxisReferenceLine` (or `xAxisReferenceLine`)
    entry, `selector: {id: "1"}`, `value` a number;
  - `{kind: "average"|"min"|"max"|"median"|"percentile", measure?, percentile?}`: a
    `referenceLine` entry following one series: `selector: {id, metadata: <queryRef>}` and `value`
    an aggregation of it (`{Aggregation: {Expression: {SelectRef: {ExpressionName}}, Function}}`, or
    `{Percentile: {..., K, Exclusive: false}}`). `measure` defaults to the first measure in `Y`;
  - `{kind: "trend"}`: the `trend` object (one, no selector);
  - `{kind: "forecast", measure?, length?, ignore_last?, confidence?, seasonality?, unit?}`: the
    `forecast` object of line charts (`selector: {metadata, id}`), with its `transform`
    (`algorithm: "Forecast"`, defaults length 10, confidence 0.95); trend and forecast need a
    continuous x axis (`axes.x.type`);
  - `{kind: "error_bars", measure?, upper?, lower?, relative?, bar?, color?, match_series?,
    width?, marker?, marker_shape?, band?, band_style?, band_color?, transparency?, data_label?,
    tooltip?}`: the `error` object, one entry per series (`selector: {metadata}`, no id);
    `upper` / `lower` (one at least) are measures or aggregations, `relative: true` adds them to
    the series value; the `band*` keys only where the type has `error.shadeShow` (line charts).
    With a legend every series of the legend gets its bars at its own points; on stacked and 100 %
    columns and bars they sit at the top of the series' segment; on combo charts both the columns
    and the line have them.
    The `errorRange` value is BIcraft's own shape, unchecked ([formats/pbir.md](formats/pbir.md));
  - `{kind: "anomaly_detection", measure?, sensitivity? (0-100, default 70), marker?,
    marker_shape? (circle|square|diamond|triangle|droplet), marker_size?, marker_color?,
    marker_transparency?, band?, band_style? (fill|line|none), band_color?, transparency?}`: the
    line chart's `anomalyDetection` (one, `selector: {metadata}`); refused with a legend, several
    values or a secondary value, and beside a forecast or a min, max, average, median or
    percentile line (Microsoft Learn's limits). The sensitivity's `transform` is BIcraft's own
    shape, unchecked; the preview adds `data.anomalies` `{column, sensitivity, estimate:
    "bicraft", expected, low, high, anomaly}` per row, BIcraft's own estimate, not Power BI's;
  - `{kind: "ratio_line"}` and `{kind: "symmetry_shading", upper_color?, lower_color?,
    transparency?}`: the scatter chart's `ratioLine` and `plotAreaShading` (one each, no
    selector);
  - every line also takes `label` (`displayName`), `color`, `style` (`solid|dashed|dotted`),
    `width`, `transparency` (not forecast), `show` (default true), and except
    trend/forecast/ratio line `position` (`front|back`) and `data_label`.

  At most one trend, forecast, ratio line and symmetry shading, and one set of error bars per
  series. Lines match the existing entries of their object by place: a matched entry keeps its
  selector id and its other settings (data label colors, shading…); new ones get the next id. An
  entry `report.extract` cannot express comes back as `{object, raw}` and is re-written as is.
- **`small_multiples`**: `{field?, rows?, columns?, title?}`: `field` (a column) is bound to the
  `Rows` role, `rows` / `columns` (1-6) set `smallMultiplesLayout` (`layoutType: 'custom'`),
  `title` (bool or `{show, font_size, color, alignment, position: top|bottom}`) the `subheader`.
  `null` removes all three.
- **`tooltip_page`**: a page name (`visualTooltip`: `show`, `type: 'Canvas'`, `section` = the
  page name), `"none"` (`show: false`), or `"default"` / `null` (removes them).
  `visual.tooltip_page` also takes a display name and warns when the page is not a tooltip page.

Page and layout commands:

- **`page.set_type`** writes `page.type` (`Tooltip` / `Drillthrough`) and `pageBinding` (`name`,
  `type`, `parameters`, `referenceScope: "CrossReport"` for `cross_report`,
  `acceptsFilterContext: "None"` for `keep_all_filters: false`). `fields` (the complete list of
  drillthrough / tooltip fields, columns or `Sum(T[C])` for "summarized") become page filters with
  `howCreated: "Drillthrough"` and binding parameters `{name, boundFilter, asAggregation?,
  fieldExpr}`. A page becoming a tooltip page gets the 320 x 240 tooltip canvas (unless
  `width` / `height` are given). `back_button: true` adds Desktop's Back button (an
  `actionButton` with `visualLink` type `Back`); `false` removes it. `type: "report"` removes the
  type, the binding and the drillthrough filters.
- **`page.interactions`** merges rules into `page.visualInteractions` (`filter` → `DataFilter`,
  `highlight` → `HighlightFilter`, `none` → `NoFilter`; `default` removes the rule). A visual
  group as `source` or `target` stands for every visual inside it (nested groups too): one rule
  per pair between them, a later rule of the call winning for a pair; a group and one of its own
  visuals have no pair and are refused.
  The app's Edit interactions mode (Visual tab) sends it with one rule per icon click.
- **`report.settings`** writes `report.json` `settings`: `default_interaction: "filter"` sets
  `defaultFilterActionIsDataFilter: true` (Desktop's report option "Change default visual
  interaction from cross highlighting to cross filtering"; `"highlight"` removes it), and
  `drill_filters_other_visuals` sets `defaultDrillFilterOtherVisuals`. The viewer follows the
  first (a selection filters the visuals that could highlight, unless their rule says
  `highlight`); the second is the default of the reader's drill (a visual's own `drillFilterOtherVisuals`, set by
  `visual.drill_filter`, wins). Returns `{changed,
  default_interaction, drill_filters_other_visuals}`; one undo step.
- **`visual.order`** reorders siblings (the page's top level, or one group's members): the listed
  visuals come first (front), unlisted siblings keep their order behind them. The siblings' own
  `z` and `tabOrder` values are redistributed, so front-to-back and tab order stay consistent.
- **`visual.mobile`** writes `visuals/<visual>/mobile.json` (`visualContainerMobileState`), the
  visual's place on the phone layout; other settings of that document are kept. With `snap:
  true` the place goes onto the phone grid Power BI Desktop 2.159.1231.0 draws (2026-11-24,
  docs/gaps.md G72): x and width on its 18-pixel columns, y and height on its 20-pixel rows, at
  least one cell, kept inside its 324-pixel width, as the app's phone canvas snaps. It is not a
  `props` key (it is a separate document). In a PBIX whose report is a legacy `Report/Layout`
  the phone layout is the visual container's layout with id 1 (plus `singleVisual.mobileObjects` /
  `mobileVcObjects`), and that is what is edited and saved.
- **`visual.mobile_auto {page}`** remakes the page's phone layout in one step by BIcraft's own
  rule (not Desktop's; docs/desktop.md "Mobile layout"), **`visual.mobile_remove_all {page}`**
  takes every visual off it. `visual.copy`'s clip carries the copied visuals' phone documents
  (`mobile`), which `visual.paste {mobile: true}` and `visual.duplicate {mobile: true}` use to
  place the new visuals on the phone layout as well.
- **Saving into a PBIX/PBIT** keeps every edit above. A PBIR package gets the PBIR documents
  (`mobile.json`, page `type` / `pageBinding` / `visualInteractions`, the theme file,
  `resourcePackages` and `themeCollection.customTheme`, selector entries). A legacy
  `Report/Layout` package gets the legacy form of selectors, analytics, axes, interactions
  (`config.relationships`), page types and phone layouts; what it has no form for (page bindings
  of tooltip / drillthrough pages, themes and `resourcePackages`, bookmarks, other PBIR documents)
  fails the save with the list, and nothing is written.

## Report themes

A report's look comes from `report.json` `themeCollection`: a **base theme** (one of Desktop's
monthly themes, `baseTheme`, embedded under `StaticResources/SharedResources/BaseThemes/`) and an
optional **custom theme** (`customTheme`) layered on top: what the custom theme does not set comes
from the base theme. The `theme.*` commands edit the custom theme the way Desktop stores an
imported one:

- the file `StaticResources/RegisteredResources/<theme name><16 digits>.json` (the digits come from
  the file's bytes, so the same theme always gets the same file name);
- an item `{name, path, type: "CustomTheme"}` in the `RegisteredResources` package of
  `resourcePackages` (created, before `settings`, when missing);
- `themeCollection.customTheme = {name, reportVersionAtImport, type: "RegisteredResources"}`, where
  `reportVersionAtImport` is `{visual, report, page}` from the report's schema versions (a version
  string for report schema 1.x/2.x).

Rules:

- **Targeted edits** (`set_colors`, `set_text_classes`, `set_visual_style`) change the custom theme,
  creating it when the report has none. A new custom theme is named `Custom theme` and starts from
  the embedded base theme's `dataColors`, `background`, `foreground`, `tableAccent` and sentiment
  colors (only a name when the base theme is not embedded). `null` removes a key, so the base
  theme's value applies again; containers left empty are dropped.
- **Colors** are `#RGB`, `#RRGGBB` or `#RRGGBBAA`. `data_colors` replaces the whole list (1 to 200
  colors).
- **`set_visual_style`** writes `visualStyles.<visual_type>.*.<object>[entry].<property>`. The entry
  is the one whose `$id` is `state` (e.g. `hover`), else the one without `$id` (kept first). Values
  are theme JSON: `true`, `12`, `"Top"`, `{color: "#hex"}` (written `{"solid": {"color": "#hex"}}`)
  or any other theme value. When the visual catalog knows the visual type, the object, the property
  and the value's type and range are checked (errors unless `force`); an unknown visual type (a
  custom visual) is a warning.
- **Validation.** Every theme is checked against Microsoft's report theme JSON schema
  (`reportThemeSchema-2.157.json`, vendored with the visual catalog). Problems fail the command with
  their JSON pointers unless `force: true`, which installs the theme and lists them in `warnings`.
  Editing a theme that already had problems only fails on new ones.
- **Idempotent, all or nothing.** A command that leaves the theme as it was changes nothing (not the
  files, not the unsaved-changes flag). Replacing or removing the custom theme drops its old file
  and resource item unless something else registers them, so no resource dangles and
  `report.validate` stays clean. Results: `{changed, created?, name, resource, removed_resource?,
  theme_name, resolved, warnings}`.
- The live viewer's `theme` (and the visuals' `format`) reflect the edits at once.
- **Desktop behaviour** (2.158): data colors, `visualStyles` (card label size and color, title color,
  borders) and imported themes show as expected. Desktop's built-in visual styles take precedence
  over `textClasses` for visual titles and axis labels, so a text class alone may not change them;
  set the visual style instead (`theme.set_visual_style {visual_type: "*", object: "title",
  property: "fontColor" | "fontSize" | "fontFamily", value}`).

## Text boxes, images, buttons and page format

The commands behind Desktop's Insert tab and the page's Format pane (ROADMAP M11.20; the JSON
they write: [formats/pbir.md](formats/pbir.md), "Text boxes, images, buttons and page format").

```json
{"tool": "batch", "commands": [
  {"command": "textbox.add", "params": {"page": "Overview", "x": 20, "y": 20, "paragraphs": [
    {"align": "center", "runs": [{"text": "Sales ", "bold": true, "font_size": 18},
                                 {"value": "[Total Sales]", "color": "#118DFF"}]},
    {"list": "bullet", "runs": [{"text": "Details", "url": "https://example.com/sales"}]}]}},
  {"command": "image.add", "params": {"page": "Overview", "path": "logo.png", "scaling": "fit",
                                  "x": 1100, "y": 10, "width": 160, "height": 60}},
  {"command": "button.add", "params": {"page": "Overview", "kind": "blank", "x": 20, "y": 660}},
  {"command": "page.format", "params": {"page": "Overview", "background_color": "#F5F7FA",
                                    "wallpaper_color": "#E1E5EA", "size": "16:9"}}
]}
```

- `textbox.add` / `textbox.set`: a run is `{text | value, bold?, italic?, underline?,
  font_family?, font_size? (points), color?, url?}`; `value` is a measure or `Sum(T[C])`
  inserted as a dynamic value (evaluated in the visual's filters, as Desktop does). `text` is
  plain text, one paragraph per line. `textbox.set` replaces the whole text. Links are http,
  https or mailto only.
- `image.add {path | data}` / `image.set`: `data` is base64 or a `data:…;base64,` URL; at most
  16 MB; typed by content (PNG, JPEG, GIF, WebP, SVG) and decoded before it is stored as a
  `RegisteredResources` image; the same bytes are stored once; a replaced image whose file
  nothing else shows is removed. In server mode `path` is resolved under the workspace.
- `shape_map.set_map {page, visual, path | data | builtin, name?, remove?}`: a shape map's custom map
  (Format › Map settings › Add a map type): `data` is the TopoJSON or GeoJSON text, base64 or a
  `data:` URL; at most 16 MB, 10,000 shapes, 2 million points; stored as a `ShapeMap` item of
  `RegisteredResources` and named by the visual's `shape.map`; `remove: true` drops it. Returns
  the shape count and the first keys (ids or property values the Location values must match,
  without case). The renderer draws it offline; a legacy-layout PBIX refuses it on save (PBIP and a
  PBIR PBIX keep it). `builtin: "world_countries"` writes Natural Earth's countries (keys: the ISO
  alpha-3 id, `name`, `iso_a2`, `iso_a3`, `name_long`, `admin`, `formal_en`, `name_fr`) as the same
  ordinary custom TopoJSON map, which Desktop reads like any custom map. Maps, filled maps and Azure
  Maps draw on an open basemap of bundled Natural Earth data without tiles (drawing only); country
  names, ISO codes and the names of major cities are placed offline, other place names need the
  online geocoding service and draw a placeholder (docs/formats/maps.md).
- `button.add {kind}`: `left_arrow`, `right_arrow`, `reset`, `back`, `information`, `help`, `qna`,
  `bookmark`, `blank`, `apply_all_slicers`, `clear_all_slicers`, `page_navigator`,
  `bookmark_navigator`.
- `visual.set_action {type, destination?, bookmark?, url?, tooltip?, destination_field?,
  tooltip_field?, enabled_tooltip?, disabled_tooltip?}` on a button, image or
  shape: `none`, `back`, `bookmark`, `drillthrough` (to a drillthrough page), `page_navigation`,
  `qna`, `web_url`, `apply_all_slicers`, `clear_all_slicers`. `destination_field` (`[Measure]` or
  `Table[Column]`, a column read as its first value) makes the destination conditional (fx) for
  page navigation, drill through, bookmark and web URL, written in place of the literal; not with a
  fixed destination. `tooltip_field` is the conditional tooltip; `enabled_tooltip` /
  `disabled_tooltip` are a drillthrough button's tooltips ([formats/buttons.md](formats/buttons.md)).
  `view.interact {action: "button", destination?}` runs it as a reader: navigation, Back, drill
  through (the selected data point, M11.19; the button is disabled without exactly one), bookmarks,
  the slicer buttons; a web URL is returned as `url` for the front end to open; `destination` (the
  value the snapshot evaluated, `interaction.button.destination`) is taken for a conditional action
  only; Q&A actions are refused. `view.interact {action: "bookmark_deselect"}` is a bookmark
  navigator's Allow deselection (no bookmark selected). Button, shape and navigator states are
  formatted with `visual.format {objects: [{object, property, value, state: "hover"}]}`
  (`default`, `hover`, `press`, `selected`, `disabled`) and drawn by the readers.
- `page.format`: `background_color`, `background_transparency` (0–100), `background_image`
  (base64; `null` removes it) or `background_image_path`, `background_scaling`; the same five
  `wallpaper_*`; `size` (`16:9`, `4:3`, `letter`, `tooltip`, `custom` with `width` / `height`);
  `vertical_alignment` (`top`, `middle`).
- `page.set_display_option {page, display_option}`: View › Page view, the page's `displayOption`
  (`FitToPage`, `FitToWidth`, `ActualSize`); the canvas scales the page by it unless the status
  bar's zoom (a view setting of the window) overrides it.

Recovery: these commands are journaled as lines (`Replay::Exact`); a run that read a file
(`path`, `*_image_path`) writes a recovery copy instead, and a journal line naming a file is never
replayed ([recovery.md](recovery.md)).

## Declarative reports

`report.apply` builds or updates a report from a *spec*: the pages and visuals it should have.
It creates what is missing, updates what differs and leaves alone what already matches, so an
agent can re-run the same spec after every edit without duplicating anything. `report.check`
(or `report.apply` with `dry_run: true`) returns the plan without changing anything, and
`report.extract` returns the open report as a spec.

```json
{"pages": [
  {"key": "overview", "display_name": "Overview", "width": 1280, "height": 720,
   "layout": "grid",
   "visuals": [
     {"key": "revenue", "type": "card", "place": {"col": 0, "row": 0, "col_span": 3, "row_span": 2},
      "fields": [{"role": "Values", "field": "[Revenue]"}], "props": {"title": "Revenue"}},
     {"key": "by-category", "type": "clusteredColumnChart", "x": 320, "y": 16, "width": 600, "height": 320,
      "fields": [{"role": "Category", "field": "Product[Category]"}, {"role": "Y", "field": "[Revenue]"}]}
   ]}
 ],
 "active_page": "overview"}
```

| Item | Properties |
|---|---|
| spec | `pages: [page]`, `active_page?`: a page key of the spec or a page name |
| page | `key` or `name`; `display_name?` (a new page defaults to its key), `width?`, `height?` (new: 1280 x 720), `display_option?` (`FitToPage`, `FitToWidth`, `ActualSize`), `hidden?`, `layout?` (see *Layout*), `visuals?` |
| visual | `key` or `name`; `type` (needed to create), `x?`, `y?`, `width?`, `height?`, `z?`, `place?` (see *Layout*), `hidden?`, `fields?: [{role, field} \| {role, raw}]`, `props?` (see *Visual properties*); existing groups are listed as `{name, group: true, x?, y?, width?, height?}` |

- **Identity.** A `key` is any string you choose (up to 200 characters). It becomes the PBIR
  name, so the same key always means the same page or visual, in every session and on every
  platform: 20 lowercase hex digits from two FNV-1a 64 hashes (different offset bases), each
  finished with the SplitMix64 mixer, of the page's `key`, or of `<page name>/<visual key>`
  for a visual. `page.add` and `visual.add` take the same optional `key`. A `name` instead
  matches an existing page or visual (exactly, then case-insensitively); `report.extract` names
  everything this way. A `name` nothing has yet creates the object under that name (letters,
  digits, `_`, `-`).
- **Omitted properties are left alone.** `fields` is the complete list of bindings
  (`[]` clears them); fields that equal the current binding at the same place are kept without a
  model lookup. A page without `visuals` keeps its visuals untouched, even with `prune`.
- **Positions.** x, y, width and height all given: that rectangle. None of them, or a `place`:
  the page `layout` places the visual (`layout::place_all`; given values then override). An
  existing visual with only some of them moves only those; a new one needs all four, none, or a
  `place`. New visuals stack above the page's others (z and tab order + 1000).
- **Page order.** Listed pages take the slots listed pages occupy, in spec order; new pages go
  after the existing ones; unlisted pages keep their place.
- **prune.** `true` (or `"visuals"`) removes the visuals of listed pages that the spec does not
  list (a group holding a listed visual is kept); `"all"` also removes unlisted pages. The active
  page moves to the first page when it is removed.
- **All or nothing.** Every check (spec shape, fields against the model, visual types, roles,
  duplicate keys and display names, placement, `props`) runs before the report is touched; any
  error fails `report.apply` with the first problems in its message, and `report.check` lists them
  all as `{at, message}` with `at` a path such as `spec.pages[0].visuals[2].fields[1]`. Limits:
  2,000 pages, 2,000 visuals per page, 100,000 visuals per spec.
- **Results.** `change_count`, `changes` (`{action: create | update | remove | reorder |
  set_active, kind: page | visual | pages | report, page, visual?, key?, properties?}`),
  `summary` counts, `warnings` and `ids` (every key with its page and visual names). A no-op
  apply leaves the report and its unsaved-changes flag exactly as they were.

## Typical agent workflow

1. **Open** an existing model with `open_file {path}`, or start one with `new_model {name}`.
2. **Inspect** it: `inspect_file`, then `model_summary` for the names to reference in DAX.
3. **Edit**: `add_table`, `add_column`, `add_measure`, `set_measure`, `add_relationship`, or any
   command through `run_command`. Check a draft expression first with `validate_dax {expression,
   table}`; it changes nothing.
4. **Validate** the whole model with `validate_model` and fix whatever an edit reported in `broken`.
   For reports: `report_summary`, then `add_page`, `add_visual`, `bind_field`, and
   `validate_report`.
5. **Save**: `save` for the open project, PBIX, PBIT or `model.bim`; `save_as {path:
   "out/Sales.pbip"}` or `new_project {dir, name}` for a new model or to turn a PBIX/PBIT into a
   project; `save_as` to a `.pbit`/`.pbix`, or `convert {from, to}`, for the other conversions.

```json
{"command": "measure.add", "params": {"table": "Sales", "name": "Margin %",
  "expression": "DIVIDE([Margin], [Revenue])", "format_string": "0.0%"}}
```

```json
{"page": "Overview", "visual_type": "clusteredColumnChart", "x": 40, "y": 40, "width": 600,
 "height": 320, "fields": [{"role": "Category", "field": "Product[Category]"},
                           {"role": "Y", "field": "[Margin %]"}]}
```

The same session from the command line:

```sh
bicraft-cli run --open Sales.pbip \
  --cmd 'measure.add={"table":"Sales","name":"Margin %","expression":"DIVIDE([Margin], [Revenue])"}' \
  --save
bicraft-cli new-project out Shop --open model.bim --format tmdl
bicraft-cli validate-report Sales.pbip   # exit 0 valid, 1 invalid, 2 unreadable
bicraft-cli dax-query Sales.pbix "EVALUATE SUMMARIZECOLUMNS('Date'[Year], \"Sales\", [Revenue])"
```

Long edits do not fit on a command line (Windows caps one at 32,767 characters): put the steps in a
file, a JSON array of `{command, params}` (the `commands` of the MCP `batch` tool; `{"commands":
[...]}` is accepted too), and pass it with `--file` (alias `--cmd-file`; `-` reads stdin). The
steps run after any `--cmd`, in order, with the same rules as `batch`: atomic unless
`--no-atomic`, `--save` adds `file.save` at the end.

```sh
bicraft-cli run --open Sales.pbip --file commands.json --save
```

## Live viewer

A read-only web page that shows what is being built while an agent works: pages at their real
size with each visual at its position (type, title, bound fields, validation issues), the model
(tables, measures, relationships) and the validation results. The common visuals (cards,
charts, tables and matrices, slicers) are drawn with the report theme and with data from the
file's VertiPaq storage or BIcraft's DAX evaluator. Editing stays in MCP and the CLI;
the page only reads. The server listens on `127.0.0.1` only.

```sh
bicraft-cli mcp --view [--view-port N] [--open] [--lang TAG] [--no-open-button]   # MCP server + viewer of the MCP session
bicraft-cli view Sales.pbip [--port N] [--open] [--data-root DIR] [--lang TAG] [--no-open-button]   # viewer of a file or project on disk
```

- `mcp --view` publishes a snapshot after every `tools/call`, including failed ones, so the page
  shows the last command and its error. A snapshot computes visual data for a bounded time; while
  visuals still wait for theirs, the server keeps republishing between calls (stdin is read on its
  own thread, so a tool call is handled as soon as the snapshot in progress ends; `bicraft_mcp::
  serve_with_idle`, `Server::with_idle`). The URL is printed on **stderr**, because stdout is the MCP
  channel. Without a port, the OS picks a free one. `--open` launches the default browser.
  `--view-port`, `--open`, `--lang` and `--no-open-button` imply `--view`. `--lang fr-FR` writes the generated visual
  text (auto titles, "(Blank)", "Total"…) in that UI language, like `view.set_language`.
- `view PATH` opens a PBIX, PBIT, PBIP project or `model.bim` and re-reads it when it changes on
  disk, whoever wrote it (`bicraft-cli run --save`, scripts, a Desktop save). It polls modification
  times and waits until the files have been quiet for about 300 ms. For a project it watches only
  the model and report files: `.tmdl`, `.bim`, `.json`, `.pbir`, `.pbism`, `.pbip`, `.platform`.
  It never looks in `.pbi/` and caps the number of files. If a re-read fails (for example on a
  half-written file), the page keeps the last good state and shows the error. Stop it with Ctrl+C.
- The page gets its data from `GET /api/state` and `GET /api/events` (server-sent events). The JSON
  is the snapshot described in [view-snapshot.md](view-snapshot.md), and `revision` goes up by one
  for each command that changed the session. A snapshot is built after every command and stays
  under about 50 ms, even for a 100-page report. To keep it that fast, PBIR schema checks are left
  out of a snapshot when the report has more than 100 visuals (`validation.report.schema_checked:
  false`). Field and structure checks always run, and `validate_report` always checks the schemas.
- **Open in Power BI Desktop.** The page header has a button that opens the session's saved file
  (the `.pbix`/`.pbit`, or the project's `.pbip` file, also when a folder was opened) with the
  operating system's default application: `explorer.exe` on Windows, `open` on macOS, `xdg-open`
  on Linux, without a shell. It is a convenience run on an explicit click; BIcraft never needs
  Desktop. The button is disabled when nothing is saved yet (a new model, a `model.bim`), and shows
  a warning dot when the session has unsaved edits (`mcp --view` after editing tools): the opened
  file does not contain them until `save`. Errors (no application registered for `.pbix`, file
  gone) appear next to the button. `--no-open-button` removes it. The same action is the
  `file.open_in_desktop` command. Security: the page sends `POST /api/open-in-desktop` with no
  body; the server opens only the file of its own session (set on every publish), only a `.pbix`,
  `.pbit` or `.pbip` that exists, and requires the per-run random token it embedded in the page
  (`X-Bicraft-Token`), a localhost `Host`, and, when present, an `Origin` equal to its own and
  `Sec-Fetch-Site: same-origin`; at most one launch every 3 s. See <!-- perf-guard: ok (a cooldown) -->
  [view-snapshot.md](view-snapshot.md) *Open in Power BI Desktop*.

To make an MCP client start the viewer, add `--view` to the server's arguments, e.g.
`"args": ["mcp", "--view", "--view-port", "8765"]`. Then open `http://127.0.0.1:8765/`.

## Document views (`bicraft-cli serve`)

Added with `feat/webui-parallel-views` (2026-11-05). On `bicraft-cli serve`, a workspace has one
session at `/w/<id>/mcp`, as before: existing clients are unchanged. A **document view** is a
private session of its own on one workspace file, beside it: the browser app opens one per
document tab ([webui.md](webui.md), "Document tabs"), and any client holding credentials can.

| Request | Effect |
|---|---|
| `POST /w/<id>/view/api/session` with `{"file": "reports/Sales.pbix"}` (optional `data_root`, `cache_abf`; `Content-Type: application/json`) | opens the view: `{version, view, token, access, file, limits}`. Without a body, the page's view as before (it follows the workspace's session) |
| `POST /w/<id>/mcp/view/<view>` with `X-Bicraft-Token: <token>` | the MCP endpoint on the view's session: the same tools, headers and checks as `/w/<id>/mcp` |
| `GET …/view/api/state?v=<view>`, `…/api/events?v=`, `…/api/asset/<id>?v=`, `POST …/api/interact?v=` | the view's snapshot, event stream, images and interactions, as a page's view |
| `POST /w/<id>/view/api/close?v=<view>` with the token | closes the view now (its session with it; unsaved edits are lost, or kept in the recovery journal with `--recovery-dir`). The browser app sends it when a tab closes and, for every tab, when its page goes (`pagehide`) |

- **Who.** The view belongs to the identity that opened it (another one gets 404) and is opened
  with bearer or proxy credentials, never the viewer's login cookie; a browser's `Origin` and
  `Sec-Fetch-Site` must say the server's page. An **editor**'s view has a session of its own,
  opened on the file, edited and saved on its own; its edits reach no other session, and its
  snapshot shows `view.session: "view"`. A **reader**'s view reads the file's saved state (the
  clean workspace session or the shared read-only copy, as readers' calls do) and runs read
  commands only; a reader's `file.open` on its view changes the file that view reads.
- **Saves.** `expected_revision` works per view (each session has its own revisions, which never
  repeat). Every session of the server, the workspace's included, is held to a **per-file
  check**: each save through the server is a new revision of that file, and a `file.save` (alone or
  in a `batch`) by a session that read an older revision is refused with a tool error, **nothing
  written**: `{"code": "file_conflict", "file", "file_revision", "read_revision"}`. Save under
  another name (`file.save_as`) or reopen the file (`file.open`, which drops this session's unsaved
  edits) to go on. An upload over a file a view has unsaved edits to answers 409; over a clean
  one, the view reopens the new file.
- **Limits.** At most `--max-views-per-user` views per identity over the server (default 8, the
  app's tab cap) and `--max-views` per workspace (default 32), both in every snapshot
  (`view.limits`) and in the answer above. At a cap, the caller's least recently used view with no
  event stream and no unsaved edit closes to make room (on a full workspace, also another
  identity's unused for a minute); otherwise **429** with the limit in words. A new view needs
  memory: above `--ready-memory-percent` (90 % by default) of `BICRAFT_MEMORY_LIMIT`, **503**; each
  view's session counts against the process-wide memory pool like any session. A view without a
  request or an open event stream for `--view-idle` seconds (default 1800) closes; one with unsaved
  edits is kept four times as long, then closed without saving (logged). Views are dropped at
  shutdown without saving (the drain report lists their workspaces as unsaved). With
  `--recovery-dir`, those unsaved edits stay in the view's recovery journal (logged as "kept for
  recovery"): when the same identity opens the file again, `recovery.list` and the snapshot's
  `recovery.offers` show them and `recovery.apply` replays them ([recovery.md](recovery.md)).
  `view.limits` also says `recovery` (journals kept) and `autosave` (`--allow-autosave`).
- **Metrics and deployment.** `/metrics` has `bicraft_document_views_open{mode="edit"|"read"}`,
  `bicraft_document_views_max` and the journals kept (`bicraft_recovery_journals`); the Helm chart
  and the controller set the three flags ([kubernetes.md](kubernetes.md), "Viewer").
- Tests: `crates/mcp/tests/http_docviews.rs` (two views edited at once from two threads, the save
  conflict, caps and eviction, the memory check, idle expiry, a release that drops a streamed
  view at once and only for its own identity with its token, readers' views, the metrics).

## UI control channel (`bicraft-cli serve --viewer-control`)

Added with `feat/control-serve` (2026-11-05). Not an MCP tool: the desktop's control channel
([desktop.md](desktop.md), "Control channel") relayed by the server to an editor's own browser
app page, so an agent that edits over MCP can also drive and screenshot the window a person
looks at. `POST /w/<id>/view/api/control[?v=<view>]` with the editor's credential and
`X-Bicraft-Control: <control token>`; `GET` the same route lists the caller's views, a page's
and the document views it opened (`?v=` of one runs the request in that file's tab).
`bicraft-cli ui --serve --workspace ID METHOD [PARAMS]` is the client. Flags, token file, routes,
caps and the security model: [web-deployment.md](web-deployment.md), "UI control channel".

## Errors

- **Protocol errors** are JSON-RPC errors: `-32700` parse error (also for lines over 64 MiB or not
  UTF-8), `-32600` invalid request, `-32601` unknown method, `-32602` unknown tool or arguments that
  are not an object.
- **Tool errors** are results with `isError: true`. The text content is the message and
  `structuredContent` is `{"error": message}`. They cover failed commands (no model open, unknown
  table, invalid DAX without `force`, existing target on save…) and bad arguments. A failed command
  leaves the session exactly as it was.
- A failed `batch` step returns `{error, failed_index, failed_command, rolled_back, results}`. With
  `atomic` (the default) the steps before it are rolled back; with `"atomic": false` they stay applied.
- An invalid expression given to `validate_dax` is *not* a tool error: it returns `valid: false`
  with its diagnostics.
- `save` on a PBIX fails, and writes nothing, when a structural edit needs data BIcraft cannot
  compute exactly (a data column added to an existing table, an M connector, DAX the evaluator
  does not cover; the error gives the reason, `pbix-package.md` §8) or the report has edits its
  legacy `Report/Layout` cannot carry; use `save_as` with a `.pbip` path.
- Report tools on a file without a readable report fail with the reason (`report_error`), and say
  how to start one.
