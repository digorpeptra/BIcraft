//! Detailed reads of the model (M9.3): `model.get`, `table.get`, `column.get`, `measure.get`,
//! `relationship.list`. Read-only.
//!
//! Every object comes out in one stable, snake_case shape (documented in `docs/mcp.md`, "Model
//! reads"): each property the model holds is a key, present even when unset (`null`, or the
//! TMSL default for booleans), whether bicraft keeps it as a typed field or in the object's
//! unmodelled TMSL properties. What has no key of its own stays under `extra`, keyed by its TMSL
//! name, so nothing the file holds is hidden.

use bicraft_model::{Column, Measure, Partition, Relationship, Role, SemanticModel, Table};
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::Session;
use crate::commands::model;
use crate::params::{allow, opt_str, str_param};
use crate::translations::{Captions, captions_param};

/// An object's properties: its typed fields (snake_case, as serde writes them) and its
/// unmodelled TMSL properties (camelCase). Keys are taken out as the output is built; what is
/// left is passed through.
struct Props {
    typed: Map<String, Value>,
    extra: Map<String, Value>,
}

impl Props {
    fn of<T: Serialize>(x: &T) -> Props {
        let mut typed = match serde_json::to_value(x) {
            Ok(Value::Object(m)) => m,
            _ => Map::new(),
        };
        let extra = match typed.remove("extra") {
            Some(Value::Object(m)) => m,
            _ => Map::new(),
        };
        Props { typed, extra }
    }

    fn from_tmsl(v: &Value) -> Props {
        Props {
            typed: Map::new(),
            extra: v.as_object().cloned().unwrap_or_default(),
        }
    }

    /// The property under its typed (snake_case) or TMSL (camelCase) name; `null` when unset.
    fn take(&mut self, snake: &str, camel: &str) -> Value {
        let typed = self.typed.remove(snake).filter(|v| !v.is_null());
        let tmsl = self.extra.remove(camel).filter(|v| !v.is_null());
        let snake_extra = self.extra.remove(snake).filter(|v| !v.is_null());
        typed.or(tmsl).or(snake_extra).unwrap_or(Value::Null)
    }

    /// A text property (TMSL multi-line arrays joined with `\n`).
    fn text(&mut self, snake: &str, camel: &str) -> Value {
        text_of(self.take(snake, camel))
    }

    /// A boolean property with its TMSL default.
    fn flag(&mut self, snake: &str, camel: &str, default: bool) -> Value {
        Value::Bool(self.take(snake, camel).as_bool().unwrap_or(default))
    }

    /// An expression object (`{expression: ...}`, e.g. `formatStringDefinition`) as its text.
    fn expression(&mut self, snake: &str, camel: &str) -> Value {
        match self.take(snake, camel) {
            Value::Object(o) => o.get("expression").cloned().map_or(Value::Null, text_of),
            other => text_of(other),
        }
    }

    fn annotations(&mut self) -> Value {
        match self.take("annotations", "annotations") {
            Value::Null => json!([]),
            v => v,
        }
    }

    /// Writes the remaining typed fields (new ones bicraft models) and `extra` into `out`.
    fn finish(self, mut out: Map<String, Value>) -> Value {
        for (k, v) in self.typed {
            out.entry(k).or_insert(v);
        }
        out.insert("extra".into(), Value::Object(self.extra));
        Value::Object(out)
    }
}

fn text_of(v: Value) -> Value {
    match v {
        Value::Array(lines) if lines.iter().all(Value::is_string) => Value::String(
            lines
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        other => other,
    }
}

fn obj(pairs: Vec<(&str, Value)>) -> Map<String, Value> {
    pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

pub(crate) fn column_json(table: &str, c: &Column) -> Value {
    let mut p = Props::of(c);
    let expression = p.text("expression", "expression");
    let kind = match p.take("kind", "type").as_str() {
        Some("calculated") => "calculated",
        Some("calculatedTableColumn") => "calculated_table_column",
        Some("rowNumber") => "row_number",
        _ if !expression.is_null() => "calculated",
        _ => "data",
    };
    let out = obj(vec![
        ("name", p.take("name", "name")),
        ("table", json!(table)),
        ("kind", json!(kind)),
        ("data_type", p.take("data_type", "dataType")),
        ("source_column", p.take("source_column", "sourceColumn")),
        ("expression", expression),
        ("format_string", p.take("format_string", "formatString")),
        ("display_folder", p.take("display_folder", "displayFolder")),
        ("summarize_by", p.take("summarize_by", "summarizeBy")),
        ("sort_by_column", p.take("sort_by_column", "sortByColumn")),
        ("data_category", p.take("data_category", "dataCategory")),
        ("description", p.text("description", "description")),
        ("hidden", p.flag("is_hidden", "isHidden", false)),
        ("is_key", p.flag("is_key", "isKey", false)),
        ("is_unique", p.flag("is_unique", "isUnique", false)),
        ("is_nullable", p.flag("is_nullable", "isNullable", true)),
        ("is_available_in_mdx", is_available_in_mdx(&mut p)),
        (
            "is_name_inferred",
            p.flag("is_name_inferred", "isNameInferred", false),
        ),
        (
            "is_data_type_inferred",
            p.flag("is_data_type_inferred", "isDataTypeInferred", false),
        ),
        (
            "is_default_label",
            p.flag("is_default_label", "isDefaultLabel", false),
        ),
        (
            "is_default_image",
            p.flag("is_default_image", "isDefaultImage", false),
        ),
        (
            "keep_unique_rows",
            p.flag("keep_unique_rows", "keepUniqueRows", false),
        ),
        ("encoding_hint", p.take("encoding_hint", "encodingHint")),
        ("alignment", p.take("alignment", "alignment")),
        (
            "source_provider_type",
            p.take("source_provider_type", "sourceProviderType"),
        ),
        ("lineage_tag", p.take("lineage_tag", "lineageTag")),
        (
            "source_lineage_tag",
            p.take("source_lineage_tag", "sourceLineageTag"),
        ),
        ("alternate_of", p.take("alternate_of", "alternateOf")),
        ("variations", list(p.take("variations", "variations"))),
        ("annotations", p.annotations()),
    ]);
    p.finish(out)
}

/// `isAvailableInMdx` (Desktop also writes `isAvailableInMDX`); TMSL default true.
fn is_available_in_mdx(p: &mut Props) -> Value {
    let v = p.take("is_available_in_mdx", "isAvailableInMdx");
    let alt = p.extra.remove("isAvailableInMDX");
    Value::Bool(v.as_bool().or(alt.and_then(|a| a.as_bool())).unwrap_or(true))
}

fn list(v: Value) -> Value {
    match v {
        Value::Null => json!([]),
        v => v,
    }
}

pub(crate) fn measure_json(table: &str, m: &Measure) -> Value {
    let mut p = Props::of(m);
    let out = obj(vec![
        ("name", p.take("name", "name")),
        ("table", json!(table)),
        ("expression", p.text("expression", "expression")),
        ("format_string", p.take("format_string", "formatString")),
        (
            "format_string_definition",
            p.expression("format_string_definition", "formatStringDefinition"),
        ),
        ("display_folder", p.take("display_folder", "displayFolder")),
        ("description", p.text("description", "description")),
        ("hidden", p.flag("is_hidden", "isHidden", false)),
        ("data_type", p.take("data_type", "dataType")),
        ("data_category", p.take("data_category", "dataCategory")),
        (
            "is_simple_measure",
            p.flag("is_simple_measure", "isSimpleMeasure", false),
        ),
        (
            "detail_rows_definition",
            p.expression("detail_rows_definition", "detailRowsDefinition"),
        ),
        ("kpi", p.take("kpi", "kpi")),
        ("lineage_tag", p.take("lineage_tag", "lineageTag")),
        (
            "source_lineage_tag",
            p.take("source_lineage_tag", "sourceLineageTag"),
        ),
        ("annotations", p.annotations()),
    ]);
    p.finish(out)
}

fn cross_filter(raw: &Value) -> &'static str {
    match raw.as_str() {
        Some("bothDirections") => "both",
        Some("automatic") => "automatic",
        Some("oneDirection") | None => "single",
        Some(_) => "other",
    }
}

pub(crate) fn relationship_json(r: &Relationship) -> Value {
    let mut p = Props::of(r);
    let raw_cross = p.take("cross_filtering_behavior", "crossFilteringBehavior");
    let from_table = p.take("from_table", "fromTable");
    let from_column = p.take("from_column", "fromColumn");
    let to_table = p.take("to_table", "toTable");
    let to_column = p.take("to_column", "toColumn");
    let mut out = obj(vec![
        ("name", p.take("name", "name")),
        (
            "from",
            json!(format!(
                "{}[{}]",
                from_table.as_str().unwrap_or(""),
                from_column.as_str().unwrap_or("")
            )),
        ),
        (
            "to",
            json!(format!(
                "{}[{}]",
                to_table.as_str().unwrap_or(""),
                to_column.as_str().unwrap_or("")
            )),
        ),
        ("from_table", from_table),
        ("from_column", from_column),
        ("to_table", to_table),
        ("to_column", to_column),
        ("is_active", p.flag("is_active", "isActive", true)),
        ("cross_filter", json!(cross_filter(&raw_cross))),
    ]);
    out.insert(
        "cross_filtering_behavior".into(),
        if raw_cross.is_null() {
            json!("oneDirection")
        } else {
            raw_cross
        },
    );
    let from_card = p.take("from_cardinality", "fromCardinality");
    let to_card = p.take("to_cardinality", "toCardinality");
    out.insert(
        "from_cardinality".into(),
        if from_card.is_null() {
            json!("many")
        } else {
            from_card
        },
    );
    out.insert(
        "to_cardinality".into(),
        if to_card.is_null() { json!("one") } else { to_card },
    );
    let security = p.take("security_filtering_behavior", "securityFilteringBehavior");
    out.insert(
        "security_filtering_behavior".into(),
        if security.is_null() {
            json!("oneDirection")
        } else {
            security
        },
    );
    out.insert(
        "rely_on_referential_integrity".into(),
        p.flag(
            "rely_on_referential_integrity",
            "relyOnReferentialIntegrity",
            false,
        ),
    );
    let join = p.take("join_on_date_behavior", "joinOnDateBehavior");
    out.insert(
        "join_on_date_behavior".into(),
        if join.is_null() {
            json!("dateAndTime")
        } else {
            join
        },
    );
    out.insert("type".into(), {
        let t = p.take("type", "type");
        if t.is_null() { json!("singleColumn") } else { t }
    });
    out.insert("annotations".into(), p.annotations());
    p.finish(out)
}

fn partition_json(x: &Partition) -> Value {
    let mut p = Props::of(x);
    let mut source = match p.typed.remove("source_extra") {
        Some(Value::Object(m)) => m,
        _ => Map::new(),
    };
    let source_type = p.take("source_type", "sourceType");
    let out = obj(vec![
        ("name", p.take("name", "name")),
        ("mode", p.take("mode", "mode")),
        ("source_type", source_type),
        ("expression", p.text("expression", "expression")),
        ("data_view", p.take("data_view", "dataView")),
        ("query_group", p.take("query_group", "queryGroup")),
        ("description", p.text("description", "description")),
        ("data_source", source.remove("dataSource").unwrap_or(Value::Null)),
        ("source", Value::Object(source)),
        ("annotations", p.annotations()),
    ]);
    p.finish(out)
}

fn hierarchy_json(v: &Value) -> Value {
    let mut p = Props::from_tmsl(v);
    let levels: Vec<Value> = p
        .take("levels", "levels")
        .as_array()
        .map(|a| {
            a.iter()
                .map(|l| {
                    let mut q = Props::from_tmsl(l);
                    let out = obj(vec![
                        ("name", q.take("name", "name")),
                        ("column", q.take("column", "column")),
                        ("ordinal", q.take("ordinal", "ordinal")),
                        ("description", q.text("description", "description")),
                        ("lineage_tag", q.take("lineage_tag", "lineageTag")),
                        ("annotations", q.annotations()),
                    ]);
                    q.finish(out)
                })
                .collect()
        })
        .unwrap_or_default();
    let out = obj(vec![
        ("name", p.take("name", "name")),
        ("description", p.text("description", "description")),
        ("hidden", p.flag("is_hidden", "isHidden", false)),
        ("display_folder", p.take("display_folder", "displayFolder")),
        ("lineage_tag", p.take("lineage_tag", "lineageTag")),
        ("levels", Value::Array(levels)),
        ("annotations", p.annotations()),
    ]);
    p.finish(out)
}

fn calculation_group_json(v: &Value) -> Value {
    // TMSL (camelCase) and typed (snake_case) shapes are read alike.
    let mut p = Props::from_tmsl(v);
    let items_raw = match p.take("calculation_items", "calculationItems") {
        Value::Null => p.take("items", "items"),
        v => v,
    };
    let items: Vec<Value> = items_raw
        .as_array()
        .map(|a| {
            a.iter()
                .map(|i| {
                    let mut q = Props::from_tmsl(i);
                    let out = obj(vec![
                        ("name", q.take("name", "name")),
                        ("expression", q.text("expression", "expression")),
                        ("ordinal", q.take("ordinal", "ordinal")),
                        (
                            "format_string_definition",
                            q.expression("format_string_definition", "formatStringDefinition"),
                        ),
                        ("description", q.text("description", "description")),
                        ("annotations", q.annotations()),
                    ]);
                    q.finish(out)
                })
                .collect()
        })
        .unwrap_or_default();
    let out = obj(vec![
        ("precedence", p.take("precedence", "precedence")),
        ("description", p.text("description", "description")),
        (
            "multiple_or_empty_selection_expression",
            p.expression(
                "multiple_or_empty_selection_expression",
                "multipleOrEmptySelectionExpression",
            ),
        ),
        (
            "no_selection_expression",
            p.expression("no_selection_expression", "noSelectionExpression"),
        ),
        ("items", Value::Array(items)),
        ("annotations", p.annotations()),
    ]);
    p.finish(out)
}

/// What a table is: `calculation_group`, `calculated` (a DAX partition) or `data`.
fn table_kind(t: &Table, calc_group: &Value) -> &'static str {
    if !calc_group.is_null() {
        "calculation_group"
    } else if t
        .partitions
        .iter()
        .any(|p| p.source_type.eq_ignore_ascii_case("calculated"))
    {
        "calculated"
    } else {
        "data"
    }
}

pub(crate) fn table_json(t: &Table) -> Value {
    let mut p = Props::of(t);
    for k in ["columns", "measures", "partitions"] {
        p.typed.remove(k);
    }
    let calc_group = match p.take("calculation_group", "calculationGroup") {
        Value::Null => Value::Null,
        v => calculation_group_json(&v),
    };
    let hierarchies: Vec<Value> = p
        .take("hierarchies", "hierarchies")
        .as_array()
        .map(|a| a.iter().map(hierarchy_json).collect())
        .unwrap_or_default();
    let out = obj(vec![
        ("name", p.take("name", "name")),
        ("kind", json!(table_kind(t, &calc_group))),
        ("description", p.text("description", "description")),
        ("hidden", p.flag("is_hidden", "isHidden", false)),
        ("is_private", p.flag("is_private", "isPrivate", false)),
        ("data_category", p.take("data_category", "dataCategory")),
        (
            "show_as_variations_only",
            p.flag("show_as_variations_only", "showAsVariationsOnly", false),
        ),
        (
            "exclude_from_model_refresh",
            p.flag("exclude_from_model_refresh", "excludeFromModelRefresh", false),
        ),
        ("lineage_tag", p.take("lineage_tag", "lineageTag")),
        (
            "source_lineage_tag",
            p.take("source_lineage_tag", "sourceLineageTag"),
        ),
        (
            "columns",
            Value::Array(
                t.columns
                    .iter()
                    .filter(|c| !c.is_row_number())
                    .map(|c| column_json(&t.name, c))
                    .collect(),
            ),
        ),
        (
            "measures",
            Value::Array(t.measures.iter().map(|m| measure_json(&t.name, m)).collect()),
        ),
        ("hierarchies", Value::Array(hierarchies)),
        (
            "partitions",
            Value::Array(t.partitions.iter().map(partition_json).collect()),
        ),
        ("calculation_group", calc_group),
        ("refresh_policy", p.take("refresh_policy", "refreshPolicy")),
        ("annotations", p.annotations()),
    ]);
    p.finish(out)
}

fn role_json(r: &Role) -> Value {
    let mut p = Props::of(r);
    let permissions: Vec<Value> = r
        .table_permissions
        .iter()
        .map(|tp| {
            let mut q = Props::of(tp);
            let out = obj(vec![
                ("table", q.take("table", "name")),
                (
                    "filter_expression",
                    q.text("filter_expression", "filterExpression"),
                ),
                (
                    "metadata_permission",
                    q.take("metadata_permission", "metadataPermission"),
                ),
                (
                    "column_permissions",
                    list(q.take("column_permissions", "columnPermissions")),
                ),
                ("annotations", q.annotations()),
            ]);
            q.finish(out)
        })
        .collect();
    p.typed.remove("table_permissions");
    let out = obj(vec![
        ("name", p.take("name", "name")),
        ("description", p.text("description", "description")),
        ("model_permission", p.take("model_permission", "modelPermission")),
        ("members", list(p.take("members", "members"))),
        ("table_permissions", Value::Array(permissions)),
        ("annotations", p.annotations()),
    ]);
    p.finish(out)
}

fn expression_json(e: &bicraft_model::NamedExpression) -> Value {
    let mut p = Props::of(e);
    let out = obj(vec![
        ("name", p.take("name", "name")),
        ("kind", p.take("kind", "kind")),
        ("expression", p.text("expression", "expression")),
        ("description", p.text("description", "description")),
        ("query_group", p.take("query_group", "queryGroup")),
        ("lineage_tag", p.take("lineage_tag", "lineageTag")),
        ("annotations", p.annotations()),
    ]);
    p.finish(out)
}

fn named_list(v: &Value) -> Vec<Value> {
    v.as_array().cloned().unwrap_or_default()
}

/// The whole model (`detail: "full"`).
pub(crate) fn model_full(m: &SemanticModel) -> Value {
    let mut p = Props::of(m);
    for k in [
        "tables",
        "relationships",
        "roles",
        "expressions",
        "database_extra",
    ] {
        p.typed.remove(k);
    }
    let out = obj(vec![
        ("name", p.take("name", "name")),
        (
            "compatibility_level",
            p.take("compatibility_level", "compatibilityLevel"),
        ),
        ("culture", p.take("culture", "culture")),
        ("description", p.text("description", "description")),
        ("default_mode", p.take("default_mode", "defaultMode")),
        (
            "discourage_implicit_measures",
            p.flag(
                "discourage_implicit_measures",
                "discourageImplicitMeasures",
                false,
            ),
        ),
        (
            "source_query_culture",
            p.take("source_query_culture", "sourceQueryCulture"),
        ),
        (
            "data_access_options",
            p.take("data_access_options", "dataAccessOptions"),
        ),
        (
            "default_power_bi_data_source_version",
            p.take(
                "default_power_bi_data_source_version",
                "defaultPowerBIDataSourceVersion",
            ),
        ),
        ("tables", Value::Array(m.tables.iter().map(table_json).collect())),
        (
            "relationships",
            Value::Array(m.relationships.iter().map(relationship_json).collect()),
        ),
        ("roles", Value::Array(m.roles.iter().map(role_json).collect())),
        (
            "expressions",
            Value::Array(m.expressions.iter().map(expression_json).collect()),
        ),
        (
            "perspectives",
            Value::Array(named_list(&p.take("perspectives", "perspectives"))),
        ),
        (
            "cultures",
            Value::Array(named_list(&p.take("cultures", "cultures"))),
        ),
        (
            "data_sources",
            Value::Array(named_list(&p.take("data_sources", "dataSources"))),
        ),
        (
            "query_groups",
            Value::Array(named_list(&p.take("query_groups", "queryGroups"))),
        ),
        (
            "functions",
            Value::Array(named_list(&p.take("functions", "functions"))),
        ),
        ("annotations", p.annotations()),
        ("database", Value::Object(m.database_extra.clone())),
    ]);
    p.finish(out)
}

fn count(v: Option<&Value>) -> usize {
    v.and_then(Value::as_array).map_or(0, Vec::len)
}

/// The compact form (`detail: "summary"`): model properties, counts, one line per table.
fn model_brief(m: &SemanticModel) -> Value {
    let full = model_full(m);
    let tables = full
        .get("tables")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut columns = 0usize;
    let mut measures = 0usize;
    let mut hierarchies = 0usize;
    let mut partitions = 0usize;
    let mut calc_groups = 0usize;
    let brief: Vec<Value> = tables
        .iter()
        .map(|t| {
            let (c, x, h, pa) = (
                count(t.get("columns")),
                count(t.get("measures")),
                count(t.get("hierarchies")),
                count(t.get("partitions")),
            );
            columns += c;
            measures += x;
            hierarchies += h;
            partitions += pa;
            if t.get("kind").and_then(Value::as_str) == Some("calculation_group") {
                calc_groups += 1;
            }
            json!({
                "name": t["name"],
                "kind": t["kind"],
                "hidden": t["hidden"],
                "data_category": t["data_category"],
                "columns": c,
                "measures": x,
                "hierarchies": h,
                "partitions": t.get("partitions").and_then(Value::as_array).map(|a| a.iter().map(|p| json!({
                    "name": p["name"], "mode": p["mode"], "source_type": p["source_type"],
                })).collect::<Vec<_>>()).unwrap_or_default(),
            })
        })
        .collect();
    let keys = [
        "name",
        "compatibility_level",
        "culture",
        "description",
        "default_mode",
        "discourage_implicit_measures",
        "source_query_culture",
    ];
    let mut out: Map<String, Value> = keys
        .iter()
        .map(|k| (k.to_string(), full.get(*k).cloned().unwrap_or(Value::Null)))
        .collect();
    out.insert(
        "counts".into(),
        json!({
            "tables": tables.len(),
            "columns": columns,
            "measures": measures,
            "hierarchies": hierarchies,
            "partitions": partitions,
            "calculation_groups": calc_groups,
            "relationships": count(full.get("relationships")),
            "roles": count(full.get("roles")),
            "expressions": count(full.get("expressions")),
            "perspectives": count(full.get("perspectives")),
            "cultures": count(full.get("cultures")),
        }),
    );
    out.insert("tables".into(), Value::Array(brief));
    Value::Object(out)
}

pub(crate) fn model_get(s: &Session, params: &Value) -> Result<Value, String> {
    allow(params, &["detail", "culture"])?;
    let refresh = crate::refresh_info::load(s);
    let m = model(s)?;
    let captions = captions_param(m, params)?;
    let mut out = match opt_str(params, "detail")?.unwrap_or("summary") {
        "summary" => model_brief(m),
        "full" => {
            let mut v = model_full(m);
            if let Some(r) = &refresh {
                crate::refresh_info::annotate_model(r, &mut v);
            }
            v
        }
        other => {
            return Err(format!(
                "detail: expected \"summary\" or \"full\", got \"{other}\""
            ));
        }
    };
    if let Some(c) = &captions {
        localize_model(c, &mut out);
    }
    Ok(out)
}

/// `culture` on model.get: translated captions on the model, its tables (and, in full reads,
/// their columns, measures, hierarchies and levels), perspectives, roles and expressions.
fn localize_model(c: &Captions, out: &mut Value) {
    c.apply(out, &[]);
    if let Some(o) = out.as_object_mut() {
        o.insert("translation_culture".into(), json!(c.culture));
    }
    for t in out
        .get_mut("tables")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        if t.get("columns").is_some_and(Value::is_array) {
            c.apply_table(t);
        } else {
            let n = t
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            c.apply(t, &[("table", &n)]);
        }
    }
    for (key, kind) in [
        ("perspectives", "perspective"),
        ("roles", "role"),
        ("expressions", "expression"),
    ] {
        for x in out
            .get_mut(key)
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            let n = x
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            c.apply(x, &[(kind, &n)]);
        }
    }
}

/// Applies `culture` to one table / column / measure read.
fn localize(c: Option<&Captions>, v: &mut Value, path: &[(&str, &str)]) {
    if let Some(c) = c {
        c.apply(v, path);
        if let Some(o) = v.as_object_mut() {
            o.insert("translation_culture".into(), json!(c.culture));
        }
    }
}

fn find_table<'m>(m: &'m SemanticModel, name: &str) -> Result<&'m Table, String> {
    m.table(name).ok_or_else(|| format!("no table named '{name}'"))
}

pub(crate) fn table_get(s: &Session, params: &Value) -> Result<Value, String> {
    allow(params, &["name", "culture"])?;
    let refresh = crate::refresh_info::load(s);
    let m = model(s)?;
    let captions = captions_param(m, params)?;
    let mut v = table_json(find_table(m, str_param(params, "name")?)?);
    if let Some(r) = &refresh {
        crate::refresh_info::annotate_table(r, &mut v);
    }
    if let Some(c) = &captions {
        c.apply_table(&mut v);
        if let Some(o) = v.as_object_mut() {
            o.insert("translation_culture".into(), json!(c.culture));
        }
    }
    Ok(v)
}

pub(crate) fn column_get(s: &Session, params: &Value) -> Result<Value, String> {
    allow(params, &["table", "name", "culture"])?;
    let m = model(s)?;
    let captions = captions_param(m, params)?;
    let t = find_table(m, str_param(params, "table")?)?;
    let name = str_param(params, "name")?;
    let c = t
        .column(name)
        .ok_or_else(|| format!("no column '{}'[{name}]", t.name))?;
    let mut v = column_json(&t.name, c);
    // The relationships using the column, as relationship.list returns them.
    let used: Vec<Value> = m
        .relationships
        .iter()
        .filter(|r| {
            (r.from_table.eq_ignore_ascii_case(&t.name) && r.from_column.eq_ignore_ascii_case(&c.name))
                || (r.to_table.eq_ignore_ascii_case(&t.name) && r.to_column.eq_ignore_ascii_case(&c.name))
        })
        .map(|r| json!(r.name))
        .collect();
    if let Some(o) = v.as_object_mut() {
        o.insert("relationships".into(), Value::Array(used));
    }
    localize(
        captions.as_ref(),
        &mut v,
        &[("table", &t.name), ("column", &c.name)],
    );
    Ok(v)
}

pub(crate) fn measure_get(s: &Session, params: &Value) -> Result<Value, String> {
    allow(params, &["name", "table", "culture"])?;
    let m = model(s)?;
    let captions = captions_param(m, params)?;
    let name = str_param(params, "name")?;
    let (t, x) = match opt_str(params, "table")? {
        Some(table) => {
            let t = find_table(m, table)?;
            (
                t,
                t.measure(name)
                    .ok_or_else(|| format!("no measure '{}'[{name}]", t.name))?,
            )
        }
        None => m
            .measure(name)
            .ok_or_else(|| format!("no measure named '{name}'"))?,
    };
    let mut v = measure_json(&t.name, x);
    localize(
        captions.as_ref(),
        &mut v,
        &[("table", &t.name), ("measure", &x.name)],
    );
    Ok(v)
}

pub(crate) fn relationship_list(s: &Session, params: &Value) -> Result<Value, String> {
    allow(params, &["table"])?;
    let m = model(s)?;
    let table = opt_str(params, "table")?;
    if let Some(t) = table {
        find_table(m, t)?;
    }
    let list: Vec<Value> = m
        .relationships
        .iter()
        .filter(|r| {
            table.is_none_or(|t| r.from_table.eq_ignore_ascii_case(t) || r.to_table.eq_ignore_ascii_case(t))
        })
        .map(relationship_json)
        .collect();
    Ok(json!({"count": list.len(), "relationships": list}))
}
