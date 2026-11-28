# Semantic diff

`bicraft-cli diff` compares two Power BI files of any format and reports what changed in the
model and the report. A PBIX is compared with a PBIP project, a PBIT, a TMSL or TMDL project,
or a `model.bim`.

- **PBIX and PBIT** are ZIP archives, so git cannot diff them.
- **PBIP, TMDL and PBIR** text diffs are noisy. Arrays get reordered, folder names are
  generated, `$schema` and version numbers change, and tab orders churn.
- **Across formats**, PBIX against PBIP or BIM against TMDL, a text diff is impossible.

The diff reads both files into the format-neutral model and report (`bicraft-model`,
`bicraft-report`) and compares objects there (crate `bicraft-diff`, engine command
`file.diff`).

## Use it

```sh
bicraft-cli diff Sales.pbix Sales/Sales.pbip              # text (the default)
bicraft-cli diff old.pbix new.pbix --markdown > comment.md    # for a pull-request comment
bicraft-cli diff old.pbit new.pbit --json                 # structured
bicraft-cli diff a.pbix b.pbix --model-only --ignore-whitespace --exit-code   # CI gate
bicraft-cli diff --git main HEAD Sales.pbip               # a project between two revisions
bicraft-cli diff --git HEAD Sales.pbip                    # last commit vs working tree
```

| Option | Effect |
|---|---|
| `--text` (default), `--markdown`, `--json` | Output form |
| `--model-only`, `--report-only` | Compare one half (`scope`) |
| `--ignore-formatting` | Leave out visual, page and report formatting (`objects`, `visualContainerObjects`), theme and theme-file changes |
| `--ignore-positions` | Leave out visual positions, sizes, z-order and tab order, and page sizes |
| `--ignore-whitespace` | Leave out DAX and M changes that only touch whitespace, comments or (in DAX) name case |
| `--exit-code` | Exit 1 when the files differ, as `git diff --exit-code` does. Errors exit 2 |

Over MCP, the tool `diff_files` (command `file.diff`) takes `{a, b, scope?, ignore_formatting?,
ignore_positions?, ignore_whitespace?, format?}`. `format: "text"` or `"markdown"` adds the
rendering next to the JSON. `a` or `b` may be `null` for "nothing", which covers a file added
or deleted. The command only reads, so a reader may run it (`docs/auth.md`). In server mode both
paths follow the workspace sandbox (`docs/server-mode.md`), and the answer shows paths relative
to the workspace.

## What is compared

**Model**

- Tables: description, hidden, annotations, other properties.
- Columns: data type, expression, source column, format string, display folder, summarize-by,
  sort-by column, data category, description, hidden. Row-number columns are skipped.
- Measures: expression, format string, display folder, description, hidden.
- Partitions and M queries: mode, source type, expression.
- Hierarchies, calculation groups and their items.
- Relationships: endpoints, active flag, cross-filter direction, cardinalities.
- Roles: model permission, the RLS filter of each table, members.
- Perspectives, cultures (translations, linguistic metadata), data sources, shared
  expressions, and parameters (an M expression with `IsParameterQuery=true`).
- Model properties: culture, compatibility level, annotations, and the other properties.

**Report**

- Pages: display name, size, display option, visibility, formatting, filters.
- Visuals: type, position and size, z-order and tab order, group, hidden flag, the fields
  bound to each role (added, removed, reordered), filters, sorting and other query
  properties, and formatting.
- Report-level: page order, filters, theme, settings and other properties, formatting.
- Bookmarks.
- Static resources. A JSON resource such as a custom theme is compared property by property.

**Data.** Two PBIX files also compare their `DataModel` parts by the size and CRC-32 stored
in the ZIP directory, without decompressing them. The `DataModel` holds the metadata too, so
a changed stamp is listed as a change only when the metadata is the same, which means a
refresh. The JSON always carries `data: {a, b, same}` for two PBIX files.

## Identity, renames and moves

Objects are matched in four steps. Each step only looks at what is still unmatched:

1. Same name.
2. Same lineage tag.
3. Same name ignoring case.
4. Same content. Content means a measure's DAX after normalisation, a column's source column
   or expression plus its type, a table's column names plus its M source, a relationship's
   endpoints, a visual's type, fields and position, a page's display name, or a filter's field.

A tag or content match counts only when it is unique on both sides. A pair matched after step
1 is reported as `renamed`, with `from` (the old id) and `how` (`lineage_tag`, `case`,
`content`).

Measures are unique in a model, so a measure found in another table is reported as `moved`. A
visual found on another page is also `moved`.

Relationships get generated names, so a different name alone is never reported. A renamed
table or column does not make its relationships look changed: `a`'s endpoints are translated
to `b`'s names first.

Pages are matched by id, then by display name. A regenerated page folder is therefore not a
change. Visuals are matched by id, then by content within the page, then by id across pages.

## Noise that is never reported

- `$schema` URLs, `version.json`, and `reportVersionAtImport`.
- The `PBIDesktopVersion` and `PBI_ProTooling` annotations.
- Timestamps: `modifiedTime`, `structureModifiedTime`, `refreshedTime`, `lastUpdate`,
  `lastSchemaUpdate`, `lastProcessed`, `createdTimestamp`.
- The database name and id.
- The active page.
- The order of tables, columns, measures, relationships, roles, pages, visuals, filters and
  named JSON arrays.
- Generated partition and relationship names.
- CRLF against LF.
- `summarizeBy: default` against no value.
- Properties only a legacy layout can hold (PBIR has no place for them), when the other side
  is PBIR. Between two legacy reports they are compared.
- An empty theme collection against none.

DAX changes that only touch whitespace, comments, quoting (`'Sales'` against `Sales`) or the
case of names and keywords are reported with `cosmetic: true`. M changes that only touch
whitespace or comments are reported the same way. `--ignore-whitespace` drops both.

## Output

The JSON form:

```json
{
  "a": {"path": "old.pbix", "kind": "pbix", "report_format": "legacy"},
  "b": {"path": "new/Sales.pbip", "kind": "pbip", "report_format": "pbir"},
  "identical": false,
  "summary": {"added": 1, "removed": 0, "changed": 2, "renamed": 1, "moved": 0},
  "changes": [
    {"area": "model", "object": "measure", "id": "measure 'Sales'[Total]", "change": "changed",
     "properties": [{"property": "expression", "before": "SUM(Sales[Amount])",
                     "after": "SUMX(Sales, Sales[Qty] * Sales[Price])"}]},
    {"area": "model", "object": "measure", "id": "measure 'Sales'[Revenue]", "change": "renamed",
     "from": "measure 'Sales'[Sales Amount]", "how": "lineage_tag"},
    {"area": "report", "object": "visual", "id": "visual 3f2a on page \"Overview\"",
     "change": "changed", "label": "clusteredColumnChart (Sales[Total], Date[Year])",
     "properties": [{"property": "position", "before": "10,20 300x200", "after": "10,40 300x200"}],
     "fields": [{"role": "Y", "added": ["Sales[Margin]"]}],
     "formatting": [{"property": "legend.show", "before": "true", "after": "false"}]}
  ]
}
```

- `before` and `after` on a change summarise a removed or an added object.
- On a property, a missing `before` means the property was added, and a missing `after`
  means it was removed.
- `notes` lists what was not compared. For example, when one side's report cannot be read,
  reports are not compared at all. With `--report-only` (`scope: "report"`) an unreadable
  report is an error instead.
- Changes are sorted by area, then section (tables with their members, relationships, roles,
  …; pages with their visuals), then id. Only byte comparisons are used, so the JSON, text and
  Markdown are byte-identical on Windows and Linux.

The text form, as the CLI prints it:

```text
a: old.pbix
b: new/Sales.pbip

Model
  ~ measure 'Sales'[Total]
      expression:
        - SUM(Sales[Amount])
        + SUMX(Sales, Sales[Qty] * Sales[Price])
  > measure 'Sales'[Revenue] (renamed from measure 'Sales'[Sales Amount], same lineage tag)
  ~ measure 'Sales'[Avg]
      expression: whitespace, comments or case only

Report
  ~ visual 3f2a on page "Overview" [clusteredColumnChart (Sales[Total], Date[Year])]
      position: "10,20 300x200" -> "10,40 300x200"
      fields Y: + Sales[Margin]
      formatting legend.show: "true" -> "false"

1 added, 0 removed, 3 changed, 1 renamed, 0 moved
```

The Markdown form has a summary table, one bullet per change, and a `diff` code block for
each expression. Its fences are longer than any backtick run in the content.

## Git

### PBIX and PBIT in `git diff` (textconv)

`bicraft-cli diff --textconv FILE` (command `file.textconv`) prints a canonical text form of one
file. Objects are sorted by name, there is one property per line, expressions are indented
blocks, and noise is left out. Git then diffs these texts line by line:

```gitattributes
# .gitattributes
*.pbix diff=bicraft
*.pbit diff=bicraft
```

```sh
git config diff.bicraft.textconv "bicraft-cli diff --textconv"
git config diff.bicraft.cachetextconv true     # optional: cache per blob
```

### A semantic diff driver

The textconv text is still a line diff. To get the semantic diff, with renames and cosmetic
changes recognised, as git's external diff:

```sh
git config diff.bicraft.command "bicraft-cli diff"
```

Git calls it with its 7 arguments (`path old-file old-hex old-mode new-file new-hex new-mode`).
An added or deleted file shows as everything added or removed. Differences exit 0, so
`git diff` goes on to the next file. Use `git diff --ext-diff` or `git log -p --ext-diff`
where git does not use external drivers by default. When both `textconv` and `command` are set,
`command` wins.

### difftool

```sh
git config difftool.bicraft.cmd 'bicraft-cli diff "$LOCAL" "$REMOTE"'
git difftool -t bicraft -- '*.pbix'
```

This works for `.pbix`, `.pbit` and `model.bim`. Git's temporary copies keep the file name.
A file whose extension git dropped is still recognised as a package by its ZIP signature.

### A PBIP project between revisions

```sh
bicraft-cli diff --git REV1 REV2 path/to/Sales.pbip
bicraft-cli diff --git REV1 path/to/Sales.pbip  # REV1 against the working tree
```

`PATH` can be a `.pbip` file, a project folder, or a `.pbix`/`.pbit`/`.bim` in the
repository. For a `.pbip` file, its `<Name>.SemanticModel` and `<Name>.Report` siblings are
copied too. Files under `.pbi/` are never copied.

The CLI only reads the repository. It runs `git rev-parse`, `git cat-file -t`, `git ls-tree`
and `git cat-file --batch`, with `GIT_OPTIONAL_LOCKS=0`. Blobs are streamed into a temporary
folder that is removed afterwards. The index, worktree, refs and config are never touched.

Revisions starting with `-` are refused. Unsafe file names, links and submodules are refused
or skipped. A revision is capped at 100,000 files and 8 GiB.

`--git` and the external-diff form run git. They exist only in the CLI: no engine command
spawns git, so they cannot be reached over MCP or HTTP.

## Bounded work on hostile input

- **Metadata only.** A PBIX is opened with its `DataModel` left in the file. Only
  `metadata.sqlitedb` is decompressed, as a stream. A PBIP's `.pbi/cache.abf` is never read.
- **Generic JSON comparisons** (the properties formats keep that bicraft does not model):
  - at most 48 levels deep;
  - at most 100 properties per object, then a `…` entry counting the rest;
  - values over 4 KiB are summarised.
- **Changes.** At most 20,000 changes, then a note.
- **Expressions.** Normalisation runs only up to 4 MiB. Longer expressions are compared as
  text.
- **Line diffs** (Myers):
  - the common prefix and suffix are stripped first;
  - at most 50,000 lines a side and 1,000 edits;
  - past either limit, the middle is shown as removed then added;
  - the backtracking trace is O(edits²).
  The text and Markdown forms show at most 400 lines per block and 30 formatting entries per
  object.
- **Tests.** The crate tests cover deep JSON, a 1 MB expression, thousands of objects, and
  duplicate and empty names. The engine tests cover empty, truncated and corrupted packages.
