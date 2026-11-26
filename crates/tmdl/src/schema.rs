//! What the reader and writer know about TMDL object types: how a keyword maps to its TMSL
//! collection, which property `= value` sets, how siblings are separated, and the order in which
//! Power BI Desktop writes properties and child objects.
//!
//! The orders were observed from the TMDL serializer Desktop ships (see `docs/formats/tmdl.md`).
//! Keys not listed keep their source order: unknown properties go to the `*` slot of `props`
//! (the end when there is none), unknown child collections to the `*` slot of `children`.

/// How an object is separated from its siblings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Style {
    /// A child object: a blank line before it.
    Block,
    /// A blank line before the first of a run of siblings of the same kind only (`member`).
    Grouped,
}

/// How the text after `=` maps to the default property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DefaultMode {
    /// Plain text or an expression.
    Text,
    /// JSON when the text is pretty-printed JSON, text otherwise (`linguisticMetadata`,
    /// `extendedProperty`).
    Json,
}

#[derive(Debug)]
pub(crate) struct Kind {
    /// TMDL keyword, canonical casing.
    pub keyword: &'static str,
    /// TMSL key in the parent object.
    pub key: &'static str,
    /// A single nested object rather than an array of them.
    pub single: bool,
    /// TMSL property holding the object's name (`None` for nameless objects).
    pub name_key: Option<&'static str>,
    /// TMSL property set by `= value`.
    pub default_key: Option<&'static str>,
    pub default_mode: DefaultMode,
    pub style: Style,
    /// Property order.
    pub props: &'static [&'static str],
    /// Child keyword order.
    pub children: &'static [&'static str],
}

const TAIL: &[&str] = &["*", "extendedProperty", "changedProperty", "annotation"];

const fn kind(keyword: &'static str, key: &'static str) -> Kind {
    Kind {
        keyword,
        key,
        single: false,
        name_key: Some("name"),
        default_key: None,
        default_mode: DefaultMode::Text,
        style: Style::Block,
        props: &["*"],
        children: TAIL,
    }
}

const fn single(keyword: &'static str, default_key: Option<&'static str>) -> Kind {
    Kind {
        single: true,
        name_key: None,
        default_key,
        ..kind(keyword, keyword)
    }
}

pub(crate) static KINDS: &[Kind] = &[
    Kind {
        props: &["compatibilityLevel", "*"],
        children: &["*"],
        ..kind("database", "")
    },
    Kind {
        props: &[
            "storageLocation",
            "defaultMode",
            "defaultDataView",
            "culture",
            "collation",
            "defaultPowerBIDataSourceVersion",
            "forceUniqueNames",
            "discourageImplicitMeasures",
            "discourageReportMeasures",
            "dataSourceVariablesOverrideBehavior",
            "dataSourceDefaultMaxConnections",
            "sourceQueryCulture",
            "mAttributes",
            "discourageCompositeModels",
            "disableAutoExists",
            "maxParallelismPerRefresh",
            "maxParallelismPerQuery",
            "directLakeBehavior",
            "valueFilterBehavior",
            "selectionExpressionBehavior",
            "metadataAccessPolicy",
            "defaultDirectLakeIndexingBehavior",
            "defaultMeasure",
            // Other scalar properties before the nested blocks: Desktop 2.158 rejects a property
            // after `dataAccessOptions` ("invalid indentation"). TOM writes `dataAccessOptions`
            // before `automaticAggregationOptions`.
            "*",
            "dataAccessOptions",
            "automaticAggregationOptions",
        ],
        children: &[
            "queryGroup",
            "*",
            "extendedProperty",
            "bindingInfo",
            "changedProperty",
            "annotation",
        ],
        ..kind("model", "")
    },
    Kind {
        props: &[
            "isHidden",
            "showAsVariationsOnly",
            "isPrivate",
            "alternateSourcePrecedence",
            "excludeFromModelRefresh",
            "lineageTag",
            "sourceLineageTag",
            "systemManaged",
            "excludeFromAutomaticAggregations",
            "directLakeIndexingBehavior",
            "dataCategory",
            "*",
        ],
        children: &[
            "defaultDetailRowsDefinition",
            "refreshPolicy",
            "calculationGroup",
            "measure",
            "column",
            "hierarchy",
            "partition",
            "calendar",
            "*",
            "extendedProperty",
            "changedProperty",
            "annotation",
        ],
        ..kind("table", "tables")
    },
    Kind {
        default_key: Some("expression"),
        props: &[
            "type",
            "dataType",
            "isDataTypeInferred",
            "isHidden",
            "isUnique",
            "isKey",
            "isNullable",
            "formatString",
            "isAvailableInMdx",
            "keepUniqueRows",
            "displayOrdinal",
            "sourceProviderType",
            "displayFolder",
            "encodingHint",
            "lineageTag",
            "sourceLineageTag",
            "stringIndexingBehavior",
            "fullTextIndexingBehavior",
            "dataCategory",
            "alignment",
            "tableDetailPosition",
            "isDefaultLabel",
            "isDefaultImage",
            "summarizeBy",
            // Calculated columns only (where `isNameInferred` and `sourceColumn` do not exist).
            "expressionContext",
            "isNameInferred",
            "sourceColumn",
            "sortByColumn",
            "*",
        ],
        children: &[
            "relatedColumnDetails",
            "alternateOf",
            "variation",
            "*",
            "extendedProperty",
            "changedProperty",
            "annotation",
        ],
        ..kind("column", "columns")
    },
    Kind {
        default_key: Some("expression"),
        props: &[
            "formatString",
            "isHidden",
            "isSimpleMeasure",
            "displayFolder",
            "lineageTag",
            "sourceLineageTag",
            "dataCategory",
            "*",
        ],
        children: &[
            "kpi",
            "detailRowsDefinition",
            "formatStringDefinition",
            "*",
            "extendedProperty",
            "changedProperty",
            "annotation",
        ],
        ..kind("measure", "measures")
    },
    Kind {
        props: &["mode", "dataView", "queryGroup", "*", "source"],
        children: &[
            "dataCoverageDefinition",
            "*",
            "extendedProperty",
            "changedProperty",
            "annotation",
        ],
        ..kind("partition", "partitions")
    },
    single("dataCoverageDefinition", Some("expression")),
    // A field parameter column's grouping columns, one `groupByColumn: X` line each.
    Kind {
        props: &["groupByColumns", "*"],
        ..single("relatedColumnDetails", None)
    },
    Kind {
        props: &["lineageTag", "sourceLineageTag", "*"],
        children: &[
            "calendarColumnGroup",
            "*",
            "extendedProperty",
            "changedProperty",
            "annotation",
        ],
        ..kind("calendar", "calendars")
    },
    // A calendar's column groups: `= <time unit>` with a primary column and associated columns,
    // or (no time unit) a group of time-related columns.
    Kind {
        key: "calendarColumnGroups",
        single: false,
        props: &["primaryColumn", "associatedColumns", "columns", "*"],
        ..single("calendarColumnGroup", Some("timeUnit"))
    },
    Kind {
        props: &["type", "connectionId", "*"],
        ..kind("bindingInfo", "bindingInfoCollection")
    },
    // G16, compatibility level 1000000 (shapes from the engine's TOM 17.0.87.18; Desktop
    // 2.158's client TOM does not know them, so the TMDL spelling follows TOM's naming rules).
    // Excluded artifacts are nameless; TMSL keeps the collection names as they are.
    Kind {
        name_key: None,
        props: &["artifactType", "reference", "*"],
        children: &["*"],
        ..kind("excludedArtifact", "excludedArtifacts")
    },
    Kind {
        props: &["measureAnalysisDefinition", "*"],
        children: &["*"],
        ..kind("analyticsAIMetadata", "analyticsAIMetadata")
    },
    Kind {
        props: &[
            "isHidden",
            "displayFolder",
            "hideMembers",
            "lineageTag",
            "sourceLineageTag",
            "*",
        ],
        children: &["level", "*", "extendedProperty", "changedProperty", "annotation"],
        ..kind("hierarchy", "hierarchies")
    },
    Kind {
        props: &["ordinal", "lineageTag", "sourceLineageTag", "column", "*"],
        ..kind("level", "levels")
    },
    Kind {
        props: &[
            "targetDescription",
            "targetExpression",
            "targetFormatString",
            "statusGraphic",
            "statusDescription",
            "statusExpression",
            "trendGraphic",
            "trendDescription",
            "trendExpression",
            "*",
        ],
        ..single("kpi", None)
    },
    single("formatStringDefinition", Some("expression")),
    single("detailRowsDefinition", Some("expression")),
    single("defaultDetailRowsDefinition", Some("expression")),
    single("multipleOrEmptySelectionExpression", Some("expression")),
    single("noSelectionExpression", Some("expression")),
    Kind {
        props: &["precedence", "*"],
        children: &[
            "multipleOrEmptySelectionExpression",
            "noSelectionExpression",
            "calculationItem",
            "*",
            "extendedProperty",
            "changedProperty",
            "annotation",
        ],
        ..single("calculationGroup", None)
    },
    Kind {
        default_key: Some("expression"),
        props: &["ordinal", "*"],
        children: &[
            "formatStringDefinition",
            "*",
            "extendedProperty",
            "changedProperty",
            "annotation",
        ],
        ..kind("calculationItem", "calculationItems")
    },
    Kind {
        props: &[
            "policyType",
            "mode",
            "rollingWindowGranularity",
            "rollingWindowPeriods",
            "incrementalGranularity",
            "incrementalPeriods",
            "incrementalPeriodsOffset",
            "pollingExpression",
            "sourceExpression",
            "*",
        ],
        ..single("refreshPolicy", None)
    },
    Kind {
        props: &["summarization", "baseColumn", "baseTable", "*"],
        ..single("alternateOf", None)
    },
    Kind {
        props: &["isDefault", "relationship", "defaultHierarchy", "*"],
        ..kind("variation", "variations")
    },
    Kind {
        default_key: Some("value"),
        children: &["*"],
        ..kind("annotation", "annotations")
    },
    Kind {
        default_key: Some("value"),
        default_mode: DefaultMode::Json,
        children: &["*"],
        ..kind("extendedProperty", "extendedProperties")
    },
    Kind {
        key: "changedProperties",
        single: false,
        children: &["*"],
        ..single("changedProperty", Some("property"))
    },
    Kind {
        name_key: Some("folder"),
        ..kind("queryGroup", "queryGroups")
    },
    Kind {
        default_key: Some("type"),
        props: &[
            "maxConnections",
            "contextExpression",
            "connectionString",
            "impersonationMode",
            "account",
            "password",
            "isolation",
            "timeout",
            "provider",
            "connectionDetails",
            "options",
            "credential",
            "*",
        ],
        ..kind("dataSource", "dataSources")
    },
    Kind {
        default_key: Some("expression"),
        props: &[
            "kind",
            "mAttributes",
            "lineageTag",
            "sourceLineageTag",
            "remoteParameterName",
            "queryGroup",
            "parameterValuesColumn",
            "expressionSource",
            "*",
        ],
        ..kind("expression", "expressions")
    },
    Kind {
        default_key: Some("expression"),
        props: &["isHidden", "lineageTag", "sourceLineageTag", "*"],
        ..kind("function", "functions")
    },
    Kind {
        props: &[
            "isActive",
            "crossFilteringBehavior",
            "joinOnDateBehavior",
            "relyOnReferentialIntegrity",
            "securityFilteringBehavior",
            "fromCardinality",
            "toCardinality",
            "fromColumn",
            "toColumn",
            "*",
        ],
        ..kind("relationship", "relationships")
    },
    Kind {
        props: &["modelPermission", "*"],
        children: &[
            "tablePermission",
            "member",
            "*",
            "extendedProperty",
            "changedProperty",
            "annotation",
        ],
        ..kind("role", "roles")
    },
    Kind {
        default_key: Some("filterExpression"),
        props: &["metadataPermission", "*"],
        children: &[
            "extendedProperty",
            "columnPermission",
            "*",
            "changedProperty",
            "annotation",
        ],
        ..kind("tablePermission", "tablePermissions")
    },
    Kind {
        default_key: Some("metadataPermission"),
        ..kind("columnPermission", "columnPermissions")
    },
    Kind {
        name_key: Some("memberName"),
        default_key: Some("memberType"),
        style: Style::Grouped,
        ..kind("member", "members")
    },
    Kind {
        children: &[
            "perspectiveTable",
            "*",
            "extendedProperty",
            "changedProperty",
            "annotation",
        ],
        ..kind("perspective", "perspectives")
    },
    Kind {
        props: &["includeAll", "*"],
        children: &[
            "perspectiveMeasure",
            "perspectiveColumn",
            "perspectiveHierarchy",
            "*",
            "extendedProperty",
            "changedProperty",
            "annotation",
        ],
        ..kind("perspectiveTable", "tables")
    },
    kind("perspectiveColumn", "columns"),
    kind("perspectiveMeasure", "measures"),
    kind("perspectiveHierarchy", "hierarchies"),
    Kind {
        props: &["translations", "*"],
        children: &[
            "linguisticMetadata",
            "*",
            "extendedProperty",
            "changedProperty",
            "annotation",
        ],
        ..kind("cultureInfo", "cultures")
    },
    Kind {
        default_mode: DefaultMode::Json,
        props: &["contentType", "*"],
        ..single("linguisticMetadata", Some("content"))
    },
];

/// The kind declared by `keyword` (case-insensitive).
pub(crate) fn kind_by_keyword(keyword: &str) -> Option<&'static Kind> {
    KINDS.iter().find(|k| k.keyword.eq_ignore_ascii_case(keyword))
}

/// The child kind of `parent` stored under TMSL key `key`.
pub(crate) fn child_kind(parent: &Kind, key: &str) -> Option<&'static Kind> {
    parent
        .children
        .iter()
        .filter_map(|kw| kind_by_keyword(kw))
        .find(|k| k.key == key)
}

/// Properties whose value is always text, even when it looks like a number or a boolean.
pub(crate) const STRING_KEYS: &[&str] = &[
    "alignment",
    "caption",
    "column",
    "contentType",
    "crossFilteringBehavior",
    "culture",
    "database",
    "dataCategory",
    "dataSource",
    "dataType",
    "dataView",
    "defaultDataView",
    "defaultMode",
    "defaultPowerBIDataSourceVersion",
    "description",
    "displayFolder",
    "encodingHint",
    "entityName",
    "expressionSource",
    "formatString",
    "fromCardinality",
    "fromColumn",
    "hideMembers",
    "identityProvider",
    "incrementalGranularity",
    "joinOnDateBehavior",
    "kind",
    "lineageTag",
    "memberId",
    "memberName",
    "memberType",
    "metadataPermission",
    "mode",
    "modelPermission",
    "name",
    "policyType",
    "protocol",
    "query",
    "queryGroup",
    "relationship",
    "rollingWindowGranularity",
    "schemaName",
    "securityFilteringBehavior",
    "server",
    "sortByColumn",
    "sourceColumn",
    "sourceLineageTag",
    "sourceProviderType",
    "sourceQueryCulture",
    "statusDescription",
    "statusGraphic",
    "summarization",
    "summarizeBy",
    "targetDescription",
    "targetFormatString",
    "toCardinality",
    "toColumn",
    "translatedCaption",
    "translatedDescription",
    "translatedDisplayFolder",
    "trendDescription",
    "trendGraphic",
    "type",
    "value",
];

/// Properties whose value is an object name, quoted like one (`sortByColumn: 'Month Number'`).
pub(crate) const NAME_KEYS: &[&str] = &[
    "baseTable",
    "column",
    "dataSource",
    "expressionSource",
    "primaryColumn",
    "queryGroup",
    "relationship",
    "sortByColumn",
];

/// Properties whose value is a `Table.Object` reference, stored in TMSL as
/// `{"table": ..., "<second key>": ...}`.
pub(crate) const DOTTED_KEYS: &[(&str, &str)] = &[
    ("defaultHierarchy", "hierarchy"),
    ("baseColumn", "column"),
    ("parameterValuesColumn", "column"),
    ("defaultMeasure", "measure"),
];

/// Properties written as expressions (`key = ...`) even on one line.
pub(crate) const EXPRESSION_KEYS: &[&str] = &[
    "contextExpression",
    "expression",
    "filterExpression",
    "pollingExpression",
    "query",
    "sourceExpression",
    "statusExpression",
    "targetExpression",
    "trendExpression",
];

/// Every property name above, to restore canonical casing of keywords read case-insensitively.
pub(crate) fn canonical_key(keyword: &str) -> String {
    let known = KINDS
        .iter()
        .flat_map(|k| {
            k.props
                .iter()
                .chain(std::iter::once(&k.keyword))
                .chain(k.default_key.iter())
        })
        .chain(STRING_KEYS.iter())
        .chain(NAME_KEYS.iter())
        .chain(DOTTED_KEYS.iter().map(|(k, _)| k))
        .chain(EXPRESSION_KEYS.iter())
        .find(|k| k.eq_ignore_ascii_case(keyword));
    known.map_or_else(|| keyword.to_string(), |k| (*k).to_string())
}

/// TMSL collection name for an unknown child keyword: `foo` -> `foos`, `policy` -> `policies`.
pub(crate) fn plural(keyword: &str) -> String {
    match keyword.strip_suffix('y') {
        Some(stem) if !stem.ends_with(['a', 'e', 'i', 'o', 'u']) => format!("{stem}ies"),
        _ => format!("{keyword}s"),
    }
}

/// The inverse of [`plural`].
pub(crate) fn singular(key: &str) -> String {
    if let Some(stem) = key.strip_suffix("ies") {
        format!("{stem}y")
    } else if let Some(stem) = key.strip_suffix('s') {
        stem.to_string()
    } else {
        key.to_string()
    }
}

/// Order of the properties of a partition's `source` block (its `type` is the partition's
/// default property).
pub(crate) const SOURCE_PROPS: &[&str] = &[
    "expression",
    "entityName",
    "schemaName",
    "expressionSource",
    "query",
    "dataSource",
    "start",
    "end",
    "granularity",
    "refreshBookmark",
    "*",
];

/// Property order inside nested property blocks (TMSL objects written as `key` followed by
/// their properties one level deeper), as TOM writes them whatever the TMSL order.
pub(crate) const NESTED_PROPS: &[(&str, &[&str])] = &[
    (
        "dataAccessOptions",
        &["fastCombine", "legacyRedirects", "returnErrorValuesAsNull", "*"],
    ),
    (
        "automaticAggregationOptions",
        &[
            "queryCoverage",
            "detailTableMinRows",
            "aggregationTableMaxRows",
            "aggregationTableSizeLimit",
            "*",
        ],
    ),
];

/// TMSL arrays of names written as one property line per name: (object keyword, TMDL key,
/// TMSL key, field), e.g. a calendar column group's `associatedColumn: Month` lines. With a
/// field, each TMSL item is an object holding the name in that field (a field parameter's
/// `groupByColumn: X` is `{"groupingColumn": X}`).
pub(crate) const LIST_PROPS: &[(&str, &str, &str, Option<&str>)] = &[
    (
        "calendarColumnGroup",
        "associatedColumn",
        "associatedColumns",
        None,
    ),
    ("calendarColumnGroup", "column", "columns", None),
    (
        "relatedColumnDetails",
        "groupByColumn",
        "groupByColumns",
        Some("groupingColumn"),
    ),
];

/// The name a [`LIST_PROPS`] item holds, if it has the expected shape.
pub(crate) fn list_item_name(item: &serde_json::Value, field: Option<&str>) -> Option<String> {
    match field {
        None => item.as_str().map(str::to_string),
        Some(f) => {
            let o = item.as_object()?;
            (o.len() == 1).then_some(())?;
            o.get(f)?.as_str().map(str::to_string)
        }
    }
}

/// TMSL collections TMDL has no syntax for (Desktop's serializer drops them and refuses them on
/// reading): (object keyword, TMSL key). Multidimensional sets are TMSL only.
pub(crate) const TMSL_ONLY: &[(&str, &str)] = &[("table", "sets"), ("perspectiveTable", "sets")];

/// Partition source properties holding a date (`policyRange` partitions): `yyyy-MM-dd` in TMDL,
/// `yyyy-MM-ddT00:00:00` in TMSL.
pub(crate) const DATE_KEYS: &[&str] = &["start", "end"];

/// Data source properties whose value is a JSON object written as `key = <JSON>` (two levels
/// deeper, like an object's default property).
pub(crate) const JSON_OBJECT_KEYS: &[&str] = &["options", "credential"];

/// Credential fields TMDL writes as properties of the `credential` object (camelCase keyword,
/// TMSL key), in Desktop's order; the other fields stay in its JSON text.
pub(crate) const CREDENTIAL_PROPS: &[(&str, &str)] = &[
    ("authenticationKind", "AuthenticationKind"),
    ("privacySetting", "PrivacySetting"),
    ("username", "Username"),
    ("password", "Password"),
    ("encryptConnection", "EncryptConnection"),
];

/// A role member without an identity provider is a Windows (Active Directory) member, written
/// `member X = activeDirectory`; an external member's provider defaults to this one.
pub(crate) const DEFAULT_IDENTITY_PROVIDER: &str = "AzureAD";

/// `translations` blocks use object keywords for their TMSL collections and short names for
/// the translated properties.
pub(crate) const TRANSLATION_PROPS: &[(&str, &str)] = &[
    ("caption", "translatedCaption"),
    ("description", "translatedDescription"),
    ("displayFolder", "translatedDisplayFolder"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plural_and_singular_are_inverse() {
        for k in ["hierarchy", "policy", "key", "foo", "alias"] {
            assert_eq!(singular(&plural(k)), k);
        }
        assert_eq!(plural("hierarchy"), "hierarchies");
        assert_eq!(plural("key"), "keys");
    }

    #[test]
    fn keywords_and_keys_are_unique() {
        for (i, a) in KINDS.iter().enumerate() {
            for b in KINDS.iter().skip(i + 1) {
                assert!(!a.keyword.eq_ignore_ascii_case(b.keyword), "{}", a.keyword);
            }
            for kw in a.children.iter().filter(|c| **c != "*") {
                assert!(kind_by_keyword(kw).is_some(), "{kw}");
            }
        }
        assert_eq!(canonical_key("FORMATSTRING"), "formatString");
        assert_eq!(canonical_key("somethingNew"), "somethingNew");
    }
}
