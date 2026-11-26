//! L3: PBIR, the file-per-object report format (`<Name>.Report/definition/…`).
//!
//! - [`read_documents`] / [`read_dir`] turn the PBIR documents into a lossless
//!   [`bicraft_report::Report`];
//! - [`write_documents`] / [`write_dir`] serialize it back the way Power BI Desktop lays it out;
//! - [`validate_documents`] checks every document against the JSON schema version it declares
//!   (Microsoft's public schemas, vendored under `schemas/`) and the cross-file rules.
//!
//! The conventions (key order, versions, where unmodelled documents go) are described in
//! `docs/formats/pbir.md`.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

use std::path::Path;

use bicraft_report::Report;

mod fields;
mod fs;
mod json;
mod paths;
mod read;
mod regex;
mod schema;
mod schema_files;
#[cfg(test)]
#[path = "../../../testsupport/tempdir.rs"]
mod tempdir;
mod validate;
mod write;

/// The JSON Schema pattern matcher, shared with the best-practice analyzer (`RegEx.IsMatch`).
pub use regex::Regex;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Unsupported(String),
    /// A document is not valid JSON, not UTF-8 without BOM, or has an unexpected shape.
    #[error("{path}: {message}")]
    Invalid { path: String, message: String },
    /// A path or an object name cannot be used as a file or folder name.
    #[error("{path}: {message}")]
    InvalidPath { path: String, message: String },
    #[error("{path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// A schema or consistency problem found by [`validate_documents`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Issue {
    /// Document path relative to the report root, e.g. `definition/pages/x/page.json`.
    pub path: String,
    /// JSON pointer inside the document (`""` for the whole document).
    pub pointer: String,
    pub message: String,
}

/// Reads a report from documents keyed by path relative to the report root
/// (`definition/report.json`, `definition/pages/<p>/page.json`, `StaticResources/…`). Works for a
/// PBIP `<Name>.Report/` folder and for the `Report/` parts of a PBIX (prefix stripped).
///
/// Files that are not part of the report (`definition.pbir`, `.platform`, `.pbi/…`) are ignored.
/// Documents the typed model does not own (`reportExtensions.json`, visual `mobile.json`,
/// `bookmarks/bookmarks.json`, unknown files under `definition/`, `CustomVisuals/…`) are kept
/// verbatim in [`Report::resources`] under their path.
pub fn read_documents(docs: &[(String, Vec<u8>)]) -> Result<Report, Error> {
    read::read(docs)
}

/// Serializes a report as PBIR documents (paths relative to the report root), UTF-8 without BOM,
/// 2-space indentation, LF, no trailing newline. Objects whose names cannot be used as folder or
/// file names are skipped; [`check_names`] reports them.
pub fn write_documents(report: &Report) -> Vec<(String, Vec<u8>)> {
    write::write(report)
}

/// Reads `<Name>.Report/` (its `definition/`, `StaticResources/` and `CustomVisuals/`).
pub fn read_dir(report_dir: &Path) -> Result<Report, Error> {
    let docs = fs::collect(report_dir)?;
    read_documents(&docs)
}

/// Writes `definition/` and `StaticResources/` under `<Name>.Report/`, removing page, visual and
/// bookmark files that no longer exist. Never touches `definition.pbir`, `.platform` or `.pbi/`.
pub fn write_dir(report: &Report, report_dir: &Path) -> Result<(), Error> {
    check_names(report)?;
    fs::write(report_dir, &write_documents(report))
}

/// Validates PBIR documents against their declared `$schema` and checks cross-file consistency.
pub fn validate_documents(docs: &[(String, Vec<u8>)]) -> Vec<Issue> {
    validate::validate(docs)
}

/// Checks that every page, visual, bookmark and resource name in `report` can be used as a file
/// or folder name on Windows and Linux, and that no two of them collide (ignoring case).
pub fn check_names(report: &Report) -> Result<(), Error> {
    paths::check_report_names(report)
}

/// A standalone JSON schema (the draft-07 keywords [`validate_documents`] implements, local `#/…`
/// references only), for report documents that are not PBIR files, such as report themes.
/// Compiled once in [`JsonSchema::new`]; keep it (e.g. in a `OnceLock`) to validate many documents.
pub struct JsonSchema {
    document: std::sync::Arc<serde_json::Value>,
    compiled: schema::Compiled,
}

impl JsonSchema {
    pub fn new(schema: serde_json::Value) -> Self {
        let document = std::sync::Arc::new(schema);
        let compiled = schema::Compiled::standalone(std::sync::Arc::clone(&document));
        Self { document, compiled }
    }

    /// `(JSON pointer, message)` for each problem (at most 100), empty when `instance` is valid.
    /// Runs on a thread with a large stack, so deep documents cannot overflow the caller's.
    pub fn validate(&self, instance: &serde_json::Value) -> Vec<(String, String)> {
        schema::on_big_stack(|| self.compiled.validate(instance))
            .unwrap_or_else(|| vec![(String::new(), "validation failed unexpectedly".to_string())])
    }

    /// The schema document itself.
    pub fn document(&self) -> Option<&serde_json::Value> {
        Some(&self.document)
    }
}
