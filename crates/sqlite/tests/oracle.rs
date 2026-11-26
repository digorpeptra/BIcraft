//! Compare our reader with what SQLite itself reads from the same files.
//!
//! Every `fixtures/<name>.db` was written by SQLite and `fixtures/<name>.json` records what SQLite
//! read back (see `gen_fixtures.py`).

use std::path::{Path, PathBuf};

use bicraft_sqlite::{Database, Error, TableKind, TextEncoding, Value};
use serde_json::Value as J;

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn open(name: &str) -> Database {
    Database::open_path(fixtures_dir().join(format!("{name}.db"))).unwrap()
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

fn check_value(ctx: &str, got: &Value, want: &J) {
    let ok = match (got, want) {
        (Value::Null, J::Null) => true,
        (Value::Integer(i), J::Object(o)) if o.contains_key("i") => o["i"].as_str() == Some(&i.to_string()),
        (Value::Real(r), J::Object(o)) if o.contains_key("r") => {
            o["r"].as_str() == Some(&hex(&r.to_be_bytes()))
        }
        (Value::Text(s), J::Object(o)) if o.contains_key("t") => o["t"].as_str() == Some(s.as_str()),
        (Value::Text(s), J::Object(o)) if o.contains_key("tl") => {
            o["tl"].as_u64() == Some(s.len() as u64) && o["fnv"].as_str() == Some(&fnv1a64(s.as_bytes()))
        }
        (Value::Blob(b), J::Object(o)) if o.contains_key("b") => o["b"].as_str() == Some(&hex(b)),
        (Value::Blob(b), J::Object(o)) if o.contains_key("bl") => {
            o["bl"].as_u64() == Some(b.len() as u64) && o["fnv"].as_str() == Some(&fnv1a64(b))
        }
        _ => false,
    };
    assert!(ok, "{ctx}: got {got:?}, SQLite read {want}");
}

fn check_row(ctx: &str, rowid: i64, values: &[Value], want: &J) {
    let want = want.as_array().unwrap();
    check_value(&format!("{ctx} rowid"), &Value::Integer(rowid), &want[0]);
    assert_eq!(values.len(), want.len() - 1, "{ctx}: column count");
    for (i, (g, w)) in values.iter().zip(&want[1..]).enumerate() {
        check_value(&format!("{ctx} column {i}"), g, w);
    }
}

fn check_fixture(name: &str) {
    let text = std::fs::read_to_string(fixtures_dir().join(format!("{name}.json"))).unwrap();
    let want: J = serde_json::from_str(&text).unwrap();
    let db = open(name);
    let h = db.header();
    assert_eq!(
        u64::from(h.page_size),
        want["page_size"].as_u64().unwrap(),
        "{name}: page size"
    );
    assert_eq!(
        u64::from(h.page_count),
        want["page_count"].as_u64().unwrap(),
        "{name}: page count"
    );
    assert_eq!(
        u64::from(h.user_version),
        want["user_version"].as_u64().unwrap(),
        "{name}: user_version"
    );
    assert_eq!(
        h.auto_vacuum(),
        want["auto_vacuum"].as_u64().unwrap() != 0,
        "{name}: auto_vacuum"
    );
    let enc = match want["encoding"].as_str().unwrap() {
        "UTF-8" => TextEncoding::Utf8,
        "UTF-16le" => TextEncoding::Utf16Le,
        "UTF-16be" => TextEncoding::Utf16Be,
        other => panic!("unknown encoding {other}"),
    };
    assert_eq!(h.text_encoding, enc, "{name}: encoding");

    let want_indexes: Vec<&str> = want["indexes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["name"].as_str().unwrap())
        .collect();
    let got_indexes: Vec<&str> = db
        .schema()
        .iter()
        .filter(|e| e.kind == "index")
        .map(|e| e.name.as_str())
        .collect();
    assert_eq!(got_indexes, want_indexes, "{name}: indexes");

    let want_tables = want["tables"].as_array().unwrap();
    let got_names: Vec<&str> = db.tables().iter().map(|t| t.name.as_str()).collect();
    let want_names: Vec<&str> = want_tables.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(got_names, want_names, "{name}: tables");

    for wt in want_tables {
        let tname = wt["name"].as_str().unwrap();
        let ctx = format!("{name}.{tname}");
        let info = db.table(tname).unwrap();
        assert_eq!(info.parse_error, None, "{ctx}");
        let wcols = wt["columns"].as_array().unwrap();
        let names = db.column_names(tname).unwrap();
        assert_eq!(names.len(), wcols.len(), "{ctx}: columns");
        for (col, (n, wc)) in info.columns.iter().zip(names.iter().zip(wcols)) {
            assert_eq!(n, wc["name"].as_str().unwrap(), "{ctx}: column name");
            assert!(
                col.decl_type.eq_ignore_ascii_case(wc["type"].as_str().unwrap()),
                "{ctx}: column {n} type {:?} vs {}",
                col.decl_type,
                wc["type"]
            );
        }
        if wt["without_rowid"].as_bool().unwrap() {
            assert_eq!(info.kind, TableKind::WithoutRowid, "{ctx}");
            assert!(matches!(db.rows(tname), Err(Error::Unsupported(_))), "{ctx}");
            continue;
        }
        assert_eq!(info.kind, TableKind::Rowid, "{ctx}");
        let rows = db.rows_vec(tname).unwrap_or_else(|e| panic!("{ctx}: {e}"));
        assert_eq!(
            rows.len() as u64,
            wt["row_count"].as_u64().unwrap(),
            "{ctx}: row count"
        );
        match wt["rows"].as_array() {
            Some(wrows) => {
                for (i, (r, w)) in rows.iter().zip(wrows).enumerate() {
                    check_row(&format!("{ctx} row {i}"), r.rowid, &r.values, w);
                }
            }
            None => {
                let mid = wt["middle_index"].as_u64().unwrap() as usize;
                let first = rows.first().unwrap();
                let last = rows.last().unwrap();
                check_row(&format!("{ctx} first"), first.rowid, &first.values, &wt["first"]);
                check_row(
                    &format!("{ctx} middle"),
                    rows[mid].rowid,
                    &rows[mid].values,
                    &wt["middle"],
                );
                check_row(&format!("{ctx} last"), last.rowid, &last.values, &wt["last"]);
            }
        }
        // query_all agrees with rows()
        let maps = db.query_all(tname).unwrap();
        assert_eq!(maps.len(), rows.len());
        if let (Some(m), Some(r)) = (maps.first(), rows.first()) {
            for (n, v) in names.iter().zip(&r.values) {
                assert_eq!(m.get(n), Some(v), "{ctx}: query_all column {n}");
            }
        }
    }
}

macro_rules! oracle_tests {
    ($($test:ident => $name:literal),* $(,)?) => {
        $(#[test] fn $test() { check_fixture($name); })*
    };
}

oracle_tests! {
    basic => "basic",
    overflow_512 => "overflow_512",
    overflow_1024 => "overflow_1024",
    overflow_4096 => "overflow_4096",
    overflow_512_utf16le => "overflow_512_utf16le",
    big_512 => "big_512",
    pagesize_65536 => "pagesize_65536",
    utf16le => "utf16le",
    utf16be => "utf16be",
    autovacuum_full => "autovacuum_full",
    no_tables => "no_tables",
    as_metadata => "as_metadata",
    reserved_32 => "reserved_32",
    freelist_512_utf16be => "freelist_512_utf16be",
    wal_8192_incremental => "wal_8192_incremental",
}

#[test]
fn big_table_every_row() {
    let db = open("big_512");
    let rows = db.rows_vec("big").unwrap();
    assert_eq!(rows.len(), 10_000);
    for (i, r) in rows.iter().enumerate() {
        let id = i as i64 + 1;
        assert_eq!(r.rowid, id);
        assert_eq!(
            r.values,
            vec![
                Value::Integer(id),
                Value::Integer(id * 3 - 5000),
                Value::Text(format!("r{id}"))
            ]
        );
    }
    // the table is deep enough to have interior pages
    let root = db.table("big").unwrap().root_page;
    let bytes = std::fs::read(fixtures_dir().join("big_512.db")).unwrap();
    assert_eq!(bytes[(root as usize - 1) * 512], 0x05);
}

#[test]
fn headers() {
    let h = open("pagesize_65536").header().clone();
    assert_eq!(h.page_size, 65536);
    let h = open("reserved_32").header().clone();
    assert_eq!((h.page_size, h.reserved_bytes, h.usable_size), (1024, 32, 992));
    let h = open("autovacuum_full").header().clone();
    assert!(h.auto_vacuum());
    assert_eq!(open("basic").header().schema_format, 4);
    // G25: a freelist of several trunk pages, a WAL-mode file, incremental auto-vacuum.
    let h = open("freelist_512_utf16be").header().clone();
    assert!(h.freelist_trunk > 0 && h.freelist_count > 2 * 126, "{h:?}");
    assert_eq!(h.text_encoding, TextEncoding::Utf16Be);
    let h = open("wal_8192_incremental").header().clone();
    assert_eq!((h.write_version, h.read_version, h.page_size), (2, 2, 8192));
    assert!(
        h.auto_vacuum() && h.incremental_vacuum && h.freelist_count > 0,
        "{h:?}"
    );
}

/// G25: `rewrite` of a database with a multi-trunk freelist, WAL header and incremental
/// auto-vacuum keeps every row (the freelist is dropped; the rows are compared with SQLite's
/// own reading of the source). Checked separately with SQLite's `integrity_check` across 101
/// generated databases (page sizes 512 to 65536, three encodings, freelists, both auto-vacuum
/// modes, WAL): `docs/formats/sqlite.md`, "Tests".
#[test]
fn rewrite_keeps_rows_of_freelist_and_wal_databases() {
    for name in ["freelist_512_utf16be", "wal_8192_incremental"] {
        let db = open(name);
        let tables: Vec<String> = db.tables().iter().map(|t| t.name.clone()).collect();
        let bytes = bicraft_sqlite::rewrite(&db, |t| {
            tables.iter().any(|x| x == t).then(|| db.rows_vec(t).unwrap())
        })
        .unwrap();
        let back = Database::open(bytes).unwrap();
        assert_eq!(back.header().freelist_count, 0, "{name}");
        assert_eq!(back.header().text_encoding, db.header().text_encoding, "{name}");
        for t in &tables {
            assert_eq!(back.rows_vec(t).unwrap(), db.rows_vec(t).unwrap(), "{name}.{t}");
        }
    }
}

#[test]
fn basic_api() {
    let db = open("basic");
    let people = db.table("PEOPLE").unwrap();
    assert_eq!(people.name, "people");
    assert!(people.columns[0].rowid_alias && people.columns[0].primary_key);
    assert!(!db.table("q").unwrap().columns[0].rowid_alias);
    assert!(db.table("orders").unwrap().columns[0].rowid_alias);
    assert!(matches!(db.rows("missing"), Err(Error::NoSuchTable(_))));
    assert!(matches!(db.column_names("missing"), Err(Error::NoSuchTable(_))));
    // views are not tables, but they are in the schema
    assert!(db.table("adults").is_none());
    assert!(db.schema().iter().any(|e| e.kind == "view" && e.name == "adults"));
    // sqlite_schema itself can be read
    let schema_rows = db.rows_vec("sqlite_master").unwrap();
    assert_eq!(schema_rows.len(), db.schema().len());
    assert_eq!(
        db.column_names("sqlite_schema").unwrap(),
        ["type", "name", "tbl_name", "rootpage", "sql"]
    );
    // a row with every column type
    let alice = db
        .query_all("people")
        .unwrap()
        .into_iter()
        .find(|m| m["name"].as_str() == Some("Alice"))
        .unwrap();
    assert_eq!(alice["id"], Value::Integer(1));
    assert_eq!(alice["score"], Value::Real(1.5));
    assert_eq!(alice["photo"], Value::Blob(vec![0, 1, 2]));
    assert_eq!(alice["note"], Value::Null);
    // ALTER TABLE ADD COLUMN defaults fill older rows
    let alt = db.rows_vec("alt").unwrap();
    assert_eq!(alt[0].values[1], Value::Integer(42));
    assert_eq!(alt[0].values[2], Value::Text("hi".into()));
}

#[test]
fn analysis_services_shaped_metadata() {
    let db = open("as_metadata");
    let names: Vec<_> = db.tables().iter().map(|t| t.name.as_str()).collect();
    for t in [
        "Model",
        "Table",
        "Column",
        "Measure",
        "Relationship",
        "Annotation",
        "DBPROPERTIES",
    ] {
        assert!(names.contains(&t), "{t}");
    }
    assert_eq!(
        db.column_names("Measure").unwrap(),
        [
            "ID",
            "TableID",
            "Name",
            "Description",
            "DataType",
            "Expression",
            "FormatString",
            "IsHidden",
            "DisplayFolder"
        ]
    );
    let measures = db.query_all("Measure").unwrap();
    assert_eq!(measures.len(), 3);
    assert_eq!(measures[0]["Expression"].as_str(), Some("SUM ( Sales[Amount] )"));
    assert_eq!(measures[2]["Name"].as_str(), Some("Products €"));
    let tables = db.query_all("Table").unwrap();
    let sales_id = tables
        .iter()
        .find(|t| t["Name"].as_str() == Some("Sales"))
        .unwrap()["ID"]
        .as_i64()
        .unwrap();
    let cols = db.query_all("Column").unwrap();
    assert_eq!(
        cols.iter()
            .filter(|c| c["TableID"].as_i64() == Some(sales_id))
            .count(),
        4
    );
    let rel = db.query_all("Relationship").unwrap();
    assert_eq!(rel[0]["FromColumnID"], Value::Integer(101));
    assert_eq!(rel[1]["IsActive"], Value::Integer(0));
}
