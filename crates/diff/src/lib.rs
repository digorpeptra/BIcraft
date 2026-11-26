//! L3: the semantic diff of two Power BI files, compared through the format-neutral model and
//! report (`docs/diff.md`). The engine loads each side from any format (PBIX, PBIT, PBIP, model.bim),
//! metadata only, and hands both to [`diff`]; the CLI and the MCP server render the result.
//!
//! - **Identity.** Objects are matched by name first, then by lineage tag, then by a
//!   case-insensitive name, then by content (same expression, same endpoints, same fields): a
//!   match that is not by name is a rename (or, for a measure in another table, a move).
//! - **Noise.** Formats differ in things that carry no meaning: `$schema` URLs, `version.json`,
//!   generated folder names, array order, timestamps, the Desktop version annotation, CRLF. They
//!   are never reported. Whitespace and comment changes in DAX and M are reported as cosmetic
//!   (and dropped with [`Options::ignore_whitespace`]).
//! - **Bounded.** Every collection, generic JSON comparison and text diff has a cap; hostile
//!   inputs make the diff shorter, never slower than linear in their size or deeper than
//!   [`json::MAX_DEPTH`].
//!
//! The output ([`Diff`]) is deterministic: changes are sorted by area, section, object and id,
//! with byte comparisons only, so the JSON, text ([`render::text`]) and Markdown
//! ([`render::markdown`]) are byte-identical on every platform.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

use bicraft_model::SemanticModel;
use bicraft_report::Report;
use serde::Serialize;
use serde_json::Value;

pub mod dump;
pub mod json;
pub mod lines;
mod model;
pub mod norm;
mod pair;
pub mod render;
mod report;

#[cfg(test)]
mod tests;

/// What to compare and what to leave out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Options {
    pub scope: Scope,
    /// Leave out visual, page and report formatting (`objects`, `visualContainerObjects`) and
    /// theme changes.
    pub ignore_formatting: bool,
    /// Leave out visual positions, sizes, z-order and tab order, and page sizes.
    pub ignore_positions: bool,
    /// Leave out DAX and M changes that only touch whitespace, comments or keyword case.
    pub ignore_whitespace: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Scope {
    #[default]
    All,
    Model,
    Report,
}

impl Scope {
    pub fn parse(s: &str) -> Option<Scope> {
        match s {
            "all" => Some(Scope::All),
            "model" => Some(Scope::Model),
            "report" => Some(Scope::Report),
            _ => None,
        }
    }
    fn model(self) -> bool {
        self != Scope::Report
    }
    fn report(self) -> bool {
        self != Scope::Model
    }
}

/// The size and CRC-32 of a PBIX `DataModel` part, read from the ZIP directory (no
/// decompression): equal stamps mean the same data snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct DataStamp {
    pub size: u64,
    pub crc32: u32,
}

/// One side of a diff. `None` model or report: the side has none (a file added or deleted in a
/// git history, a model.bim without report); everything on the other side is added or removed.
#[derive(Debug, Clone, Copy, Default)]
pub struct Input<'a> {
    pub model: Option<&'a SemanticModel>,
    pub report: Option<&'a Report>,
    pub data: Option<DataStamp>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Area {
    Model,
    Report,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Added,
    Removed,
    Changed,
    Renamed,
    Moved,
}

/// How a pair was matched when not by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum How {
    Name,
    LineageTag,
    Case,
    Content,
}

/// One changed property: `before` absent means it was added, `after` absent that it was removed.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Prop {
    pub property: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<Value>,
    /// Only whitespace, comments or keyword case changed (DAX, M).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub cosmetic: bool,
}

impl Prop {
    pub fn new(property: impl Into<String>, before: Option<Value>, after: Option<Value>) -> Prop {
        Prop {
            property: property.into(),
            before,
            after,
            cosmetic: false,
        }
    }
}

/// The fields bound to one visual role, compared.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FieldDelta {
    pub role: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub added: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<String>,
    /// Same fields in another order.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub reordered: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Change {
    pub area: Area,
    /// `table`, `column`, `measure`, `relationship`, `role`, `hierarchy`, `calculation_item`,
    /// `partition`, `expression`, `parameter`, `perspective`, `culture`, `data_source`, `model`,
    /// `data`, `report`, `theme`, `page`, `visual`, `filter`, `bookmark`, `resource`.
    pub object: &'static str,
    /// The object as it is in `b` (in `a` when removed), e.g. `measure 'Sales'[Total]`.
    pub id: String,
    pub change: Kind,
    /// The object's id in `a`, for renames and moves.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// How a renamed or moved object was recognised.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub how: Option<How>,
    /// A readable description (visuals: type and first fields).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub properties: Vec<Prop>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<FieldDelta>,
    /// Formatting properties (`card.property`), summarised.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub formatting: Vec<Prop>,
    /// The removed object, summarised.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<Value>,
    /// The added object, summarised.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<Value>,
    #[serde(skip)]
    pub(crate) key: SortKey,
}

/// Sort order: area, section (tables, relationships, …), section name (the table), object rank
/// inside the section, id.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SortKey {
    pub section: u8,
    pub group: String,
    pub rank: u8,
}

impl Change {
    pub(crate) fn new(area: Area, object: &'static str, id: String, change: Kind) -> Change {
        Change {
            area,
            object,
            id,
            change,
            from: None,
            how: None,
            label: None,
            properties: Vec::new(),
            fields: Vec::new(),
            formatting: Vec::new(),
            before: None,
            after: None,
            key: SortKey::default(),
        }
    }

    pub(crate) fn sorted(mut self, section: u8, group: &str, rank: u8) -> Change {
        self.key = SortKey {
            section,
            group: group.to_string(),
            rank,
        };
        self
    }

    /// Whether a matched pair has anything to report.
    pub(crate) fn has_content(&self) -> bool {
        !self.properties.is_empty() || !self.fields.is_empty() || !self.formatting.is_empty()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Summary {
    pub added: usize,
    pub removed: usize,
    pub changed: usize,
    pub renamed: usize,
    pub moved: usize,
}

/// The data snapshots of two PBIX files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct DataCompare {
    pub a: DataStamp,
    pub b: DataStamp,
    pub same: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Diff {
    pub identical: bool,
    pub summary: Summary,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<DataCompare>,
    pub changes: Vec<Change>,
    /// What was not compared, and why.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// Most changes reported; the rest are counted in a note.
pub const MAX_CHANGES: usize = 20_000;

/// Compares `a` (before) with `b` (after).
pub fn diff(a: &Input<'_>, b: &Input<'_>, opts: &Options) -> Diff {
    let mut changes = Vec::new();
    let mut notes = Vec::new();
    let empty_model = SemanticModel::default();
    let empty_report = Report::default();
    if opts.scope.model() {
        let ma = a.model.unwrap_or(&empty_model);
        let mb = b.model.unwrap_or(&empty_model);
        model::compare(ma, mb, opts, &mut changes);
    }
    let mut data = None;
    if opts.scope.model()
        && let (Some(da), Some(db)) = (a.data, b.data)
    {
        let same = da == db;
        data = Some(DataCompare { a: da, b: db, same });
        // the DataModel holds the metadata too: a changed snapshot only says something new when
        // the metadata is the same (a refresh)
        if !same && !changes.iter().any(|c| c.area == Area::Model) {
            let mut c = Change::new(Area::Model, "data", "data snapshot".into(), Kind::Changed);
            c.properties.push(Prop::new(
                "DataModel",
                Some(serde_json::json!(format!(
                    "{} bytes, CRC-32 {:08x}",
                    da.size, da.crc32
                ))),
                Some(serde_json::json!(format!(
                    "{} bytes, CRC-32 {:08x}",
                    db.size, db.crc32
                ))),
            ));
            changes.push(c.sorted(model::SECTION_DATA, "", 0));
        }
    }
    if opts.scope.report() {
        match (a.report, b.report) {
            (None, None) => {}
            (ra, rb) => report::compare(
                ra.unwrap_or(&empty_report),
                rb.unwrap_or(&empty_report),
                opts,
                &mut changes,
            ),
        }
    }
    changes.sort_by(|x, y| {
        (x.area, &x.key, &x.id, x.change, &x.from).cmp(&(y.area, &y.key, &y.id, y.change, &y.from))
    });
    if changes.len() > MAX_CHANGES {
        notes.push(format!(
            "{} more changes not listed (limit {MAX_CHANGES})",
            changes.len() - MAX_CHANGES
        ));
        changes.truncate(MAX_CHANGES);
    }
    let mut summary = Summary::default();
    for c in &changes {
        match c.change {
            Kind::Added => summary.added += 1,
            Kind::Removed => summary.removed += 1,
            Kind::Changed => summary.changed += 1,
            Kind::Renamed => summary.renamed += 1,
            Kind::Moved => summary.moved += 1,
        }
    }
    Diff {
        identical: changes.is_empty(),
        summary,
        data,
        changes,
        notes,
    }
}

/// `'Name'` with `'` doubled, as DAX quotes table names.
pub(crate) fn quote_table(name: &str) -> String {
    format!("'{}'", name.replace('\'', "''"))
}

/// `[Name]` with `]` doubled.
pub(crate) fn bracket(name: &str) -> String {
    format!("[{}]", name.replace(']', "]]"))
}

/// `"Name"` (JSON string quoting) for pages and bookmarks.
pub(crate) fn dq(name: &str) -> String {
    serde_json::to_string(name).unwrap_or_else(|_| format!("\"{name}\""))
}
