# ABF container (as implemented in `crates/abf`)

Status: implemented reader, 2026-10-13 (milestone M4.4, container half); implemented writer
(replace files, byte-exact identity), 2026-10-13 (milestone M5); add and remove files,
2026-10-13 (milestone M5.4). Companion to
[`datamodel-abf.md`](datamodel-abf.md) (research and sources) and [`xpress9.md`](xpress9.md)
(the compression wrapped around the ABF in a PBIX).

**Sources.** [MS-XLDM] §2.1 "Storage Format of the Stream" and subsections (rev 8.3,
2025-08-19, [landing page][xldm]), summarized, not copied; the CC0 Kaitai specs of
Hugoberry/pbixray ([`abf_xpress9.ksy`][ksy-x9]) for the outer framing; the reader algorithm of
pbixray's MIT `abf/parser.py` (read for understanding only, no code reused). Every Power BI fact
below is an **observation** on two Desktop-saved backups of the same model (a PBIX `DataModel`
and a PBIP `.pbi/cache.abf`, saved by the Desktop installed here, Store 2.158.1304.0), decompressed with a
dev-only oracle. Those files are held locally and never committed: this note records structure
only (names of XML elements, file kinds, counts), never names or content from the model.

The input to `Abf::parse` is the **decompressed** stream. Passing the XPress9-compressed part
(it starts with a UTF-16LE banner) returns `Error::Xpress9Compressed`.

---

## Layout

```
offset 0      header page (4096 bytes)
                FF FE                                   BOM (§2.1.1.1)
                35 UTF-16LE code units                  stream storage signature (§2.1.1.2)
                BackupLog XML, UTF-16LE, no BOM         BackupLogHeaderType (§2.1.1.3)
                zero padding to 4096
offset 4096   files section: files back to back, no gaps (§2.1.2)
                [Excel: each file followed by a CRC end marker]
                last file = BackupLog XML (§2.1.2.3)
                zero padding to the next page
page aligned  VirtualDirectory XML (§2.1.3), no BOM
                zero padding to the next page = end of stream
```

### Header page

The XML starts at byte 72 and ends at the first UTF-16 NUL code unit (or the page end).
Fields the reader uses, all optional unless marked:

| Element | Use |
|---|---|
| `BackupRestoreSyncVersion` | reported (observed **140** in Power BI, the same value [MS-XLDM] gives for Excel) |
| `ErrorCode` | `true` means CRC end markers follow each file (observed **false** in Power BI) |
| `EncryptionFlag` | `true` → `Error::Unsupported` |
| `ApplyCompression` | Excel "xpress8": each file the BackupLog describes is compressed on its own with Plain LZ77 and its `Size` is the decompressed size; such entries are marked `compressed`, `data()` gives `None` and `stored()` the stored bytes ([powerpivot.md](powerpivot.md)). Observed **false** in Power BI. If set and the directory does not parse, the reader reports `Unsupported` |
| `m_cbOffsetHeader` (**required**) | absolute offset of the VirtualDirectory (page aligned) |
| `DataSize` (**required**) | byte length of the VirtualDirectory XML (padding excluded) |
| `Files` | entry count; reported, not enforced (observed equal to the directory count) |
| `ObjectID`, `m_cbOffsetData` | reported (`m_cbOffsetData` observed 4096) |

Also present and ignored: `Fault`, `faultcode`, `EncryptionKey`.

### VirtualDirectory

Root `VirtualDirectory`, one `BackupFile` per stored file, in stream order:
`Path` (storage name), `Size`, `m_cbOffsetHeader` (absolute offset), `Delete`,
`CreatedTimestamp`, `Access`, `LastWriteTime` (FILETIME ticks since 1601).

Observed in Power BI: **UTF-8, no BOM, no XML declaration**. The reader also accepts UTF-16LE
(with or without BOM) because [MS-XLDM] does not state the encoding.

### Files section (Power BI flavor)

Observed order of directory entries:

| # | Storage name | Content |
|---|---|---|
| 1 | `ADDITIONAL_LOG` | BOM + UTF-16LE `<Property>` XML (a product name). **Not in [MS-XLDM].** |
| 2 | `PARTITIONS` | BOM + UTF-16LE `<Partitions/>` (empty; §2.1.2.1) |
| 3…n−1 | 20 uppercase hex digits | the database files, in their native bytes |
| n | `LOG` | BOM + UTF-16LE `BackupLog` XML (§2.1.2.3) |

Each BOM belongs to its file: the directory `Size` of an XML entry includes it. [MS-XLDM]
describes the BOMs as section framing instead, and has no `ADDITIONAL_LOG`.

With `ErrorCode` false there are **no CRC markers**: directory offsets are contiguous
(`offset[i] + Size[i] == offset[i+1]`), and the directory `Size` equals the BackupLog `Size`
for every file. With `ErrorCode` true (Excel), the directory `Size` includes the marker and
the BackupLog `Size` excludes it; the reader exposes both (`stored_size`, `size`) and returns
only the payload. The marker's width and coverage are still unconfirmed (no Excel sample yet),
so CRCs are not verified.

### BackupLog (files-section)

Root `BackupLog`. Observed children, in order: `BackupRestoreSyncVersion`, `ServerRoot`,
`SvrEncryptPwdFlag`, `ServerEnableBinaryXML`, `ServerEnableCompression`, `CompressionFlag`,
`EncryptionFlag`, `ObjectName`, `ObjectId`, `Write`, `OlapInfo`, `IsTabular`, `Collations`,
`Languages`, `FileGroups`.

- `BackupRestoreSyncVersion` is a **dotted string** in Power BI (`11.53`), where [MS-XLDM]
  gives the integer 1153 for Excel. The reader keeps it as a string.
- `IsTabular` (true) and `ServerEnableBinaryXML`/`ServerEnableCompression` are not in the
  Excel description. `Write` is `ReadWrite`. `Collations` is empty.
- `FileGroups` → `FileGroup` (`Class`, `ID`, `Name`, `ObjectVersion`, `PersistLocation`,
  `PersistLocationPath`, `StorageLocationPath`, `ObjectID`, `FileList`) → `FileList` →
  `BackupFile` (`Path`, `StoragePath`, `LastWriteTime`, `Size`).
- Two groups observed:
  - `Class` **100002**, the database object: 2 files (the database `.db.xml` and `CryptKey.bin`).
  - `Class` **100069**: 305 files (`metadata.sqlitedb` and all VertiPaq files).
- `Path` is the **absolute path** on the machine that saved the file, in the Analysis Services
  workspace folder (`\\?\` prefix). It contains the local user profile directory, so
  `FileEntry::original_path` must never be logged. `PersistLocationPath` is the database
  folder (`<db-guid>.<version>.db`) in both groups.

### Reader algorithm

1. Check the BOM and signature (or detect the XPress9 banner); parse the header XML.
2. Slice and parse the VirtualDirectory. Every entry is bounds-checked; duplicate storage
   names (ASCII case-insensitive) are an error.
3. The BackupLog is the entry named `LOG`, else the last entry ([MS-XLDM]: the log is the
   last file). Parse it.
4. Join each BackupLog `BackupFile` to the directory by `StoragePath`. Records with no
   directory entry (or a repeated one) are skipped and counted in
   `Summary::unmatched_log_records`. A log `Size` larger than the directory `Size` is an error.
5. Logical path = log `Path` minus its group's `PersistLocationPath` (ASCII case-insensitive),
   with `/` separators; outside that folder, just the file name. Entries the log does not
   describe (`ADDITIONAL_LOG`, `PARTITIONS`, `LOG`) keep their storage name.

API: `Abf::parse`, `files()` (stream order), `entry`/`read` (by logical path, case-insensitive,
`\` or `/`, or by storage name), `data(&FileEntry)`, `metadata_sqlitedb()` (by file name,
case-insensitive), `header()`, `backup_log()`, `summary()` (counts only, no names). Data are
zero-copy slices of the input. The tiny XML reader (`xml.rs`) skips attributes, namespaces,
comments, PIs and DOCTYPE, decodes the predefined and numeric entities, caps nesting at 64 and
ignores everything after the root element (padding, CRC markers).

---

## Writer

`Abf::to_writer()` (or `AbfWriter::new(&abf)`) holds every file of a parsed backup, borrowed
from the input. `replace(path, bytes)` swaps one file's content (path resolved as in
`Abf::entry`, except that a path matching several files is `Error::AmbiguousPath` instead of
the first match; unknown paths are `Error::FileNotFound`). `replace_metadata_sqlitedb(bytes)`
is the shortcut for the M5 edit loop. `remove(path)` drops a file and `add(path, bytes)`
stores a new one (see "Adding and removing files"). `finish() -> Vec<u8>` writes the stream.

### Patching, not re-serialization

The three XML documents are **patched in place**: the writer keeps each one's original text,
records the byte range of every element's content while parsing (`xml::Element::content`),
and rewrites only the numbers that the layout changes. Everything else (element order,
unknown elements, the server paths, timestamps, escaping) is carried over untouched. An
element whose value did not change is not rewritten, and a document with no change is copied
byte for byte.

| Document | Elements rewritten |
|---|---|
| header `BackupLog` (UTF-16LE, no BOM, at byte 72) | `m_cbOffsetHeader`, `DataSize`, `Files`, `m_cbOffsetData` (4096) |
| `VirtualDirectory` (original encoding kept) | `Size` and `m_cbOffsetHeader` of every `BackupFile` |
| files-section `BackupLog` (BOM + UTF-16LE) | `Size` of each `BackupFile` record whose `StoragePath` names a replaced file; records of removed files cut out, records of added files inserted |
| `VirtualDirectory` (removed / added files) | the whole `<BackupFile>` entry cut out / inserted |

A document is re-encoded in the encoding it was read in (UTF-8 or UTF-16LE, with the BOM it
had). If decoding and re-encoding the original does not give back the same bytes (an unpaired
surrogate, an odd trailing byte), the document is still copied when unchanged, but patching
it is `Error::Unsupported` rather than a silent change.

### Layout written

The Power BI layout observed on both real backups, reproduced exactly:

1. Header page: the original BOM and signature (bytes 0–71), the header XML, zero padding to
   4096. An unchanged header page is copied as is.
2. Files back to back from offset 4096, **in the original virtual-directory order** (Power BI:
   `ADDITIONAL_LOG`, `PARTITIONS`, the database files, `LOG` last), with no gaps. Removed files
   are left out; added files go just before `metadata.sqlitedb` (see below).
3. Zero padding to the next page boundary, then the virtual directory (`DataSize` = its
   encoded length, padding excluded), then zero padding to the next page boundary, which is
   the end of the stream.

Every offset and size is computed with checked arithmetic (`Error::Write` on overflow). The
output is parsed back before `finish` returns it.

A canonical input (contiguous files from 4096, zero padding, full final page) comes out
**byte-identical** when nothing is replaced: checked on both real backups of the sample
model (the PBIX `DataModel` and the PBIP `cache.abf`) and on every synthetic flavor. A
non-canonical input (gaps, non-zero padding, a truncated last page, out-of-order offsets)
is normalized to the layout above instead.

### Decisions

- **Timestamps are kept.** The directory's `CreatedTimestamp`/`Access`/`LastWriteTime` and the
  BackupLog's `LastWriteTime` keep their original values for replaced files too, so the output
  depends only on the input and the replacements (byte-stable on every platform, no clock in
  an L0 crate). Observed (both real backups): the BackupLog value looks like each file's write
  time in the server's workspace folder, and the directory `LastWriteTime` is always later
  (typically 1–7 ms, i.e. the time the file was copied into the backup). The three directory
  values are equal on all entries but one, whose `CreatedTimestamp` is ~14 ms earlier. One
  BackupLog record carries a tiny value (about 2 minutes after 1601), so the field is not
  always a real date. A restore does not check them: replaced files keep their times and load
  (M5), and so does a backup whose `DBPROPERTIES` markers and row times are all moved to 1970
  or to 2174 (G17, 2026-10-15: engine `ImageLoad` of a PBIX BIcraft built, the experiment of
  `xpress9.md` §6.2, case `markers`).
- **Add and remove** are container operations only: the `StorageFolder`/`StorageFile` rows of
  `metadata.sqlitedb` that name the files are written by `bicraft-metadata`
  (`write_structural_sqlitedb` returns the matching `StoragePlan`), and the caller applies both
  in one save.
- **The BackupLog is generated:** replacing `LOG` is `Error::Unsupported`. `ADDITIONAL_LOG`
  and `PARTITIONS` can be replaced like any file (they have no BackupLog record).
- **CRC end markers** (`ErrorCode` true, Excel): identity works (stored bytes, markers
  included, are copied), but `replace` is `Error::Unsupported` because the marker format is
  unconfirmed (open question 1).

### Adding and removing files

Observed on the real backup (structure only):

- **Storage names** are 20 upper-case hex digits whose digit 13 is always `4` and digit 17 is
  one of `8`–`B`: the first 80 bits of a random version-4 GUID, without hyphens. Nothing links a
  name to its path except the BackupLog record. The writer derives a new file's name from its
  logical path (FNV-1a + SplitMix64, salted until it is unused), shaped the same way, so the
  output is deterministic; names are compared ASCII case-insensitively for uniqueness.
- **Directory order equals BackupLog order** for the database files, and `metadata.sqlitedb` is
  the last database file (just before `LOG`). Desktop lists the other files table by
  table and column by column (`.idf`, `.idfmeta`, `.dictionary`, then the `H$` files), but no order
  rule is needed to read a backup (the reader joins by storage name).
- **BackupLog record** of a database file: `<BackupFile><Path>` absolute path = the group's
  `PersistLocationPath` + `\` + the logical path with `\` separators, `<StoragePath>`,
  `<LastWriteTime>`, `<Size>`. Every VertiPaq file and `metadata.sqlitedb` sit in the file group
  of `Class` 100069. `.tbl`/`.prt` folders have no record of their own.
- **Directory entry**: `Path` (storage name), `Size`, `m_cbOffsetHeader`, `Delete` (false),
  `CreatedTimestamp`, `Access`, `LastWriteTime`.

What the writer does:

| Operation | Effect |
|---|---|
| `remove(path)` | The file's data, its directory entry and every BackupLog record naming it are dropped. Removing an added file cancels the addition. The BackupLog itself cannot be removed. |
| `add(path, bytes)` | `path` is relative to the database folder (`/` or `\`); empty, absolute, `.`/`..` segments, `:` and control characters are refused (`Error::Unsupported`), an existing path is `Error::PathExists` (ASCII case-insensitive). The new directory entry and BackupLog record go **just before `metadata.sqlitedb`** (directory, stream and the record list of its file group), in the order the files were added; without a `metadata.sqlitedb`, before `LOG` and at the end of the last group's `FileList`. |
| Timestamps of added files | Directory: the `Delete`/`CreatedTimestamp`/`Access`/`LastWriteTime` elements of the `metadata.sqlitedb` entry are copied verbatim (no clock in an L0 crate). BackupLog: `LastWriteTime` = the largest one in the group. |
| `replace(path, …)` | Also works on an added file (by its path). |
| Header | `Files` is the new count; the rest as for replacement. |

Backups with CRC end markers (`ErrorCode` true) refuse add and remove like replace. `finish`
parses the output back and checks that every added file reads back at its path with its size.
Checked: synthetic backups (both directory encodings, Unicode and `&` in paths, 300 added
files, corrupted inputs) and the real backup (one table folder's files removed, two files
added: every other file keeps its bytes, the log and directory stay consistent).

### Round trip into a PBIX (dev check)

With the dev-only XPress9 oracle, the rewritten ABF was recompressed in the sequential form
(the 102-byte single-threaded banner copied from the original part, then one
`u32 uncompressed, u32 compressed, payload` chunk per 2 MiB; the sample's original `DataModel`
is that form with a single chunk), verified by decompressing it again, and stored (not
deflated) as `/DataModel` in a copy of the PBIX. Three variants: identity, `metadata.sqlitedb`
replaced by a same-size `VACUUM INTO` copy, and by a copy grown with 15 free pages (+61,440
bytes, which shifts the `LOG` offset and the directory page). Opening them in Desktop is the
pending acceptance check. The local tool `cargo run -p bicraft-abf --example abf_repack --
<in.abf> <out.abf> [<path> <file>]...` does the ABF step.

Large backups ([`vertipaq-large.md`](vertipaq-large.md)): `replace` of 2 271 files in a
113 MB backup and of 78 files (386 MB of new content) in a 578 MB one, re-compressed and loaded
into a local engine instance with AMO `ImageLoad`: accepted, with the original timestamps and storage
names kept.

---

## Power BI vs Excel

| Aspect | Excel ([MS-XLDM]) | Power BI (observed) |
|---|---|---|
| Outer compression | xpress8 (`ApplyCompression` / per-file) | XPress9 around the whole ABF; `ApplyCompression` false inside |
| CRC end markers | yes (`ErrorCode` true) | **none** (`ErrorCode` false) |
| Files section start | BOM + Partitions XML + CRC | `ADDITIONAL_LOG` entry, then `PARTITIONS` entry, each with its own BOM |
| BackupLog sync version | integer 1153 | string `11.53` |
| Directory encoding | not stated | UTF-8, no BOM |
| Metadata | XML per object (`*.tbl.xml`, …) | one SQLite file, `metadata.sqlitedb` |
| Column statistics | `XMColumnSegment` XML | binary `.idfmeta` per `.idf` |
| Storage layout | flat names (`4.Table1.Cat.0.idf`) | folders `<Table> (<id>).tbl/<n>.prt/` |

---

## VertiPaq file inventory (names and kinds only)

Observed in the sample model (310 directory entries): 1 `metadata.sqlitedb`, 1
`CryptKey.bin`, 1 database `.db.xml`, 132 `.idf`, 132 `.idfmeta`, 36 `.dictionary`,
4 `.hidx`, plus `ADDITIONAL_LOG`, `PARTITIONS`, `LOG`. All 132 `.idf` files are segment 0.

**Large models** ([`vertipaq-large.md`](vertipaq-large.md), engine-written backups of our
synthetic models, 113–845 MB, up to 7 235 files): a column partition of several segments is
still **one** `.idf` and one `.idfmeta` (the segments are consecutive blocks of that file);
each partition has its own `<n>.prt` folder and file pair, whose name ends in the partition's
position, `….<p>.idf` (`.0` to `.2` on a 3-partition table, `.0` to `.7` on the 8 policy-range
partitions of an incremental-refresh table, the DirectQuery partition of a hybrid table
included, with an empty segment); the dictionary and `.hidx` stay one per column in the table
folder. A hash-encoded column holding only blanks has no `.dictionary` at all. The reader
parses these backups in under 0.1 s (directory and BackupLog of 7 235 entries).

Logical path shapes (relative to the database folder; `<…>` are placeholders):

| File | Path shape |
|---|---|
| column data | `<T> (<id>).tbl/<n>.prt/<k>.<T> (<id>).<C> (<id>).<p>.idf` (+ `.idfmeta`), `<p>` the partition's position; the row-number column ([MS-XLDM] §2.3.4) is named `RowNumber-<guid> (<id>)` |
| dictionary | `<T> (<id>).tbl/<k>.<T> (<id>).<C> (<id>).dictionary` (table folder, not partition) |
| hash index | `<T> (<id>).tbl/<k>.H$<T> (<id>)$<C> (<id>).hidx` (table folder) |
| attribute hierarchy | `H$<T> (<id>)$<C> (<id>)$(<id>).tbl/<n>.prt/<k>.H$<T> (<id>)$<C> (<id>).{POS_TO_ID,ID_TO_POS}.0.idf` (+ `.idfmeta`) |
| relationship index | `R$<T> (<id>)$<rel-guid> (<id>)$(<id>).tbl/<n>.prt/<k>.R$<T> (<id>)$<rel-guid> (<id>).INDEX.0.idf` (+ `.idfmeta`) |
| user hierarchy | `U$<T> (<id>)$<H> (<id>)$(<id>).tbl/<n>.prt/<k>.U$<T> (<id>)$<H> (<id>).{PARENT_POS,FIRST_CHILD_POS,CHILD_COUNT,MULTI_LEVEL_ID}.0.idf` (+ `.idfmeta`) |

`.tbl` and `.prt` are **folders**, not files. `FileEntry::system_table` flags `H$`/`R$`/`U$`
from any path segment (also after an Excel-style `<n>.` ordinal), so the `.hidx` files count
as `H$`. In the sample: H$ 148 files (72 `.idf`, 72 `.idfmeta`, 4 `.hidx`), R$ 8, U$ 8.

---

## metadata.sqlitedb schema (observed)

SQLite 3, page size 4096, UTF-8, `user_version` 0, journal mode `delete`, no auto-vacuum,
`schema_version` 69. **69 tables** and one index (`sqlite_autoindex_DBPROPERTIES_1`).
Every table except `DBPROPERTIES` has `ID INTEGER PRIMARY KEY`; foreign keys are plain
`INTEGER` columns named `<Object>ID` (no declared constraints). Most objects carry
`ModifiedTime` (and often `StructureModifiedTime`, `RefreshedTime`) as INTEGER, plus
`State`/`ErrorMessage` for computed objects. Columns below omit `ID`. Types: I = INTEGER,
T = TEXT, R = REAL, B = BLOB.

### Model objects (TOM)

| Table | Columns |
|---|---|
| `Model` | Name T, Description T, StorageLocation T, DefaultMode I, DefaultDataView I, Culture T, Collation T, ModifiedTime I, StructureModifiedTime I, DataAccessOptions T, DefaultMeasureID I, DefaultPowerBIDataSourceVersion I, ForceUniqueNames I, DiscourageImplicitMeasures I, DiscourageReportMeasures I, DataSourceVariablesOverrideBehavior I, DataSourceDefaultMaxConnections I, SourceQueryCulture T, MAttributes T, DiscourageCompositeModels I, AutomaticAggregationOptions T, DisableAutoExists I, MaxParallelismPerRefresh I, MaxParallelismPerQuery I, DisableSystemDefaultExpression I, DirectLakeBehavior I, ValueFilterBehavior I, SelectionExpressionBehavior I, MetadataAccessPolicy I, DefaultDirectLakeIndexingBehavior I |
| `Table` | ModelID I, Name T, DataCategory T, Description T, IsHidden I, TableStorageID I, ModifiedTime I, StructureModifiedTime I, SystemFlags I, ShowAsVariationsOnly I, IsPrivate I, DefaultDetailRowsDefinitionID I, AlternateSourcePrecedence I, RefreshPolicyID I, CalculationGroupID I, ExcludeFromModelRefresh I, LineageTag T, SourceLineageTag T, SystemManaged I, ExcludeFromAutomaticAggregations I, DirectLakeIndexingBehavior I |
| `Column` | TableID I, ExplicitName T, InferredName T, ExplicitDataType I, InferredDataType I, DataCategory T, Description T, IsHidden I, State I, IsUnique I, IsKey I, IsNullable I, Alignment I, TableDetailPosition I, IsDefaultLabel I, IsDefaultImage I, SummarizeBy I, ColumnStorageID I, Type I, SourceColumn T, ColumnOriginID I, Expression T, FormatString T, IsAvailableInMDX I, SortByColumnID I, AttributeHierarchyID I, ModifiedTime I, StructureModifiedTime I, RefreshedTime I, SystemFlags I, KeepUniqueRows I, DisplayOrdinal I, ErrorMessage T, SourceProviderType T, DisplayFolder T, EncodingHint I, RelatedColumnDetailsID I, AlternateOfID I, LineageTag T, SourceLineageTag T, ExpressionContext I, StringIndexingBehavior I, FullTextIndexingBehavior I |
| `Measure` | TableID I, Name T, Description T, DataType I, Expression T, FormatString T, IsHidden I, State I, ModifiedTime I, StructureModifiedTime I, KPIID I, IsSimpleMeasure I, ErrorMessage T, DisplayFolder T, DetailRowsDefinitionID I, DataCategory T, FormatStringDefinitionID I, LineageTag T, SourceLineageTag T |
| `Partition` | TableID I, Name T, Description T, DataSourceID I, QueryDefinition T, State I, Type I, PartitionStorageID I, Mode I, DataView I, ModifiedTime I, RefreshedTime I, SystemFlags I, ErrorMessage T, RetainDataTillForceCalculate I, RangeStart R, RangeEnd R, RangeGranularity I, RefreshBookmark T, QueryGroupID I, ExpressionSourceID I, MAttributes T, DataCoverageDefinitionID I, SchemaName T |
| `Relationship` | ModelID I, Name T, IsActive I, Type I, CrossFilteringBehavior I, JoinOnDateBehavior I, RelyOnReferentialIntegrity I, FromTableID I, FromColumnID I, FromCardinality I, ToTableID I, ToColumnID I, ToCardinality I, State I, RelationshipStorageID I, RelationshipStorage2ID I, ModifiedTime I, RefreshedTime I, SecurityFilteringBehavior I |
| `Hierarchy` | TableID I, Name T, Description T, IsHidden I, State I, HierarchyStorageID I, ModifiedTime I, StructureModifiedTime I, RefreshedTime I, DisplayFolder T, HideMembers I, LineageTag T, SourceLineageTag T |
| `Level` | HierarchyID I, Ordinal I, Name T, Description T, ColumnID I, ModifiedTime I, LineageTag T, SourceLineageTag T |
| `Expression` | ModelID I, Name T, Description T, Kind I, Expression T, ModifiedTime I, QueryGroupID I, ParameterValuesColumnID I, MAttributes T, LineageTag T, SourceLineageTag T, RemoteParameterName T, ExpressionSourceID I |
| `Function` | ModelID I, Name T, Description T, Expression T, IsHidden I, State I, ErrorMessage T, ModifiedTime I, StructureModifiedTime I, LineageTag T, SourceLineageTag T |
| `DataSource` | ModelID I, Name T, Description T, Type I, ConnectionString T, ImpersonationMode I, Account T, Password T, MaxConnections I, Isolation I, Timeout I, Provider T, ModifiedTime I, ConnectionDetails T, Options T, Credential T, ContextExpression T |
| `Role` | ModelID I, Name T, Description T, ModelPermission I, ModifiedTime I |
| `RoleMembership` | RoleID I, MemberName T, MemberID T, IdentityProvider T, MemberType I, ModifiedTime I |
| `TablePermission` | RoleID I, TableID I, FilterExpression T, ModifiedTime I, State I, ErrorMessage T, MetadataPermission I |
| `ColumnPermission` | TablePermissionID I, ColumnID I, ModifiedTime I, MetadataPermission I |
| `Perspective` | ModelID I, Name T, Description T, ModifiedTime I |
| `PerspectiveTable` | PerspectiveID I, TableID I, IncludeAll I, ModifiedTime I |
| `PerspectiveColumn` / `PerspectiveMeasure` / `PerspectiveHierarchy` / `PerspectiveSet` | PerspectiveTableID I, ColumnID / MeasureID / HierarchyID / SetID I, ModifiedTime I |
| `Culture` | ModelID I, Name T, LinguisticMetadataID I, ModifiedTime I, StructureModifiedTime I |
| `LinguisticMetadata` | CultureID I, Content T, ModifiedTime I, ContentType I |
| `ObjectTranslation` | CultureID I, ObjectID I, ObjectType I, Property I, Value T, ModifiedTime I, Altered I |
| `Annotation` | ObjectID I, ObjectType I, Name T, Value T, ModifiedTime I |
| `ExtendedProperty` | ObjectID I, ObjectType I, Name T, Type I, Value T, ModifiedTime I |
| `ChangedProperty` | ObjectID I, ObjectType I, Property T |
| `ExcludedArtifact` | ObjectID I, ObjectType I, ArtifactType I, Reference T |
| `KPI` | MeasureID I, Description T, TargetDescription T, TargetExpression T, TargetFormatString T, StatusGraphic T, StatusDescription T, StatusExpression T, TrendGraphic T, TrendDescription T, TrendExpression T, ModifiedTime I |
| `FormatStringDefinition` | ObjectID I, ObjectType I, Expression T, ModifiedTime I, State I, ErrorMessage T |
| `DetailRowsDefinition` | ObjectID I, ObjectType I, Expression T, ModifiedTime I, State I, ErrorMessage T |
| `CalculationGroup` | TableID I, Description T, ModifiedTime I, Precedence I |
| `CalculationItem` | CalculationGroupID I, FormatStringDefinitionID I, Name T, Description T, ModifiedTime I, State I, ErrorMessage T, Expression T, Ordinal I |
| `CalculationExpression` | CalculationGroupID I, FormatStringDefinitionID I, Description T, Expression T, ModifiedTime I, State I, ErrorMessage T, SelectionMode I |
| `Set` | TableID I, Name T, Description T, Expression T, IsDynamic I, IsHidden I, State I, ModifiedTime I, StructureModifiedTime I, ErrorMessage T, DisplayFolder T |
| `Variation` | ColumnID I, Name T, Description T, RelationshipID I, DefaultHierarchyID I, DefaultColumnID I, IsDefault I |
| `AlternateOf` | ColumnID I, BaseColumnID I, BaseTableID I, Summarization I |
| `RelatedColumnDetails` | ColumnID I, ModifiedTime I |
| `GroupByColumn` | RelatedColumnDetailsID I, GroupingColumnID I, ModifiedTime I |
| `QueryGroup` | ModelID I, Folder T, Description T |
| `RefreshPolicy` | TableID I, PolicyType I, RollingWindowGranularity I, RollingWindowPeriods I, IncrementalGranularity I, IncrementalPeriods I, IncrementalPeriodsOffset I, PollingExpression T, SourceExpression T, Mode I |
| `DataCoverageDefinition` | PartitionID I, Description T, Expression T, ModifiedTime I, State I, ErrorMessage T |
| `Calendar` | TableID I, Name T, Description T, LineageTag T, SourceLineageTag T, ModifiedTime I |
| `CalendarColumnGroup` | CalendarID I, TimeUnit I, ModifiedTime I |
| `CalendarColumnReference` | CalendarColumnGroupID I, ColumnID I, IsPrimaryColumn I, ModifiedTime I |
| `BindingInfo` | ModelID I, Name T, Description T, Type I, ConnectionId T |
| `AnalyticsAIMetadata` | ModelID I, Name T, MeasureAnalysisDefinition T |
| `DBPROPERTIES` | NAME STRING (primary key), VALUE INTEGER |

### Storage objects (link the model to ABF files)

| Table | Columns |
|---|---|
| `TableStorage` | TableID I, Name T, Version I, Settings I, RIViolationCount I, StorageFolderID I |
| `PartitionStorage` | PartitionID I, Name T, StoragePosition I, SegmentMapStorageID I, DataObjectId I, StorageFolderID I, DeltaTableMetadataStorageID I |
| `ColumnStorage` | ColumnID I, Name T, StoragePosition I, DictionaryStorageID I, Settings I, ColumnFlags I, Collation T, OrderByColumn T, Locale I, BinaryCharacters I, Statistics_DistinctStates I, Statistics_MinDataID I, Statistics_MaxDataID I, Statistics_OriginalMinSegmentDataID I, Statistics_RLESortOrder I, Statistics_RowCount I, Statistics_HasNulls I, Statistics_RLERuns I, Statistics_OthersRLERuns I, Statistics_Usage I, Statistics_DBType I, Statistics_XMType I, Statistics_CompressionType I, Statistics_CompressionParam I, Statistics_EncodingHint I, IsDeltaPartitionColumn I, DeltaColumnMappingPhysicalName T, DeltaColumnMappingId I, FramedSourceColumn T, DeltaColumnPrimitiveType I, DeltaTableColumnStorageID I, StringIndexStorageID I, ColumnIndexStorageID I, FullTextIndexStorageID I |
| `ColumnPartitionStorage` | ColumnStorageID I, PartitionStorageID I, DataVersion I, State I, SegmentStorageID I, StorageFileID I |
| `SegmentStorage` | ColumnPartitionStorageID I, SegmentCount I, StorageFileID I |
| `SegmentMapStorage` | PartitionStorageID I, Type I, RecordCount I, SegmentCount I, RecordsPerSegment I |
| `GeneralSegmentMapSegmentMetadataStorage` | SegmentMapStorageID I, RecordCount I, Ordinal I |
| `DictionaryStorage` | ColumnStorageID I, Type I, DataType I, DataVersion I, BaseId I, Magnitude R, LastId I, IsNullable I, IsUnique I, IsOperatingOn32 I, DictionaryFlags I, StorageFileID I, Size I |
| `AttributeHierarchy` | ColumnID I, State I, AttributeHierarchyStorageID I, ModifiedTime I, RefreshedTime I |
| `AttributeHierarchyStorage` | AttributeHierarchyID I, SortOrder I, OptimizationLevel I, MaterializationType I, ColumnPositionToData I, ColumnDataToPosition I, DistinctDataCount I, DataVersion I, StorageFileID I, SystemTableID I, HasStatistics I, MinValue T, MaxValue T, StringValueMaxLength I |
| `HierarchyStorage` | HierarchyID I, Name T, LevelDefinition T, MaterializationType I, StructureType I, SystemTableID I |
| `RelationshipStorage` | RelationshipID I, Name T, DefinitionType I, Cardinality I, Flags I, RelationshipIndexStorageID I |
| `RelationshipIndexStorage` | RelationshipStorageID I, IndexType I, Flags I, RecordCount I, SecondaryRecordCount I, StorageFolderID I, StorageFileID I, SystemTableID I, SecondarySystemTableID I |
| `ColumnIndexStorage` | ColumnStorageID I, StorageFileID I, DataVersion I, DeltaVersion I, LastId I, IsNullable I, IsOperatingOn32 I |
| `StringIndexStorage` / `FullTextIndexStorage` | ColumnStorageID I, StorageFileID I, DataVersion I, ImplementationVersion I, SourceDictionaryDataVersion I |
| `StorageFolder` | OwnerID I, OwnerType I, Path T |
| `StorageFile` | OwnerID I, OwnerType I, StorageFolderID I, FileName T |
| `DeltaTableMetadataStorage` | PartitionStorageID I, DataSourceReference T, TableName T, RootLocation T, CurrentVersion T, TableObjectID T, DatamartObjectID T, FramedSchemaName T, FallbackReason I, DeltaColumnMappingMode I, DeltaLogETag T, ArtifactObjectId T, ArtifactSecurityVersion T |
| `DeltaTableColumnStorage` | ColumnStorageID I, UncompressedSize I, MinCardinality I, MaxCardinality I |
| `ParquetFileStorage` | DeltaTableMetadataStorageID I, Size I, Location T, CreatedVersion T, DeletedVersion T, RowgroupCount I, Ordinal I, ETag T, DeletionVectorFileLocation T, DeletionVectorBitmapOffset I, DeletionVectorBitmapSize I, DeletionVectorInlineBitmap B, DeletionVectorFileETag T |

The three Delta/Parquet tables (Direct Lake) were empty in the sample import model.

### How IDs link objects (checked on the sample, counts only)

- `Table.TableStorageID → TableStorage.ID`, `Column.ColumnStorageID → ColumnStorage.ID`,
  `Partition.PartitionStorageID → PartitionStorage.ID`: every row resolves.
- Engine system tables are rows of `Table` too: their names start with `H$`, `R$` or `U$`
  and `SystemFlags` was 1 for them (2 for the user tables seen). A model reader must filter
  them out. `AttributeHierarchyStorage.SystemTableID`, `HierarchyStorage.SystemTableID` and
  `RelationshipIndexStorage.SystemTableID` point to `Table.ID`; 0 means "no system table"
  (seen on attribute hierarchies only).
- `Annotation.ObjectType` uses the TOM object-type numbering (1 Model, 3 Table, 4 Column
  observed), with `ObjectID` the row ID in the matching table. `ExtendedProperty`,
  `ObjectTranslation`, `FormatStringDefinition` and `DetailRowsDefinition` have the same
  `ObjectID`/`ObjectType` pair (not checked against rows).
- **ABF file of a storage object:** `StorageFile.StorageFolderID → StorageFolder.ID`, and
  `StorageFolder.Path + "\" + StorageFile.FileName` equals the ABF logical path
  (`FileEntry::path` with `\`). All 304 `StorageFile` rows of the sample resolved to a
  directory entry; the only class-100069 file without a row is `metadata.sqlitedb` itself.
- `StorageFile.OwnerType` / `OwnerID` name the owning storage row: 22 → `DictionaryStorage`
  (`.dictionary`), 23 → `ColumnPartitionStorage` (`.idf`), 24 → `SegmentStorage`
  (`.idfmeta`), 27 → `AttributeHierarchyStorage` (`.hidx`). The owner's `StorageFileID`
  points back to the same row. `StorageFolder.OwnerType` 18 → `TableStorage` (`.tbl`
  folder), 20 → `PartitionStorage` (`.prt` folder, path `<tbl>\<n>.prt`).

---

## Streamed backups (BIcraft)

A backup too large to hold in memory is indexed from three byte ranges, read in this order
(`bicraft_abf::streamed_header`, `streamed_log_range`, `streamed_index`): the header page (it
gives the virtual directory's offset and size), the virtual directory (it gives the BackupLog's
range: the `LOG` entry, else the last one), and the BackupLog. The result is the same file list
as `Abf::files`, without the data; offsets are checked against the end of the directory, which is
the end of the stream. Encrypted and `ApplyCompression` backups are refused. Offsets are 64-bit
throughout, so files past 4 GiB are addressed normally (`pbix-package.md` §11, "Packages over
4 GiB").

## Open questions

1. **CRC markers**: width (presumably 4 bytes), byte order and coverage are unconfirmed; we
   have no CRC-bearing (Excel) sample. The reader trusts BackupLog `Size` and does not verify.
   For writing Power BI files the question is moot: the engine and Desktop never set
   `ErrorCode` (every image seen), BIcraft writes it false, and the engine honours the flag:
   our backup with `ErrorCode` set to true and no markers fails to load (the BackupLog is then
   misread, an XML error; G17, 2026-10-15). [MS-XLDM] §2.1.2.2.1.1 gives the algorithm (an
   MSB-first CRC-32 with polynomial 0x04C11DB7, initial value 0xFFFFFFFF, no final XOR).
2. `ApplyCompression` / Excel xpress8: BIcraft reads the virtual directory and the BackupLog
   as uncompressed and each logged file as compressed on its own ([powerpivot.md](powerpivot.md),
   from [MS-XLDM] and DashCraft's description); not yet checked on a workbook Excel saved
   (gaps.md G100). The reader returns `Unsupported` when the flag is set and the directory is
   unreadable.
3. `ADDITIONAL_LOG` semantics (only a product name observed). A backup built from scratch
   writes it as the engine does for a new database (`Default`), and Desktop restores such
   backups ([`datamodel-fresh.md`](datamodel-fresh.md)); whether it is required was not tested.
4. `FileGroup` class values beyond 100002 and 100069, and whether multi-database or
   multi-group backups occur in Power BI.
5. ~~Multi-segment `.idf` naming~~: one file per column partition (see the inventory), named
   after the partition's position; incremental-refresh policy-range partitions (8, one of them
   DirectQuery) follow the same layout ([`vertipaq-large.md`](vertipaq-large.md) §5).
6. Meaning of `Table.SystemFlags` values beyond 1/2, and of the `OwnerType` codes not seen
   (e.g. relationship index files, string/column/full-text index storage).
7. Whether `metadata.sqlitedb` schema varies across Desktop releases (record `schema_version`
   per release; 69 on 2.158.1304.0).
8. Writer acceptance: Desktop opens re-packed backups with a replaced `metadata.sqlitedb`
   (M5.1). Pending for add/remove: does a restore accept generated storage names and copied
   timestamps (`m5/remove_column.pbix`, `remove_table.pbix`, `add_table.pbix`)?

[xldm]: https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-xldm/8c62e8ce-f605-488d-81e9-4ecdb7686a52
[ksy-x9]: https://github.com/Hugoberry/pbixray/blob/main/docs/abf_xpress9.ksy
