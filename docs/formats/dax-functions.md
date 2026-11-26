# DAX function catalog

`crates/dax/src/catalog/` holds a typed `Signature` (vocabulary in `crates/dax/src/types.rs`) for
every DAX function. `functions::signature(name)` returns it (case-insensitive, binary search over a
sorted index); `functions::lookup` (argument counts), `declares_columns` and `is_keyword_argument`
are derived from it.

Engine checked: Power BI Desktop (Store) **2.158.1304.0**, local Analysis Services instance,
2026-10-13. Public reference checked: learn.microsoft.com DAX function reference table of
contents, same day.

Sources: the public DAX reference, the engine's `MDSCHEMA_FUNCTIONS` DMV and black-box probes of
the engine of Power BI Desktop 2.158. Every verdict is an engine answer. Detailed provenance is
kept privately by the maintainers.

## Coverage

| Source | Functions |
|---|---|
| Public DAX reference (all categories, `DISTINCT` counted once) | 460 |
| Engine `$SYSTEM.MDSCHEMA_FUNCTIONS`, DAX rows (`ORIGIN` 3 and 4; `ORIGIN` 1 is MDX) | 470 |
| **Catalog (union)** | **472** |

- In the reference but not in the DMV: `TEXTCONTAINS`, `TEXTSIMILARITY`. The engine still knows
  them (argument-count probe: 2 to 3), it just does not list them.
- In the DMV but not in the reference (12): the visual-calculation functions documented with Power
  BI visual calculations (`COLLAPSE`, `COLLAPSEALL`, `EXPAND`, `EXPANDALL`, `FIRST`, `ISATLEVEL`,
  `LAST`, `NEXT`, `PREVIOUS`; category `visual calculations`) and three functions the DMV lists
  that the reference does not document and that visuals generate (`NATURALJOINUSAGE`,
  `SAMPLEAXISWITHLOCALMINMAX`, `TOPNSKIP`; the catalog files them under `undocumented`).

Per category (the reference's categories, lower-case):

| Category | Count | Category | Count |
|---|---|---|---|
| aggregation | 22 | math and trig | 49 |
| date and time | 24 | other | 6 |
| filter | 31 | parent and child | 5 |
| financial | 51 | relationship | 4 |
| info (`INFO.*`) | 72 | statistical | 44 |
| information | 45 | table manipulation | 30 |
| logical | 15 | text | 21 |
| time intelligence | 41 | visual calculations | 9 |
| undocumented | 3 | | |

No function is flagged `deprecated`: the current reference marks none (the only "deprecated"
mentions are result columns of `INFO.STORAGETABLES`/`INFO.TABLESTORAGES`, and "not recommended"
notes on time-intelligence functions apply to visual calculations only).

## Engine comparison

**Function list.** `SELECT * FROM $SYSTEM.MDSCHEMA_FUNCTIONS` over ADOMD.NET
(the ADOMD.NET client shipped with Desktop, `Locale Identifier=1033` so
parameter names come back in English instead of the UI language). Each row has a nested
`PARAMETERINFO` rowset with `NAME`, `OPTIONAL`, `REPEATABLE`, `REPEATGROUP` and `SKIPPABLE`; the
catalog's parameter names are those names in snake_case, optional/repeating flags follow them.

**Argument counts.** For each of the 472 functions, `EVALUATE {F()}` and `EVALUATE {F(1,…)}` with
40 and 41 arguments; the engine answers "Too few arguments … minimum argument count is N" and
"Too many arguments … maximum argument count is N" before any type check, so min and max can be
read off. No count error at 40 arguments means variadic. The results are the `ENGINE` table in
`catalog/tests.rs`; the test requires `functions::arity(signature)` to equal them.

- 467 of 472 match exactly.
- 5 deliberate deviations (listed in the test with reasons):
  - `TRUE`, `FALSE` (`TRUE(1)` is a syntax error) and `NOT` (operator form): probe artifacts.
  - `USERNAME`: the engine ignores extra arguments; the catalog keeps the documented 0.
  - `RANK`: resolved by shape, so the engine reports "argument unused" instead of a count; the
    catalog keeps the documented 7 parameters.
- `SUMMARIZECOLUMNS` (engine min 1), `GROUPBY` and `SUMMARIZE` (engine min 2) used to deviate:
  their sections are all optional, yet the engine wants one argument in them. `Signature::min_args`
  now carries that minimum (2026-10-13, G19).

Shape checks run the same way (`EVALUATE <expr>`): `IF`, `IF.EAGER`, `SWITCH`, `IFERROR` and
`COALESCE` cannot return tables; `EVALUATEANDLOG` and `DISTINCT` accept table expressions,
`VALUES` only a table or column reference; `TOPN` accepts `0`/`TRUE` as sort orders.

## Conventions

**Parameter layout.** `params[..repeat_from]` is a fixed prefix. `params[repeat_from..]` starts
with the *repeating unit*, the run of params with `repeating: true`, repeated as a whole; it may
be followed by a non-repeating *tail* that comes after the last repetition:

| Function | Prefix | Unit | Tail |
|---|---|---|---|
| `SWITCH` | `expression` | `value, result` | `else?` |
| `LOOKUPVALUE` | `result_column` | `search_column, search_value` | `alternate_result?` |
| `CALCULATE` | `expression` | `filter?` | |
| `TOPN` | `n, table` | `order_by?, order?` | |
| `DATATABLE` | | `name, data_type` | `data` |
| `LINEST` | `column_y` | `column_x` | `const?` |

- The unit is mandatory (one occurrence at least) iff it contains a required param. Optional
  params inside the unit may be left out of any occurrence (`TOPN`'s `order`); arguments are then
  matched by shape (a sort keyword is not an expression).
- Outside the unit, optional params are trailing. Exceptions, matched by shape: `WINDOW`
  (`from_type`/`to_type` are `ABS`/`REL` keywords before the required `to`), `COLLAPSEALL` and
  `EXPANDALL` (the engine marks `expression` optional, `axis` required), `ROLLUPADDISSUBTOTAL` and
  `ROLLUPISSUBTOTAL` (an optional grand-total filter table before the group-by columns).
- Multi-section functions keep their params in one unit of optional params, in engine order, and
  `Param::group` gives each the engine's repeat group (`REPEATGROUP` of `MDSCHEMA_FUNCTIONS`):
  `SUMMARIZECOLUMNS` (`group_by_column` 1, `filter_table` 2, `name`/`expression` 3), `SUMMARIZE`
  and `GROUPBY` (`group_by_column` 1, `name`/`expression` 2), `ADDMISSINGITEMS`
  (`show_all_column` 1, `group_by_column` 2, `filter_table` 3). Every other param has group 0.
  The type checker matches these arguments by shape, not by position.
  `SAMPLEAXISWITHLOCALMINMAX`'s second repeating section (`dynamic_series`, which the engine
  lists without a group) is kept as a single param.
- Arity: `min` = `Signature::min_args` when set (the multi-section functions), else the number of
  required params (a mandatory unit counts once); `max` = number of params, or none when the
  signature repeats.

**Kinds.**
- Numbers are `Scalar(Decimal)` (any number); `Scalar(Integer)` where a whole number is expected
  (counts, positions, `basis`, `frequency`). Implicit DAX conversions still apply.
- `RowExpression` for arguments evaluated in a row context the function creates (`SUMX`
  expression, `FILTER` condition, `ADDCOLUMNS`/`SELECTCOLUMNS` expressions, `TOPN`/`ORDERBY`
  order-by expressions, `FIRSTNONBLANK` expression). `SUMMARIZE`/`SUMMARIZECOLUMNS`/`GROUPBY`/`ROW`
  expressions are plain scalars.
- `Filter` for `CALCULATE`/`CALCULATETABLE` filters and the `filter` param of the time-intelligence
  totals and balances; `KEEPFILTERS` takes a `Filter`.
- `NewColumnName` for the string literals that name new columns; `declares_columns` is "has a
  `NewColumnName` param".
- `Keyword(set)` for bare keywords: sort order (`ASC`/`DESC`; the engine also accepts
  `0`/`FALSE`/`1`/`TRUE`), ties (`SKIP`/`DENSE`), intervals, `DATEDIFF` intervals, cross-filter
  directions, blanks (`DEFAULT`/`FIRST`/`LAST`), `WINDOW` bound types, visual axes, data types,
  `DATEADD` extension/truncation, `DATESINPERIOD` end behavior, `TEXTCONTAINS` match modes,
  `NAMEOF` component/escaping, `LOOKUP` associated-columns behavior. `is_keyword_argument` is the
  union of all sets plus the keywords of `Any` params (`ROWS`, `COLUMNS`, `NONE`, `LOWESTPARENT`,
  `HIGHESTPARENT`).
- `Dates` for time-intelligence `dates` (a date column, a one-column table of dates with a model
  lineage, or a calendar reference), `Calendar` for calendar references, `MeasureRef` for measure
  references (`DETAILROWS`, `ISSELECTEDMEASURE`), `TableOrColumn` for `DISTINCT`, `Relation` for a
  window function's relation, `OrderBy`/`PartitionBy`/`MatchBy` for the window clauses, and
  `KeywordOrScalar` for sort orders (`ASC`/`DESC` or `0`/`1`/`FALSE`/`TRUE`). These kinds were
  added on 2026-10-13 (G19; additive to `types.rs`).
- `Any` where the vocabulary still has no exact kind: visual-calculation `reset`, the first
  argument of `MIN`/`MAX` (a column, or a scalar in the two-argument form),
  `SAMPLEAXISWITHLOCALMINMAX`'s axis and measures.

**Returns.**
- `SameAsArg`/`CommonOfArgs` indices are *parameter* indices. Every argument bound to a repeating
  param counts (`SWITCH` is `CommonOfArgs([2, 3])`: every `result` plus `else`), and omitted
  arguments are ignored (`MAX` is `CommonOfArgs([0, 1])`).
- `Fixed(Unknown)` for calls that are not values: filter modifiers only valid in `CALCULATE`
  (`USERELATIONSHIP`, `CROSSFILTER`, `REMOVEFILTERS`, `ALLCROSSFILTERED`), the `ORDERBY`/
  `PARTITIONBY`/`MATCHBY` clauses, the `ROLLUP*` group-by modifiers and `NATURALJOINUSAGE`.
- `Fixed(Scalar(Variant))` when the type depends on runtime values or on a keyword argument
  (`CONVERT`, `SELECTEDMEASURE`, `PATHITEM`, `BLANK`, `ERROR`, `EXTERNALMEASURE`).

## Column variations (auto date/time)

With auto date/time on, Desktop gives every date column a TMSL `variations` entry
(`{name, relationship, defaultHierarchy: {table, hierarchy}, isDefault}`) pointing, through a
relationship from the column (`joinOnDateBehavior: datePartOnly`), to a hidden
`LocalDateTable_<guid>` (columns `Date`, `Year`, `MonthNo`, `Month`, `QuarterNo`, `Quarter`, `Day`;
hierarchy `Date Hierarchy`; `showAsVariationsOnly`). DAX reads `'Sales'[OrderDate].[Year]` as a
column of that table. Observed on the engine of Desktop 2.158.1304.0 (private instance, a model
built that way; corpus category `variations` of the DAX oracle):

| Rule | Engine behavior |
|---|---|
| Syntax | `<column reference>.[<name>]`; spaces around `.` allowed; `.Name` without brackets and a second level (`.[Date].[Year]`) are syntax errors; `[Col].[Year]` without a table works |
| Name resolution | `<name>` is a **column** of the variation's table (the relationship's other end), case-insensitive: `[MonthNo]`, `[QuarterNo]` and `[Date]` resolve although they are not hierarchy levels, and a level renamed `Yr` is not found (`[Yr]` fails, `[Year]` still works) |
| Errors | unknown name → "variation column not found"; a column without variations (a non-date column, a date column without auto date/time, the local table's own `Date`) → "cannot be used with a variation"; a measure → "variation notation cannot be used for a measure" |
| Type | the target column's type (`[Year]` int64, `[Month]` string, `[Date]` dateTime) |
| Aggregation | `MAX('Sales'[OrderDate].[Day])` aggregates the whole local date table (31), not only dates present in `Sales` |
| Row context | a row context over the variation's table **or over the base column's table** gives a value (`SUMX(Sales, Sales[OrderDate].[MonthNo])` follows the relationship, like `RELATED`); `RELATED(Sales[OrderDate].[Year])` is an error |
| Filters | a boolean filter on a variation filters the variation's table: combined with a column of the base table in one filter it is the "columns from multiple tables" error; with another column of the local table it is fine |
| Time intelligence | `DATESYTD('Sales'[OrderDate].[Date])`, `TOTALYTD`, `SAMEPERIODLASTYEAR`, `DATEADD` accept `.[Date]`; `DATESYTD(….[Year])` is accepted at definition and fails at evaluation (not a date column) |
| Queries | `SUMMARIZECOLUMNS('Sales'[OrderDate].[Year], …)` returns a column named after the local table's column |

BIcraft: the parser keeps the variation on `Expr::Column`; the binder resolves it through
`SemanticModel::variation_column` (`unknown-variation`, `column-without-variations`,
`variation-on-measure`); the type checker types it as the target column and accepts a row context
over either table; the dependency graph records both columns; the viewer evaluator reads the
target column and, in a row context of the base table only, follows the relationship. TMSL and TMDL
keep `variations` in the column's `extra`; PBIX metadata maps `Variation` rows to the same shape
(see metadata-sqlitedb.md).

## Currency arithmetic

Currency (`Fixed Decimal Number`, TMSL `decimal`) is a 64-bit integer of 1/10 000 units. Engine of
Desktop 2.158.1304.0, `EVALUATE ROW(…)` with the result type read from the client (Decimal =
Currency, Double = real):

| Expression | Result type | Value rule |
|---|---|---|
| `c + c`, `c - c`, `c + int`, `BLANK() + c`, `c * TRUE()` | Currency | exact |
| `c + real`, `c - real`, `c + "2.5"` | Double | no rounding |
| `c * int`, `c * real`, `real * c` | Currency | rounded (`1.2345 * 1.1111` → 1.3717) |
| `c * c` | Double | |
| `c / int`, `c / real`, `DIVIDE(c, int)`, `DIVIDE(c, real)` | Currency | rounded (`10 / 3` → 3.3333) |
| `c / c`, `int / c`, `real / c`, `DIVIDE(c, c)` | Double | |
| `c / 0`, `c / 0.0` | error (division by zero) | `DIVIDE` gives blank |
| `c ^ int` | Double | |
| `-c`, `ABS(c)`, `INT(c)`, `TRUNC(c)`, `MOD(c, 3)`, `ROUND(c, 2)` | Currency | |
| `c + date` | DateTime | |
| `MIN(c, real)`, `MAX(c, real)`, `IF(…, c, real)` | Double | value already rounded |
| `SUM`, `MIN`, `MAX`, `MEDIAN`, `AVERAGE`, `STDEV.S` of a Currency column | Currency | `AVERAGE` = 982.45 / 16 = 61.403125 → 61.4031 |
| `VAR.S` of a Currency column | Double | |
| `SUMX` / `AVERAGEX` / `MAXX` over Currency values | Currency | each row rounded, then the total |
| `DIVIDE(SUM(c), COUNTROWS(t))`, `SUM(c) / SUM(real)` | Currency | rounded |
| `CURRENCY(x)` | Currency | the exact double rounded: `CURRENCY(0.00015)` → 0.0001 (the double is below the tie), `CURRENCY(0.00025)` → 0.0003 |
| `int * c` beyond ±922 337 203 685 477.5807 | Currency, wraps silently; `c * real` overflowing is an error | |

Rounding is half away from zero on the exact scaled result: `CURRENCY(0.0001) / 2` → 0.0001,
`CURRENCY(0.0003) / 2` → 0.0002, `CURRENCY(0.0005) / 2` → 0.0003, `CURRENCY(-0.0003) / 2` →
−0.0002, `CURRENCY(-2.5) / 10000` → −0.0003. The arithmetic is done on the scaled integer
(`3 / 2 = 1.5 → 2`), not on the double value of the Currency (which would give 0.0001 for
`CURRENCY(0.0003) / 2`).

BIcraft's viewer evaluator (`crates/engine/src/preview_currency.rs`) follows this table in the fast
path and the general evaluator. One approximation: a double operand holding a whole value counts as
a whole number (`c + 2.0` stays Currency where the engine gives a Double; the value is the same).
`c * int` overflow is an error instead of wrapping.

## Gaps in the type vocabulary

`types.rs` only takes additive changes. Done on 2026-10-13 (G19): the per-param repeat group
(`Param::group`), the minimum argument count of multi-section functions (`Signature::min_args`),
and the kinds `Dates`, `Calendar`, `MeasureRef`, `TableOrColumn`, `Relation`, `OrderBy`,
`PartitionBy`, `MatchBy` and `KeywordOrScalar`.

Filter modifiers (2026-10-14) are typed inside the type checker, not in `types.rs`: their
`Returns` stay as above, so `signature()` and the serialized result types do not change. The type
checker marks a call to `USERELATIONSHIP`, `CROSSFILTER`, `REMOVEFILTERS`, `ALLCROSSFILTERED`,
`KEEPFILTERS`, `NONVISUAL`, or `ALL`/`ALLSELECTED` without arguments as a modifier and checks where
it stands (`crates/dax/src/typecheck_modifiers.rs`; the engine's placement rules are in
`docs/dax-oracle.md`).

Still open: `SAMPLEAXISWITHLOCALMINMAX`'s two repeating sections, which the engine lists without
groups.

## Reserved words

Besides DAX keywords and older function names, the engine also refuses many MDX and
DMX keywords. An unquoted reserved word is a syntax error in these
places (Power BI Desktop 2.158.1304.0, Analysis Services 17.0.87.18, probed on 2026-10-14):

| Place | Example | Refused for |
|---|---|---|
| Column qualifier | `SUM(Typed[N])` | every reserved word |
| Table of a query definition | `DEFINE TABLE Status = …`, `MEASURE Date[m] = …` | every reserved word |
| Variable, function or parameter name | `VAR Value = 1`, `FUNCTION F = (Typed) => …` | every reserved word, plus `DENSIFY` and `ROWPAGES` |
| Bare table reference | `COUNTROWS(Typed)`, `EVALUATE Status`, `FILTER(Name, …)` | every reserved word except `BOOLEAN`, `CHAPTERS`, `COLUMNS`, `DATE`, `DOUBLE`, `LOG`, `LONG`, `ROWS`, `TEXT`, `VALUES` |

Quoting always works (`'Typed'[N]`, `COUNTROWS('Status')`). Column and measure names are bracketed
and never clash (`[Typed]`, `"Typed"` in `ADDCOLUMNS`). Model measures and queries behave the
same.

The list (`crates/dax/src/reserved.rs`, 568 words) holds:
- the older DAX functions (`SUM`, `FILTER`, `VALUES`, `DATE`, `RANK`, the time-intelligence
  functions…);
- the query keywords (`DEFINE`, `EVALUATE`, `ORDER`, `BY`, `START`, `AT`, `ASC`, `DESC`…);
- MDX and DMX words, many of which are common table names: `Name`, `Key`, `ID`, `Type`, `Typed`,
  `Status`, `Version`, `Description`, `Caption`, `Action`, `Address`, `Language`, `Server`,
  `Url`, `Weight`, `Children`, `Content`, `Level`, `Member`, `Cube`, `Model`, `Session`…

Not reserved: newer functions (`INDEX`, `WINDOW`, `OFFSET`, `CALENDAR`, `YEAR`, `CURRENCY`…) and the
words of the user-defined-function syntax (`FUNCTION`, `VAL`, `EXPR`, `SCALAR`, `ANYVAL`, `ANYREF`,
`INT64`, `DECIMAL`, `STRING`, `NUMERIC`, `DATETIME`, `VARIANT`, `TABLEREF`…). `TABLE` is
reserved, but as a type hint after `:` it is fine.

How it was found. The first list (147 words, 2026-10-13) probed only the function names of
`MDSCHEMA_FUNCTIONS` and the DAX keywords, and missed the MDX/DMX words (`Typed[N]` was a false
accept). The second probe took a much larger list of candidate words covering
the DAX, MDX and DMX vocabularies and asked the engine about each
with `EVALUATE ROW("x", VAR <w> = 1 RETURN <w>)` and `DEFINE TABLE '<w>' = {1} EVALUATE
ROW("x", SUM(<w>[Value]))`. The 570 words refused there were then probed in every place of the
table above, and a deployed model with a table named `Typed`, `Status`, `Name` or `Date` confirmed
the measure verdicts. Every verdict is the engine's answer; only the resulting word list is kept,
no engine text is.

## Evaluator coverage

The catalog above is what the **validator** knows. The **evaluator** (`dax.evaluate`, the viewer,
calculated tables and columns; `crates/engine/src/preview_eval*.rs`) evaluates a subset; anything
else is refused with an error naming the function (`not supported by bicraft's DAX evaluator:
F() is not supported`), never approximated. Values, blanks, errors and result types are checked
against the engine of Power BI Desktop 2.159 (`tests/dax_coverage`, see `docs/dax-oracle.md`).

Added on 2026-10-23 (branch `feat/dax-coverage`):

| Module | Functions |
|---|---|
| `preview_eval_math.rs` | `PI`, `SIN`, `COS`, `TAN`, `ASIN`, `ACOS`, `ATAN`, `SINH`, `COSH`, `TANH`, `ASINH`, `ACOSH`, `ATANH`, `COT`, `COTH`, `DEGREES`, `RADIANS`, `EXP`, `LN`, `LOG`, `LOG10`, `SQRTPI`, `FACT`, `COMBIN`, `COMBINA`, `PERMUT`, `GCD`, `LCM`, `EVEN`, `ODD`, `QUOTIENT`, `CEILING`, `FLOOR`, `ISO.CEILING`, `MROUND`, `BITAND`, `BITOR`, `BITXOR`, `BITLSHIFT`, `BITRSHIFT`, `ISEVEN`, `ISODD`, `ISNUMBER`, `ISTEXT`, `ISNONTEXT`, `ISLOGICAL`, `ISERROR`, `IFERROR`, `CONVERT`, `TIME`, `TIMEVALUE`, `DATEVALUE`, `YEARFRAC`, `NETWORKDAYS`, `FIXED`, `PATHITEM`, `PATHITEMREVERSE`, `PATHLENGTH`, `PATHCONTAINS`, `COMBINEVALUES` |
| `preview_eval_more.rs` | `LOOKUPVALUE`, `CONTAINS`, `CONTAINSROW`, `HASONEFILTER`, `ISCROSSFILTERED`, `FILTERS`, `FIRSTNONBLANK`, `LASTNONBLANK`, `FIRSTNONBLANKVALUE`, `LASTNONBLANKVALUE`, `RELATEDTABLE`, `PRODUCT`, `PRODUCTX`, `GEOMEAN`, `GEOMEANX`, `MEDIANX`, `PERCENTILE.INC`, `PERCENTILE.EXC`, `PERCENTILEX.INC`, `PERCENTILEX.EXC`, `STDEVX.S`, `STDEVX.P`, `VARX.S`, `VARX.P`, `MAXA`, `MINA`, `AVERAGEA`, `RANK.EQ`, `ERROR` |
| `preview_eval_calc.rs` | calculation groups (items applied to measure references, precedence), `SELECTEDMEASURE`, `SELECTEDMEASURENAME`, `ISSELECTEDMEASURE`, `SELECTEDMEASUREFORMATSTRING`, dynamic format strings, `[_<measure> FormatString]` |
| general evaluator | `IGNORE` (in `SUMMARIZECOLUMNS`), `SAMPLEAXISWITHLOCALMINMAX` (a table not larger than its size), `UTCNOW`/`UTCTODAY` apart from `NOW`/`TODAY` (`utc_offset_minutes`) |

Added on 2026-11-18 (ROADMAP M11, lot 20 of `docs/target-app-parity.md`):

| Module | Functions |
|---|---|
| `preview_eval_window.rs` | `OFFSET`, `INDEX`, `WINDOW` (`ABS`/`REL` bounds), `RANK` (`SKIP`/`DENSE`), `ROWNUMBER`, with `ORDERBY` (`ASC`/`DESC` and a blanks keyword per expression), `PARTITIONBY`, `MATCHBY` and the `DEFAULT`/`FIRST`/`LAST` blanks argument; `GROUPBY` with `CURRENTGROUP()`; `NATURALINNERJOIN`, `NATURALLEFTOUTERJOIN`; `OPENINGBALANCEMONTH`, `OPENINGBALANCEQUARTER`, `OPENINGBALANCEYEAR`, `CLOSINGBALANCEMONTH`, `CLOSINGBALANCEQUARTER`, `CLOSINGBALANCEYEAR` |

Their semantics come from Microsoft Learn's DAX reference (the module header lists the rules
kept): a relation left out is `ALLSELECTED` of the `ORDERBY`/`PARTITIONBY` columns; the current
row comes from the outer value of each key column (the innermost row context, else the column's
one value in the filter context; a column with several values gives the union of the rows, or a
blank `RANK`/`ROWNUMBER` when they disagree); order ties are broken by the fewest relation
columns that tell the rows apart (`MATCHBY` columns when given), and rows nothing tells apart are
an error; `DEFAULT` blanks sort a blank number between the negative numbers and zero and a blank
text first. `ALL(table)` brings the table's virtual blank row into the relation (the engine
counts it: `fn-rownumber`, `fn-window`). The 15 entries of `tests/dax_coverage` for these
functions agree with the engine; the other cases (`crates/engine/tests/model/dax_window_tests.rs`:
running totals, previous rows, ties and partitions, first and last rows, `GROUPBY` over related
columns, balances with a fiscal year end, joins through relationships, hostile bounds) are
**documented, not oracle-checked**. The visual-calculation forms (axes, `reset`) are an error
outside a visual calculation and the `*WEEK` balances take a calendar (both since 2026-11-25,
below). Each call sorts its relation again
(no memo across the cells of a visual), so a window over a large relation in every cell is
bounded by the evaluator's work cap.

Added on 2026-11-20 (ROADMAP M11, parity row "Every DAX function a measure uses is evaluated"):

| Module | Functions |
|---|---|
| `preview_eval_pick.rs` | `ADDMISSINGITEMS`, `SUBSTITUTEWITHINDEX`, `SAMPLE`, `TOPNSKIP` |

Sources: Microsoft Learn's DAX reference pages
[ADDMISSINGITEMS](https://learn.microsoft.com/en-us/dax/addmissingitems-function-dax),
[SUBSTITUTEWITHINDEX](https://learn.microsoft.com/en-us/dax/substitutewithindex-function-dax) and
[SAMPLE](https://learn.microsoft.com/en-us/dax/sample-function-dax); `TOPNSKIP` has no Learn page,
its parameters come from the engine's function list (`$SYSTEM.MDSCHEMA_FUNCTIONS`, the
`undocumented` catalog) and its behaviour from the public description on
[dax.guide](https://dax.guide/topnskip/). The rules kept:

- `ADDMISSINGITEMS(show_all…, table, group_by… [, filter_table]…)`: the rows of `table`, then one
  row per combination of the show-all columns that no row of `table` has on the group-by columns,
  with that combination in those columns and blank in the others (the measures). The combinations
  are what `SUMMARIZECOLUMNS` of the show-all columns and the filter tables returns: existing
  combinations within a table, crossed between tables, under the filter context. Show-all columns
  left out are the group-by columns. The subtotal form (`ROLLUPISSUBTOTAL`, `ROLLUPGROUP`): see
  "Added on 2026-11-25" below.
- `SUBSTITUTEWITHINDEX(table, name, index_table [, order_by [, order]]…)`: a left semijoin on the
  columns both tables have (same name, same kind of data type: text, number, date, boolean); left
  rows without a match are dropped; the common columns are replaced by one `Int64` column `name`,
  the 0-based position of the matching `index_table` row sorted by the keys, **descending when the
  order is left out** (as the reference says). The index column takes the place of the first
  common column (the reference does not say where it goes, nor what happens without a common
  column: an error here).
- `SAMPLE(n, table, order_by [, order]…)`: the rows sorted by the keys, then the first, the last
  and evenly spaced rows between them (row `round(i·(len−1)/(n−1))`, `i = 0…n−1`, rounded half
  up; the dax.guide example, 3 rows of 8 at positions 1, 5 and 8, agrees); every row when `n` is
  at least the row count, the first row when `n` is 1, empty when `n` is 0 or less or blank. The
  reference gives no default order: ascending here (unconfirmed). Without an order key the
  reference's sample is random: refused.
- `TOPNSKIP(n, skip, table [, order_by [, order]]…)`: the rows sorted by the keys (descending by
  default; the table's order without keys), the first `skip` left out, then at most `n`; rows that
  only tie on the keys are not added (unlike `TOPN`), rows identical to the last one kept are. The
  `ties` argument (`SKIP`/`DENSE`): see "Added on 2026-11-25" below.
- Counts are truncated to whole rows; blank, negative and NaN counts are 0. Sorting is DAX's
  order (blank first ascending), stable: rows the keys do not tell apart keep the table's order.

The four `tests/dax_coverage` entries for these functions (`fn-addmissingitems`,
`fn-substitutewithindex`, `fn-sample`, `fn-topnskip`, row counts recorded on Desktop 2.159) agree
with the engine; the other cases (`crates/engine/tests/model/dax_pick_tests.rs`: positions, orders,
blanks, ties, semijoins, crossed show-all columns, filter tables, empty tables, hostile counts) are
**documented, not oracle-checked**.

Added on 2026-11-25 (gap G30's leftovers):

| Module | Functions and forms |
|---|---|
| `preview_eval_pick.rs` | `ADDMISSINGITEMS` with `ROLLUPISSUBTOTAL` (and `ROLLUPGROUP`), the `ties` argument of `TOPNSKIP` |
| `preview_eval_calendar.rs` | `OPENINGBALANCEWEEK`, `CLOSINGBALANCEWEEK`; the month, quarter and year balances over a calendar |
| `preview_eval_window.rs` | the visual-calculation forms of `OFFSET`, `INDEX`, `WINDOW`, `RANK`, `ROWNUMBER` outside a visual calculation (an error) |
| `preview_eval_text.rs` | wildcards of `SEARCH` and `CONTAINSSTRING` |
| `preview_filters.rs` | `NOW`/`TODAY` and `UTCNOW`/`UTCTODAY` without `utc_offset_minutes` |

Sources: Microsoft Learn's DAX reference pages
[ADDMISSINGITEMS](https://learn.microsoft.com/en-us/dax/addmissingitems-function-dax),
[ROLLUPISSUBTOTAL](https://learn.microsoft.com/en-us/dax/rollupissubtotal-function-dax),
[CLOSINGBALANCEWEEK](https://learn.microsoft.com/en-us/dax/closingbalanceweek-function-dax),
OPENINGBALANCEWEEK, OFFSET (its `axis` and `reset` parameters, "available in visual calculations
only"), SEARCH and CONTAINSSTRING (wildcards), the Power BI page "Implement time-based
calculations in Power BI" (calendar-based time intelligence: categories, period uniqueness,
context clearing), and for `TOPNSKIP` the public description on
[dax.guide](https://dax.guide/topnskip/). The rules kept:

- `ADDMISSINGITEMS(show_all…, table, ROLLUPISSUBTOTAL([grand_total_filter,] group, [is_subtotal]
  [, [level_filter,] group, [is_subtotal]]…), …)`: each group (a column or a `ROLLUPGROUP` of
  columns) is a subtotal level whose rows have the table's `is_subtotal` column TRUE. The levels
  nest as `ROLLUPADDISSUBTOTAL` builds them: the detail rows (every flag FALSE), then the rows
  with the last level rolled up, …, then every level rolled up. At each level, the combinations
  of the show-all columns still grouped there (from `SUMMARIZECOLUMNS` of them and the filter
  tables) that no row of the level has are added, with blank in the rolled-up columns and the
  measures and the level's flags; a level whose show-all columns are all rolled up gets its one
  row when the table has none (the total row of a `SUMMARIZECOLUMNS` whose values are all blank).
  Plain group-by columns stay grouped at every level. Unconfirmed: the engine's order of the
  added rows (here after the table's, level by level), and whether a missing total row is added.
  The filter arguments of `ROLLUPISSUBTOTAL` and a second `ROLLUPISSUBTOTAL`: see "Calendar-based
  time intelligence" below (added later the same day).
- `TOPNSKIP(…, SKIP|DENSE)`, the last argument: the rows are ranked in the sort order, rows that
  tie on every key sharing a rank (text without regard to case, as the sort): `SKIP` ranks a row
  one more than the rows before it (1, 2, 3, 3, 5), `DENSE` one more than the distinct keys
  before it (1, 2, 3, 3, 4); the rows ranked after `skip` and up to `skip + n` are returned (so
  `skip` and `n` count ranks, and a tie at the boundary comes whole). This reading of dax.guide's
  two sentences is unconfirmed.
- Calendars are the model's `calendars` (TMSL, under a table; `calendar` in TMDL). A calendar
  reference is a bare or quoted name. The period is identified by the primary columns of the
  first set of categories the calendar has, in the page's "Period uniqueness" order: for a week
  Week; Week of Year and Year; Week of Quarter and Quarter; Week of Quarter, Quarter of Year and
  Year; Week of Month and Month; Week of Month, Month of Year and Year; Week of Month, Month of
  Quarter and Quarter; Week of Month, Month of Quarter, Quarter of Year and Year (then the
  semester sets); for a month, a quarter and a year the matching sets. The days are the Date
  category's primary column. `CLOSINGBALANCE*(expression, calendar [, filter])` evaluates the
  expression on the last day the calendar's table has in the period of the last day in context;
  `OPENINGBALANCE*` on the last day before the first day of the period of the first day in
  context (blank when there is none: an empty day filter). The filters on every column tagged in
  any calendar of the table (categories and time-related columns: the page's cross-calendar
  rule) are removed, the untagged columns' filters kept (also on a table related by its date
  column, as the page's example shows: the classic date-key rule does not apply), then the day and the filter argument
  applied with `CALCULATE`'s context transition. A calendar without the period's categories or
  without a Date category, and a `*WEEK` balance of something that is not a calendar, are
  errors. The tagged columns' sort-by columns are cleared too, and the other calendar-based
  functions are evaluated: see "Calendar-based time intelligence" below.
- `OFFSET`, `INDEX`, `WINDOW`, `RANK` and `ROWNUMBER` with an axis (`ROWS`, `COLUMNS`,
  `ROWPAGES`), a reset keyword (`NONE`, `HIGHESTPARENT`, `LOWESTPARENT`) or a reset after the
  relation are an error in a measure, a column or a query (no longer "not supported"); visual
  calculations evaluate them (`docs/formats/visual-calculations.md`).
- `SEARCH` and `CONTAINSSTRING`: `?` matches one character (UTF-16 unit), `*` any run of
  characters including none, `~` before `?`, `*` or `~` makes it literal (before anything else a
  `~` is itself, unconfirmed). `SEARCH` returns the first position from which the pattern
  matches (a leading `*` matches at the start position); `FIND` and `CONTAINSSTRINGEXACT` have
  no wildcards.
- Without `utc_offset_minutes`, `NOW()`/`TODAY()` are the process's local time and `UTCNOW()`/
  `UTCTODAY()` that minus its offset at the instant (`crate::clock`: the machine's time zone in
  the desktop app and the CLI, UTC or `--local-offset` in `bicraft serve`), in `dax.evaluate`,
  visuals, the viewer and RLS; an `as_of` is local time, read in the same zone. The parameter,
  when given, still wins. Calculated tables and columns read the same clock since the
  "Calendar-based time intelligence" lot below.

None of these cases was checked against the engine (no oracle ran for them): their tests
(`crates/engine/tests/model/dax_leftovers_tests.rs`, `crates/engine/tests/local_clock.rs`) are
**documented, not oracle-checked**.

### Calendar-based time intelligence

Added on 2026-11-25 (gap G30's leftovers, branch `feat/dax-calendars`):

| Module | Functions and forms |
|---|---|
| `preview_eval_calendar.rs` | over a calendar: `TOTALWTD`, `TOTALMTD`, `TOTALQTD`, `TOTALYTD`; `DATESWTD`, `DATESMTD`, `DATESQTD`, `DATESYTD`; `STARTOFWEEK`, `ENDOFWEEK` and the month, quarter and year forms; `FIRSTDATE`, `LASTDATE`; `PREVIOUSDAY`, `PREVIOUSWEEK`, `PREVIOUSMONTH`, `PREVIOUSQUARTER`, `PREVIOUSYEAR` and the `NEXT*` forms; `PARALLELPERIOD`; `DATEADD` (with `extension` and `truncation`); `SAMEPERIODLASTYEAR`; `DATESINPERIOD` (with `endbehavior`); `DATESBETWEEN`; the sort-by columns of tagged columns cleared (also by the balances) |
| `preview_eval_pick.rs` | the filter arguments of `ROLLUPISSUBTOTAL`; several `ROLLUPISSUBTOTAL` in one `ADDMISSINGITEMS` |
| `preview_eval_struct.rs`, `refresh.rs` | calculated tables and columns on the local clock; `model.refresh`'s `utc_offset_minutes` |

Sources: the Power BI page
[Implement time-based calculations in Power BI](https://learn.microsoft.com/en-us/power-bi/transform-model/desktop-time-intelligence)
(section "Calendar-based time intelligence": categories, primary columns used for sorting,
period uniqueness, context clearing with sort-by columns, time-related columns, cross-calendar
behaviour, fixed and flexible functions, `DATEADD` and `DATESINPERIOD` with calendars), and
Microsoft Learn's DAX reference pages of
[DATEADD](https://learn.microsoft.com/en-us/dax/dateadd-function-dax),
[DATESINPERIOD](https://learn.microsoft.com/en-us/dax/datesinperiod-function-dax),
[DATESBETWEEN](https://learn.microsoft.com/en-us/dax/datesbetween-function-dax),
[PARALLELPERIOD](https://learn.microsoft.com/en-us/dax/parallelperiod-function-dax),
[SAMEPERIODLASTYEAR](https://learn.microsoft.com/en-us/dax/sameperiodlastyear-function-dax),
[DATESYTD](https://learn.microsoft.com/en-us/dax/datesytd-function-dax),
[STARTOFWEEK](https://learn.microsoft.com/en-us/dax/startofweek-function-dax),
[STARTOFYEAR](https://learn.microsoft.com/en-us/dax/startofyear-function-dax),
[PREVIOUSWEEK](https://learn.microsoft.com/en-us/dax/previousweek-function-dax),
[PREVIOUSDAY](https://learn.microsoft.com/en-us/dax/previousday-function-dax),
[FIRSTDATE](https://learn.microsoft.com/en-us/dax/firstdate-function-dax),
[TOTALWTD](https://learn.microsoft.com/en-us/dax/totalwtd-function-dax),
[ROLLUPISSUBTOTAL](https://learn.microsoft.com/en-us/dax/rollupissubtotal-function-dax). The
rules kept:

- **Positions and periods.** A calendar's rows are ordered by its Date category's primary
  column, else by its primary columns from the coarsest unit to the finest (Year, semesters,
  Quarter, Quarter of Year…, Month, Month of Year…, Week…, Date, Day of…); a primary column with
  a sort-by column sorts by that column. Rows that order the same are one position (a day when
  there is a Date category). A period of a unit is identified by the first category set of the
  page's "Period uniqueness" order the calendar has (for a day: Date, then Day of Year and Year,
  …); periods are ordered by their first position and a shift of `n` periods moves `n` places
  in that order, among the periods the calendar's table has (sparse dates are allowed: the day
  before a Monday can be a Friday). A calendar without the unit's categories is an error.
- **The selection** is the positions of the calendar's rows visible in the current context,
  after context transition. `DATEADD` and `SAMEPERIODLASTYEAR` read it with the filters on the
  table's time-related and untagged columns left out (they still apply afterwards: the page's
  lateral-shift example, where the shifted year keeps the same `IsWorkingDay` values).
- **The result** is the calendar's rows at the selected positions, as a table of the
  calendar's primary columns and its time-related columns (`DATEADD` and `SAMEPERIODLASTYEAR`:
  the primary columns only), in the calendar's order; used as a filter (a `CALCULATE` or
  `CALCULATETABLE` argument, `TOTAL*TD`, the balances), the filters on the columns tagged in a
  category of any calendar of the table, on the time-related columns of any calendar of the
  table (not by `DATEADD` and `SAMEPERIODLASTYEAR`), and on the sort-by columns of those columns
  are removed first, then the rows set; filters on untagged columns and other tables are kept
  (no date-key rule). Since the result sets every primary column, every category is cleared, as
  the page's dependency lattice would clear the categories of the finest one.
- `DATESWTD`/`DATESMTD`/`DATESQTD`/`DATESYTD(calendar)`: the positions of the period holding the
  last selected position, up to it; `TOTAL*TD(expression, calendar [, filter])` evaluates the
  expression on them, the filter argument after (a year-end date is an error with a calendar,
  as the reference says, also for `DATESYTD`, `STARTOFYEAR`, `ENDOFYEAR`, `PREVIOUSYEAR`,
  `NEXTYEAR`).
- `STARTOF*(calendar)`: the first position of the period of the first selected position;
  `ENDOF*`: the last position of the period of the last one; `FIRSTDATE`/`LASTDATE`: the first
  and last selected positions; `PREVIOUS*`: the whole period before the first selected
  position's; `NEXT*`: the whole period after the last one's (`*DAY`: a day). Empty when there is
  no such period.
- `PARALLELPERIOD(calendar, n, unit)`: the whole periods from the first selected position's
  period moved by `n` to the last one's moved by `n`, clamped to the periods the calendar has.
- `DATEADD(calendar, n, unit [, extension [, truncation]])`: the selected positions are grouped
  by period of the unit and in runs of consecutive positions; each run moves to the period `n`
  places on, keeping its positions within the period (the reference's index-based approach:
  March 3–10 → April 3–10; the 60th day of 2024, February 29, → March 1, 2023; May 15, the 45th
  day of its quarter, a quarter on → August 14). A run covering its whole period moves to the
  whole target period. A run made of whole periods of a finer unit the calendar identifies
  (quarters, months, weeks for a year; months, weeks for a quarter; weeks for a month, the
  coarsest first) keeps its index among the periods of that unit starting in the period instead:
  February a year back is February (the page's "selection at a finer grain" reading of whole
  months; unconfirmed for weeks). The rules below then count those periods. A run reaching its period's end, when the
  target is longer: `EXTENDING` (the default) extends it to the target's end (February 25–28 →
  March 25–31), `PRECISE` keeps its positions (→ March 25–28), `ENDALIGNED` keeps its length,
  aligned on the target's end (→ March 28–31; February 28 → March 31, February 27 → March 27).
  Positions the target lacks: `BLANKS` (the default) drops them (March 31 one month back →
  nothing), `ANCHORED` puts them on the target's last position (→ February 28). `EXTENDED` and
  `BLANK` are read as `EXTENDING` and `BLANKS`. `SAMEPERIODLASTYEAR(calendar)` is
  `DATEADD(calendar, -1, YEAR)`.
- `DATESINPERIOD(calendar, start, n, unit [, endbehavior])`: `start` is a value of the Date
  category's primary column; the boundary is `DATEADD` of that position alone by `n` units with
  `PRECISE` (the default) or `ENDALIGNED`, a position the target lacks anchored on its last one;
  the result is `(boundary, start]` for a negative `n`, `[start, boundary)` for a positive one
  (to the calendar's end when the target period is beyond it, from its start when before). A
  start the calendar does not have (the reference: undefined) or `n` = 0 is an empty table. The
  reference's examples agree: February 28 six months back is August 28 (`PRECISE`, seven month
  names) or August 31 (`ENDALIGNED`, six); a year back from the last day of a 366-day fiscal year
  is anchored on the previous year's last day (July 1 to June 30).
- `DATESBETWEEN(calendar, start, end)`: the positions from `start` to `end` (values of the Date
  category's primary column); a blank one, or one the calendar does not have, is its first or
  last position, as the reference says. Both need a Date category (an error otherwise).
- **Sort-by columns.** The sort-by column of a column tagged in a calendar is cleared with it,
  by the balances too ("context is set on a column or its associated sort by column").
- **`ROLLUPISSUBTOTAL` filters.** `ROLLUPISSUBTOTAL([grandTotalFilter,] group, is_subtotal
  [, [groupLevelFilter,] group, is_subtotal]…)`: each filter is a table expression added to the
  filter tables of one level's combinations: `grandTotalFilter` (before the first group) to the
  level where every group is rolled up, a `groupLevelFilter` (after a group's subtotal column) to
  the level where that group is the deepest one still grouped (the detail level for the last
  group). The reference only says "filter applied to the grand total level" and "to the current
  level": this reading is unconfirmed. Two filters for one level are an error.
- **Several `ROLLUPISSUBTOTAL`** in one `ADDMISSINGITEMS` are crossed, as `SUMMARIZECOLUMNS`
  crosses several `ROLLUPADDISSUBTOTAL`: every combination of their levels (each with its own
  flags), the first one's levels outermost; unconfirmed.
- **Nested calendar functions** (2026-11-25, branch `fix/g30-format-calendar`): a calendar
  function whose calendar argument is itself a calendar function over a calendar
  (`PREVIOUSDAY(PREVIOUSMONTH(Cal))`, `TOTALYTD([m], DATEADD(Cal, -1, YEAR))`,
  `CLOSINGBALANCEMONTH([m], PREVIOUSYEAR(Cal))`) is an error naming both functions, as table,
  as filter argument, in `TOTAL*TD` and in the balances: the page's "Considerations" say such
  nesting is not supported. Its alternative, the inner function as a `CALCULATETABLE` filter
  (`CALCULATETABLE(PREVIOUSDAY(Cal), PREVIOUSMONTH(Cal))`), is evaluated by the rules above.
  The engine's own error text is not known.
- **Clock of calculated tables and columns.** `NOW()`/`TODAY()` in a calculated table or column
  are local time and `UTCNOW()`/`UTCTODAY()` that minus the offset, as in `dax.evaluate`: the
  process's local clock (`crate::clock`) at the evaluation's instant (a refresh's start for
  `model.refresh`), or `model.refresh`'s `utc_offset_minutes` (-1080 to 1080) when given, which
  also sets the offset `DateTime.LocalNow()` adds in its M queries.

Unconfirmed choices: the cross-calendar clearing of time-related columns (kept from the balances:
the page's text clears them for every calendar of the table, while its hierarchical-shift
example keeps `IsWorkingDay` with the calendar that does not tag it although another calendar of
the same table does); `DATEADD`'s selection without the time-related and untagged filters;
`ENDALIGNED` for a run of several days; the anchored boundary of `DATESINPERIOD`; the order of
rows without a Date category. The first example of `DATESINPERIOD`'s page (a year back from
June 30, 2020 starts on July 1, 2019) holds here for a fiscal year ending on June 30 (the
anchored boundary above); over a Gregorian year the range starts a day later, July 2, since
June 30, 2020 is the 182nd day of its year and the 182nd day of 2019 is July 1 (the
position rule the reference gives for February 29). None of these cases was checked against
the engine: their tests
(`crates/engine/tests/model/dax_calendar_tests.rs`, the clock test in
`crates/engine/src/preview_eval_tests.rs`) are **documented, not oracle-checked**.

Behavior changes of functions that were already evaluated, all taken from the engine: text to
number conversion reads dates and times (culture of the model), dates and booleans as text
(`3/5/2024`, `TRUE`), numbers as text with 15 significant digits, `DATE` year rules and rounded
arguments, date arithmetic result types, `ROUND`/`TRUNC` of integers stay integers, `POWER` of
blank is blank, `IF` and other conditions accept the texts `"TRUE"`, `"FALSE"` and `""`.

Still refused (deliberately or not yet): `RAND`, `RANDBETWEEN` (not deterministic), `USERNAME`,
`USERPRINCIPALNAME`, `USEROBJECTID`, `USERCULTURE`, `CUSTOMDATA` (no user), `SAMPLE` without an order key (random), `FORMAT`
with an unquoted date, time or numeric letter in a number format (`"#,##0,, M"`: the reference
says those characters cannot be literal, their meaning there is unknown), and the rest of the
catalog not listed here or in the evaluator's module headers.

`FORMAT` scaling commas (2026-11-25, branch `fix/g30-format-calendar`): one or more commas right
before the decimal point or the end of the number's placeholders divide by 1000 each, also when
a literal suffix follows, quoted (`"#,##0,, ""M"""` of 1234567 is `1 M`) or not (`"0,K"` of 1234
is `1K`, `"#,0,.0 k€"` of 12345 is `12,3 k€` in fr-FR); commas after the decimal point neither
group nor scale (the engine's results, `crates/locale/tests/oracle`). A letter that is none of
the reference's date, time or numeric characters (`a c d h m n p q s t w y`, `E e`) is a
literal in a number format. Source: the custom numeric format characters of Microsoft Learn's
[FORMAT](https://learn.microsoft.com/en-us/dax/format-function-dax) page. The unquoted-letter
cases are documented, not oracle-checked (`crates/locale/src/tests.rs`,
`scaling_commas_before_a_literal_suffix`); the 701-probe `FORMAT` corpus still agrees as before.

Known differences (`KNOWN` in `coverage_oracle_matches_the_engine`): BIcraft has one number
type, so a blank or a sum of integers can come out typed `double` where the engine says `int64`
(`ABS(BLANK())`, `BLANK() + 1`), `1E15 & ""` is `1000000000000000` (the engine keeps the double's
`1E+15`), `ISERROR(1 / 0)` is FALSE (TRUE in the engine), `AVERAGEX` over Currency values is
typed `double`.

## Calendar

Date serials are OLE Automation dates: days from 1899-12-30, the time of day as the fraction.
DAX, M and the VertiPaq storage share them, and the DAX and M functions accept 0100-01-01
(serial −657 434) to 9999-12-31 (2 958 465). Serials have no 1900-02-29: serial 60 is
1900-02-28 and 61 is 1900-03-01 (Excel's 1900 leap-year bug is not part of DAX or M). Days
before the epoch are negative serials, and `-1.25` is 1899-12-28 18:00 (the fraction counts
forward from the floor).

All calendar math goes through `bicraft_locale::calendar` (ROADMAP M11.9, docs/method.md
decision 3), which is built on the `jiff` crate (`civil::Date`, dates only, `std` without its
time zone features; it replaced `chrono`, flagged unmaintained, in M11.43): civil date ↔ day
number, leap years, month lengths, month shifts with the day clipped (`EDATE`,
`Date.AddMonths`, `DATEADD`), month ends, ISO weeks (`WEEKNUM(…, 21)`).
DAX (`crates/engine/src/preview_eval*.rs`, `preview_filters.rs`), M (`crates/data`), the
.NET and VB patterns of `bicraft-locale`, the readers and writers (`pbix`, `metadata`,
`databricks`) and the timestamps of logs and files use it; no crate keeps its own copy of the
calendar any more. jiff's range (years −9999 to 9999) holds every day DAX and M accept; beyond
it (carries past 9999 and hostile input), the module answers with the same calendar's
400-year-cycle arithmetic (`calendar::fast`), which a test checks against jiff on every day DAX
and M accept, on the first and last thousand days of jiff's range and on every 97th day in
between. The weekday is that arithmetic too (`x mod 7`), with the same answers.

**Differential test.** `crates/engine/tests/calendar_golden.rs` runs a generated corpus
(167 values: the days around the epoch, leap days of 1900/2000/2100/2400, month starts and ends
of 2023–2024, ISO week edges, years 100 and 9999, negative and fractional serials, values out of
range, and 50 pseudo-random days) through the DAX date functions (`YEAR` … `SECOND`, `WEEKDAY`,
`WEEKNUM` with every return type, `EOMONTH`, `EDATE`, `DATEDIFF` in every unit, `YEARFRAC` in
every basis, `NETWORKDAYS`, `FORMAT`, `CALENDAR`, `DATE` argument carries, `TIME`, `DATEVALUE`),
33 time-intelligence functions over a 2019–2025 calendar under single-day and whole-month
filters, 48 M `Date.*`/`DateTime.*` functions per day, `DateTimeZone` switches across days,
months and years, culture parsing, and `bicraft-locale`'s .NET and VB patterns in three
cultures. The answers (`tests/calendar/golden.txt`) were recorded from the hand-written code
before the move to chrono, and the ported code gives the same 4 134 lines: the port changed no
answer, and neither did the move from chrono to jiff (M11.43). `BICRAFT_BLESS=1` rewrites the file for a change that is meant.

**Speed.** `crates/engine/tests/calendar_bench.rs` (ignored, `--ignored --nocapture`) times DAX
date functions over a million rows, M date functions over 300 000 values and CSV date columns;
`cargo test -p bicraft-locale calendar_speed -- --ignored --nocapture` times the conversions
alone. Measured on the maintainers' machine (test profile, 2026-11-05): the end-to-end timings
are unchanged by the move (within run-to-run noise, see the M11.9 progress-log line); per call,
chrono's conversion of a serial to a date costs about 14 ns against 12 ns for the arithmetic and
its ISO week is ×2.5 faster than the arithmetic's. After the move to jiff (2026-11-23, same
machine and profile, one run of `calendar_speed` while other builds were running): a serial to a
date 22 ns against 13 ns for the arithmetic, a date to a serial 18 ns against 11 ns, a month
shift 91 ns, an ISO week 40 ns against 37 ns; the end-to-end benches were not rerun.
## INFO functions

The `INFO.*` functions return the model's metadata as DAX tables, with the columns of the
matching `TMSCHEMA_*` schema rowsets; the four `INFO.VIEW.*` return the same objects with names
instead of IDs. BIcraft evaluates ten of them (`crates/engine/src/preview_eval_info.rs`); the
others are in the catalog (the validator knows them) and are refused by the evaluator by name.

Source: Microsoft's DAX reference (learn.microsoft.com/dax), the overview page "INFO functions"
(updated 2026-04-13) and each function's page (column tables and the "Applies to" line, updated
2026-08-14 for `INFO.TABLES`/`PARTITIONS`/`MODEL`/`COLUMNS`/`MEASURES`, 2026-11-02 for
`INFO.VIEW.*`). **Documented, not measured** unless a cell says otherwise: the probes for
Desktop and the maintainers' checklist are in `tools/info-oracle/`.

### Where they run

| Function | Measure | Calculated column | Calculated table | Row filter | DAX query |
|---|---|---|---|---|---|
| `INFO.VIEW.TABLES`, `.COLUMNS`, `.MEASURES`, `.RELATIONSHIPS` | yes | yes | yes | not documented (allowed) | yes |
| every other `INFO.*` | no | no | no | not documented (refused) | yes |

BIcraft applies the same rule, so a measure that BIcraft accepts is one Desktop accepts:

- The validator refuses a query-only `INFO.*` call in a measure, calculated column, calculated
  table or row filter (`info-query-only`, an error: `measure.add`, `table.add`, `column.add` and
  the other edits refuse it without `force`). A `DEFINE MEASURE` or `DEFINE TABLE` of a query may
  call them (not documented; allowed).
- The evaluator refuses them inside a measure of the model (one that `force` let in, or a file
  that holds one): the call fails with "can only be used in a DAX query".

So `Last refresh = MAXX(INFO.PARTITIONS(), [RefreshedTime])` is **not** a measure Desktop
accepts: `INFO.PARTITIONS` is query-only, and no `INFO.VIEW.*` function has a refresh time. The
allowed way to show the last refresh in a visual is a table refreshed with the data:
`table.add_refresh_info` adds one (`docs/mcp.md`, "Refresh info table"). `INFO.PARTITIONS` stays
the way to read the refresh state in a DAX query (`dax.evaluate`).

### Columns and values

Columns, in this order, with the types the reference gives:

- `INFO.TABLES` (20): `ID`, `ModelID`, `Name`, `DataCategory`, `Description`, `IsHidden`,
  `TableStorageID`, `ModifiedTime`, `StructureModifiedTime`, `SystemFlags`,
  `ShowAsVariationsOnly`, `IsPrivate`, `DefaultDetailRowsDefinitionID`,
  `AlternateSourcePrecedence`, `RefreshPolicyID`, `CalculationGroupID`, `ExcludeFromModelRefresh`,
  `LineageTag`, `SourceLineageTag`, `SystemManaged`.
- `INFO.PARTITIONS` (23): `ID`, `TableID`, `Name`, `Description`, `DataSourceID`,
  `QueryDefinition`, `State`, `Type`, `PartitionStorageID`, `Mode`, `DataView`, `ModifiedTime`,
  `RefreshedTime`, `SystemFlags`, `ErrorMessage`, `RetainDataTillForceCalculate`, `RangeStart`,
  `RangeEnd`, `RangeGranularity`, `RefreshBookmark`, `QueryGroupID`, `ExpressionSourceID`,
  `MAttributes`.
- `INFO.MODEL` (24): `ID`, `Name`, `Description`, `StorageLocation`, `DefaultMode`,
  `DefaultDataView`, `Culture`, `Collation`, `ModifiedTime`, `StructureModifiedTime`, `Version`,
  `DataAccessOptions`, `DefaultMeasureID`, `DefaultPowerBIDataSourceVersion`, `ForceUniqueNames`,
  `DiscourageImplicitMeasures`, `DataSourceVariablesOverrideBehavior`,
  `DataSourceDefaultMaxConnections`, `SourceQueryCulture`, `MAttributes`,
  `DiscourageCompositeModels`, `AutomaticAggregationOptions`, `DisableAutoExists`,
  `VersionMarker`.
- `INFO.COLUMNS` (41): `ID`, `TableID`, `ExplicitName`, `InferredName`, `ExplicitDataType`,
  `InferredDataType`, `DataCategory`, `Description`, `IsHidden`, `State`, `IsUnique`, `IsKey`,
  `IsNullable`, `Alignment`, `TableDetailPosition`, `IsDefaultLabel`, `IsDefaultImage`,
  `SummarizeBy`, `ColumnStorageID`, `Type`, `SourceColumn`, `ColumnOriginID`, `Expression`,
  `FormatString`, `IsAvailableInMDX`, `SortByColumnID`, `AttributeHierarchyID`, `ModifiedTime`,
  `StructureModifiedTime`, `RefreshedTime`, `SystemFlags`, `KeepUniqueRows`, `DisplayOrdinal`,
  `ErrorMessage`, `SourceProviderType`, `DisplayFolder`, `EncodingHint`,
  `RelatedColumnDetailsID`, `AlternateOfID`, `LineageTag`, `SourceLineageTag`.
- `INFO.MEASURES` (19): `ID`, `TableID`, `Name`, `Description`, `DataType`, `Expression`,
  `FormatString`, `IsHidden`, `State`, `ModifiedTime`, `StructureModifiedTime`, `KPIID`,
  `IsSimpleMeasure`, `ErrorMessage`, `DisplayFolder`, `DetailRowsDefinitionID`, `DataCategory`,
  `LineageTag`, `SourceLineageTag`. The page gives no types: IDs, `DataType` and `State` are
  int64 here (their TOM codes), the flags boolean, the times dateTime, the rest text.
- `INFO.RELATIONSHIPS` (20; the reference does not list its columns, these are the
  `TMSCHEMA_RELATIONSHIPS` rowset's): `ID`, `ModelID`, `Name`, `IsActive`, `Type`,
  `CrossFilteringBehavior`, `JoinOnDateBehavior`, `RelyOnReferentialIntegrity`, `FromTableID`,
  `FromColumnID`, `FromCardinality`, `ToTableID`, `ToColumnID`, `ToCardinality`, `State`,
  `RelationshipStorageID`, `RelationshipStorage2ID`, `ModifiedTime`, `RefreshedTime`,
  `SecurityFilteringBehavior`.
- `INFO.VIEW.TABLES` (14): `ID`, `Name`, `Model`, `DataCategory`, `Description`, `IsHidden`,
  `StorageMode`, `TableStorage`, `Expression`, `ShowAsVariationOnly` (singular, as documented),
  `IsPrivate`, `CalculationGroupPrecedence`, `LineageTag`, `ExtendedProperties`.
- `INFO.VIEW.COLUMNS` (28): `ID`, `Name`, `Table`, `DataType`, `DataCategory`, `Description`,
  `IsHidden`, `IsUnique`, `IsKey`, `IsNullable`, `Alignment`, `SummarizeBy`, `ColumnStorage`,
  `Type`, `SourceColumn`, `Expression`, `FormatString`, `IsAvailableInMDX`, `SortByColumn`,
  `GroupingBehavior`, `SourceProviderType`, `DisplayFolder`, `AlternateOf`, `LineageTag`,
  `StringIndexingBehavior`, `FullTextIndexingBehavior`, `ExtendedProperties`, `GroupByColumns`.
- `INFO.VIEW.MEASURES` (17): `ID`, `Name`, `Table`, `Description`, `DataType`, `Expression`,
  `FormatString`, `IsHidden`, `State`, `KPIID`, `IsSimpleMeasure`, `DisplayFolder`,
  `DetailRowsDefinition`, `DataCategory`, `FormatStringDefinition`, `LineageTag`,
  `ExtendedProperties`.
- `INFO.VIEW.RELATIONSHIPS` (16): `ID`, `Name`, `Relationship`, `Model`, `IsActive`,
  `CrossFilteringBehavior`, `RelyOnReferentialIntegrity` (boolean here; the page says text),
  `FromTable`, `FromColumn`, `FromCardinality`, `ToTable`, `ToColumn`, `ToCardinality`, `State`,
  `SecurityFilteringBehavior`, `ExtendedProperties`.

Values:

- **Times.** `RefreshedTime` (partitions; columns: their table's latest partition), `ModifiedTime`
  and `StructureModifiedTime` (tables, columns, measures: their table's; the model: the model's)
  come from the open file's backup (`metadata.sqlitedb` of a PBIX `DataModel` or a PBIP
  `.pbi/cache.abf`, `docs/formats/metadata-sqlitedb.md`, "Refresh state"), in UTC. When
  `model.refresh` wrote the data the session reads (`.pbi/bicraft-data.bin`), its tables have its
  `refreshed_at` as `RefreshedTime` and `ModifiedTime`, and `State` 1. Without stored data (a
  new model, a PBIT, a PBIP never refreshed) they are blank, and so is `State`.
- **Codes** (`INFO.*`) are the TOM enumeration values: `State` 1 Ready, 3 NoData,
  4 CalculationNeeded, 5 SemanticError, 6 EvaluationError, 7 DependencyError, 8 Incomplete,
  10 ForceCalculationNeeded; partition `Type` 1 Query, 2 Calculated, 3 None, 4 M, 5 Entity,
  6 PolicyRange, 7 CalculationGroup, 8 Inferred, 9 Parquet; `Mode` 0 Import, 1 DirectQuery,
  2 Default, 3 Push, 4 Dual, 5 DirectLake; data types 2 String, 6 Int64, 8 Double, 9 DateTime,
  10 Decimal, 11 Boolean, 17 Binary, 19 Unknown, 20 Variant; column `Type` 1 Data, 2 Calculated,
  3 RowNumber, 4 CalculatedTableColumn; `SummarizeBy` 1 Default … 8 DistinctCount; `Alignment`
  1 Default, 2 Left, 3 Right, 4 Center; `CrossFilteringBehavior` 1 OneDirection,
  2 BothDirections, 3 Automatic; cardinality 1 One, 2 Many.
- **Names** (`INFO.VIEW.*`) are the TOM names of the same values (`Import`, `Data`, `Default`,
  `OneDirection`, `Many`, `Ready`), and `DataType` is the DAX data type name: `Text` (the one the
  reference's example shows), `Whole Number`, `Decimal Number`, `Currency`, `Date`,
  `True/False`, `Binary`, `Variant`. Not measured; `tools/info-oracle` lists what to check.
- **IDs** are BIcraft's own numbering (1, 2, … per object kind in model order), consistent across
  the functions (`INFO.PARTITIONS()[TableID]` joins `INFO.TABLES()[ID]`); they are not the
  engine's IDs. `INFO.VIEW.*` `[Model]` is the model's name (the engine gives an ID);
  `TableStorage`/`ColumnStorage` are `Name (ID)`; `Relationship` is
  `'From'[Column] *-1 'To'[Column] (OneDirection)`. Measures report `State` Ready and data type
  Variant (BIcraft does not keep the engine's per-measure state).
- **Rows**: the objects of the model as it is. The engine's own `RowNumber` columns appear only
  when the model holds them.
- **Restrictions**: `INFO.X("Name", "Sales", …)` keeps the rows whose column of that name equals
  the value (text without regard to case, numbers by value); a name that is not a column is an
  error. `INFO.VIEW.*` take no argument.

## Sources

- Public DAX function reference (learn.microsoft.com/dax), table of contents and syntax sections
  of each function page: function names, categories, keyword values.
- Power BI Desktop 2.158.1304.0 local engine: `$SYSTEM.MDSCHEMA_FUNCTIONS` (names, parameter
  names, optional/repeatable flags) and argument-count errors. Raw outputs are kept outside the
  repository; no description text is copied.
