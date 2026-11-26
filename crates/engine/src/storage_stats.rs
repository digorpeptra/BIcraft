//! VertiPaq storage statistics (M9.3): `model.storage_stats`, the numbers VertiPaq Analyzer
//! shows (rows, cardinality, encoding, dictionary / data / hierarchy sizes, relationship and
//! user-hierarchy structures, segments, partitions), read from the stored backup of the open
//! file (a PBIX `DataModel` or a PBIP `.pbi/cache.abf`). Read-only.
//!
//! Only `metadata.sqlitedb`, the file list and the `.idfmeta` segment maps are read
//! (`bicraft_pbix::storage`); the column data itself is never decoded. Backups too large to
//! decompress in memory are streamed. How each figure is derived, and how it compares with the
//! engine's storage DMVs, is in `docs/formats/vertipaq-stats.md`.

use std::collections::HashMap;

use bicraft_sqlite::Value as Sql;
use bicraft_vertipaq::{Compression, InnerCompression, parse_idfmeta};
use serde_json::{Value, json};

use crate::params::{allow, opt_bool, opt_str};
use crate::{Session, Source};

type SqlRow = std::collections::BTreeMap<String, Sql>;

/// One stored segment of a column.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Segment {
    /// The partition (its name) and its position in the table.
    pub partition: String,
    pub partition_number: i64,
    pub number: usize,
    pub records: u64,
    /// In-memory size of the segment's data (the engine's `USED_SIZE`).
    pub used_size: u64,
    /// `COMPRESSION_TYPE`: `NOSPLIT` (bit-packed values, alone or in a hybrid RLE segment) or
    /// `C123` (consecutive ids, nothing stored).
    pub compression: &'static str,
    /// Bits per packed value (`BITS_COUNT`); 0 when nothing is packed.
    pub bits: u32,
    /// `BOOKMARK_BITS_COUNT`: the engine reports the segment's RLE run count there.
    pub bookmark_bits: u64,
    /// The engine's row-number column is never compressed (`VERTIPAQ_STATE` `SKIPPED`).
    pub skipped: bool,
}

/// A column of a user or system table.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct ColumnStats {
    pub id: i64,
    pub name: String,
    /// `ColumnStorage.Name`, e.g. `Amount (123)` (the DMV `COLUMN_ID`).
    pub storage_name: String,
    pub is_row_number: bool,
    /// TOM data type name (`String`, `Int64`, ...), when known.
    pub data_type: Option<&'static str>,
    /// `HASH`, `VALUE` or `NONE`.
    pub encoding: &'static str,
    pub cardinality: u64,
    pub dictionary_size: u64,
    pub segments: Vec<Segment>,
    /// The `H$` system table's segments (all of its columns).
    pub hierarchy: Vec<(String, Segment)>,
    /// `H$` table storage name (the DMV `TABLE_ID`).
    pub hierarchy_table: Option<String>,
}

impl ColumnStats {
    pub fn data_size(&self) -> u64 {
        self.segments
            .iter()
            .map(|s| s.used_size)
            .fold(0, u64::saturating_add)
    }
    pub fn hierarchy_size(&self) -> u64 {
        self.hierarchy
            .iter()
            .map(|(_, s)| s.used_size)
            .fold(0, u64::saturating_add)
    }
    pub fn total_size(&self) -> u64 {
        self.data_size()
            .saturating_add(self.dictionary_size)
            .saturating_add(self.hierarchy_size())
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct PartitionStats {
    pub name: String,
    pub number: i64,
    pub rows: u64,
    pub segments: u64,
    pub records_per_segment: u64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct UserHierarchyStats {
    pub name: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct TableStats {
    pub id: i64,
    pub name: String,
    pub storage_name: String,
    pub rows: u64,
    pub ri_violations: u64,
    pub partitions: Vec<PartitionStats>,
    pub columns: Vec<ColumnStats>,
    pub user_hierarchies: Vec<UserHierarchyStats>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct RelationshipStats {
    pub name: String,
    pub from_table: String,
    pub from_column: String,
    pub to_table: String,
    pub to_column: String,
    /// `R$` structure kept with the from table (and, one-to-one, with the to table).
    pub size_from: u64,
    pub size_to: u64,
    /// Keys of the many side with no row on the one side (`RelationshipIndexStorage.Flags`).
    pub missing_keys: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Storage {
    pub source: &'static str,
    pub streamed: bool,
    pub tables: Vec<TableStats>,
    pub relationships: Vec<RelationshipStats>,
    /// Partition states and refresh times (`refresh_info.rs`).
    pub refresh: crate::refresh_info::RefreshInfo,
}

impl TableStats {
    pub fn sum(&self, f: impl Fn(&ColumnStats) -> u64) -> u64 {
        self.columns.iter().map(f).fold(0, u64::saturating_add)
    }
    pub fn user_hierarchies_size(&self) -> u64 {
        self.user_hierarchies
            .iter()
            .map(|h| h.size)
            .fold(0, u64::saturating_add)
    }
}

impl Storage {
    /// `R$` sizes of the relationships kept with `table` (from and to ends).
    pub fn relationships_size(&self, table: &str) -> u64 {
        self.relationships
            .iter()
            .map(|r| {
                let mut n = 0u64;
                if r.from_table.eq_ignore_ascii_case(table) {
                    n = n.saturating_add(r.size_from);
                }
                if r.to_table.eq_ignore_ascii_case(table) {
                    n = n.saturating_add(r.size_to);
                }
                n
            })
            .fold(0, u64::saturating_add)
    }

    pub fn table_size(&self, t: &TableStats) -> u64 {
        t.sum(ColumnStats::total_size)
            .saturating_add(t.user_hierarchies_size())
            .saturating_add(self.relationships_size(&t.name))
    }
}

/// The open file's stored backup, its statistics read.
pub(crate) fn load(s: &Session) -> Result<Storage, String> {
    let want = |f: &bicraft_abf::FileEntry| f.path.to_ascii_lowercase().ends_with(".idfmeta");
    let limits = bicraft_pbix::DataModelLimits::default();
    let (source, files) = match &s.source {
        Some(Source::Package { package, .. }) => {
            let part = package.get(bicraft_pbix::DATA_MODEL).ok_or(
                "the file has no stored data (a PBIT, or a PBIX without DataModel): storage \
                 statistics need a PBIX or a PBIP project with .pbi/cache.abf",
            )?;
            (
                "DataModel",
                bicraft_pbix::storage::read_part(part, &want, &limits).map_err(|e| e.to_string())?,
            )
        }
        Some(Source::Project { .. }) => {
            let path = crate::cache_abf::path(s)?.ok_or(
                "the project has no .pbi/cache.abf (open it in Power BI Desktop and save, or \
                 save it from a PBIX with file.save_as, or pass cache_abf): no stored data",
            )?;
            (
                crate::cache_abf::label(s),
                bicraft_pbix::storage::read_file(&path, &want, &limits).map_err(|e| e.to_string())?,
            )
        }
        Some(Source::Bim { .. }) => return Err("a model.bim holds no data: no storage statistics".into()),
        None => return Err("no file is open (run file.open first)".into()),
    };
    let mut st = from_files(&files)?;
    st.source = source;
    st.refresh.source = source;
    Ok(st)
}

fn int(row: &SqlRow, col: &str) -> Option<i64> {
    match row.get(col)? {
        Sql::Integer(n) => Some(*n),
        Sql::Real(x) if x.fract() == 0.0 && x.abs() < 9.0e15 => Some(*x as i64),
        _ => None,
    }
}

fn uint(row: &SqlRow, col: &str) -> u64 {
    int(row, col).and_then(|n| u64::try_from(n).ok()).unwrap_or(0)
}

fn text(row: &SqlRow, col: &str) -> String {
    match row.get(col) {
        Some(Sql::Text(s)) => s.clone(),
        _ => String::new(),
    }
}

fn by_id(rows: &[SqlRow]) -> HashMap<i64, &SqlRow> {
    rows.iter().filter_map(|r| Some((int(r, "ID")?, r))).collect()
}

/// TOM data type name of a `Column.ExplicitDataType` / `InferredDataType` code.
fn data_type_name(code: i64) -> Option<&'static str> {
    Some(match code {
        2 => "String",
        6 => "Int64",
        8 => "Double",
        9 => "DateTime",
        10 => "Decimal",
        11 => "Boolean",
        17 => "Binary",
        _ => return None,
    })
}

/// In-memory size of one segment, from its `.idfmeta` entry and data blocks.
fn segment_size(meta: &bicraft_vertipaq::SegmentMeta, blocks: Option<(u64, Option<u64>)>) -> u64 {
    match blocks {
        Some((primary, sub)) => primary.saturating_add(sub.unwrap_or(0)).saturating_mul(8),
        None => 0,
    }
    .max(match meta.rle {
        Some(r) => r.storage_used_size.saturating_mul(4),
        None => 0,
    })
}

/// The statistics of a backup's files (`metadata.sqlitedb` plus the `.idfmeta` files).
pub(crate) fn from_files(files: &bicraft_pbix::storage::StorageFiles) -> Result<Storage, String> {
    let db = bicraft_sqlite::Database::from_bytes(&files.sqlitedb).map_err(|e| e.to_string())?;
    let load = |name: &str| -> Result<Vec<SqlRow>, String> {
        if db.table(name).is_none() {
            return Ok(Vec::new());
        }
        db.query_all(name).map_err(|e| format!("{name}: {e}"))
    };
    let tables = load("Table")?;
    let table_storage = load("TableStorage")?;
    let columns = load("Column")?;
    let column_storage = load("ColumnStorage")?;
    let dictionary_storage = load("DictionaryStorage")?;
    let column_partitions = load("ColumnPartitionStorage")?;
    let segment_storage = load("SegmentStorage")?;
    let partition_storage = load("PartitionStorage")?;
    let segment_maps = load("SegmentMapStorage")?;
    let partitions = load("Partition")?;
    let storage_files = load("StorageFile")?;
    let folders = load("StorageFolder")?;
    let attribute_hierarchies = load("AttributeHierarchy")?;
    let attribute_storage = load("AttributeHierarchyStorage")?;
    let hierarchies = load("Hierarchy")?;
    let hierarchy_storage = load("HierarchyStorage")?;
    let relationships = load("Relationship")?;
    let relationship_storage = load("RelationshipStorage")?;
    let relationship_index = load("RelationshipIndexStorage")?;

    let tables_by_id = by_id(&tables);
    let ts_by_id = by_id(&table_storage);
    let cs_by_id = by_id(&column_storage);
    let ds_by_id = by_id(&dictionary_storage);
    let ss_by_id = by_id(&segment_storage);
    let ps_by_id = by_id(&partition_storage);
    let sm_by_id = by_id(&segment_maps);
    let partitions_by_id = by_id(&partitions);
    let files_by_id = by_id(&storage_files);
    let folders_by_id = by_id(&folders);
    let columns_by_id = by_id(&columns);
    let ahs_by_id = by_id(&attribute_storage);
    let hs_by_id = by_id(&hierarchy_storage);
    let rs_by_id = by_id(&relationship_storage);
    let ri_by_id = by_id(&relationship_index);

    // logical path (lower case, `/`) → size
    let sizes: HashMap<String, u64> = files
        .files
        .iter()
        .map(|f| (f.path.to_ascii_lowercase(), f.size as u64))
        .collect();
    let contents: HashMap<String, &Vec<u8>> = files
        .contents
        .iter()
        .map(|(k, v)| (k.to_ascii_lowercase(), v))
        .collect();
    let file_path = |id: i64| -> Option<String> {
        let f = files_by_id.get(&id)?;
        let name = text(f, "FileName");
        let folder = int(f, "StorageFolderID")
            .and_then(|fid| folders_by_id.get(&fid))
            .map(|r| text(r, "Path"))
            .unwrap_or_default();
        let path = if folder.is_empty() {
            name
        } else {
            format!("{folder}\\{name}")
        };
        Some(path.replace('\\', "/").to_ascii_lowercase())
    };

    // partition storage id → (partition name, position)
    let partition_of = |ps: i64| -> (String, i64) {
        let row = ps_by_id.get(&ps);
        let number = row.and_then(|r| int(r, "StoragePosition")).unwrap_or(0);
        let name = row
            .and_then(|r| int(r, "PartitionID"))
            .and_then(|p| partitions_by_id.get(&p))
            .map(|p| text(p, "Name"))
            .filter(|n| !n.is_empty())
            .or_else(|| row.map(|r| text(r, "Name")))
            .unwrap_or_default();
        (name, number)
    };

    // column storage id → segments
    let mut segments_of: HashMap<i64, Vec<Segment>> = HashMap::new();
    let mut ordered: Vec<&SqlRow> = column_partitions.iter().collect();
    ordered.sort_by_key(|r| {
        let ps = int(r, "PartitionStorageID").unwrap_or(0);
        (
            ps_by_id
                .get(&ps)
                .and_then(|p| int(p, "StoragePosition"))
                .unwrap_or(0),
            ps,
        )
    });
    for cps in ordered {
        let (Some(cs), Some(seg)) = (int(cps, "ColumnStorageID"), int(cps, "SegmentStorageID")) else {
            continue;
        };
        let (partition, partition_number) = partition_of(int(cps, "PartitionStorageID").unwrap_or(0));
        let meta_path = ss_by_id
            .get(&seg)
            .and_then(|s| int(s, "StorageFileID"))
            .and_then(file_path);
        let idf_size = int(cps, "StorageFileID")
            .and_then(file_path)
            .and_then(|p| sizes.get(&p).copied())
            .unwrap_or(0);
        let Some(meta) = meta_path
            .and_then(|p| contents.get(&p).copied())
            .and_then(|b| parse_idfmeta(b).ok())
        else {
            continue;
        };
        let total_records = meta
            .segments
            .iter()
            .map(|s| s.records)
            .fold(0u64, u64::saturating_add);
        let list = segments_of.entry(cs).or_default();
        for (i, sm) in meta.segments.iter().enumerate() {
            let blocks = meta
                .data_objects
                .as_ref()
                .and_then(|d| d.get(i))
                .map(|d| (d.primary.words, d.sub.map(|x| x.words)));
            // Without data objects, the .idf is shared out by rows.
            let blocks = blocks.or_else(|| {
                let share = if total_records == 0 {
                    0
                } else {
                    (u128::from(idf_size) * u128::from(sm.records) / u128::from(total_records)) as u64
                };
                Some((share / 8, None))
            });
            let (compression, bits) = match sm.compression() {
                Some(Compression::NoSplit(n) | Compression::HybridRle(InnerCompression::NoSplit(n))) => {
                    ("NOSPLIT", u32::from(n))
                }
                Some(Compression::HybridRle(InnerCompression::Xm123) | Compression::Xm123) => ("C123", 0),
                None => ("UNKNOWN", 0),
            };
            list.push(Segment {
                partition: partition.clone(),
                partition_number,
                number: i,
                records: sm.records,
                used_size: segment_size(sm, blocks),
                compression,
                bits,
                bookmark_bits: sm.stats.rle_runs,
                skipped: false,
            });
        }
    }

    // table id → columns (all tables, system ones included)
    let mut columns_of: HashMap<i64, Vec<ColumnStats>> = HashMap::new();
    let mut sorted_columns: Vec<&SqlRow> = columns.iter().collect();
    sorted_columns.sort_by_key(|c| {
        let cs = int(c, "ColumnStorageID").and_then(|i| cs_by_id.get(&i));
        (
            cs.and_then(|r| int(r, "StoragePosition")).unwrap_or(0),
            int(c, "ID").unwrap_or(0),
        )
    });
    for c in sorted_columns {
        let Some(table_id) = int(c, "TableID") else {
            continue;
        };
        let cs_id = int(c, "ColumnStorageID").unwrap_or(0);
        let cs = cs_by_id.get(&cs_id);
        let ds = cs
            .and_then(|r| int(r, "DictionaryStorageID"))
            .and_then(|d| ds_by_id.get(&d));
        let encoding = match ds.and_then(|d| int(d, "Type")) {
            Some(1) => "HASH",
            Some(2) => "VALUE",
            _ => "NONE",
        };
        let name = Some(text(c, "ExplicitName"))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| text(c, "InferredName"));
        let data_type = int(c, "ExplicitDataType")
            .filter(|t| *t != 1)
            .or_else(|| int(c, "InferredDataType"))
            .and_then(data_type_name);
        let is_row_number = int(c, "Type") == Some(3);
        let mut segments = segments_of.remove(&cs_id).unwrap_or_default();
        for seg in &mut segments {
            seg.skipped = is_row_number;
        }
        columns_of.entry(table_id).or_default().push(ColumnStats {
            id: int(c, "ID").unwrap_or(0),
            name,
            storage_name: cs.map(|r| text(r, "Name")).unwrap_or_default(),
            is_row_number,
            data_type,
            encoding,
            cardinality: cs.map_or(0, |r| uint(r, "Statistics_DistinctStates")),
            dictionary_size: ds.map_or(0, |d| uint(d, "Size")),
            segments,
            hierarchy: Vec::new(),
            hierarchy_table: None,
        });
    }

    // table id → partitions and rows
    let mut parts_of: HashMap<i64, Vec<PartitionStats>> = HashMap::new();
    for p in &partitions {
        let (Some(table), Some(ps)) = (int(p, "TableID"), int(p, "PartitionStorageID")) else {
            continue;
        };
        let psr = ps_by_id.get(&ps);
        let sm = psr
            .and_then(|r| int(r, "SegmentMapStorageID"))
            .and_then(|i| sm_by_id.get(&i));
        parts_of.entry(table).or_default().push(PartitionStats {
            name: text(p, "Name"),
            number: psr.and_then(|r| int(r, "StoragePosition")).unwrap_or(0),
            rows: sm.map_or(0, |r| uint(r, "RecordCount")),
            segments: sm.map_or(0, |r| uint(r, "SegmentCount")),
            records_per_segment: sm.map_or(0, |r| uint(r, "RecordsPerSegment")),
        });
    }

    let mut out = Storage {
        streamed: files.streamed,
        refresh: crate::refresh_info::from_db(&db)?,
        ..Storage::default()
    };
    let mut system: HashMap<i64, TableStats> = HashMap::new();
    let mut user: Vec<TableStats> = Vec::new();
    let mut table_rows: Vec<&SqlRow> = tables.iter().collect();
    table_rows.sort_by_key(|t| int(t, "ID").unwrap_or(0));
    for t in table_rows {
        let Some(id) = int(t, "ID") else {
            continue;
        };
        let mut parts = parts_of.remove(&id).unwrap_or_default();
        parts.sort_by_key(|p| p.number);
        let ts = int(t, "TableStorageID").and_then(|i| ts_by_id.get(&i));
        let stats = TableStats {
            id,
            name: text(t, "Name"),
            storage_name: ts.map(|r| text(r, "Name")).unwrap_or_default(),
            rows: parts.iter().map(|p| p.rows).fold(0, u64::saturating_add),
            ri_violations: ts.map_or(0, |r| uint(r, "RIViolationCount")),
            partitions: parts,
            columns: columns_of.remove(&id).unwrap_or_default(),
            user_hierarchies: Vec::new(),
        };
        if int(t, "SystemFlags").unwrap_or(0) & 1 != 0 {
            system.insert(id, stats);
        } else {
            user.push(stats);
        }
    }
    let system_segments = |id: i64| -> Vec<(String, Segment)> {
        system
            .get(&id)
            .map(|t| {
                t.columns
                    .iter()
                    .flat_map(|c| c.segments.iter().map(|s| (c.storage_name.clone(), s.clone())))
                    .collect()
            })
            .unwrap_or_default()
    };
    let system_size = |id: i64| -> u64 {
        system_segments(id)
            .iter()
            .map(|(_, s)| s.used_size)
            .fold(0, u64::saturating_add)
    };

    // attribute hierarchies (H$) and cardinalities
    let mut h_of_column: HashMap<i64, i64> = HashMap::new();
    let mut distinct_of_column: HashMap<i64, u64> = HashMap::new();
    for ah in &attribute_hierarchies {
        let (Some(col), Some(ahs)) = (int(ah, "ColumnID"), int(ah, "AttributeHierarchyStorageID")) else {
            continue;
        };
        let Some(ahs) = ahs_by_id.get(&ahs) else {
            continue;
        };
        if let Some(sys) = int(ahs, "SystemTableID").filter(|i| *i != 0) {
            h_of_column.insert(col, sys);
        }
        if let Some(n) = int(ahs, "DistinctDataCount").and_then(|n| u64::try_from(n).ok()) {
            distinct_of_column.insert(col, n);
        }
    }
    for t in &mut user {
        for c in &mut t.columns {
            if let Some(&sys) = h_of_column.get(&c.id) {
                c.hierarchy = system_segments(sys);
                c.hierarchy_table = system.get(&sys).map(|s| s.storage_name.clone());
                // The H$ table holds the distinct values plus 3 reserved rows.
                if let Some(rows) = system.get(&sys).map(|s| s.rows).filter(|r| *r >= 3) {
                    c.cardinality = rows - 3;
                    continue;
                }
            }
            if let Some(&n) = distinct_of_column.get(&c.id) {
                c.cardinality = n;
            }
        }
    }

    // user hierarchies (U$)
    for h in &hierarchies {
        let (Some(table), Some(hs)) = (int(h, "TableID"), int(h, "HierarchyStorageID")) else {
            continue;
        };
        let size = hs_by_id
            .get(&hs)
            .and_then(|r| int(r, "SystemTableID"))
            .map_or(0, system_size);
        if let Some(t) = user.iter_mut().find(|t| t.id == table) {
            t.user_hierarchies.push(UserHierarchyStats {
                name: text(h, "Name"),
                size,
            });
        }
    }

    // relationships (R$)
    let table_name = |id: i64| tables_by_id.get(&id).map(|r| text(r, "Name")).unwrap_or_default();
    let column_name = |id: i64| {
        columns_by_id
            .get(&id)
            .map(|c| {
                Some(text(c, "ExplicitName"))
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| text(c, "InferredName"))
            })
            .unwrap_or_default()
    };
    let index_of = |rs: i64| -> (u64, bool) {
        let Some(ri) = rs_by_id
            .get(&rs)
            .and_then(|r| int(r, "RelationshipIndexStorageID"))
            .and_then(|i| ri_by_id.get(&i))
        else {
            return (0, false);
        };
        let size = int(ri, "SystemTableID")
            .filter(|i| *i != 0)
            .map_or(0, system_size);
        (size, int(ri, "Flags").unwrap_or(0) & 1 != 0)
    };
    for r in &relationships {
        let (size_from, missing_from) = int(r, "RelationshipStorageID").map_or((0, false), index_of);
        let (size_to, missing_to) = int(r, "RelationshipStorage2ID").map_or((0, false), index_of);
        out.relationships.push(RelationshipStats {
            name: text(r, "Name"),
            from_table: table_name(int(r, "FromTableID").unwrap_or(0)),
            from_column: column_name(int(r, "FromColumnID").unwrap_or(0)),
            to_table: table_name(int(r, "ToTableID").unwrap_or(0)),
            to_column: column_name(int(r, "ToColumnID").unwrap_or(0)),
            size_from,
            size_to,
            missing_keys: missing_from || missing_to,
        });
    }
    out.tables = user;
    Ok(out)
}

fn segment_json(s: &Segment) -> Value {
    json!({
        "partition": s.partition,
        "partition_number": s.partition_number,
        "segment": s.number,
        "rows": s.records,
        "used_size": s.used_size,
        "compression": s.compression,
        "bits": s.bits,
        "bookmark_bits": s.bookmark_bits,
        "state": if s.skipped { "SKIPPED" } else { "COMPLETED" },
    })
}

fn column_json(t: &TableStats, c: &ColumnStats, segments: bool) -> Value {
    let rows = t.rows;
    let mut v = json!({
        "name": c.name,
        "data_type": c.data_type,
        "encoding": c.encoding.to_ascii_lowercase(),
        "cardinality": c.cardinality,
        "selectivity": if rows == 0 || c.cardinality == 0 { Value::Null } else { json!(c.cardinality as f64 / rows as f64) },
        "is_row_number": c.is_row_number,
        "dictionary_size": c.dictionary_size,
        "data_size": c.data_size(),
        "hierarchy_size": c.hierarchy_size(),
        "total_size": c.total_size(),
        "segment_count": c.segments.len(),
        "storage_name": c.storage_name,
    });
    if segments && let Some(o) = v.as_object_mut() {
        o.insert("segments".into(), c.segments.iter().map(segment_json).collect());
        o.insert(
            "hierarchy_segments".into(),
            c.hierarchy
                .iter()
                .map(|(col, s)| {
                    let mut x = segment_json(s);
                    if let Some(o) = x.as_object_mut() {
                        o.insert("structure".into(), json!(col));
                    }
                    x
                })
                .collect(),
        );
    }
    v
}

pub(crate) fn storage_stats(s: &Session, params: &Value) -> Result<Value, String> {
    allow(params, &["table", "columns", "segments", "row_number"])?;
    let only = opt_str(params, "table")?;
    let with_columns = opt_bool(params, "columns")?.unwrap_or(true);
    let with_segments = opt_bool(params, "segments")?.unwrap_or(false);
    let with_row_number = opt_bool(params, "row_number")?.unwrap_or(false);
    let st = load(s)?;
    if let Some(name) = only
        && !st.tables.iter().any(|t| t.name.eq_ignore_ascii_case(name))
    {
        return Err(format!("no stored table named '{name}'"));
    }
    let total: u64 = st
        .tables
        .iter()
        .map(|t| st.table_size(t))
        .fold(0, u64::saturating_add);
    let tables: Vec<Value> = st
        .tables
        .iter()
        .filter(|t| only.is_none_or(|n| t.name.eq_ignore_ascii_case(n)))
        .map(|t| {
            let size = st.table_size(t);
            let mut v = json!({
                "name": t.name,
                "rows": t.rows,
                "total_size": size,
                "percent_of_model": if total == 0 { 0.0 } else { size as f64 * 100.0 / total as f64 },
                "columns_size": t.sum(ColumnStats::total_size),
                "dictionary_size": t.sum(|c| c.dictionary_size),
                "data_size": t.sum(ColumnStats::data_size),
                "hierarchy_size": t.sum(ColumnStats::hierarchy_size),
                "user_hierarchies_size": t.user_hierarchies_size(),
                "relationships_size": st.relationships_size(&t.name),
                "segment_count": t.partitions.iter().map(|p| p.segments).fold(0, u64::saturating_add),
                "referential_integrity_violations": t.ri_violations,
                "partitions": t.partitions.iter().map(|p| {
                    let mut v = json!({
                        "name": p.name, "number": p.number, "rows": p.rows, "segments": p.segments,
                        "records_per_segment": p.records_per_segment,
                    });
                    let r = st.refresh.table(&t.name).and_then(|r| r.partition(&p.name));
                    if let (Some(r), Some(o)) = (r, v.as_object_mut()) {
                        r.insert_into(o);
                    }
                    v
                }).collect::<Vec<_>>(),
                "user_hierarchies": t.user_hierarchies.iter().map(|h| json!({"name": h.name, "size": h.size})).collect::<Vec<_>>(),
            });
            if with_columns && let Some(o) = v.as_object_mut() {
                o.insert(
                    "columns".into(),
                    t.columns
                        .iter()
                        .filter(|c| with_row_number || !c.is_row_number)
                        .map(|c| column_json(t, c, with_segments))
                        .collect(),
                );
            }
            v
        })
        .collect();
    let relationships: Vec<Value> = st
        .relationships
        .iter()
        .filter(|r| {
            only.is_none_or(|n| r.from_table.eq_ignore_ascii_case(n) || r.to_table.eq_ignore_ascii_case(n))
        })
        .map(|r| {
            json!({
                "name": r.name,
                "from": format!("{}[{}]", r.from_table, r.from_column),
                "to": format!("{}[{}]", r.to_table, r.to_column),
                "size_from": r.size_from,
                "size_to": r.size_to,
                "size": r.size_from.saturating_add(r.size_to),
                "missing_keys": r.missing_keys,
            })
        })
        .collect();
    Ok(json!({
        "source": st.source,
        "streamed": st.streamed,
        "total_size": total,
        "table_count": st.tables.len(),
        "refresh": st.refresh.json(),
        "tables": tables,
        "relationships": relationships,
    }))
}

#[cfg(test)]
#[path = "storage_stats_tests.rs"]
mod storage_stats_tests;
