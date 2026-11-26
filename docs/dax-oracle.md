# DAX engine oracle (M6.3)

How often does BIcraft's DAX validator agree with the real Analysis Services engine? The oracle
suite answers that with a corpus of DAX expressions, the verdicts the engine of Power BI Desktop
gave on them, and a test that replays those verdicts against `bicraft_dax` on any platform.

The verdicts are recorded during development against Microsoft's Analysis Services engine, with
the maintainers' development oracles. They are kept outside this repository
(`BICRAFT_ORACLES_DIR` points to them). The recorded verdicts are stored here as our own data, and
the public tests need neither Windows nor the engine.

| File | What it is |
|---|---|
| `crates/dax/tests/oracle/model.bim` | The synthetic test model (TMSL), ours |
| `crates/dax/tests/oracle/corpus.json` | 1166 expressions, each with its role and category (generated) |
| `crates/dax/tests/oracle/verdicts.json` | The engine's recorded verdicts, normalized to our error classes |
| `crates/dax/tests/oracle.rs` | The test: agreement statistics, disagreement list, floors |
| `crates/dax/tests/oracle/signatures.json`, `crates/dax/tests/oracle/signatures.rs` | The signature corpus (M11.40): argument counts of every catalogued function and 3807 signature probes with the engine's verdicts, and the tests that hold the catalogue and the binder to them (see *Signature corpus* below) |
| `crates/mlang/tests/oracle/m_signatures.json`, `crates/mlang/tests/oracle.rs` | The M half of the signature corpus: the counts, parameter names and types and return type Power BI Desktop's M engine reports for each function of BIcraft's M catalogue (see *M signatures* below) |
| `xtask/src/oracles/dax_corpus_data.rs` | Authoring source of the corpus; `cargo xtask dax-corpus` regenerates `corpus.json` |
| `crates/engine/tests/calc_tables/expected.json` | The columns and rows the engine stores for the calculated tables of `probes.json` (G21) |
| `crates/engine/tests/dax_queries/expected.json`, `crates/engine/tests/dax_coverage/expected.json` | The engine's results for the `dax.evaluate` query corpora (M9.4; see *Query results* below) |

`cargo test -p bicraft-dax --test oracle -- --nocapture` prints the figures and the
disagreements. With `BICRAFT_ORACLE_REPORT=<file>` it also writes them as a Markdown table (the one
at the end of this page).

## Test model

`model.bim` is a small star schema made only of DAX calculated tables, so it deploys and computes
without any data source:

| Table | Columns (type) | Measures |
|---|---|---|
| `Customer` | CustomerKey (int64), Name, City, Segment (string), IsActive (boolean), BirthDate (dateTime) | Customer Count |
| `Product` | ProductKey (int64), Product, Category (string), ListPrice (decimal), Weight (double) | |
| `Date` | Date (dateTime, key; date table), Year, Month (int64), Month Name, Quarter (string) | |
| `Sales` | SalesKey, CustomerKey, ProductKey, Quantity (int64), OrderDate (dateTime), UnitPrice (decimal), Discount (double), Channel (string), IsReturn (boolean); calculated column Line Amount | Total Sales, Order Count, Total Quantity, Average Price, Sales LY |
| `Top Products` | ProductKey, Product, ListPrice (calculated table derived from `Product`) | |
| `LocalDateTable_0001` | Date (dateTime), Year, MonthNo (int64), Month (string), QuarterNo (int64), Quarter (string), Day (int64); hierarchy Date Hierarchy; hidden, `showAsVariationsOnly` | |

Relationships (many to one): `Sales[CustomerKey]` → `Customer`, `Sales[ProductKey]` → `Product`,
`Sales[OrderDate]` → `Date[Date]`, `Sales[OrderDate]` → `LocalDateTable_0001[Date]`.

`LocalDateTable_0001` and the `variations` entry of `Sales[OrderDate]` reproduce what Power BI
Desktop's auto date/time creates for a date column, so `'Sales'[OrderDate].[Year]` resolves;
`Customer[BirthDate]` has no variation. Adding them changed no verdict of the earlier entries
(the whole corpus was recorded again with them and compared).

## Corpus

Each entry is placed in the model as a **probe** object, the same way by the harness and by the
test:

| Role | Probe | BIcraft call |
|---|---|---|
| `measure` | measure `Probe` on `table` | `validate_as(.., ExprKind::Measure)` |
| `column` | calculated column `Probe` of `table` | `validate_as(.., ExprKind::CalculatedColumn)` |
| `table` | calculated table `Probe Table` | `validate_as(.., ExprKind::CalculatedTable)` |
| `rls` | role `Probe Role` with this row filter on `table` | `validate_as(.., ExprKind::RowFilter)` |
| `query` | none: the query runs against the base model | `validate_query` |

An entry may add helper objects (`with`), e.g. the second measure of a circular pair. The test also
runs `deps::cycle_diagnostics` on the model with the probe in it, because circular references are a
model-level property for the engine too.

Categories (entries): syntax (32), unknown objects (25), arity (25), types (41), row context (45),
RELATED/EARLIER (25), CALCULATE filter rules (35), iterators (25), time intelligence (30), table
constructors (21), variables (25), coercion (25), blanks (15), operators (27), `IN` (11), deprecated
and unusual functions (12), window functions (13), circular references (11), calculated tables (35),
row-level security (18), queries (28), realistic valid measures and columns (59), column variations
(48).

Added on 2026-10-13 for G4, G19 and G20 (286 entries, with valid counterparts for each rule):
reserved words (21), `GROUPBY`/`CURRENTGROUP` (17), window relations and clauses (28),
time-intelligence `dates` arguments (31), natural joins and calculated-table column names (31),
query cycles (16), comparisons of incompatible types (36), run-time errors (56), and the
multi-section functions `SUMMARIZE`/`GROUPBY`/`SUMMARIZECOLUMNS` (50).

Added on 2026-10-14 (26 entries, `var-shadowing`): a variable named like a table, referenced
quoted or not, as a table argument, a CALCULATE filter, a column qualifier, in each role.

Added on 2026-10-14 for `sections-024`, `sections-031` and G19 (199 entries, in three rounds,
each probing the edges the previous one showed):
- `sections-2` (99): `NONVISUAL` filters of `SUMMARIZECOLUMNS` on various columns and in other
  places, and a column's `VALUES`/`DISTINCT`/`ALL` used as a `SUMMARIZECOLUMNS` or `SUMMARIZE`
  value under different group-by columns, keys and filters.
- `modifiers` (100): `USERELATIONSHIP`, `CROSSFILTER`, `REMOVEFILTERS`, `ALLCROSSFILTERED`,
  `KEEPFILTERS`, and `ALL()`/`ALLSELECTED()` without arguments, inside and outside `CALCULATE`
  filters, in each role.

Added on 2026-10-14 (24 entries, `keyword-names`): engine keywords shared with MDX/DMX (`Typed`,
`Status`, `Name`, `Type`…) as unquoted column qualifiers, bare table references, query table
names, variable, function and parameter names, with quoted counterparts and the non-reserved
user-defined-function type words (`VAL`, `SCALAR`, `ANYVAL`, `INT64`, `FUNCTION`).

All expressions were written for this suite. To add some, edit `xtask/src/oracles/dax_corpus_data.rs`,
run `cargo xtask dax-corpus`, then record the new entries with the maintainers' development
oracles (not in this repository; `BICRAFT_ORACLES_DIR` points to them): the test fails while an
entry has no recorded verdict. `cargo test -p xtask dax_corpus` fails while `corpus.json` is out
of date.

## Recorded verdicts

For each entry, the model with its probe is deployed on the engine and computed. The probe's
state and error come from the engine's schema views. Measures the engine accepts are then
evaluated at the grand total, and row filters are exercised by a query under the probe role.
Queries are executed as written.

Engine messages are Microsoft text and are **never committed**. `verdicts.json` keeps only our
normalized error class.

Recorded with the Analysis Services engine (version 17.0.87.18, as reported by the server), locale
1033, on 2026-10-13. A spot re-run of a subset reproduced the recorded verdicts exactly. Later that day the
whole corpus was recorded again on a fresh engine instance, and the 631 earlier verdicts came back
unchanged. The last 60 additions were recorded on their own and merged into `verdicts.json` in
corpus order. The 26 `var-shadowing` entries were recorded the same way on 2026-10-14 (same
engine version), and so were the 199 `sections-2` and `modifiers` entries. Then the whole corpus
(1142 entries) was recorded again on a fresh engine instance on 2026-10-14. Every verdict came
back unchanged. The 24 `keyword-names` entries were recorded later that day on their own and
appended.

### Verdict semantics

- `valid`: the engine accepted the **definition**. That is the state Desktop shows when the object
  is created, and what a static validator can be held to. A deployment refused outright (circular
  dependency, duplicate column name in a calculated table) and the `SemanticError` state are
  invalid. A calculated column or table whose definition is accepted but which fails while
  computing (`EvaluationError`) counts as valid.
- `evaluates`: the runtime outcome where one was observed (measure evaluated at the grand total,
  row filter applied, column or table computed). `false` means a runtime error such as a failed
  text-to-number conversion.
- `class` / `eval_class`: our category for the engine's message: `syntax`, `arity`,
  `unknown-column`, `unknown-table`, `unknown-name`, `unknown-reference`, `row-context`,
  `earlier-context`, `related-no-relationship`, `calculate-filter`, `table-as-scalar`,
  `scalar-as-table`, `column-ref-expected`, `table-shape`, `duplicate-name`,
  `circular-dependency`, `conversion`, `type`, `argument`, `relationship`, `window`,
  `visual-calc-only`, `groupby`, `parameter`, `variation`, `other`.

The test's main figure compares BIcraft's `valid` with the engine's `valid`. It also prints, for
information, the agreement with "valid and evaluates".

## Results (2026-10-14)

| | Entries | Agreement | False accepts | False rejects |
|---|---|---|---|---|
| Definitions | 1166 | **99.9%** (1165) | 1 | 0 |
| Including evaluation | 1166 | 90.5% | 111 | 0 |
| measure | 879 | 100% | 0 | 0 |
| calculated column | 96 | 100% | 0 | 0 |
| calculated table | 71 | 100% | 0 | 0 |
| row filter | 27 | 100% | 0 | 0 |
| query | 93 | 98.9% | 1 | 0 |

2026-10-14, `sections-2` and `modifiers` (199 entries). Before the new rules, BIcraft was
measured on the first two rounds (170 entries): 58 false accepts, no false reject, and none of
their 35 run-time errors flagged. Now one false accept is left, and it is deliberate
(`sections-2-061`, see the work list). `sections-024` is fixed, and `sections-031` is flagged.

Before this round (same date, 631 entries): 617 agreeing (97.8%), 14 false accepts, 0 false
rejects. All 14 are fixed. The corpus grew to 917 entries and still has no false reject.

2026-10-14, `var-shadowing` (26 entries): before the fix BIcraft resolved a quoted name to the
table even with a variable of that name in scope (false accepts such as
`VAR Product = 3 RETURN COUNTROWS('Product')`), and refused a column qualifier named like a
variable and `DEFINE VAR Product = FILTER(Product, …)` (false rejects). All 26 agree now.

**Run-time errors (G20).** In 110 entries the engine accepts the definition but evaluation fails.
BIcraft flags 109 of them, with a `runtime-*` warning or, for queries and calculated columns, the
matching error. No `runtime-*` warning fires on an entry the engine evaluates without error. The
test prints both figures and fails if such a false warning appears. The "including evaluation" row
still counts these 110 entries as disagreements, because warnings do not change the `valid`
verdict.

Column variations: 48 of 48 agree (26 valid measures, 2 valid calculated columns, 2 valid queries;
20 rejections: unknown variation name, column without variations, measure with a variation, `.Name`
without brackets, two levels, missing row context, `RELATED` of a variation, a boolean filter mixing
the variation's table with the base table).

The test asserts:
- agreement ≥ 99% (`AGREEMENT_FLOOR`)
- no false reject (`MAX_FALSE_REJECTS = 0`)
- no run-time warning on an entry that evaluates

Raise the floors as the validator improves, and re-measure whenever the validator or the corpus
changes.

What the engine showed, in our words:

- Measure definitions are checked statically for names, arity, column/scalar/table argument kinds,
  row context, `EARLIER` depth, `RELATED` reachability, the CALCULATE boolean filter rules, window
  function placement and cycles. Implicit conversions (`"a" + 1`, `1 = "1"`, `YEAR("abc")`) and a
  multi-value table used as a scalar (`VALUES(...)` as a measure) are only runtime errors: the
  definition is accepted.
- In a CALCULATE boolean filter, a measure reference is rejected but an aggregation such as
  `AVERAGE(Sales[Quantity])` is accepted. A filter without any column (`TRUE()`, `1`, a string) is
  rejected. Columns from two tables in one boolean filter are rejected.
- `RELATED` must follow a many-to-one path from the current row context: from `Product` to `Sales`
  it fails, and so does `RELATED` to an unrelated table.
- A variable can shadow a table name (`VAR Sales = 1` is accepted). Redefining a variable in the
  same block is an error. `t[col]` on a table variable is an unknown-table error.
- A variable wins over a table of the same name **even quoted**: with `VAR Product = 3` in scope,
  `COUNTROWS('Product')` and `COUNTROWS(Product)` count a scalar (rejected), `CALCULATE(…,
  'Customer')` under `VAR Customer = 2` is a filter without a column, and a calculated table
  `VAR Product = 1 RETURN 'Product'` is a scalar. A table variable of that name works as the table
  (`VAR Product = FILTER(Product, …) RETURN COUNTROWS('Product')`). The variable is visible only
  after its definition, also for query variables (`DEFINE VAR Product = FILTER(Product, …)` is
  not circular). A column qualifier always names the table, never the variable:
  `SUM('Product'[ListPrice])` and `VALUES(Product[Category])` work with `VAR Product = 1` in
  scope.
- `ROW`, `SELECTCOLUMNS`, `CROSSJOIN`, `GENERATE` and calculated tables reject duplicate column
  names. Table constructors and `UNION` require equal column counts. `x IN table` requires the
  table to have as many columns as the left side has values.
- A row filter that returns a non-boolean value is accepted at definition and fails when applied.
- `MOVINGAVERAGE` and `RUNNINGSUM` are rejected outside visual calculations.

Found on 2026-10-13. Each rule lives in `crates/dax/src/typecheck_rules.rs` or
`typecheck_runtime.rs` and has corpus entries:

- **Unknown functions are errors.** The catalog holds every DAX function of the engine's
  `MDSCHEMA_FUNCTIONS`; this was re-checked and no engine function is missing. So an unknown name
  fails, like the engine's "failed to resolve name". A user-defined function of the model
  (`functions` in the model's TMSL/TMDL) is not unknown. Without a model it stays a warning.
- **Reserved words.** 568 names cannot be variables, function or parameter names, query table
  names or unquoted table qualifiers, and all but ten cannot be bare table references either.
  - Syntax errors: `VAR Value`, `VAR SUM`, `VAR Rank`, `VAR Filter`, `Date[Year]`, `date[Year]`,
    `SUM(Typed[N])`, `COUNTROWS(Status)`, `EVALUATE Name`.
  - Fine: `VAR Year`, `VAR Index`, `VAR Calendar`, `VAR Scalar`, `COUNTROWS(Date)`,
    `EVALUATE Date`, `SUM('Typed'[N])`.
  - The list comes from a broad list of candidate words, the engine's MDX/DMX keywords among
    them (`Typed[N]` is refused in Desktop too); every verdict is the engine's answer.
  - See `docs/formats/dax-functions.md` §"Reserved words".
- **`WEEK` intervals** (`DATEADD`, `PARALLELPERIOD`, `DATESINPERIOD`) need a calendar reference.
  With a date column or any table they are rejected. `DATEDIFF` accepts `WEEK`. Other intervals
  are only `DAY`, `MONTH`, `QUARTER` and `YEAR` (not `DECADE` or a string).
- **`dates` arguments** of time intelligence.
  - Accepted:
    - a column, of any type (a non-date column fails only when evaluated)
    - a one-column table with model lineage (`VALUES`, `DISTINCT`, `CALCULATETABLE(VALUES(…))`)
    - an expression that is a column (`VAR d = 1 RETURN 'Date'[Date]`)
  - Rejected:
    - a table of several columns (`'Date'`, `ALL('Date')`, `FILTER('Date', …)`)
    - a table whose columns have no lineage (`ROW`, `SELECTCOLUMNS` with a new name,
      `GENERATESERIES`, `CALENDAR`)
    - a scalar (`TODAY()`, a string, a measure)
    - a year-end date without a digit (`"x"`)
  - A table constructor fails only when evaluated.
- **`GROUPBY` expressions** must be exactly one aggregation over `CURRENTGROUP()`.
  - Accepted aggregations: `SUMX`, `AVERAGEX`, `MINX`, `MAXX`, `COUNTX`, `COUNTAX`, `PRODUCTX`,
    `STDEVX.P`, `VARX.S`.
  - Rejected aggregations: `CONCATENATEX`, `MEDIANX`, `PERCENTILEX.INC`, `RANKX`, `COUNTROWS`.
  - Also rejected: anything around the aggregation (`SUMX(…) + 1`), a constant, a measure, and a
    measure or `CALCULATE` inside the aggregation.
  - `CURRENTGROUP()` anywhere else is rejected.
- **Window functions.**
  - A relation that is a column is rejected: `OFFSET(-1, 'Date'[Year], …)`,
    `RANK(DENSE, 'Date'[Year])`, `ROWNUMBER('Date'[Year])`.
  - `ORDERBY` expressions are evaluated in a row context over the relation only, not its expanded
    table. So `ORDERBY(Product[Category])` over `ALLSELECTED(Sales)` is rejected. An outer row
    context still counts.
  - `PARTITIONBY` and `MATCHBY` columns must be columns of the relation.
  - `ORDERBY`, `PARTITIONBY` and `MATCHBY` are rejected anywhere else.
- **Natural joins** need common join columns by lineage.
  - Two model tables join when one expands to the other through many-to-one relationships
    (`NATURALINNERJOIN(Customer, Sales)`).
  - Equal names are not enough: `'Date'` and `LocalDateTable_0001` share column names but fail.
    Unrelated tables fail too.
- **Calculated tables** reject two result columns with one name from different tables:
  `CROSSJOIN(Sales, Product)`, `NATURALINNERJOIN(Sales, Product)`,
  `SUMMARIZE(Sales, Sales[ProductKey], Product[ProductKey])`. The same expressions are fine inside
  a measure.
- **`CALENDARAUTO`** needs a date column in a data table. It does not scan the columns of
  calculated tables or calculated columns. The test model has only calculated tables, so it fails.
- **Query cycles** count only when the query uses them: an `EVALUATE`, its `ORDER BY`, or a query
  variable or table it uses reaches the cycle. Unused cyclic `DEFINE MEASURE`/`COLUMN`
  definitions run. A query measure overrides a model measure for the query's own references only:
  model measures keep the model's definitions.
- **Comparisons of incompatible types**: text with a number or a date, a boolean with a number.
  - Definition errors in calculated columns, calculated tables and `EVALUATE` statements (even in a
    branch never taken).
  - Run-time errors in measures and row filters.
  - Numbers and dates compare freely, and `BLANK()` compares with anything. The same holds for
    `IN`, `SWITCH` and `CONTAINS`.
- **Multi-section functions.**
  - `SUMMARIZE` and `GROUPBY` take group-by columns, then name/expression pairs.
    `SUMMARIZECOLUMNS` takes group-by columns, filter tables, then pairs.
  - Rejected:
    - a non-column or a measure where a group-by column is expected
    - anything after the pairs
    - a name without an expression
    - a group-by column given twice
    - the subtotal modifiers in `SUMMARIZE`
  - `SUMMARIZE` expressions have a row context over the group-by columns. `SUMMARIZECOLUMNS`
    expressions have none.

Found on 2026-10-14 (categories `sections-2` and `modifiers`; rules in
`crates/dax/src/typecheck_modifiers.rs`):

- **`NONVISUAL`** filters of `SUMMARIZECOLUMNS` may only filter its group-by columns (code
  `nonvisual-not-group-by`).
  - The group-by columns are the bare column arguments. A column inside `ROLLUPADDISSUBTOTAL` does
    not count, and with no group-by column every `NONVISUAL` filter is refused.
  - A filter's columns are found by lineage, through `FILTER`, `KEEPFILTERS`, `CALCULATETABLE`,
    `TOPN`, `TREATAS`, `CROSSJOIN` and a whole table (`NONVISUAL(Product)` covers every column of
    `Product`).
  - Not checked, because the engine accepts them all:
    - filter removers (`NONVISUAL(ALL(Sales))`, `NONVISUAL(ALL(Product[Category]))`)
    - tables without lineage (`DATATABLE`, `SELECTCOLUMNS` with a new name)
    - variables
  - `KEEPFILTERS` may wrap `NONVISUAL` or be wrapped by it.
- **`NONVISUAL` elsewhere.** Rejected at definition:
  - in calculated columns and tables
  - as a measure itself
  - as the argument of `COUNTROWS` or `ISEMPTY`

  In a measure's `CALCULATE` filter or an iterator's table it is accepted, then fails when
  evaluated.
- **`USERELATIONSHIP` and `CROSSFILTER`** must be a direct filter argument of `CALCULATE`,
  `CALCULATETABLE` or a function with a `CALCULATE`-like filter (`TOTALYTD`). They are rejected
  everywhere else at definition:
  - inside `KEEPFILTERS`
  - in a variable
  - as a `SUMMARIZECOLUMNS` filter
  - as an operand
  - as a measure
- **`REMOVEFILTERS`, `ALLCROSSFILTERED`, and `ALL()`/`ALLSELECTED()` without arguments** are
  accepted:
  - as `CALCULATE` filters, possibly inside `KEEPFILTERS`
  - as `TOTALYTD` filters
  - as `SUMMARIZECOLUMNS` filters

  Calculated columns, calculated tables and row filters reject them anywhere else. Measures are
  more lenient, and the verdict depends on the function:
  - Rejected: `REMOVEFILTERS` as an iterator's table (`SUMX`, `FILTER`, `TOPN`, `CONCATENATEX`),
    in `DISTINCT`, as `CALCULATETABLE`'s table, in a variable, inside `IF`, or as the measure.
    `ALL()` and `ALLSELECTED()` in `FILTER`, in a variable, or as the measure.
  - Accepted, then failing when evaluated: any of them as the argument of `COUNTROWS` or
    `ISEMPTY`. `ALL()` and `ALLSELECTED()` as the table of `SUMX` or `MAXX`.
    `ALLCROSSFILTERED` as the table of `SUMX` or `FILTER`.
  - `ALL(…)` and `ALLSELECTED(…)` with arguments are ordinary table functions.
- **`KEEPFILTERS`** is accepted:
  - as a top-level `CALCULATE`/`CALCULATETABLE` filter or `TOTALYTD` filter
  - as a `SUMMARIZECOLUMNS` filter
  - as the table of a function that scans it: `SUMX`, `MAXX`, `FILTER`, `ADDCOLUMNS`,
    `SELECTCOLUMNS`, `TOPN`, `RANKX`, `CONCATENATEX`, `GENERATE`'s first table

  It is rejected at definition, in every role:
  - in `COUNTROWS` or `ISEMPTY`
  - as a `FILTER` condition
  - as `CALCULATETABLE`'s table
  - as `GENERATE`'s second table
  - in a variable
  - as the whole expression
- **A table as a `SUMMARIZECOLUMNS` value.** A column's `VALUES`, `DISTINCT` or `ALL` used as a
  value fails when evaluated for any group that sees several values of the column. The
  definition is accepted. The column has one value per group when:
  - it is a group-by column itself
  - or a group-by column is a key of its table (marked `isKey`, or the one side of a relationship)
  - or a filter argument narrows its table

  It does not have one value per group when:
  - the group-by column is another column of the same table (`Sales[Channel]` → `Sales[Quantity]`)
  - or the group-by column is a column of a related table, on either side
    (`Sales[ProductKey]` → `Product[Category]`)

  `ALL` ignores the group's filters, so `ALL` of a group-by column fails too. The same column in
  `SUMMARIZE` or `ADDCOLUMNS` was not evaluated by `COUNTROWS` and did not fail.

Run-time errors BIcraft reports as warnings (`runtime-*` codes, G20):

| Code | When |
|---|---|
| `runtime-incompatible-comparison` | comparison of incompatible types in a measure or row filter |
| `runtime-conversion` | a text literal that is not a number, date or boolean where one is needed (`"a" + 1`, `"" + 1`, `IF("yes", …)`, `YEAR("")`, `VALUE("twelve")`, `CONVERT("x", INTEGER)`); text columns in arithmetic or logic; `GENERATESERIES` over text |
| `runtime-multiple-values` | a table constructor of several rows used as a value; a measure that is `VALUES`/`DISTINCT`/`ALL` of a column; the same as a `SUMMARIZECOLUMNS` value when the group-by columns do not determine the column (directly or through operators) |
| `runtime-misplaced-filter-modifier` | a filter modifier a measure's definition accepts but evaluation refuses (`COUNTROWS(REMOVEFILTERS(T))`, `SUMX(ALL(), …)`, `NONVISUAL` in a `CALCULATE` filter); an error in `EVALUATE` statements |
| `runtime-invalid-argument` | `MOD`/`QUOTIENT` by 0, `SQRT` of a negative, `LN`/`LOG` of a non-positive number, a negative length in `LEFT`/`RIGHT`/`REPT`, `MID` before the first character |
| `runtime-aggregation-type` | `SUM`/`AVERAGE`(`X`) over text or booleans |
| `runtime-dates-type` | a time-intelligence column that is not of type date; dates from a table constructor |
| `runtime-non-boolean-filter` | a row filter returning a number or text |

These run fine and are not flagged: `"1" + 1`, `"2024-01-01" + 1`, `TRUE() + 1`, `1 / 0`
(infinity), `IF("", …)`, `IF("TRUE", …)`, `DIVIDE(1, 0, "n/a")`, `MAX` of a text column, `SUM` of a
date column, `COUNTX` over text.

## Work list: current disagreements

One false accept, no false reject:

| Id | Role | Kind | Engine class | BIcraft errors | Expression |
|---|---|---|---|---|---|
| sections-2-061 | query | false accept | table-as-scalar | - | `EVALUATE SUMMARIZECOLUMNS(Sales[Channel], "x", VALUES(Sales[Quantity]))` |

The false accept is deliberate. The query fails because of the data: a channel has several
quantities. BIcraft reports `runtime-multiple-values` as a warning. Making it an error in queries
would reject valid queries that group by a unique column the model does not mark as a key.

One run-time error is not flagged: sections-2-060, `VAR v = VALUES(Sales[Quantity]) RETURN
COUNTROWS(SUMMARIZECOLUMNS(Sales[Channel], "x", v))`. The variable is evaluated outside the
`SUMMARIZECOLUMNS`, and table variables are not followed to their definition there.

Known false warning outside the corpus: grouping by a column whose values are unique but which is
not marked as a key, and taking another column of the same table. Examples: a fact table's
surrogate key without `isKey`, or a unique name column. BIcraft cannot tell such a column from an
ordinary one, so `runtime-multiple-values` fires although every group has one value. Marking the
key column (`isKey`) or using `SELECTEDVALUE` silences it.

Known false accepts outside the corpus figures, left alone because a rule would risk false
rejects:
- `VAR r = ALLSELECTED(…)` as a window relation: columns are not tracked through variables.
- Unqualified `[Name]` group-by columns in `SUMMARIZECOLUMNS`: the engine wants fully qualified
  references.

## Query results: `dax.evaluate` (M9.4)

The suite above checks verdicts (is this DAX valid?). A second corpus checks **values**: DAX
queries run by `dax.evaluate` on BIcraft's evaluator, compared cell by cell with what the engine
returns for the same queries on the same data.

| File | What it is |
|---|---|
| `crates/engine/tests/dax_queries/model.bim` | Synthetic model, ours: `Product` (6 rows), `Customer` (5), `Date` (2023–2024, marked as date table), `Sales` (600 rows from `GENERATESERIES`, with product key 7 and customer key 6 that match nothing), `Targets` (KPI targets by year and category, no relationship); 12 measures (totals, `SAMEPERIODLASTYEAR`, `TOTALYTD`, YoY, `TREATAS` targets, `SUMX`/`RELATED`) |
| `crates/engine/tests/dax_queries/queries.json` | 65 queries, ours |
| `crates/engine/tests/dax_queries/expected.json` | The engine's columns (name, client type) and rows per `EVALUATE`, or `error` |
| `query_oracle_matches_the_engine` in `crates/engine/tests/model/dax_query_tests.rs` | Saves the model as a PBIX (data built by BIcraft), runs each query with `dax.evaluate` and compares (runs on Linux) |

Comparison: same result count, column names and order, row count; rows compared in order when the
query has `ORDER BY`, else as multisets; numbers within 1e-9 relative; dates as ISO text; column
types (`int64`, `double`, `decimal`, `dateTime`, `string`, `boolean`) must match too. An engine
error must be an error in BIcraft. A query BIcraft refuses (unsupported) is not a disagreement
but is counted; a different value is a failure of the test.

Categories: control totals (by year, category, channel, sums of parts equal the total), base data
(`EVALUATE Sales`), `CALCULATE`/`CALCULATETABLE`/`KEEPFILTERS`/`REMOVEFILTERS`/`ALLEXCEPT`,
`ALLSELECTED` (under `SUMMARIZECOLUMNS` filters, `CALCULATETABLE`, iterators), `SUMMARIZECOLUMNS`
with `FILTER` and `TREATAS` filter tables, orphan keys (`EXCEPT` of key values, `FILTER` +
`ISBLANK(RELATED(…))`, the blank row in groups), KPI targets through `TREATAS`, time intelligence
(LY, YTD, QTD, MTD, `DATEADD`, `DATESINPERIOD`, `DATESBETWEEN`, `PREVIOUSMONTH`, `PREVIOUSYEAR`,
`PARALLELPERIOD`), `DEFINE` `MEASURE`/`VAR`/`TABLE`/`COLUMN`, several `EVALUATE`s, `ORDER BY` on
columns and measures, `START AT`, `TOPN`, `RANKX`, `GENERATE`, `UNION`/`INTERSECT`, iterators,
`HASONEVALUE`/`SELECTEDVALUE`, text functions and `FORMAT`, two run-time errors, the clock
(`TODAY`, `NOW`, `UTCTODAY`, `UTCNOW`: checked through clock-independent relations such as
`INT(NOW()) = TODAY()`), `DATEDIFF` units, `CONCATENATEX` (order keys, `TOPN`/`VALUES` inputs,
blanks: an empty string sorted first, blank for an empty table) and table total rows
(`ROLLUPADDISSUBTOTAL` grand totals of share, dense `RANKX` over `ALLSELECTED` and
year-on-year measures, as Desktop's table visuals query them).

### Results (2026-10-14, Power BI Desktop 2.158.1304.0, locale 1033)

| | Queries |
|---|---|
| Agree (values and types) | **78 / 79 (98.7%)** |
| Refused by BIcraft (error naming the construct) | 1 |
| Different values | **0** |

The first recording showed three behaviors BIcraft then adopted:
- **`ALLSELECTED` in a query** restores the *shadow* filter context: the filter arguments of the
  enclosing `SUMMARIZECOLUMNS`, or the context of the innermost iterator with its iterated rows
  (`ADDCOLUMNS` inside `CALCULATETABLE(…, Category = "Toys")` gives shares of Toys, not of all
  products). The viewer keeps the visual's outer filters, as before.
- **`DEFINE TABLE`** is a new table: its columns lose the lineage of the model columns, so
  `SUMX(T, Sales[Amount])` over it is the engine's "a single value for column … cannot be
  determined" error, while a `DEFINE VAR` table keeps the lineage.
- **`DEFINE MEASURE` with a model measure's name** replaces it for the query's own references
  only: model measures that use it keep the model's definition. BIcraft refuses such a query
  (the one refused entry) rather than evaluate it differently; redefining a measure no model
  measure uses agrees.

After a change of the model or the queries, `expected.json` is regenerated with the maintainers'
development oracles (not in this repository; `BICRAFT_ORACLES_DIR` points to them). Engine
messages are never committed.

## Function coverage: `tests/dax_coverage` (2026-10-23)

A third corpus checks the evaluator **function by function**: values, blanks, errors and result
types of single expressions (`EVALUATE ROW("v", …)`), plus calculation groups and format strings.

| File | What it is |
|---|---|
| `crates/engine/tests/dax_coverage/model.bim` | The `dax_queries` model at compatibility level 1606, with two measures with format strings (one dynamic, `SELECTEDMEASURE()` inside), a calculation group `Time Calc` (precedence 10: `Current`, `YTD`, `PY`, `YoY %` with a format string, `Name`, `IsSel`, `Fmt`, `Double`, `PlusUnits`, `Ref Self`) and a second group `Scale` (precedence 20) |
| `crates/engine/tests/dax_coverage/queries.json` | 501 queries, ours (`cg-*`: calculation groups and format strings; `fn-*`: functions, blanks, errors, types, implicit conversions) |
| `crates/engine/tests/dax_coverage/expected.json` | The engine's results, recorded with the maintainers' development oracles |
| `coverage_oracle_matches_the_engine` in `crates/engine/tests/model/dax_query_tests.rs` | Runs every query with `dax.evaluate` (`as_of` fixed, `utc_offset_minutes` 120) and compares like the query oracle; unsupported functions are counted, a different value fails the test except the listed known differences |

Recorded against the Analysis Services engine (version as reported by the server), locale 1033,
on 2026-10-23. Clock-dependent and environment-dependent
probes (`RAND`, `USERNAME`, `USERPRINCIPALNAME`, `USERCULTURE`, `CUSTOMDATA`) are not in the corpus;
the clock is checked through relations (`INT(NOW()) = TODAY()`).

What the engine showed (each rule is implemented and has entries):

- **Calculation groups** (`crates/engine/src/preview_eval_calc.rs`): an item applies to a measure
  reference when the item column (TMSL `sourceColumn` `Name`) is filtered directly to one item; a
  filter on the ordinal column, several items or none apply nothing; expressions that are not
  measure references are not transformed. Higher precedence applies first. Once applied, a group
  is consumed: measure references inside the measure or the item are not transformed again,
  until a `CALCULATE` filter or a context transition sets or removes a filter on its table.
  `SELECTEDMEASURE()` outside an item is an error. Result columns of measures are untyped
  (variant) in a model with calculation groups.
- **Format strings**: `[_<measure> FormatString]` (what Power BI's visual queries read) and
  `SELECTEDMEASUREFORMATSTRING()` give the first applied item's format string expression, else
  the measure's dynamic format string (its `SELECTEDMEASURE()` sees the items applied), else the
  static one, else blank.
- **Clock**: `NOW()`/`TODAY()` are the machine's local time, `UTCNOW()`/`UTCTODAY()` UTC (Desktop
  on a CEST machine: 16:43 and 14:43). `dax.evaluate` takes `utc_offset_minutes`; without it
  (since 2026-11-25) the offset is the process's local clock at the instant (the machine's time
  zone in the desktop app and the CLI, UTC or `--local-offset` on a server, UTC in tests);
  `as_of` is the local time.
- **Blanks in math functions**: the result for 0, or blank when that is 0 (`SIN(BLANK())`,
  `CEILING(BLANK(), 1)` blank; `COS`, `EXP`, `FACT`, `ODD` of blank 1; `LN(BLANK())` -Infinity,
  while `LN(0)` is an error). Non-finite results are errors (`EXP(1000)`, `ASIN(2)`).
- **Whole-number arguments are rounded** half away from zero (`GCD(24.9, 36.2)` = 1,
  `ISEVEN(3.9)`, `COMBIN(8.6, 3.6)` = 126, `TIME(1.9, 30.9, 0)` = 02:31, `DATE(2024.7, 2.9, 3.9)`
  = 2025-03-04); `FACT` truncates.
- **`DATE`**: years 0–49 are 2000–2049, 50–99 1950–1999, negative years count back from 1900,
  100–9999 as given; months and days carry, but a day of exactly 0 is an error.
- **Implicit conversions**: text to number also reads dates and times in the model's culture
  (`"2024-01-15" + 0`, `"March 5, 2024" + 0`, `"14:30" + 0`, `VALUE("2024-03-05")`); dates as text
  are the culture's general date (`3/5/2024`, `3/5/2024 2:30:00 PM`, `2:30:00 PM`), booleans
  `TRUE`/`FALSE`, numbers with 15 significant digits (`1E+20`, `0.00001`, infinity `inf`);
  `IF("TRUE", …)` reads the text.
- **Types**: date + number and date − anything are dates (`DATE − DATE` is a date counted from
  1899-12-30), number − date and −date are numbers; `ROUND`, `TRUNC`, `ROUNDUP` of an integer
  stay integers; `CEILING`/`FLOOR`/`MROUND` take the significance's type (Currency wins);
  two-argument `MIN`/`MAX` and `IFERROR` the wider type.
- **`LOOKUPVALUE`** ignores the filter context.
- **`IGNORE`** in `SUMMARIZECOLUMNS` keeps the expression out of the non-blank test (a row with
  only ignored expressions is kept); `SAMPLEAXISWITHLOCALMINMAX` returns a table that is not
  larger than its size unchanged (both seen in Desktop's own visual queries: the
  interaction oracle, recorded with Desktop 2.160.1151.0, where 438 of 438 queries agree).

- **Window functions** (recorded 2026-10-23, evaluated since 2026-11-18): `ALL(table)` as a
  window relation keeps the table's virtual blank row, which sorts first ascending and last
  descending (`fn-rownumber` is 5 for the fourth price, `fn-window` sums the blank row and the
  first two products); `OFFSET` and `WINDOW` with a relative bound find the current row from a
  filter on the relation's key (`fn-offset`, `fn-window-rel`). The balances, `GROUPBY` and the
  natural joins (which join `Sales` and `Product` through their relationship) agree too. Since
  then the test also fails when more than six queries are refused. Since 2026-11-20
  `ADDMISSINGITEMS`, `SUBSTITUTEWITHINDEX`, `SAMPLE` and `TOPNSKIP` are evaluated and agree
  (`fn-addmissingitems` 4, `fn-substitutewithindex` 6, `fn-sample` 3, `fn-topnskip` 2 rows); the
  cap is two refusals (`fn-search-wild`, `fn-format-fr-scale`).

### Results (2026-10-23)

See the test output; the figures are in `docs/formats/dax-functions.md` §"Evaluator coverage".

## Signature corpus (M11.40, 2026-11-23)

A fourth corpus checks the **signatures** of the binder's catalogue: how many arguments each
function takes and which kind each parameter accepts. The idea comes from DashCraft's arity
recordings; none of its recorded data is used. The probes are generated from BIcraft's own
catalogue (`probes()` in `crates/dax/tests/oracle/signatures.rs`), so a new catalogue entry gets
probes the next time the corpus is recorded.

For each of the 472 functions of the catalogue:

- `/none` and `/many`: the call with no arguments and with 40, which give the engine's minimum
  and maximum argument counts (`functions` in `signatures.json`, `max` null when 40 arguments
  raise no count error);
- `/min`, `/max`, `/below`, `/above`: the shortest and longest valid calls of the catalogue, and
  one argument fewer or more;
- `/reset`: the `reset` parameter of the window functions;
- `/{i}-{kind}`: the full call with parameter `i` replaced by a value of another kind (a number
  for a column, a table for a scalar, a constant for a keyword, …).

Each call stands where its function may stand: query-only `INFO.*` functions in an `EVALUATE`,
filter modifiers in `CALCULATE(1, …)`, `ORDERBY`/`PARTITIONBY`/`MATCHBY` in an `INDEX`,
`ROLLUP`/`ROLLUPGROUP` in a `SUMMARIZE`, table functions in a `COUNTROWS`. The verdict is
accepted or refused with BIcraft's error class; accepted measures are also evaluated, and an
evaluation error is recorded as its class (`eval_class`). Recorded on the Analysis Services
engine (version 17.0.93.23, as reported by the server), locale 1033 (`SOURCE.txt` in the folder).

Tests (`cargo test -p bicraft-dax --test oracle -- --nocapture signatures`):

- `catalogue_arity_matches_the_engine`: measured 467 of 472 functions have the engine's minimum
  and maximum; the 5 others are the justified deviations of G19 (`TRUE`, `FALSE`, `NOT`,
  `USERNAME`, `RANK`), listed in `ARITY_DEVIATIONS` with their reasons.
- `binder_matches_the_engine_on_signature_probes`: measured 3750 of 3807 probes agree (98.5 %),
  442 of 472 functions on every probe. The 57 disagreements (56 accepted by BIcraft and refused
  by the engine, 1 the reverse) are all covered by the 33 entries of `PROBE_DEVIATIONS`, each with
  its reason; the test fails on any other disagreement and on an entry that no longer matches.
  A `runtime-*` warning of BIcraft on a probe the engine evaluates counts as a disagreement.

Before the fixes of 2026-11-23 between 478 and 536 probes disagreed. What the engine showed, now
implemented (regression tests in `crates/dax/src/tests_rules.rs`):

- Keyword parameters refuse constants other than text (`TOPN(1, T, [x], 1)` is accepted,
  `PARALLELPERIOD(…, 1, DAY)` refused: `PARALLELPERIOD` takes `WEEK`, `MONTH`, `QUARTER`, `YEAR`).
- Visual-calculation functions (`RUNNINGSUM`, `MOVINGAVERAGE`, `COLLAPSE`, `EXPAND`, `FIRST`,
  `LAST`, `NEXT`, `PREVIOUS`, `LOOKUP`, `RANGE`, `ISATLEVEL`, …) are refused outside a visual
  calculation; `IGNORE`, `NATURALJOINUSAGE` and `ROLLUPADDISSUBTOTAL` need an enclosing
  `SUMMARIZECOLUMNS`, `ROLLUPISSUBTOTAL` an enclosing `ADDMISSINGITEMS`.
- Table-reference parameters refuse table expressions; measure-reference parameters refuse
  anything but a reference; window relations, `ORDERBY`, `PARTITIONBY` and `MATCHBY` refuse
  constants, and a window function without a relation is refused outside visual calculations.
- A constant filter (`CALCULATE(1, 1)`) is refused; a text year-end date stays allowed.
- `AVERAGEA`, `MAXA`, `MINA`, `MEDIAN`, `PERCENTILE.INC`/`.EXC` accept a measure where a column
  is expected.
- Calendar-only arguments (the extension and truncation of `DATEADD`, the end behaviour of
  `DATESINPERIOD`) need a calendar.
- `CONTAINSROW` and `TREATAS` take as many values as the table has columns; `DATATABLE` takes
  pairs then one row list; `SELECTCOLUMNS` needs a name before an expression that is not a
  column; `INFO.*` restriction names are text; `CROSSFILTER` needs a model relationship like
  `USERELATIONSHIP`.

The development engine does not evaluate Power Query: the M functions are recorded with Power BI
Desktop instead (*M signatures* below).

## M signatures (M11.40, 2026-11-23)

The M half of the signature corpus holds BIcraft's M catalogue (`bicraft-mlang`, the 409
functions the evaluator runs) to the signatures Power BI Desktop's own M engine reports.

M describes its own library through public functions, so Desktop is used as a black box: BIcraft
wrote a PBIX (`bicraft run`, `table.add` with an M partition, `file.save_as {empty_tables}`) whose
queries, our own M, read for each catalogue name its function type from `#shared` (and
`#binary`, `#date`, `#datetime`, `#datetimezone`, `#duration`, `#table`, `#time` by name, since
`#shared` does not list them): `Type.FunctionRequiredParameters`, `Type.FunctionParameters`
(names and types), `Type.FunctionReturn`, and each type's kind (`Type.Is` against the primitive
types) and nullability (`Type.IsNullable`). A second query listed every name of `#shared` with
its kind and counts. Power BI Desktop 2.159.1231.0 (Store) opened the file, Home › Refresh ran
the queries, Desktop saved it, and `bicraft dax-query FILE "EVALUATE MSig"` read the loaded
tables back. No documentation text is read or kept (`Value.Metadata` is not used).

| File | What it is |
|---|---|
| `crates/mlang/tests/oracle/m_signatures.json` | Per catalogue function: `min` (required), `max` (total), parameter `names` and `params` types, `returns`; `library_only`: the library functions Desktop has and BIcraft lacks, with their counts; `shared`: how many functions and other values `#shared` holds |
| `crates/mlang/tests/oracle/SOURCE.txt` | Desktop version, date and method |
| `crates/mlang/tests/oracle.rs` | The tests (`cargo test -p bicraft-mlang --test oracle -- --nocapture`) |

Types are BIcraft's codes: the primitive kind (`number`, `text`, `list`, `function`, `type`, …)
with `?` when nullable; `any?` is M's `any`, `any` is `anynonnull`. Facets are not kept
(`Int64.Type` reads as `number`). The catalogue now writes each parameter's type and the return
type the way M does (`Text.PadStart(text as nullable text, count as number, optional character as
nullable text) as nullable text`), available as `Param::ty` and `Function::returns`.

Measured on the recording:

- `#shared` holds 941 functions and 201 other values (constants, types); 402 of the 941 are in
  BIcraft's catalogue, which also has the 7 `#` constructors; 539 library functions (connectors
  included) are listed in `library_only` as the coverage left.
- Of the 900 parameters of the catalogue's functions, 682 have a primitive type other than `any`;
  215 are `any`, 3 `anynonnull`. 65 functions return `any`.
- Before the fixes, 10 of 409 functions had other counts than Desktop's and 4 more other parameter
  names; the catalogue had no types. Fixed (evaluator and catalogue, regression test
  `argument_counts_follow_the_signature_corpus` in `crates/data/src/m/tests_library.rs`):
  `Date.FromText`, `DateTime.FromText`, `DateTimeZone.FromText` and `Time.FromText` take
  `(text, optional options)` (no third culture argument); `List.LastN`'s count or condition is
  optional; `Comparer.FromCulture` needs its culture; `Lines.FromText` takes no encoding;
  `Table.NestedJoin` has no join algorithm (seven parameters); the names of `Date.DayOfYear`,
  `Date.DaysInMonth` (`dateTime`), `Logical.ToText` (`logicalValue`) and `List.Times` (`start`).
- Now 407 of 409 functions have Desktop's counts, names and types; the 2 deviations
  (`DEVIATIONS` in the test, G108) lack trailing optional parameters the evaluator does not run:
  `Error.Record`'s `parameters` and `errorCode`, `Splitter.SplitTextByDelimiter`'s `csvStyle`.
- The same day the Power Query editor's Scientific and Trigonometry menus added seven functions
  to the catalogue (`Number.Acos`, `Asin`, `Atan`, `Cos`, `Factorial`, `Sin`, `Tan`); a second
  probe of those names in the same Desktop recorded their signatures (one nullable number in, a
  nullable number out), so they moved from `library_only` to `functions`: 414 of 416 agree.

The probe query also showed that BIcraft's M parser refused `type type` (the primitive type of
types); fixed, with the regression test `type_type_is_the_primitive_type_of_types`.
