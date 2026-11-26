//! Regenerate the SQLite test fixtures (`fixtures/<name>.db` and `fixtures/<name>.json`).
//!
//! SQLite itself is the oracle: every `<name>.db` is written by SQLite (the `rusqlite`
//! dev-dependency, with SQLite compiled from its bundled source), and the matching `<name>.json`
//! records what SQLite reads back from it (page size, encoding, tables, declared column types and
//! every row). `oracle.rs` opens the `.db` with our reader and compares against the `.json`.
//!
//! All data is invented for these tests. Ignored by default, since it rewrites committed files:
//!
//! ```text
//! cargo test -p bicraft-sqlite --test gen_fixtures -- --ignored
//! ```
//!
//! Each statement is stepped once, then reset, outside any explicit transaction unless the SQL
//! opens one, as the fixtures were first written; the file header records the SQLite version
//! that wrote it, so a newer SQLite gives other bytes at offset 96 with the same `.json`.

use std::path::{Path, PathBuf};

use rusqlite::types::{Null, ValueRef};
use rusqlite::{Connection, FileControl, ToSql, params};
use serde_json::{Map, Value as J, json};

/// Text and blob values longer than this are recorded as (length, FNV-1a 64) only, which keeps
/// the expected JSON small.
const INLINE_LIMIT: usize = 200;

fn out_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn fnv1a64(data: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in data {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn enc_value(v: ValueRef) -> J {
    match v {
        ValueRef::Null => J::Null,
        ValueRef::Integer(i) => json!({ "i": i.to_string() }),
        ValueRef::Real(r) => json!({ "r": hex(&r.to_be_bytes()) }),
        ValueRef::Text(_) => {
            let s = v.as_str().unwrap();
            if s.chars().count() > INLINE_LIMIT {
                json!({ "tl": s.len(), "fnv": fnv1a64(s.as_bytes()) })
            } else {
                json!({ "t": s })
            }
        }
        ValueRef::Blob(b) => {
            if b.len() > INLINE_LIMIT {
                json!({ "bl": b.len(), "fnv": fnv1a64(b) })
            } else {
                json!({ "b": hex(b) })
            }
        }
    }
}

/// A pragma or catalogue value as plain JSON.
fn plain(v: ValueRef) -> J {
    match v {
        ValueRef::Null => J::Null,
        ValueRef::Integer(i) => json!(i),
        ValueRef::Real(r) => json!(r),
        ValueRef::Text(_) => json!(v.as_str().unwrap()),
        ValueRef::Blob(b) => json!(hex(b)),
    }
}

fn qi(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Every row of `sql`, each value through `f`.
fn rows(con: &Connection, sql: &str, args: &[&dyn ToSql], f: fn(ValueRef) -> J) -> Vec<Vec<J>> {
    let mut stmt = con.prepare(sql).unwrap();
    let n = stmt.column_count();
    let mut it = stmt.query(args).unwrap();
    let mut out = Vec::new();
    while let Some(r) = it.next().unwrap() {
        out.push((0..n).map(|i| f(r.get_ref(i).unwrap())).collect());
    }
    out
}

fn pragma(con: &Connection, name: &str) -> J {
    con.query_row(&format!("PRAGMA {name}"), [], |r| Ok(plain(r.get_ref(0)?)))
        .unwrap()
}

fn describe(path: &Path, full_rows_limit: usize) -> J {
    let con = Connection::open(path).unwrap();
    let mut tables = Vec::new();
    let mut indexes = Vec::new();
    let schema = rows(
        &con,
        "SELECT type, name, tbl_name, sql FROM sqlite_schema ORDER BY rowid",
        &[],
        plain,
    );
    for e in &schema {
        let (typ, name, tbl) = (
            e[0].as_str().unwrap_or_default(),
            e[1].as_str().unwrap_or_default(),
            e[2].as_str().unwrap_or_default(),
        );
        if typ == "index" {
            indexes.push(json!({ "name": name, "table": tbl }));
        }
        if typ != "table" {
            continue;
        }
        // cid, name, type, notnull, dflt_value, pk, hidden
        let xinfo = rows(&con, &format!("PRAGMA table_xinfo({})", qi(name)), &[], plain);
        let cols: Vec<(String, J)> = xinfo
            .iter()
            .map(|c| (c[1].as_str().unwrap().to_owned(), c[2].clone()))
            .collect();
        let virtual_gen: Vec<&str> = xinfo
            .iter()
            .filter(|c| c[6] == json!(2))
            .map(|c| c[1].as_str().unwrap())
            .collect();
        let without_rowid = con
            .query_row("SELECT wr FROM pragma_table_list WHERE name = ?", [name], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap()
            != 0;
        let mut t = json!({
            "name": name,
            "without_rowid": without_rowid,
            "columns": cols.iter().map(|(n, ty)| json!({ "name": n, "type": ty })).collect::<Vec<_>>(),
        });
        if !without_rowid {
            let collist: Vec<String> = cols.iter().map(|(n, _)| qi(n)).collect();
            let sel = format!(
                "SELECT _rowid_{}{} FROM {} ORDER BY _rowid_",
                if cols.is_empty() { "" } else { ", " },
                collist.join(", "),
                qi(name)
            );
            let mut stmt = con.prepare(&sel).unwrap();
            let mut it = stmt.query([]).unwrap();
            let mut all = Vec::new();
            while let Some(r) = it.next().unwrap() {
                let mut vals = vec![enc_value(r.get_ref(0).unwrap())];
                for (i, (c, _)) in cols.iter().enumerate() {
                    // We do not evaluate expressions: VIRTUAL generated columns read as NULL.
                    vals.push(if virtual_gen.contains(&c.as_str()) {
                        J::Null
                    } else {
                        enc_value(r.get_ref(i + 1).unwrap())
                    });
                }
                all.push(J::Array(vals));
            }
            t["row_count"] = json!(all.len());
            if all.len() <= full_rows_limit {
                t["rows"] = J::Array(all);
            } else {
                let mid = all.len() / 2;
                t["rows"] = J::Null;
                t["first"] = all[0].clone();
                t["last"] = all[all.len() - 1].clone();
                t["middle"] = all[mid].clone();
                t["middle_index"] = json!(mid);
            }
        }
        tables.push(t);
    }
    json!({
        "page_size": pragma(&con, "page_size"),
        "page_count": pragma(&con, "page_count"),
        "encoding": pragma(&con, "encoding"),
        "auto_vacuum": pragma(&con, "auto_vacuum"),
        "user_version": pragma(&con, "user_version"),
        "tables": tables,
        "indexes": indexes,
    })
}

fn sorted(v: J) -> J {
    match v {
        J::Object(m) => {
            let mut kv: Vec<(String, J)> = m.into_iter().collect();
            kv.sort_by(|a, b| a.0.cmp(&b.0));
            J::Object(kv.into_iter().map(|(k, v)| (k, sorted(v))).collect::<Map<_, _>>())
        }
        J::Array(a) => J::Array(a.into_iter().map(sorted).collect()),
        other => other,
    }
}

fn fresh(name: &str) -> PathBuf {
    let path = out_dir().join(format!("{name}.db"));
    for suffix in ["", "-journal", "-wal", "-shm"] {
        let p = PathBuf::from(format!("{}{suffix}", path.display()));
        if p.exists() {
            std::fs::remove_file(&p).unwrap();
        }
    }
    path
}

fn finish(name: &str, path: &Path) {
    let desc = sorted(describe(path, 2000));
    let mut s = serde_json::to_string(&desc).unwrap();
    s.push('\n');
    std::fs::write(out_dir().join(format!("{name}.json")), s).unwrap();
}

fn pattern(n: usize, seed: usize) -> Vec<u8> {
    (0..n)
        .map(|i| ((i * 31 + seed * 7 + (i >> 8)) & 0xff) as u8)
        .collect()
}

fn text_pattern(n: usize, seed: usize) -> String {
    let alphabet: Vec<char> = "abcdefghijklmnopqrstuvwxyz0123456789 éàü日本語".chars().collect();
    (0..n)
        .map(|i| alphabet[(i * 7 + seed) % alphabet.len()])
        .collect()
}

/// Run one statement: step it once, then reset it, as the fixtures were first written.
fn exec(con: &Connection, sql: &str, args: &[&dyn ToSql]) {
    let mut stmt = con.prepare(sql).unwrap();
    let mut it = stmt.query(args).unwrap();
    it.next().unwrap();
}

/// A connection in autocommit mode, with SQLite's default of foreign keys off (the bundled
/// build turns them on).
fn open(path: &Path) -> Connection {
    let con = Connection::open(path).unwrap();
    exec(&con, "PRAGMA foreign_keys = OFF", &[]);
    con
}

fn connect(
    path: &Path,
    page_size: Option<u32>,
    encoding: Option<&str>,
    auto_vacuum: Option<u32>,
) -> Connection {
    let con = open(path);
    exec(&con, "PRAGMA journal_mode = DELETE", &[]);
    if let Some(p) = page_size {
        exec(&con, &format!("PRAGMA page_size = {p}"), &[]);
    }
    if let Some(e) = encoding {
        exec(&con, &format!("PRAGMA encoding = '{e}'"), &[]);
    }
    if let Some(a) = auto_vacuum {
        exec(&con, &format!("PRAGMA auto_vacuum = {a}"), &[]);
    }
    con
}

const INTS: &[i64] = &[
    0,
    1,
    -1,
    2,
    127,
    128,
    -128,
    -129,
    255,
    32767,
    32768,
    -32768,
    -32769,
    (1 << 23) - 1,
    1 << 23,
    -(1 << 23),
    (1 << 31) - 1,
    1 << 31,
    -(1 << 31),
    (1 << 47) - 1,
    1 << 47,
    -(1 << 47),
    i64::MAX,
    i64::MIN,
];
const REALS: &[f64] = &[
    0.0,
    -0.0,
    0.5,
    -1.25,
    std::f64::consts::PI,
    1e300,
    -1e-300,
    f64::INFINITY,
    f64::NEG_INFINITY,
];

fn gen_basic() {
    let name = "basic";
    let path = fresh(name);
    let con = connect(&path, Some(4096), None, None);
    con.execute_batch(
        r#"
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
        "#,
    )
    .unwrap();
    let ins = "INSERT INTO people VALUES (?,?,?,?,?,?)";
    let first50: Vec<u8> = (0..50u8).collect();
    exec(&con, ins, params![1, "Alice", 30, 1.5, &[0u8, 1, 2][..], Null]);
    exec(&con, ins, params![2, "Bob", Null, Null, Null, "plain note"]);
    exec(
        &con,
        ins,
        params![5, "Zoë 日本語 🎉", -7, -0.25, Vec::<u8>::new(), 42],
    );
    exec(&con, ins, params![100, "", 0, 0.0, first50, 3.5]);
    exec(
        &con,
        ins,
        params![-3, "negative rowid", 1i64 << 40, 1e-10, Null, &b"blob note"[..]],
    );
    exec(&con, ins, params![i64::MAX, "max rowid", 1, 2.0, Null, Null]);
    for (i, n) in INTS.iter().enumerate() {
        exec(
            &con,
            "INSERT INTO numbers VALUES (?,?,?)",
            params![n, REALS[i % REALS.len()], format!("n{i}")],
        );
    }
    let ins = "INSERT INTO q VALUES (?,?)";
    exec(&con, ins, params![10, "ten"]);
    exec(&con, ins, params![3, "three"]);
    exec(&con, ins, params![-1, Null]);
    let ins = r#"INSERT INTO "weird ""name""" VALUES (?,?,?,?)"#;
    exec(&con, ins, params!["a", 1, "x", "12"]);
    exec(&con, ins, params![Null, Null, Null, "3.5"]);
    exec(&con, ins, params!["tab\tnewline\n", -1, &[0xffu8][..], "text"]);
    let ins = "INSERT INTO orders (order_id, cust, amount) VALUES (?,?,?)";
    exec(&con, ins, params![7, "1", 9.99]);
    exec(&con, ins, params![8, "2", 100]);
    exec(&con, ins, params![20, Null, 0]);
    exec(&con, "INSERT INTO kv VALUES (?,?)", params!["b", 2]);
    exec(&con, "INSERT INTO kv VALUES (?,?)", params!["a", 1]);
    exec(&con, "INSERT INTO alt VALUES (?)", params![1]);
    exec(&con, "INSERT INTO alt VALUES (?)", params!["one"]);
    for col in [
        "b INTEGER DEFAULT 42",
        "c TEXT DEFAULT 'hi'",
        "d REAL DEFAULT -1.5",
        "e",
        "f INTEGER DEFAULT '17'",
        "g BLOB DEFAULT x'CAFE'",
        "h DEFAULT NULL",
        "i REAL DEFAULT 3",
        "j TEXT DEFAULT -5",
    ] {
        exec(&con, &format!("ALTER TABLE alt ADD COLUMN {col}"), &[]);
    }
    exec(
        &con,
        "INSERT INTO alt VALUES (2, 3, 'x', 0.5, 'e', 4, x'00', 1, 2.5, 'j')",
        &[],
    );
    let ins = "INSERT INTO gen (a, d) VALUES (?, ?)";
    exec(&con, ins, params![1, "one"]);
    exec(&con, ins, params![Null, "null"]);
    exec(&con, ins, params![-4, Null]);
    exec(&con, "PRAGMA user_version = 7", &[]);
    con.close().unwrap();
    finish(name, &path);
}

fn gen_overflow(page_size: usize, encoding: Option<&str>, suffix: &str) {
    let name = format!("overflow_{page_size}{suffix}");
    let path = fresh(&name);
    let con = connect(&path, Some(page_size as u32), encoding, None);
    exec(
        &con,
        "CREATE TABLE blobs (id INTEGER PRIMARY KEY, data BLOB, txt TEXT)",
        &[],
    );
    exec(&con, "CREATE TABLE idx_target (k TEXT, v INT)", &[]);
    exec(&con, "CREATE INDEX idx_k ON idx_target(k)", &[]);
    let usable = page_size;
    let max_local = usable - 35;
    let min_local = (usable - 12) * 32 / 255 - 23;
    let mut sizes = vec![
        0,
        1,
        100,
        max_local - 10,
        max_local - 3,
        max_local - 2,
        max_local,
        max_local + 1,
        max_local + 2,
        max_local + 50,
        usable,
        usable + 1,
        2 * usable,
        2 * usable + 7,
        3 * (usable - 4) + min_local,
        5 * usable + 13,
    ];
    sizes.sort_unstable();
    sizes.dedup();
    sizes.retain(|&s| s <= 70000);
    for (i, &s) in sizes.iter().enumerate() {
        exec(
            &con,
            "INSERT INTO blobs VALUES (?,?,?)",
            params![i as i64 + 1, pattern(s, i), Null],
        );
    }
    // A long text too, so text crossing overflow pages is exercised in every encoding.
    exec(
        &con,
        "INSERT INTO blobs VALUES (?,?,?)",
        params![1000, Null, text_pattern(3 * usable / 2 + 11, 3)],
    );
    for i in 0..40 {
        let k = text_pattern(20 + (i * 37) % (usable / 3).min(300), i);
        exec(&con, "INSERT INTO idx_target VALUES (?,?)", params![k, i as i64]);
    }
    con.close().unwrap();
    finish(&name, &path);
}

fn gen_big() {
    let name = "big_512";
    let path = fresh(name);
    let con = connect(&path, Some(512), None, None);
    exec(
        &con,
        "CREATE TABLE big (id INTEGER PRIMARY KEY, v INT, s TEXT)",
        &[],
    );
    for i in 1..10001i64 {
        exec(
            &con,
            "INSERT INTO big VALUES (?,?,?)",
            params![i, i * 3 - 5000, format!("r{i}")],
        );
    }
    con.close().unwrap();
    finish(name, &path);
}

fn gen_page_65536() {
    let name = "pagesize_65536";
    let path = fresh(name);
    let con = connect(&path, Some(65536), None, None);
    exec(
        &con,
        "CREATE TABLE t (id INTEGER PRIMARY KEY, data BLOB, s TEXT)",
        &[],
    );
    exec(
        &con,
        "INSERT INTO t VALUES (1, ?, 'small')",
        params![pattern(70000, 9)],
    );
    exec(
        &con,
        "INSERT INTO t VALUES (2, x'01', ?)",
        params!["ü".repeat(10)],
    );
    con.close().unwrap();
    finish(name, &path);
}

fn gen_encoding(enc: &str) {
    let name = if enc.ends_with("le") { "utf16le" } else { "utf16be" };
    let path = fresh(name);
    let con = connect(&path, Some(1024), Some(enc), None);
    con.execute_batch(
        r#"
        BEGIN;
        CREATE TABLE "Tâble ü" ("Naïve" TEXT, n INTEGER PRIMARY KEY, r REAL);
        COMMIT;
        "#,
    )
    .unwrap();
    let ins = r#"INSERT INTO "Tâble ü" VALUES (?,?,?)"#;
    exec(&con, ins, params!["héllo", 1, 1.0]);
    exec(&con, ins, params!["日本語テキスト", 2, Null]);
    exec(&con, ins, params!["emoji 🎉🚀", 3, -2.5]);
    exec(&con, ins, params!["", 4, 0.0]);
    exec(&con, ins, params![text_pattern(2500, 1), 5, 1e10]);
    exec(&con, ins, params![Null, 6, Null]);
    con.close().unwrap();
    finish(name, &path);
}

fn gen_autovacuum() {
    let name = "autovacuum_full";
    let path = fresh(name);
    let con = connect(&path, Some(1024), None, Some(1));
    exec(&con, "CREATE TABLE a (id INTEGER PRIMARY KEY, data BLOB)", &[]);
    exec(&con, "CREATE TABLE b (x TEXT, y INT)", &[]);
    for i in 1..60usize {
        exec(
            &con,
            "INSERT INTO a VALUES (?,?)",
            params![i as i64, pattern(300 + i * 17, i)],
        );
        exec(
            &con,
            "INSERT INTO b VALUES (?,?)",
            params![format!("row {i}"), (i * i) as i64],
        );
    }
    exec(&con, "DELETE FROM a WHERE id % 3 = 0", &[]);
    exec(&con, "DELETE FROM b WHERE y % 2 = 0", &[]);
    exec(&con, "CREATE TABLE c (z)", &[]);
    exec(&con, "INSERT INTO c VALUES ('after vacuum')", &[]);
    con.close().unwrap();
    finish(name, &path);
}

fn gen_no_tables() {
    let name = "no_tables";
    let path = fresh(name);
    let con = connect(&path, None, None, None);
    exec(&con, "PRAGMA user_version = 3", &[]);
    con.close().unwrap();
    finish(name, &path);
}

/// Shaped like the tabular-model metadata store: invented model, plausible columns.
fn gen_as_metadata() {
    let name = "as_metadata";
    let path = fresh(name);
    let con = connect(&path, Some(4096), None, None);
    con.execute_batch(
        r#"
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
        "#,
    )
    .unwrap();
    exec(
        &con,
        "INSERT INTO [Model] VALUES (1, 'Model', NULL, 'en-US', 133000000000000000, 0)",
        &[],
    );
    let ins = "INSERT INTO [Table] VALUES (?,?,?,?,?,?,?)";
    exec(
        &con,
        ins,
        params![10, 1, "Sales", Null, "Fact table", 0, 133000000000000001i64],
    );
    exec(
        &con,
        ins,
        params![11, 1, "Product", Null, Null, 0, 133000000000000002i64],
    );
    exec(
        &con,
        ins,
        params![12, 1, "Calendar", "Time", Null, 1, 133000000000000003i64],
    );
    let ins = "INSERT INTO [Column] VALUES (?,?,?,?,?,?,?,?,?,?,?)";
    exec(
        &con,
        ins,
        params![100, 10, "Amount", Null, 8, 8, 0, 1, Null, "#,0.00", 1],
    );
    exec(
        &con,
        ins,
        params![101, 10, "ProductKey", Null, 6, 6, 1, 1, Null, Null, 0],
    );
    exec(
        &con,
        ins,
        params![102, 10, "OrderDate", Null, 9, 9, 0, 1, Null, Null, 0],
    );
    exec(
        &con,
        ins,
        params![
            103,
            10,
            Null,
            "RowNumber-2662979B-1795-4F74-8F37-6A1BA8059B61",
            6,
            6,
            1,
            3,
            Null,
            Null,
            0
        ],
    );
    exec(
        &con,
        ins,
        params![110, 11, "ProductKey", Null, 6, 6, 0, 1, Null, Null, 0],
    );
    exec(
        &con,
        ins,
        params![111, 11, "Name", Null, 2, 2, 0, 1, Null, Null, 0],
    );
    exec(
        &con,
        ins,
        params![112, 11, "Margin", Null, 8, 8, 0, 2, "[Price] - [Cost]", "0.0%", 1],
    );
    exec(
        &con,
        ins,
        params![120, 12, "Date", Null, 9, 9, 0, 1, Null, "yyyy-mm-dd", 0],
    );
    let ins = "INSERT INTO [Measure] VALUES (?,?,?,?,?,?,?,?,?)";
    exec(
        &con,
        ins,
        params![
            200,
            10,
            "Total Amount",
            "Sum of amounts",
            8,
            "SUM ( Sales[Amount] )",
            "#,0",
            0,
            Null
        ],
    );
    exec(
        &con,
        ins,
        params![
            201,
            10,
            "Avg Amount",
            Null,
            8,
            "DIVIDE ( [Total Amount], COUNTROWS ( Sales ) )\n-- multi-line\n",
            Null,
            0,
            "KPIs\\Averages"
        ],
    );
    exec(
        &con,
        ins,
        params![
            202,
            11,
            "Products €",
            Null,
            6,
            "DISTINCTCOUNT ( Product[ProductKey] )",
            Null,
            1,
            Null
        ],
    );
    let ins = "INSERT INTO [Relationship] VALUES (?,?,?,?,?,?,?,?,?,?,?,?)";
    exec(
        &con,
        ins,
        params![300, 1, "a1b2c3", 1, 1, 1, 10, 101, 2, 11, 110, 1],
    );
    exec(
        &con,
        ins,
        params![301, 1, "d4e5f6", 0, 1, 2, 10, 102, 2, 12, 120, 1],
    );
    let ins = "INSERT INTO [Annotation] VALUES (?,?,?,?,?)";
    exec(
        &con,
        ins,
        params![400, 1, 1, "PBI_QueryOrder", r#"["Sales","Product","Calendar"]"#],
    );
    exec(&con, ins, params![401, 10, 3, "PBI_ResultType", "Table"]);
    let ins = "INSERT INTO [DBPROPERTIES] VALUES (?,?,?)";
    exec(&con, ins, params!["Version", "1550", 2]);
    exec(&con, ins, params!["CompatibilityLevel", "1550", 2]);
    con.close().unwrap();
    finish(name, &path);
}

/// A database with 32 reserved bytes per page (SQLITE_FCNTL_RESERVE_BYTES then VACUUM).
fn gen_reserved() {
    let name = "reserved_32";
    let path = fresh(name);
    let con = open(&path);
    con.execute_batch("PRAGMA page_size = 1024; PRAGMA journal_mode = DELETE;")
        .unwrap();
    let mut n = 32;
    con.file_control(Some("main"), FileControl::ReserveBytes(&mut n))
        .unwrap();
    con.execute_batch("CREATE TABLE r (id INTEGER PRIMARY KEY, data BLOB, t TEXT);")
        .unwrap();
    con.execute_batch("BEGIN;").unwrap();
    for i in 1..40 {
        con.execute_batch(&format!("INSERT INTO r VALUES ({i}, randomblob(0), 'row {i}');"))
            .unwrap();
    }
    con.execute_batch(&format!(
        "INSERT INTO r VALUES (100, x'{}', NULL);",
        hex(&pattern(3000, 5))
    ))
    .unwrap();
    con.execute_batch(&format!(
        "INSERT INTO r VALUES (101, x'{}', NULL);",
        hex(&pattern(1024 - 32 - 35 + 1, 6))
    ))
    .unwrap();
    con.execute_batch("COMMIT;").unwrap();
    con.execute_batch("VACUUM;").unwrap();
    con.close().unwrap();
    let hdr = std::fs::read(&path).unwrap();
    assert_eq!(hdr[20], 32, "reserved bytes not applied");
    finish(name, &path);
}

/// G25: a freelist with several trunk pages (512-byte pages hold 126 leaves per trunk), in a
/// UTF-16be database whose rows straddle the overflow thresholds.
fn gen_freelist_trunks() {
    let name = "freelist_512_utf16be";
    let path = fresh(name);
    let con = connect(&path, Some(512), Some("UTF-16be"), None);
    exec(
        &con,
        "CREATE TABLE t (id INTEGER PRIMARY KEY, s TEXT, b BLOB)",
        &[],
    );
    exec(&con, "CREATE INDEX t_s ON t(s)", &[]);
    let max_local = 512 - 35;
    let min_local = (512 - 12) * 32 / 255 - 23;
    for i in 1..141usize {
        let size = [min_local, max_local, max_local + 1, 2 * 508 + min_local][i % 4];
        exec(
            &con,
            "INSERT INTO t VALUES (?,?,?)",
            params![i as i64, text_pattern(40 + i % 50, i), pattern(size, i)],
        );
    }
    exec(&con, "DELETE FROM t WHERE id % 5 != 0", &[]);
    con.close().unwrap();
    finish(name, &path);
}

/// G25: a WAL-mode database (header read/write versions 2), checkpointed and closed, with
/// incremental auto-vacuum leaving part of its freelist, on 8192-byte pages.
fn gen_wal_incremental() {
    let name = "wal_8192_incremental";
    let path = fresh(name);
    let con = open(&path);
    exec(&con, "PRAGMA page_size = 8192", &[]);
    exec(&con, "PRAGMA auto_vacuum = 2", &[]);
    exec(&con, "PRAGMA journal_mode = WAL", &[]);
    exec(
        &con,
        "CREATE TABLE t (id INTEGER PRIMARY KEY, b BLOB, n INT)",
        &[],
    );
    let usable = 8192;
    let max_local = usable - 35;
    let min_local = (usable - 12) * 32 / 255 - 23;
    let sizes = [
        0,
        min_local,
        max_local,
        max_local + 1,
        usable - 4,
        2 * (usable - 4) + min_local + 1,
    ];
    for (i, &size) in sizes.iter().cycle().take(sizes.len() * 3).enumerate() {
        exec(
            &con,
            "INSERT INTO t VALUES (?,?,?)",
            params![i as i64 + 1, pattern(size, i), size as i64],
        );
    }
    exec(&con, "DELETE FROM t WHERE id > 12", &[]);
    // One step frees one page: the freelist keeps the rest.
    exec(&con, "PRAGMA incremental_vacuum(2)", &[]);
    exec(&con, "PRAGMA wal_checkpoint(TRUNCATE)", &[]);
    con.close().unwrap();
    for suffix in ["-wal", "-shm"] {
        let p = PathBuf::from(format!("{}{suffix}", path.display()));
        if p.exists() {
            std::fs::remove_file(&p).unwrap();
        }
    }
    finish(name, &path);
}

#[test]
#[ignore = "rewrites the committed fixtures: cargo test -p bicraft-sqlite --test gen_fixtures -- --ignored"]
fn regenerate_fixtures() {
    std::fs::create_dir_all(out_dir()).unwrap();
    gen_basic();
    for ps in [512, 1024, 4096] {
        gen_overflow(ps, None, "");
    }
    gen_overflow(512, Some("UTF-16le"), "_utf16le");
    gen_big();
    gen_page_65536();
    gen_encoding("UTF-16le");
    gen_encoding("UTF-16be");
    gen_autovacuum();
    gen_no_tables();
    gen_as_metadata();
    gen_reserved();
    gen_freelist_trunks();
    gen_wal_incremental();
    let total: u64 = std::fs::read_dir(out_dir())
        .unwrap()
        .map(|e| e.unwrap().metadata().unwrap().len())
        .sum();
    eprintln!(
        "fixtures written to {} ({total} bytes) by SQLite {}",
        out_dir().display(),
        rusqlite::version()
    );
}
