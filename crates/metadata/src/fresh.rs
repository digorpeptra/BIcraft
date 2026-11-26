//! A brand-new `metadata.sqlitedb` for a model built from scratch (M5: a PBIX written without an
//! existing `DataModel`). See `docs/formats/datamodel-fresh.md`.
//!
//! [`fresh_sqlitedb_at`] writes the database the engine creates for an empty database: the 68
//! TMSCHEMA tables plus `DBPROPERTIES` with the column lists and `CREATE` text of Desktop 2.158's
//! engine, the `Model` row carrying every model-level property of `model`, the `QueryGroup` rows
//! and the shared expressions (`Expression`). Nothing else: tables, columns, measures,
//! relationships, roles, cultures, perspectives and annotations are then added to it by the
//! structural writer ([`crate::write_structural_sqlitedb_at`]), exactly as edits of an existing
//! file, so a model built from scratch and an edited one share every row writer.
//!
//! Model-level properties that point at objects which do not exist yet are refused
//! ([`Error::UnsupportedChange`]): `defaultMeasure`, and an expression's
//! `parameterValuesColumn`.

use std::collections::HashMap;

use bicraft_model::SemanticModel;
use bicraft_sqlite::{DatabaseBuilder, Value};
use serde_json::Value as Json;

use crate::codes::{self, code_of};
use crate::{Error, UnsupportedChange};

/// The TMSCHEMA tables of a Desktop 2.158 `metadata.sqlitedb` (`SCHEMAVERSION` 123), in creation
/// order, with their columns after `ID` (`I` INTEGER, `T` TEXT, `R` REAL, `B` BLOB). Table and
/// column names are the public TMSCHEMA rowsets ([MS-SSAS-T] §2.2.5); the order and the 2.158
/// additions were observed on databases the engine creates (`datamodel-fresh.md`).
pub(crate) const SCHEMA: &[(&str, &str)] = &[
    (
        "Model",
        "Name:T Description:T StorageLocation:T DefaultMode:I DefaultDataView:I Culture:T Collation:T ModifiedTime:I StructureModifiedTime:I DataAccessOptions:T DefaultMeasureID:I DefaultPowerBIDataSourceVersion:I ForceUniqueNames:I DiscourageImplicitMeasures:I DiscourageReportMeasures:I DataSourceVariablesOverrideBehavior:I DataSourceDefaultMaxConnections:I SourceQueryCulture:T MAttributes:T DiscourageCompositeModels:I AutomaticAggregationOptions:T DisableAutoExists:I MaxParallelismPerRefresh:I MaxParallelismPerQuery:I DisableSystemDefaultExpression:I DirectLakeBehavior:I ValueFilterBehavior:I SelectionExpressionBehavior:I MetadataAccessPolicy:I DefaultDirectLakeIndexingBehavior:I",
    ),
    (
        "DataSource",
        "ModelID:I Name:T Description:T Type:I ConnectionString:T ImpersonationMode:I Account:T Password:T MaxConnections:I Isolation:I Timeout:I Provider:T ModifiedTime:I ConnectionDetails:T Options:T Credential:T ContextExpression:T",
    ),
    (
        "Table",
        "ModelID:I Name:T DataCategory:T Description:T IsHidden:I TableStorageID:I ModifiedTime:I StructureModifiedTime:I SystemFlags:I ShowAsVariationsOnly:I IsPrivate:I DefaultDetailRowsDefinitionID:I AlternateSourcePrecedence:I RefreshPolicyID:I CalculationGroupID:I ExcludeFromModelRefresh:I LineageTag:T SourceLineageTag:T SystemManaged:I ExcludeFromAutomaticAggregations:I DirectLakeIndexingBehavior:I",
    ),
    (
        "Column",
        "TableID:I ExplicitName:T InferredName:T ExplicitDataType:I InferredDataType:I DataCategory:T Description:T IsHidden:I State:I IsUnique:I IsKey:I IsNullable:I Alignment:I TableDetailPosition:I IsDefaultLabel:I IsDefaultImage:I SummarizeBy:I ColumnStorageID:I Type:I SourceColumn:T ColumnOriginID:I Expression:T FormatString:T IsAvailableInMDX:I SortByColumnID:I AttributeHierarchyID:I ModifiedTime:I StructureModifiedTime:I RefreshedTime:I SystemFlags:I KeepUniqueRows:I DisplayOrdinal:I ErrorMessage:T SourceProviderType:T DisplayFolder:T EncodingHint:I RelatedColumnDetailsID:I AlternateOfID:I LineageTag:T SourceLineageTag:T ExpressionContext:I StringIndexingBehavior:I FullTextIndexingBehavior:I",
    ),
    (
        "AttributeHierarchy",
        "ColumnID:I State:I AttributeHierarchyStorageID:I ModifiedTime:I RefreshedTime:I",
    ),
    (
        "Partition",
        "TableID:I Name:T Description:T DataSourceID:I QueryDefinition:T State:I Type:I PartitionStorageID:I Mode:I DataView:I ModifiedTime:I RefreshedTime:I SystemFlags:I ErrorMessage:T RetainDataTillForceCalculate:I RangeStart:R RangeEnd:R RangeGranularity:I RefreshBookmark:T QueryGroupID:I ExpressionSourceID:I MAttributes:T DataCoverageDefinitionID:I SchemaName:T",
    ),
    (
        "Relationship",
        "ModelID:I Name:T IsActive:I Type:I CrossFilteringBehavior:I JoinOnDateBehavior:I RelyOnReferentialIntegrity:I FromTableID:I FromColumnID:I FromCardinality:I ToTableID:I ToColumnID:I ToCardinality:I State:I RelationshipStorageID:I RelationshipStorage2ID:I ModifiedTime:I RefreshedTime:I SecurityFilteringBehavior:I",
    ),
    (
        "Measure",
        "TableID:I Name:T Description:T DataType:I Expression:T FormatString:T IsHidden:I State:I ModifiedTime:I StructureModifiedTime:I KPIID:I IsSimpleMeasure:I ErrorMessage:T DisplayFolder:T DetailRowsDefinitionID:I DataCategory:T FormatStringDefinitionID:I LineageTag:T SourceLineageTag:T",
    ),
    (
        "Hierarchy",
        "TableID:I Name:T Description:T IsHidden:I State:I HierarchyStorageID:I ModifiedTime:I StructureModifiedTime:I RefreshedTime:I DisplayFolder:T HideMembers:I LineageTag:T SourceLineageTag:T",
    ),
    (
        "Level",
        "HierarchyID:I Ordinal:I Name:T Description:T ColumnID:I ModifiedTime:I LineageTag:T SourceLineageTag:T",
    ),
    (
        "Annotation",
        "ObjectID:I ObjectType:I Name:T Value:T ModifiedTime:I",
    ),
    (
        "KPI",
        "MeasureID:I Description:T TargetDescription:T TargetExpression:T TargetFormatString:T StatusGraphic:T StatusDescription:T StatusExpression:T TrendGraphic:T TrendDescription:T TrendExpression:T ModifiedTime:I",
    ),
    (
        "Culture",
        "ModelID:I Name:T LinguisticMetadataID:I ModifiedTime:I StructureModifiedTime:I",
    ),
    (
        "ObjectTranslation",
        "CultureID:I ObjectID:I ObjectType:I Property:I Value:T ModifiedTime:I Altered:I",
    ),
    (
        "LinguisticMetadata",
        "CultureID:I Content:T ModifiedTime:I ContentType:I",
    ),
    ("StorageFolder", "OwnerID:I OwnerType:I Path:T"),
    (
        "StorageFile",
        "OwnerID:I OwnerType:I StorageFolderID:I FileName:T",
    ),
    (
        "TableStorage",
        "TableID:I Name:T Version:I Settings:I RIViolationCount:I StorageFolderID:I",
    ),
    (
        "ColumnStorage",
        "ColumnID:I Name:T StoragePosition:I DictionaryStorageID:I Settings:I ColumnFlags:I Collation:T OrderByColumn:T Locale:I BinaryCharacters:I Statistics_DistinctStates:I Statistics_MinDataID:I Statistics_MaxDataID:I Statistics_OriginalMinSegmentDataID:I Statistics_RLESortOrder:I Statistics_RowCount:I Statistics_HasNulls:I Statistics_RLERuns:I Statistics_OthersRLERuns:I Statistics_Usage:I Statistics_DBType:I Statistics_XMType:I Statistics_CompressionType:I Statistics_CompressionParam:I Statistics_EncodingHint:I IsDeltaPartitionColumn:I DeltaColumnMappingPhysicalName:T DeltaColumnMappingId:I FramedSourceColumn:T DeltaColumnPrimitiveType:I DeltaTableColumnStorageID:I StringIndexStorageID:I ColumnIndexStorageID:I FullTextIndexStorageID:I",
    ),
    (
        "PartitionStorage",
        "PartitionID:I Name:T StoragePosition:I SegmentMapStorageID:I DataObjectId:I StorageFolderID:I DeltaTableMetadataStorageID:I",
    ),
    (
        "SegmentMapStorage",
        "PartitionStorageID:I Type:I RecordCount:I SegmentCount:I RecordsPerSegment:I",
    ),
    (
        "DictionaryStorage",
        "ColumnStorageID:I Type:I DataType:I DataVersion:I BaseId:I Magnitude:R LastId:I IsNullable:I IsUnique:I IsOperatingOn32:I DictionaryFlags:I StorageFileID:I Size:I",
    ),
    (
        "ColumnPartitionStorage",
        "ColumnStorageID:I PartitionStorageID:I DataVersion:I State:I SegmentStorageID:I StorageFileID:I",
    ),
    (
        "SegmentStorage",
        "ColumnPartitionStorageID:I SegmentCount:I StorageFileID:I",
    ),
    (
        "RelationshipStorage",
        "RelationshipID:I Name:T DefinitionType:I Cardinality:I Flags:I RelationshipIndexStorageID:I",
    ),
    (
        "RelationshipIndexStorage",
        "RelationshipStorageID:I IndexType:I Flags:I RecordCount:I SecondaryRecordCount:I StorageFolderID:I StorageFileID:I SystemTableID:I SecondarySystemTableID:I",
    ),
    (
        "AttributeHierarchyStorage",
        "AttributeHierarchyID:I SortOrder:I OptimizationLevel:I MaterializationType:I ColumnPositionToData:I ColumnDataToPosition:I DistinctDataCount:I DataVersion:I StorageFileID:I SystemTableID:I HasStatistics:I MinValue:T MaxValue:T StringValueMaxLength:I",
    ),
    (
        "HierarchyStorage",
        "HierarchyID:I Name:T LevelDefinition:T MaterializationType:I StructureType:I SystemTableID:I",
    ),
    ("Perspective", "ModelID:I Name:T Description:T ModifiedTime:I"),
    (
        "PerspectiveTable",
        "PerspectiveID:I TableID:I IncludeAll:I ModifiedTime:I",
    ),
    (
        "PerspectiveColumn",
        "PerspectiveTableID:I ColumnID:I ModifiedTime:I",
    ),
    (
        "PerspectiveHierarchy",
        "PerspectiveTableID:I HierarchyID:I ModifiedTime:I",
    ),
    (
        "PerspectiveMeasure",
        "PerspectiveTableID:I MeasureID:I ModifiedTime:I",
    ),
    (
        "Role",
        "ModelID:I Name:T Description:T ModelPermission:I ModifiedTime:I",
    ),
    (
        "RoleMembership",
        "RoleID:I MemberName:T MemberID:T IdentityProvider:T MemberType:I ModifiedTime:I",
    ),
    (
        "TablePermission",
        "RoleID:I TableID:I FilterExpression:T ModifiedTime:I State:I ErrorMessage:T MetadataPermission:I",
    ),
    (
        "Variation",
        "ColumnID:I Name:T Description:T RelationshipID:I DefaultHierarchyID:I DefaultColumnID:I IsDefault:I",
    ),
    (
        "Set",
        "TableID:I Name:T Description:T Expression:T IsDynamic:I IsHidden:I State:I ModifiedTime:I StructureModifiedTime:I ErrorMessage:T DisplayFolder:T",
    ),
    ("PerspectiveSet", "PerspectiveTableID:I SetID:I ModifiedTime:I"),
    (
        "ExtendedProperty",
        "ObjectID:I ObjectType:I Name:T Type:I Value:T ModifiedTime:I",
    ),
    (
        "Expression",
        "ModelID:I Name:T Description:T Kind:I Expression:T ModifiedTime:I QueryGroupID:I ParameterValuesColumnID:I MAttributes:T LineageTag:T SourceLineageTag:T RemoteParameterName:T ExpressionSourceID:I",
    ),
    (
        "ColumnPermission",
        "TablePermissionID:I ColumnID:I ModifiedTime:I MetadataPermission:I",
    ),
    (
        "DetailRowsDefinition",
        "ObjectID:I ObjectType:I Expression:T ModifiedTime:I State:I ErrorMessage:T",
    ),
    ("RelatedColumnDetails", "ColumnID:I ModifiedTime:I"),
    (
        "GroupByColumn",
        "RelatedColumnDetailsID:I GroupingColumnID:I ModifiedTime:I",
    ),
    (
        "CalculationGroup",
        "TableID:I Description:T ModifiedTime:I Precedence:I",
    ),
    (
        "CalculationItem",
        "CalculationGroupID:I FormatStringDefinitionID:I Name:T Description:T ModifiedTime:I State:I ErrorMessage:T Expression:T Ordinal:I",
    ),
    (
        "AlternateOf",
        "ColumnID:I BaseColumnID:I BaseTableID:I Summarization:I",
    ),
    (
        "RefreshPolicy",
        "TableID:I PolicyType:I RollingWindowGranularity:I RollingWindowPeriods:I IncrementalGranularity:I IncrementalPeriods:I IncrementalPeriodsOffset:I PollingExpression:T SourceExpression:T Mode:I",
    ),
    (
        "FormatStringDefinition",
        "ObjectID:I ObjectType:I Expression:T ModifiedTime:I State:I ErrorMessage:T",
    ),
    ("QueryGroup", "ModelID:I Folder:T Description:T"),
    (
        "AnalyticsAIMetadata",
        "ModelID:I Name:T MeasureAnalysisDefinition:T",
    ),
    ("ChangedProperty", "ObjectID:I ObjectType:I Property:T"),
    (
        "ExcludedArtifact",
        "ObjectID:I ObjectType:I ArtifactType:I Reference:T",
    ),
    (
        "GeneralSegmentMapSegmentMetadataStorage",
        "SegmentMapStorageID:I RecordCount:I Ordinal:I",
    ),
    (
        "DeltaTableMetadataStorage",
        "PartitionStorageID:I DataSourceReference:T TableName:T RootLocation:T CurrentVersion:T TableObjectID:T DatamartObjectID:T FramedSchemaName:T FallbackReason:I DeltaColumnMappingMode:I DeltaLogETag:T ArtifactObjectId:T ArtifactSecurityVersion:T",
    ),
    (
        "ParquetFileStorage",
        "DeltaTableMetadataStorageID:I Size:I Location:T CreatedVersion:T DeletedVersion:T RowgroupCount:I Ordinal:I ETag:T DeletionVectorFileLocation:T DeletionVectorBitmapOffset:I DeletionVectorBitmapSize:I DeletionVectorInlineBitmap:B DeletionVectorFileETag:T",
    ),
    (
        "DataCoverageDefinition",
        "PartitionID:I Description:T Expression:T ModifiedTime:I State:I ErrorMessage:T",
    ),
    (
        "CalculationExpression",
        "CalculationGroupID:I FormatStringDefinitionID:I Description:T Expression:T ModifiedTime:I State:I ErrorMessage:T SelectionMode:I",
    ),
    (
        "Calendar",
        "TableID:I Name:T Description:T LineageTag:T SourceLineageTag:T ModifiedTime:I",
    ),
    ("CalendarColumnGroup", "CalendarID:I TimeUnit:I ModifiedTime:I"),
    (
        "CalendarColumnReference",
        "CalendarColumnGroupID:I ColumnID:I IsPrimaryColumn:I ModifiedTime:I",
    ),
    (
        "Function",
        "ModelID:I Name:T Description:T Expression:T IsHidden:I State:I ErrorMessage:T ModifiedTime:I StructureModifiedTime:I LineageTag:T SourceLineageTag:T",
    ),
    (
        "BindingInfo",
        "ModelID:I Name:T Description:T Type:I ConnectionId:T",
    ),
    (
        "DeltaTableColumnStorage",
        "ColumnStorageID:I UncompressedSize:I MinCardinality:I MaxCardinality:I",
    ),
    (
        "StringIndexStorage",
        "ColumnStorageID:I StorageFileID:I DataVersion:I ImplementationVersion:I SourceDictionaryDataVersion:I",
    ),
    (
        "ColumnIndexStorage",
        "ColumnStorageID:I StorageFileID:I DataVersion:I DeltaVersion:I LastId:I IsNullable:I IsOperatingOn32:I",
    ),
    (
        "FullTextIndexStorage",
        "ColumnStorageID:I StorageFileID:I DataVersion:I ImplementationVersion:I SourceDictionaryDataVersion:I",
    ),
];

/// `DBPROPERTIES` of a new database (rowid, name, value), as the engine writes them right after
/// creating it: the rowids differ with the model's Power BI data source version, which
/// `DATASOURCEVERSION` repeats (`datamodel-fresh.md`, "metadata.sqlitedb").
fn db_properties(max_id: i64, source_version: i64, now: i64) -> Vec<(i64, &'static str, i64)> {
    let mut out = vec![
        (1, "MAXID", max_id),
        (2, "SCHEMAVERSION", 123),
        (3, "LASTSCHEMAVERSION", 123),
        (5, "DATABASEBLOCKINGSTATE", 0),
        (6, "RESTOREABFSOURCE", 0),
        (7, "LASTCUBEDATACHANGE", now),
        (8, "LASTCUBESCHEMACHANGE", now),
    ];
    let markers = if source_version == 0 {
        out.push((4, "DATASOURCEVERSION", 0));
        15
    } else {
        out.push((12, "DATASOURCEVERSION", source_version));
        16
    };
    out.push((markers, "LASTCOMMITTED", now));
    out.push((markers + 1, "LASTMETADATACHANGEFORSYNCCOMMITTED", now));
    out.push((markers + 2, "LASTPBIPUBLICXMLACOMMITTEDTIME", 0));
    out.sort_by_key(|p| p.0);
    out
}

/// The `CREATE TABLE` text the engine writes for a TMSCHEMA table.
fn create_sql(name: &str, cols: &str) -> String {
    let mut parts = vec!["[ID] INTEGER".to_string()];
    for c in cols.split_whitespace() {
        let (n, t) = c.rsplit_once(':').unwrap_or((c, "I"));
        let ty = match t {
            "T" => "TEXT",
            "R" => "REAL",
            "B" => "BLOB",
            _ => "INTEGER",
        };
        parts.push(format!("[{n}] {ty}"));
    }
    parts.push("PRIMARY KEY(\"ID\" ASC)".into());
    format!("CREATE TABLE [{name}]( {} )", parts.join(" ,  "))
}

/// One new row: values by column name; every other column is NULL.
type Cells = Vec<(&'static str, Value)>;

fn int(v: i64) -> Value {
    Value::Integer(v)
}

fn text(s: &str) -> Value {
    Value::Text(s.to_string())
}

fn opt_text(s: Option<&str>) -> Value {
    s.map_or(Value::Null, text)
}

/// Rows of a database being built, by table.
struct Rows {
    next_id: i64,
    rows: HashMap<&'static str, Vec<(i64, Cells)>>,
}

impl Rows {
    fn add(&mut self, table: &'static str, mut cells: Cells) -> Result<i64, Error> {
        let id = self.next_id;
        self.next_id = id
            .checked_add(1)
            .ok_or_else(|| Error::Invalid("object IDs overflow".into()))?;
        cells.retain(|(c, _)| *c != "ID");
        self.rows.entry(table).or_default().push((id, cells));
        Ok(id)
    }
}

fn unsupported(object: &str, what: impl Into<String>) -> UnsupportedChange {
    UnsupportedChange {
        what: what.into(),
        object: object.to_string(),
    }
}

/// Reads the model-level properties of a model, collecting what cannot be written.
struct Props<'m> {
    model: &'m SemanticModel,
    bad: Vec<UnsupportedChange>,
}

impl Props<'_> {
    /// An enumeration property's code, from its TMSL spelling (`default` when absent).
    fn code(&mut self, key: &str, f: fn(i64) -> Option<&'static str>, default: i64) -> Value {
        let v = match self.model.extra.get(key) {
            None | Some(Json::Null) => default,
            Some(Json::String(s)) => match code_of(f, s) {
                Some(c) => c,
                None => {
                    self.bad
                        .push(unsupported("model", format!("unknown `{key}` value `{s}`")));
                    default
                }
            },
            Some(v) => {
                self.bad
                    .push(unsupported("model", format!("`{key}` is not a string: {v}")));
                default
            }
        };
        int(v)
    }

    fn number(&mut self, key: &str, default: i64) -> Value {
        let v = match self.model.extra.get(key) {
            None | Some(Json::Null) => default,
            Some(v) => match v.as_i64() {
                Some(n) => n,
                None => {
                    self.bad
                        .push(unsupported("model", format!("`{key}` is not an integer: {v}")));
                    default
                }
            },
        };
        int(v)
    }

    fn flag(&self, key: &str) -> Value {
        int(i64::from(
            self.model.extra.get(key).and_then(Json::as_bool).unwrap_or(false),
        ))
    }

    fn text(&self, key: &str) -> Value {
        opt_text(extra_text(&self.model.extra, key))
    }

    /// JSON kept as text (`dataAccessOptions`, `automaticAggregationOptions`), indented with
    /// two spaces as the engine stores it.
    fn json(&self, key: &str) -> Value {
        match self.model.extra.get(key) {
            None | Some(Json::Null) => Value::Null,
            Some(Json::String(s)) => text(s),
            Some(v) => Value::Text(serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())),
        }
    }
}

fn extra_text<'m>(extra: &'m bicraft_model::Properties, key: &str) -> Option<&'m str> {
    extra.get(key).and_then(Json::as_str)
}

/// The `Model` row of `model` (see `docs/formats/datamodel-fresh.md`, "Model row").
fn model_cells(model: &SemanticModel, now: i64, bad: &mut Vec<UnsupportedChange>) -> Cells {
    if model.extra.contains_key("defaultMeasure") {
        bad.push(unsupported(
            "model",
            "`defaultMeasure` cannot be set on a model built from scratch (set it after the first save)",
        ));
    }
    let mut p = Props {
        model,
        bad: Vec::new(),
    };
    let cells = vec![
        ("Name", text(extra_text(&model.extra, "name").unwrap_or("Model"))),
        ("Description", opt_text(model.description.as_deref())),
        ("StorageLocation", p.text("storageLocation")),
        ("DefaultMode", p.code("defaultMode", codes::mode, 0)),
        ("DefaultDataView", p.code("defaultDataView", codes::data_view, 0)),
        ("Culture", opt_text(model.culture.as_deref())),
        ("Collation", p.text("collation")),
        ("ModifiedTime", int(now)),
        ("StructureModifiedTime", int(now)),
        ("DataAccessOptions", p.json("dataAccessOptions")),
        ("DefaultMeasureID", int(0)),
        (
            "DefaultPowerBIDataSourceVersion",
            p.code(
                "defaultPowerBIDataSourceVersion",
                codes::power_bi_data_source_version,
                0,
            ),
        ),
        ("ForceUniqueNames", p.flag("forceUniqueNames")),
        ("DiscourageImplicitMeasures", p.flag("discourageImplicitMeasures")),
        ("DiscourageReportMeasures", p.flag("discourageReportMeasures")),
        (
            "DataSourceVariablesOverrideBehavior",
            p.code(
                "dataSourceVariablesOverrideBehavior",
                codes::data_source_variables_override,
                0,
            ),
        ),
        (
            "DataSourceDefaultMaxConnections",
            p.number("dataSourceDefaultMaxConnections", 10),
        ),
        ("SourceQueryCulture", p.text("sourceQueryCulture")),
        ("MAttributes", p.text("mAttributes")),
        ("DiscourageCompositeModels", p.flag("discourageCompositeModels")),
        (
            "AutomaticAggregationOptions",
            p.json("automaticAggregationOptions"),
        ),
        ("DisableAutoExists", p.number("disableAutoExists", -1)),
        (
            "MaxParallelismPerRefresh",
            p.number("maxParallelismPerRefresh", -1),
        ),
        ("MaxParallelismPerQuery", p.number("maxParallelismPerQuery", 0)),
        (
            "DisableSystemDefaultExpression",
            p.flag("disableSystemDefaultExpression"),
        ),
        (
            "DirectLakeBehavior",
            p.code("directLakeBehavior", codes::direct_lake_behavior, 0),
        ),
        (
            "ValueFilterBehavior",
            p.code("valueFilterBehavior", codes::value_filter_behavior, 0),
        ),
        (
            "SelectionExpressionBehavior",
            p.code(
                "selectionExpressionBehavior",
                codes::selection_expression_behavior,
                0,
            ),
        ),
        (
            "MetadataAccessPolicy",
            p.code("metadataAccessPolicy", codes::metadata_category, 0),
        ),
        (
            "DefaultDirectLakeIndexingBehavior",
            p.code(
                "defaultDirectLakeIndexingBehavior",
                codes::direct_lake_indexing_behavior,
                0,
            ),
        ),
    ];
    bad.append(&mut p.bad);
    cells
}

/// Writes a new `metadata.sqlitedb` holding the model-level part of `model` (module docs);
/// `now`: Windows FILETIME ticks for every time column and commit marker.
pub fn fresh_sqlitedb_at(model: &SemanticModel, now: i64) -> Result<Vec<u8>, Error> {
    let mut bad = Vec::new();
    let mut rows = Rows {
        next_id: 1,
        rows: HashMap::new(),
    };
    let model_id = rows.add("Model", model_cells(model, now, &mut bad))?;

    // Query groups: those the model lists, then any an expression names.
    let mut groups: Vec<(String, Option<String>)> = Vec::new();
    for g in model
        .extra
        .get("queryGroups")
        .and_then(Json::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(folder) = g.get("folder").and_then(Json::as_str) {
            let description = g.get("description").and_then(Json::as_str).map(str::to_string);
            groups.push((folder.to_string(), description));
        }
    }
    for e in &model.expressions {
        if let Some(f) = extra_text(&e.extra, "queryGroup")
            && !groups.iter().any(|(g, _)| g == f)
        {
            groups.push((f.to_string(), None));
        }
    }
    let mut group_ids: HashMap<String, i64> = HashMap::new();
    for (folder, description) in &groups {
        let id = rows.add(
            "QueryGroup",
            vec![
                ("ModelID", int(model_id)),
                ("Folder", text(folder)),
                ("Description", opt_text(description.as_deref())),
            ],
        )?;
        group_ids.insert(folder.clone(), id);
    }

    // Shared expressions (M queries and parameters), in model order.
    let mut expression_ids: HashMap<String, i64> = HashMap::new();
    let mut sources: Vec<(usize, String)> = Vec::new();
    for e in &model.expressions {
        let object = format!("expression '{}'", e.name);
        if expression_ids.contains_key(&e.name) {
            bad.push(unsupported(&object, "duplicate expression name"));
            continue;
        }
        let kind = match code_of(codes::expression_kind, &e.kind) {
            Some(k) => k,
            None if e.kind.is_empty() => 0,
            None => {
                bad.push(unsupported(&object, format!("unknown kind `{}`", e.kind)));
                0
            }
        };
        if e.extra.contains_key("parameterValuesColumn") {
            bad.push(unsupported(
                &object,
                "`parameterValuesColumn` cannot be set on a model built from scratch",
            ));
        }
        let group = extra_text(&e.extra, "queryGroup")
            .and_then(|g| group_ids.get(g))
            .copied()
            .unwrap_or(0);
        let pos = rows.rows.get("Expression").map_or(0, Vec::len);
        let id = rows.add(
            "Expression",
            vec![
                ("ModelID", int(model_id)),
                ("Name", text(&e.name)),
                ("Description", opt_text(e.description.as_deref())),
                ("Kind", int(kind)),
                ("Expression", text(&e.expression)),
                ("ModifiedTime", int(now)),
                ("QueryGroupID", int(group)),
                ("ParameterValuesColumnID", int(0)),
                ("MAttributes", opt_text(extra_text(&e.extra, "mAttributes"))),
                ("LineageTag", opt_text(e.lineage_tag.as_deref())),
                (
                    "SourceLineageTag",
                    opt_text(extra_text(&e.extra, "sourceLineageTag")),
                ),
                (
                    "RemoteParameterName",
                    opt_text(extra_text(&e.extra, "remoteParameterName")),
                ),
                ("ExpressionSourceID", int(0)),
            ],
        )?;
        expression_ids.insert(e.name.clone(), id);
        if let Some(src) = extra_text(&e.extra, "expressionSource") {
            sources.push((pos, src.to_string()));
        }
    }
    // `expressionSource` names another expression, which may come later.
    for (pos, src) in sources {
        let Some(target) = expression_ids.get(&src).copied() else {
            bad.push(unsupported(
                "model",
                format!("expression source `{src}` is not an expression of the model"),
            ));
            continue;
        };
        if let Some((_, cells)) = rows.rows.get_mut("Expression").and_then(|r| r.get_mut(pos)) {
            for (c, v) in cells.iter_mut() {
                if *c == "ExpressionSourceID" {
                    *v = int(target);
                }
            }
        }
    }
    if !bad.is_empty() {
        return Err(Error::UnsupportedChange(bad));
    }

    let max_id = rows.next_id.saturating_sub(1);
    let fail = |e: bicraft_sqlite::Error| Error::Invalid(format!("new metadata.sqlitedb: {e}"));
    let mut b = DatabaseBuilder::new(4096).map_err(fail)?;
    b.create_table("DBPROPERTIES", DB_PROPERTIES_SQL).map_err(fail)?;
    for (name, cols) in SCHEMA {
        b.create_table(name, &create_sql(name, cols)).map_err(fail)?;
    }
    let source_version = model
        .extra
        .get("defaultPowerBIDataSourceVersion")
        .and_then(Json::as_str)
        .and_then(|v| code_of(codes::power_bi_data_source_version, v))
        .unwrap_or(0);
    for (rowid, name, value) in db_properties(max_id, source_version, now) {
        b.insert("DBPROPERTIES", rowid, vec![text(name), int(value)])
            .map_err(fail)?;
    }
    for (name, cols) in SCHEMA {
        let Some(list) = rows.rows.get(name) else {
            continue;
        };
        let columns: Vec<&str> = cols
            .split_whitespace()
            .map(|c| c.rsplit_once(':').map_or(c, |(n, _)| n))
            .collect();
        for (id, cells) in list {
            let mut values = Vec::with_capacity(columns.len().saturating_add(1));
            values.push(int(*id));
            for col in &columns {
                let v = cells
                    .iter()
                    .find(|(c, _)| c == col)
                    .map_or(Value::Null, |(_, v)| v.clone());
                values.push(v);
            }
            b.insert(name, *id, values).map_err(fail)?;
        }
    }
    b.finish().map_err(fail)
}

/// The `CREATE` text of `DBPROPERTIES`.
const DB_PROPERTIES_SQL: &str =
    "CREATE TABLE [DBPROPERTIES] ([NAME] STRING, [VALUE] INTEGER, PRIMARY KEY ([NAME] ASC))";

#[cfg(test)]
#[allow(clippy::indexing_slicing)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: i64 = 134_000_000_000_000_000;

    fn model() -> SemanticModel {
        let mut m = SemanticModel {
            culture: Some("fr-FR".into()),
            ..SemanticModel::default()
        };
        m.extra
            .insert("defaultPowerBIDataSourceVersion".into(), json!("powerBI_V3"));
        m.extra.insert("sourceQueryCulture".into(), json!("fr-FR"));
        m.extra.insert(
            "dataAccessOptions".into(),
            json!({"legacyRedirects": true, "returnErrorValuesAsNull": true}),
        );
        m.extra.insert("discourageImplicitMeasures".into(), json!(true));
        m.extra.insert(
            "queryGroups".into(),
            json!([{"folder": "Params", "description": "d"}]),
        );
        m.expressions.push(bicraft_model::NamedExpression {
            name: "Rate".into(),
            kind: "m".into(),
            expression: "0.2 meta [IsParameterQuery=true, Type=\"Number\", IsParameterQueryRequired=true]"
                .into(),
            lineage_tag: Some("7d1f4a2e-6c3b-4b8a-9e1d-2f0c5a7b3e91".into()),
            extra: [("queryGroup".to_string(), json!("Params"))]
                .into_iter()
                .collect(),
            ..Default::default()
        });
        m.expressions.push(bicraft_model::NamedExpression {
            name: "Copy".into(),
            kind: "m".into(),
            expression: "Rate".into(),
            ..Default::default()
        });
        m
    }

    #[test]
    fn model_level_rows_read_back() {
        let bytes = fresh_sqlitedb_at(&model(), NOW).unwrap();
        let back = crate::read_sqlitedb(&bytes).unwrap();
        let m = model();
        assert_eq!(back.culture, m.culture);
        for key in [
            "defaultPowerBIDataSourceVersion",
            "sourceQueryCulture",
            "dataAccessOptions",
            "discourageImplicitMeasures",
            "queryGroups",
        ] {
            assert_eq!(back.extra.get(key), m.extra.get(key), "{key}");
        }
        assert_eq!(back.extra.get("name"), Some(&json!("Model")));
        assert_eq!(back.expressions.len(), 2);
        assert_eq!(back.expressions[0].expression, m.expressions[0].expression);
        assert_eq!(
            back.expressions[0].extra.get("queryGroup"),
            Some(&json!("Params"))
        );
        assert_eq!(back.expressions[0].lineage_tag, m.expressions[0].lineage_tag);
        assert!(back.tables.is_empty());
        // The schema is the engine's: every TMSCHEMA table, DBPROPERTIES markers at `now`.
        let db = bicraft_sqlite::Database::from_bytes(&bytes).unwrap();
        assert_eq!(db.tables().len(), SCHEMA.len() + 1);
        let props = db.query_all("DBPROPERTIES").unwrap();
        let get = |n: &str| {
            props
                .iter()
                .find(|r| r.get("NAME").and_then(|v| v.as_str()) == Some(n))
                .and_then(|r| r.get("VALUE").and_then(bicraft_sqlite::Value::as_i64))
        };
        assert_eq!(get("MAXID"), Some(4));
        assert_eq!(get("LASTCOMMITTED"), Some(NOW));
        // A metadata-only edit works on it like on an engine-written file.
        let mut edited = back.clone();
        edited.description = Some("described".into());
        let again = crate::write_sqlitedb_at(&bytes, &edited, NOW + 1).unwrap();
        assert_eq!(
            crate::read_sqlitedb(&again).unwrap().description.as_deref(),
            Some("described")
        );
    }

    #[test]
    fn unresolvable_references_are_refused() {
        let mut m = model();
        m.extra
            .insert("defaultMeasure".into(), json!({"table": "T", "measure": "M"}));
        m.expressions[1]
            .extra
            .insert("expressionSource".into(), json!("Nope"));
        m.extra.insert("defaultMode".into(), json!("sideways"));
        let Err(Error::UnsupportedChange(list)) = fresh_sqlitedb_at(&m, NOW) else {
            panic!("expected a refusal");
        };
        assert_eq!(list.len(), 3, "{list:?}");
    }

    #[test]
    fn create_text_matches_the_engine() {
        assert_eq!(
            create_sql(
                "StorageFile",
                "OwnerID:I OwnerType:I StorageFolderID:I FileName:T"
            ),
            "CREATE TABLE [StorageFile]( [ID] INTEGER ,  [OwnerID] INTEGER ,  [OwnerType] INTEGER ,  \
             [StorageFolderID] INTEGER ,  [FileName] TEXT ,  PRIMARY KEY(\"ID\" ASC) )"
        );
    }
}
