//! Scaffolding a new PBIP project and saving a model back into one. The file set, `$schema`
//! URIs and `version` values are documented in `docs/formats/pbip.md`.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use bicraft_model::SemanticModel;
use bicraft_report::{Page, Report};
use serde_json::{Value, json};

use crate::{Error, ModelFormat, Project, locate, read_capped};

// `$schema` URIs and `version` values: the single table in `bicraft-versions`.
use bicraft_versions::{
    PAGE as PAGE_SCHEMA, PAGES_METADATA as PAGES_SCHEMA, PBIP as PBIP_SCHEMA, PBIP_VERSION,
    PBIR as PBIR_SCHEMA, PBIR_DEFINITION_VERSION, PBIR_VERSION, PBISM as PBISM_SCHEMA, PBISM_VERSION,
    PLATFORM as PLATFORM_SCHEMA, PLATFORM_CONFIG_VERSION, REPORT as REPORT_SCHEMA, Schema,
    VERSION_METADATA as VERSION_SCHEMA,
};

/// Desktop's two local files, and the data snapshot `bicraft refresh` writes next to them.
const GITIGNORE_LINES: [&str; 3] = [
    "**/.pbi/localSettings.json",
    "**/.pbi/cache.abf",
    "**/.pbi/bicraft-data.bin",
];

/// Longest accepted project name, in bytes. Keeps `<name>.SemanticModel/definition/tables/…`
/// comfortably under Windows' legacy 260-character path limit for typical parents.
const MAX_NAME_BYTES: usize = 128;
/// PBIR page names are limited to 50 characters by the page schema.
const MAX_PAGE_NAME: usize = 50;

/// The generated identifiers of a new project. [`create_project`] draws them at random;
/// [`create_project_with_ids`] takes them as given, which makes the output byte-stable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectIds {
    /// `config.logicalId` of `<name>.SemanticModel/.platform` (a GUID).
    pub semantic_model_logical_id: String,
    /// `config.logicalId` of `<name>.Report/.platform` (a GUID).
    pub report_logical_id: String,
    /// Name (and folder) of the blank report page: 1–50 characters of `[A-Za-z0-9_-]`.
    pub page_name: String,
}

impl ProjectIds {
    /// Fresh random ids: two version-4 GUIDs and a 20-hex-digit page name, like Desktop's.
    pub fn random() -> Self {
        let page = random_bytes();
        Self {
            semantic_model_logical_id: guid_v4(random_bytes()),
            report_logical_id: guid_v4(random_bytes()),
            page_name: hex(page.get(..10).unwrap_or_default()),
        }
    }

    fn validate(&self) -> Result<(), Error> {
        for id in [&self.semantic_model_logical_id, &self.report_logical_id] {
            if !is_guid(id) {
                return Err(Error::InvalidId {
                    id: id.clone(),
                    reason: "a logical id must be a GUID like 01234567-89ab-cdef-0123-456789abcdef",
                });
            }
        }
        let page = &self.page_name;
        if page.is_empty()
            || page.len() > MAX_PAGE_NAME
            || !page
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(Error::InvalidId {
                id: page.clone(),
                reason: "a page name must be 1-50 characters of A-Z, a-z, 0-9, `_` or `-`",
            });
        }
        Ok(())
    }
}

/// Creates a new PBIP project in `parent`: `<name>.pbip`, `<name>.SemanticModel/` (model written
/// in `format`), a blank PBIR `<name>.Report/` with one empty page, and a `.gitignore` for the
/// local `.pbi` files. Refuses to overwrite anything that already exists.
pub fn create_project(
    parent: &Path,
    name: &str,
    model: &SemanticModel,
    format: ModelFormat,
) -> Result<Project, Error> {
    create_project_with_ids(parent, name, model, format, &ProjectIds::random())
}

/// [`create_project`] with caller-chosen ids, so the output is fully deterministic.
pub fn create_project_with_ids(
    parent: &Path,
    name: &str,
    model: &SemanticModel,
    format: ModelFormat,
    ids: &ProjectIds,
) -> Result<Project, Error> {
    validate_project_name(name)?;
    ids.validate()?;
    if format == ModelFormat::Missing {
        return Err(Error::NoFormat);
    }
    std::fs::create_dir_all(parent).map_err(|e| Error::Io(parent.into(), e))?;
    ensure_free(parent, name)?;

    let pbip = parent.join(format!("{name}.pbip"));
    let model_dir = parent.join(format!("{name}.SemanticModel"));
    let report_dir = parent.join(format!("{name}.Report"));
    let mut created = Created::default();

    created.dir(&model_dir)?;
    created.dir(&report_dir)?;
    match format {
        ModelFormat::Tmdl => {
            let definition = model_dir.join("definition");
            created.dir(&definition)?;
            bicraft_tmdl::write_folder(model, &definition)?;
        }
        _ => write_new(
            &model_dir.join("model.bim"),
            &normalize(&bicraft_tmsl::write(model)),
        )?,
    }
    for (relative, value) in metadata_files(name, ids) {
        let path = parent.join(&relative);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::Io(dir.into(), e))?;
        }
        write_new(&path, &json_text(&path, &value)?)?;
    }
    let pbip_text = json_text(&pbip, &pbip_value(name))?;
    write_new(&pbip, &pbip_text)?;
    created.file(&pbip);
    ensure_gitignore(parent)?;
    created.keep();
    locate(&pbip)
}

/// Writes `model` back into the project's semantic model folder in the project's model format
/// (`model.bim` or the TMDL `definition/` folder). The report folder is never touched.
pub fn save_model(project: &Project, model: &SemanticModel) -> Result<(), Error> {
    let Some(dir) = &project.semantic_model else {
        return Err(Error::NoModel(project.root.clone()));
    };
    match project.model_format {
        ModelFormat::Tmsl => replace_file(&dir.join("model.bim"), &normalize(&bicraft_tmsl::write(model))),
        ModelFormat::Tmdl => Ok(bicraft_tmdl::write_folder(model, &dir.join("definition"))?),
        ModelFormat::Missing => Err(Error::NoModel(project.root.clone())),
    }
}

/// Checks that `name` can be used as `<name>.pbip`, `<name>.SemanticModel` and `<name>.Report`
/// on both Windows and Linux.
pub fn validate_project_name(name: &str) -> Result<(), Error> {
    let fail = |reason| {
        Err(Error::InvalidName {
            name: name.to_owned(),
            reason,
        })
    };
    if name.trim().is_empty() {
        return fail("the name is empty");
    }
    if name.len() > MAX_NAME_BYTES {
        return fail("the name is longer than 128 bytes");
    }
    if name.contains("..") {
        return fail("the name contains `..`");
    }
    if let Some(c) = name
        .chars()
        .find(|c| c.is_control() || "<>:\"/\\|?*".contains(*c))
    {
        return if c == '/' || c == '\\' {
            fail("the name contains a path separator")
        } else {
            fail("the name contains a character Windows does not allow in file names")
        };
    }
    if name.ends_with('.') || name.ends_with(' ') || name.starts_with(' ') {
        return fail("the name starts with a space or ends with a space or a dot");
    }
    let device = name
        .split('.')
        .next()
        .unwrap_or(name)
        .trim_end()
        .to_ascii_uppercase();
    let reserved = matches!(device.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((device.starts_with("COM") || device.starts_with("LPT"))
            && device.len() == 4
            && device
                .as_bytes()
                .get(3)
                .is_some_and(|d| (b'1'..=b'9').contains(d)));
    if reserved {
        return fail("the name is a reserved Windows device name");
    }
    Ok(())
}

/// Every metadata file of a new project as `(path relative to the parent, JSON)`. The model
/// itself and the `.pbip` file are written separately.
fn metadata_files(name: &str, ids: &ProjectIds) -> Vec<(String, Value)> {
    let model_dir = format!("{name}.SemanticModel");
    let report_dir = format!("{name}.Report");
    let page = &ids.page_name;
    vec![
        (
            format!("{model_dir}/definition.pbism"),
            json!({
                "$schema": schema(PBISM_SCHEMA),
                "version": PBISM_VERSION,
                "settings": {},
            }),
        ),
        (
            format!("{model_dir}/.platform"),
            platform("SemanticModel", name, &ids.semantic_model_logical_id),
        ),
        (
            format!("{report_dir}/definition.pbir"),
            json!({
                "$schema": schema(PBIR_SCHEMA),
                "version": PBIR_VERSION,
                "datasetReference": { "byPath": { "path": format!("../{model_dir}") } },
            }),
        ),
        (
            format!("{report_dir}/.platform"),
            platform("Report", name, &ids.report_logical_id),
        ),
        (
            format!("{report_dir}/definition/version.json"),
            json!({ "$schema": schema(VERSION_SCHEMA), "version": PBIR_DEFINITION_VERSION }),
        ),
        (
            format!("{report_dir}/definition/report.json"),
            json!({
                "$schema": schema(REPORT_SCHEMA),
                // No base theme: Desktop's built-in theme files are Microsoft assets we do not
                // ship, so the report starts on the default theme.
                "themeCollection": {},
                "settings": new_report_settings(),
            }),
        ),
        (
            format!("{report_dir}/definition/pages/pages.json"),
            json!({
                "$schema": schema(PAGES_SCHEMA),
                "pageOrder": [page],
                "activePageName": page,
            }),
        ),
        (
            format!("{report_dir}/definition/pages/{page}/page.json"),
            json!({
                "$schema": schema(PAGE_SCHEMA),
                "name": page,
                "displayName": BLANK_PAGE_DISPLAY_NAME,
                "displayOption": "FitToPage",
                "height": 720,
                "width": 1280,
            }),
        ),
    ]
}

const BLANK_PAGE_DISPLAY_NAME: &str = "Page 1";

/// `report.json` `settings` as Desktop writes them for a new report.
fn new_report_settings() -> Value {
    json!({
        "useStylableVisualContainerHeader": true,
        "exportDataMode": "AllowSummarized",
        "defaultDrillFilterOtherVisuals": true,
        "allowChangeFilterTypes": true,
        "useEnhancedTooltips": true,
        "useDefaultAggregateDisplayName": true,
    })
}

/// The blank report [`create_project`] writes (one empty 1280 × 720 page named `page_name`), as
/// the in-memory report. `$schema` URLs and versions are left to the PBIR writer.
pub fn blank_report(page_name: &str) -> Report {
    let mut extra = serde_json::Map::new();
    extra.insert("settings".into(), new_report_settings());
    Report {
        pages: vec![Page {
            name: page_name.to_string(),
            display_name: BLANK_PAGE_DISPLAY_NAME.to_string(),
            display_option: Some("FitToPage".into()),
            width: 1280.0,
            height: 720.0,
            ..Default::default()
        }],
        active_page: Some(page_name.to_string()),
        theme_collection: Some(json!({})),
        extra,
        ..Default::default()
    }
}

fn pbip_value(name: &str) -> Value {
    json!({
        "$schema": schema(PBIP_SCHEMA),
        "version": PBIP_VERSION,
        "artifacts": [{ "report": { "path": format!("{name}.Report") } }],
        "settings": { "enableAutoRecovery": true },
    })
}

fn platform(kind: &str, display_name: &str, logical_id: &str) -> Value {
    json!({
        "$schema": schema(PLATFORM_SCHEMA),
        "metadata": { "type": kind, "displayName": display_name },
        "config": { "version": PLATFORM_CONFIG_VERSION, "logicalId": logical_id },
    })
}

fn schema(schema: Schema) -> String {
    schema.url()
}

/// Indented JSON (two spaces, as Desktop writes it), LF line endings, no trailing newline.
pub(crate) fn json_text(path: &Path, value: &Value) -> Result<String, Error> {
    serde_json::to_string_pretty(value).map_err(|e| Error::Json(path.into(), e))
}

/// UTF-8 without BOM and with LF line endings. JSON strings cannot hold raw CR/LF, so this
/// never changes a value.
fn normalize(text: &str) -> String {
    text.strip_prefix('\u{feff}')
        .unwrap_or(text)
        .replace("\r\n", "\n")
}

/// Fails with [`Error::AlreadyExists`] when `parent` already holds an item of this project.
/// Names are compared case-insensitively so a project made on Linux still opens on Windows.
fn ensure_free(parent: &Path, name: &str) -> Result<(), Error> {
    let taken = [".pbip", ".SemanticModel", ".Dataset", ".Report"].map(|s| format!("{name}{s}"));
    let entries = std::fs::read_dir(parent).map_err(|e| Error::Io(parent.into(), e))?;
    for entry in entries {
        let entry = entry.map_err(|e| Error::Io(parent.into(), e))?;
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        if taken.iter().any(|t| t.to_lowercase() == file_name.to_lowercase()) {
            return Err(Error::AlreadyExists(entry.path()));
        }
    }
    Ok(())
}

/// Creates `path`, failing if it exists.
fn write_new(path: &Path, text: &str) -> Result<(), Error> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| Error::Io(path.into(), e))?;
    file.write_all(text.as_bytes())
        .map_err(|e| Error::Io(path.into(), e))
}

/// Replaces `path` through a sibling temporary file, so a failed write leaves the old file.
pub(crate) fn replace_file(path: &Path, text: &str) -> Result<(), Error> {
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temp = path.with_file_name(format!(".{file_name}.bicraft-tmp"));
    let _ = std::fs::remove_file(&temp);
    let result = write_new(&temp, text)
        .and_then(|()| std::fs::rename(&temp, path).map_err(|e| Error::Io(path.into(), e)));
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

/// Creates `.gitignore` with the local-file patterns, or appends the missing ones.
fn ensure_gitignore(parent: &Path) -> Result<(), Error> {
    let path = parent.join(".gitignore");
    if !path.exists() {
        let text: String = GITIGNORE_LINES.iter().map(|l| format!("{l}\n")).collect();
        return write_new(&path, &text);
    }
    let bytes = read_capped(&path)?;
    let existing = String::from_utf8_lossy(&bytes);
    let missing: Vec<&str> = GITIGNORE_LINES
        .iter()
        .copied()
        .filter(|l| !existing.lines().any(|e| e.trim() == *l))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    let mut text = String::new();
    if !existing.is_empty() && !existing.ends_with('\n') {
        text.push('\n');
    }
    for line in missing {
        text.push_str(line);
        text.push('\n');
    }
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .map_err(|e| Error::Io(path.clone(), e))?;
    file.write_all(text.as_bytes()).map_err(|e| Error::Io(path, e))
}

/// What `create_project` made so far; removed again unless [`Created::keep`] is called. Only
/// paths this call created are listed, so nothing pre-existing is ever deleted.
#[derive(Default)]
struct Created {
    dirs: Vec<PathBuf>,
    files: Vec<PathBuf>,
    keep: bool,
}

impl Created {
    fn dir(&mut self, path: &Path) -> Result<(), Error> {
        std::fs::create_dir(path).map_err(|e| Error::Io(path.into(), e))?;
        self.dirs.push(path.into());
        Ok(())
    }

    fn file(&mut self, path: &Path) {
        self.files.push(path.into());
    }

    fn keep(mut self) {
        self.keep = true;
    }
}

impl Drop for Created {
    fn drop(&mut self) {
        if self.keep {
            return;
        }
        for file in &self.files {
            let _ = std::fs::remove_file(file);
        }
        for dir in self.dirs.iter().rev() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// 16 bytes for ids. Not cryptographic: it only has to make collisions practically impossible.
/// Each `RandomState` is keyed from OS entropy (per thread, then stepped), mixed with the clock,
/// the process id and a counter.
fn random_bytes() -> [u8; 16] {
    use std::hash::{BuildHasher as _, Hasher as _};
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let mut out = [0u8; 16];
    for (i, chunk) in out.chunks_mut(8).enumerate() {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u128(nanos);
        hasher.write_u64(count);
        hasher.write_u32(std::process::id());
        hasher.write_usize(i);
        chunk.copy_from_slice(&hasher.finish().to_le_bytes());
    }
    out
}

fn guid_v4(mut bytes: [u8; 16]) -> String {
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let h = hex(&bytes);
    let part = |a: usize, b: usize| h.get(a..b).unwrap_or_default();
    format!(
        "{}-{}-{}-{}-{}",
        part(0, 8),
        part(8, 12),
        part(12, 16),
        part(16, 20),
        part(20, 32)
    )
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn is_guid(text: &str) -> bool {
    text.len() == 36
        && text.char_indices().all(|(i, c)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                c == '-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}

#[cfg(test)]
mod tests;
