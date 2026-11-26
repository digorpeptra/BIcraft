//! Row generation for new tables, columns and relationships, with their storage rows, system
//! tables and files. Every value is documented in `docs/formats/metadata-sqlitedb.md`
//! ("Structural writing"); "observed" means read off the Desktop 2.158 sample file.

use bicraft_model::{Column, DataType, Relationship, Table};
use serde_json::Value;

use crate::Error;
use crate::codes::{self, column_type as ct, object_type as ot};
use crate::input::MetaValue;
use crate::structural::data::{
    ColumnStorageData, EncodedRelationship, EncodedTable, PartitionSegments, SegmentMap, SystemTableData,
};
use crate::structural::names;
use crate::structural::store::{NEVER, Store};

/// Name of the row-number column of every table (the same GUID in every table, observed).
pub const ROW_NUMBER_NAME: &str = "RowNumber-2662979B-1795-4F74-8F37-6A1BA8059B61";

/// `StorageFolder.OwnerType` / `StorageFile.OwnerType` codes (observed).
pub(crate) mod owner {
    pub(crate) const TABLE_STORAGE: i64 = 18;
    pub(crate) const PARTITION_STORAGE: i64 = 20;
    pub(crate) const DICTIONARY_STORAGE: i64 = 22;
    pub(crate) const COLUMN_PARTITION_STORAGE: i64 = 23;
    pub(crate) const SEGMENT_STORAGE: i64 = 24;
    pub(crate) const ATTRIBUTE_HIERARCHY_STORAGE: i64 = 27;
}

/// `ColumnStorage.ColumnFlags` bits (inferred from the observed values 0, 8, 11, 15, 27, 31).
mod flags {
    /// Unique values: `IsUnique`, or the "one" end of a relationship.
    pub(crate) const UNIQUE: i64 = 3;
    /// `IsKey`.
    pub(crate) const KEY: i64 = 4;
    /// The column has a materialized attribute hierarchy.
    pub(crate) const HIERARCHY: i64 = 8;
    /// The row-number column.
    pub(crate) const ROW_NUMBER: i64 = 16;
}

/// `TableStorage.Settings` of user tables (observed 4353 on every user table of the sample,
/// all calculated; import tables are not observed).
const USER_TABLE_SETTINGS: i64 = 4353;
/// `ColumnStorage.Settings`: 1 for data, row-number and calculated-table columns, 2049 for
/// calculated columns (observed).
const DATA_COLUMN_SETTINGS: i64 = 1;
const CALCULATED_COLUMN_SETTINGS: i64 = 2049;

/// What kind of system table to create.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SystemKind {
    /// `H$`: attribute hierarchy of a column.
    Hierarchy,
    /// `R$`: relationship index.
    Relationship,
    /// `U$`: user hierarchy nodes.
    UserHierarchy,
}

impl SystemKind {
    /// (`TableStorage.Version`, `TableStorage.Settings`), observed.
    fn table_storage(self) -> (i64, i64) {
        match self {
            SystemKind::Hierarchy => (1, 4),
            SystemKind::Relationship => (0, 2),
            SystemKind::UserHierarchy => (0, 5),
        }
    }
}

/// `ColumnStorage.Settings` of a system column, by name (observed).
fn system_column_settings(name: &str) -> i64 {
    match name {
        "POS_TO_ID" => 7,
        "ID_TO_POS" => 5,
        "INDEX" => 3,
        "MULTI_LEVEL_ID" => 16,
        "PARENT_POS" => 8,
        "FIRST_CHILD_POS" => 9,
        "CHILD_COUNT" => 17,
        _ => 0,
    }
}

/// Column `ExplicitDataType` code of a model data type.
pub(crate) fn data_type_code(t: DataType) -> Option<i64> {
    Some(match t {
        DataType::String => 2,
        DataType::Int64 => 6,
        DataType::Double => 8,
        DataType::DateTime => 9,
        DataType::Decimal => 10,
        DataType::Boolean => 11,
        DataType::Binary => 17,
        DataType::Variant => 20,
        _ => return None,
    })
}

fn text(v: Option<&str>) -> MetaValue {
    v.map_or(MetaValue::Null, MetaValue::from)
}

fn extra_bool(c: &serde_json::Map<String, Value>, key: &str, default: bool) -> bool {
    c.get(key).and_then(Value::as_bool).unwrap_or(default)
}

fn extra_str<'a>(c: &'a serde_json::Map<String, Value>, key: &str) -> Option<&'a str> {
    c.get(key).and_then(Value::as_str)
}

fn extra_int(c: &serde_json::Map<String, Value>, key: &str, default: i64) -> i64 {
    c.get(key).and_then(Value::as_i64).unwrap_or(default)
}

fn extra_code(
    c: &serde_json::Map<String, Value>,
    key: &str,
    f: fn(i64) -> Option<&'static str>,
    default: i64,
) -> i64 {
    extra_str(c, key)
        .and_then(|s| codes::code_of(f, s))
        .unwrap_or(default)
}

/// The table a new column goes into, and where its files go.
#[derive(Debug, Clone)]
pub(crate) struct ColumnHome {
    pub table_id: i64,
    pub table_storage_name: String,
    pub table_folder: i64,
    /// `(PartitionStorage ID, partition StorageFolder ID)` of each partition, in storage order.
    pub partitions: Vec<(i64, i64)>,
    /// `SystemFlags` of the table's columns: 2 in a calculated table, else 0.
    pub system_flags: i64,
    /// `ColumnStorage.Locale` (copied from the model's existing columns).
    pub locale: i64,
}

/// A user column to create.
pub(crate) struct NewColumn<'a> {
    /// `None` for the row-number column.
    pub model: Option<&'a Column>,
    pub data: &'a ColumnStorageData,
    pub position: i64,
    /// The "one" end of a relationship of the final model.
    pub to_end: bool,
    /// The row-number column keeps `IsKey` unless another column of the table is the key.
    pub row_number_is_key: bool,
    /// Lineage tag to write when the model has none.
    pub lineage_tag: Option<String>,
    /// Segments per partition (tables of several partitions), else `data`'s.
    pub partitions: &'a [PartitionSegments],
}

impl Store {
    /// Adds a table with its partitions (one, or several with `enc.partitions`), its row-number
    /// column and every model column. Returns the new table's ID.
    pub(crate) fn add_table(
        &mut self,
        model_id: i64,
        t: &Table,
        enc: &EncodedTable,
        to_end: &dyn Fn(&str, &str) -> bool,
        locale: i64,
        lineage: &mut dyn FnMut(&str) -> String,
    ) -> Result<i64, Error> {
        let partition = t
            .partitions
            .first()
            .ok_or_else(|| Error::Invalid("a new table needs a partition".into()))?;
        let part_type = |p: &bicraft_model::Partition| match p.source_type.as_str() {
            "calculated" => Ok(2),
            "m" => Ok(4),
            "query" => Ok(1),
            // Its rows are the calculation items (engine-observed: stored like an import table).
            "calculationGroup" => Ok(7),
            other => Err(Error::Invalid(format!(
                "partition source type `{other}` is not supported for new tables"
            ))),
        };
        let calculated = part_type(partition)? == 2;
        let maps: Vec<&SegmentMap> = if enc.partitions.is_empty() {
            vec![&enc.segment_map]
        } else {
            enc.partitions.iter().map(|p| &p.segment_map).collect()
        };
        if maps.len() != t.partitions.len() || (calculated && maps.len() != 1) {
            return Err(Error::Invalid(format!(
                "table `{}`: {} encoded partitions for {} partitions",
                t.name,
                maps.len(),
                t.partitions.len()
            )));
        }
        let system_flags = if calculated { 2 } else { 0 };
        let now = self.now;
        let table_id = self.alloc()?;
        let ts_id = self.alloc()?;
        let ts_name = names::object_storage_name(&t.name, table_id);
        let tbl_folder = self.new_folder(ts_id, owner::TABLE_STORAGE, &names::table_folder(&ts_name))?;
        let tbl_path = names::table_folder(&ts_name);
        let x = &t.extra;
        let lineage_tag = t.lineage_tag.clone().unwrap_or_else(|| lineage(&t.name));
        self.insert(
            "Table",
            vec![
                ("ID", table_id.into()),
                ("ModelID", model_id.into()),
                ("Name", t.name.as_str().into()),
                ("DataCategory", text(extra_str(x, "dataCategory"))),
                ("Description", text(t.description.as_deref())),
                ("IsHidden", t.is_hidden.into()),
                ("TableStorageID", ts_id.into()),
                ("ModifiedTime", now.into()),
                ("StructureModifiedTime", now.into()),
                ("SystemFlags", system_flags.into()),
                (
                    "ShowAsVariationsOnly",
                    extra_bool(x, "showAsVariationsOnly", false).into(),
                ),
                ("IsPrivate", extra_bool(x, "isPrivate", false).into()),
                ("DefaultDetailRowsDefinitionID", 0.into()),
                (
                    "AlternateSourcePrecedence",
                    extra_int(x, "alternateSourcePrecedence", 0).into(),
                ),
                ("RefreshPolicyID", 0.into()),
                ("CalculationGroupID", 0.into()),
                (
                    "ExcludeFromModelRefresh",
                    extra_bool(x, "excludeFromModelRefresh", false).into(),
                ),
                ("LineageTag", lineage_tag.into()),
                ("SourceLineageTag", text(extra_str(x, "sourceLineageTag"))),
                ("SystemManaged", extra_bool(x, "systemManaged", false).into()),
                (
                    "ExcludeFromAutomaticAggregations",
                    extra_bool(x, "excludeFromAutomaticAggregations", false).into(),
                ),
                ("DirectLakeIndexingBehavior", 3.into()),
            ],
        );
        self.insert(
            "TableStorage",
            vec![
                ("ID", ts_id.into()),
                ("TableID", table_id.into()),
                ("Name", ts_name.as_str().into()),
                ("Version", 1.into()),
                ("Settings", USER_TABLE_SETTINGS.into()),
                ("RIViolationCount", 0.into()),
                ("StorageFolderID", tbl_folder.into()),
            ],
        );

        // The partitions, in model order (their storage positions).
        let mut home_partitions = Vec::with_capacity(maps.len());
        for (position, (partition, map)) in t.partitions.iter().zip(&maps).enumerate() {
            let part_type = part_type(partition)?;
            let part_id = self.alloc()?;
            let ps_id = self.alloc()?;
            let map_id = self.alloc()?;
            let prt_folder = self.new_folder(
                ps_id,
                owner::PARTITION_STORAGE,
                &names::partition_folder(&tbl_path, part_id),
            )?;
            let px = &partition.extra;
            let mode = partition
                .mode
                .as_deref()
                .map_or(Some(2), |m| codes::code_of(codes::mode, m))
                .ok_or_else(|| Error::Invalid("unknown partition mode".into()))?;
            self.insert(
                "Partition",
                vec![
                    ("ID", part_id.into()),
                    ("TableID", table_id.into()),
                    ("Name", partition.name.as_str().into()),
                    ("Description", text(extra_str(px, "description"))),
                    ("DataSourceID", 0.into()),
                    ("QueryDefinition", text(partition.expression.as_deref())),
                    ("State", 1.into()),
                    ("Type", part_type.into()),
                    ("PartitionStorageID", ps_id.into()),
                    ("Mode", mode.into()),
                    ("DataView", extra_code(px, "dataView", codes::data_view, 3).into()),
                    ("ModifiedTime", now.into()),
                    ("RefreshedTime", now.into()),
                    ("SystemFlags", system_flags.into()),
                    ("ErrorMessage", MetaValue::Null),
                    (
                        "RetainDataTillForceCalculate",
                        extra_bool(px, "retainDataTillForceCalculate", false).into(),
                    ),
                    ("RangeStart", 0.0.into()),
                    ("RangeEnd", 0.0.into()),
                    ("RangeGranularity", (-1).into()),
                    ("RefreshBookmark", MetaValue::Null),
                    ("QueryGroupID", 0.into()),
                    ("ExpressionSourceID", 0.into()),
                    ("MAttributes", MetaValue::Null),
                    ("DataCoverageDefinitionID", 0.into()),
                    ("SchemaName", MetaValue::Null),
                ],
            );
            self.insert(
                "PartitionStorage",
                vec![
                    ("ID", ps_id.into()),
                    ("PartitionID", part_id.into()),
                    (
                        "Name",
                        names::object_storage_name(&partition.name, part_id).into(),
                    ),
                    ("StoragePosition", (position as i64).into()),
                    ("SegmentMapStorageID", map_id.into()),
                    ("DataObjectId", (position as i64).into()),
                    ("StorageFolderID", prt_folder.into()),
                    ("DeltaTableMetadataStorageID", 0.into()),
                ],
            );
            self.insert_segment_map(map_id, ps_id, map);
            home_partitions.push((ps_id, prt_folder));
        }

        // The columns: row number first, then the model's in order.
        let home = ColumnHome {
            table_id,
            table_storage_name: ts_name,
            table_folder: tbl_folder,
            partitions: home_partitions,
            system_flags,
            locale,
        };
        let row_number_segments: Vec<PartitionSegments> =
            enc.partitions.iter().map(|p| p.row_number.clone()).collect();
        let has_key = t
            .columns
            .iter()
            .any(|c| !is_row_number(c) && extra_bool(&c.extra, "isKey", false));
        self.add_column(
            &home,
            &NewColumn {
                model: None,
                data: &enc.row_number,
                position: 0,
                to_end: false,
                row_number_is_key: !has_key,
                lineage_tag: None,
                partitions: &row_number_segments,
            },
        )?;
        let mut position = 1i64;
        for c in t.columns.iter().filter(|c| !is_row_number(c)) {
            let data = enc
                .columns
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(&c.name))
                .map(|(_, d)| d)
                .ok_or_else(|| Error::Invalid(format!("no encoded data for column `{}`", c.name)))?;
            let tag = c
                .lineage_tag
                .is_none()
                .then(|| lineage(&format!("{}\0{}", t.name, c.name)));
            let mut segments = Vec::with_capacity(enc.partitions.len());
            for p in &enc.partitions {
                let seg = p
                    .columns
                    .iter()
                    .find(|(n, _)| n.eq_ignore_ascii_case(&c.name))
                    .map(|(_, s)| s.clone())
                    .ok_or_else(|| {
                        Error::Invalid(format!("no partition segments for column `{}`", c.name))
                    })?;
                segments.push(seg);
            }
            self.add_column(
                &home,
                &NewColumn {
                    model: Some(c),
                    data,
                    position,
                    to_end: to_end(&t.name, &c.name),
                    row_number_is_key: false,
                    lineage_tag: tag,
                    partitions: &segments,
                },
            )?;
            position = position.saturating_add(1);
        }
        Ok(table_id)
    }

    fn insert_segment_map(&mut self, id: i64, partition_storage: i64, m: &SegmentMap) {
        self.insert(
            "SegmentMapStorage",
            vec![
                ("ID", id.into()),
                ("PartitionStorageID", partition_storage.into()),
                ("Type", m.kind.into()),
                ("RecordCount", m.record_count.into()),
                ("SegmentCount", m.segment_count.into()),
                ("RecordsPerSegment", m.records_per_segment.into()),
            ],
        );
    }

    /// Adds a user column (or the row-number column) with its storage rows, files, attribute
    /// hierarchy and `H$` table. Returns the column ID.
    pub(crate) fn add_column(&mut self, home: &ColumnHome, c: &NewColumn<'_>) -> Result<i64, Error> {
        let now = self.now;
        let column_id = self.alloc()?;
        let cs_id = self.alloc()?;
        let dict_id = self.alloc()?;
        let cps_id = self.alloc()?;
        let seg_id = self.alloc()?;
        let ah_id = self.alloc()?;
        let ahs_id = self.alloc()?;
        let empty = serde_json::Map::new();
        let (name, x) = match c.model {
            Some(m) => (m.name.as_str(), &m.extra),
            None => (ROW_NUMBER_NAME, &empty),
        };
        let row_number = c.model.is_none();
        let kind = match c.model {
            None => ct::ROW_NUMBER,
            Some(m) => match extra_str(&m.extra, "type") {
                Some("calculatedTableColumn") => ct::CALCULATED_TABLE_COLUMN,
                Some("calculated") => ct::CALCULATED,
                Some("rowNumber") => ct::ROW_NUMBER,
                _ if m.expression.is_some() => ct::CALCULATED,
                _ => ct::DATA,
            },
        };
        let type_code = match c.model {
            None => 6,
            Some(m) => data_type_code(m.data_type)
                .ok_or_else(|| Error::Invalid(format!("column `{name}` has no storable data type")))?,
        };
        let explicit_type = if extra_bool(x, "isDataTypeInferred", false) {
            codes::DATA_TYPE_AUTOMATIC
        } else {
            type_code
        };
        // Observed: 19 (unknown) on row-number columns, the data type elsewhere.
        let inferred_type = if row_number { 19 } else { type_code };
        let name_inferred = extra_bool(x, "isNameInferred", false);
        let is_unique = extra_bool(x, "isUnique", row_number);
        let is_key = if row_number {
            c.row_number_is_key
        } else {
            extra_bool(x, "isKey", false)
        };
        let has_hierarchy = c.data.hierarchy.is_some();
        let available_in_mdx = crate::column_available_in_mdx(x);
        if available_in_mdx != has_hierarchy && !row_number {
            return Err(Error::Invalid(format!(
                "column `{name}`: isAvailableInMdx and the encoded attribute hierarchy disagree"
            )));
        }
        let column_flags = if row_number { flags::ROW_NUMBER } else { 0 }
            | if has_hierarchy { flags::HIERARCHY } else { 0 }
            | if is_key { flags::KEY } else { 0 }
            | if is_unique || c.to_end { flags::UNIQUE } else { 0 };
        let summarize = c
            .model
            .and_then(|m| m.summarize_by.as_deref())
            .map_or(Some(1), |s| codes::code_of(codes::summarize_by, s))
            .ok_or_else(|| Error::Invalid(format!("column `{name}`: unknown summarizeBy")))?;
        let lineage = c
            .model
            .and_then(|m| m.lineage_tag.clone())
            .or_else(|| c.lineage_tag.clone());
        let m = c.model;
        self.insert(
            "Column",
            vec![
                ("ID", column_id.into()),
                ("TableID", home.table_id.into()),
                (
                    "ExplicitName",
                    if name_inferred {
                        MetaValue::Null
                    } else {
                        name.into()
                    },
                ),
                (
                    "InferredName",
                    if name_inferred {
                        name.into()
                    } else {
                        MetaValue::Null
                    },
                ),
                ("ExplicitDataType", explicit_type.into()),
                ("InferredDataType", inferred_type.into()),
                ("DataCategory", text(m.and_then(|m| m.data_category.as_deref()))),
                ("Description", text(m.and_then(|m| m.description.as_deref()))),
                ("IsHidden", m.is_none_or(|m| m.is_hidden).into()),
                ("State", 1.into()),
                ("IsUnique", is_unique.into()),
                ("IsKey", is_key.into()),
                ("IsNullable", extra_bool(x, "isNullable", !row_number).into()),
                (
                    "Alignment",
                    extra_code(x, "alignment", codes::alignment, 1).into(),
                ),
                (
                    "TableDetailPosition",
                    extra_int(x, "tableDetailPosition", -1).into(),
                ),
                ("IsDefaultLabel", extra_bool(x, "isDefaultLabel", false).into()),
                ("IsDefaultImage", extra_bool(x, "isDefaultImage", false).into()),
                ("SummarizeBy", summarize.into()),
                ("ColumnStorageID", cs_id.into()),
                ("Type", kind.into()),
                ("SourceColumn", text(m.and_then(|m| m.source_column.as_deref()))),
                ("ColumnOriginID", 0.into()),
                ("Expression", text(m.and_then(|m| m.expression.as_deref()))),
                ("FormatString", text(m.and_then(|m| m.format_string.as_deref()))),
                ("IsAvailableInMDX", available_in_mdx.into()),
                ("SortByColumnID", 0.into()),
                ("AttributeHierarchyID", ah_id.into()),
                ("ModifiedTime", now.into()),
                ("StructureModifiedTime", now.into()),
                ("RefreshedTime", NEVER.into()),
                ("SystemFlags", home.system_flags.into()),
                ("KeepUniqueRows", extra_bool(x, "keepUniqueRows", false).into()),
                ("DisplayOrdinal", extra_int(x, "displayOrdinal", 0).into()),
                ("ErrorMessage", MetaValue::Null),
                ("SourceProviderType", text(extra_str(x, "sourceProviderType"))),
                ("DisplayFolder", text(m.and_then(|m| m.display_folder.as_deref()))),
                (
                    "EncodingHint",
                    extra_code(x, "encodingHint", codes::encoding_hint, 0).into(),
                ),
                ("RelatedColumnDetailsID", 0.into()),
                ("AlternateOfID", 0.into()),
                ("LineageTag", text(lineage.as_deref())),
                ("SourceLineageTag", text(extra_str(x, "sourceLineageTag"))),
                ("ExpressionContext", 1.into()),
                ("StringIndexingBehavior", 1.into()),
                ("FullTextIndexingBehavior", 0.into()),
            ],
        );

        let cs_name = names::object_storage_name(name, column_id);
        let ts = home.table_storage_name.as_str();
        let s = &c.data.statistics;
        let settings = if kind == ct::CALCULATED {
            CALCULATED_COLUMN_SETTINGS
        } else {
            DATA_COLUMN_SETTINGS
        };
        self.insert(
            "ColumnStorage",
            vec![
                ("ID", cs_id.into()),
                ("ColumnID", column_id.into()),
                ("Name", cs_name.as_str().into()),
                ("StoragePosition", c.position.into()),
                ("DictionaryStorageID", dict_id.into()),
                ("Settings", settings.into()),
                ("ColumnFlags", column_flags.into()),
                ("Collation", MetaValue::Null),
                ("OrderByColumn", MetaValue::Null),
                ("Locale", home.locale.into()),
                ("BinaryCharacters", 0.into()),
                ("Statistics_DistinctStates", s.distinct_states.into()),
                ("Statistics_MinDataID", s.min_data_id.into()),
                ("Statistics_MaxDataID", s.max_data_id.into()),
                (
                    "Statistics_OriginalMinSegmentDataID",
                    s.original_min_segment_data_id.into(),
                ),
                ("Statistics_RLESortOrder", s.rle_sort_order.into()),
                ("Statistics_RowCount", s.row_count.into()),
                ("Statistics_HasNulls", s.has_nulls.into()),
                ("Statistics_RLERuns", s.rle_runs.into()),
                ("Statistics_OthersRLERuns", s.others_rle_runs.into()),
                ("Statistics_Usage", if c.to_end { 0 } else { 3 }.into()),
                ("Statistics_DBType", s.db_type.into()),
                ("Statistics_XMType", s.xm_type.into()),
                ("Statistics_CompressionType", s.compression_type.into()),
                ("Statistics_CompressionParam", s.compression_param.into()),
                ("Statistics_EncodingHint", (row_number || c.to_end).into()),
                ("IsDeltaPartitionColumn", 0.into()),
                ("DeltaColumnMappingPhysicalName", MetaValue::Null),
                ("DeltaColumnMappingId", (-1).into()),
                ("FramedSourceColumn", MetaValue::Null),
                ("DeltaColumnPrimitiveType", 0.into()),
                ("DeltaTableColumnStorageID", 0.into()),
                ("StringIndexStorageID", 0.into()),
                ("ColumnIndexStorageID", 0.into()),
                ("FullTextIndexStorageID", 0.into()),
            ],
        );

        // Dictionary.
        let d = &c.data.dictionary;
        let dict_file = match &d.file {
            Some(bytes) => self.new_file(
                dict_id,
                owner::DICTIONARY_STORAGE,
                home.table_folder,
                &names::dictionary(1, ts, &cs_name),
                bytes.clone(),
            )?,
            None => 0,
        };
        self.insert(
            "DictionaryStorage",
            vec![
                ("ID", dict_id.into()),
                ("ColumnStorageID", cs_id.into()),
                ("Type", d.kind.into()),
                ("DataType", d.data_type.into()),
                ("DataVersion", 1.into()),
                ("BaseId", d.base_id.into()),
                ("Magnitude", d.magnitude.into()),
                ("LastId", d.last_id.into()),
                ("IsNullable", d.is_nullable.into()),
                ("IsUnique", (d.is_unique || (c.to_end && d.kind == 1)).into()),
                ("IsOperatingOn32", d.is_operating_on_32.into()),
                ("DictionaryFlags", d.flags.into()),
                ("StorageFileID", dict_file.into()),
                ("Size", d.size.into()),
            ],
        );

        // Segment data and statistics, one file pair per partition (the first partition's ids
        // allocated above, the others' after the attribute hierarchy's).
        let single = [PartitionSegments {
            idf: c.data.idf.clone(),
            idfmeta: c.data.idfmeta.clone(),
            segment_count: c.data.segment_count,
            rows: c.data.statistics.row_count,
        }];
        let segments: &[PartitionSegments] = if c.partitions.is_empty() {
            &single
        } else {
            c.partitions
        };
        if segments.len() != home.partitions.len() {
            return Err(Error::Invalid(format!(
                "column `{name}`: {} encoded partitions for a table of {}",
                segments.len(),
                home.partitions.len()
            )));
        }
        let mut later = Vec::new();
        for (k, (seg, &(partition_storage, partition_folder))) in
            segments.iter().zip(&home.partitions).enumerate()
        {
            let (cps_id, seg_id) = if k == 0 {
                (cps_id, seg_id)
            } else {
                later.push((k, seg, partition_storage, partition_folder));
                continue;
            };
            self.write_column_partition(
                (cps_id, seg_id, cs_id),
                (partition_storage, partition_folder),
                &names::idf(1, ts, &cs_name, k),
                &names::idfmeta(1, ts, &cs_name, k),
                seg,
            )?;
        }

        // Attribute hierarchy.
        self.insert(
            "AttributeHierarchy",
            vec![
                ("ID", ah_id.into()),
                ("ColumnID", column_id.into()),
                ("State", 1.into()),
                ("AttributeHierarchyStorageID", ahs_id.into()),
                ("ModifiedTime", now.into()),
                ("RefreshedTime", now.into()),
            ],
        );
        let mut ahs: Vec<(&str, MetaValue)> = vec![
            ("ID", ahs_id.into()),
            ("AttributeHierarchyID", ah_id.into()),
            ("SortOrder", 0.into()),
            ("OptimizationLevel", 0.into()),
            ("DataVersion", 1.into()),
        ];
        match &c.data.hierarchy {
            None => ahs.extend([
                ("MaterializationType", (-1).into()),
                ("ColumnPositionToData", (-1).into()),
                ("ColumnDataToPosition", (-1).into()),
                ("DistinctDataCount", (-1).into()),
                ("StorageFileID", 0.into()),
                ("SystemTableID", 0.into()),
                ("HasStatistics", 0.into()),
                ("MinValue", MetaValue::Null),
                ("MaxValue", MetaValue::Null),
                ("StringValueMaxLength", 0.into()),
            ]),
            Some(h) => {
                let hidx_file = match &h.hidx {
                    Some(bytes) => self.new_file(
                        ahs_id,
                        owner::ATTRIBUTE_HIERARCHY_STORAGE,
                        home.table_folder,
                        &names::hidx(1, ts, &cs_name),
                        bytes.clone(),
                    )?,
                    None => 0,
                };
                let (system_table, pos_to_data, data_to_pos) = match &h.system_table {
                    Some(sys) => {
                        let position = |n: &str| {
                            sys.columns
                                .iter()
                                .position(|c| c.name == n)
                                .map_or(-1, |p| p as i64)
                        };
                        let tid = self.add_system_table(
                            SystemKind::Hierarchy,
                            &names::hierarchy_table(ts, &cs_name),
                            sys,
                        )?;
                        (tid, position("POS_TO_ID"), position("ID_TO_POS"))
                    }
                    None => (0, -1, -1),
                };
                ahs.extend([
                    ("MaterializationType", h.materialization_type.into()),
                    ("ColumnPositionToData", pos_to_data.into()),
                    ("ColumnDataToPosition", data_to_pos.into()),
                    ("DistinctDataCount", h.distinct_data_count.into()),
                    ("StorageFileID", hidx_file.into()),
                    ("SystemTableID", system_table.into()),
                    ("HasStatistics", 1.into()),
                    ("MinValue", text(h.min_value.as_deref())),
                    ("MaxValue", text(h.max_value.as_deref())),
                    ("StringValueMaxLength", h.string_value_max_length.into()),
                ]);
            }
        }
        self.insert("AttributeHierarchyStorage", ahs);
        for (k, seg, partition_storage, partition_folder) in later {
            let ids = (self.alloc()?, self.alloc()?, cs_id);
            self.write_column_partition(
                ids,
                (partition_storage, partition_folder),
                &names::idf(1, ts, &cs_name, k),
                &names::idfmeta(1, ts, &cs_name, k),
                seg,
            )?;
        }
        Ok(column_id)
    }

    /// `ColumnPartitionStorage` + `SegmentStorage` rows and the `.idf`/`.idfmeta` files of one
    /// column in one partition. `ids`: (ColumnPartitionStorage, SegmentStorage, ColumnStorage);
    /// `partition`: (PartitionStorage, its StorageFolder).
    fn write_column_partition(
        &mut self,
        ids: (i64, i64, i64),
        partition: (i64, i64),
        idf_name: &str,
        idfmeta_name: &str,
        seg: &PartitionSegments,
    ) -> Result<(), Error> {
        let (cps_id, seg_id, cs_id) = ids;
        let (partition_storage, partition_folder) = partition;
        let idf_file = self.new_file(
            cps_id,
            owner::COLUMN_PARTITION_STORAGE,
            partition_folder,
            idf_name,
            seg.idf.clone(),
        )?;
        self.insert(
            "ColumnPartitionStorage",
            vec![
                ("ID", cps_id.into()),
                ("ColumnStorageID", cs_id.into()),
                ("PartitionStorageID", partition_storage.into()),
                ("DataVersion", 1.into()),
                ("State", 1.into()),
                ("SegmentStorageID", seg_id.into()),
                ("StorageFileID", idf_file.into()),
            ],
        );
        let meta_file = self.new_file(
            seg_id,
            owner::SEGMENT_STORAGE,
            partition_folder,
            idfmeta_name,
            seg.idfmeta.clone(),
        )?;
        self.insert(
            "SegmentStorage",
            vec![
                ("ID", seg_id.into()),
                ("ColumnPartitionStorageID", cps_id.into()),
                ("SegmentCount", seg.segment_count.into()),
                ("StorageFileID", meta_file.into()),
            ],
        );
        Ok(())
    }

    /// Adds an engine system table (`H$` or `R$`) holding `data`. Returns its table ID.
    pub(crate) fn add_system_table(
        &mut self,
        kind: SystemKind,
        name: &str,
        data: &SystemTableData,
    ) -> Result<i64, Error> {
        let now = self.now;
        let table_id = self.alloc()?;
        let ts_id = self.alloc()?;
        let part_id = self.alloc()?;
        let ps_id = self.alloc()?;
        let map_id = self.alloc()?;
        let folder_path = names::system_table_folder(name, table_id);
        let tbl_folder = self.new_folder(ts_id, owner::TABLE_STORAGE, &folder_path)?;
        let prt_folder = self.new_folder(
            ps_id,
            owner::PARTITION_STORAGE,
            &names::partition_folder(&folder_path, part_id),
        )?;
        self.insert(
            "Table",
            vec![
                ("ID", table_id.into()),
                ("ModelID", 0.into()),
                ("Name", name.into()),
                ("DataCategory", MetaValue::Null),
                ("Description", MetaValue::Null),
                ("IsHidden", 1.into()),
                ("TableStorageID", ts_id.into()),
                ("ModifiedTime", now.into()),
                ("StructureModifiedTime", now.into()),
                ("SystemFlags", 1.into()),
                ("ShowAsVariationsOnly", 0.into()),
                ("IsPrivate", 0.into()),
                ("DefaultDetailRowsDefinitionID", 0.into()),
                ("AlternateSourcePrecedence", 0.into()),
                ("RefreshPolicyID", 0.into()),
                ("CalculationGroupID", 0.into()),
                ("ExcludeFromModelRefresh", 0.into()),
                ("LineageTag", MetaValue::Null),
                ("SourceLineageTag", MetaValue::Null),
                ("SystemManaged", 0.into()),
                ("ExcludeFromAutomaticAggregations", 0.into()),
                ("DirectLakeIndexingBehavior", 3.into()),
            ],
        );
        let (version, settings) = kind.table_storage();
        self.insert(
            "TableStorage",
            vec![
                ("ID", ts_id.into()),
                ("TableID", table_id.into()),
                ("Name", name.into()),
                ("Version", version.into()),
                ("Settings", settings.into()),
                ("RIViolationCount", 0.into()),
                ("StorageFolderID", tbl_folder.into()),
            ],
        );
        self.insert(
            "Partition",
            vec![
                ("ID", part_id.into()),
                ("TableID", table_id.into()),
                ("Name", name.into()),
                ("Description", MetaValue::Null),
                ("DataSourceID", 0.into()),
                ("QueryDefinition", MetaValue::Null),
                ("State", 1.into()),
                ("Type", 3.into()),
                ("PartitionStorageID", ps_id.into()),
                ("Mode", 2.into()),
                ("DataView", 3.into()),
                ("ModifiedTime", now.into()),
                ("RefreshedTime", NEVER.into()),
                ("SystemFlags", 1.into()),
                ("ErrorMessage", MetaValue::Null),
                ("RetainDataTillForceCalculate", 0.into()),
                ("RangeStart", 0.0.into()),
                ("RangeEnd", 0.0.into()),
                ("RangeGranularity", (-1).into()),
                ("RefreshBookmark", MetaValue::Null),
                ("QueryGroupID", 0.into()),
                ("ExpressionSourceID", 0.into()),
                ("MAttributes", MetaValue::Null),
                ("DataCoverageDefinitionID", 0.into()),
                ("SchemaName", MetaValue::Null),
            ],
        );
        self.insert(
            "PartitionStorage",
            vec![
                ("ID", ps_id.into()),
                ("PartitionID", part_id.into()),
                ("Name", name.into()),
                ("StoragePosition", 0.into()),
                ("SegmentMapStorageID", map_id.into()),
                ("DataObjectId", 0.into()),
                ("StorageFolderID", prt_folder.into()),
                ("DeltaTableMetadataStorageID", 0.into()),
            ],
        );
        self.insert_segment_map(map_id, ps_id, &data.segment_map);
        for (position, col) in data.columns.iter().enumerate() {
            let column_id = self.alloc()?;
            let cs_id = self.alloc()?;
            let dict_id = self.alloc()?;
            let cps_id = self.alloc()?;
            let seg_id = self.alloc()?;
            self.insert(
                "Column",
                vec![
                    ("ID", column_id.into()),
                    ("TableID", table_id.into()),
                    ("ExplicitName", col.name.as_str().into()),
                    ("InferredName", MetaValue::Null),
                    ("ExplicitDataType", 6.into()),
                    ("InferredDataType", 19.into()),
                    ("DataCategory", MetaValue::Null),
                    ("Description", MetaValue::Null),
                    ("IsHidden", 0.into()),
                    ("State", 1.into()),
                    ("IsUnique", 0.into()),
                    ("IsKey", 0.into()),
                    ("IsNullable", 1.into()),
                    ("Alignment", 1.into()),
                    ("TableDetailPosition", (-1).into()),
                    ("IsDefaultLabel", 0.into()),
                    ("IsDefaultImage", 0.into()),
                    ("SummarizeBy", 1.into()),
                    ("ColumnStorageID", cs_id.into()),
                    ("Type", ct::DATA.into()),
                    ("SourceColumn", MetaValue::Null),
                    ("ColumnOriginID", 0.into()),
                    ("Expression", MetaValue::Null),
                    ("FormatString", MetaValue::Null),
                    ("IsAvailableInMDX", 1.into()),
                    ("SortByColumnID", 0.into()),
                    ("AttributeHierarchyID", 0.into()),
                    ("ModifiedTime", now.into()),
                    ("StructureModifiedTime", now.into()),
                    ("RefreshedTime", NEVER.into()),
                    ("SystemFlags", 1.into()),
                    ("KeepUniqueRows", 0.into()),
                    ("DisplayOrdinal", 0.into()),
                    ("ErrorMessage", MetaValue::Null),
                    ("SourceProviderType", MetaValue::Null),
                    ("DisplayFolder", MetaValue::Null),
                    ("EncodingHint", 0.into()),
                    ("RelatedColumnDetailsID", 0.into()),
                    ("AlternateOfID", 0.into()),
                    ("LineageTag", MetaValue::Null),
                    ("SourceLineageTag", MetaValue::Null),
                    ("ExpressionContext", 1.into()),
                    ("StringIndexingBehavior", 1.into()),
                    ("FullTextIndexingBehavior", 0.into()),
                ],
            );
            // System columns carry placeholder statistics (observed on every one of them).
            self.insert(
                "ColumnStorage",
                vec![
                    ("ID", cs_id.into()),
                    ("ColumnID", column_id.into()),
                    ("Name", col.name.as_str().into()),
                    ("StoragePosition", (position as i64).into()),
                    ("DictionaryStorageID", dict_id.into()),
                    (
                        "Settings",
                        col.settings
                            .unwrap_or_else(|| system_column_settings(&col.name))
                            .into(),
                    ),
                    ("ColumnFlags", 0.into()),
                    ("Collation", MetaValue::Null),
                    ("OrderByColumn", MetaValue::Null),
                    ("Locale", 0.into()),
                    (
                        "BinaryCharacters",
                        if kind == SystemKind::UserHierarchy {
                            10240
                        } else {
                            0
                        }
                        .into(),
                    ),
                    ("Statistics_DistinctStates", 1.into()),
                    ("Statistics_MinDataID", 2.into()),
                    ("Statistics_MaxDataID", 2.into()),
                    ("Statistics_OriginalMinSegmentDataID", 2.into()),
                    ("Statistics_RLESortOrder", (-1).into()),
                    ("Statistics_RowCount", 0.into()),
                    ("Statistics_HasNulls", 0.into()),
                    ("Statistics_RLERuns", 0.into()),
                    ("Statistics_OthersRLERuns", 0.into()),
                    ("Statistics_Usage", 3.into()),
                    ("Statistics_DBType", 0.into()),
                    ("Statistics_XMType", 0.into()),
                    ("Statistics_CompressionType", 0.into()),
                    ("Statistics_CompressionParam", 0.into()),
                    ("Statistics_EncodingHint", 0.into()),
                    ("IsDeltaPartitionColumn", 0.into()),
                    ("DeltaColumnMappingPhysicalName", MetaValue::Null),
                    ("DeltaColumnMappingId", (-1).into()),
                    ("FramedSourceColumn", MetaValue::Null),
                    ("DeltaColumnPrimitiveType", 0.into()),
                    ("DeltaTableColumnStorageID", 0.into()),
                    ("StringIndexStorageID", 0.into()),
                    ("ColumnIndexStorageID", 0.into()),
                    ("FullTextIndexStorageID", 0.into()),
                ],
            );
            self.insert(
                "DictionaryStorage",
                vec![
                    ("ID", dict_id.into()),
                    ("ColumnStorageID", cs_id.into()),
                    ("Type", 0.into()),
                    ("DataType", 19.into()),
                    ("DataVersion", 0.into()),
                    ("BaseId", 0.into()),
                    ("Magnitude", 0.0.into()),
                    ("LastId", 0.into()),
                    ("IsNullable", 0.into()),
                    ("IsUnique", 0.into()),
                    ("IsOperatingOn32", 0.into()),
                    ("DictionaryFlags", 0.into()),
                    ("StorageFileID", 0.into()),
                    ("Size", 0.into()),
                ],
            );
            let idf_file = self.new_file(
                cps_id,
                owner::COLUMN_PARTITION_STORAGE,
                prt_folder,
                &names::idf(0, name, &col.name, 0),
                col.idf.clone(),
            )?;
            self.insert(
                "ColumnPartitionStorage",
                vec![
                    ("ID", cps_id.into()),
                    ("ColumnStorageID", cs_id.into()),
                    ("PartitionStorageID", ps_id.into()),
                    ("DataVersion", 0.into()),
                    ("State", 3.into()),
                    ("SegmentStorageID", seg_id.into()),
                    ("StorageFileID", idf_file.into()),
                ],
            );
            let meta_file = self.new_file(
                seg_id,
                owner::SEGMENT_STORAGE,
                prt_folder,
                &names::idfmeta(0, name, &col.name, 0),
                col.idfmeta.clone(),
            )?;
            self.insert(
                "SegmentStorage",
                vec![
                    ("ID", seg_id.into()),
                    ("ColumnPartitionStorageID", cps_id.into()),
                    ("SegmentCount", col.segment_count.into()),
                    ("StorageFileID", meta_file.into()),
                ],
            );
        }
        Ok(table_id)
    }

    /// Adds a relationship between existing columns with its storage rows and `R$` table(s),
    /// and marks the "one" end(s) as unique. Many-to-many relationships get no storage (no
    /// index, `RelationshipStorageID` 0, `RefreshedTime` never), one-to-one ones two indexes
    /// (`RelationshipStorage2ID`), both ends unique (engine-observed). Returns the ID.
    pub(crate) fn add_relationship(
        &mut self,
        model_id: i64,
        r: &Relationship,
        ends: RelationshipEnds,
        enc: Option<&EncodedRelationship>,
    ) -> Result<i64, Error> {
        let now = self.now;
        let shape = RelationshipShape::of(r)
            .ok_or_else(|| Error::Invalid(format!("relationship `{}`: unsupported cardinality", r.name)))?;
        let enc = match (shape, enc) {
            (RelationshipShape::ManyToMany, _) => None,
            (_, Some(e)) => Some(e),
            (_, None) => {
                return Err(Error::Invalid(format!(
                    "relationship `{}`: no encoded index",
                    r.name
                )));
            }
        };
        let rel_id = self.alloc()?;
        let x = &r.extra;
        let code = |v: Option<&str>, f: fn(i64) -> Option<&'static str>, default: i64| {
            v.map_or(Some(default), |s| codes::code_of(f, s))
                .ok_or_else(|| Error::Invalid(format!("relationship `{}`: unknown code", r.name)))
        };
        let cross = code(r.cross_filtering_behavior.as_deref(), codes::cross_filtering, 1)?;
        let from_card = code(r.from_cardinality.as_deref(), codes::cardinality, 2)?;
        let to_card = code(r.to_cardinality.as_deref(), codes::cardinality, 1)?;
        let join = code(extra_str(x, "joinOnDateBehavior"), codes::join_on_date, 1)?;
        let security = code(
            extra_str(x, "securityFilteringBehavior"),
            codes::security_filtering,
            1,
        )?;
        let rs_name = names::object_storage_name(&r.name, rel_id);
        let table_storage = |st: &Self, table: i64| {
            st.reference("Table", table, "TableStorageID")
                .and_then(|ts| st.text("TableStorage", ts, "Name"))
                .ok_or_else(|| Error::Invalid(format!("relationship `{}`: no table storage", r.name)))
        };
        // (index, unmatched, table storage name of the side the index maps from)
        let mut indexes = Vec::new();
        if let Some(e) = enc {
            indexes.push((&e.index, e.unmatched, table_storage(self, ends.from_table)?));
            if shape == RelationshipShape::OneToOne {
                let (reverse, unmatched) = e
                    .reverse
                    .as_ref()
                    .ok_or_else(|| Error::Invalid(format!("relationship `{}`: no second index", r.name)))?;
                indexes.push((reverse, *unmatched, table_storage(self, ends.to_table)?));
            }
        }
        let mut storage_ids = Vec::with_capacity(indexes.len());
        for _ in &indexes {
            storage_ids.push((self.alloc()?, self.alloc()?));
        }
        let storage = |k: usize| storage_ids.get(k).map_or(0, |(rs, _)| *rs);
        let refreshed = if indexes.is_empty() { NEVER } else { now };
        self.insert(
            "Relationship",
            vec![
                ("ID", rel_id.into()),
                ("ModelID", model_id.into()),
                ("Name", r.name.as_str().into()),
                ("IsActive", r.is_active.into()),
                ("Type", 1.into()),
                ("CrossFilteringBehavior", cross.into()),
                ("JoinOnDateBehavior", join.into()),
                (
                    "RelyOnReferentialIntegrity",
                    extra_bool(x, "relyOnReferentialIntegrity", false).into(),
                ),
                ("FromTableID", ends.from_table.into()),
                ("FromColumnID", ends.from_column.into()),
                ("FromCardinality", from_card.into()),
                ("ToTableID", ends.to_table.into()),
                ("ToColumnID", ends.to_column.into()),
                ("ToCardinality", to_card.into()),
                ("State", 1.into()),
                ("RelationshipStorageID", storage(0).into()),
                ("RelationshipStorage2ID", storage(1).into()),
                ("ModifiedTime", now.into()),
                ("RefreshedTime", refreshed.into()),
                ("SecurityFilteringBehavior", security.into()),
            ],
        );
        for ((index, unmatched, from_ts), &(rs_id, ris_id)) in indexes.iter().zip(&storage_ids) {
            self.insert(
                "RelationshipStorage",
                vec![
                    ("ID", rs_id.into()),
                    ("RelationshipID", rel_id.into()),
                    ("Name", rs_name.as_str().into()),
                    ("DefinitionType", 0.into()),
                    ("Cardinality", 0.into()),
                    ("Flags", 0.into()),
                    ("RelationshipIndexStorageID", ris_id.into()),
                ],
            );
            let sys = self.add_system_table(
                SystemKind::Relationship,
                &names::relationship_table(from_ts, &rs_name),
                index,
            )?;
            self.insert(
                "RelationshipIndexStorage",
                vec![
                    ("ID", ris_id.into()),
                    ("RelationshipStorageID", rs_id.into()),
                    ("IndexType", 1.into()),
                    ("Flags", i64::from(*unmatched).into()),
                    ("RecordCount", index.segment_map.record_count.into()),
                    ("SecondaryRecordCount", 0.into()),
                    ("StorageFolderID", 0.into()),
                    ("StorageFileID", 0.into()),
                    ("SystemTableID", sys.into()),
                    ("SecondarySystemTableID", 0.into()),
                ],
            );
        }
        for a in &r.annotations {
            let id = self.alloc()?;
            self.insert(
                "Annotation",
                vec![
                    ("ID", id.into()),
                    ("ObjectID", rel_id.into()),
                    ("ObjectType", ot::RELATIONSHIP.into()),
                    ("Name", a.name.as_str().into()),
                    ("Value", a.value.as_str().into()),
                    ("ModifiedTime", now.into()),
                ],
            );
        }
        match shape {
            RelationshipShape::ManyToOne => self.mark_unique_end(ends.to_column),
            RelationshipShape::OneToOne => {
                self.mark_unique_end(ends.from_column);
                self.mark_unique_end(ends.to_column);
            }
            RelationshipShape::ManyToMany => {}
        }
        Ok(rel_id)
    }

    /// The storage of the "one" end of a relationship: `ColumnFlags` unique bits, usage 0,
    /// encoding hint 1, unique dictionary (observed on every such column).
    fn mark_unique_end(&mut self, column_id: i64) {
        let Some(cs) = self.reference("Column", column_id, "ColumnStorageID") else {
            return;
        };
        let flags = self.int("ColumnStorage", cs, "ColumnFlags").unwrap_or(0) | flags::UNIQUE;
        self.set("ColumnStorage", cs, "ColumnFlags", flags.into());
        self.set("ColumnStorage", cs, "Statistics_Usage", 0.into());
        self.set("ColumnStorage", cs, "Statistics_EncodingHint", 1.into());
        if let Some(d) = self.reference("ColumnStorage", cs, "DictionaryStorageID") {
            self.set("DictionaryStorage", d, "IsUnique", 1.into());
        }
    }
}

/// The cardinalities a structural save writes (TMSL defaults: from many, to one).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RelationshipShape {
    ManyToOne,
    OneToOne,
    ManyToMany,
}

impl RelationshipShape {
    pub(crate) fn of(r: &Relationship) -> Option<Self> {
        let from = r.from_cardinality.as_deref().unwrap_or("many");
        let to = r.to_cardinality.as_deref().unwrap_or("one");
        let many = |s: &str| s.eq_ignore_ascii_case("many");
        let one = |s: &str| s.eq_ignore_ascii_case("one");
        if many(from) && one(to) {
            Some(Self::ManyToOne)
        } else if one(from) && one(to) {
            Some(Self::OneToOne)
        } else if many(from) && many(to) {
            Some(Self::ManyToMany)
        } else {
            None
        }
    }
}

/// Resolved ends of a new relationship (table and column IDs).
#[derive(Debug, Clone, Copy)]
pub(crate) struct RelationshipEnds {
    pub from_table: i64,
    pub from_column: i64,
    pub to_table: i64,
    pub to_column: i64,
}

/// The row-number column of a model table, as `build` reads it back.
pub(crate) fn is_row_number(c: &Column) -> bool {
    c.extra.get("type").and_then(Value::as_str) == Some("rowNumber")
}

/// Whether `data` can be stored as the `H$`/hash shape the engine expects for a relationship's
/// "one" end: a hash dictionary.
pub(crate) fn is_hash_encoded(data: &ColumnStorageData) -> bool {
    data.dictionary.kind == 1
}
