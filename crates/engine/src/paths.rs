//! MAX_PATH guard. Power BI Desktop cannot read a project file whose full path is 260 characters
//! or longer ("the fully qualified file name must be less than 260 characters, and the directory
//! name must be less than 248 characters"), and PBIR paths are deep:
//! `<dir>\<Name>.Report\definition\pages\<20 hex>\visuals\<20 hex>\visual.json`.
//!
//! The check runs on every OS (projects written on Linux are opened on Windows), counts UTF-16 code
//! units like Windows does, and measures the path as it is on this machine.

use std::path::{Path, PathBuf};

use bicraft_model::SemanticModel;
use bicraft_pbip::{ModelFormat, Project};
use bicraft_report::Report;
use serde::Serialize;
use serde_json::{Value, json};

/// Longest full file path Desktop reads (it must be *less than* 260).
pub(crate) const MAX_FILE_PATH: usize = 259;
/// Longest directory path Desktop reads (it must be *less than* 248).
pub(crate) const MAX_DIR_PATH: usize = 247;
/// Most offending paths listed in a result; the count is always exact.
const MAX_LISTED: usize = 20;

/// Files Desktop itself adds to a project when it opens and saves it (local, not source-controlled).
const DESKTOP_LOCAL_FILES: [&str; 3] = [
    "{model}/.pbi/localSettings.json",
    "{model}/.pbi/editorSettings.json",
    "{report}/.pbi/localSettings.json",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct LongPath {
    pub path: String,
    pub length: usize,
    pub limit: usize,
    pub kind: &'static str,
}

/// Paths (files and their directories) under `root` that exceed Desktop's limits. `files` are
/// `/`-separated paths relative to `root`.
pub(crate) fn long_paths<'a>(root: &Path, files: impl IntoIterator<Item = &'a str>) -> Vec<LongPath> {
    let root = absolute(root);
    let mut root_text = root.to_string_lossy().replace('/', "\\");
    while root_text.ends_with('\\') && root_text.len() > 3 {
        root_text.pop();
    }
    let mut out = Vec::new();
    let mut dirs_seen = std::collections::BTreeSet::new();
    for rel in files {
        let full = format!("{root_text}\\{}", rel.replace('/', "\\"));
        let length = utf16_len(&full);
        if length > MAX_FILE_PATH {
            out.push(LongPath {
                path: full.clone(),
                length,
                limit: MAX_FILE_PATH,
                kind: "file",
            });
        }
        if let Some((dir, _)) = full.rsplit_once('\\') {
            let length = utf16_len(dir);
            if length > MAX_DIR_PATH && dirs_seen.insert(dir.to_string()) {
                out.push(LongPath {
                    path: dir.to_string(),
                    length,
                    limit: MAX_DIR_PATH,
                    kind: "directory",
                });
            }
        }
    }
    out.sort_by(|a, b| b.length.cmp(&a.length).then_with(|| a.path.cmp(&b.path)));
    out
}

/// Every file a project `<dir>/<name>.pbip` contains once written (and once Desktop has added its
/// local files), relative to `dir`.
pub(crate) fn project_files(
    name: &str,
    model: &SemanticModel,
    format: ModelFormat,
    report: Option<&Report>,
) -> Vec<String> {
    let model_dir = format!("{name}.SemanticModel");
    let report_dir = format!("{name}.Report");
    let mut files = vec![
        format!("{name}.pbip"),
        ".gitignore".to_string(),
        format!("{model_dir}/definition.pbism"),
        format!("{model_dir}/.platform"),
        format!("{report_dir}/definition.pbir"),
        format!("{report_dir}/.platform"),
    ];
    files.extend(model_files(&model_dir, model, format));
    match report {
        Some(r) => files.extend(report_files(&report_dir, r)),
        // The blank report: one page with a 20-character name.
        None => {
            for f in [
                "version.json",
                "report.json",
                "pages/pages.json",
                "pages/00000000000000000000/page.json",
            ] {
                files.push(format!("{report_dir}/definition/{f}"));
            }
        }
    }
    files.extend(
        DESKTOP_LOCAL_FILES
            .iter()
            .map(|f| f.replace("{model}", &model_dir).replace("{report}", &report_dir)),
    );
    files
}

fn model_files(model_dir: &str, model: &SemanticModel, format: ModelFormat) -> Vec<String> {
    match format {
        ModelFormat::Tmsl => vec![format!("{model_dir}/model.bim")],
        _ => bicraft_tmdl::write_documents(model)
            .into_iter()
            .map(|(rel, _)| format!("{model_dir}/definition/{rel}"))
            .collect(),
    }
}

fn report_files(report_dir: &str, report: &Report) -> Vec<String> {
    bicraft_pbir::write_documents(report)
        .into_iter()
        .map(|(rel, _)| format!("{report_dir}/{rel}"))
        .collect()
}

/// The long paths of an existing project after saving `model` and `report` into it.
pub(crate) fn project_long_paths(
    project: &Project,
    model: Option<&SemanticModel>,
    report: Option<&Report>,
) -> Vec<LongPath> {
    let item = |p: &Option<PathBuf>| {
        p.as_ref()
            .and_then(|d| d.file_name())
            .map(|n| n.to_string_lossy().into_owned())
    };
    let mut files = Vec::new();
    if let (Some(dir), Some(m)) = (item(&project.semantic_model), model) {
        files.extend(model_files(&dir, m, project.model_format));
    }
    if let (Some(dir), Some(r)) = (item(&project.report), report) {
        files.extend(report_files(&dir, r));
    }
    long_paths(&project.root, files.iter().map(String::as_str))
}

/// A human summary plus the JSON listing, or `None` when every path fits.
pub(crate) fn describe(long: &[LongPath]) -> Option<(String, Value)> {
    let first = long.first()?;
    let message = format!(
        "{} path(s) are too long for Power BI Desktop (files must be under 260 characters, folders under \
         248): the longest is {} characters ({}). Move the project to a shorter folder or use a shorter name",
        long.len(),
        first.length,
        first.path
    );
    let listed: Vec<&LongPath> = long.iter().take(MAX_LISTED).collect();
    Some((message, json!({"count": long.len(), "paths": listed})))
}

fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

fn absolute(p: &Path) -> PathBuf {
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_paths_pass_and_long_ones_are_reported_longest_first() {
        let root = Path::new("C:/p");
        assert!(long_paths(root, ["a/b.json"]).is_empty());
        let deep = format!("{}/x.json", "d".repeat(260));
        let deeper = format!("{}/x.json", "d".repeat(270));
        let long = long_paths(root, [deep.as_str(), deeper.as_str()]);
        assert_eq!(long.first().map(|l| l.kind), Some("file"));
        assert!(long.windows(2).all(|w| w[0].length >= w[1].length));
        assert!(long.iter().any(|l| l.kind == "directory"));
    }

    #[test]
    fn the_limits_are_exact() {
        let root = absolute(Path::new("."));
        let base = utf16_len(&root.to_string_lossy()) + 1; // + separator
        let fits = "f".repeat(MAX_FILE_PATH - base);
        let too_long = "f".repeat(MAX_FILE_PATH - base + 1);
        assert!(long_paths(&root, [fits.as_str()]).is_empty());
        assert_eq!(long_paths(&root, [too_long.as_str()]).len(), 1);
    }

    #[test]
    fn length_counts_utf16_units() {
        assert_eq!(utf16_len("é"), 1);
        assert_eq!(utf16_len("😀"), 2);
    }

    #[test]
    fn project_files_cover_report_visuals() {
        let files = project_files("Sales", &SemanticModel::default(), ModelFormat::Tmsl, None);
        assert!(files.iter().any(|f| f == "Sales.SemanticModel/model.bim"));
        assert!(files.iter().any(|f| f.ends_with("page.json")));
        assert!(files.iter().any(|f| f.ends_with(".pbi/localSettings.json")));
    }
}
