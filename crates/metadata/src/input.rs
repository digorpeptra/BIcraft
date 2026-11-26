//! The input abstraction: TMSCHEMA tables as plain rows, independent of any SQLite reader.

use std::collections::BTreeMap;

/// One cell of a metadata table, with SQLite's storage classes.
#[derive(Debug, Clone, PartialEq)]
pub enum MetaValue {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

/// A row: column name → value.
pub type MetaRow = BTreeMap<String, MetaValue>;

/// The tables of a `metadata.sqlitedb` (or any source with the same shape), keyed by table name
/// (`Model`, `Table`, `Column`, …). Lookups try the exact name first, then an ASCII
/// case-insensitive match, as SQLite does. A missing table reads as empty.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MetadataTables {
    tables: BTreeMap<String, Vec<MetaRow>>,
}

impl MetadataTables {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds (or replaces) a whole table.
    pub fn insert_table(&mut self, name: impl Into<String>, rows: Vec<MetaRow>) {
        self.tables.insert(name.into(), rows);
    }

    /// Appends one row to `table`, creating the table if needed.
    pub fn push_row(&mut self, table: &str, row: MetaRow) {
        self.tables.entry(table.to_string()).or_default().push(row);
    }

    /// The rows of `table`; empty when the table is absent.
    pub fn rows(&self, table: &str) -> &[MetaRow] {
        self.tables
            .get(table)
            .or_else(|| {
                self.tables
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case(table))
                    .map(|(_, v)| v)
            })
            .map_or(&[], Vec::as_slice)
    }

    pub fn has_table(&self, table: &str) -> bool {
        self.tables.keys().any(|k| k.eq_ignore_ascii_case(table))
    }

    /// Table names, sorted.
    pub fn table_names(&self) -> impl Iterator<Item = &str> {
        self.tables.keys().map(String::as_str)
    }

    /// The stored name of `table`, resolved like [`rows`](Self::rows).
    pub(crate) fn key(&self, table: &str) -> Option<&str> {
        if self.tables.contains_key(table) {
            return self.tables.get_key_value(table).map(|(k, _)| k.as_str());
        }
        self.tables
            .keys()
            .find(|k| k.eq_ignore_ascii_case(table))
            .map(String::as_str)
    }

    /// Mutable rows of `table`, resolved like [`rows`](Self::rows).
    pub(crate) fn rows_mut(&mut self, table: &str) -> Option<&mut Vec<MetaRow>> {
        let key = self.key(table)?.to_string();
        self.tables.get_mut(&key)
    }

    /// Every table with its rows.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&str, &[MetaRow])> {
        self.tables.iter().map(|(k, v)| (k.as_str(), v.as_slice()))
    }
}

/// The key of `row` matching `col` (exact, then ASCII case-insensitive).
pub(crate) fn column_key<'r>(row: &'r MetaRow, col: &str) -> Option<&'r str> {
    if let Some((k, _)) = row.get_key_value(col) {
        return Some(k.as_str());
    }
    row.keys()
        .find(|k| k.eq_ignore_ascii_case(col))
        .map(String::as_str)
}

impl From<i64> for MetaValue {
    fn from(v: i64) -> Self {
        Self::Integer(v)
    }
}

impl From<f64> for MetaValue {
    fn from(v: f64) -> Self {
        Self::Real(v)
    }
}

impl From<&str> for MetaValue {
    fn from(v: &str) -> Self {
        Self::Text(v.to_string())
    }
}

impl From<String> for MetaValue {
    fn from(v: String) -> Self {
        Self::Text(v)
    }
}

impl From<bool> for MetaValue {
    fn from(v: bool) -> Self {
        Self::Integer(i64::from(v))
    }
}

/// A read-only view of one row with lenient typed accessors. Values of an unexpected storage
/// class read as absent, except that integral reals and numeric text are accepted as integers.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Row<'a>(pub(crate) &'a MetaRow);

impl<'a> Row<'a> {
    pub(crate) fn get(&self, col: &str) -> Option<&'a MetaValue> {
        self.0.get(col).or_else(|| {
            self.0
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(col))
                .map(|(_, v)| v)
        })
    }

    pub(crate) fn int(&self, col: &str) -> Option<i64> {
        match self.get(col)? {
            MetaValue::Integer(i) => Some(*i),
            MetaValue::Real(f) => {
                // Only exactly integral values in i64 range.
                if f.is_finite() && f.fract() == 0.0 && *f >= -9.0e18 && *f <= 9.0e18 {
                    Some(*f as i64)
                } else {
                    None
                }
            }
            MetaValue::Text(s) => s.trim().parse().ok(),
            MetaValue::Null | MetaValue::Blob(_) => None,
        }
    }

    /// The object ID (`ID`), when present.
    pub(crate) fn id(&self) -> Option<i64> {
        self.int("ID")
    }

    /// A reference to another object; `0` and absent both mean "none".
    pub(crate) fn reference(&self, col: &str) -> Option<i64> {
        self.int(col).filter(|&v| v != 0)
    }

    /// Text, including the empty string.
    pub(crate) fn raw_text(&self, col: &str) -> Option<&'a str> {
        match self.get(col)? {
            MetaValue::Text(s) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Non-empty text.
    pub(crate) fn text(&self, col: &str) -> Option<&'a str> {
        self.raw_text(col).filter(|s| !s.is_empty())
    }

    pub(crate) fn string(&self, col: &str) -> Option<String> {
        self.text(col).map(str::to_string)
    }

    /// A boolean stored as an integer; absent or unreadable gives `default`.
    pub(crate) fn flag(&self, col: &str, default: bool) -> bool {
        self.int(col).map_or(default, |v| v != 0)
    }
}
