# VertiPaq on large models: engine oracle (G10)

Status: 2026-10-14. Companion to [`vertipaq.md`](vertipaq.md) (column files),
[`vertipaq-system-tables.md`](vertipaq-system-tables.md) (`H$`/`R$`/`U$`), [`abf.md`](abf.md)
and [`xpress9.md`](xpress9.md).

**Sources.** Observations only: synthetic models of ours, processed by the Analysis Services
engine of Power BI Desktop Store 2.158.1304.0, saved with AMO `ImageSave`, read back with BIcraft
and compared with the engine's own DAX answers. No Microsoft text
is recorded. The models' data are generated (CSV files, or DAX calculated tables).

## 1. How

The engine runs are done with the maintainers' development oracles, kept outside this repository
(`BICRAFT_ORACLES_DIR` points to them). For each model they generate the data, deploy and fully
refresh the model, save its image and record DAX aggregates; then they load images back, extract
column files, replace files in the ABF and re-compress it, and run BIcraft's metadata-only and
structural saves (`incr`/`hybrid`), each time comparing the engine's answers.

| Piece | Role |
|---|---|
| `crates/vertipaq/tests/large.rs` | Env-gated tests over `$BICRAFT_TESTDATA/large/<case>/`, opt-in with `BICRAFT_LARGE=1` (slow: run in release, see its header) |

Imported partitions were loaded from the generated CSV files through a text OLE DB provider; M
partitions were not used in this setup.

Aggregates recorded per column: row count, blank count (`ISBLANK`), `DISTINCTCOUNT`, `MIN`,
`MAX`, sum, sum of `LEN` (strings), count of `TRUE` (booleans) and a key-weighted checksum
`SUMX(T, MOD(T[key], 1009) * f(T[c]))` that ties every value to its row. Plus, per model, queries
through relationships and hierarchies (`RELATED`, filters across relationships, `TOPN` over a
related column, `FIRSTNONBLANK`).

## 2. Models

| Case | Content | ABF | Files | Image |
|---|---|---|---|---|
| `many` | 200 calculated tables (10–3 009 rows, int/string/double columns, 200 chained relationships), `Million` (1 000 000 rows: int, string, currency, date, boolean, int with 50 % blanks; a 2-level hierarchy), `Wide` (20 000 rows × 120 columns) | 113 MB | 7 235 | 25 MB, single-threaded framing, 55 chunks |
| `fact` | `Fact` 5 000 000 rows in 3 partitions (2M/2M/1M) × 13 imported + 2 calculated columns: value- and hash-encoded int64 (`encodingHint`), 5M distinct strings, low-cardinality strings with blanks, medium-cardinality text, currency, double, date with blanks, boolean, int64 beyond 32 bits; `Product` 20 000 rows (3-level hierarchy), `Customer` 300 000 rows in 2 partitions (2-level hierarchy), a calculated table; relationships on a value-encoded int and on a string key | 578 MB | 267 | 351 MB, multithreaded framing |
| `big20` | `Big20` 20 000 000 rows, 1 partition (20 segments): value-encoded key, 1 000-value int, 10M-value hash int | 845 MB | 27 | 345 MB, multithreaded framing (10 sessions, 404 chunks) |
| `probe` | String-page probes: the compression threshold, multiple character sets, a 1-bit code, a single-symbol page, a dictionary whose last page holds 5 strings | 3 MB | 97 | 0.9 MB |
| `incr` | Incremental refresh (§5): `Sales` 60 000 rows with a refresh policy (rolling 3 years, incremental 3 months) and 8 policy-range partitions (2 years, 3 quarters, 3 months), a calculated column, a 2-level hierarchy, a relationship to `Product` (2 M partitions, 3-level hierarchy) | 5 MB | 221 | 2 MB |
| `hybrid` | `incr` with the policy in hybrid mode: the last month is a DirectQuery partition (no calculated column: the engine refuses one on a hybrid table) | 1 MB | 170 | 0.5 MB |
| `rare` | Rare layouts (§5): columns of blanks of every type, one-value columns, 4 000-character and 100 000-character strings, empty strings next to blanks, supplementary-plane characters, int64 near ±2^62, pure-RLE and alternating columns, doubles scaled by 10^4 and by 10^−12, date-times with minutes, a calculated column on a calculated table, and int64 sweeps (1 000–30 000 rows) for the hash-versus-value choice | 7 MB | 421 | 1.2 MB |

Local data: `$BICRAFT_TESTDATA/large`, 2.2 GB (image + extracted column files + JSON per case),
never committed.

## 3. Results

### Reader

Every column of every case decodes (3 151 + 82 + 9 + 39 stored columns, user and system, 253M
cells) and **all 968 user columns match the engine on every aggregate**: multi-segment and
multi-partition columns, RowNumber columns spanning partitions, 5M-string dictionaries of 96 pages,
130 Huffman-compressed pages (single and multiple character sets), value-encoded doubles, 8-byte
long dictionaries, blanks in every type.

Two reader bugs were found and fixed:

1. **`.hidx` bins with more entries than local slots.** A bin's count is the number of entries
   that hash to it; when it exceeds the local entry count (6), the extra entries are in the
   overflow list. The reader rejected such files (`bin claims 7 of 6 entries`). It now accepts
   them and checks that the overflow list holds exactly the spilled entries.
2. **1-bit Huffman codes.** The engine gives a dominant symbol a 1-bit codeword (`0` in
   `x00000`…`x01169`); the reader only accepted lengths 2–15. Lengths are now 1–15.

### Encoder (re-encoded with the engine's dictionary order and value parameters)

| | many | fact | big20 | probe |
|---|---|---|---|---|
| `.idf` identical | 1 128/1 129 | 65/65 | 4/4 | 15/15 |
| `.idfmeta` identical | 1 128/1 129 | 65/65 | 4/4 | 15/15 |
| `.dictionary` identical | 891/891 | 18/18 | 2/2 | 11/11 |
| `.hidx` identical | 36/36 | 7/7 | 1/1 | 2/2 |

The one difference (`Million[Mod7]`, rows re-sorted by the engine into 7 runs) is the RLE
allocation: 32 entries allocated for 14 used, where the rule gives 16. Encoder fixes that got there (rules in `vertipaq.md` §11):
`bookmark_bits`, `first_run_value` with blanks, the `.hidx` bin growth / blank entry / overflow
order, value-encoded doubles, 8-byte long dictionary headers, RowNumber ids continuing across
partitions, string page size, the page compression rule and Huffman page layout,
`Statistics_DistinctStates` with a `.hidx`, `IsUnique`, and whole-column RLE run counts.

Statistics compared with the `ColumnStorage`/`DictionaryStorage` rows of single-partition
columns: `DistinctStates`, `Min/MaxDataID`, `HasNulls`, `RLERuns`, `OthersRLERuns`, `RowCount`,
`LastId`, `IsNullable`, `IsUnique`, `IsOperatingOn32` all match (1 157 columns). `Size` (the
in-memory dictionary size) matches only for the formula's cases (783/1 157); it is not needed
(see below).

### System tables (rebuilt from the decoded user columns)

`H$` 1 888/1 890 (the two others: strings in Greek/CJK ranges, where BIcraft's collation is
approximate), `R$` 202/202 (including a value-encoded from-column and 300 000 string keys), `U$`
12/12, `DistinctDataCount` 968/968, `RelationshipIndexStorage.RecordCount` 202/202, all byte for
byte. One fix: the `R$` bit width follows the to-table's row count (`vertipaq-system-tables.md`
§3).

### Restore (AMO `ImageLoad` of rewritten images)

The `rewrite` test re-encodes **every hash-encoded column with the encoder's own choices**
(dictionary ids in first-occurrence order) and rebuilds every `H$`/`R$`/`U$` table from the new
ids, then replaces only the files whose bytes changed (`many`: 2 271 files, 567 columns; `fact`:
78 files, 12 columns including the 5M-string column and both relationship key columns; `big20`: 5
files, the 10M-value column), re-compresses the ABF and loads it. `metadata.sqlitedb` is left as
the engine wrote it (dictionary `Size` values now stale). **The engine loads all of them and every
aggregate and relationship query gives the original answer** (931 + 33 + 5 + 13 checks). This
was run on `many`, `fact` and `big20` with string pages written uncompressed (the encoder's earlier default,
so the 5M-, 300 000- and 100 003-string dictionaries were single uncompressed pages of up to
50M units, in place of the engine's compressed pages), and again on `many` and `probe` with the
final encoder (engine page split and Huffman pages); the `fact`/`big20` re-run with the final
encoder was cut short by the memory guard (other processes left under 2 GB free).

Negative controls (same `many` image):

- An `H$` whose `POS_TO_ID` order was reversed (inconsistent with `ID_TO_POS`): the load fails
  with a database consistency check error. An `H$` file whose second segment block was cut off:
  the load fails reading the segment's data object.
- An `R$` index mapping every key to the first row of the to-table: the load **succeeds** and
  relationship queries return the wrong answer. The engine trusts `R$` as stored: a writer must
  get it right (BIcraft's is byte-identical).
- An `H$` with a different (but self-consistent) order for CJK strings loads, and `MIN`/`MAX`
  stay correct.

### Timings and memory (this machine, release build)

| | many | fact | big20 |
|---|---|---|---|
| XPress9 decompression (BIcraft) | 0.4 s | 2.8 s (205 MB/s) | 3.1 s (276 MB/s) |
| ABF parse | 0.08 s | 0.03 s | < 0.01 s |
| Decode every column (test) | 35 s ¹ | 4.5 s, peak 0.8 GB | 4.2 s, peak 1.5 GB |
| Re-encode user columns | 2.5 s | 78 s, peak 1.5 GB | 34 s, peak 1.8 GB |
| Rebuild system tables | 2.8 s | 286 s ² | 22 s |
| Replace files + re-compress | 0.4 s | 3.1 s | 3.7 s |

¹ 12 000 small files opened (I/O and antivirus scanning dominate). ² `H$` of the 5M-string
column: the collation sort key is rebuilt on every comparison.

## 4. Format facts learned (now in the format notes)

- Segments: **2^20 rows** per segment (`RecordsPerSegment` 1 048 576) on imported and calculated
  tables alike; one `.idf`/`.idfmeta` pair per column **partition** holding all its segments as
  consecutive blocks (the file name ends in the partition's position, `.0.idf`, `.1.idf`, …); the last segment of each partition is
  short. `ColumnStorage.Statistics_*` aggregate every segment of every partition (sums of RLE
  runs, global min/max, any blank, total rows).
- The engine re-sorts rows within segments for RLE (a 1 000-value column became 1 000 runs per
  segment); the encoder keeps the given order, which is what the files store.
- `H$` tables keep `RecordsPerSegment` = member count even for 10M members (no 2^20 split).
- Encoding choices without a hint: int64 columns of small tables (≤ 3 009 rows) were hashed and
  those of 20 000+ rows value-encoded; doubles that become integers when scaled by 10^k
  (k ≤ 3 seen) are value-encoded with `Magnitude` 10^k and `DataType` 8.
- The XPress9 framing of `ImageSave` is single-threaded up to at least 113 MB and multithreaded
  from 578 MB (the threshold was not probed).

## 5. Incremental refresh and rare layouts (2026-10-14)

**How the incremental-refresh models were made.** The engine refuses a refresh policy next to
a provider (non-M) data source, and requires Power BI (V3) data access
(`defaultPowerBIDataSourceVersion` `powerBI_V3`), also on `ImageLoad`; M does not run on the
local engine instance. So `incr`/`hybrid` are deployed with query partitions of the final names on the
generated CSV file, refreshed, then converted with TOM in three saves: M sources (a policy-range
source set directly keeps the partition's data-source reference, which then blocks removing the
data source), the provider data source removed and V3 set, then policy-range sources (`Start`,
`End`, `Granularity`), the DirectQuery mode of the last partition (`hybrid`), `RangeStart`/
`RangeEnd` parameters and the refresh policy. Partitions keep their data (state Ready).

**Read.** Both decode like the engine (every user column's aggregates, relationship and
partition-filtered queries: `incr` 11 columns + 7 queries, `hybrid` the `Product` columns; the
hybrid table cannot be aggregated: queries reach its DirectQuery partition). Layout facts:

- One `.idf`/`.idfmeta` pair per column partition, the name ending in the partition's position
  (`.0` … `.7`); one dictionary and `.hidx` per column; RowNumber ids continue across the 8
  partitions. `Partition.Type` 6 (policy range) with `RangeStart`/`RangeEnd` (OLE dates) and
  `RangeGranularity` (3 year, 2 quarter, 1 month); the DirectQuery partition has `Mode` 1.
- Hybrid tables: the DirectQuery partition stores an empty segment per column (`vertipaq.md`
  §1, "Empty partitions"); the dictionary keeps the values that partition held when it was
  still imported; **no `H$`, `U$` or `R$`** for the table (`MaterializationType` −1, the
  relationship's `RelationshipIndexStorage` has `RecordCount` 0 and no system table). The
  engine refuses calculated columns on a hybrid table.

**Re-encode.** Every file byte for byte except the RLE allocation of three small re-sorted
partitions (`vertipaq.md` §11, "Primary block"); with the empty-segment rules, `hybrid`'s empty
partition files are identical too. System tables: `H$` 19/19, `R$` 1/1, `U$` 8/8 (`incr`).

**Keep and write (BIcraft saves, loaded by the engine).**

| Save | Content | Result (`incr`, `hybrid`) |
|---|---|---|
| metadata-only (`write_data_model`) | a measure and a table description added | loads; refresh policy (basic, import/hybrid), 8 policy-range partitions (and the DirectQuery one) intact; every aggregate and query unchanged; the measure answers |
| structural (`DataModelEditor`) | `Sales[Qty]` renamed (by lineage tag), the calculated column removed (`incr`), a new table with a relationship to `Product`, two new int64 columns on the 8-partition table (one hash-, one value-encoded) | loads; policy and partitions intact; every remaining column unchanged; the new columns hold their values in every partition (checked by DAX against `K`), the new relationship filters |
| re-encoded (`large.rs` rewrite) | every hash column re-encoded in BIcraft's order, `H$`/`R$`/`U$` rebuilt | `incr`, `hybrid`, `rare` load; 18 + 4 + 56 columns and queries identical |

The structural writer gained new columns on multi-partition tables for this
(`EncodedNewColumn::partitions`: one segment set per partition, encoded with the whole column's
dictionary order or value parameters; `ColumnStorage` RLE statistics summed over partitions).
Not supported: replacing the data of a multi-partition table (a replaced table gets one
partition, and is refused when it has several).

**Rare layouts (`rare`).** All 177 columns decode; all 52 compared user columns match the
engine (blanks, distinct counts, min/max, sums, checksums; int64 near ±2^62 by min/max only:
sums cancel in floating point). Facts now in `vertipaq.md`: columns of blanks (no
`.dictionary`), doubles stored with `Magnitude` 10^4 and 10^−12, 1-bit Huffman codes at 8
characters per byte (the reader's input bound was 4 per byte: fixed), the int64
hash-versus-value rule (at most 20 rows per distinct value, ids below 2^31; 117/117 engine
columns). Every segment was hybrid RLE (XM123 or a known `XMRENoSplit` width): no `General`
class, no data ids 0/1. Re-encoded byte for byte: 73/73 `.idf`/`.idfmeta`, 31/31 dictionaries,
30/30 `.hidx` (with the engine's dictionary order); `DictionaryStorage.Size` 61/73.

Fixes from this round: missing `.dictionary` of columns of blanks (reader, `bicraft-pbix`
column reads, preview), the Huffman output bound, the encoder's columns of blanks, empty
partitions and negative double magnitudes, the int64 `Auto` choice, multi-partition column adds.

## 6. Still open

- When the engine allocates more RLE entries than used + 1 (row re-sorting).
- `DictionaryStorage.Size`: settled on 2026-10-15 up to hash collisions (`vertipaq.md`,
  "Dictionary size"; not needed for a load to succeed).
- Pages of exactly 8 191 units (8 190 stay plain, 8 192 are compressed).
- The exact int64 threshold (between 20 and 22 rows per distinct value) and whether blanks
  count; the single-/multithreaded framing threshold.
- When the engine picks a negative double magnitude without parameters (10^−12 for multiples
  of 10^15).
- Replacing the data of a multi-partition (incremental-refresh) table; whether a new column of
  a hybrid table should skip its `H$` like the engine's columns (the engine loads either).
- Collation of strings outside Latin/Greek/Cyrillic in `H$` (approximate).
