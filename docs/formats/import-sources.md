# Other files `file.open` reads (M11.36), and `file.redact` (M11.37)

Sources: Microsoft Learn ("Create Power BI data source files (.pbids)", "Power Query template",
"Dataflow definition" of the Fabric REST API item definitions, "The metadata file (model.json)
for the Common Data Model", "Power Query M language specification": *Lexical structure* and
*Sections*); the public VertiPaq Analyzer project (sql-bi/VertiPaq-Analyzer, MIT) for the
`DaxModel.json` serialization BIcraft already writes (`crates/engine/src/vpax.rs`,
[vertipaq-stats.md](vertipaq-stats.md)); a blog description of the `.pqt` ZIP (RADACAD, "Power
Query Template: One Giant Leap") for the part name `MashupDocument.pq`. The fixtures are written
by hand (`crates/engine/tests/fixtures/import-sources/SOURCE.txt`). The idea of these readers
and of redaction comes from DashCraft (MIT OR Apache-2.0, commit 0958fd82); the code is ours.

Each file opens as a **new, unsaved model** (as `model.new` makes one; `file.save_as` saves it),
so `objects.import` takes it as a source too. `crates/engine/src/readers.rs`.

## `.vpax` (VertiPaq Analyzer)

A ZIP (OPC) with `DaxModel.json` and, optionally, `Model.bim` (TMSL). The model comes from
`Model.bim` when present, else from `DaxModel.json`: `Tables.$values[]` (`TableName`, `IsHidden`,
`Description`, `TableExpression` for calculated tables), their `Columns` (`ColumnName`,
`DataType` in TOM names, `ColumnType` — `RowNumber` columns are left out —, `ColumnExpression`,
`SourceColumn`, `FormatString`, `DisplayFolder`, `SortByColumnName`) and `Measures`
(`MeasureName`, `MeasureExpression`, …), and `Relationships` whose `FromColumn` / `ToColumn`
are Json.NET references (`{"$ref": id}`) to the column objects (`$id`). Names and notes are read
as plain strings or as objects holding `Name` / `Note` / `Expression`. Arrays are read wrapped
(`{"$id", "$values": […]}`) or plain. Without `Model.bim`, tables have no partition (no query).
A VPAX holds no data. The storage statistics are returned by `file.open` as `statistics`:
per table `rows` (`RowsCount`), `columns_size` (sum of the columns' `TotalSize`),
`user_hierarchies_size`, `total_size`, `referential_integrity_violations`, per column
`cardinality`, `data_size`, `dictionary_size`, `hierarchies_size`, `total_size`, `encoding`;
per relationship `used_size_from`, `used_size_to`, `missing_keys`, `invalid_rows`.

## `.pbids` (Power BI data source file)

JSON: `{"version": "0.1", "connections": [{"details": {"protocol", "address": {…},
"authentication", "query"}, "options", "mode"}]}`. Each connection becomes one import (or, with
`"mode": "DirectQuery"`, DirectQuery) table named after the file, with the query Desktop's Get
data starts from:

| `protocol` | `address` | Query |
|---|---|---|
| `tds` | `server`, `database?` (+ `details.query`) | `Sql.Database(server, database[, [Query = …]])`, `Sql.Databases(server)` |
| `analysis-services` | `server`, `database?` | `AnalysisServices.Database(…)` / `.Databases(server)` |
| `folder` | `path` | `Folder.Files(path)` |
| `file` | `path` | by extension: `Csv.Document` + `Table.PromoteHeaders` (csv, txt, tsv), `Excel.Workbook`, `Json.Document`, `Xml.Tables`, `Parquet.Document`, else `File.Contents` |
| `odata` | `url` | `OData.Feed(url)` |
| `http` | `url` | `Web.Contents(url)` |
| `sharepoint-list` | `url` | `SharePoint.Tables(url, [ApiVersion = 15])` |

Other protocols (`powerbi-dataflows`, …) are refused with the list. The table has no columns
until its query runs (Desktop shows the Navigator at this point; BIcraft does not).

## `.pq`, `.m` and `.pqt` (Power Query)

M text is either a **section document** (`section Name; [attributes] shared Member = expr; …`,
the M specification's *Sections*), split into members by a token scan that counts brackets,
braces and parentheses (`bicraft_data::m::scan::section_members`), or **one expression**,
named after the file. A `.pqt` is a ZIP whose `MashupDocument.pq` (else any `.pq`, else any
`.m` part) holds the section document; a JSON part holding `queriesMetadata` (at any depth up to
4) says which queries load (`loadEnabled`), with `documentLocale` / `culture`. Loaded queries
become import tables (no columns until they run: `table.import` with `replace: true` loads them), the others shared expressions. Without
metadata, every query loads except functions (`(x) => …`, `each …`) and parameters
(`meta [IsParameterQuery = true …]`). Connections and credentials are not in a template (Learn).

## Dataflow Gen2 and the CDM `model.json`

A Dataflow Gen2 definition has the parts `mashup.pq` (the section document) and
`queryMetadata.json` (`name`, `documentLocale`, `queriesMetadata: {Query: {queryId,
queryName, loadEnabled}}`, …). Opening either file reads the other beside it; the model is named
after `name`.

A CDM `model.json` (`name`, `culture`, `entities`, `relationships`, and for a Power BI dataflow
`pbi:mashup: {document, queriesMetadata}`): each `LocalEntity` becomes a table with its
`attributes` as columns (`dataType`: string, int64, double, decimal, dateTime, dateTimeOffset,
date, time, boolean; others as text), its query the mashup member of the same name, else its
first partition's `location` read as a CSV without header row (`Csv.Document(Web.Contents(…),
[Columns = n, …])`). The other mashup queries follow their `loadEnabled`.
`SingleKeyRelationship`s (`fromAttribute` → `toAttribute`, entity and attribute names) become
relationships when both columns exist.

## Hostile input

Text files are read up to 64 MiB, ZIPs up to 512 MiB with BIcraft's decompression caps (512 MiB
a part, 1 GiB in all: a ZIP bomb is refused part by part, a forged size is refused before any
allocation). JSON nesting is capped at 128 levels by the parser, M nesting by
`bicraft_data::Limits::max_depth`; at most 10 000 connections, queries, entities, tables,
columns or relationships are read. No reader parses XML, so DTDs, external entities and entity
expansion in a `[Content_Types].xml` are never resolved. Fuzz smoke:
`crates/engine/tests/fuzz_smoke.rs` (`import_readers`).

## `file.redact`

`crates/engine/src/redact.rs`. Collected: the text arguments of data source functions (any call
outside the M standard library's namespaces), the values of credential record fields
(`Password`, `Pwd`, `ApiKey`, `Key`, `Token`, `Authorization`, `User`, `UserName`, `Uid`, …),
the value of a parameter passed to a source function, M literals that read as a URL, an
absolute path (`C:\…`, `\\server\…`, `/a/b`) or a connection string (`server=`, `data source=`,
`pwd=`… with `;`), the model's `connectionString`, `server`, `database`, `url`, `path`,
`account`, `domain` and credential properties, and strings of the model, report and other
package text parts that read as a URL or an absolute path, or hold a URL inside a longer text
(a text box sentence) (`$schema` URLs kept). Placeholders,
numbered in the order found: `server_N.redacted.invalid`, `database_N`, `C:\redacted\path_N.ext`
(the extension kept), `https://redacted.invalid/url_N`, `connection_N`, `credential_N`, `text_N`;
with `data`, stored text values become `value_N` (and the report's literals of them). Values
shorter than 4 characters are replaced only as whole strings and are not searched for in the
output. `SecurityBindings` is left out. A PBIX keeps its stored rows (a table whose query was
redacted is written from its stored columns, not run again); a PBIP copy is a new project
without its `.pbi` folder. The written copy is opened again and searched byte by byte (each
package part and the decompressed `DataModel`, UTF-8 and UTF-16LE); a survivor deletes it.
