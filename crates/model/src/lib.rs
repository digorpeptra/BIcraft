//! L1: the format-neutral semantic model. Every reader converts into these types and every writer
//! converts out of them; no format crate talks to another.
//!
//! **Lossless by design.** Properties bicraft understands have typed fields. Everything else is kept
//! in each object's `extra` map, keyed by its TMSL property name (TMDL uses the same names), in its
//! original order, so a read → write round trip preserves what we don't model. Child collections
//! we don't model (hierarchies, cultures, perspectives, …) live in `extra` as JSON arrays under
//! their TMSL collection name.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

use serde::{Deserialize, Serialize};

/// Unmodelled properties, keyed by TMSL name, in source order.
pub type Properties = serde_json::Map<String, serde_json::Value>;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SemanticModel {
    /// Database name.
    pub name: String,
    /// Analysis Services compatibility level (e.g. 1567).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compatibility_level: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub culture: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub tables: Vec<Table>,
    #[serde(default)]
    pub relationships: Vec<Relationship>,
    /// Shared expressions (Power Query queries and parameters), `expressions` in TMSL.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expressions: Vec<NamedExpression>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<Role>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    /// Unmodelled properties of the database object (TMSL root / TMDL `database`).
    #[serde(default, skip_serializing_if = "Properties::is_empty")]
    pub database_extra: Properties,
    /// Unmodelled properties and collections of the model object (TMSL `model` / TMDL `model`).
    #[serde(default, skip_serializing_if = "Properties::is_empty")]
    pub extra: Properties,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Table {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage_tag: Option<String>,
    #[serde(default)]
    pub is_hidden: bool,
    #[serde(default)]
    pub columns: Vec<Column>,
    #[serde(default)]
    pub measures: Vec<Measure>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub partitions: Vec<Partition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    /// Unmodelled properties and collections (e.g. `hierarchies`, `calculationGroup`).
    #[serde(default, skip_serializing_if = "Properties::is_empty")]
    pub extra: Properties,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Column {
    pub name: String,
    #[serde(default)]
    pub data_type: DataType,
    /// DAX expression for calculated columns; `None` for data columns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    /// Source column name for data columns (`sourceColumn`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_column: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format_string: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_folder: Option<String>,
    /// `summarizeBy` (e.g. `sum`, `none`), kept as written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summarize_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort_by_column: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage_tag: Option<String>,
    #[serde(default)]
    pub is_hidden: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    #[serde(default, skip_serializing_if = "Properties::is_empty")]
    pub extra: Properties,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Measure {
    pub name: String,
    pub expression: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format_string: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_folder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage_tag: Option<String>,
    #[serde(default)]
    pub is_hidden: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    #[serde(default, skip_serializing_if = "Properties::is_empty")]
    pub extra: Properties,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Relationship {
    /// Relationship names are usually GUIDs; they must be unique in the model.
    #[serde(default)]
    pub name: String,
    pub from_table: String,
    pub from_column: String,
    pub to_table: String,
    pub to_column: String,
    #[serde(default = "yes")]
    pub is_active: bool,
    /// `crossFilteringBehavior` (`oneDirection`, `bothDirections`, `automatic`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cross_filtering_behavior: Option<String>,
    /// `fromCardinality` / `toCardinality` (`one`, `many`, `none`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_cardinality: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_cardinality: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    #[serde(default, skip_serializing_if = "Properties::is_empty")]
    pub extra: Properties,
}

/// A table partition: where the table's rows come from.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Partition {
    pub name: String,
    /// `mode` (`import`, `directQuery`, `dual`, …), kept as written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// `source.type`: `m` (Power Query), `calculated` (DAX), `query`, `entity`, …
    pub source_type: String,
    /// `source.expression`: M or DAX text depending on `source_type`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    /// Unmodelled properties of the partition itself.
    #[serde(default, skip_serializing_if = "Properties::is_empty")]
    pub extra: Properties,
    /// Unmodelled properties of its `source` object.
    #[serde(default, skip_serializing_if = "Properties::is_empty")]
    pub source_extra: Properties,
}

/// A shared expression (`expressions` collection): a named M query or parameter.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NamedExpression {
    pub name: String,
    /// `kind`, normally `m`.
    pub kind: String,
    pub expression: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage_tag: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    #[serde(default, skip_serializing_if = "Properties::is_empty")]
    pub extra: Properties,
}

/// A security role. Its row filters are DAX and are validated like measures.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Role {
    pub name: String,
    /// `modelPermission` (`read`, `readRefresh`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_permission: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub table_permissions: Vec<TablePermission>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    #[serde(default, skip_serializing_if = "Properties::is_empty")]
    pub extra: Properties,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TablePermission {
    /// Table name (`name` in TMSL).
    pub table: String,
    /// Row-level security DAX filter (`filterExpression`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter_expression: Option<String>,
    #[serde(default, skip_serializing_if = "Properties::is_empty")]
    pub extra: Properties,
}

/// `annotations` entries: free-form name/value pairs tools attach to objects.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Annotation {
    pub name: String,
    pub value: String,
}

fn yes() -> bool {
    true
}

/// Column data types, named as TMSL/TOM name them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DataType {
    #[default]
    Unknown,
    String,
    Int64,
    Double,
    Decimal,
    DateTime,
    Boolean,
    Binary,
    Variant,
}

impl DataType {
    /// Parses a TMSL `dataType` value, case-insensitively; unknown names map to `Unknown`.
    pub fn from_tmsl(name: &str) -> Self {
        match name.to_ascii_lowercase().as_str() {
            "string" => Self::String,
            "int64" => Self::Int64,
            "double" => Self::Double,
            "decimal" => Self::Decimal,
            "datetime" => Self::DateTime,
            "boolean" => Self::Boolean,
            "binary" => Self::Binary,
            "variant" => Self::Variant,
            _ => Self::Unknown,
        }
    }

    /// The TMSL/TMDL spelling; `None` for `Unknown` (writers then omit `dataType`).
    pub fn as_tmsl(self) -> Option<&'static str> {
        match self {
            Self::Unknown => None,
            Self::String => Some("string"),
            Self::Int64 => Some("int64"),
            Self::Double => Some("double"),
            Self::Decimal => Some("decimal"),
            Self::DateTime => Some("dateTime"),
            Self::Boolean => Some("boolean"),
            Self::Binary => Some("binary"),
            Self::Variant => Some("variant"),
        }
    }
}

impl SemanticModel {
    /// Finds a table by name. DAX and the engine compare object names case-insensitively.
    pub fn table(&self, name: &str) -> Option<&Table> {
        self.tables.iter().find(|t| t.name.eq_ignore_ascii_case(name))
    }

    pub fn table_mut(&mut self, name: &str) -> Option<&mut Table> {
        self.tables.iter_mut().find(|t| t.name.eq_ignore_ascii_case(name))
    }

    /// Finds a measure anywhere in the model (measure names are unique per model).
    pub fn measure(&self, name: &str) -> Option<(&Table, &Measure)> {
        self.tables.iter().find_map(|t| {
            t.measures
                .iter()
                .find(|m| m.name.eq_ignore_ascii_case(name))
                .map(|m| (t, m))
        })
    }
}

/// One entry of a column's TMSL `variations` collection (kept in [`Column::extra`]).
///
/// Power BI Desktop's auto date/time gives every date column a variation that points, through a
/// relationship, to a hidden `LocalDateTable_<guid>`; DAX then reads `'T'[Date column].[Year]` as
/// that table's `Year` column. Only the properties name resolution needs are typed here; the
/// collection itself stays in `extra`, untouched.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Variation {
    pub name: String,
    /// Name of the relationship from the column to the variation's table.
    pub relationship: Option<String>,
    /// `defaultHierarchy`: (table, hierarchy).
    pub default_hierarchy: Option<(String, String)>,
    /// `defaultColumn`: (table, column).
    pub default_column: Option<(String, String)>,
    pub is_default: bool,
}

impl Variation {
    /// The TMSL object of this variation (the shape TMSL `variations` entries and TMDL
    /// `variation` blocks read into).
    pub fn to_tmsl(&self) -> serde_json::Value {
        let mut o = Properties::new();
        o.insert("name".into(), self.name.clone().into());
        if let Some(r) = &self.relationship {
            o.insert("relationship".into(), r.clone().into());
        }
        if let Some((t, h)) = &self.default_hierarchy {
            o.insert(
                "defaultHierarchy".into(),
                serde_json::json!({ "table": t, "hierarchy": h }),
            );
        }
        if let Some((t, c)) = &self.default_column {
            o.insert(
                "defaultColumn".into(),
                serde_json::json!({ "table": t, "column": c }),
            );
        }
        if self.is_default {
            o.insert("isDefault".into(), true.into());
        }
        serde_json::Value::Object(o)
    }

    fn from_tmsl(v: &serde_json::Value) -> Option<Variation> {
        let text = |o: &serde_json::Value, k: &str| o.get(k).and_then(|x| x.as_str()).map(str::to_string);
        let pair = |k: &str, second: &str| {
            let o = v.get(k)?;
            Some((text(o, "table")?, text(o, second)?))
        };
        Some(Variation {
            name: text(v, "name")?,
            relationship: text(v, "relationship"),
            default_hierarchy: pair("defaultHierarchy", "hierarchy"),
            default_column: pair("defaultColumn", "column"),
            is_default: v.get("isDefault").and_then(serde_json::Value::as_bool) == Some(true),
        })
    }
}

impl Column {
    /// Engine-internal `RowNumber-…` column (TMSL `"type": "rowNumber"`, kept in `extra` for
    /// round-tripping). Never visible to DAX or users.
    pub fn is_row_number(&self) -> bool {
        self.extra.get("type").and_then(serde_json::Value::as_str) == Some("rowNumber")
    }

    /// The column's variations (TMSL `variations`, in `extra`), the default one first. Entries
    /// without a name are skipped.
    pub fn variations(&self) -> Vec<Variation> {
        let mut out: Vec<Variation> = self
            .extra
            .get("variations")
            .and_then(serde_json::Value::as_array)
            .map(|a| a.iter().filter_map(Variation::from_tmsl).collect())
            .unwrap_or_default();
        // Stable: the default variation first, the others in model order.
        out.sort_by_key(|v| !v.is_default);
        out
    }

    /// Replaces the column's `variations` collection (removes it when `variations` is empty).
    pub fn set_variations(&mut self, variations: &[Variation]) {
        if variations.is_empty() {
            self.extra.remove("variations");
        } else {
            let list = variations.iter().map(Variation::to_tmsl).collect();
            self.extra
                .insert("variations".into(), serde_json::Value::Array(list));
        }
    }
}

impl SemanticModel {
    /// The relationships `self` renamed from `before` (`relationship.set {new_name}`), as
    /// (old name, new name): same position, same ends, the old name gone and the new one new.
    /// Savers keep such a relationship (and its storage) under its new name.
    pub fn relationship_renames(&self, before: &SemanticModel) -> Vec<(String, String)> {
        before
            .relationships
            .iter()
            .zip(&self.relationships)
            .filter(|(o, r)| {
                o.name != r.name
                    && o.from_table == r.from_table
                    && o.from_column == r.from_column
                    && o.to_table == r.to_table
                    && o.to_column == r.to_column
                    && !self.relationships.iter().any(|x| x.name == o.name)
                    && !before.relationships.iter().any(|x| x.name == r.name)
            })
            .map(|(o, r)| (o.name.clone(), r.name.clone()))
            .collect()
    }

    /// The table a variation of `table[column]` points to: the other end of its relationship,
    /// else the table of its default hierarchy or default column.
    pub fn variation_table(&self, table: &str, column: &str, v: &Variation) -> Option<&Table> {
        let by_relationship = v.relationship.as_deref().and_then(|name| {
            let r = self
                .relationships
                .iter()
                .find(|r| r.name.eq_ignore_ascii_case(name))?;
            let is = |t: &str, c: &str| t.eq_ignore_ascii_case(table) && c.eq_ignore_ascii_case(column);
            if is(&r.from_table, &r.from_column) {
                Some(r.to_table.as_str())
            } else if is(&r.to_table, &r.to_column) {
                Some(r.from_table.as_str())
            } else {
                None
            }
        });
        let name = by_relationship
            .or(v.default_hierarchy.as_ref().map(|(t, _)| t.as_str()))
            .or(v.default_column.as_ref().map(|(t, _)| t.as_str()))?;
        self.table(name)
    }

    /// Resolves the column variation `table[column].[name]`: the column `name` of the table one of
    /// the column's variations points to (the default variation first). The engine matches the
    /// name against that table's columns, not against its hierarchy levels.
    pub fn variation_column(&self, table: &str, column: &str, name: &str) -> Option<(&Table, &Column)> {
        let col = self.table(table)?.column(column)?;
        col.variations().iter().find_map(|v| {
            let t = self.variation_table(table, column, v)?;
            t.column(name).map(|c| (t, c))
        })
    }
}

impl Table {
    /// Finds a user-visible column (engine-internal row-number columns are skipped).
    pub fn column(&self, name: &str) -> Option<&Column> {
        self.columns
            .iter()
            .find(|c| !c.is_row_number() && c.name.eq_ignore_ascii_case(name))
    }

    pub fn column_mut(&mut self, name: &str) -> Option<&mut Column> {
        self.columns
            .iter_mut()
            .find(|c| c.name.eq_ignore_ascii_case(name))
    }

    pub fn measure(&self, name: &str) -> Option<&Measure> {
        self.measures.iter().find(|m| m.name.eq_ignore_ascii_case(name))
    }

    pub fn measure_mut(&mut self, name: &str) -> Option<&mut Measure> {
        self.measures
            .iter_mut()
            .find(|m| m.name.eq_ignore_ascii_case(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookups_are_case_insensitive() {
        let mut model = SemanticModel {
            tables: vec![Table {
                name: "Sales".into(),
                columns: vec![Column {
                    name: "Amount".into(),
                    data_type: DataType::Double,
                    ..Default::default()
                }],
                measures: vec![Measure {
                    name: "Total".into(),
                    expression: "SUM(Sales[Amount])".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(model.table("sales").and_then(|t| t.column("AMOUNT")).is_some());
        assert_eq!(
            model.measure("total").map(|(t, _)| t.name.as_str()),
            Some("Sales")
        );
        if let Some(m) = model.table_mut("SALES").and_then(|t| t.measure_mut("TOTAL")) {
            m.format_string = Some("0.0".into());
        }
        assert_eq!(
            model
                .measure("Total")
                .and_then(|(_, m)| m.format_string.as_deref()),
            Some("0.0")
        );
    }

    #[test]
    fn serde_round_trip_keeps_extra_properties_in_order() {
        let mut extra = Properties::new();
        extra.insert("zeta".into(), serde_json::json!(1));
        extra.insert("alpha".into(), serde_json::json!({"nested": [1, 2]}));
        let model = SemanticModel {
            name: "M".into(),
            tables: vec![Table {
                name: "T".into(),
                extra,
                ..Default::default()
            }],
            ..Default::default()
        };
        let json = serde_json::to_string(&model).unwrap();
        let back = serde_json::from_str::<SemanticModel>(&json).unwrap();
        assert_eq!(back, model);
        let keys: Vec<_> = back.tables[0].extra.keys().collect();
        assert_eq!(keys, ["zeta", "alpha"]);
    }

    #[test]
    fn data_type_names_round_trip() {
        for t in [
            DataType::String,
            DataType::Int64,
            DataType::DateTime,
            DataType::Boolean,
        ] {
            assert_eq!(t.as_tmsl().map(DataType::from_tmsl), Some(t));
        }
        assert_eq!(DataType::Unknown.as_tmsl(), None);
    }

    fn variation_model() -> SemanticModel {
        let mut date = Column {
            name: "OrderDate".into(),
            data_type: DataType::DateTime,
            ..Default::default()
        };
        date.set_variations(&[
            Variation {
                name: "Other".into(),
                default_column: Some(("Elsewhere".into(), "X".into())),
                ..Default::default()
            },
            Variation {
                name: "Variation".into(),
                relationship: Some("r1".into()),
                default_hierarchy: Some(("LocalDateTable_1".into(), "Date Hierarchy".into())),
                is_default: true,
                ..Default::default()
            },
        ]);
        let col = |name: &str, data_type| Column {
            name: name.into(),
            data_type,
            ..Default::default()
        };
        SemanticModel {
            tables: vec![
                Table {
                    name: "Sales".into(),
                    columns: vec![date, col("Amount", DataType::Decimal)],
                    ..Default::default()
                },
                Table {
                    name: "LocalDateTable_1".into(),
                    columns: vec![col("Date", DataType::DateTime), col("Year", DataType::Int64)],
                    ..Default::default()
                },
            ],
            relationships: vec![Relationship {
                name: "r1".into(),
                from_table: "Sales".into(),
                from_column: "OrderDate".into(),
                to_table: "LocalDateTable_1".into(),
                to_column: "Date".into(),
                is_active: true,
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn variations_read_write_and_resolve() {
        let m = variation_model();
        let c = m.table("Sales").and_then(|t| t.column("OrderDate")).unwrap();
        let vs = c.variations();
        assert_eq!(vs.len(), 2);
        assert_eq!(vs[0].name, "Variation", "the default variation comes first");
        assert_eq!(vs[0].relationship.as_deref(), Some("r1"));
        assert_eq!(
            c.extra["variations"][1],
            serde_json::json!({
                "name": "Variation",
                "relationship": "r1",
                "defaultHierarchy": {"table": "LocalDateTable_1", "hierarchy": "Date Hierarchy"},
                "isDefault": true
            })
        );
        let (t, col) = m.variation_column("sales", "ORDERDATE", "year").unwrap();
        assert_eq!((t.name.as_str(), col.name.as_str()), ("LocalDateTable_1", "Year"));
        assert!(m.variation_column("Sales", "OrderDate", "Month").is_none());
        assert!(m.variation_column("Sales", "Amount", "Year").is_none());
        assert!(m.variation_column("Nope", "OrderDate", "Year").is_none());

        // Without the relationship, the default hierarchy's table still answers.
        let mut m2 = m.clone();
        m2.relationships.clear();
        assert!(m2.variation_column("Sales", "OrderDate", "Date").is_some());

        let mut c2 = c.clone();
        c2.set_variations(&[]);
        assert!(c2.extra.get("variations").is_none());
        assert!(c2.variations().is_empty());
    }

    #[test]
    fn malformed_variations_are_skipped() {
        let mut c = Column::default();
        c.extra.insert(
            "variations".into(),
            serde_json::json!([1, {"relationship": "r"}, {"name": "V", "defaultHierarchy": "flat"}]),
        );
        let vs = c.variations();
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].default_hierarchy, None);
        c.extra.insert("variations".into(), serde_json::json!("x"));
        assert!(c.variations().is_empty());
    }
}
