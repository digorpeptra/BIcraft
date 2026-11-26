//! The declarative description of the TMSCHEMA objects that the model keeps as JSON in `extra`
//! (hierarchies, perspectives, cultures, calculation groups, KPIs, …). One [`Kind`] per object
//! type says which table holds its rows, how they hang off their owner, which column carries
//! which TMSL property and with which conversion, and which child collections it has. The same
//! description drives reading (`nested::Reader`, rows → JSON) and writing (`nested::Writer`, JSON
//! diff → row edits), so the two directions cannot drift apart.
//!
//! Shapes and codes were checked against the Analysis Services engine of Power BI Desktop
//! 2.158.1304.0 (17.0.87.18): synthetic TMSL deployed, scripted back with TOM and backed up; see
//! `docs/formats/metadata-sqlitedb.md`, "Nested objects".

use crate::codes::{self, object_type as ot};

pub(crate) type CodeFn = fn(i64) -> Option<&'static str>;

/// How a column value becomes a TMSL value.
#[derive(Clone, Copy)]
pub(crate) enum Ty {
    /// Non-empty text; left out when NULL or empty.
    Text,
    /// Text that is always present (`""` when NULL): expressions.
    Expr,
    /// Integer flag, present when it differs from the default.
    Bool(bool),
    /// Integer, present when it differs from the default (`None`: always present).
    Int(Option<i64>),
    /// Enumeration code, present when known and different from the default (`None`: always).
    Code(CodeFn, Option<i64>),
    /// JSON stored as text (kept as a string when it does not parse).
    Json,
    /// Linguistic metadata content: JSON, or XML text when the row's `ContentType` is 0.
    Content,
    /// A user table, by name. Also the scope of the row's column/measure/hierarchy references.
    Table,
    /// A column of the scope table.
    Column,
    /// Any column, as `{table, column}`.
    ColumnPair,
    /// A measure of the scope table.
    Measure,
    /// A hierarchy of the scope table.
    Hierarchy,
    /// Any hierarchy, as `{table, hierarchy}`.
    HierarchyPair,
    Relationship,
    /// A set of the scope table.
    Set,
}

/// One TMSL property stored in one column.
#[derive(Clone, Copy)]
pub(crate) struct Prop {
    pub key: &'static str,
    pub col: &'static str,
    pub ty: Ty,
    /// Whether a change of this property can be written to an existing row.
    pub write: bool,
}

const fn p(key: &'static str, col: &'static str, ty: Ty) -> Prop {
    Prop {
        key,
        col,
        ty,
        write: true,
    }
}

/// A property that is fixed once the object exists (names that storage or other rows embed).
const fn fixed(key: &'static str, col: &'static str, ty: Ty) -> Prop {
    Prop {
        key,
        col,
        ty,
        write: false,
    }
}

/// How the rows of a child collection find their owner.
#[derive(Clone, Copy)]
pub(crate) enum Link {
    /// The child row holds the owner's ID in this column (`Level.HierarchyID`).
    Fk(&'static str),
    /// The owner row holds the child's ID in `owner_col` (`Measure.KPIID`); the child points
    /// back through `back`.
    Pointer { owner_col: &'static str, back: Back },
}

#[derive(Clone, Copy)]
pub(crate) enum Back {
    /// The child row holds the owner's ID in this column (`KPI.MeasureID`).
    Column(&'static str),
    /// The child row holds the owner as `ObjectID` / `ObjectType`.
    Object,
}

/// The JSON shape of a child collection.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shape {
    /// An array of objects.
    Array,
    /// One object (at most one row).
    Object,
    /// An array of the single property of each row (`associatedColumns: ["A", "B"]`).
    ScalarArray,
    /// The single property of the only row (`primaryColumn: "A"`).
    Scalar,
    /// `ObjectTranslation` rows as the TMSL `translations` tree.
    Translations,
}

#[derive(Clone, Copy)]
pub(crate) struct Child {
    pub key: &'static str,
    /// `(column, key)`: the key used instead of `key` when the *owner* row holds 0 in `column`
    /// (calendar column groups without a time unit list `columns`, not `associatedColumns`).
    pub alt_key: Option<(&'static str, &'static str)>,
    pub kind: &'static Kind,
    pub link: Link,
    pub shape: Shape,
    /// Only rows with this value in this column belong to the collection; new rows get it.
    pub filter: Option<(&'static str, i64)>,
}

const fn array(key: &'static str, kind: &'static Kind, fk: &'static str) -> Child {
    Child {
        key,
        alt_key: None,
        kind,
        link: Link::Fk(fk),
        shape: Shape::Array,
        filter: None,
    }
}

/// The `excludedArtifacts` of an owner of `ObjectType` `owner_type`: rows that point back at
/// their owner through `ObjectID` / `ObjectType`.
const fn excluded_artifacts(owner_type: i64) -> Child {
    Child {
        key: "excludedArtifacts",
        alt_key: None,
        kind: &EXCLUDED_ARTIFACT,
        link: Link::Fk("ObjectID"),
        shape: Shape::Array,
        filter: Some(("ObjectType", owner_type)),
    }
}

const fn pointer(key: &'static str, kind: &'static Kind, owner_col: &'static str, back: Back) -> Child {
    Child {
        key,
        alt_key: None,
        kind,
        link: Link::Pointer { owner_col, back },
        shape: Shape::Object,
        filter: None,
    }
}

/// Order of the rows of a collection.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Order {
    /// By `ID` (creation order).
    Id,
    /// By `Ordinal`, then `ID`.
    Ordinal,
}

#[derive(Clone, Copy)]
pub(crate) struct Kind {
    /// Name used in messages.
    pub what: &'static str,
    pub table: &'static str,
    /// `ObjectType` of the object's annotations, extended and changed properties; `None` when
    /// the object type carries none.
    pub object_type: Option<i64>,
    /// Whether `annotations` are mapped (TOM accepts them on this object).
    pub annotations: bool,
    pub props: &'static [Prop],
    pub children: &'static [Child],
    pub order: Order,
    /// The JSON key that identifies an item of an array when writing (`None`: position).
    pub key: Option<&'static str>,
    /// Whether items can be added or removed by a metadata-only edit.
    pub insert: bool,
    pub delete: bool,
    /// Bookkeeping columns of new rows, beyond the ID, links, properties and times.
    pub fixed: &'static [(&'static str, i64)],
    /// The time columns the table has (set to the edit time on new rows).
    pub times: Times,
    /// Rows elsewhere that point at this object ((table, column)): it cannot be removed
    /// while one does.
    pub referenced_by: &'static [(&'static str, &'static str)],
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Times {
    None,
    /// ModifiedTime.
    Modified,
    /// ModifiedTime and StructureModifiedTime.
    Structure,
}

const BASE: Kind = Kind {
    what: "",
    table: "",
    object_type: None,
    annotations: true,
    props: &[],
    children: &[],
    order: Order::Id,
    key: Some("name"),
    insert: true,
    delete: true,
    fixed: &[],
    times: Times::Modified,
    referenced_by: &[],
};

// --- leaves ---------------------------------------------------------------------------------

pub(crate) static FORMAT_STRING_DEFINITION: Kind = Kind {
    what: "format string definition",
    table: "FormatStringDefinition",
    object_type: Some(ot::FORMAT_STRING_DEFINITION),
    annotations: false,
    props: &[p("expression", "Expression", Ty::Expr)],
    key: None,
    fixed: &[("State", 1)],
    ..BASE
};

pub(crate) static DETAIL_ROWS_DEFINITION: Kind = Kind {
    what: "detail rows definition",
    table: "DetailRowsDefinition",
    object_type: Some(ot::DETAIL_ROWS_DEFINITION),
    annotations: false,
    props: &[p("expression", "Expression", Ty::Expr)],
    key: None,
    fixed: &[("State", 1)],
    ..BASE
};

pub(crate) static KPI: Kind = Kind {
    what: "KPI",
    table: "KPI",
    object_type: Some(ot::KPI),
    props: &[
        p("description", "Description", Ty::Text),
        p("targetDescription", "TargetDescription", Ty::Text),
        p("targetExpression", "TargetExpression", Ty::Text),
        p("targetFormatString", "TargetFormatString", Ty::Text),
        p("statusGraphic", "StatusGraphic", Ty::Text),
        p("statusDescription", "StatusDescription", Ty::Text),
        p("statusExpression", "StatusExpression", Ty::Text),
        p("trendGraphic", "TrendGraphic", Ty::Text),
        p("trendDescription", "TrendDescription", Ty::Text),
        p("trendExpression", "TrendExpression", Ty::Text),
    ],
    key: None,
    ..BASE
};

/// An excluded artifact (`ExcludedArtifact`): TOM's `artifactType` (an `ObjectType` code, kept
/// as a number as TOM writes it) and `reference` (JSON). The engine accepts relationships under
/// the model, columns, measures and hierarchies under a table, tables under an expression.
pub(crate) static EXCLUDED_ARTIFACT: Kind = Kind {
    what: "excluded artifact",
    table: "ExcludedArtifact",
    annotations: false,
    props: &[
        p("artifactType", "ArtifactType", Ty::Int(None)),
        p("reference", "Reference", Ty::Json),
    ],
    key: None,
    times: Times::None,
    ..BASE
};

/// Measure: `kpi`, `detailRowsDefinition`, `formatStringDefinition`.
pub(crate) static MEASURE_CHILDREN: &[Child] = &[
    pointer("kpi", &KPI, "KPIID", Back::Column("MeasureID")),
    pointer(
        "detailRowsDefinition",
        &DETAIL_ROWS_DEFINITION,
        "DetailRowsDefinitionID",
        Back::Object,
    ),
    pointer(
        "formatStringDefinition",
        &FORMAT_STRING_DEFINITION,
        "FormatStringDefinitionID",
        Back::Object,
    ),
];

// --- hierarchies ----------------------------------------------------------------------------

pub(crate) static LEVEL: Kind = Kind {
    what: "level",
    table: "Level",
    object_type: Some(ot::LEVEL),
    props: &[
        fixed("name", "Name", Ty::Text),
        fixed("ordinal", "Ordinal", Ty::Int(None)),
        p("description", "Description", Ty::Text),
        fixed("column", "ColumnID", Ty::Column),
        p("lineageTag", "LineageTag", Ty::Text),
        p("sourceLineageTag", "SourceLineageTag", Ty::Text),
    ],
    order: Order::Ordinal,
    insert: false,
    delete: false,
    ..BASE
};

pub(crate) static HIERARCHY: Kind = Kind {
    what: "hierarchy",
    table: "Hierarchy",
    object_type: Some(ot::HIERARCHY),
    props: &[
        fixed("name", "Name", Ty::Text),
        p("description", "Description", Ty::Text),
        p("isHidden", "IsHidden", Ty::Bool(false)),
        p("displayFolder", "DisplayFolder", Ty::Text),
        p(
            "hideMembers",
            "HideMembers",
            Ty::Code(codes::hide_members, Some(0)),
        ),
        p("lineageTag", "LineageTag", Ty::Text),
        p("sourceLineageTag", "SourceLineageTag", Ty::Text),
    ],
    children: &[array("levels", &LEVEL, "HierarchyID")],
    insert: false,
    delete: false,
    times: Times::Structure,
    ..BASE
};

// --- calculation groups ---------------------------------------------------------------------

static CALC_FORMAT_STRING: &[Child] = &[pointer(
    "formatStringDefinition",
    &FORMAT_STRING_DEFINITION,
    "FormatStringDefinitionID",
    Back::Object,
)];

pub(crate) static CALCULATION_ITEM: Kind = Kind {
    what: "calculation item",
    table: "CalculationItem",
    object_type: Some(ot::CALCULATION_ITEM),
    annotations: false,
    props: &[
        fixed("name", "Name", Ty::Text),
        p("description", "Description", Ty::Text),
        p("expression", "Expression", Ty::Expr),
        fixed("ordinal", "Ordinal", Ty::Int(None)),
    ],
    children: CALC_FORMAT_STRING,
    // The items are the rows of the calculation group table: adding or removing one changes
    // stored data.
    insert: false,
    delete: false,
    fixed: &[("State", 1)],
    ..BASE
};

pub(crate) static CALCULATION_EXPRESSION: Kind = Kind {
    what: "calculation expression",
    table: "CalculationExpression",
    object_type: Some(ot::CALCULATION_EXPRESSION),
    annotations: false,
    props: &[
        p("description", "Description", Ty::Text),
        p("expression", "Expression", Ty::Expr),
    ],
    children: CALC_FORMAT_STRING,
    key: None,
    fixed: &[("State", 1)],
    ..BASE
};

const fn selection(key: &'static str, mode: i64) -> Child {
    Child {
        key,
        alt_key: None,
        kind: &CALCULATION_EXPRESSION,
        link: Link::Fk("CalculationGroupID"),
        shape: Shape::Object,
        filter: Some(("SelectionMode", mode)),
    }
}

pub(crate) static CALCULATION_GROUP: Kind = Kind {
    what: "calculation group",
    table: "CalculationGroup",
    object_type: Some(ot::CALCULATION_GROUP),
    props: &[
        p("description", "Description", Ty::Text),
        p("precedence", "Precedence", Ty::Int(Some(0))),
    ],
    children: &[
        selection("multipleOrEmptySelectionExpression", 1),
        selection("noSelectionExpression", 2),
        array("calculationItems", &CALCULATION_ITEM, "CalculationGroupID"),
    ],
    key: None,
    insert: false,
    delete: false,
    ..BASE
};

// --- incremental refresh, calendars -------------------------------------------------------

pub(crate) static REFRESH_POLICY: Kind = Kind {
    what: "refresh policy",
    table: "RefreshPolicy",
    object_type: Some(ot::REFRESH_POLICY),
    annotations: false,
    props: &[
        p(
            "policyType",
            "PolicyType",
            Ty::Code(codes::refresh_policy_type, None),
        ),
        p(
            "rollingWindowGranularity",
            "RollingWindowGranularity",
            Ty::Code(codes::refresh_granularity, None),
        ),
        p("rollingWindowPeriods", "RollingWindowPeriods", Ty::Int(None)),
        p(
            "incrementalGranularity",
            "IncrementalGranularity",
            Ty::Code(codes::refresh_granularity, None),
        ),
        p("incrementalPeriods", "IncrementalPeriods", Ty::Int(None)),
        p(
            "incrementalPeriodsOffset",
            "IncrementalPeriodsOffset",
            Ty::Int(Some(0)),
        ),
        p("pollingExpression", "PollingExpression", Ty::Text),
        p("sourceExpression", "SourceExpression", Ty::Text),
        p("mode", "Mode", Ty::Code(codes::refresh_policy_mode, Some(0))),
    ],
    key: None,
    times: Times::None,
    ..BASE
};

pub(crate) static CALENDAR_COLUMN_REFERENCE: Kind = Kind {
    what: "calendar column reference",
    table: "CalendarColumnReference",
    object_type: Some(ot::CALENDAR_COLUMN_REFERENCE),
    annotations: false,
    props: &[p("column", "ColumnID", Ty::Column)],
    key: Some("column"),
    ..BASE
};

pub(crate) static CALENDAR_COLUMN_GROUP: Kind = Kind {
    what: "calendar column group",
    table: "CalendarColumnGroup",
    object_type: Some(ot::CALENDAR_COLUMN_GROUP),
    annotations: false,
    props: &[p("timeUnit", "TimeUnit", Ty::Code(codes::time_unit, Some(0)))],
    children: &[
        Child {
            key: "primaryColumn",
            alt_key: None,
            kind: &CALENDAR_COLUMN_REFERENCE,
            link: Link::Fk("CalendarColumnGroupID"),
            shape: Shape::Scalar,
            filter: Some(("IsPrimaryColumn", 1)),
        },
        Child {
            key: "associatedColumns",
            alt_key: Some(("TimeUnit", "columns")),
            kind: &CALENDAR_COLUMN_REFERENCE,
            link: Link::Fk("CalendarColumnGroupID"),
            shape: Shape::ScalarArray,
            filter: Some(("IsPrimaryColumn", 0)),
        },
    ],
    key: None,
    ..BASE
};

pub(crate) static CALENDAR: Kind = Kind {
    what: "calendar",
    table: "Calendar",
    object_type: Some(ot::CALENDAR),
    annotations: false,
    props: &[
        p("name", "Name", Ty::Text),
        p("description", "Description", Ty::Text),
        p("lineageTag", "LineageTag", Ty::Text),
        p("sourceLineageTag", "SourceLineageTag", Ty::Text),
    ],
    children: &[array(
        "calendarColumnGroups",
        &CALENDAR_COLUMN_GROUP,
        "CalendarID",
    )],
    ..BASE
};

/// Table: `defaultDetailRowsDefinition`, `calculationGroup`, `refreshPolicy`, `hierarchies`,
/// `calendars`.
pub(crate) static TABLE_CHILDREN: &[Child] = &[
    pointer(
        "defaultDetailRowsDefinition",
        &DETAIL_ROWS_DEFINITION,
        "DefaultDetailRowsDefinitionID",
        Back::Object,
    ),
    pointer(
        "calculationGroup",
        &CALCULATION_GROUP,
        "CalculationGroupID",
        Back::Column("TableID"),
    ),
    pointer(
        "refreshPolicy",
        &REFRESH_POLICY,
        "RefreshPolicyID",
        Back::Column("TableID"),
    ),
    array("hierarchies", &HIERARCHY, "TableID"),
    array("calendars", &CALENDAR, "TableID"),
    array("sets", &SET, "TableID"),
    excluded_artifacts(ot::TABLE),
];

/// A set (`Set`): a named DAX table expression of a table. Only the expression is stored, no
/// data, so sets can be added and removed. The engine accepts them only when sets are enabled
/// in its configuration, which Desktop leaves off (`metadata-sqlitedb.md`, "Engine oracle").
pub(crate) static SET: Kind = Kind {
    what: "set",
    table: "Set",
    object_type: Some(ot::SET),
    props: &[
        p("name", "Name", Ty::Text),
        p("description", "Description", Ty::Text),
        p("expression", "Expression", Ty::Expr),
        p("isDynamic", "IsDynamic", Ty::Bool(false)),
        p("isHidden", "IsHidden", Ty::Bool(false)),
        p("displayFolder", "DisplayFolder", Ty::Text),
    ],
    fixed: &[("State", 1)],
    times: Times::Structure,
    referenced_by: &[("PerspectiveSet", "SetID")],
    ..BASE
};

/// Shared expression: `excludedArtifacts`.
pub(crate) static EXPRESSION_CHILDREN: &[Child] = &[excluded_artifacts(ot::EXPRESSION)];

// --- columns ------------------------------------------------------------------------------

pub(crate) static VARIATION: Kind = Kind {
    what: "variation",
    table: "Variation",
    object_type: Some(ot::VARIATION),
    props: &[
        p("name", "Name", Ty::Text),
        p("description", "Description", Ty::Text),
        p("relationship", "RelationshipID", Ty::Relationship),
        p("defaultHierarchy", "DefaultHierarchyID", Ty::HierarchyPair),
        p("defaultColumn", "DefaultColumnID", Ty::ColumnPair),
        p("isDefault", "IsDefault", Ty::Bool(false)),
    ],
    times: Times::None,
    ..BASE
};

pub(crate) static GROUP_BY_COLUMN: Kind = Kind {
    what: "group-by column",
    table: "GroupByColumn",
    object_type: Some(ot::GROUP_BY_COLUMN),
    annotations: false,
    props: &[p("groupingColumn", "GroupingColumnID", Ty::Column)],
    key: Some("groupingColumn"),
    ..BASE
};

pub(crate) static RELATED_COLUMN_DETAILS: Kind = Kind {
    what: "related column details",
    table: "RelatedColumnDetails",
    object_type: Some(ot::RELATED_COLUMN_DETAILS),
    annotations: false,
    children: &[array(
        "groupByColumns",
        &GROUP_BY_COLUMN,
        "RelatedColumnDetailsID",
    )],
    key: None,
    ..BASE
};

pub(crate) static ALTERNATE_OF: Kind = Kind {
    what: "alternateOf",
    table: "AlternateOf",
    object_type: Some(ot::ALTERNATE_OF),
    annotations: false,
    props: &[
        p("baseColumn", "BaseColumnID", Ty::ColumnPair),
        p("baseTable", "BaseTableID", Ty::Table),
        p(
            "summarization",
            "Summarization",
            Ty::Code(codes::summarization, Some(0)),
        ),
    ],
    key: None,
    times: Times::None,
    ..BASE
};

/// Column: `variations`, `relatedColumnDetails`, `alternateOf`.
pub(crate) static COLUMN_CHILDREN: &[Child] = &[
    array("variations", &VARIATION, "ColumnID"),
    pointer(
        "relatedColumnDetails",
        &RELATED_COLUMN_DETAILS,
        "RelatedColumnDetailsID",
        Back::Column("ColumnID"),
    ),
    pointer(
        "alternateOf",
        &ALTERNATE_OF,
        "AlternateOfID",
        Back::Column("ColumnID"),
    ),
];

// --- partitions ---------------------------------------------------------------------------

pub(crate) static DATA_COVERAGE_DEFINITION: Kind = Kind {
    what: "data coverage definition",
    table: "DataCoverageDefinition",
    object_type: Some(ot::DATA_COVERAGE_DEFINITION),
    annotations: false,
    props: &[
        p("description", "Description", Ty::Text),
        p("expression", "Expression", Ty::Expr),
    ],
    key: None,
    fixed: &[("State", 1)],
    ..BASE
};

/// Partition: `dataCoverageDefinition`.
pub(crate) static PARTITION_CHILDREN: &[Child] = &[pointer(
    "dataCoverageDefinition",
    &DATA_COVERAGE_DEFINITION,
    "DataCoverageDefinitionID",
    Back::Column("PartitionID"),
)];

// --- security -----------------------------------------------------------------------------

pub(crate) static ROLE_MEMBERSHIP: Kind = Kind {
    what: "role member",
    table: "RoleMembership",
    object_type: Some(ot::ROLE_MEMBERSHIP),
    props: &[
        p("memberName", "MemberName", Ty::Expr),
        p("memberId", "MemberID", Ty::Text),
        p("identityProvider", "IdentityProvider", Ty::Text),
        p("memberType", "MemberType", Ty::Code(codes::member_type, Some(1))),
    ],
    key: Some("memberName"),
    ..BASE
};

/// Role: `members`.
pub(crate) static ROLE_CHILDREN: &[Child] = &[array("members", &ROLE_MEMBERSHIP, "RoleID")];

pub(crate) static COLUMN_PERMISSION: Kind = Kind {
    what: "column permission",
    table: "ColumnPermission",
    object_type: Some(ot::COLUMN_PERMISSION),
    props: &[
        p("name", "ColumnID", Ty::Column),
        p(
            "metadataPermission",
            "MetadataPermission",
            Ty::Code(codes::metadata_permission, Some(0)),
        ),
    ],
    ..BASE
};

/// Table permission: `columnPermissions`.
pub(crate) static TABLE_PERMISSION_CHILDREN: &[Child] = &[array(
    "columnPermissions",
    &COLUMN_PERMISSION,
    "TablePermissionID",
)];

// --- model collections --------------------------------------------------------------------

pub(crate) static OBJECT_TRANSLATION: Kind = Kind {
    what: "translation",
    table: "ObjectTranslation",
    annotations: false,
    key: None,
    fixed: &[("Altered", 0)],
    ..BASE
};

pub(crate) static LINGUISTIC_METADATA: Kind = Kind {
    what: "linguistic metadata",
    table: "LinguisticMetadata",
    object_type: Some(ot::LINGUISTIC_METADATA),
    props: &[
        p("content", "Content", Ty::Content),
        p(
            "contentType",
            "ContentType",
            Ty::Code(codes::content_type, Some(0)),
        ),
    ],
    key: None,
    ..BASE
};

pub(crate) static CULTURE: Kind = Kind {
    what: "culture",
    table: "Culture",
    object_type: Some(ot::CULTURE),
    props: &[p("name", "Name", Ty::Text)],
    children: &[
        pointer(
            "linguisticMetadata",
            &LINGUISTIC_METADATA,
            "LinguisticMetadataID",
            Back::Column("CultureID"),
        ),
        Child {
            key: "translations",
            alt_key: None,
            kind: &OBJECT_TRANSLATION,
            link: Link::Fk("CultureID"),
            shape: Shape::Translations,
            filter: None,
        },
    ],
    times: Times::Structure,
    ..BASE
};

pub(crate) static PERSPECTIVE_COLUMN: Kind = Kind {
    what: "perspective column",
    table: "PerspectiveColumn",
    object_type: Some(ot::PERSPECTIVE_COLUMN),
    props: &[p("name", "ColumnID", Ty::Column)],
    ..BASE
};

pub(crate) static PERSPECTIVE_HIERARCHY: Kind = Kind {
    what: "perspective hierarchy",
    table: "PerspectiveHierarchy",
    object_type: Some(ot::PERSPECTIVE_HIERARCHY),
    props: &[p("name", "HierarchyID", Ty::Hierarchy)],
    ..BASE
};

pub(crate) static PERSPECTIVE_MEASURE: Kind = Kind {
    what: "perspective measure",
    table: "PerspectiveMeasure",
    object_type: Some(ot::PERSPECTIVE_MEASURE),
    props: &[p("name", "MeasureID", Ty::Measure)],
    ..BASE
};

pub(crate) static PERSPECTIVE_SET: Kind = Kind {
    what: "perspective set",
    table: "PerspectiveSet",
    object_type: Some(ot::PERSPECTIVE_SET),
    props: &[p("name", "SetID", Ty::Set)],
    ..BASE
};

pub(crate) static PERSPECTIVE_TABLE: Kind = Kind {
    what: "perspective table",
    table: "PerspectiveTable",
    object_type: Some(ot::PERSPECTIVE_TABLE),
    props: &[
        p("name", "TableID", Ty::Table),
        p("includeAll", "IncludeAll", Ty::Bool(false)),
    ],
    children: &[
        array("columns", &PERSPECTIVE_COLUMN, "PerspectiveTableID"),
        array("hierarchies", &PERSPECTIVE_HIERARCHY, "PerspectiveTableID"),
        array("measures", &PERSPECTIVE_MEASURE, "PerspectiveTableID"),
        array("sets", &PERSPECTIVE_SET, "PerspectiveTableID"),
    ],
    ..BASE
};

pub(crate) static PERSPECTIVE: Kind = Kind {
    what: "perspective",
    table: "Perspective",
    object_type: Some(ot::PERSPECTIVE),
    props: &[
        p("name", "Name", Ty::Text),
        p("description", "Description", Ty::Text),
    ],
    children: &[array("tables", &PERSPECTIVE_TABLE, "PerspectiveID")],
    ..BASE
};

pub(crate) static QUERY_GROUP: Kind = Kind {
    what: "query group",
    table: "QueryGroup",
    object_type: Some(ot::QUERY_GROUP),
    props: &[
        p("folder", "Folder", Ty::Text),
        p("description", "Description", Ty::Text),
    ],
    key: Some("folder"),
    times: Times::None,
    referenced_by: &[("Expression", "QueryGroupID"), ("Partition", "QueryGroupID")],
    ..BASE
};

pub(crate) static FUNCTION: Kind = Kind {
    what: "function",
    table: "Function",
    object_type: Some(ot::FUNCTION),
    props: &[
        p("name", "Name", Ty::Text),
        p("description", "Description", Ty::Text),
        p("expression", "Expression", Ty::Expr),
        p("isHidden", "IsHidden", Ty::Bool(false)),
        p("lineageTag", "LineageTag", Ty::Text),
        p("sourceLineageTag", "SourceLineageTag", Ty::Text),
    ],
    fixed: &[("State", 1)],
    times: Times::Structure,
    ..BASE
};

/// Legacy and structured data sources. `ConnectionString`, `Password` and `Credential` are
/// stored encrypted with the engine instance's key, so they are neither read nor written.
pub(crate) static DATA_SOURCE: Kind = Kind {
    what: "data source",
    table: "DataSource",
    object_type: Some(ot::DATA_SOURCE),
    props: &[
        fixed("type", "Type", Ty::Code(codes::data_source_type, Some(1))),
        fixed("name", "Name", Ty::Text),
        p("description", "Description", Ty::Text),
        p(
            "impersonationMode",
            "ImpersonationMode",
            Ty::Code(codes::impersonation_mode, Some(1)),
        ),
        p("account", "Account", Ty::Text),
        p("maxConnections", "MaxConnections", Ty::Int(Some(10))),
        p("isolation", "Isolation", Ty::Code(codes::isolation, Some(1))),
        p("timeout", "Timeout", Ty::Int(Some(0))),
        p("provider", "Provider", Ty::Text),
        p("connectionDetails", "ConnectionDetails", Ty::Json),
        p("options", "Options", Ty::Json),
        p("contextExpression", "ContextExpression", Ty::Text),
    ],
    // A new data source would need an encrypted connection string or credential.
    insert: false,
    referenced_by: &[("Partition", "DataSourceID")],
    ..BASE
};

/// Analytics AI metadata (`AnalyticsAIMetadata`): a name and a JSON measure analysis
/// definition; TOM gives it neither annotations nor extended properties.
pub(crate) static ANALYTICS_AI_METADATA: Kind = Kind {
    what: "analytics AI metadata",
    table: "AnalyticsAIMetadata",
    annotations: false,
    props: &[
        p("name", "Name", Ty::Text),
        p("measureAnalysisDefinition", "MeasureAnalysisDefinition", Ty::Json),
    ],
    times: Times::None,
    ..BASE
};

/// Binding information (`BindingInfo`, TMSL `bindingInfoCollection`); the type is fixed because
/// it selects the TOM class (`dataBindingHint`).
pub(crate) static BINDING_INFO: Kind = Kind {
    what: "binding info",
    table: "BindingInfo",
    object_type: Some(ot::BINDING_INFO),
    props: &[
        fixed("type", "Type", Ty::Code(codes::binding_info_type, None)),
        p("name", "Name", Ty::Text),
        p("description", "Description", Ty::Text),
        p("connectionId", "ConnectionId", Ty::Text),
    ],
    times: Times::None,
    ..BASE
};

/// Model: `cultures`, `perspectives`, `queryGroups`, `functions`, `dataSources`,
/// `analyticsAIMetadata`, `excludedArtifacts`, `bindingInfoCollection`.
pub(crate) static MODEL_CHILDREN: &[Child] = &[
    array("dataSources", &DATA_SOURCE, "ModelID"),
    array("cultures", &CULTURE, "ModelID"),
    array("perspectives", &PERSPECTIVE, "ModelID"),
    array("queryGroups", &QUERY_GROUP, "ModelID"),
    array("functions", &FUNCTION, "ModelID"),
    array("analyticsAIMetadata", &ANALYTICS_AI_METADATA, "ModelID"),
    excluded_artifacts(ot::MODEL),
    array("bindingInfoCollection", &BINDING_INFO, "ModelID"),
];

/// Every kind, for checks over the whole description.
#[cfg(test)]
pub(crate) static ALL: &[&Kind] = &[
    &FORMAT_STRING_DEFINITION,
    &DETAIL_ROWS_DEFINITION,
    &KPI,
    &LEVEL,
    &HIERARCHY,
    &CALCULATION_ITEM,
    &CALCULATION_EXPRESSION,
    &CALCULATION_GROUP,
    &REFRESH_POLICY,
    &CALENDAR_COLUMN_REFERENCE,
    &CALENDAR_COLUMN_GROUP,
    &CALENDAR,
    &VARIATION,
    &GROUP_BY_COLUMN,
    &RELATED_COLUMN_DETAILS,
    &ALTERNATE_OF,
    &DATA_COVERAGE_DEFINITION,
    &ROLE_MEMBERSHIP,
    &COLUMN_PERMISSION,
    &OBJECT_TRANSLATION,
    &LINGUISTIC_METADATA,
    &CULTURE,
    &PERSPECTIVE_COLUMN,
    &PERSPECTIVE_HIERARCHY,
    &PERSPECTIVE_MEASURE,
    &PERSPECTIVE_TABLE,
    &PERSPECTIVE,
    &QUERY_GROUP,
    &FUNCTION,
    &DATA_SOURCE,
    &EXCLUDED_ARTIFACT,
    &SET,
    &PERSPECTIVE_SET,
    &ANALYTICS_AI_METADATA,
    &BINDING_INFO,
];
