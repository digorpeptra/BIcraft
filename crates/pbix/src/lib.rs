//! L4: PBIX and PBIT packages. A PBIT carries its model as TMSL text (`DataModelSchema`); a PBIX
//! carries it as an XPress9-compressed Analysis Services backup (`DataModel`). See
//! `docs/formats/pbix-package.md` for the part layout.
//!
//! [`read_model`] reads the model of either. [`write_model`] writes an edited model back into
//! an existing package: the TMSL text of a PBIT, or, for a PBIX, the metadata-only edits
//! (measures, descriptions, display properties, annotations, roles, shared expression text)
//! through XPress9 → ABF → `metadata.sqlitedb` and back.
//!
//! [`read_report`] reads the report: PBIR parts (`Report/definition/…`, through
//! `bicraft_pbir`) or the legacy UTF-16LE `Report/Layout` document (through `bicraft_layout`).
//! [`write_report`] writes an edited report back in the same form, touching only the report
//! parts that change.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

use bicraft_model::SemanticModel;
use bicraft_opc::{Encoding, Package, decode_text, encode_text};
use bicraft_report::{Report, Resource};
use serde::Serialize;

pub const DATA_MODEL: &str = "DataModel";
pub const DATA_MODEL_SCHEMA: &str = "DataModelSchema";
pub const LEGACY_LAYOUT: &str = "Report/Layout";
pub const PBIR_PREFIX: &str = "Report/definition/";
/// Folder of the report parts; PBIR documents are keyed relative to it.
pub const REPORT_PREFIX: &str = "Report/";
pub const STATIC_RESOURCES_PREFIX: &str = "Report/StaticResources/";
/// Embedded custom visuals: `Report/CustomVisuals/<visual guid>/package.json` and
/// `…/resources/<visual guid>.pbiviz.json` (`docs/formats/pbix-package.md` §2).
pub const CUSTOM_VISUALS_PREFIX: &str = "Report/CustomVisuals/";
/// The Copilot container (AI instructions, AI data schema, example prompts, settings and, in
/// `Copilot/VerifiedAnswers/`, verified answers): `docs/formats/pbix-package.md` §2, "Copilot
/// containers". In a project it is `<Name>.SemanticModel/Copilot/`.
pub const COPILOT_PREFIX: &str = "Copilot/";
/// The older, root-level verified-answers container (not allowed next to `Copilot/`).
pub const VERIFIED_ANSWERS_PREFIX: &str = "VerifiedAnswers/";

/// Whether a part belongs to a Copilot or verified-answers container.
pub fn is_copilot_part(name: &str) -> bool {
    starts_with_ignore_case(name, COPILOT_PREFIX) || starts_with_ignore_case(name, VERIFIED_ANSWERS_PREFIX)
}

fn starts_with_ignore_case(name: &str, prefix: &str) -> bool {
    name.get(..prefix.len())
        .is_some_and(|h| h.eq_ignore_ascii_case(prefix))
}

/// The Copilot and verified-answers container files in their project form: (path relative to
/// the package root, e.g. `Copilot/Instructions/instructions.md`, bytes). Text files
/// ([`bicraft_opc::is_container_text`]) go from UTF-16LE to UTF-8; a text file that is not valid
/// UTF-16LE, and every other file, keeps its bytes. Part names are percent-decoded.
pub fn copilot_files(pkg: &Package) -> Vec<(String, Vec<u8>)> {
    pkg.parts
        .iter()
        .filter(|p| is_copilot_part(&p.name) && !p.is_deferred())
        .map(|p| {
            let path = percent_decode(&p.name);
            let bytes = if bicraft_opc::is_container_text(&path) {
                bicraft_opc::container_text_to_project(&p.data).unwrap_or_else(|| p.data.clone())
            } else {
                p.data.clone()
            };
            (path, bytes)
        })
        .collect()
}

/// Decodes `%XX` escapes; text that does not decode to UTF-8 is returned unchanged.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (b, hex) {
            (b'%', Some(v)) => {
                out.push(v);
                i += 3;
            }
            _ => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_string())
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Package(#[from] bicraft_opc::Error),
    #[error("DataModelSchema: {0}")]
    Schema(#[from] bicraft_tmsl::Error),
    #[error("DataModel: XPress9: {0}")]
    Xpress9(#[from] bicraft_xpress9::Error),
    #[error("DataModel: ABF: {0}")]
    Abf(#[from] bicraft_abf::Error),
    #[error("DataModel: the backup has no metadata.sqlitedb")]
    NoMetadata,
    #[error("DataModel: metadata.sqlitedb: {0}")]
    Metadata(#[from] bicraft_metadata::Error),
    #[error(
        "the package has no semantic model: a thin report, whose model is published in the Power BI \
         service and reached through a live connection"
    )]
    NoModel,
    #[error("Report/definition: {0}")]
    Pbir(#[from] bicraft_pbir::Error),
    #[error("Report/Layout: {0}")]
    Layout(#[from] bicraft_layout::Error),
    #[error("the package has no report (neither `Report/definition/` nor `Report/Layout`)")]
    NoReport,
    /// The edited model differs from the file's in ways a metadata-only save cannot write
    /// (they would need new VertiPaq data). Lists every such difference.
    #[error(
        "these model changes cannot be saved into a PBIX (only metadata edits are supported: measures,          descriptions, display properties, annotations, roles, shared expression text): {}",
        list_changes(.0)
    )]
    UnsupportedChange(Vec<bicraft_metadata::UnsupportedChange>),
    /// A structural model change (tables, columns, relationships, data) that cannot be written,
    /// with the reasons.
    #[error("these model changes cannot be saved into the PBIX: {0}")]
    Structural(String),
    /// The re-compressed `DataModel` did not decode back to the backup that was written.
    /// The `DataModel` part is too large to rewrite (it was left in the file when the package
    /// was opened, see `bicraft_opc::Part::deferred`).
    #[error(
        "DataModel: the {0}-byte part is too large to rewrite; model edits cannot be saved into this \
         package (report edits can, and the part is kept as it is)"
    )]
    DataModelTooLargeToWrite(u64),
    #[error("DataModel: internal check failed: {0}")]
    Verify(&'static str),
    /// The `DataModel` part declares more decompressed bytes than [`DataModelLimits`] allow.
    #[error(
        "DataModel: the {part}-byte part declares {declared} decompressed bytes, over the {limit}-byte \
         limit for its size (DataModelLimits)"
    )]
    DataModelTooLarge {
        part: usize,
        declared: u64,
        limit: usize,
    },
}

fn list_changes(changes: &[bicraft_metadata::UnsupportedChange]) -> String {
    changes
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

/// Keeps unsupported changes as their own variant.
fn metadata_error(e: bicraft_metadata::Error) -> Error {
    match e {
        bicraft_metadata::Error::UnsupportedChange(list) => Error::UnsupportedChange(list),
        other => Error::Metadata(other),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Model with data (`DataModel`).
    Pbix,
    /// Template: model schema without data (`DataModelSchema`).
    Pbit,
    /// Report only, no model part.
    Thin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReportFormat {
    /// `Report/Layout` (single JSON document).
    Legacy,
    /// `Report/definition/…` (PBIR, one file per page and visual).
    Pbir,
    None,
}

#[derive(Debug, Clone, Serialize)]
pub struct PartInfo {
    pub name: String,
    pub bytes: usize,
    pub stored: bool,
    /// Detected text encoding for parts known to be text; `None` for binary or unknown parts.
    pub encoding: Option<&'static str>,
    /// Set when the part was too large to read into memory and stays in the file.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub deferred: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Inventory {
    pub kind: Kind,
    pub report_format: ReportFormat,
    pub parts: Vec<PartInfo>,
}

/// The `Connections` part of a thin report (`docs/formats/pbit.md`, "`Connections` (thin
/// PBIX)").
pub const CONNECTIONS: &str = "Connections";
/// Largest `Connections` part read (a few hundred bytes in the files seen).
const MAX_CONNECTIONS: usize = 1 << 20;
/// Longest connection value kept.
const MAX_CONNECTION_VALUE: usize = 2048;

/// The live connection of a thin report: the first entry of its `Connections` part. Only the
/// connection type, the data source and the catalog are kept, never the rest of the connection
/// string (it may hold credentials).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LiveConnection {
    /// `ConnectionType`, e.g. `pbiServiceXmlaStyleLive` or `analysisServicesDatabaseLive`.
    #[serde(rename = "type")]
    pub connection_type: Option<String>,
    /// `Data Source` of the connection string (a `powerbi://…` workspace, a server).
    pub data_source: Option<String>,
    /// `Initial Catalog` of the connection string (the published semantic model or database).
    pub catalog: Option<String>,
}

/// Value of `key` in an OLE DB style connection string (`Key=Value;…`, a value in double or
/// single quotes may hold `;`, a doubled quote is one quote), the key matched without case.
fn connection_value(conn: &str, key: &str) -> Option<String> {
    let mut rest = conn;
    while !rest.is_empty() {
        let eq = rest.find('=')?;
        let k = rest.get(..eq)?.trim().trim_start_matches(';').trim();
        let after = rest.get(eq + 1..)?.trim_start();
        let (value, next) = match after.chars().next() {
            Some(q @ ('"' | '\'')) => {
                let mut v = String::new();
                let mut it = after.char_indices().skip(1).peekable();
                let mut end = after.len();
                while let Some((i, c)) = it.next() {
                    if c == q {
                        if it.peek().is_some_and(|(_, n)| *n == q) {
                            v.push(q);
                            it.next();
                            continue;
                        }
                        end = i + c.len_utf8();
                        break;
                    }
                    v.push(c);
                }
                let tail = after.get(end..).unwrap_or("");
                (v, tail.find(';').map_or("", |i| tail.get(i + 1..).unwrap_or("")))
            }
            _ => match after.find(';') {
                Some(i) => (
                    after.get(..i).unwrap_or("").trim().to_string(),
                    after.get(i + 1..).unwrap_or(""),
                ),
                None => (after.trim().to_string(), ""),
            },
        };
        if k.eq_ignore_ascii_case(key) {
            return (!value.is_empty() && value.len() <= MAX_CONNECTION_VALUE).then_some(value);
        }
        rest = next;
    }
    None
}

/// The live connection a thin report names in its `Connections` part, when it has one that
/// reads (UTF-8 JSON, at most 1 MiB).
pub fn live_connection(pkg: &Package) -> Option<LiveConnection> {
    let part = pkg.get(CONNECTIONS)?;
    if part.is_deferred() || part.data.len() > MAX_CONNECTIONS {
        return None;
    }
    let (text, _) = decode_text(CONNECTIONS, &part.data).ok()?;
    let doc: serde_json::Value = serde_json::from_str(&text).ok()?;
    let first = doc.get("Connections")?.as_array()?.first()?;
    let short = |s: &str| (!s.is_empty() && s.len() <= MAX_CONNECTION_VALUE).then(|| s.to_string());
    let conn = first
        .get("ConnectionString")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    Some(LiveConnection {
        connection_type: first
            .get("ConnectionType")
            .and_then(|v| v.as_str())
            .and_then(short),
        data_source: connection_value(conn, "Data Source"),
        catalog: connection_value(conn, "Initial Catalog"),
    })
}

/// Parts known to hold text; everything else is reported as binary/unknown.
const TEXT_PARTS: [&str; 8] = [
    "Version",
    "Metadata",
    "Settings",
    "DiagramLayout",
    "Connections",
    "[Content_Types].xml",
    LEGACY_LAYOUT,
    DATA_MODEL_SCHEMA,
];

pub fn inventory(pkg: &Package) -> Inventory {
    let kind = if pkg.get(DATA_MODEL_SCHEMA).is_some() {
        Kind::Pbit
    } else if pkg.get(DATA_MODEL).is_some() {
        Kind::Pbix
    } else {
        Kind::Thin
    };
    let report_format = if pkg.parts.iter().any(|p| p.name.starts_with(PBIR_PREFIX)) {
        ReportFormat::Pbir
    } else if pkg.get(LEGACY_LAYOUT).is_some() {
        ReportFormat::Legacy
    } else {
        ReportFormat::None
    };
    let parts =
        pkg.parts
            .iter()
            .map(|p| {
                let texty = TEXT_PARTS.iter().any(|t| t.eq_ignore_ascii_case(&p.name))
                    || (p.name.starts_with(PBIR_PREFIX) && p.name.ends_with(".json"));
                let encoding = texty
                    .then(|| decode_text(&p.name, &p.data).ok())
                    .flatten()
                    .map(|(_, e)| match e {
                        Encoding::Utf8 => "utf-8",
                        Encoding::Utf8Bom => "utf-8-bom",
                        Encoding::Utf16Le => "utf-16le",
                        Encoding::Utf16LeBom => "utf-16le-bom",
                    });
                PartInfo {
                    name: p.name.clone(),
                    bytes: usize::try_from(p.len()).unwrap_or(usize::MAX),
                    stored: p.stored,
                    encoding,
                    deferred: p.is_deferred(),
                }
            })
            .collect();
    Inventory {
        kind,
        report_format,
        parts,
    }
}

/// Largest decompressed `DataModel` accepted. Input is hostile: the XPress9 framing declares sizes.
pub const MAX_DATA_MODEL_BYTES: usize = 4 << 30;

/// Default [`DataModelLimits::max_ratio`]. Desktop's `DataModel` parts expand about 7×; XPress9
/// can reach ~43 000× on zeros.
pub const MAX_DATA_MODEL_RATIO: usize = 1000;

/// Default [`DataModelLimits::min_allowance`]: 64 MiB.
pub const MIN_DATA_MODEL_ALLOWANCE: usize = 64 << 20;

/// Caps on decompressing a `DataModel` part (docs/fuzzing.md, F3). The sizes declared by the
/// XPress9 chunk envelopes are checked against [`DataModelLimits::limit_for`] before anything
/// is decoded, so a small part cannot make the reader allocate gigabytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataModelLimits {
    /// Largest decompressed backup accepted, whatever the part size.
    pub max_bytes: usize,
    /// Largest expansion accepted, as a multiple of the part size.
    pub max_ratio: usize,
    /// Size always accepted (up to `max_bytes`), so small models need no ratio.
    pub min_allowance: usize,
}

impl DataModelLimits {
    /// [`MAX_DATA_MODEL_BYTES`], [`MAX_DATA_MODEL_RATIO`], [`MIN_DATA_MODEL_ALLOWANCE`].
    pub const DEFAULT: DataModelLimits = DataModelLimits {
        max_bytes: MAX_DATA_MODEL_BYTES,
        max_ratio: MAX_DATA_MODEL_RATIO,
        min_allowance: MIN_DATA_MODEL_ALLOWANCE,
    };

    /// The decompressed size accepted for a part of `part_len` bytes:
    /// `min(max_bytes, max(part_len × max_ratio, min_allowance))`.
    pub fn limit_for(&self, part_len: usize) -> usize {
        part_len
            .saturating_mul(self.max_ratio)
            .max(self.min_allowance)
            .min(self.max_bytes)
    }
}

impl Default for DataModelLimits {
    fn default() -> Self {
        DataModelLimits::DEFAULT
    }
}

/// Decompresses a `DataModel` part within `limits`, refusing it from its declared sizes first.
fn decompress_data_model(part: &[u8], limits: &DataModelLimits) -> Result<Vec<u8>, Error> {
    let limit = limits.limit_for(part.len());
    let declared = bicraft_xpress9::inspect_datamodel(part)?.total_uncompressed;
    if declared > u64::try_from(limit).unwrap_or(u64::MAX) {
        return Err(Error::DataModelTooLarge {
            part: part.len(),
            declared,
            limit,
        });
    }
    Ok(bicraft_xpress9::decompress_datamodel(part, limit)?)
}

/// Reads the semantic model: from `DataModelSchema` (PBIT, TMSL text) or from the binary
/// `DataModel` (PBIX): XPress9 → ABF backup → `metadata.sqlitedb` → model.
pub fn read_model(pkg: &Package) -> Result<SemanticModel, Error> {
    read_model_with(pkg, &DataModelLimits::default())
}

/// [`read_model`] with explicit [`DataModelLimits`].
pub fn read_model_with(pkg: &Package, limits: &DataModelLimits) -> Result<SemanticModel, Error> {
    read_model_keeping(pkg, limits).map(|(model, _)| model)
}

/// [`read_model_with`], also returning the decompressed backup of a `DataModel` part when it
/// was decompressed in memory ([`read_data_model_keeping`]); `None` for a PBIT, a deferred part
/// or a part read as a stream.
pub fn read_model_keeping(
    pkg: &Package,
    limits: &DataModelLimits,
) -> Result<(SemanticModel, Option<Vec<u8>>), Error> {
    if let Some(part) = pkg.get(DATA_MODEL_SCHEMA) {
        let (text, _) = decode_text(&part.name, &part.data)?;
        return Ok((bicraft_tmsl::read(&text)?, None));
    }
    if let Some(part) = pkg.get(DATA_MODEL) {
        if part.is_deferred() {
            return Ok((large::read_data_model_streamed(part)?, None));
        }
        return read_data_model_keeping(&part.data, limits);
    }
    Err(Error::NoModel)
}

/// Decodes a PBIX `DataModel` part (or a PBIP `.pbi/cache.abf`, which has the same framing).
pub fn read_data_model(part: &[u8]) -> Result<SemanticModel, Error> {
    read_data_model_with(part, &DataModelLimits::default())
}

/// [`read_data_model`] with explicit [`DataModelLimits`].
///
/// A part whose backup is larger than `limits.max_bytes` but within `limits.max_ratio` of the
/// part is read as a stream instead ([`large`]): only `metadata.sqlitedb` is kept in memory.
pub fn read_data_model_with(part: &[u8], limits: &DataModelLimits) -> Result<SemanticModel, Error> {
    read_data_model_keeping(part, limits).map(|(model, _)| model)
}

/// [`read_data_model_with`], also returning the decompressed backup when it was decompressed
/// in memory (`None` when the part was read as a stream), so that a caller about to read the
/// stored data does not decompress the part a second time.
pub fn read_data_model_keeping(
    part: &[u8],
    limits: &DataModelLimits,
) -> Result<(SemanticModel, Option<Vec<u8>>), Error> {
    let declared = bicraft_xpress9::inspect_datamodel(part)?.total_uncompressed;
    let by_ratio = u64::try_from(part.len().saturating_mul(limits.max_ratio)).unwrap_or(u64::MAX);
    let max_bytes = u64::try_from(limits.max_bytes).unwrap_or(u64::MAX);
    if declared > max_bytes && declared <= by_ratio {
        return Ok((large::read_data_model_bytes_streamed(part)?, None));
    }
    let abf_bytes = decompress_data_model(part, limits)?;
    let model = {
        let abf = bicraft_abf::Abf::parse(&abf_bytes)?;
        let sqlitedb = abf.metadata_sqlitedb().ok_or(Error::NoMetadata)?;
        bicraft_metadata::read_sqlitedb(sqlitedb)?
    };
    Ok((model, Some(abf_bytes)))
}

/// The semantic model of a backup's `metadata.sqlitedb` (read apart, e.g. by
/// [`backup_index_at`]).
pub fn model_from_sqlitedb(sqlitedb: &[u8]) -> Result<SemanticModel, Error> {
    Ok(bicraft_metadata::read_sqlitedb(sqlitedb)?)
}

/// Writes `model` into a copy of `pkg`, which must be the package the model was read from (with
/// metadata-only edits applied).
///
/// - PBIT (`DataModelSchema`): the TMSL text of `model`, in the part's original text encoding.
/// - PBIX (`DataModel`): [`write_data_model`]; the part is stored (not deflated), as Desktop
///   writes it.
///
/// Every other part keeps its bytes, its compression flag and its position. Differences that
/// need new VertiPaq data are refused with [`Error::UnsupportedChange`].
pub fn write_model(pkg: &Package, model: &SemanticModel) -> Result<Package, Error> {
    write_model_with(pkg, model, &DataModelLimits::default())
}

/// [`write_model`] with explicit [`DataModelLimits`] for decompressing the original part.
pub fn write_model_with(
    pkg: &Package,
    model: &SemanticModel,
    limits: &DataModelLimits,
) -> Result<Package, Error> {
    if let Some(part) = pkg.get(DATA_MODEL_SCHEMA) {
        let (_, encoding) = decode_text(&part.name, &part.data)?;
        let data = encode_text(&bicraft_tmsl::write(model), encoding);
        let (name, stored) = (part.name.clone(), part.stored);
        let mut out = pkg.clone();
        out.put(&name, data, stored);
        return Ok(out);
    }
    let part = pkg.get(DATA_MODEL).ok_or(Error::NoModel)?;
    if part.len() > bicraft_opc::MAX_PART_BYTES {
        return Err(Error::DataModelTooLargeToWrite(part.len()));
    }
    // a part left in the file by choice (under a memory budget) is read for the write
    let bytes = part.bytes()?;
    let data = write_data_model_with(&bytes, model, limits)?;
    let name = part.name.clone();
    let mut out = pkg.clone();
    out.put(&name, data, true);
    Ok(out)
}

/// Writes `model` into a PBIX `DataModel` part (or a PBIP `cache.abf`): decompress → parse the
/// ABF → rewrite `metadata.sqlitedb` (`bicraft_metadata::write_sqlitedb`) → re-pack the ABF
/// (only that file changes) → re-compress in the single-threaded XPress9 framing, following
/// the original part's banner, encoder parameters and chunk size
/// (`bicraft_xpress9::compress_datamodel`). A part that was a bare ABF stays one.
///
/// The result is decoded again and compared with the written ABF before it is returned
/// (`bicraft_xpress9::verify_datamodel`, in parallel). When the ABF comes out byte-identical,
/// `part` is returned unchanged.
pub fn write_data_model(part: &[u8], model: &SemanticModel) -> Result<Vec<u8>, Error> {
    write_data_model_with(part, model, &DataModelLimits::default())
}

/// [`write_data_model`] with explicit [`DataModelLimits`] for decompressing `part`.
pub fn write_data_model_with(
    part: &[u8],
    model: &SemanticModel,
    limits: &DataModelLimits,
) -> Result<Vec<u8>, Error> {
    let info = bicraft_xpress9::inspect_datamodel(part)?;
    let abf_bytes = decompress_data_model(part, limits)?;
    let abf = bicraft_abf::Abf::parse(&abf_bytes)?;
    let sqlitedb = abf.metadata_sqlitedb().ok_or(Error::NoMetadata)?;
    let new_sqlitedb = bicraft_metadata::write_sqlitedb(sqlitedb, model).map_err(metadata_error)?;
    let mut writer = abf.to_writer()?;
    if new_sqlitedb != sqlitedb {
        writer.replace_metadata_sqlitedb(new_sqlitedb)?;
    }
    let new_abf = writer.finish()?;
    if new_abf == abf_bytes {
        // the backup did not change: the part already decodes to it, keep its bytes
        return Ok(part.to_vec());
    }
    if info.layout == bicraft_xpress9::Layout::Uncompressed {
        return Ok(new_abf);
    }
    let out = bicraft_xpress9::compress_datamodel(&new_abf, Some(&info))?;
    // decodes the new part in parallel against the backup (bicraft_xpress9::verify_datamodel)
    if !bicraft_xpress9::verify_datamodel(&out, &new_abf)? {
        return Err(Error::Verify(
            "the compressed DataModel does not decode to the written backup",
        ));
    }
    Ok(out)
}

/// Reads the report of a PBIX or PBIT. PBIR parts win over a legacy `Report/Layout`, like
/// Desktop does. For a legacy layout, the `Report/StaticResources/` parts are attached as
/// resources when the layout reader did not provide any. In both forms the embedded custom
/// visuals (`Report/CustomVisuals/…`) are attached as resources, byte for byte; saving keeps
/// their parts as they are.
pub fn read_report(pkg: &Package) -> Result<Report, Error> {
    if pkg.parts.iter().any(|p| p.name.starts_with(PBIR_PREFIX)) {
        let docs: Vec<(String, Vec<u8>)> = pkg
            .parts
            .iter()
            .filter(|p| {
                p.name.starts_with(PBIR_PREFIX)
                    || p.name.starts_with(STATIC_RESOURCES_PREFIX)
                    || p.name.starts_with(CUSTOM_VISUALS_PREFIX)
            })
            .filter_map(|p| {
                p.name
                    .strip_prefix(REPORT_PREFIX)
                    .map(|rel| (rel.to_string(), p.data.clone()))
            })
            .collect();
        return Ok(bicraft_pbir::read_documents(&docs)?);
    }
    let Some(part) = pkg.get(LEGACY_LAYOUT) else {
        return Err(Error::NoReport);
    };
    let (text, _) = decode_text(&part.name, &part.data)?;
    let mut report = bicraft_layout::read_layout(&text)?;
    if report.resources.is_empty() {
        report.resources = pkg
            .parts
            .iter()
            .filter(|p| p.name.starts_with(STATIC_RESOURCES_PREFIX))
            .filter_map(|p| {
                p.name.strip_prefix(REPORT_PREFIX).map(|rel| Resource {
                    path: rel.to_string(),
                    bytes: p.data.clone(),
                })
            })
            .collect();
    }
    let known: std::collections::HashSet<String> =
        report.resources.iter().map(|r| r.path.to_lowercase()).collect();
    let visuals: Vec<Resource> = pkg
        .parts
        .iter()
        .filter(|p| p.name.starts_with(CUSTOM_VISUALS_PREFIX))
        .filter_map(|p| p.name.strip_prefix(REPORT_PREFIX).map(|rel| (rel, p)))
        .filter(|(rel, _)| !known.contains(&rel.to_lowercase()))
        .map(|(rel, p)| Resource {
            path: rel.to_string(),
            bytes: p.data.clone(),
        })
        .collect();
    report.resources.extend(visuals);
    Ok(report)
}

pub mod fresh;
pub mod large;
pub mod pbiviz;
pub mod storage;
pub mod structural;

mod report;
pub use large::{
    MAX_STREAMED_FILE_BYTES, backup_index_at, read_data_model_bytes_streamed, read_data_model_streamed,
};
pub use report::{CONTENT_TYPES, ReportWrite, SECURITY_BINDINGS, report_format, write_report};

#[doc(hidden)]
pub mod synthetic;

#[cfg(test)]
mod tests {
    use super::*;
    use bicraft_opc::encode_text;

    fn pbit() -> Package {
        let schema =
            r#"{"name":"m","model":{"tables":[{"name":"T","measures":[{"name":"M","expression":"1"}]}]}}"#;
        let mut pkg = Package::default();
        pkg.put("Version", encode_text("1.28", Encoding::Utf16Le), false);
        pkg.put(DATA_MODEL_SCHEMA, encode_text(schema, Encoding::Utf16Le), false);
        pkg.put(LEGACY_LAYOUT, encode_text("{}", Encoding::Utf16Le), false);
        pkg.put("Report/StaticResources/x.png", vec![0x89, b'P', b'N', b'G'], true);
        pkg
    }

    #[test]
    fn inventories_a_template() {
        let inv = inventory(&pbit());
        assert_eq!(inv.kind, Kind::Pbit);
        assert_eq!(inv.report_format, ReportFormat::Legacy);
        assert_eq!(inv.parts[0].encoding, Some("utf-16le"));
        assert_eq!(inv.parts[3].encoding, None);
    }

    #[test]
    fn reads_the_template_model() {
        let m = read_model(&pbit()).unwrap();
        assert_eq!(m.measure("M").map(|(t, _)| t.name.as_str()), Some("T"));
    }

    #[test]
    fn binary_models_and_thin_reports_are_explained() {
        let mut pkg = Package::default();
        assert!(matches!(read_model(&pkg), Err(Error::NoModel)));
        pkg.put(DATA_MODEL, vec![0; 16], true);
        assert!(matches!(read_model(&pkg), Err(Error::Xpress9(_) | Error::Abf(_))));
        assert_eq!(inventory(&pkg).kind, Kind::Pbix);
    }

    #[test]
    fn a_thin_reports_live_connection_is_read_without_the_rest_of_its_string() {
        let mut pkg = Package::default();
        assert_eq!(live_connection(&pkg), None);
        let conn = r#"{"Version":4,"Connections":[{"Name":"EntityDataSource","ConnectionString":"Data Source=powerbi://api.powerbi.com/v1.0/myorg/Finance;Initial Catalog=\"Sales; Model\";Password=secret","ConnectionType":"pbiServiceXmlaStyleLive"}]}"#;
        pkg.put(CONNECTIONS, conn.as_bytes().to_vec(), true);
        assert_eq!(inventory(&pkg).kind, Kind::Thin);
        assert!(matches!(read_model(&pkg), Err(Error::NoModel)));
        let live = live_connection(&pkg).unwrap();
        assert_eq!(
            live,
            LiveConnection {
                connection_type: Some("pbiServiceXmlaStyleLive".into()),
                data_source: Some("powerbi://api.powerbi.com/v1.0/myorg/Finance".into()),
                catalog: Some("Sales; Model".into()),
            }
        );
        assert!(!serde_json::to_string(&live).unwrap().contains("secret"));
        assert_eq!(connection_value("a='x''y';b=2", "A").as_deref(), Some("x'y"));
        assert_eq!(connection_value("a=1; B = 2 ", "b").as_deref(), Some("2"));
        assert_eq!(
            connection_value("a=\"unterminated", "a").as_deref(),
            Some("unterminated")
        );
        assert_eq!(connection_value("junk", "a"), None);
        // a part that is not JSON gives no connection
        pkg.put(CONNECTIONS, b"\xff\xfe{".to_vec(), true);
        assert_eq!(live_connection(&pkg), None);
    }

    #[test]
    fn report_errors_are_explained() {
        assert!(matches!(read_report(&Package::default()), Err(Error::NoReport)));
        // A layout that is not valid UTF-16LE text.
        let mut pkg = Package::default();
        pkg.put(LEGACY_LAYOUT, vec![0x7B, 0x00, 0x7D], false);
        assert!(read_report(&pkg).is_err());
    }

    /// PBIR parts are handed to the PBIR reader relative to `Report/`.
    #[test]
    fn reads_a_pbir_report() {
        let mut pkg = pbit();
        let json = |v: &str| encode_text(v, Encoding::Utf8);
        pkg.put(
            "Report/definition/version.json",
            json(r#"{"$schema":"https://developer.microsoft.com/json-schemas/fabric/item/report/definition/versionMetadata/1.0.0/schema.json","version":"2.0.0"}"#),
            false,
        );
        pkg.put(
            "Report/definition/report.json",
            json(r#"{"$schema":"https://developer.microsoft.com/json-schemas/fabric/item/report/definition/report/3.3.0/schema.json","themeCollection":{}}"#),
            false,
        );
        pkg.put(
            "Report/definition/pages/pages.json",
            json(r#"{"$schema":"https://developer.microsoft.com/json-schemas/fabric/item/report/definition/pagesMetadata/1.1.0/schema.json","pageOrder":["p1"],"activePageName":"p1"}"#),
            false,
        );
        pkg.put(
            "Report/definition/pages/p1/page.json",
            json(r#"{"$schema":"https://developer.microsoft.com/json-schemas/fabric/item/report/definition/page/2.1.0/schema.json","name":"p1","displayName":"Overview","displayOption":"FitToPage","height":720,"width":1280}"#),
            false,
        );
        let report = read_report(&pkg).unwrap();
        assert_eq!(report.pages.len(), 1);
        assert_eq!(report.pages[0].display_name, "Overview");
        assert_eq!(report.active_page.as_deref(), Some("p1"));
        assert!(report.resources.iter().any(|r| r.path == "StaticResources/x.png"));
    }

    #[test]
    fn reads_a_legacy_layout() {
        let mut pkg = pbit();
        let layout = r#"{"config":"{}","layoutOptimization":0,"sections":[{"name":"ReportSection","displayName":"Page 1","width":1280,"height":720,"displayOption":1,"visualContainers":[],"config":"{}","filters":"[]"}]}"#;
        pkg.put(LEGACY_LAYOUT, encode_text(layout, Encoding::Utf16Le), false);
        let report = read_report(&pkg).unwrap();
        assert_eq!(report.pages.len(), 1);
        assert!(report.resources.iter().any(|r| r.path == "StaticResources/x.png"));
    }
}
