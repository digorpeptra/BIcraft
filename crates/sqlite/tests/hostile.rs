//! Hostile input: mutated, truncated and hand-crafted files must give `Err` or valid results,
//! never a panic, a hang or an unbounded allocation.

use std::path::{Path, PathBuf};

use bicraft_sqlite::{Database, Error};

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

fn fixture(name: &str) -> Vec<u8> {
    let p: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(format!("{name}.db"));
    std::fs::read(p).unwrap()
}

/// Open and read everything; returns whether any step failed.
fn exercise(bytes: &[u8]) -> bool {
    let Ok(db) = Database::from_bytes(bytes) else {
        return true;
    };
    let mut failed = false;
    for name in ["sqlite_master"]
        .into_iter()
        .chain(db.tables().iter().map(|t| t.name.as_str()))
    {
        let _ = db.column_names(name);
        match db.rows(name) {
            Ok(rows) => {
                for row in rows {
                    if row.is_err() {
                        failed = true;
                    }
                }
            }
            // WITHOUT ROWID and virtual tables are listed but not readable
            Err(Error::Unsupported(_)) => continue,
            Err(_) => failed = true,
        }
        if db.query_all(name).is_err() {
            failed = true;
        }
    }
    failed
}

/// Small deterministic PRNG (xorshift64*).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn page_size(bytes: &[u8]) -> usize {
    match u16::from_be_bytes([bytes[16], bytes[17]]) {
        1 => 65536,
        n => n as usize,
    }
}

#[test]
fn fixtures_read_cleanly() {
    for name in FIXTURES {
        assert!(!exercise(&fixture(name)), "{name} should read without errors");
    }
}

#[test]
fn truncations() {
    for name in FIXTURES {
        let bytes = fixture(name);
        let ps = page_size(&bytes);
        let mut cuts: Vec<usize> = vec![0, 1, 15, 16, 99, 100, 101, 107, 108, 511, 512, ps - 1, ps, ps + 1];
        cuts.extend((1..20).map(|k| bytes.len() * k / 20));
        cuts.push(bytes.len() - 1);
        for cut in cuts {
            if cut <= bytes.len() {
                exercise(&bytes[..cut]);
            }
        }
    }
}

#[test]
fn random_byte_flips() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for name in FIXTURES {
        let base = fixture(name);
        let ps = page_size(&base);
        let iterations = if base.len() > 100_000 { 120 } else { 300 };
        for i in 0..iterations {
            let mut bytes = base.clone();
            let flips = 1 + i % 8;
            for _ in 0..flips {
                // bias towards page headers and cell pointer arrays, where structure lives
                let at = if rng.next().is_multiple_of(2) {
                    let page = rng.below(bytes.len() / ps);
                    let base_off = if page == 0 { 100 } else { 0 };
                    page * ps + base_off + rng.below(48)
                } else {
                    rng.below(bytes.len())
                };
                if let Some(b) = bytes.get_mut(at) {
                    *b ^= 1 << rng.below(8);
                    if rng.next().is_multiple_of(4) {
                        *b = rng.next() as u8;
                    }
                }
            }
            exercise(&bytes);
        }
    }
}

#[test]
fn header_fields() {
    let base = fixture("basic");
    let edits: &[(usize, &[u8])] = &[
        (0, b"SQLite format 4"),
        (16, &[0, 0]),
        (16, &[0, 1]),
        (16, &[0x01, 0x00]),
        (16, &[0x03, 0x00]),
        (16, &[0x80, 0x00]),
        (16, &[0xff, 0xff]),
        (19, &[3]),
        (20, &[255]),
        (20, &[64]),
        (28, &[0xff, 0xff, 0xff, 0xff]),
        (28, &[0, 0, 0, 0]),
        (44, &[0, 0, 0, 9]),
        (56, &[0, 0, 0, 2]),
        (56, &[0, 0, 0, 3]),
        (56, &[0, 0, 0, 7]),
        (56, &[0, 0, 0, 0]),
    ];
    for (at, data) in edits {
        let mut bytes = base.clone();
        bytes[*at..*at + data.len()].copy_from_slice(data);
        exercise(&bytes);
    }
    assert!(matches!(
        Database::from_bytes(b"not a database at all"),
        Err(Error::NotSqlite(_))
    ));
    assert!(matches!(Database::from_bytes(&[]), Err(Error::NotSqlite(_))));
}

/// Point the first 4 bytes of each page at itself: on an overflow page that makes a one-page
/// cycle; on b-tree pages it scrambles the header.
#[test]
fn self_referencing_pages() {
    for name in ["overflow_512", "overflow_1024", "autovacuum_full", "big_512"] {
        let base = fixture(name);
        let ps = page_size(&base);
        let mut saw_error = false;
        for page in 2..=base.len() / ps {
            let mut bytes = base.clone();
            let off = (page - 1) * ps;
            bytes[off..off + 4].copy_from_slice(&(page as u32).to_be_bytes());
            saw_error |= exercise(&bytes);
        }
        assert!(saw_error, "{name}: some self-reference should be detected");
    }
}

/// Make an interior page of `big` point back at itself (rightmost child and each left child).
#[test]
fn btree_cycles_are_errors() {
    let base = fixture("big_512");
    let db = Database::from_bytes(&base).unwrap();
    let root = db.table("big").unwrap().root_page as usize;
    let off = (root - 1) * 512;
    assert_eq!(base[off], 0x05);
    // rightmost pointer
    let mut bytes = base.clone();
    bytes[off + 8..off + 12].copy_from_slice(&(root as u32).to_be_bytes());
    let db = Database::from_bytes(&bytes).unwrap();
    assert!(db.rows("big").unwrap().any(|r| r.is_err()));
    // first cell's left child
    let mut bytes = base.clone();
    let cell = u16::from_be_bytes([bytes[off + 12], bytes[off + 13]]) as usize;
    bytes[off + cell..off + cell + 4].copy_from_slice(&(root as u32).to_be_bytes());
    let db = Database::from_bytes(&bytes).unwrap();
    assert!(db.rows_vec("big").is_err());
    // the iterator stops after its error
    let mut rows = db.rows("big").unwrap();
    assert!(rows.by_ref().any(|r| r.is_err()));
    assert!(rows.next().is_none());
}

// --- hand-built databases --------------------------------------------------------------------

fn varint(mut v: u64) -> Vec<u8> {
    if v <= 0x7f {
        return vec![v as u8];
    }
    let mut out = Vec::new();
    while v > 0 {
        out.push((v & 0x7f) as u8);
        v >>= 7;
    }
    out.reverse();
    let n = out.len();
    for b in &mut out[..n - 1] {
        *b |= 0x80;
    }
    out
}

/// A record holding text, text, text, integer, text (a `sqlite_schema` row).
fn schema_record(name: &str, root: u8, sql: &str) -> Vec<u8> {
    let texts = ["table", name, name];
    let mut types: Vec<u8> = Vec::new();
    for t in texts {
        types.extend(varint(13 + 2 * t.len() as u64));
    }
    types.push(1);
    types.extend(varint(13 + 2 * sql.len() as u64));
    let mut rec = varint(1 + types.len() as u64);
    rec.extend(types);
    for t in texts {
        rec.extend(t.as_bytes());
    }
    rec.push(root);
    rec.extend(sql.as_bytes());
    rec
}

const PS: usize = 512;

/// A database whose table `t` is a chain of `depth` interior pages, each with no cells and a
/// right child pointing to the next, ending in an empty leaf.
fn chain_db(depth: usize) -> Vec<u8> {
    let pages = depth + 2;
    let mut db = vec![0u8; pages * PS];
    db[..16].copy_from_slice(b"SQLite format 3\0");
    db[16..18].copy_from_slice(&(PS as u16).to_be_bytes());
    db[18] = 1;
    db[19] = 1;
    db[21] = 64;
    db[22] = 32;
    db[23] = 32;
    db[28..32].copy_from_slice(&(pages as u32).to_be_bytes());
    db[44..48].copy_from_slice(&4u32.to_be_bytes());
    db[56..60].copy_from_slice(&1u32.to_be_bytes());
    // page 1: schema leaf with one cell
    let rec = schema_record("t", 2, "CREATE TABLE t(a)");
    let mut cell = varint(rec.len() as u64);
    cell.extend(varint(1));
    cell.extend(&rec);
    let cell_off = PS - cell.len();
    db[cell_off..PS].copy_from_slice(&cell);
    db[100] = 0x0d;
    db[103..105].copy_from_slice(&1u16.to_be_bytes());
    db[105..107].copy_from_slice(&(cell_off as u16).to_be_bytes());
    db[108..110].copy_from_slice(&(cell_off as u16).to_be_bytes());
    // pages 2..=depth+1: interior, no cells, right child = next page
    for p in 2..=depth + 1 {
        let off = (p - 1) * PS;
        db[off] = 0x05;
        db[off + 5..off + 7].copy_from_slice(&(PS as u16).to_be_bytes());
        db[off + 8..off + 12].copy_from_slice(&((p + 1) as u32).to_be_bytes());
    }
    let off = (pages - 1) * PS;
    db[off] = 0x0d;
    db[off + 5..off + 7].copy_from_slice(&(PS as u16).to_be_bytes());
    db
}

#[test]
fn shallow_chain_reads_and_deep_chain_is_rejected() {
    let db = Database::open(chain_db(10)).unwrap();
    assert_eq!(db.column_names("t").unwrap(), ["a"]);
    assert!(db.rows_vec("t").unwrap().is_empty());
    let db = Database::open(chain_db(200)).unwrap();
    assert!(db.rows_vec("t").is_err());
}

#[test]
fn huge_payload_claims_are_rejected_without_allocating() {
    let mut bytes = chain_db(1);
    // rewrite the schema cell's payload size as ~2^62
    let cell_off = u16::from_be_bytes([bytes[108], bytes[109]]) as usize;
    let mut huge = varint(1 << 62);
    huge.truncate(9);
    let (pre, _) = bytes.split_at_mut(cell_off + huge.len());
    pre[cell_off..].copy_from_slice(&huge);
    assert!(Database::open(bytes).is_err());
}

#[test]
fn bad_schema_sql_is_reported_per_table() {
    let mut bytes = chain_db(1);
    let cell_off = u16::from_be_bytes([bytes[108], bytes[109]]) as usize;
    let sql_at = bytes[cell_off..].windows(6).position(|w| w == b"CREATE").unwrap() + cell_off;
    bytes[sql_at..sql_at + 6].copy_from_slice(b"BOGUS!");
    let db = Database::open(bytes).unwrap();
    assert!(db.table("t").unwrap().parse_error.is_some());
    assert!(matches!(db.rows("t"), Err(Error::Schema { .. })));
    assert!(matches!(db.column_names("t"), Err(Error::Schema { .. })));
}
