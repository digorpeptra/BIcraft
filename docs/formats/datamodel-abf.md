# PBIX `DataModel`: ABF container, XPress9, VertiPaq, `metadata.sqlitedb`

Status: research note (2026-10-13). Every claim cites a public source. Claims marked **[unverified]**
come from secondary sources or third-party READMEs and have not been checked against a
file we generated ourselves. Spec text and third-party code are summarized, not copied.

The layers, outermost first:

1. The PBIX ZIP member `DataModel` is *stored*, with ZIP compression method 0 ([QueryPower blog][qp]).
2. Its content is an **XPress9 stream**: a UTF-16LE banner, then framed chunks ([pbixray `abf_xpress9.ksy`][ksy-x9]).
3. Decompressing that stream yields an **ABF**, an Analysis Services backup. ABF uses the
   "Spreadsheet Data Model" stream layout of [MS-XLDM] §2.1: a header page, a files section,
   and a virtual directory.
4. The ABF holds **`metadata.sqlitedb`** (the tabular schema) and **VertiPaq storage files**
   (`.idf`, `.idfmeta`, `.dictionary`, `.hidx`, plus H$/R$/U$ system tables).

---

## Sources

### Primary (Microsoft Open Specifications)

| Spec | Current version | What we use it for |
|---|---|---|
| **[MS-XLDM] Spreadsheet Data Model File Format** ([landing page][xldm]) | **Rev 8.3, 2025-08-19** (minor). Previous: 8.2 (2022-11-15), 8.1 (2021-11-16), 8.0 (2021-08-17). | ABF stream layout, VertiPaq column/dictionary/hash-index layouts, RLE and bit-packing, Huffman string pages |
| [MS-WUSP] §2.1.1 "Xpress Compression" ([page][wusp]) | n/a | Cited by [MS-XLDM] §2.7.5 as *the* compression for the whole file. It specifies LZ77 "Win2k3" Xpress with ≤65535-byte blocks, each prefixed by two LE i32 sizes (original, compressed). **This is not XPress9.** |
| [MS-XCA] Xpress Compression Algorithm ([landing page][xca]) | Rev 10.0, 2024-04-23 | Specifies only LZ77+Huffman, Plain LZ77 and LZNT1. **It does not specify XPress9.** It is relevant only to the Excel/legacy "xpress8" path. |
| [MS-SSAS-T] SQL Server Analysis Services Tabular Protocol ([overview][ssast]) | Rev 18.0, 2026-06-17 per search results **[unverified]** | Defines the tabular metadata objects (compatibility level 1200+). These are the TMSCHEMA rowsets that `metadata.sqlitedb` mirrors. §2.2.5 lists the objects and their properties. |
| [MS-SSAS] | n/a | The base XMLA/SSAS protocol. [MS-XLDM] cites it only for UserID normalization (Appendix B notes 3–8). It describes no file layout. |
| [MS-XLSX] | not consulted in depth | Only defines where Excel stores the model (`xl/model/item.data`). Not needed for PBIX. |

**IP notice.** The Open Specifications IP notice lets anyone copy the docs to build
implementations. It grants **no patent licence** by itself. A given spec *may* be covered
by the Open Specifications Promise or the Community Promise. Check the Patent Map before a
1.0 release **[unverified for MS-XLDM]** ([xldm] page, "Intellectual Property Rights Notice").

**Scope warning.** [MS-XLDM] Appendix B lists Excel 2013–LTSC 2024 and SQL Server 2008 R2/2012
([§6][xldm-b]). It does **not** list Power BI. It describes the *Excel* flavor:

- XML metadata (`*.tbl.xml`, XMObject §2.5, OLAP Load documents §2.6)
- fixed constants such as `BackupRestoreSyncVersion`, 140 in the header (§2.1.1.3) and 1153 in the log (§2.1.2.3.1)
- xpress compression of the whole file

Power BI uses compatibility level ≥1500 tabular metadata in `metadata.sqlitedb`, binary
`.idfmeta` files, and XPress9. The container and the VertiPaq encodings carry over. The
metadata layer and the constants must be confirmed by observing Desktop output (inspecting our
own Desktop-saved files).

### Secondary (open-source, used as cross-checks)

- Hugoberry/**pbixray** ([repo][pbixray], MIT). Kaitai specs under `docs/`, all **CC0-1.0**:
  [`abf_xpress9.ksy`][ksy-x9], [`idf.ksy`][ksy-idf], [`idfmeta.ksy`][ksy-idfmeta], [`dictionary.ksy`][ksy-dict].
  CC0 makes these the safest third-party layout references for us.
- Igor Cotruta, "Lessons Learned from Unpacking VertiPaq" ([QueryPower, 2024-05-15][qp]).
- bgarcevic/**ripbi** PR #123 ([link][ripbi-pr]) and the [`ripbi-xpress9` NOTICE][ripbi-notice].
- Hugoberry/**PowerPivotPeek** ([repo][ppp], MIT): the Excel/legacy xpress8 path.
- Microsoft Learn: [PBIP semantic model folder][pbip-ds] and [PBIT templates][pbit].

---

## ABF container

All section numbers below are [MS-XLDM] numbers. The stream is built from 4096-byte
**pages** (§2.1). Each of the three major regions starts on a page boundary and is
zero-padded to the next one.

```
page 0          : header (never compressed, exactly one page)
page 1 ..       : files section = BOM, Partitions XML, CRC, file, CRC, file, CRC, ...,
                  BOM, BackupLog XML, CRC, zero pad to page
next page       : VirtualDirectory XML, zero pad to page (end of stream)
```

### Header (§2.1.1, §2.1.1.1–§2.1.1.3)

| Offset | Size | Content |
|---|---|---|
| 0 | 2 | BOM `FF FE` (§2.1.1.1) |
| 2 | 70 | Stream storage signature (§2.1.1.2): a fixed ASCII marker string beginning `STREAM_STORAGE_SIGNATURE_`, stored as UTF-16LE (35 code units). pbixray reads the header XML from offset **72**, which is consistent with this layout ([parser.py][pbixray-parser]). |
| 72 | variable | `BackupLog` XML of type `BackupLogHeaderType` (§2.1.1.3), then zero padding to 4096 |

`BackupLogHeaderType` fields (§2.1.1.3), in order, paraphrased:

| Field | Type | Meaning / constraint (per the Excel spec) |
|---|---|---|
| `BackupRestoreSyncVersion` | int | Writer version. Excel requires 140. **Power BI value: verify.** |
| `Fault` | bool | Must be false. The CRC end-marker scheme is in use. |
| `faultcode` | uint | Ignored. |
| `ErrorCode` | bool | Must be true. Means "CRC markers present", and readers branch on it (pbixray passes it to the BackupLog parser). |
| `EncryptionFlag` / `EncryptionKey` | bool / int | Not encrypted. Key is ignored. |
| `ApplyCompression` | bool | Whole-file compression applied. Individual files may also be compressed independently. |
| `m_cbOffsetHeader` | u64 | Byte offset of the **VirtualDirectory**. Page-aligned. |
| `DataSize` | u64 | Byte size of the VirtualDirectory XML. |
| `Files` | uint | Number of entries in the VirtualDirectory. |
| `ObjectID` | GUID (uppercase hex) | Ignored, but must be a well-formed GUID. |
| `m_cbOffsetData` | u64 | Start of the files section (normally 4096). |

### Files section (§2.1.2, §2.1.2.1, §2.1.2.2)

- The section opens with a BOM, then the **Partitions** XML document
  (`SdfPartitionsType`, a list of `Partition`, §2.1.2.1), then a CRC marker.
- Each file follows **in its native bytes** (binary or XML), each terminated by a CRC marker
  (§2.1.2.2, §2.1.2.2.1). There are no gaps between files.
- The last file is preceded by another BOM. It is the **BackupLog** (§2.1.2.3), an XML document
  of type `SdfBackupLogType` (§2.1.2.3.1). Its fields are `BackupRestoreSyncVersion`,
  `ServerRoot`, encryption/binary-XML/compression flags, `ObjectName`, `ObjectId`, `Write`,
  `OlapInfo`, `Collations`, `Languages`, and `FileGroups`. `FileGroups` holds `SdfFileGroupType`
  entries, and each of those holds an `SdfFileListType` of `SdfFileListBackupFileType` records
  (§2.1.2.3.1.3.x).
- Each **BackupLog file record** (§2.1.2.3.1.3.1.3) has:
  - `Path`: the original path, giving the logical name, e.g. `...\metadata.sqlitedb`
  - `StoragePath`: the join key into the VirtualDirectory
  - `LastWriteTime`: a 1601-epoch timestamp
  - `Size`: **excludes** the CRC marker
- **CRC marker** (§2.1.2.2.1.1): the spec gives a table-driven CRC-32 with:
  - polynomial 0x04C11DB7
  - MSB-first (non-reflected) processing
  - initial value 0xFFFFFFFF
  - no final XOR shown

  These parameters look like CRC-32/MPEG-2 **[unverified, test against real files]**. The
  spec page does not state the marker's byte width or which bytes the CRC covers. Settle
  both empirically by diffing VirtualDirectory `Size` against BackupLog `Size`.
- `CryptKey.bin` (§2.1.2.4) is a key-blob file with an "exponent-of-one" private key
  procedure (§2.1.2.4.2). pbix-mcp reports writing a self-authored one **[unverified]** ([pbix-mcp][pbixmcp]).

### Virtual directory (§2.1.3, §2.1.3.1, §2.1.3.2)

The virtual directory starts on the page after the files section, has **no** BOM, and is
zero-padded to the end of the stream. It is the XML document `VirtualDirectory`, holding one
`BackupFile` (`VirtualDirectoryBackupFileType`) per file:

| Field | Meaning |
|---|---|
| `Path` | Storage path. Equals the BackupLog `StoragePath`. |
| `Size` | Bytes in storage, **including** the CRC marker |
| `m_cbOffsetHeader` | Absolute offset of the file in the stream |
| `Delete` | Ignored |
| `CreatedTimestamp`, `Access`, `LastWriteTime` | Timestamps counted from 1601-01-01. The spec says "nanoseconds", which is probably 100-ns FILETIME ticks **[unverified]**. |

**Reader algorithm.** pbixray uses these steps ([parser.py][pbixray-parser]):

1. Parse the header XML from offset 72 to 4096.
2. Slice the VirtualDirectory at `m_cbOffsetHeader..+DataSize`.
3. Take the **last** VirtualDirectory entry. That entry is the BackupLog.
4. Parse the BackupLog.
5. Join the BackupLog `StoragePath` to the VirtualDirectory `Path`. This gives a
   *logical name → (offset, size)* map.

`metadata.sqlitedb` is found by logical name.

**Excel/legacy variant.** When the ABF is *not* wrapped in XPress9 (it starts directly with
`FF FE` + signature), individual files may be **xpress8**-compressed. xpress8 is a chunked
framing around MS-XCA Plain LZ77, which ntdll's `RtlDecompressBuffer` with
`COMPRESSION_FORMAT_XPRESS` can decode ([PowerPivotPeek][ppp]). pbixray detects this
"uncompressed ABF" case for PBIX too, by looking for the signature in the first 72 bytes
([search summary of pbix_unpacker.py][pbixray-unpacker]). That file is absent from the current
pbixray tree, so the logic has probably moved under `pbixray/abf/`.

---

## XPress9

### Is it publicly specified?

**No.**

- [MS-XLDM] §2.7.5 ([page][xldm-xpress]) only points to [MS-WUSP] §2.1.1, which is classic LZ77 Xpress.
- [MS-XCA] covers only LZ77+Huffman, Plain LZ77 and LZNT1 ([xca]).

No Microsoft open specification describes XPress9's bitstream. What *is* public is
**Microsoft's own C source**, under the **MIT licence**, in
[microsoft/Extensible-Storage-Engine][ese] at `dev/ese/src/_xpress9/`:

- Files: `Xpress9DecLz77.c`, `Xpress9DecHuffman.c`, `Xpress9EncLz77.c`, `Xpress9EncHuffman.c`, `Xpress9Internal.h`, etc.
- Each file carries a "Copyright (c) Microsoft Corporation. Licensed under the MIT License." header.
- The repo is SPDX MIT per the GitHub API.

### Stream framing in PBIX

From the CC0 [`abf_xpress9.ksy`][ksy-x9], a third-party format description
**[unverified by us]**.

- **Banner, 102 bytes**: 51 UTF-16LE characters.
  - Single-threaded files say the backup was created "using XPress9 compression".
  - Multithreaded files say "using multithreaded XPrs9".
  - Detection: test whether the 13 characters at char offset 30 spell "multithreaded".
- **Single-threaded body**: chunks repeat until EOF.
- **Multithreaded body**: a 40-byte header of five LE u64 fields:
  1. main chunks per thread
  2. prefix chunks per thread
  3. prefix thread count
  4. main thread count
  5. chunk uncompressed size (typically 2 MiB)

  Then come the prefix-group chunks (possibly none), then the main-group chunks, stored
  thread by thread. The output is their concatenation in stored order.
- **Chunk envelope**: LE u32 uncompressed size (normally 2,097,152, with a shorter last chunk),
  then LE u32 compressed size, then the compressed bytes.
- **XPress9 block header, 32 bytes = 8 × LE u32**:
  - magic `0x4E86D72A`
  - original size
  - encoded size **in bits**
  - flags
  - reserved (0)
  - session signature (constant within a session)
  - block index (0,1,2,…; out-of-order blocks are rejected)
  - CRC-32 over the first 28 header bytes (payload not covered)
- **Flags word**:
  - bits 0–12: Huffman table length in bits
  - bits 13–15: log2(window) − 16
  - bits 16–17: MTF entry count ÷ 2 (0, 2 or 4)
  - bit 18: minimum pointer match length 3 or 4
  - bit 19: minimum MTF match length 2 or 3
  - bits 20–31: reserved, must be 0
- **Payload**: optional MTF initial offsets, a short-symbol Huffman table (≤1024 symbols), a
  long-length Huffman table (≤256 symbols), then LZ77 literals and matches.
- **Window**: the sliding window **spans chunk boundaries**, so chunks must be decoded in order
  with shared history. ripbi confirms this ([PR #123][ripbi-pr]).
  - Window range: 64 KiB to 4 MiB per one comment. The 3-bit field allows up to 8 MiB.
    This conflict is open.

### Open-source implementations

| Project | Language | Licence | Provenance | Enc/Dec | Pure Rust? |
|---|---|---|---|---|---|
| [microsoft/Extensible-Storage-Engine][ese] `_xpress9` | C | MIT | Microsoft original | both | — |
| [Hugoberry/xpress9-python][x9py] (PyPI `xpress9`) | C + Cython | MIT (wrapper © 2025 Igor Cotruta; bundled C © Microsoft, MIT) | GCC/Clang port of the ESE sources ([ripbi NOTICE][ripbi-notice]) | both | no |
| [bgarcevic/ripbi][ripbi] `ripbi-xpress9` crate | Rust FFI over vendored C | Apache-2.0 (crate); vendored C is MIT | Taken from xpress9-python at a pinned commit, checked against ESE commit `7030fe74…` | decoder; encoder also compiled (with GCC flags to avoid an `-O3` miscompile) | **no** (`cc` build) |
| [StrongWind1/ntcompress][ntc] | pure Python | Apache-2.0 | Attributed port of the MIT ESE source | both | n/a (pure, not Rust) |
| [Hugoberry/duckdb-pbix-extension][duckpbix] | C++ | MIT | Vendors the same ESE-derived C | decode | no |

**Licence compatibility for BIcraft (MIT OR Apache-2.0):**

- **The licences are compatible.** MIT code may be included in, or ported into, an
  MIT/Apache-2.0 project. The Microsoft copyright and permission notice must be kept
  (e.g. a `THIRD-PARTY-NOTICES` file and a header on the ported module). The ESE code is
  plain MIT with no field-of-use restriction.
- **No project in the table derives from Microsoft code under a restrictive licence.** All of
  them trace back to the ESE MIT release.
- **Patent caveat.** MIT carries no explicit patent grant, and XPress9 is outside the
  Open-Spec promise because it is not in any open spec. The risk is small, but it is a legal
  question, not a technical one **[unverified]**.
- **Project-policy caveat.** The project copies no Microsoft code or strings into its source
  except under a licence. A *port of Microsoft's own MIT-published source* is such a case: it
  is licensed code. The project should still decide explicitly between:
  - (i) a pure-Rust port of the ESE sources with attribution (fastest, lowest risk of
    incompatibility), or
  - (ii) an independent Rust codec written from a prose note derived from the CC0 ksy plus
    black-box testing against the C decoder.

  Either way, the C sources may be used as a **test oracle on the dev machine only** (not as a
  build dependency), so the CI on Linux stays pure Rust.
- **Encoder shortcut.** Any valid XPress9 stream decodes. A minimal encoder could emit
  literal-heavy blocks with simple Huffman tables, which avoids the hard LZ77 match search.
  Size and speed would suffer, but correctness would be easier. Whether Desktop rejects
  unusual-but-valid parameters is **[unverified]**.

### Does Desktop accept an uncompressed or differently compressed `DataModel`?

There is no authoritative evidence either way.

- **For acceptance:** PowerPivotPeek describes "legacy" PBIX files whose `DataModel` is a bare
  ABF starting with the stream signature ([ppp]). pbixray and ripbi both still detect that
  layout ([ripbi-pr]).
- **Against assuming acceptance:** no source shows *current* Desktop opening such a file.
- **pbix-mcp** writes XPress9 (single- and multithreaded framing, core via xpress9-python). It
  claims its builder output opens in Desktop (March 2026) with refresh working **[unverified]**
  ([pbix-mcp][pbixmcp]).
- **Test to run (Windows, dev only):** take a Desktop-saved PBIX, then:
  1. Re-wrap its decompressed ABF as (a) a bare ABF and (b) single-threaded XPress9.
  2. Open both in Desktop 2.158.1304.0.
  3. Record the result in this note.

---

## VertiPaq storage

### Files and naming

[MS-XLDM] §2.2 defines the Excel naming, e.g. `4.Table1.Cat.0.idf`, `4.Table1.Label.dictionary`
and `1.H$Table1$Cat.hidx` (§2.3.1, §2.3.2, §2.3.3). Power BI ABFs use a folder layout instead
([QueryPower][qp]):

- `{Table} ({TableID}).tbl/` folders containing `{PartitionID}.prt/` subfolders
- system tables `H$…` (attribute hierarchies), `R$…` (relationship indexes) and `U$…` (user hierarchies)

The authoritative mapping from columns to files is the `StorageFile` table in
`metadata.sqlitedb` (see below).

| File | Spec section | Role |
|---|---|---|
| `.idf` | §2.3.1, §2.3.1.1.1–3 | Column data. One data ID per row, possibly reordered for compression. |
| `.idfmeta` | not in [MS-XLDM]; Power BI binary, see [`idfmeta.ksy`][ksy-idfmeta] | Per-segment compression class, bit width, min/max data ID, row count. In Excel this information lives in `XMColumnSegment` XML (§2.5.2.12). |
| `.dictionary` | §2.3.2, §2.3.2.1.x, §2.7.4 | Value ↔ data ID for hash-encoded columns |
| `.hidx` | §2.3.3, §2.3.3.1.x | Hash index for *value-encoded* columns (`XMValueDataDictionary<Long/Real>`) |
| RowNumber column | §2.3.4 | Uses XM123 compression (§2.7.2) |
| Position↔ID maps, relationship index, user-hierarchy files | §2.4.1–§2.4.4 | System-generated. A writer must produce them or Desktop must rebuild them. Whether Desktop rebuilds them is **[unverified]**. |

### `.idf` layout and encodings

- **Segments** (§2.3.1.1.1): a sequence of `{SegmentSize: u64, Segment bytes}`. With hybrid
  compression (§2.3.1.1.2), each primary segment is followed by its own `{u64 size, SubSegment}`.
  The CC0 ksy reads both sizes as counts of **8-byte units** ([idf.ksy][ksy-idf]). Segment size
  limits are in §2.3.1.1.3. Multiple segments are typical above about 2^20 rows **[unverified]**.
- **Hybrid RLE** (§2.7.3, §2.7.3.1):
  - The primary segment is a list of 8-byte entries (u32 + u32).
  - An **RLE entry** is (value, repeat count). The spec requires runs of at least 64.
  - A **bit-packing entry** reuses the same two slots as (negative running offset into the
    sub-segment, count). The offsets go −1, then −1−n₁, and so on.
  - Equivalently, a reader treats an entry as bit-packed when its value equals
    `0xFFFFFFFF − cumulative_bitpacked_count` ([idf.ksy][ksy-idf]).
  - Trailing slots are padding. Stop when cumulative repeats reach the row count.
- **Bit packing / XMRENoSplit\<N\>** (§2.7.1, §2.7.1.1–14, Appendix A §5): range encoding.
  - Subtract a minimum, then pack each value in N bits, N ∈ {1–10, 12, 16, 21, 32}.
  - pbixray packs `64/N` values per LE u64 word, low bits first, and adds the segment's
    `min_data_id` back ([vertipaq_decoder.py][pbixray]).
  - The storage must be zeroed before packing (§2.7.1).
- **Data IDs**: pbixray treats ID 2 as NULL and dictionary IDs as starting at 3 (`XM_DATA_ID_NULL = 2`) **[unverified, pbixray-derived]**.
- **`.idfmeta`** ([idfmeta.ksy][ksy-idfmeta], CC0):
  - The format is a nested tagged binary with ASCII open/close tags (`<1:CP`, `<1:CS`, `<1:SS`, `<1:SDOs`, `<1:CSDOs`).
  - Per segment it stores:
    - records, as u64
    - base ID, as u64
    - compression class and sub-class, as u32 IDs that map to N
    - RLE facet data: bookmark bits plus allocated and used sizes
    - first run value
    - SS statistics: distinct states u64, min/max/original-min data ID u32, sort order i64,
      row count u64, has-nulls u8, RLE run counts u64
    - an optional nested sub-segment
  - The SDOs block gives each segment's (size, offset) inside the `.idf`.

### `.dictionary` and `.hidx`

- **Dictionary** (§2.3.2.1, [dictionary.ksy][ksy-dict]):
  - Starts with an i32 type tag: 0 = long, 1 = real, 2 = string (§2.3.2.1.3.1 `XM_TYPE`), followed by hash parameters.
  - **Numeric dictionaries** are a plain vector: count, element size, values.
  - **String dictionaries** (§2.3.2.1.2) hold page-layout counters, then pages:
    - Each page has a mask, null flag, start index, count, a compression flag, and a store
      framed by begin/end markers (§2.3.2.1.2.4.3).
    - Uncompressed pages hold UTF-16LE strings (§2.3.2.1.2.4.1).
    - Compressed pages (§2.3.2.1.2.4.2) use a **Huffman** code per §2.7.4: a classical
      unbalanced tree, bounded codeword sizes, an alphabet-size limit, and single or multiple
      character-set modes. The page carries a 128-entry encode table.
    - A **record-handle vector** (§2.3.2.1.2.5) follows, with an (offset u32, page u32) pair per string.
  - BLOBs are base64-encoded and stored as strings (§2.3.2.1.2.1).
- **Hash index `.hidx`** (§2.3.3.1.x):
  - Hash statistics, then hash bins (`HashBin`, §2.3.3.1.4.4) and overflow entries (`HashEntry`, §2.3.3.1.4.5).
  - Hash algorithm constants are in §2.3.3.1.3–§2.3.3.1.4.
  - The file is unordered and uncompressed.
  - QueryPower notes that value-encoded IDs map back to values via `BaseId` and `Magnitude`
    in the metadata ([qp]). The exact formula is **[unverified]**.

---

## metadata.sqlitedb

- **What it is.** A SQLite 3 database inside the ABF, found by its logical name in the BackupLog.
  Its tables mirror the tabular object model, i.e. the TMSCHEMA DMVs defined in [MS-SSAS-T] §2.2.5:
  - `Table`, `Column`, `Partition`, `Measure`, `Relationship`, `Hierarchy`, `Role`, `Expression`, `Annotation`, …
  - plus **storage tables** that link objects to ABF files:
    - `ColumnStorage` → `DictionaryStorage` → `StorageFile`
    - `ColumnPartitionStorage` → `StorageFile` (the `.idf`)
    - `AttributeHierarchy` → `AttributeHierarchyStorage` → `StorageFile` (`.hidx`/H$)
    - `PartitionStorage` (`StoragePosition` orders partitions)
    - `RelationshipStorage` / `RelationshipIndexStorage` (R$)

  Sources: the joins in pbixray's [`sqlite_source.py`][pbixray] and the dump in its
  [`metadata.sqlitedb.csv`][pbixray].
- **Size of the schema.** pbix-mcp reports **68 system tables** in the file it generates **[unverified]** ([pbixmcp]).
- **Reading.** Any SQLite reader works. ripbi opens it read-only with `rusqlite` and maps it to
  a tabular model ([ripbi-pr]). For BIcraft, note that `rusqlite` bundles the C SQLite, so it is
  **not pure Rust**. Pure-Rust options:
  - Write a minimal reader/writer for the public SQLite file format (B-tree pages, record format).
  - Evaluate `turso_core` ([tursodatabase/turso][turso], MIT, SQLite-compatible Rust rewrite).
    Its file-format compatibility for this use is **[unverified]**.
- **Writing.** This is the hardest metadata piece. Exact schema DDL, IDs, `ModifiedTime` fields,
  storage rows and consistency with every VertiPaq file must match what the engine expects on
  restore. Read the `metadata.sqlitedb` of our own Desktop-saved files (any SQLite reader) to
  capture the schema per Desktop version.

---

## Existing implementations

"Read" means parsing the `DataModel` offline. "Write" means producing a `DataModel` that Power BI
Desktop opens.

| Project | Language | Licence | Read | Write | Notes |
|---|---|---|---|---|---|
| [Hugoberry/pbixray][pbixray] | Python (+C ext.) | MIT; ksy docs CC0 | **Yes, full**: XPress9, ABF, sqlitedb, VertiPaq data, DAX/M text, 40 TMSCHEMA views; also `.abf` and `.xlsx` | **No** (read-only by design) | Most mature reader. The CC0 Kaitai specs are directly reusable as references. |
| [d0nk3yhm/pbix-mcp][pbixmcp] | Python | MIT | Yes: VertiPaq, sqlitedb, ABF list/extract | **Claims yes**: builds a full DataModel (ABF, 68-table sqlitedb, VertiPaq, H$/R$, XPress9); metadata splicing; "verified with PBI Desktop March 2026" | **[unverified]** claims. RLE is disabled in its encoder (bit-pack only). Adding partitions is blocked. PATH() fails over builder tables. XPress9 core comes from xpress9-python (C). **The only public evidence that writing is feasible.** |
| [bgarcevic/ripbi][ripbi] (`ripbi-core`, `ripbi-xpress9`) | Rust (+vendored C) | Apache-2.0 (C parts MIT) | Metadata only: XPress9 (all 3 framings), ABF, sqlitedb via rusqlite; "VertiPaq data is never read" | No | Closest in spirit to us, but not pure Rust (`cc` and bundled SQLite). Fuzz-found infinite-loop fix in the C decoder. |
| [Hugoberry/duckdb-pbix-extension][duckpbix] | C++ | MIT | Yes: metadata and table data | No | — |
| [Hugoberry/PowerPivotPeek][ppp] | PowerShell | MIT | Bare ABF and xpress8 only (Excel, legacy PBIX); detects XPress9 but cannot decode it | No | Windows-only (ntdll) |
| [StrongWind1/ntcompress][ntc] | Python | Apache-2.0 | XPress9 codec only | XPress9 *encode* (ESE scheme) | Pure-language port of the ESE codec. Useful as a second reference implementation. |
| [pbi-tools][pbitools] | C#/.NET | **AGPL-3.0** | AGPL, not consulted | — | — |
| [Tabular Editor 2][te2] | C# | MIT | No offline PBIX DataModel parsing. Edits model.bim/PBIT (unsupported by Desktop) or connects to Desktop's AS instance ([TE docs][te-lim]). | No | Dev-time oracle only |
| TahirRiaz/SQLFlow `tools/pbix-extract` | C | **GPL-3.0** | GPL, not consulted | — | — |
| crates.io [`pbix`][crate-pbix] 0.1.0 (powerlint) | Rust | MIT OR Apache-2.0 | Report-file parsing (2023, repo now 404) | No | Not a DataModel reader **[unverified]** |

**Bottom line.**

- Several projects can **read**.
- Exactly one public project, pbix-mcp, **claims to write** a DataModel that Desktop accepts.
  It does so in Python on top of Microsoft's C codec, and with a list of caveats.
- None is pure Rust, and none is a licence obstacle (MIT/Apache/CC0). The AGPL and GPL
  projects must not be used as code sources.

---

## Writing strategy recommendation

| Option | What we write | Feasibility | What is lost or at risk |
|---|---|---|---|
| **(a) Full PBIX writer** | ABF container + XPress9 encoder + `metadata.sqlitedb` + VertiPaq (`.idf`/`.idfmeta`/dictionaries/`.hidx`/H$/R$/U$ + CryptKey.bin) | **Hard, but shown possible** (pbix-mcp, [unverified]). The container is fully spec'd ([MS-XLDM] §2.1). The encodings are spec'd (§2.3, §2.7). XPress9 has MIT source. The unknowns are Power BI-specific: sqlitedb schema/IDs, `.idfmeta`, H$/R$ requirements, constants, version drift per Desktop release. | Highest maintenance. Every Desktop release can drift. Large test matrix. |
| **(b) PBIT** | `DataModelSchema` (UTF-16LE TMSL) + report, no `DataModel` | **Easy.** TMSL is documented. | **All data.** On open, Desktop prompts for parameters and data locations, then loads from sources ([pbit]). Models whose data cannot be re-fetched (e.g. "Enter data" tables are fine since they live in M, but one-off files and snapshots are not) lose content. It is single-file and familiar. |
| **(c) PBIP primary** | `.pbip` + `*.SemanticModel` (TMDL or `model.bim`) + `*.Report` (PBIR or `report.json`), no `.pbi/cache.abf` | **Easiest and officially supported.** Desktop opens a PBIP without `cache.abf` "connected to a model with its entire definition but without data" ([pbip-ds]). Text formats are documented. External edits are supported. | **Data** (needs a refresh, like PBIT). It is a folder, not a single file. |
| (d) *Variants of (a)* | (d1) **metadata splice**: decompress an existing DataModel, change only `metadata.sqlitedb` rows, re-pack (pbix-mcp's "binary splice"); (d2) **schema-only DataModel** with empty partitions | Medium. Still needs ABF + XPress9 encoder + SQLite writer, but **no VertiPaq encoder**. Whether Desktop accepts (d2) is **[unverified]**. | (d1) only edits; it cannot add tables or columns with data. (d2) loses data. |

**Recommendation.**

1. **Make PBIP (option c) the primary "compile" target**, and **PBIT (b) the single-file
   output**. Both are text-based, documented and cross-platform, and Desktop opens them
   natively. "Requires refresh" is the honest cost.
2. **Treat PBIX as read-mostly, built in layers that are useful on their own:**
   1. L0 `xpress9` **decoder** in pure Rust, with fuzzing and bounded allocations, ported from
      the MIT ESE code or written independently. Cross-check it on dev against the C oracle.
   2. L0 `abf` **reader** ([MS-XLDM] §2.1).
   3. A pure-Rust SQLite **reader** for `metadata.sqlitedb` → `model`.
   4. A VertiPaq **reader** (§2.3/§2.7 + CC0 ksy).

   This already gives full PBIX → model → PBIP/PBIT conversion.
3. **Add PBIX writing incrementally behind a feature flag, gated by the Desktop round-trip check:**
   1. ABF writer and XPress9 encoder. Start with a simple valid encoder. Verify with our own
      decoder, the C oracle and Desktop.
   2. (d1) metadata splice for measure/description/format edits on existing PBIX.
   3. Only then (a) the full writer, starting with bit-pack-only VertiPaq (no RLE, as pbix-mcp
      does) and hash dictionaries.

   Record the Desktop version for every accepted file.
4. **Before step 3, decide and document the XPress9 provenance policy:** an attributed port of
   the MIT ESE source, or an independent implementation. Add the MIT notice either way.

---

## Open questions

1. Power BI values for the header and BackupLog constants (`BackupRestoreSyncVersion`, `ServerRoot`,
   `Write`, collations/languages, Partitions XML). [MS-XLDM] gives only Excel's values. Verify on
   Desktop 2.158.1304.0 output.
2. The CRC marker's exact width, coverage and parameters (§2.1.2.2.1.1 gives the algorithm only).
3. ~~Does current Desktop open a `DataModel` that is (a) a bare ABF, (b) single-threaded XPress9 when it
   would normally write multithreaded, or (c) XPress9 with non-default window/MTF parameters?~~
   Yes to all three (G17, 2026-10-15, [`xpress9.md`](xpress9.md) §6.2: (a) and (c) in Desktop
   2.158 and its engine, (b) in the engine up to 845 MB of ABF).
4. The XPress9 window maximum (4 MiB vs 8 MiB; 2^16 and 2^22 load). Settled (G17): chunks of at
   most 2 MiB are required (2 MiB + 1 byte fails), any constant session signature loads
   ([`xpress9.md`](xpress9.md) §6.2).
5. Must a writer emit the H$ / R$ / U$ system tables, position↔ID maps (§2.4) and `.hidx`, or does
   the engine rebuild them on restore? pbix-mcp generates H$/R$ **[unverified]**.
6. The exact `metadata.sqlitedb` DDL per Desktop version, which columns are mandatory, and how
   `StorageFile` paths map to ABF logical names.
7. Is a schema-only DataModel (empty partitions, option d2) accepted, and does it refresh cleanly?
8. Is MS-XLDM covered by the Open Specifications Promise (Patent Map)? Is any patent risk attached
   to XPress9, which is MIT-licensed code with no explicit patent grant?
9. How value-encoded columns map back to values (`BaseId`/`Magnitude`), and the meaning of the
   reserved data IDs 0–2.
10. The status of the `pbix` crate (powerlint): its repository returns 404.

[xldm]: https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-xldm/8c62e8ce-f605-488d-81e9-4ecdb7686a52
[xldm-b]: https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-xldm/fb3a525b-3996-4328-ac4b-037fe1204cf4
[xldm-xpress]: https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-xldm/3e3dac06-3f45-48a5-93d4-abacd4b34000
[wusp]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/MS-WUSP/3e24630e-8000-4894-a967-315df7ed996e
[xca]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-xca/a8b7cb0a-92a6-4187-a23b-5e14273b96f8
[ssast]: https://learn.microsoft.com/en-us/openspecs/sql_server_protocols/ms-ssas-t/b98ed40e-c27a-4988-ab2d-c9c904fe13cf
[pbip-ds]: https://learn.microsoft.com/en-us/power-bi/developer/projects/projects-dataset
[pbit]: https://learn.microsoft.com/en-us/power-bi/create-reports/desktop-templates
[qp]: https://querypower.com/2024/05/15/lessons-learned-from-unpacking-vertipaq-a-developers-journey/
[pbixray]: https://github.com/Hugoberry/pbixray
[pbixray-parser]: https://github.com/Hugoberry/pbixray/blob/main/pbixray/abf/parser.py
[pbixray-unpacker]: https://github.com/Hugoberry/pbixray/blob/main/pbixray/pbix_unpacker.py
[ksy-x9]: https://github.com/Hugoberry/pbixray/blob/main/docs/abf_xpress9.ksy
[ksy-idf]: https://github.com/Hugoberry/pbixray/blob/main/docs/idf.ksy
[ksy-idfmeta]: https://github.com/Hugoberry/pbixray/blob/main/docs/idfmeta.ksy
[ksy-dict]: https://github.com/Hugoberry/pbixray/blob/main/docs/dictionary.ksy
[x9py]: https://github.com/Hugoberry/xpress9-python
[ese]: https://github.com/microsoft/Extensible-Storage-Engine
[ripbi]: https://github.com/bgarcevic/ripbi
[ripbi-pr]: https://github.com/bgarcevic/ripbi/pull/123
[ripbi-notice]: https://github.com/bgarcevic/ripbi/blob/main/crates/ripbi-xpress9/NOTICE.md
[ntc]: https://github.com/StrongWind1/ntcompress
[duckpbix]: https://github.com/Hugoberry/duckdb-pbix-extension
[ppp]: https://github.com/Hugoberry/PowerPivotPeek
[pbixmcp]: https://github.com/d0nk3yhm/pbix-mcp
[pbitools]: https://github.com/pbi-tools/pbi-tools
[te2]: https://github.com/TabularEditor/TabularEditor
[te-lim]: https://docs.tabulareditor.com/getting-started/desktop-limitations.html
[crate-pbix]: https://crates.io/crates/pbix
[turso]: https://github.com/tursodatabase/turso
