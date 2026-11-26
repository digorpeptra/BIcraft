//! The VertiPaq storage of a table: its `<dimension id>.<n>.tbl.xml` (an `XMObject` tree,
//! [MS-XLDM] §2.5) gives each column's segments, dictionary and `.idf` files; the segment
//! properties are those of a Power BI `.idfmeta`, so `bicraft-vertipaq` decodes the columns
//! (`docs/formats/powerpivot.md`, "Storage").

use bicraft_model::DataType;
use bicraft_vertipaq::compression::{HYBRID_RLE, NO_SPLIT_BASE, NONE, XM123, no_split_width};
use bicraft_vertipaq::idfmeta::{ColumnMeta, RleFacet, SegmentMeta, SegmentStats, SubSegmentMeta};
use bicraft_vertipaq::{CellValue, ColumnEncoding, LogicalType, StorageType, decode_column_with_limits};
use roxmltree::Node;

use crate::backup::Backup;
use crate::xml::{TableSource, child, children, object_id, parse, text};
use crate::{Error, TableRows};

/// Most segments of one column, and most partitions of one table.
const MAX_SEGMENTS: usize = 1 << 20;

fn bad(message: impl Into<String>) -> Error {
    Error::Metadata(message.into())
}

/// `Members/Member[Name = name]/XMObject`.
fn member<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    children(child(node, "Members")?, "Member")
        .find(|m| text(*m, "Name").as_deref() == Some(name))
        .and_then(|m| child(m, "XMObject"))
}

/// The `XMObject`s of `Collections/Collection[Name = name]`.
fn collection<'a, 'i: 'a>(node: Node<'a, 'i>, name: &'a str) -> Vec<Node<'a, 'i>> {
    child(node, "Collections")
        .into_iter()
        .flat_map(move |c| children(c, "Collection"))
        .filter(move |c| text(*c, "Name").as_deref() == Some(name))
        .flat_map(|c| children(c, "XMObject").collect::<Vec<_>>())
        .collect()
}

fn prop<T: std::str::FromStr>(node: Option<Node<'_, '_>>, name: &str) -> Option<T> {
    text(child(node?, "Properties")?, name)?.parse().ok()
}

fn prop_bool(node: Option<Node<'_, '_>>, name: &str) -> Option<bool> {
    text(child(node?, "Properties")?, name).map(|v| v.eq_ignore_ascii_case("true") || v == "1")
}

fn class<'a>(node: Node<'a, '_>) -> &'a str {
    node.attribute("class").unwrap_or_default().trim()
}

/// The `.idfmeta` class code of a compression class name (`XMRENoSplitCompressionInfo<6>`,
/// `XM123CompressionInfo`, `XMHybridRLECompressionInfo<…>`).
fn class_code(name: &str) -> Option<u32> {
    let name = name.trim();
    let name = name.strip_prefix("class ").unwrap_or(name).trim();
    if name.starts_with("XMHybridRLECompressionInfo") {
        Some(HYBRID_RLE)
    } else if let Some(rest) = name.strip_prefix("XMRENoSplitCompressionInfo<") {
        let width: u32 = rest.split('>').next()?.trim().parse().ok()?;
        let code = NO_SPLIT_BASE.checked_add(width)?;
        no_split_width(code).map(|_| code)
    } else if name.starts_with("XM123CompressionInfo") {
        Some(XM123)
    } else {
        None
    }
}

/// The inner compression of a hybrid segment: its `SubCompression` member, else the class
/// template argument (`XMHybridRLECompressionInfo<class XMRENoSplitCompressionInfo<6>>`).
fn sub_class_code(info: Node<'_, '_>) -> Option<u32> {
    if let Some(sub) = member(info, "SubCompression") {
        return class_code(class(sub));
    }
    let inner = class(info).strip_prefix("XMHybridRLECompressionInfo<")?;
    let inner = inner.strip_suffix('>').unwrap_or(inner);
    class_code(inner)
}

fn segment(node: Node<'_, '_>) -> Result<SegmentMeta, Error> {
    let info =
        member(node, "CompressionInfo").ok_or_else(|| bad("a column segment has no CompressionInfo"))?;
    let compression_class = class_code(class(info))
        .ok_or_else(|| bad(format!("unsupported compression class {}", class(info))))?;
    let (rle, sub_compression_class) = if compression_class == HYBRID_RLE {
        let rle = member(info, "RLECompression");
        let sub = sub_class_code(info)
            .ok_or_else(|| bad(format!("unsupported inner compression of {}", class(info))))?;
        (
            Some(RleFacet {
                bookmark_bits: prop(rle, "BookmarkBits").unwrap_or(0),
                storage_alloc_size: prop(rle, "StorageAllocSize").unwrap_or(0),
                storage_used_size: prop(rle, "StorageUsedSize").unwrap_or(0),
                segment_needs_resizing: u8::from(prop_bool(rle, "SegmentNeedsResizing").unwrap_or(false)),
            }),
            sub,
        )
    } else {
        (None, NONE)
    };
    let stats = member(node, "ColumnSegmentStats");
    Ok(SegmentMeta {
        records: prop(Some(node), "Records").ok_or_else(|| bad("a column segment has no Records"))?,
        base_id: 0,
        compression_class,
        sub_compression_class,
        rle,
        first_run_value: 0,
        stats: SegmentStats {
            distinct_states: prop(stats, "DistinctStates").unwrap_or(0),
            min_data_id: prop(stats, "MinDataID").unwrap_or(0),
            max_data_id: prop(stats, "MaxDataID").unwrap_or(0),
            original_min_segment_data_id: prop(stats, "OriginalMinSegmentDataID").unwrap_or(0),
            rle_sort_order: prop(stats, "RLESortOrder").unwrap_or(-1),
            row_count: prop(stats, "RowCount").unwrap_or(0),
            has_nulls: u8::from(prop_bool(stats, "HasNulls").unwrap_or(false)),
            rle_runs: prop(stats, "RLERuns").unwrap_or(0),
            others_rle_runs: prop(stats, "OthersRLERuns").unwrap_or(0),
        },
        sub_segment: member(node, "SubSegment").map(|s| SubSegmentMeta {
            records: prop(Some(s), "Records").unwrap_or(0),
            base_id: 0,
        }),
    })
}

/// How a column's data ids become values.
enum Dict {
    /// `XMHashDataDictionary<…>`: its `.dictionary` file (none for a column of blanks).
    Hash {
        file: Option<String>,
        storage: StorageType,
    },
    /// `XMValueDataDictionary<…>`: value = (id + `BaseId`) / `Magnitude`.
    Value {
        base_id: i64,
        magnitude: f64,
        storage: StorageType,
    },
}

fn storage_type(class: &str) -> Option<StorageType> {
    if class.contains("XM_String") {
        Some(StorageType::String)
    } else if class.contains("XM_Long") {
        Some(StorageType::Long)
    } else if class.contains("XM_Real") {
        Some(StorageType::Real)
    } else {
        None
    }
}

fn logical(t: DataType) -> Option<LogicalType> {
    Some(match t {
        DataType::String => LogicalType::String,
        DataType::Int64 => LogicalType::Int64,
        DataType::Double => LogicalType::Double,
        DataType::Decimal => LogicalType::Decimal,
        DataType::DateTime => LogicalType::DateTime,
        DataType::Boolean => LogicalType::Boolean,
        DataType::Binary => LogicalType::Binary,
        DataType::Unknown | DataType::Variant => return None,
    })
}

/// The values of one column from its `XMRawColumn` object.
fn column(
    backup: &Backup<'_>,
    node: Node<'_, '_>,
    data_type: DataType,
) -> Result<(Vec<CellValue>, StorageType), Error> {
    let segment_nodes = collection(node, "Segments");
    if segment_nodes.len() > MAX_SEGMENTS {
        return Err(Error::Limit(format!(
            "{} segments in a column",
            segment_nodes.len()
        )));
    }
    let mut segments = segment_nodes
        .into_iter()
        .map(segment)
        .collect::<Result<Vec<_>, _>>()?;
    let mut dict = None;
    let mut partitions = Vec::new();
    let objects = child(node, "DataObjects")
        .into_iter()
        .flat_map(|d| children(d, "DataObject"))
        .filter_map(|d| child(d, "XMObject"));
    for object in objects {
        let c = class(object);
        if c.starts_with("XMRawColumnPartitionDataObject") {
            let count = prop::<usize>(Some(object), "SegmentCount")
                .unwrap_or(segments.len())
                .min(segments.len());
            let taken: Vec<SegmentMeta> = segments.drain(..count).collect();
            let idf = object
                .attribute("name")
                .ok_or_else(|| bad("a column partition names no .idf file"))?;
            partitions.push((idf.to_owned(), taken));
            if partitions.len() > MAX_SEGMENTS {
                return Err(Error::Limit("too many column partitions".into()));
            }
        } else if c.starts_with("XMHashDataDictionary") {
            let storage = storage_type(c).ok_or_else(|| bad(format!("unsupported dictionary class {c}")))?;
            dict = Some(Dict::Hash {
                file: object.attribute("name").map(str::to_owned),
                storage,
            });
        } else if c.starts_with("XMValueDataDictionary") {
            let storage = storage_type(c).ok_or_else(|| bad(format!("unsupported dictionary class {c}")))?;
            dict = Some(Dict::Value {
                base_id: prop(Some(object), "BaseId").unwrap_or(0),
                magnitude: prop(Some(object), "Magnitude").unwrap_or(1.0),
                storage,
            });
        }
    }
    let dict = dict.ok_or_else(|| bad("a column has no dictionary"))?;
    let (encoding, dictionary, storage) = match &dict {
        Dict::Hash { file, storage } => {
            let bytes = match file {
                Some(name) => {
                    let entry = backup
                        .by_name(name)
                        .ok_or_else(|| bad("a column's .dictionary file is missing"))?;
                    Some(backup.read(entry)?)
                }
                None => None,
            };
            (ColumnEncoding::hash(*storage), bytes, *storage)
        }
        Dict::Value {
            base_id,
            magnitude,
            storage,
        } => (
            ColumnEncoding::value(*base_id, *magnitude, *storage),
            None,
            *storage,
        ),
    };
    let logical = logical(data_type).or(match storage {
        StorageType::String => Some(LogicalType::String),
        StorageType::Long => Some(LogicalType::Int64),
        StorageType::Real => Some(LogicalType::Double),
        StorageType::None => None,
    });
    let encoding = encoding.with_logical_type(logical);
    let mut values = Vec::new();
    for (idf_name, segments) in partitions {
        let entry = backup
            .by_name(&idf_name)
            .ok_or_else(|| bad("a column's .idf file is missing"))?;
        let idf = backup.read(entry)?;
        let meta = ColumnMeta {
            segments,
            data_objects: None,
        }
        .to_bytes();
        let cells = decode_column_with_limits(
            &idf,
            &meta,
            dictionary.as_deref(),
            encoding,
            &backup.limits.vertipaq,
        )
        .map_err(|e| match e {
            bicraft_vertipaq::Error::LimitExceeded(m) => Error::Limit(m),
            e => bad(e.to_string()),
        })?;
        if values.is_empty() {
            values = cells;
        } else {
            values.extend(cells);
        }
    }
    Ok((values, storage))
}

/// The rows of a table, and the type each calculated column gets from its storage.
pub(crate) fn read_table(
    backup: &Backup<'_>,
    source: &TableSource,
    cells_left: &mut u64,
) -> Result<(TableRows, Vec<(String, DataType)>), Error> {
    let prefix = format!("{}.", source.dimension_id);
    let file = backup
        .xml_files("tbl")
        .into_iter()
        .find(|f| {
            f.name().starts_with(&prefix)
                && object_id(f.name(), "tbl").as_deref() == Some(source.dimension_id.as_str())
        })
        .ok_or_else(|| bad("no table storage (.tbl.xml)"))?;
    let body = backup.read_text(file)?;
    let doc = parse(&body, backup, "table storage")?;
    let root = doc
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "XMObject")
        .ok_or_else(|| bad("the table storage has no XMObject"))?;
    let stored = collection(root, "Columns");
    let mut columns = Vec::with_capacity(source.columns.len());
    let mut inferred = Vec::new();
    let mut rows = None;
    for c in &source.columns {
        let node = stored
            .iter()
            .find(|n| n.attribute("name") == Some(c.attribute_id.as_str()))
            .ok_or_else(|| bad(format!("column '{}' has no storage", c.name)))?;
        let (values, storage) = column(backup, *node, c.data_type)?;
        let n = values.len() as u64;
        *cells_left = cells_left.checked_sub(n).ok_or_else(|| {
            Error::Limit(format!(
                "more than {} cells in the model",
                backup.limits.max_cells
            ))
        })?;
        match rows {
            None => rows = Some(values.len()),
            Some(r) if r != values.len() => {
                return Err(bad(format!(
                    "column '{}' has {} rows, the table's other columns {r}",
                    c.name,
                    values.len()
                )));
            }
            Some(_) => {}
        }
        if c.data_type == DataType::Unknown {
            let t = match storage {
                StorageType::String => DataType::String,
                StorageType::Long => DataType::Int64,
                StorageType::Real => DataType::Double,
                StorageType::None => DataType::Unknown,
            };
            inferred.push((c.name.clone(), t));
        }
        columns.push((c.name.clone(), values));
    }
    Ok((
        TableRows {
            name: source.name.clone(),
            columns,
        },
        inferred,
    ))
}
