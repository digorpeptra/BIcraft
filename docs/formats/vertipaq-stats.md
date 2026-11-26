# VertiPaq storage statistics and VPAX

How `model.storage_stats` and `model.export_vpax` (`crates/engine/src/storage_stats.rs`,
`vpax.rs`, `crates/pbix/src/storage.rs`) derive the figures VertiPaq Analyzer shows, and how they
compare with the Analysis Services engine. Observed on Power BI Desktop 2.158.1304.0 (Analysis
Services 17.0.87.18), 2026-10-14, on synthetic models of ours.

## Sources

Everything comes from the stored backup (a PBIX `DataModel`, or a PBIP `.pbi/cache.abf`), and
only from three things in it:

- `metadata.sqlitedb`: the storage tables (`docs/formats/abf.md`, "Storage objects");
- the file list (logical path and size of every file, from the BackupLog);
- the `.idfmeta` segment maps (`docs/formats/vertipaq.md`). The `.idf` data, `.dictionary`
  and `.hidx` files are never read.

A backup within `DataModelLimits` is decompressed once in memory. A larger one (or a part left in
the file) is streamed: one short pass for the header page, one for the virtual directory (with
the 64 MiB before it, where the BackupLog usually is), then one pass that keeps
`metadata.sqlitedb` and every `.idfmeta`, so a backup of any size needs only those in memory.

## Figures

The engine's storage DMVs name the same objects: `DISCOVER_STORAGE_TABLES` (`TABLE_ID`,
`ROWS_COUNT`), `DISCOVER_STORAGE_TABLE_COLUMNS` (`DICTIONARY_SIZE`, `COLUMN_ENCODING`) and
`DISCOVER_STORAGE_TABLE_COLUMN_SEGMENTS` (one row per segment of every column of every user and
system table). `TABLE_ID` and `COLUMN_ID` are `TableStorage.Name` / `ColumnStorage.Name`.

| Figure | Derived from | DMV |
|---|---|---|
| table rows | sum of `SegmentMapStorage.RecordCount` of the table's partitions | `ROWS_COUNT` |
| partition rows, segments | `SegmentMapStorage.RecordCount`, `SegmentCount`, `RecordsPerSegment`; partition number `PartitionStorage.StoragePosition` | `TABLE_PARTITION_NUMBER` |
| segment rows | `.idfmeta` segment `records` | `RECORDS_COUNT` |
| segment used size | 8 × (words of the segment's primary block + words of its bit-packed sub-segment), from the `.idfmeta` data objects (`CSDOs`); 0 for a segment that stores nothing | `USED_SIZE` (= `ALLOCATED_SIZE`) |
| segment bits | the `XMRENoSplit<N>` width of the segment, or of the bit-packed half of a hybrid RLE segment; 0 for `XM123` | `BITS_COUNT` |
| segment bookmark bits | the segment statistics' RLE run count (`SS` `rle_runs`) | `BOOKMARK_BITS_COUNT` |
| compression | `NOSPLIT` for bit-packed and hybrid RLE segments, `C123` for `XM123` (alone or as the hybrid's inner class) | `COMPRESSION_TYPE` |
| state | `SKIPPED` for the row-number column, `COMPLETED` otherwise | `VERTIPAQ_STATE` |
| dictionary size | `DictionaryStorage.Size` | `DICTIONARY_SIZE` (after a load, see below) |
| encoding | `DictionaryStorage.Type` 1 hash, 2 value | `COLUMN_ENCODING` 1 / 2 |
| column data size | sum of the column's segment used sizes, every partition | |
| cardinality | rows of the column's `H$` table minus 3 (`AttributeHierarchyStorage.SystemTableID`); without one, `DistinctDataCount`, else `ColumnStorage.Statistics_DistinctStates` | `ROWS_COUNT - 3` of the `H$` table (VertiPaq Analyzer's rule) |
| column hierarchy size | sum of the used sizes of every segment of every column of its `H$` table | `H$` segments |
| user hierarchy size | the same over the hierarchy's `U$` table (`HierarchyStorage.SystemTableID`) | `U$` segments |
| relationship size | the same over the `R$` table of `RelationshipStorageID`'s index (kept with the from table) and of `RelationshipStorage2ID`'s (one-to-one, kept with the to table) | `R$` segments |
| missing keys | `RelationshipIndexStorage.Flags` bit 1 | |
| RI violations | `TableStorage.RIViolationCount` | `RIVIOLATION_COUNT` |

Totals follow VertiPaq Analyzer: column total = data + dictionary + hierarchy; table total =
columns + user hierarchies + relationships kept with the table; percent of model over the sum of
the table totals. When an `.idfmeta` has no data objects (some writers leave them out), the
`.idf` file size is shared out among its segments by rows.

## Engine check

Checked during development against Microsoft's Analysis Services engine (the maintainers'
development oracles, kept outside this repository): a synthetic TMSL model of ours is deployed and
refreshed, its image is saved and loaded back (the state of a file Desktop opens), the three DMVs
are read, `model.storage_stats` runs on the image wrapped in a package, and every figure is
compared (`cargo xtask storage-compare`). Results, all equal:

| Case | Model | Figures compared | Differences |
|---|---|---|---|
| `stats` | 2.5 million rows in 3 segments (unique, low-cardinality, text, double, date, boolean and sparse columns), a dimension with a user hierarchy, a currency column, a many-to-one relationship | 249 | 0 |
| `rels` (structural check case) | many-to-many, one-to-one, inactive and both-direction relationships, blanks | 183 | 0 |
| `hier` (structural check case) | user hierarchies, text with case variants | 57 | 0 |

One difference is expected and not compared: right after a refresh (before any save and load),
the engine reports text dictionaries about 1 MiB larger than `DictionaryStorage.Size` (the
in-memory hash table built during processing). A loaded file, as Desktop has it, reports exactly
`DictionaryStorage.Size`. The processed figures are kept in `<case>.processed.dmv.json`.

## VPAX

A VPAX file is VertiPaq Analyzer's exchange format (sql-bi/VertiPaq-Analyzer, MIT license;
`Dax.Vpax` and `Dax.Metadata`, read 2026-10-14 for the shape only, no code reused). It is an OPC
package (`System.IO.Packaging`, so a `[Content_Types].xml` is required) with:

- `DaxModel.json`: the `Dax.Metadata.Model` object graph serialized by Json.NET with
  `PreserveReferencesHandling.All`, `ReferenceLoopHandling.Serialize`,
  `DefaultValueHandling.Ignore`: every object carries `$id`, every list is
  `{"$id", "$values": [...]}`, an object met again is `{"$ref": id}` (references point backwards
  only); names, notes and expressions are plain strings; enums are numbers (partition `Mode`:
  0 import, 1 DirectQuery, 2 default, 3 push, 4 dual, 5 Direct Lake; `Type`: 1 query,
  2 calculated, 3 none, 4 M, 5 entity, 6 policy range, 7 calculation group, 8 inferred).
- `DaxVpaView.json` (optional, for the Excel template): not written.
- `Model.bim` (optional): the model as TMSL; BIcraft writes it unless `model_bim: false`.

What BIcraft fills: model (`ModelName`, the file name when the model has none, as Desktop;
`CompatibilityLevel`, `Culture`, extractor fields `bicraft` / `bicraft-engine` with the version,
`DaxModelVersion` 1.9.0), tables (`RowsCount`, RI violations, hidden, local / template date
tables, calculated table expression, `IsDateTable` from `dataCategory` `Time`), columns (the
row-number column first; cardinality, data type, column type, encoding, dictionary size, format
string, display folder, description, sort-by column, key / nullable / unique / MDX flags,
`ColumnSegments` with their partition, `ColumnHierarchies`), measures (expression, dynamic format
string, format string, folder, description, hidden), user hierarchies (size and levels as column
references), partitions, relationships (cardinalities, cross filtering, security filtering,
activity, `UsedSizeFrom` / `UsedSizeTo`). As in VertiPaq Analyzer's extractor, a column's
`ColumnHierarchies` are the segment-0 rows of its `H$` structures (`POS_TO_ID`, `ID_TO_POS`);
`HierarchiesSize` is their sum. Roles are left empty; `MissingKeys` / `InvalidRows` (which need
DAX queries) are 0.

### DAX Studio check (DAX Studio 3.6.1, 2026-10-14)

A PBIX BIcraft built from scratch (two M tables, a relationship, a sort-by column, a measure
with a dynamic format string) was exported with `bicraft vpax` and imported with DAX Studio's
Advanced → Import Metrics (driven by UI Automation), next to the VPAX DAX Studio's own
`dscmd vpax` extracts from Desktop 2.158 with the same file open.

- **Import failed at first** (`NullReferenceException`, then `ArgumentNullException`, in
  ADOTabular's VPAX metadata visitor, which builds the metadata pane): DAX Studio reads every
  table, column and measure `Description`, every measure `DisplayFolder` without a null check,
  and parses every measure `DataType` as a TOM `DataType`. BIcraft left empty notes out and had
  no measure type. Now written as VertiPaq Analyzer's extractor writes them: `Description`,
  `DisplayFolder`, `FormatString` always (`""` when unset), `EncodingHint` (`Default`), measure
  `DataType` from the DAX type checker (as `model.analyze`; `Variant` when unknown).
- The extractor's row-number column has no `ColumnCardinality` (0), `IsUnique` true and `IsKey`
  true unless a model column is the key: BIcraft now writes the same.
- Lists: the extractor writes plain JSON arrays (`PreserveReferencesHandling.Objects`), BIcraft
  writes `{"$id", "$values"}` wrappers; Json.NET reads both (checked with DAX Studio's own
  `Dax.Vpax` / `Dax.ViewModel` libraries as well as in the UI).
- After the fix the import succeeds and the VertiPaq Analyzer view (`Dax.ViewModel.VpaModel`)
  shows the same tables, rows, columns, cardinalities, encodings, data, hierarchy and
  relationship sizes as the extractor's VPAX, with two differences that are not VPAX matters:
  `ModelName` (the extractor reports Desktop's TOM model name `Model`; BIcraft the model or
  file name) and the dictionary size of hash-encoded double/date columns (Desktop's engine
  reported 1 304 bytes for a 6-value dictionary that BIcraft's PBIX writer stores as 88, the
  file size: the engine reports its own in-memory size for files it loads). Fixed on 2026-10-15
  (G17): the writer now stores the engine's figure, 1 304 for that column (`vertipaq.md`,
  "Dictionary size"; hash collisions of 8-byte keys and strings excepted). On engine-written
  files the stored size is the engine's, so the figures agree.

Fields the extractor writes that BIcraft leaves out (DAX Studio does not need them):
`ServerName`, `ServerVersion`, `ServerLocation`, `LastDataRefresh`, `LastProcessed`,
`LastUpdate`, `Version`, `ExtractorProperties`, `IsReferenced`, and the `DaxVpaView.json` part.
