# Best-practice rule files (BPA format)

Crate: `crates/bpa` (`bicraft-bpa`, layer 4). Engine command: `model.analyze`. CLI: `bicraft-cli analyze`.

This note was written from Microsoft's published rule file
(microsoft/Analysis-Services `BestPracticeRules/BPARules.json`, MIT).

## Rule file

The file is a JSON array of rules. A `{"rules": [...]}` wrapper and a UTF-8 BOM are also
accepted. Keys are matched case-insensitively.

| Key | Use |
|---|---|
| `ID` | Required; entries without an ID are listed in `invalid_rules` |
| `Name`, `Category`, `Description` | Reported |
| `Severity` | 1 to 5, or `info`/`warning`/`error`. Reported as `info` (1), `warning` (2), `error` (3 and above) |
| `Scope` | Comma-separated object kinds (below) |
| `Expression` | Dynamic LINQ predicate. `true` means the object violates the rule. At most 64 KiB |
| `FixExpression` | Reported, never run |
| `CompatibilityLevel` | The rule is `skipped` when it is above the model's level |

## Scopes and objects

| Scope | BIcraft objects |
|---|---|
| `Model` | the model |
| `Table`, `CalculatedTable`, `CalculationGroup` | tables, split by kind (calculated: has a calculated partition; calculation group: `calculationGroup`) |
| `DataColumn`, `CalculatedColumn`, `CalculatedTableColumn` | columns by kind (row-number columns are never listed) |
| `Measure`, `KPI` | measures; the `kpi` object of a measure |
| `Hierarchy`, `Level` | `hierarchies` / `levels` of a table |
| `Relationship` | relationships |
| `Partition`, `NamedExpression` | partitions; shared expressions |
| `ModelRole`, `TablePermission` | roles; their table permissions (row filters) |
| `CalculationItem` | calculation items |
| `ProviderDataSource`, `StructuredDataSource` | the model's `dataSources` (`type: structured` or not) |
| `Perspective` | the model's `perspectives` |
| `Culture` | the model's `cultures` |

Other scopes (for example `Variation`) are reported as not supported. A rule whose
scopes are all unsupported is `not_evaluated`. A rule with some unsupported scopes is `partial`.

## Expression language

The grammar is in `crates/bpa/src/linq.rs`.

- **Literals and keywords:** numbers (type suffixes ignored), `"…"` and `'…'` strings (a doubled
  quote is a quote; backslashes are kept for regular expressions), `true`, `false`, `null`,
  `it`, `outerIt`, `current`.
- **Operators:**
  - logic: `and`/`&&`, `or`/`||`, `not`/`!`
  - comparison: `=`/`==`, `!=`/`<>`, `<`, `<=`, `>`, `>=`
  - arithmetic and text: `+` (string concatenation when one side is text), `-`, `*`, `/`, `%`/`mod`, `&`
  - conditional: `?:`, `iif(c, a, b)`, `??`
  - access: indexing `x[i]` (by position, or by name on object lists). On a string, `s[i]` is
    the character at position `i`, counted in Unicode scalar values (not UTF-16 units), as a
    one-character string; an index outside the string is an error on that object.
- **Comparing values:**
  - Enum values (`DataType`, `SummarizeBy`, cardinalities, `Type`, ...) compare with strings case-insensitively.
  - `Type.Member` with a known enum type name (`DataType.Double`, `CrossFilteringBehavior.BothDirections`, `AggregateFunction.None`, ...) is an enum literal.
  - Other strings compare case-sensitively, as in .NET.
- **Collections:**
  - `Any`, `All`, `Count`, `Where`, `Select`, `First`/`FirstOrDefault`/`Last`/`LastOrDefault`, `Sum`/`Min`/`Max`/`Average`, `Contains`, `Distinct`, `ToList`.
  - Predicates are expressions over `it`, or lambdas `x => …`.
  - Inside a predicate, `outerIt` is the `it` of the enclosing scope, as in Dynamic LINQ (its
    parser sets the parent to the current `it` when it enters a method's arguments). Directly
    inside one method, that is the object the rule runs on; two methods deep, it is the item of
    the outer method. `current` is always the object the rule runs on. `outerIt` outside any
    method is an error.
  - A collection method called on something that is not a collection (`Name.Any(…)`, or an
    unexposed JSON array) is an error naming the receiver; its predicate is not run.
- **Strings:**
  - `Length`, `Contains`/`StartsWith`/`EndsWith` (with an optional `StringComparison`), `IndexOf`/`LastIndexOf` (also `"OrdinalIgnoreCase"`).
  - `ToUpper`/`ToLower`, `Trim`/`TrimStart`/`TrimEnd`, `Replace`, `Substring`, `Split`, `ToCharArray`, `ToString`, `Equals`.
- **Static helpers:**
  - `string.IsNullOrEmpty`, `string.IsNullOrWhiteSpace`, `string.Concat`
  - `Convert.ToInt32/ToInt64/ToDouble/ToDecimal/ToString` (`null` converts to 0)
  - `Math.Abs/Min/Max/Round/Floor/Ceiling`
  - `char(n)`, `char.IsControl/IsWhiteSpace/IsLetter/IsDigit/IsLetterOrDigit/IsUpper/IsLower/IsPunctuation`, `char.ToUpper/ToLower` (on a one-character string, such as `Name[0]`)
  - `RegEx.IsMatch(input, pattern[, RegexOptions])`
- **Regular expressions:**
  - They use the workspace's own matcher (`bicraft_pbir::Regex`, a Pike VM with no backtracking), with a case-insensitive mode.
  - `(?i)` turns on ignore-case for the whole pattern. Patterns where `(?i)` appears after a case-sensitive letter are refused.
  - Syntax the matcher lacks (lookbehind, backreferences, ...) makes the rule not evaluated for that object.
- **Object properties:**
  - **Common:** `Name`, `ObjectTypeName` (`Table (DirectQuery)` for tables with a DirectQuery partition, `Calculated Table`, `Data Column`, ...), `ObjectType`, `DaxObjectName`, `DaxObjectFullName`, `DaxTableName`, `Table`, `Model`, `Description`, `IsHidden`/`IsVisible`, `GetAnnotation(name)`/`HasAnnotation(name)`, `InPerspective[name]`.
  - **Typed fields:** the model's typed fields (data type, expression, format string, summarize by, sort by column, data category, cardinalities, cross-filtering, partition source type and mode, ...).
  - **Usage lists:** `UsedInRelationships`, `UsedInSortBy`, `UsedInHierarchies`, `UsedInLevels`, `UsedInVariations`, `RowLevelSecurity` (non-empty row filters), `ObjectLevelSecurity` (the metadata permission in each role), `Members`, `UsedByPartitions`, `AllColumns`, `AllMeasures`, `AllCalculationItems`, ...
  - **Measure `DataType`:** the result type `bicraft_dax` infers for the expression (the
    validator of the whole model, so measure references are typed from their own expressions),
    with TOM's names: `Int64`, `Double`, `Decimal` (currency), `String`, `DateTime`, `Boolean`,
    `Variant`. A measure that is only `BLANK()` is `Int64`. A type that cannot be inferred is
    an error on that measure. See *Measure data types* below.
  - **Translations:** `TranslatedNames`, `TranslatedDescriptions` (model, tables, columns,
    measures, hierarchies, levels, perspectives, roles, shared expressions) and
    `TranslatedDisplayFolders` (columns, measures, hierarchies). Indexed by a culture object
    (`Model.Cultures[i]`, `it` inside `Model.Cultures.Any(…)`) or a culture name
    (case-insensitive); an unknown culture is an error. Enumerated (`.Any`, `.Count`), they give
    one string per model culture. An untranslated property is `""`. They read the
    `translations` tree of each `cultures[]` entry (M10.4). `Model.Cultures` lists `Culture`
    objects (`Name`, other properties from the TMSL entry).
  - **Other TMSL properties:** looked up by name (case-insensitive), with the TOM defaults for absent ones (`IsAvailableInMDX` true, `IsKey` false, `AlternateOf` null, ...).
  - **Unknown properties:** an error.
- **Dependencies:**
  - `ReferencedBy` lists the objects whose DAX references the object, from `bicraft_dax::deps`. It also offers `.AllMeasures` / `.AllColumns` / `.AllTables` (transitive), and `.Measures` / `.Columns` / `.Tables`.
  - `DependsOn` enumerates entries with `Key` (the referenced table, column or measure) and `Value` (one item per reference, with `FullyQualified`: the reference names its table). They come from the DAX parser.
  - An unqualified `[X]` is a measure when one is named X, otherwise a column of the home table.
- **`Tokenize()`:** on measures, columns, tables, calculation items, KPIs and row filters (an
  error on other objects; an empty list without DAX). The tokens come from `bicraft_dax`'s
  lexer, comments dropped. Each token has `Type`, `Text` (as written, quotes and brackets
  included), `Next` and `Previous` (`null` past either end) and `Index`. Bare upper-case names
  written exactly as a type below are type constants (`Type = DIV and Next.Type <>
  INTEGER_LITERAL`). The names are bicraft's; they follow the ones public rule files use.

  | Type | Token |
  |---|---|
  | `TABLE` | `'Quoted'` table name, or a bare name of a model table |
  | `VARIABLE` | a bare name declared by `VAR` in the expression |
  | `TABLE_OR_VARIABLE` | another bare name; also equal to `TABLE` and `VARIABLE` |
  | `COLUMN` | `[X]` after a table or variable name or `.`, or an unqualified column of the home table |
  | `MEASURE` | an unqualified `[X]` naming a measure |
  | `COLUMN_OR_MEASURE` | another `[X]`; also equal to `COLUMN` and `MEASURE` |
  | `FUNCTION` | a name followed by `(` |
  | `VAR`, `RETURN`, `IN`, `NOT`, `TRUE`, `FALSE`, `ASC`, `DESC` | keywords (not followed by `(`) |
  | `STRING_LITERAL`, `INTEGER_LITERAL`, `REAL_LITERAL` | literals (real: with `.` or an exponent) |
  | `PLUS`, `MINUS`, `MULT`, `DIV`, `EXP` (`^`), `CONCAT` (`&`) | arithmetic and text operators |
  | `EQUALS`, `STRICT_EQUALS` (`==`), `NOT_EQUALS`, `LESS_THAN`, `LESS_THAN_OR_EQUAL`, `GREATER_THAN`, `GREATER_THAN_OR_EQUAL` | comparisons |
  | `AND` (`&&`), `OR` (`\|\|`) | logical operators |
  | `OPEN_PARENS`, `CLOSE_PARENS`, `OPEN_CURLY`, `CLOSE_CURLY`, `COMMA`, `DOT` | punctuation |
  | `OTHER` | anything else (query parameters) |

  `AND`, `OR`, `NOT`, `TRUE` and `FALSE` are also rule-language keywords, so a rule cannot name
  those types as bare constants; it can compare `Type.ToString()` with a string.

## Bounds and failures

- **Parser:**
  - At most 20,000 tokens.
  - A tree depth of 256: every operator, member access and parenthesis counts, so evaluation recursion is bounded too.
  - 40 nested parentheses, arguments or branches.
- **Evaluation:**
  - 5 million steps per object and 100 million per rule.
  - Beyond that, the objects left are "not evaluated".
  - At most 4,096 compiled regular expressions are cached.
- **Errors:** any of these is an error on that object, never a `false`:
  - a type mismatch or a null dereference
  - an unknown property or method
  - an unsupported regular expression
  - a non-boolean result
- **Rule status:** `partial` when some objects failed, `not_evaluated` when all did. The first
  reason is reported.

## Measure data types

The engine of Power BI Desktop 2.158.1304.0 reported `TMSCHEMA_MEASURES.DataType` for 68
measures of our own (aggregations of each column type, `DIVIDE` and `/`, measure references and
arithmetic, `IF`/`SWITCH` with equal, mixed, missing and `BLANK()` branches, `FORMAT`, `&`,
comparisons, iterators, `ROUND`/`INT`/`CONVERT`, dates) on the DAX oracle model. The inferred
`DataType` agrees on all 68 (`measure_data_types_match_the_engine` in
`crates/bpa/src/tests_constructs.rs`). What the engine showed, and `bicraft_dax` now follows:

- currency (`Decimal`) stays currency through `AVERAGE`, `AVERAGEX`, `ROUND`, `INT` and `/` or
  `DIVIDE` by an integer; `DIVIDE` of two integers is `Double`;
- a `BLANK()` branch of `IF`/`SWITCH` takes the type of the other branches;
- `CONVERT(x, DOUBLE)` is `Double` (the type named by its second argument);
- integer and real branches give `Double`; a text and a number branch give `Variant`.

## Coverage of the default rules

On synthetic models in `crates/bpa/src/tests_default.rs`, all 71 rules are `evaluated`:

- A model that follows every rule gets no findings.
- A model that breaks every rule fires each of the 71 at least once.

Real PBIX models from the local test data were also checked; all 71 rules were evaluated on each.
