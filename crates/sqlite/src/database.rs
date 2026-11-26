//! The database: header, schema and access to table rows.

use std::collections::BTreeMap;
use std::path::Path;

use crate::btree::{PageSource, RowLayout, Rows};
use crate::header::Header;
use crate::record::{Row, Value};
use crate::schema::{TableInfo, TableKind};
use crate::{Error, Result};

/// One row of `sqlite_schema` (tables, indexes, views and triggers).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaEntry {
    /// `table`, `index`, `view` or `trigger`.
    pub kind: String,
    pub name: String,
    pub tbl_name: String,
    /// Root b-tree page; 0 for views, triggers and virtual tables.
    pub root_page: u32,
    /// The `CREATE` statement; empty for automatic indexes.
    pub sql: String,
}

/// A read-only SQLite database held in memory.
#[derive(Debug, Clone)]
pub struct Database {
    bytes: Vec<u8>,
    header: Header,
    schema: Vec<SchemaEntry>,
    tables: Vec<TableInfo>,
}

/// The built-in table that describes the schema, rooted at page 1.
const SCHEMA_TABLE_NAMES: [&str; 4] = [
    "sqlite_schema",
    "sqlite_master",
    "sqlite_temp_schema",
    "sqlite_temp_master",
];
const SCHEMA_COLUMNS: [&str; 5] = ["type", "name", "tbl_name", "rootpage", "sql"];

impl Database {
    /// Open a database image, taking ownership of the bytes.
    pub fn open(bytes: Vec<u8>) -> Result<Self> {
        let header = Header::parse(&bytes)?;
        let mut db = Self {
            bytes,
            header,
            schema: Vec::new(),
            tables: Vec::new(),
        };
        db.load_schema()?;
        Ok(db)
    }

    /// Open a database image from a borrowed buffer (the bytes are copied).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        Self::open(bytes.to_vec())
    }

    /// Read and open a database file. Any `-wal` or `-journal` file next to it is ignored.
    pub fn open_path(path: impl AsRef<Path>) -> Result<Self> {
        Self::open(std::fs::read(path)?)
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    /// Every entry of `sqlite_schema`, in storage order.
    pub fn schema(&self) -> &[SchemaEntry] {
        &self.schema
    }

    /// The tables (not views, indexes or triggers), in schema order.
    pub fn tables(&self) -> &[TableInfo] {
        &self.tables
    }

    /// The table called `name`: exact match first, then ASCII case-insensitive like SQLite.
    pub fn table(&self, name: &str) -> Option<&TableInfo> {
        self.tables
            .iter()
            .find(|t| t.name == name)
            .or_else(|| self.tables.iter().find(|t| t.name.eq_ignore_ascii_case(name)))
    }

    /// The column names of `table`, in declaration order.
    pub fn column_names(&self, table: &str) -> Result<Vec<String>> {
        if is_schema_table(table) && self.table(table).is_none() {
            return Ok(SCHEMA_COLUMNS.iter().map(|s| (*s).to_string()).collect());
        }
        let t = self
            .table(table)
            .ok_or_else(|| Error::NoSuchTable(table.to_string()))?;
        if let Some(reason) = &t.parse_error {
            return Err(Error::Schema {
                table: t.name.clone(),
                reason: reason.clone(),
            });
        }
        Ok(t.column_names())
    }

    /// Iterate over the rows of a rowid table in rowid order. `sqlite_schema` (or
    /// `sqlite_master`) can be read too.
    pub fn rows(&self, table: &str) -> Result<Rows<'_>> {
        if is_schema_table(table) && self.table(table).is_none() {
            return Rows::new(self.source(), 1, RowLayout::plain(SCHEMA_COLUMNS.len()));
        }
        let t = self
            .table(table)
            .ok_or_else(|| Error::NoSuchTable(table.to_string()))?;
        if let Some(reason) = &t.parse_error {
            return Err(Error::Schema {
                table: t.name.clone(),
                reason: reason.clone(),
            });
        }
        match t.kind {
            TableKind::Rowid => {}
            TableKind::WithoutRowid => {
                return Err(Error::Unsupported(format!(
                    "`{}` is a WITHOUT ROWID table",
                    t.name
                )));
            }
            TableKind::Virtual => {
                return Err(Error::Unsupported(format!("`{}` is a virtual table", t.name)));
            }
        }
        Rows::new(self.source(), t.root_page, t.layout())
    }

    /// All rows of `table`, collected.
    pub fn rows_vec(&self, table: &str) -> Result<Vec<Row>> {
        self.rows(table)?.collect()
    }

    /// All rows of `table` as column-name → value maps.
    pub fn query_all(&self, table: &str) -> Result<Vec<BTreeMap<String, Value>>> {
        let names = self.column_names(table)?;
        self.rows(table)?
            .map(|row| {
                let row = row?;
                Ok(names.iter().cloned().zip(row.values).collect())
            })
            .collect()
    }

    /// The raw database image.
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) fn source(&self) -> PageSource<'_> {
        PageSource {
            bytes: &self.bytes,
            page_size: self.header.page_size,
            usable_size: self.header.usable_size,
            page_count: self.header.page_count,
            encoding: self.header.text_encoding,
        }
    }

    fn load_schema(&mut self) -> Result<()> {
        let mut schema = Vec::new();
        for row in Rows::new(self.source(), 1, RowLayout::plain(SCHEMA_COLUMNS.len()))? {
            let row = row?;
            let text = |i: usize| {
                row.values
                    .get(i)
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string()
            };
            let root_page = match row.values.get(3) {
                Some(Value::Integer(n)) => u32::try_from(*n).unwrap_or(0),
                _ => 0,
            };
            schema.push(SchemaEntry {
                kind: text(0),
                name: text(1),
                tbl_name: text(2),
                root_page,
                sql: text(4),
            });
        }
        self.tables = schema
            .iter()
            .filter(|e| e.kind == "table")
            .map(|e| TableInfo::from_schema(e.name.clone(), e.root_page, e.sql.clone()))
            .collect();
        self.schema = schema;
        Ok(())
    }
}

fn is_schema_table(name: &str) -> bool {
    SCHEMA_TABLE_NAMES.iter().any(|n| n.eq_ignore_ascii_case(name))
}
