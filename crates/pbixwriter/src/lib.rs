//! L4: writes Power BI single-file packages: PBIT templates (model schema + report, no data) and
//! thin PBIX reports (report only, bound to a published semantic model).
//!
//! The layout follows `docs/formats/pbix-package.md` (loader rules, encodings, content types) and
//! `docs/formats/pbit.md` (what this writer emits and why). In short:
//!
//! - every part is Deflate-compressed; text parts are UTF-16LE without BOM, except `Connections`
//!   and the PBIR documents under `Report/definition/`, which are UTF-8 without BOM;
//! - `[Content_Types].xml` is generated the way System.IO.Packaging writes it (UTF-8 with BOM,
//!   one `Default` per extension first seen, `Override` for extension-less parts);
//! - `Version` is the lowest value that covers the features written (`1.32` once PBIR is present);
//! - `SecurityBindings` is never written (optional, and only Desktop can produce a valid one).
//!
//! Every package is checked with [`check_package`] before it is returned.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

mod check;
mod content_types;
mod names;

use bicraft_model::SemanticModel;
use bicraft_opc::{Encoding, Package, Part, encode_text};
use bicraft_report::Report;

pub use check::check_package;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Unsupported(String),
    /// A report document has an unusable path or encoding.
    #[error("report document `{path}`: {message}")]
    Document { path: String, message: String },
    /// The assembled package breaks a loader rule (see [`check_package`]).
    #[error("the package would not open in Power BI Desktop: {}", .0.join("; "))]
    Invalid(Vec<String>),
}

/// How a thin report reaches its published semantic model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteModel {
    /// Workspace connection, e.g. `powerbi://api.powerbi.com/v1.0/myorg/<Workspace>`.
    pub connection: String,
    /// Published semantic model name or id.
    pub model: String,
}

/// Part names (package paths without the leading `/`).
pub mod part {
    pub const VERSION: &str = "Version";
    pub const CONTENT_TYPES: &str = "[Content_Types].xml";
    pub const DATA_MODEL: &str = "DataModel";
    pub const DATA_MODEL_SCHEMA: &str = "DataModelSchema";
    pub const DIAGRAM_LAYOUT: &str = "DiagramLayout";
    pub const SETTINGS: &str = "Settings";
    pub const METADATA: &str = "Metadata";
    pub const CONNECTIONS: &str = "Connections";
    pub const SECURITY_BINDINGS: &str = "SecurityBindings";
    pub const LEGACY_LAYOUT: &str = "Report/Layout";
    pub const REPORT_PREFIX: &str = "Report/";
    pub const PBIR_PREFIX: &str = "Report/definition/";
    pub const STATIC_RESOURCES_PREFIX: &str = "Report/StaticResources/";
}

/// Content type of the JSON parts Desktop tags explicitly; every other part has `""`.
pub(crate) const JSON_CONTENT_TYPE: &str = "application/json";

/// `Version` minors (`1.<n>`) for the features this writer can emit (pbix-package.md Â§4).
const BASELINE_MINOR: u32 = bicraft_versions::PBIX_VERSION_BASELINE_MINOR;
const EXTERNAL_DATASET_MINOR: u32 = bicraft_versions::PBIX_VERSION_EXTERNAL_DATASET_MINOR;
const PBIR_MINOR: u32 = bicraft_versions::PBIX_VERSION_PBIR_MINOR;
const VERIFIED_ANSWERS_MINOR: u32 = bicraft_versions::PBIX_VERSION_VERIFIED_ANSWERS_MINOR;
const COPILOT_MINOR: u32 = bicraft_versions::PBIX_VERSION_COPILOT_MINOR;

/// `Settings`: inner version 4 (the latest Desktop 2.158 reads); defaults omitted, as Desktop's
/// serializer does. The two query flags are Desktop's defaults for a new file.
///
/// `QueriesSettings.Version` is **required**: Desktop validates it as the Power Query (Mashup)
/// package version when it opens the file, and a missing value makes it reject the whole file as
/// "damaged or created by an unrecognized version" (verified in Desktop 2.158). It must parse,
/// be at least `1.0.3207.0` and not be newer (by major.minor) than the opening Desktop's engine,
/// so the writer uses [`MASHUP_PACKAGE_VERSION`], older than any Desktop that reads PBIR in a
/// PBIX.
const SETTINGS_JSON: &str = r#"{"Version":4,"ReportSettings":{},"QueriesSettings":{"TypeDetectionEnabled":true,"RelationshipImportEnabled":true,"Version":"2.130.0.0"}}"#;
/// The Mashup package version written in `Settings` (see [`SETTINGS_JSON`]).
pub const MASHUP_PACKAGE_VERSION: &str = bicraft_versions::MASHUP_PACKAGE_VERSION;
/// `Metadata`: inner version 5, no auto-created relationships (Desktop opens it; PK-METADATA).
const METADATA_JSON: &str = r#"{"Version":5,"AutoCreatedRelationships":[]}"#;
/// `DiagramLayout`: one empty model diagram (Desktop lays the tables out itself).
const DIAGRAM_LAYOUT_JSON: &str = r#"{"version":"1.1.0","diagrams":[{"ordinal":0,"scrollPosition":{"x":0,"y":0},"nodes":[],"name":"All tables","zoomValue":100,"pinKeyFieldsToTop":false,"showExtraHeaderInfo":false,"hideKeyFieldsWhenCollapsed":false,"tablesLocked":false}],"selectedDiagram":"All tables","defaultDiagram":"All tables"}"#;

/// The report of a package being built, already serialized. Paths are relative to the report
/// root (`definition/â€¦`, `StaticResources/â€¦`, `CustomVisuals/â€¦`), as they sit in a PBIP
/// `<Name>.Report/` folder.
#[derive(Debug, Clone, Copy)]
pub enum ReportParts<'a> {
    /// PBIR documents, as `bicraft_pbir::write_documents` returns them, plus resources.
    Pbir(&'a [(String, Vec<u8>)]),
    /// A legacy report: the layout JSON (written as the UTF-16LE `Report/Layout`) and the raw
    /// files under `StaticResources/` and `CustomVisuals/`.
    Legacy {
        layout: &'a str,
        resources: &'a [(String, Vec<u8>)],
    },
}

/// Optional parts carried over from the file being converted.
#[derive(Debug, Clone, Copy, Default)]
pub struct Extras<'a> {
    /// `DiagramLayout` JSON (a PBIX `DiagramLayout` part or a PBIP `diagramLayout.json`); an
    /// empty model diagram when `None`.
    pub diagram_layout: Option<&'a str>,
    /// Copilot and verified-answers container files in their project form (path relative to the
    /// semantic model folder, under `Copilot/` or `VerifiedAnswers/`; text files UTF-8). They
    /// become `/Copilot/…` and `/VerifiedAnswers/…` parts, text as UTF-16LE (pbix-package.md
    /// §2, "Copilot containers"), and raise `Version` to 1.36 (1.34 for verified answers only).
    pub copilot: &'a [(String, Vec<u8>)],
}

/// Builds a PBIT: `DataModelSchema` (TMSL, UTF-16LE), the report (PBIR parts), `Version` and the
/// other parts Desktop requires (see docs/formats/pbix-package.md).
pub fn write_pbit(model: &SemanticModel, report: &Report) -> Result<Package, Error> {
    write_pbit_documents(model, &bicraft_pbir::write_documents(report))
}

/// [`write_pbit`] with the report in either form (PBIR or legacy layout) and carried-over parts.
pub fn write_pbit_with(
    model: &SemanticModel,
    report: ReportParts<'_>,
    extras: &Extras<'_>,
) -> Result<Package, Error> {
    let (report, pbir) = package_report(report)?;
    let mut parts = vec![
        text_part(part::DATA_MODEL_SCHEMA, &data_model_schema(model)),
        text_part(part::DIAGRAM_LAYOUT, diagram_layout(extras)?),
        text_part(part::SETTINGS, SETTINGS_JSON),
        text_part(part::METADATA, METADATA_JSON),
    ];
    parts.extend(report);
    let (copilot, minor) = copilot_parts(extras.copilot)?;
    parts.extend(copilot);
    finish(version(false, pbir).max_minor(minor), parts)
}

/// Builds a PBIX with data: `data_model` is the `DataModel` part (an XPress9-framed ABF backup,
/// e.g. a PBIP `.pbi/cache.abf`, which has the same framing), stored uncompressed as Desktop
/// writes it, plus the report and the parts Desktop requires. The backup is not inspected here.
pub fn write_pbix(
    data_model: Vec<u8>,
    report: ReportParts<'_>,
    extras: &Extras<'_>,
) -> Result<Package, Error> {
    if data_model.is_empty() {
        return Err(Error::Unsupported("the DataModel part is empty".into()));
    }
    let (report, pbir) = package_report(report)?;
    let mut parts = vec![
        Part::new(part::DATA_MODEL, data_model, true),
        text_part(part::DIAGRAM_LAYOUT, diagram_layout(extras)?),
        text_part(part::SETTINGS, SETTINGS_JSON),
        text_part(part::METADATA, METADATA_JSON),
    ];
    parts.extend(report);
    let (copilot, minor) = copilot_parts(extras.copilot)?;
    parts.extend(copilot);
    finish(version(false, pbir).max_minor(minor), parts)
}

/// The `DiagramLayout` text: the carried-over one when it is a JSON object, else an empty diagram.
fn diagram_layout<'a>(extras: &Extras<'a>) -> Result<&'a str, Error> {
    match extras.diagram_layout {
        None => Ok(DIAGRAM_LAYOUT_JSON),
        Some(text) => match serde_json::from_str::<serde_json::Value>(text) {
            Ok(serde_json::Value::Object(_)) => Ok(text),
            _ => Err(Error::Unsupported(
                "the diagram layout is not a JSON object".into(),
            )),
        },
    }
}

/// Report parts for [`ReportParts`], and whether they are PBIR.
fn package_report(report: ReportParts<'_>) -> Result<(Vec<Part>, bool), Error> {
    match report {
        ReportParts::Pbir(docs) => Ok((report_parts(docs)?, true)),
        ReportParts::Legacy { layout, resources } => {
            match serde_json::from_str::<serde_json::Value>(layout) {
                Ok(serde_json::Value::Object(_)) => {}
                _ => {
                    return Err(Error::Document {
                        path: part::LEGACY_LAYOUT.into(),
                        message: "the legacy layout is not a JSON object".into(),
                    });
                }
            }
            let mut parts = resource_parts(resources)?;
            parts.push(text_part(part::LEGACY_LAYOUT, layout));
            Ok((parts, false))
        }
    }
}

/// Raw report files (`StaticResources/â€¦`, `CustomVisuals/â€¦`) as package parts.
fn resource_parts(docs: &[(String, Vec<u8>)]) -> Result<Vec<Part>, Error> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(docs.len());
    for (path, data) in docs {
        let bad = |message: &str| Error::Document {
            path: path.clone(),
            message: message.to_string(),
        };
        names::check_relative_path(path).map_err(|m| bad(&m))?;
        if !seen.insert(path.to_ascii_lowercase()) {
            return Err(bad("duplicate path (package part names ignore case)"));
        }
        if !is_resource(path) {
            return Err(bad(
                "only `StaticResources/` and `CustomVisuals/` files go next to a legacy layout",
            ));
        }
        out.push(deflated(
            &names::encode_part_name(&format!("{}{path}", part::REPORT_PREFIX)),
            data.clone(),
        ));
    }
    Ok(out)
}

fn is_resource(path: &str) -> bool {
    path.starts_with("StaticResources/") || path.starts_with("CustomVisuals/")
}

/// Builds a thin PBIX: the report plus a live connection to a published semantic model.
pub fn write_thin_pbix(report: &Report, remote: &RemoteModel) -> Result<Package, Error> {
    write_thin_pbix_documents(&bicraft_pbir::write_documents(report), remote)
}

/// [`write_pbit`] from report documents that are already serialized: paths relative to the report
/// root (`definition/â€¦`, `StaticResources/â€¦`), as `bicraft_pbir::write_documents` returns them
/// and as they sit in a PBIP `<Name>.Report/` folder.
pub fn write_pbit_documents(
    model: &SemanticModel,
    report_docs: &[(String, Vec<u8>)],
) -> Result<Package, Error> {
    write_pbit_with(model, ReportParts::Pbir(report_docs), &Extras::default())
}

/// [`write_thin_pbix`] from report documents that are already serialized (see
/// [`write_pbit_documents`]).
pub fn write_thin_pbix_documents(
    report_docs: &[(String, Vec<u8>)],
    remote: &RemoteModel,
) -> Result<Package, Error> {
    let connections = connections_json(remote)?;
    let report = report_parts(report_docs)?;
    let mut parts = vec![
        deflated(part::CONNECTIONS, connections.into_bytes()),
        text_part(part::SETTINGS, SETTINGS_JSON),
        text_part(part::METADATA, METADATA_JSON),
    ];
    parts.extend(report);
    finish(version(true, true), parts)
}

/// The `Connections` document for a live connection to a model published in a Power BI workspace
/// (connection type `pbiServiceXmlaStyleLive`, `Connections` version 4: BIcraft's choice, not
/// verified against a Desktop-saved part, docs/formats/pbit.md).
pub fn connections_json(remote: &RemoteModel) -> Result<String, Error> {
    let ws = remote.connection.trim();
    let has_scheme = ws
        .get(..XMLA_SCHEME.len())
        .is_some_and(|s| s.eq_ignore_ascii_case(XMLA_SCHEME));
    if !has_scheme || ws.len() == XMLA_SCHEME.len() {
        return Err(Error::Unsupported(format!(
            "the workspace connection must be a `{XMLA_SCHEME}â€¦` URL (got `{ws}`)"
        )));
    }
    let model = remote.model.trim();
    if model.is_empty() {
        return Err(Error::Unsupported(
            "the published semantic model name is empty".into(),
        ));
    }
    if [ws, model].iter().any(|s| s.chars().any(char::is_control)) {
        return Err(Error::Unsupported(
            "the connection contains control characters".into(),
        ));
    }
    let connection_string = format!(
        "Data Source={};Initial Catalog={}",
        quote_conn_value(ws),
        quote_conn_value(model)
    );
    let doc = serde_json::json!({
        "Version": 4,
        "Connections": [{
            "Name": "EntityDataSource",
            "ConnectionString": connection_string,
            "ConnectionType": "pbiServiceXmlaStyleLive",
        }],
    });
    Ok(doc.to_string())
}

const XMLA_SCHEME: &str = "powerbi://";

/// OLE DB connection-string quoting: a value containing `;`, quotes or edge whitespace is wrapped
/// in double quotes, inner double quotes doubled.
fn quote_conn_value(v: &str) -> String {
    let needs = v.contains([';', '"', '\''])
        || v.starts_with(char::is_whitespace)
        || v.ends_with(char::is_whitespace);
    if needs {
        format!("\"{}\"", v.replace('"', "\"\""))
    } else {
        v.to_string()
    }
}

/// Copilot container files as package parts, and the `Version` minor they need (0: none).
fn copilot_parts(files: &[(String, Vec<u8>)]) -> Result<(Vec<Part>, u32), Error> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(files.len());
    let mut minor = 0;
    for (path, data) in files {
        let bad = |message: &str| Error::Document {
            path: path.clone(),
            message: message.to_string(),
        };
        names::check_relative_path(path).map_err(|m| bad(&m))?;
        if !seen.insert(path.to_ascii_lowercase()) {
            return Err(bad("duplicate path (package part names ignore case)"));
        }
        let lower = path.to_ascii_lowercase();
        if lower.starts_with("copilot/") {
            minor = minor.max(COPILOT_MINOR);
        } else if lower.starts_with("verifiedanswers/") {
            minor = minor.max(VERIFIED_ANSWERS_MINOR);
        } else {
            return Err(bad(
                "only `Copilot/` and `VerifiedAnswers/` files are Copilot container files",
            ));
        }
        let bytes = if bicraft_opc::is_container_text(path) {
            bicraft_opc::container_text_to_package(data).ok_or_else(|| bad("not UTF-8 text"))?
        } else {
            data.clone()
        };
        out.push(deflated(&names::encode_part_name(path), bytes));
    }
    Ok((out, minor))
}

/// Raises a `1.<minor>` version text to at least `1.<at_least>`.
trait MaxMinor {
    fn max_minor(self, at_least: u32) -> String;
}

impl MaxMinor for String {
    fn max_minor(self, at_least: u32) -> String {
        match self.strip_prefix("1.").and_then(|m| m.parse::<u32>().ok()) {
            Some(m) if m < at_least => format!("1.{at_least}"),
            _ => self,
        }
    }
}

/// `Version` text: `1.<minor>` for the highest feature written.
fn version(external_dataset: bool, pbir: bool) -> String {
    let mut minor = BASELINE_MINOR;
    if external_dataset {
        minor = minor.max(EXTERNAL_DATASET_MINOR);
    }
    if pbir {
        minor = minor.max(PBIR_MINOR);
    }
    format!("1.{minor}")
}

/// TMSL as Desktop stores it in a PBIT: CRLF line endings, no trailing newline. A model without a
/// name gets a deterministic GUID-shaped one (Desktop always names the database).
fn data_model_schema(model: &SemanticModel) -> String {
    let text = if model.name.trim().is_empty() {
        let mut named = model.clone();
        named.name = names::stable_guid(&bicraft_tmsl::write(model));
        bicraft_tmsl::write(&named)
    } else {
        bicraft_tmsl::write(model)
    };
    // JSON escapes line breaks inside strings, so every raw `\n` here is layout whitespace.
    text.trim_end_matches('\n').replace('\n', "\r\n")
}

fn text_part(name: &str, text: &str) -> Part {
    deflated(name, encode_text(text, Encoding::Utf16Le))
}

fn deflated(name: &str, data: Vec<u8>) -> Part {
    Part::new(name, data, false)
}

/// Validates report documents and turns them into package parts: `StaticResources/` first, then
/// `definition/` in Desktop's order (version, report, pages, bookmarks).
fn report_parts(docs: &[(String, Vec<u8>)]) -> Result<Vec<Part>, Error> {
    let mut resources = Vec::new();
    let mut definition = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (path, data) in docs {
        let bad = |message: &str| Error::Document {
            path: path.clone(),
            message: message.to_string(),
        };
        names::check_relative_path(path).map_err(|m| bad(&m))?;
        if !seen.insert(path.to_ascii_lowercase()) {
            return Err(bad("duplicate path (package part names ignore case)"));
        }
        if path.starts_with("definition/") {
            if data.starts_with(&[0xEF, 0xBB, 0xBF]) {
                return Err(bad("PBIR documents must be UTF-8 without BOM"));
            }
            definition.push((path, data));
        } else if is_resource(path) {
            resources.push((path, data));
        } else {
            return Err(bad(
                "only `definition/`, `StaticResources/` and `CustomVisuals/` documents belong in a package",
            ));
        }
    }
    if definition.is_empty() {
        return Err(Error::Unsupported(
            "the report has no PBIR `definition/` documents".into(),
        ));
    }
    definition.sort_by_key(|(p, _)| definition_rank(p));
    Ok(resources
        .into_iter()
        .chain(definition)
        .map(|(path, data)| {
            deflated(
                &names::encode_part_name(&format!("{}{path}", part::REPORT_PREFIX)),
                data.clone(),
            )
        })
        .collect())
}

/// Stable sort key: Desktop writes `version.json`, `report.json`, `reportExtensions.json`,
/// `pages/pages.json`, then each page followed by its visuals, then bookmarks.
fn definition_rank(path: &str) -> u8 {
    match path {
        "definition/version.json" => 0,
        "definition/report.json" => 1,
        "definition/reportExtensions.json" => 2,
        "definition/pages/pages.json" => 3,
        p if p.starts_with("definition/pages/") => 4,
        p if p.starts_with("definition/bookmarks/") => 5,
        _ => 6,
    }
}

/// Puts `Version` first and `[Content_Types].xml` second (System.IO.Packaging's order), then
/// checks the loader rules.
fn finish(version: String, rest: Vec<Part>) -> Result<Package, Error> {
    let mut parts = Vec::with_capacity(rest.len().saturating_add(2));
    parts.push(text_part(part::VERSION, &version));
    parts.extend(rest);
    let xml = content_types::build(&parts);
    parts.insert(1, deflated(part::CONTENT_TYPES, xml));
    let pkg = Package { parts };
    let violations = check_package(&pkg);
    if violations.is_empty() {
        Ok(pkg)
    } else {
        Err(Error::Invalid(violations))
    }
}

/// Content type Desktop gives a part (all `""` except a few JSON parts).
pub(crate) fn content_type_of(name: &str) -> &'static str {
    const JSON_PARTS: [&str; 3] = [part::SETTINGS, part::METADATA, "UnappliedChanges"];
    if JSON_PARTS.iter().any(|p| p.eq_ignore_ascii_case(name)) {
        JSON_CONTENT_TYPE
    } else {
        ""
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_carry_the_mashup_package_version() {
        let v: serde_json::Value = serde_json::from_str(SETTINGS_JSON).unwrap();
        assert_eq!(v["QueriesSettings"]["Version"], MASHUP_PACKAGE_VERSION);
    }

    #[test]
    fn version_is_the_lowest_covering_value() {
        assert_eq!(version(false, false), "1.28");
        assert_eq!(version(true, false), "1.29");
        assert_eq!(version(false, true), "1.32");
        assert_eq!(version(true, true), "1.32");
    }

    #[test]
    fn connection_values_are_quoted_only_when_needed() {
        assert_eq!(quote_conn_value("Sales"), "Sales");
        assert_eq!(quote_conn_value("a;b"), "\"a;b\"");
        assert_eq!(quote_conn_value("say \"hi\""), "\"say \"\"hi\"\"\"");
    }

    #[test]
    fn connections_document_shape() {
        let remote = RemoteModel {
            connection: "powerbi://api.powerbi.com/v1.0/myorg/Finance Team".into(),
            model: "Sales".into(),
        };
        let v: serde_json::Value = serde_json::from_str(&connections_json(&remote).unwrap()).unwrap();
        assert_eq!(v["Version"], 4);
        let c = &v["Connections"][0];
        assert_eq!(c["Name"], "EntityDataSource");
        assert_eq!(c["ConnectionType"], "pbiServiceXmlaStyleLive");
        assert_eq!(
            c["ConnectionString"],
            "Data Source=powerbi://api.powerbi.com/v1.0/myorg/Finance Team;Initial Catalog=Sales"
        );
    }

    #[test]
    fn bad_remotes_are_rejected() {
        for (connection, model) in [
            ("https://example.com", "M"),
            ("powerbi://", "M"),
            ("powerbi://api.powerbi.com/v1.0/myorg/W", " "),
            ("powerbi://api.powerbi.com/v1.0/myorg/W\n", "M\u{0}"),
        ] {
            let remote = RemoteModel {
                connection: connection.into(),
                model: model.into(),
            };
            assert!(connections_json(&remote).is_err(), "{connection} / {model}");
        }
    }

    #[test]
    fn schema_text_uses_crlf_without_trailing_newline() {
        let model = SemanticModel {
            name: "m".into(),
            ..Default::default()
        };
        let text = data_model_schema(&model);
        assert!(text.contains("\r\n"));
        assert!(!text.ends_with('\n'));
        assert!(!text.replace("\r\n", "").contains('\n'));
    }

    #[test]
    fn unnamed_models_get_a_stable_name() {
        let a = data_model_schema(&SemanticModel::default());
        let b = data_model_schema(&SemanticModel::default());
        assert_eq!(a, b);
        let m = bicraft_tmsl::read(&a).unwrap();
        assert_eq!(m.name.len(), 36);
    }

    const LAYOUT: &str = r#"{"config":"{}","layoutOptimization":0,"sections":[]}"#;

    fn part_names(pkg: &Package) -> Vec<&str> {
        pkg.parts.iter().map(|p| p.name.as_str()).collect()
    }

    #[test]
    fn legacy_templates_carry_the_layout_and_resources() {
        let resources = vec![
            (
                "StaticResources/RegisteredResources/a b.png".to_string(),
                vec![1, 2],
            ),
            ("CustomVisuals/v/package.json".to_string(), b"{}".to_vec()),
        ];
        let report = ReportParts::Legacy {
            layout: LAYOUT,
            resources: &resources,
        };
        let pkg = write_pbit_with(&SemanticModel::default(), report, &Extras::default()).unwrap();
        let names = part_names(&pkg);
        assert!(names.contains(&"Report/Layout"), "{names:?}");
        assert!(
            names.contains(&"Report/CustomVisuals/v/package.json"),
            "{names:?}"
        );
        assert!(
            names
                .iter()
                .any(|n| n.starts_with("Report/StaticResources/RegisteredResources/a"))
        );
        assert!(!names.iter().any(|n| n.starts_with("Report/definition/")));
        let layout = pkg.get("Report/Layout").unwrap();
        assert_eq!(
            bicraft_opc::decode_text("Report/Layout", &layout.data).unwrap().0,
            LAYOUT
        );
        let version = pkg.get("Version").unwrap();
        assert_eq!(
            bicraft_opc::decode_text("Version", &version.data).unwrap().0,
            "1.28"
        );
        assert!(check_package(&pkg).is_empty());
    }

    #[test]
    fn pbix_packages_store_the_data_model() {
        let extras = Extras {
            diagram_layout: Some(r#"{"version":"1.1.0","diagrams":[]}"#),
            ..Extras::default()
        };
        let report = ReportParts::Legacy {
            layout: LAYOUT,
            resources: &[],
        };
        let pkg = write_pbix(vec![7; 32], report, &extras).unwrap();
        let dm = pkg.get("DataModel").unwrap();
        assert!(dm.stored);
        assert_eq!(dm.data, vec![7; 32]);
        assert!(pkg.get("DataModelSchema").is_none());
        let diagram = pkg.get("DiagramLayout").unwrap();
        assert!(
            bicraft_opc::decode_text("DiagramLayout", &diagram.data)
                .unwrap()
                .0
                .contains("1.1.0")
        );
        assert!(check_package(&pkg).is_empty());
        // Round trip through bytes.
        let back = Package::read_bytes(&pkg.to_bytes().unwrap()).unwrap();
        assert_eq!(back.get("DataModel").unwrap().data, vec![7; 32]);
    }

    /// Copilot container files become UTF-16LE parts (raw resource packages stay raw) and
    /// raise `Version`.
    #[test]
    fn copilot_files_become_container_parts() {
        let files = vec![
            (
                "Copilot/version.json".to_string(),
                br#"{"version":"1.0.0"}"#.to_vec(),
            ),
            (
                "Copilot/Instructions/instructions.md".to_string(),
                "Réponds en français".as_bytes().to_vec(),
            ),
            (
                "Copilot/VerifiedAnswers/ResourcePackages/a b.json".to_string(),
                vec![1, 2, 3],
            ),
        ];
        let extras = Extras {
            copilot: &files,
            ..Extras::default()
        };
        let report = ReportParts::Legacy {
            layout: LAYOUT,
            resources: &[],
        };
        let pkg = write_pbix(vec![7; 32], report, &extras).unwrap();
        assert!(check_package(&pkg).is_empty(), "{:?}", check_package(&pkg));
        let md = pkg.get("Copilot/Instructions/instructions.md").unwrap();
        assert_eq!(md.data, encode_text("Réponds en français", Encoding::Utf16Le));
        assert_eq!(
            pkg.get("Copilot/VerifiedAnswers/ResourcePackages/a%20b.json")
                .unwrap()
                .data,
            vec![1, 2, 3]
        );
        let version = pkg.get("Version").unwrap();
        assert_eq!(
            bicraft_opc::decode_text("Version", &version.data).unwrap().0,
            "1.36"
        );
        // Verified answers alone: 1.34; anything else is refused.
        let only = vec![("VerifiedAnswers/version.json".to_string(), b"{}".to_vec())];
        let extras = Extras {
            copilot: &only,
            ..Extras::default()
        };
        let pkg = write_pbit_with(&SemanticModel::default(), report, &extras).unwrap();
        let version = pkg.get("Version").unwrap();
        assert_eq!(
            bicraft_opc::decode_text("Version", &version.data).unwrap().0,
            "1.34"
        );
        for bad in [("Other/x.json", b"{}".to_vec()), ("Copilot/x.json", vec![0xFF])] {
            let files = vec![(bad.0.to_string(), bad.1)];
            let extras = Extras {
                copilot: &files,
                ..Extras::default()
            };
            assert!(write_pbix(vec![7; 32], report, &extras).is_err());
        }
    }

    #[test]
    fn bad_conversion_inputs_are_refused() {
        let none: &[(String, Vec<u8>)] = &[];
        let legacy = |layout| ReportParts::Legacy {
            layout,
            resources: none,
        };
        assert!(write_pbix(Vec::new(), legacy(LAYOUT), &Extras::default()).is_err());
        assert!(write_pbix(vec![1], legacy("[1]"), &Extras::default()).is_err());
        let bad_diagram = Extras {
            diagram_layout: Some("not json"),
            ..Extras::default()
        };
        assert!(write_pbix(vec![1], legacy(LAYOUT), &bad_diagram).is_err());
        let outside = vec![("definition/report.json".to_string(), b"{}".to_vec())];
        let report = ReportParts::Legacy {
            layout: LAYOUT,
            resources: &outside,
        };
        assert!(write_pbit_with(&SemanticModel::default(), report, &Extras::default()).is_err());
    }

    #[test]
    fn report_paths_are_validated() {
        let doc = |p: &str| vec![(p.to_string(), b"{}".to_vec())];
        for p in [
            "../x.json",
            "/definition/x.json",
            "definition\\x.json",
            "definition//x.json",
            "other/x.json",
            "definition.pbir",
        ] {
            assert!(report_parts(&doc(p)).is_err(), "{p}");
        }
        let mut dup = doc("definition/a.json");
        dup.extend(doc("definition/A.json"));
        assert!(report_parts(&dup).is_err());
        assert!(report_parts(&doc("StaticResources/x.png")).is_err());
        let bom = vec![("definition/a.json".to_string(), b"\xEF\xBB\xBF{}".to_vec())];
        assert!(report_parts(&bom).is_err());
    }
}
