//! Numeric enumeration codes of the TMSCHEMA objects → TMSL spellings.
//!
//! Codes come from [MS-SSAS-T] §2.2.5 (and, where the spec is silent, from the public TOM enum
//! reference); see `docs/formats/metadata-sqlitedb.md`. Unknown codes map to `None` so callers
//! leave the property out instead of inventing a value.

use bicraft_model::DataType;

/// `ObjectType` codes used by `Annotation`, `ExtendedProperty` and `ChangedProperty` rows.
pub(crate) mod object_type {
    pub(crate) const MODEL: i64 = 1;
    pub(crate) const DATA_SOURCE: i64 = 2;
    pub(crate) const TABLE: i64 = 3;
    pub(crate) const COLUMN: i64 = 4;
    pub(crate) const PARTITION: i64 = 6;
    pub(crate) const RELATIONSHIP: i64 = 7;
    pub(crate) const MEASURE: i64 = 8;
    pub(crate) const HIERARCHY: i64 = 9;
    pub(crate) const LEVEL: i64 = 10;
    pub(crate) const KPI: i64 = 12;
    pub(crate) const CULTURE: i64 = 13;
    pub(crate) const LINGUISTIC_METADATA: i64 = 15;
    pub(crate) const PERSPECTIVE: i64 = 29;
    pub(crate) const PERSPECTIVE_TABLE: i64 = 30;
    pub(crate) const PERSPECTIVE_COLUMN: i64 = 31;
    pub(crate) const PERSPECTIVE_HIERARCHY: i64 = 32;
    pub(crate) const PERSPECTIVE_MEASURE: i64 = 33;
    pub(crate) const ROLE: i64 = 34;
    pub(crate) const ROLE_MEMBERSHIP: i64 = 35;
    pub(crate) const TABLE_PERMISSION: i64 = 36;
    pub(crate) const VARIATION: i64 = 37;
    pub(crate) const SET: i64 = 38;
    pub(crate) const PERSPECTIVE_SET: i64 = 39;
    pub(crate) const EXPRESSION: i64 = 41;
    pub(crate) const COLUMN_PERMISSION: i64 = 42;
    pub(crate) const DETAIL_ROWS_DEFINITION: i64 = 43;
    pub(crate) const RELATED_COLUMN_DETAILS: i64 = 44;
    pub(crate) const GROUP_BY_COLUMN: i64 = 45;
    pub(crate) const CALCULATION_GROUP: i64 = 46;
    pub(crate) const CALCULATION_ITEM: i64 = 47;
    pub(crate) const ALTERNATE_OF: i64 = 48;
    pub(crate) const REFRESH_POLICY: i64 = 49;
    pub(crate) const FORMAT_STRING_DEFINITION: i64 = 50;
    pub(crate) const QUERY_GROUP: i64 = 51;
    pub(crate) const DATA_COVERAGE_DEFINITION: i64 = 58;
    pub(crate) const CALCULATION_EXPRESSION: i64 = 59;
    pub(crate) const CALENDAR: i64 = 60;
    pub(crate) const CALENDAR_COLUMN_GROUP: i64 = 61;
    pub(crate) const CALENDAR_COLUMN_REFERENCE: i64 = 62;
    pub(crate) const FUNCTION: i64 = 63;
    pub(crate) const BINDING_INFO: i64 = 64;
}

/// Model `SelectionExpressionBehavior` (*TOM*; 1 confirmed on the engine). `0` is the default.
pub(crate) fn selection_expression_behavior(code: i64) -> Option<&'static str> {
    Some(match code {
        0 => "automatic",
        1 => "visual",
        2 => "nonVisual",
        _ => return None,
    })
}

/// Model `DataSourceVariablesOverrideBehavior`. `0` is the default.
pub(crate) fn data_source_variables_override(code: i64) -> Option<&'static str> {
    Some(match code {
        0 => "disallow",
        1 => "allow",
        _ => return None,
    })
}

/// CalendarColumnGroup `TimeUnit`. `0` (unknown) marks a group of plain time-related columns.
pub(crate) fn time_unit(code: i64) -> Option<&'static str> {
    Some(match code {
        0 => "unknown",
        1 => "year",
        2 => "semester",
        3 => "semesterOfYear",
        4 => "quarter",
        5 => "quarterOfYear",
        6 => "quarterOfSemester",
        7 => "month",
        8 => "monthOfYear",
        9 => "monthOfSemester",
        10 => "monthOfQuarter",
        11 => "week",
        12 => "weekOfYear",
        13 => "weekOfSemester",
        14 => "weekOfQuarter",
        15 => "weekOfMonth",
        16 => "date",
        17 => "dayOfYear",
        18 => "dayOfSemester",
        19 => "dayOfQuarter",
        20 => "dayOfMonth",
        21 => "dayOfWeek",
        _ => return None,
    })
}

/// RefreshPolicy `RollingWindowGranularity` / `IncrementalGranularity`, Partition
/// `RangeGranularity` (`-1`: none).
pub(crate) fn refresh_granularity(code: i64) -> Option<&'static str> {
    Some(match code {
        0 => "day",
        1 => "month",
        2 => "quarter",
        3 => "year",
        _ => return None,
    })
}

/// RefreshPolicy `PolicyType`.
pub(crate) fn refresh_policy_type(code: i64) -> Option<&'static str> {
    Some(match code {
        0 => "basic",
        _ => return None,
    })
}

/// RefreshPolicy `Mode`. `0` is the default.
pub(crate) fn refresh_policy_mode(code: i64) -> Option<&'static str> {
    Some(match code {
        0 => "import",
        1 => "hybrid",
        _ => return None,
    })
}

/// AlternateOf `Summarization`. `0` is the default.
pub(crate) fn summarization(code: i64) -> Option<&'static str> {
    Some(match code {
        0 => "groupBy",
        1 => "sum",
        2 => "count",
        3 => "min",
        4 => "max",
        _ => return None,
    })
}

/// DataSource `Type`. `1` (provider) is the default.
pub(crate) fn data_source_type(code: i64) -> Option<&'static str> {
    Some(match code {
        1 => "provider",
        2 => "structured",
        _ => return None,
    })
}

/// DataSource `ImpersonationMode`. `1` is the default.
pub(crate) fn impersonation_mode(code: i64) -> Option<&'static str> {
    Some(match code {
        1 => "default",
        2 => "impersonateAccount",
        3 => "impersonateAnonymous",
        4 => "impersonateCurrentUser",
        5 => "impersonateServiceAccount",
        6 => "impersonateUnattendedAccount",
        _ => return None,
    })
}

/// DataSource `Isolation`. `1` is the default.
pub(crate) fn isolation(code: i64) -> Option<&'static str> {
    Some(match code {
        1 => "readCommitted",
        2 => "snapshot",
        _ => return None,
    })
}

/// ObjectTranslation `Property` → the TMSL key of the translated value.
pub(crate) fn translated_property(code: i64) -> Option<&'static str> {
    Some(match code {
        1 => "translatedCaption",
        2 => "translatedDescription",
        3 => "translatedDisplayFolder",
        _ => return None,
    })
}

/// Column `ExplicitDataType` / `InferredDataType`, Measure `DataType`. `1` (automatic) and `19`
/// (unknown) give `Unknown`.
pub(crate) fn data_type(code: i64) -> DataType {
    match code {
        2 => DataType::String,
        6 => DataType::Int64,
        8 => DataType::Double,
        9 => DataType::DateTime,
        10 => DataType::Decimal,
        11 => DataType::Boolean,
        17 => DataType::Binary,
        20 => DataType::Variant,
        _ => DataType::Unknown,
    }
}

/// `ExplicitDataType` value meaning "infer the type from the expression".
pub(crate) const DATA_TYPE_AUTOMATIC: i64 = 1;

/// Column `Type`.
pub(crate) mod column_type {
    pub(crate) const DATA: i64 = 1;
    pub(crate) const CALCULATED: i64 = 2;
    pub(crate) const ROW_NUMBER: i64 = 3;
    pub(crate) const CALCULATED_TABLE_COLUMN: i64 = 4;
}

pub(crate) fn column_type(code: i64) -> Option<&'static str> {
    Some(match code {
        column_type::DATA => "data",
        column_type::CALCULATED => "calculated",
        column_type::ROW_NUMBER => "rowNumber",
        column_type::CALCULATED_TABLE_COLUMN => "calculatedTableColumn",
        _ => return None,
    })
}

/// Column `SummarizeBy`. `1` is the default.
pub(crate) fn summarize_by(code: i64) -> Option<&'static str> {
    Some(match code {
        1 => "default",
        2 => "none",
        3 => "sum",
        4 => "min",
        5 => "max",
        6 => "count",
        7 => "average",
        8 => "distinctCount",
        _ => return None,
    })
}

/// Column `Alignment`. `1` is the default.
pub(crate) fn alignment(code: i64) -> Option<&'static str> {
    Some(match code {
        1 => "default",
        2 => "left",
        3 => "right",
        4 => "center",
        _ => return None,
    })
}

/// Column `EncodingHint`. `0` is the default.
pub(crate) fn encoding_hint(code: i64) -> Option<&'static str> {
    Some(match code {
        0 => "default",
        1 => "hash",
        2 => "value",
        _ => return None,
    })
}

/// Partition `Type` codes whose source is not a plain expression.
pub(crate) mod partition_type {
    pub(crate) const QUERY: i64 = 1;
    pub(crate) const ENTITY: i64 = 5;
    pub(crate) const POLICY_RANGE: i64 = 6;
    pub(crate) const PARQUET: i64 = 9;
}

/// Partition `Type` → `source.type`.
pub(crate) fn partition_source_type(code: i64) -> Option<&'static str> {
    Some(match code {
        1 => "query",
        2 => "calculated",
        3 => "none",
        4 => "m",
        5 => "entity",
        6 => "policyRange",
        7 => "calculationGroup",
        8 => "inferred",
        // TOM `ParquetPartitionSource`; TOM gates it behind an unreleased compatibility level,
        // so no engine has written one yet.
        9 => "parquet",
        _ => return None,
    })
}

/// Partition `Mode` and Model `DefaultMode`. `2` (default) is a partition's default, `0`
/// (import) the model's.
pub(crate) fn mode(code: i64) -> Option<&'static str> {
    Some(match code {
        0 => "import",
        1 => "directQuery",
        2 => "default",
        3 => "push",
        4 => "dual",
        5 => "directLake",
        _ => return None,
    })
}

/// Partition `DataView` (default `3`) and Model `DefaultDataView` (default `0`).
pub(crate) fn data_view(code: i64) -> Option<&'static str> {
    Some(match code {
        0 => "full",
        1 => "sample",
        3 => "default",
        _ => return None,
    })
}

/// Relationship `CrossFilteringBehavior`. `1` is the default.
pub(crate) fn cross_filtering(code: i64) -> Option<&'static str> {
    Some(match code {
        1 => "oneDirection",
        2 => "bothDirections",
        3 => "automatic",
        _ => return None,
    })
}

/// Relationship `SecurityFilteringBehavior`. `1` is the default.
pub(crate) fn security_filtering(code: i64) -> Option<&'static str> {
    Some(match code {
        1 => "oneDirection",
        2 => "bothDirections",
        3 => "none",
        _ => return None,
    })
}

/// Relationship `FromCardinality` (default many) / `ToCardinality` (default one).
pub(crate) fn cardinality(code: i64) -> Option<&'static str> {
    Some(match code {
        1 => "one",
        2 => "many",
        _ => return None,
    })
}

/// Relationship `JoinOnDateBehavior`. `1` is the default.
pub(crate) fn join_on_date(code: i64) -> Option<&'static str> {
    Some(match code {
        1 => "dateAndTime",
        2 => "datePartOnly",
        _ => return None,
    })
}

/// Role `ModelPermission`.
pub(crate) fn model_permission(code: i64) -> Option<&'static str> {
    Some(match code {
        1 => "none",
        2 => "read",
        3 => "readRefresh",
        4 => "refresh",
        5 => "administrator",
        _ => return None,
    })
}

/// TablePermission / ColumnPermission `MetadataPermission`. `0` is the default.
pub(crate) fn metadata_permission(code: i64) -> Option<&'static str> {
    Some(match code {
        0 => "default",
        1 => "none",
        2 => "read",
        _ => return None,
    })
}

/// RoleMembership `MemberType`. `1` is the default.
pub(crate) fn member_type(code: i64) -> Option<&'static str> {
    Some(match code {
        1 => "auto",
        2 => "user",
        3 => "group",
        _ => return None,
    })
}

/// Hierarchy `HideMembers`. `0` is the default.
pub(crate) fn hide_members(code: i64) -> Option<&'static str> {
    Some(match code {
        0 => "default",
        1 => "hideBlankMembers",
        _ => return None,
    })
}

/// Expression `Kind`.
pub(crate) fn expression_kind(code: i64) -> Option<&'static str> {
    Some(match code {
        0 => "m",
        _ => return None,
    })
}

/// Model `DefaultPowerBIDataSourceVersion`. `0` is the default.
pub(crate) fn power_bi_data_source_version(code: i64) -> Option<&'static str> {
    Some(match code {
        0 => "powerBI_V1",
        1 => "powerBI_V2",
        2 => "powerBI_V3",
        _ => return None,
    })
}

/// Model `ValueFilterBehavior`. `0` is the default.
pub(crate) fn value_filter_behavior(code: i64) -> Option<&'static str> {
    Some(match code {
        0 => "automatic",
        1 => "independent",
        2 => "coalesced",
        _ => return None,
    })
}

/// Model `DirectLakeBehavior`. `0` is the default.
pub(crate) fn direct_lake_behavior(code: i64) -> Option<&'static str> {
    Some(match code {
        0 => "automatic",
        1 => "directLakeOnly",
        2 => "directQueryOnly",
        _ => return None,
    })
}

/// Model `DefaultDirectLakeIndexingBehavior` (default `0`) and Table
/// `DirectLakeIndexingBehavior` (default `3`). *TOM*; 1, 2 confirmed on the engine.
pub(crate) fn direct_lake_indexing_behavior(code: i64) -> Option<&'static str> {
    Some(match code {
        0 => "auto",
        1 => "explicit",
        2 => "full",
        3 => "default",
        _ => return None,
    })
}

/// Model `MetadataAccessPolicy` (TOM `MetadataCategory`). `0` is the default; 2 confirmed.
pub(crate) fn metadata_category(code: i64) -> Option<&'static str> {
    Some(match code {
        0 => "inherited",
        1 => "basic",
        2 => "calculationDefinitions",
        _ => return None,
    })
}

/// BindingInfo `Type`. 1 confirmed on the engine.
pub(crate) fn binding_info_type(code: i64) -> Option<&'static str> {
    Some(match code {
        0 => "unknown",
        1 => "dataBindingHint",
        _ => return None,
    })
}

/// ExtendedProperty `Type`.
pub(crate) const EXTENDED_PROPERTY_JSON: i64 = 1;

/// LinguisticMetadata `ContentType`.
pub(crate) fn content_type(code: i64) -> Option<&'static str> {
    Some(match code {
        0 => "xml",
        1 => "json",
        _ => return None,
    })
}

/// The code whose TMSL spelling is `name` (ASCII case-insensitive), searching `-1..=32`.
pub(crate) fn code_of(f: fn(i64) -> Option<&'static str>, name: &str) -> Option<i64> {
    (-1..=32).find(|&c| f(c).is_some_and(|s| s.eq_ignore_ascii_case(name)))
}

/// Measure `DataType` values written for new or changed measures.
pub(crate) mod measure_data_type {
    pub(crate) const STRING: i64 = 2;
    pub(crate) const INT64: i64 = 6;
    pub(crate) const DOUBLE: i64 = 8;
    pub(crate) const BOOLEAN: i64 = 11;
    pub(crate) const VARIANT: i64 = 20;
}
