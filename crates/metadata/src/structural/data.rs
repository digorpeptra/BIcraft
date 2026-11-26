//! Inputs and outputs of the structural writer: encoded VertiPaq content for new objects (filled
//! by an adapter from the encoder and the system-table builders) and the list of ABF files to
//! add and remove.
//!
//! Field names follow the `metadata.sqlitedb` columns they end up in (see
//! `docs/formats/metadata-sqlitedb.md`, "Structural writing").

/// A file the structural writer wants stored in the ABF backup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileToWrite {
    /// Logical path, `/`-separated, relative to the database folder (what
    /// `bicraft_abf::AbfWriter::add` takes), e.g. `Sales (12).tbl/13.prt/1.Sales (12).Qty (14).0.idf`.
    pub path: String,
    pub bytes: Vec<u8>,
}

/// The container side of a structural edit: ABF files to remove (logical paths, `/`-separated)
/// and files to add. Every path comes from a `StorageFolder.Path` + `StorageFile.FileName` pair
/// of the base (removals) or of the new rows (additions).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoragePlan {
    pub remove: Vec<String>,
    pub add: Vec<FileToWrite>,
}

impl StoragePlan {
    pub fn is_empty(&self) -> bool {
        self.remove.is_empty() && self.add.is_empty()
    }
}

/// Encoded content for the objects a structural edit adds. Objects are matched by name (ASCII
/// case-insensitive). An added object without an entry here is refused with
/// [`crate::UnsupportedChange`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EncodedData {
    /// New tables (one partition each).
    pub tables: Vec<EncodedTable>,
    /// New columns of tables that already exist.
    pub columns: Vec<EncodedNewColumn>,
    /// New relationships: their `R$` index.
    pub relationships: Vec<EncodedRelationship>,
    /// New user hierarchies (of new or existing tables): their `U$` node table.
    pub hierarchies: Vec<EncodedHierarchy>,
}

/// The `U$` node table of a new user hierarchy (`bicraft_vertipaq::build_user_hierarchy`).
#[derive(Debug, Clone, PartialEq)]
pub struct EncodedHierarchy {
    pub table: String,
    pub name: String,
    pub nodes: SystemTableData,
    /// First node row of each level, top level first (`HierarchyStorage.LevelDefinition`).
    pub level_starts: Vec<u32>,
}

/// The single partition of a new table.
#[derive(Debug, Clone, PartialEq)]
pub struct EncodedTable {
    pub name: String,
    /// `SegmentMapStorage` of the partition ([`SegmentMap::partition`]).
    pub segment_map: SegmentMap,
    /// The hidden row-number column every table has ([MS-XLDM] §2.3.4).
    pub row_number: ColumnStorageData,
    /// The other columns, by name.
    pub columns: Vec<(String, ColumnStorageData)>,
    /// Empty for a table of one partition. For a table of several partitions, one entry per
    /// model partition in order: its segment map and the segments of every column there.
    /// `segment_map` and the columns' `ColumnStorageData` then describe the whole table
    /// (statistics summed, one dictionary, `.hidx` and `H$` per column) and their own
    /// `idf`/`idfmeta` are not written.
    pub partitions: Vec<EncodedPartition>,
}

/// One partition of a new table of several partitions.
#[derive(Debug, Clone, PartialEq)]
pub struct EncodedPartition {
    pub segment_map: SegmentMap,
    /// The row-number column's segments (ids continue across partitions).
    pub row_number: PartitionSegments,
    /// The other columns' segments, by name.
    pub columns: Vec<(String, PartitionSegments)>,
}

/// A new column of an existing table.
#[derive(Debug, Clone, PartialEq)]
pub struct EncodedNewColumn {
    pub table: String,
    pub column: String,
    /// The column: dictionary, statistics (over every partition) and hierarchy, plus the
    /// segments of a single-partition table.
    pub data: ColumnStorageData,
    /// Empty for a single-partition table. For a table of several partitions (incremental
    /// refresh policy ranges, hybrid tables): the segments of each partition, in storage
    /// order, all sharing `data`'s dictionary; `data.idf`/`idfmeta` are then not written.
    pub partitions: Vec<PartitionSegments>,
}

/// The segments of one column in one partition (`ColumnPartitionStorage` + `SegmentStorage`).
#[derive(Debug, Clone, PartialEq)]
pub struct PartitionSegments {
    pub idf: Vec<u8>,
    pub idfmeta: Vec<u8>,
    /// `SegmentStorage.SegmentCount`.
    pub segment_count: i64,
    /// Rows of the partition (must match its `SegmentMapStorage.RecordCount`).
    pub rows: i64,
}

/// The `R$` system table (column `INDEX`) of a new relationship. Many-to-many relationships
/// store no index (engine-observed): they need no entry.
#[derive(Debug, Clone, PartialEq)]
pub struct EncodedRelationship {
    pub name: String,
    /// `RelationshipIndexStorage.RecordCount` comes from `index.segment_map.record_count`.
    pub index: SystemTableData,
    /// Some from value (blank included) has no match on the "one" side (referential-integrity
    /// violation): `RelationshipIndexStorage.Flags` 1, which gives the "one" table its virtual
    /// blank row.
    pub unmatched: bool,
    /// One-to-one relationships: the second index, to-column data id → from-table row
    /// (`RelationshipStorage2ID`, its `R$` named after the to table), and its unmatched flag.
    pub reverse: Option<(SystemTableData, bool)>,
}

impl EncodedRelationship {
    /// A relationship with one index (many-to-one).
    pub fn new(name: impl Into<String>, index: SystemTableData, unmatched: bool) -> Self {
        Self {
            name: name.into(),
            index,
            unmatched,
            reverse: None,
        }
    }
}

/// `SegmentMapStorage` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SegmentMap {
    /// `Type` (observed): on user partitions 3 with one segment and 1 with several (Desktop's
    /// consistency check refuses a partition of several segments typed 3, G50); 3 on `R$` and
    /// `U$` tables, 2 on `H$` tables.
    pub kind: i64,
    pub record_count: i64,
    pub segment_count: i64,
    pub records_per_segment: i64,
}

impl SegmentMap {
    /// One segment of `rows` records, the shape Desktop writes for user partitions.
    pub fn single(rows: i64) -> Self {
        Self {
            kind: 3,
            record_count: rows,
            segment_count: 1,
            records_per_segment: rows,
        }
    }

    /// A user partition of `record_count` rows in `segment_count` segments: `Type` 3 for one
    /// segment, 1 for several (the values Desktop writes, docs/formats/vertipaq.md).
    pub fn partition(record_count: i64, segment_count: i64, records_per_segment: i64) -> Self {
        Self {
            kind: if segment_count > 1 { 1 } else { 3 },
            record_count,
            segment_count,
            records_per_segment,
        }
    }
}

/// One column of one partition, encoded: the files and the values of its storage rows.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnStorageData {
    /// Segment data (`.idf`).
    pub idf: Vec<u8>,
    /// Segment statistics (`.idfmeta`).
    pub idfmeta: Vec<u8>,
    /// `SegmentStorage.SegmentCount`: segments inside the `.idf`.
    pub segment_count: i64,
    pub dictionary: DictionaryData,
    pub statistics: ColumnStatistics,
    /// The attribute hierarchy. `None` writes the shape of a column without one
    /// (`IsAvailableInMDX` false: `MaterializationType` −1, no `H$` table).
    pub hierarchy: Option<AttributeHierarchyData>,
}

/// `DictionaryStorage` values, plus the `.dictionary` file of a hash-encoded column.
#[derive(Debug, Clone, PartialEq)]
pub struct DictionaryData {
    /// The `.dictionary` file. `None` for value-encoded columns (no file, `StorageFileID` 0).
    pub file: Option<Vec<u8>>,
    /// `Type`: 1 hash, 2 value (observed).
    pub kind: i64,
    /// `DataType`: the storage type (observed 2 string, 6 integer, 8 date/time).
    pub data_type: i64,
    pub base_id: i64,
    pub magnitude: f64,
    pub last_id: i64,
    pub is_nullable: bool,
    pub is_unique: bool,
    pub is_operating_on_32: bool,
    /// `DictionaryFlags` (observed 3 on string dictionaries, 0 otherwise).
    pub flags: i64,
    /// `Size` (observed 144 on value dictionaries; larger than the file on hash ones).
    pub size: i64,
}

/// The `Statistics_*` columns of `ColumnStorage` that describe the data. `Statistics_Usage` and
/// `Statistics_EncodingHint` are derived from the model (relationships), not taken from here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnStatistics {
    pub distinct_states: i64,
    pub min_data_id: i64,
    pub max_data_id: i64,
    pub original_min_segment_data_id: i64,
    pub rle_sort_order: i64,
    pub row_count: i64,
    pub has_nulls: bool,
    pub rle_runs: i64,
    pub others_rle_runs: i64,
    /// OLE DB type (observed 20 int64, 130 string, 6 currency, 7 date, 11 boolean).
    pub db_type: i64,
    /// Observed 0 numeric, 1 date, 2 string.
    pub xm_type: i64,
    pub compression_type: i64,
    pub compression_param: i64,
}

/// `AttributeHierarchyStorage` values and the files behind them.
#[derive(Debug, Clone, PartialEq)]
pub struct AttributeHierarchyData {
    /// Observed 0 (hash column: `H$` with `POS_TO_ID` and `ID_TO_POS`), 1 (value column: `H$`
    /// with `POS_TO_ID` only, plus a `.hidx`), 3 (row number: no system table).
    pub materialization_type: i64,
    pub distinct_data_count: i64,
    pub min_value: Option<String>,
    pub max_value: Option<String>,
    pub string_value_max_length: i64,
    /// The `.hidx` hash index (value-encoded columns).
    pub hidx: Option<Vec<u8>>,
    /// The `H$` system table. `ColumnPositionToData` / `ColumnDataToPosition` are the storage
    /// positions of its `POS_TO_ID` / `ID_TO_POS` columns (−1 when absent).
    pub system_table: Option<SystemTableData>,
}

/// An engine system table (`H$`, `R$`): one partition, a few columns.
#[derive(Debug, Clone, PartialEq)]
pub struct SystemTableData {
    pub segment_map: SegmentMap,
    pub columns: Vec<SystemColumnData>,
}

/// A column of a system table (`POS_TO_ID`, `ID_TO_POS`, `INDEX`, …).
#[derive(Debug, Clone, PartialEq)]
pub struct SystemColumnData {
    pub name: String,
    pub idf: Vec<u8>,
    pub idfmeta: Vec<u8>,
    /// `SegmentStorage.SegmentCount`.
    pub segment_count: i64,
    /// `ColumnStorage.Settings`; `None` takes the value observed for the column name
    /// (`POS_TO_ID` 7, `ID_TO_POS` 5, `INDEX` 3).
    pub settings: Option<i64>,
}
