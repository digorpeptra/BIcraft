//! L1: the format-neutral report. It mirrors PBIR (the modern, file-per-object format) because
//! PBIR is the canonical target; the legacy single-document layout (`report.json` /
//! `Report/Layout`) is converted into it.
//!
//! **Lossless by design**, like `bicraft-model`: properties bicraft edits have typed fields;
//! everything else stays in each object's `extra` map, keyed by its PBIR JSON property name, in its
//! original order. A PBIR read → write round trip must reproduce the same JSON.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

use serde::{Deserialize, Serialize};

pub mod query;

/// Unmodelled properties, keyed by PBIR property name, in source order.
pub type Properties = serde_json::Map<String, serde_json::Value>;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Report {
    /// `$schema` URLs and versions are format concerns: the PBIR writer chooses them. Readers keep
    /// what they saw here so a round trip can preserve them.
    #[serde(default, skip_serializing_if = "Properties::is_empty")]
    pub schemas: Properties,
    /// Pages in display order (`pages.json` `pageOrder`).
    #[serde(default)]
    pub pages: Vec<Page>,
    /// `pages.json` `activePageName`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_page: Option<String>,
    /// Theme references (`report.json` `themeCollection`), kept verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme_collection: Option<serde_json::Value>,
    /// Report-level filters (`report.json` `filterConfig`), kept verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter_config: Option<serde_json::Value>,
    /// Bookmarks (`definition/bookmarks/`), as raw documents keyed by bookmark name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bookmarks: Vec<NamedDocument>,
    /// Files under `StaticResources/` (themes, images), path relative to the report root,
    /// e.g. `StaticResources/RegisteredResources/logo.png`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resources: Vec<Resource>,
    /// Unmodelled `report.json` properties (resourcePackages, settings, objects, …).
    #[serde(default, skip_serializing_if = "Properties::is_empty")]
    pub extra: Properties,
    /// Unmodelled `pages.json` properties.
    #[serde(default, skip_serializing_if = "Properties::is_empty")]
    pub pages_extra: Properties,
    /// Unmodelled `version.json` properties.
    #[serde(default, skip_serializing_if = "Properties::is_empty")]
    pub version_extra: Properties,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Page {
    /// Stable identifier, also the folder name (`pages/<name>/`); 20 hex chars when Desktop
    /// creates it.
    pub name: String,
    pub display_name: String,
    /// `displayOption` (`FitToPage`, `FitToWidth`, `ActualSize`), kept as written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_option: Option<String>,
    #[serde(default)]
    pub width: f64,
    #[serde(default)]
    pub height: f64,
    /// `visibility` (e.g. `HiddenInViewMode`), kept as written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    /// Page filters (`filterConfig`), kept verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter_config: Option<serde_json::Value>,
    #[serde(default)]
    pub visuals: Vec<Visual>,
    /// Unmodelled `page.json` properties (objects, pageBinding, type, …).
    #[serde(default, skip_serializing_if = "Properties::is_empty")]
    pub extra: Properties,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Visual {
    /// Stable identifier, also the folder name (`visuals/<name>/`).
    pub name: String,
    #[serde(default)]
    pub position: Position,
    /// `visual.visualType` (e.g. `card`, `clusteredColumnChart`, `tableEx`, `slicer`); `None` for
    /// visual groups.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visual_type: Option<String>,
    /// Data bindings, by projection role (`Values`, `Category`, `Y`, …), in source order. Derived
    /// from `visual.query.queryState`; the raw query stays in `visual_extra` for losslessness.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<FieldBinding>,
    /// Parent group (`parentGroupName`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_group: Option<String>,
    #[serde(default)]
    pub is_hidden: bool,
    /// Visual filters (`filterConfig`), kept verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter_config: Option<serde_json::Value>,
    /// Unmodelled properties of the `visual` object (query, objects, visualContainerObjects, …).
    #[serde(default, skip_serializing_if = "Properties::is_empty")]
    pub visual_extra: Properties,
    /// Unmodelled top-level `visual.json` properties (visualGroup, howCreated, …).
    #[serde(default, skip_serializing_if = "Properties::is_empty")]
    pub extra: Properties,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub z: f64,
    pub width: f64,
    pub height: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_order: Option<i64>,
}

/// One field placed in a visual role.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldBinding {
    /// Projection role, e.g. `Values`, `Category`, `Y`, `Legend`, `Rows`.
    pub role: String,
    pub field: FieldRef,
    /// The name the visual shows for this field ("Rename for this visual"): the projection's
    /// `displayName`; `None` shows the field's own name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// "Show value as" a percentage of a total: the projection's field is then
    /// `Arithmetic(field / ScopedEval(field, scope))` (docs/formats/pbir.md); `None` shows the
    /// value itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_as: Option<ShowAs>,
}

/// What a value field is shown as ("Show value as" in Power BI Desktop): its share of a total.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShowAs {
    /// The value divided by the visual's grand total.
    #[serde(rename = "percent_of_grand_total")]
    GrandTotal,
    /// A matrix value divided by its column's total (the scope keeps the `Columns` fields).
    #[serde(rename = "percent_of_column_total")]
    ColumnTotal,
    /// A matrix value divided by its row's total (the scope keeps the `Rows` fields).
    #[serde(rename = "percent_of_row_total")]
    RowTotal,
}

impl ShowAs {
    /// The parameter and snapshot spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            ShowAs::GrandTotal => "percent_of_grand_total",
            ShowAs::ColumnTotal => "percent_of_column_total",
            ShowAs::RowTotal => "percent_of_row_total",
        }
    }

    /// The prefix Desktop gives the field's name (`%GT Sum of Amount`).
    pub fn prefix(self) -> &'static str {
        match self {
            ShowAs::GrandTotal => "%GT",
            ShowAs::ColumnTotal => "%CT",
            ShowAs::RowTotal => "%RT",
        }
    }

    /// The role whose fields the total keeps (`None`: the grand total keeps none).
    pub fn scope_role(self) -> Option<&'static str> {
        match self {
            ShowAs::GrandTotal => None,
            ShowAs::ColumnTotal => Some("Columns"),
            ShowAs::RowTotal => Some("Rows"),
        }
    }
}

/// A reference to a model object, as visuals bind them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum FieldRef {
    Column {
        table: String,
        column: String,
    },
    Measure {
        table: String,
        measure: String,
    },
    /// Aggregated column (`Sum(Sales[Amount])`); `function` is the PBIR aggregation code name.
    Aggregation {
        table: String,
        column: String,
        function: String,
    },
    /// Anything else (hierarchy levels, expressions…), kept as the raw PBIR field JSON.
    Other {
        raw: serde_json::Value,
    },
}

impl Default for FieldRef {
    fn default() -> Self {
        Self::Other {
            raw: serde_json::Value::Null,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NamedDocument {
    pub name: String,
    pub document: serde_json::Value,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resource {
    pub path: String,
    #[serde(with = "bytes_as_len", default)]
    pub bytes: Vec<u8>,
}

/// Resources can be megabytes of images: serialize their size only (for summaries), never the
/// bytes. Deserialization yields an empty buffer.
mod bytes_as_len {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(bytes.len() as u64)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let _ = u64::deserialize(d)?;
        Ok(Vec::new())
    }
}

impl Report {
    /// Finds a page by name or display name (case-insensitive).
    pub fn page(&self, key: &str) -> Option<&Page> {
        self.pages
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(key) || p.display_name.eq_ignore_ascii_case(key))
    }

    pub fn page_mut(&mut self, key: &str) -> Option<&mut Page> {
        self.pages
            .iter_mut()
            .find(|p| p.name.eq_ignore_ascii_case(key) || p.display_name.eq_ignore_ascii_case(key))
    }

    /// Every model reference used by any visual: what a model rename or removal can break.
    pub fn field_refs(&self) -> impl Iterator<Item = (&Page, &Visual, &FieldRef)> {
        self.pages.iter().flat_map(|p| {
            p.visuals
                .iter()
                .flat_map(move |v| v.fields.iter().map(move |f| (p, v, &f.field)))
        })
    }
}

impl Page {
    pub fn visual(&self, name: &str) -> Option<&Visual> {
        self.visuals.iter().find(|v| v.name == name)
    }

    pub fn visual_mut(&mut self, name: &str) -> Option<&mut Visual> {
        self.visuals.iter_mut().find(|v| v.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Report {
        Report {
            pages: vec![Page {
                name: "abc".into(),
                display_name: "Overview".into(),
                width: 1280.0,
                height: 720.0,
                visuals: vec![Visual {
                    name: "v1".into(),
                    visual_type: Some("card".into()),
                    fields: vec![FieldBinding {
                        display_name: None,
                        show_as: None,
                        role: "Values".into(),
                        field: FieldRef::Measure {
                            table: "Sales".into(),
                            measure: "Total".into(),
                        },
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn lookups() {
        let r = sample();
        assert!(r.page("overview").is_some());
        assert!(r.page("ABC").and_then(|p| p.visual("v1")).is_some());
        assert_eq!(r.field_refs().count(), 1);
    }

    #[test]
    fn serde_round_trip_without_resources() {
        let r = sample();
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(serde_json::from_str::<Report>(&json).unwrap(), r);
    }

    #[test]
    fn resources_serialize_as_sizes() {
        let r = Report {
            resources: vec![Resource {
                path: "StaticResources/x.png".into(),
                bytes: vec![0; 1000],
            }],
            ..Default::default()
        };
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["resources"][0]["bytes"], 1000);
    }
}
