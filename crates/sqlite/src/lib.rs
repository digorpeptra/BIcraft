//! L0: a reader and writer for SQLite database files, in pure Rust.
//!
//! Power BI's `DataModel` keeps the tabular model metadata in an SQLite database
//! (`metadata.sqlitedb`) inside the ABF backup. This crate reads such a file from memory and
//! exposes its tables and rows, and writes new or rewritten database images
//! ([`DatabaseBuilder`], [`rewrite`]). It is written from the public file-format description
//! (<https://www.sqlite.org/fileformat2.html>); see `docs/formats/sqlite.md` for the supported
//! subset.
//!
//! ```no_run
//! # fn main() -> bicraft_sqlite::Result<()> {
//! let db = bicraft_sqlite::Database::open_path("metadata.sqlitedb")?;
//! for table in db.tables() {
//!     println!("{} ({} columns)", table.name, table.columns.len());
//! }
//! for row in db.rows("Table")? {
//!     let row = row?;
//!     println!("{} {:?}", row.rowid, row.values);
//! }
//! # Ok(()) }
//! ```
//!
//! Input files are treated as hostile: malformed input yields an [`Error`], never a panic, an
//! endless loop or an allocation larger than the file itself.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable,
    clippy::indexing_slicing
)]
#![cfg_attr(test, allow(clippy::indexing_slicing))]

mod btree;
mod database;
mod header;
mod record;
mod schema;
mod write;

pub use btree::Rows;
pub use database::{Database, SchemaEntry};
pub use header::{Header, TextEncoding};
pub use record::{Row, Value};
pub use schema::{Affinity, ColumnInfo, Generated, TableInfo, TableKind};
pub use write::{DatabaseBuilder, rewrite};

/// Everything that can go wrong while reading a database file.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("i/o: {0}")]
    Io(#[from] std::io::Error),
    #[error("not an SQLite database: {0}")]
    NotSqlite(&'static str),
    #[error("corrupt database: {0}")]
    Corrupt(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("no such table: `{0}`")]
    NoSuchTable(String),
    #[error("cannot parse the schema of table `{table}`: {reason}")]
    Schema { table: String, reason: String },
    /// The writer was given something it cannot store (bad page size, duplicate rowid, wrong
    /// number of values, a UNIQUE violation, a size limit...).
    #[error("invalid input: {0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn corrupt(msg: impl Into<String>) -> Error {
    Error::Corrupt(msg.into())
}
