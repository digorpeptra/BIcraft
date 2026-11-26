# TMDL (reader and writer)

Crate: `crates/tmdl` (`bicraft-tmdl`). All test fixtures are synthetic models written for this
crate.

Sources:

- [Tabular Model Definition Language (TMDL) overview](https://learn.microsoft.com/en-us/analysis-services/tmdl/tmdl-overview)
  (syntax, folder layout, expressions, descriptions, `ref`, indentation, casing)
- [Power BI Project semantic model folder](https://learn.microsoft.com/en-us/power-bi/developer/projects/projects-dataset)
  (`definition/` folder of a `*.SemanticModel` item)
- [Roles object (TMSL)](https://learn.microsoft.com/en-us/analysis-services/tmsl/roles-object-tmsl)
  (a role member is a Windows member, without `identityProvider` and `memberType`, or an
  external member, with an optional `identityProvider` and a `memberType` of `auto`, `user` or
  `group`) and the TOM reference
  ([RoleMemberType](https://learn.microsoft.com/en-us/dotnet/api/microsoft.analysisservices.tabular.rolemembertype),
  [WindowsModelRoleMember](https://learn.microsoft.com/en-us/dotnet/api/microsoft.analysisservices.tabular.windowsmodelrolemember):
  a Windows member has no member type)
- **Black-box observation of the public TOM library** (17.0.87.18, the version installed with
  Power BI Desktop 2.158.1304.0): its public
  `Microsoft.AnalysisServices.Tabular.TmdlSerializer` (`SerializeDatabaseToFolder`,
  `DeserializeDatabaseFromFolder`) and `JsonSerializer.DeserializeDatabase` were called from
  PowerShell on synthetic `model.bim` files to see what it writes and what it accepts.
  Tabular Editor 2's AMO (19.117) produced byte-identical output. The rules below are
  descriptions of observed output. The fixtures in
  `crates/tmdl/tests/fixtures/{contoso,edge,order}` are that serializer's output for our own
  synthetic models, stored with LF line endings (tests convert them to CRLF).
- The names of five Desktop annotations (`SummarizationSetBy`, `PBI_FormatHint`,
  `PBI_ResultType`, `PBI_NavigationStepName`, `PBI_QueryOrder`) as they appear in the projects
  Desktop saves.

## API

- `read_folder(definition_dir)` reads every `*.tmdl` file below the folder (extension matched
  case-insensitively, up to 4 sub-folder levels, symlinks not followed), in byte-sorted relative
  path order. Caps: 64 MiB per file, 512 MiB and 100 000 files in total. Files must be UTF-8
  (a BOM is accepted).
- `read_documents(&[(relative_path, text)])` does the same from memory.
- `parse(file, text)` exposes the generic tree (`Document` / `Node`).
- `write_documents(&model) -> Vec<(path, text)>` produces the documents, sorted by path.
- `write_folder(&model, definition_dir)` writes them (see [Folder writing](#folder-writing)).
- `file_stem(name)` is the file-name rule.
- `bicraft-pbip` calls `read_folder` / `write_folder` on `<Name>.SemanticModel/definition`.

Errors carry the relative file path and 1-based line (`tables/Sales.tmdl:12: ...`).

## Parsing

Two steps. First, each document becomes a tree of *entries* nested by indentation. Each line is:

| Line | Entry |
|---|---|
| `kind name` / `kind 'quoted name'` | object declaration |
| `kind name = value` | declaration with its default property (expression, partition source type, annotation value, ...) |
| `key: value` | text property; surrounding `"` stripped, `""` unescaped, value trimmed |
| `key = value` | expression property, or a nameless object with a default property (`source =`, `formatStringDefinition =`) |
| `key` | boolean shortcut (`isHidden` = true), or a nameless object when deeper lines follow |
| `ref kind name` | reference that only fixes collection order |
| `/// text` | description of the next entry; a blank line in between drops it |

- Keywords are case-insensitive (canonical casing is restored for known keys). Names are quoted
  with `'...'` (`''` escapes a quote); unquoted names end at whitespace or `=`. `key: value`
  has no space before the colon; `kind :name` (space, then a colon glued to more text) declares
  a name starting with `:`.
- Indentation: a tab is one level; for space-indented documents the level width is the smallest
  space-only indent of an entry line (fenced and multi-line expression bodies are not entries;
  without any indented entry, the expression bodies give it), 4 if none. LF and CRLF are
  accepted. Depth is capped at 64 levels.
- Expressions after `=`:
  - single line: the rest of the line, trimmed;
  - multi-line (nothing after `=`): the following lines that are blank or indented at least to
    the expression base, which is **two levels below an object's line** (`measure X =`,
    `source =`, `formatStringDefinition =`, `linguisticMetadata =`, ...) and **one level below a
    property's line** (`targetExpression =`, `sourceExpression =`, `query =`). That many levels
    are stripped; relative indentation is kept; whitespace-only lines are empty lines; trailing
    empty lines and trailing whitespace are dropped. For nameless objects a first line indented
    only one level is accepted (lenient);
  - fenced: `= ```` then the lines up to a line that is exactly ```` ``` ```` (indentation
    aside), verbatim; the closing delimiter's indentation is the margin removed from each line.
    ```` = ```x``` ```` on one line is also accepted. An unterminated fence is an error.

## Mapping to `SemanticModel` (lossless)

Each object is first converted into the JSON object TMSL would hold for it, then the typed fields
are taken out; everything else stays in the owning object's `extra`, in order:

- object keyword -> TMSL collection: `hierarchy` -> `hierarchies`, `level` -> `levels`,
  `calculationItem` -> `calculationItems`, `variation`, `annotation`, `extendedProperty`,
  `changedProperty`, `queryGroup` (name key `folder`), `dataSource`, `function`, `perspective`,
  `perspectiveTable` -> `tables`, `perspectiveColumn`/`Measure`/`Hierarchy` -> `columns`/
  `measures`/`hierarchies`, `cultureInfo` -> `cultures`, `member` (name key `memberName`,
  default property `memberType`), `tablePermission`, `columnPermission` (default
  `metadataPermission`). Unknown named keywords pluralize (`foo` -> `foos`).
- single nested objects: `kpi`, `calculationGroup`, `refreshPolicy`, `alternateOf`,
  `formatStringDefinition`, `detailRowsDefinition`, `defaultDetailRowsDefinition`,
  `multipleOrEmptySelectionExpression`, `noSelectionExpression` (default `expression`),
  `linguisticMetadata` (default `content`), and any nameless entry with children
  (`dataAccessOptions`, `connectionDetails`, ...).
- `= value` -> the kind's default property (`expression` unless listed above); a partition's
  `= m` is `source.type`, and `source = ...` is `source.expression` (or, as a nested block,
  the rest of `source`).
- `key: value`: double-quoted values are text; object-name keys (`sortByColumn`, level `column`,
  `relationship`, `queryGroup`, `dataSource`, `baseTable`) are unquoted names; `defaultHierarchy`
  and `baseColumn` become `{"table", "hierarchy"/"column"}`; known text keys stay text; other
  values become `true`/`false`/`null`/numbers when their JSON spelling is exactly the text
  (`007`, `1.50` stay text). A bare key is `true`. `description` (`///`) is `description`.
- `linguisticMetadata` content and `extendedProperty` values are JSON when the text is exactly
  `serde_json`'s pretty print of it (Desktop's format), text otherwise; JSON extended properties
  get `type: json`, text ones no `type` (TOM's TMSL leaves `string` implied).
- Data sources: `= provider` is a provider data source, which TMSL writes without `type`; no
  value is `type: structured`. `options = <JSON>` and `credential = <JSON>` (two levels deeper,
  like an object's default property) are JSON objects; a credential's `authenticationKind`,
  `privacySetting`, `username`, `password` and `encryptConnection` properties below it are its
  `AuthenticationKind`, ... fields.
- Role members: `member X = activeDirectory` is a Windows member (TMSL: no `identityProvider`,
  no `memberType`); otherwise the member is external with `identityProvider` `AzureAD` unless
  given, and `memberType` `user` when there is no value (`= auto` is TMSL's implied type, so it
  has no `memberType`). The TMDL overview lists `MemberType` as the default property of an
  external member only. A TMSL member with a `memberType` and no `identityProvider` matches the
  external member of the published TMSL schema, not the Windows one: bicraft writes it with its
  type and no `identityProvider` line, so it reads back with the default provider (`AzureAD`):
  the type survives, the absence of a provider does not. Before this, such a member was written
  `= activeDirectory` and lost its type. Black-box check with Power BI Desktop 2.159.883.0 on a
  PBIP BIcraft generated (one table, a card, role `R`; opened, then saved with Desktop's Save
  button): `member X` and `member X = group` without an `identityProvider` line open, and Desktop
  saves them back unchanged (it adds only the role's `PBI_Id` annotation); the same members with
  `identityProvider: ""` are refused (Desktop opens an empty untitled report instead, no dialog
  read).
- Name lists are one line per name: a calendar column group's `associatedColumn:` / `column:`
  lines are TMSL `associatedColumns` / `columns`, a field parameter's
  `relatedColumnDetails` `groupByColumn: X` lines are `groupByColumns: [{"groupingColumn": X}]`.
- `policyRange` partition dates are `yyyy-MM-dd` in TMDL (Desktop's reader refuses a time of
  day) and `yyyy-MM-ddT00:00:00` in TMSL.
- `translations` blocks map `model`/`table`/`column`/`measure`/... to TMSL collections and
  `caption`/`description`/`displayFolder` to `translatedCaption`/... (always text). Entries
  name their objects (`table Sales` / `column Amount`), so renames rewrite them. bicraft's
  `translation.set` keeps them in model order and drops entries that translate nothing, as the
  metadata reader returns them, so TMDL, TMSL and PBIX round trips give back the same tree.
- Implied values are restored: level and calculation item `ordinal` (their position), a data
  source's `type: structured`, an expression's `kind: m`.

Typed fields (as in the TMSL reader, a property is typed only when the writer emits it again the
same way):

| TMDL | Model |
|---|---|
| `database <name>` / `compatibilityLevel:` / `///` | `name`, `compatibility_level`, `database_extra.description` |
| `model` / `culture:` / `///` / top-level `annotation` | `culture`, `description`, `annotations`; model name other than `Model` in `extra.name` |
| top-level `queryGroup`, `perspective`, `cultureInfo`, `dataSource`, `function`, `extendedProperty`, ... | `extra` collections |
| `table` / `lineageTag` / `isHidden` / `///` | `Table` (partial declarations across files merge) |
| `column X [= dax]` + `dataType`, `sourceColumn`, `formatString`, `displayFolder`, `summarizeBy`, `sortByColumn`, `dataCategory`, `lineageTag`, `isHidden`, `annotation` | `Column` |
| `measure X = dax` + `formatString`, `displayFolder`, `lineageTag`, `isHidden`, `annotation` | `Measure` |
| `partition X = type` + `mode`, `source` | `Partition` (`extra`: `queryGroup`, `description`, ...; `source_extra`: `query`, `dataSource`, `entityName`, ...) |
| `relationship` + `fromColumn`/`toColumn: T.C`, `isActive`, `crossFilteringBehavior`, `from/toCardinality` | `Relationship`; an endpoint that is not exactly `Table.Column` stays as text in `extra` and the relationship is kept |
| `expression X = m` + `lineageTag`, `kind`, `///` | `NamedExpression` (`kind` defaults to `m`) |
| `role` + `modelPermission`, `tablePermission T = dax` | `Role`, `TablePermission` (`members`, `description` in `extra`) |
| `ref table/role/perspective/cultureInfo X` | order of those collections |

Explicit defaults stay in `extra` (`isHidden: false`, `isActive`, `kind: m`) so they are
written back. Declaring the same column or measure twice in one table is an error, as in TOM.

## Writer conventions

Observed in the TOM serializer's output and reproduced byte for byte (the five fixtures round-trip
exactly).

- **Layout**: `database.tmdl`, `model.tmdl`, `relationships.tmdl`, `expressions.tmdl`,
  `dataSources.tmdl`, `functions.tmdl` (each only when non-empty, except the first two),
  `tables/<table>.tmdl`, `roles/<role>.tmdl`, `perspectives/<perspective>.tmdl`,
  `cultures/<culture>.tmdl`. `model.tmdl` holds the `model` block, then the model's own child
  objects at the top level (query groups, extended properties, annotations), then `ref table`,
  `ref role`, `ref perspective`, `ref cultureInfo` groups in model order.
- **Text**: UTF-8 without BOM; tab indentation; **CRLF** line endings on every platform, so
  output is byte-identical on Windows and Linux. Desktop writes CRLF on Windows (Microsoft Learn,
  PBIP overview; P-EOL in `packaging-verification.md`); matching it keeps diffs against
  Desktop-saved projects minimal. Readers accept
  both.
- **Blank lines**: every top-level object and every `ref` group is followed by a blank line;
  every child object is preceded by one. `member` runs have one only before the first and after
  a member with child objects; a member's own first child object follows it directly. Properties,
  nested property blocks (`dataAccessOptions`, a non-M partition's `source`, `credential`) and
  `translations` have none.
- **Order**: properties first, then child objects, both in Desktop's order per object type
  (`crates/tmdl/src/schema.rs`; e.g. measures before columns, `dataType` first and
  `sourceColumn`, `sortByColumn` last in columns, `fromColumn`/`toColumn` last in
  relationships; tables in [Property order](#property-order)), independent of the order the
  properties arrive in. Unknown keys keep their order after the known ones.
- **Names** are quoted with `'` (`''` inside) when empty or containing whitespace, `.`, `=` or
  `'`; leading digits, keywords (`table`, `ref`), `-`, `/`, `//`, `///`, `:`, `"`, `#`, `,`,
  `;`, `(`, `[`, `{`, `` ` ``, `\`, non-ASCII stay bare (checked against TOM for each). bicraft
  also quotes a leading `:` (Desktop writes it bare but cannot read it back). Name-valued
  properties and `Table.Column` references use the same rule per part.
- **Text values** are double-quoted (`""` inside) when empty, starting **and** ending with `"`
  (`"q"` -> `"""q"""`, but `"$"#,0` stays bare), or starting or ending with whitespace; for keys
  not known to be text, also when they would read as a boolean, `null` or a number. Text
  containing a line break is written as `key = <expression>`.
- **Booleans**: `true` is the bare key, `false` is `key: false`. Numbers as JSON.
- **Expressions**: one line -> `= text` on the line. Several lines -> `=` then each line at the
  expression base, empty lines written as the base indentation. Fenced with ```` ``` ```` when
  the first non-empty line starts with whitespace, a line ends with whitespace, or the text ends
  with a line break, unless a line is ```` ``` ```` apart from whitespace (it would close the
  fence; TMDL has no escape): then the indented form is used, losing only whitespace and empty
  lines at the very end. Line breaks are normalized to the file's CRLF (TMSL `\r\n` inside an
  expression is not preserved, as in Desktop). Empty measure expressions and annotation values
  are written without `=`.
- **Descriptions**: one `/// ` line per line of text (`/// ` alone for an empty line), trailing
  whitespace kept.
- **JSON** (linguistic metadata, JSON extended properties): pretty-printed, two spaces.
- **Implied values omitted**: `kind: m`, data source `type: structured`, extended property
  `type` matching the value, `type: calculatedTableColumn` in a calculated table, a calculation
  group's default partition (see above). Levels and calculation items are written sorted by
  `ordinal` (items without one last, in their order) and never with an `ordinal`: TMDL has none,
  Desktop's reader numbers them by position and ignores an explicit `ordinal:`.
- **TMSL only** (Desktop's serializer drops them and its reader refuses them): a role member's
  `memberId`, multidimensional `sets` (table and perspective table), a time of day on partition
  dates. bicraft drops them too.

Lossless choices that differ from Desktop's output (Desktop's reader accepts all of them):
explicit defaults from `extra` are written (`isHidden: false`, `isNameInferred: false`,
`encryptConnection: false`); `isDataTypeInferred` and the `dataType` it would make Desktop omit
are both written, and so is a measure's `dataType`; a calculation group partition other than the
default one is written; an empty calculated-column expression is an empty fence. `rowNumber`
columns (TMSL only) are never written, as Desktop never declares them in TMDL.

## TOM oracle

During development, bicraft is compared with the TOM serializer of Power BI Desktop
2.158.1304.0 (its client TOM library, 17.0.87.18), through the maintainers' development oracles,
kept outside this repository. Nothing of Desktop is committed. The comparison converts between
TMSL and TMDL with TOM's serializers, TOM-normalizes the model, diffs TOM's TMDL with bicraft's,
has TOM read bicraft's TMDL and bicraft rewrite TOM's.

| File | What it does |
|---|---|
| `cargo xtask tmdl-model [LEVEL] [--out FILE]` (xtask/src/oracles/tmdl_model.rs) | Writes our synthetic TMSL model: every object kind TOM serializes to TMDL (calendars, functions, binding info, data coverage definitions; field parameters and role members with child objects come from `objects.bim`, below) and the text edge cases (quoting, escaping, whitespace, fences, CRLF, multi-line descriptions and annotations) |

`crates/tmdl/tests/fixtures/tom` is TOM's TMDL for `cargo xtask tmdl-model` (compatibility level 1706),
`fixtures/tom-objects` TOM's TMDL for `crates/metadata/tests/engine/objects.bim`; both are read
and rewritten byte for byte, and `tom_fixture_reads_like_tom` checks the TMSL shape TOM's reader
gives. Results on 2026-10-14:

- bicraft rewrites TOM's TMDL byte for byte, and reads it into the same TMSL as TOM's reader
  (apart from whitespace TOM trims in descriptions and the explicit `type` of string extended
  properties, since fixed).
- TOM reads everything bicraft writes. The remaining differences in bicraft's TMDL are the
  lossless choices above.
- Found and fixed: provider data sources, `options`/`credential`, role members, `memberId`,
  translation property order, `alternateOf`/partition source/data source/expression property
  order, `parameterValuesColumn: Table.Column`, quoting of text starting with `"`, dates,
  ordinals, the implied calculation group partition and calculated table column type, sets,
  blank lines around role members with child objects, a field parameter's `groupByColumn:`
  lines, the implied `type` of string extended properties, and
  the object kinds of TOM 17 (`calendar`, `calendarColumnGroup`, `dataCoverageDefinition`,
  `bindingInfo`, `relatedColumnDetails`) with the new model, table, column and expression
  properties.
- Compatibility level 1000000 (G16, 2026-10-14): Desktop 2.158's client TOM refuses
  `analyticsAIMetadata`, `excludedArtifacts`, `metadataAccessPolicy` and the Direct Lake
  indexing behaviors as unknown TMSL, so there is no TOM TMDL to compare with. bicraft writes
  nameless `excludedArtifact` blocks (`artifactType`, `reference` as a nested block) and named
  `analyticsAIMetadata` objects, and reads them back losslessly
  (`tom_edge_cases.rs::preview_objects_survive_tmdl`).
- TOM itself loses on TMDL: CRLF inside expressions and descriptions, leading and trailing
  whitespace of descriptions, `memberId`, a role member's annotations and extended properties
  (written but not read back), `EncryptConnection: false`, password values (masked).

## Property order

Derived on 2026-10-15 from the TOM of Power BI Desktop 2.158.1304.0
(client TOM library 17.0.87.18): the maintainers' development oracles build, through TOM's
object model, one object of each kind with **every writable scalar property set** to a
non-default value (compatibility level 1000000, so that TOM accepts all of them), plus an
annotation, extended property and changed property where the kind has them; TOM serializes it
to TMDL. The result is `crates/tmdl/tests/fixtures/tom-props` (TOM's TMSL of
the same model: `crates/tmsl/tests/fixtures/tom-props.bim`). TOM writes a fixed order per
object kind, whatever order the properties were set in or appear in the TMSL. The `*` slot of
`crates/tmdl/src/schema.rs` (properties TOM does not know) cannot be checked: TOM refuses
unknown TMSL properties, so it never writes one.

| Object | Properties, in TOM's order | Child objects, in order |
|---|---|---|
| model | `storageLocation`, `defaultMode`, `defaultDataView`, `culture`, `collation`, `defaultPowerBIDataSourceVersion`, `forceUniqueNames`, `discourageImplicitMeasures`, `dataSourceVariablesOverrideBehavior`, `dataSourceDefaultMaxConnections`, `sourceQueryCulture`, `mAttributes`, `discourageCompositeModels`, `disableAutoExists`, `maxParallelismPerRefresh`, `maxParallelismPerQuery`, `directLakeBehavior`, `valueFilterBehavior`, `selectionExpressionBehavior`, `metadataAccessPolicy`, `defaultDirectLakeIndexingBehavior`, `defaultMeasure: Table.Measure`, then the blocks `dataAccessOptions` (`fastCombine`, `legacyRedirects`, `returnErrorValuesAsNull`) and `automaticAggregationOptions` (`queryCoverage`, `detailTableMinRows`, `aggregationTableMaxRows`, `aggregationTableSizeLimit`) | `queryGroup`, `extendedProperty`, `bindingInfo`, `annotation` (`changedProperty`: not on a model) |
| table | `isHidden`, `showAsVariationsOnly`, `isPrivate`, `alternateSourcePrecedence`, `excludeFromModelRefresh`, `lineageTag`, `sourceLineageTag`, `systemManaged`, `excludeFromAutomaticAggregations`, `directLakeIndexingBehavior`, `dataCategory` | `defaultDetailRowsDefinition`, `refreshPolicy`, `calculationGroup`, `measure`, `column`, `hierarchy`, `partition`, (`calendar`), `extendedProperty`, `changedProperty`, `annotation` |
| column (data, calculated, calculated table) | `dataType`, `isHidden`, `isUnique`, `isKey`, `isNullable`, `formatString`, `isAvailableInMdx`, `keepUniqueRows`, `displayOrdinal`, `sourceProviderType`, `displayFolder`, `encodingHint`, `lineageTag`, `sourceLineageTag`, `stringIndexingBehavior`, `fullTextIndexingBehavior`, `dataCategory`, `alignment`, `tableDetailPosition`, `isDefaultLabel`, `isDefaultImage`, `summarizeBy`, then `expressionContext` (calculated) or `isNameInferred` (calculated table), `sourceColumn` (data, calculated table), `sortByColumn` | `relatedColumnDetails`, `alternateOf`, `variation`, `extendedProperty`, `changedProperty`, `annotation` |
| measure | `formatString`, `isHidden`, `isSimpleMeasure`, `displayFolder`, `lineageTag`, `sourceLineageTag`, `dataCategory` | `kpi`, `detailRowsDefinition`, `formatStringDefinition`, `extendedProperty`, `changedProperty`, `annotation` |
| kpi | `targetDescription`, `targetExpression`, `targetFormatString`, `statusGraphic`, `statusDescription`, `statusExpression`, `trendGraphic`, `trendDescription`, `trendExpression` | |
| partition | `mode`, `dataView`, `queryGroup`, `source` | `dataCoverageDefinition`, `extendedProperty`, `annotation` (no `changedProperty`) |
| hierarchy | `isHidden`, `displayFolder`, `hideMembers`, `lineageTag`, `sourceLineageTag` | `level`, `extendedProperty`, `changedProperty`, `annotation` |
| level | `lineageTag`, `sourceLineageTag`, `column` (`ordinal` is never written) | `extendedProperty`, `changedProperty`, `annotation` |
| relationship | `isActive`, `crossFilteringBehavior`, `joinOnDateBehavior`, `relyOnReferentialIntegrity`, `securityFilteringBehavior`, `fromCardinality`, `toCardinality`, `fromColumn`, `toColumn` | `extendedProperty`, `changedProperty`, `annotation` |
| calculationGroup | `precedence` | `multipleOrEmptySelectionExpression`, `noSelectionExpression` (each with a `formatStringDefinition` child), `calculationItem`, `annotation` |
| calculationItem | none besides the expression (`ordinal` is never written) | `formatStringDefinition` (no annotations in TOM) |
| expression | (`kind`), `mAttributes`, `lineageTag`, `sourceLineageTag`, `remoteParameterName`, `queryGroup`, `parameterValuesColumn: Table.Column`, `expressionSource` | `extendedProperty`, `annotation` |
| function | `isHidden`, `lineageTag`, `sourceLineageTag` | `extendedProperty`, `changedProperty`, `annotation` |
| refreshPolicy | `policyType`, `mode`, `rollingWindowGranularity`, `rollingWindowPeriods`, `incrementalGranularity`, `incrementalPeriods`, `incrementalPeriodsOffset`, `pollingExpression`, `sourceExpression` | |
| alternateOf / variation | `summarization`, `baseColumn` / `isDefault`, `relationship`, `defaultHierarchy` | — / `extendedProperty`, `annotation` |

Notes:

- **`isHidden` comes before `isKey`.** A peer report (2026-10-15) said Desktop writes
  `dataType`, `isKey`, `isHidden`; TOM writes `dataType`, `isHidden`, `isUnique`, `isKey`.
  bicraft already wrote TOM's order; `tom_prop_order.rs` pins it whatever order commands set the properties in.
- Fixed on 2026-10-15: `dataAccessOptions` now comes before `automaticAggregationOptions`, the
  properties inside both blocks are sorted (`NESTED_PROPS`; they used to keep the TMSL order),
  `expressionContext` has its place (it was written last, after `sortByColumn`), and
  `defaultMeasure` is a `Table.Measure` reference (`{"table", "measure"}` in TMSL; a TMSL
  `defaultMeasure` used to be written as a bare `defaultMeasure` line).
- TOM writes an unknown `dataAccessOptions` key as JSON after `dataAccessOptions =`, before the
  known ones; bicraft writes it as a property of the block (not seen in Desktop files).
- TOM's TMDL writer formats numbers with the **current culture** (`queryCoverage: 0,5` on a
  French system); the TOM oracle runs with the invariant culture, and bicraft always
  writes `0.5`.
- With `isDataTypeInferred: true`, TOM writes neither `dataType` nor `isDataTypeInferred`
  (bicraft writes both, see the lossless choices above), so the synthetic model has no such
  column.

**TMSL (`model.bim`).** TOM's `JsonSerializer` has its own order (`fixtures/tom-props.bim`: for
a column `name`, `dataType`, `description`, `isHidden`, `isUnique`, `isKey`, `isNullable`,
`sourceColumn`/`expression`, `formatString`, `isAvailableInMdx`, `sortByColumn`, ...; tables
list `columns`, `partitions`, `measures`, `hierarchies`; a calculated column starts with
`type`), and a PBIT `DataModelSchema` saved by Desktop uses it. A PBIP `model.bim` does not.
Observed on Desktop 2.160.1151.0 (2026-11-25): BIcraft wrote a PBIP project of its own star
model with the keys of every JSON object in reverse order, Desktop opened it and saved it
(ribbon Save, TMDL upgrade declined). In the `model.bim` it wrote (`crates/tmsl/tests/fixtures/desktop-sorted.bim`), every object, at every
depth, has `name` first and its other keys in alphabetical order, and arrays keep their order
(tables, columns, measures, hierarchies, levels, relationships, roles, table permissions,
partitions and annotations stay in model order, not by name). That model's keys sort the same
case-sensitively or not, so how Desktop orders keys that differ only in case or punctuation was
not observed; bicraft uses ordinal (UTF-16 code unit) order (`A1` < `B` < `_x` < `a` < `aB` <
`ab`). Object kinds that model lacks (KPIs, `dataAccessOptions`, `defaultMeasure`, calculation
groups, extended property JSON values) get the same rule in bicraft; that is not verified.
`bicraft_tmsl::write` sorts every object, those it keeps in `extra` (hierarchies, KPIs, refresh
policies, calculation groups, unknown properties) included.
`round_trip.rs::reversed_keys_are_written_in_the_order_desktop_saved` checks that the reversed
input comes back as Desktop's file byte for byte, and
`round_trip.rs::tom_ordered_input_is_written_like_a_desktop_pbip_model_bim` checks the same rule
on every property TOM writes (`fixtures/tom-props.pbip.bim` is TOM's output sorted by it; Desktop
no longer opens that compatibility-level-1000000 model as a project). bicraft writes the same
sorted text into a PBIT `DataModelSchema` (Desktop reads either order); TOM's JSON order is not
reproduced there. `crates/pbip/tests/tom_props.rs` checks that TOM's JSON order, read through
`bicraft_tmsl` and written by `bicraft_tmdl`, gives TOM's TMDL byte for byte.

## New models

Observed on Desktop 2.158.1304.0 (2026-10-14): the model of a blank Desktop file (TOM, read
through its local engine) and the `model.tmdl` / `database.tmdl` Desktop writes when it saves a
project bicraft created.

| Property | Blank Desktop model | `model.new` / `project.new` |
|---|---|---|
| `compatibilityLevel` | 1606 (a 1567 project is upgraded to 1606 on save) | 1606 |
| `culture` | the Windows display language | the `culture` parameter (default en-US) |
| `sourceQueryCulture` | same as `culture` | same as `culture` |
| `defaultPowerBIDataSourceVersion` | `powerBI_V3` | `powerBI_V3` |
| `dataAccessOptions` | `legacyRedirects`, `returnErrorValuesAsNull` | same |
| `valueFilterBehavior` | `independent` | `independent` |
| `discourageImplicitMeasures` | false (absent) | absent |
| `cultures` | one linguistic-metadata culture object | none (Desktop adds it) |
| annotation `__PBI_TimeIntelligenceEnabled` | 1 | 0: auto date/time stays off, so Desktop does not add hidden `LocalDateTable_…` tables. Without the annotation Desktop applies its own option (on by default) at the first refresh: it added the annotation with 1, a `DateTableTemplate_…` table, a `LocalDateTable_…` per date column and their variations (first-save check, 2026-10-14) |

Without `powerBI_V3`, Desktop does not refresh M partitions (`Partition_<guid>` not
recognized); with it, a bicraft project with M (`#table`) partitions opens and refreshes. Models
read from files keep their own settings: bicraft never adds these.

## Desktop's default annotations (M9.7)

Objects bicraft creates get the properties and annotations Power BI Desktop gives the objects
it creates (`crates/engine/src/desktop_defaults.rs`), so that Desktop's first save of the
project has nothing to add. Objects read from files and objects changed by `*.set` keep theirs
byte for byte.

| Object | Written by bicraft |
|---|---|
| table with an M partition (`table.add`, `table.import`, first `partition.set`/`partition.add`) | `PBI_ResultType = Table`; `PBI_NavigationStepName = Navigation` when the query has a `Navigation` step (written before `PBI_ResultType`) |
| shared expression (`expression.add`) | `PBI_ResultType` = the result type the text tells (`Table`, `Text`, `Number`, `Function`, …; see `data-sources.md`), `PBI_NavigationStepName` as for tables |
| column (`column.add`, `table.import`) | `summarizeBy` (numbers `sum`, others `none`) and `SummarizationSetBy = Automatic`, or `User` when `summarize_by` was given |
| `int64` column | `formatString: 0` |
| `double` column | `PBI_FormatHint = {"isGeneralNumber":true}` (after `SummarizationSetBy`) |
| `boolean` column | `formatString: """TRUE"";""TRUE"";""FALSE"""` |
| `dateTime` column from an M `date` | `formatString: Long Date`, `UnderlyingDateTimeDataType = Date` |
| `dateTime` column from an M `time` | `formatString: Long Time`, `UnderlyingDateTimeDataType = Time` |
| `dateTime` column from an M `datetime` | `formatString: General Date` |
| model | `PBI_QueryOrder`: JSON list (compact, `["A","B"]`) of the query names, kept up to date (see `docs/mcp.md`) |

A given `format_string` wins over these. The M type of a loaded column is known for
`table.import` (the evaluator keeps it, `TableData::date_kind`; a CSV's date-only columns are
written as `type date`). For `column.add` of a `dateTime` data column (and `column.set` turning
a data column into `dateTime`) it is read from the text of the table's M query without running
it (`bicraft_data::m::Analysis::date_kinds`): a typed `#table` literal, `Table.TransformColumnTypes`
(`type date`, `Date.Type`, …), a typed `Table.AddColumn`, carried through `Table.RenameColumns`,
the last step winning; then from the shared queries the partition references. When the text
does not tell, no format is written. `column.set` changing a data column's type replaces a
default format string (or none) with the new type's default and moves
`UnderlyingDateTimeDataType` with it; a custom format string is kept. Not written: the currency
format and `PBI_FormatHint = {"currencyCulture": …}` of `decimal` columns, on purpose: Desktop
takes them from the machine's regional format, not from the model culture (see *Currency
columns* below), so no rule bicraft could apply on any host gives Desktop's result; the
`PBI_Id` annotation Desktop gives calculated tables,
and the columns of a calculated table added by `table.add` (Desktop's engine computes them on
open).

Evidence, Desktop 2.158.1304.0 (2026-10-14):

- The annotation names (`SummarizationSetBy`, `PBI_FormatHint`, `PBI_ResultType`,
  `PBI_NavigationStepName`, `PBI_QueryOrder`) are those of Desktop-saved TMDL and of published
  Microsoft samples.
- A project bicraft wrote with every kind of object (M tables with every column type, an M
  table without columns, a parameter, a shared query, a calculated column, a measure, a
  calculated table) opens in Desktop: its engine holds every object ready, no error, the
  annotations as written, and computes the calculated table's columns.
- The values follow the TMDL Desktop saves in published Microsoft samples.
- **First-save diff in Desktop's UI** (2026-10-14, Desktop 2.158.1304.0, session locked; the
  ribbon's Save and Refresh buttons invoked through UI Automation, no keyboard input). Project:
  `table.import` of `#table(type table [I = Int64.Type, N = number, C = Currency.Type, D = date,
  DT = datetime, T = time, S = text, B = logical], …)`, the same query as a shared expression
  and a `table.add` M partition referencing it without columns (Desktop creates the columns on
  refresh, the reference for its own defaults), a parameter, a calculated column, a measure, a
  calculated table, then `project.new`. Open → Save, then Refresh → Save, diffed with ours:
  - Fixed from it: the boolean format string was stored already TMDL-escaped, so the files had
    it quoted twice (`"""""""TRUE""""…`); Desktop keeps a format string as it is, so it would
    have stayed wrong. It is now TMSL `"TRUE";"TRUE";"FALSE"` (TMDL
    `"""TRUE"";""TRUE"";""FALSE"""`, byte for byte Desktop's). And new models now write
    `__PBI_TimeIntelligenceEnabled = 0` (see *New models*).
  - After both fixes, the semantic model files Desktop rewrites are identical to ours except
    Desktop-only changes: `database` loses its name, `annotation PBI_ProTooling = ["DevMode"]`
    is added to `model.tmdl`, `diagramLayout.json` and `.pbi/` files appear, `$schema` is
    removed from `definition.pbism`, `definition.pbir` and the `.pbip`, and every JSON file is
    rewritten with CRLF line ends (bicraft writes LF; TMDL is CRLF on both sides, unchanged).
  - Desktop's engine fills in a calculated table's columns at the first save
    (`isNameInferred`, `sourceColumn: Kinds[S]`, no data type until a refresh): bicraft leaves
    them to it, as documented above.
  - At refresh Desktop re-derives what it marks `SummarizationSetBy = Automatic`: it
    auto-detected a one-to-one relationship between the two tables' unique `I` columns and set
    `summarizeBy: none` on both (data-driven, not a default). The columns it created match
    ours property for property (`0`, `isGeneralNumber`, `Long Date` + `Date`, `General Date`,
    `Long Time` + `Time`, the boolean format), except the currency column: Desktop writes the
    machine culture's currency format and `PBI_FormatHint = {"currencyCulture": …}` (not
    written by bicraft, as above).

### Currency columns

Desktop 2.158.1304.0 (Store, French UI, Windows regional format fr-FR), 2026-10-15: two
projects of ours, identical but for the model culture (`en-US`, `ja-JP`), each
with one M table `#table(type table [C = Currency.Type, N = number, F = Decimal.Type, I =
Int64.Type], …)` and no columns; opened, refreshed (ribbon Refresh) and saved in Desktop. Both
models got, on `C` (`dataType: decimal`):

```
formatString: #,0.###############\ "€";-#,0.###############\ "€";#,0.###############\ "€"
annotation PBI_FormatHint = {"currencyCulture":"fr-FR"}
```

so the format and the hint come from the machine's culture (Desktop's regional setting), not
from the model culture; the other columns got the defaults of the table above (`N` and `F`
`isGeneralNumber`, `I` `0`). bicraft's `column.add` and type changes therefore write no
currency format (a bicraft model is the same on every host; writing the en-US or the model
culture's format would differ from what Desktop writes on the user's machine anyway). Whether a refresh adds the format to a currency column that
already exists without one was not tried.

## File names

`file_stem(name)` (Desktop's rule plus safety additions):

- the characters Windows forbids in file names, `" * / : < > ? \ |`, and a leading space are
  percent-encoded as `%XX` (uppercase hex of the UTF-8 bytes), as Desktop does: `a/b` -> `a%2Fb`,
  `../x` -> `..%2Fx`, ` lead` -> `%20lead`;
- bicraft also encodes control characters (Desktop fails on them) and the first character of a
  Windows device name (`CON`, `PRN`, `AUX`, `NUL`, `COM0-9`, `LPT0-9`, also before an extension:
  `nul.txt` -> `%6Eul.txt`);
- stems longer than 120 bytes are truncated at a character boundary (to keep paths under
  Windows' 260-character limit);
- `%` itself is not encoded (Desktop does not), so `a/b` and `a%2Fb` collide: within a folder,
  a stem already used (compared case-insensitively) gets `~2`, `~3`, ... Desktop instead appends
  both tables to one file. Readers take names from file contents, never from file names.

The result never contains a path separator and, with `.tmdl` appended, is never `.` or `..`.

## Folder writing

`write_folder` refuses names TMDL cannot hold (see Known gaps), validates every produced path
(one or two plain components ending in `.tmdl`), creates `definition/` and its sub-folders
(refusing to write through a symbolic link), writes each file to a temporary
`.<name>.bicraft-tmp` and renames it into place (identical files are left untouched), then
deletes the `.tmdl` files below `definition/` (same walk as the reader) that it did not produce.
A listed file whose name differs from a produced one only in case is compared by canonical path,
so a case-only rename on a case-insensitive file system is not deleted. Other files are never
touched. Files are written, and read by `read_folder` (one open each, sizes from the folder
listing), on up to 16 threads: on Windows each open costs far more than the bytes
(benchmarks.md, G23).

## Known gaps

- `dataType` names outside the TOM set map to `Unknown` (and stay in `extra`).
- Names containing a control character (tab, line break, ...) have no TMDL form Desktop reads.
  `write_folder` refuses the model with `Error::UnwritableName` before writing anything;
  `unwritable_name(model)` reports the first one; `write_documents` still writes them (bicraft
  reads them back).
- Object kinds TOM only allows at compatibility level 1000000 (`excludedArtifact`,
  `analyticsAIMetadata`) were not observed and are unknown keys; unknown nested objects are
  written as property blocks and unknown property names are kept, but Desktop rejects keys its
  schema does not know. Every kind and property TOM 17.0.87 writes at level 1706 is known.
- Keys that are not TMDL keywords (letters, digits and `_` only), which JSON values can carry:
  a nested object or collection holding one is written as a single `key: <compact JSON>` line and
  reads back as that text, not as an object. An object's own such key (TMSL only) is not written.
- TMSL arrays of expression lines in `extra` are written as joined text and read back as a
  string, and empty arrays are not written: TOM's own TMSL reader joins and drops them the same
  way.
- Space-indented documents: the level width is the smallest indentation of an entry line;
  expression bodies are skipped, and only used (body indentation over the levels below their
  entry) when no entry line is indented.
