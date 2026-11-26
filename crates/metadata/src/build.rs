//! TMSCHEMA rows → `SemanticModel`. See the crate docs and `docs/formats/metadata-sqlitedb.md`.
//!
//! Typed objects (tables, columns, measures, partitions, relationships, expressions, roles) are
//! mapped here; the objects the model keeps as JSON in `extra` (hierarchies, perspectives,
//! cultures, calculation groups, KPIs, …) come from the declarative [`crate::spec`] through
//! [`crate::nested`], which the writer shares.

use std::collections::HashMap;

use bicraft_model::{
    Annotation, Column, DataType, Measure, NamedExpression, Partition, Properties, Relationship, Role,
    SemanticModel, Table, TablePermission,
};
use serde_json::{Map, Value, json};

use crate::Error;
use crate::codes::{self, column_type, object_type as ot};
use crate::input::{MetadataTables, Row};
use crate::nested::{Lookups, Owner, column_name, ole_date_value};
use crate::spec;

type Obj = Map<String, Value>;

pub(crate) fn build(t: &MetadataTables) -> Result<SemanticModel, Error> {
    let ctx = Ctx::new(t);
    let model = ctx.sorted("Model").into_iter().next().ok_or(Error::NoModel)?;
    let model_id = model.id();
    ctx.model(model, model_id)
}

/// Shared lookups over every table, built once.
struct Ctx<'a> {
    t: &'a MetadataTables,
    look: Lookups<'a>,
}

impl<'a> Ctx<'a> {
    fn new(t: &'a MetadataTables) -> Self {
        Self {
            t,
            look: Lookups::new(t),
        }
    }

    /// The rows of `table`, sorted by `ID` (creation order); rows without an ID come last.
    fn sorted(&self, table: &str) -> Vec<Row<'a>> {
        let mut rows: Vec<Row<'a>> = self.t.rows(table).iter().map(Row).collect();
        rows.sort_by_key(|r| r.id().unwrap_or(i64::MAX));
        rows
    }

    /// Rows of `table` grouped by the reference column `fk`, each group in ID order.
    fn grouped(&self, table: &str, fk: &str) -> HashMap<i64, Vec<Row<'a>>> {
        let mut out: HashMap<i64, Vec<Row<'a>>> = HashMap::new();
        for r in self.sorted(table) {
            if let Some(k) = r.int(fk) {
                out.entry(k).or_default().push(r);
            }
        }
        out
    }

    /// Rows of `table` by `ID`.
    fn by_id(&self, table: &str) -> HashMap<i64, Row<'a>> {
        let mut out = HashMap::new();
        for r in self.sorted(table) {
            if let Some(id) = r.id() {
                out.entry(id).or_insert(r);
            }
        }
        out
    }

    fn annotations(&self, kind: i64, id: Option<i64>) -> Vec<Annotation> {
        self.look.annotations(kind, id)
    }

    /// Adds `changedProperties` and `extendedProperties` of a typed object to `extra`.
    fn tail(&self, extra: &mut Obj, kind: i64, id: Option<i64>) {
        self.look.tail(extra, kind, id, false);
    }

    /// The child collections of a typed object (`spec::*_CHILDREN`).
    fn children(&self, extra: &mut Obj, children: &[spec::Child], r: Row<'a>, scope: Option<i64>) {
        let owner = Owner {
            row: Some(r),
            id: r.id(),
            scope,
        };
        self.look.children(extra, children, &owner);
    }

    fn column_ref(&self, id: Option<i64>) -> Option<&(i64, String)> {
        self.look.columns.get(&id?)
    }

    fn table_name(&self, id: &i64) -> Option<&str> {
        self.look.table_name(*id)
    }

    fn model(&self, m: Row<'a>, model_id: Option<i64>) -> Result<SemanticModel, Error> {
        let in_model = |r: &Row<'_>| match (model_id, r.int("ModelID")) {
            (Some(a), Some(b)) => a == b,
            _ => true,
        };

        // User tables: in this model and not a system table (SystemFlags bit 0: H$/R$/U$ tables).
        let table_rows: Vec<Row<'a>> = self
            .sorted("Table")
            .into_iter()
            .filter(|r| in_model(r) && r.int("SystemFlags").unwrap_or(0) & 1 == 0)
            .collect();

        let columns = self.grouped("Column", "TableID");
        let measures = self.grouped("Measure", "TableID");
        let partitions = self.grouped("Partition", "TableID");
        let side = Side {
            data_sources: self.by_id("DataSource"),
            expressions: self.by_id("Expression"),
            query_groups: self.by_id("QueryGroup"),
        };

        let empty = Vec::new();
        let mut tables = Vec::with_capacity(table_rows.len());
        for r in &table_rows {
            let id = r.id();
            let get = |g: &'_ HashMap<i64, Vec<Row<'a>>>| -> Vec<Row<'a>> {
                id.and_then(|id| g.get(&id)).unwrap_or(&empty).clone()
            };
            tables.push(self.table(*r, &get(&columns), &get(&measures), &get(&partitions), &side));
        }

        let relationships = self
            .sorted("Relationship")
            .into_iter()
            .filter(&in_model)
            .filter_map(|r| self.relationship(r))
            .collect();
        let expressions = self
            .sorted("Expression")
            .into_iter()
            .filter(&in_model)
            .map(|r| self.expression(r, &side))
            .collect();
        let roles = self.roles(&in_model);

        let mut extra = Obj::new();
        if let Some(name) = m.text("Name") {
            extra.insert("name".into(), json!(name));
        }
        self.model_properties(m, &mut extra);
        self.children(&mut extra, spec::MODEL_CHILDREN, m, None);
        self.tail(&mut extra, ot::MODEL, model_id);

        Ok(SemanticModel {
            name: String::new(),
            compatibility_level: None,
            culture: m.string("Culture"),
            description: m.string("Description"),
            tables,
            relationships,
            expressions,
            roles,
            annotations: self.annotations(ot::MODEL, model_id),
            database_extra: Properties::new(),
            extra,
        })
    }

    fn model_properties(&self, m: Row<'a>, extra: &mut Obj) {
        nondefault_code(extra, "defaultMode", m.int("DefaultMode"), 0, codes::mode);
        nondefault_code(
            extra,
            "defaultDataView",
            m.int("DefaultDataView"),
            0,
            codes::data_view,
        );
        put_text(extra, "collation", m.text("Collation"));
        put_text(extra, "storageLocation", m.text("StorageLocation"));
        if let Some(raw) = m.text("DataAccessOptions")
            && let Ok(v @ Value::Object(_)) = serde_json::from_str::<Value>(raw)
        {
            extra.insert("dataAccessOptions".into(), v);
        }
        if let Some((table, name)) = m
            .reference("DefaultMeasureID")
            .and_then(|id| self.look.measures.get(&id))
        {
            // TOM: `defaultMeasure: {table, measure}` (checked on the engine).
            let mut o = Obj::new();
            if let Some(t) = self.table_name(table) {
                o.insert("table".into(), json!(t));
            }
            o.insert("measure".into(), json!(name));
            extra.insert("defaultMeasure".into(), Value::Object(o));
        }
        true_flag(extra, "forceUniqueNames", m, "ForceUniqueNames");
        true_flag(
            extra,
            "discourageImplicitMeasures",
            m,
            "DiscourageImplicitMeasures",
        );
        true_flag(extra, "discourageReportMeasures", m, "DiscourageReportMeasures");
        nondefault_code(
            extra,
            "defaultPowerBIDataSourceVersion",
            m.int("DefaultPowerBIDataSourceVersion"),
            0,
            codes::power_bi_data_source_version,
        );
        put_text(extra, "sourceQueryCulture", m.text("SourceQueryCulture"));
        nondefault_code(
            extra,
            "dataSourceVariablesOverrideBehavior",
            m.int("DataSourceVariablesOverrideBehavior"),
            0,
            codes::data_source_variables_override,
        );
        nondefault_int(
            extra,
            "dataSourceDefaultMaxConnections",
            m,
            "DataSourceDefaultMaxConnections",
            10,
        );
        true_flag(extra, "discourageCompositeModels", m, "DiscourageCompositeModels");
        nondefault_int(
            extra,
            "maxParallelismPerRefresh",
            m,
            "MaxParallelismPerRefresh",
            -1,
        );
        nondefault_int(extra, "maxParallelismPerQuery", m, "MaxParallelismPerQuery", 0);
        nondefault_int(extra, "disableAutoExists", m, "DisableAutoExists", -1);
        true_flag(
            extra,
            "disableSystemDefaultExpression",
            m,
            "DisableSystemDefaultExpression",
        );
        nondefault_code(
            extra,
            "directLakeBehavior",
            m.int("DirectLakeBehavior"),
            0,
            codes::direct_lake_behavior,
        );
        nondefault_code(
            extra,
            "valueFilterBehavior",
            m.int("ValueFilterBehavior"),
            0,
            codes::value_filter_behavior,
        );
        nondefault_code(
            extra,
            "selectionExpressionBehavior",
            m.int("SelectionExpressionBehavior"),
            0,
            codes::selection_expression_behavior,
        );
        nondefault_code(
            extra,
            "metadataAccessPolicy",
            m.int("MetadataAccessPolicy"),
            0,
            codes::metadata_category,
        );
        nondefault_code(
            extra,
            "defaultDirectLakeIndexingBehavior",
            m.int("DefaultDirectLakeIndexingBehavior"),
            0,
            codes::direct_lake_indexing_behavior,
        );
        put_text(extra, "mAttributes", m.text("MAttributes"));
        if let Some(raw) = m.text("AutomaticAggregationOptions") {
            extra.insert(
                "automaticAggregationOptions".into(),
                crate::nested::parse_or_text(raw),
            );
        }
    }

    fn table(
        &self,
        r: Row<'a>,
        columns: &[Row<'a>],
        measures: &[Row<'a>],
        partitions: &[Row<'a>],
        side: &Side<'a>,
    ) -> Table {
        let id = r.id();
        let mut extra = Obj::new();
        put_text(&mut extra, "dataCategory", r.text("DataCategory"));
        true_flag(&mut extra, "showAsVariationsOnly", r, "ShowAsVariationsOnly");
        true_flag(&mut extra, "isPrivate", r, "IsPrivate");
        nondefault_int(
            &mut extra,
            "alternateSourcePrecedence",
            r,
            "AlternateSourcePrecedence",
            0,
        );
        true_flag(
            &mut extra,
            "excludeFromModelRefresh",
            r,
            "ExcludeFromModelRefresh",
        );
        put_text(&mut extra, "sourceLineageTag", r.text("SourceLineageTag"));
        true_flag(&mut extra, "systemManaged", r, "SystemManaged");
        true_flag(
            &mut extra,
            "excludeFromAutomaticAggregations",
            r,
            "ExcludeFromAutomaticAggregations",
        );
        nondefault_code(
            &mut extra,
            "directLakeIndexingBehavior",
            r.int("DirectLakeIndexingBehavior"),
            3,
            codes::direct_lake_indexing_behavior,
        );
        self.children(&mut extra, spec::TABLE_CHILDREN, r, id);
        self.tail(&mut extra, ot::TABLE, id);

        Table {
            name: r.string("Name").unwrap_or_default(),
            description: r.string("Description"),
            lineage_tag: r.string("LineageTag"),
            is_hidden: r.flag("IsHidden", false),
            columns: columns.iter().map(|c| self.column(*c)).collect(),
            measures: measures.iter().map(|m| self.measure(*m)).collect(),
            partitions: partitions.iter().map(|p| self.partition(*p, side)).collect(),
            annotations: self.annotations(ot::TABLE, id),
            extra,
        }
    }

    fn column(&self, r: Row<'a>) -> Column {
        let id = r.id();
        let kind = r.int("Type").unwrap_or(column_type::DATA);
        let explicit = r.int("ExplicitDataType");
        let data_type = match explicit {
            Some(code) if code != codes::DATA_TYPE_AUTOMATIC => codes::data_type(code),
            _ => r
                .int("InferredDataType")
                .map_or(DataType::Unknown, codes::data_type),
        };
        let mut expression = r.raw_text("Expression").map(str::to_string);
        if kind != column_type::CALCULATED {
            expression = expression.filter(|e| !e.is_empty());
        }

        let mut extra = Obj::new();
        // `type`: implied by an expression for calculated columns, omitted for data columns.
        if kind != column_type::DATA
            && !(kind == column_type::CALCULATED && expression.is_some())
            && let Some(t) = codes::column_type(kind)
        {
            extra.insert("type".into(), json!(t));
        }
        if kind == column_type::CALCULATED_TABLE_COLUMN && r.text("ExplicitName").is_none() {
            extra.insert("isNameInferred".into(), json!(true));
        }
        if matches!(
            kind,
            column_type::CALCULATED | column_type::CALCULATED_TABLE_COLUMN
        ) && explicit == Some(codes::DATA_TYPE_AUTOMATIC)
        {
            extra.insert("isDataTypeInferred".into(), json!(true));
        }
        true_flag(&mut extra, "isUnique", r, "IsUnique");
        true_flag(&mut extra, "isKey", r, "IsKey");
        if !r.flag("IsNullable", true) {
            extra.insert("isNullable".into(), json!(false));
        }
        nondefault_code(&mut extra, "alignment", r.int("Alignment"), 1, codes::alignment);
        nondefault_int(&mut extra, "tableDetailPosition", r, "TableDetailPosition", -1);
        true_flag(&mut extra, "isDefaultLabel", r, "IsDefaultLabel");
        true_flag(&mut extra, "isDefaultImage", r, "IsDefaultImage");
        if !r.flag("IsAvailableInMDX", true) {
            // TOM's spelling (`isAvailableInMdx`), not the engine's: Desktop refuses the latter.
            extra.insert(crate::AVAILABLE_IN_MDX.into(), json!(false));
        }
        true_flag(&mut extra, "keepUniqueRows", r, "KeepUniqueRows");
        nondefault_int(&mut extra, "displayOrdinal", r, "DisplayOrdinal", 0);
        put_text(&mut extra, "sourceProviderType", r.text("SourceProviderType"));
        nondefault_code(
            &mut extra,
            "encodingHint",
            r.int("EncodingHint"),
            0,
            codes::encoding_hint,
        );
        put_text(&mut extra, "sourceLineageTag", r.text("SourceLineageTag"));
        self.children(&mut extra, spec::COLUMN_CHILDREN, r, r.int("TableID"));
        self.tail(&mut extra, ot::COLUMN, id);

        Column {
            name: column_name(r),
            data_type,
            expression,
            source_column: r.string("SourceColumn"),
            format_string: r.string("FormatString"),
            display_folder: r.string("DisplayFolder"),
            summarize_by: r
                .int("SummarizeBy")
                .filter(|&c| c != 1)
                .and_then(codes::summarize_by)
                .map(str::to_string),
            sort_by_column: self
                .column_ref(r.reference("SortByColumnID"))
                .map(|(_, n)| n.clone()),
            data_category: r.string("DataCategory"),
            description: r.string("Description"),
            lineage_tag: r.string("LineageTag"),
            is_hidden: r.flag("IsHidden", false),
            annotations: self.annotations(ot::COLUMN, id),
            extra,
        }
    }

    fn measure(&self, r: Row<'a>) -> Measure {
        let id = r.id();
        let mut extra = Obj::new();
        put_text(&mut extra, "dataCategory", r.text("DataCategory"));
        true_flag(&mut extra, "isSimpleMeasure", r, "IsSimpleMeasure");
        put_text(&mut extra, "sourceLineageTag", r.text("SourceLineageTag"));
        self.children(&mut extra, spec::MEASURE_CHILDREN, r, r.int("TableID"));
        self.tail(&mut extra, ot::MEASURE, id);
        Measure {
            name: r.string("Name").unwrap_or_default(),
            expression: r.raw_text("Expression").unwrap_or_default().to_string(),
            format_string: r.string("FormatString"),
            display_folder: r.string("DisplayFolder"),
            description: r.string("Description"),
            lineage_tag: r.string("LineageTag"),
            is_hidden: r.flag("IsHidden", false),
            annotations: self.annotations(ot::MEASURE, id),
            extra,
        }
    }

    fn partition(&self, r: Row<'a>, side: &Side<'a>) -> Partition {
        let id = r.id();
        let code = r.int("Type");
        let source_type = code
            .and_then(codes::partition_source_type)
            .map(str::to_string)
            .unwrap_or_default();
        let definition = r.text("QueryDefinition").map(str::to_string);
        let mut expression = None;
        let mut source_extra = Obj::new();
        // Where `QueryDefinition` goes depends on the source type (TMSL source objects).
        match code {
            Some(codes::partition_type::QUERY) => put_text(&mut source_extra, "query", definition.as_deref()),
            Some(codes::partition_type::ENTITY) => {
                put_text(&mut source_extra, "entityName", definition.as_deref());
            }
            // TOM's only `ParquetPartitionSource` property; inferred, no engine writes one yet.
            Some(codes::partition_type::PARQUET) => {
                put_text(&mut source_extra, "location", definition.as_deref());
            }
            Some(codes::partition_type::POLICY_RANGE) => {
                if let Some(v) = ole_date_value(r.get("RangeStart")) {
                    source_extra.insert("start".into(), v);
                }
                if let Some(v) = ole_date_value(r.get("RangeEnd")) {
                    source_extra.insert("end".into(), v);
                }
                if let Some(g) = r.int("RangeGranularity").and_then(codes::refresh_granularity) {
                    source_extra.insert("granularity".into(), json!(g));
                }
                if definition.is_some() {
                    expression = definition;
                }
            }
            _ => expression = definition,
        }
        if let Some(ds) = r
            .reference("DataSourceID")
            .and_then(|i| side.data_sources.get(&i))
        {
            put_text(&mut source_extra, "dataSource", ds.text("Name"));
        }
        if let Some(e) = r
            .reference("ExpressionSourceID")
            .and_then(|i| side.expressions.get(&i))
        {
            put_text(&mut source_extra, "expressionSource", e.text("Name"));
        }
        put_text(&mut source_extra, "schemaName", r.text("SchemaName"));
        put_text(&mut source_extra, "refreshBookmark", r.text("RefreshBookmark"));

        let mut extra = Obj::new();
        put_text(&mut extra, "description", r.text("Description"));
        nondefault_code(&mut extra, "dataView", r.int("DataView"), 3, codes::data_view);
        true_flag(
            &mut extra,
            "retainDataTillForceCalculate",
            r,
            "RetainDataTillForceCalculate",
        );
        if let Some(q) = r
            .reference("QueryGroupID")
            .and_then(|i| side.query_groups.get(&i))
        {
            put_text(&mut extra, "queryGroup", q.text("Folder"));
        }
        self.children(&mut extra, spec::PARTITION_CHILDREN, r, r.int("TableID"));
        self.look.tail(&mut extra, ot::PARTITION, id, true);

        Partition {
            name: r.string("Name").unwrap_or_default(),
            mode: r
                .int("Mode")
                .filter(|&m| m != 2)
                .and_then(codes::mode)
                .map(str::to_string),
            source_type,
            expression,
            extra,
            source_extra,
        }
    }

    /// A relationship is kept only when both ends resolve to columns of user tables.
    fn relationship(&self, r: Row<'a>) -> Option<Relationship> {
        let end = |table_col: &str, column_col: &str| -> Option<(String, String)> {
            let (owner, column) = self.column_ref(r.int(column_col))?;
            let table_id = r
                .int(table_col)
                .filter(|t| self.table_name(t).is_some())
                .unwrap_or(*owner);
            Some((self.table_name(&table_id)?.to_string(), column.clone()))
        };
        let (from_table, from_column) = end("FromTableID", "FromColumnID")?;
        let (to_table, to_column) = end("ToTableID", "ToColumnID")?;
        let id = r.id();
        let mut extra = Obj::new();
        nondefault_code(
            &mut extra,
            "joinOnDateBehavior",
            r.int("JoinOnDateBehavior"),
            1,
            codes::join_on_date,
        );
        true_flag(
            &mut extra,
            "relyOnReferentialIntegrity",
            r,
            "RelyOnReferentialIntegrity",
        );
        nondefault_code(
            &mut extra,
            "securityFilteringBehavior",
            r.int("SecurityFilteringBehavior"),
            1,
            codes::security_filtering,
        );
        self.tail(&mut extra, ot::RELATIONSHIP, id);
        Some(Relationship {
            name: r.string("Name").unwrap_or_default(),
            from_table,
            from_column,
            to_table,
            to_column,
            is_active: r.flag("IsActive", true),
            cross_filtering_behavior: code_unless(r.int("CrossFilteringBehavior"), 1, codes::cross_filtering),
            from_cardinality: code_unless(r.int("FromCardinality"), 2, codes::cardinality),
            to_cardinality: code_unless(r.int("ToCardinality"), 1, codes::cardinality),
            annotations: self.annotations(ot::RELATIONSHIP, id),
            extra,
        })
    }

    fn expression(&self, r: Row<'a>, side: &Side<'a>) -> NamedExpression {
        let id = r.id();
        let mut extra = Obj::new();
        put_text(&mut extra, "sourceLineageTag", r.text("SourceLineageTag"));
        if let Some(q) = r
            .reference("QueryGroupID")
            .and_then(|i| side.query_groups.get(&i))
        {
            put_text(&mut extra, "queryGroup", q.text("Folder"));
        }
        if let Some(e) = r
            .reference("ExpressionSourceID")
            .and_then(|i| side.expressions.get(&i))
        {
            put_text(&mut extra, "expressionSource", e.text("Name"));
        }
        if let Some((table, column)) = self.column_ref(r.reference("ParameterValuesColumnID")) {
            let mut o = Obj::new();
            o.insert("name".into(), json!(column));
            if let Some(t) = self.table_name(table) {
                o.insert("table".into(), json!(t));
            }
            extra.insert("parameterValuesColumn".into(), Value::Object(o));
        }
        put_text(&mut extra, "remoteParameterName", r.text("RemoteParameterName"));
        put_text(&mut extra, "mAttributes", r.text("MAttributes"));
        self.children(&mut extra, spec::EXPRESSION_CHILDREN, r, None);
        self.tail(&mut extra, ot::EXPRESSION, id);
        NamedExpression {
            name: r.string("Name").unwrap_or_default(),
            kind: r
                .int("Kind")
                .and_then(codes::expression_kind)
                .unwrap_or_default()
                .to_string(),
            expression: r.raw_text("Expression").unwrap_or_default().to_string(),
            description: r.string("Description"),
            lineage_tag: r.string("LineageTag"),
            annotations: self.annotations(ot::EXPRESSION, id),
            extra,
        }
    }

    fn roles(&self, in_model: &dyn Fn(&Row<'_>) -> bool) -> Vec<Role> {
        let perms = self.grouped("TablePermission", "RoleID");
        self.sorted("Role")
            .into_iter()
            .filter(|r| in_model(r))
            .map(|r| {
                let id = r.id();
                let mut extra = Obj::new();
                put_text(&mut extra, "description", r.text("Description"));
                self.children(&mut extra, spec::ROLE_CHILDREN, r, None);
                self.tail(&mut extra, ot::ROLE, id);
                let table_permissions = id
                    .and_then(|i| perms.get(&i))
                    .into_iter()
                    .flatten()
                    .filter_map(|p| {
                        let table_id = p.int("TableID")?;
                        let table = self.table_name(&table_id)?.to_string();
                        let mut extra = Obj::new();
                        nondefault_code(
                            &mut extra,
                            "metadataPermission",
                            p.int("MetadataPermission"),
                            0,
                            codes::metadata_permission,
                        );
                        self.children(&mut extra, spec::TABLE_PERMISSION_CHILDREN, *p, Some(table_id));
                        self.look.tail(&mut extra, ot::TABLE_PERMISSION, p.id(), true);
                        Some(TablePermission {
                            table,
                            filter_expression: p.text("FilterExpression").map(str::to_string),
                            extra,
                        })
                    })
                    .collect();
                Role {
                    name: r.string("Name").unwrap_or_default(),
                    model_permission: r
                        .int("ModelPermission")
                        .and_then(codes::model_permission)
                        .map(str::to_string),
                    table_permissions,
                    annotations: self.annotations(ot::ROLE, id),
                    extra,
                }
            })
            .collect()
    }
}

/// Lookups used while building partitions and expressions.
struct Side<'a> {
    data_sources: HashMap<i64, Row<'a>>,
    expressions: HashMap<i64, Row<'a>>,
    query_groups: HashMap<i64, Row<'a>>,
}

fn put_text(o: &mut Obj, key: &str, v: Option<&str>) {
    if let Some(v) = v {
        o.insert(key.into(), json!(v));
    }
}

/// Emits `key: true` when the integer column is non-zero (the TMSL default is `false`).
fn true_flag(o: &mut Obj, key: &str, r: Row<'_>, col: &str) {
    if r.flag(col, false) {
        o.insert(key.into(), json!(true));
    }
}

fn nondefault_int(o: &mut Obj, key: &str, r: Row<'_>, col: &str, default: i64) {
    if let Some(v) = r.int(col).filter(|&v| v != default) {
        o.insert(key.into(), json!(v));
    }
}

fn nondefault_code(
    o: &mut Obj,
    key: &str,
    code: Option<i64>,
    default: i64,
    f: fn(i64) -> Option<&'static str>,
) {
    if let Some(s) = code.filter(|&c| c != default).and_then(f) {
        o.insert(key.into(), json!(s));
    }
}

fn code_unless(code: Option<i64>, default: i64, f: fn(i64) -> Option<&'static str>) -> Option<String> {
    code.filter(|&c| c != default).and_then(f).map(str::to_string)
}
