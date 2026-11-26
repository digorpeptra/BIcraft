# VertiPaq column storage (as implemented in `crates/vertipaq`)

Status: implemented reader, 2026-10-13 (milestone M4.5); writer notes for M5. Companion to
[`abf.md`](abf.md) (the container and the file inventory), [`datamodel-abf.md`](datamodel-abf.md)
(research and sources) and [`metadata-sqlitedb.md`](metadata-sqlitedb.md).

**Sources.**

- [MS-XLDM] rev 8.3 (2025-08-19), summarized, never copied: §2.3.1 column data storage (`.idf`),
  §2.3.2 dictionaries, §2.3.3 hash index (`.hidx`), §2.7.1 `XMRENoSplit` bit packing, §2.7.2 XM123,
  §2.7.3 hybrid RLE, §2.7.4 Huffman compression ([landing page][xldm]).
- The CC0 Kaitai specs of Hugoberry/pbixray ([`idf.ksy`][ksy-idf], [`idfmeta.ksy`][ksy-idfmeta],
  [`dictionary.ksy`][ksy-dict]) for the Power BI-only `.idfmeta` and for field names.
- pbixray (MIT) `vertipaq_decoder.py` and the README of xmhuffman (MIT), read for understanding
  only; no code reused.
- **Observations** on two Desktop-saved backups of one model (PBIX `DataModel` and PBIP
  `.pbi/cache.abf`, Desktop Store 2.158.1304.0). Held locally, never committed: this note records structure,
  counts and constants only, never names or values.

API (`bicraft-vertipaq`, layer L0, pure functions over byte slices):
`decode_column(idf, idfmeta, dictionary, ColumnEncoding) -> Vec<CellValue>`, plus
`parse_idfmeta`, `split_idf`, `decode_data_ids`, `parse_dictionary`, `parse_hash_index`,
`ids_to_values`, `bitpack::{pack, unpack}`, `huffman::{HuffmanCode, BitWriter}` and
`ColumnMeta::to_bytes` (a byte-exact `.idfmeta` serializer). Hostile input returns an `Error`;
rows are capped by `Limits::max_rows` (default 2^24), decoded output (4 B per data id, one
`CellValue` per cell plus its text) by `Limits::max_bytes` (default 1 GiB), checked before allocation,
and decoded strings by 4 × input size.

All integers are little-endian. "Word" = 8 bytes (`u64`).

---

## 1. Files of one column and how to find them

| File | Owner (`StorageFile.OwnerType`) | One per | Needed to decode? |
|---|---|---|---|
| `.idf` | 23 `ColumnPartitionStorage` | column × partition | yes |
| `.idfmeta` | 24 `SegmentStorage` | column × partition | yes |
| `.dictionary` | 22 `DictionaryStorage` | column (hash-encoded only; none when every row is blank) | for hash encoding |
| `.hidx` | 27 `AttributeHierarchyStorage` | column (some value-encoded columns) | no |

Join path (all IDs are `ID` columns of the named tables):

```
Column.ColumnStorageID ─► ColumnStorage
ColumnStorage.DictionaryStorageID ─► DictionaryStorage (.StorageFileID ─► .dictionary, 0 if none)
ColumnPartitionStorage[ColumnStorageID = ColumnStorage.ID]   one row per partition
    .StorageFileID ─► .idf
    .SegmentStorageID ─► SegmentStorage.StorageFileID ─► .idfmeta
    .PartitionStorageID ─► PartitionStorage (.StoragePosition orders partitions)
AttributeHierarchy[ColumnID] ─► AttributeHierarchyStorage.StorageFileID ─► .hidx (0 if none)
StorageFile.StorageFolderID ─► StorageFolder.Path;  ABF logical path = Path + "\" + FileName
```

A column's rows are its partitions' rows concatenated in `StoragePosition` order; within a
partition, its segments in `.idfmeta` order. Every column of a table has the same row count.

File names: `1.<TableStorage.Name>.<ColumnStorage.Name>.<k>.idf` / `.idfmeta` in the
partition's folder (`<table folder>\<Partition.ID>.prt`), `k` the partition's position (0 to 7
on an 8-partition incremental-refresh table, 0 to 2 on a 3-partition table); one file pair per
column partition holds all of its segments. The `.dictionary` (`1.<table>.<column>.dictionary`)
and `.hidx` live in the table folder, one per column whatever the partition count.

**Columns of blanks** (engine, calculated columns `IF([Value] < 0, …)` of int64, string,
double and date type): hash encoding without a `.dictionary` (`DictionaryStorage.StorageFileID`
0, `LastId` 2, `IsNullable` 1; `IsOperatingOn32` 1 for int64, 0 otherwise; `Size` 1 296 int64,
16 904 string, 2 280 double/date), every row data id 2 (one RLE run), `.idfmeta`
`original_min_segment_data_id` 0x8000_0000 and `Statistics_OriginalMinSegmentDataID` −2^31,
`Statistics_DistinctStates` 1, `H$` of one member. A boolean `IF([Value] < 0, TRUE())` is
stored as `FALSE`, not blank. The reader accepts a missing `.dictionary` when every id is 2.

**Empty partitions** (engine: the DirectQuery partition of a hybrid table, 0 rows): one
segment per column with `records` 0, `CS.base_id` 0, `bookmark_bits` 1, `SS` min/max 2 and
`original_min_segment_data_id` 2 (value encoding too); the row-number column keeps one RLE
entry `(0xFFFF_FFFF, 0)` (`storage_used_size` 2), `first_run_value` the next row number.

---

## 2. Data ids

Every row of every column is stored as a **data id** (`u32`):

| Id | Meaning |
|---|---|
| 0, 1 | not seen in user columns (system tables use them, see §8) |
| 2 | blank (NULL) |
| ≥ 3 | a value: hash encoding → dictionary index `id − 3`; value encoding → arithmetic (§6) |

`DictionaryStorage.LastId` is the largest id (`2 + dictionary length` for hash encoding).
`ColumnStorage.Statistics_MinDataID/MaxDataID` equal the first segment's `SS` min/max (checked:
52/52 user columns).

---

## 3. `.idfmeta`

A tagged binary. Tags are ASCII with a trailing NUL; the opening tag is `<1:XX\0`, the closing tag
`XX:1>\0`. Fields between tags have fixed widths.

```
CP     "<1:CP\0"   u64 segment_count   CS × segment_count   "CP:1>\0"
SDOs   (optional)  "<1:SDOs\0"   CSDOs × segment_count   "SDOs:1>\0"
```

**`CS` (column segment), primary:**

| Field | Type | Meaning / observed |
|---|---|---|
| tag | 6 | `<1:CS\0` |
| records | u64 | rows in the segment |
| base_id | u64 | observed 1 for hybrid RLE segments, 0 otherwise. **Meaning unknown**; not used to decode hybrid segments |
| compression_class | u32 | §4 |
| sub_compression_class | u32 | §4 (1 = none) |
| RLE facet | 25 | only when `compression_class` = 703066 (hybrid RLE): `bookmark_bits` u64, `storage_alloc_size` u64, `storage_used_size` u64, `needs_resizing` u8 |
| first_run_value | u32 | the packing base of the segment: `SS.min_data_id`, or 2 (the null id) when the segment holds blanks (every engine-written segment of [`vertipaq-large.md`](vertipaq-large.md)) |
| SS | 65 | statistics, below |
| has_sub_segment | u8 | 1 for hybrid segments |
| sub `CS` | 29 | if `has_sub_segment`: `<1:CS\0`, `records` u64 (= number of bit-packed values), `base_id` u64 (0), `has_sub_segment` u8 (must be 0), `CS:1>\0` |
| tag | 6 | `CS:1>\0` |

**`SS` (segment statistics)**, 65 bytes: `<1:SS\0`, `distinct_states` u64 (observed 0),
`min_data_id` u32, `max_data_id` u32, `original_min_segment_data_id` u32 (2 or 3),
`rle_sort_order` i64 (−1), `row_count` u64 (= records), `has_nulls` u8, `rle_runs` u64,
`others_rle_runs` u64, `SS:1>\0`.

- `min/max_data_id` exclude the null id. `rle_runs` = number of RLE run entries when the segment
  is pure RLE, 0 when it has a bit-packed run (no observed segment mixes both).
  `others_rle_runs` was 0 or 1 (1 on most bit-packed segments); meaning unknown.
- RLE facet (52/52 hybrid files): `storage_alloc_size` = 2 × allocated RLE entries;
  `storage_used_size` = 2 × (used entries + 1), or 2 × used entries when the inner compression is
  XM123. `bookmark_bits` = the smallest `b` with `2^b × (entries + 1) ≥ 10 × (records + 1)`
  (401 distinct (rows, entries) pairs, 1–1 048 576 rows; 24 for XM123).

**`CSDOs` (segment data objects)**: `<1:CSDOs\0`, `offset` u64, `words` u64, [nested `CSDOs` for
the sub-segment when the segment has one], `CSDOs:1>\0`. `offset` is the byte offset in the
`.idf` of the block's size field and `words` its size in words. (pbixray's ksy names these two
fields the other way round; the observed values are offset first: e.g. 0 then 16.)

Sizes: a hybrid single-segment `.idfmeta` is 264 bytes; a non-hybrid segment is 26 bytes shorter
(no facet, no sub `CS`), and its `CSDOs` has no nested entry.

The serializer `ColumnMeta::to_bytes` reproduces all 132 observed `.idfmeta` files byte for byte.

---

## 4. Compression classes

| Id | Class ([MS-XLDM] name) | Used for |
|---|---|---|
| 703030 + N, N ∈ {1–10, 12, 16, 21, 32} | `XMRENoSplitCompressionInfo<N>`: bit packing, §5.2 | inner class of hybrid segments; whole segments of system tables |
| 703063 | `General` (per pbixray ksy) | not observed, unsupported |
| 703066 | `XMHybridRLECompressionInfo<…>`: RLE + bit packing | every user column segment |
| 703067 | `XM123CompressionInfo`: consecutive ids | inner class of RowNumber columns |

Observed (both backups): user columns always `(703066, 703030+N)` or `(703066, 703067)`; system
tables `(703062, 1)` (NoSplit<32>) for `POS_TO_ID`, `ID_TO_POS` and user-hierarchy files, and
`(703033|703035|703040, 1)` for relationship `INDEX` files.

---

## 5. `.idf`

A sequence of **blocks** `{u64 words; words × 8 bytes}` (the ksy and the data agree that the size
counts words, not bytes). A hybrid segment has two blocks (primary, then sub-segment); every
other segment has one. The block positions are repeated in the `.idfmeta` `CSDOs` (the reader
checks them). Nothing follows the last block.

### 5.1 Hybrid RLE segment (§2.7.3)

Primary block = RLE entries of 8 bytes: `(value u32, count u32)`.

- A **run** entry: `count` rows of data id `value`.
- A **bit-packed** entry: `count` rows taken from the sub-segment. Its `value` is
  `0xFFFFFFFF − p`, where `p` is the number of values consumed by earlier bit-packed entries of
  the segment (so −1, −1−n₁, … as an `i32`).
- Entries are read until their counts reach `records`; the rest is zero padding. Allocation:
  at least 16 entries (all 52 user files had 16, 32 or 64 allocated). The reader caps the last
  entry at `records` (as pbixray does) but rejects segments whose entries cover fewer rows.

Sub-segment block = bit-packed values (§5.2) with the inner width, `sub CS.records` values.
A stored value `v` decodes to data id `v + base`, where **`base` = 2 if `SS.has_nulls`, else
`SS.min_data_id`** (pbixray's rule; consistent with all observed data). With an XM123 inner class
the sub-segment is empty (0 words) and the k-th bit-packed value is `SS.min_data_id + k`.

Allocation observed in all 52 hybrid files: the sub-segment has `ceil((n + 1) / (64 / N))` words
for `n` packed values, i.e. room for one extra value; a pure-RLE segment still has one zero word.
The inner width N is the smallest allowed width with `2^N > max(max_data_id − min_data_id, 1)`.

### 5.2 Bit packing, `XMRENoSplit<N>` (§2.7.1)

Each word holds `64 / N` values (N = 1 → 64, 2 → 32, 3 → 21, 4 → 16, 5 → 12, 6 → 10, 7 → 9,
8 → 8, 9 → 7, 10 → 6, 12 → 5, 16 → 4, 21 → 3, 32 → 2), the first value in the **lowest** bits.
Values never straddle words; spare high bits are zero.

### 5.3 Non-hybrid segments (system tables)

One block per segment holding `records` values packed at width N; data id = value + `CS.base_id`
(observed 0). Allocation is again `ceil((records + 1) / (64 / N))` words (all system segments).
XM123 as a top-level class would be `min_data_id + row` (not observed).

---

## 6. From data ids to values

`DictionaryStorage` says how (`ColumnEncoding::from_dictionary_storage`):

| `Type` | Encoding | `DataType` observed | Value of id ≥ 3 |
|---|---|---|---|
| 1 | **hash**: `.dictionary` present | 2 string, 6 int64, 8 real | dictionary entry `id − 3` |
| 2 | **value**: no dictionary | 6 int64, 8 double | `(id + BaseId) / Magnitude` |
| 0 | **none** (system tables, `DataType` 19) | 19 | the data id itself |

Id 2 is blank in both hash and value encoding. The reader computes value encoding exactly:
Magnitude 1 → integer; Magnitude 10^−k → `(id + BaseId) × 10^k`; otherwise a float division.

Observed `BaseId`/`Magnitude` pairs: RowNumber columns `−3 / 1` (so row number = id − 3 = the
0-based row index; in a multi-partition table the ids continue across partitions, partition k
starting at 3 + the rows of the partitions before it), int64 columns `−2 / 1`, decimal columns
`BaseId` arbitrary with Magnitude 10^−4, 10^−2, 10^−1 or 1, **double columns** (`DataType` 8)
whose values become integers when multiplied by 10^k: `Magnitude` 10^k (10 and 1000 seen) and
`BaseId` = the smallest scaled value − 3. Hash dictionaries carry `BaseId` 2, `Magnitude` 0.

Logical types (from `Column.ExplicitDataType`, else `InferredDataType`), converted by
`ColumnEncoding::with_logical_type`:

| Column type | Physical storage observed | `CellValue` |
|---|---|---|
| string (2) | hash, string dictionary | `Text` |
| int64 (6) | hash (int dictionary) or value | `Int` |
| double (8) | hash, real dictionary | `Real` |
| dateTime (9) | hash, **real** dictionary | `DateTime(f64)`: OLE Automation days since 1899-12-30 |
| decimal (10) | value, int64 | `Decimal(i64)`: the value × 10 000 (Magnitude folds in the scale) |
| boolean (11) | hash, int dictionary of 0/1 | `Bool` |

---

## 7. `.dictionary` (§2.3.2)

```
i32  type                 0 XM_TYPE_LONG, 1 XM_TYPE_REAL, 2 XM_TYPE_STRING
24   hash header          i32 algorithm, u32 entry size, u32 bin size, u32 local entry count,
                          u64 bin count = 0xFFFFFFFFFFFFFFFF (no hash table follows)
...  body                 numeric vector or string store
```

Observed hash headers: long with 4-byte elements `(−1, 8, 64, 6)`, long with 8-byte elements
and real `(−1, 16, 64, 3)`, string `(0, 8, 64, 6)`.

**Numeric body** (§2.3.2.1.1.2): `u64 count`, `u32 element size`, `count` elements. Long:
element size 4 (`i32`, then `DictionaryStorage.IsOperatingOn32` = 1) or 8 (`i64`); real: 8
(`f64`). Values are in data-id order and not sorted.

**String body:**

```
u64 string_count   u8 store_compressed (observed 1 even with no compressed page)
u64 longest_string (UTF-16 units)   u64 page_count
page × page_count
u64 handle_count (= string_count)   u32 handle size (8)   (u32 offset, u32 page) × handle_count
```

Page:

| Field | Type | Notes |
|---|---|---|
| mask | u64 | 1 on compressed pages, 0 on uncompressed ones |
| contains_nulls | u8 | 1 on pages holding an empty string |
| start_index | u64 | index of the page's first string |
| string_count | u64 | |
| compressed | u8 | 0 or 1 |
| begin mark | 4 | `DD CC BB AA` |
| store | var | uncompressed or compressed, below |
| end mark | 4 | `CD AB CD AB` |

Uncompressed store: `u64 remaining_chars` (observed 0), `u64 used_chars`, `u64 allocation`
(bytes, observed = 2 × used_chars), then `allocation` bytes of NUL-terminated UTF-16LE strings.
A handle's offset is the string's **character** offset.

Compressed store: `u32 total_bits`, `u32 charset_mode` (703121 single, as in pbixray's `dictionary.ksy`;
703122 multiple; both values as found on the 130 engine-written pages of the verification below),
`u64 allocation`, `u8 charset` (single mode only), `u32 decode_bits` (the longest codeword
length on all 130 engine pages seen), 128-byte encode array, `u64 buffer_size`, `buffer_size`
bytes of bit stream. Engine pages: `buffer_size` = `allocation` = `2 × (ceil(total_bits / 16) +
1)`, room for one more 16-bit word. A handle's offset
is the string's **bit** offset; a string ends at the next handle's offset on the same page (equal
offsets: empty string), the last one at `total_bits`.

Huffman (§2.7.4):

- Encode array: byte i holds the code length of symbol 2i in its low nibble and of 2i+1 in its
  high nibble; 0 = unused, otherwise 1–15 (the engine gives a dominant symbol a 1-bit code). A
  page can therefore decode to 8 characters per compressed byte (strings of 4 000 and
  100 000 repeated letters); the reader bounds the output at 8 symbols per input byte.
- The lengths are a plain Huffman code of the page's symbol frequencies (low bytes of the units
  in single mode, both bytes in multiple mode): all 130 engine pages equal the encoder's
  `limited_code_lengths`.
- Canonical code: sort used symbols by (length, value), give consecutive codes, shift left when
  the length grows. This reproduces the [MS-XLDM] §2.7.4.2 example bit for bit (unit test).
- Bit stream: bit p is bit `15 − p mod 16` of the little-endian `u16` at byte `2 × ⌊p / 16⌋`
  (MSB-first within byte-swapped pairs).
- Single charset mode: each decoded byte b is the UTF-16 unit `charset × 256 + b`. Multiple
  mode: decoded bytes are UTF-16LE pairs.

Pages (engine-written dictionaries, [`vertipaq-large.md`](vertipaq-large.md)): a page holds
strings while their UTF-16 units, terminators included, stay **below 2^19**; the engine
Huffman-codes a page of **8 192 units or more** (8 190 stayed plain; a page of one distinct symbol
stays plain at any size; a 5-string last page stayed plain).

Also seen (`rare` case): one-value dictionaries (string, int64, boolean), a 100 000-character
string next to 2-character ones, `""` next to blanks (a distinct value), supplementary-plane
characters (surrogate pairs, stored as two units), int64 values near ±2^62 (8-byte elements).

Verification: the sample's 20 dictionaries are uncompressed; 130 engine-written compressed pages
(single and multiple character sets, 1-bit to 8-bit codes, up to 96 pages per dictionary) decode
to the values the engine reports (DAX aggregates over every row), and the encoder reproduces
them byte for byte.

---

## 8. `.hidx` (§2.3.3)

```
i32 algorithm (−1)   u32 entry size (8)   u32 bin size   u32 local entries per bin
u64 bin count        i64 record count     i64 current mask (bin count − 1)
u8  has_stats
[stats: u64 × 7 (elements, bins, used bins, fast-access elements, locals per bin, max chain,
 histogram length), u32 histogram element size, histogram]
bin × bin_count:  u64 chain, u32 used count, (u32 hash, u32 key) × local entries, u32 padding
u64 overflow count, (u32 hash, u32 key) × overflow count
```

bin size = 16 + 8 × local entries (observed 64 = 6 locals). A bin's `used count` is the number
of entries that hash to it; past the local entries they sit in the overflow list, which holds
every full bin's extra entries bin by bin (key order within a bin). Record count = sum of the
counts = local + overflow entries (the reader checks both). Bins: the smallest power of two, at
least 16, with `records ≤ 4.5 × bins` (16 to 8 388 608 bins seen). Stats present with an empty
histogram. The entries are the column's distinct data ids, **the blank id 2 included** (key 0).
Keys are data ids. The hash function (§2.3.3.1.3) is not implemented; the index is not needed to
read values. `.hidx` files exist only for value-encoded columns whose attribute hierarchy is
materialized (`AttributeHierarchyStorage.MaterializationType` 1); 4 of 8 value-encoded
non-row-number columns had one.

System tables: `H$<table>$<column>` hold `POS_TO_ID` / `ID_TO_POS` (NoSplit<32>, data ids and
positions as raw values; their `SegmentMapStorage` has `RecordsPerSegment` = table rows and
`RecordCount` = rows + 3, giving a second 3-row segment), `R$…` hold `INDEX`, `U$…` the
user-hierarchy files. Their `DictionaryStorage` rows have `Type` 0, `DataType` 19.

---

## 9. Verification on local data (counts only)

On the sample backup (8 user tables, 43 system tables):

- All 132 `.idf`/`.idfmeta` pairs decode (user and system); all 132 `.idfmeta` round-trip byte
  for byte; all 36 dictionaries and 4 `.hidx` parse; both backups give the same counts.
- All 132 columns with storage decode through the metadata joins, 52 of them user columns
  (8 RowNumber), 87 447 cells.
- Oracle pbixray 0.15.5 on the same PBIX: 8 tables, 44 non-RowNumber columns. **40 columns
  match on every cell** (58 112 cells: strings, int64, booleans, dates, value-encoded int64 and
  two value-encoded decimals). The other 4 columns (19 034 cells) are value-encoded decimals
  without an attribute-hierarchy `.hidx`; pbixray returns blanks for them (it only applies
  BaseId/Magnitude when a `.hidx` exists), so they could not be compared. Our values for them
  follow the same formula verified on the two decimal columns that do match.
- The distinct-value count of our output equals `ColumnStorage.Statistics_DistinctStates` for
  all 36 hash-encoded columns and the 10 value-encoded int64 columns; the min/max of 24 numeric
  columns equal `AttributeHierarchyStorage.MinValue/MaxValue` (the only ones that carry them as
  plain numbers).

---

## 10. What a writer must produce (M5)

Given a column's values (all partitions), a writer emits:

1. **Choose the encoding.** Strings → hash. Numbers: hash (int dictionary, `i32` elements when all
   fit) or value encoding. Value encoding needs integers (`DataType` 6): pick
   `BaseId = min − 3` (so the smallest value gets id 3) and `Magnitude` 1, or a power of ten
   `10^−k` when all values are multiples of `10^k` (decimals: scale by 10 000 first). Dates →
   hash with a real dictionary. Booleans → hash, int dictionary `[0, 1]` subset.
2. **Assign data ids**: blank → 2; hash: distinct values in any order → 3, 4, …; value:
   `(value × Magnitude) − BaseId`.
3. **`.dictionary`** (hash only): §7 layout with the observed hash headers; for strings, pages
   may all be uncompressed (allocation = 2 × used chars, remaining 0, NUL after each string,
   set `contains_nulls` when an empty string is present, store flag 1). Huffman pages are
   optional; if written, use a complete length-limited (2–15) canonical code and
   `decode_bits` = min(max length, 12).
4. **`.idf`**: per segment, a hybrid RLE primary block (≥ 16 entries, zero padded) and a
   sub-segment block of `ceil((n + 1) / (64 / N))` words. Simplest valid form: one bit-packed
   entry `(0xFFFFFFFF, rows)` and all rows bit-packed relative to `min_data_id` (or 2 when the
   segment has blanks). RLE runs (spec: length ≥ 64) are an optimization. RowNumber columns:
   inner class XM123, entry `(0xFFFFFFFF, rows)`, empty sub-segment.
5. **`.idfmeta`**: §3 with `records`, classes, `base_id` 1, the RLE facet
   (`alloc` = 2 × allocated entries, `used` = 2 × (used entries + 1), or 2 × used for XM123),
   `first_run_value` = min id, `SS` (`distinct_states` 0, min/max ids, `original_min` 2 (44 of
   52 observed segments; the rule for 3 is unknown), `rle_sort_order` −1, `row_count`,
   `has_nulls`, `rle_runs`), the sub `CS` with the
   packed count, and `SDOs` with each block's offset and size. `ColumnMeta::to_bytes` writes it.
6. **`.hidx`**: only for value-encoded columns with a materialized hierarchy. Whether Desktop
   rebuilds it (and the H$/R$/U$ system tables) on load is an open question; writing them
   requires the §2.3.3.1.3 hash function.
7. **`metadata.sqlitedb` rows** (observed values for an import table):
   - `ColumnStorage`: `Name` (`<column> (<Column.ID>)` for most columns), `StoragePosition`
     (unique per table), `DictionaryStorageID`, `Settings` (1; 2049 for calculated columns),
     `ColumnFlags`, `Statistics_*` (DistinctStates, Min/MaxDataID = idfmeta SS, RowCount,
     HasNulls, RLERuns, XMType 0 long / 1 real / 2 string, DBType as OLE DB type: 20 I8,
     130 WSTR, 7 DATE, 6 CY, 11 BOOL).
   - `DictionaryStorage`: `Type` (1 hash / 2 value), `DataType` (2/6/8), `DataVersion` 1,
     `BaseId`, `Magnitude`, `LastId` (largest id), `IsNullable`, `IsUnique`, `IsOperatingOn32`
     (1 for `i32` long dictionaries), `DictionaryFlags` (3 for strings, else 0),
     `StorageFileID` (0 for value encoding), `Size` (in-memory size, not the file size).
   - `ColumnPartitionStorage`: `DataVersion` 1, `State` 1, `SegmentStorageID`, `StorageFileID`.
   - `SegmentStorage`: `SegmentCount`, `StorageFileID` (the `.idfmeta`).
   - `SegmentMapStorage` per partition: `Type` 3, `RecordCount` = rows, `SegmentCount` 1,
     `RecordsPerSegment` = rows (single segment). A partition of several segments has `Type` 1
     (§11, "Segment map type").
   - `PartitionStorage` (`StoragePosition`, `SegmentMapStorageID`, `StorageFolderID`),
     `TableStorage` (`Version` 1, `Settings` 4353, `StorageFolderID`).
   - `StorageFolder` (`OwnerType` 18 table folder `<T> (<id>).tbl`, 20 partition folder
     `<tbl>\<n>.prt`) and `StorageFile` (`OwnerType` 22/23/24/27, file names as in
     [`abf.md`](abf.md)).
   - `AttributeHierarchy` / `AttributeHierarchyStorage` (materialization, H$ system table ID,
     min/max text, distinct count).

---

## 11. Encoder (M5.2, `bicraft_vertipaq::encode`)

Status: implemented 2026-10-13. Sources: the observations of §9 (same two backups, counts only),
[MS-XLDM] §2.7.3.1 (RLE runs ≥ 64) and §2.3.3.1.3 (hash-index hash).

API: `encode_column(values: &[CellValue], LogicalType, &EncodeOptions) -> EncodedColumn`
(`idf`, `idfmeta`, `dictionary: Option`, `hidx: Option`, `stats: ColumnStats`) and
`encode_row_number(rows, &EncodeOptions)`. `ColumnStats` carries what the storage rows need
(§10.7): DictionaryStorage `Type`/`DataType`/`BaseId`/`Magnitude`/`LastId`/`IsNullable`/
`IsUnique`/`IsOperatingOn32`/`DictionaryFlags`/`Size` (when known), ColumnStorage
`Statistics_*` (distinct states, min/max ids, RLE runs, has nulls, XMType, DBType, original
min), `RecordsPerSegment`, segment count and per-segment facts. Rows are capped by
`EncodeOptions::max_rows` (default 2^26); errors, never panics.

### Rules (all reproduced byte for byte on the sample and on the large engine-written models of [`vertipaq-large.md`](vertipaq-large.md))

| Item | Rule |
|---|---|
| Encoding (`Auto`) | decimal → value encoding when the ids fit in 32 bits; int64 → value encoding when the column has **at most 20 rows per distinct value** and every id stays below 2^31, else hash; everything else → hash. Overridable (`EncodingRequest::Hash` / `Value { params }`). The int64 rule comes from an engine sweep (`rare` case of [`vertipaq-large.md`](vertipaq-large.md): 15, 16.7 and 20 rows per value value-encoded, 22, 24, 25, 26, 28, 30, 33, 40 and 100 hashed, a 4·10^9-wide range of distinct values hashed, ranges of 128× the distinct count value-encoded) and reproduces the choice of all 117 engine-written int64 columns without a hint outside relationships, and of the Desktop sample's (ratios 9.3 and 14.1 value, 30.5 and more hashed). Relationship ends are hashed (the "one" end must be; the engine also hashed the "many" ends seen). |
| Hash ids | blank → 2; distinct values get 3, 4, … in **first-occurrence order of the stored rows**. Exception: Desktop re-sorts a table's rows after building the dictionary (one table of the sample, rows sorted for RLE), so its dictionaries follow source order instead. `EncodeOptions::dictionary_seed` pre-assigns ids (e.g. a column's existing dictionary). |
| Value parameters | `Magnitude` = `10^-k` for the largest `k` such that `10^k` divides every physical value (decimals: the value × 10 000), `BaseId` = min(value × Magnitude) − 3. Matches all 8 value-encoded columns. Doubles (`EncodingRequest::Value` only): `Magnitude` = `10^k` for the smallest `k` ≤ 15 making every value an exact integer, `DataType` 8, as the engine writes them (k = 4 seen for `Value / 10000`). With given parameters, a double column may also have `Magnitude` 10^−k (k ≤ 15): the engine stored `Value * 1E+15` with `Magnitude` 10^−12, `BaseId` 997 (id = value / 10^12 − BaseId); `Auto` never picks k < 0 (when the engine does is unknown). |
| Long dictionary | 4-byte elements (`IsOperatingOn32` 1, hash header `(−1, 8, 64, 6)`) when every value fits `i32`, else 8 (header `(−1, 16, 64, 3)`). Booleans: long `0/1`; dates and doubles: real. |
| String pages | a new page when the next string would bring the page to 2^19 units (`max_page_chars` 2^19 − 1, terminators included); `StringCompression::LargePages` (default) Huffman-codes a page of ≥ 8 192 units with at least two symbols (plain Huffman lengths, canonical code, `mask` 1, buffer = allocation = `2 × (ceil(bits / 16) + 1)`); `store_compressed` 1, `contains_nulls` = page holds `""`, `longest_string` in UTF-16 units without the terminator. U+0000 inside a string is rejected (it is the terminator). |
| Segments | 2^20 rows (verified: engine tables of 1M–20M rows in 1–3 partitions; every partition restarts its segments, the last one short; one `.idf`/`.idfmeta` per column partition). |
| Segment map type | `SegmentMapStorage.Type` of a user partition: 3 with one segment, **1 with several** (see below). `RecordCount` is the partition's rows, `SegmentCount` its segments, `RecordsPerSegment` the full segment size (2^20); none of them is per segment, and `GeneralSegmentMapSegmentMetadataStorage` stays empty. `H$` tables keep 2, `R$`/`U$` 3. |
| RLE entries | maximal runs of ≥ `rle_min_run` (64) equal ids become `(id, count)` entries; every stretch between them becomes one bit-packed entry. The sample segments are all-RLE (runs ≥ 90) or all-bit-packed (runs ≤ 31). |
| Primary block | allocated entries = max(16, next power of two > used entries); zero padded. Exceptions, all on segments whose rows the engine re-sorted for RLE: 32 allocated for 14 used (`Million[Mod7]`), and 32 for 15 or 14 used on the three smallest partitions (1 642–1 696 rows) of the incremental-refresh table's 21-value text column (its larger partitions follow the rule). |
| Columns of blanks | hash encoding without `.dictionary`, `original_min_segment_data_id` 0x8000_0000, `DistinctStates` 1, `IsOperatingOn32` 1 for int64, observed `Size` constants (§1). |
| Empty partitions | the engine's empty segment (§1): `CS.base_id` 0, `bookmark_bits` 1, `SS` ids 2; row numbers keep one RLE entry of count 0. |
| Sub-segment | width: smallest allowed N with 2^N > max(max − base, 1), base = 2 with blanks else min id; values `id − base`; `ceil((packed + 1) / (64 / N))` words. |
| `.idfmeta` | `CS.base_id` = `EncodeOptions::segment_base_id` (1 on calculated-table and imported columns, 0 on calculated columns; meaning unknown; RowNumber 0); `storage_alloc_size` 2 × allocated, `storage_used_size` 2 × (used + 1); `first_run_value` = the packing base (min id, or 2 with blanks); `SS.original_min_segment_data_id` **3 for value encoding, 2 for hash** (ColumnStorage's copy is 2 for all); `rle_runs` = RLE entries, `others_rle_runs` = bit-packed entries. |
| `bookmark_bits` | smallest `b` with `2^b × (entries + 1) ≥ 10 × (records + 1)` (401 distinct pairs of engine segments, 1–1 048 576 rows, 1–616 entries); XM123 segments: 24. `first_run_value` = the packing base (2 with blanks). |
| RowNumber | XM123 inner class, one entry `(0xFFFFFFFF, rows)`, empty sub-segment, `bookmark_bits` 24, `storage_used_size` 2, value encoding `BaseId` −3 / `Magnitude` 1; partition k starts at id 3 + earlier rows (`encode_row_number_from`). |
| `.hidx` | (value encoding, `build_hidx`) one entry per distinct id: `hash` field = the data id, `key` = rank in ascending id order; **bin = top `log2(bins)` bits of `id × 0x12B9B6A5 mod 2^32`** (the §2.3.3.1.3 hash; §8's "hash not implemented" is superseded), the blank id 2 included (key 0); bins: smallest power of two ≥ 16 with entries ≤ 4.5 × bins; bin count = entries hashing to the bin, the first 6 local (key order), the rest in the overflow list bin by bin; stats flag 1 with all-zero statistics and an empty 8-byte histogram. Verified on 46 engine files of 3 to 10 000 000 entries. |

Statistics fitted on the sample and checked on 1 157 large engine-written columns:
`Statistics_DistinctStates` = distinct count; value encoding with `.hidx` → distinct ids, blank
included; without `.hidx` → id range; `Statistics_RLERuns`/`OthersRLERuns` = sums over every
segment of every partition (`ColumnStats::rle_runs()`), min/max ids and `HasNulls` over the whole
column; `IsUnique` 1 only for hash-encoded columns with a 4-byte long dictionary whose rows are all
distinct; `IsNullable` 1 for value encoding; dictionary `Size` = 144 (value), else the formulas
of "Dictionary size" below. DBType of double (5, R8) is assumed.

### Dictionary size (G17, 2026-10-15)

`DictionaryStorage.Size` is the engine's in-memory size of the dictionary, which the engine
recomputes when it loads a file (the stored value is not checked) and writes back on the next
save. Derived from 702 engine-written long and real dictionaries of the large-model images
([`vertipaq-large.md`](vertipaq-large.md)) and from a sweep of our own (one calculated table of
100 000 rows, doubles `MOD(v, k) + 0.5`, `MOD(v, k) / 7`, dates, 8-byte longs `MOD(v, k) × 10^10`
and 4-byte longs, k from 1 to 4 000 around every growth point), read with
`DISCOVER_STORAGE_TABLE_COLUMNS.DICTIONARY_SIZE` after the refresh, after `ImageSave` +
`ImageLoad` of the engine's image, and after `ImageLoad` of the PBIX BIcraft writes for the same
model (Desktop 2.158.1304.0, engine 17.0.87.18).

BIcraft's model of these sizes, fitted to every reading above: the sizes behave as
if the engine held a hash table of 64-byte bins at load, the [MS-XLDM] §2.3.3.1.4 layout (a 12-byte
bin header, then local entries): a 4-byte long entry (key, data id) is 8 bytes, 6 to a bin;
an 8-byte long or real entry is 12 bytes, 4 to a bin. The table has at least 16 bins and doubles
while it holds more than 3/4 of its local entries (n > 4.5 × bins for 4-byte longs, n > 3 × bins
for 8-byte keys). Then:

| Dictionary | `Size` |
|---|---|
| long, 4-byte elements | 272 + 64 × bins + 4n |
| long, 8-byte elements | 272 + 64 × bins + 8n + 12 × collisions |
| real (double, date) | 232 + 64 × bins + 8n + 12 × collisions |

"Collisions" are the entries past the 4 local ones of their bin (overflow chain entries, 12
bytes each). The 4-byte formula matched all 460 engine dictionaries without a remainder (1 to
10 000 019 values): those never showed a collision cost. For 8-byte keys the remainder was a
multiple of 12 on all 242 dictionaries, zero on some (sequences of multiples of 1 000 003, of
10^10 up to 97 values, `j + 0.5` up to 49) and up to about 11 % of the size on others (whole
dates: 1 536 collisions for 3 000 consecutive days in 1 024 bins). The engine's hash of 64-bit
keys was not identified: none of the multiplicative (§2.3.3.1.3 constant on the low, high or
folded words), FNV, CRC-32C or mixer candidates tried reproduces the collision counts. BIcraft
writes the formula without collisions, a lower bound that is exact when there are none.

String dictionaries (fitted on the 267 engine-written string dictionaries of the large-model
images, 1 to 300 000 strings, plain and Huffman pages, and on three sweeps of our own around the
growth points, `"s" & MOD(v, k)` for k from 1 000 to 4 000 and 100-character strings):

| Part | Bytes |
|---|---|
| fixed | 688 |
| hash table | 64 × bins: at least 256 bins (the [MS-XLDM] §2.3.3.1.4.3 minimum), doubled while it holds more than 5.4 strings per bin (1 382 strings: 256 bins, 1 383: 512; 2 764: 512, 2 765: 1 024) |
| per string | 8 |
| plain page | 2 × its UTF-16 units, terminators included |
| Huffman page | its buffer size + 1 409 |
| collisions | 8 per string past the 6 local entries of its bin (8-byte entries: hash and data id) |

With 256 bins this is the former single-page formula (17 072 + 8n + 2 × units). Every remainder
over the formula without collisions was a non-negative multiple of 8 (0 for 4 strings of 100 000
characters on one Huffman page, which fixes the 1 409). The string hash (the
`XM_HASH_ALGORITHM_*` of the header; 0 on every engine file) is not identified, so BIcraft
writes the lower bound without collisions; right after a refresh the engine reports about 1 MiB
more for a string dictionary (`vertipaq-stats.md`), not the stored figure.

Right after a refresh (before a save and a load), a real dictionary of up to 48 values still has
the 32 bins processing starts with (1 024 bytes more; 8-byte longs start at 16); the engine
writes that figure in its own `ImageSave`, and reports the 16-bin figure once the image is loaded.
BIcraft writes the loaded figure, the one Desktop has for the file and saves next. Before this
fix the writer stored the `.dictionary` file size for real and 8-byte long dictionaries and for
multi-page or Huffman string dictionaries (88 bytes for a 6-value double column whose engine
figure is 1 304; 2 412 for 100 strings of 100 characters, engine 20 669) and 1 296 + 4n for
4-byte longs past 72 values (1 696 where the engine has 2 720 for 100 values).

Check (2026-10-15): for the 39-column sweep model, the engine's figure after loading the PBIX
BIcraft wrote equals its figure after loading its own image of the same model, column for
column; BIcraft's stored `Size` equals it for every 4-byte long, every string or numeric
dictionary without collisions, and is below it by exactly the collision bytes elsewhere.

### Segment map type of a multi-segment partition (G50)

Black box, Power BI Desktop 2.160.1151.0 (Store edition), 2026-11-25, synthetic one-column
tables (int64 `qty`, nine values) loaded from our own CSV files.

- Desktop's refresh of a BIcraft PBIX to 10,000,000 rows wrote `SegmentMapStorage` `Type` 1,
  `RecordCount` 10,000,000, `SegmentCount` 10, `RecordsPerSegment` 1,048,576 for the table's
  partition, and no `GeneralSegmentMapSegmentMetadataStorage` row; tables of one segment
  (1,048,576 rows and fewer) keep `Type` 3. Every other storage row and file of that backup
  matched BIcraft's 10,000,000-row file apart from the data-dependent ones (the `.idf`/`.idfmeta`
  of `qty`, which Desktop had sorted for RLE, and its `Statistics_RLERuns`/`OthersRLERuns`); the
  row-number `.idf`/`.idfmeta`, the dictionary and the `H$` files were byte-identical.
- Desktop refuses on open a PBIX whose multi-segment partition has `Type` 3 ("Échec des
  vérifications de cohérence de la base de données (DBCC) lors de la vérification des segments
  de données"). Changing only that value to 1 in BIcraft's `metadata.sqlitedb` (1,048,577 rows in
  two segments; 10,000,000 rows in ten), the backup otherwise unchanged and repacked, made both
  files open with their row counts.
- The writer now picks the type from the segment count (`SegmentMap::partition`,
  `crates/metadata/src/structural/data.rs`), per partition of a table of several partitions too.
  Opened after the fix: PBIX files `bicraft run` built with 1,048,577, 2,097,153 and 10,000,000
  rows, and one `bicraft refresh` grew from 1,000 to 1,200,000 rows, each with a card and a table
  of `qty`, `COUNTROWS` and `SUM` measures equal to the expected counts and sums.

### Verification (local, counts only)

`tests/encode_real.rs` reads extracted column files from `BICRAFT_VERTIPAQ_DIR` (manifest
format in the test) and skips when unset. On the sample (52 columns, 44 non-RowNumber), every
column round-trips (`decode(encode(x)) == x`), and:

- with the original dictionary order as seed and the original value parameters: **52/52
  columns byte-identical** in all four files (52 `.idf`, 52 `.idfmeta`, 36 `.dictionary`,
  4 `.hidx`); deriving the value parameters instead gives the same bytes;
- with no hints (first-occurrence order): 47/52 (39/44 non-RowNumber). The 5 others are the
  re-sorted table's hash columns: same `.idfmeta`, different id numbering in `.idf` and
  `.dictionary`.
- every compared metadata statistic matches, except the real-dictionary `Size` of 2 columns
  (hash collisions, see "Dictionary size").

`tests/encode.rs` covers every logical type, Unicode, blanks, all-blank and empty columns,
all-equal columns, mixed RLE/bit-packed segments, 100 000 distinct values, multi-segment
columns (including > 2^20 rows), multi-page and Huffman string pages, dictionary seeds,
`.hidx` growth and rejected input.

### Encoder open questions

1. ~~Int64 value-versus-hash choice without `encodingHint`~~: at most 20 rows per distinct
   value and ids below 2^31 (rule table above). Open: the exact threshold between 20 and 22,
   whether blanks count, and decimals/doubles of many rows per value (the encoder keeps
   value encoding for decimals). Row re-sorting before RLE (the encoder keeps the given order)
   and the RLE allocation of re-sorted segments.
2. `DictionaryStorage.Size`: settled for long, real and string dictionaries up to the hash
   collisions (12 bytes per collision of an 8-byte key, 8 per string): the engine's hashes of
   64-bit keys and of strings are not identified ("Dictionary size"). A load does not need the
   size to be exact.
3. Pages of exactly 8 191 units; `decode_bits` above 12 (never seen; the encoder writes
   min(longest, 12)).

---

## Open questions

1. ~~Real compressed (Huffman) string pages~~: answered by the large-model oracle (§7).
2. `CS.base_id` (1 on hybrid segments), `others_rle_runs`, `SS.distinct_states` (always 0)
   and `original_min_segment_data_id` (2 vs 3): meanings. (`bookmark_bits` now has a formula
   reproducing every engine segment, §3.)
3. ~~Multi-segment and multi-partition columns~~: 2^20-row segments, one file pair per column
   partition, blank-holding segments pack relative to id 2 (verified on engine tables of up to
   20M rows, [`vertipaq-large.md`](vertipaq-large.md)).
4. Ids 0 and 1 in user columns (never observed, including columns of blanks, one-value
   columns, int64 near ±2^62 and 100 000-character strings; the reader rejects them as out of
   range for hash encoding and maps them arithmetically for value encoding).
5. ~~`Statistics_DistinctStates` of value-encoded columns~~: distinct ids with blank when the
   column has a `.hidx`, the id range without (§11).
6. Whether the engine accepts files without `.hidx`, H$/R$/U$ system tables and RLE (writer).
   Known: it checks `H$` consistency on load and trusts `R$` as stored.
7. `XMRENoSplit` widths: every width of the spec (1–10, 12, 16, 21, 32) has now been seen in
   engine-written segments.
8. `General` compression (703063) and any other class ids: still never produced (the `rare`
   and `incr` models added columns of blanks, one-value columns, pure-RLE and alternating
   columns, huge strings and empty partitions; every segment was hybrid RLE with the known
   `XMRENoSplit` widths or XM123).
9. Hybrid tables (an import + DirectQuery refresh policy): the engine materializes no `H$`,
   `U$` or `R$` for the table's columns, hierarchies and outgoing relationships
   (`MaterializationType` −1, `RelationshipIndexStorage.RecordCount` 0). BIcraft's structural
   writer still builds them for a new column of such a table; the engine loads that, but does
   not need it.

[xldm]: https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-xldm/8c62e8ce-f605-488d-81e9-4ecdb7686a52
[ksy-idf]: https://github.com/Hugoberry/pbixray/blob/main/docs/idf.ksy
[ksy-idfmeta]: https://github.com/Hugoberry/pbixray/blob/main/docs/idfmeta.ksy
[ksy-dict]: https://github.com/Hugoberry/pbixray/blob/main/docs/dictionary.ksy
