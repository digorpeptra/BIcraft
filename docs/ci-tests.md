# CI quality gates: `bicraft test`

`bicraft test` runs a Power BI project's quality gates from a declarative test file kept in the
repository next to the project. It is built for CI pipelines (GitHub Actions, Azure DevOps,
GitLab, Jenkins) and works the same way for an agent over MCP (`run_tests`) or through the engine
command `project.test`.

It covers what the community asks for in PBI-Inspector issues, Tabular Editor BPA issues and
pipeline-check requests:

- **DAX assertions.** A measure, expression or query is evaluated over the model's data by
  BIcraft's own engine, never Power BI Desktop, and compared with an expected value or table.
- **Model rules.** `model.validate`, Microsoft's BPA rules and your own BPA rule files, and
  declarative rules: naming, required properties, forbidden functions, expression length.
- **Report rules.**
  - `report.validate` findings: fields that do not exist, broken bookmarks, filters, structure,
    schemas.
  - Declarative rules: visuals per page, alt text, page size, theme, navigation to hidden pages.
- **Usage rules.** "No unused measures", once `model.unused` is available (see below).

Everything runs on Windows and Linux and needs no Power BI installation.

```text
bicraft test [PATH] [--tests FILE] [--format text|json|junit|sarif] [--output FILE]
             [--fail-on error|warning]
```

| Option | Meaning |
|---|---|
| `PATH` | A `.pbip`, a project folder, a `.pbix`, a `.pbit` or a `model.bim`. Default: the current folder. |
| `--tests FILE` | The test file. Default: `bicraft.tests.yaml`, `bicraft.tests.yml` or `bicraft.tests.json` next to the project. |
| `--format` | `text` (default), `json`, `junit` (JUnit XML) or `sarif` (SARIF 2.1.0). |
| `--output FILE` | Write the report to FILE. The text summary still goes to stdout for the CI log. |
| `--fail-on` | The lowest severity of a failed test that fails the run: `error` (default) or `warning`. |

Paths in reports are relative to the current folder. In a CI job that is the repository
checkout, which is what GitHub code scanning expects.

## Exit codes

These codes are stable.

| Code | Meaning |
|---|---|
| 0 | Every gate passed. Tests failed below `--fail-on`, and skipped tests, do not count. |
| 1 | A test failed (`failed`) or could not run (`error`) at or above `--fail-on`. |
| 2 | Nothing was tested: a usage error, a test file that cannot be read or is not valid (YAML/JSON syntax, unknown keys, wrong types, duplicate ids, an invalid regular expression, a missing BPA rule file), or a project that cannot be opened. |

A valid test file can still name objects the model does not have, for example `measure: Nope`.
Such a test is **not** a configuration error. It runs, the engine reports the unknown measure,
and the test ends with status `error`, which gives exit code 1. This is deliberate:

- The model is what changes from commit to commit. A measure that is renamed or removed must
  fail the gate like any other regression.
- The other tests of the run still report their results.

Exit code 2 is kept for "the gates could not be evaluated at all".

## The test file

Put it next to the `.pbip` (or `.pbix`) and version it with the project. YAML is preferred:

- it holds comments;
- multi-line DAX fits naturally with `|`.

JSON is accepted too: `bicraft.tests.json`, or any file ending in `.json`. Both forms follow the
same schema.

```yaml
version: 1                       # required: the format version
name: Sales model gates          # optional, shown in reports (JUnit <testsuites name>)
defaults:                        # optional, for every DAX assertion
  tolerance: 0                   # absolute tolerance on numbers
  relative_tolerance: 1.0e-9     # plus this fraction of |expected|
  require_data: false            # without loaded data: skip (false) or fail (true)
  timeout_ms: 60000              # time budget of one assertion (at most 600000)

dax:
  - name: Total sales
    measure: Total Sales         # or "[Total Sales]" or "Sales[Total Sales]"
    expect: 125.5
    type: number
  - name: Toys only
    measure: Total Sales
    filters:                     # CALCULATE filter arguments, in DAX
      - Sales[Category] = "Toys"
      - 'Date'[Year] = 2024
    expect: 25.5
    tolerance: 0.01
  - name: Row count in range
    expression: COUNTROWS(Sales)
    min: 1
    max: 1000000
  - name: Sales by category
    query: |
      EVALUATE SUMMARIZECOLUMNS(Sales[Category], "Total", [Total Sales])
      ORDER BY Sales[Category]
    expect:
      columns: [Category, Total]   # "Sales[Category]" or the bracketed part
      row_count: 2
      rows:
        - [Toys, 25.5]             # positional ...
        - {Category: Wheels, Total: 100}   # ... or by column
    ordered: true                  # default; false compares rows as a multiset

model:
  validate: true                 # model.validate: DAX, M, relationships (default true)
  bpa:                           # Best Practice Analyzer (model.analyze)
    default_rules: true          # Microsoft's embedded BPARules.json
    rules: [rules/team-bpa.json] # more rule files, relative to the test file
    include: ["DAX_*", "META_*"] # rule ids, * and ? wildcards (default: all)
    exclude: [DAX_DIVISION_COLUMNS]
    min_severity: warning        # 1-5, info, warning or error (default warning)
  rules:
    - name: Measures in Title Case
      kind: naming
      scope: measure             # table, column, measure, hierarchy, role, expression
      pattern: "^[A-Z]"          # must match (ECMAScript subset, unanchored)
      forbid_pattern: "  "       # must not match
    - name: Visible measures are described
      kind: required
      property: description      # description, format_string, display_folder
      scope: measure
      where: {hidden: false}     # also table: [globs], name: [globs]
    - name: No IFERROR or EARLIER
      kind: forbidden_functions
      functions: [IFERROR, EARLIER]
      # scope (default every DAX object): measure, calculated_column, calculated_table,
      # calculation_item, row_filter
    - name: Readable expressions
      kind: max_expression_length
      max: 2000
      severity: warning          # error (default), warning or info

report:
  validate:                      # report.validate findings, one test per category
    exclude: [schema]            # schema, structure, fields, filters, bookmarks
  rules:
    - kind: fields_exist         # report.validate "fields" findings
    - kind: bookmarks_valid      # report.validate "bookmarks" findings
    - kind: max_visuals_per_page
      max: 15
      count_hidden: false
    - kind: alt_text
      exclude_types: [textbox, shape, basicShape, actionButton, line]   # the default
    - kind: page_size
      allowed: [{width: 1280, height: 720}, {width: 1920, height: 1080}]
      include_tooltips: false    # tooltip pages are left out by default
    - kind: theme
      custom_theme: Corporate    # a name (wildcards allowed) or true for any custom theme
      base_theme: CY24SU10
      forbid_hardcoded_colors: true
    - kind: no_hidden_page_navigation
      pages: ["Overview*"]       # any report rule: only these pages (name or display name)

usage:
  no_unused_measures: warning    # true (warning), or a severity
```

### Common keys

- Every DAX assertion and rule takes an optional `id`: 1 to 200 characters, letters, digits,
  `-`, `_` or `.`.
  - Without one, the id is derived from `name` (`Toys only` → `toys-only`).
  - Ids are unique within a suite. A duplicate is a configuration error.
  - Reports show `suite/id`, for example `dax/toys-only`. That is also the SARIF rule id.
- `severity` is `error` (the default), `warning` or `info`.
  - A failed test fails the run only when its severity reaches `--fail-on`.
  - A failed test below that level is a *warning*: JUnit shows it as passed, with its findings
    in `<system-out>`; SARIF and text show its own level.
- Any section takes `enabled: false`.
- Unknown keys are errors, so a typo never disables a gate silently.

### DAX assertions

Each assertion becomes one query, run by `dax.evaluate`:

- `measure` or `expression` without filters: `EVALUATE ROW("Value", <expr>)`;
- with filters: `EVALUATE ROW("Value", CALCULATE(<expr>, <filter>, ...))`;
- `query`: the query as written. Its first `EVALUATE` is compared.

The data comes from where `dax.evaluate` finds it:

- a PBIX's DataModel;
- a PBIP's BIcraft data snapshot (written by `bicraft refresh`);
- a PBIP's `.pbi/cache.abf`;
- the tables' partitions that BIcraft can evaluate (DATATABLE, calculated tables, M queries over
  literal data).

When the data cannot be read (`data not available`), the assertion is **skipped** with the
reason. For example, a project committed without its cache whose M sources are databases or
files. With `require_data: true` it is an **error** instead.

Comparison is typed:

| Expected | Matches |
|---|---|
| a number | a number within `tolerance + relative_tolerance × |expected|`; never a text |
| a text | the same text (case-sensitive) |
| `true` / `false` | the same boolean |
| `null` | BLANK only; never 0 or "" |
| `"2024-01-01"` | a date column at midnight, or the full ISO text (`2024-01-01T12:30:00`) |

Other keys:

- `min` / `max` bound a single numeric value. They take `tolerance` into account.
- `type` checks the returned type of every value: `number` (integer or decimal), `integer`,
  `text`, `boolean`, `date`, or `blank` (the value must be BLANK). A BLANK value passes every
  other type rule.
- A query compared with a single value (`expect: 3`) must return one row and one column.
- Without `expect`, `min`, `max` or `type`, the assertion passes when the query runs.

Each assertion runs under its own time budget (`timeout_ms`, default 60 s) and under the same
memory budget and process memory limit (`BICRAFT_MEMORY_LIMIT`) as any query. One slow test
cannot hang a CI job or a server.

### Model rules

`model.validate` gives three tests:

- `model/validate-dax`: every measure, calculated column and table, calculation item and row
  filter is bound against the model;
- `model/validate-m`: Power Query syntax and undefined names;
- `model/validate-relationships`.

Diagnostics keep their own level (error or warning).

BPA runs `model.analyze`, and each rule that runs becomes its own test (`bpa/<RULE_ID>`):

- its level is the rule's severity (1 info, 2 warning, 3 and above error);
- a rule BIcraft's interpreter cannot evaluate is **skipped** with the reason;
- `min_severity` leaves lower rules out entirely;
- invalid rules in a rule file are reported as `bpa/invalid-rules`.

Declarative rules look at model objects:

- `where.hidden` filters on the hidden flag.
- `where.table` and `where.name` filter with case-insensitive `*` / `?` globs. `where.table` is
  the table itself for tables and the home table otherwise.
- `naming` patterns use the same ECMAScript regular-expression subset as PBIR schema patterns:
  classes, groups, alternation, quantifiers, anchors, lookahead. They are unanchored, so write
  `^...$` for a full match.
- `forbidden_functions` reads the DAX tokens. A function name inside a string or a comment does
  not count.

### Report rules

`report.validate` findings are sorted into categories:

| Category | Contents |
|---|---|
| `schema` | PBIR JSON schema issues |
| `fields` | Fields bound by visuals that are not in the model |
| `filters` | Filter containers |
| `bookmarks` | Bookmark targets and bookmarked fields |
| `structure` | Pages, names, sizes, overlaps, active page |

`report.validate` gives one test per category, and `fields_exist` / `bookmarks_valid` select
one.

The declarative rules:

- `max_visuals_per_page` counts visuals but not groups. Hidden visuals count only with
  `count_hidden`.
- `alt_text` reads `general.altText` and leaves out hidden visuals and `exclude_types`.
- `page_size` leaves out tooltip pages unless `include_tooltips`.
- `theme` compares the report's `themeCollection` names. `forbid_hardcoded_colors` flags visuals
  whose formatting holds literal colors (`solid.color` literals) instead of theme colors.
- `no_hidden_page_navigation` flags visible page-navigation buttons whose target page is hidden
  or missing.

### Usage rules

`no_unused_measures` runs `model.unused` (see `docs/mcp.md` "Where objects are used, and what is
unused") over the open model and its report, with the default conservative rules, and reports
each measure of `unused.measures` (`[{table, name, why}]`) with its reason, e.g. `measure
'Sales'[Lines] is not used (used only by unused objects: measure [safe ratio])`. Unused columns,
tables and shared expressions are not judged by this rule. A build without `model.unused`
skips the test with a note.

## Outputs

All formats list the tests in the same order: suites in run order (`dax`, `model`, `bpa`,
`report`, `usage`), and tests in file order. Apart from durations (`elapsed_ms`, JUnit `time`),
two runs over the same files give the same bytes.

- **text**: one line per test, `PASS` / `FAIL` / `ERROR` / `WARN` / `SKIP`, with findings and
  their `file:line`.
- **json**: the result of `project.test`:
  - `passed`, `exit_code`, `fail_on`;
  - `summary` {tests, passed, failed, errors, skipped, warnings};
  - `suites` [{name, tests: [{id, name, kind, status, severity, gates, message, findings:
    [{level, message, location: {path, line}}], elapsed_ms, details}]}];
  - `notes`.

  `details` holds the query, its data source and the actual value or rows of a DAX assertion,
  and the source and status of a BPA rule.
- **junit**: `<testsuites>` with one `<testsuite>` per suite, and `<testcase classname="bicraft.<suite>">`.
  - A gating failure is `<failure message type="error|warning|info">` with the findings.
  - A test that could not run is `<error>`.
  - A skipped test is `<skipped message>`.
  - A failure below `--fail-on` passes, with `WARNING` and its findings in `<system-out>`.
- **sarif**: SARIF 2.1.0 (`$schema` `https://json.schemastore.org/sarif-2.1.0.json`).
  - One run, tool `bicraft`, one rule per failed test (`suite/id`).
  - One result per finding: level `error`, `warning` or `note`.
  - Physical locations are relative URIs with a line where known:
    - TMDL `tables/<Table>.tmdl` at the measure, column, hierarchy or partition line, and
      `relationships.tmdl`, `roles/`, `expressions.tmdl`;
    - `model.bim` at the object's `"name"`;
    - PBIR `pages/<page>/visuals/<visual>/visual.json`, `page.json`, `bookmarks/*.bookmark.json`
      and `report.json`;
    - the test file, at the assertion's `name:` line, for DAX assertions.

    A PBIX finding points at the `.pbix`.

The SARIF output is checked structurally by the tests: required properties, levels, rule
indexes, relative URIs. The OASIS SARIF JSON schema is not vendored: its licence (OASIS IPR
policy) is not one of the permissive licences this repository takes in.

## In a pipeline

### GitHub Actions, with a release binary

```yaml
# .github/workflows/powerbi-gates.yml
name: Power BI gates
on: [pull_request]
permissions:
  contents: read
  security-events: write          # upload SARIF to code scanning
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Install bicraft
        run: |
          # the Linux archive of a BIcraft release (docs/releasing.md); pin the version
          curl -fsSL -o bicraft.tar.gz "$BICRAFT_URL/bicraft-0.1.0-linux-x64.tar.gz"
          echo "$BICRAFT_SHA256  bicraft.tar.gz" | sha256sum -c -
          tar -xzf bicraft.tar.gz
          echo "$PWD/bicraft-0.1.0-linux-x64" >> "$GITHUB_PATH"
        env:
          BICRAFT_URL: https://example.com/bicraft/releases   # where you host the archives
          BICRAFT_SHA256: <sha256 of the archive>
      - name: Run the gates
        run: bicraft test Sales/Sales.pbip --format sarif --output bicraft.sarif
      - name: JUnit report
        if: always()
        run: bicraft test Sales/Sales.pbip --format junit --output bicraft-junit.xml || true
      - uses: github/codeql-action/upload-sarif@v3
        if: always()
        with:
          sarif_file: bicraft.sarif
          category: bicraft
```

`bicraft test` exits 1 when a gate fails, which fails the step. The SARIF upload runs anyway
(`if: always()`), so the findings show up on the pull request at their TMDL / PBIR lines.

### With the Docker image

The image (`deploy/docker/Dockerfile`, `docs/kubernetes.md`) starts `bicraft serve` by default.
Override the entrypoint to run the gates:

```bash
docker run --rm -v "$PWD:/work" -w /work --entrypoint /usr/local/bin/bicraft \
  registry.example.com/bicraft:0.1.0 test Sales/Sales.pbip --format junit --output bicraft-junit.xml
```

### Any CI system

```bash
bicraft test path/to/Project.pbip --format junit --output reports/bicraft-junit.xml
status=$?            # 0 passed, 1 a gate failed, 2 configuration or usage error
# publish reports/bicraft-junit.xml with the CI's JUnit publisher (Azure DevOps
# PublishTestResults@2, GitLab artifacts:reports:junit, Jenkins junit step)
exit $status
```

On Windows the same commands work with `bicraft.exe` from the Windows archive.

## MCP and the engine command

- MCP tool `run_tests {path?, tests?, format?, fail_on?, base_dir?}`.
- Engine command `project.test` takes the same parameters, through `run_command` or
  `bicraft run --cmd`.
  - `tests` is a file path, or the test object inline: an agent can check a gate without
    writing a file.
  - `format` is one name, or a list rendered once each into `outputs`.

Server mode:

- **Read-only.** Readers may run it (`docs/auth.md`, "Roles").
- **Isolated.** With `path`, the project opens in a private session; the caller's open file,
  its edits and undo history are untouched. That session counts against the process memory
  limit like any other.
- **Paths go through the sandbox** (`docs/server-mode.md`).
  - `path` is resolved by the private session.
  - `tests` and `base_dir` are resolved under the workspace root.
  - BPA rule files named in the test file are resolved under the root, relative to the test file.
  - A test file found next to a project must be under the root.
- **No server path in answers.** Paths in results are relative to the base folder or the
  workspace.
