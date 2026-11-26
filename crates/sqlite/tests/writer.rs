//! The writer, checked against SQLite itself.
//!
//! Every file written here is read back with our reader and opened by SQLite itself (the
//! `rusqlite` dev-dependency, with SQLite compiled from its bundled source; `sqlite_report`): it
//! must pass `PRAGMA integrity_check` and `PRAGMA foreign_key_check`, and SQLite must see exactly
//! the rows that were written.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use rusqlite::OptionalExtension;

use bicraft_sqlite::{Database, DatabaseBuilder, Error, Generated, Row, Value, rewrite};
use serde_json::{Value as J, json};

const FIXTURES: &[&str] = &[
    "basic",
    "overflow_512",
    "overflow_1024",
    "overflow_4096",
    "overflow_512_utf16le",
    "big_512",
    "pagesize_65536",
    "utf16le",
    "utf16be",
    "autovacuum_full",
    "no_tables",
    "as_metadata",
    "reserved_32",
];

fn tests_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests")
}

fn fixture_bytes(name: &str) -> Vec<u8> {
    std::fs::read(tests_dir().join("fixtures").join(format!("{name}.db"))).unwrap()
}

fn fixture(name: &str) -> Database {
    Database::open(fixture_bytes(name)).unwrap()
}

// ---------------------------------------------------------------------------------------------
// SQLite oracle

fn scratch(label: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("sqlite-writer");
    std::fs::create_dir_all(&dir).unwrap();
    let safe: String = label
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    dir.join(format!(
        "{}-{}-{safe}.db",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ))
}

/// What SQLite reads from `bytes`; asserts that SQLite finds the file intact.
fn sqlite_view(label: &str, bytes: &[u8]) -> J {
    let v = sqlite_raw(label, bytes);
    assert_eq!(v["integrity"], json!(["ok"]), "{label}: integrity_check");
    assert_eq!(v["foreign_keys"], json!([]), "{label}: foreign_key_check");
    v
}

/// Open `bytes` with SQLite and return its report (see [`sqlite_report`]).
fn sqlite_raw(label: &str, bytes: &[u8]) -> J {
    let path = scratch(label);
    std::fs::write(&path, bytes).unwrap();
    let out = sqlite_report(&path);
    let _ = std::fs::remove_file(&path);
    out.unwrap_or_else(|e| panic!("{label}: SQLite could not read the file: {e}"))
}

/// What SQLite itself sees in the database at `path`: `PRAGMA integrity_check` and
/// `PRAGMA foreign_key_check`, the header pragmas, the schema, every row of every ordinary table,
/// and the row count of a full scan through each index (`null` where SQLite cannot serve one,
/// e.g. a partial index).
fn sqlite_report(path: &Path) -> rusqlite::Result<J> {
    use rusqlite::types::ValueRef;

    fn enc(v: ValueRef) -> rusqlite::Result<J> {
        Ok(match v {
            ValueRef::Null => J::Null,
            ValueRef::Integer(i) => json!({ "i": i.to_string() }),
            ValueRef::Real(r) => json!({ "r": hex(&r.to_be_bytes()) }),
            ValueRef::Text(_) => json!({ "t": v.as_str()? }),
            ValueRef::Blob(b) => json!({ "b": hex(b) }),
        })
    }
    /// A pragma or schema value as plain JSON.
    fn plain(v: ValueRef) -> rusqlite::Result<J> {
        Ok(match v {
            ValueRef::Null => J::Null,
            ValueRef::Integer(i) => json!(i),
            ValueRef::Real(r) => json!(r),
            ValueRef::Text(_) => json!(v.as_str()?),
            ValueRef::Blob(b) => json!(hex(b)),
        })
    }
    /// Every row of `sql`, each value through `f`.
    fn rows(
        con: &rusqlite::Connection,
        sql: &str,
        f: fn(ValueRef) -> rusqlite::Result<J>,
    ) -> rusqlite::Result<Vec<Vec<J>>> {
        let mut stmt = con.prepare(sql)?;
        let n = stmt.column_count();
        let mut out = Vec::new();
        let mut it = stmt.query([])?;
        while let Some(r) = it.next()? {
            out.push(
                (0..n)
                    .map(|i| f(r.get_ref(i)?))
                    .collect::<rusqlite::Result<_>>()?,
            );
        }
        Ok(out)
    }
    fn qi(name: &str) -> String {
        format!("\"{}\"", name.replace('"', "\"\""))
    }

    let con = rusqlite::Connection::open(path)?;
    let integrity: Vec<J> = rows(&con, "PRAGMA integrity_check", plain)?
        .into_iter()
        .filter_map(|r| r.into_iter().next())
        .collect();
    // Each value as text, NULL as `None`: the form the earlier oracle recorded.
    let foreign_keys: Vec<J> = rows(&con, "PRAGMA foreign_key_check", plain)?
        .into_iter()
        .map(|r| {
            J::Array(
                r.into_iter()
                    .map(|v| match v {
                        J::Null => json!("None"),
                        J::String(s) => J::String(s),
                        other => J::String(other.to_string()),
                    })
                    .collect(),
            )
        })
        .collect();
    let mut pragmas = serde_json::Map::new();
    for p in [
        "page_size",
        "page_count",
        "encoding",
        "auto_vacuum",
        "user_version",
        "application_id",
        "freelist_count",
    ] {
        let v = con.query_row(&format!("PRAGMA {p}"), [], |r| plain(r.get_ref(0)?))?;
        pragmas.insert(p.into(), v);
    }
    let schema = rows(
        &con,
        "SELECT type, name, tbl_name, sql FROM sqlite_schema ORDER BY rowid",
        plain,
    )?;
    let text = |v: &J| v.as_str().unwrap_or_default().to_owned();
    let mut tables = serde_json::Map::new();
    for e in &schema {
        let (typ, name, sql) = (text(&e[0]), text(&e[1]), text(&e[3]));
        if typ != "table" || sql.to_uppercase().starts_with("CREATE VIRTUAL") {
            continue;
        }
        let wr = con
            .query_row("SELECT wr FROM pragma_table_list WHERE name = ?", [&name], |r| {
                r.get::<_, Option<i64>>(0)
            })
            .optional()?
            .flatten();
        let sel = if wr.is_some_and(|w| w != 0) {
            format!("SELECT * FROM {}", qi(&name))
        } else {
            format!("SELECT _rowid_, * FROM {} ORDER BY _rowid_", qi(&name))
        };
        let rs = rows(&con, &sel, enc)?;
        tables.insert(name, J::Array(rs.into_iter().map(J::Array).collect()));
    }
    let mut out = json!({
        "sqlite_version": rusqlite::version(),
        "integrity": integrity,
        "foreign_keys": foreign_keys,
        "pragmas": pragmas,
        "schema": schema,
        "tables": tables,
    });
    // Every index must also answer a full scan in index order with the right row count.
    for e in &schema {
        if text(&e[0]) != "index" {
            continue;
        }
        let (name, tbl) = (text(&e[1]), text(&e[2]));
        let n = con
            .query_row(
                &format!("SELECT count(*) FROM {} INDEXED BY {}", qi(&tbl), qi(&name)),
                [],
                |r| r.get::<_, i64>(0),
            )
            .ok();
        out["index_counts"][name] = json!(n);
    }
    Ok(out)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn jv(v: &Value) -> J {
    match v {
        Value::Null => J::Null,
        Value::Integer(i) => json!({ "i": i.to_string() }),
        Value::Real(r) => json!({ "r": hex(&r.to_be_bytes()) }),
        Value::Text(s) => json!({ "t": s }),
        Value::Blob(b) => json!({ "b": hex(b) }),
    }
}

fn rows_json(rows: &[Row]) -> J {
    J::Array(
        rows.iter()
            .map(|r| {
                let mut v = vec![json!({ "i": r.rowid.to_string() })];
                v.extend(r.values.iter().map(jv));
                J::Array(v)
            })
            .collect(),
    )
}

/// Open `bytes` with our reader and SQLite; both must see the same rows in every table we can
/// read. Returns the reader's database.
fn check_both(label: &str, bytes: &[u8]) -> Database {
    let db = Database::from_bytes(bytes).unwrap_or_else(|e| panic!("{label}: our reader: {e}"));
    let view = sqlite_view(label, bytes);
    for t in db.tables() {
        // SQLite computes VIRTUAL generated columns; our reader does not
        let computed = t.columns.iter().any(|c| c.generated == Some(Generated::Virtual));
        let Ok(rows) = db.rows_vec(&t.name) else {
            continue;
        };
        if computed {
            continue;
        }
        assert_eq!(
            view["tables"][&t.name],
            rows_json(&rows),
            "{label}: table {}",
            t.name
        );
    }
    for e in db.schema().iter().filter(|e| e.kind == "index") {
        let n = db.rows_vec(&e.tbl_name).map(|r| r.len()).ok();
        if let (Some(n), Some(c)) = (n, view["index_counts"][&e.name].as_u64()) {
            assert_eq!(c as usize, n, "{label}: entries of index {}", e.name);
        }
    }
    db
}

// ---------------------------------------------------------------------------------------------
// Helpers for building test data

fn pattern(n: usize, seed: usize) -> Vec<u8> {
    (0..n)
        .map(|i| ((i * 31 + seed * 7 + (i >> 8)) & 0xff) as u8)
        .collect()
}

fn text(n: usize, seed: usize) -> String {
    const ALPHABET: &[char] = &[
        'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's', 't',
        'u', 'v', 'w', 'x', 'y', 'z', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', ' ', 'é', 'à', 'ü',
        '日', '本', '語',
    ];
    (0..n)
        .map(|i| ALPHABET[(i * 7 + seed) % ALPHABET.len()])
        .collect()
}

/// Depth of a b-tree, following the leftmost child pointers.
fn depth(bytes: &[u8], page_size: usize, root: u32) -> usize {
    let mut page = root as usize;
    let mut d = 1;
    loop {
        let base = (page - 1) * page_size + if page == 1 { 100 } else { 0 };
        let kind = bytes[base];
        if kind == 0x0d || kind == 0x0a {
            return d;
        }
        let first = u16::from_be_bytes([bytes[base + 12], bytes[base + 13]]) as usize;
        let at = (page - 1) * page_size + first;
        page = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        d += 1;
    }
}

fn root_of(db: &Database, name: &str) -> u32 {
    db.schema().iter().find(|e| e.name == name).unwrap().root_page
}

// ---------------------------------------------------------------------------------------------
// Builder

#[test]
fn all_value_types_and_indexes() {
    let mut b = DatabaseBuilder::new(4096).unwrap();
    b.create_table(
        "t",
        "CREATE TABLE t (id INTEGER PRIMARY KEY, i INTEGER, r REAL, s TEXT COLLATE NOCASE, x BLOB, \
         n NUMERIC, a, u TEXT UNIQUE, CHECK (i IS NULL OR i = i))",
    )
    .unwrap();
    b.create_index("t_s", "t", "CREATE INDEX t_s ON t(s)").unwrap();
    b.create_index("t_r_desc", "t", "CREATE INDEX t_r_desc ON t (r DESC, i ASC)")
        .unwrap();
    b.create_index("t_a", "t", "CREATE INDEX \"t_a\" ON t(a COLLATE RTRIM)")
        .unwrap();
    b.create_index("t_x", "t", "CREATE UNIQUE INDEX t_x ON t(x)")
        .unwrap();
    b.create_index("t_id", "t", "CREATE INDEX t_id ON t(id, n)")
        .unwrap();

    let ints = [
        0i64,
        1,
        -1,
        2,
        127,
        128,
        -128,
        -129,
        32767,
        32768,
        -32769,
        (1 << 23) - 1,
        1 << 23,
        (1 << 31) - 1,
        1 << 31,
        (1 << 47) - 1,
        1 << 47,
        i64::MAX,
        i64::MIN,
    ];
    let reals = [
        0.0f64,
        -0.0,
        0.5,
        -1.25,
        3.0,
        1e300,
        -1e-300,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
        9.3e18,
        -2.0,
    ];
    let texts = [
        "",
        "abc",
        "ABC",
        "Abd",
        "Zoë 日本語 🎉",
        "trail  ",
        "trail",
        "a\0b",
        "\u{10ffff}",
    ];
    let mut expected = Vec::new();
    for k in 0..60usize {
        let rowid = if k == 7 { -5 } else { (k as i64) * 3 + 1 };
        let a = match k % 6 {
            0 => Value::Integer(ints[k % ints.len()]),
            1 => Value::Real(reals[k % reals.len()]),
            2 => Value::Text(texts[k % texts.len()].into()),
            3 => Value::Blob(pattern(k, k)),
            4 => Value::Null,
            _ => Value::Integer(k as i64 / 2),
        };
        let row = vec![
            if k % 2 == 0 {
                Value::Null
            } else {
                Value::Integer(rowid)
            },
            if k % 5 == 0 {
                Value::Real(4.0)
            } else {
                Value::Integer(ints[k % ints.len()])
            },
            if k % 4 == 0 {
                Value::Integer(k as i64)
            } else {
                Value::Real(reals[k % reals.len()])
            },
            Value::Text(texts[k % texts.len()].into()),
            if k % 3 == 0 {
                Value::Null
            } else {
                Value::Blob(pattern(k * 13 % 300, k))
            },
            if k % 2 == 0 {
                Value::Real(2.5)
            } else {
                Value::Real(k as f64)
            },
            a.clone(),
            if k % 4 == 1 {
                Value::Null
            } else {
                Value::Text(format!("u{k}"))
            },
        ];
        b.insert("t", rowid, row.clone()).unwrap();
        // What SQLite stores: the rowid alias reads as the rowid; NaN is NULL; integral reals in
        // INTEGER/NUMERIC columns are integers; REAL columns read integers as reals.
        let norm = |v: &Value, col: usize| match (col, v) {
            (0, _) => Value::Integer(rowid),
            (_, Value::Real(r)) if r.is_nan() => Value::Null,
            (1 | 5, Value::Real(r)) if r.fract() == 0.0 && r.abs() < 9e18 => Value::Integer(*r as i64),
            (2, Value::Integer(i)) => Value::Real(*i as f64),
            _ => v.clone(),
        };
        expected.push(Row {
            rowid,
            values: row.iter().enumerate().map(|(c, v)| norm(v, c)).collect(),
        });
    }
    expected.sort_by_key(|r| r.rowid);
    let bytes = b.finish().unwrap();
    let db = check_both("all_value_types", &bytes);
    assert_eq!(db.rows_vec("t").unwrap(), expected);
    let names: Vec<(&str, &str)> = db
        .schema()
        .iter()
        .map(|e| (e.kind.as_str(), e.name.as_str()))
        .collect();
    assert_eq!(
        names,
        [
            ("table", "t"),
            ("index", "sqlite_autoindex_t_1"),
            ("index", "t_s"),
            ("index", "t_r_desc"),
            ("index", "t_a"),
            ("index", "t_x"),
            ("index", "t_id"),
        ]
    );
    assert_eq!(db.header().schema_format, 4);
}

#[test]
fn many_rows_build_deep_trees() {
    for (page_size, n, max_text) in [
        (512usize, 5000usize, 200usize),
        (1024, 4000, 700),
        (4096, 4000, 3000),
        (65536, 250, 40_000),
    ] {
        let label = format!("many_rows_{page_size}");
        let mut b = DatabaseBuilder::new(page_size as u32).unwrap();
        b.create_table(
            "big",
            "CREATE TABLE big (id INTEGER PRIMARY KEY, v INT, s TEXT, d BLOB)",
        )
        .unwrap();
        b.create_index("big_s", "big", "CREATE INDEX big_s ON big(s)")
            .unwrap();
        b.create_index("big_v", "big", "CREATE INDEX big_v ON big(v DESC)")
            .unwrap();
        let mut expected = Vec::new();
        for i in 0..n {
            let rowid = (i as i64) * 7 - 1000;
            let s = text((i * 37) % max_text, i);
            let d = if i % 10 == 0 {
                pattern((i * 131) % (page_size * 3), i)
            } else {
                Vec::new()
            };
            let values = vec![
                Value::Null,
                Value::Integer((i as i64 * 7919) % 1000 - 500),
                Value::Text(s),
                Value::Blob(d),
            ];
            b.insert("big", rowid, values.clone()).unwrap();
            let mut values = values;
            values[0] = Value::Integer(rowid);
            expected.push(Row { rowid, values });
        }
        let bytes = b.finish().unwrap();
        let db = check_both(&label, &bytes);
        assert_eq!(db.rows_vec("big").unwrap(), expected, "{label}");
        let td = depth(&bytes, page_size, root_of(&db, "big"));
        let id = depth(&bytes, page_size, root_of(&db, "big_s"));
        assert!(td >= 2 && id >= 3, "{label}: table depth {td}, index depth {id}");
        if page_size == 512 {
            assert!(td >= 3, "{label}: table depth {td}");
        }
    }
}

#[test]
fn overflow_thresholds() {
    for page_size in [512usize, 1024, 4096, 65536] {
        let label = format!("overflow_{page_size}");
        let u = page_size;
        let max_local = u - 35;
        let min_local = (u - 12) * 32 / 255 - 23;
        let index_max = (u - 12) * 64 / 255 - 23;
        let mut sizes = vec![0usize, 1, 2, 100];
        for t in [
            max_local,
            index_max,
            u,
            u - 4,
            2 * (u - 4) + min_local,
            3 * (u - 4) + min_local,
        ] {
            sizes.extend(t.saturating_sub(12)..t + 12);
        }
        sizes.push(5 * u + 13);
        sizes.sort_unstable();
        sizes.dedup();
        let mut b = DatabaseBuilder::new(page_size as u32).unwrap();
        b.create_table(
            "blobs",
            "CREATE TABLE blobs (id INTEGER PRIMARY KEY, data BLOB, txt TEXT)",
        )
        .unwrap();
        b.create_index("blobs_txt", "blobs", "CREATE INDEX blobs_txt ON blobs(txt)")
            .unwrap();
        b.create_index("blobs_data", "blobs", "CREATE INDEX blobs_data ON blobs(data)")
            .unwrap();
        for (i, &s) in sizes.iter().enumerate() {
            let t = "x".repeat(s.saturating_sub(4)) + &format!("{i:04}");
            b.insert(
                "blobs",
                i as i64 + 1,
                vec![Value::Null, Value::Blob(pattern(s, i)), Value::Text(t)],
            )
            .unwrap();
        }
        let bytes = b.finish().unwrap();
        let db = check_both(&label, &bytes);
        assert_eq!(db.rows_vec("blobs").unwrap().len(), sizes.len());
    }
}

#[test]
fn reserved_bytes_and_large_schema() {
    // `like` a database with 32 reserved bytes per page: usable size 992.
    let src = fixture("reserved_32");
    for (label, base) in [
        ("reserved", DatabaseBuilder::like(&src)),
        ("p512", DatabaseBuilder::new(512).unwrap()),
    ] {
        let mut b = base;
        // Statements of many lengths, including records too large for page 1 alone.
        for i in 0..120usize {
            let pad = "c".repeat(i * 9);
            let sql = format!("CREATE TABLE t{i} (a INTEGER PRIMARY KEY, b /* {pad} */, c TEXT UNIQUE)");
            b.create_table(&format!("t{i}"), &sql).unwrap();
            b.insert(
                &format!("t{i}"),
                1,
                vec![Value::Null, Value::Text(pad), Value::Text("u".into())],
            )
            .unwrap();
        }
        let bytes = b.finish().unwrap();
        let db = check_both(&format!("large_schema_{label}"), &bytes);
        assert_eq!(db.tables().len(), 120);
        assert_eq!(db.schema().len(), 240);
    }
    // A single schema record that fits a page but not page 1: page 1 becomes an interior page
    // with no cells, as SQLite does.
    for len in [380usize, 395, 400, 405, 420, 470, 476, 480, 600] {
        let mut b = DatabaseBuilder::new(512).unwrap();
        let sql = format!("CREATE TABLE t (a /*{}*/)", "p".repeat(len - 23));
        assert_eq!(sql.len(), len);
        b.create_table("t", &sql).unwrap();
        b.insert("t", 1, vec![Value::Integer(5)]).unwrap();
        let bytes = b.finish().unwrap();
        let db = check_both(&format!("page1_{len}"), &bytes);
        assert_eq!(db.schema()[0].sql, sql);
    }
}

#[test]
fn utf16_databases() {
    for fx in ["utf16le", "utf16be"] {
        let src = fixture(fx);
        let mut b = DatabaseBuilder::like(&src);
        b.create_table(
            "w",
            "CREATE TABLE w (id INTEGER PRIMARY KEY, s TEXT, n TEXT COLLATE NOCASE)",
        )
        .unwrap();
        b.create_index("w_s", "w", "CREATE INDEX w_s ON w(s)").unwrap();
        b.create_index("w_n", "w", "CREATE UNIQUE INDEX w_n ON w(n)")
            .unwrap();
        let strings = [
            "\u{0001}",
            "\u{0100}",
            "\u{00ff}",
            "\u{ffff}",
            "\u{10000}",
            "\u{e000}",
            "a",
            "B",
            "日本",
            "🎉",
            "",
        ];
        for (i, s) in strings.iter().enumerate() {
            b.insert(
                "w",
                i as i64 + 1,
                vec![
                    Value::Null,
                    Value::Text((*s).into()),
                    Value::Text(format!("{s}{i}")),
                ],
            )
            .unwrap();
        }
        let bytes = b.finish().unwrap();
        let db = check_both(fx, &bytes);
        assert_eq!(db.header().text_encoding, src.header().text_encoding);
        assert_eq!(db.rows_vec("w").unwrap().len(), strings.len());
    }
}

#[test]
fn autoincrement_keeps_sqlite_sequence() {
    let mut b = DatabaseBuilder::new(1024).unwrap();
    b.create_table(
        "a",
        "CREATE TABLE a (id INTEGER PRIMARY KEY AUTOINCREMENT, v TEXT)",
    )
    .unwrap();
    b.create_table("e", "CREATE TABLE e (id INTEGER PRIMARY KEY AUTOINCREMENT)")
        .unwrap();
    for id in [1, 5, 9] {
        b.insert("a", id, vec![Value::Null, Value::Text(format!("v{id}"))])
            .unwrap();
    }
    assert!(
        b.insert("sqlite_sequence", 1, vec![Value::Null, Value::Null])
            .is_err()
    );
    let bytes = b.finish().unwrap();
    let db = check_both("autoincrement", &bytes);
    let names: Vec<&str> = db.schema().iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["a", "sqlite_sequence", "e"]);
    let seq = db.rows_vec("sqlite_sequence").unwrap();
    assert_eq!(seq.len(), 1);
    assert_eq!(seq[0].values, vec![Value::Text("a".into()), Value::Integer(9)]);

    // Rebuilding `a` with a larger rowid raises the counter; untouched entries stay.
    let out = rewrite(&db, |t| {
        (t == "a").then(|| {
            vec![Row {
                rowid: 40,
                values: vec![Value::Integer(40), Value::Text("x".into())],
            }]
        })
    })
    .unwrap();
    let db2 = check_both("autoincrement_rewrite", &out);
    let seq = db2.rows_vec("sqlite_sequence").unwrap();
    assert_eq!(seq[0].values, vec![Value::Text("a".into()), Value::Integer(40)]);
}

#[test]
fn builder_rejects_bad_input() {
    assert!(matches!(DatabaseBuilder::new(1000), Err(Error::Invalid(_))));
    assert!(matches!(DatabaseBuilder::new(256), Err(Error::Invalid(_))));
    assert!(matches!(DatabaseBuilder::new(131072), Err(Error::Invalid(_))));
    let mut b = DatabaseBuilder::new(4096).unwrap();
    assert!(matches!(
        b.create_table("k", "CREATE TABLE k (a PRIMARY KEY, b) WITHOUT ROWID"),
        Err(Error::Unsupported(_))
    ));
    assert!(matches!(
        b.create_table("v", "CREATE VIRTUAL TABLE v USING fts5(a)"),
        Err(Error::Unsupported(_))
    ));
    assert!(matches!(
        b.create_table("x", "CREATE TABLE x ("),
        Err(Error::Schema { .. })
    ));
    assert!(matches!(
        b.create_table("x", "CREATE TABLE y (a)"),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        b.create_table("sqlite_x", "CREATE TABLE sqlite_x (a)"),
        Err(Error::Invalid(_))
    ));
    b.create_table(
        "t",
        "CREATE TABLE t (id INTEGER PRIMARY KEY, a TEXT, b INT AS (id + 1))",
    )
    .unwrap();
    assert!(matches!(
        b.create_table("T", "CREATE TABLE T (a)"),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        b.create_index("i", "t", "CREATE INDEX i ON t(lower(a))"),
        Err(Error::Unsupported(_))
    ));
    assert!(matches!(
        b.create_index("i", "t", "CREATE INDEX i ON t(a + 1)"),
        Err(Error::Unsupported(_))
    ));
    assert!(matches!(
        b.create_index("i", "t", "CREATE INDEX i ON t(a) WHERE a IS NOT NULL"),
        Err(Error::Unsupported(_))
    ));
    assert!(matches!(
        b.create_index("i", "t", "CREATE INDEX i ON t(a COLLATE klingon)"),
        Err(Error::Unsupported(_))
    ));
    assert!(matches!(
        b.create_index("i", "t", "CREATE INDEX i ON t(b)"),
        Err(Error::Unsupported(_))
    ));
    assert!(matches!(
        b.create_index("i", "t", "CREATE INDEX i ON t(zz)"),
        Err(Error::Schema { .. })
    ));
    assert!(matches!(
        b.create_index("i", "nope", "CREATE INDEX i ON nope(a)"),
        Err(Error::NoSuchTable(_))
    ));
    assert!(matches!(
        b.create_index("i", "t", "CREATE INDEX j ON t(a)"),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        b.create_index("i", "t", "CREATE INDEX i ON t(a"),
        Err(Error::Schema { .. })
    ));
    assert!(matches!(
        b.insert("nope", 1, vec![Value::Null]),
        Err(Error::NoSuchTable(_))
    ));
    assert!(matches!(
        b.insert("t", 1, vec![Value::Null]),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        b.insert("t", 1, vec![Value::Integer(2), Value::Null, Value::Null]),
        Err(Error::Invalid(_))
    ));
    b.insert("t", 1, vec![Value::Integer(1), Value::Null, Value::Null])
        .unwrap();
    assert!(matches!(
        b.insert("t", 1, vec![Value::Null, Value::Null, Value::Null]),
        Err(Error::Invalid(_))
    ));

    // UNIQUE violations are found when the index is built.
    let mut b = DatabaseBuilder::new(512).unwrap();
    b.create_table("u", "CREATE TABLE u (a TEXT COLLATE NOCASE UNIQUE, b)")
        .unwrap();
    b.insert("u", 1, vec![Value::Text("x".into()), Value::Null])
        .unwrap();
    b.insert("u", 2, vec![Value::Null, Value::Null]).unwrap();
    b.insert("u", 3, vec![Value::Null, Value::Null]).unwrap();
    let ok = b.clone().finish().unwrap();
    check_both("unique_nulls", &ok);
    b.insert("u", 4, vec![Value::Text("X".into()), Value::Null])
        .unwrap();
    assert!(matches!(b.finish(), Err(Error::Invalid(_))));
}

#[test]
fn constraint_indexes_follow_sqlite() {
    let mut b = DatabaseBuilder::new(4096).unwrap();
    // duplicate constraints share one index; INTEGER PRIMARY KEY DESC is not a rowid alias
    b.create_table(
        "c",
        "CREATE TABLE c (k INTEGER PRIMARY KEY DESC, a UNIQUE, b TEXT UNIQUE COLLATE NOCASE, \
         UNIQUE (a), UNIQUE (b), UNIQUE (b COLLATE BINARY), CONSTRAINT x UNIQUE (a DESC, b))",
    )
    .unwrap();
    b.create_table("p", "CREATE TABLE p (x INT, y TEXT, PRIMARY KEY (y, x))")
        .unwrap();
    b.create_table("q", "CREATE TABLE q (id integer, v, PRIMARY KEY (id))")
        .unwrap();
    for i in 0..30i64 {
        b.insert(
            "c",
            i + 1,
            vec![
                Value::Integer(100 - i),
                Value::Integer(i * i),
                Value::Text(format!("B{i}")),
            ],
        )
        .unwrap();
        b.insert(
            "p",
            i + 1,
            vec![Value::Integer(i % 3), Value::Text(format!("y{}", i / 3))],
        )
        .unwrap();
        b.insert("q", i * 2, vec![Value::Integer(i * 2), Value::Null])
            .unwrap();
    }
    let bytes = b.finish().unwrap();
    let db = check_both("constraint_indexes", &bytes);
    let idx: Vec<&str> = db
        .schema()
        .iter()
        .filter(|e| e.kind == "index")
        .map(|e| e.name.as_str())
        .collect();
    assert_eq!(
        idx,
        [
            "sqlite_autoindex_c_1",
            "sqlite_autoindex_c_2",
            "sqlite_autoindex_c_3",
            "sqlite_autoindex_c_4",
            "sqlite_autoindex_c_5",
            "sqlite_autoindex_p_1",
        ]
    );
    // SQLite agrees on the automatic indexes it expects: rebuild the same schema with SQLite.
    let ours = scratch("constraint_indexes_ours");
    std::fs::write(&ours, &bytes).unwrap();
    let theirs = rusqlite::Connection::open_in_memory().unwrap();
    {
        let src = rusqlite::Connection::open(&ours).unwrap();
        let mut stmt = src
            .prepare("SELECT sql FROM sqlite_schema WHERE sql IS NOT NULL")
            .unwrap();
        let sqls: Vec<String> = stmt
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        for sql in sqls {
            theirs.execute(&sql, []).unwrap();
        }
    }
    let _ = std::fs::remove_file(&ours);
    let mut stmt = theirs
        .prepare("SELECT name FROM sqlite_schema WHERE type='index' ORDER BY rowid")
        .unwrap();
    let names: Vec<String> = stmt
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(names, idx);
}

#[test]
fn output_is_deterministic() {
    let build = || {
        let mut b = DatabaseBuilder::new(1024).unwrap();
        b.create_table("t", "CREATE TABLE t (id INTEGER PRIMARY KEY, s TEXT UNIQUE)")
            .unwrap();
        for i in (0..500i64).rev() {
            b.insert(
                "t",
                i,
                vec![
                    Value::Null,
                    Value::Text(text((i as usize * 17) % 900, i as usize)),
                ],
            )
            .unwrap();
        }
        b.finish().unwrap()
    };
    assert_eq!(build(), build());
    let db = fixture("basic");
    let keep = rewrite(&db, |_| None).unwrap();
    assert_eq!(keep, rewrite(&db, |_| None).unwrap());
    let rebuilt = |t: &str| db.rows_vec(t).ok();
    assert_eq!(rewrite(&db, rebuilt).unwrap(), rewrite(&db, rebuilt).unwrap());
}

/// The oracle itself: SQLite must notice an index whose keys are out of order.
#[test]
fn oracle_detects_misordered_index() {
    let mut b = DatabaseBuilder::new(512).unwrap();
    b.create_table("t", "CREATE TABLE t (id INTEGER PRIMARY KEY, s TEXT)")
        .unwrap();
    b.create_index("t_s", "t", "CREATE INDEX t_s ON t(s)").unwrap();
    for (i, s) in ["a", "b", "c"].iter().enumerate() {
        b.insert("t", i as i64 + 1, vec![Value::Null, Value::Text((*s).into())])
            .unwrap();
    }
    let mut bytes = b.finish().unwrap();
    let db = check_both("misordered_ok", &bytes);
    let base = (root_of(&db, "t_s") as usize - 1) * 512;
    assert_eq!(bytes[base], 0x0a);
    // swap the first two cell pointers of the index leaf
    let (p0, p1) = (base + 8, base + 10);
    let a = [bytes[p0], bytes[p0 + 1]];
    bytes[p0] = bytes[p1];
    bytes[p0 + 1] = bytes[p1 + 1];
    bytes[p1] = a[0];
    bytes[p1 + 1] = a[1];
    let v = sqlite_raw("misordered", &bytes);
    assert_ne!(v["integrity"], json!(["ok"]));
}

#[test]
fn empty_database() {
    let bytes = DatabaseBuilder::new(4096).unwrap().finish().unwrap();
    assert_eq!(bytes.len(), 4096);
    let db = check_both("empty", &bytes);
    assert!(db.schema().is_empty());
}

// ---------------------------------------------------------------------------------------------
// rewrite

/// Identity rewrites of every fixture: SQLite must see the same schema and rows.
#[test]
fn rewrite_identity_every_fixture() {
    for name in FIXTURES {
        let db = fixture(name);
        let original = sqlite_view(&format!("{name}_original"), &fixture_bytes(name));
        for mode in ["keep", "rebuild"] {
            let label = format!("{name}_{mode}");
            let out = if mode == "keep" {
                rewrite(&db, |_| None)
            } else {
                rewrite(&db, |t| db.rows_vec(t).ok())
            }
            .unwrap_or_else(|e| panic!("{label}: {e}"));
            let back = Database::from_bytes(&out).unwrap();
            assert_eq!(back.schema().len(), db.schema().len(), "{label}");
            for (a, b) in back.schema().iter().zip(db.schema()) {
                assert_eq!(
                    (&a.kind, &a.name, &a.tbl_name, &a.sql),
                    (&b.kind, &b.name, &b.tbl_name, &b.sql),
                    "{label}"
                );
            }
            for t in db.tables() {
                if let Ok(rows) = db.rows_vec(&t.name) {
                    assert_eq!(back.rows_vec(&t.name).unwrap(), rows, "{label}: {}", t.name);
                }
            }
            let h = (back.header(), db.header());
            assert_eq!(h.0.page_size, h.1.page_size, "{label}");
            assert_eq!(h.0.text_encoding, h.1.text_encoding, "{label}");
            assert_eq!(h.0.user_version, h.1.user_version, "{label}");
            assert_eq!(h.0.auto_vacuum(), h.1.auto_vacuum(), "{label}");
            assert_eq!(h.0.reserved_bytes, h.1.reserved_bytes, "{label}");
            let (want, got) = (&original, sqlite_view(&label, &out));
            assert_eq!(got["schema"], want["schema"], "{label}: schema");
            assert_eq!(got["tables"], want["tables"], "{label}: rows");
            assert_eq!(
                got["index_counts"], want["index_counts"],
                "{label}: index entries"
            );
            for p in [
                "page_size",
                "encoding",
                "auto_vacuum",
                "user_version",
                "application_id",
            ] {
                assert_eq!(got["pragmas"][p], want["pragmas"][p], "{label}: {p}");
            }
            assert_eq!(got["pragmas"]["freelist_count"], json!(0), "{label}");
        }
    }
}

#[test]
fn rewrite_with_changed_rows() {
    let db = fixture("basic");
    let mut people = db.rows_vec("people").unwrap();
    people.retain(|r| r.rowid != 100);
    people[0].values[1] = Value::Text("Alice Ü".into());
    people.push(Row {
        rowid: 7,
        values: vec![
            Value::Integer(7),
            Value::Text("Grace".into()),
            Value::Integer(36),
            Value::Real(9.5),
            Value::Blob(pattern(5000, 1)),
            Value::Text("long ".repeat(2000)),
        ],
    });
    let mut orders = db.rows_vec("orders").unwrap();
    for (i, r) in orders.iter_mut().enumerate() {
        r.values[2] = Value::Real(i as f64 + 0.25);
    }
    let out = rewrite(&db, |t| match t {
        "people" => Some(people.clone()),
        "orders" => Some(orders.clone()),
        "q" => Some(vec![Row {
            rowid: 1,
            values: vec![Value::Integer(42), Value::Text("forty-two".into())],
        }]),
        _ => None,
    })
    .unwrap();
    let back = check_both("rewrite_changed", &out);
    let mut people_sorted = people.clone();
    people_sorted.sort_by_key(|r| r.rowid);
    assert_eq!(back.rows_vec("people").unwrap(), people_sorted);
    assert_eq!(back.rows_vec("numbers").unwrap(), db.rows_vec("numbers").unwrap());
    assert_eq!(back.rows_vec("q").unwrap().len(), 1);

    // A UNIQUE violation in new rows is an error, not a corrupt file.
    let mut dup = db.rows_vec("orders").unwrap();
    let first = dup[0].values.clone();
    dup[1].values[1] = first[1].clone();
    dup[1].values[2] = first[2].clone();
    let r = rewrite(&db, |t| (t == "orders").then(|| dup.clone()));
    assert!(matches!(r, Err(Error::Invalid(_))), "{r:?}");
    // WITHOUT ROWID tables can only be kept.
    let r = rewrite(&db, |t| (t == "kv").then(Vec::new));
    assert!(matches!(r, Err(Error::Unsupported(_))), "{r:?}");
    // Wrong number of values.
    let r = rewrite(&db, |t| {
        (t == "alt").then(|| {
            vec![Row {
                rowid: 1,
                values: vec![Value::Null],
            }]
        })
    });
    assert!(matches!(r, Err(Error::Invalid(_))), "{r:?}");
}

/// Expression and partial indexes survive when their table keeps its rows; rebuilding the table
/// is refused clearly.
#[test]
fn rewrite_expression_and_partial_indexes() {
    let path = scratch("expr_src");
    let con = rusqlite::Connection::open(&path).unwrap();
    con.execute_batch(
        "CREATE TABLE e (id INTEGER PRIMARY KEY, a TEXT, b INT);
         CREATE INDEX e_lower ON e(lower(a));
         CREATE INDEX e_part ON e(b) WHERE b > 2;
         CREATE TABLE f (x TEXT UNIQUE, y);
         CREATE TRIGGER f_t AFTER INSERT ON f BEGIN SELECT 1; END;
         INSERT INTO e (a, b) VALUES ('A', 1), ('b', 3), ('C', 5);
         INSERT INTO f VALUES ('k', 1), ('l', 2);",
    )
    .unwrap();
    con.close().unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    let db = Database::from_bytes(&bytes).unwrap();
    let kept = rewrite(&db, |t| (t == "f").then(|| db.rows_vec("f").unwrap())).unwrap();
    let want = sqlite_view("expr_original", &bytes);
    let got = sqlite_view("expr_kept", &kept);
    assert_eq!(got["schema"], want["schema"]);
    assert_eq!(got["tables"], want["tables"]);
    let r = rewrite(&db, |t| (t == "e").then(|| db.rows_vec("e").unwrap()));
    assert!(matches!(r, Err(Error::Unsupported(_))), "{r:?}");
}

/// Rewriting damaged files returns errors (or a file), never panics.
#[test]
fn rewrite_hostile_inputs() {
    for name in [
        "basic",
        "overflow_512",
        "autovacuum_full",
        "as_metadata",
        "utf16be",
    ] {
        let clean = fixture_bytes(name);
        let stride = (clean.len() / 150).max(1);
        for (n, at) in (100..clean.len()).step_by(stride).enumerate() {
            let mut bytes = clean.clone();
            bytes[at] ^= 1 << (n % 8);
            let Ok(db) = Database::from_bytes(&bytes) else {
                continue;
            };
            let _ = rewrite(&db, |_| None);
            let _ = rewrite(&db, |t| db.rows_vec(t).ok());
        }
        // truncations
        for len in [clean.len() / 2, clean.len() - 1, 1024] {
            if let Ok(db) = Database::from_bytes(&clean[..len.min(clean.len())]) {
                let _ = rewrite(&db, |_| None);
            }
        }
    }
}

/// Local-only: an identity rewrite of a real `metadata.sqlitedb` (client data; never committed).
/// Set `BICRAFT_METADATA_DB` to its path. Only counts are printed.
#[test]
fn real_metadata_db_identity() {
    let Ok(path) = bicraft_env::var("BICRAFT_METADATA_DB") else {
        eprintln!("note: BICRAFT_METADATA_DB is not set; skipping the real metadata.sqlitedb check");
        return;
    };
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("note: cannot read BICRAFT_METADATA_DB; skipping");
        return;
    };
    let db = Database::from_bytes(&bytes).unwrap();
    let original = sqlite_view("real_original", &bytes);
    for mode in ["rebuild", "keep"] {
        let out = if mode == "keep" {
            rewrite(&db, |_| None)
        } else {
            rewrite(&db, |t| db.rows_vec(t).ok())
        }
        .unwrap();
        let back = Database::from_bytes(&out).unwrap();
        assert_eq!(back.schema().len(), db.schema().len());
        for (a, b) in back.schema().iter().zip(db.schema()) {
            assert_eq!((&a.kind, &a.name, &a.sql), (&b.kind, &b.name, &b.sql));
        }
        let mut total_rows = 0usize;
        for t in db.tables() {
            let rows = db.rows_vec(&t.name).unwrap();
            total_rows += rows.len();
            assert_eq!(back.rows_vec(&t.name).unwrap(), rows, "table #{}", t.root_page);
        }
        let (want, got) = (&original, sqlite_view(&format!("real_{mode}"), &out));
        assert_eq!(got["schema"], want["schema"], "{mode}: schema");
        assert_eq!(got["tables"], want["tables"], "{mode}: rows");
        assert_eq!(got["index_counts"], want["index_counts"], "{mode}: index entries");
        eprintln!(
            "real metadata.sqlitedb ({mode}): {} schema objects, {} tables, {total_rows} rows, {} -> {} bytes",
            db.schema().len(),
            db.tables().len(),
            bytes.len(),
            out.len()
        );
    }
}
