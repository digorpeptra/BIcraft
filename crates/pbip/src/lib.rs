//! L4: PBIP project folders. Locates the semantic model and report folders of a project and
//! reads the model, whether it is stored as TMSL (`model.bim`) or as TMDL (a `definition/`
//! folder of `.tmdl` files). Both at once is refused, as Power BI Desktop refuses it
//! (docs/formats/packaging-verification.md, P-BOTH).
//!
//! [`create_project`] scaffolds a new project (see `docs/formats/pbip.md`) and [`save_model`]
//! writes a model back in the project's existing format. [`read_report`] reads the report (PBIR
//! or legacy `report.json`) and [`save_report`] writes it back, always as PBIR.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

mod create;
mod dax_queries;
mod extras;
mod report;

use std::path::{Path, PathBuf};

use bicraft_model::SemanticModel;
use serde::Serialize;

pub use create::{
    ProjectIds, blank_report, create_project, create_project_with_ids, save_model, validate_project_name,
};
pub use dax_queries::{
    DAX_QUERIES, DAX_QUERIES_JSON, DaxQueries, DaxQuery, MAX_QUERY_BYTES, MAX_QUERY_NAME, MAX_QUERY_TABS,
    check_query_name, read_dax_queries, write_dax_queries,
};
pub use extras::{
    CACHE_ABF, COPILOT_FOLDERS, COPILOT_PBISM_VERSION, DIAGRAM_LAYOUT, MAX_CACHE_BYTES, RESOURCE_FOLDERS,
    cache_abf_path, read_cache_abf, read_copilot_files, read_diagram_layout, read_legacy_layout,
    read_report_files, write_cache_abf, write_copilot_files, write_diagram_layout, write_legacy_report,
};
pub use report::{read_report, save_report};

/// Largest JSON file read from a project. Input is hostile.
const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("{0} is larger than {MAX_FILE_BYTES} bytes")]
    TooLarge(PathBuf),
    #[error("{0}: {1}")]
    Text(PathBuf, bicraft_opc::Error),
    #[error("{0}: {1}")]
    Tmsl(PathBuf, bicraft_tmsl::Error),
    #[error("no `*.SemanticModel` folder found next to {0}")]
    NoModel(PathBuf),
    #[error(transparent)]
    Tmdl(#[from] bicraft_tmdl::Error),
    #[error("invalid project name {name:?}: {reason}")]
    InvalidName { name: String, reason: &'static str },
    #[error("invalid project id {id:?}: {reason}")]
    InvalidId { id: String, reason: &'static str },
    #[error("{0} already exists; refusing to overwrite an existing project")]
    AlreadyExists(PathBuf),
    #[error("a project model must be written as TMDL or TMSL, not `missing`")]
    NoFormat,
    #[error("{0}: {1}")]
    Json(PathBuf, serde_json::Error),
    #[error("{0}: {1}")]
    Pbir(PathBuf, bicraft_pbir::Error),
    #[error("{0}: {1}")]
    Layout(PathBuf, bicraft_layout::Error),
    #[error("no `*.Report` folder with a report found in {0}")]
    NoReport(PathBuf),
    #[error("{0}: {1}")]
    BadDefinition(PathBuf, &'static str),
    #[error("{0} was not written; the legacy report.json is kept")]
    ReportNotWritten(PathBuf),
    #[error(
        "{0} holds both `model.bim` and a TMDL `definition/` folder; Power BI Desktop refuses such a \
         project: keep one of them"
    )]
    BothModelFormats(PathBuf),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelFormat {
    /// A single `model.bim` TMSL JSON file.
    Tmsl,
    /// A `definition/` folder of `.tmdl` files.
    Tmdl,
    Missing,
}

impl ModelFormat {
    /// Parses a user-facing format name: `tmdl`, or `tmsl` / `bim` / `model.bim` (any case).
    /// `missing` is not a format one can ask for, so it is rejected.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "tmdl" => Some(Self::Tmdl),
            "tmsl" | "bim" | "model.bim" => Some(Self::Tmsl),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tmsl => "tmsl",
            Self::Tmdl => "tmdl",
            Self::Missing => "missing",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReportFormat {
    Legacy,
    Pbir,
    Missing,
}

#[derive(Debug, Clone, Serialize)]
pub struct Project {
    /// Folder holding the `.pbip` file and the item folders.
    pub root: PathBuf,
    pub semantic_model: Option<PathBuf>,
    pub model_format: ModelFormat,
    pub report: Option<PathBuf>,
    pub report_format: ReportFormat,
}

/// Locates the project items for a `.pbip` file, a project folder, or an item folder.
///
/// When several projects share a folder, a `.pbip` file or item folder selects the items with the
/// same name (`Sales.pbip` → `Sales.SemanticModel`, `Sales.Report`); otherwise the first item of
/// each kind in name order is used.
pub fn locate(path: &Path) -> Result<Project, Error> {
    let item_dir = path.is_dir() && (is_item(path, SEMANTIC_MODEL) || is_item(path, REPORT));
    let (root, stem) = if path.is_dir() && !item_dir {
        (path.to_path_buf(), None)
    } else {
        (parent_dir(path), project_stem(path))
    };
    let entries = std::fs::read_dir(&root).map_err(|e| Error::Io(root.clone(), e))?;
    let mut items: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    items.sort();
    let semantic_model = pick_item(&items, SEMANTIC_MODEL, stem.as_deref());
    let report = pick_item(&items, REPORT, stem.as_deref());
    let model_format = match &semantic_model {
        Some(dir) if dir.join("model.bim").is_file() => ModelFormat::Tmsl,
        Some(dir) if dir.join("definition").is_dir() => ModelFormat::Tmdl,
        _ => ModelFormat::Missing,
    };
    let report_format = match &report {
        Some(dir) if dir.join("definition").is_dir() => ReportFormat::Pbir,
        Some(dir) if dir.join("report.json").is_file() => ReportFormat::Legacy,
        _ => ReportFormat::Missing,
    };
    Ok(Project {
        root,
        semantic_model,
        model_format,
        report,
        report_format,
    })
}

const SEMANTIC_MODEL: &str = ".SemanticModel";
const REPORT: &str = ".Report";

/// The folder holding `path`. A bare name (`Sales.pbip`) has an empty parent, which means the
/// current directory.
fn parent_dir(path: &Path) -> PathBuf {
    match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// The project name a `.pbip` file or item folder stands for.
fn project_stem(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    [".pbip", SEMANTIC_MODEL, REPORT]
        .iter()
        .find_map(|suffix| strip_suffix_ignore_case(name, suffix))
        .map(str::to_owned)
}

fn pick_item(items: &[PathBuf], suffix: &str, stem: Option<&str>) -> Option<PathBuf> {
    let mut candidates = items.iter().filter(|p| is_item(p, suffix));
    let first = candidates.clone().next();
    stem.and_then(|stem| {
        candidates.find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| strip_suffix_ignore_case(n, suffix))
                .is_some_and(|s| s.eq_ignore_ascii_case(stem))
        })
    })
    .or(first)
    .cloned()
}

/// `name` without `suffix` (ASCII, compared case-insensitively), if it ends with it and keeps a
/// non-empty stem.
fn strip_suffix_ignore_case<'a>(name: &'a str, suffix: &str) -> Option<&'a str> {
    let cut = name.len().checked_sub(suffix.len()).filter(|&n| n > 0)?;
    let tail = name.get(cut..)?;
    if tail.eq_ignore_ascii_case(suffix) {
        name.get(..cut)
    } else {
        None
    }
}

pub fn read_model(project: &Project) -> Result<SemanticModel, Error> {
    let Some(dir) = &project.semantic_model else {
        return Err(Error::NoModel(project.root.clone()));
    };
    match project.model_format {
        ModelFormat::Tmsl => {
            if dir.join("definition").is_dir() {
                return Err(Error::BothModelFormats(dir.clone()));
            }
            read_bim(&dir.join("model.bim"))
        }
        ModelFormat::Tmdl => Ok(bicraft_tmdl::read_folder(&dir.join("definition"))?),
        ModelFormat::Missing => Err(Error::NoModel(project.root.clone())),
    }
}

/// Reads a standalone `model.bim` file.
pub fn read_bim(path: &Path) -> Result<SemanticModel, Error> {
    let bytes = read_capped(path)?;
    let (text, _) =
        bicraft_opc::decode_text(&path.to_string_lossy(), &bytes).map_err(|e| Error::Text(path.into(), e))?;
    bicraft_tmsl::read(&text).map_err(|e| Error::Tmsl(path.into(), e))
}

pub(crate) fn read_capped(path: &Path) -> Result<Vec<u8>, Error> {
    let meta = std::fs::metadata(path).map_err(|e| Error::Io(path.into(), e))?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(Error::TooLarge(path.into()));
    }
    std::fs::read(path).map_err(|e| Error::Io(path.into(), e))
}

/// Item folders end in `.SemanticModel`/`.Report`; compare case-insensitively so projects
/// authored on Windows still resolve on case-sensitive Linux file systems.
fn is_item(path: &Path, suffix: &str) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| strip_suffix_ignore_case(n, suffix).is_some())
}

#[cfg(test)]
#[path = "../../../testsupport/tempdir.rs"]
mod tempdir;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tempdir::TempDir;

    #[test]
    fn a_bare_project_name_resolves_in_the_current_directory() {
        assert_eq!(parent_dir(Path::new("Sales.pbip")), PathBuf::from("."));
        assert_eq!(parent_dir(Path::new("x/Sales.pbip")), PathBuf::from("x"));
    }

    fn temp_project(name: &str) -> TempDir {
        let root = TempDir::new(&format!("pbip-{name}"));
        std::fs::create_dir_all(root.join("Sales.SemanticModel")).unwrap();
        std::fs::create_dir_all(root.join("Sales.Report/definition")).unwrap();
        std::fs::write(root.join("Sales.pbip"), "{}").unwrap();
        std::fs::write(
            root.join("Sales.SemanticModel/model.bim"),
            r#"{"name":"Sales","model":{"tables":[{"name":"T","columns":[{"name":"C","dataType":"string"}]}]}}"#,
        )
        .unwrap();
        root
    }

    #[test]
    fn locates_items_from_the_pbip_file_or_the_folder() {
        let root = temp_project("locate");
        for start in [
            root.join("Sales.pbip"),
            root.to_path_buf(),
            root.join("Sales.SemanticModel"),
        ] {
            let p = locate(&start).unwrap();
            assert_eq!(p.model_format, ModelFormat::Tmsl);
            assert_eq!(p.report_format, ReportFormat::Pbir);
            assert!(read_model(&p).unwrap().table("T").is_some());
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn reads_a_tmdl_semantic_model() {
        let root = TempDir::new("pbip-tmdl");
        let def = root.join("Shop.SemanticModel/definition");
        std::fs::create_dir_all(def.join("tables")).unwrap();
        std::fs::create_dir_all(root.join("Shop.Report")).unwrap();
        std::fs::write(root.join("Shop.pbip"), "{}").unwrap();
        std::fs::write(root.join("Shop.Report/report.json"), "{}").unwrap();
        std::fs::write(root.join("Shop.SemanticModel/definition.pbism"), "{}").unwrap();
        std::fs::write(
            def.join("database.tmdl"),
            "database Shop\r\n\tcompatibilityLevel: 1567\r\n",
        )
        .unwrap();
        std::fs::write(def.join("model.tmdl"), "model Model\n\tculture: en-US\n").unwrap();
        std::fs::write(
            def.join("relationships.tmdl"),
            "relationship r1\n\tfromColumn: 'Order Lines'.Product\n\ttoColumn: Product.Product\n",
        )
        .unwrap();
        std::fs::write(
            def.join("tables/Order Lines.tmdl"),
            "table 'Order Lines'\n\tmeasure Units =\n\t\t\tSUM ( 'Order Lines'[Qty] )\n\tcolumn Qty\n\t\tdataType: int64\n\tcolumn Product\n\t\tdataType: string\n",
        )
        .unwrap();
        std::fs::write(
            def.join("tables/Product.tmdl"),
            "table Product\n\tcolumn Product\n\t\tdataType: string\n",
        )
        .unwrap();

        let p = locate(&root.join("Shop.pbip")).unwrap();
        assert_eq!(p.model_format, ModelFormat::Tmdl);
        assert_eq!(p.report_format, ReportFormat::Legacy);
        let m = read_model(&p).unwrap();
        assert_eq!(m.name, "Shop");
        assert_eq!(m.compatibility_level, Some(1567));
        assert_eq!(
            m.measure("Units").map(|(_, x)| x.expression.as_str()),
            Some("SUM ( 'Order Lines'[Qty] )")
        );
        assert_eq!(m.relationships.len(), 1);

        std::fs::write(def.join("tables/Bad.tmdl"), "table Bad Name\n").unwrap();
        let err = read_model(&p).unwrap_err().to_string();
        assert!(err.contains("tables/Bad.tmdl:1:"), "{err}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Regression (packaging-verification.md P-BOTH, oracle case j07): Desktop refuses a semantic
    /// model folder holding both `model.bim` and a TMDL `definition/` folder; the reader used to
    /// take `model.bim` silently.
    #[test]
    fn model_bim_next_to_tmdl_is_refused() {
        let root = TempDir::new("pbip-both-models");
        let model = root.join("Shop.SemanticModel");
        std::fs::create_dir_all(model.join("definition")).unwrap();
        std::fs::write(root.join("Shop.pbip"), "{}").unwrap();
        std::fs::write(model.join("definition.pbism"), "{}").unwrap();
        std::fs::write(model.join("definition/model.tmdl"), "model Model\n").unwrap();
        std::fs::write(
            model.join("model.bim"),
            r#"{"name":"Shop","compatibilityLevel":1606,"model":{}}"#,
        )
        .unwrap();
        let p = locate(&root.join("Shop.pbip")).unwrap();
        let err = read_model(&p).unwrap_err();
        assert!(matches!(err, Error::BothModelFormats(_)), "{err}");
        let text = err.to_string();
        assert!(
            text.contains("model.bim") && text.contains("definition/"),
            "{text}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn item_suffix_match_ignores_case() {
        assert!(is_item(Path::new("x/Sales.semanticmodel"), ".SemanticModel"));
        assert!(!is_item(Path::new("x/.SemanticModel"), ".SemanticModel"));
    }
}
