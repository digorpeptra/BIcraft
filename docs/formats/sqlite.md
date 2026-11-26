# SQLite database files (`metadata.sqlitedb`)

| | |
|---|---|
| Used for | `metadata.sqlitedb` inside the `DataModel` ABF backup: Analysis Services keeps the tabular model metadata (tables such as `Model`, `Table`, `Column`, `Measure`, `Relationship`, `Annotation`) in it |
| Crate | `bicraft-sqlite` (layer L0, no workspace dependencies) |
| Sources | The public-domain format description: <https://www.sqlite.org/fileformat2.html> (file format), <https://www.sqlite.org/datatype3.html> (affinity), <https://www.sqlite.org/lang_createtable.html> (`CREATE TABLE` grammar, rowid aliases, `WITHOUT ROWID`), <https://www.sqlite.org/gencol.html> (generated columns) |
| Method | Implemented from those documents only. SQLite itself is used as the test oracle (fixtures written and read back through Python's `sqlite3` module); no SQLite code is ported. |
| Date | 2026-10-13 |

The reader works on an in-memory image: the file comes out of a decompressed archive, so there is
never a live writer, a WAL or a hot journal. The [writer](#writer) produces a complete image in
memory, so an edited `metadata.sqlitedb` can be packed back into a `DataModel` without bundling
C SQLite.

## API

```rust
let db = Database::open(bytes)?;            // or from_bytes(&[u8]) / open_path(path)
db.header();                                // page size, encoding, page count, ...
db.schema();                                // every sqlite_schema row (tables, indexes, views, triggers)
db.tables();                                // &[TableInfo]: name, root page, sql, kind, columns
db.column_names("Measure")?;                // Vec<String>
for row in db.rows("Measure")? { let row = row?; /* Row { rowid, values } */ }
db.rows_vec("Measure")?;                    // Vec<Row>
db.query_all("Measure")?;                   // Vec<BTreeMap<String, Value>>
```

`Value` is `Null | Integer(i64) | Real(f64) | Text(String) | Blob(Vec<u8>)`. Table lookup is
exact first, then ASCII case-insensitive (as in SQLite). `sqlite_schema` / `sqlite_master` can be
read like a table.

## Supported subset

| Area | Support |
|---|---|
| Header | Magic; page size 512 to 32768 and the `1` encoding of 65536; reserved bytes per page (usable size must be at least 480); read version 1 and 2 (read version > 2 is rejected); schema format 0 to 4; text encoding UTF-8, UTF-16le, UTF-16be (0 is read as UTF-8). |
| Page count | The in-header size (offset 28) is used only when it is non-zero and the change counter (offset 24) equals version-valid-for (offset 92); otherwise the count comes from the file length. Either way it is capped at the number of whole pages in the file. |
| B-trees | Table interior (`0x05`) and leaf (`0x0D`) pages; page 1 has its b-tree header at offset 100 while cell offsets stay relative to the page start. Index pages (`0x02`, `0x0A`) are recognised but never read: a table traversal that reaches one reports corruption. |
| Payloads | Table-leaf local size: with usable size U and payload P, X = U-35, M = ((U-12)\*32/255)-23, K = M+((P-M) mod (U-4)); local = P if P ≤ X, else K if K ≤ X, else M. Overflow pages: 4-byte next-page number then U-4 content bytes. |
| Records | Varints (1 to 9 bytes); serial types 0 to 9, even ≥ 12 blob, odd ≥ 13 text; 10 and 11 are rejected. Text is decoded per the database encoding; invalid sequences become U+FFFD. |
| Rows | In rowid order. An `INTEGER PRIMARY KEY` column reads as the rowid. Columns with REAL affinity read stored integers as reals (SQLite stores integral reals as integers). Records shorter than the table (rows written before `ALTER TABLE ADD COLUMN`) take the column's literal `DEFAULT`, with affinity applied, else NULL. |
| Schema | `CREATE TABLE` parsing: `TEMP`, `IF NOT EXISTS`, `schema.name`; quoted identifiers `"x"`, `[x]`, `` `x` `` and `'x'`; `--` and `/* */` comments; type names of several words with `(n)` / `(n, m)`; column and table constraints (skipped except for `PRIMARY KEY`, `DEFAULT` and generated columns); `WITHOUT ROWID` and `STRICT` options. `decl_type` is the type text as written. |

### Rowid alias rules

A column is an alias for the rowid when the table is a rowid table, the primary key has exactly one
column, and that column's declared type is exactly `INTEGER` (any case). `INTEGER PRIMARY KEY DESC`
written as a *column* constraint is the documented exception and is not an alias; the same column
named in a table constraint `PRIMARY KEY (x DESC)` is. `INT PRIMARY KEY` is not an alias.

## Not supported (reported, not misread)

| Case | Behaviour |
|---|---|
| `WITHOUT ROWID` tables | Listed with `kind = WithoutRowid`; `rows()` returns `Error::Unsupported`. |
| Virtual tables (`CREATE VIRTUAL TABLE`) | Listed with `kind = Virtual`; `rows()` returns `Error::Unsupported`. Their shadow tables are ordinary tables and readable. |
| VIRTUAL generated columns | Not stored in the record; read as `Null` (expressions are not evaluated). STORED generated columns are read normally. |
| Non-literal `DEFAULT` (`CURRENT_TIMESTAMP`, expressions) | Missing values read as `Null`. |
| Index b-trees | Not exposed (index names are in `schema()`). |
| WAL / rollback journal / freelist / pointer-map pages | Ignored. A WAL-mode file (header versions 2/2) reads like any other once checkpointed; frames still in a `-wal` file are not seen (the reader takes one image, never the side files). Freelist trunk and leaf pages and pointer-map pages are never reached from a b-tree, so databases with freelists and both auto-vacuum modes (full and incremental) read normally. |
| `CREATE TABLE ... AS SELECT` text | SQLite never stores it (it stores a synthesised column list); if found, the table gets a `parse_error`. |
| A `CREATE TABLE` that does not parse | The table is listed with `parse_error` set; `rows()` / `column_names()` return `Error::Schema`. Other tables stay readable. |

## Limits and hostile input

Every input is treated as hostile; malformed files produce `Err`, never a panic.

- **Cycles and shared pages.** Each traversal tracks every page it reaches (b-tree and overflow
  pages alike). A valid table never reaches the same page twice, so a repeat is reported as
  corruption. This also bounds the work of one traversal by the number of pages.
- **Depth.** B-trees deeper than 64 levels are rejected (SQLite itself stops at 20).
- **Allocation.** A cell's payload size must not exceed `page_count * usable_size`, so no allocation
  is larger than the file. Record values must lie inside their payload.
- **Bounds.** Cell counts must fit the page, cell pointers must point inside the page past the
  pointer array, and pages beyond the page count or the file are errors.
- **Schema SQL.** Statements above 1 MiB are not parsed.
- **Iteration.** After yielding an `Err`, the `Rows` iterator stops.

## Tests

`crates/sqlite/tests/gen_fixtures.py` regenerates the fixtures under `crates/sqlite/tests/fixtures/`
with Python's `sqlite3` module (the 32-reserved-bytes fixture uses `ctypes` to call
`sqlite3_file_control(SQLITE_FCNTL_RESERVE_BYTES)` then `VACUUM`). Each `<name>.db` has a
`<name>.json` with what SQLite reads back: header facts, tables, declared types, and every row
(long text and blobs as length + FNV-1a 64 hash; the 10,000-row table as first/middle/last rows).
The fixtures cover page sizes 512, 1024, 4096 and 65536, payloads at and around the overflow
thresholds, UTF-16le/be, auto-vacuum FULL, reserved bytes, indexes, a `WITHOUT ROWID` table, a view,
`ALTER TABLE ADD COLUMN` defaults, generated columns, quoted identifiers, every integer width and
special reals, an invented model shaped like the Analysis Services metadata store, and (G25) a
freelist of three trunk pages in a UTF-16be file (`freelist_512_utf16be`) and a checkpointed
WAL-mode file with incremental auto-vacuum and a leftover freelist on 8192-byte pages
(`wal_8192_incremental`); `rewrite` of both keeps every row and drops the freelist.

**G25 differential campaign (2026-10-14).** 101 databases generated with Python's `sqlite3`
(SQLite 3.49.1): page sizes 512, 1024, 2048, 4096, 8192, 16384, 32768 and 65536; UTF-8, UTF-16le
and UTF-16be; plain, with freelist trunk pages (rows deleted, no vacuum), auto-vacuum full,
auto-vacuum incremental with a partial `incremental_vacuum`, and WAL (checkpointed); table and
index payloads at every overflow threshold (minLocal, maxLocal and the `K` spill sizes for one,
two and three overflow pages, ±1) in blobs and text, a `UNIQUE` index and a text index. Every
database matched SQLite's own reading (header, schema, every row), and both identity `rewrite`s
(tables kept and tables rebuilt) of each passed `PRAGMA integrity_check` and read back identically
in SQLite. No reader or writer change was needed; the two fixtures above keep the cases that the
committed set did not cover.

`tests/hostile.rs` truncates, bit-flips and hand-crafts files (page cycles, overflow self-loops, a
200-level chain, a 2^62-byte payload claim, broken schema SQL) and checks that reading returns
`Err` or valid data without panicking.

## Writer

```rust
let mut b = DatabaseBuilder::new(4096)?;            // or DatabaseBuilder::like(&db)
b.create_table("Measure", "CREATE TABLE [Measure] ([ID] INTEGER PRIMARY KEY, [Name] TEXT)")?;
b.create_index("ix", "Measure", "CREATE INDEX ix ON [Measure]([Name] COLLATE NOCASE)")?;
b.insert("Measure", 1, vec![Value::Null, Value::Text("Total".into())])?;
let bytes: Vec<u8> = b.finish()?;

// Same schema objects, new rows for some tables (None keeps a table's rows):
let bytes = rewrite(&db, |table| (table == "Measure").then(|| new_rows.clone()))?;
```

Written from the same public format description; nothing is ported from SQLite. SQLite itself is
the oracle in the tests.

### What a written file looks like

| Area | Rule |
|---|---|
| Header | Magic; page size (65536 stored as 1); write/read version 1 for a new file (copied by `like`); reserved bytes 0 (copied); payload fractions 64/32/32; change counter 1 for a new file, source + 1 with `like`, and version-valid-for equal to it so the in-header page count (offset 28) is valid; freelist empty; schema cookie = number of objects for a new file, source + 1 with `like` (root pages move, so connections must reload the schema); schema format 4 (copied; a source with 0 gets 4 once it has objects); default cache size, user version, application id and `SQLITE_VERSION_NUMBER` copied (a new file says 3046001); text encoding 1/2/3 (a source that left it 0 keeps 0 only while the file has no objects). |
| Page order | Page 1 holds `sqlite_schema`. Root pages are allocated first, consecutively from page 2 in schema order (as SQLite does, and as auto-vacuum requires); then each b-tree's other pages and overflow chains; the non-root pages of `sqlite_schema` come last. No freelist pages. |
| Auto-vacuum | When `like` copies auto-vacuum (or incremental vacuum), pointer-map pages are written at page 2 and every `U/5 + 1` pages after it, with an entry for every page: root (type 1, parent 0), b-tree child (5, parent page), first overflow page (3, page of the cell), later overflow pages (4, previous overflow page). Offset 52 holds the largest root page (at least 1). |
| Records | Header-size varint (counting itself), then serial types, then values. Integers use the smallest of the 1/2/3/4/6/8-byte forms, and 0 and 1 use serial types 8 and 9 when the schema format is 4. REAL is serial 7. Text is encoded in the database encoding (`2n+13`), blobs `2n+12`. |
| Values | Stored with the storage class given, except: NaN becomes NULL; an integral REAL in an INTEGER or NUMERIC column becomes an INTEGER; in a REAL column an integral value (in range, as `sqlite3VdbeIntegerAffinity` decides) is stored in integer form and reads back as REAL. This is what SQLite does on `INSERT`. Text is not converted to numbers (nor numbers to text) by column affinity. An `INTEGER PRIMARY KEY` column stores NULL (its value is the rowid); VIRTUAL generated columns are not stored; STORED ones are stored as given. Records always carry every stored column. |
| Table b-trees | Built bottom-up from rows sorted by rowid. Leaves are filled greedily and the last two pages of every level are rebalanced to the most even split; interior cells hold the largest rowid of their left child. A page never has fewer than one cell (a non-root interior page has at least two children). Cells shorter than 4 bytes are given 4, the smallest unit SQLite tracks. Page 1 has 100 bytes less room; when a single schema record fits a normal page but not page 1, page 1 becomes an interior page with no cells and one right child, the only zero-cell interior page SQLite accepts. |
| Overflow | Local payload as in the reader (table leaves: X = U-35) and, for index cells (leaf and interior), X = ((U-12)\*64/255)-23, with the same M and K. The rest goes to a chain of overflow pages (4-byte next pointer, then U-4 bytes). |
| Index b-trees | Key = indexed columns then the rowid; an indexed `INTEGER PRIMARY KEY` column holds the rowid. Sorted in SQLite's order: NULL < numbers (INTEGER and REAL compared exactly, like `sqlite3IntFloatCompare`) < text (by collation) < blobs (`memcmp`); `DESC` columns reversed (only with schema format 4, as in SQLite); ties broken by rowid. Collations: BINARY (`memcmp` of the stored bytes, so UTF-16 databases sort by their UTF-16 bytes), NOCASE (ASCII folding over UTF-8), RTRIM (trailing spaces ignored, over UTF-8). Index trees are B-trees, not B+trees: every entry appears once, either in a leaf or as a divider in an interior page. UNIQUE indexes reject equal keys that contain no NULL (`Error::Invalid`). |
| `sqlite_schema` | One row per object (type, name, tbl_name, rootpage, sql), rowids 1.. in registration order: each table, then its automatic indexes, then the indexes created for it later. Automatic indexes store NULL sql; views and triggers have root page 0. |
| Automatic indexes | `create_table` derives them from the statement as SQLite does: one per `PRIMARY KEY` (unless it is the `INTEGER PRIMARY KEY` rowid alias, by the reader's rule) and per `UNIQUE` constraint, column or table form, in textual order, named `sqlite_autoindex_<table>_<n>`; a constraint with the same columns and collations as an earlier one reuses it. Key collation: the explicit `COLLATE` in a table constraint, else the column's `COLLATE` (its final one, even when `COLLATE` follows `UNIQUE`), else BINARY. |
| `AUTOINCREMENT` | The first such table brings `sqlite_sequence` (`CREATE TABLE sqlite_sequence(name,seq)`, right after that table and its automatic indexes). At `finish`, each AUTOINCREMENT table with rows has a row whose seq is at least its largest rowid. |

### `rewrite`

`rewrite(db, rows_for)` keeps every schema object in its order with its SQL verbatim. `rows_for`
is called once per table in schema order (including `sqlite_sequence` and `sqlite_stat*`).

| Case | Behaviour |
|---|---|
| `None` | The table's b-tree and its indexes are copied page by page: cells, freeblocks and record encodings stay byte-identical; only child and overflow page numbers change. So anything the writer cannot rebuild (`WITHOUT ROWID` tables, expression and partial indexes, unknown collations) survives as long as its rows are kept. Reserved bytes are zeroed. |
| `Some(rows)` | The table is rebuilt from `rows` (`Row { rowid, values }`, one value per declared column, as the reader returns them) and every index on it is rebuilt: explicit indexes from their `CREATE INDEX`, automatic indexes by matching `sqlite_autoindex_<table>_<n>` to the n-th derived constraint index (the source must have exactly the indexes the constraints imply). |
| Not rebuildable | `Some` for a `WITHOUT ROWID`, virtual or unparseable table, or for a table with an expression index, a partial index or an unsupported collation, returns `Error::Unsupported` naming the object. |
| AUTOINCREMENT | When a rebuilt table has AUTOINCREMENT, `sqlite_sequence` is rebuilt too (from the source unless supplied) and its seq raised to the table's largest rowid. |
| Freelist | Dropped: the output has no free pages. |

### Not supported by the writer

| Case | Behaviour |
|---|---|
| `WITHOUT ROWID` and virtual tables in `create_table` | `Error::Unsupported` (kept as-is by `rewrite`). |
| Expression and partial indexes, collations other than BINARY/NOCASE/RTRIM, indexes on VIRTUAL generated columns | `Error::Unsupported` (kept as-is by `rewrite` when the table keeps its rows). |
| Evaluating SQL | Defaults, `CHECK`, `NOT NULL`, foreign keys and generated-column expressions are not evaluated; the caller supplies final values. |
| Text affinity conversions | Not applied (see Values above). |
| Names starting with `sqlite_` | Reserved: `create_table`, `create_index` and `insert` refuse them (`sqlite_sequence` is maintained by the writer). |
| Files of 1 GiB or more | `Error::Invalid`: the writer stays below the page holding the lock byte at offset 2^30, which SQLite never uses. |

Other limits: a record payload is at most 10^9 bytes (SQLite's default `SQLITE_MAX_LENGTH`);
`CREATE` statements above 1 MiB are not parsed; page copies follow at most 64 levels and treat a
page reached twice (cycle or shared page) as corruption. The writer never panics: bad input
returns `Error::Invalid`, `Error::Schema`, `Error::Unsupported`, or `Error::Corrupt` for a damaged
source given to `rewrite`. Output is deterministic: the same calls give the same bytes.

### Writer tests

`crates/sqlite/tests/writer.rs` checks every written file with our reader and with SQLite itself:
`sqlite_check.py` opens it with Python's `sqlite3` module (`python3`, else `python`; skipped with a
note when neither is available), requires `PRAGMA integrity_check` = `ok` and an empty
`PRAGMA foreign_key_check`, scans every index (`INDEXED BY`) and dumps the schema and all rows as
JSON. Covered: every value type and integer width, special reals, unicode (including UTF-16le/be
through `like`), NOCASE/RTRIM/DESC/UNIQUE indexes, automatic indexes (names checked against SQLite
building the same schema), page sizes 512/1024/4096/65536, 32 reserved bytes, trees three or more
levels deep, payloads around every overflow threshold for tables and indexes, a large schema and
schema records too large for page 1, AUTOINCREMENT, identity `rewrite` of every committed fixture
(tables kept and tables rebuilt; SQLite must see the same schema, rows, index entry counts and
header pragmas), `rewrite` with changed rows, expression and partial indexes kept by page copy,
determinism, mutated and truncated sources, and a check that the oracle notices a misordered
index.

A local-only test reads a real `metadata.sqlitedb` from `BICRAFT_METADATA_DB` (held locally, never
committed) and requires both identity rewrites (every table rebuilt, and every table kept) to pass
`integrity_check` with identical schema SQL and rows. Both pass on the file available on
2026-10-13 (69 tables, one automatic index, 1,537 rows, page size 4096).
