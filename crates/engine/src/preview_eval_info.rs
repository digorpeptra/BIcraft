//! INFO functions of the preview evaluator ([`super`]): `INFO.TABLES`, `INFO.PARTITIONS`,
//! `INFO.MODEL`, `INFO.COLUMNS`, `INFO.MEASURES`, `INFO.RELATIONSHIPS` (the `TMSCHEMA_*` schema
//! rowsets as DAX tables) and the four `INFO.VIEW.*` (names instead of IDs).
//!
//! Columns, their order and their types follow Microsoft's DAX reference pages of each function
//! (`docs/formats/dax-functions.md`, "INFO functions", which marks what is documented and what
//! was measured in Desktop). The values come from the session's model, and the refresh times
//! from the open file's backup (`refresh_info.rs`) or from bicraft's own refresh snapshot
//! (`model.refresh`): `RefreshedTime`, `ModifiedTime` and `StructureModifiedTime` are blank when
//! the file holds no data (a PBIT, a new model, a PBIP never refreshed).
//!
//! What differs from the engine, by design:
//! - IDs are bicraft's own numbering (1, 2, … in model order per object kind), consistent
//!   between the functions (`INFO.PARTITIONS()[TableID]` is `INFO.TABLES()[ID]`), not the
//!   engine's.
//! - Only the objects of the model are listed: the engine's own `RowNumber` columns and system
//!   tables appear only when the model holds them.
//! - Restrictions (`INFO.TABLES("Name", "Sales")`) filter on the result's columns, by name, case
//!   insensitively; the engine also has a few restrictions that are not columns.
//!
//! Outside queries, every `INFO.*` but `INFO.VIEW.*` fails, as in the engine: the validator
//! refuses the definition (`info-query-only`), and this evaluator refuses the call inside a
//! measure of the model.

use bicraft_dax::ast::Expr;
use bicraft_model::{Column, DataType, SemanticModel, Table};
use serde_json::Value;

use super::super::Fail;
use super::tables::parse_date_text;
use super::{Engine, Scope, TCol, Tab, V, arg, compare_vals, dax_error, text_of};
use crate::refresh_info::{RefreshInfo, TableRefresh};

/// Most rows an INFO function returns (one per object of the model).
const MAX_ROWS: usize = 1_000_000;

use bicraft_dax::info::{InfoType as T, info_columns};

fn data_type(t: T) -> DataType {
    match t {
        T::Int => DataType::Int64,
        T::Str => DataType::String,
        T::Bool => DataType::Boolean,
        T::Time => DataType::DateTime,
    }
}

/// The INFO functions this module evaluates, with their columns.
fn layout(up: &str) -> Option<&'static [(&'static str, T)]> {
    info_columns(up)
}

// --- values -----------------------------------------------------------------------------------

fn s(v: Option<&str>) -> V {
    v.map_or(V::Blank, |t| V::Text(t.to_string()))
}

fn i(n: i64) -> V {
    V::Num(n as f64)
}

/// A 1-based position as an ID.
fn id(pos: usize) -> V {
    i(i64::try_from(pos).unwrap_or(i64::MAX).saturating_add(1))
}

fn b(v: bool) -> V {
    V::Bool(v)
}

/// A FILETIME value (100 ns ticks since 1601-01-01 UTC) as a DAX date/time (days since
/// 1899-12-30, UTC); blank for "never" and out-of-range values.
pub(crate) fn filetime(ticks: Option<i64>) -> V {
    match ticks.and_then(crate::refresh_info::filetime_serial) {
        Some(x) => V::Date(x),
        None => V::Blank,
    }
}

fn extra_str<'a>(extra: &'a serde_json::Map<String, Value>, key: &str) -> Option<&'a str> {
    extra.get(key).and_then(Value::as_str)
}

fn extra_bool(extra: &serde_json::Map<String, Value>, key: &str) -> bool {
    extra.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn extra_int(extra: &serde_json::Map<String, Value>, key: &str) -> V {
    extra.get(key).and_then(Value::as_i64).map_or(V::Blank, i)
}

/// A JSON property as text (objects and arrays serialized); blank when absent.
fn extra_json(extra: &serde_json::Map<String, Value>, key: &str) -> V {
    match extra.get(key) {
        None | Some(Value::Null) => V::Blank,
        Some(Value::String(t)) => V::Text(t.clone()),
        Some(other) => V::Text(other.to_string()),
    }
}

/// The TOM `DataType` code of a column type (`docs/formats/dax-functions.md`).
fn data_type_code(t: DataType) -> i64 {
    match t {
        DataType::String => 2,
        DataType::Int64 => 6,
        DataType::Double => 8,
        DataType::DateTime => 9,
        DataType::Decimal => 10,
        DataType::Boolean => 11,
        DataType::Binary => 17,
        DataType::Variant => 20,
        DataType::Unknown => 19,
    }
}

/// The type name `INFO.VIEW.COLUMNS` and `INFO.VIEW.MEASURES` give (the DAX data type names).
fn data_type_name(t: DataType) -> &'static str {
    match t {
        DataType::String => "Text",
        DataType::Int64 => "Whole Number",
        DataType::Double => "Decimal Number",
        DataType::DateTime => "Date",
        DataType::Decimal => "Currency",
        DataType::Boolean => "True/False",
        DataType::Binary => "Binary",
        DataType::Variant | DataType::Unknown => "Variant",
    }
}

/// The TOM `ColumnType`: name and code.
fn column_type(t: &Table, c: &Column) -> (&'static str, i64) {
    let calculated_table = t
        .partitions
        .iter()
        .any(|p| p.source_type.eq_ignore_ascii_case("calculated"));
    match extra_str(&c.extra, "type") {
        Some(k) if k.eq_ignore_ascii_case("rowNumber") => ("RowNumber", 3),
        Some(k) if k.eq_ignore_ascii_case("calculatedTableColumn") => ("CalculatedTableColumn", 4),
        _ if c.expression.is_some() => ("Calculated", 2),
        _ if calculated_table => ("CalculatedTableColumn", 4),
        _ => ("Data", 1),
    }
}

/// The TOM `AggregateFunction` of `summarizeBy`: name and code.
fn summarize_by(c: &Column) -> (&'static str, i64) {
    const NAMES: &[(&str, i64)] = &[
        ("Default", 1),
        ("None", 2),
        ("Sum", 3),
        ("Min", 4),
        ("Max", 5),
        ("Count", 6),
        ("Average", 7),
        ("DistinctCount", 8),
    ];
    let given = c.summarize_by.as_deref().unwrap_or("default");
    NAMES
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(given))
        .copied()
        .unwrap_or(("Default", 1))
}

/// The TOM `Alignment`: name and code.
fn alignment(c: &Column) -> (&'static str, i64) {
    match extra_str(&c.extra, "alignment")
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("left") => ("Left", 2),
        Some("right") => ("Right", 3),
        Some("center") => ("Center", 4),
        _ => ("Default", 1),
    }
}

/// The TOM `PartitionSourceType` code of a partition's source type.
fn partition_type(source_type: &str) -> i64 {
    match source_type.to_ascii_lowercase().as_str() {
        "query" => 1,
        "calculated" => 2,
        "none" => 3,
        "m" => 4,
        "entity" => 5,
        "policyrange" => 6,
        "calculationgroup" => 7,
        "inferred" => 8,
        "parquet" => 9,
        _ => 4,
    }
}

/// The TOM `ModeType`: name and code (`Default`: the model's mode).
fn mode(m: Option<&str>) -> (&'static str, i64) {
    match m.map(str::to_ascii_lowercase).as_deref() {
        Some("import") => ("Import", 0),
        Some("directquery") => ("DirectQuery", 1),
        Some("push") => ("Push", 3),
        Some("dual") => ("Dual", 4),
        Some("directlake") => ("DirectLake", 5),
        _ => ("Default", 2),
    }
}

/// The TOM `CrossFilteringBehavior`: name and code.
fn cross_filter(v: Option<&str>) -> (&'static str, i64) {
    match v.map(str::to_ascii_lowercase).as_deref() {
        Some("bothdirections") => ("BothDirections", 2),
        Some("automatic") => ("Automatic", 3),
        _ => ("OneDirection", 1),
    }
}

/// The TOM `RelationshipEndCardinality`: name and code (`from` defaults to many, `to` to one).
fn cardinality(v: Option<&str>, from: bool) -> (&'static str, i64) {
    match v.map(str::to_ascii_lowercase).as_deref() {
        Some("one") => ("One", 1),
        Some("many") => ("Many", 2),
        Some("none") => ("None", 0),
        _ if from => ("Many", 2),
        _ => ("One", 1),
    }
}

/// The TOM `SecurityFilteringBehavior`: name and code.
fn security_filter(v: Option<&str>) -> (&'static str, i64) {
    match v.map(str::to_ascii_lowercase).as_deref() {
        Some("bothdirections") => ("BothDirections", 2),
        Some("none") => ("None", 3),
        _ => ("OneDirection", 1),
    }
}

/// The refresh state of each table, when the session has one: the backup's, or bicraft's own
/// snapshot (all its tables refreshed at its `refreshed_at`, state Ready).
struct Times {
    backup: Option<std::sync::Arc<RefreshInfo>>,
    /// `model.refresh`'s snapshot: its time and the tables it holds.
    own: Option<(Option<f64>, Vec<String>)>,
}

impl Times {
    fn table(&self, name: &str) -> Option<&TableRefresh> {
        self.backup.as_ref().and_then(|b| b.table(name))
    }

    /// The snapshot's refresh time of a table it holds.
    fn own_time(&self, table: &str) -> Option<Option<f64>> {
        let (at, tables) = self.own.as_ref()?;
        tables
            .iter()
            .any(|t| t.eq_ignore_ascii_case(table))
            .then_some(*at)
    }

    /// A partition's refresh time and state.
    fn partition(&self, table: &str, partition: &str) -> (V, V, V, V) {
        if let Some(at) = self.own_time(table) {
            let at = at.map_or(V::Blank, V::Date);
            return (at.clone(), at, i(1), V::Blank);
        }
        match self.table(table).and_then(|t| t.partition(partition)) {
            Some(p) => (
                filetime(p.refreshed),
                filetime(p.modified),
                p.state.map_or(V::Blank, i),
                s(p.error.as_deref()),
            ),
            None => (V::Blank, V::Blank, V::Blank, V::Blank),
        }
    }

    /// A table's `ModifiedTime` and `StructureModifiedTime`.
    fn table_times(&self, table: &str) -> (V, V) {
        match self.table(table) {
            Some(t) => (filetime(t.modified), filetime(t.structure_modified)),
            None => (V::Blank, V::Blank),
        }
    }

    /// A table's last refresh (its latest partition).
    fn table_refreshed(&self, table: &str) -> V {
        if let Some(at) = self.own_time(table) {
            return at.map_or(V::Blank, V::Date);
        }
        filetime(
            self.table(table)
                .and_then(|t| t.partitions.iter().filter_map(|p| p.refreshed).max()),
        )
    }
}

/// The model's tables with their 1-based IDs, and a column ID lookup.
struct Ids<'m> {
    model: &'m SemanticModel,
}

impl Ids<'_> {
    fn table(&self, name: &str) -> V {
        match self
            .model
            .tables
            .iter()
            .position(|t| t.name.eq_ignore_ascii_case(name))
        {
            Some(p) => id(p),
            None => V::Blank,
        }
    }

    /// Columns are numbered in model order across tables.
    fn column(&self, table: &str, column: &str) -> V {
        let mut n = 0usize;
        for t in &self.model.tables {
            for c in &t.columns {
                if t.name.eq_ignore_ascii_case(table) && c.name.eq_ignore_ascii_case(column) {
                    return id(n);
                }
                n = n.saturating_add(1);
            }
        }
        V::Blank
    }
}

/// The model's ID in `INFO.VIEW.*` `[Model]` (a name in bicraft: the engine gives a GUID).
fn model_ref(m: &SemanticModel) -> V {
    V::Text(m.name.clone())
}

fn storage_mode(t: &Table) -> &'static str {
    let first = t.partitions.first().and_then(|p| p.mode.as_deref());
    mode(first).0
}

fn calculated_expression(t: &Table) -> Option<&str> {
    t.partitions
        .iter()
        .find(|p| p.source_type.eq_ignore_ascii_case("calculated"))
        .and_then(|p| p.expression.as_deref())
}

fn is_calculation_group(t: &Table) -> bool {
    t.extra.contains_key("calculationGroup")
}

fn rows_for(up: &str, m: &SemanticModel, times: &Times) -> Vec<Vec<V>> {
    let ids = Ids { model: m };
    let mut rows: Vec<Vec<V>> = Vec::new();
    match up {
        "INFO.TABLES" => {
            for (n, t) in m.tables.iter().enumerate() {
                let (modified, structure) = times.table_times(&t.name);
                rows.push(vec![
                    id(n),
                    i(1),
                    V::Text(t.name.clone()),
                    s(extra_str(&t.extra, "dataCategory")),
                    s(t.description.as_deref()),
                    b(t.is_hidden),
                    V::Blank,
                    modified,
                    structure,
                    i(0),
                    b(extra_bool(&t.extra, "showAsVariationsOnly")),
                    b(extra_bool(&t.extra, "isPrivate")),
                    V::Blank,
                    extra_int(&t.extra, "alternateSourcePrecedence"),
                    V::Blank,
                    if is_calculation_group(t) { id(n) } else { V::Blank },
                    b(extra_bool(&t.extra, "excludeFromModelRefresh")),
                    s(t.lineage_tag.as_deref()),
                    s(extra_str(&t.extra, "sourceLineageTag")),
                    b(extra_bool(&t.extra, "systemManaged")),
                ]);
            }
        }
        "INFO.PARTITIONS" => {
            let mut n = 0usize;
            for t in &m.tables {
                for p in &t.partitions {
                    let (refreshed, modified, state, error) = times.partition(&t.name, &p.name);
                    rows.push(vec![
                        id(n),
                        ids.table(&t.name),
                        V::Text(p.name.clone()),
                        s(extra_str(&p.extra, "description")),
                        V::Blank,
                        s(p.expression.as_deref()),
                        state,
                        i(partition_type(&p.source_type)),
                        V::Blank,
                        i(mode(p.mode.as_deref()).1),
                        V::Blank,
                        modified,
                        refreshed,
                        i(0),
                        error,
                        b(false),
                        s(extra_str(&p.source_extra, "start")),
                        s(extra_str(&p.source_extra, "end")),
                        V::Blank,
                        s(extra_str(&p.extra, "refreshBookmark")),
                        V::Blank,
                        V::Blank,
                        V::Blank,
                    ]);
                    n = n.saturating_add(1);
                }
            }
        }
        "INFO.MODEL" => {
            let x = &m.extra;
            let (modified, structure) = match &times.backup {
                Some(r) => (filetime(r.modified), filetime(r.structure_modified)),
                None => (V::Blank, V::Blank),
            };
            let pbi_version = extra_str(x, "defaultPowerBIDataSourceVersion")
                .and_then(|v| v.strip_prefix("powerBI_V").and_then(|n| n.parse::<i64>().ok()));
            rows.push(vec![
                i(1),
                V::Text(m.name.clone()),
                s(m.description.as_deref()),
                V::Blank,
                i(i64::from(mode(extra_str(x, "defaultMode")).1 == 1)),
                V::Blank,
                s(m.culture.as_deref()),
                s(extra_str(x, "collation")),
                modified,
                structure,
                V::Blank,
                extra_json(x, "dataAccessOptions"),
                V::Blank,
                pbi_version.map_or(V::Blank, i),
                b(extra_bool(x, "forceUniqueNames")),
                b(extra_bool(x, "discourageImplicitMeasures")),
                V::Blank,
                extra_int(x, "dataSourceDefaultMaxConnections"),
                s(extra_str(x, "sourceQueryCulture")),
                V::Blank,
                b(extra_bool(x, "discourageCompositeModels")),
                extra_json(x, "automaticAggregationOptions"),
                V::Blank,
                V::Blank,
            ]);
        }
        "INFO.COLUMNS" => {
            let mut n = 0usize;
            for t in &m.tables {
                let (modified, structure) = times.table_times(&t.name);
                let refreshed = times.table_refreshed(&t.name);
                for (pos, c) in t.columns.iter().enumerate() {
                    let ty = data_type_code(c.data_type);
                    rows.push(vec![
                        id(n),
                        ids.table(&t.name),
                        V::Text(c.name.clone()),
                        V::Text(c.name.clone()),
                        i(ty),
                        i(ty),
                        s(c.data_category.as_deref()),
                        s(c.description.as_deref()),
                        b(c.is_hidden),
                        V::Blank,
                        b(extra_bool(&c.extra, "isUnique")),
                        b(extra_bool(&c.extra, "isKey")),
                        b(c.extra.get("isNullable").and_then(Value::as_bool).unwrap_or(true)),
                        i(alignment(c).1),
                        V::Blank,
                        b(extra_bool(&c.extra, "isDefaultLabel")),
                        b(extra_bool(&c.extra, "isDefaultImage")),
                        i(summarize_by(c).1),
                        V::Blank,
                        i(column_type(t, c).1),
                        s(c.source_column.as_deref()),
                        V::Blank,
                        s(c.expression.as_deref()),
                        s(c.format_string.as_deref()),
                        b(c.extra
                            .get("isAvailableInMdx")
                            .and_then(Value::as_bool)
                            .unwrap_or(true)),
                        c.sort_by_column
                            .as_deref()
                            .map_or(V::Blank, |sb| ids.column(&t.name, sb)),
                        V::Blank,
                        modified.clone(),
                        structure.clone(),
                        refreshed.clone(),
                        i(0),
                        b(extra_bool(&c.extra, "keepUniqueRows")),
                        id(pos),
                        V::Blank,
                        s(extra_str(&c.extra, "sourceProviderType")),
                        s(c.display_folder.as_deref()),
                        V::Blank,
                        V::Blank,
                        V::Blank,
                        s(c.lineage_tag.as_deref()),
                        s(extra_str(&c.extra, "sourceLineageTag")),
                    ]);
                    n = n.saturating_add(1);
                }
            }
        }
        "INFO.MEASURES" => {
            let mut n = 0usize;
            for t in &m.tables {
                let (modified, structure) = times.table_times(&t.name);
                for me in &t.measures {
                    rows.push(vec![
                        id(n),
                        ids.table(&t.name),
                        V::Text(me.name.clone()),
                        s(me.description.as_deref()),
                        i(data_type_code(DataType::Variant)),
                        V::Text(me.expression.clone()),
                        s(me.format_string.as_deref()),
                        b(me.is_hidden),
                        i(1),
                        modified.clone(),
                        structure.clone(),
                        V::Blank,
                        b(extra_bool(&me.extra, "isSimpleMeasure")),
                        V::Blank,
                        s(me.display_folder.as_deref()),
                        V::Blank,
                        s(extra_str(&me.extra, "dataCategory")),
                        s(me.lineage_tag.as_deref()),
                        s(extra_str(&me.extra, "sourceLineageTag")),
                    ]);
                    n = n.saturating_add(1);
                }
            }
        }
        "INFO.RELATIONSHIPS" => {
            for (n, r) in m.relationships.iter().enumerate() {
                let x = &r.extra;
                rows.push(vec![
                    id(n),
                    i(1),
                    V::Text(r.name.clone()),
                    b(r.is_active),
                    i(1),
                    i(cross_filter(r.cross_filtering_behavior.as_deref()).1),
                    V::Blank,
                    b(extra_bool(x, "relyOnReferentialIntegrity")),
                    ids.table(&r.from_table),
                    ids.column(&r.from_table, &r.from_column),
                    i(cardinality(r.from_cardinality.as_deref(), true).1),
                    ids.table(&r.to_table),
                    ids.column(&r.to_table, &r.to_column),
                    i(cardinality(r.to_cardinality.as_deref(), false).1),
                    i(1),
                    V::Blank,
                    V::Blank,
                    V::Blank,
                    V::Blank,
                    i(security_filter(extra_str(x, "securityFilteringBehavior")).1),
                ]);
            }
        }
        "INFO.VIEW.TABLES" => {
            for (n, t) in m.tables.iter().enumerate() {
                rows.push(vec![
                    id(n),
                    V::Text(t.name.clone()),
                    model_ref(m),
                    s(extra_str(&t.extra, "dataCategory")),
                    s(t.description.as_deref()),
                    b(t.is_hidden),
                    V::Text(storage_mode(t).into()),
                    V::Text(format!("{} ({})", t.name, n.saturating_add(1))),
                    s(calculated_expression(t)),
                    b(extra_bool(&t.extra, "showAsVariationsOnly")),
                    b(extra_bool(&t.extra, "isPrivate")),
                    t.extra
                        .get("calculationGroup")
                        .and_then(|g| g.get("precedence"))
                        .and_then(Value::as_i64)
                        .map_or(if is_calculation_group(t) { i(0) } else { V::Blank }, i),
                    s(t.lineage_tag.as_deref()),
                    V::Blank,
                ]);
            }
        }
        "INFO.VIEW.COLUMNS" => {
            let mut n = 0usize;
            for t in &m.tables {
                for c in &t.columns {
                    let string = c.data_type == DataType::String;
                    rows.push(vec![
                        id(n),
                        V::Text(c.name.clone()),
                        V::Text(t.name.clone()),
                        V::Text(data_type_name(c.data_type).into()),
                        s(c.data_category.as_deref()),
                        s(c.description.as_deref()),
                        b(c.is_hidden),
                        b(extra_bool(&c.extra, "isUnique")),
                        b(extra_bool(&c.extra, "isKey")),
                        b(c.extra.get("isNullable").and_then(Value::as_bool).unwrap_or(true)),
                        V::Text(alignment(c).0.into()),
                        V::Text(summarize_by(c).0.into()),
                        V::Text(format!("{} ({})", c.name, n.saturating_add(1))),
                        V::Text(column_type(t, c).0.into()),
                        s(c.source_column.as_deref()),
                        s(c.expression.as_deref()),
                        s(c.format_string.as_deref()),
                        b(c.extra
                            .get("isAvailableInMdx")
                            .and_then(Value::as_bool)
                            .unwrap_or(true)),
                        s(c.sort_by_column.as_deref()),
                        s(extra_str(&c.extra, "groupingBehavior")),
                        s(extra_str(&c.extra, "sourceProviderType")),
                        s(c.display_folder.as_deref()),
                        V::Blank,
                        s(c.lineage_tag.as_deref()),
                        if string { s(Some("Default")) } else { V::Blank },
                        if string { s(Some("Default")) } else { V::Blank },
                        V::Blank,
                        V::Blank,
                    ]);
                    n = n.saturating_add(1);
                }
            }
        }
        "INFO.VIEW.MEASURES" => {
            let mut n = 0usize;
            for t in &m.tables {
                for me in &t.measures {
                    rows.push(vec![
                        id(n),
                        V::Text(me.name.clone()),
                        V::Text(t.name.clone()),
                        s(me.description.as_deref()),
                        V::Text(data_type_name(DataType::Variant).into()),
                        V::Text(me.expression.clone()),
                        s(me.format_string.as_deref()),
                        b(me.is_hidden),
                        V::Text("Ready".into()),
                        V::Blank,
                        b(extra_bool(&me.extra, "isSimpleMeasure")),
                        s(me.display_folder.as_deref()),
                        me.extra
                            .get("detailRowsDefinition")
                            .and_then(|d| d.get("expression"))
                            .map_or(V::Blank, |e| V::Text(expression_text(e))),
                        s(extra_str(&me.extra, "dataCategory")),
                        me.extra
                            .get("formatStringDefinition")
                            .and_then(|d| d.get("expression"))
                            .map_or(V::Blank, |e| V::Text(expression_text(e))),
                        s(me.lineage_tag.as_deref()),
                        V::Blank,
                    ]);
                    n = n.saturating_add(1);
                }
            }
        }
        "INFO.VIEW.RELATIONSHIPS" => {
            for (n, r) in m.relationships.iter().enumerate() {
                let x = &r.extra;
                let from = cardinality(r.from_cardinality.as_deref(), true).0;
                let to = cardinality(r.to_cardinality.as_deref(), false).0;
                let cross = cross_filter(r.cross_filtering_behavior.as_deref()).0;
                let arrow = |c: &str| if c == "Many" { "*" } else { "1" };
                rows.push(vec![
                    id(n),
                    V::Text(r.name.clone()),
                    V::Text(format!(
                        "'{}'[{}] {}-{} '{}'[{}] ({cross})",
                        r.from_table,
                        r.from_column,
                        arrow(from),
                        arrow(to),
                        r.to_table,
                        r.to_column
                    )),
                    model_ref(m),
                    b(r.is_active),
                    V::Text(cross.into()),
                    b(extra_bool(x, "relyOnReferentialIntegrity")),
                    V::Text(r.from_table.clone()),
                    V::Text(r.from_column.clone()),
                    V::Text(from.into()),
                    V::Text(r.to_table.clone()),
                    V::Text(r.to_column.clone()),
                    V::Text(to.into()),
                    V::Text("Ready".into()),
                    V::Text(
                        security_filter(extra_str(x, "securityFilteringBehavior"))
                            .0
                            .into(),
                    ),
                    V::Blank,
                ]);
            }
        }
        _ => {}
    }
    rows
}

/// A TMSL expression (a string, or an array of lines) as text.
fn expression_text(e: &Value) -> String {
    match e {
        Value::String(t) => t.clone(),
        Value::Array(lines) => lines
            .iter()
            .map(|l| l.as_str().unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\n"),
        other => other.to_string(),
    }
}

/// Whether two values are equal as a restriction compares them: text without case, numbers and
/// dates by value, `"true"`/`"false"` for booleans.
fn restriction_matches(cell: &V, want: &V) -> bool {
    match (cell, want) {
        (V::Text(a), V::Text(b)) => a.eq_ignore_ascii_case(b) || a.to_lowercase() == b.to_lowercase(),
        (V::Bool(a), V::Text(t)) => t.eq_ignore_ascii_case(if *a { "true" } else { "false" }),
        (V::Num(a), V::Text(t)) => t.trim().parse::<f64>().is_ok_and(|x| x == *a),
        (V::Blank, w) => w.is_blank(),
        (a, b) => compare_vals(a, b).is_ok_and(|o| o == std::cmp::Ordering::Equal),
    }
}

impl Engine<'_, '_> {
    /// The INFO functions of this module; `None` for the others.
    pub(super) fn info_table(
        &mut self,
        up: &str,
        args: &[Option<Expr>],
        s: &Scope,
    ) -> Result<Option<Tab>, Fail> {
        if !up.starts_with("INFO.") {
            return Ok(None);
        }
        if bicraft_dax::is_query_only_info(up) && self.in_model_measure > 0 {
            return dax_error(format!(
                "{up} can only be used in a DAX query, not in a measure of the model"
            ));
        }
        let Some(layout) = layout(up) else {
            return super::unsupported(format!("{up}() is not evaluated by bicraft"));
        };
        if up.starts_with("INFO.VIEW.") && !args.is_empty() {
            return dax_error(format!("{up}() takes no argument"));
        }
        if !args.len().is_multiple_of(2) {
            return dax_error(format!(
                "{up}(): restrictions come in pairs (a column name and a value)"
            ));
        }
        let mut restrictions: Vec<(usize, V)> = Vec::new();
        for pair in args.chunks(2) {
            let name = match pair.first().and_then(Option::as_ref) {
                Some(e) => text_of(&self.scalar(e, s)?),
                None => return dax_error(format!("{up}(): a restriction without a name")),
            };
            let Some(pos) = layout.iter().position(|(n, _)| n.eq_ignore_ascii_case(&name)) else {
                return dax_error(format!("{up}(): `{name}` is not a restriction of this function"));
            };
            let value = match arg(pair, 1) {
                Some(e) => self.scalar(e, s)?,
                None => V::Blank,
            };
            restrictions.push((pos, value));
        }
        let session: &crate::Session = self.ctx.s;
        let times = Times {
            backup: crate::refresh_info::load(session),
            own: self.ctx.own().map(|o| {
                let at = parse_date_text(o.snapshot.refreshed_at.trim_end_matches('Z'));
                (at, o.snapshot.tables.iter().map(|t| t.name.clone()).collect())
            }),
        };
        let model = session.model.as_ref().unwrap_or(self.model);
        let mut rows = rows_for(up, model, &times);
        rows.truncate(MAX_ROWS);
        rows.retain(|r| {
            restrictions
                .iter()
                .all(|(pos, want)| r.get(*pos).is_some_and(|cell| restriction_matches(cell, want)))
        });
        let cols = layout
            .iter()
            .map(|(n, t)| TCol::named(*n, Some(data_type(*t))))
            .collect();
        Ok(Some(Tab::tuples(cols, rows)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_matches_its_layout() {
        let mut m = SemanticModel {
            name: "M".into(),
            ..Default::default()
        };
        let mut t = Table {
            name: "Sales".into(),
            ..Default::default()
        };
        t.columns.push(Column {
            name: "Amount".into(),
            data_type: DataType::Decimal,
            sort_by_column: Some("Amount".into()),
            ..Default::default()
        });
        t.measures.push(bicraft_model::Measure {
            name: "Total".into(),
            expression: "SUM(Sales[Amount])".into(),
            ..Default::default()
        });
        t.partitions.push(bicraft_model::Partition {
            name: "p".into(),
            source_type: "m".into(),
            ..Default::default()
        });
        m.tables.push(t);
        m.relationships.push(bicraft_model::Relationship {
            name: "r".into(),
            from_table: "Sales".into(),
            from_column: "Amount".into(),
            to_table: "Sales".into(),
            to_column: "Amount".into(),
            ..Default::default()
        });
        let times = Times {
            backup: None,
            own: None,
        };
        for up in [
            "INFO.TABLES",
            "INFO.PARTITIONS",
            "INFO.MODEL",
            "INFO.COLUMNS",
            "INFO.MEASURES",
            "INFO.RELATIONSHIPS",
            "INFO.VIEW.TABLES",
            "INFO.VIEW.COLUMNS",
            "INFO.VIEW.MEASURES",
            "INFO.VIEW.RELATIONSHIPS",
        ] {
            let layout = layout(up).unwrap();
            let rows = rows_for(up, &m, &times);
            assert_eq!(rows.len(), 1, "{up}");
            for r in &rows {
                assert_eq!(r.len(), layout.len(), "{up}");
                for (v, (name, t)) in r.iter().zip(layout) {
                    let ok = matches!(
                        (v, t),
                        (V::Blank, _)
                            | (V::Num(_), T::Int)
                            | (V::Text(_), T::Str)
                            | (V::Bool(_), T::Bool)
                            | (V::Date(_), T::Time)
                    );
                    assert!(ok, "{up}[{name}]: {v:?}");
                }
            }
        }
        assert!(layout("INFO.CALCDEPENDENCY").is_none());
    }

    #[test]
    fn restrictions_compare_like_text() {
        assert!(restriction_matches(
            &V::Text("Sales".into()),
            &V::Text("sales".into())
        ));
        assert!(restriction_matches(&V::Num(3.0), &V::Text("3".into())));
        assert!(restriction_matches(&V::Num(3.0), &V::Num(3.0)));
        assert!(restriction_matches(&V::Bool(true), &V::Text("TRUE".into())));
        assert!(!restriction_matches(&V::Text("a".into()), &V::Text("b".into())));
        assert!(restriction_matches(&V::Blank, &V::Blank));
    }
}
