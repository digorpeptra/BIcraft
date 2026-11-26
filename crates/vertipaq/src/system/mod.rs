//! Engine system tables that VertiPaq keeps next to user data ([MS-XLDM] §2.4):
//!
//! - `H$<table>$<column>`: the attribute hierarchy of a column — `POS_TO_ID` (data ids in
//!   sort order) and, for hash-encoded columns, `ID_TO_POS` (sort position of each data id).
//!   Built by [`build_attribute_hierarchy`].
//! - `R$<table>$<relationship>`: the relationship index of the "from" table — for each data id
//!   of the from column, the row of the "to" table holding the same key. Built by
//!   [`build_relationship_index`].
//! - `U$<table>$<hierarchy>`: the node table of a user hierarchy — `MULTI_LEVEL_ID`,
//!   `PARENT_POS`, `FIRST_CHILD_POS`, `CHILD_COUNT`. Built by [`build_user_hierarchy`].
//!
//! Each builder returns a [`SystemTableData`]: the stored column vectors plus the segment
//! layout. [`SystemTableData::encode`] turns a column into its `.idf` / `.idfmeta` bytes
//! (system tables use plain `XMRENoSplit` bit packing, never hybrid RLE). Layouts and the
//! metadata rows that reference these tables are described in
//! `docs/formats/vertipaq-system-tables.md`.

mod attribute;
pub mod collation;
mod relationship;
mod tailoring;
mod user_hierarchy;

pub use attribute::{
    AttributeHierarchy, AttributeHierarchyOptions, build_attribute_hierarchy, build_attribute_hierarchy_with,
    compare_values,
};
pub use collation::{Collation, EquivalenceKey, SortKey, Tailoring, TextEquivalence, TextStorage};
pub use relationship::{RelationshipIndex, build_relationship_index};
pub use user_hierarchy::{UserHierarchy, UserHierarchyLevel, build_user_hierarchy};

use crate::bitpack;
use crate::compression::{NO_SPLIT_BASE, NONE};
use crate::idfmeta::{ColumnMeta, DataObject, SegmentDataObject, SegmentMeta, SegmentStats};

/// Cap on the rows of one system table (same default as [`crate::Limits::max_rows`]).
pub const MAX_SYSTEM_ROWS: usize = 1 << 26;

/// Data id of the blank value.
pub const BLANK_DATA_ID: u32 = crate::idf::NULL_DATA_ID;

/// Errors of the system-table builders.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SystemTableError {
    #[error("data id {0} appears twice")]
    DuplicateDataId(u32),
    #[error("data id {0} is reserved (ids 0 and 1 are never column values)")]
    ReservedDataId(u32),
    #[error("data id 2 is the blank id but its value is not blank")]
    BlankIdNotBlank,
    #[error("too many rows for a system table ({0})")]
    TooManyRows(usize),
    #[error("the levels of a user hierarchy have different row counts")]
    LevelLengthMismatch,
    #[error("a user hierarchy needs at least one level")]
    NoLevels,
    #[error("data id {0} is not a member of the level's attribute hierarchy")]
    UnknownMember(u32),
    #[error("value {0} does not fit the stored column")]
    ValueOverflow(i64),
    #[error("data id {0}: blank values must use data id 2 and only blank values may")]
    BlankMismatch(u32),
    #[error("no system column at index {0}")]
    NoSuchColumn(usize),
}

/// Which kind of system table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemTableKind {
    /// `H$`: attribute hierarchy.
    AttributeHierarchy,
    /// `R$`: relationship index.
    RelationshipIndex,
    /// `U$`: user hierarchy.
    UserHierarchy,
}

impl SystemTableKind {
    /// Table-name prefix (`H$`, `R$`, `U$`).
    pub fn prefix(self) -> &'static str {
        match self {
            SystemTableKind::AttributeHierarchy => "H$",
            SystemTableKind::RelationshipIndex => "R$",
            SystemTableKind::UserHierarchy => "U$",
        }
    }

    /// Observed `TableStorage.Settings` of the system table.
    pub fn table_storage_settings(self) -> i64 {
        match self {
            SystemTableKind::AttributeHierarchy => 4,
            SystemTableKind::RelationshipIndex => 2,
            SystemTableKind::UserHierarchy => 5,
        }
    }

    /// Observed `TableStorage.Version`.
    pub fn table_storage_version(self) -> i64 {
        match self {
            SystemTableKind::AttributeHierarchy => 1,
            _ => 0,
        }
    }

    /// Observed `SegmentMapStorage.Type` of the system partition.
    pub fn segment_map_type(self) -> i64 {
        match self {
            SystemTableKind::AttributeHierarchy => 2,
            _ => 3,
        }
    }
}

/// One stored column of a system table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemColumn {
    /// Column name (`POS_TO_ID`, `ID_TO_POS`, `INDEX`, `MULTI_LEVEL_ID`, …).
    pub name: &'static str,
    /// Stored values, one per record. The logical value is `stored + base`.
    pub values: Vec<u32>,
    /// `−1` for columns whose logical value can be `−1` (stored with a +1 bias), else 0.
    pub base: i64,
    /// Bit width of the `XMRENoSplit` packing.
    pub width: u8,
    /// Observed `ColumnStorage.Settings` of this column.
    pub storage_settings: i64,
    /// Observed `ColumnStorage.BinaryCharacters` (10240 on user-hierarchy columns, else 0).
    pub binary_characters: i64,
}

impl SystemColumn {
    fn new(name: &'static str, values: Vec<u32>, base: i64, width: u8, settings: i64) -> Self {
        SystemColumn {
            name,
            values,
            base,
            width,
            storage_settings: settings,
            binary_characters: 0,
        }
    }

    /// Logical values (`stored + base`).
    pub fn logical_values(&self) -> Vec<i64> {
        self.values.iter().map(|&v| i64::from(v) + self.base).collect()
    }

    /// `first_run_value` written in every segment of the `.idfmeta` (the base as `u32`).
    pub fn first_run_value(&self) -> u32 {
        if self.base < 0 { u32::MAX } else { 0 }
    }
}

/// A system table ready to be encoded: its columns and segment layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemTableData {
    pub kind: SystemTableKind,
    pub columns: Vec<SystemColumn>,
    /// `SegmentMapStorage.RecordsPerSegment` (≥ 1).
    pub records_per_segment: u64,
}

/// The encoded files of one system column.
#[derive(Debug, Clone, PartialEq)]
pub struct EncodedSystemColumn {
    /// `.idf` bytes.
    pub idf: Vec<u8>,
    /// `.idfmeta` bytes.
    pub idfmeta: Vec<u8>,
    /// The `.idfmeta` content (segment count, for `SegmentStorage.SegmentCount`).
    pub meta: ColumnMeta,
}

impl SystemTableData {
    /// `SegmentMapStorage.RecordCount`: records of the (single) partition.
    pub fn record_count(&self) -> u64 {
        self.columns.first().map_or(0, |c| c.values.len() as u64)
    }

    /// `SegmentMapStorage.SegmentCount`.
    pub fn segment_count(&self) -> u64 {
        self.record_count()
            .div_ceil(self.records_per_segment.max(1))
            .max(1)
    }

    /// The column called `name`.
    pub fn column(&self, name: &str) -> Option<&SystemColumn> {
        self.columns.iter().find(|c| c.name == name)
    }

    /// Encodes column `index` as `.idf` + `.idfmeta` (one `XMRENoSplit` block per segment).
    pub fn encode(&self, index: usize) -> Result<EncodedSystemColumn, SystemTableError> {
        let column = self
            .columns
            .get(index)
            .ok_or(SystemTableError::NoSuchColumn(index))?;
        encode_column(column, self.records_per_segment)
    }
}

/// Encodes one system column split into segments of `records_per_segment` values.
pub fn encode_column(
    column: &SystemColumn,
    records_per_segment: u64,
) -> Result<EncodedSystemColumn, SystemTableError> {
    let per_segment =
        usize::try_from(records_per_segment.max(1)).map_err(|_| SystemTableError::TooManyRows(usize::MAX))?;
    let per_word = bitpack::values_per_word(column.width);
    if per_word == 0 {
        return Err(SystemTableError::ValueOverflow(i64::from(column.width)));
    }
    let mut idf = Vec::new();
    let mut segments = Vec::new();
    let mut objects = Vec::new();
    let chunks: Vec<&[u32]> = if column.values.is_empty() {
        vec![&[]]
    } else {
        column.values.chunks(per_segment).collect()
    };
    for chunk in chunks {
        let mut block = bitpack::pack(chunk, column.width).ok_or_else(|| {
            let max = chunk.iter().copied().max().unwrap_or(0);
            SystemTableError::ValueOverflow(i64::from(max))
        })?;
        // The engine allocates room for one more value than it stores.
        let words = (chunk.len() + 1).div_ceil(per_word);
        block.resize(words * 8, 0);
        let offset = idf.len() as u64;
        idf.extend_from_slice(&(words as u64).to_le_bytes());
        idf.extend_from_slice(&block);
        objects.push(SegmentDataObject {
            primary: DataObject {
                offset,
                words: words as u64,
            },
            sub: None,
        });
        segments.push(SegmentMeta {
            records: chunk.len() as u64,
            base_id: 0,
            compression_class: NO_SPLIT_BASE + u32::from(column.width),
            sub_compression_class: NONE,
            rle: None,
            first_run_value: column.first_run_value(),
            stats: SegmentStats {
                distinct_states: 0,
                min_data_id: BLANK_DATA_ID,
                max_data_id: BLANK_DATA_ID,
                original_min_segment_data_id: BLANK_DATA_ID,
                rle_sort_order: -1,
                row_count: 0,
                has_nulls: 0,
                rle_runs: 0,
                others_rle_runs: 0,
            },
            sub_segment: None,
        });
    }
    let meta = ColumnMeta {
        segments,
        data_objects: Some(objects),
    };
    Ok(EncodedSystemColumn {
        idf,
        idfmeta: meta.to_bytes(),
        meta,
    })
}

/// Stores logical values `v ≥ −1` as `v + 1` (`INDEX`, `PARENT_POS`, … columns).
fn biased(values: impl IntoIterator<Item = i64>) -> Result<Vec<u32>, SystemTableError> {
    values
        .into_iter()
        .map(|v| {
            v.checked_add(1)
                .and_then(|s| u32::try_from(s).ok())
                .ok_or(SystemTableError::ValueOverflow(v))
        })
        .collect()
}

fn check_rows(n: usize) -> Result<(), SystemTableError> {
    if n > MAX_SYSTEM_ROWS {
        Err(SystemTableError::TooManyRows(n))
    } else {
        Ok(())
    }
}
