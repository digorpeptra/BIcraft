//! L3: the legacy single-document report layout (PBIX `Report/Layout`, PBIP `report.json`) and
//! its conversion to and from the PBIR-shaped [`bicraft_report::Report`].
//!
//! Every fact used here comes from public schemas, open-source code read for facts, or black-box
//! observation of Power BI Desktop's own legacy → PBIR conversions. The format note is
//! `docs/formats/legacy-layout-bb.md`, its source log `docs/formats/legacy-layout-bb.sources.md`.
//!
//! - [`read_layout`] turns the legacy document into a report whose objects carry their PBIR
//!   property names. Nothing is dropped silently: every legacy property without a PBIR form is
//!   kept under [`UNMAPPED_KEY`] in the `extra` map of its report, page or visual, keyed by its
//!   path in the legacy document, and listed by [`read_layout_with_summary`].
//! - [`write_layout`] writes an edited report back into the legacy document it was read from.
//!   Sections and visual containers whose content did not change keep their original text; an
//!   edit the legacy form cannot hold is refused with every such edit named.
//! - [`strip_legacy`] and [`report_extensions`] help the engine turn a legacy report into pure
//!   PBIR the way Desktop does when it upgrades a report.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable,
    clippy::indexing_slicing
)]

mod codes;
mod convert;
mod path;
mod query;
mod read;
mod write;

use serde::Serialize;

pub use convert::{report_extensions, strip_legacy};

/// The legacy layout's numeric code of a report measure's PBIR `dataType` (`Double` → 3), `None`
/// for a type without a known code.
pub fn measure_data_type_code(name: &str) -> Option<i64> {
    codes::code(codes::DATA_TYPE, name)
}

/// The PBIR `dataType` of a legacy report measure's numeric code, `None` for an unknown code.
pub fn measure_data_type_name(code: &serde_json::Value) -> Option<&'static str> {
    codes::name(codes::DATA_TYPE, code)
}

/// Every key this crate adds to an `extra` map starts with this prefix. Such keys are never
/// PBIR properties: [`strip_legacy`] removes them.
pub const LEGACY_PREFIX: &str = "legacy:";

/// The legacy properties without a PBIR form, as a JSON object keyed by their path in the legacy
/// document (relative to the report root, the section or the visual container, e.g.
/// `config.modelExtensions`, `layoutOptimization`, `config.singleVisual.cachedFilterDisplayItems`).
/// Present in the report's `extra` map of every report read from a legacy layout (possibly
/// empty), and in a page's or visual's `extra` map when it has such properties.
pub const UNMAPPED_KEY: &str = "legacy:unmapped";

/// A visual's phone layout (the container layout with id 1 and its phone formatting), in the shape
/// of a PBIR `mobile.json` without `$schema`: `{"position": …, "objects": …,
/// "visualContainerObjects": …}`.
pub const MOBILE_STATE_KEY: &str = "legacy:mobileState";

/// The bookmark list (order and groups) in the shape of a PBIR `bookmarks.json` without `$schema`:
/// `{"items": [{"name": …}, {"name": …, "displayName": …, "children": […]}]}`.
pub const BOOKMARKS_METADATA_KEY: &str = "legacy:bookmarks";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not valid JSON: {0}")]
    Json(String),
    #[error("the layout is not a JSON object")]
    NotObject,
    #[error("these report edits have no legacy layout form: {}", .0.join("; "))]
    Unwritable(Vec<String>),
}

/// What [`read_layout_with_summary`] found.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Summary {
    pub pages: usize,
    pub visuals: usize,
    pub bookmarks: usize,
    /// Every legacy property kept without a PBIR form, as `<where>: <legacy path>` (`report`,
    /// `page <name>`, `visual <page>/<visual>`).
    pub unmapped: Vec<String>,
}

/// Reads a legacy layout (`Report/Layout` text or a PBIP `report.json`).
pub fn read_layout(text: &str) -> Result<bicraft_report::Report, Error> {
    read::read(text).map(|r| r.report)
}

/// [`read_layout`] with a [`Summary`] of what was read and kept.
pub fn read_layout_with_summary(text: &str) -> Result<(bicraft_report::Report, Summary), Error> {
    let read = read::read(text)?;
    let summary = read::summary(&read.report);
    Ok((read.report, summary))
}

/// Writes `report` (read from `original` by [`read_layout`], then edited) into the legacy layout
/// `original`. Returns `original` itself when the report did not change.
pub fn write_layout(original: &str, report: &bicraft_report::Report) -> Result<String, Error> {
    write::write(original, report)
}
