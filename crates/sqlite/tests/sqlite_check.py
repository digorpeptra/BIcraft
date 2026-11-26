#!/usr/bin/env python3
"""Report what SQLite itself sees in a database file, as JSON on stdout.

Used by `tests/writer.rs`: every file the writer produces is opened with Python's `sqlite3`
module, checked with `PRAGMA integrity_check` and `PRAGMA foreign_key_check`, and its schema
and rows are dumped so the test can compare them with what was written.

Usage:  python sqlite_check.py <database>
"""

import json
import sqlite3
import struct
import sys


def enc_value(v):
    if v is None:
        return None
    if isinstance(v, int):
        return {"i": str(v)}
    if isinstance(v, float):
        return {"r": struct.pack(">d", v).hex()}
    if isinstance(v, str):
        return {"t": v}
    if isinstance(v, (bytes, bytearray)):
        return {"b": bytes(v).hex()}
    raise TypeError(type(v))


def qi(name: str) -> str:
    return '"' + name.replace('"', '""') + '"'


def main(path: str) -> None:
    con = sqlite3.connect(path)
    con.text_factory = str
    out = {
        "sqlite_version": sqlite3.sqlite_version,
        "integrity": [r[0] for r in con.execute("PRAGMA integrity_check")],
        "foreign_keys": [[str(x) for x in r] for r in con.execute("PRAGMA foreign_key_check")],
        "pragmas": {
            p: con.execute(f"PRAGMA {p}").fetchone()[0]
            for p in (
                "page_size",
                "page_count",
                "encoding",
                "auto_vacuum",
                "user_version",
                "application_id",
                "freelist_count",
            )
        },
        "schema": [],
        "tables": {},
    }
    schema = con.execute("SELECT type, name, tbl_name, sql FROM sqlite_schema ORDER BY rowid").fetchall()
    for typ, name, tbl, sql in schema:
        out["schema"].append([typ, name, tbl, sql])
        if typ != "table" or (sql or "").upper().startswith("CREATE VIRTUAL"):
            continue
        wr = con.execute("SELECT wr FROM pragma_table_list WHERE name = ?", (name,)).fetchone()
        if wr and wr[0]:
            sel = f"SELECT * FROM {qi(name)}"
        else:
            sel = f"SELECT _rowid_, * FROM {qi(name)} ORDER BY _rowid_"
        out["tables"][name] = [[enc_value(v) for v in r] for r in con.execute(sel)]
    # Every index must also answer a full scan in index order with the right row count.
    for typ, name, tbl, _ in schema:
        if typ != "index":
            continue
        try:
            n = con.execute(f"SELECT count(*) FROM {qi(tbl)} INDEXED BY {qi(name)}").fetchone()[0]
        except sqlite3.Error:
            n = None  # e.g. a partial index cannot serve an unrestricted scan
        out.setdefault("index_counts", {})[name] = n
    con.close()
    json.dump(out, sys.stdout, ensure_ascii=True, sort_keys=True)


if __name__ == "__main__":
    main(sys.argv[1])
