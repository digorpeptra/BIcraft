# VertiPaq system tables: `H$`, `R$`, `U$` (as implemented in `crates/vertipaq/src/system`)

Status: builders implemented and verified, 2026-10-13 (milestone M5.3). Companion to
[`vertipaq.md`](vertipaq.md) (column files), [`abf.md`](abf.md) (file inventory and the
`metadata.sqlitedb` schema) and [`datamodel-abf.md`](datamodel-abf.md) (sources).

**Sources.**

- [MS-XLDM] rev 8.3, summarized, never copied: §2.4.1 position-to-identifier map
  (`POS_TO_ID`), §2.4.2 identifier-to-position map (`ID_TO_POS`), §2.4.3 relationship index
  (`INDEX`), §2.4.4 and §2.4.4.1–§2.4.4.4 user-hierarchy files; file naming in §2.2.3.7.2.2–
  §2.2.3.7.2.7. The spec describes the Excel flavor; the Power BI differences are noted below.
- **Observations, real file**: the local sample backup (Desktop Store 2.158.1304.0, culture
  fr-FR, 38 `H$`, 4 `R$`, 1 `U$`). Held locally, never committed: this note records counts and structure only.
- **Observations, oracle**: synthetic models of ours (calculated tables only: strings with
  accents, case variants, punctuation and other scripts; numbers, dates, booleans and decimals
  with blanks; relationships with missing keys, case-variant string keys, one-to-one; a
  three-level user hierarchy with blanks and a sort-by column). They were deployed to the
  Analysis Services engine of Desktop (the maintainers' development oracles), then
  - saved with AMO `Server.ImageSave` (the instance runs in diskless mode, where TMSL `backup`
    is refused; `ImageSave` returns the same XPress9-framed ABF as a PBIX `DataModel`), and
  - listed with MDX (`SELECT [T].[C].[C].MEMBERS ON 0 FROM [Model]`), whose member order is
    the attribute-hierarchy order, for collation probes (fr-FR, en-US, and an explicit
    `Latin1_General_CS_AS` model collation).

API (`bicraft_vertipaq::system`): `build_attribute_hierarchy[_with]`,
`build_relationship_index`, `build_user_hierarchy`, `Collation`, and
`SystemTableData::encode` / `encode_column` (→ `.idf` + `.idfmeta` bytes).

---

## 1. Where they live

System tables are ordinary rows of `Table` with `SystemFlags` bit 0 set, `ModelID` 0 and
`IsHidden` 1, each with one partition and one storage folder (names: [`abf.md`](abf.md) §3):

| Table name | Owner | Columns (storage position order) |
|---|---|---|
| `H$<table> (<id>)$<column> (<id>)` | `AttributeHierarchyStorage.SystemTableID` | `POS_TO_ID`, `ID_TO_POS` (hash-encoded columns only) |
| `R$<from table> (<id>)$<relationship name, '-' → ' '> (<rel id>)` | `RelationshipIndexStorage.SystemTableID` | `INDEX` |
| `U$<table> (<id>)$<hierarchy> (<id>)` | `HierarchyStorage.SystemTableID` | `MULTI_LEVEL_ID`, `PARENT_POS`, `FIRST_CHILD_POS`, `CHILD_COUNT` |

Which columns get an `H$` (`AttributeHierarchyStorage.MaterializationType`):

| Type | When | System table |
|---|---|---|
| 0 | hash-encoded column | `POS_TO_ID` + `ID_TO_POS` |
| 1 | value-encoded column | `POS_TO_ID` only, plus a `.hidx` in the user table folder |
| 3 | RowNumber column | none (`SystemTableID` 0) |
| −1 | `Column.IsAvailableInMDX` = 0 | none; `DistinctDataCount` −1, `DataVersion` 0 |

---

## 2. `H$`: attribute hierarchy

Let *members* be the column's distinct data ids (blank, id 2, included when present) in sort
order, *M* their number and *N* the number of non-blank members.

- **`POS_TO_ID`** (`ColumnStorage.Settings` 7): *members*, then zeros up to **N + 3** records.
  So with a blank the table ends with two zeros, without one with three.
- **`ID_TO_POS`** (`Settings` 5): indexed by data id, `LastId + 1` records:
  `[0]` = 0, **`[1]` = M**, `[id]` = position of `id` (so `[2]` is the blank's position, 0 when
  there is no blank), ids that are not members = 0.
- Both columns: 32-bit `XMRENoSplit`, logical value = stored value.
- **Segments**: `SegmentMapStorage` `Type` 2, `RecordsPerSegment` = M, `RecordCount` = N + 3,
  so the records spill into a second (or third, when M ≤ 2) segment: M = 5, 7 records →
  segments of 5 and 2.

**Sort order** (blank first unless a sort-by column says otherwise):

| Column type | Order |
|---|---|
| int64, decimal, double, dateTime | numeric; dates by their OLE Automation value |
| boolean | `false`, `true` |
| string | the model collation (§5) |
| any, with `Column.SortByColumnID` | by the sort-by column's value on the rows carrying the member, ascending, blank keys first; the blank member moves with its key. Ties: own value, then data id (not observed). |

**`AttributeHierarchyStorage` facts** (all matched on real and oracle data):
`DistinctDataCount` = M; `MinValue` / `MaxValue` = the first / last **non-blank** member in
sort order (so with a sort-by column, not the alphabetical extremes), as text: integers
plain, decimals as the decimal value (`-4.5`), doubles in .NET round-trip form (`1E+300`),
dates as `M/D/YYYY` (also in a fr-FR model), booleans `TRUE`/`FALSE`, strings verbatim (the
empty string counts); `StringValueMaxLength` = longest string member in UTF-16 units (0 for
non-strings); `HasStatistics` 1, `DataVersion` 1, `SortOrder` 0, `OptimizationLevel` 0,
`ColumnPositionToData` 0, `ColumnDataToPosition` 1 (type 0) or −1 (type 1).

Difference from [MS-XLDM] §2.4.2: the spec says `ID_TO_POS` only exists with a value hash
table; in Power BI it exists exactly for hash-encoded (dictionary) columns.

---

## 3. `R$`: relationship index

One `INDEX` column (`Settings` 3) indexed by the **data id of the from column** (the "many"
side), `LastId + 1` records (`RelationshipIndexStorage.RecordCount`).

- Logical value: the 0-based row, in storage order, of the to-table row with the same key, or
  **−1** for ids 0–2, the blank, and keys with no match. Stored as **value + 1** (the segment's
  `first_run_value` is `0xFFFFFFFF`, i.e. −1, the bias).
- Keys match by value: numbers by value, strings by collation equivalence (§5), so `"ABC"`
  joins `"abc"` but `"DEF"` does not join `"Déf"`. Blank never joins.
- Packing: one segment (`SegmentMapStorage` `Type` 3, `RecordsPerSegment` = `RecordCount`),
  the smallest `XMRENoSplit` width that holds the **to-table's row count** (the largest value
  any row could store), even when the keys reach only the first rows: 605 rows → 10 bits for
  keys 1–10 (200 engine-written indexes of [`vertipaq-large.md`](vertipaq-large.md); the
  sample's 731 → 10 bits fits both readings).
- **One-to-one** relationships get two indexes: `Relationship.RelationshipStorageID` → index of
  the from table (from → to), `RelationshipStorage2ID` → index of the to table (to → from);
  two `RelationshipStorage` rows with the same `RelationshipID` and name.
- `RelationshipIndexStorage`: `IndexType` 1, `Flags` **1 when some from value has no match,
  blank included** (synthetic data with missing keys), 0 otherwise (real data: all keys
  match). The flag is what gives the "one" table its blank row: with `Flags` 0 Desktop omits it
  from `VALUES` until a refresh, with 1 it matches Desktop's own refresh (2026-10-14); a
  relationship that cannot be built (one side table failed to process) has `IndexType` 0 and
  `SystemTableID` 0.
- `RelationshipStorage`: `Name` = `<relationship name with '-' → ' '> (<rel id>)`,
  `DefinitionType`/`Cardinality`/`Flags` 0.

Difference from [MS-XLDM] §2.4.3: the spec describes one entry per distinct value in
first-seen order with −1 stored as such; Power BI indexes by data id and stores value + 1.

---

## 4. `U$`: user hierarchy

The node table of §2.4.4: one row per distinct combination of level members that occurs on
some row, level by level; within a level, nodes are grouped under their parent (in parent
order) and sorted by their own member's `H$` position (blank is a regular member; a sort-by
column applies through the `H$`). No root row.

| Column | `Settings` | Logical value | Stored |
|---|---|---|---|
| `MULTI_LEVEL_ID` | 16 | the node's data id in its level column | as is |
| `PARENT_POS` | 8 | parent row, −1 on the top level | + 1 |
| `FIRST_CHILD_POS` | 9 | first child row, −1 on the leaf level | + 1 |
| `CHILD_COUNT` | 17 | number of children, −1 on the leaf level | + 1 |

All four: 32-bit, `ColumnStorage.BinaryCharacters` 10240, one segment
(`SegmentMapStorage` `Type` 3, `RecordsPerSegment` = node count); the three biased columns
have `first_run_value` `0xFFFFFFFF`.

`HierarchyStorage`: `MaterializationType` 0, `StructureType` 2, `LevelDefinition` =
`$<column storage name>$<first row of the level>` per level, then `$`
(e.g. `$Country (77)$0$Region (78)$5$City (79)$13$`).

Difference from [MS-XLDM] §2.4.4.1–§2.4.4.4: the spec's example stores −1 directly and gives
the child count of every row; Power BI stores value + 1 and −1 child counts on leaves.

---

## 5. String collation

**Findings** (oracle, ~580 probe strings per culture; real file):

1. A model without `Model.Collation` (Desktop's default; the real file's is NULL, culture
   fr-FR, `ColumnStorage.Locale` 1036 on string columns) sorts with the **Windows linguistic
   sort of its culture**: case-insensitive, accent-, kana- and width-sensitive; hyphen-minus,
   apostrophe and the dashes U+2010–2015, U+2027 only break ties ("word sort", compared last
   by position); soft hyphen and other format characters are ignored. Punctuation <
   symbols/currency < digits < Latin < Greek < Cyrillic < … < CJK; ligatures and `æ`, `œ`,
   `ß`, `þ` expand; fractions sort between digits (`½` between 0 and 1).
2. **French cultures compare accents from the end** (`cote < côte < coté < côté`), en-US from
   the start (`cote < coté < côte < côté`). The real file's data does not discriminate the two.
3. **Value identity follows the collation**: hash dictionaries merge values with equal weights
   at every level *and the same UTF-16 length* (fr-FR: `A`=`a`, `À`=`à`, `Ω`=`ω`, `Ａ`=`ａ`;
   but `é`≠`e◌́`, `ß`≠`ss`, `ﬁ`≠`fi`). The first occurrence's spelling is kept. In en-US only
   ASCII case variants merge (`À`/`à`, `Ω`/`ω`, `А`/`а` stay apart), for calculated tables and
   for data imported by Power Query alike (confirmed 2026-10-14 on Desktop 2.158 refreshes).
   Relationship keys use the same equality, applied to the stored values of both ends: in
   en-US a "one" side holding both `À` and `à` is accepted (two keys) and `"A "` on the "many"
   side (stored `A`) matches `a` (confirmed 2026-10-14, Desktop 2.158, en-US and fr-FR, per-key
   totals identical before and after a Desktop refresh of a BIcraft structural save; the
   viewer matches keys the same way). Imported text also loses
   XML-invalid control characters and trailing blanks before this comparison
   ([`data-sources.md`](data-sources.md), *Stored text*). *Encoder note*: the structural save
   dedupes string dictionaries this way (`TextStorage`); its output matched Desktop's own
   refresh value for value in en-US and fr-FR.
4. Values the engine considers equal but keeps apart (`æ`/`ae`, `ĳ`/`ij`, `é`/`e◌́`, `ß`/`ss`,
   soft-hyphen variants) come out in an order that is not even consistent across columns of
   one model, i.e. depends on the engine's sort algorithm, not on the strings.
5. An explicit SQL-style `Model.Collation` (`Latin1_General_CS_AS`) changes everything: case
   variants stay apart, width and kana variants merge, supplementary characters merge, and
   symbol order differs (older sort tables). BIcraft does not model these.

**BIcraft** (`Collation`): `Linguistic { french_accents }` (from `Collation::for_culture`)
builds a four-level key — primary weights from an explicit punctuation table (observed order),
digit and Latin letter weights with accent decomposition for Latin-1 and Latin Extended-A,
script blocks in the observed order elsewhere; accent weights in the observed order (acute <
grave < dot < circumflex < diaeresis < caron < breve < macron < tilde < ring < ogonek <
double acute < cedilla < stroke …); width/kana variants third; word-sort characters fourth —
and breaks remaining ties by UTF-16 code units, so it is a total order. `Ordinal` compares code
units. Equivalence = equal key and equal UTF-16 length.

Exactness: identical to the engine on every real-file column and on all probe pairs except
(fr-FR) 12 of ~580 adjacent pairs: 9 engine ties of point 4, plus `ß`/`ſ`, `þ`/`ŧ` and one
symbol outside the table (`☃`). Outside Latin/Greek/Cyrillic the order is approximate
(code-point order within a script).

---

## 6. Storage of system columns

Every segment is a plain `XMRENoSplit<N>` block (class `703030 + N`, sub-class 1) — never
hybrid RLE:

- `.idf`: per segment `u64 words` then `words` × 8 bytes, `words = ⌈(records + 1) / (64/N)⌉`
  (room for one extra value, as for user columns), values packed low bits first.
- `.idfmeta` `CS`: `records`, `base_id` 0, no RLE facet, `first_run_value` 0 or `0xFFFFFFFF`
  (§3–4), `SS` constant: `distinct_states` 0, `min/max/original_min_data_id` 2,
  `rle_sort_order` −1, `row_count` **0**, `has_nulls` 0, `rle_runs` 0, `others_rle_runs` 0; no
  sub-segment; `SDOs` with each block's offset and word count.

`encode_column` reproduces byte for byte all 80 system `.idf`/`.idfmeta` pairs of the real
file and 51 of the 53 of the synthetic file (the other two hold the engine ties of §5.4).

## 7. Metadata rows of a system table

Observed on the synthetic file; the storage constants (`TableStorage`, `ColumnStorage`,
`SegmentMapStorage`, `Partition`, `DictionaryStorage`) are identical in the real file.

| Row | Values |
|---|---|
| `Table` | `Name` as §1, `IsHidden` 1, `SystemFlags` 1, `ModelID` 0, `DirectLakeIndexingBehavior` 3 |
| `TableStorage` | `Name` = table name, `Version` 1 (`H$`) / 0, `Settings` 4 `H$` / 2 `R$` / 5 `U$` |
| `StorageFolder` | `OwnerType` 18 `<name>$(<table id>).tbl`; partition folder `OwnerType` 20 `…tbl\<partition id>.prt` |
| `Partition` | `Name` = table name, `Type` 3, `Mode` 2, `DataView` 3, `State` 1, `SystemFlags` 1, `RangeGranularity` −1 |
| `PartitionStorage` / `SegmentMapStorage` | one partition; map `Type` 2 (`H$`) / 3, counts as §2–4 |
| `Column` | `ExplicitName` = column name, `ExplicitDataType` 6, `InferredDataType` 19, `Type` 1, `IsNullable` 1, `Alignment` 1, `TableDetailPosition` −1, `SummarizeBy` 1, `IsAvailableInMDX` 1, `SystemFlags` 1, `ExpressionContext` 1, `StringIndexingBehavior` 1 |
| `ColumnStorage` | `Name` = column name, `StoragePosition` 0.., `Settings` per column (§2–4), `Statistics_DistinctStates` 1, `Min/MaxDataID` 2, `OriginalMinSegmentDataID` 2, `RLESortOrder` −1, `Usage` 3, `BinaryCharacters` 10240 on `U$` |
| `DictionaryStorage` | `Type` 0, `DataType` 19, everything else 0 |
| `ColumnPartitionStorage` | `State` 3 (user columns: 1) |
| `StorageFile` | `0.<table name>.<column>.0.idf` (`OwnerType` 23) and `.idfmeta` (24) in the partition folder |

---

## 8. Verification

Recomputed from the decoded user columns of the real file and compared value by value
(scratch harness, counts only):

| | Tables | Columns | Records | Files byte-identical |
|---|---|---|---|---|
| `H$` | 38/38 | 72/72 | 3 883/3 883 | 72/72 |
| `R$` | 4/4 | 4/4 | 777/777 | 4/4 |
| `U$` | 1/1 | 4/4 | 3 060/3 060 | 4/4 (+ `LevelDefinition`) |

Plus `DistinctDataCount` and `StringValueMaxLength` 38/38, string `MinValue`/`MaxValue` 19/19,
`RecordCount` and `Flags` 4/4. With `Collation::Ordinal` instead, 7 `H$` tables (14 columns)
differ, all of them string columns. The synthetic oracle file matches everywhere except one
string column with three engine ties (§5.4). Committed tests (`tests/system.rs`) replay the
oracle's arrays and orders.

**Large models** ([`vertipaq-large.md`](vertipaq-large.md)): rebuilt from the decoded user
columns of engine-written models of up to 20M rows, `H$` 1 888/1 890 (the other two hold
Greek/CJK strings, where the collation is approximate), `R$` 202/202 and `U$` 12/12 tables are
byte-identical. The engine runs a consistency check on `H$` when it loads a backup (an `H$`
whose `POS_TO_ID` disagrees with `ID_TO_POS` is refused) but trusts `R$` as stored (a wrong
index loads and gives wrong relationship answers).

## Open questions

1. Whether the engine rebuilds *missing* system tables on load (then a writer could skip
   them) — untested. Stale ones are not rebuilt: a stale `R$` is used as is, a stale `H$` fails
   the load's consistency check.
2. ~~Segment size of very large system tables~~: an `H$` keeps `RecordsPerSegment` = its
   member count even with 10M members (no 2^20 split); `R$`/`U$` stay single-segment.
3. ~~`R$` of a value-encoded from column~~: `max id + 1` records, confirmed (a 20 000-key
   value-encoded column, byte for byte).
4. Sort-by ties, NaN, and mixed-type sort keys: order assumed.
5. Collation: why en-US dictionary equality folds ASCII case only, which other cultures do the
   same, explicit SQL collations, and the engine's order for weight-equal strings.
6. A blank key on the one side made the oracle table fail to process; how Desktop handles it
   for imported tables is unknown.
