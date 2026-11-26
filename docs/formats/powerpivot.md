# Excel Power Pivot workbooks (import)

Sources: [MS-XLDM] (the spreadsheet data model: the `xl/model/item.data` part, the backup
header, CRC end markers, the XMObject storage trees of §2.5), [MS-XCA] §2.3 and §2.4 (the Plain
LZ77 compression, "xpress8"), [MS-SSAS] and the Analysis Services Scripting Language reference
on Microsoft Learn (the `Load` documents of `Database`, `Cube`, `Dimension`, `Partition` and
`MdxScript`); the open-source description of the same files in DashCraft
(<https://github.com/Jxck-S/dashcraft>, MIT OR Apache-2.0, commit 0958fd82,
`crates/pbi-vertipaq/src/xpress8.rs` and `crates/pbi-vertipaq/src/metadata/xml.rs`); black-box
reading of a workbook Microsoft Excel 16.0 (build 16.0.20625.20108) saved from our own synthetic
tables through its public COM object model (2026-11-23, the fixture
`crates/powerpivot/tests/fixtures/excel/`, its `SOURCE.txt`). Facts marked *(Excel 16.0.20625)*
were observed in that workbook; the others are our reading of the sources and are exercised only
by synthetic workbooks written by BIcraft's own code.

BIcraft imports such a workbook (`file.open` of `.xlsx` / `.xlsm`, M11.33): the model and the
values become a new Power BI model held in memory (`crates/engine/src/workbook_import.rs`),
saved with `file.save_as` / `file.convert` as PBIX, PBIP, PBIT or model.bim. The workbook is never
written. The reader is `crates/powerpivot`; the decompressor is `crates/xpress8`.

## Package

An `.xlsx` / `.xlsm` is an OPC package. The data model is the part `/xl/model/item.data`. A
package with `xl/workbook.xml` and no `item.data` has no data model: `file.open` says so ("this
Excel workbook has no Power Pivot data model"). The part is the backup of an Analysis Services
database, either a bare ABF or one wrapped in the XPress9 stream Power BI uses
([xpress9.md](xpress9.md)); BIcraft accepts both and checks the ABF signature after
decompression. *(Excel 16.0.20625)*: a bare ABF (sync version 150 in the header, `11.53` in the
BackupLog), stored with ZIP compression.

Excel finds the model through the workbook, not only through the part: `xl/workbook.xml`
carries an `x15:dataModel` extension (`modelTables`, `modelRelationships`) and
`xl/connections.xml` the `ThisWorkbookDataModel` connection plus one connection per table
(a worksheet table: `type="102"`, `x15:rangePr sourceName="_xlcn.WorksheetConnection_<table>"`, a
hidden defined name `<table>[]`) *(Excel 16.0.20625)*. BIcraft's reader ignores them. The
synthetic workbook of `bicraft_powerpivot::synthetic` has none, so Excel opens it with an empty
data model: it exercises the reader only.

## Backup (ABF)

The layout is the one of [abf.md](abf.md) with two header flags Power BI leaves false:

| Header flag | Meaning for the reader |
|---|---|
| `ErrorCode` true | every stored file is followed by a 4-byte CRC end marker ([MS-XLDM] §2.1.2.2.1.1): an MSB-first CRC-32 (polynomial 0x04C11DB7, initial value 0xFFFFFFFF, **final XOR 0xFFFFFFFF**, i.e. CRC-32/BZIP2) of the stored bytes before it (the compressed chunks for a compressed file), written little-endian *(Excel 16.0.20625: all 81 markers)*. The virtual directory `Size` includes the marker; for the files the BackupLog does not describe (`PARTITIONS`, the BackupLog itself) that is also their size. The reader drops the marker and does not verify it. |
| `ApplyCompression` true | each file the BackupLog describes is compressed on its own with Plain LZ77; the BackupLog `Size` is the **decompressed** size, so it may exceed the stored range. The header page, the virtual directory and the BackupLog itself stay uncompressed (the reader needs them to find anything). |

A compressed file is a sequence of chunks: a little-endian u16 decompressed size, a u16
compressed size, then the payload; a chunk whose two sizes are equal is stored as is. Each
chunk decompresses on its own (Plain LZ77 of [MS-XCA] §2.3/§2.4: 32-bit flag words read most
significant bit first, a 16-bit match token whose high 13 bits are the offset minus one and
whose low 3 bits are the length, with the shared-nibble, byte, u16 and u32 length escapes; the
final length is the sum plus 3). The reader caps every decompressed file
(`powerpivot::Limits::max_file_bytes`) and checks the total against the BackupLog size.

Backups whose model is stored as `metadata.sqlitedb` (the tabular metadata of a newer Excel)
are refused with a clear error for now (G100). *(Excel 16.0.20625)* still writes the ASSL
documents of compatibility level 1103 and no `metadata.sqlitedb` for a model built through the
object model; no workbook with that file has been seen.

*(Excel 16.0.20625)* What the backup records about the machine: the BackupLog's `ServerRoot` and
every `Path` are under the engine's working folder, `%TEMP%\{<guid>}\VertiPaq_<hex>` of the Excel
process, so a user's temporary folder appears there; the `PARTITIONS` file (UTF-16 XML, one
`Partition` per table with `ObjectPath` `<computer name>\LocalCube.<db id>.Model.<table>.<table>`,
`Name`, `DataSize`) holds the computer name. Neither is read by BIcraft.

*(Excel 16.0.20625)* File layout under the BackupLog's paths: `<db id>.0.db.xml` sits next to the
`<db id>.0.db` folder (the file group's `PersistLocationPath`), which holds
`Model.<n>.cub.xml`, the `<table>.<n>.dim.xml` dimensions with their `<table>.<n>.dim` folders
(storage) and `Model.<n>.cub\` with the measure groups `<table>.<n>.det.xml`, their partitions
`<table>.<n>.det\<table>.<n>.prt.xml` and `MdxScript.<n>.scr.xml`. A file whose name starts with
its group folder's name (`….0.db.xml` next to `….0.db`) is outside that folder: BIcraft's ABF
reader takes the whole name (it once kept only `.xml`, which hid the database and measure group
documents).

## Metadata (ASSL `Load` documents)

One XML document per object, named `<ObjectID>.<version>.<kind>.xml` (IDs may contain dots; the
version is the digits before the kind). UTF-8 or UTF-16 with a BOM. BIcraft reads:

| File | What it gives |
|---|---|
| `<db>.<n>.db.xml` | `Database/Name` (the model's name before the engine renames it after the file; `Microsoft_SQLServer_AnalysisServices` with a GUID `ID` *(Excel 16.0.20625)*) and `CompatibilityLevel` (1100, 1103; `ddl200:CompatibilityLevel` 1103 *(Excel 16.0.20625)*) |
| `Model.<n>.cub.xml` (else the first `.cub.xml`) | `Cube/Dimensions/Dimension`: the tables in order, by `DimensionID` (or `ID`), with `Name` and `Visible` |
| `<dim>.<n>.dim.xml` | `Dimension`: `ID`, `Name`, `Description`; `Attributes/Attribute` (the columns); `Hierarchies/Hierarchy` (user hierarchies); `Relationships/Relationship` |
| `<id>.<n>.prt.xml` | `ParentObject/MeasureGroupID` (the table) and `Partition/Source/QueryDefinition` (its source query) |
| `MdxScript.<n>.scr.xml` | `Command/Text`: the `CREATE MEASURE 'Table'[Name] = <DAX>;` statements; `CalculationProperties/CalculationProperty`: each measure's properties (below) |

An `Attribute` is a column: `ID`, `Name` (default the ID), `Description`, `FormatString`,
`AttributeHierarchyVisible` (false = hidden), `AttributeHierarchyDisplayFolder`, and
`OrderBy` `AttributeKey` with `OrderByAttributeID` (sort by column). Its first
`KeyColumns/KeyColumn` gives `DataType` (`WChar` text, `BigInt` and the other integers whole
number, `Double` decimal number, `Currency` fixed decimal, `Date` date/time, `Boolean`,
`Binary`; `Empty` for a calculated column) and a `Source` whose `xsi:type` is
`ColumnBinding` (`ColumnID`: the source column, XML-encoded as `Unit_x0020_price`),
`ExpressionBinding` (`Expression`: a calculated column's DAX) or `RowNumberBinding`. The
attribute of `Type` `RowNumber` is the hidden row-number column and is not a model column.

A `Hierarchy` has `Name`, `Description`, `Visible`, `DisplayFolder` and `Levels/Level`
(`Name`, `SourceAttributeID`); BIcraft keeps it as a TMSL hierarchy.

A `Relationship` has `ID`, `Visible` (false = inactive) and `FromRelationshipEnd` /
`ToRelationshipEnd`, each with `DimensionID`, `Attributes/Attribute/AttributeID` and
`Multiplicity` (`One` / `Many`). The from end is the many side of an ordinary relationship.
BIcraft writes a cardinality only when the XML states it.

*(Excel 16.0.20625)* The MDX script's first command is `CALCULATE;` and the hidden placeholder
`[__No measures defined]`; then one command per measure, a comment banner ("PowerPivot measures
command (do not modify manually)") and `CREATE MEASURE 'Sales'[Revenue]=<DAX>;` with
annotations `FullName` and `Table`. The DAX is stored with commas as separators (a French
Excel's object model shows the same formula with `;`). Each measure has a `CalculationProperty`:

| Element | What BIcraft takes |
|---|---|
| `CalculationReference` | `[<measure name>]`; the last bracketed name (`]]` is `]`) matches the measure |
| `FormatString` | an MDX string literal: `'#,0'` (whole number, thousands separator), `'#,0.00'` (two decimals), `''` for General (no format string); the quotes are removed, doubled quotes undone |
| `Description` | the measure's description (empty: none) |
| `DisplayFolder` | its display folder (empty: none) |
| `Visible` | `false`: a hidden measure (present only on the placeholder in the workbook seen) |

The `Format` annotation (`<Format Format="NumberDecimal" Accuracy="2" ThousandSeparator="True"/>`)
repeats Excel's format choice and is not read. The model's culture (`Language` 1036 in the
workbook seen, the Excel UI language) is not read yet (G100).

*(Excel 16.0.20625)* Relationships: `ID` a GUID, the from end `Many` and the to end `One`, both
stated (`ddl300_300:Multiplicity`); BIcraft keeps the default cardinality (many to one) without
writing it.

## Storage (XMObject trees)

Each table's storage is `<dim>.<n>.tbl.xml`: an `XMObject` tree ([MS-XLDM] §2.5) whose
`XMSimpleTable` has a `Columns` collection of `XMRawColumn` objects, one per attribute, named
by the attribute ID. An `XMRawColumn` has:

- `Segments`: `XMColumnSegment` objects with `Records`, a `CompressionInfo` member
  (`XMRENoSplitCompressionInfo<width>`, `XM123CompressionInfo` or
  `XMHybridRLECompressionInfo<inner>` with an `RLECompression` member: `BookmarkBits`,
  `StorageAllocSize`, `StorageUsedSize`, `SegmentNeedsResizing`, and the inner class as a
  `SubCompression` member or as the template argument), a `ColumnSegmentStats` member
  (`DistinctStates`, `MinDataID`, `MaxDataID`, `OriginalMinSegmentDataID`, `RLESortOrder`,
  `RowCount`, `HasNulls`, `RLERuns`, `OthersRLERuns`) and an optional `SubSegment` (`Records`).
  These are the fields of a Power BI `.idfmeta` ([vertipaq.md](vertipaq.md)), so the decoder is
  the one Power BI files use.
- `DataObjects`: an `XMHashDataDictionary<XM_String|XM_Long|XM_Real>` whose `name` attribute is
  the `.dictionary` file (none for a column of blanks), or an
  `XMValueDataDictionary<…>` with `BaseId` and `Magnitude` (value = (data id + BaseId) /
  Magnitude); and one `XMRawColumnPartitionDataObject` per partition, whose `name` is the
  `.idf` file and whose `SegmentCount` is how many of the column's segments it holds, in order.

The model type of a column comes from its key column `DataType`; a calculated column gets the
type of its storage (text, whole number or decimal number). Every column of a table must have the
same number of rows. *(Excel 16.0.20625)*: every column of the three tables seen decodes to the
values put in (worksheet numbers with decimals are `Double`, whole numbers `BigInt`, cells
formatted as dates `Date`); attribute hierarchy (`H$…`) and relationship (`R$…`) storage
files are present and not read. The reader caps the cells of the whole model
(`powerpivot::Limits::max_cells`) and XML documents by size and node count, and refuses DTDs.

## Mapping to the Power BI model

- One table per cube dimension, in cube order; hidden when the dimension is not visible.
- Each table gets one import partition in M: the Power Pivot source query as a comment (a
  table linked to a worksheet range has none: its partition has no `Source`, Excel pushes the
  rows *(Excel 16.0.20625)*), then
  `error "This table was imported from an Excel Power Pivot workbook: its source is not
  translated. Replace this query to refresh the table."`. The data is the imported values; a
  refresh fails clearly until the query is replaced. Workbook connections are not translated
  (G100).
- The values are written into a new `DataModel` exactly as a model created in BIcraft
  ([datamodel-fresh.md](datamodel-fresh.md)), with a blank one-page PBIR report.
- A table whose storage cannot be read gives a warning in `file.open`'s `imported.warnings` and
  empty columns; a limit is an error.

## Open questions

1. Whether other Excel versions write an XPress9-wrapped `item.data`; Excel 16.0.20625 writes a
   bare ABF, and both are read.
2. Answered *(Excel 16.0.20625)*: a compressed file's CRC marker follows its compressed bytes
   and covers them, with the final XOR (above).
3. Partly answered: format strings, descriptions and display folders are in
   `CalculationProperties` (above). KPIs, measures Excel hides, calculated columns, hierarchies,
   display folders and hidden columns were not in the workbook seen: Excel's object model
   cannot create them (they need the Power Pivot window), so the ASSL of those comes from the
   sources only.
4. Workbooks saved by Excel with the tabular metadata (`metadata.sqlitedb` inside the backup):
   none seen yet.
5. How to turn a worksheet-linked table (a `WORKSHEET` connection to a table of the same
   workbook) into a refreshable query; such tables are imported as values only (G100).
