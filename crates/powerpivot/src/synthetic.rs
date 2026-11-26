//! Synthetic Power Pivot workbooks for tests (BIcraft's own; BIcraft never writes workbooks for
//! users). [`workbook`] lays a model out the way `docs/formats/powerpivot.md` describes a
//! workbook Excel saves: ASSL `Load` documents for the database, cube, dimensions, partitions
//! and MDX script, an `XMObject` storage tree per table, VertiPaq files from
//! `bicraft-vertipaq`'s encoder, every file compressed with `bicraft-xpress8`'s encoder and
//! followed by a CRC marker, in an ABF with `ErrorCode` and `ApplyCompression` set, stored as
//! `xl/model/item.data` of a minimal SpreadsheetML package. It carries only what the reader
//! uses; it has not been opened in Excel.

use bicraft_model::DataType;
use bicraft_opc::{Package, Part};
use bicraft_vertipaq::compression::{Compression, InnerCompression};
use bicraft_vertipaq::encode::EncodedColumn;
use bicraft_vertipaq::idfmeta::{SegmentMeta, parse_idfmeta};
use bicraft_vertipaq::{
    CellValue, EncodeOptions, EncodingKind, LogicalType, StorageType, encode_column, encode_row_number,
};

use crate::xml::data_type;

/// A column of a synthetic table.
#[derive(Debug, Clone)]
pub struct SynthColumn {
    pub name: String,
    /// The ASSL key column `DataType` (`WChar`, `BigInt`, `Double`, `Currency`, `Date`,
    /// `Boolean`, or `Empty` for a calculated column).
    pub assl_type: &'static str,
    /// The column binding's `ColumnID` (XML-encoded source column), default the name.
    pub source_id: Option<String>,
    /// A calculated column's DAX.
    pub expression: Option<String>,
    pub hidden: bool,
    pub values: Vec<CellValue>,
}

#[derive(Debug, Clone)]
pub struct SynthTable {
    pub name: String,
    pub columns: Vec<SynthColumn>,
    /// User hierarchies: (name, level columns).
    pub hierarchies: Vec<(String, Vec<String>)>,
    /// The partition's source query.
    pub query: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct SynthModel {
    pub tables: Vec<SynthTable>,
    /// (table, name, DAX).
    pub measures: Vec<(String, String, String)>,
    /// (from table, from column, to table, to column): many-to-one.
    pub relationships: Vec<(String, String, String, String)>,
    pub compatibility_level: u32,
}

fn col(name: &str, assl_type: &'static str, values: Vec<CellValue>) -> SynthColumn {
    SynthColumn {
        name: name.into(),
        assl_type,
        source_id: None,
        expression: None,
        hidden: false,
        values,
    }
}

/// The model of the committed fixture `crates/powerpivot/tests/fixtures/sales.xlsx`: a product table
/// and a sales table of 240 rows (blanks, a calculated column, two measures, a relationship, a
/// hierarchy, a hidden column, an XML-encoded source column name).
pub fn sample_model() -> SynthModel {
    use CellValue::*;
    let names = ["Chair", "Desk", "Lamp", "Shelf", "Stool"];
    let categories = ["Seating", "Tables", "Lighting", "Storage", "Seating"];
    let prices = [450_000i64, 1_299_900, 249_500, 899_000, 199_900];
    let mut product = SynthTable {
        name: "Product".into(),
        columns: vec![
            col("ProductKey", "BigInt", (1..=5).map(Int).collect()),
            col("Name", "WChar", names.iter().map(|n| Text((*n).into())).collect()),
            col(
                "Category",
                "WChar",
                categories.iter().map(|n| Text((*n).into())).collect(),
            ),
            SynthColumn {
                source_id: Some("Unit_x0020_price".into()),
                ..col(
                    "Unit price",
                    "Currency",
                    prices.iter().map(|p| Decimal(*p)).collect(),
                )
            },
        ],
        hierarchies: vec![("Catalogue".into(), vec!["Category".into(), "Name".into()])],
        query: Some("SELECT [ProductKey], [Name], [Category], [Unit price]\nFROM [Product]".into()),
    };
    if let Some(c) = product.columns.first_mut() {
        c.hidden = true;
    }
    let rows = 240usize;
    let key = |i: usize| (i * 7 + i / 11) % 5 + 1;
    let quantity = |i: usize| ((i * 13) % 9 + 1) as i64;
    let sales = SynthTable {
        name: "Sales".into(),
        columns: vec![
            col(
                "ProductKey",
                "BigInt",
                (0..rows).map(|i| Int(key(i) as i64)).collect(),
            ),
            col(
                "Order date",
                "Date",
                (0..rows).map(|i| DateTime(45_292.0 + (i / 8) as f64)).collect(),
            ),
            col(
                "Quantity",
                "BigInt",
                (0..rows).map(|i| Int(quantity(i))).collect(),
            ),
            col(
                "Amount",
                "Double",
                (0..rows)
                    .map(|i| {
                        if i % 37 == 5 {
                            Null
                        } else {
                            Real(quantity(i) as f64 * 12.5 + (i % 4) as f64 * 0.25)
                        }
                    })
                    .collect(),
            ),
            col(
                "Returned",
                "Boolean",
                (0..rows)
                    .map(|i| if i % 50 == 3 { Null } else { Bool(i % 9 == 0) })
                    .collect(),
            ),
            col(
                "Channel",
                "WChar",
                (0..rows)
                    .map(|i| {
                        if i % 29 == 7 {
                            Null
                        } else {
                            Text(["Store", "Web", "Phone"][i % 3].into())
                        }
                    })
                    .collect(),
            ),
            SynthColumn {
                expression: Some("Sales[Quantity] * RELATED(Product[Unit price])".into()),
                ..col(
                    "Line total",
                    "Empty",
                    (0..rows)
                        .map(|i| {
                            Real(
                                quantity(i) as f64 * prices.get(key(i) - 1).copied().unwrap_or(0) as f64
                                    / 10_000.0,
                            )
                        })
                        .collect(),
                )
            },
        ],
        hierarchies: Vec::new(),
        query: Some("SELECT * FROM [Sales]".into()),
    };
    SynthModel {
        tables: vec![product, sales],
        measures: vec![
            ("Sales".into(), "Total amount".into(), "SUM(Sales[Amount])".into()),
            ("Sales".into(), "Units".into(), "SUM('Sales'[Quantity])".into()),
        ],
        relationships: vec![(
            "Sales".into(),
            "ProductKey".into(),
            "Product".into(),
            "ProductKey".into(),
        )],
        compatibility_level: 1103,
    }
}

/// The committed fixture's bytes.
pub fn sample() -> Result<Vec<u8>, String> {
    workbook(&sample_model())
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

const NS: &str = r#"xmlns="http://schemas.microsoft.com/analysisservices/2003/engine" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance""#;

fn load(parent: &str, object: &str) -> String {
    format!(
        "<Load {NS}><ParentObject>{parent}</ParentObject><ObjectDefinition>{object}</ObjectDefinition></Load>"
    )
}

fn logical_of(assl: &str, values: &[CellValue]) -> LogicalType {
    match data_type(Some(assl)) {
        DataType::String => LogicalType::String,
        DataType::Int64 => LogicalType::Int64,
        DataType::Decimal => LogicalType::Decimal,
        DataType::DateTime => LogicalType::DateTime,
        DataType::Boolean => LogicalType::Boolean,
        DataType::Binary => LogicalType::Binary,
        DataType::Double => LogicalType::Double,
        _ => match values.iter().find(|v| !matches!(v, CellValue::Null)) {
            Some(CellValue::Text(_)) => LogicalType::String,
            Some(CellValue::Int(_)) => LogicalType::Int64,
            _ => LogicalType::Double,
        },
    }
}

fn props(pairs: &[(&str, String)]) -> String {
    let mut s = String::from("<Properties>");
    for (k, v) in pairs {
        s.push_str(&format!("<{k}>{v}</{k}>"));
    }
    s.push_str("</Properties>");
    s
}

fn member(name: &str, object: &str) -> String {
    format!("<Member><Name>{name}</Name>{object}</Member>")
}

fn compression_name(c: Compression) -> String {
    let no_split = |n: u8| format!("XMRENoSplitCompressionInfo<{n}>");
    match c {
        Compression::NoSplit(n) => no_split(n),
        Compression::Xm123 => "XM123CompressionInfo".into(),
        Compression::HybridRle(InnerCompression::NoSplit(n)) => {
            format!("XMHybridRLECompressionInfo<class {}>", no_split(n))
        }
        Compression::HybridRle(InnerCompression::Xm123) => {
            "XMHybridRLECompressionInfo<class XM123CompressionInfo>".into()
        }
    }
}

fn segment_xml(s: &SegmentMeta) -> Result<String, String> {
    let c = s.compression().ok_or("unknown compression")?;
    let mut info_members = String::new();
    if let (Compression::HybridRle(inner), Some(rle)) = (c, s.rle) {
        info_members.push_str(&member(
            "RLECompression",
            &format!(
                "<XMObject>{}</XMObject>",
                props(&[
                    ("BookmarkBits", rle.bookmark_bits.to_string()),
                    ("StorageAllocSize", rle.storage_alloc_size.to_string()),
                    ("StorageUsedSize", rle.storage_used_size.to_string()),
                    (
                        "SegmentNeedsResizing",
                        (rle.segment_needs_resizing != 0).to_string()
                    ),
                ])
            ),
        ));
        let inner = match inner {
            InnerCompression::NoSplit(n) => Compression::NoSplit(n),
            InnerCompression::Xm123 => Compression::Xm123,
        };
        info_members.push_str(&member(
            "SubCompression",
            &format!("<XMObject class=\"{}\"/>", esc(&compression_name(inner))),
        ));
    }
    let info = format!(
        "<XMObject class=\"{}\"><Members>{info_members}</Members></XMObject>",
        esc(&compression_name(c))
    );
    let st = &s.stats;
    let stats = format!(
        "<XMObject>{}</XMObject>",
        props(&[
            ("DistinctStates", st.distinct_states.to_string()),
            ("MinDataID", st.min_data_id.to_string()),
            ("MaxDataID", st.max_data_id.to_string()),
            (
                "OriginalMinSegmentDataID",
                st.original_min_segment_data_id.to_string()
            ),
            ("RLESortOrder", st.rle_sort_order.to_string()),
            ("RowCount", st.row_count.to_string()),
            ("HasNulls", (st.has_nulls != 0).to_string()),
            ("RLERuns", st.rle_runs.to_string()),
            ("OthersRLERuns", st.others_rle_runs.to_string()),
        ])
    );
    let mut members = member("CompressionInfo", &info);
    if let Some(sub) = s.sub_segment {
        members.push_str(&member(
            "SubSegment",
            &format!(
                "<XMObject class=\"XMColumnSegment\">{}</XMObject>",
                props(&[("Records", sub.records.to_string())])
            ),
        ));
    }
    members.push_str(&member("ColumnSegmentStats", &stats));
    Ok(format!(
        "<XMObject class=\"XMColumnSegment\">{}<Members>{members}</Members></XMObject>",
        props(&[("Records", s.records.to_string())])
    ))
}

fn xm_type(t: StorageType) -> &'static str {
    match t {
        StorageType::String => "XM_String",
        StorageType::Real => "XM_Real",
        _ => "XM_Long",
    }
}

/// The `XMRawColumn` of an encoded column; its files are named `<prefix>.idf` and
/// `<prefix>.dictionary`.
fn raw_column_xml(attribute_id: &str, enc: &EncodedColumn, prefix: &str) -> Result<String, String> {
    let meta = parse_idfmeta(&enc.idfmeta).map_err(|e| e.to_string())?;
    let mut segments = String::new();
    for s in &meta.segments {
        segments.push_str(&segment_xml(s)?);
    }
    let st = &enc.stats;
    let dictionary = match st.kind {
        EncodingKind::Hash => format!(
            "<DataObject><XMObject class=\"{}\"{}>{}</XMObject></DataObject>",
            esc(&format!("XMHashDataDictionary<{}>", xm_type(st.storage_type))),
            if enc.dictionary.is_some() {
                format!(" name=\"{}\"", esc(&format!("{prefix}.dictionary")))
            } else {
                String::new()
            },
            props(&[
                ("LastId", st.last_id.to_string()),
                ("Nullable", st.is_nullable.to_string()),
                ("Unique", st.is_unique.to_string()),
                ("OperatingOn32", st.is_operating_on_32.to_string()),
                ("DictionaryFlags", st.dictionary_flags.to_string()),
            ])
        ),
        _ => format!(
            "<DataObject><XMObject class=\"{}\">{}</XMObject></DataObject>",
            esc(&format!("XMValueDataDictionary<{}>", xm_type(st.storage_type))),
            props(&[
                ("BaseId", st.base_id.to_string()),
                ("Magnitude", st.magnitude.to_string())
            ])
        ),
    };
    let partition = format!(
        "<DataObject><XMObject class=\"XMRawColumnPartitionDataObject\" name=\"{}\">{}</XMObject></DataObject>",
        esc(&format!("{prefix}.0.idf")),
        props(&[
            ("Partition", "0".into()),
            ("SegmentCount", meta.segments.len().to_string())
        ])
    );
    let stats = format!(
        "<XMObject>{}</XMObject>",
        props(&[
            ("DistinctStates", st.distinct_states.to_string()),
            ("MinDataID", st.min_data_id.to_string()),
            ("MaxDataID", st.max_data_id.to_string()),
            ("RowCount", st.rows.to_string()),
            ("HasNulls", st.has_nulls.to_string()),
        ])
    );
    Ok(format!(
        "<XMObject class=\"XMRawColumn\" name=\"{}\"><Members>{}</Members><Collections><Collection><Name>Segments</Name>{segments}</Collection></Collections><DataObjects>{dictionary}{partition}</DataObjects></XMObject>",
        esc(attribute_id),
        member("ColumnStats", &stats)
    ))
}

/// The files of the backup: (name, content).
fn model_files(model: &SynthModel) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let db_parent = "<DatabaseID>Workbook</DatabaseID>";
    let cube_parent = format!("{db_parent}<CubeID>Model</CubeID>");
    let mut dims = String::new();
    for t in &model.tables {
        dims.push_str(&format!(
            "<Dimension><ID>{0}</ID><Name>{0}</Name><DimensionID>{0}</DimensionID><Visible>true</Visible></Dimension>",
            esc(&t.name)
        ));
    }
    files.push((
        "Model.1.cub.xml".into(),
        load(
            db_parent,
            &format!("<Cube><ID>Model</ID><Name>Model</Name><Dimensions>{dims}</Dimensions></Cube>"),
        )
        .into_bytes(),
    ));
    let options = EncodeOptions {
        build_hidx: false,
        ..EncodeOptions::default()
    };
    for t in &model.tables {
        let rows = t.columns.first().map_or(0, |c| c.values.len());
        // dimension: attributes, hierarchies, relationships from this table
        let mut attrs = String::from(
            "<Attribute><ID>RowNumber 1</ID><Name>RowNumber</Name><Type>RowNumber</Type><KeyColumns><KeyColumn><DataType>Integer</DataType><Source xsi:type=\"RowNumberBinding\"/></KeyColumn></KeyColumns><AttributeHierarchyVisible>false</AttributeHierarchyVisible></Attribute>",
        );
        for c in &t.columns {
            let source = match &c.expression {
                Some(e) => format!(
                    "<Source xsi:type=\"ExpressionBinding\"><Expression>{}</Expression></Source>",
                    esc(e)
                ),
                None => format!(
                    "<Source xsi:type=\"ColumnBinding\"><TableID>{}</TableID><ColumnID>{}</ColumnID></Source>",
                    esc(&t.name),
                    esc(c.source_id.as_deref().unwrap_or(&c.name))
                ),
            };
            attrs.push_str(&format!(
                "<Attribute><ID>{0}</ID><Name>{0}</Name><KeyColumns><KeyColumn><DataType>{1}</DataType>{source}</KeyColumn></KeyColumns><AttributeHierarchyVisible>{2}</AttributeHierarchyVisible></Attribute>",
                esc(&c.name),
                c.assl_type,
                !c.hidden
            ));
        }
        let mut hierarchies = String::new();
        for (name, levels) in &t.hierarchies {
            let levels: String = levels
                .iter()
                .map(|l| format!("<Level><ID>{0}</ID><Name>{0}</Name><SourceAttributeID>{0}</SourceAttributeID></Level>", esc(l)))
                .collect();
            hierarchies.push_str(&format!(
                "<Hierarchy><ID>{0}</ID><Name>{0}</Name><Levels>{levels}</Levels></Hierarchy>",
                esc(name)
            ));
        }
        let mut relationships = String::new();
        for (i, (ft, fc, tt, tc)) in model
            .relationships
            .iter()
            .enumerate()
            .filter(|(_, r)| r.0 == t.name)
        {
            relationships.push_str(&format!(
                "<Relationship><ID>Relationship{i}</ID><Visible>true</Visible><FromRelationshipEnd><DimensionID>{}</DimensionID><Multiplicity>Many</Multiplicity><Attributes><Attribute><AttributeID>{}</AttributeID></Attribute></Attributes></FromRelationshipEnd><ToRelationshipEnd><DimensionID>{}</DimensionID><Multiplicity>One</Multiplicity><Attributes><Attribute><AttributeID>{}</AttributeID></Attribute></Attributes></ToRelationshipEnd></Relationship>",
                esc(ft),
                esc(fc),
                esc(tt),
                esc(tc)
            ));
        }
        let dim = format!(
            "<Dimension><ID>{0}</ID><Name>{0}</Name><Attributes>{attrs}</Attributes><Hierarchies>{hierarchies}</Hierarchies><Relationships>{relationships}</Relationships></Dimension>",
            esc(&t.name)
        );
        files.push((
            format!("{}.1.dim.xml", t.name),
            load(db_parent, &dim).into_bytes(),
        ));
        if let Some(q) = &t.query {
            let partition = format!(
                "<Partition><ID>{0}</ID><Name>{0}</Name><Source xsi:type=\"QueryBinding\"><DataSourceID>Source</DataSourceID><QueryDefinition>{1}</QueryDefinition></Source></Partition>",
                esc(&t.name),
                esc(q)
            );
            files.push((
                format!("{}.1.prt.xml", t.name),
                load(
                    &format!("{cube_parent}<MeasureGroupID>{}</MeasureGroupID>", esc(&t.name)),
                    &partition,
                )
                .into_bytes(),
            ));
        }
        // storage
        let mut columns = String::new();
        let row_number = encode_row_number(rows as u64, &options).map_err(|e| e.to_string())?;
        let prefix = format!("1.{}.RowNumber", t.name);
        columns.push_str(&raw_column_xml("RowNumber 1", &row_number, &prefix)?);
        files.push((format!("{prefix}.0.idf"), row_number.idf.clone()));
        for c in &t.columns {
            if c.values.len() != rows {
                return Err(format!(
                    "column '{}' has {} rows, not {rows}",
                    c.name,
                    c.values.len()
                ));
            }
            let enc = encode_column(&c.values, logical_of(c.assl_type, &c.values), &options)
                .map_err(|e| e.to_string())?;
            let prefix = format!("1.{}.{}", t.name, c.name);
            columns.push_str(&raw_column_xml(&c.name, &enc, &prefix)?);
            files.push((format!("{prefix}.0.idf"), enc.idf.clone()));
            if let Some(d) = &enc.dictionary {
                files.push((format!("{prefix}.dictionary"), d.clone()));
            }
        }
        let table = format!(
            "<XMObject class=\"XMSimpleTable\" name=\"{0}\"><Members>{1}</Members><Collections><Collection><Name>Columns</Name>{columns}</Collection></Collections></XMObject>",
            esc(&t.name),
            member(
                "SegmentMap",
                &format!(
                    "<XMObject class=\"XMMultiPartSegmentMap\"><Collections><Collection><Name>Partitions</Name><XMObject class=\"XMSegment1Map\">{}</XMObject></Collection></Collections></XMObject>",
                    props(&[("Records", rows.to_string())])
                )
            )
        );
        files.push((format!("{}.1.tbl.xml", t.name), table.into_bytes()));
    }
    let mut commands = String::from("<Command><Text>CALCULATE;</Text></Command>");
    for (t, n, e) in &model.measures {
        commands.push_str(&format!(
            "<Command><Text>{}</Text></Command>",
            esc(&format!(
                "CREATE MEASURE '{}'[{}]={e};",
                t.replace('\'', "''"),
                n.replace(']', "]]")
            ))
        ));
    }
    files.push((
        "MdxScript.1.scr.xml".into(),
        load(
            &cube_parent,
            &format!("<MdxScript><ID>MdxScript</ID><Name>MdxScript</Name><Commands>{commands}</Commands></MdxScript>"),
        )
        .into_bytes(),
    ));
    files.push((
        "Workbook.1.db.xml".into(),
        load(
            "",
            &format!(
                "<Database><ID>Workbook</ID><Name>Workbook</Name><CompatibilityLevel>{}</CompatibilityLevel></Database>",
                model.compatibility_level
            ),
        )
        .into_bytes(),
    ));
    Ok(files)
}

/// The CRC end marker of [MS-XLDM] §2.1.2.2.1.1: an MSB-first CRC-32 (polynomial 0x04C11DB7,
/// initial value and final XOR 0xFFFFFFFF) of the stored bytes before it, little-endian, as
/// Excel writes it.
pub fn crc_marker(bytes: &[u8]) -> [u8; 4] {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in bytes {
        crc ^= u32::from(b) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 {
                (crc << 1) ^ 0x04C1_1DB7
            } else {
                crc << 1
            };
        }
    }
    (!crc).to_le_bytes()
}

fn utf16_bom(s: &str) -> Vec<u8> {
    let mut out = vec![0xFF, 0xFE];
    for u in s.encode_utf16() {
        out.extend_from_slice(&u.to_le_bytes());
    }
    out
}

fn pad_to_page(v: &mut Vec<u8>) {
    let page = bicraft_abf::PAGE_SIZE;
    v.resize(v.len().div_ceil(page) * page, 0);
}

/// The backup of `model` (the `item.data` bytes).
pub fn backup(model: &SynthModel) -> Result<Vec<u8>, String> {
    const ROOT: &str = r"\\?\C:\bicraft\PowerPivot";
    const TIME: u64 = 134_000_000_000_000_000;
    let db_folder = format!(r"{ROOT}\Workbook.0.db");
    let mut stream = vec![0u8; bicraft_abf::PAGE_SIZE];
    let mut dir: Vec<(String, usize, usize)> = Vec::new();
    let mut store = |stream: &mut Vec<u8>, name: String, stored: &[u8]| {
        let offset = stream.len();
        stream.extend_from_slice(stored);
        stream.extend_from_slice(&crc_marker(stored));
        dir.push((name, offset, stored.len() + 4));
    };
    store(&mut stream, "PARTITIONS".into(), &utf16_bom("<Partitions/>"));
    let mut records = String::new();
    for (i, (name, content)) in model_files(model)?.into_iter().enumerate() {
        let storage = format!("{:020X}", i + 1);
        let path = if name.ends_with(".db.xml") {
            format!(r"{ROOT}\{name}")
        } else {
            format!(r"{db_folder}\{name}")
        };
        records.push_str(&format!(
            "<BackupFile><Path>{}</Path><StoragePath>{storage}</StoragePath><LastWriteTime>{TIME}</LastWriteTime><Size>{}</Size></BackupFile>",
            esc(&path),
            content.len()
        ));
        let packed = bicraft_xpress8::compress_chunked(&content, bicraft_xpress8::DEFAULT_CHUNK_SIZE);
        store(&mut stream, storage, &packed);
    }
    let log = format!(
        "<BackupLog><BackupRestoreSyncVersion>1153</BackupRestoreSyncVersion><ServerRoot>{}</ServerRoot><EncryptionFlag>false</EncryptionFlag><ObjectName>Workbook</ObjectName><ObjectId>Workbook</ObjectId><Write>ReadWrite</Write><OlapInfo>false</OlapInfo><Collations/><Languages><Language>1033</Language></Languages><FileGroups><FileGroup><Class>100002</Class><ID>Workbook</ID><Name>Workbook</Name><PersistLocationPath>{}</PersistLocationPath><FileList>{records}</FileList></FileGroup></FileGroups></BackupLog>",
        esc(ROOT),
        esc(&db_folder)
    );
    store(&mut stream, "LOG".into(), &utf16_bom(&log));
    pad_to_page(&mut stream);
    let dir_offset = stream.len();
    let mut dir_xml = String::from("<VirtualDirectory>");
    for (name, offset, size) in &dir {
        dir_xml.push_str(&format!(
            "<BackupFile><Path>{name}</Path><Size>{size}</Size><m_cbOffsetHeader>{offset}</m_cbOffsetHeader><Delete>false</Delete><CreatedTimestamp>{TIME}</CreatedTimestamp><Access>{TIME}</Access><LastWriteTime>{TIME}</LastWriteTime></BackupFile>"
        ));
    }
    dir_xml.push_str("</VirtualDirectory>");
    stream.extend_from_slice(dir_xml.as_bytes());
    pad_to_page(&mut stream);
    let header = format!(
        "<BackupLog><BackupRestoreSyncVersion>140</BackupRestoreSyncVersion><Fault>false</Fault><faultcode>0</faultcode><ErrorCode>true</ErrorCode><EncryptionFlag>false</EncryptionFlag><EncryptionKey>0</EncryptionKey><ApplyCompression>true</ApplyCompression><m_cbOffsetHeader>{dir_offset}</m_cbOffsetHeader><DataSize>{}</DataSize><Files>{}</Files><ObjectID>0A1B2C3D-4E5F-4061-8273-94A5B6C7D8E9</ObjectID><m_cbOffsetData>{}</m_cbOffsetData></BackupLog>",
        dir_xml.len(),
        dir.len(),
        bicraft_abf::PAGE_SIZE
    );
    let mut page = utf16_bom("STREAM_STORAGE_SIGNATURE_)!@#$%^&*(");
    page.extend(utf16_bom(&header).get(2..).unwrap_or_default());
    if page.len() > bicraft_abf::PAGE_SIZE {
        return Err("the backup header does not fit its page".into());
    }
    if let Some(dst) = stream.get_mut(..page.len()) {
        dst.copy_from_slice(&page);
    }
    Ok(stream)
}

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="data" ContentType="application/vnd.openxmlformats-officedocument.model+data"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#;
const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;
const WORKBOOK: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
const SHEET: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#;

fn workbook_rels(with_model: bool) -> String {
    let model = if with_model {
        r#"<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/powerPivotData" Target="model/item.data"/>"#
    } else {
        ""
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>{model}</Relationships>"#
    )
}

fn package(item: Option<Vec<u8>>) -> Result<Vec<u8>, String> {
    let mut parts = vec![
        Part::new("[Content_Types].xml", CONTENT_TYPES.as_bytes().to_vec(), false),
        Part::new("_rels/.rels", ROOT_RELS.as_bytes().to_vec(), false),
        Part::new("xl/workbook.xml", WORKBOOK.as_bytes().to_vec(), false),
        Part::new(
            "xl/_rels/workbook.xml.rels",
            workbook_rels(item.is_some()).into_bytes(),
            false,
        ),
        Part::new("xl/worksheets/sheet1.xml", SHEET.as_bytes().to_vec(), false),
    ];
    if let Some(item) = item {
        parts.push(Part::new(crate::ITEM_DATA, item, true));
    }
    Package { parts }.to_bytes().map_err(|e| e.to_string())
}

/// A workbook (`.xlsx` bytes) holding `model` as its Power Pivot data model.
pub fn workbook(model: &SynthModel) -> Result<Vec<u8>, String> {
    package(Some(backup(model)?))
}

/// A workbook without a data model.
pub fn without_model() -> Result<Vec<u8>, String> {
    package(None)
}

#[cfg(test)]
mod tests {
    #[test]
    fn crc_marker_is_the_crc32_excel_writes() {
        // The CRC-32/BZIP2 check value of "123456789" (MSB first, 0x04C11DB7, initial and
        // final XOR 0xFFFFFFFF), stored little-endian: what every marker of a workbook Excel
        // 16.0.20625 saved holds over the bytes before it (docs/formats/powerpivot.md).
        assert_eq!(super::crc_marker(b"123456789"), 0xFC89_1918u32.to_le_bytes());
    }
}
