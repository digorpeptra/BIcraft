//! Where `project.test` findings are in the project's files, for SARIF and the text report: TMDL
//! declarations (file and line), `model.bim` objects (line of their `"name"`), PBIR page, visual
//! and bookmark files, and the test file itself for DAX assertions. Best effort: a finding whose
//! object cannot be found points at the model or report file, or at nothing.
//!
//! Paths are shown relative to the base folder (the `base_dir` parameter, else the project
//! folder), `/`-separated; in server mode never outside the workspace root.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bicraft_pbip::{ModelFormat, ReportFormat};

use crate::citest::{Case, Target, TestSource};
use crate::{ServerConfig, Session, Source};

/// Most TMDL files indexed.
const MAX_FILES: usize = 10_000;
/// Deepest folder level walked under `definition/`.
const MAX_DEPTH: usize = 8;
/// Largest file read for locating.
const MAX_FILE_BYTES: u64 = 16 << 20;

/// `path` as results show it.
pub(crate) fn relative(path: &Path, base: &Path, server: Option<&ServerConfig>) -> String {
    let strip = |p: &Path, b: &Path| p.strip_prefix(b).ok().map(Path::to_path_buf);
    let canon = |p: &Path| std::fs::canonicalize(p).ok();
    let rel = strip(path, base).or_else(|| match (canon(path), canon(base)) {
        (Some(p), Some(b)) => strip(&p, &b),
        _ => None,
    });
    let shown = match rel {
        Some(r) => r,
        None => match server {
            Some(cfg) => strip(path, cfg.root())
                .or_else(|| canon(path).and_then(|p| canon(cfg.root()).and_then(|r| strip(&p, &r))))
                .unwrap_or_else(|| PathBuf::from(path.file_name().unwrap_or_default())),
            None => path.to_path_buf(),
        },
    };
    let text = shown.to_string_lossy().replace('\\', "/");
    if text.is_empty() { ".".into() } else { text }
}

fn read(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// TMDL declarations: (kind, table, name) in lower case → (file, line).
type Index = HashMap<(String, String, String), (PathBuf, usize)>;

fn tmdl_files(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > MAX_DEPTH || out.len() >= MAX_FILES {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut items: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    items.sort();
    for p in items {
        if out.len() >= MAX_FILES {
            return;
        }
        let Ok(meta) = std::fs::symlink_metadata(&p) else {
            continue;
        };
        if meta.is_dir() {
            tmdl_files(&p, depth + 1, out);
        } else if meta.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("tmdl")) {
            out.push(p);
        }
    }
}

/// The name after a TMDL keyword: `'quoted ''name'''` or a bare word.
fn tmdl_name(rest: &str) -> String {
    let rest = rest.trim_start();
    if let Some(q) = rest.strip_prefix('\'') {
        let mut name = String::new();
        let mut chars = q.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\'' {
                if chars.peek() == Some(&'\'') {
                    chars.next();
                    name.push('\'');
                } else {
                    break;
                }
            } else {
                name.push(c);
            }
        }
        name
    } else {
        rest.split(|c: char| c.is_whitespace() || c == '=')
            .next()
            .unwrap_or_default()
            .to_string()
    }
}

fn tmdl_index(definition: &Path) -> Index {
    let mut files = Vec::new();
    tmdl_files(definition, 0, &mut files);
    let mut index = Index::new();
    for f in files {
        let Some(text) = read(&f) else { continue };
        let mut table = String::new();
        for (i, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            let top = trimmed.len() == line.len();
            let Some((word, rest)) = trimmed.split_once(' ') else {
                continue;
            };
            let kind = match word {
                "table" if top => {
                    table = tmdl_name(rest).to_lowercase();
                    let key = ("table".to_string(), String::new(), table.clone());
                    index.entry(key).or_insert((f.clone(), i + 1));
                    continue;
                }
                "role" | "expression" | "relationship" if top => {
                    let name = tmdl_name(rest).to_lowercase();
                    // a role is the context of its table permissions (row filters)
                    table = if word == "role" {
                        name.clone()
                    } else {
                        String::new()
                    };
                    let key = (word.to_string(), String::new(), name);
                    index.entry(key).or_insert((f.clone(), i + 1));
                    continue;
                }
                "column" | "measure" | "hierarchy" | "partition" | "calculationItem" if !top => word,
                "tablePermission" if !top => "row_filter",
                _ => continue,
            };
            let kind = if kind == "calculationItem" {
                "calculation_item"
            } else {
                kind
            };
            let key = (kind.to_string(), table.clone(), tmdl_name(rest).to_lowercase());
            index.entry(key).or_insert((f.clone(), i + 1));
        }
    }
    index
}

/// The model's files: a TMDL index, or one JSON / package file.
enum ModelFiles {
    Tmdl { definition: PathBuf, index: Index },
    Bim { path: PathBuf, text: Option<String> },
    Package(PathBuf),
    None,
}

enum ReportFiles {
    Pbir(PathBuf),
    Legacy(PathBuf),
    Package(PathBuf),
    None,
}

fn model_files(s: &Session) -> ModelFiles {
    match &s.source {
        Some(Source::Project { project, .. }) => match (&project.semantic_model, project.model_format) {
            (Some(dir), ModelFormat::Tmdl) => {
                let definition = dir.join("definition");
                ModelFiles::Tmdl {
                    index: tmdl_index(&definition),
                    definition,
                }
            }
            (Some(dir), ModelFormat::Tmsl) => {
                let path = dir.join("model.bim");
                ModelFiles::Bim {
                    text: read(&path),
                    path,
                }
            }
            _ => ModelFiles::None,
        },
        Some(Source::Bim { path }) => ModelFiles::Bim {
            text: read(path),
            path: path.clone(),
        },
        Some(Source::Package { path, .. }) => ModelFiles::Package(path.clone()),
        None => ModelFiles::None,
    }
}

fn report_files(s: &Session) -> ReportFiles {
    match &s.source {
        Some(Source::Project { project, .. }) => match (&project.report, project.report_format) {
            (Some(dir), ReportFormat::Pbir) => ReportFiles::Pbir(dir.clone()),
            (Some(dir), ReportFormat::Legacy) => ReportFiles::Legacy(dir.join("report.json")),
            _ => ReportFiles::None,
        },
        Some(Source::Package { path, .. }) => ReportFiles::Package(path.clone()),
        _ => ReportFiles::None,
    }
}

/// The 1-based line of the first `"name": "<name>"` at or after line `from`.
fn json_name_line(text: &str, name: &str, from: usize) -> Option<usize> {
    let quoted = serde_json::to_string(name).ok()?;
    text.lines()
        .enumerate()
        .skip(from)
        .find(|(_, l)| {
            l.match_indices("\"name\"").any(|(i, k)| {
                l.get(i + k.len()..)
                    .map(str::trim_start)
                    .and_then(|x| x.strip_prefix(':'))
                    .is_some_and(|x| x.trim_start().starts_with(&quoted))
            })
        })
        .map(|(i, _)| i + 1)
}

fn locate_model(
    files: &ModelFiles,
    kind: &str,
    table: Option<&str>,
    name: &str,
) -> Option<(PathBuf, Option<usize>)> {
    let kind = match kind {
        "calculated_column" => "column",
        "calculated_table" => "table",
        k => k,
    };
    match files {
        ModelFiles::Tmdl { definition, index } => {
            let (k, t, n) = match kind {
                "table" | "role" | "expression" | "relationship" => (kind, String::new(), name),
                // a row filter: the role's table permission
                "row_filter" => ("row_filter", name.to_lowercase(), table.unwrap_or_default()),
                _ => (kind, table.unwrap_or_default().to_lowercase(), name),
            };
            let key = (k.to_string(), t.clone(), n.to_lowercase());
            if let Some((f, l)) = index.get(&key) {
                return Some((f.clone(), Some(*l)));
            }
            if kind == "row_filter" {
                let role = ("role".to_string(), String::new(), name.to_lowercase());
                if let Some((f, l)) = index.get(&role) {
                    return Some((f.clone(), Some(*l)));
                }
            }
            if let Some(t) = table {
                let key = ("table".to_string(), String::new(), t.to_lowercase());
                if let Some((f, l)) = index.get(&key) {
                    return Some((f.clone(), Some(*l)));
                }
            }
            let model = definition.join("model.tmdl");
            model.is_file().then_some((model, None))
        }
        ModelFiles::Bim { path, text } => {
            let line = text.as_deref().and_then(|text| match table {
                Some(t) if kind != "table" => {
                    let from = json_name_line(text, t, 0).unwrap_or(0);
                    json_name_line(text, name, from).or((from > 0).then_some(from))
                }
                _ => json_name_line(text, name, 0),
            });
            Some((path.clone(), line))
        }
        ModelFiles::Package(p) => Some((p.clone(), None)),
        ModelFiles::None => None,
    }
}

fn locate_report(
    s: &Session,
    files: &ReportFiles,
    page: Option<&str>,
    visual: Option<&str>,
    bookmark: Option<&str>,
) -> Option<(PathBuf, Option<usize>)> {
    match files {
        ReportFiles::Pbir(dir) => {
            let def = dir.join("definition");
            if let Some(b) = bookmark {
                let f = def.join("bookmarks").join(format!("{b}.bookmark.json"));
                if f.is_file() {
                    return Some((f, None));
                }
            }
            let page_name = page
                .and_then(|p| s.report.as_ref().and_then(|r| r.page(p)))
                .map(|p| p.name.clone());
            if let Some(p) = page_name {
                let page_dir = def.join("pages").join(&p);
                if let Some(v) = visual {
                    let f = page_dir.join("visuals").join(v).join("visual.json");
                    if f.is_file() {
                        return Some((f, None));
                    }
                }
                let f = page_dir.join("page.json");
                if f.is_file() {
                    return Some((f, None));
                }
            }
            let f = def.join("report.json");
            f.is_file().then_some((f, None))
        }
        ReportFiles::Legacy(f) | ReportFiles::Package(f) => Some((f.clone(), None)),
        ReportFiles::None => None,
    }
}

fn test_line(tests: &TestSource, needle: &str) -> Option<usize> {
    let text = tests.text.as_deref()?;
    let quoted = serde_json::to_string(needle).ok();
    text.lines()
        .enumerate()
        .find(|(_, l)| {
            let t = l.trim_start().trim_start_matches("- ");
            t.strip_prefix("name:")
                .or_else(|| t.strip_prefix("\"name\":"))
                .map(str::trim)
                .is_some_and(|v| {
                    v == needle
                        || quoted.as_deref().is_some_and(|q| v.trim_end_matches(',') == q)
                        || v.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')) == Some(needle)
                })
        })
        .map(|(i, _)| i + 1)
}

/// Fills in [`crate::citest::Finding::location`] for every finding.
pub(crate) fn locate(s: &Session, cases: &mut [Case], tests: &TestSource, base: &Path) {
    let needs =
        |pred: &dyn Fn(&Target) -> bool| cases.iter().any(|c| c.findings.iter().any(|f| pred(&f.target)));
    let model = if needs(&|t| matches!(t, Target::Model { .. })) {
        model_files(s)
    } else {
        ModelFiles::None
    };
    let report = report_files(s);
    let server = s.server_config();
    for c in cases.iter_mut() {
        for f in &mut c.findings {
            let found = match &f.target {
                Target::None => None,
                Target::Model { kind, table, name } => locate_model(&model, kind, table.as_deref(), name),
                Target::Report {
                    page,
                    visual,
                    bookmark,
                } => locate_report(
                    s,
                    &report,
                    page.as_deref(),
                    visual.as_deref(),
                    bookmark.as_deref(),
                ),
                Target::ReportFile(rel) => match &report {
                    ReportFiles::Pbir(dir) => {
                        let p = dir.join(rel);
                        Some(if p.is_file() {
                            (p, None)
                        } else {
                            (dir.clone(), None)
                        })
                    }
                    ReportFiles::Legacy(p) | ReportFiles::Package(p) => Some((p.clone(), None)),
                    ReportFiles::None => None,
                },
                Target::TestFile(needle) => {
                    tests.path.as_ref().map(|p| (p.clone(), test_line(tests, needle)))
                }
            };
            f.location = found.map(|(p, line)| (relative(&p, base, server), line));
        }
    }
}
