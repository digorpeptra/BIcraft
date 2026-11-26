//! bicraft's own data snapshot: the rows a refresh loaded, in a file next to a PBIP project
//! (`<Name>.SemanticModel/.pbi/bicraft-data.bin`), read back by the engine's DAX evaluator.
//!
//! The format belongs to bicraft and is not an Analysis Services backup: Power BI Desktop never
//! reads it, and bicraft never writes `.pbi/cache.abf`. Layout (all integers little-endian):
//!
//! | Bytes | Content |
//! |---|---|
//! | 16 | magic `BICRAFT-DATA` + four NUL bytes |
//! | 4 | format version (1) |
//! | 8 | length of the body |
//! | n | the body, raw DEFLATE (RFC 1951) |
//!
//! The body: `refreshed_at` and `producer` (strings), a count and that many `(key, value)`
//! string notes, a table count, then per table: name, fingerprint (`u64`, [`fingerprint`]),
//! origin, row count (`u64`), column count (`u32`), and per column its name, data type (`u8`)
//! and one tagged value per row (0 blank, 1 `i64`, 2 double, 3 fixed decimal ×10 000, 4 date
//! (double), 5 boolean, 6 text). Strings are a `u32` byte length and UTF-8. See
//! `docs/formats/data-sources.md`, *bicraft data snapshot*.

use crate::{CellValue, ColumnData, DataType, Error, Limits, TableData};

const MAGIC: &[u8; 16] = b"BICRAFT-DATA\0\0\0\0";
const VERSION: u32 = 1;

/// One refreshed table.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredTable {
    pub name: String,
    /// What the rows were loaded from ([`fingerprint`] of the partition and what it reads):
    /// a table whose definition changed since is stale.
    pub fingerprint: u64,
    /// `m` (a Power Query partition), `dax` (a calculated table) or `columns` (calculated
    /// columns added to an imported table, which is then also stored).
    pub origin: String,
    pub data: TableData,
}

/// A refresh's tables.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snapshot {
    /// ISO 8601 UTC time the refresh started (`2026-10-23T08:15:00Z`).
    pub refreshed_at: String,
    /// The program that wrote it (`bicraft 0.1.0`).
    pub producer: String,
    /// Free `(key, value)` notes (connector, host); never a credential.
    pub notes: Vec<(String, String)>,
    pub tables: Vec<StoredTable>,
}

impl Snapshot {
    /// A table by name (exact, then case-insensitive).
    pub fn table(&self, name: &str) -> Option<&StoredTable> {
        self.tables
            .iter()
            .find(|t| t.name == name)
            .or_else(|| self.tables.iter().find(|t| t.name.eq_ignore_ascii_case(name)))
    }
}

/// A stable 64-bit FNV-1a hash of `parts` (each length-prefixed), for [`StoredTable::fingerprint`].
pub fn fingerprint(parts: &[&str]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for b in bytes {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    };
    for p in parts {
        eat(&(p.len() as u64).to_le_bytes());
        eat(p.as_bytes());
    }
    h
}

fn type_code(t: DataType) -> u8 {
    match t {
        DataType::Unknown => 0,
        DataType::String => 1,
        DataType::Int64 => 2,
        DataType::Double => 3,
        DataType::Decimal => 4,
        DataType::DateTime => 5,
        DataType::Boolean => 6,
        DataType::Binary => 7,
        DataType::Variant => 8,
    }
}

fn type_of(code: u8) -> Option<DataType> {
    Some(match code {
        0 => DataType::Unknown,
        1 => DataType::String,
        2 => DataType::Int64,
        3 => DataType::Double,
        4 => DataType::Decimal,
        5 => DataType::DateTime,
        6 => DataType::Boolean,
        7 => DataType::Binary,
        8 => DataType::Variant,
        _ => return None,
    })
}

fn put_str(out: &mut Vec<u8>, s: &str) -> Result<(), Error> {
    let n = u32::try_from(s.len()).map_err(|_| Error::LimitExceeded("a text over 4 GiB".into()))?;
    out.extend_from_slice(&n.to_le_bytes());
    out.extend_from_slice(s.as_bytes());
    Ok(())
}

/// The file bytes of `snap`.
pub fn write_snapshot(snap: &Snapshot) -> Result<Vec<u8>, Error> {
    let mut body = Vec::new();
    put_str(&mut body, &snap.refreshed_at)?;
    put_str(&mut body, &snap.producer)?;
    body.extend_from_slice(&(snap.notes.len() as u32).to_le_bytes());
    for (k, v) in &snap.notes {
        put_str(&mut body, k)?;
        put_str(&mut body, v)?;
    }
    body.extend_from_slice(&(snap.tables.len() as u32).to_le_bytes());
    for t in &snap.tables {
        put_str(&mut body, &t.name)?;
        body.extend_from_slice(&t.fingerprint.to_le_bytes());
        put_str(&mut body, &t.origin)?;
        body.extend_from_slice(&(t.data.row_count() as u64).to_le_bytes());
        body.extend_from_slice(&(t.data.columns().len() as u32).to_le_bytes());
        for c in t.data.columns() {
            put_str(&mut body, &c.name)?;
            body.push(type_code(c.data_type));
            for v in &c.values {
                match v {
                    CellValue::Null => body.push(0),
                    CellValue::Int(n) => {
                        body.push(1);
                        body.extend_from_slice(&n.to_le_bytes());
                    }
                    CellValue::Real(x) => {
                        body.push(2);
                        body.extend_from_slice(&x.to_bits().to_le_bytes());
                    }
                    CellValue::Decimal(n) => {
                        body.push(3);
                        body.extend_from_slice(&n.to_le_bytes());
                    }
                    CellValue::DateTime(x) => {
                        body.push(4);
                        body.extend_from_slice(&x.to_bits().to_le_bytes());
                    }
                    CellValue::Bool(b) => {
                        body.push(5);
                        body.push(u8::from(*b));
                    }
                    CellValue::Text(s) => {
                        body.push(6);
                        put_str(&mut body, s)?;
                    }
                }
            }
        }
    }
    let packed = miniz_oxide::deflate::compress_to_vec(&body, 6);
    let mut out = Vec::with_capacity(packed.len().saturating_add(28));
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&(packed.len() as u64).to_le_bytes());
    out.extend_from_slice(&packed);
    Ok(out)
}

/// Whether `bytes` start like a snapshot file.
pub fn is_snapshot(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

fn bad(what: &str) -> Error {
    Error::InvalidTable(format!("bicraft data file: {what}"))
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self.at.checked_add(n).ok_or_else(|| bad("truncated"))?;
        let s = self.b.get(self.at..end).ok_or_else(|| bad("truncated"))?;
        self.at = end;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8, Error> {
        Ok(*self.take(1)?.first().ok_or_else(|| bad("truncated"))?)
    }

    fn u32(&mut self) -> Result<u32, Error> {
        let s = self.take(4)?;
        let mut a = [0u8; 4];
        a.copy_from_slice(s);
        Ok(u32::from_le_bytes(a))
    }

    fn u64(&mut self) -> Result<u64, Error> {
        let s = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(s);
        Ok(u64::from_le_bytes(a))
    }

    fn string(&mut self, max: usize) -> Result<String, Error> {
        let n = self.u32()? as usize;
        if n > max {
            return Err(Error::LimitExceeded(format!(
                "bicraft data file: a text of {n} bytes"
            )));
        }
        let s = self.take(n)?;
        String::from_utf8(s.to_vec()).map_err(|_| bad("text is not UTF-8"))
    }
}

/// Reads a snapshot file, within `limits` (`max_bytes` caps the file and its body, `max_rows`
/// the rows of a table, `max_columns` its columns, `max_cell_bytes` a text).
pub fn read_snapshot(bytes: &[u8], limits: &Limits) -> Result<Snapshot, Error> {
    limits.check_bytes(bytes.len(), "bicraft data file")?;
    let mut head = Reader { b: bytes, at: 0 };
    if head.take(MAGIC.len())? != MAGIC {
        return Err(bad("not a bicraft data file (refresh again)"));
    }
    let version = head.u32()?;
    if version != VERSION {
        return Err(bad(&format!(
            "format version {version} (this bicraft reads version {VERSION}): refresh again"
        )));
    }
    let len = usize::try_from(head.u64()?).map_err(|_| bad("body too large"))?;
    let packed = head.take(len)?;
    let body =
        miniz_oxide::inflate::decompress_to_vec_with_limit(packed, limits.max_bytes).map_err(|_| {
            Error::LimitExceeded(format!(
                "bicraft data file: body over {} bytes or corrupt",
                limits.max_bytes
            ))
        })?;
    let mut r = Reader { b: &body, at: 0 };
    let cell = limits.max_cell_bytes;
    let refreshed_at = r.string(cell)?;
    let producer = r.string(cell)?;
    let notes_n = r.u32()? as usize;
    if notes_n > 4096 {
        return Err(bad("too many notes"));
    }
    let mut notes = Vec::with_capacity(notes_n);
    for _ in 0..notes_n {
        notes.push((r.string(cell)?, r.string(cell)?));
    }
    let tables_n = r.u32()? as usize;
    if tables_n > 100_000 {
        return Err(bad("too many tables"));
    }
    let mut tables = Vec::with_capacity(tables_n.min(1024));
    for _ in 0..tables_n {
        let name = r.string(cell)?;
        let fingerprint = r.u64()?;
        let origin = r.string(cell)?;
        let rows = usize::try_from(r.u64()?).map_err(|_| bad("row count"))?;
        limits.check_rows(rows).map_err(|_| {
            Error::LimitExceeded(format!("bicraft data file: table '{name}' has {rows} rows"))
        })?;
        let cols_n = r.u32()? as usize;
        limits.check_columns(cols_n)?;
        let mut columns = Vec::with_capacity(cols_n);
        for _ in 0..cols_n {
            let cname = r.string(cell)?;
            let data_type = type_of(r.u8()?).ok_or_else(|| bad("data type"))?;
            // Every value takes at least one byte: a row count beyond the body is corrupt.
            if rows > body.len().saturating_sub(r.at) {
                return Err(bad("truncated"));
            }
            let mut values = Vec::with_capacity(rows);
            for _ in 0..rows {
                let f64_of = |r: &mut Reader<'_>| r.u64().map(f64::from_bits);
                values.push(match r.u8()? {
                    0 => CellValue::Null,
                    1 => CellValue::Int(r.u64()? as i64),
                    2 => CellValue::Real(f64_of(&mut r)?),
                    3 => CellValue::Decimal(r.u64()? as i64),
                    4 => CellValue::DateTime(f64_of(&mut r)?),
                    5 => CellValue::Bool(r.u8()? != 0),
                    6 => CellValue::Text(r.string(cell)?),
                    _ => return Err(bad("value tag")),
                });
            }
            columns.push(ColumnData {
                name: cname,
                data_type,
                values,
            });
        }
        let data = TableData::new(columns)?;
        if data.row_count() != rows && !data.columns().is_empty() {
            return Err(bad("row count"));
        }
        tables.push(StoredTable {
            name,
            fingerprint,
            origin,
            data,
        });
    }
    if r.at != body.len() {
        return Err(bad("trailing bytes"));
    }
    Ok(Snapshot {
        refreshed_at,
        producer,
        notes,
        tables,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Snapshot {
        let data = TableData::new(vec![
            ColumnData {
                name: "Libellé".into(),
                data_type: DataType::String,
                values: vec![
                    CellValue::Text("Café".into()),
                    CellValue::Null,
                    CellValue::Text(String::new()),
                ],
            },
            ColumnData {
                name: "Montant".into(),
                data_type: DataType::Decimal,
                values: vec![
                    CellValue::Decimal(-125_000),
                    CellValue::Decimal(0),
                    CellValue::Null,
                ],
            },
            ColumnData {
                name: "Date".into(),
                data_type: DataType::DateTime,
                values: vec![
                    CellValue::DateTime(46_027.5),
                    CellValue::Null,
                    CellValue::DateTime(-1.25),
                ],
            },
            ColumnData {
                name: "N".into(),
                data_type: DataType::Int64,
                values: vec![
                    CellValue::Int(i64::MIN),
                    CellValue::Int(i64::MAX),
                    CellValue::Null,
                ],
            },
            ColumnData {
                name: "X".into(),
                data_type: DataType::Double,
                values: vec![
                    CellValue::Real(f64::NAN),
                    CellValue::Real(-0.0),
                    CellValue::Real(1e300),
                ],
            },
            ColumnData {
                name: "B".into(),
                data_type: DataType::Boolean,
                values: vec![CellValue::Bool(true), CellValue::Bool(false), CellValue::Null],
            },
        ])
        .unwrap();
        Snapshot {
            refreshed_at: "2026-10-23T08:00:00Z".into(),
            producer: "bicraft test".into(),
            notes: vec![("connector".into(), "databricks".into())],
            tables: vec![
                StoredTable {
                    name: "Transactions".into(),
                    fingerprint: fingerprint(&["a", "b"]),
                    origin: "m".into(),
                    data,
                },
                StoredTable {
                    name: "Vide".into(),
                    fingerprint: 7,
                    origin: "dax".into(),
                    data: TableData::new(vec![]).unwrap(),
                },
            ],
        }
    }

    #[test]
    fn round_trip() {
        let s = sample();
        let bytes = write_snapshot(&s).unwrap();
        assert!(is_snapshot(&bytes));
        let back = read_snapshot(&bytes, &Limits::default()).unwrap();
        assert_eq!(back.refreshed_at, s.refreshed_at);
        assert_eq!(back.notes, s.notes);
        assert_eq!(back.tables.len(), 2);
        let (a, b) = (&back.tables[0].data, &s.tables[0].data);
        assert_eq!(a.row_count(), 3);
        for (x, y) in a.columns().iter().zip(b.columns()) {
            assert_eq!(x.name, y.name);
            assert_eq!(x.data_type, y.data_type);
            for (p, q) in x.values.iter().zip(&y.values) {
                match (p, q) {
                    (CellValue::Real(p), CellValue::Real(q)) => assert_eq!(p.to_bits(), q.to_bits()),
                    _ => assert_eq!(p, q),
                }
            }
        }
        assert_eq!(back.table("transactions").map(|t| t.origin.as_str()), Some("m"));
    }

    #[test]
    fn fingerprint_is_stable_and_length_prefixed() {
        assert_eq!(fingerprint(&[]), 0xcbf2_9ce4_8422_2325);
        assert_ne!(fingerprint(&["ab", "c"]), fingerprint(&["a", "bc"]));
        assert_eq!(fingerprint(&["x"]), fingerprint(&["x"]));
    }

    #[test]
    fn hostile_files_are_errors() {
        let bytes = write_snapshot(&sample()).unwrap();
        // every truncation and many single-byte corruptions fail cleanly
        for n in 0..bytes.len() {
            assert!(
                read_snapshot(&bytes[..n], &Limits::default()).is_err(),
                "truncated at {n}"
            );
        }
        let mut seed = 12345u32;
        for _ in 0..2000 {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            let mut b = bytes.clone();
            let i = (seed as usize >> 4) % b.len();
            b[i] ^= (seed >> 24) as u8 | 1;
            let _ = read_snapshot(&b, &Limits::default());
        }
        let tiny = Limits {
            max_rows: 2,
            ..Limits::default()
        };
        assert!(matches!(
            read_snapshot(&bytes, &tiny),
            Err(Error::LimitExceeded(_))
        ));
        let mut wrong = bytes.clone();
        wrong[16] = 9;
        let e = read_snapshot(&wrong, &Limits::default()).unwrap_err();
        assert!(e.to_string().contains("version 9"), "{e}");
    }
}
