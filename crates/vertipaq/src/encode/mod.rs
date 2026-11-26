//! VertiPaq column encoder: the inverse of [`crate::decode_column`].
//!
//! [`encode_column`] turns one column's values (all rows of one partition, in storage order)
//! into the bytes of its `.idf`, `.idfmeta`, `.dictionary` (hash encoding) and `.hidx` (value
//! encoding with a materialized attribute hierarchy), plus the [`ColumnStats`] that the
//! `metadata.sqlitedb` storage rows need. [`encode_row_number`] writes a table's RowNumber
//! column.
//!
//! The choices follow what Power BI Desktop writes (see "Encoder" in
//! `docs/formats/vertipaq.md`): dictionary ids in first-occurrence order, hybrid RLE segments
//! whose RLE entries are the runs of at least 64 equal ids, bit-packed sub-segments relative
//! to the segment minimum (the null id when the segment holds blanks), string pages of fewer
//! than 2^19 units, Huffman-coded from 8 192 units, and 1 048 576-row segments. Every
//! choice can be overridden through [`EncodeOptions`].

mod dictionary;
mod hidx;
mod huffman;
mod ids;
mod segments;

use crate::column::{CellValue, EncodingKind, LogicalType, StorageType};
use crate::idf::NULL_DATA_ID;
use crate::{DEFAULT_MAX_ROWS, Error};

pub use huffman::limited_code_lengths;

/// Default rows per segment (`SegmentMapStorage.RecordsPerSegment`): 2^20, the Power BI
/// Desktop default (Analysis Services uses 2^23).
pub const DEFAULT_SEGMENT_ROWS: u64 = 1 << 20;
/// Shortest run of equal ids that becomes an RLE entry ([MS-XLDM] §2.7.3.1: at least 64).
pub const DEFAULT_RLE_MIN_RUN: u32 = 64;

/// Which encoding to use.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum EncodingRequest {
    /// Desktop's observed choice: value encoding for decimals (when the ids fit) and for
    /// int64 columns of at most 20 rows per distinct value (when the ids fit), hash encoding
    /// for everything else.
    #[default]
    Auto,
    /// Hash encoding with a `.dictionary`.
    Hash,
    /// Value encoding (integers, decimals and doubles). `params` forces `BaseId`/`Magnitude`;
    /// `None` derives them from the data (the smallest value gets id 3; Magnitude is
    /// `10^-k` for the largest `k` that divides every integer or decimal value, and `10^k`
    /// for the smallest `k` that makes every double an integer, as the engine writes them).
    Value { params: Option<ValueParams> },
}

/// `DictionaryStorage.BaseId` and `Magnitude` of a value-encoded column.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ValueParams {
    pub base_id: i64,
    pub magnitude: f64,
}

/// Fewest UTF-16 units (terminators included) of a page the engine compresses: pages of
/// 8 190 units or fewer were stored plain, pages of 8 192 or more Huffman-coded (probed on
/// engine-written dictionaries; 8 191 was not observed).
pub const COMPRESS_PAGE_UNITS: u64 = 8192;
/// Most UTF-16 units (terminators included) of one string page: engine pages hold fewer than
/// 2^19 units, and the next string would have reached 2^19 (113 full pages observed).
pub const MAX_PAGE_UNITS: u64 = (1 << 19) - 1;

/// When string pages are Huffman-compressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StringCompression {
    /// The engine's rule: compress a page that holds at least [`COMPRESS_PAGE_UNITS`] units.
    #[default]
    LargePages,
    /// Never.
    Never,
    /// Compress a page when that makes it smaller.
    WhenSmaller,
    /// Compress every non-empty page.
    Always,
}

/// Encoder options. `Default` reproduces Power BI Desktop's observed choices.
#[derive(Debug, Clone, PartialEq)]
pub struct EncodeOptions {
    pub encoding: EncodingRequest,
    /// Hash encoding: values given ids first, in this order (e.g. the existing dictionary of
    /// a column being edited, so unchanged values keep their ids). Nulls and repeats are
    /// skipped; values of the data not listed here follow in first-occurrence order.
    pub dictionary_seed: Vec<CellValue>,
    /// Rows per segment (> 0, at most `u32::MAX`).
    pub segment_rows: u64,
    /// Shortest run that is written as an RLE entry (≥ 1); shorter runs are bit-packed.
    pub rle_min_run: u32,
    pub string_compression: StringCompression,
    /// Largest number of UTF-16 units (terminators included) in one string page (default
    /// [`MAX_PAGE_UNITS`], as the engine).
    pub max_page_chars: u64,
    /// Write a `.hidx` for a value-encoded column (Desktop does when its attribute
    /// hierarchy is materialized, `AttributeHierarchyStorage.MaterializationType` 1).
    pub build_hidx: bool,
    /// Long dictionaries: always use 8-byte elements (else 4 bytes when every value fits).
    pub wide_long_dictionary: bool,
    /// `CS.base_id` of hybrid segments: observed 1 on calculated-table columns and 0 on
    /// calculated columns; meaning unknown.
    pub segment_base_id: u64,
    /// Cap on the rows of one column.
    pub max_rows: u64,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        EncodeOptions {
            encoding: EncodingRequest::Auto,
            dictionary_seed: Vec::new(),
            segment_rows: DEFAULT_SEGMENT_ROWS,
            rle_min_run: DEFAULT_RLE_MIN_RUN,
            string_compression: StringCompression::LargePages,
            max_page_chars: MAX_PAGE_UNITS,
            build_hidx: true,
            wide_long_dictionary: false,
            segment_base_id: 1,
            max_rows: DEFAULT_MAX_ROWS,
        }
    }
}

/// The files of one encoded column partition and its statistics.
#[derive(Debug, Clone, PartialEq)]
pub struct EncodedColumn {
    pub idf: Vec<u8>,
    pub idfmeta: Vec<u8>,
    /// Hash encoding only.
    pub dictionary: Option<Vec<u8>>,
    /// Value encoding with [`EncodeOptions::build_hidx`] only.
    pub hidx: Option<Vec<u8>>,
    pub stats: ColumnStats,
}

/// What the metadata rows (`ColumnStorage`, `DictionaryStorage`, `SegmentMapStorage`,
/// `AttributeHierarchyStorage`) need to describe an encoded column.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnStats {
    /// `DictionaryStorage.Type`: [`EncodingKind::Hash`] (1) or [`EncodingKind::Value`] (2).
    pub kind: EncodingKind,
    /// `DictionaryStorage.DataType`.
    pub storage_type: StorageType,
    /// `DictionaryStorage.BaseId` (2 for hash encoding).
    pub base_id: i64,
    /// `DictionaryStorage.Magnitude` (0 for hash encoding).
    pub magnitude: f64,
    /// `DictionaryStorage.LastId`: `2 + dictionary length` (hash), 2 (value).
    pub last_id: u32,
    /// `DictionaryStorage.IsNullable`: blanks present (hash), always true (value).
    pub is_nullable: bool,
    /// `DictionaryStorage.IsUnique`: observed true only for hash-encoded columns with a 4-byte
    /// long dictionary whose rows are all distinct and non-blank (never for doubles, strings or
    /// 8-byte long dictionaries, even when every row is distinct).
    pub is_unique: bool,
    /// `DictionaryStorage.IsOperatingOn32`: 4-byte long dictionary.
    pub is_operating_on_32: bool,
    /// `DictionaryStorage.DictionaryFlags`: 3 for strings, else 0.
    pub dictionary_flags: i64,
    /// `DictionaryStorage.Size` where the in-memory size formula is known (value encoding,
    /// 4-byte long dictionaries, single uncompressed string page).
    pub dictionary_size: Option<u64>,
    /// Entries of the `.dictionary` (hash encoding), 0 otherwise.
    pub dictionary_len: u64,
    /// Longest string of a string dictionary, in UTF-16 units.
    pub longest_string: u64,
    pub rows: u64,
    /// Distinct non-blank values.
    pub distinct_values: u64,
    /// `max_data_id − min_data_id + 1` over non-blank ids (0 when there are none).
    pub id_range: u64,
    /// `ColumnStorage.Statistics_DistinctStates` as Desktop writes it: the distinct count;
    /// for value encoding with a `.hidx` the blank counts as a value, without a `.hidx` it is
    /// [`ColumnStats::id_range`].
    pub distinct_states: u64,
    /// Smallest and largest non-blank id over the whole column (2 when there are none).
    pub min_data_id: u32,
    pub max_data_id: u32,
    pub has_nulls: bool,
    /// `ColumnStorage.Statistics_OriginalMinSegmentDataID`: observed 2 on every column,
    /// although value-encoded segments carry 3 in their `.idfmeta`.
    pub original_min_segment_data_id: u32,
    /// `ColumnStorage.Statistics_XMType`: 0 long, 1 real, 2 string.
    pub xm_type: i64,
    /// `ColumnStorage.Statistics_DBType` (OLE DB type): 20 I8, 5 R8, 130 WSTR, 7 DATE,
    /// 6 CY, 11 BOOL.
    pub db_type: i64,
    /// Rows per segment (`SegmentMapStorage.RecordsPerSegment`).
    pub records_per_segment: u64,
    /// One entry per segment, in order. `ColumnStorage.Statistics_RLERuns` /
    /// `Statistics_OthersRLERuns` are the sums over every segment of every partition
    /// ([`ColumnStats::rle_runs`]), not the first segment's counts (checked on engine-written
    /// columns of up to 20 segments and 3 partitions).
    pub segments: Vec<SegmentStatsOut>,
}

/// Per-segment facts (mirrors the `.idfmeta` `CS`/`SS` elements).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SegmentStatsOut {
    pub records: u64,
    pub min_data_id: u32,
    pub max_data_id: u32,
    pub original_min_segment_data_id: u32,
    pub has_nulls: bool,
    /// RLE entries.
    pub rle_runs: u64,
    /// Bit-packed entries.
    pub others_rle_runs: u64,
    /// Rows stored in the bit-packed sub-segment.
    pub bit_packed_rows: u64,
    /// `XMRENoSplit` width of the sub-segment (0 for XM123).
    pub bit_width: u8,
    pub rle_entries_allocated: u64,
}

impl ColumnStats {
    /// `SegmentMapStorage.SegmentCount`.
    pub fn segment_count(&self) -> u64 {
        self.segments.len() as u64
    }

    /// RLE entries over all segments (`ColumnStorage.Statistics_RLERuns` of a one-partition
    /// column; add the partitions' counts for a multi-partition one).
    pub fn rle_runs(&self) -> u64 {
        self.segments
            .iter()
            .fold(0u64, |a, s| a.saturating_add(s.rle_runs))
    }

    /// Bit-packed entries over all segments (`ColumnStorage.Statistics_OthersRLERuns`, summed
    /// like [`ColumnStats::rle_runs`]).
    pub fn others_rle_runs(&self) -> u64 {
        self.segments
            .iter()
            .fold(0u64, |a, s| a.saturating_add(s.others_rle_runs))
    }
}

/// Encodes a column. `values` are the rows in storage order; `logical_type` is the column's
/// TOM data type. Hostile or oversized input yields an [`Error`], never a panic.
pub fn encode_column(
    values: &[CellValue],
    logical_type: LogicalType,
    options: &EncodeOptions,
) -> Result<EncodedColumn, Error> {
    check_common(values.len() as u64, options)?;
    let assigned = ids::assign(values, logical_type, options)?;
    let value_encoded = assigned.kind == EncodingKind::Value;
    let (idf, idfmeta, segs) = segments::encode_hybrid(&assigned.ids, value_encoded, options)?;
    // A hash-encoded column holding only blanks has no `.dictionary` (engine-written:
    // `DictionaryStorage.StorageFileID` 0).
    let only_blanks = assigned.kind == EncodingKind::Hash && assigned.distinct == 0 && !values.is_empty();
    let dictionary = match &assigned.dictionary {
        Some(_) if only_blanks => None,
        Some(d) => Some(dictionary::write(d, options)?),
        None => None,
    };
    let hidx = if value_encoded && options.build_hidx {
        Some(hidx::write(&assigned.ids)?)
    } else {
        None
    };
    let stats = column_stats(
        &assigned,
        logical_type,
        &dictionary,
        hidx.is_some(),
        segs,
        options,
    );
    Ok(EncodedColumn {
        idf,
        idfmeta,
        dictionary: dictionary.map(|d| d.bytes),
        hidx,
        stats,
    })
}

/// Encodes the RowNumber column of a table with `rows` rows: value encoding with
/// `BaseId` −3 and `Magnitude` 1 (row `i` has id `i + 3`), XM123 segments.
pub fn encode_row_number(rows: u64, options: &EncodeOptions) -> Result<EncodedColumn, Error> {
    encode_row_number_from(0, rows, options)
}

/// Encodes the RowNumber column of one partition of a multi-partition table: its rows are the
/// table's rows `first_row ..first_row + rows` (partitions in storage order), so its ids start
/// at `first_row + 3` (observed on engine-written tables of 2 and 3 partitions).
pub fn encode_row_number_from(
    first_row: u64,
    rows: u64,
    options: &EncodeOptions,
) -> Result<EncodedColumn, Error> {
    check_common(rows, options)?;
    let (idf, idfmeta, segs) = segments::encode_row_number(first_row, rows, options)?;
    let first = first_row.checked_add(3).ok_or_else(|| too_many(rows))?;
    let min = u32::try_from(first).map_err(|_| too_many(rows))?;
    let max = u32::try_from(first.saturating_add(rows).saturating_sub(1)).map_err(|_| too_many(rows))?;
    let stats = ColumnStats {
        kind: EncodingKind::Value,
        storage_type: StorageType::Long,
        base_id: -3,
        magnitude: 1.0,
        last_id: NULL_DATA_ID,
        is_nullable: true,
        is_unique: false,
        is_operating_on_32: false,
        dictionary_flags: 0,
        dictionary_size: Some(VALUE_DICTIONARY_SIZE),
        dictionary_len: 0,
        longest_string: 0,
        rows,
        distinct_values: rows,
        id_range: rows,
        distinct_states: rows,
        min_data_id: if rows == 0 { NULL_DATA_ID } else { min },
        max_data_id: if rows == 0 { NULL_DATA_ID } else { max },
        has_nulls: false,
        original_min_segment_data_id: NULL_DATA_ID,
        xm_type: 0,
        db_type: 20,
        records_per_segment: records_per_segment(rows, options),
        segments: segs,
    };
    Ok(EncodedColumn {
        idf,
        idfmeta,
        dictionary: None,
        hidx: None,
        stats,
    })
}

/// In-memory `DictionaryStorage.Size` of a dictionary-less column (observed constant).
const VALUE_DICTIONARY_SIZE: u64 = 144;

fn check_common(rows: u64, options: &EncodeOptions) -> Result<(), Error> {
    if rows > options.max_rows {
        return Err(too_many(rows));
    }
    if options.segment_rows == 0 || options.segment_rows > u64::from(u32::MAX) {
        return Err(Error::LimitExceeded(format!(
            "segment size {} (must be 1..=2^32-1)",
            options.segment_rows
        )));
    }
    if options.rle_min_run == 0 {
        return Err(Error::LimitExceeded("RLE minimum run 0".into()));
    }
    Ok(())
}

fn too_many(rows: u64) -> Error {
    Error::LimitExceeded(format!("{rows} rows"))
}

fn records_per_segment(rows: u64, options: &EncodeOptions) -> u64 {
    if rows <= options.segment_rows {
        rows
    } else {
        options.segment_rows
    }
}

/// `DictionaryStorage.Size` of a hash-encoded long or real dictionary of `n` values: the size
/// of the engine's in-memory dictionary (docs/formats/vertipaq.md, "Dictionary size").
///
/// A model fitted to the sizes the engine reports (docs/formats/vertipaq.md): the sizes behave as
/// if the engine held a hash table of 64-byte bins at load, in the [MS-XLDM] §2.3.3.1.4 layout.
/// Entries of 4-byte longs (key and data id, 8 bytes) fit 6 to a bin; those of 8-byte longs and
/// reals (12 bytes) fit 4. The table starts at 16 bins and doubles while it holds more than 3/4
/// of its local entries. The size is a fixed part (272 bytes for longs, 232 for reals) + 64 per bin + the values (4 or 8
/// bytes each). This is what the engine reports for a file it loads, and what Desktop saves
/// then; right after a refresh, a real dictionary of up to 48 values still has the 32 bins
/// processing starts with (1 024 bytes more).
///
/// Exact for every 4-byte long dictionary seen. For 8-byte keys this is a lower bound: each
/// value past the 4 local entries of its bin adds 12 bytes, and the engine's hash of 64-bit
/// keys is not known, so those collisions are not counted (exact when there are none).
fn numeric_dictionary_size(four_bytes: bool, real: bool, n: u64) -> Option<u64> {
    let (locals, element): (u64, u64) = if four_bytes { (6, 4) } else { (4, 8) };
    let mut bins: u64 = 16;
    // grow while n > 0.75 × locals × bins
    while n.checked_mul(4)? > bins.checked_mul(3 * locals)? {
        bins = bins.checked_mul(2)?;
    }
    let fixed: u64 = if real { 232 } else { 272 };
    fixed
        .checked_add(bins.checked_mul(64)?)?
        .checked_add(n.checked_mul(element)?)
}

/// `DictionaryStorage.Size` of a string dictionary (docs/formats/vertipaq.md, "Dictionary size"):
/// 688 bytes + 64 per bin of the string hash table (at least 256 bins, doubled while it holds
/// more than 5.4 strings per bin) + 8 per string + 2 per stored UTF-16 unit of the plain pages
/// (terminators included) + the buffer of each Huffman page and 1 409 bytes per such page.
/// Collisions (strings past the 6 local entries of their bin, 8 bytes each) are not counted:
/// the engine's string hash is not known, so this is a lower bound, exact without them.
fn string_dictionary_size(d: &dictionary::Written) -> Option<u64> {
    let n = d.len;
    let mut bins: u64 = 256;
    while n.checked_mul(10)? > bins.checked_mul(54)? {
        bins = bins.checked_mul(2)?;
    }
    688u64
        .checked_add(bins.checked_mul(64)?)?
        .checked_add(n.checked_mul(8)?)?
        .checked_add(d.used_chars.checked_mul(2)?)?
        .checked_add(d.compressed_bytes)?
        .checked_add(d.compressed_pages.checked_mul(1409)?)
}

fn column_stats(
    a: &ids::Assigned,
    logical: LogicalType,
    dictionary: &Option<dictionary::Written>,
    has_hidx: bool,
    segments: Vec<SegmentStatsOut>,
    options: &EncodeOptions,
) -> ColumnStats {
    let rows = a.ids.len() as u64;
    let mut min = u32::MAX;
    let mut max = 0u32;
    let mut has_nulls = false;
    for &id in &a.ids {
        if id == NULL_DATA_ID {
            has_nulls = true;
        } else {
            min = min.min(id);
            max = max.max(id);
        }
    }
    let (min, max, id_range) = if min > max {
        (NULL_DATA_ID, NULL_DATA_ID, 0)
    } else {
        (min, max, u64::from(max - min) + 1)
    };
    let hash = a.kind == EncodingKind::Hash;
    let string = a.storage_type == StorageType::String;
    let dict_len = dictionary.as_ref().map_or(0, |d| d.len);
    let element_size = dictionary.as_ref().and_then(|d| d.element_size);
    let only_blanks = hash && rows > 0 && a.distinct == 0;
    let dictionary_size = match (a.kind, a.storage_type, dictionary) {
        (EncodingKind::Value, _, _) => Some(VALUE_DICTIONARY_SIZE),
        // no `.dictionary` (only blanks): observed constants
        (_, StorageType::Long, None) if only_blanks => Some(1296),
        (_, StorageType::Real, None) if only_blanks => Some(2280),
        (_, StorageType::String, None) if only_blanks => Some(16904),
        (_, StorageType::Long, Some(_)) => numeric_dictionary_size(element_size == Some(4), false, dict_len),
        (_, StorageType::Real, Some(_)) => numeric_dictionary_size(false, true, dict_len),
        (_, StorageType::String, Some(d)) => string_dictionary_size(d),
        _ => None,
    };
    // Value encoding: with a `.hidx`, the distinct ids blank included (= the `.hidx` records);
    // without, the id range.
    let distinct_states = match (a.kind, has_hidx) {
        (EncodingKind::Value, false) => id_range,
        (EncodingKind::Value, true) => a.distinct.saturating_add(u64::from(has_nulls)),
        // only blanks: the blank counts (observed)
        _ if only_blanks => 1,
        _ => a.distinct,
    };
    ColumnStats {
        kind: a.kind,
        storage_type: a.storage_type,
        base_id: if hash { 2 } else { a.base_id },
        magnitude: if hash { 0.0 } else { a.magnitude },
        last_id: if hash {
            NULL_DATA_ID.saturating_add(u32::try_from(dict_len).unwrap_or(u32::MAX))
        } else {
            NULL_DATA_ID
        },
        is_nullable: !hash || has_nulls,
        is_unique: hash && element_size == Some(4) && !has_nulls && rows > 0 && a.distinct == rows,
        // a long column of blanks (no dictionary): 1 (observed)
        is_operating_on_32: element_size == Some(4) || (only_blanks && a.storage_type == StorageType::Long),
        dictionary_flags: if string { 3 } else { 0 },
        dictionary_size,
        dictionary_len: dict_len,
        longest_string: dictionary.as_ref().map_or(0, |d| d.longest),
        rows,
        distinct_values: a.distinct,
        id_range,
        distinct_states,
        min_data_id: min,
        max_data_id: max,
        has_nulls,
        original_min_segment_data_id: if only_blanks {
            segments::ONLY_BLANKS_ORIGINAL_MIN
        } else {
            NULL_DATA_ID
        },
        xm_type: match a.storage_type {
            StorageType::Real => 1,
            StorageType::String => 2,
            _ => 0,
        },
        db_type: match logical {
            LogicalType::String => 130,
            LogicalType::Double => 5,
            LogicalType::DateTime => 7,
            LogicalType::Decimal => 6,
            LogicalType::Boolean => 11,
            LogicalType::Int64 | LogicalType::Binary => 20,
        },
        records_per_segment: records_per_segment(rows, options),
        segments,
    }
}
