# Data sources and environment profiles

A Power BI model often points at a different place in each environment. File sources use absolute
paths, such as `C:\Users\me\Finance\sales.xlsx`. Databases use a development server, and
Databricks uses a development workspace. Power BI Desktop has no relative paths for Excel and CSV
sources, and deployment rules in the service cover only some cases. BIcraft rewrites these sources
in the model's M text, without running it:

- `sources.list` lists every data source and its arguments;
- `sources.rebase` moves file paths to another root folder, optionally relative to a text
  parameter, and maps servers, databases, Databricks hosts and URLs;
- `env.apply` and `env.check` use named profiles from a versioned file, `bicraft.env.yaml`.

All of them work on PBIP projects (TMDL and TMSL), model.bim files and PBIT templates. On a PBIX
that stores data, the writing commands are refused (only `dry_run` works), because the stored rows
would no longer match the queries. Save the PBIX as a PBIP first, then refresh it in Desktop.

## What counts as a source

The scan reads the M of every M partition and shared expression, token by token. It recognizes the
source functions of the public Power Query M function reference and gives each leading argument a
role:

| Role | Functions |
|---|---|
| `path` | `File.Contents` |
| `folder` | `Folder.Files`, `Folder.Contents` |
| `server`, `database` | `Sql.Database`, `Sql.Databases`, `PostgreSQL.Database`, `MySQL.Database`, `Oracle.Database`, `Teradata.Database`, `Sybase.Database`, `SapHana.Database`, `AnalysisServices.Database(s)`, `Snowflake.Databases` (server, warehouse) |
| `host`, `http_path` | `Databricks.Catalogs`, `Databricks.Contents`, `Databricks.Query`, `DatabricksMultiCloud.Catalogs` |
| `url` | `Web.Contents`, `OData.Feed`, `SharePoint.Files/Contents/Tables`, `AzureStorage.DataLake`, `Hdfs.Files/Contents` |
| `account` | `AzureStorage.Blobs`, `AzureStorage.Tables` |
| `connection_string` | `Odbc.DataSource/Query`, `OleDb.DataSource/Query` |
| `content` | `Excel.Workbook`, `Csv.Document`, `Json.Document`, `Xml.Tables/Document`, `Parquet.Document`, `Pdf.Tables`, `Lines.FromBinary`, `Access.Database` |

A `content` function is shown as the `format` of the call it reads, for example `File.Contents`
read by `Excel.Workbook`. The link is found when the call is nested directly in the format
function, or when it goes through one step of the same query.

Each argument has a form:

| Form | Example | Rewritten |
|---|---|---|
| `literal` | `File.Contents("C:\Data\a.xlsx")` | yes |
| `step_literal` | `Path = "C:\Data\a.csv", Source = Csv.Document(File.Contents(Path))` (one binding of that name) | yes, at the step |
| `concat` | `File.Contents("C:\Data\" & Name)` or `File.Contents(RootFolder & "a.xlsx")` | the leading literal only (paths and URLs) |
| `parameter` | `Databricks.Catalogs(Host, HttpPath)` | the parameter's value |
| `query`, `step`, `name` | another query, a computed step, an unknown name | no |
| `call`, `computed` | `File.Contents(Text.Combine({...}))` | no: reported, never guessed |

Each argument also has its byte span in the M text, plus its line and column. A query that does not
tokenize is listed under `unparsed` and left alone.

## Rebasing paths

```json
{"command": "sources.rebase", "params": {"from": "C:\\Data\\Finance", "to": "\\\\files\\share\\fin"}}
```

- A root matches whole folder names: `C:\Data` does not match `C:\Database`. When several roots
  match (in a profile), the longest one wins.
- Windows drive paths and UNC paths compare without regard to case, and `\` and `/` count as the
  same separator. POSIX paths (`/mnt/data`) compare exactly. A drive path never matches a UNC or
  POSIX root.
- The rest of the path takes the new root's separator, so a root can move from Windows to Linux.
  A trailing separator is kept.
- Text parameters whose value is a path under the root are rebased too.
- When the new root is not an absolute path, a warning says so: Power Query reads files by
  absolute path.

**Relative to a parameter.** With `param`, a path under `from` becomes
`RootFolder & "rest\of\path.xlsx"`. The parameter is set to `to` (default `from`) with a trailing
separator. When it does not exist yet, it is added as Desktop writes a parameter:

```
"D:\fin\" meta [IsParameterQuery=true, Type="Text", IsParameterQueryRequired=true]
```

The new parameter gets a lineage tag, Desktop's `PBI_ResultType` annotation and a place at the end
of `PBI_QueryOrder`. A name that needs quoting is written `#"Data Root"`. An existing parameter must
have type Text, and the name must not belong to a table or to another query. When no path is under
`from`, the parameter is not added and a warning says so.

**Servers and the rest.** `servers`, `databases` and `hosts` (`{old: new}`) match exactly without
regard to case, `http_paths` match exactly, and `urls` match by prefix. A text parameter used as a
server, a host and so on is mapped the same way.

**Guarantees.** Only the literals are replaced. Every other byte of the M text stays the same,
including comments, spacing and line ends, and the new text is escaped the way M expects: `"`
doubled, control characters and `#(` written as `#(…)` escapes. The whole rewrite is one edit, so
one `edit.undo` reverts it. `dry_run: true` returns the same result without changing anything.
Each entry of `changes` gives the object, function, role, `before`/`after` values,
`m_before`/`m_after` texts and the span. `parameters` lists parameter values that change, and
`unchanged` lists the location arguments left alone with the reason: `no_mapping`, `parameter`
or `computed`.

## Environment profiles

`bicraft.env.yaml` (or `.yml`, `.json`) sits next to the project. `env_file` can name another
file or give the profiles inline.

```yaml
version: 1
profiles:
  dev:
    parameters:
      RootFolder: 'C:\Data\Finance\'
  prod:
    description: production
    parameters:
      RootFolder: '\\files\share\fin\'
      Year: 2026
      Live: true
      Start: 2026-01-01
    paths:
      'C:\Data\Finance': '\\files\share\fin'
    servers:
      dev-sql.contoso.local: prod-sql.contoso.local
    databases:
      FinanceDev: Finance
    hosts:
      adb-1.azuredatabricks.net: adb-2.azuredatabricks.net
    http_paths:
      /sql/1.0/warehouses/dev: /sql/1.0/warehouses/prod
    urls:
      'https://dev.example.com/': 'https://api.example.com/'
```

- Single quotes keep backslashes as written in YAML.
- Unknown keys are errors, so a typo in a deployment file is reported instead of being ignored.
- Mappings may list several old values for one new value, so one profile can bring a model from
  any other environment.

**Typed values.** Each value is checked against the parameter's `Type`, or the type of its current
value when it has none:

| Type | Value | M |
|---|---|---|
| Text | a string | `"…"` |
| Number | a number | `2026`, `-2.5` |
| Logical | true/false | `true` |
| Date | `YYYY-MM-DD` | `#date(2026, 1, 1)` |
| DateTime | `YYYY-MM-DDTHH:MM:SS` | `#datetime(…)` |
| Time | `HH:MM:SS` | `#time(…)` |
| Any | a string, a number or true/false | as above |

A parameter with a `List` of allowed values only accepts one of them. The whole profile is refused,
with every problem listed, when it names an unknown profile or a name that is not a parameter (a
case slip gets a "did you mean"), or when a value has the wrong type or is not allowed.

**`env.apply {profile, env_file?, dry_run?}`** sets the parameter values. Values set by the profile
win over mappings. It then rewrites literals as `sources.rebase` does, in one edit. Applying the
same profile again changes nothing.

**`env.check {profile, env_file?}`** is read-only. For each parameter it reports the current value,
the target and one status: `same`, `differs`, `covered` (by a mapping) or `not_covered`, plus how
the sources use it. For each source argument it reports `same`, `differs`, `covered` (through a
parameter the profile covers), `not_covered` or `computed`. `ok` is true when the profile has no
error and covers every literal source and every parameter the sources use. Computed arguments
only add a warning, because no profile can rewrite them.

**`env.list {env_file?}`** is read-only: the file's profiles as `{env_file, count, profiles:
[{name, description, parameters}]}` (`parameters`: how many values the profile sets). The app's
parameters forms run it to offer the profiles in a picker.

**Hostile files.** The file is read by the same bounded YAML reader as `bicraft.tests.yaml`
([ci-tests.md](ci-tests.md)):

- at most 4 MiB, 64 levels and 200,000 values;
- no anchors or aliases, no tags other than the core ones, one document only;
- duplicate keys are errors.

## Command line and CI

```
bicraft-cli sources PATH
bicraft-cli sources rebase PATH --from C:\Data\Finance --param RootFolder [--to D:\fin] [--dry-run]
bicraft-cli sources rebase PATH --server dev-sql=prod-sql --host adb-1=adb-2 [--url OLD=NEW] ...
bicraft-cli env check PATH prod [--env-file envs.yaml]
bicraft-cli env apply PATH prod [--env-file envs.yaml] [--dry-run]
```

Output is JSON. `rebase` and `apply` save the file unless `--dry-run`. Exit codes:

- 0: done, or the profile covers the model (`check`);
- 1: `check` found gaps or errors, or the edit or the save failed;
- 2: usage error, unreadable file or profile file, unknown profile, invalid profile.

A deployment pipeline can run `bicraft-cli env check` as a gate and then `bicraft-cli env apply` on a copy
of the project.

## MCP and server mode

The MCP tools are `list_sources`, `rebase_sources`, `apply_env` and `check_env` ([mcp.md](mcp.md)).
`sources.list`, `env.list` and `env.check` are read-only, so readers can run them. `sources.rebase` and
`env.apply` are edits. In server mode, `from`, `to` and the mappings are text in M, not files of
the server, so they are not resolved. `env_file` is a path parameter: it is resolved under the
workspace root, and the default file next to the project must be inside the root too.

## Limits

- Only literals are rewritten. A path built by a function, such as `Text.Combine` or a custom
  function, is listed as computed.
- A step name bound twice in one query (nested `let`s) is not resolved.
- Mappings change the M text only. A PBIP's cached data (`.pbi/cache.abf`) still holds the old
  rows until Desktop or `model.refresh` refreshes them.
