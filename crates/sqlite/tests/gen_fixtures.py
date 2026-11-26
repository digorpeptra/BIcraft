#!/usr/bin/env python3
"""Regenerate the SQLite test fixtures for bicraft-sqlite.

SQLite itself is the oracle: every `<name>.db` is written by Python's `sqlite3`
module, and the matching `<name>.json` records what SQLite reads back from it
(page size, encoding, tables, declared column types and every row).  The Rust
tests open the `.db` with our reader and compare against the `.json`.

All data is invented for these tests.

Usage:  python crates/sqlite/tests/gen_fixtures.py
"""

import ctypes
import ctypes.util
import json
import os
import sqlite3
import struct
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "fixtures")

# Text/blob values longer than this are recorded as (length, FNV-1a 64) only,
# which keeps the expected JSON small.
INLINE_LIMIT = 200


def fnv1a64(data: bytes) -> str:
    h = 0xCBF29CE484222325
    for b in data:
        h ^= b
        h = (h * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return f"{h:016x}"


def enc_value(v):
    if v is None:
        return None
    if isinstance(v, int):
        return {"i": str(v)}
    if isinstance(v, float):
        return {"r": struct.pack(">d", v).hex()}
    if isinstance(v, str):
        if len(v) > INLINE_LIMIT:
            b = v.encode("utf-8")
            return {"tl": len(b), "fnv": fnv1a64(b)}
        return {"t": v}
    if isinstance(v, (bytes, bytearray)):
        b = bytes(v)
        if len(b) > INLINE_LIMIT:
            return {"bl": len(b), "fnv": fnv1a64(b)}
        return {"b": b.hex()}
    raise TypeError(type(v))


def qi(name: str) -> str:
    return '"' + name.replace('"', '""') + '"'


def describe(path: str, full_rows_limit: int = 2000) -> dict:
    con = sqlite3.connect(path)
    con.text_factory = str
    out = {
        "page_size": con.execute("PRAGMA page_size").fetchone()[0],
        "page_count": con.execute("PRAGMA page_count").fetchone()[0],
        "encoding": con.execute("PRAGMA encoding").fetchone()[0],
        "auto_vacuum": con.execute("PRAGMA auto_vacuum").fetchone()[0],
        "user_version": con.execute("PRAGMA user_version").fetchone()[0],
        "tables": [],
        "indexes": [],
    }
    schema = con.execute(
        "SELECT type, name, tbl_name, sql FROM sqlite_schema ORDER BY rowid"
    ).fetchall()
    for typ, name, tbl, sql in schema:
        if typ == "index":
            out["indexes"].append({"name": name, "table": tbl})
        if typ != "table":
            continue
        xinfo = con.execute(f"PRAGMA table_xinfo({qi(name)})").fetchall()
        # cid, name, type, notnull, dflt_value, pk, hidden
        cols = [{"name": c[1], "type": c[2]} for c in xinfo]
        virtual_gen = {c[1] for c in xinfo if c[6] == 2}
        without_rowid = bool(
            con.execute("SELECT wr FROM pragma_table_list WHERE name = ?", (name,)).fetchone()[0]
        )
        t = {
            "name": name,
            "without_rowid": without_rowid,
            "columns": cols,
        }
        if not without_rowid:
            collist = ", ".join(qi(c["name"]) for c in cols)
            sel = f"SELECT _rowid_{', ' if cols else ''}{collist} FROM {qi(name)} ORDER BY _rowid_"
            rows = []
            for r in con.execute(sel):
                vals = [enc_value(r[0])]
                for c, v in zip(cols, r[1:]):
                    # We do not evaluate expressions: VIRTUAL generated columns read as NULL.
                    vals.append(None if c["name"] in virtual_gen else enc_value(v))
                rows.append(vals)
            t["row_count"] = len(rows)
            if len(rows) <= full_rows_limit:
                t["rows"] = rows
            else:
                t["rows"] = None
                t["first"] = rows[0]
                t["last"] = rows[-1]
                t["middle"] = rows[len(rows) // 2]
                t["middle_index"] = len(rows) // 2
        out["tables"].append(t)
    con.close()
    return out


def fresh(name: str) -> str:
    path = os.path.join(OUT, name + ".db")
    for suffix in ("", "-journal", "-wal", "-shm"):
        if os.path.exists(path + suffix):
            os.remove(path + suffix)
    return path


def finish(name: str, path: str, **kw):
    desc = describe(path, **kw)
    with open(os.path.join(OUT, name + ".json"), "w", encoding="utf-8", newline="\n") as f:
        json.dump(desc, f, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
        f.write("\n")


def pattern(n: int, seed: int) -> bytes:
    return bytes(((i * 31 + seed * 7 + (i >> 8)) & 0xFF) for i in range(n))


def text_pattern(n: int, seed: int) -> str:
    alphabet = "abcdefghijklmnopqrstuvwxyz0123456789 éàü日本語"
    return "".join(alphabet[(i * 7 + seed) % len(alphabet)] for i in range(n))


def connect(path: str, page_size=None, encoding=None, auto_vacuum=None) -> sqlite3.Connection:
    con = sqlite3.connect(path, isolation_level=None)
    con.execute("PRAGMA journal_mode = DELETE")
    if page_size:
        con.execute(f"PRAGMA page_size = {page_size}")
    if encoding:
        con.execute(f"PRAGMA encoding = '{encoding}'")
    if auto_vacuum is not None:
        con.execute(f"PRAGMA auto_vacuum = {auto_vacuum}")
    return con


INTS = [0, 1, -1, 2, 127, 128, -128, -129, 255, 32767, 32768, -32768, -32769,
        (1 << 23) - 1, 1 << 23, -(1 << 23), (1 << 31) - 1, 1 << 31, -(1 << 31),
        (1 << 47) - 1, 1 << 47, -(1 << 47), (1 << 63) - 1, -(1 << 63)]
REALS = [0.0, -0.0, 0.5, -1.25, 3.141592653589793, 1e300, -1e-300, float("inf"), float("-inf")]


def gen_basic():
    name = "basic"
    path = fresh(name)
    con = connect(path, page_size=4096)
    con.executescript(
        '''
        BEGIN;
        CREATE TABLE people (
            id INTEGER PRIMARY KEY,   -- rowid alias
            name TEXT NOT NULL,
            age INT,
            score REAL,
            photo BLOB,
            note
        );
        CREATE INDEX people_name ON people(name);
        CREATE TABLE numbers (n INTEGER, r REAL, label VARCHAR(20));
        CREATE TABLE q (x INTEGER PRIMARY KEY DESC, y);
        CREATE TABLE "weird ""name""" (
            [col 1] text,
            `col``2` int,
            'col3' /* a comment */ ,
            "select" NUMERIC
        );
        CREATE TABLE orders (
            order_id integer,
            cust TEXT REFERENCES people(id) ON DELETE CASCADE,
            amount DECIMAL(10, 2) DEFAULT 0 CHECK (amount >= 0),
            flags UNSIGNED BIG INT NOT NULL DEFAULT (0),
            CONSTRAINT pk PRIMARY KEY (order_id),
            UNIQUE (cust, amount)
        );
        CREATE TABLE kv (k TEXT PRIMARY KEY, v) WITHOUT ROWID;
        CREATE VIEW adults AS SELECT * FROM people WHERE age >= 18;
        CREATE TABLE alt (a);
        CREATE TABLE gen (
            a INT,
            b INT GENERATED ALWAYS AS (a * 2) VIRTUAL,
            c INT AS (a + 1) STORED,
            d TEXT
        );
        CREATE TABLE empty_table (x, y);
        COMMIT;
        '''
    )
    people = [
        (1, "Alice", 30, 1.5, b"\x00\x01\x02", None),
        (2, "Bob", None, None, None, "plain note"),
        (5, "Zoë 日本語 🎉", -7, -0.25, b"", 42),
        (100, "", 0, 0.0, bytes(range(256))[:50], 3.5),
        (-3, "negative rowid", 1 << 40, 1e-10, None, b"blob note"),
        ((1 << 63) - 1, "max rowid", 1, 2.0, None, None),
    ]
    con.executemany("INSERT INTO people VALUES (?,?,?,?,?,?)", people)
    for i, n in enumerate(INTS):
        con.execute("INSERT INTO numbers VALUES (?,?,?)", (n, REALS[i % len(REALS)], f"n{i}"))
    con.executemany("INSERT INTO q VALUES (?,?)", [(10, "ten"), (3, "three"), (-1, None)])
    con.executemany(
        'INSERT INTO "weird ""name""" VALUES (?,?,?,?)',
        [("a", 1, "x", "12"), (None, None, None, "3.5"), ("tab\tnewline\n", -1, b"\xff", "text")],
    )
    con.executemany(
        "INSERT INTO orders (order_id, cust, amount) VALUES (?,?,?)",
        [(7, "1", 9.99), (8, "2", 100), (20, None, 0)],
    )
    con.executemany("INSERT INTO kv VALUES (?,?)", [("b", 2), ("a", 1)])
    con.executemany("INSERT INTO alt VALUES (?)", [(1,), ("one",)])
    con.execute("ALTER TABLE alt ADD COLUMN b INTEGER DEFAULT 42")
    con.execute("ALTER TABLE alt ADD COLUMN c TEXT DEFAULT 'hi'")
    con.execute("ALTER TABLE alt ADD COLUMN d REAL DEFAULT -1.5")
    con.execute("ALTER TABLE alt ADD COLUMN e")
    con.execute("ALTER TABLE alt ADD COLUMN f INTEGER DEFAULT '17'")
    con.execute("ALTER TABLE alt ADD COLUMN g BLOB DEFAULT x'CAFE'")
    con.execute("ALTER TABLE alt ADD COLUMN h DEFAULT NULL")
    con.execute("ALTER TABLE alt ADD COLUMN i REAL DEFAULT 3")
    con.execute("ALTER TABLE alt ADD COLUMN j TEXT DEFAULT -5")
    con.execute("INSERT INTO alt VALUES (2, 3, 'x', 0.5, 'e', 4, x'00', 1, 2.5, 'j')")
    con.executemany("INSERT INTO gen (a, d) VALUES (?, ?)", [(1, "one"), (None, "null"), (-4, None)])
    con.execute("PRAGMA user_version = 7")
    con.close()
    finish(name, path)


def gen_overflow(page_size: int, encoding=None, suffix=""):
    name = f"overflow_{page_size}{suffix}"
    path = fresh(name)
    con = connect(path, page_size=page_size, encoding=encoding)
    con.execute("CREATE TABLE blobs (id INTEGER PRIMARY KEY, data BLOB, txt TEXT)")
    con.execute("CREATE TABLE idx_target (k TEXT, v INT)")
    con.execute("CREATE INDEX idx_k ON idx_target(k)")
    usable = page_size
    max_local = usable - 35
    min_local = (usable - 12) * 32 // 255 - 23
    sizes = sorted({0, 1, 100, max_local - 10, max_local - 3, max_local - 2, max_local, max_local + 1,
                    max_local + 2, max_local + 50, usable, usable + 1, 2 * usable, 2 * usable + 7,
                    3 * (usable - 4) + min_local, 5 * usable + 13})
    sizes = [s for s in sizes if s >= 0 and s <= 70000]
    with con:
        for i, s in enumerate(sizes):
            con.execute("INSERT INTO blobs VALUES (?,?,?)", (i + 1, pattern(s, i), None))
        # A long text too, so text crossing overflow pages is exercised in every encoding.
        con.execute("INSERT INTO blobs VALUES (?,?,?)", (1000, None, text_pattern(3 * usable // 2 + 11, 3)))
        for i in range(40):
            k = text_pattern(20 + (i * 37) % min(usable // 3, 300), i)
            con.execute("INSERT INTO idx_target VALUES (?,?)", (k, i))
    con.close()
    finish(name, path)


def gen_big():
    name = "big_512"
    path = fresh(name)
    con = connect(path, page_size=512)
    con.execute("CREATE TABLE big (id INTEGER PRIMARY KEY, v INT, s TEXT)")
    with con:
        con.executemany(
            "INSERT INTO big VALUES (?,?,?)",
            ((i, i * 3 - 5000, f"r{i}") for i in range(1, 10001)),
        )
    con.close()
    finish(name, path)


def gen_page_65536():
    name = "pagesize_65536"
    path = fresh(name)
    con = connect(path, page_size=65536)
    con.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, data BLOB, s TEXT)")
    with con:
        con.execute("INSERT INTO t VALUES (1, ?, 'small')", (pattern(70000, 9),))
        con.execute("INSERT INTO t VALUES (2, x'01', ?)", ("ü" * 10,))
    con.close()
    finish(name, path)


def gen_encoding(enc: str):
    name = "utf16" + ("le" if enc.endswith("le") else "be")
    path = fresh(name)
    con = connect(path, page_size=1024, encoding=enc)
    con.executescript(
        """
        BEGIN;
        CREATE TABLE "Tâble ü" ("Naïve" TEXT, n INTEGER PRIMARY KEY, r REAL);
        COMMIT;
        """
    )
    with con:
        con.executemany(
            'INSERT INTO "Tâble ü" VALUES (?,?,?)',
            [("héllo", 1, 1.0), ("日本語テキスト", 2, None), ("emoji 🎉🚀", 3, -2.5), ("", 4, 0.0),
             (text_pattern(2500, 1), 5, 1e10), (None, 6, None)],
        )
    con.close()
    finish(name, path)


def gen_autovacuum():
    name = "autovacuum_full"
    path = fresh(name)
    con = connect(path, page_size=1024, auto_vacuum=1)
    con.execute("CREATE TABLE a (id INTEGER PRIMARY KEY, data BLOB)")
    con.execute("CREATE TABLE b (x TEXT, y INT)")
    with con:
        for i in range(1, 60):
            con.execute("INSERT INTO a VALUES (?,?)", (i, pattern(300 + i * 17, i)))
            con.execute("INSERT INTO b VALUES (?,?)", (f"row {i}", i * i))
    with con:
        con.execute("DELETE FROM a WHERE id % 3 = 0")
        con.execute("DELETE FROM b WHERE y % 2 = 0")
    con.execute("CREATE TABLE c (z)")
    con.execute("INSERT INTO c VALUES ('after vacuum')")
    con.close()
    finish(name, path)


def gen_no_tables():
    name = "no_tables"
    path = fresh(name)
    con = connect(path)
    con.execute("PRAGMA user_version = 3")
    con.close()
    finish(name, path)


def gen_as_metadata():
    """Shaped like the tabular-model metadata store: invented model, plausible columns."""
    name = "as_metadata"
    path = fresh(name)
    con = connect(path, page_size=4096)
    con.executescript(
        """
        BEGIN;
        CREATE TABLE [Model] ([ID] INTEGER PRIMARY KEY, [Name] TEXT, [Description] TEXT,
            [Culture] TEXT, [ModifiedTime] INTEGER, [DefaultMode] INTEGER);
        CREATE TABLE [Table] ([ID] INTEGER PRIMARY KEY, [ModelID] INTEGER, [Name] TEXT,
            [DataCategory] TEXT, [Description] TEXT, [IsHidden] INTEGER, [ModifiedTime] INTEGER);
        CREATE TABLE [Column] ([ID] INTEGER PRIMARY KEY, [TableID] INTEGER, [ExplicitName] TEXT,
            [InferredName] TEXT, [ExplicitDataType] INTEGER, [InferredDataType] INTEGER,
            [IsHidden] INTEGER, [Type] INTEGER, [Expression] TEXT, [FormatString] TEXT,
            [SummarizeBy] INTEGER);
        CREATE TABLE [Measure] ([ID] INTEGER PRIMARY KEY, [TableID] INTEGER, [Name] TEXT,
            [Description] TEXT, [DataType] INTEGER, [Expression] TEXT, [FormatString] TEXT,
            [IsHidden] INTEGER, [DisplayFolder] TEXT);
        CREATE TABLE [Relationship] ([ID] INTEGER PRIMARY KEY, [ModelID] INTEGER, [Name] TEXT,
            [IsActive] INTEGER, [Type] INTEGER, [CrossFilteringBehavior] INTEGER,
            [FromTableID] INTEGER, [FromColumnID] INTEGER, [FromCardinality] INTEGER,
            [ToTableID] INTEGER, [ToColumnID] INTEGER, [ToCardinality] INTEGER);
        CREATE TABLE [Annotation] ([ID] INTEGER PRIMARY KEY, [ObjectID] INTEGER, [ObjectType] INTEGER,
            [Name] TEXT, [Value] TEXT);
        CREATE TABLE [DBPROPERTIES] ([Name] TEXT, [Value] TEXT, [Type] INTEGER);
        COMMIT;
        """
    )
    with con:
        con.execute("INSERT INTO [Model] VALUES (1, 'Model', NULL, 'en-US', 133000000000000000, 0)")
        con.executemany(
            "INSERT INTO [Table] VALUES (?,?,?,?,?,?,?)",
            [(10, 1, "Sales", None, "Fact table", 0, 133000000000000001),
             (11, 1, "Product", None, None, 0, 133000000000000002),
             (12, 1, "Calendar", "Time", None, 1, 133000000000000003)],
        )
        cols = [
            (100, 10, "Amount", None, 8, 8, 0, 1, None, "#,0.00", 1),
            (101, 10, "ProductKey", None, 6, 6, 1, 1, None, None, 0),
            (102, 10, "OrderDate", None, 9, 9, 0, 1, None, None, 0),
            (103, 10, None, "RowNumber-2662979B-1795-4F74-8F37-6A1BA8059B61", 6, 6, 1, 3, None, None, 0),
            (110, 11, "ProductKey", None, 6, 6, 0, 1, None, None, 0),
            (111, 11, "Name", None, 2, 2, 0, 1, None, None, 0),
            (112, 11, "Margin", None, 8, 8, 0, 2, "[Price] - [Cost]", "0.0%", 1),
            (120, 12, "Date", None, 9, 9, 0, 1, None, "yyyy-mm-dd", 0),
        ]
        con.executemany("INSERT INTO [Column] VALUES (?,?,?,?,?,?,?,?,?,?,?)", cols)
        con.executemany(
            "INSERT INTO [Measure] VALUES (?,?,?,?,?,?,?,?,?)",
            [(200, 10, "Total Amount", "Sum of amounts", 8, "SUM ( Sales[Amount] )", "#,0", 0, None),
             (201, 10, "Avg Amount", None, 8,
              "DIVIDE ( [Total Amount], COUNTROWS ( Sales ) )\n-- multi-line\n", None, 0, "KPIs\\Averages"),
             (202, 11, "Products €", None, 6, "DISTINCTCOUNT ( Product[ProductKey] )", None, 1, None)],
        )
        con.executemany(
            "INSERT INTO [Relationship] VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
            [(300, 1, "a1b2c3", 1, 1, 1, 10, 101, 2, 11, 110, 1),
             (301, 1, "d4e5f6", 0, 1, 2, 10, 102, 2, 12, 120, 1)],
        )
        con.executemany(
            "INSERT INTO [Annotation] VALUES (?,?,?,?,?)",
            [(400, 1, 1, "PBI_QueryOrder", '["Sales","Product","Calendar"]'),
             (401, 10, 3, "PBI_ResultType", "Table")],
        )
        con.executemany(
            "INSERT INTO [DBPROPERTIES] VALUES (?,?,?)",
            [("Version", "1550", 2), ("CompatibilityLevel", "1550", 2)],
        )
    con.close()
    finish(name, path)


def load_sqlite_library():
    """The SQLite shared library behind Python's sqlite3 module, for file controls Python lacks."""
    candidates = []
    try:
        import _sqlite3  # noqa: PLC0415

        d = os.path.dirname(_sqlite3.__file__)
        candidates.append(os.path.join(d, "sqlite3.dll"))
    except Exception:
        pass
    found = ctypes.util.find_library("sqlite3")
    if found:
        candidates.append(found)
    candidates.append("libsqlite3.so.0")
    for c in candidates:
        try:
            return ctypes.CDLL(c)
        except OSError:
            continue
    return None


def gen_reserved():
    """A database with 32 reserved bytes per page (SQLITE_FCNTL_RESERVE_BYTES then VACUUM)."""
    name = "reserved_32"
    path = fresh(name)
    lib = load_sqlite_library()
    if lib is None:
        print("warning: SQLite library not found through ctypes; skipping", name, file=sys.stderr)
        return
    db = ctypes.c_void_p()
    if lib.sqlite3_open(path.encode("utf-8"), ctypes.byref(db)) != 0:
        raise RuntimeError("sqlite3_open failed")

    def exec_sql(sql):
        err = ctypes.c_char_p()
        rc = lib.sqlite3_exec(db, sql.encode("utf-8"), None, None, ctypes.byref(err))
        if rc != 0:
            raise RuntimeError(f"sqlite3_exec failed ({rc}): {err.value}")

    exec_sql("PRAGMA page_size = 1024; PRAGMA journal_mode = DELETE;")
    n = ctypes.c_int(32)
    SQLITE_FCNTL_RESERVE_BYTES = 38
    rc = lib.sqlite3_file_control(db, b"main", SQLITE_FCNTL_RESERVE_BYTES, ctypes.byref(n))
    if rc != 0:
        raise RuntimeError(f"file_control failed ({rc})")
    exec_sql("CREATE TABLE r (id INTEGER PRIMARY KEY, data BLOB, t TEXT);")
    exec_sql("BEGIN;")
    for i in range(1, 40):
        exec_sql(f"INSERT INTO r VALUES ({i}, randomblob(0), 'row {i}');")
    exec_sql(f"INSERT INTO r VALUES (100, x'{pattern(3000, 5).hex()}', NULL);")
    exec_sql(f"INSERT INTO r VALUES (101, x'{pattern(1024 - 32 - 35 + 1, 6).hex()}', NULL);")
    exec_sql("COMMIT;")
    exec_sql("VACUUM;")
    lib.sqlite3_close(db)
    with open(path, "rb") as f:
        hdr = f.read(100)
    if hdr[20] != 32:
        raise RuntimeError(f"reserved bytes not applied (got {hdr[20]})")
    finish(name, path)


def gen_freelist_trunks():
    """G25: a freelist with several trunk pages (512-byte pages hold 126 leaves per trunk), in a
    UTF-16be database whose rows straddle the overflow thresholds."""
    name = "freelist_512_utf16be"
    path = fresh(name)
    con = connect(path, page_size=512, encoding="UTF-16be")
    con.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, s TEXT, b BLOB)")
    con.execute("CREATE INDEX t_s ON t(s)")
    max_local = 512 - 35
    min_local = (512 - 12) * 32 // 255 - 23
    with con:
        for i in range(1, 141):
            size = [min_local, max_local, max_local + 1, 2 * 508 + min_local][i % 4]
            con.execute("INSERT INTO t VALUES (?,?,?)", (i, text_pattern(40 + i % 50, i), pattern(size, i)))
    with con:
        con.execute("DELETE FROM t WHERE id % 5 != 0")
    con.close()
    finish(name, path)


def gen_wal_incremental():
    """G25: a WAL-mode database (header read/write versions 2), checkpointed and closed, with
    incremental auto-vacuum leaving part of its freelist, on 8192-byte pages."""
    name = "wal_8192_incremental"
    path = fresh(name)
    con = sqlite3.connect(path, isolation_level=None)
    con.execute("PRAGMA page_size = 8192")
    con.execute("PRAGMA auto_vacuum = 2")
    con.execute("PRAGMA journal_mode = WAL")
    con.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, b BLOB, n INT)")
    usable = 8192
    max_local = usable - 35
    min_local = (usable - 12) * 32 // 255 - 23
    sizes = [0, min_local, max_local, max_local + 1, usable - 4, 2 * (usable - 4) + min_local + 1]
    with con:
        for i, size in enumerate(sizes * 3):
            con.execute("INSERT INTO t VALUES (?,?,?)", (i + 1, pattern(size, i), size))
    with con:
        con.execute("DELETE FROM t WHERE id > 12")
    con.execute("PRAGMA incremental_vacuum(2)")
    con.execute("PRAGMA wal_checkpoint(TRUNCATE)")
    con.close()
    for suffix in ("-wal", "-shm"):
        if os.path.exists(path + suffix):
            os.remove(path + suffix)
    finish(name, path)


def main():
    os.makedirs(OUT, exist_ok=True)
    gen_basic()
    for ps in (512, 1024, 4096):
        gen_overflow(ps)
    gen_overflow(512, encoding="UTF-16le", suffix="_utf16le")
    gen_big()
    gen_page_65536()
    gen_encoding("UTF-16le")
    gen_encoding("UTF-16be")
    gen_autovacuum()
    gen_no_tables()
    gen_as_metadata()
    gen_reserved()
    gen_freelist_trunks()
    gen_wal_incremental()
    total = sum(os.path.getsize(os.path.join(OUT, f)) for f in os.listdir(OUT))
    print(f"fixtures written to {OUT} ({total} bytes)")


if __name__ == "__main__":
    main()
