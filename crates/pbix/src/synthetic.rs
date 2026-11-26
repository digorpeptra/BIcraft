//! Synthetic PBIX packages for tests (here and in the crates above): a small Analysis Services
//! backup shaped like Power BI's ([MS-XLDM] §2.1 and the layout observed in
//! `docs/formats/abf.md`) around a given `metadata.sqlitedb`, XPress9-framed like Desktop's
//! `DataModel`. All content is our own; nothing is copied from a real file.

use bicraft_opc::{Encoding, Package, encode_text};
use bicraft_sqlite::{DatabaseBuilder, Value};

use crate::{DATA_MODEL, Error, LEGACY_LAYOUT};

const PAGE: usize = 4096;
const PERSIST_ROOT: &str = r"\\?\C:\Synth\Data\00000000-0000-0000-0000-000000000001.0.db";

/// A legacy report with one empty page.
pub const LAYOUT: &str = r#"{"config":"{}","layoutOptimization":0,"sections":[{"name":"ReportSection","displayName":"Page 1","width":1280,"height":720,"displayOption":1,"visualContainers":[],"config":"{}","filters":"[]"}]}"#;

fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

fn bom_utf16(s: &str) -> Vec<u8> {
    let mut v = vec![0xFF, 0xFE];
    v.extend(utf16(s));
    v
}

/// An ABF holding `files` (relative logical path, payload), with Power BI's bookkeeping files
/// (`ADDITIONAL_LOG`, `PARTITIONS`) and the `LOG` BackupLog last.
pub fn abf(files: &[(&str, &[u8])]) -> Result<Vec<u8>, Error> {
    let s = sparse_abf(files, None)?;
    Ok([s.head, s.tail].concat())
}

/// A backup kept in two pieces around a run of zeros, so one larger than memory can be
/// described: the stream is `head`, then `zeros` zero bytes, then `tail`.
#[derive(Debug, Clone)]
pub struct SparseAbf {
    pub head: Vec<u8>,
    pub zeros: u64,
    pub tail: Vec<u8>,
}

impl SparseAbf {
    pub fn len(&self) -> u64 {
        self.head.len() as u64 + self.zeros + self.tail.len() as u64
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// [`abf`] with, after `files`, a file of `big.1` zero bytes named `big.0` that is not
/// materialized (see [`SparseAbf`]).
pub fn sparse_abf(files: &[(&str, &[u8])], big: Option<(&str, u64)>) -> Result<SparseAbf, Error> {
    let mut stream = vec![0u8; PAGE];
    // Bytes of the zero run, and where it starts in `stream`.
    let mut gap: Option<(usize, u64)> = None;
    let logical = |stream: &Vec<u8>, gap: Option<(usize, u64)>| stream.len() as u64 + gap.map_or(0, |g| g.1);
    let mut dir: Vec<(String, u64, u64)> = Vec::new();
    let push = |dir: &mut Vec<(String, u64, u64)>,
                stream: &mut Vec<u8>,
                gap: Option<(usize, u64)>,
                name: String,
                data: &[u8]| {
        dir.push((name, logical(stream, gap), data.len() as u64));
        stream.extend_from_slice(data);
    };
    push(
        &mut dir,
        &mut stream,
        gap,
        "ADDITIONAL_LOG".into(),
        &bom_utf16("<Property><ProductName>Synth</ProductName></Property>"),
    );
    push(
        &mut dir,
        &mut stream,
        gap,
        "PARTITIONS".into(),
        &bom_utf16("<Partitions/>"),
    );
    let mut log_files = String::new();
    let mut log_entry = |storage: &str, path: &str, size: u64| {
        log_files.push_str(&format!(
            "<BackupFile><Path>{PERSIST_ROOT}\\{}</Path><StoragePath>{storage}</StoragePath>\
             <LastWriteTime>134000000000000000</LastWriteTime><Size>{size}</Size></BackupFile>",
            path.replace('/', "\\").replace('&', "&amp;"),
        ));
    };
    for (i, (path, data)) in files.iter().enumerate() {
        let storage = format!("{:020X}", 0xABF0_0000u64 + i as u64);
        push(&mut dir, &mut stream, gap, storage.clone(), data);
        log_entry(&storage, path, data.len() as u64);
    }
    if let Some((path, len)) = big {
        let storage = format!("{:020X}", 0xABF1_0000u64);
        dir.push((storage.clone(), logical(&stream, gap), len));
        gap = Some((stream.len(), len));
        log_entry(&storage, path, len);
    }
    let log = format!(
        "<BackupLog><BackupRestoreSyncVersion>11.53</BackupRestoreSyncVersion><ServerRoot>x</ServerRoot>\
         <ObjectName>synth</ObjectName><ObjectId>synth</ObjectId><Write>ReadWrite</Write><IsTabular>true</IsTabular>\
         <Collations/><Languages><Language>1033</Language></Languages><FileGroups><FileGroup><Class>100069</Class>\
         <ID>g</ID><Name>g</Name><PersistLocationPath>{PERSIST_ROOT}</PersistLocationPath><StorageLocationPath/>\
         <FileList>{log_files}</FileList></FileGroup></FileGroups></BackupLog>"
    );
    push(&mut dir, &mut stream, gap, "LOG".into(), &bom_utf16(&log));
    let pad = |stream: &mut Vec<u8>, gap: Option<(usize, u64)>| {
        let rem = (logical(stream, gap) % PAGE as u64) as usize;
        if rem != 0 {
            stream.resize(stream.len() + PAGE - rem, 0);
        }
    };
    pad(&mut stream, gap);

    let dir_offset = logical(&stream, gap);
    let mut dir_xml = String::from("<VirtualDirectory>");
    for (name, offset, size) in &dir {
        dir_xml.push_str(&format!(
            "<BackupFile><Path>{name}</Path><Size>{size}</Size><m_cbOffsetHeader>{offset}</m_cbOffsetHeader>\
             <Delete>false</Delete><CreatedTimestamp>1</CreatedTimestamp><Access>1</Access>\
             <LastWriteTime>134000000000000001</LastWriteTime></BackupFile>"
        ));
    }
    dir_xml.push_str("</VirtualDirectory>");
    stream.extend_from_slice(dir_xml.as_bytes());
    pad(&mut stream, gap);

    let header = format!(
        "<BackupLog><BackupRestoreSyncVersion>140</BackupRestoreSyncVersion><Fault>false</Fault>\
         <faultcode>0</faultcode><ErrorCode>false</ErrorCode><EncryptionFlag>false</EncryptionFlag>\
         <EncryptionKey>0</EncryptionKey><ApplyCompression>false</ApplyCompression>\
         <m_cbOffsetHeader>{dir_offset}</m_cbOffsetHeader><DataSize>{}</DataSize><Files>{}</Files>\
         <ObjectID>00000000-0000-0000-0000-000000000002</ObjectID><m_cbOffsetData>4096</m_cbOffsetData></BackupLog>",
        dir_xml.len(),
        dir.len()
    );
    let mut page = vec![0xFF, 0xFE];
    page.extend(utf16("STREAM_STORAGE_SIGNATURE_)!@#$%^&*("));
    page.extend(utf16(&header));
    let dst = stream
        .get_mut(..page.len())
        .ok_or(Error::Verify("synthetic ABF header larger than a page"))?;
    dst.copy_from_slice(&page);
    let (at, zeros) = gap.unwrap_or((stream.len(), 0));
    let tail = stream.split_off(at);
    Ok(SparseAbf {
        head: stream,
        zeros,
        tail,
    })
}

/// A PBIX: `Version`, an XPress9-framed `DataModel` whose backup holds `metadata_sqlitedb` (plus
/// a few placeholder storage files), a legacy `Report/Layout` and one static resource.
pub fn pbix(metadata_sqlitedb: &[u8]) -> Result<Package, Error> {
    let backup = abf(&[
        ("metadata.sqlitedb", metadata_sqlitedb),
        ("0.CryptKey.bin", b"\x01\x02\x03"),
        (
            "Sales (3).tbl/4.prt/1.Sales (3).Amount (5).0.idf",
            b"synthetic column data",
        ),
    ])?;
    let data_model = bicraft_xpress9::compress_datamodel(&backup, None)?;
    let mut pkg = Package::default();
    pkg.put("Version", encode_text("1.28", Encoding::Utf16Le), false);
    pkg.put(DATA_MODEL, data_model, true);
    pkg.put(LEGACY_LAYOUT, encode_text(LAYOUT, Encoding::Utf16Le), false);
    pkg.put("Settings", encode_text("{}", Encoding::Utf16Le), false);
    pkg.put(
        "Report/StaticResources/x.png",
        vec![0x89, b'P', b'N', b'G'],
        false,
    );
    Ok(pkg)
}

/// PBIR documents of a one-page report with one card, keyed by path relative to `Report/`.
pub fn pbir_documents() -> Vec<(String, String)> {
    const BASE: &str = "https://developer.microsoft.com/json-schemas/fabric/item/report/definition";
    vec![
        (
            "definition/version.json".into(),
            format!(r#"{{"$schema":"{BASE}/versionMetadata/1.0.0/schema.json","version":"2.0.0"}}"#),
        ),
        (
            "definition/report.json".into(),
            format!(
                r#"{{"$schema":"{BASE}/report/3.3.0/schema.json","themeCollection":{{"baseTheme":{{"name":"Base","reportVersionAtImport":{{"visual":"2.1.0","report":"3.0.0","page":"2.3.0"}},"type":"SharedResources"}}}},"resourcePackages":[{{"name":"SharedResources","type":"SharedResources","items":[{{"name":"Base","path":"BaseThemes/Base.json","type":"BaseTheme"}}]}}]}}"#
            ),
        ),
        (
            "definition/pages/pages.json".into(),
            format!(
                r#"{{"$schema":"{BASE}/pagesMetadata/1.1.0/schema.json","pageOrder":["p1"],"activePageName":"p1"}}"#
            ),
        ),
        (
            "definition/pages/p1/page.json".into(),
            format!(
                r#"{{"$schema":"{BASE}/page/2.1.0/schema.json","name":"p1","displayName":"Overview","displayOption":"FitToPage","height":720,"width":1280}}"#
            ),
        ),
        (
            "definition/pages/p1/visuals/v1/visual.json".into(),
            format!(
                r#"{{"$schema":"{BASE}/visualContainer/2.12.0/schema.json","name":"v1","position":{{"x":10,"y":10,"z":0,"height":120,"width":240,"tabOrder":0}},"visual":{{"visualType":"card","query":{{"queryState":{{"Values":{{"projections":[{{"field":{{"Measure":{{"Expression":{{"SourceRef":{{"Entity":"Sales"}}}},"Property":"Total"}}}},"queryRef":"Sales.Total","nativeQueryRef":"Total"}}]}}}}}}}}}}"#
            ),
        ),
        (
            "StaticResources/SharedResources/BaseThemes/Base.json".into(),
            r#"{"name":"Base"}"#.into(),
        ),
    ]
}

/// `[Content_Types].xml` of [`pbir_pbix`], shaped like Desktop's (UTF-8 with BOM).
const PBIR_CONTENT_TYPES: &str = "\u{feff}<?xml version=\"1.0\" encoding=\"utf-8\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"json\" ContentType=\"\" /><Override PartName=\"/Version\" ContentType=\"\" /><Override PartName=\"/Settings\" ContentType=\"application/json\" /><Override PartName=\"/SecurityBindings\" ContentType=\"\" /><Override PartName=\"/DataModel\" ContentType=\"\" /></Types>";

/// [`pbix`] with the report as PBIR parts instead of `Report/Layout`, laid out like a Desktop
/// save: `[Content_Types].xml`, the report parts, a `SecurityBindings` placeholder (our own
/// bytes, not a real binding), then `DataModel`.
pub fn pbir_pbix(metadata_sqlitedb: &[u8]) -> Result<Package, Error> {
    let legacy = pbix(metadata_sqlitedb)?;
    let data_model = legacy.get(DATA_MODEL).map(|p| p.data.clone()).unwrap_or_default();
    let mut pkg = Package::default();
    pkg.put("Version", encode_text("1.32", Encoding::Utf16Le), false);
    pkg.put(
        "[Content_Types].xml",
        PBIR_CONTENT_TYPES.as_bytes().to_vec(),
        false,
    );
    pkg.put("Settings", encode_text("{}", Encoding::Utf16Le), false);
    for (path, text) in pbir_documents() {
        pkg.put(&format!("Report/{path}"), text.into_bytes(), false);
    }
    pkg.put("SecurityBindings", vec![1, 0, 0, 0, 0xAB, 0xCD], false);
    pkg.put(DATA_MODEL, data_model, true);
    Ok(pkg)
}

/// `ModifiedTime` / `LASTCOMMITTED` of [`storage_database`] (Windows FILETIME ticks).
pub const SYNTHETIC_TIME: i64 = 134_116_991_999_999_999;

/// The column lists of the 2.158 schema (docs/formats/abf.md) for the tables these tests touch.
const SCHEMA: &[(&str, &str)] = &[
    ("Model", "Name T, Description T, Culture T, ModifiedTime I"),
    (
        "Table",
        "ModelID I, Name T, DataCategory T, Description T, IsHidden I, TableStorageID I, ModifiedTime I, StructureModifiedTime I, SystemFlags I, ShowAsVariationsOnly I, IsPrivate I, DefaultDetailRowsDefinitionID I, AlternateSourcePrecedence I, RefreshPolicyID I, CalculationGroupID I, ExcludeFromModelRefresh I, LineageTag T, SourceLineageTag T, SystemManaged I, ExcludeFromAutomaticAggregations I, DirectLakeIndexingBehavior I",
    ),
    (
        "Column",
        "TableID I, ExplicitName T, InferredName T, ExplicitDataType I, InferredDataType I, DataCategory T, Description T, IsHidden I, State I, IsUnique I, IsKey I, IsNullable I, Alignment I, TableDetailPosition I, IsDefaultLabel I, IsDefaultImage I, SummarizeBy I, ColumnStorageID I, Type I, SourceColumn T, ColumnOriginID I, Expression T, FormatString T, IsAvailableInMDX I, SortByColumnID I, AttributeHierarchyID I, ModifiedTime I, StructureModifiedTime I, RefreshedTime I, SystemFlags I, KeepUniqueRows I, DisplayOrdinal I, ErrorMessage T, SourceProviderType T, DisplayFolder T, EncodingHint I, RelatedColumnDetailsID I, AlternateOfID I, LineageTag T, SourceLineageTag T, ExpressionContext I, StringIndexingBehavior I, FullTextIndexingBehavior I",
    ),
    (
        "Measure",
        "TableID I, Name T, Description T, DataType I, Expression T, FormatString T, IsHidden I, State I, ModifiedTime I, StructureModifiedTime I, KPIID I, IsSimpleMeasure I, ErrorMessage T, DisplayFolder T, DetailRowsDefinitionID I, DataCategory T, FormatStringDefinitionID I, LineageTag T, SourceLineageTag T",
    ),
    (
        "Partition",
        "TableID I, Name T, Description T, DataSourceID I, QueryDefinition T, State I, Type I, PartitionStorageID I, Mode I, DataView I, ModifiedTime I, RefreshedTime I, SystemFlags I, ErrorMessage T, RetainDataTillForceCalculate I, RangeStart R, RangeEnd R, RangeGranularity I, RefreshBookmark T, QueryGroupID I, ExpressionSourceID I, MAttributes T, DataCoverageDefinitionID I, SchemaName T",
    ),
    (
        "Relationship",
        "ModelID I, Name T, IsActive I, Type I, CrossFilteringBehavior I, JoinOnDateBehavior I, RelyOnReferentialIntegrity I, FromTableID I, FromColumnID I, FromCardinality I, ToTableID I, ToColumnID I, ToCardinality I, State I, RelationshipStorageID I, RelationshipStorage2ID I, ModifiedTime I, RefreshedTime I, SecurityFilteringBehavior I",
    ),
    (
        "Annotation",
        "ObjectID I, ObjectType I, Name T, Value T, ModifiedTime I",
    ),
    (
        "ExtendedProperty",
        "ObjectID I, ObjectType I, Name T, Type I, Value T, ModifiedTime I",
    ),
    ("RelatedColumnDetails", "ColumnID I, ModifiedTime I"),
    (
        "GroupByColumn",
        "RelatedColumnDetailsID I, GroupingColumnID I, ModifiedTime I",
    ),
    (
        "AttributeHierarchy",
        "ColumnID I, State I, AttributeHierarchyStorageID I, ModifiedTime I, RefreshedTime I",
    ),
    ("StorageFolder", "OwnerID I, OwnerType I, Path T"),
    (
        "StorageFile",
        "OwnerID I, OwnerType I, StorageFolderID I, FileName T",
    ),
    (
        "TableStorage",
        "TableID I, Name T, Version I, Settings I, RIViolationCount I, StorageFolderID I",
    ),
    (
        "ColumnStorage",
        "ColumnID I, Name T, StoragePosition I, DictionaryStorageID I, Settings I, ColumnFlags I, Collation T, OrderByColumn T, Locale I, BinaryCharacters I, Statistics_DistinctStates I, Statistics_MinDataID I, Statistics_MaxDataID I, Statistics_OriginalMinSegmentDataID I, Statistics_RLESortOrder I, Statistics_RowCount I, Statistics_HasNulls I, Statistics_RLERuns I, Statistics_OthersRLERuns I, Statistics_Usage I, Statistics_DBType I, Statistics_XMType I, Statistics_CompressionType I, Statistics_CompressionParam I, Statistics_EncodingHint I, IsDeltaPartitionColumn I, DeltaColumnMappingPhysicalName T, DeltaColumnMappingId I, FramedSourceColumn T, DeltaColumnPrimitiveType I, DeltaTableColumnStorageID I, StringIndexStorageID I, ColumnIndexStorageID I, FullTextIndexStorageID I",
    ),
    (
        "PartitionStorage",
        "PartitionID I, Name T, StoragePosition I, SegmentMapStorageID I, DataObjectId I, StorageFolderID I, DeltaTableMetadataStorageID I",
    ),
    (
        "SegmentMapStorage",
        "PartitionStorageID I, Type I, RecordCount I, SegmentCount I, RecordsPerSegment I",
    ),
    (
        "DictionaryStorage",
        "ColumnStorageID I, Type I, DataType I, DataVersion I, BaseId I, Magnitude R, LastId I, IsNullable I, IsUnique I, IsOperatingOn32 I, DictionaryFlags I, StorageFileID I, Size I",
    ),
    (
        "ColumnPartitionStorage",
        "ColumnStorageID I, PartitionStorageID I, DataVersion I, State I, SegmentStorageID I, StorageFileID I",
    ),
    (
        "SegmentStorage",
        "ColumnPartitionStorageID I, SegmentCount I, StorageFileID I",
    ),
    (
        "RelationshipStorage",
        "RelationshipID I, Name T, DefinitionType I, Cardinality I, Flags I, RelationshipIndexStorageID I",
    ),
    (
        "RelationshipIndexStorage",
        "RelationshipStorageID I, IndexType I, Flags I, RecordCount I, SecondaryRecordCount I, StorageFolderID I, StorageFileID I, SystemTableID I, SecondarySystemTableID I",
    ),
    (
        "AttributeHierarchyStorage",
        "AttributeHierarchyID I, SortOrder I, OptimizationLevel I, MaterializationType I, ColumnPositionToData I, ColumnDataToPosition I, DistinctDataCount I, DataVersion I, StorageFileID I, SystemTableID I, HasStatistics I, MinValue T, MaxValue T, StringValueMaxLength I",
    ),
    (
        "Hierarchy",
        "TableID I, Name T, Description T, IsHidden I, State I, HierarchyStorageID I, ModifiedTime I, StructureModifiedTime I, RefreshedTime I, DisplayFolder T, HideMembers I, LineageTag T, SourceLineageTag T",
    ),
    (
        "Level",
        "HierarchyID I, Ordinal I, Name T, Description T, ColumnID I, ModifiedTime I, LineageTag T, SourceLineageTag T",
    ),
    (
        "HierarchyStorage",
        "HierarchyID I, Name T, MaterializationType I, StructureType I, SystemTableID I, LevelDefinition T",
    ),
];

/// An empty `metadata.sqlitedb` with the tables a structural edit touches (columns of the
/// Desktop 2.158 schema) and a `Model` row (culture en-US): the base of synthetic structural
/// saves.
pub fn storage_database() -> Result<Vec<u8>, Error> {
    let fail = |e: bicraft_sqlite::Error| Error::Structural(format!("synthetic database: {e}"));
    let mut b = DatabaseBuilder::new(4096).map_err(fail)?;
    for (name, cols) in SCHEMA {
        let cols: Vec<String> = cols
            .split(", ")
            .map(|c| {
                let (n, t) = c.rsplit_once(' ').unwrap_or((c, "I"));
                let ty = match t {
                    "I" => "INTEGER",
                    "T" => "TEXT",
                    _ => "REAL",
                };
                format!("[{n}] {ty}")
            })
            .collect();
        let sql = format!(
            "CREATE TABLE [{name}] ([ID] INTEGER, {}, PRIMARY KEY([ID] ASC))",
            cols.join(", ")
        );
        b.create_table(name, &sql).map_err(fail)?;
    }
    b.create_table(
        "DBPROPERTIES",
        "CREATE TABLE [DBPROPERTIES] ([NAME] STRING PRIMARY KEY, [VALUE] INTEGER)",
    )
    .map_err(fail)?;
    b.insert(
        "Model",
        1,
        vec![
            Value::Integer(1),
            Value::Text("Model".into()),
            Value::Null,
            Value::Text("en-US".into()),
            Value::Integer(SYNTHETIC_TIME),
        ],
    )
    .map_err(fail)?;
    b.insert(
        "DBPROPERTIES",
        1,
        vec![Value::Text("MAXID".into()), Value::Integer(1)],
    )
    .map_err(fail)?;
    b.insert(
        "DBPROPERTIES",
        2,
        vec![
            Value::Text("LASTCOMMITTED".into()),
            Value::Integer(SYNTHETIC_TIME),
        ],
    )
    .map_err(fail)?;
    b.finish().map_err(fail)
}
