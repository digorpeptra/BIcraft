//! L0: VertiPaq column storage reader.
//!
//! Decodes the column files that a Power BI / Analysis Services Tabular backup (ABF) keeps
//! per column: `.idf` (one data id per row, hybrid RLE + bit packing), `.idfmeta` (segment
//! map and statistics), `.dictionary` (the distinct values of hash-encoded columns, with
//! Huffman-compressed string pages) and `.hidx` (hash index, parsed for validation).
//!
//! All functions are pure functions over byte slices; the caller extracts the files from the
//! ABF and reads the encoding from `metadata.sqlitedb` (`DictionaryStorage`). Hostile input
//! yields an [`Error`], never a panic, and row counts and decoded bytes are capped by
//! [`Limits`].
//!
//! The byte layouts are described in `docs/formats/vertipaq.md`.
//!
//! ```
//! use bicraft_vertipaq::{decode_column, CellValue, ColumnEncoding, StorageType};
//! # fn demo(idf: &[u8], idfmeta: &[u8]) -> Result<(), bicraft_vertipaq::Error> {
//! // A value-encoded integer column: value = data id + BaseId (Magnitude 1).
//! let encoding = ColumnEncoding::value(-2, 1.0, StorageType::Long);
//! let cells: Vec<CellValue> = decode_column(idf, idfmeta, None, encoding)?;
//! # let _ = cells; Ok(()) }
//! ```

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

pub mod bitpack;
mod column;
pub mod compression;
mod cursor;
pub mod dictionary;
pub mod encode;
pub mod hidx;
pub mod huffman;
pub mod idf;
pub mod idfmeta;
pub mod system;

pub use column::{
    CellValue, ColumnEncoding, EncodingKind, LogicalType, StorageType, convert, decode_column,
    decode_column_with_limits, ids_to_values,
};
pub use compression::{Compression, InnerCompression};
pub use dictionary::{Dictionary, DictionaryValues, parse_dictionary};
pub use encode::{
    ColumnStats, EncodeOptions, EncodedColumn, EncodingRequest, SegmentStatsOut, StringCompression,
    ValueParams, encode_column, encode_row_number, encode_row_number_from,
};
pub use hidx::{HashIndex, parse_hash_index};
pub use idf::{FIRST_DATA_ID, NULL_DATA_ID, decode_data_ids, split_idf};
pub use idfmeta::{ColumnMeta, SegmentMeta, parse_idfmeta};
pub use system::{
    AttributeHierarchy, Collation, RelationshipIndex, SystemTableData, SystemTableError, UserHierarchy,
    build_attribute_hierarchy, build_relationship_index, build_user_hierarchy,
};

/// Default cap on the rows of one decoded column: 2^24, sixteen Power BI Desktop segments
/// (2^20 rows each) or two Analysis Services ones (2^23). Callers that know the table's row
/// count (from `metadata.sqlitedb`) should pass it instead.
pub const DEFAULT_MAX_ROWS: u64 = 1 << 24;

/// Default cap on the memory one decode produces (data ids, cells and their text): 1 GiB.
pub const DEFAULT_MAX_BYTES: u64 = 1 << 30;

/// Resource limits for decoding.
///
/// RLE runs legitimately describe many rows in a few bytes (a constant column of any length
/// is a single 8-byte run), so the input size cannot bound the output. These caps do, and
/// they are checked before the memory is allocated (docs/fuzzing.md, F2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Maximum rows of one column.
    pub max_rows: u64,
    /// Maximum bytes of decoded output: 4 per data id, plus `size_of::<CellValue>()` per cell
    /// and the bytes of every text cell for [`decode_column`].
    pub max_bytes: u64,
}

impl Limits {
    /// The default limits ([`DEFAULT_MAX_ROWS`], [`DEFAULT_MAX_BYTES`]), usable in `const`.
    pub const DEFAULT: Limits = Limits {
        max_rows: DEFAULT_MAX_ROWS,
        max_bytes: DEFAULT_MAX_BYTES,
    };
}

impl Default for Limits {
    fn default() -> Self {
        Limits::DEFAULT
    }
}

/// Which file an error is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Idf,
    IdfMeta,
    Dictionary,
    Hidx,
    Column,
}

impl std::fmt::Display for FileKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            FileKind::Idf => ".idf",
            FileKind::IdfMeta => ".idfmeta",
            FileKind::Dictionary => ".dictionary",
            FileKind::Hidx => ".hidx",
            FileKind::Column => "column",
        })
    }
}

/// Decoding errors.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("{kind}: truncated at byte {offset} while reading {what}")]
    Truncated {
        kind: FileKind,
        offset: usize,
        what: &'static str,
    },
    #[error("{kind}: malformed at byte {offset}: {message}")]
    Malformed {
        kind: FileKind,
        offset: usize,
        message: String,
    },
    #[error("{kind}: unsupported {message}")]
    Unsupported { kind: FileKind, message: String },
    #[error("limit exceeded: {0}")]
    LimitExceeded(String),
    #[error("a hash-encoded column needs its .dictionary")]
    MissingDictionary,
    #[error("data id {id} is outside the dictionary ({len} values)")]
    DataIdOutOfRange { id: u32, len: usize },
    #[error("value of data id {id} overflows")]
    ValueOverflow { id: u32 },
}
