# `metadata.sqlitedb` → semantic model

| | |
|---|---|
| Used for | Reading the tabular model out of a PBIX `DataModel` (the ABF backup holds `metadata.sqlitedb`; see [datamodel-abf.md](datamodel-abf.md)) |
| Crate | `bicraft-metadata` (layer L3): `build_model(&MetadataTables)`, `read_sqlitedb(&[u8])`, `tables_from_sqlitedb(&[u8])`; writing (see below): `apply_model`, `apply_model_at`, `tables_to_rows`, `write_sqlitedb`; structural writing: `tables_from_sqlitedb_all`, `apply_structural`, `write_structural_sqlitedb`, `dangling_references`, `storage_file_paths` |
| Sources | [MS-SSAS-T] §2.2.5 "Common Data Structures" (one page per object: [Model][model], [Table][table], [Column][column], [Partition][partition], [Relationship][relationship], [Measure][measure], [Hierarchy][hierarchy], [Annotation][annotation], [Role][role], [RoleMembership][rolemembership], [TablePermission][tablepermission], [ExtendedProperty][extprop], [Expression][expression], [LinguisticMetadata][linguistic], [PerspectiveTable][perspectivetable], [CalculationItem][calcitem]); the public TOM enum reference for codes the spec leaves out ([ModeType][modetype], [PartitionSourceType][pst], [ValueFilterBehaviorType][vfb]) |
| Observed on | One `metadata.sqlitedb` from a Power BI Desktop 2.158 PBIX (local test data, not committed), read with Python's `sqlite3` as an oracle, and its PBIP/TMDL twin used as the reference model; for the nested objects (G16), the engine of Desktop 2.158.1304.0 (Analysis Services 17.0.87.18), a local engine instance, run on our own synthetic models (see "Engine oracle") |
| Date | 2026-10-14 |

Spec text is summarized, not copied. The engine oracle drives the engine through the public
TOM/AMO API, and enumeration codes the spec leaves out were read from TOM's public enum types and
then confirmed on the engine.

## Physical layout (observed)

- One SQLite table per TMSCHEMA object type, named after the object: `Model`, `Table`, `Column`,
  `Measure`, `Partition`, `Relationship`, `Expression`, `Annotation`, `Hierarchy`, `Level`,
  `Culture`, `Role`, `TablePermission`, … plus storage-only tables (`*Storage`, `StorageFile`,
  `StorageFolder`, `SegmentMapStorage`, …) and a `DBPROPERTIES` name/value table. The observed
  file has 69 tables and one index.
- DDL style: bracket-quoted identifiers, columns declared `INTEGER`, `TEXT` or `REAL`, and
  `PRIMARY KEY("ID" ASC)` as a table constraint, so `ID` is the rowid.
- Column names are the [MS-SSAS-T] property names (`ExplicitName`, `TableID`, `IsHidden`, …).
  A few tables carry columns newer than the published spec (e.g. `Column.ExpressionContext`,
  `Table.DirectLakeIndexingBehavior`, `Model.MetadataAccessPolicy`).
- Booleans are `0`/`1` integers; enumerations are integers; `ModifiedTime` and friends are
  integers that look like Windows FILETIME ticks (100 ns since 1601); `Partition.RangeStart/End`
  are reals; absent strings are `NULL`.
- References are numeric IDs; `0` means "none" (`SortByColumnID = 0`, `KPIID = 0`, …).
- IDs come from one counter shared by every object type (`DBPROPERTIES.MAXID`), so they reflect
  creation order. Sorting tables, columns, measures, partitions, relationships and annotations by
  `ID` reproduced the TMDL order of the reference project exactly.
- **System tables.** Besides user tables, `Table` holds engine-internal tables (attribute
  hierarchy `H$…`, relationship `R$…`, `U$…`): `SystemFlags` bit 0 set, `ModelID = 0`, with their
  own columns and partitions (`Type = 3`, none). User calculated tables have bit 1 set.

## Object mapping

General rules:

- Object names: `Name`, except columns: `ExplicitName`, falling back to `InferredName`
  (calculated-table columns whose name is inferred), then `Name`.
- Every collection is sorted by `ID`; hierarchy levels by `Ordinal`, then `ID`.
- Codes become the TMSL spellings TOM writes, and **a property at its TOM default is left out**,
  the way TOM serializes TMSL/TMDL. This makes the output equal to what `bicraft-tmsl` reads from
  the same model's `model.bim` (no `"isHidden": false`, no `"summarizeBy": "default"`, …).
- Properties without a typed field go into the object's `extra` under their TMSL name, in a fixed
  order. Child collections the model does not type become JSON arrays in `extra`.
- Annotations, extended properties and changed properties are child rows keyed by
  (`ObjectType`, `ObjectID`) and attach to the matching object (codes below).
- The database name and compatibility level are **not** in `metadata.sqlitedb`; `name` stays
  empty and `compatibility_level` `None` (see open questions).

| Source table | Model target | Notes |
|---|---|---|
| `Model` (lowest `ID`) | `SemanticModel` | `Culture` → `culture`, `Description` → `description`; `Name` → `extra.name`. Missing → `Error::NoModel`. |
| `Table` (`ModelID` = model, `SystemFlags` bit 0 clear) | `tables` | `Name`, `Description`, `LineageTag`, `IsHidden`. |
| `Column` (by `TableID`) | `Table.columns` | See column rules below. |
| `Measure` (by `TableID`) | `Table.measures` | `Name`, `Expression`, `FormatString`, `DisplayFolder`, `Description`, `LineageTag`, `IsHidden`. |
| `Partition` (by `TableID`) | `Table.partitions` | `Type` → `source_type`, `QueryDefinition` → `expression`, `Mode` → `mode`. |
| `Relationship` (`ModelID` = model) | `relationships` | Ends resolved through `FromColumnID`/`ToColumnID` (table from `From/ToTableID`, else the column's table). Skipped if an end does not resolve to a user table. |
| `Expression` | `expressions` | `Kind` → `kind`, `Expression`, `Description`, `LineageTag`. |
| `Role` + `TablePermission` + `ColumnPermission` + `RoleMembership` | `roles` | `ModelPermission` → `model_permission`; permissions resolve `TableID`/`ColumnID` to names (dangling ones skipped). |
| `Annotation` | `annotations` of model/table/column/measure/relationship/expression/role; JSON `annotations` in `extra` of every other object that TOM gives annotations | See "Nested objects". |
| Hierarchies, levels, variations, calculation groups and items, KPIs, detail rows and format string definitions, refresh policies, calendars, related column details, alternateOf, data coverage definitions, role members, column permissions, cultures, translations, linguistic metadata, perspectives, query groups, functions, data sources | JSON in `extra` of their owner | Declarative mapping shared by reading and writing: see "Nested objects". |
| `ExtendedProperty` | `extendedProperties` in `extra` of any object | `{type: "json", name, value}` (value parsed) or `{name, value}` for strings. |
| `ChangedProperty` | `changedProperties` in `extra` of any object | `[{property}]` |

### Columns

| Source | Target |
|---|---|
| `Type` | `1` data → no `type`; `2` calculated → implied by `expression` (`extra.type = "calculated"` only if the expression is missing); `3` → `extra.type = "rowNumber"`; `4` → `extra.type = "calculatedTableColumn"` |
| `ExplicitDataType`, else `InferredDataType` when explicit is `1` (automatic) | `data_type` |
| `ExplicitName` missing on a calculated-table column | `extra.isNameInferred = true` |
| `ExplicitDataType = 1` on a calculated / calculated-table column | `extra.isDataTypeInferred = true` |
| `SourceColumn`, `Expression`, `FormatString`, `DisplayFolder`, `DataCategory`, `Description`, `LineageTag`, `IsHidden` | typed fields |
| `SummarizeBy` (≠ default) | `summarize_by` |
| `SortByColumnID` | `sort_by_column` (column name) |
| `IsUnique`, `IsKey`, `IsDefaultLabel`, `IsDefaultImage`, `KeepUniqueRows` (when true); `IsNullable`, `IsAvailableInMDX` (when false); `Alignment`, `EncodingHint` (≠ default); `TableDetailPosition` (≠ −1); `DisplayOrdinal` (≠ 0); `SourceProviderType`, `SourceLineageTag` | `extra` under the camelCase TMSL name |

A row-number column thus reads as `{type: rowNumber, isUnique: true, isKey: true, isNullable:
false}` with `data_type = int64` and `is_hidden`, as in TMSL.

### Model properties

`DefaultMode` (≠ import), `DefaultDataView` (≠ full), `Collation`, `StorageLocation`,
`DataAccessOptions` (JSON text parsed into an object), `DefaultMeasureID` (→ `{table, measure}`,
TOM's shape; checked on the engine), `ForceUniqueNames` / `DiscourageImplicitMeasures` /
`DiscourageReportMeasures` / `DiscourageCompositeModels` / `DisableSystemDefaultExpression` (when
true), `DefaultPowerBIDataSourceVersion` (≠ V1), `SourceQueryCulture`,
`DataSourceVariablesOverrideBehavior` (≠ disallow; `allow` is 1), `DataSourceDefaultMaxConnections`
(≠ 10), `MaxParallelismPerRefresh` (≠ −1), `MaxParallelismPerQuery` (≠ 0), `DisableAutoExists`
(≠ −1), `DirectLakeBehavior` (≠ automatic), `ValueFilterBehavior` (≠ automatic),
`SelectionExpressionBehavior` (≠ automatic), `MetadataAccessPolicy` (≠ inherited),
`DefaultDirectLakeIndexingBehavior` (≠ auto), `MAttributes` (text), `AutomaticAggregationOptions`
(JSON). The defaults −1, 10, 0 were inferred from the reference project, whose TMDL omits these
values; the engine confirmed every one of them (it scripts a model that sets them all, see
"Engine oracle"). `DiscourageReportMeasures` and `DisableSystemDefaultExpression` are not TMSL
properties of this engine (deploying them is refused); they are kept because a file may hold
them.

### Partitions, relationships, roles, expressions

- Partition `extra`: `description`, `dataView` (≠ default), `retainDataTillForceCalculate`,
  `queryGroup`, `dataCoverageDefinition`, `annotations`. `Mode = 2` (default) is omitted.
- Partition `source` (checked on the engine for every type): `QueryDefinition` is `expression`
  for `m` and `calculated`, `query` for `query` (with `dataSource`, the data source's name),
  `entityName` for `entity` (with `expressionSource`, the shared expression's name, and
  `schemaName`), `location` for `parquet` (inferred from TOM, see "Direct Lake"). `policyRange` partitions have no definition: `RangeStart` / `RangeEnd` (OLE
  automation dates, `REAL` days since 1899-12-30) become `start` / `end`
  (`yyyy-MM-ddTHH:mm:ss`), `RangeGranularity` becomes `granularity` (−1: none), and
  `RefreshBookmark` becomes `refreshBookmark` (inside `source`, where TOM puts it).
  `calculationGroup` sources are empty.
- Relationship: `crossFilteringBehavior` (≠ oneDirection), `fromCardinality` (≠ many),
  `toCardinality` (≠ one), `IsActive`; `extra`: `joinOnDateBehavior` (≠ dateAndTime),
  `relyOnReferentialIntegrity`, `securityFilteringBehavior` (≠ oneDirection).
- Role `extra`: `description`, `members`. Table permission `extra`: `metadataPermission`
  (≠ default), `columnPermissions`, `annotations`.
- Expression `extra`: `sourceLineageTag`, `queryGroup`, `expressionSource`,
  `parameterValuesColumn: {name, table}`, `remoteParameterName`, `mAttributes`,
  `excludedArtifacts`.
- Table `extra` also holds `directLakeIndexingBehavior` (≠ default, code 3: the value the engine
  stores on every table, which TOM leaves out).

## Enumeration codes

From [MS-SSAS-T] unless marked *TOM* (public TOM enum reference) or *obs.* (observed against the
reference project). The default is in bold where the mapping omits it.

| Property | Codes |
|---|---|
| Column `ExplicitDataType` / `InferredDataType`, Measure `DataType` | 1 automatic, 2 string, 6 int64, 8 double, 9 dateTime, 10 decimal, 11 boolean, 17 binary, 19 unknown (error state), 20 variant (measures) |
| Column `Type` | 1 data, 2 calculated, 3 rowNumber, 4 calculatedTableColumn |
| Column `SummarizeBy` | **1 default**, 2 none, 3 sum, 4 min, 5 max, 6 count, 7 average, 8 distinctCount |
| Column `Alignment` | **1 default**, 2 left, 3 right, 4 center |
| Column `EncodingHint` | **0 default**, 1 hash, 2 value |
| Column / Partition / Table `SystemFlags` | bit 0 system object, bit 1 calculated table (column of / partition of) |
| Partition `Type` | 1 query, 2 calculated, 3 none, 4 m, 5 entity, 6 policyRange (*TOM*), 7 calculationGroup, 8 inferred (*TOM*), 9 parquet (*TOM*; unobserved) |
| Partition `Mode`, Model `DefaultMode` | 0 import (**model default**), 1 directQuery, **2 default** (partition default), 3 push, 4 dual (*TOM*), 5 directLake (*TOM*) |
| Partition `DataView`, Model `DefaultDataView` | 0 full (**model default**), 1 sample, **3 default** (partition default) |
| Relationship `CrossFilteringBehavior` | **1 oneDirection**, 2 bothDirections, 3 automatic |
| Relationship `SecurityFilteringBehavior` | **1 oneDirection**, 2 bothDirections, 3 none |
| Relationship `From/ToCardinality` | 1 one (**to** default), 2 many (**from** default) |
| Relationship `JoinOnDateBehavior` | **1 dateAndTime**, 2 datePartOnly |
| Role `ModelPermission` | 1 none, 2 read, 3 readRefresh, 4 refresh, 5 administrator |
| TablePermission / ColumnPermission `MetadataPermission` | **0 default**, 1 none, 2 read |
| RoleMembership `MemberType` | **1 auto**, 2 user, 3 group |
| Hierarchy `HideMembers` | **0 default**, 1 hideBlankMembers |
| Expression `Kind` | 0 m |
| Model `DefaultPowerBIDataSourceVersion` | **0 powerBI_V1**, 1 powerBI_V2, 2 powerBI_V3 |
| Model `ValueFilterBehavior` | **0 automatic**, 1 independent, 2 coalesced (*TOM*; 1 confirmed *obs.*) |
| Model `DirectLakeBehavior` | **0 automatic**, 1 directLakeOnly, 2 directQueryOnly |
| Model `DefaultDirectLakeIndexingBehavior`, Table `DirectLakeIndexingBehavior` | 0 auto (**model default**), 1 explicit, 2 full, 3 default (**table default**) (*TOM*; 1, 2, 3 confirmed) |
| Model `MetadataAccessPolicy` | **0 inherited**, 1 basic, 2 calculationDefinitions (*TOM* `MetadataCategory`; 2 confirmed) |
| BindingInfo `Type` | 0 unknown, 1 dataBindingHint (*TOM*; 1 confirmed; always written) |
| ExcludedArtifact `ArtifactType` | an `ObjectType` code, kept as a number as TOM writes it; the engine accepts 7 under the model, 4, 8, 9 under a table, 3 under an expression |
| Model `SelectionExpressionBehavior` | **0 automatic**, 1 visual, 2 nonVisual (*TOM*; 1 confirmed on the engine) |
| Model `DataSourceVariablesOverrideBehavior` | **0 disallow**, 1 allow (*TOM*; confirmed) |
| ExtendedProperty `Type` | 0 string, 1 json |
| LinguisticMetadata `ContentType` | **0 xml**, 1 json |
| ObjectTranslation `Property` | 1 caption, 2 description, 3 displayFolder (*TOM* `TranslatedProperty`; confirmed) |
| CalculationExpression `SelectionMode` | 1 multipleOrEmptySelection, 2 noSelection (confirmed) |
| CalendarColumnGroup `TimeUnit` | **0 unknown** (a group of plain time-related `columns`), 1 year, 2 semester, 3 semesterOfYear, 4 quarter, 5 quarterOfYear, 6 quarterOfSemester, 7 month, 8 monthOfYear, 9 monthOfSemester, 10 monthOfQuarter, 11 week, 12 weekOfYear, 13 weekOfSemester, 14 weekOfQuarter, 15 weekOfMonth, 16 date, 17 dayOfYear, 18 dayOfSemester, 19 dayOfQuarter, 20 dayOfMonth, 21 dayOfWeek (*TOM*; 0, 1, 7, 8, 16 confirmed) |
| RefreshPolicy `PolicyType` / `Mode` | 0 basic / **0 import**, 1 hybrid (*TOM*; confirmed) |
| RefreshPolicy granularities, Partition `RangeGranularity` | −1 none, 0 day, 1 month, 2 quarter, 3 year (*TOM*; confirmed) |
| AlternateOf `Summarization` | **0 groupBy**, 1 sum, 2 count, 3 min, 4 max (*TOM*; 0, 1, 2 confirmed) |
| DataSource `Type` / `ImpersonationMode` / `Isolation` | **1 provider**, 2 structured / **1 default**, 2 impersonateAccount, 3 impersonateAnonymous, 4 impersonateCurrentUser, 5 impersonateServiceAccount, 6 impersonateUnattendedAccount / **1 readCommitted**, 2 snapshot (*TOM*; 1, 2, 5 confirmed) |
| `ObjectType` (Annotation, ExtendedProperty, ChangedProperty, ObjectTranslation, FormatStringDefinition, DetailRowsDefinition) | 1 Model, 2 DataSource, 3 Table, 4 Column, 5 AttributeHierarchy, 6 Partition, 7 Relationship, 8 Measure, 9 Hierarchy, 10 Level, 12 KPI, 13 Culture, 15 LinguisticMetadata, 29 Perspective, 30 PerspectiveTable, 31 PerspectiveColumn, 32 PerspectiveHierarchy, 33 PerspectiveMeasure, 34 Role, 35 RoleMembership, 36 TablePermission, 37 Variation, 38 Set, 41 Expression, 42 ColumnPermission, 43 DetailRowsDefinition, 44 RelatedColumnDetails, 45 GroupByColumn, 46 CalculationGroup, 47 CalculationItem, 48 AlternateOf, 49 RefreshPolicy, 50 FormatStringDefinition, 51 QueryGroup, 52 AnalyticsAIMetadata, 58 DataCoverageDefinition, 59 CalculationExpression, 60 Calendar, 61 CalendarColumnGroup, 62 CalendarColumnReference, 63 Function, 64 BindingInfo (*TOM* `ObjectType`; every code used by the mapping confirmed) |

Unknown codes are dropped (typed field `None`/empty, no `extra` entry), never guessed.

## Not mapped

| Item | Why |
|---|---|
| `State`, `ErrorMessage`, `ModifiedTime`, `StructureModifiedTime`, `RefreshedTime` (all objects) | Engine runtime state, not part of TMSL. The engine crate reads the partition, table and model values read-only (see "Refresh state"). |
| `*Storage`, `StorageFile`, `StorageFolder`, `SegmentMapStorage`, `AttributeHierarchy`, `DBPROPERTIES`, … | VertiPaq/ABF storage bookkeeping. Column `attributeHierarchy` is therefore not produced. |
| System tables (`SystemFlags` bit 0) and their columns/partitions | Engine-internal. |
| `DataSource.ConnectionString`, `Password`, `Credential` | Encrypted with the engine instance's key; kept opaque and byte for byte (see "Encrypted data-source secrets"). |
| `Partition.MAttributes` | Not a TOM property (TOM's `Partition` class has none; refused on the partition and on its source). |
| `DeltaTableMetadataStorage`, `ParquetFileStorage`, `DeltaTableColumnStorage`, `PartitionStorage.DeltaTableMetadataStorageID` | Direct Lake framing state (storage bookkeeping, like the VertiPaq storage tables), not TMSL; preserved untouched by every metadata write (see "Direct Lake"). |
| `Measure.DataType`, `Column.InferredDataType` beyond the fallback, `Column.ExpressionContext`, `StringIndexingBehavior`, `FullTextIndexingBehavior`, `Relationship.Type` (always single column) | Engine values: TOM scripts `measure.dataType` (read-only, computed from the DAX), the others are not TMSL properties of this engine (refused as unrecognized). |
| Translations of object types other than model, table, column, measure, hierarchy, level, perspective, role and expression | None observed; the engine silently drops a role translation, so role and expression translations are mapped but unconfirmed. |

## Verification against the reference project

`read_sqlitedb` on the observed file, compared with `bicraft_tmdl::read_folder` on the PBIP's
`definition/` folder (scratch program outside the repo; partitions compared with a small ad-hoc
TMDL scan because the TMDL reader skips them):

- Tables: same set, same order (all 8); system tables excluded (43 in the file).
- Columns: all 44 user columns match on name, data type, hidden flag and order; the 13
  calculated-column expressions are identical. Row-number columns (one per table) are extra on our
  side, as in TMSL.
- Measures: all 25 match on name, expression, format string, hidden flag and order.
- Relationships: all 4 match on endpoints, active flag and order.
- Partitions: all 8 match on name, source type, mode and source text.
- Model: culture, `dataAccessOptions`, `defaultPowerBIDataSourceVersion`, `sourceQueryCulture`,
  `valueFilterBehavior`, the 4 model annotations (same order) and the culture list match the
  TMDL `model` object. Property counts (format strings, lineage tags, descriptions, sort-by
  columns, summarize-by, column annotations, data categories, hierarchy levels) match the TMDL text.
- One divergence: six hidden calculated-table columns have `IsAvailableInMDX = 0` in the database
  but the TMDL does not show `isAvailableInMdx: false`. We keep the database value (see below).

## Nested objects (G16)

Every object the model keeps as JSON in `extra` is described once, declaratively, in
`crates/metadata/src/spec.rs`: its table, how its rows hang off their owner, which column carries
which TMSL property with which conversion, and its child collections. `src/nested.rs` reads
(rows → JSON) and writes (JSON difference → row edits) from that description, so the two
directions share one mapping. Shapes and codes below were checked on the engine (see "Engine
oracle"); TMSL keys are TOM's.

### Rules

- **Links.** A child row points at its owner (`Level.HierarchyID`), or the owner points at the
  child (`Measure.KPIID` → `KPI`, which points back through `KPI.MeasureID`; detail rows and format
  string definitions point back through `ObjectID` / `ObjectType`). `CalculationExpression` rows
  hang off their calculation group and are told apart by `SelectionMode`; calendar column
  references by `IsPrimaryColumn`.
- **Order.** Rows in `ID` order; levels by `Ordinal`, then `ID`.
- **Values.** Text is left out when NULL or empty; expressions are always present (`""` when
  NULL); flags, integers and codes are left out at their default (some, e.g. `ordinal`,
  `policyType` or the refresh granularities, are always present, as TOM writes them); JSON text is
  parsed (kept as a string when it does not parse); references become names: tables, columns,
  measures and hierarchies of the *scope* table (the owner's table, or the perspective table's
  table), `{table, column}` / `{table, hierarchy}` pairs, relationship names.
- **Tail.** `annotations` on every object TOM gives annotations (not on calculation items,
  calculation expressions, detail rows, format string definitions, calendars, refresh policies,
  related column details, alternateOf: the engine refuses them there),
  `changedProperties` and `extendedProperties` on every object with an `ObjectType`. Extended
  properties of type JSON must hold an object (TOM refuses anything else).
- **Writing.** Items of a collection are matched by their key (`name`, `folder`, `memberName`,
  `column`, …, ASCII case-insensitive, n-th occurrence to n-th occurrence) or by position when
  the kind has none; a renamed item is therefore removed and added again. Matched items get the
  changed columns only; unmatched old items are removed with their whole subtree and their
  dependent rows (annotations, extended and changed properties, translations, excluded
  artifacts); new items get the next IDs, the link columns, their properties (defaults for absent
  ones), the bookkeeping values below and `ModifiedTime` (and `StructureModifiedTime` where the
  table has it) = the edit time. A changed expression also resets `State` to 1 and `ErrorMessage`.
  A new pointer child sets the owner's pointer column; a removed one resets it to 0.
- **Refused** (`UnsupportedChange`, nothing written): a property the kind does not map, a value
  of the wrong type, an unknown code, a reference to an object the model does not have; adding or
  removing hierarchies, levels and calculation items (their content is stored data: `U$` tables,
  the calculation group table's rows), adding a calculation group, changing names, ordinals or
  columns of hierarchies, levels and calculation items; adding a data source (it would need an
  encrypted connection string or credential); removing a query group or data source that rows
  still use; changes under partitions (partitions are written by the structural writer only).

### Objects

"Read" and "write" say what the engine confirmed: *read* = BIcraft's reading of the
engine-written file equals the model the engine scripts; *write* = the engine loaded a file
BIcraft wrote with that edit and scripts exactly what BIcraft reads back from it.

| TMSL (owner → key) | Rows | Read | Write (metadata-only) |
|---|---|---|---|
| model → `cultures[]` `{name, linguisticMetadata, translations, annotations}` | `Culture` | engine | add, change, remove: engine |
| culture → `linguisticMetadata` `{content, contentType}` | `LinguisticMetadata` (pointer `Culture.LinguisticMetadataID`) | engine | add, change content, remove, annotations: engine. JSON content written indented by two spaces, as the engine stores it |
| culture → `translations` `{model: {name, translated…, tables: [{columns, measures, hierarchies: [{levels}]}], perspectives, roles, expressions}}` | `ObjectTranslation` (`ObjectType`, `ObjectID`, `Property`) | engine (model, table, column, measure, hierarchy, level, perspective) | add, change, remove: engine. New rows `Altered` 0 |
| model → `perspectives[]` → `tables[]` (`name`, `includeAll`) → `columns[]`, `hierarchies[]`, `measures[]` | `Perspective`, `PerspectiveTable`, `PerspectiveColumn/Hierarchy/Measure` | engine | add, change, remove at every level: engine |
| model → `queryGroups[]` `{folder, description}` | `QueryGroup` | engine | add, change: engine; remove unless an expression or partition uses it (unconfirmed) |
| model → `functions[]` `{name, description, expression, isHidden, lineageTag, sourceLineageTag}` | `Function` | engine | add, change: engine; remove (unconfirmed) |
| model → `dataSources[]` `{type, name, description, impersonationMode, account, maxConnections, isolation, timeout, provider, connectionDetails, options, contextExpression}` | `DataSource` | engine (secrets excepted, see "Not mapped") | change: engine; remove unless a partition uses it (unconfirmed); add refused |
| table → `defaultDetailRowsDefinition`, measure → `detailRowsDefinition` `{expression}` | `DetailRowsDefinition` (pointer, back `ObjectID/ObjectType`) | engine | add, change, remove: engine |
| measure → `formatStringDefinition`, calculation item → `formatStringDefinition`, calculation expression → `formatStringDefinition` | `FormatStringDefinition` (`ObjectType` 8, 47, 59) | engine | add, change, remove: engine |
| measure → `kpi` `{description, target…, status…, trend…}` | `KPI` (pointer `Measure.KPIID`, back `MeasureID`) | engine | add (also on a new measure), change, remove (also with its measure): engine |
| table → `calculationGroup` `{description, precedence, multipleOrEmptySelectionExpression, noSelectionExpression, calculationItems[]}` | `CalculationGroup`, `CalculationItem`, `CalculationExpression` | engine | description, precedence, item description / expression / format string, selection expressions (change, remove): engine; adding a selection expression (unconfirmed) |
| table → `refreshPolicy` `{policyType, rollingWindowGranularity, rollingWindowPeriods, incrementalGranularity, incrementalPeriods, incrementalPeriodsOffset, pollingExpression, sourceExpression, mode}` | `RefreshPolicy` (pointer `Table.RefreshPolicyID`) | engine | change: engine; add, remove (unconfirmed) |
| table → `hierarchies[]` → `levels[]` | `Hierarchy`, `Level` | engine | description, display folder, level description, annotations: engine; hidden, hideMembers, lineage tags (unconfirmed); structure refused |
| table → `calendars[]` → `calendarColumnGroups[]` `{timeUnit, primaryColumn, associatedColumns}` or `{columns}` | `Calendar`, `CalendarColumnGroup`, `CalendarColumnReference` | engine | add, change, remove at every level: engine. The engine checks that a column is used once per calendar and with one time unit across calendars; BIcraft does not |
| column → `variations[]` `{name, description, relationship, defaultHierarchy, defaultColumn, isDefault}` | `Variation` | engine (also the auto date/time model of the DAX oracle; DAX resolves `'T'[Date].[Year]` through it, dax-functions.md) | change, rename (= remove + add): engine |
| column → `relatedColumnDetails` `{groupByColumns: [{groupingColumn}]}` (field parameters) | `RelatedColumnDetails`, `GroupByColumn` | engine | group-by column added, details removed: engine |
| column → `alternateOf` `{baseColumn: {table, column}, baseTable, summarization}` | `AlternateOf` | engine | change, remove: engine; add (unconfirmed) |
| column → `extendedProperties` (e.g. field parameters' `ParameterMetadata`) | `ExtendedProperty` | engine | add, change, remove: engine |
| partition → `dataCoverageDefinition` `{description, expression}` | `DataCoverageDefinition` | engine | refused (partitions) |
| role → `members[]` `{memberName, memberId, identityProvider, memberType}` | `RoleMembership` | engine (Windows and Azure AD members) | add, change, remove: engine with Windows members (by SID); an engine without a directory cannot load Azure AD members, so their writing is unconfirmed |
| table permission → `columnPermissions[]` `{name, metadataPermission}` | `ColumnPermission` | engine | add, remove, annotations: engine; change (unconfirmed) |
| table → `sets[]` `{name, description, expression, isDynamic, isHidden, displayFolder}` | `Set` (`TableID`) | engine (sets enabled) | add, change, remove (also with the perspective entry that names it): engine. No stored data. A set a perspective still names cannot be removed |
| perspective table → `sets[]` `{name}` | `PerspectiveSet` (`SetID`) | engine | add, remove: engine. Only sets the base already has can be named (not a set added by the same edit) |
| model → `bindingInfoCollection[]` `{type, name, description, connectionId}` | `BindingInfo` | engine | add, change, remove, annotations: engine; `type` fixed |
| model → `analyticsAIMetadata[]` `{name, measureAnalysisDefinition}` | `AnalyticsAIMetadata` | engine (level 1000000) | add, change, remove: engine. JSON written indented by two spaces, as the engine stores it |
| model, table, expression → `excludedArtifacts[]` `{artifactType, reference}` | `ExcludedArtifact` (`ObjectID`, `ObjectType` 1, 3, 41) | engine (level 1000000) | add, change, remove: engine; matched by position |
| every object → `annotations`, `extendedProperties`, `changedProperties` | `Annotation`, `ExtendedProperty`, `ChangedProperty` | engine | add, change, remove: engine |

New rows: `State` 1 on rows with an expression (detail rows, format strings, calculation items and
expressions, data coverage, functions), `Altered` 0 on translations, `IsPrimaryColumn` and
`SelectionMode` from the collection, pointer columns of children not given 0.

**Translations in a culture (M10.4).** BIcraft's `culture.add` / `translation.set` edits are
written as `Culture` and `ObjectTranslation` rows by both PBIX paths: a DataModel built from
scratch (`file.save_as` of a new model) and a metadata-only save of an opened PBIX (a culture
added, a caption changed). Checked 2026-10-14 against the local engine instance of Desktop 2.158.1304.0
(Analysis Services 17.0.87.18): each written image was loaded with `Server.ImageLoad` and the
`MDSCHEMA_DIMENSIONS`, `MDSCHEMA_MEASURES`, `MDSCHEMA_HIERARCHIES` and `MDSCHEMA_LEVELS` rowsets
were read with `LocaleIdentifier` 1033, 1036 and 1031 on the connection. Each session saw its
culture's translated table caption and description, column caption, measure caption, hierarchy
caption, level caption and column display folder (as the attribute hierarchy's display
folder). Objects without a translation, and every object in a locale the model has no culture
for (1031 on a model without de-DE), showed their names. Fallback between cultures of one
language (`fr-CA` → `fr-FR`) was not tested; BIcraft's reads match the culture name exactly.

## Engine oracle

The mapping is checked during development against Microsoft's Analysis Services engine (the
maintainers' development oracles, kept outside this repository; `BICRAFT_ORACLES_DIR` points to
them). The check works on our own synthetic models only:

1. Each case, a synthetic TMSL model of ours, is deployed and computed.
2. The engine saves the database as the XPress9 `DataModel` stream a PBIX holds (public AMO
   API); `bicraft-xpress9` decompresses it, `bicraft-abf` opens the ABF and BIcraft maps its
   `metadata.sqlitedb`.
3. TOM scripts the database; the scripted model, read by `bicraft-tmsl`, must equal BIcraft's
   reading of the image.
4. Each edit (`cargo xtask metadata-edit`, xtask/src/oracles/metadata_edits.rs) changes BIcraft's TMSL of the model;
   `write_sqlitedb_at` writes it (fixed clock), `AbfWriter` re-packs the ABF and
   `compress_datamodel` re-compresses it; the engine loads the result and scripts it again; that
   model must equal BIcraft's reading of the written file.

Normalizations applied before comparing, all because TOM and BIcraft write the same fact
differently: runtime state (`state`, `errorMessage`, `attributeHierarchy`), timestamps and the
engine-computed `measure.dataType` are dropped from the engine's side; TOM writes
`isNameInferred: false` on every calculated-table column where BIcraft (and TMDL) leave the
default out; TOM leaves the default model name `Model` out; encrypted data-source secrets are not
compared.

Results (Desktop 2.158.1304.0, Analysis Services 17.0.87.18, 2026-10-14), all with no difference:

| Case | Model | Read | Edits written and loaded |
|---|---|---|---|
| `objects` | cultures, translations, linguistic metadata, perspectives, calculation group with selection expressions, KPI, detail rows, format strings, hierarchies, field parameter, roles with members and column permissions, calendars, query group, annotations and extended properties everywhere TOM allows them | equal | `add_and_change` (every kind changed or added), `remove` (every removable kind removed, a measure with its KPI), `calendars` |
| `sources` | legacy and structured data sources, query, M and policy-range partitions, refresh policy, function, changed properties | equal | `sources` |
| `properties` | every model property set off its default, column properties, aggregation table (`alternateOf`), entity partition, hybrid table with data coverage | equal | `aggregations` |
| `variations` | the DAX oracle model (auto date/time variation, local date table) | equal | `variation_props`, `variation_rename` |
| `preview` | compatibility level 1000000: sets and perspective sets, binding info, analytics AI metadata, excluded artifacts under the model, a table and an expression, metadata access policy, Direct Lake indexing behaviors, a DirectQuery entity partition over a shared expression | equal | `preview_change` (every new kind changed and added), `preview_remove` (every new kind removed, a set with its perspective entry) |

The `preview` case uses sets, so it was recorded on a development engine configured to accept
them. The `sources` case
also checks that the engine, after loading the file BIcraft rewrote, still decrypts the data
source secrets to the values of the deployed model (2026-10-14).

The engine-written `metadata.sqlitedb` of each case, BIcraft's model of it and the request /
result of each edit are committed in `crates/metadata/tests/engine` (provenance in `SOURCE.txt`
there); `tests/engine_fixtures.rs` replays both directions on any platform. By-product for G17:
the engine loads `DataModel` streams that BIcraft re-packed and re-compressed (single-threaded
XPress9 framing, small models).

Limits of the setup: TMSL `backup` / `restore` and `ImageLoad` of a model with Azure AD role
members fail in this local engine instance ("an unexpected exception occurred"), so the oracle uses
images and Windows members (by well-known SID; the engine resolves their names in the machine's
language); Direct Lake partitions are refused by Desktop's engine (see "Direct Lake"), so
`directLake` mode and filled `DeltaTable*` rows remain unobserved.

## Compatibility level 1000000 (G16)

What the engine of Desktop 2.158 (17.0.87.18) accepts, probed with synthetic TMSL on the
local engine instance (2026-10-14); the lowest levels are the ones the engine's TOM enforces:

| TMSL | Lowest level | Engine |
|---|---|---|
| `bindingInfoCollection` (model) | 1702 | accepted (the collection is not named `bindingInfos`) |
| `sets` (table, perspective table) | 1702 | refused ("Set feature is not enabled") unless sets are enabled in the configuration |
| `metadataAccessPolicy` (model) | 1703 | accepted |
| `analyticsAIMetadata`, `excludedArtifacts` (model, table, expression), `defaultDirectLakeIndexingBehavior` (model), `directLakeIndexingBehavior` (table) | 1000000 | accepted at level 1000000 (a database cannot be downgraded afterwards) |
| partition source `parquet` (`location`) | 2147483647 | not reachable: TOM refuses it at every real level |
| partition `mode: directLake` | — | refused at every level ("may not operate in Direct Lake mode ... not available in this product") |

The engine checks `excludedArtifacts.artifactType` against the owner (relationship under the
model; column, measure, hierarchy under a table; table under an expression) and stores
`reference` as JSON text indented by two spaces. The public TOM serializer 17.0.87.18
(`JsonSerializer`) knows `bindingInfoCollection` and drops `sets`, but refuses the level-1000000
names above as unknown JSON. BIcraft reads, writes and
round-trips them through TMSL and through its own TMDL (`excludedArtifact` blocks, named
`analyticsAIMetadata` objects; no TOM reference exists for that spelling yet).

## Direct Lake

Direct Lake is a partition mode, not a separate object (published TOM: `ModeType.DirectLake`,
`EntityPartitionSource`): a table has exactly one partition, `mode: directLake`, with an `entity`
source naming the Delta table (`entityName`, `schemaName`) and the shared expression that
locates the lakehouse or warehouse (`expressionSource`, an `m` expression such as
`Sql.Database(...)`). What Desktop's engine accepts and produces:

- `mode: directLake` is refused in every form (entity source, model `defaultMode`, every
  compatibility level); an `m` source in Direct Lake mode is refused because a Direct Lake table
  must have exactly one `entity` partition; an `entity` source in import mode is refused (entity
  partitions must be DirectQuery or Direct Lake).
- The same partition in `directQuery` mode is accepted, and its row is the Direct Lake row
  minus the mode: `Type` 5, `QueryDefinition` = entity name, `ExpressionSourceID` → the shared
  expression, `SchemaName`, `DataSourceID` 0, `Mode` 1. Direct Lake is that row with `Mode` 5,
  read as `mode: directLake` with the same `source`.
- Model `directLakeBehavior` (`directLakeOnly`, `directQueryOnly`) and the indexing behaviors
  are accepted and mapped (codes above).
- The framing tables exist in every file, empty here: `DeltaTableMetadataStorage`
  (`PartitionStorageID`, `DataSourceReference`, `TableName`, `RootLocation`, `CurrentVersion`,
  `TableObjectID`, `DatamartObjectID`, `FramedSchemaName`, `FallbackReason`,
  `DeltaColumnMappingMode`, `DeltaLogETag`, `ArtifactObjectId`, `ArtifactSecurityVersion`),
  `ParquetFileStorage` (`DeltaTableMetadataStorageID`, `Size`, `Location`, `CreatedVersion`,
  `DeletedVersion`, `RowgroupCount`, `Ordinal`, `ETag`, deletion vector location / offset / size
  / inline bitmap / ETag) and `DeltaTableColumnStorage` (`ColumnStorageID`,
  `UncompressedSize`, `MinCardinality`, `MaxCardinality`), linked from
  `PartitionStorage.DeltaTableMetadataStorageID`. They record the last framing of the Delta
  table, are not part of TMSL, and are kept untouched by metadata-only writes (the writer only
  rewrites tables it changes). Structural edits of Direct Lake tables are not supported.

`tests/engine_fixtures.rs` flips the engine-written DirectQuery entity partition of the
`preview` fixture to `Mode` 5 and checks the reading and that metadata edits leave the
partition rows unchanged. A Fabric-written file is still needed to observe filled framing rows.

## Encrypted data-source secrets

`DataSource.ConnectionString`, `Password` and `Credential` hold base64 text: an 8-byte
little-endian header (value 8, hence the `CAAAAAAAAA` prefix) followed by ciphertext under the
engine instance's key. For a structured data source the whole `credential` JSON is encrypted,
non-secret fields (`AuthenticationKind`, `kind`, `path`) included, which is why the model has no
`credential` at all. BIcraft never opens them:

- **Read.** The model leaves them out (TMSL and TMDL from a PBIX therefore lack
  `connectionString` / `credential`, which TOM would script decrypted).
  `bicraft_metadata::encrypted_secrets` lists them opaquely (data source, property, stored
  text) for inspection.
- **Preserve.** Every write starts from the base rows and sets only the columns that changed,
  so a kept data source keeps its secret text byte for byte, also when its other properties
  change. Removing a data source removes them with its row.
- **Rewrite.** Checked on the engine: after `ImageLoad` of a file BIcraft rewrote (data source
  description, timeout and connection limit changed), the engine decrypts the connection string
  and credential to the values originally deployed.
- **Add.** Refused, with evidence: the engine requires a credential for a structured data
  source (it reports the credential JSON as malformed) and a connection string for a provider
  data source (the M engine rejects an empty one), and both must be encrypted with a key BIcraft
  does not have.

## Writing: model → existing `metadata.sqlitedb`

`apply_model(base, model)` is the inverse of the mapping above for **metadata-only edits**: it
diffs `model` against `build_model(base)` and applies the difference to a copy of the base rows.
Every edit that would change stored data (column storage, the `H$`/`R$`/`U$` system tables,
`*Storage` rows) is refused here; added and removed tables, columns and relationships go
through `apply_structural` instead (see "Structural writing").
`apply_model_at(base, model, filetime)` is the same with an explicit clock (tests, reproducible
output).

### Scope

| Object | Written | Refused (`Error::UnsupportedChange`) |
|---|---|---|
| Model | `description`, annotations; the nested collections (cultures with translations and linguistic metadata, perspectives, query groups, functions, data sources) and extended / changed properties, see "Nested objects" | `culture`, scalar `extra` properties (data access options, default measure, …) |
| Table | `description`, `isHidden`, annotations; nested: default detail rows, calculation group (description, precedence, item expressions, selection expressions, format strings), refresh policy, hierarchies (display properties), calendars, extended / changed properties | added, removed, renamed; `lineageTag`, partitions, scalar `extra` |
| Column | `description`, `displayFolder`, `formatString`, `isHidden`, `summarizeBy`, `dataCategory`, `sortByColumn`, annotations; nested: variations, related column details, alternateOf, extended / changed properties | added, removed, renamed; `dataType`, `expression`, `sourceColumn`, `lineageTag`, scalar `extra` |
| Measure | added, removed (with its KPI, detail rows and format string definition), moved to another table, case-only rename; `expression`, `formatString`, `displayFolder`, `description`, `isHidden`, `lineageTag`, `extra.dataCategory` / `sourceLineageTag` / `isSimpleMeasure`, annotations; nested: `kpi`, `detailRowsDefinition`, `formatStringDefinition` (also on new measures), extended / changed properties | other `extra`; removing a measure that is still in a perspective or is the default measure |
| Relationship | — | any difference |
| Shared expression | `expression` (text only, no refresh semantics), `description`, annotations, extended / changed properties | added, removed, renamed; `kind`, `lineageTag`, other `extra` |
| Role | added, removed, case-only rename; `modelPermission`, `extra.description`, annotations; nested: `members`, extended / changed properties | other `extra` |
| Table permission | added, removed; `filterExpression`, `extra.metadataPermission`; nested: `columnPermissions`, annotations, extended / changed properties | other `extra`; a permission on a table the model does not have |

The database name, compatibility level and `database_extra` are not stored in this file and
are ignored. All refused differences are collected and returned together, each as
`UnsupportedChange { what, object }` (e.g. `column 'Sales'[Amount]: dataType changed`); nothing
is written if there is any.

### Rules

- **Matching.** Tables, roles and expressions by name; columns by name within their table;
  measures by name across the whole model (measure names are unique per model), so a measure
  found in another table is a move (`TableID` change, same `ID`). All ASCII case-insensitive.
  Annotations by name, n-th occurrence to n-th occurrence.
- **Untouched rows stay equal**, every column and the `ID`. `apply_model(base,
  build_model(base))` returns `base` exactly (checked on the synthetic fixture and the local
  real file). Only columns whose model value differs are written.
- **Collections come back in `ID` order** (creation order); new objects are appended, so a
  reordering in the model is not stored.
- **Removing an object** removes its rows that hang off it by (`ObjectType`, `ObjectID`):
  `Annotation`, `ExtendedProperty`, `ChangedProperty`, `ObjectTranslation`,
  `ExcludedArtifact`. A removed role also loses its `TablePermission`, `ColumnPermission` and
  `RoleMembership` rows; a removed table permission its `ColumnPermission` rows.
- **IDs are global.** Observed: the 1526 IDs across all tables of the real file are unique and
  their maximum equals `DBPROPERTIES.MAXID`. New rows take `max(MAXID, every loaded ID) + 1, +2,
  …`, and `DBPROPERTIES.MAXID` is raised to the last one. The base must have unique IDs (else
  `Error::Invalid`); running out of IDs is `Error::Invalid`.
- **Times.** `ModifiedTime`, `StructureModifiedTime` and `RefreshedTime` are Windows FILETIME
  ticks (100 ns since 1601-01-01 UTC); decoding the real file's values gives dates in the week
  the file was saved. A changed row gets `ModifiedTime = now`; a new row, or a structural
  change (measure expression, move or rename, column sort-by), also `StructureModifiedTime =
  now`. Only columns the row already has are bumped.
- **The `Model` row is not touched** unless its description changes. Observed: the model's
  `ModifiedTime` is older than its tables' and measures' times, so the engine does not keep it
  at the maximum of its children. `DBPROPERTIES` `LASTCOMMITTED`, `LASTCUBESCHEMACHANGE`, … are
  left as they are (assumption; see open questions).
- **New rows** take exactly the columns of the table's existing rows (the file's schema). A
  bookkeeping value BIcraft picks is dropped when the schema lacks its column; model content is
  kept (the SQLite adapter then reports the missing column). Columns BIcraft does not know get
  the integer value every existing row shares, else `NULL`. When the table is empty, the full
  column set below is used.

### Values of new rows

Chosen to match what the real file holds for Desktop-created objects (column profile of its
measure rows; no content copied):

| Table | Column | Value | Why |
|---|---|---|---|
| `Measure` | `ID` | next ID | global counter |
| | `TableID`, `Name`, `Expression`, `Description`, `FormatString`, `DisplayFolder`, `IsHidden` | from the model | |
| | `DataType` | 6 int64 / 8 double / 2 string / 11 boolean for a literal expression, else 20 variant | The engine computes it from the DAX; this layer has no DAX type inference. Existing rows hold 6, 8 and 10. A changed expression keeps the old value unless it is a literal. |
| | `State` | 1 (ready) | all existing rows |
| | `ModifiedTime`, `StructureModifiedTime` | now | |
| | `KPIID`, `DetailRowsDefinitionID`, `FormatStringDefinitionID` | 0 (none) | all existing rows |
| | `IsSimpleMeasure` | 0 unless `extra.isSimpleMeasure` | all existing rows |
| | `ErrorMessage`, `DataCategory`, `SourceLineageTag` | NULL unless in `extra` | all existing rows |
| | `LineageTag` | the model's, else a generated lower-case version-4 GUID | existing tags are lower-case GUIDs; generation is deterministic for a given clock |
| `Annotation` | `ObjectID`, `ObjectType`, `Name`, `Value`, `ModifiedTime` | owner, its code, model, now | |
| `Role` | `ModelID`, `Name`, `Description`, `ModelPermission`, `ModifiedTime` | model ID, model, `extra.description`, code (absent → 2 read), now | |
| `TablePermission` | `RoleID`, `TableID`, `FilterExpression`, `MetadataPermission`, `State`, `ErrorMessage`, `ModifiedTime` | role, table, model, code (absent → 0 default), 1, NULL, now | |

A changed measure expression or RLS filter also resets `State` to 1 and `ErrorMessage` to NULL
(where present).

### SQLite adapter

`tables_to_rows(&tables)` shapes the edited tables for an SQLite writer (`bicraft_sqlite::Row`:
rowid plus one value per declared column):

- `changed_tables(&base)` names the tables whose rows differ; a writer keeps every other table
  as it is.
- `rows_for(&TableInfo)`: tables whose `ID` is the `INTEGER PRIMARY KEY` (all TMSCHEMA object
  tables): rowid = `ID`, rows sorted by rowid. A non-NULL value for a column the table does not
  declare is `Error::Invalid`; missing columns take the declared default or NULL.
- `rows_for_keyed(&TableInfo, &original_rows)`: `DBPROPERTIES`, keyed by `NAME` with its own
  non-contiguous rowids (observed), which `MetadataTables` does not carry; rows are matched to
  the original ones by primary key to keep their rowids.

On the real file both reproduce every row of every loaded table exactly.

`write_sqlitedb(original, model)` does the whole step on database bytes: read →
`apply_model` → `bicraft_sqlite::rewrite` with new rows for the changed tables only (every
other table and its indexes are copied page by page; the SQLite change counter and schema
cookie move forward). An unchanged model returns the original bytes. The output is read back
and must map to the edited model. Change markers it moves, besides `MAXID`: the
`DBPROPERTIES` commit markers `LASTCOMMITTED` and `LASTMETADATACHANGEFORSYNCCOMMITTED` are
raised to the edit time (never lowered). Observed on the local Desktop file: every row's
`ModifiedTime` / `StructureModifiedTime` / `RefreshedTime` is at or before both markers, so
raising them keeps that invariant for the new times; `LASTCUBESCHEMACHANGE` is older than some
row times there, so it is not such a bound and is left alone, as are
`LASTCUBEDATACHANGE` (no data changes) and the `Model` row.

### Open questions (writing)

1. ~~**Desktop round trip not done yet.**~~ **Done** (M5.1, 2026-10-13): Desktop 2.158 opens a
   PBIX whose edited `metadata.sqlitedb` (added measure, format change, RLS role) was written
   this way, through a rewritten ABF, and shows the edits.
2. **Measure `DataType`.** Whether the engine recomputes it on load or trusts the stored value;
   20 (variant) is the fallback when BIcraft cannot infer it.
3. **`SortByColumnID`.** Changing it may require the engine to recompute the attribute hierarchy
   sort order stored with the data; untested.
4. **`DBPROPERTIES` change markers**: `write_sqlitedb` raises `LASTCOMMITTED` and
   `LASTMETADATACHANGEFORSYNCCOMMITTED` and leaves `LASTCUBESCHEMACHANGE` /
   `LASTCUBEDATACHANGE` alone (see "SQLite adapter"). Desktop accepts files written with these
   markers (M5.1). The engine does not check them on restore: a backup with the four markers
   (`LASTCOMMITTED`, `LASTMETADATACHANGEFORSYNCCOMMITTED`, `LASTCUBESCHEMACHANGE`,
   `LASTCUBEDATACHANGE`) moved to 1970 or to 2174, or with every row time moved to 1970 too,
   loads (G17, 2026-10-15, `abf.md` "Decisions").

## Structural writing: tables, columns, relationships (M5.4)

`apply_structural(base, model, data)` (and `_at` with an explicit clock) writes the edits that
`apply_model` refuses because they change what is stored: **tables, columns of kept tables and
relationships added or removed**. It returns `StructuralEdit { tables, plan, model }`: every
metadata row (storage tables included), the ABF files to remove and add (`StoragePlan`), and
the model as written (`build_model(&tables)`). `write_structural_sqlitedb(original, model,
data)` does it on database bytes and returns `StructuralWrite { sqlitedb, plan, model }`.
`base` must hold every table of the database: `tables_from_sqlitedb_all` (the plain
`tables_from_sqlitedb` loads only the model tables of `TABLES`).

Sources: observations on the Desktop 2.158 sample (local test data; structure, codes and
counts only, nothing of its content is recorded here), [MS-XLDM] §2.3–2.4 for the role of
dictionaries, segments and the `H$`/`R$`/`U$` system tables, [MS-SSAS-T] for the object rows.

### How a save uses it

```text
decompress DataModel → Abf::parse → metadata.sqlitedb bytes
encode the new objects (bicraft-vertipaq)          → EncodedData   (adapter, see below)
write_structural_sqlitedb(sqlitedb, model, &data)   → { sqlitedb, plan, model }
AbfWriter: remove(each plan.remove), add(each plan.add), replace_metadata_sqlitedb(sqlitedb)
finish → compress_datamodel → DataModel part
```

The plan paths are ABF logical paths (`StorageFolder.Path` + `\` + `StorageFile.FileName`,
with `/`), exactly what `AbfWriter::add`/`remove` take. After a save the `StorageFile` rows and
the class-100069 files of the backup (`metadata.sqlitedb` aside) are the same set; the real
round trip checks it.

### Two passes

1. **Structure.** The edited model is diffed against `build(base)`: tables and columns by name
   (ASCII case-insensitive), relationships by exact name, as `apply_model` matches them. Every
   difference is validated first (all refusals are collected and returned together as
   `UnsupportedChange`; nothing is written then), then removals run (relationships, columns,
   tables), then additions (tables, columns, relationships).
2. **Metadata.** `apply_model` runs on the result with the written model, so every other
   difference (descriptions, measures of a new table, annotations, sort-by columns, format
   strings, …) is applied exactly as in a metadata-only save, and its refusals still apply.
   Measures new to the base are inserted before the model's nested objects (perspectives,
   translations), which may name them: a replaced table's measures come back as new rows, and
   its perspective memberships with them.

A model without structural differences takes only pass 2 (same rows as `apply_model`, empty
plan). Self-checks before returning: no reference that did not dangle in the base dangles
(`dangling_references`), every removed file is gone from `StorageFile` and every added file has
a row. `write_structural_sqlitedb` also reads the database back (it must map to the written
model).

### Ownership graph (observed)

```text
Table ─ TableStorage ─ StorageFolder (.tbl, OwnerType 18)
  ├─ Partition ─ PartitionStorage ─ SegmentMapStorage
  │                               └ StorageFolder (.prt, OwnerType 20)
  ├─ Column ─ ColumnStorage ─ DictionaryStorage ─ StorageFile .dictionary (22, table folder)
  │    │                   └ ColumnPartitionStorage ─ StorageFile .idf (23, partition folder)
  │    │                                            └ SegmentStorage ─ StorageFile .idfmeta (24)
  │    └─ AttributeHierarchy ─ AttributeHierarchyStorage ─ StorageFile .hidx (27, table folder)
  │                                                    └ SystemTableID → H$ table
  ├─ Measure (─ KPI, DetailRowsDefinition, FormatStringDefinition)
  └─ Hierarchy ─ Level; HierarchyStorage ─ SystemTableID → U$ table
Relationship ─ RelationshipStorage ─ RelationshipIndexStorage ─ SystemTableID → R$ table
```

Each pointer exists in both directions (`ColumnStorage.ColumnID` / `Column.ColumnStorageID`,
`StorageFile.OwnerID` / owner `StorageFileID`, …). A system table is a full table: `Table` row
(`SystemFlags` 1, `ModelID` 0), `TableStorage`, `.tbl` folder, one `Partition` (`Type` 3 none)
with its storage, segment map and `.prt` folder, and `Column` rows with the same storage chain
as user columns but no attribute hierarchy. On the sample, every `StorageFile` row resolves to
an ABF file and no reference dangles (`dangling_references` is empty).

### Names and paths (observed)

| Item | Pattern |
|---|---|
| storage form of a name | every character that is not a letter, digit or space becomes a space (observed: `_`, `-`, `>` replaced; accented letters kept; other punctuation not observed, also replaced) |
| `TableStorage.Name`, `ColumnStorage.Name`, `PartitionStorage.Name`, `RelationshipStorage.Name` (user objects) | `<storage form of the name> (<object ID>)`; the row-number column's name is `RowNumber-2662979B-1795-4F74-8F37-6A1BA8059B61` in every table |
| system table `Name` = its `TableStorage.Name` = `PartitionStorage.Name` = `Partition.Name` | `H$<TableStorage.Name>$<ColumnStorage.Name>`, `R$<from TableStorage.Name>$<RelationshipStorage.Name>`, `U$<TableStorage.Name>$<storage form of the hierarchy name> (<hierarchy ID>)` |
| system `ColumnStorage.Name` | the column name (`POS_TO_ID`, `ID_TO_POS`, `INDEX`, `MULTI_LEVEL_ID`, `PARENT_POS`, `FIRST_CHILD_POS`, `CHILD_COUNT`) |
| table folder | `<TableStorage.Name>.tbl`; system tables `<TableStorage.Name>$(<table ID>).tbl` |
| partition folder | `<table folder>\<partition ID>.prt` |
| `.idf` / `.idfmeta` (partition folder) | `<v>.<TableStorage.Name>.<ColumnStorage.Name>.0.idf` / `.0.idfmeta`, `v` = 1 for user columns, 0 for system columns (their `DataVersion`); the `.0` is not the segment: `H$` files with 2 segments are still `.0` |
| `.dictionary` (table folder) | `1.<TableStorage.Name>.<ColumnStorage.Name>.dictionary` |
| `.hidx` (table folder) | `1.H$<TableStorage.Name>$<ColumnStorage.Name>.hidx` |

Names embed object IDs, so new objects never collide with old files.

### Rows written for a new table

"Model" = from the edited model; "encoder" = from `EncodedData`; "obs." = the value every
comparable row of the sample holds; "inf." = inferred from the sample's variations.

| Table | Column | Value |
|---|---|---|
| `Table` | `ModelID`, `Name`, `Description`, `IsHidden`, `LineageTag` | model (lineage tag generated, lower-case v4 GUID, when absent) |
| | `DataCategory`, `ShowAsVariationsOnly`, `IsPrivate`, `AlternateSourcePrecedence`, `ExcludeFromModelRefresh`, `SourceLineageTag`, `SystemManaged`, `ExcludeFromAutomaticAggregations` | model `extra` (TMSL names), else 0 / NULL |
| | `SystemFlags` | 2 for a calculated partition (obs.), 0 otherwise (inf.: bit 1 = calculated table) |
| | `ModifiedTime`, `StructureModifiedTime` | now |
| | `DefaultDetailRowsDefinitionID`, `RefreshPolicyID`, `CalculationGroupID` | 0 |
| | `DirectLakeIndexingBehavior` | 3 (obs. on every table) |
| `TableStorage` | `Name`, `StorageFolderID` | pattern above |
| | `Version`, `Settings`, `RIViolationCount` | 1, 4353, 0 (obs. on every user table) |
| `Partition` | `Name`, `QueryDefinition`, `Description`, `DataView`, `RetainDataTillForceCalculate` | model |
| | `Type` | `calculated` 2, `m` 4, `query` 1 (others refused) |
| | `Mode` | model code; 2 (default) when absent |
| | `State`, `DataSourceID`, `RangeStart`, `RangeEnd`, `RangeGranularity`, `QueryGroupID`, `ExpressionSourceID`, `DataCoverageDefinitionID` | 1, 0, 0.0, 0.0, −1, 0, 0, 0 (obs.) |
| | `SystemFlags` | as the table |
| | `ModifiedTime`, `RefreshedTime` | now (processed) |
| `PartitionStorage` | `Name`, `StorageFolderID`, `SegmentMapStorageID` | pattern above |
| | `StoragePosition`, `DataObjectId` | the partition's position in the table, 0, 1, … (obs. on engine-written tables of 2, 3 and 8 partitions; the engine refuses an image whose `DataObjectId` differs from the `.<k>.idf` position: "column partition file names … do not match") |
| | `DeltaTableMetadataStorageID` | 0 (obs.) |
| `SegmentMapStorage` | `Type`, `RecordCount`, `SegmentCount`, `RecordsPerSegment` | encoder (obs. user partitions: 3, rows, 1, rows; several 2^20-row segments: 1, rows, segments, 1,048,576, [`vertipaq.md`](vertipaq.md) §11) |

A table of several partitions (`EncodedTable::partitions`, one per model partition in order;
`m` or `query` sources only) gets one `Partition` / `PartitionStorage` / `SegmentMapStorage` /
`.prt` folder per partition; each column (row number included) gets one
`ColumnPartitionStorage` + `SegmentStorage` and one `1.<table>.<column>.<k>.idf`/`.idfmeta`
pair per partition, and one dictionary, `.hidx` and `H$` for the whole column; row numbers
continue across partitions (engine-checked, case `parts` of the structural check below).

Then the row-number column (position 0) and every model column in order, as below.

### Rows written for a new column

| Table | Column | Value |
|---|---|---|
| `Column` | `ExplicitName` / `InferredName` | the name; in `InferredName` (explicit NULL) when `extra.isNameInferred` (obs. on calculated-table columns) |
| | `ExplicitDataType`, `InferredDataType` | the data type code; explicit 1 when `extra.isDataTypeInferred`; inferred 19 on row-number columns (obs.) |
| | `Type` | `extra.type` (`calculatedTableColumn` 4, `rowNumber` 3), else 2 with an expression, else 1 |
| | `SourceColumn`, `Expression`, `FormatString`, `DataCategory`, `Description`, `DisplayFolder`, `IsHidden`, `SummarizeBy`, `LineageTag` | model (lineage generated when absent; row number: hidden, no tag) |
| | `IsUnique`, `IsKey`, `IsNullable`, `Alignment`, `TableDetailPosition`, `IsDefaultLabel`, `IsDefaultImage`, `IsAvailableInMDX`, `KeepUniqueRows`, `DisplayOrdinal`, `SourceProviderType`, `EncodingHint`, `SourceLineageTag` | model `extra`, else the TOM default; row number: unique, not nullable, key unless another column is `isKey` (obs.: the date table's row number loses `IsKey`) |
| | `State`, `ColumnOriginID`, `SortByColumnID`, `RelatedColumnDetailsID`, `AlternateOfID` | 1, 0, 0 (sort-by set by pass 2), 0, 0 |
| | `RefreshedTime` | 31240512000000000 (1699-12-31T00:00Z, the engine's "never": obs. on every column; see "Refresh state") |
| | `SystemFlags` | the table's bit 1 (2 in calculated tables, obs.) |
| | `ExpressionContext`, `StringIndexingBehavior`, `FullTextIndexingBehavior` | 1, 1, 0 (obs.) |
| `ColumnStorage` | `Name` | pattern above |
| | `StoragePosition` | 0 row number, then 1, 2, … in model order; a column added to an existing table takes max + 1 |
| | `Settings` | 2049 for calculated columns, 1 otherwise (obs.) |
| | `ColumnFlags` | bit sum (inf. from the observed 0, 8, 11, 15, 27, 31): 16 row number, 8 materialized attribute hierarchy, 4 `IsKey`, 3 unique (`IsUnique`, or the "one" end of a relationship) |
| | `Locale` | the most common non-zero `Locale` of the base's columns (all 1036 on the fr-FR sample), else 1033 |
| | `Collation`, `OrderByColumn`, `BinaryCharacters` | NULL, NULL, 0 (obs.) |
| | `Statistics_DistinctStates`, `_MinDataID`, `_MaxDataID`, `_OriginalMinSegmentDataID`, `_RLESortOrder`, `_RowCount`, `_HasNulls`, `_RLERuns`, `_OthersRLERuns`, `_DBType`, `_XMType`, `_CompressionType`, `_CompressionParam` | encoder |
| | `Statistics_Usage` | 0 on the "one" end of a relationship, else 3 (obs.) |
| | `Statistics_EncodingHint` | 1 on row-number columns and "one" ends, else 0 (obs.) |
| | Delta / index columns | `DeltaColumnMappingId` −1, the rest 0 / NULL (obs.) |
| `DictionaryStorage` | `Type`, `DataType`, `BaseId`, `Magnitude`, `LastId`, `IsNullable`, `IsOperatingOn32`, `DictionaryFlags`, `Size` | encoder |
| | `IsUnique` | encoder; forced 1 on a hash-encoded "one" end (obs. on all four) |
| | `DataVersion` | 1; `StorageFileID` the `.dictionary` row, 0 for value encoding (no file) |
| `ColumnPartitionStorage` | `DataVersion`, `State` | 1, 1 (obs. user columns) |
| `SegmentStorage` | `SegmentCount` | encoder |
| `AttributeHierarchy` | `State`, `ModifiedTime`, `RefreshedTime` | 1, now, now |
| `AttributeHierarchyStorage` | `MaterializationType`, `DistinctDataCount`, `MinValue`, `MaxValue`, `StringValueMaxLength` | encoder (obs.: 0 hash + `H$` with `POS_TO_ID`, `ID_TO_POS`; 1 value + `.hidx` + `H$` with `POS_TO_ID` only; 3 row number, no system table) |
| | `ColumnPositionToData`, `ColumnDataToPosition` | storage position of `POS_TO_ID` / `ID_TO_POS` in the `H$` table, −1 when absent |
| | `SystemTableID`, `StorageFileID` | the `H$` table, the `.hidx` row (0 when absent) |
| | `SortOrder`, `OptimizationLevel`, `DataVersion`, `HasStatistics` | 0, 0, 1, 1 |
| | no hierarchy (`isAvailableInMdx` false) | `MaterializationType`, both positions and `DistinctDataCount` −1, `HasStatistics` 0, min/max NULL (obs.) |

### Rows written for a system table (`H$`, `R$`)

| Table | Values (all obs.) |
|---|---|
| `Table` | `ModelID` 0, `IsHidden` 1, `SystemFlags` 1, `LineageTag` NULL, `DirectLakeIndexingBehavior` 3, times now |
| `TableStorage` | `Version` 1 / `Settings` 4 (`H$`); `Version` 0 / `Settings` 2 (`R$`); (`U$`: 0 / 5) |
| `Partition` | `Type` 3, `Mode` 2, `DataView` 3, `State` 1, `SystemFlags` 1, `RefreshedTime` 1699-12-31 (never) |
| `SegmentMapStorage` | encoder (obs. `H$`: type 2, distinct + 3 records, 2–3 segments; `R$`/`U$`: type 3, one segment) |
| `Column` | `ExplicitName` = column name, `ExplicitDataType` 6, `InferredDataType` 19, `Type` 1, `IsHidden` 0, `IsNullable` 1, `SystemFlags` 1, `AttributeHierarchyID` 0 |
| `ColumnStorage` | `Settings` by name: `POS_TO_ID` 7, `ID_TO_POS` 5, `INDEX` 3 (`U$`: `MULTI_LEVEL_ID` 16, `PARENT_POS` 8, `FIRST_CHILD_POS` 9, `CHILD_COUNT` 17, `BinaryCharacters` 10240); `ColumnFlags` 0, `Locale` 0; placeholder statistics: DistinctStates 1, Min/Max/OriginalMin DataID 2, RLESortOrder −1, RowCount 0, Usage 3, the rest 0 |
| `DictionaryStorage` | `Type` 0, `DataType` 19, everything else 0 (no file) |
| `ColumnPartitionStorage` | `DataVersion` 0, `State` 3 |

### Rows written for a new relationship

| Table | Column | Value |
|---|---|---|
| `Relationship` | `Name`, `IsActive`, ends, `CrossFilteringBehavior`, cardinalities, `JoinOnDateBehavior`, `RelyOnReferentialIntegrity`, `SecurityFilteringBehavior`, annotations | model (codes as in "Enumeration codes") |
| | `Type`, `State`, `RelationshipStorage2ID` | 1, 1, 0 (obs.) |
| | `ModifiedTime`, `RefreshedTime` | now |
| `RelationshipStorage` | `Name` | pattern above (the name is a GUID: hyphens become spaces) |
| | `DefinitionType`, `Cardinality`, `Flags` | 0, 0, 0 (obs.) |
| `RelationshipIndexStorage` | `IndexType`, `Flags`, `SecondaryRecordCount`, `StorageFolderID`, `StorageFileID`, `SecondarySystemTableID` | 1, 0, 0, 0, 0, 0 (obs.) |
| | `RecordCount` | the `R$` segment map's record count (obs.: the "one" column's last data id + 1) |
| | `SystemTableID` | the new `R$` table |

The "one" end's `ColumnStorage` gets the unique bits, `Statistics_Usage` 0 and
`Statistics_EncodingHint` 1, and its dictionary `IsUnique` 1, as observed on every "one" end.

Other cardinalities (engine-observed, Desktop 2.158 engine, case `rels` of the structural check):

| Cardinality | Storage |
|---|---|
| many-to-many (`fromCardinality` and `toCardinality` `many`) | none: `RelationshipStorageID` and `RelationshipStorage2ID` 0, `RefreshedTime` the OLE date zero, no `R$` table; both ends keep their storage (no unique bits); blanks and duplicates on both sides are allowed. The engine resolves the relationship at query time |
| one-to-one (both `one`) | two `RelationshipStorage` rows with the same name, each with its `RelationshipIndexStorage` and `R$` table: `RelationshipStorageID` maps from-column data ids to to-table rows (`R$<from table storage>$<name>`), `RelationshipStorage2ID` maps to-column data ids to from-table rows (`R$<to table storage>$<name>`); each index's `Flags` is 1 when some of its keys have no match; both ends are hash-encoded and marked unique |
| one-to-many (`from` one, `to` many) | not written (TOM puts the "one" side on `to`) |

`crossFilteringBehavior` (`bothDirections` 2) is a metadata property only: the storage is the
same as with one direction. A one-to-one relationship must have `bothDirections`: Desktop 2.158
refuses to open a PBIX or PBIP whose one-to-one relationship has one direction (observed with
bicraft-generated files); Microsoft's public "Model relationships in Power BI Desktop" article
lists *Both* as the only cross filter option for one-to-one. BIcraft
defaults one-to-one to both, refuses `single` and flags such a relationship in `model.validate`. The engine also re-encodes a value-encoded "many" end as hash when
it builds a relationship; BIcraft keeps the stored encoding (the index then spans the column's
value-encoded ids) and the engine loads it and answers the same queries.

### Rows written for a new hierarchy

Engine-observed (Desktop 2.158 engine, case `hier` of the structural check: a 2-level and a 3-level
hierarchy over hash- and value-encoded columns with blanks; BIcraft's `U$` tables equal the
engine's for the same data).

| Table | Column | Value |
|---|---|---|
| `Hierarchy` | `TableID`, `Name`, `State` 1, `HierarchyStorageID`, times now | model; display properties (`description`, `isHidden`, `displayFolder`, `hideMembers`, lineage tags, annotations) written by pass 2 |
| `Level` | `HierarchyID`, `Ordinal` 0, 1, … (top first), `Name`, `ColumnID` | model (levels sorted by `ordinal`) |
| `HierarchyStorage` | `Name` | `<storage form of the hierarchy name> (<hierarchy ID>)` |
| | `MaterializationType`, `StructureType` | 0, 2 (obs.) |
| | `LevelDefinition` | `$<level column's ColumnStorage.Name>$<first node row of the level>$` per level, then nothing: `$Country (13)$0$City (14)$4$` |
| | `SystemTableID` | the `U$` table |
| `U$` system table | name | `U$<TableStorage.Name>$<HierarchyStorage.Name>`, folder `…$(<table ID>).tbl`, files `0.<name>.<column>.0.idf` |
| | `TableStorage` | `Version` 0, `Settings` 5 |
| | columns | `MULTI_LEVEL_ID` (16), `PARENT_POS` (8), `FIRST_CHILD_POS` (9), `CHILD_COUNT` (17) (`ColumnStorage.Settings`), `BinaryCharacters` 10240 |

The nodes ([MS-XLDM] §2.4.4) are the distinct member combinations that occur on some row,
level by level, each level's nodes grouped under their parent and sorted by the attribute
hierarchy position of their own member (blank is a member): `MULTI_LEVEL_ID` is the member's
data id, the three position columns are stored with a +1 bias (−1: none). A level's member
order is its column's `H$` `POS_TO_ID` (first `DistinctDataCount` values) for stored columns,
the encoder's for new ones. The level columns' own storage is not touched. Removing a
hierarchy drops its rows, its `U$` table and its `PerspectiveHierarchy` rows; changing its
levels (names, columns, order) is a removal and an addition.

### Removals

Removing an object deletes every row of its subtree in the graph above (system tables
included) and the rows keyed by (`ObjectType`, `ObjectID`) (annotations, extended and changed
properties, translations, excluded artifacts); every `StorageFile` of a deleted owner or folder
goes to `plan.remove`.

| Object | Also |
|---|---|
| column | `PerspectiveColumn`, `ColumnPermission`, `Variation` rows; other columns sorted by it get `SortByColumnID` 0; the table's remaining `StoragePosition`s are renumbered 0, 1, … (a removed column leaves no gap; a fresh load never has one); the table's `ModifiedTime`/`StructureModifiedTime` become now |
| table | measures (with `KPI`, `DetailRowsDefinition`, `FormatStringDefinition`, `PerspectiveMeasure`), hierarchies (with `Level`, `HierarchyStorage`, the `U$` table), `TablePermission` + `ColumnPermission`, `PerspectiveTable` + children, `RefreshPolicy`, the default detail rows definition, a calculation group (`CalculationGroup`, its `CalculationItem` and `CalculationExpression` rows with their `FormatStringDefinition`s) |
| relationship | `RelationshipStorage` (both IDs), `RelationshipIndexStorage`, `R$` tables; the former "one" end keeps its flags (its data is still unique) |

### Refused (`UnsupportedChange`)

- removing the row-number column, a calculated-table column (the table expression defines the
  columns: the engine brings it back on the next calculation, engine-observed; the engine's
  `column.remove` rewrites the expression instead, which replaces the table), a column or table a kept relationship uses, a column referenced by `Level`,
  `Variation`, `AlternateOf`, `GroupByColumn`, `RelatedColumnDetails`,
  `CalendarColumnReference`, `ColumnPermission`, `PerspectiveColumn` or an expression's
  `ParameterValuesColumnID`, a column still named as `sortByColumn` in the edited model;
- removing a table with calendars or holding the default measure;
- adding a table without encoded data, whose partitions do not match the encoded ones (one,
  or several `m`/`query` partitions with `EncodedTable::partitions`), a partition source
  other than `calculated`/`m`/`query`/`calculationGroup` or with source properties (data
  source, expression source), or with default detail rows;
- adding a hierarchy without encoded nodes, without levels, with a repeated level name or
  column, or a level on a column without attribute hierarchy; removing one a date variation
  uses;
- adding a column without encoded data, to a table without a partition, a row-number or
  calculated-table column, encoded data whose row count differs from the partition's, or whose
  attribute hierarchy disagrees with `isAvailableInMdx`. A table of several partitions
  (incremental-refresh policy ranges, hybrid tables) needs one segment set per partition in
  storage order (`EncodedNewColumn::partitions`, each with that partition's row count); the
  column gets one `ColumnPartitionStorage` + `SegmentStorage` and one
  `1.<table>.<column>.<k>.idf`/`.idfmeta` pair per partition, `k` the partition's position, in
  the partition's folder, and one dictionary, `.hidx` and `H$` (G10, engine-verified);
- adding a relationship without a name, whose ends do not resolve, of a cardinality other
  than many-to-one, one-to-one or many-to-many, or (many-to-one, one-to-one) without its
  encoded `R$` index(es) or whose "one" end(s) are not hash-encoded;
- everything pass 2 refuses (renames, data type or expression changes, partition changes, …).

### Calculation groups and in-place relationship changes (M9.2)

Power BI Desktop 2.158 (the engine of that release), engine-observed in the committed
`tests/engine/objects.sqlitedb` fixture and checked by opening BIcraft's saves in Desktop:

- A calculation group table is stored like an import table. Its partition has `Type` 7
  (`calculationGroup`), `Mode` 2, `DataView` 3; the table has `SystemFlags` 0 and its
  `CalculationGroupID` points at the `CalculationGroup` row (`TableID` back). Each item is one
  row: the `Name`-sourced column holds the item names and the `Ordinal`-sourced column holds
  their ordinals, with the usual row-number column, `H$` tables and storage rows.
- New calculation group: the structural pass adds the table with these rows. Pass 2 then
  inserts the `CalculationGroup` and `CalculationItem` rows. Their nested kinds stay
  `insert: false` and are allowed only for a table whose stored data this edit wrote
  (`apply_fresh`).
- Items added, removed, renamed or moved: the group table's rows change, so the table is
  replaced (removed in a first stage with its calculation group rows, then added back). An
  item's expression, description or format string is a metadata change.
- `relationship.set` keeps the relationship's name. Activity (`IsActive`),
  `CrossFilteringBehavior`, `SecurityFilteringBehavior`, `RelyOnReferentialIntegrity` and
  `JoinOnDateBehavior` are metadata on its row. A cardinality change rebuilds its storage:
  the relationship is removed (with `RelationshipStorage`, `R$`) and added again under the
  same name.
- Marking a date table: `Table.DataCategory` `Time` and `Column.IsKey` 1 on the date column
  are metadata. The table's row-number column then has `IsKey` 0, as on tables the engine
  loads with a key column.

### Model and column properties (M9.1)

Engine checks with the Analysis Services engine of Power BI Desktop 2.158.1304.0 (the
maintainers' development oracles: a PBIX edited by BIcraft loaded by the engine, and TMSL
deploys), 2026-10-14:

| Change | Engine verdict | BIcraft |
|---|---|---|
| Measure with both `FormatString` and `FormatStringDefinitionID` | refused at load and deploy ("both FormatString property and FormatStringDefinition property defined") | setting a dynamic format string clears the static one, as TOM does (no copy is kept) |
| `FormatStringDefinition` row (`ObjectType` 8, `ObjectID` the measure) + `Measure.FormatStringDefinitionID`, `FormatString` NULL | loads | written by the nested-object writer |
| `Column.IsKey` on a user column while the row-number column keeps `IsKey` | refused ("two columns with the IsKey property") | the row-number column's `IsKey` follows (1 only while no user column is the key) |
| `Model.DiscourageImplicitMeasures`, `Model.SourceQueryCulture`, `SortByColumnID`, `DataCategory`, annotations | load | metadata-only updates |
| `isAvailableInMdx: false` on a sort-by column | refused at deploy (`SortByColumn` "set to an invalid column ID") | refused by `column.set` |
| `isAvailableInMdx: false` on a hierarchy level's column | refused at deploy | refused |
| `isAvailableInMdx: false` on a sorted column, either end of a relationship, a variation's default column | deploys, calculates and answers queries | warning only |
| A variation's target table without `showAsVariationsOnly` | refused at deploy | (Desktop's auto date tables have it) |

Visual check in Desktop 2.158 (2026-10-14, WebView2 capture of the report canvas with the
screen locked, `tools/render-oracle/capture.mjs`): a PBIX BIcraft built from scratch with a
measure whose dynamic format string switches on its value, `sort_by_column` (month name by
month number), `data_category` `WebUrl` and `City`, `is_key`, `discourage_implicit_measures` and
annotations opens with every property live: the card and the table total show the large-value
format, the rows and the column chart's axis the small-value one, months are ordered by number
on the axis and in the table, and the URL column is drawn as links.

Removing a table that `Variation` rows reference (an auto date table) is refused by the structural
writer only while the edited model keeps a variation that uses one of its columns; the
variations `column.remove_variation` / `table.remove` drop go with the edit.

### Encoder adapter

`EncodedData` is shaped so that the `bicraft-vertipaq` output maps field by field (a scratch
adapter wrote `m5/add_table.pbix` this way):

| `bicraft-metadata` | from `bicraft-vertipaq` |
|---|---|
| `ColumnStorageData { idf, idfmeta }` | `EncodedColumn { idf, idfmeta }` |
| `segment_count` | `ColumnStats::segment_count()` |
| `DictionaryData.file` | `EncodedColumn.dictionary` |
| `DictionaryData { kind, data_type }` | `stats.kind` (Hash 1, Value 2), `stats.storage_type` (Long 6, Real 8, String 2) |
| `DictionaryData { base_id, magnitude, last_id, is_nullable, is_unique, is_operating_on_32, flags, size }` | `stats.{base_id, magnitude, last_id, is_nullable, is_unique, is_operating_on_32, dictionary_flags, dictionary_size}` |
| `ColumnStatistics { distinct_states, min_data_id, max_data_id, original_min_segment_data_id, row_count, has_nulls, db_type, xm_type }` | `stats.{distinct_states, min_data_id, max_data_id, original_min_segment_data_id, rows, has_nulls, db_type, xm_type}` |
| `ColumnStatistics { rle_runs, others_rle_runs }` | `stats.segments[0].{rle_runs, others_rle_runs}` |
| `ColumnStatistics { rle_sort_order, compression_type, compression_param }` | −1, 0, 0 (obs. on every column) |
| `AttributeHierarchyData` (hash column) | `build_attribute_hierarchy(entries, Collation::for_culture(model culture))`: `materialization_type`, `distinct_data_count`, `min_value`/`max_value` as text, `string_value_max_length`, `system_table` from `.table` |
| `AttributeHierarchyData` (value column) | the same with `id_to_pos: false`, plus `hidx` = `EncodedColumn.hidx` |
| `AttributeHierarchyData` (row number) | `encode_row_number`; materialization 3, rows distinct, min `0`, max rows − 1, no system table |
| `SystemTableData { segment_map, columns }` | `SegmentMap { kind: kind.segment_map_type(), record_count(), segment_count(), records_per_segment }`; each column: `encode(i)` idf/idfmeta, `segment_count()`, `storage_settings` |
| `EncodedRelationship.index` | `build_relationship_index(…)` table, same mapping |
| `EncodedTable.segment_map` | `SegmentMap::partition(rows, segments, records_per_segment)` (`Type` 3 for one segment, 1 for several) |

### Verification

- Synthetic (`tests/structural.rs`): two new tables (an `m` table with hash, value and
  hierarchy-less columns; a calculated table), a relationship and a measure; every planned file
  has a `StorageFile` row, no reference dangles, names/flags/positions match the patterns
  above, the model reads back; then a column added to an existing table, removals of a
  value-less column (positions compacted), a value column (`H$` and `.hidx` gone), the
  relationship (`R$` gone) and both tables, which brings every row back to the base and
  removes exactly the files added; refusals; the same through an SQLite file with the real
  schema's columns.
- Real (`tests/real_structural.rs`, local only): removing an unreferenced calculated column
  (16 tables change, 7 files go: its `.idf`/`.idfmeta`/`.dictionary` and its `H$` table's
  four files) and an unreferenced table (56 files); the base and the result have no dangling
  reference, the plan equals the `StorageFile` rows that went away, the re-packed ABF holds
  exactly the files the rows name, and the re-compressed PBIX reads back as the edited model.
  Outputs: `$BICRAFT_TESTDATA/m5/remove_column.pbix`, `remove_table.pbix`; a scratch adapter
  over the vertipaq encoder also wrote `add_table.pbix` (a 3-row, 2-column calculated table,
  16 files). Opening them in Desktop is the pending acceptance check.

### TODO (not derivable from the sample)

1. **Import (`m`) tables and data columns**: the sample has only calculated tables. Assumed:
   `TableStorage.Settings` 4353, `Table`/`Partition`/`Column.SystemFlags` 0, `ColumnStorage.Settings`
   1, `InferredDataType` = the data type. A Desktop-saved import table would settle them. Desktop
   2.158 opened import tables written with these values (M `#table` partitions, up to 200 000
   rows), answered queries over them and refreshed one in place (local check, 2026-10-13).
2. **Calculated columns and tables without data**: definitions are written only with evaluated
   data (`EncodedData`). The engine now evaluates them with BIcraft's DAX evaluator
   (`pbix-package.md`, "Structural saves"); what it cannot evaluate is refused. Whether the engine
   would accept a definition with an unprocessed state and compute it on load is still unknown.
3. **Multi-partition tables**: done (G10 for new columns, 2026-10-14 for new and replaced
   tables, engine-checked).
4. **New user hierarchies**: done (2026-10-14, engine-checked, see above).
5. **Relationships to a value-encoded "one" end**: refused; the column would have to be
   re-encoded as hash first. `RIViolationCount` stays 0 (referential-integrity violations are
   not counted).
6. **Renames** (table, column): `write_structural_sqlitedb_renamed_at` renames the object rows
   only (`Table.Name`, `Column.ExplicitName`, times now); storage names, folders, files and
   `H$`/`R$` names keep the old name next to the object ID. Desktop 2.158 opened a file with a
   renamed table and a renamed column (used by a relationship) and answered queries through both
   (local check, 2026-10-13). Columns named by their table expression (`InferredName`) are refused;
   relationship renames are not offered.
7. **Name sanitization** of characters other than `_`, `-`, `>`, and any length limit on
   storage names, are unconfirmed.
8. **`DBPROPERTIES` markers**: as for metadata-only saves, `LASTCOMMITTED` and
   `LASTMETADATACHANGEFORSYNCCOMMITTED` move to the edit time; `LASTCUBEDATACHANGE` moves too
   (never backward) when the save adds data files (new tables, partitions, columns: the engine
   raises it on every refresh commit, and reports it as `MDSCHEMA_CUBES.LAST_DATA_UPDATE`);
   `LASTCUBESCHEMACHANGE` is left alone (a structural change might be expected to move it).
9. **Column positions after a removal** are compacted; leaving a gap is the alternative if
   Desktop rejects `remove_column.pbix`-style files with a middle column removed (the real
   check removes the last column, so it does not exercise compaction).
10. **Sort-by columns of new columns** change the `H$` member order: the encoder must build
   the attribute hierarchy with `sort_by` (pass 2 only writes `SortByColumnID`).

### Engine check of the structural writer

The structural check (one of the maintainers' development oracles, kept outside this
repository; Analysis Services engine of Desktop 2.158) deploys our small synthetic models, saves
them with AMO `ImageSave`, writes BIcraft's structural save of the target model over the base
image (`DataModelEditor`), `ImageLoad`s it (the engine checks the image) and compares DAX and MDX
answers and the TOM structure with the engine's own deployment of the target:

| Case | Edit | Result (2026-10-14) |
|---|---|---|
| `rels` | many-to-many (both directions, an inactive text one with blanks), one-to-one (both directions), many-to-one (both directions) relationships added between stored tables | loads; 11 queries equal (filters both ways, `USERELATIONSHIP`, blank rows, `RELATED` over 1:1); `R$` of the 1:1 equal to the engine's |
| `hier` | two hierarchies added to a stored table | loads; `U$` tables and `LevelDefinition` identical to the engine's; MDX member sets equal |
| `persp` | a calculated table in two perspectives (columns, measure, hierarchy) with a relationship given new data | loads; perspectives, measure, hierarchy and relationship as the engine's |
| `parts` | a new table of two `m` partitions with a relationship; its second partition given new data; a data column and a hierarchy added to it | each step loads; answers equal to calculated reference tables holding the expected rows |
| `calccol` | a calculated table's column removed by `column.remove` (expression rewritten with `SELECTCOLUMNS`) | loads; the engine infers the same columns and types from the rewritten expression; same values |

## Refresh state (read-only)

`Partition.State`, `Partition.RefreshedTime`, `Partition.ModifiedTime`, `Partition.ErrorMessage`,
`Table.ModifiedTime` / `StructureModifiedTime`, `Model.ModifiedTime` / `StructureModifiedTime`
and the `DBPROPERTIES` entries `LASTCUBEDATACHANGE` / `LASTCUBESCHEMACHANGE` are the engine's
record of when data was last loaded. BIcraft reads them (`model.storage_stats`,
`model.get {detail:"full"}`, `table.get`; see `docs/mcp.md`) and never edits existing values.

Engine check (a private instance of the engine of Desktop 2.158.1304.0, 2026-10-14, our own synthetic models):
a database of three calculated tables deployed with TMSL `createOrReplace`, one refreshed
(`refresh` type `calculate`, one table), one left unrefreshed and one with an invalid DAX
expression; `ImageSave`, wrapped as a PBIX, read by BIcraft, and compared with
`TMSCHEMA_PARTITIONS`, `TMSCHEMA_TABLES`, `TMSCHEMA_MODEL`, `MDSCHEMA_CUBES` and TOM, before and
after `ImageLoad` of the image (identical):

| Item | Stored | Engine view |
|---|---|---|
| Times | Windows FILETIME ticks (100 ns since 1601-01-01), **UTC** | DMV `DateTime` (kind unspecified) equal to the UTC value, rounded to 1/300 s (the decoded FILETIME is the more precise one; differences under 3.4 ms) |
| "Never" | `31240512000000000` | `1699-12-31T00:00:00` (`RefreshedTime` of unrefreshed partitions, of columns and of system partitions); read as `null` |
| `State` | integer | TOM `ObjectState`: 1 Ready, 3 NoData, 4 CalculationNeeded, 5 SemanticError, 6 EvaluationError, 7 DependencyError, 8 Incomplete, 10 ForceCalculationNeeded |
| Refreshed partition | `State` 1, `RefreshedTime` = `ModifiedTime` = the refresh commit | same |
| Deployed, not refreshed (calculated table) | `State` 4, `RefreshedTime` never | `CalculationNeeded` |
| Invalid expression | `State` 5, `RefreshedTime` never, `ErrorMessage` an XML `<Errors><Error><Hresult>…</Hresult><Notes>…</Notes></Error></Errors>` (the HRESULT and its arguments) | the DMV shows a rendered message text instead; BIcraft passes the stored form through |
| `LASTCUBEDATACHANGE` | FILETIME | `MDSCHEMA_CUBES.LAST_DATA_UPDATE` (just after the last refresh commit) |
| `LASTCUBESCHEMACHANGE` | FILETIME | `MDSCHEMA_CUBES.LAST_SCHEMA_UPDATE` |
| Database `LastProcessed` (TOM) | not stored as such | on our images: the latest partition `RefreshedTime`; "never" while a partition of the engine database above was unrefreshed |

The model-level `last_refresh` BIcraft reports is the latest partition `RefreshedTime` of the
user tables (system tables, `SystemFlags` bit 0, left out).

What BIcraft writes (same check, images loaded by the engine and its DMVs read back):

- **PBIX from scratch and new tables / partitions of a structural save**: `State` 1,
  `ModifiedTime` = `RefreshedTime` = the save time (the time the data was materialized);
  `LASTCUBEDATACHANGE` raised to it. The engine loads them as `Ready` with those times and
  `LAST_DATA_UPDATE` follows. A replaced table (new partition data) gets new partitions and so
  the new time; untouched tables keep theirs.
- **Metadata-only save**: partition rows untouched, so `RefreshedTime` and `State` keep the
  values of the base file; `LASTCUBEDATACHANGE` unchanged.
- **`model.refresh` on a PBIX** (M11.18): each refreshed table is a replaced table, so its new
  partitions are written as above (`State` 1, `ModifiedTime` = `RefreshedTime` = the refresh's
  start time, `LASTCUBEDATACHANGE` raised to it); the tables it does not refresh keep their rows
  and times. Existing values are still never edited in place. Engine check: not run for refreshes
  yet (the same writer as replaced tables, which is engine-checked).
- The structural check (all cases) still loads every image with no
  difference after the `LASTCUBEDATACHANGE` change.

## Open questions

1. **Compatibility level and database name.** Not in `metadata.sqlitedb` (`DBPROPERTIES` has
   `SCHEMAVERSION`, not the compatibility level). Probably in the ABF's database XML; the
   container reader should fill them in.
2. **`IsAvailableInMDX`.** Why the reference TMDL omits `false` for columns where the database
   stores `0`: Desktop may set it as an engine optimization without exposing it through TOM, or
   the serializer may skip it for calculated-table columns. Needs a Desktop round trip. Settled
   on the engine: TOM's TMSL name is `isAvailableInMdx`, and the engine scripts `false` for a
   column deployed with it. Since 2026-10-14 BIcraft reads it as `isAvailableInMdx` (TOM's
   name); the writers still accept the engine spelling `isAvailableInMDX` on input.
3. Resolved: the integer model defaults (10, −1, −1, 0) are TOM's; a model setting every one of
   them reads back equal to the engine's TMSL.
4. Resolved: `SelectionExpressionBehavior` uses TOM's codes (0 automatic, 1 visual,
   2 nonVisual), not the spec's.
5. Resolved: entity partitions keep `expressionSource`, `schemaName` and `entityName` in
   `source`.
6. Key order inside `extra` is not significant for model equality (JSON maps compare
   unordered); writers keep the order they read.

[model]: https://learn.microsoft.com/en-us/openspecs/sql_server_protocols/ms-ssas-t/60094cd5-1c7e-4353-9299-251bfa838cc6
[table]: https://learn.microsoft.com/en-us/openspecs/sql_server_protocols/ms-ssas-t/6360ac84-0717-4170-bce0-284cbef419ca
[column]: https://learn.microsoft.com/en-us/openspecs/sql_server_protocols/ms-ssas-t/00a9ec7a-5f4d-4517-8091-b370fe2dc18b
[partition]: https://learn.microsoft.com/en-us/openspecs/sql_server_protocols/ms-ssas-t/81badb81-31a8-482b-ae16-5fc9d8291d9e
[relationship]: https://learn.microsoft.com/en-us/openspecs/sql_server_protocols/ms-ssas-t/35bb4a68-b97e-409b-a5dd-14695fd99139
[measure]: https://learn.microsoft.com/en-us/openspecs/sql_server_protocols/ms-ssas-t/ab331e04-78f7-49f0-861f-3f155b8b4c5b
[hierarchy]: https://learn.microsoft.com/en-us/openspecs/sql_server_protocols/ms-ssas-t/4eff6661-1458-4c5a-9875-07218f1458e5
[annotation]: https://learn.microsoft.com/en-us/openspecs/sql_server_protocols/ms-ssas-t/7a16a837-cb88-4cb2-a766-a97c4d0e1f43
[role]: https://learn.microsoft.com/en-us/openspecs/sql_server_protocols/ms-ssas-t/94a8e609-b1ae-4814-b8dc-963005eebade
[rolemembership]: https://learn.microsoft.com/en-us/openspecs/sql_server_protocols/ms-ssas-t/dbecc1f4-142b-4765-8374-a4d4dc51313b
[tablepermission]: https://learn.microsoft.com/en-us/openspecs/sql_server_protocols/ms-ssas-t/ac2ceeb3-a54e-4bf5-85b0-a770d4b1716e
[extprop]: https://learn.microsoft.com/en-us/openspecs/sql_server_protocols/ms-ssas-t/5c1521e5-defe-4ba2-9558-b67457e94569
[expression]: https://learn.microsoft.com/en-us/openspecs/sql_server_protocols/ms-ssas-t/61f98e45-d5e3-4435-b829-2f2f043839c1
[linguistic]: https://learn.microsoft.com/en-us/openspecs/sql_server_protocols/ms-ssas-t/f8924a45-70da-496a-947a-84b8d5beaae6
[perspectivetable]: https://learn.microsoft.com/en-us/openspecs/sql_server_protocols/ms-ssas-t/06bc5956-20e3-4bd2-8e5f-68a200efc18b
[calcitem]: https://learn.microsoft.com/en-us/openspecs/sql_server_protocols/ms-ssas-t/f5a398a7-ff65-45f0-a865-b561416f1cb4
[modetype]: https://learn.microsoft.com/en-us/dotnet/api/microsoft.analysisservices.tabular.modetype
[pst]: https://learn.microsoft.com/en-us/dotnet/api/microsoft.analysisservices.tabular.partitionsourcetype
[vfb]: https://learn.microsoft.com/en-us/dotnet/api/microsoft.analysisservices.tabular.valuefilterbehaviortype
