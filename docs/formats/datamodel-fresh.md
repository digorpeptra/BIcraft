# A `DataModel` built from scratch (M5)

Status: implemented 2026-10-14. BIcraft writes a PBIX without an existing `DataModel`: a new
model (`model.new`, `project.new`), a `model.bim`, a PBIT, or a PBIP project without
`.pbi/cache.abf`. Companion to [`abf.md`](abf.md) (container), [`metadata-sqlitedb.md`](metadata-sqlitedb.md)
(rows), [`vertipaq.md`](vertipaq.md) (column data) and [`pbix-package.md`](pbix-package.md) §8
(structural saves).

**Sources.** Images the Analysis Services engine of Power BI Desktop 2.158.1304.0 (engine
17.0.87.18) writes with AMO `ImageSave` for databases it has just created from synthetic TMSL
scripts of ours (an empty model; one M table never processed; one calculated table without
rows; small calculated and M models), compared with Desktop-saved backups for structure only.
Tooling: the maintainers' development oracles, kept outside this repository. Nothing is copied
from an engine file except the format facts below.

## How BIcraft builds it

```text
model ──► bicraft_metadata::fresh_sqlitedb_at   Model row, query groups, shared expressions
      ──► bicraft_abf::new_backup               header, ADDITIONAL_LOG, PARTITIONS, <id>.1.db.xml,
                                                 0.CryptKey.bin, metadata.sqlitedb, LOG, directory
      ──► XPress9 (compress_datamodel)           = an empty database image
      ──► structural save (pbix-package.md §8)   every table, column, relationship, hierarchy,
                                                 measure, role, culture, annotation … as additions
```

The empty image holds only what the structural writer cannot add later (model-level scalar
properties, shared expressions and their query groups). Everything else is written by the same
code that adds objects to an existing PBIX, so a model built from scratch and an edited one share
every row and file writer. The engine side (`crates/engine/src/pbix_new.rs`) gathers the data as
for any structural save: M partitions run in BIcraft's Power Query evaluator (files only under
the session's `data_root`), DAX literal tables, calculated tables and calculated columns in the
DAX evaluator.

- `file.save_as {path: "x.pbix"}`, `file.convert`, `bicraft-cli convert IN OUT.pbix [--data-root DIR]`
  take this path whenever the source has no data (`built_from_scratch: true` in the result).
- A table whose source BIcraft cannot run (database, web and script connectors, a file outside
  `data_root`) is refused with its name. With `empty_tables: true` (`--empty-tables`) it is
  written without rows from a stand-in empty `#table` of its columns, then its real query is put
  back in `Partition.QueryDefinition` (`bicraft_pbix::fresh::set_partition_queries`), so
  Desktop runs it on the next refresh (`empty_tables` in the result lists them).
- Refused because they point at objects that do not exist yet: a model `defaultMeasure`, an
  expression's `parameterValuesColumn` (set them after the first save).

## Backup layout of a new database (observed)

Same container as [`abf.md`](abf.md); a new database has exactly these files:

| # | Storage name | Logical path | File group | Content |
|---|---|---|---|---|
| 1 | `ADDITIONAL_LOG` | | — | BOM + UTF-16LE `<Property><ProductName>Default</ProductName></Property>` |
| 2 | `PARTITIONS` | | — | BOM + UTF-16LE `<Partitions/>` |
| 3 | random | `<root>\<id>.<ObjectVersion>.db.xml` (outside the database folder) | 100002 | database object |
| 4 | random | `<root>\<id>.0.db\0.CryptKey.bin` | 100002 | database key |
| 5… | random | VertiPaq files | 100069 | added by the structural writer, before `metadata.sqlitedb` |
| n−1 | random | `<root>\<id>.0.db\metadata.sqlitedb` | 100069 | |
| n | `LOG` | | — | BackupLog |

BackupLog: `BackupRestoreSyncVersion` 11.53, `ServerRoot`, `SvrEncryptPwdFlag` true,
`ServerEnableBinaryXML`/`ServerEnableCompression`/`CompressionFlag`/`EncryptionFlag` false,
`ObjectName` = `ObjectId` = the database id, `Write` ReadWrite, `OlapInfo` false, `IsTabular`
true, empty `Collations`, `Languages` (the LCID), then the two file groups: class 100002
(`ObjectVersion` = the database's, `PersistLocation` 0, `ObjectID` = the database's) and class
100069 (`ObjectVersion` −1, `PersistLocation` −1, `ObjectID` all zeros), both with
`PersistLocationPath` = the database folder. BIcraft writes `ServerRoot` `\\?\C:\bicraft\Data`:
Desktop restores into its own workspace and only the paths relative to the database folder
matter (checked: images with that root open in Desktop and load in the engine).

### Database object (`<id>.1.db.xml`)

UTF-8, no declaration: an ASSL `Load` → `ObjectDefinition` → `Database` with `Name`, `ID`,
`CreatedTimestamp`, `LastSchemaUpdate` (`xs:dateTime`, UTC), `ObjectVersion`, `ObjectID`,
`Ordinal`, `PersistLocation` 0, `System` false, `DataFileList` (`0.CryptKey.bin`),
`Description`, `ddl800:DbUniqueId`, `AggregationPrefix`, `Language` (LCID), `Collation`,
`ddl400:DefaultCollationVersion` 100, `Visible` true, `MasterDataSourceID`,
`ProcessingPriority` 0, `ddl200_200:StorageEngineUsed` `TabularMetadata`,
`ddl200:CompatibilityLevel`, `Translations`, `DataSourceImpersonationInfo`
(`ImpersonationMode` Default, `Account`, and in engine files an encrypted `Password`), and
`DataVersion` 0. The engine declares about forty `ddl…` namespaces; BIcraft declares only the
ones it uses. The database name and compatibility level of the model go here (they are not in
`metadata.sqlitedb`); BIcraft leaves `Password` out.

### Database key (`0.CryptKey.bin`)

Required: every image without it (with or without `DataFileList`, `Password` or
`SvrEncryptPwdFlag`) is refused by Desktop, and so is one whose key block is not well formed;
the engine's `ImageLoad` refuses a backup without it, saying the password encryption setting
requires the key in the database object's file group.
144 bytes, identical in layout in every engine-written backup:

| Offset | Size | Content |
|---|---|---|
| 0 | 16 | an identifier, the same in every key file seen |
| 16 | 32 | eight little-endian 32-bit fields: 4, 48 (key blob offset), 80 (key blob length), 16 (identifier length), 1, 7, −1, 0 |
| 48 | 12 | a CryptoAPI `SIMPLEBLOB` header: blob type 1, version 2, `CALG_3DES` (0x6603), then `CALG_RSA_KEYX` (0xA400) |
| 60 | 64 | a PKCS #1 v1.5 type-2 block, little-endian: the 24-byte 3DES key (odd parity in every byte), a zero byte, 37 non-zero padding bytes, `02 00` |
| 124 | 4 | zeros |
| 128 | 16 | the identifier again |

The key is therefore stored in the clear (as an exponent-one `SIMPLEBLOB` export would leave
it), which is why a backup opens on any machine. BIcraft writes its own random-looking key
derived from the model name and the clock (`bicraft_abf::database_key`): Desktop opens such
images. The key protects only secrets the engine would store in the database (data source
passwords); a model built by BIcraft has none.

## `metadata.sqlitedb` of a new database (observed)

- Page size 4096, UTF-8, `user_version` 0, `schema_version` 69: `DBPROPERTIES` (`[NAME] STRING`,
  `[VALUE] INTEGER`, `PRIMARY KEY ([NAME] ASC)`, hence `sqlite_autoindex_DBPROPERTIES_1`) then
  the 68 TMSCHEMA tables in a fixed order, each created as
  `CREATE TABLE [T]( [ID] INTEGER ,  [Col] TYPE ,  … ,  PRIMARY KEY("ID" ASC) )`. Column lists:
  [`abf.md`](abf.md), "metadata.sqlitedb schema"; order and text in
  `crates/metadata/src/fresh.rs` (`SCHEMA`).
- `DBPROPERTIES`: `MAXID` (the largest ID), `SCHEMAVERSION` and `LASTSCHEMAVERSION` 123,
  `DATABASEBLOCKINGSTATE` 0, `RESTOREABFSOURCE` 0, `LASTCUBEDATACHANGE`, `LASTCUBESCHEMACHANGE`,
  `DATASOURCEVERSION`, `LASTCOMMITTED`, `LASTMETADATACHANGEFORSYNCCOMMITTED`,
  `LASTPBIPUBLICXMLACOMMITTEDTIME` 0. `DATASOURCEVERSION` repeats the model's
  `DefaultPowerBIDataSourceVersion` code (2 for `powerBI_V3`; then rowid 12 and the commit
  markers at rowids 16–18; 0 with rowid 4 and markers at 15–17 otherwise). Times are FILETIME.
- Only the `Model` row (ID 1): `Name` `Model`, every scalar property (defaults:
  `DefaultMode` 0, `DefaultDataView` 0, `DataSourceDefaultMaxConnections` 10,
  `DisableAutoExists` −1, `MaxParallelismPerRefresh` −1, the rest 0 or NULL),
  `DataAccessOptions` as JSON indented with two spaces. No culture and no annotation unless the
  model has them (the structural writer adds those).

## Tables without rows (observed)

A calculated table that returns no row, processed by the engine: one `.idf` / `.idfmeta` pair per
column (row number included) and nothing else. No `.dictionary` file (`DictionaryStorage`
`StorageFileID` 0 with a non-zero `Size`), no `H$` system table: every attribute hierarchy has
`MaterializationType` 2, `DistinctDataCount` 0, no minimum or maximum, `ColumnPositionToData` /
`ColumnDataToPosition` −1; `SegmentMapStorage` type 3 with 0 records. The engine's consistency
check (DBCC on `ImageLoad`) refuses a zero-row column given a dictionary file and an `H$` table,
so the structural writer now writes the observed form for every column of a table without rows
(`crates/pbix/src/structural.rs`).

An import (M) table that was never processed looks different (partition `State` 3, files of data
version 0, no attribute hierarchy data); BIcraft does not write that form.

## Checks

- **BIcraft's reader** (Windows and Linux): `crates/pbix/tests/fresh.rs` (two tables, every data
  type, blanks, relationship, measure, role: model and values read back; reproducible bytes; an
  empty model), `crates/engine/tests/files/pbix_new_tests.rs` (a new model with an M literal table; a
  `model.bim` with four tables — `#table` literal, a CSV under `data_root`, a `CALENDAR` date
  table with a calculated column, a `FILTER` calculated table —, a calculated column, two
  relationships, a user hierarchy, three measures, an RLS role, a parameter in a query group,
  text/int/double/decimal/date/bool columns and blanks; PBIT → PBIX and PBIP without cache →
  PBIX; an external source refused, then written empty and its query kept).
- **Engine** (Analysis Services engine of Desktop 2.158, through the maintainers' development
  oracles):
  every image loads (DBCC passes) and DAX queries return the expected values: row counts of every
  table, every row of `Product`, `Sales` and the `FILTER` table, the three measures,
  `SUMMARIZECOLUMNS` across both relationships, the calendar range, and the RLS role (2 of 4
  products, 3 of 4 sales); the empty table counts no row.
- **Engine schema**: the engine's TMSL script of the loaded image lists the four tables, the
  user hierarchy, both relationships, the role, the parameter and compatibility level 1606.
- **Desktop 2.158**: see `pbix-package.md` §10, "Built from scratch".

## Refresh of an existing PBIX (M11.18)

Status: implemented 2026-11-06 (`model.refresh` on a PBIX, `crates/engine/src/refresh_pbix.rs`).
Opened in Desktop 2.160.1151.0 on 2026-11-25: refreshed tables of up to 1,048,576 rows open with
their new data; a table of more rows (more than one segment) is refused by Desktop's consistency
check, whether refreshed or built from scratch (checklist below, "Outcome").

`model.refresh` writes new rows into an existing `DataModel` with the same writers as a structural
save: no new format knowledge, so the sources are those of this note, of
[`metadata-sqlitedb.md`](metadata-sqlitedb.md) ("Writing", "Refresh state") and of
[`pbix-package.md`](pbix-package.md) §8.

```text
stored model ──► plan          the tables to refresh, and what reads them (bicraft_dax::deps)
             ──► gather        M partitions (Power Query evaluator), calculated tables and
                               columns (DAX evaluator); stored data columns of tables that
                               only get calculated columns recomputed
             ──► DataModelEditor::write_refreshed   each refreshed table replaced in two
                               stages (pbix-package.md §8, step 4), other storage files kept
             ──► temporary file ──► reopened and checked ──► renamed over the file
```

- **Replaced, not patched.** A refreshed table is written as a replaced table: removed with the
  relationships, role permissions and perspective memberships that touch it, then added back with
  its definition and the new rows. Its storage files, `H$` tables and its relationships' `R$`
  indexes are new; every other storage file of the backup is copied as it was (checked byte for
  byte by `refreshing_one_table_keeps_the_others_byte_for_byte`).
- **Refresh state.** The new partitions get `State` 1, `RefreshedTime` = `ModifiedTime` = the
  refresh's start time (the same instant M's clock functions read), `LASTCUBEDATACHANGE` raised
  to it; the untouched tables keep their times ("Refresh state" in
  [`metadata-sqlitedb.md`](metadata-sqlitedb.md)).
- **Checks before the file is replaced.** The new part decompresses to the written backup
  (`verify_datamodel`); the temporary file reopens with BIcraft's reader; its model has no
  structural difference from the refreshed one (`bicraft_pbix::structural::diff` empty); every
  refreshed table reads back its new row count.

### Maintainers' checklist: Desktop opens a refreshed PBIX (black box)

Not available in CI (Windows and Desktop only). With Power BI Desktop 2.159 (Store edition), on
our own synthetic files only (never client files), and without timing anything:

1. Build a PBIX with BIcraft from CSV and Excel sources (for example the fixture of
   `crates/engine/src/refresh_pbix_tests.rs`: `BICRAFT_FRESH_OUT` is not wired there, so run
   `bicraft-cli convert model.bim shop.pbix --data-root DIR` on a copy of its `model.bim`).
2. Open it in Desktop, note the row counts (`EVALUATE ROW("n", COUNTROWS(T))` in DAX query
   view) and close it without saving.
3. Change the sources (new rows, a changed value, a new key on the "one" side), then run
   `bicraft-cli refresh shop.pbix --data-root DIR` and `bicraft-cli refresh shop.pbix --data-root DIR
   --table Product`.
4. Open the file in Desktop: it opens without a repair prompt; the row counts, a measure over a
   calculated column, a relationship (`RELATED`) and the RLS role (View as) show the new data;
   Model view › table › Properties shows the refresh time; Desktop's own Refresh then succeeds and
   the file saves.
5. Record the Desktop version and the outcome in this section (pass or the error text), never a
   duration.

**Outcome, Power BI Desktop 2.160.1151.0 (Store edition, French UI, 2026-11-25).** A model built
by `bicraft-cli run` from two CSV files under `data_root` (Product, 40 rows; Lines, 1,000 to
1,200,000 rows), a relationship, a calculated column `Lines[Qty] * RELATED(Product[Price])`,
three measures, an RLS role and a page of cards, a column chart and a table; the sources then
changed (a 41st product, new prices, more rows) and `bicraft-cli refresh` run.

- **Pass up to 1,048,576 rows a table.** Refreshed to 1,200 rows (both tables), refreshed with
  unchanged sources, refreshed with `--table Product` (Lines, which reads Product through its
  calculated column, refreshed with it) and refreshed from 1,000,000 to 1,000,000 changed rows:
  each opened without a message and showed the new data, the same as `bicraft-cli dax-query` on the
  file (1,000,000 rows, revenue 17,890,218.50, 41 products; per category, for example Bakery
  243,903 rows and 3,902,445.90). On the 1,000,000-row file Desktop's own Refresh (Schéma et
  données) then succeeded and the file saved.
- **Fail above 1,048,576 rows.** Refreshed to 1,200,000 rows, the file does not open: Desktop
  reports that its database consistency check (DBCC) failed on the data segments while loading
  the tables' data. The refresh is not the cause: a PBIX `bicraft` builds from
  scratch fails the same way with one table of 1,048,577 rows (one int64 column of 9 values, from
  a CSV), and opens with 1,048,576. BIcraft writes such a table as two segments of 2^20 and 1
  rows. Desktop, refreshing the same file to 1,048,577 rows, wrote one segment of 1,048,577 rows
  (`RecordsPerSegment` 1,048,577); refreshed to 10,000,000 rows, ten segments of 2^20 rows (the
  last 562,816), each sorted so the column is run-length encoded, which BIcraft's statistics
  read the same way. Which part of BIcraft's multi-segment output the check refuses is not known
  yet (G50).
- Not checked: the RLS role through View as, Model view's refresh time, Excel sources.
