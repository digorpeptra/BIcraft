# Table data sources (as implemented in `crates/data`)

Status: implemented, 2026-10-13 (milestone M5.5). The M evaluator was added on 2026-10-14 (G21)
and wired into the engine the same day (*In the engine*). The Databricks connector, query folding
and BIcraft's own refresh were added on 2026-10-23 (*Connectors*, *Refresh by BIcraft*).
These sources feed the VertiPaq encoder ([`vertipaq.md`](vertipaq.md)), so a PBIX can be
compiled **with** data, without Power BI Desktop.

**Sources.** Public documentation only:

- RFC 4180 (CSV)
- the Power Query M formula language specification and function reference
- the DAX reference (`DATATABLE`, table constructors, `ROW`, `CALENDAR`, `DATE`, `TIME`)

Behaviour the M documentation leaves open was settled by comparing results with Power BI Desktop
(see *Desktop agreement*).

BIcraft reads:

- CSV
- partition sources written in M, through a small sandboxed evaluator: literals, Desktop's
  "Enter data", the common transforms, and local CSV/JSON/Excel files and folders under an
  explicit allow-list of roots
- DAX literal tables; other DAX calculated tables are evaluated by the engine's DAX evaluator
  (*DAX calculated tables* below)

An external connector (`Sql.Database`, `Web.Contents`, `OData.Feed`, `Python.Execute`, …) is
not run unless the caller explicitly allows its family (`SourceOptions::connectors`; only
Databricks exists, *Connectors*): otherwise it is `Error::ExternalSource` ("external source, not
evaluated"). Anything else
BIcraft cannot evaluate is `Error::UnsupportedSource`, with a message naming the construct: an
unsupported function, or a DAX column reference in `bicraft-data`.

## API (`bicraft-data`, layer L2)

| Item | Purpose |
|---|---|
| `TableData` / `ColumnData { name, data_type, values: Vec<CellValue> }` | Row-aligned typed columns. `CellValue` is the VertiPaq crate's value type, so the encoder consumes it directly. |
| `TableData::new(columns)` | Validates: equal lengths, non-empty names unique case-insensitively, every value fits its column type. |
| `csv::read_csv(bytes, &CsvOptions)` / `csv::write_csv(&TableData, &CsvWriteOptions)` | CSV in and out. |
| `m::load_m(expr, &Limits)` | Evaluates an M expression (no file access). |
| `m::load_m_with(expr, &SourceOptions)` | The same, with a file root, culture, shared queries/parameters and error mode. |
| `dax::load_dax(expr, &Limits)` | DAX literal tables. |
| `conform(data, &[Column])` | Maps loaded data onto a model table's declared columns and types. |
| `load_partition(&Partition, &[Column])` | `source_type` `m` → `load_m`, `calculated` → `load_dax`, then `conform`. Other source types → `UnsupportedSource`. |
| `load_partition_opts(&Partition, &[Column], &SourceOptions)` | `load_partition` with `SourceOptions` (M only; DAX ignores them). |

### Value representation

| Model `DataType` | `CellValue` |
|---|---|
| `String` | `Text` |
| `Int64` | `Int` |
| `Double` | `Real` |
| `Decimal` (fixed decimal, "currency") | `Decimal(value × 10 000)` |
| `DateTime` | `DateTime(days since 1899-12-30)` — continuous: the fraction is the time of day, also for negative days |
| `Boolean` | `Bool` |
| `Unknown`, `Variant` | any |
| `Binary` | blanks only |

`Null` (blank) fits every type. Date-only values have a zero fraction; a time-only value is a
fraction on day 0 (1899-12-30), which is how Power BI stores `time` columns.

## CSV

- **Encoding.** UTF-8 (BOM stripped); UTF-16LE/BE when the file starts with a BOM (Excel "Unicode
  text"). Invalid text is `Error::Syntax` with the line.
- **Records (RFC 4180).** Fields may be quoted; `""` inside quotes is a quote; quoted fields may
  contain delimiters and line breaks. Line endings CRLF, LF and lone CR are all accepted. A quote
  inside an unquoted field is kept literally; text after a closing quote (`"a"b`) and an
  unterminated quote are syntax errors.
- **Delimiter.** Given explicitly, or auto-detected among tab, `;` and `,`: the first 20 records
  are split with each candidate, and the candidate that yields more than one field, consistently,
  with the most fields wins (ties: tab, then `;`, then `,`); fallback `,`.
- **Header.** `has_header` (default on) takes names from the first record (trimmed; an empty name
  becomes `ColumnN`). Without a header, columns are `Column1…N`. Duplicate names (case-insensitive)
  are an error.
- **Shape.** Every record must have as many fields as the first one (`Error::RowLength`). Blank
  lines are skipped, except in single-column files, where they are blank values.
- **Blanks.** An unquoted empty field is `Null`. A quoted empty field `""` is the empty string in
  text columns and `Null` elsewhere.
- **Types.** `column_types` sets types by column name (case-insensitive); a cell that does not
  parse is `Error::Value { row, column }`. Other columns are inferred (unless `infer_types` is
  off, which reads them as text): the first of **int64 → double → dateTime → boolean** that
  every non-empty cell parses as, else **string**; an all-empty column is string. Fixed decimal
  is never inferred (ask for it explicitly).

### Culture

`Culture { decimal_separator, date_order }`; `Culture::from_name("fr-FR")` maps a culture name by
language (approximation: `en-US` → `.`/MDY, other `en-*` → `.`/DMY; fr, de, es, it, pt, nl, …
→ `,`/DMY; hu, lt, lv → `,`/YMD; ja, zh, ko → `.`/YMD; unknown → invariant). The invariant
culture is `.` and month/day/year.

| Kind | Accepted text |
|---|---|
| int64 | optional sign, ASCII digits, fits in i64 (`007` is 7) |
| double | optional sign, digits with at most one culture decimal separator, optional exponent (`1,5e3` in fr-FR). No thousands separators. `NaN`/`inf` only with an explicit type. |
| decimal | as double, rounded half away from zero to 4 places, range-checked |
| dateTime | ISO 8601 first: `2026-01-05`, `2026-01-05T13:45[:00[.123]]` (`T` or space), optional `Z` or `±hh:mm` (converted to UTC). Then the culture short date: `d/m/yyyy` (DMY), `m/d/yyyy` (MDY) or `yyyy/m/d` (YMD), separators `/`, `-` or `.`, 4-digit years, optional ` H:MM[:SS[.fff]]` and ` AM`/` PM`. Millisecond precision. |
| boolean | `true` / `false`, any case |

With fr-FR, `1,5` is a number and `1.5` is text; a `,`-delimited fr-FR file must quote such
numbers, which is why fr-FR exports use `;`.

### Writer

`write_csv` quotes a field when it contains the delimiter, a quote, CR or LF (and the empty
string, written `""`); blanks are empty. Numbers use the culture's decimal separator; doubles are
written with full round-trip precision (`1.0`, `1e300`); decimals minimally (`12.34`); dates as
ISO 8601 (`2026-01-05`, `2026-01-05T12:00:00[.fff]`); booleans `true`/`false`. Reading the output
back with the same delimiter, culture and column types reproduces the table (property-tested).

## Power Query (M)

`m::load_m(expr, &Limits)` and `m::load_m_with(expr, &SourceOptions)` evaluate a partition's M
expression with a small interpreter (`crates/data/src/m/`). It covers the queries agents write and
the ones Desktop generates for local data.

**Sources and checks.**

- **Language.** The interpreter follows the public *Power Query M formula language
  specification*: lexical grammar, expressions, values, operators, `let`, `each`, `try` and
  `meta`.
- **Library.** It follows the public M function reference.
- **Desktop.** Behaviour the reference leaves open was settled by comparing results with Power BI
  Desktop 2.158.1304.0, row by row (see *Desktop agreement*).

### Options (`SourceOptions`)

| Field | Default | Meaning |
|---|---|---|
| `limits` | `Limits::default()` | Caps (see *Hostile input*). |
| `file_roots` | empty | The allow-list of folders `File.Contents`, `Folder.Files` and `Folder.Contents` may read (recursively). Empty disables file access. |
| `culture` | `None` (`en-US`) | Culture for conversions without an explicit culture: the model's `culture` / `sourceQueryCulture`. |
| `shared` | empty | Named M expressions the partition may reference: other queries and parameters (a model's `expressions`), as `(name, M text)`. |
| `connectors` | empty | The connectors the evaluation may run (*Connectors*); empty: none. |
| `clock_utc_ms` | `None` | What `DateTimeZone.UtcNow()` / `FixedUtcNow()` return (ms since 1899-12-30 UTC); `None`: unsupported. |
| `clock_local_offset_minutes` | `None` | Local time minus UTC (minutes) for `DateTime.LocalNow()`, `DateTimeZone.LocalNow()` and their `Fixed` forms, with `clock_utc_ms`; `None`: unsupported. The engine passes the process's local clock at the evaluation's instant (below, "Clock"). |
| `strict_errors` | `false` | When set, a cell holding an M error is `Error::Value { row, column }`. By default it loads as a blank, as Desktop does whether or not the model sets `returnErrorValuesAsNull`. |

`load_partition_opts(&Partition, &[Column], &SourceOptions)` is `load_partition` with options.
`load_partition` and `load_partition_with` keep their signatures and allow no file access.

### Language

Supported:

- `let`/`in`, with lazy, mutually recursive bindings. A step that is never used is never evaluated.
- Records `[a = 1, b = a + 1]`: a field can see the other fields.
- Lists `{1, 2}`, ranges `{1..10}`.
- Functions `(x, optional y as number) as number => …` and `each … _ …`. `[Col]` is `_[Col]`, and
  `@name` refers to the binding itself.
- `if`/`then`/`else`, `try` (alone, `otherwise`, `catch (e) =>`), `error` (text or record).
- `meta`: evaluated, then ignored.
- Field access `x[f]`, `x[f]?`, projection `x[[a], [b]]`.
- Item access `x{0}`, `x{0}?`, and `table{[Key = value]}`.
- Operators: arithmetic, `&` (text, lists, records, tables, date & time), comparisons, `and`, `or`,
  `not`, `??`, `is`, `as`.
- Literals: numbers (decimal, exponent, hex), text with `#(…)` escapes, `#date`, `#datetime`,
  `#datetimezone`, `#time`, `#duration`, `#binary`, `#table` (header as a `type table`, a list of
  names, or a column count), `#infinity`, `#nan`.
- Type expressions: `type text`, `type nullable number`, `type table [A = text, B = Int64.Type]`,
  `type {number}`, `type [a = text]`, function types (parsed only).

Not supported, each an `UnsupportedSource`:

- sections, `#shared`, `#sections`, `Expression.Evaluate`
- two-argument comparer functions in `Table.Sort` and `List.Sort`
- `JoinKind.RightOuter`, `FullOuter` and `RightAnti`
- functions that read the clock (`Date.IsInCurrentYear`, …), except `DateTimeZone.UtcNow`,
  `DateTimeZone.FixedUtcNow` when the caller gives the time (`SourceOptions::clock_utc_ms`), and
  `DateTime.LocalNow`, `DateTimeZone.LocalNow` and their `Fixed` forms when it also gives the
  offset (`clock_local_offset_minutes`)
- random functions (`Number.Random`, `Text.NewGuid`)
- conversions to `datetimezone` that need the machine's time zone
- every connector (`Sql.Database`, `Web.Contents`, `SharePoint.*`, …), except
  `DatabricksMultiCloud.Catalogs` / `Databricks.Catalogs` when the Databricks connector is allowed
  and `Sql.Database` / `Sql.Databases` when the SQL Server one is ([tds.md](tds.md)), `Web.Contents`
  when the Web one is and `OData.Feed` when the OData one is ([odata.md](odata.md)), and
  `SharePoint.Files` / `SharePoint.Contents` / `SharePoint.Tables` when the SharePoint one is
  ([sharepoint.md](sharepoint.md))

How unknown names are reported:

- An unknown dotted name: ``M function `X` is not supported by BIcraft``.
- An unknown plain name: a reference to another query or parameter that was not supplied in
  `shared`.

### Evaluation model

- **Names.** A name is looked up in the innermost scope first: the fields of the record being
  built (so `[Threshold = 3, A = Threshold]` gives `A = 3`), then each enclosing `let`, record or
  function, then the shared queries and parameters (`SourceOptions::shared`, evaluated once each,
  lazily, in any order, and allowed to reference each other), then the library. Another record's
  fields are never in scope. A parameter's `meta` record does not change its value: `6 meta
  [IsParameterQuery=true, Type="Number"]` is 6 and `"6"` stays text. Pinned by
  `shared_expressions_in_record_initializers` (crates/data) and
  `import_m_reads_a_parameter_in_a_record_initializer` and `query_preview_reads_a_shared_expression_in_a_record` (crates/engine).
- **Errors.** An M error (`error`, a failed conversion, a missing field) can be caught with
  `try`. Inside a list, a record field or a table cell, the error is stored and raised only when
  that value is read.
  - These are *fatal*: an unsupported function, a cap, a file outside the root. `try` does not
    catch them, so an unsupported source never turns silently into fallback data.
  - An M error that reaches the top is `Error::Evaluation { reason, message }`.
  - Error cells of the result load as blanks, unless `strict_errors` is set.
- **Numbers** are doubles or exact decimals (see *Numbers* below). Facets come from the
  library type names (`Int64.Type`, `Currency.Type`, …).
  - Conversions to an integer facet (`Table.TransformColumnTypes`, `Int64.From`) round half to
    even, exactly: 2.5 → 2, 3.5 → 4, −2.5 → −2, `9007199254740993` stays itself.
  - `Currency.Type` keeps 4 decimal places (the decimal rounded half to even).
- **Text** positions and lengths count Unicode scalar values. M counts UTF-16 code units, so the
  two differ only outside the BMP.
- **Comparisons.** Text compares ordinally (by UTF-16 code unit), for the operators and for
  `Table.Sort`/`List.Sort`. Desktop agrees: `10 < 9 < A < Ab < B < Z < _x < a`.
  - `null` sorts first. Sorting is stable.
  - Equality is structural for lists, records and tables. `null = null` is true, and `null < 1` is
    null.
  - Joins and `Table.Group` use M equality: case-sensitive, and `null` keys match.
  - A `Comparer.OrdinalIgnoreCase` criterion is honoured by `Table.Distinct`, `Table.Group` and the
    `List.*` set functions.
- **Dates.** Dates and times are kept in milliseconds; years run 1–9999. Out-of-range arithmetic is
  an M error.

### Numbers

A Power Query number is either a double or a decimal (`Decimal.Type` in the M specification).
BIcraft keeps a decimal in the .NET `System.Decimal` layout (a 96-bit mantissa, 0 to 28 decimal
places, a sign; `crates/data/src/m/decimal.rs`, `Num` in `value.rs`), which gives the results
below. Every rule below was settled by probing
Desktop 2.158.1304.0 (*Desktop agreement*, round 3).

**Where decimals come from.**

| Source | Rule |
|---|---|
| Literals, `Number.FromText`, `Number.From(text)`, `Value.FromText`, text → `type number`, `Json.Document`, `Excel.Workbook` cells | A **decimal** when the text has more than 15 significant digits or more than 15 decimal places and fits a decimal (`123456789012345678`, `0.1234567890123456789`, `1e-16`; digits past 28 places round half to even). Otherwise a **double** (`2.50`, `0.1`, `1e-15`, `1e30`, `1e-29`). |
| Hex literals, `{1..n}` | Integers: a double up to 15 digits, else a decimal. |
| `Decimal.From`, `Decimal.Type` | A decimal. A double converts with 15 significant digits (`1/3` → `0.333333333333333`); NaN, infinities and values beyond ±7.9e28 are an error. |
| `Currency.From`, `Currency.Type` | A decimal rounded half to even to 4 places. |
| `Int64.From` and the other integer types | An exact integer. |
| `Number.Round`, `RoundUp`, `RoundDown`, `RoundTowardZero`, `RoundAwayFromZero` | A decimal, also for doubles within the decimal range (converted with 15 digits). NaN, infinities and larger doubles are returned as they are. |
| `Value.Add`/`Subtract`/`Multiply`/`Divide`, `List.Sum`, `List.Average` with `Precision.Decimal` | Decimal arithmetic. An overflow gives ±∞, a division by zero ±∞ or NaN. |
| Negation, `Number.Abs`, `List.Max`/`Min`, record fields, table cells, `type number`, `Number.From(number)`, the first row of `Table.AddIndexColumn` | The value as it is. |

Everything else computes in **double**: the operators `+ - * /` (M specification),
comparisons and equality (`123456789012345678 = 123456789012345679` is true, and
`List.Distinct` keeps one of them), `Number.Mod`, `IntegerDivide`, `Power`, `Sqrt`,
`List.Sum`/`Average`/`Product`/`Median`/`StandardDeviation` by default, `List.Numbers`.

**Decimal → double.** A decimal read from text converts to the nearest double. A computed
decimal converts as the documented OLE Automation function `VarR8FromDec` does (the mantissa as a
double divided by the double `10^scale`), which reproduces Desktop's results, sometimes one unit
off the nearest double: `Value.Divide(29, 7, Precision.Decimal)` gives `4.1428571428571423`, not `…432`. Desktop loads the same doubles into VertiPaq.

**Double → decimal** (`VarDecFromR8`): rounded to 15 significant digits, trailing zeros
removed: `2.5` → `2.5`, `1.005` → `1.005`, `1e20/3` → `33333333333333300000`.

**Decimal arithmetic** follows .NET: sums and products are exact when they fit, else rounded
half to even; a quotient is exact at the smallest scale that holds it (`2.680 / 2` → `1.340`,
`12.000 / 0.01` → `1200.0`), else rounded at 28-29 significant digits.

**`Number.Round(x, digits, mode)`** on a decimal:

1. `s = x × 10^digits` (decimal).
2. When `s` is exactly halfway (fractional part .5), `mode` decides, and the result keeps the
   scale: `Up` adds 0.5, `Down` subtracts 0.5, `AwayFromZero` moves away from zero, `ToEven`
   (the default) towards the even neighbour, `TowardZero` truncates (scale 0).
3. Otherwise `s ± 0.5` (decimal, so a 29-digit value first rounds to 28-29 digits), truncated.
4. The result is divided by `10^digits` (decimal division, so exact quotients keep the scale).

So the mode only breaks ties, and ties keep a trailing zero: `Number.Round(2.675, 2)` →
`2.680`, `Number.Round(-2.5)` → `-2.0` (loaded as −0.0 from `-0.5`), `Number.Round(2.25, 1)` →
`2.20`, `Number.Round(10/4)` → `2.0`, but `Number.Round(2.674, 2)` → `2.67`,
`Number.Round(2.675, 2, RoundingMode.TowardZero)` → `2.67`, `Number.Round(1250, -2)` → `1200`.
More than 28 digits leaves the value as it is, fewer than −28 gives 0, a fractional `digits`
leaves the value as it is. `RoundUp` and the like scale, take the ceiling (floor, …) and divide
back: `Number.RoundUp(123456789012345678.1)` → `123456789012345679`.

**`List.Average`** in double precision computes `x0 + Σ(x − x0) / n` (deviations from the first
value): `{1, 2, 4}` → `2.333333333333333`, `{-1, 2, 10}` → `3.666666666666667`,
`{-33, 3, 62}` → `10.666666666666664`. These last digits matched 45 Desktop averages.

**Decimals as text.**

- `Text.From` writes every digit of the scale and no exponent: `2.680`, `123456789012345678`,
  `0.0000000000000001`; no sign on zero (`Number.Round(-0.5)` → `0.0`).
- `Number.ToText` formats decimals from their exact digits, rounding half away from zero: `F20`,
  `N0` (`123,456,789,012,345,678`), `E3` (`1.235E+017`), `G5` (`1.2346E+17`), `G` without
  precision (all digits), `D20`, `X` (`1B69B4BA630F34E`), `P` and custom patterns.
- Whole doubles below 2^96 are written out in full (`1152921504606846976`), larger ones in the
  round-trip form (`1E+30`). Infinities are `∞` and `-∞`.
- Doubles in `G`/`E` with fewer than 15 digits round their first 15 significant digits half
  away from zero, like the other formats: `Number.ToText(0.125, "G2")` → `0.13`.

### Conversions and text (as Desktop does them)

**`Table.TransformColumnTypes`, `*.From`, `*.FromText`.** These use the culture argument (text,
or a record with `Culture`), else `SourceOptions::culture`.

- **Numbers.** Surrounding blanks are ignored and group separators are accepted (`1,234.5` in
  en-US). `(2.5)` is −2.5; `NaN` is read, `Infinity` and `0x1F` are not. In fr-FR the group separator is exactly U+202F: a plain or no-break space makes the value
  an error.
- **Dates.** A `date` accepts ISO dates, the culture's short date and month names
  (`Jan 7, 2026`). Text with a time part is an error.
- **Second argument.** `*.FromText(text, "fr-FR")` reads a text second argument as a culture.
  `*.ToText(value, "yyyy-MM")` reads it as a format.

**Default text.** This is what `Text.From` and `*.ToText` without a format produce:

- **Whole numbers** are written in full: `100000000000000000000`.
- **Other doubles** use the .NET Framework round-trip form: 15 significant digits when they
  read back to the same double, else 17. So `0.1 + 0.2` → `0.30000000000000004`, and `1e-5` →
  `1E-05`. Decimals: see *Numbers*.
- **Dates and times.**
  - Dates use the culture's short date (`1/5/2026`, `05/01/2026`).
  - Datetimes add the long time without fractions of a second (`1/5/2026 1:04:05 PM`).
  - `time` values use the short time (`9:05 AM`, `09:05`).
- **Durations** are written as `d.hh:mm:ss[.fffffff]`.

**Formats.**

- **Fixed formats.** `F`, `N`, `P` and custom patterns round half away from zero, after
  limiting the value to 15 significant digits: `Number.ToText(2.5, "F0")` → `3`.
- **Separators.** `N` groups digits with the culture's separator. `P` writes `25,6 %` in fr-FR
  and `25.6%` in en-US.
- **Culture names.** Month and day names, and their abbreviations, are tabulated for en, fr
  (`janv.`, `lun.`), de and es. The other languages use the English names.

**Other behaviour Desktop confirmed.**

| Function | Behaviour |
|---|---|
| `Text.Proper` | A letter that does not follow a letter starts a word: `O'Neil`, `2Nd`. |
| `Text.Middle` | A start beyond the end gives `""`. |
| `Text.BeforeDelimiter`, `Text.AfterDelimiter`, `Text.BetweenDelimiters` | `null` input reads as `""`. |
| `Table.PromoteHeaders` | Text and numbers are promoted. Other scalars are promoted only with `[PromoteAllScalars = true]`. A `null` keeps the column's name. Duplicates get `_1`, `_2` (not compared with Desktop). |
| `List.Sum`, `List.Average`, `List.Min` | Of an empty or all-null list: `null`. |

### Library

| Area | Functions |
|---|---|
| Tables | `#table`, `Table.FromRows`, `FromRecords`, `FromColumns`, `FromList`, `FromValue`, `ToRows`, `ToRecords`, `ToColumns`, `TransformRows`, `Column`, `ColumnNames`, `ColumnCount`, `RowCount`, `IsEmpty`, `First`, `Last`, `FirstValue`, `HasColumns`, `PromoteHeaders`, `DemoteHeaders`, `TransformColumnTypes`, `TransformColumns`, `RenameColumns`, `SelectColumns`, `RemoveColumns`, `ReorderColumns`, `DuplicateColumn`, `PrefixColumns`, `SelectRows`, `AddColumn`, `AddIndexColumn`, `Sort`, `Distinct`, `FirstN`, `LastN`, `Skip`, `RemoveFirstN`, `RemoveLastN`, `Range`, `ReverseRows`, `Repeat`, `InsertRows`, `ReplaceValue`, `ReplaceErrorValues`, `RemoveRowsWithErrors`, `SelectRowsWithErrors`, `Group` (global or local), `NestedJoin` and `Join` (inner, left outer, left anti), `ExpandTableColumn`, `ExpandRecordColumn`, `ExpandListColumn`, `Combine`, `FillDown`, `FillUp`, `Unpivot`, `UnpivotOtherColumns`, `Pivot`, `SplitColumn`, `CombineColumns`, `Transpose`, `Max`, `Min`, `MatchesAnyRows`, `MatchesAllRows`, `Contains`, `PositionOf`; `Buffer`, `StopFolding`, `AddKey` and `ReplaceKeys` return their input |
| Lists | `List.Count`, `NonNullCount`, `IsEmpty`, `Sum`, `Product`, `Average`, `Min`, `Max`, `Median`, `Mode`, `StandardDeviation`, `First`, `Last`, `FirstN`, `LastN`, `Skip`, `RemoveFirstN`, `RemoveLastN`, `Range`, `Reverse`, `Distinct`, `Sort`, `Transform`, `TransformMany`, `Select`, `Contains`, `ContainsAny`, `ContainsAll`, `PositionOf`, `Combine`, `Zip`, `Numbers`, `Dates`, `DateTimes`, `Durations`, `Times`, `Repeat`, `RemoveNulls`, `RemoveItems`, `Difference`, `Intersect`, `Union`, `Accumulate`, `Generate`, `AnyTrue`, `AllTrue`, `MatchesAny`, `MatchesAll`, `Single`, `SingleOrDefault`, `InsertRange`, `ReplaceValue`, `Split`, `Alternate`, `Buffer` |
| Text | `Text.From`, `Upper`, `Lower`, `Proper`, `Trim`, `TrimStart`, `TrimEnd`, `Clean`, `Length`, `Start`, `End`, `Middle`, `Range`, `At`, `Contains`, `StartsWith`, `EndsWith`, `PositionOf`, `PositionOfAny`, `Replace`, `ReplaceRange`, `Insert`, `Remove`, `RemoveRange`, `Select`, `Split`, `SplitAny`, `Combine`, `PadStart`, `PadEnd`, `Repeat`, `Reverse`, `ToList`, `BeforeDelimiter`, `AfterDelimiter`, `BetweenDelimiters`, `Format`, `FromBinary`, `ToBinary`; `Character.FromNumber`, `ToNumber`; `Lines.FromText`, `FromBinary`, `ToText` |
| Numbers | `Number.From`, `FromText`, `ToText` (`G`, `R`, `F`, `N`, `P`, `D`, `E`, `X` and `0`/`#`/`,`/`.`/`%` patterns), `Round` (with `RoundingMode`), `RoundUp`, `RoundDown`, `RoundTowardZero`, `RoundAwayFromZero`, `Abs`, `Sign`, `Sqrt`, `Power`, `Exp`, `Ln`, `Log`, `Log10`, `Mod`, `IntegerDivide`, `IsEven`, `IsOdd`, `IsNaN`; `Int64.From` and the `Int32`, `Int16`, `Int8`, `Byte`, `Double`, `Single`, `Decimal`, `Currency` and `Percentage` variants; `Logical.From`, `FromText`, `ToText`; `Value.Equals`, `Compare`; `Precision.Double`/`Decimal` in `Value.Add`/`Subtract`/`Multiply`/`Divide`, `List.Sum`, `List.Average` |
| Dates | `Date.From`, `FromText`, `ToText`, `Year`, `Month`, `Day`, `DayOfWeek`, `DayOfWeekName`, `DayOfYear`, `DaysInMonth`, `WeekOfYear`, `WeekOfMonth`, `QuarterOfYear`, `MonthName`, `IsLeapYear`, `AddDays`, `AddWeeks`, `AddMonths`, `AddQuarters`, `AddYears`, `StartOf…` and `EndOf…` (`Day`, `Week`, `Month`, `Quarter`, `Year`), `ToRecord`; `DateTime.From`, `FromText`, `ToText`, `Date`, `Time`, `AddZone`, `ToRecord`; `DateTimeZone.From`, `FromText`, `ToText`, `RemoveZone`, `ZoneHours`, `ZoneMinutes`, `ToUtc`, `SwitchZone`; `Time.From`, `FromText`, `ToText`, `Hour`, `Minute`, `Second`, `StartOfHour`, `EndOfHour`; `Duration.From`, `FromText`, `ToText`, `Days`, `Hours`, `Minutes`, `Seconds`, `TotalDays`, `TotalHours`, `TotalMinutes`, `TotalSeconds` |
| Records, values | `Record.Field`, `FieldOrDefault`, `FieldNames`, `FieldValues`, `ToList`, `FieldCount`, `HasFields`, `AddField`, `RemoveFields`, `RenameFields`, `SelectFields`, `TransformFields`, `Combine`, `ToTable`, `FromList`, `FromTable`; `Value.Is`, `As`, `ReplaceType` (renames and retypes table columns), `Metadata` (empty), `RemoveMetadata`, `FromText`, `Add`, `Subtract`, `Multiply`, `Divide`; `Error.Record`; `Function.Invoke`; `Diagnostics.Trace` |
| Splitters, combiners, replacers | `Splitter.SplitTextByDelimiter`, `SplitTextByAnyDelimiter` (`QuoteStyle.Csv` quoting), `SplitTextByPositions`, `SplitTextByLengths`, `SplitByNothing`; `Combiner.CombineTextByDelimiter`; `Replacer.ReplaceText`, `ReplaceValue`; `Comparer.Ordinal`, `OrdinalIgnoreCase`, `FromCulture` (as equality criteria) |
| Binary | `Binary.FromText` (base64, hex), `ToText`, `FromList`, `ToList`, `Length`, `Buffer`, `Decompress` (`Compression.Deflate`, which is raw DEFLATE, and `Compression.GZip`); `#binary` |
| Sources | `File.Contents`, `Folder.Files`, `Folder.Contents` (sandboxed), `Csv.Document`, `Json.Document`, `Excel.Workbook` |

Enumerations: `Order`, `JoinKind`, `MissingField`, `GroupKind`, `QuoteStyle`, `Compression`,
`BinaryEncoding`, `Occurrence`, `RoundingMode`, `Day`, `ExtraValues`, `TextEncoding`.

Type names: `Int64.Type` and the other number facets, `Text.Type`, `Date.Type`, and so on.
`Culture.Current` is the options' culture.

#### Added in M11.34

Each function follows its Microsoft Learn reference page
(`learn.microsoft.com/powerquery-m/<function name, dots as dashes, lower case>`, e.g.
`list-percentile`); the tests (`crates/data/src/m/tests_library.rs`) evaluate the pages'
examples and the edge cases below. The code is ported from DashCraft's `crates/pbi-m/src/stdlib/`
at commit `0958fd82` (MIT, see THIRD_PARTY_NOTICES.md) where it fitted bicraft's values
(`lib/list_more.rs`, `lib/table_more.rs`, `lib/misc_more.rs`). None of these was compared with
Desktop: the M oracle data covers none of them.

| Area | Functions and behaviour |
|---|---|
| Lists | `List.Modes` (every most frequent value, in order of first appearance; `{}` for an empty list), `Covariance` (population; lists of different lengths are an error), `Percentile` (`PercentileMode.ExcelInc`, the default, `ExcelExc`, `SqlDisc`, `SqlCont`; one percentile or a list; `null` for an empty list), `MaxN` / `MinN` (a count or a condition, nulls left out unless asked, a count past the end gives the whole list), `PositionOfAny` (`Occurrence.First`, `Last`, `All`), `Positions`, `IsDistinct` (with a comparer), `RemoveMatchingItems`, `RemoveRange` (an offset past the end is an error), `ReplaceMatchingItems`, `ReplaceRange`, `DateTimeZones`. `List.Mode` now returns, among the most frequent values, the one whose run ends last (`List.Mode({"A", 1, 2, 3, 3, 4, 5, 5})` is 5, as on Learn) and an empty list is an error |
| Tables | `Table.AddJoinColumn`, `AddRankColumn` (`RankKind.Competition`, the default, `Dense`, `Ordinal`; rows in rank order), `AggregateTableColumn`, `AlternateRows`, `ApproximateRowCount`, `ColumnsOfType`, `CombineColumnsToRecord`, `ContainsAll`, `ContainsAny` (partial records match, as `Table.Contains`), `FindText` (text anywhere in a cell, nested values included, within the evaluation depth), `IsDistinct`, `MaxN`, `MinN` (stable: rows that tie keep their order), `Partition`, `PositionOfAny`, `RemoveMatchingRows`, `ReplaceMatchingRows` (whole rows unless equation criteria name the columns), `RemoveRows`, `ReplaceRows`, `Schema` (its 19 columns), `SingleRow` (an error unless exactly one row), `Split`, `SplitAt`, `ToList` (default combiner: texts joined with commas), `TransformColumnNames` |
| Numbers | `Number.BitwiseAnd`, `BitwiseOr`, `BitwiseXor`, `BitwiseNot`, `BitwiseShiftLeft`, `BitwiseShiftRight` (64-bit integers; a fraction or a number out of range is an error; shift counts modulo 64; null gives null); constants `Number.PI`, `E`, `Epsilon`, `PositiveInfinity`, `NegativeInfinity`, `NaN` |
| Binary, JSON | `Binary.From` (null, binary, or a text decoded as `Binary.FromText`), `Combine`, `Range` (offsets clamp to the end), `Split` (page size at least 1), `ApproximateLength`; `Json.FromValue` (records and tables as objects, lists as arrays, dates `yyyy-MM-dd`, times `HH:mm:ss[.fff]`, durations as ISO 8601, binary as base64, `/` and non-ASCII escaped; functions and types are an error), UTF-8 unless an encoding is given |
| Values, types, records | `Value.Type`, `Value.ReplaceMetadata` (bicraft keeps no metadata: the value comes back as it is), `Value.NullableEquals`; `Type.Is`, `IsNullable`, `NonNullable` (`any` gives `anynonnull`), `TableColumn`; `Record.ReorderFields` (the listed fields take the listed fields' places, others stay; `MissingField` honoured); `Time.ToRecord`, `DateTimeZone.ToRecord` |
| Splitters | `Splitter.SplitTextByEachDelimiter` (each delimiter once, in order; `startAtEnd`), `SplitTextByRanges`, `SplitTextByRepeatedLengths`, `SplitTextByWhitespace` (`QuoteStyle.Csv` by default), `SplitTextByCharacterTransition` (a list of characters or a function on each side) |

Enumerations added: `PercentileMode`, `RankKind`. A list range of single characters
(`{"a".."z"}`, as in Learn's examples) is now evaluated. Hostile inputs are capped: lists, binary
pages and split parts count against `Limits::max_rows`, binary values against `max_bytes`,
`Json.FromValue` and `Table.FindText` nesting against `max_eval_depth`, and counts past the
addressable range clamp.

### Desktop's "Enter data"

The query Desktop writes for a typed-in table loads as is:

```text
Table.FromRows(Json.Document(Binary.Decompress(Binary.FromText("…", BinaryEncoding.Base64),
    Compression.Deflate)), let _t = ((type nullable text) meta [Serialized.Text = true]) in
    type table [A = _t, B = _t])
```

It is usually followed by `Table.TransformColumnTypes`. The payload is raw DEFLATE (RFC 1951) of
UTF-8 JSON: a list of rows of texts or nulls. Decompression is capped at `max_bytes`.

**Writing it** (`bicraft_data::m::entered`, the `table.enter_data` command). BIcraft writes the
same shape from a grid of cells: every empty or `null` cell is written as `""`, never `null`
(the type step reads `""` as null in a typed column; a text column keeps `""`), every column is
`_t`, and one
`#"Changed Type" = Table.TransformColumnTypes(Source,{{…}})` step without a culture follows.
That step reads the texts with the model's query culture, so each cell is written as that
culture reads it back: numbers with its decimal separator and no grouping, dates and times in ISO
8601, logicals `true`/`false`. A column without a type takes the type detected from its first 200
texts (whole number, decimal number, date, date/time, true/false, else text). Limits: 1000
columns, 1 000 000 cells. Power BI Desktop 2.159.1231.0 (2026-11-23): its Edit data (the Create
Table dialog) failed on rows holding `null`, which BIcraft used to write; Desktop's own payload
holds `""` for an empty cell. Written with `""`, the query opens in the dialog with every row,
and Desktop's save after an edit changes only the payload and adds the annotation
`PBI_NavigationStepName = Navigation` to the partition.

**Reading it back** (`table.entered_data`). Accepted: this shape with or without its type step,
and a plain `#table` literal (the query `table.import {csv}` writes); any M that names a library
function outside these shapes is refused, so a hand-written query is never rewritten as a
literal. The query is evaluated (no files, no other queries) and its values written back as texts
in the query culture.

### Navigator (Get data)

`bicraft_data::m::navigate` (the `source.navigate` command) lists what a local source holds and
writes the M query of each item. Every query reads the source through `File.Contents` or
`Folder.Files`, so only under the session's data root. Shapes (BIcraft's own; the step names
follow Power Query's usual English names, not checked against a Desktop capture):

| Source | Items | Query |
|---|---|---|
| Excel (`.xlsx`, `.xlsm`) | each sheet, table and defined name | `Excel.Workbook(File.Contents(p), null, true)`, `Source{[Item=…,Kind=…]}[Data]` |
| Text/CSV (`.csv`, `.tsv`, `.txt`, `.prn`) | the file | `Csv.Document(File.Contents(p),[Delimiter=…, Encoding=65001 or 1252, QuoteStyle=QuoteStyle.Csv])` |
| JSON | a list: the file; a record: the record and each list or record field | `Json.Document(File.Contents(p))`, then `Table.FromRecords` (records) or `Table.FromList` (other values), `Record.ToTable` |
| Folder | the file list, and `<folder> (combined)` when it holds CSV files | `Folder.Files(p)` then `Table.SelectColumns` (without `Content`); the combined item `Table.Combine` of the `.csv` files read as the first one |
| SQL Server database (`server`, `database`, `query?`) | each user table and view, or the SQL statement's result | `Sql.Database(server, database)`, `Source{[Schema=…,Item=…]}[Data]`; with a statement `Sql.Database(server, database, [Query=…])` ([tds.md](tds.md)) |
| Web (`url`, Desktop's *From Web*, Basic) | as a JSON file (the content starts with `{` or `[`) or as a text file (anything else); an HTML or XML page is refused | `Json.Document(Web.Contents(url))` or `Csv.Document(Web.Contents(url),[…])`, the rest as for files; the item is named after the URL's last path segment ([odata.md](odata.md), *Web.Contents*) |
| OData feed (`url`) | each entity set of the service document (`Kind` `Table`); a feed URL that is an entity set: that table | `OData.Feed(url, null, [Implementation="2.0"])`, `Source{[Name=…,Signature="table"]}[Data]` in a step `<Name>_table` ([odata.md](odata.md)) |
| SharePoint folder (`url`, a site) | the site's files as a folder: `Combine` and `Load` items, as for a local folder | `SharePoint.Files(url, [ApiVersion = 15])`, the rest as for a folder ([sharepoint.md](sharepoint.md)) |
| SharePoint list (`url`, a site) | each list of the site that is not hidden, named by its `Title` (`Kind` `Table`) | `SharePoint.Tables(url, [Implementation = "2.0", ViewMode = "All"])`, `Source{[Id="<id>"]}[Items]` in a step named after the list's id ([sharepoint.md](sharepoint.md)) |

The delimiter is the one given, else tab for `.tsv`, else the most frequent of `,` `;` tab `|` in
the first line. The encoding is 65001 when the first bytes are valid UTF-8, else 1252. The first
row is promoted (`Table.PromoteHeaders(…, [PromoteAllScalars=true])`) when it reads as headers:
no cell is a number, at least one is not empty, and a second row follows (CSV). A last
`#"Changed Type"` step types the columns detected from the first 200 rows; `detect_types: false`
leaves it out. `Csv.Document` is written without `Columns=`, unlike Desktop's usual query. An item
whose first rows do not load keeps its query and gives the reason (`error`).

### Files

`File.Contents(path)`, `Folder.Files(path)` and `Folder.Contents(path)` read only under the
allow-list `SourceOptions::file_roots`; an empty list disables file access.

- **Paths.** A relative path is joined to the first root. An absolute path is accepted as
  written. Both are resolved (`..` and symbolic links) and must lie under one of the resolved
  roots. Anything else is `UnsupportedSource`, and `try` cannot catch it.
- **Missing files.** A missing file or folder inside a root is a catchable `DataSource.Error`.
- **Size.** Files larger than `max_bytes` are `LimitExceeded`, except CSV files the fast path
  streams (*Fast CSV loading*), capped by `max_file_bytes` and the memory budget.
- **Access.** There is no network, environment or registry access.

**`Folder.Files(path)`** lists every file under the folder and its subfolders;
**`Folder.Contents(path)`** lists the files and folders directly in it, a folder's `Content`
being its own contents table.

- **Columns.** `Content` (binary), `Name`, `Extension`, `Date accessed`, `Date modified`,
  `Date created` (datetime), `Attributes` (record), `Folder Path` (text), as in Desktop.
- **Order.** Per folder, the files sorted by name ignoring case, then each subfolder (sorted the
  same way); `Folder.Contents` lists files and folders together, by name.
- **Values.**
  - `Extension` runs from the last dot of the name, in lower case (`b.CSV` → `.csv`); it is
    empty for folders and names without a dot.
  - `Folder Path` is the folder as the query names it, with a trailing separator; on Windows
    `/` is spelled `\` (`Folder.Files("C:/data")` lists `C:\data\`).
  - `Attributes` holds, in this order, `Content Type`, `Kind`, `Size`, `ReadOnly`, `Hidden`,
    `System`, `Directory`, `Archive`, `Device`, `Normal`, `Temporary`, `SparseFile`,
    `ReparsePoint`, `Compressed`, `Offline`, `NotContentIndexed`, `Encrypted`, `ChangeTime`,
    `SymbolicLink`, `MountPoint`. `Content Type` and `Kind` come from the extension: `.csv` is
    `text/csv`, `CSV File`; `.txt` and no extension `text/plain`, `Text File`; `.json`
    `application/json`, `JSON File`; a folder `Folder` (other extensions follow the same
    pattern, not checked). The flags are Windows' own; on Linux read-only comes from the
    permissions and hidden means a dot file. `ChangeTime` is the modification time.
  - Dates are the file times in UTC: BIcraft reads no time zone, Desktop shows local time.
- **Laziness.** A file's content is read only when used (each read capped at `max_bytes`), so
  `Table.SelectRows(Folder.Files(…), each [Extension] = ".csv")` never reads the other files.
- **Links.** An entry whose resolved path is outside the roots is skipped; a linked folder is
  never entered (no loops). Entries count against `max_rows` and the step budget; nesting is
  capped by `max_depth`.

Desktop's "Combine files" queries load: the filter on `[Attributes]?[Hidden]?`, a function
query over `[Content]`, `Table.ExpandTableColumn`.

### Fast CSV loading

CSV files load without building an M value per cell when the query has the common shape:
`Csv.Document(File.Contents(path), options)`, optionally followed by
`Table.PromoteHeaders(…)` and then `Table.TransformColumnTypes(…, {{name, type}, …}, culture)`
(as `let` steps or as nested calls; `crates/data/src/m/fastcsv.rs`). `read_csv` (`table.import
{csv}`) uses the same tokenizer (`crates/data/src/csv_fast.rs`).

- **Same result.** Names, types, values, blanks, date kinds and errors are the evaluator's (or
  `read_csv`'s sequential reader's): differential tests compare both paths on generated files
  (quotes, embedded line breaks, empty fields, short and long records, en-US/fr-FR/de-DE,
  dates, decimals, currency, BOM, CRLF and lone CR, code pages, multi-byte delimiters,
  malformed and binary input). Cells are converted by the same functions, with exact fast forms
  for plain integers, fixed decimals, plain numbers and ISO dates.
- **What it takes.** `Delimiter` (any text without `"`, CR or LF), `Columns` as a count,
  `Encoding` 65001, 1200, 1201, 1252, 28591, 20127 or none, `QuoteStyle.Csv` or `.None`,
  `ExtraValues.Ignore` or `.Error`; column types text, any, number (every facet), logical,
  date, datetime, time, duration. Anything else (another option or type, a column list, a
  missing column, a step that shadows a library name, a parameter as the path) runs in the
  evaluator as before.
- **Parallel and streamed.** The file is read in regions of 64 MiB. Each region is cut into
  chunks at line feeds and the chunks are parsed on all cores, typed straight into the
  columns. A chunk that started inside a quoted field (a line feed in quotes) is detected
  when its start does not match where the previous chunk really ended, and parsed again from
  there. UTF-8 and ASCII-only single-byte code pages stream; UTF-16, other bytes of a code
  page and invalid UTF-8 are decoded whole first, as the evaluator decodes them.
- **Limits.** `max_rows`, `max_cell_bytes`, `max_steps` (the evaluator's count for these
  steps) and `max_millis` apply as in the evaluator, with the same messages. `max_bytes` does
  not cap these files: `max_file_bytes` (default 64 GiB) does, and the memory budget
  `max_load_bytes` (default 16 GiB) refuses a load whose table (24 bytes a cell plus texts),
  estimated from the first region and counted while loading, would not fit. The same bytes are
  reserved against the process-wide limit (`BICRAFT_MEMORY_LIMIT`, class `io`) while loading.
  `read_csv` applies the budget too.

### External sources

Connectors that reach a database or analytical server (`Sql.Database`, `Oracle.Database`,
`Odbc.Query`, `AnalysisServices.Database`, `Snowflake.Databases`, …), a web service or online
store (`Web.Contents`, `OData.Feed`, `SharePoint.Files`, `AzureStorage.Blobs`,
`Salesforce.Objects`, `PowerPlatform.Dataflows`, …) or a script runtime (`Python.Execute`,
`R.Execute`) are never run, unless the caller registered a connector for them (*Connectors*). Naming one is
`Error::ExternalSource` ("external source, not evaluated: `Sql.Database` reads a database or
analytical server; …"); `try` cannot catch it.
The query text is never changed: a PBIP, PBIT or `.bim` save keeps it as written, the viewer
shows the table as unavailable with that reason, and a PBIX save that needs its rows is refused
with it (give the table data with `table.set_data`, or refresh it in Desktop).

**`Csv.Document(source, [Delimiter, Columns, Encoding, QuoteStyle, ExtraValues])`.** The older
positional form `(source, columns, delimiter, extraValues, encoding)` is also accepted.

- **Source.** Binary or text. The encoding is a code page: 65001 (the default), 1200, 1201, 1252,
  28591 or 20127. A byte-order mark wins.
- **Quoting.** Fields may be quoted (`""` is a quote).
  - `QuoteStyle.Csv`: a quoted field may span lines.
  - `QuoteStyle.None`: a line break always ends the record, so the rest of a multi-line field
    starts a new row (Desktop does the same).
- **Width.** `Columns` gives the width. Without it, the **first** record does.
  - Extra fields are dropped.
  - Missing fields are `""` (empty text, not `null`).
- **Values.** Every value is text, in columns `Column1…N`.

**`Json.Document(source)`.** Objects become records (key order kept), arrays lists and numbers
numbers. Nesting and the number of values are capped.

**`Excel.Workbook(source, useHeaders)`** reads `.xlsx`. `.xlsb` and `.xls` are unsupported.

- **Result.** A navigation table `[Name, Data, Item, Kind, Hidden]` with one row per sheet
  (`Kind = "Sheet"`) and one per Excel table (`Kind = "Table"`, headers promoted).
- **Sheet data.**
  - The data runs from the first used row and column to the last, and includes empty rows inside
    that range.
  - Values: shared and inline strings, numbers, booleans, cached formula results, error cells
    (stored errors).
- **Dates.**
  - Numbers whose cell format is a date or time format become `datetime`. Built-in ids 14–22,
    27–36, 45–47 and 50–58 count, as do custom formats with date or time tokens.
  - In the 1900 date system, serials 1–60 are shifted one day, as Desktop does, so serial 60 reads
    as 1900-03-01.
  - The 1904 date system is honoured.

### Static checks without evaluation (`m::analyze`, M9.7)

`bicraft_data::m::analyze` / `analyze_all` parse an expression with the same lexer and parser
and evaluate nothing, so connectors (Databricks, SQL, …) are fine. They return:

- the first **syntax error** with its 1-based line and column;
- the **free names**: identifiers the expression uses but does not define, with position and
  byte span (for rewrites). Scoping per the M specification: `let` variables are visible in all
  variable expressions and the body (mutually recursive), record fields in all field
  expressions, function parameters in the body, `each` defines `_`, a `catch` parameter only in
  its handler. A `[Field]` outside `each` uses an undefined `_`;
- the **steps** (variables of the outermost `let`) and, when the text tells it, the **result
  type** in Desktop's `PBI_ResultType` spelling: `Table` (a `#table`, a `Table.*` function that
  returns a table, a connector or file function, a navigation step `x{[...]}[Data]`), `Text`,
  `Number`, `Logical`, `Date`, `DateTime`, `Time`, `Duration`, `List`, `Record`, `Function`.
  `let … in Step` is followed to the step; `meta` is looked through (parameters).

`library_name` classifies a free name: `Evaluated` (in the evaluator's library), `Documented`
(the public M function reference, enumeration members such as `JoinKind.Inner`, `*.Type`,
`#date` and the other intrinsics), `Connector` (the external namespaces above), or `Unknown`.
`identifier` writes a name as M needs it (`Sales`, `#"My Query"`, `#"a""b"`).

The engine's `m.validate` turns this into diagnostics: `m-syntax` (error), `m-undefined-name`
(error: not a step, a query or parameter of the model, nor a library name) and
`m-unknown-function` (warning: a dotted name BIcraft does not know; the reference list is not
exhaustive). A quoted name (`#"Sales.Fact"`) is a query name, never a library function. The
walk uses an explicit stack and runs on a large-stack thread, like the evaluator, so deep or
long expressions are bounded by the parser's `max_depth` and `max_bytes` only.

### Formatting (`m::format`, M11.35)

`m::format(text, options, limits)` lays an M expression out; the engine's `m.format` command
and the Format buttons of the Power Query editor (Advanced editor, formula bar) call it. The
layout is ported from DashCraft's `crates/pbi-m/src/service/format.rs` at commit `0958fd82`
(MIT, see THIRD_PARTY_NOTICES.md), driven by a syntax tree of token ranges that mirrors
bicraft's own grammar (`crates/data/src/m/format.rs`):

- Only white space changes. Tokens, text literals, quoted identifiers and generalized names
  (`[Order   Date]`) are written as they were; comments stay after their token on its line or on
  their own lines before the next token; one blank line between steps or before a comment is
  kept. A BOM and CR LF line breaks outside literals become `\n`.
- `let` puts each step on its own line and the result after `in` on the next, indented, as
  Desktop writes queries. A group (a call's arguments, a record, a list, parentheses, a chain of
  operators of one precedence, an `if`, a `try`, a step's value) stays on one line when it fits
  in `max_width` (default 100), else one item per line with the closing bracket on its own line,
  operators at the start of the continuation lines, `then` / `else` branches indented.
- `indent` (default 4 spaces, at most 16) or tabs.
- The result is checked before it is returned: it must lex to the same tokens and comments,
  parse to the same tree and keep the same generalized names; otherwise the text is refused as
  `Unstable` rather than changed. Formatting is idempotent.
- Section documents are not formatted (bicraft's parser reads expressions only). The work runs
  on the evaluator's large-stack thread under the same `Limits` (`max_bytes`, `max_depth`); long
  operator chains and lists are flat in the tree, so their length does not nest.

### Result mapping

The result must be a table. Its column types map to model types:

| M column type | Column type |
|---|---|
| `text` | String |
| `number`, `Double.Type`, `Single.Type`, `Decimal.Type`, `Percentage.Type` | Double |
| `Int64.Type`, `Int32.Type`, `Int16.Type`, `Int8.Type`, `Byte.Type` | Int64 |
| `Currency.Type` | Decimal |
| `date`, `datetime`, `time` | DateTime (`time` on 1899-12-30) |
| `datetimezone` | DateTime: the local clock time, offset dropped (as Desktop loads it) |
| `logical` | Boolean |
| `duration` | Double (days) |
| `any` (untyped) | inferred from the values: one kind gives that kind, only numbers give Double, any other mix gives String |
| `list`, `record`, `table`, `function`, `type` | `UnsupportedSource` |

Typed columns convert strictly: numeric widening only. A value of the wrong kind is
`Error::Value`. `conform` then maps the data onto the model's declared columns.

### Desktop agreement

The test projects were PBIPs whose tables are M queries over our own synthetic data, including
CSV, JSON and `.xlsx` files we generated. Each was compared with Power BI Desktop 2.158.1304.0:

1. Open the PBIP.
2. Refresh each table through the local Analysis Services instance (TMSL `refresh`).
3. Run `EVALUATE 'table'`.
4. Compare the rows, as a multiset, with `load_partition_opts`.

The queries and expected rows are kept as golden tests in `crates/data/src/m/tests_desktop.rs`.

| Round | Model | Tables | Result |
|---|---|---|---|
| 1 | en-US, `returnErrorValuesAsNull` | 22 | 22 identical (the 6 decimal cells since round 3). |
| 2 | fr-FR, `returnErrorValuesAsNull` off | 14 | 14 identical. |
| 3 | en-US: decimal and double numbers | 2 | 52 probes and a typed load table (double, whole-number and currency columns) identical; about 480 further probes agree except for error-message wording and `$2.675`. |
| 4 | en-US: folders and files (2026-10-14) | 6 | `Folder.Files` (names, extensions, folder paths, every `Attributes` field, sizes, flags, order), the "Combine files" shape over it, `Folder.Contents` (nested folders), `Json.Document`, a forward-slash path: identical once the extension case, the `Kind` names, the four last attributes and the path separator were fixed; file dates differ by the time zone (see *Files*). `Folder.Files` and `Folder.Contents` are golden-tested in `crates/data/src/m/tests_safety.rs` (`folder_sources_match_desktop`). |

Round 1 tables:

- Enter data with `TransformColumnTypes`
- a 13-step transform chain
- `Group`
- `NestedJoin` left, inner and anti
- `Distinct`, `Combine`, `ReplaceValue`
- a `List.Dates` calendar
- rounding and number formats
- text sort
- 15 `Text.*` functions
- 38 to-text conversions
- duration, datetimezone and time columns
- three CSV reads
- JSON
- two Excel reads
- fill, split and unpivot with a parameter
- fr-FR conversions
- 54 expressions

Round 2 tables:

- fr-FR defaults and formats
- error cells
- `Pivot`, `ExpandListColumn`, `TransformColumns`, `Text.Format`
- case-insensitive `Distinct`, local `Group`, flat `Join`
- `ReplaceErrorValues` and `RemoveRowsWithErrors`
- cp1252 CSV and CSV extra values
- an Excel table
- `PromoteHeaders`
- 62 expressions

**Analysis Services storage, not M.** M results keep every string as computed. What the engine
then stores differs (see *Stored text* below); the comparisons above account for it, and the
engine applies it after loading (`TextStorage`).

## Connectors (`bicraft_data::connect`, `crates/databricks`)

Status: implemented 2026-10-23 (G21). Five families: **Databricks**, **SQL Server**
(2026-11-18, M11 lot 11: `crates/sqlserver`, described in [tds.md](tds.md)), and **Web**
(`Web.Contents`) and **OData** (`OData.Feed`) (2026-11-19, lot 11: `crates/webdata`, described in
[odata.md](odata.md)), and **SharePoint** (`SharePoint.Files`, `SharePoint.Contents`,
`SharePoint.Tables`, 2026-11-20: `crates/webdata`, described in [sharepoint.md](sharepoint.md),
with the web credentials and privacy levels). A connector function may return a binary value instead of a navigation
root (`Connector::returns_binary` and `contents`: `Web.Contents`), which the evaluator's own
readers then read.

**Sources.** Public documentation only:

- Databricks, *Statement Execution API* reference (`/api/2.0/sql/statements`) and the guide *Run
  SQL statements on warehouses* (request fields, states, manifest, chunks, external links, error
  shape);
- Databricks, *SQL data types* and *Collation* references (type names, `collate` in a type's
  text);
- the Power Query M function reference (`Table.SelectRows`, `Table.SelectColumns`,
  `Table.FirstN`, navigation tables).

The query shape is the one Power BI Desktop
writes for Databricks (the shape of a Databricks partition template).

### Activation

A connector runs only when the caller registers it in `SourceOptions::connectors`. The engine does
so only for `model.refresh {allow_connectors: ["databricks"]}` (CLI `bicraft refresh PATH
--allow-connector databricks`, MCP `refresh_model`). Everything else is as before:

- `dax.evaluate`, `report.render`, the viewer, the structural PBIX save, `table.import` never run
  a connector;
- without registration a connector function is `Error::ExternalSource`;
- registering one family does not enable another (`Sql.Database` needs `"sqlserver"`).

`query.preview`, `source.navigate` (SQL Server, Web, OData), `source.load` and `table.import` take the same
`allow_connectors` list since lot 11; `table.import` and `source.load` then read only the
columns (`Table.FirstN(query, 0)`), the rows come with `model.refresh`.

### Interface (`bicraft-data`, L2)

`bicraft-data` never opens a socket. A `Connector` (`Send + Sync`) names the M functions it
implements (`handles`) and `open`s a `Source` per call, with the evaluated arguments as plain
data (`Arg`).

| `Source` method | Meaning |
|---|---|
| `lookup(path, name, kind)` | The child of a navigation node, when it can be named without listing (`{[Name = "main", Kind = "Database"]}`); `None` → the node is listed |
| `list(path)` | The children: `NavEntry {name, kind, leaf}` |
| `columns(path)` | A table's columns (`RemoteColumn {name, ty, comparable}`) |
| `fetch(RemoteQuery, max_rows)` | Rows of `{table, columns?, filter: [Pred], sort: [SortKey], limit?}` |
| `folds_sort()` | The source orders rows as M does for the columns it marks comparable (default false) |
| `root_is_table()` | The call itself is a table that nothing folds into (a native query; default false) |

A `NavEntry` may carry extra `fields` (SQL Server: `Schema`, `Item`), which become columns of
the navigation table between `Data` and `Kind`, the same for every entry of a node.

In the evaluator a connector call is a lazy **remote value** (`crates/data/src/m/remote.rs`):

- A navigation node becomes a table `[Name, Data, Kind]` only when it is used as a table (listed
  once); its `Data` cells are again remote values, so `Table.SelectRows(Schema, each [Name] =
  "t"){0}[Data]` lists the schema and keeps the table itself lazy.
- A key lookup on a node the connector can name descends without a query.
- Any use the evaluator cannot fold fetches the rows once (kept for later uses) and continues in
  the local evaluator: operators, field access, `Table.Buffer`, every other library function.

**Errors.** A source error (SQL error, missing table, unresolved column) is a catchable M
`DataSource.Error`, as in Desktop. Missing or rejected credentials, network and service failures
are `Error::Connector`; limits are `Error::LimitExceeded`. `try` cannot catch these, so a
credentials problem never turns into fallback data.

### Query folding

Folding only moves work to the source; the result is the one the local evaluator would give
(tested: every case is also evaluated after `Table.Buffer`, which fetches first, and both results
are compared). While a remote table has not been fetched:

| M | Folds into | When |
|---|---|---|
| `Table.SelectColumns(t, names)` | the column list, in that order | names are texts without duplicates; missing-field mode `MissingField.Error` (default); a selection after a selection only narrows it |
| `Table.RemoveColumns(t, names)` | the column list | same; the names exist (from `columns`) |
| `Table.FirstN(t, n)` | `LIMIT n` | `n` a whole number (not a condition) |
| `Table.SelectRows(t, each …)` | the filter | no limit before it; the function has one parameter and its body is a conjunction (`and`) of comparisons below |
| `Table.Sort(t, keys)` | the sort (`RemoteQuery::sort`; a later sort's keys first) | only for a source whose `folds_sort` is true (SQL Server, OData), no limit before it, every key a visible column the source compares as M does, and not text |

A comparison is `[column] op constant` or `constant op [column]`, `op` one of `=`, `<>`, `<`,
`<=`, `>`, `>=`. The constant is evaluated once in the function's environment (literals,
parameters, `#date(…)`); it must not mention the row. The column must be visible (selected) and
the source must compare it as M does (`comparable`):

| Column | Constant | Operators |
|---|---|---|
| any | `null` | `=` (→ `IS NULL`), `<>` (→ `IS NOT NULL`) |
| text (binary collation) | text | `=`, `<>` |
| logical | logical | `=`, `<>` |
| whole numbers, decimal, double | whole number, \|n\| ≤ 2^53 | all |
| date | date | all |

**M semantics in the filter (`Pred`).** `null = v` is false, `null <> v` is **true**, `null < v`
is null, which `Table.SelectRows` treats as false. In SQL `NULL <> v` is null (excluded), so `<>`
becomes `(c <> v OR c IS NULL)`; the other comparisons exclude nulls in both.

Not folded (evaluated locally after the fetch): text order comparisons (M orders by UTF-16 code
unit, SQL by code point or collation), non-binary collations, `FLOAT` columns (SQL compares them
in single precision), timestamps (M `date` vs `datetime` types and time zones), fractional
numbers, `or`, `not`, functions of the row (`Text.StartsWith([c], …)`), comparisons between two
columns, a filter after a limit, `Table.RenameColumns`, `Table.TransformColumnTypes` and the
rest. Caveat: a `BIGINT` or `DECIMAL` value beyond 2^53 compares exactly in SQL but as a double
in M; a folded comparison on such values can keep a row M would drop (or the reverse).

Column names are case-sensitive in M and not in Databricks SQL: the names the source returns
must equal the selected ones exactly, else the M error *The column 'X' of the table wasn't
found*.

### Folding indicators

Power Query's editor marks each applied step with a folding indicator and offers *View native
query* on a step's context menu (Microsoft Learn: "Query folding indicators", "Query folding
basics", "View native query"). BIcraft's indicators come from its own evaluator, not from
Desktop's rules: each step is the query `let … in <step>`, evaluated with
`bicraft_data::m::folding::probe`, which never reads a remote table's rows (navigation listings
and the `columns` metadata may be read):

| Indicator | The query up to the step | View native query |
|---|---|---|
| `folded` | is still a connector value: one request to the source | the table's query (`Source::native`); none for a navigation table |
| `partial` | needs the rows of a source's query: the source runs the folded part, BIcraft the rest | greyed out |
| `not_folded` | reads no source that folds (local data, `Web.Contents`) | greyed out |
| `unknown` | could not be evaluated (connector not allowed, credentials, an error) | greyed out |

The indicator is that of the whole query up to the step, so a navigation step after a listing
can be `folded` again (Learn's indicator is per step and may differ; not compared with Desktop).
`Source::native` returns what `fetch` would send, built by the same code: Databricks SQL with
its parameter markers listed as `-- :p0 = value (TYPE)` comment lines (the values travel apart
from the text), SQL Server's `SELECT` (or the `[Query=…]` text as written), the OData URL with
its `$select`, `$filter`, `$orderby` and `$top`.

**Query diagnostics.** `bicraft_data::m::folding::diagnose` evaluates a query as a refresh does
and keeps each request sent to a source (`list` of a navigation node, `query` of a table: its
native text, rows returned and BIcraft's own time in milliseconds).

### Databricks

`DatabricksMultiCloud.Catalogs(host, httpPath, options)` (and `Databricks.Catalogs`, Azure):

- **Host**: a DNS name with an optional port, with or without `https://` and a trailing `/`;
  anything else (a path, `user@`, `http://`) is refused before any request. Requests go to
  `https://<host>` only: no proxy, no redirect.
- **HTTP path**: `/sql/1.0/warehouses/<id>` (or the older `/sql/1.0/endpoints/<id>`); the id is the
  SQL warehouse. A cluster path (`/sql/protocolv1/o/…`) is refused: the Statement Execution API
  runs on SQL warehouses only.
- **Options** (`Catalog`, `Database`, `Implementation`, …) are accepted and not used.

**Navigation.** Catalogs (`Kind = "Database"`, `SHOW CATALOGS`), schemas (`"Schema"`, `SHOW
SCHEMAS IN`), tables and views (`"Table"`, `"View"`; `SELECT table_name, table_type FROM
<catalog>.information_schema.tables WHERE table_schema = :p0`, else `SHOW TABLES IN` without the
kind when there is no `information_schema`, as in `hive_metastore`). A lookup by name with the
kind `Database`, `Schema`, `Table` or `View` needs no query; a missing object is then reported by
the query that reads it (`TABLE_OR_VIEW_NOT_FOUND`, a `DataSource.Error`). The column list for a
folding decision is one `SELECT * … LIMIT 0` per table (manifest only), shared by the tables of a
refresh.

**SQL.** `SELECT <columns or *> FROM `catalog`.`schema`.`table` [WHERE …] [LIMIT n]`.
Identifiers are quoted with backticks (doubled inside); every literal is a named parameter
(`:p0`, sent in `parameters` with its type: `STRING`, `BOOLEAN`, `BIGINT`, `DATE`), never spliced
into the text.

**API flow.** `POST /api/2.0/sql/statements/` with `statement`, `warehouse_id`, `parameters`,
`format: "JSON_ARRAY"`, `disposition: "INLINE"` (or `"EXTERNAL_LINKS"`), `wait_timeout: "30s"`,
`on_wait_timeout: "CONTINUE"`:

| State | Then |
|---|---|
| `PENDING`, `RUNNING` | `GET /api/2.0/sql/statements/{id}` after 0.5 s, doubling up to 5 s; past 15 min: `POST …/{id}/cancel` and an error |
| `SUCCEEDED` | rows of the first chunk from the response, then `GET …/{id}/result/chunks/{n}` while there is a `next_chunk_index` (per external link in `EXTERNAL_LINKS`) |
| `FAILED` | `DataSource.Error` "Databricks: `error_code`: `message`" |
| `CANCELED`, `CLOSED` | connector error |

`manifest.truncated: true` is refused (`LimitExceeded`): rows are never silently missing. HTTP
401/403 are credentials errors; 429, 500, 502, 503 and 504 and transport failures are retried 4
times (honouring `Retry-After`, at most 60 s), then fail; any other status fails with its
`error_code` and `message`. Responses are capped at 512 MiB, rows at the evaluator's `max_rows`.

**Why this API (not ODBC or Thrift).**

| | SQL Statement Execution API (chosen) | ODBC (Simba Spark / Databricks ODBC driver) | Thrift / HiveServer2 over HTTP |
|---|---|---|---|
| What it is | Documented, versioned REST + JSON over HTTPS | A native driver library loaded by the process | The binary RPC protocol drivers speak (`TCLIService`) |
| In pure Rust, static musl | Yes: an HTTPS client (ureq + rustls) and JSON | No: a C driver per platform, a driver manager, a licence to accept; nothing to link statically | Possible, but a Thrift client to generate and maintain, Arrow decoding, an undocumented-in-detail session protocol |
| Auth | Bearer token (PAT, or an OAuth access token) | Driver options | HTTP bearer header |
| Results | `JSON_ARRAY` inline (≤ 25 MiB per statement) or presigned links (large results); `ARROW_STREAM` also offered | Driver buffers | Arrow batches or row sets |
| Needs | A SQL warehouse | Cluster or warehouse | Cluster or warehouse |

The API is the only one of the three that is public, stable and implementable with the
workspace's existing dependencies. `JSON_ARRAY` was chosen over `ARROW_STREAM`: it needs no Arrow
IPC decoder (no new dependency, no `arrow` crate tree), every value comes as text that is parsed
exactly (decimals keep all their digits), and for the table sizes of a Power BI import model the
parsing cost is small next to the network. Arrow is the next step if large tables are slow.
`INLINE` is the default because it keeps every request on the workspace host; `EXTERNAL_LINKS`
(opt-in, `external_links: true`) lifts the 25 MiB cap at the price of requests to the cloud
storage host named by the API, made **without** the `Authorization` header (the links are
presigned; their `http_headers` are sent, except any authorization).

### Types

The M type the connector gives each column, from the manifest's `type_name` (and `type_text` for
collations). This is what a query that does not convert the column loads; the queries Desktop
writes for a model then apply `Table.TransformColumnTypes`, which decides the model types
(*Result mapping*). It is aligned on what the column holds and on Desktop's model types for these
SQL types (fixed decimal only through `Currency.Type`, dates without time); it was not compared
with Desktop's own Databricks connector (no workspace was available while it was written).

| Databricks | M type | Values (`JSON_ARRAY` text) | Folded comparisons |
|---|---|---|---|
| `STRING`, `CHAR`, `VARCHAR` | `text` | as is | `=`/`<>` unless the type text has a `collate` other than `UTF8_BINARY` |
| `BOOLEAN` | `logical` | `true`/`false` | `=`/`<>` |
| `TINYINT`/`BYTE`, `SMALLINT`/`SHORT`, `INT`, `BIGINT`/`LONG` | `Int8.Type` … `Int64.Type` | decimal integers | all |
| `FLOAT` | `Single.Type` | `NaN`, `Infinity`, exponents | none |
| `DOUBLE` | `Double.Type` | same | all |
| `DECIMAL(p,s)` | `Decimal.Type`, exact decimals (> 28 digits: the double) | `-123.4500` | all |
| `DATE` | `date` | `YYYY-MM-DD` | all |
| `TIMESTAMP`, `TIMESTAMP_NTZ` | `datetime` | ISO 8601, `T` or space, fractions truncated to the millisecond, `Z` or `±hh:mm` converted to UTC (the warehouse session time zone is UTC by default) | none |
| `BINARY` | `binary` | base64 | none |
| `VOID` | `any` | null | none |
| `INTERVAL`, `ARRAY`, `MAP`, `STRUCT`, `VARIANT`, others | `text` | the text the API returns (JSON for complex types) | none |

A value that does not parse as its type is a `DataSource.Error` naming the column and row.

### Authentication

- The connector receives a **token provider**, called once, when the first request is made
  (navigation by key needs none): no request, no prompt for a table that is never read.
- The engine's provider reads the environment variable `BICRAFT_DATABRICKS_TOKEN` (a personal
  access token, or any bearer token). The CLI adds a masked prompt on an interactive console when
  the variable is unset. The MCP server reads the variable of its own process: the user sets it
  where the server is started. A command-line option for the token does not exist (`--token` is
  refused with a message), and no command parameter accepts one.
- The token is sent only as `Authorization: Bearer …` to the workspace host. It is never written
  (not in the snapshot, the model, a log), never in a result, an error or `Debug` output (`Token`
  prints `Token(***)`); any server message is scrubbed of it. Errors name the host and the
  variable, not the token.
- **OAuth U2M** (user-to-machine, the browser sign-in Desktop offers) is not implemented. The
  plan: the OAuth 2.0 authorization-code flow with PKCE against the workspace's
  `/oidc/v1/authorize` and `/oidc/v1/token` endpoints, with the public `databricks-cli` client id
  and a loopback redirect, the access token kept in memory only and fed through the same token
  provider (so nothing else changes); refresh tokens would need an OS keychain, hence not stored.
  M2M (service principal client credentials) fits the same provider.

## Refresh by BIcraft (`model.refresh`, `crates/engine/src/refresh.rs`)

`model.refresh {tables?, allow_connectors?, external_links?, data_root?}` on an open PBIP project:

1. Every import table in scope (M partitions; DirectQuery and other source types are listed in
   `skipped`) is loaded with `load_partition_opts`: the model's culture and shared queries, the
   allowed connectors, files only under `data_root`, `DateTimeZone.UtcNow()` = the refresh's start
   time, `max_rows` 50 million, no time cap. Text is stored as Analysis Services stores it
   (*Stored text*).
2. If any of them fails, nothing is written and the error lists every failure (an external source
   without `allow_connectors` says so).
3. Calculated tables and calculated columns are evaluated by the DAX evaluator over the refreshed
   tables, in passes of at most 16; one that cannot be evaluated is listed in `not_computed` with
   its reason and stays unavailable (the refresh still succeeds).
4. The snapshot is written atomically (temporary file, then rename).

With `tables`, only those are refreshed; the other tables of the previous snapshot that still
match the model are kept (`kept`) and seen by the calculated objects. The result lists per table
its origin (`m`, `dax`), rows, columns, time, and the SQL statements run (with parameter values,
never a credential). A warning says when no `.gitignore` between the project and its repository
ignores the snapshot (BIcraft's `project.new` writes `**/.pbi/bicraft-data.bin` next to
Desktop's two lines; a heuristic: the file name, `.pbi/` or `*.bin` patterns).

**On an open PBIX** (M11.18, `crates/engine/src/refresh_pbix.rs`) the sources run the same way
(the stored model's partitions, its culture and shared queries, the allowed connectors, files
only under `data_root`, the refresh's start time as the clock), but through the structural
save's loaders (at most 16,777,216 rows per table) and every calculated object must evaluate:
the rows go into the file's `DataModel` ([`datamodel-fresh.md`](datamodel-fresh.md), *Refresh of
an existing PBIX*), never into a snapshot.

### BIcraft data snapshot (`bicraft_data::store`)

`<Name>.SemanticModel/.pbi/bicraft-data.bin`, next to Desktop's files, in BIcraft's own format:
it is **not** an Analysis Services backup, Desktop never reads it, and BIcraft never writes,
modifies or copies `.pbi/cache.abf` (nor reads `localSettings.json`).

| Bytes | Content |
|---|---|
| 16 | `BICRAFT-DATA` and four NUL bytes |
| 4 | format version, 1 (another version is refused: refresh again) |
| 8 | body length |
| n | body, raw DEFLATE |

Body: refresh time (ISO UTC), producer, notes (connector names), then per table its name, a
**fingerprint**, its origin and its columns (name, data type, one tagged value per row: blank,
int64, double, fixed decimal ×10 000, date, boolean, text). Integers are little-endian, strings
`u32` length + UTF-8. Reading is bounded (4 GiB file and body, 50 million rows, the column and
text caps of `Limits`) and checks every length; truncated or corrupted files are errors (tested
by truncating and flipping bytes).

The fingerprint is a stable 64-bit FNV-1a hash of what produced the rows: the table name, its
partitions (type and text), its columns (name, source column, expression, data type), the model's
culture and source query culture, and the shared queries and parameters its M partitions
reference (as `shared` gives them). Descriptions and other metadata do not count. A table whose
fingerprint no longer matches is **stale**: it is not read. A calculated table's data also depends
on the tables it reads, which the fingerprint does not follow: refresh again after changing them.

**Which data the session reads** (`dax.evaluate`, `report.render`, the viewer, edit-time checks):

1. an explicit `cache_abf` (session or command) when set: the snapshot is not read;
2. else the snapshot, when it exists and the project's `.pbi/cache.abf` is not newer (a refresh
   in Desktop afterwards wins): `cache.abf` is then not read at all, so tables never mix the two
   refreshes; tables missing from it or stale fall back to their partitions (which, with a
   connector, are unavailable: refresh again);
3. else `.pbi/cache.abf`, then the partitions, as before.

`dax.evaluate` reports `data_source: "bicraft-data"`. An unreadable snapshot is ignored (the
data falls back as if it were absent).

### Limits

- Databricks only; one SQL warehouse per query; no `Value.NativeQuery`, no `Databricks.Query`.
- No Arrow (`ARROW_STREAM`) results; inline results capped at 25 MiB per statement by the API
  (use `external_links`).
- Folding covers selections, simple filters and limits; joins, groupings and renames run locally
  over the fetched rows.
- The Databricks type mapping and folding were tested against a local fake of the API
  (`testsupport/fake_databricks.rs`, shapes from the public documentation), not yet against a
  workspace; Desktop's own connector was not compared.
- Corporate proxies and TLS interception are not supported (no proxy; the bundled Mozilla root
  certificates, `webpki-roots`).
- Incremental refresh policies, DirectQuery and Dual partitions are not refreshed.
- On a server the local-time clock functions (`DateTime.LocalNow`) read UTC (offset 0), as in
  the Power BI service, unless `--local-offset` or `TZ` says otherwise; the desktop app and the
  CLI read the machine's time zone, as Desktop does ("Clock", below).

## In the engine (`crates/engine/src/datasrc.rs`)

Every place the engine needs a partition's rows calls `load_partition_opts` / `load_m_with`:

- `table.import {m}` / `table.set_data {m}`: the query is evaluated to get the columns and rows,
  and kept as the partition expression;
- a structural PBIX save (`file.save`, `file.save_as`) of a new or replaced import table;
- the viewer's preview of a table without stored data (a PBIP without `cache.abf`, a new model).

`SourceOptions` come from the model and the session:

| Field | Value |
|---|---|
| `culture` | the model's `sourceQueryCulture`, else its `culture` |
| `shared` | the model's M `expressions` and the single M partition of every other table (by table name), restricted to the names the query mentions, directly or through each other (`#"name"` or a whole identifier). Unrelated queries are never parsed. |
| `file_roots` | the session's `data_root` as a one-folder allow-list: empty unless a command was given `data_root` (`file.open`, `file.save`, `file.save_as`, `table.import`, `table.set_data`, `bicraft view --data-root`). `file.open` without it resets it. Never inferred from the file's location. |
| `limits` | defaults, `max_rows` = the save's or the preview's row cap |
| `clock_utc_ms`, `clock_local_offset_minutes` | the time of the evaluation (`model.refresh`: its start time) and the local clock's offset at that instant: `DateTimeZone.UtcNow()` and `DateTime.LocalNow()` give the import, save or preview time (`table.add_refresh_info` relies on it) |

### Clock

`DateTime.LocalNow()`, `DateTimeZone.LocalNow()` and their `Fixed` forms are the UTC clock plus
an offset that comes from the **process's local clock** (`bicraft_engine::set_local_clock`,
`crates/engine/src/clock.rs`), read at the instant of each evaluation, so a refresh after a
daylight-saving change gets the new offset:

| Host | Local clock |
|---|---|
| `bicraft-desktop`, the CLI (`bicraft run`, `render`, `mcp`, `view`, …) | the machine's time zone, as Power BI Desktop (jiff's system time zone, `bicraft-locale` feature `clock`: on Windows the system's time zone, resolved in jiff's bundled tz database; on Unix `TZ`, then `/etc/localtime`, names resolved in `/usr/share/zoneinfo`; UTC when none is found) |
| `bicraft serve` (and the pods of `bicraft controller`) | **UTC**, as the Power BI service, unless `--local-offset` says otherwise: `±HH:MM` (`+02:00`), a whole number of minutes (`-300`), `UTC`, or `machine` (the host's zone). Without the flag, a non-empty `TZ` means `machine`. The container image has no zone files: give `TZ` as a POSIX rule (`CET-1CEST,M3.5.0,M10.5.0/3`) or use a fixed offset (Helm: `server.localOffset`, controller: `workspaces.pod.localOffset`) |
| a library caller (tests, other embedders) | UTC until it calls `set_local_clock` |

`model.refresh`'s `utc_offset_minutes` (-1080 to 1080), when given, replaces that offset for the
refresh, in its M queries and in the `NOW()`/`TODAY()` of its calculated tables and columns,
which otherwise read the same local clock as DAX measures (`docs/formats/dax-functions.md`,
"Calendar-based time intelligence").

An offset outside M's ±14:00, or a zone that cannot answer for that instant, is UTC. Tests:
`crates/engine/tests/local_clock.rs` (UTC by default, a fixed offset, an injected zone read at the
instant, out-of-range offsets), `crates/engine/src/clock.rs` (the `--local-offset` forms),
`apps/bicraft-cli/src/serve_cmd.rs` (`server_clock`: flag, `TZ`, default).

A query that calls `File.Contents` or `Folder.*` without a data root fails with the evaluator's message plus a
hint to pass `data_root`. The viewer reloads its partition data when the data root, the culture,
the expressions or any partition change.

### Stored text

Desktop 2.158.1304.0, our synthetic probe table (68 strings: case, accent, width and kana
variants, trailing and leading blanks, control characters) loaded by Power Query in en-US and
fr-FR models, `EVALUATE` of every row as code points, `VALUES` and `DISTINCTCOUNT`; then the same
table written by BIcraft into a PBIX, opened in Desktop and refreshed there. The values BIcraft
writes are identical to Desktop's own, row by row, in both cultures.

| Rule | Imported (M) | Calculated (DAX) |
|---|---|---|
| Control characters XML forbids (U+0000–U+0008, U+000B, U+000C, U+000E–U+001F, U+FFFE, U+FFFF) | removed anywhere (`"a\u{1}b"` → `"ab"`, `"\u{1}"` → `""`) | (DAX cannot produce them) |
| Trailing space, tab, CR, LF | removed (`"Q  "` → `"Q"`, `"   "` → `""`, not blank) | kept |
| Leading blanks; U+00A0, U+0085, U+2003, U+3000, U+200B, U+007F; inner tab, CR, LF | kept | kept |
| Equal values | one dictionary entry, spelled as the first one loaded (after the rules above: `"y "` then `"Y"` → both `"y"`) | same |
| en-US equality | ASCII case only: `apple` = `APPLE`, `à` ≠ `À`, `Ω` ≠ `ω` | same |
| fr-FR equality | the collation's: `à` = `À`, `Ω` = `ω`, `А` = `а`, `Œ` = `œ`; width (`Ａ`/`A`), kana, `é`/`e◌́`, `ß`/`ss` stay apart | same |

Other cultures are assumed to behave like fr-FR (not checked). DAX comparisons do not trim:
`'T'[S] = "APPLE "` matches nothing once `APPLE ` is stored as `apple`. The structural save, the
viewer's literal tables and the values given to the DAX evaluator during a save all go through
the same `bicraft_vertipaq::system::TextStorage`, so `DISTINCTCOUNT` over `a`, `A ` is 1 in the
viewer as in Desktop.

## DAX literal tables

Keywords are case-insensitive; comments `--`, `//`, `/* */`.

| Expression | Columns |
|---|---|
| `DATATABLE("A", STRING, "B", INTEGER, {{"x", 1}, {"y", 2}})` | declared: `STRING` String, `INTEGER` Int64, `DOUBLE` Double, `CURRENCY` Decimal, `DATETIME` DateTime, `BOOLEAN` Boolean; `{}` = no rows |
| `{1, 2, 3}` | `Value`, inferred |
| `{(1, "a"), (2, "b")}` | `Value1`, `Value2`, …, inferred |
| `ROW("A", 1, "B", "x")` | named, inferred, one row |
| `CALENDAR(DATE(2026,1,1), DATE(2026,12,31))` | `Date` (DateTime), one row per day, capped by `max_rows` |

Values: numbers (unary `+`/`-`; integers that overflow i64 become doubles), strings (`""`
escape), `TRUE`/`FALSE` with or without `()`, `BLANK()`, `DATE(y,m,d)` with DAX semantics
(years 0–1899 add 1900; months and days overflow into the next year/month), `TIME(h,m,s)` (hours
wrap at 24), `DT"2026-01-05[T12:00:00]"`. In `DATATABLE`, text is parsed into the declared type
(invariant culture: ISO dates or `m/d/yyyy`), numbers are formatted into `STRING` columns.
Inference for constructors and `ROW` is the same as for untyped M columns.

The auto date tables Desktop creates (`Calendar(Date(2015,1,1), Date(2015,1,1))` placeholders)
load; the real ones (`Calendar(Date(Year(MIN('T'[Date])), 1, 1), …)`) are `UnsupportedSource`,
as is anything with a table or column reference, an operator or another function.

## DAX calculated tables (in the engine)

A calculated table over the model's data (`SUMMARIZECOLUMNS`, `ADDCOLUMNS`, `FILTER`,
`SELECTCOLUMNS`, `GENERATESERIES`, `CALENDARAUTO`, `UNION`, `DISTINCT`/`VALUES`, `TOPN`, …) is
evaluated by the engine's DAX evaluator (`crates/engine/src/preview_eval*.rs`), not by
`bicraft-data`. The evaluator needs the session's stored data and the other tables, which only
the engine (layer L5) has, and the layer rules forbid `bicraft-data` (L2) from depending on it;
moving the evaluator down would split one 6 000-line module. So `bicraft-data` keeps the literal
tables and returns `UnsupportedSource` for the rest, and the engine takes over:

- **PBIX save** (`file.save`, `file.save_as`): a new or replaced calculated table is evaluated in
  an empty filter context; its columns (name, data type, source column) are created as the
  engine creates them when the table declares none (`crates/engine/src/pbix_struct.rs`).
- **`table.import {dax}` / `table.set_data {dax}`**: a literal table loads through
  `bicraft-data`; any other expression is evaluated over the open file's stored data (a PBIX's
  `DataModel`, a PBIP's `cache.abf`) and the tables BIcraft can load (M queries, literal tables).
- **Viewer**: calculated tables evaluate on demand.

**Engine rules** (Analysis Services of Power BI Desktop 2.158, our synthetic model
`crates/engine/tests/table_functions/model.bim` plus one calculated table per probe, recorded with
the maintainers' development oracles into `crates/engine/tests/calc_tables/expected.json`; the
test `calculated_tables_match_the_engine` checks all 66 probes: columns with name, data type and
source column, and the stored rows):

| Case | Engine |
|---|---|
| A column with lineage (`VALUES(T[C])`, a group-by column, an unnamed `SELECTCOLUMNS` column) | named `C` (`T_C` for `SELECTCOLUMNS`), source `T[C]` |
| A computed or explicitly named column | source `[Name]` |
| `UNION` of columns with different lineages | the first table's column (`Customer[City]`), also when the other is a literal; none when the first one has none or is explicitly named |
| A column of blanks only (`"X", BLANK()`) | whole number |
| Arithmetic types | as the evaluator computes them: Currency `+`/`-` whole number → Currency, Currency `+` double → double, Currency `*` double or whole number → Currency, Currency `*` Currency → double, Currency `/` whole number or double → Currency, `x /` Currency → double |
| `{1, "a"}`, `{1, 2.5}` | a text column (`"1"`, `"a"`), a double column |
| `CALENDARAUTO()` without dates in data tables | an empty table |
| `CALENDARAUTO()`, `CALENDARAUTO(6)` over an import table with 2022-03-04 and 2025-11-30 | 1 461 days from 2022-01-01, 1 826 days from 2021-07-01 (Desktop round 4) |
| `GENERATESERIES` with a negative step | refused |
| Text functions on a blank (`UPPER`, `LOWER`, `LEN`, `LEFT`, `RIGHT`, `MID`, `TRIM`, `SUBSTITUTE`, `REPT`, `CONCATENATE(BLANK(), BLANK())`, `BLANK() & BLANK()`) | blank; `""` stays `""`; `REPLACE(BLANK(), 1, 1, "x")` is `"x"`, `FORMAT(BLANK(), "0")` is `""` |
| `CALCULATE` in a row context | the row becomes a filter once; a measure inside does not transition again (`CALCULATE([Total], ALL(Sales))` in `ADDCOLUMNS` is the grand total) |
| Nested row contexts on the same column | the inner one wins (`RANKX(ALL(T[C]), [M])` in `ADDCOLUMNS(VALUES(T[C]), …)`) |

The text, conversion and date-part functions calculated tables commonly use were added to the
evaluator for this (`crates/engine/src/preview_eval_text.rs`): `UPPER`, `LOWER`, `LEN`, `LEFT`,
`RIGHT`, `MID`, `TRIM`, `SUBSTITUTE`, `REPLACE`, `REPT`, `CONCATENATE`, `EXACT`, `FIND`, `SEARCH`,
`CONTAINSSTRING`, `CONTAINSSTRINGEXACT`, `UNICHAR`, `UNICODE`, `VALUE`, `FORMAT`, `WEEKDAY`,
`WEEKNUM`, `HOUR`, `MINUTE`, `SECOND`, `TRUNC`, `ROUNDUP`, `ROUNDDOWN`, `SQRT`, `POWER`, `MOD`.
`FORMAT` covers the named formats (`General Number`, `Currency`, `Fixed`, `Standard`, `Percent`,
`Short Date`, `Long Date`, `Medium Date`, `General Date`, the time formats, `Yes/No`, …) and
custom number (`0`, `#`, `,`, `.`, `%`, sections, quoted text) and date/time (`yyyy`, `mmmm`,
`dddd`, `q`, `hh:nn`, `AM/PM`, …) format strings, plus scientific formats, in the model's
culture or the culture argument (crate `bicraft-locale`; agreement with the engine in 59
cultures in [locales.md](locales.md)). `SEARCH` and `CONTAINSSTRING` wildcards and `FORMAT` in a
language BIcraft has no data for are refused, not guessed.

## Mapping onto the model (`conform`)

For each model column **without** a DAX `expression` (data columns and calculated-table columns):

1. The source name is `sourceColumn` (with `[...]` / `Table[...]` brackets of calculated-table
   columns removed), else the column name. It is looked up exactly, then case-insensitively;
   missing → `Error::MissingColumn`.
2. Values are converted to the declared `dataType` (`Unknown` keeps the source type). Errors are
   `Error::Value { row, column }` with the model column name and the 1-based row.

Calculated columns (with an `expression`) are not produced: the engine computes them. Source
columns no model column reads are dropped. Two model columns may read the same source column.

### Coercion

| From → To | Rule |
|---|---|
| blank → any | blank |
| Int → Double | `n as f64` |
| Decimal → Double | `n / 10 000` |
| Real → Int64 | only when integral and in range |
| Decimal → Int64 | only when integral |
| Int → Decimal | `n × 10 000`, range-checked |
| Real → Decimal | rounded to 4 places, range-checked |
| anything → String | invariant text: `1`, `1.5`, `true`/`false`, ISO dates, minimal decimals (`conform` and DAX only, not typed M columns) |
| Text → other | parsed with the invariant culture (`conform` and `DATATABLE` only) |
| Bool ↔ number, number → DateTime, DateTime → number | error |

## Hostile input

`Limits` (defaults). Exceeding any of them is `Error::LimitExceeded`:

| Limit | Default | Covers |
|---|---|---|
| `max_bytes` | 256 MiB | CSV input, M/DAX expression, files read by M, decompressed binaries |
| `max_rows` | 10 000 000 | rows, list items |
| `max_columns` | 4096 | columns |
| `max_cell_bytes` | 1 MiB | one CSV field or text value |
| `max_depth` | 64 | M/DAX parse nesting |
| `max_eval_depth` | 2000 | nested M calls, step references and recursion |
| `max_steps` | 500 000 000 | units of M work: expression steps, rows and items touched |
| `max_millis` | 120 000 | wall-clock time of one M evaluation (0 = none) |
| `max_file_bytes` | 64 GiB | CSV files streamed by the fast path (*Fast CSV loading*) |
| `max_load_bytes` | 16 GiB | memory of one fast CSV load (cells, texts, buffers) |

The M evaluator runs on its own thread with a 256 MiB stack, so `max_eval_depth` is reached
before the stack runs out. `try` does not catch these errors.

Parsers never index without bounds checks, and arithmetic on parsed numbers is checked or
range-validated first. Date arithmetic saturates and is then range-checked. The tests fuzz:

- random-byte CSV input
- random-token and random-byte M programs
- extreme M values: huge durations, counts and offsets
