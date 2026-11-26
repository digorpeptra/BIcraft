//! Project files beyond the model and the PBIR report, for conversions to and from packages:
//!
//! - `<Name>.SemanticModel/.pbi/cache.abf`: Desktop's local data cache. It has the framing of a
//!   PBIX `DataModel` part (an XPress9-compressed ABF backup, see `docs/formats/pbip.md`,
//!   "Data cache"); this module only moves its bytes.
//! - `<Name>.SemanticModel/diagramLayout.json`: the model diagram (the PBIX `DiagramLayout`).
//! - a legacy report: `<Name>.Report/report.json` and the raw files under `StaticResources/` and
//!   `CustomVisuals/` next to it.

use std::path::{Component, Path, PathBuf};

use crate::create::replace_file;
use crate::{Error, Project, ReportFormat, locate, read_capped};

/// The data cache, relative to the semantic model folder.
pub const CACHE_ABF: &str = ".pbi/cache.abf";
/// The model diagram, relative to the semantic model folder.
pub const DIAGRAM_LAYOUT: &str = "diagramLayout.json";
/// Largest `cache.abf` read (the same cap as a PBIX `DataModel` part read into memory).
pub const MAX_CACHE_BYTES: u64 = 4 << 30;
/// Report folders whose files are carried as raw resources.
pub const RESOURCE_FOLDERS: [&str; 2] = ["StaticResources", "CustomVisuals"];
/// Semantic model folders holding the Copilot container (`Copilot/`: AI instructions, AI data
/// schema, example prompts, settings, verified answers) and the older root-level verified
/// answers (`docs/formats/pbix-package.md` §2, "Copilot containers"). Their files are moved as
/// they are; text files are UTF-8 without BOM.
pub const COPILOT_FOLDERS: [&str; 2] = ["Copilot", "VerifiedAnswers"];
/// Caps on the raw report files read by [`read_report_files`].
const MAX_RESOURCE_FILES: usize = 10_000;
const MAX_RESOURCE_BYTES: u64 = 2 << 30;
const MAX_RESOURCE_DEPTH: usize = 16;

/// Where the project's `cache.abf` is (or would be).
pub fn cache_abf_path(project: &Project) -> Option<PathBuf> {
    project
        .semantic_model
        .as_ref()
        .map(|d| d.join(".pbi").join("cache.abf"))
}

/// The bytes of the project's `.pbi/cache.abf`, or `None` when it has none.
pub fn read_cache_abf(project: &Project) -> Result<Option<Vec<u8>>, Error> {
    let Some(path) = cache_abf_path(project).filter(|p| p.is_file()) else {
        return Ok(None);
    };
    let meta = std::fs::metadata(&path).map_err(|e| Error::Io(path.clone(), e))?;
    if meta.len() > MAX_CACHE_BYTES {
        return Err(Error::TooLarge(path));
    }
    std::fs::read(&path).map(Some).map_err(|e| Error::Io(path, e))
}

/// Writes `bytes` as the project's `.pbi/cache.abf` (through a temporary sibling and a rename).
pub fn write_cache_abf(project: &Project, bytes: &[u8]) -> Result<(), Error> {
    let path = cache_abf_path(project).ok_or_else(|| Error::NoModel(project.root.clone()))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| Error::Io(dir.into(), e))?;
    }
    write_bytes(&path, bytes)
}

/// The project's `diagramLayout.json` text, or `None` when it has none.
pub fn read_diagram_layout(project: &Project) -> Result<Option<String>, Error> {
    let Some(path) = project
        .semantic_model
        .as_ref()
        .map(|d| d.join(DIAGRAM_LAYOUT))
        .filter(|p| p.is_file())
    else {
        return Ok(None);
    };
    read_text(&path).map(Some)
}

/// Writes `text` (JSON) as the project's `diagramLayout.json`, UTF-8 without BOM.
pub fn write_diagram_layout(project: &Project, text: &str) -> Result<(), Error> {
    let dir = project
        .semantic_model
        .as_ref()
        .ok_or_else(|| Error::NoModel(project.root.clone()))?;
    let path = dir.join(DIAGRAM_LAYOUT);
    check_json(&path, text)?;
    replace_file(&path, text.strip_prefix('\u{feff}').unwrap_or(text))
}

/// The text of a legacy project's `report.json`, or `None` when the report is not legacy.
pub fn read_legacy_layout(project: &Project) -> Result<Option<String>, Error> {
    match (&project.report, project.report_format) {
        (Some(dir), ReportFormat::Legacy) => read_text(&dir.join("report.json")).map(Some),
        _ => Ok(None),
    }
}

/// The raw files under the report folder's `StaticResources/` and `CustomVisuals/`, as
/// `(path relative to the report folder, bytes)` with `/` separators, sorted by path. Bounded:
/// at most 10 000 files, 2 GiB in all, 16 folder levels.
pub fn read_report_files(project: &Project) -> Result<Vec<(String, Vec<u8>)>, Error> {
    let Some(dir) = &project.report else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    let mut total = 0u64;
    for top in RESOURCE_FOLDERS {
        let root = dir.join(top);
        if root.is_dir() {
            walk(&root, top, 0, &mut out, &mut total)?;
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// The files under the semantic model folder's `Copilot/` and `VerifiedAnswers/`, as
/// `(path relative to the semantic model folder, bytes)` with `/` separators, sorted by path,
/// within the same bounds as [`read_report_files`].
pub fn read_copilot_files(project: &Project) -> Result<Vec<(String, Vec<u8>)>, Error> {
    let Some(dir) = &project.semantic_model else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    let mut total = 0u64;
    for top in COPILOT_FOLDERS {
        let root = dir.join(top);
        if root.is_dir() {
            walk(&root, top, 0, &mut out, &mut total)?;
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// Writes Copilot container files (paths relative to the semantic model folder, under
/// [`COPILOT_FOLDERS`]) into the project. Every path is checked before anything is written.
pub fn write_copilot_files(project: &Project, files: &[(String, Vec<u8>)]) -> Result<(), Error> {
    let dir = project
        .semantic_model
        .as_ref()
        .ok_or_else(|| Error::NoModel(project.root.clone()))?;
    let mut targets = Vec::with_capacity(files.len());
    for (rel, bytes) in files {
        let path = relative_path(dir, rel, &COPILOT_FOLDERS)
            .map_err(|why| Error::BadDefinition(dir.join(rel), why))?;
        targets.push((path, bytes));
    }
    for (path, bytes) in targets {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::Io(parent.into(), e))?;
        }
        write_bytes(&path, bytes)?;
    }
    if files.iter().any(|(rel, _)| rel.starts_with("Copilot/")) {
        raise_pbism_version(dir)?;
    }
    Ok(())
}

/// `definition.pbism` version from which Desktop reads the `Copilot/` folder: 6.0
/// (`CopilotVersionRelease`; with 4.2 the folder is ignored, checked on Desktop 2.158).
pub const COPILOT_PBISM_VERSION: &str = bicraft_versions::PBISM_COPILOT_VERSION;

/// Raises `definition.pbism`'s `version` to [`COPILOT_PBISM_VERSION`] when it is lower, changing
/// nothing else in the file.
fn raise_pbism_version(dir: &Path) -> Result<(), Error> {
    let path = dir.join("definition.pbism");
    let text = read_text(&path)?;
    let doc: serde_json::Value = serde_json::from_str(&text).map_err(|e| Error::Json(path.clone(), e))?;
    let Some(current) = doc.get("version").and_then(serde_json::Value::as_str) else {
        return Ok(());
    };
    let number = |v: &str| -> Option<(u32, u32)> {
        let (major, minor) = v.split_once('.')?;
        Some((major.parse().ok()?, minor.parse().ok()?))
    };
    if number(current).is_none_or(|v| v >= (6, 0)) {
        return Ok(());
    }
    let needle = serde_json::to_string(current).map_err(|e| Error::Json(path.clone(), e))?;
    let Some(at) = text
        .find(&format!("\"version\": {needle}"))
        .map(|i| i + "\"version\": ".len())
    else {
        return Err(Error::BadDefinition(
            path,
            "unexpected layout of the version property",
        ));
    };
    let mut out = String::with_capacity(text.len());
    out.push_str(text.get(..at).unwrap_or_default());
    out.push_str(&format!("\"{COPILOT_PBISM_VERSION}\""));
    out.push_str(text.get(at + needle.len()..).unwrap_or_default());
    replace_file(&path, &out)
}

fn walk(
    dir: &Path,
    rel: &str,
    depth: usize,
    out: &mut Vec<(String, Vec<u8>)>,
    total: &mut u64,
) -> Result<(), Error> {
    if depth > MAX_RESOURCE_DEPTH {
        return Err(Error::BadDefinition(
            dir.into(),
            "resource folders are nested too deeply",
        ));
    }
    let entries = std::fs::read_dir(dir).map_err(|e| Error::Io(dir.into(), e))?;
    for entry in entries {
        let entry = entry.map_err(|e| Error::Io(dir.into(), e))?;
        let path = entry.path();
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            return Err(Error::BadDefinition(
                path,
                "a resource file name is not valid Unicode",
            ));
        };
        let child = format!("{rel}/{name}");
        let kind = entry.file_type().map_err(|e| Error::Io(path.clone(), e))?;
        if kind.is_dir() {
            walk(&path, &child, depth.saturating_add(1), out, total)?;
        } else if kind.is_file() {
            if out.len() >= MAX_RESOURCE_FILES {
                return Err(Error::BadDefinition(dir.into(), "too many resource files"));
            }
            let bytes = read_capped(&path)?;
            *total = total.saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
            if *total > MAX_RESOURCE_BYTES {
                return Err(Error::TooLarge(path));
            }
            out.push((child, bytes));
        }
        // Symbolic links and other special files are skipped.
    }
    Ok(())
}

/// Replaces the report of a project just made by `create_project` (a blank PBIR report) with a
/// legacy one: writes `report.json` (UTF-8 without BOM) and the raw `resources` (paths relative
/// to the report folder, under `StaticResources/` or `CustomVisuals/`), then removes the PBIR
/// `definition/` folder, since Desktop refuses a report folder holding both forms.
/// `definition.pbir` is left as written (version 4.0): with no `definition/` folder, Desktop reads
/// `report.json`. Returns the project as located again.
pub fn write_legacy_report(
    project: &Project,
    layout: &str,
    resources: &[(String, Vec<u8>)],
) -> Result<Project, Error> {
    let dir = project
        .report
        .as_ref()
        .ok_or_else(|| Error::NoReport(project.root.clone()))?;
    let report_json = dir.join("report.json");
    check_json(&report_json, layout)?;
    // Validate every path before writing anything.
    let mut targets = Vec::with_capacity(resources.len());
    for (rel, bytes) in resources {
        targets.push((resource_path(dir, rel)?, bytes));
    }
    for (path, bytes) in targets {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::Io(parent.into(), e))?;
        }
        write_bytes(&path, bytes)?;
    }
    replace_file(&report_json, layout.strip_prefix('\u{feff}').unwrap_or(layout))?;
    let definition = dir.join("definition");
    if definition.is_dir() {
        std::fs::remove_dir_all(&definition).map_err(|e| Error::Io(definition, e))?;
    }
    let anchor = project.semantic_model.clone().unwrap_or_else(|| dir.clone());
    let mut again = locate(&anchor)?;
    if again.report.as_deref() != Some(dir.as_path()) {
        again.report = Some(dir.clone());
        again.report_format = ReportFormat::Legacy;
    }
    Ok(again)
}

/// `dir/rel` for a resource path: `/`-separated, under one of [`RESOURCE_FOLDERS`], with plain
/// components only.
fn resource_path(dir: &Path, rel: &str) -> Result<PathBuf, Error> {
    relative_path(dir, rel, &RESOURCE_FOLDERS).map_err(|why| {
        let why = if why == OUTSIDE {
            "a report resource must be under StaticResources/ or CustomVisuals/"
        } else {
            why
        };
        Error::BadDefinition(dir.join(rel), why)
    })
}

const OUTSIDE: &str = "the path is outside the folders it may be in";

/// `dir/rel` for a `/`-separated path under one of `tops`, with plain components only.
fn relative_path(dir: &Path, rel: &str, tops: &[&str]) -> Result<PathBuf, &'static str> {
    let bad = |why| Err(why);
    if !rel.split('/').next().is_some_and(|top| tops.contains(&top)) {
        return bad(OUTSIDE);
    }
    let mut path = dir.to_path_buf();
    for (i, part) in rel.split('/').enumerate() {
        let plain = matches!(
            Path::new(part).components().collect::<Vec<_>>().as_slice(),
            [Component::Normal(_)]
        );
        if part.is_empty()
            || !plain
            || part.contains(['\\', ':'])
            || part.chars().any(char::is_control)
            || i > MAX_RESOURCE_DEPTH
        {
            return bad("invalid path");
        }
        path.push(part);
    }
    if rel.split('/').count() < 2 {
        return bad("invalid path");
    }
    Ok(path)
}

fn read_text(path: &Path) -> Result<String, Error> {
    let bytes = read_capped(path)?;
    bicraft_opc::decode_text(&path.to_string_lossy(), &bytes)
        .map(|(text, _)| text)
        .map_err(|e| Error::Text(path.into(), e))
}

fn check_json(path: &Path, text: &str) -> Result<(), Error> {
    match serde_json::from_str::<serde_json::Value>(text.strip_prefix('\u{feff}').unwrap_or(text)) {
        Ok(serde_json::Value::Object(_)) => Ok(()),
        Ok(_) => Err(Error::BadDefinition(path.into(), "not a JSON object")),
        Err(e) => Err(Error::Json(path.into(), e)),
    }
}

/// Writes `bytes` through a temporary sibling and a rename.
fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temp = path.with_file_name(format!(".{file_name}.bicraft-tmp"));
    let result = std::fs::write(&temp, bytes)
        .map_err(|e| Error::Io(temp.clone(), e))
        .and_then(|()| std::fs::rename(&temp, path).map_err(|e| Error::Io(path.into(), e)));
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tempdir::TempDir;
    use crate::{ModelFormat, create_project};
    use bicraft_model::SemanticModel;

    /// A guarded temp folder and a path inside it that does not exist yet.
    fn temp(name: &str) -> (TempDir, PathBuf) {
        let tmp = TempDir::new(&format!("pbip-extras-{name}"));
        let dir = tmp.join("out");
        (tmp, dir)
    }

    const LAYOUT: &str = r#"{"config":"{}","layoutOptimization":0,"sections":[]}"#;

    #[test]
    fn cache_and_diagram_round_trip() {
        let (_tmp, root) = temp("cache");
        let model = SemanticModel {
            name: "Shop".into(),
            ..Default::default()
        };
        let project = create_project(&root, "Shop", &model, ModelFormat::Tmsl).unwrap();
        assert_eq!(read_cache_abf(&project).unwrap(), None);
        write_cache_abf(&project, &[1, 2, 3]).unwrap();
        assert_eq!(read_cache_abf(&project).unwrap(), Some(vec![1, 2, 3]));
        assert!(root.join("Shop.SemanticModel/.pbi/cache.abf").is_file());
        assert_eq!(read_diagram_layout(&project).unwrap(), None);
        assert!(write_diagram_layout(&project, "[1]").is_err());
        write_diagram_layout(&project, r#"{"version":"1.1.0"}"#).unwrap();
        assert_eq!(
            read_diagram_layout(&project).unwrap().as_deref(),
            Some(r#"{"version":"1.1.0"}"#)
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The Copilot container round-trips byte for byte and survives a model save.
    #[test]
    fn copilot_files_round_trip_and_survive_model_saves() {
        let (_tmp, root) = temp("copilot");
        let model = SemanticModel {
            name: "Shop".into(),
            ..Default::default()
        };
        let project = create_project(&root, "Shop", &model, ModelFormat::Tmdl).unwrap();
        assert!(read_copilot_files(&project).unwrap().is_empty());
        for bad in [
            "Copilot",
            "../Copilot/x.json",
            "definition/x.tmdl",
            "Copilot/../x.json",
        ] {
            let files = vec![(bad.to_string(), vec![0])];
            assert!(write_copilot_files(&project, &files).is_err(), "{bad}");
        }
        let files = vec![
            (
                "Copilot/Instructions/instructions.md".to_string(),
                "Réponds".as_bytes().to_vec(),
            ),
            (
                "Copilot/Instructions/version.json".to_string(),
                br#"{"version":"1.0.0"}"#.to_vec(),
            ),
            (
                "Copilot/settings.json".to_string(),
                br#"{"indexingEnabled":true}"#.to_vec(),
            ),
            ("VerifiedAnswers/version.json".to_string(), b"{}".to_vec()),
        ];
        let pbism = root.join("Shop.SemanticModel/definition.pbism");
        let before = std::fs::read_to_string(&pbism).unwrap();
        assert!(before.contains("\"version\": \"4.2\""), "{before}");
        write_copilot_files(&project, &files).unwrap();
        let after = std::fs::read_to_string(&pbism).unwrap();
        assert_eq!(
            after,
            before.replace("\"version\": \"4.2\"", "\"version\": \"6.0\"")
        );
        let mut sorted = files.clone();
        sorted.sort();
        assert_eq!(read_copilot_files(&project).unwrap(), sorted);
        crate::save_model(&project, &model).unwrap();
        assert_eq!(read_copilot_files(&project).unwrap(), sorted);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn legacy_reports_replace_the_blank_pbir_one() {
        let (_tmp, root) = temp("legacy");
        let project = create_project(&root, "Shop", &SemanticModel::default(), ModelFormat::Tmdl).unwrap();
        assert_eq!(project.report_format, ReportFormat::Pbir);
        for bad in [
            "../x.png",
            "StaticResources/../x.png",
            "StaticResources",
            "definition/report.json",
            "StaticResources//x.png",
            "StaticResources/a:b",
            "/StaticResources/x.png",
        ] {
            let res = vec![(bad.to_string(), vec![0])];
            assert!(write_legacy_report(&project, LAYOUT, &res).is_err(), "{bad}");
        }
        assert!(write_legacy_report(&project, "[]", &[]).is_err());
        // Nothing was written by the refused calls.
        assert!(!root.join("Shop.Report/report.json").exists());
        assert!(root.join("Shop.Report/definition").is_dir());

        let res = vec![
            (
                "StaticResources/RegisteredResources/logo.png".to_string(),
                vec![9],
            ),
            ("CustomVisuals/v1/package.json".to_string(), b"{}".to_vec()),
        ];
        let project = write_legacy_report(&project, LAYOUT, &res).unwrap();
        assert_eq!(project.report_format, ReportFormat::Legacy);
        assert!(!root.join("Shop.Report/definition").exists());
        assert_eq!(read_legacy_layout(&project).unwrap().as_deref(), Some(LAYOUT));
        let files = read_report_files(&project).unwrap();
        let names: Vec<&str> = files.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(
            names,
            [
                "CustomVisuals/v1/package.json",
                "StaticResources/RegisteredResources/logo.png"
            ]
        );
        // The report reads back through the legacy reader.
        assert!(crate::read_report(&project).unwrap().pages.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }
}
