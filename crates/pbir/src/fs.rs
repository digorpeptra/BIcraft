//! `read_dir` / `write_dir`: the PBIR documents of a `<Name>.Report/` folder.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;

use crate::Error;
use crate::paths::{self, DocKind};
use crate::write::is_owned_kind;

/// Folders of the report root that hold report content.
const CONTENT: [&str; 3] = ["definition", "StaticResources", "CustomVisuals"];
const MAX_DEPTH: usize = 32;
const MAX_FILES: usize = 200_000;
const MAX_FILE_BYTES: u64 = 1 << 30;

fn io(path: &Path, source: std::io::Error) -> Error {
    Error::Io {
        path: path.display().to_string(),
        source,
    }
}

/// Every file under the content folders, as (`/`-separated relative path, bytes), sorted.
///
/// The tree is read one folder level at a time, the folders of a level by several threads (each
/// lists its folders and reads their files): a PBIR report keeps every visual in a folder of its
/// own, and on Windows each open costs far more than the bytes (docs/benchmarks.md, G23).
pub(crate) fn collect(root: &Path) -> Result<Vec<(String, Vec<u8>)>, Error> {
    let meta = std::fs::metadata(root).map_err(|e| io(root, e))?;
    if !meta.is_dir() {
        return Err(io(
            root,
            std::io::Error::new(std::io::ErrorKind::NotADirectory, "not a directory"),
        ));
    }
    let mut level: Vec<(PathBuf, String)> = CONTENT
        .iter()
        .map(|top| (root.join(top), (*top).to_string()))
        .filter(|(dir, _)| dir.is_dir())
        .collect();
    let count = AtomicUsize::new(0);
    let mut out = Vec::new();
    let mut depth = 0usize;
    while !level.is_empty() {
        if depth > MAX_DEPTH {
            return Err(Error::InvalidPath {
                path: level.first().map(|(_, rel)| rel.clone()).unwrap_or_default(),
                message: "folders nested too deeply".into(),
            });
        }
        let mut next = Vec::new();
        for listed in parallel_map(&level, |(dir, rel)| read_folder(dir, rel, &count)) {
            let listed = listed?;
            out.extend(listed.files);
            next.extend(listed.dirs);
        }
        level = next;
        depth = depth.saturating_add(1);
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// One folder's files (read) and sub-folders.
struct Listed {
    files: Vec<(String, Vec<u8>)>,
    dirs: Vec<(PathBuf, String)>,
}

fn read_folder(dir: &Path, rel: &str, count: &AtomicUsize) -> Result<Listed, Error> {
    let mut listed = Listed {
        files: Vec::new(),
        dirs: Vec::new(),
    };
    for entry in std::fs::read_dir(dir).map_err(|e| io(dir, e))? {
        let entry = entry.map_err(|e| io(dir, e))?;
        let path = entry.path();
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            return Err(Error::InvalidPath {
                path: path.display().to_string(),
                message: "file name is not valid Unicode".into(),
            });
        };
        let child = format!("{rel}/{name}");
        // Both come from the folder listing; no extra system call per entry.
        let ty = entry.file_type().map_err(|e| io(&path, e))?;
        if ty.is_symlink() {
            continue;
        }
        if ty.is_dir() {
            listed.dirs.push((path, child));
        } else if ty.is_file() {
            if count.fetch_add(1, Ordering::Relaxed) >= MAX_FILES {
                return Err(Error::InvalidPath {
                    path: child,
                    message: format!("more than {MAX_FILES} files"),
                });
            }
            let len = entry.metadata().map_err(|e| io(&path, e))?.len();
            if len > MAX_FILE_BYTES {
                return Err(Error::InvalidPath {
                    path: child,
                    message: "file larger than 1 GiB".into(),
                });
            }
            let bytes = std::fs::read(&path).map_err(|e| io(&path, e))?;
            listed.files.push((child, bytes));
        }
    }
    Ok(listed)
}

/// Fewer items than this per call are handled on the calling thread: starting threads costs more
/// than it saves.
const PARALLEL_MIN_ITEMS: usize = 16;
/// Most worker threads for file system work.
const MAX_WORKERS: usize = 16;

/// `items.iter().map(f)`, in order, spread over up to [`MAX_WORKERS`] threads in contiguous
/// chunks. When a thread cannot be started (or fails) its chunk runs on the calling thread.
fn parallel_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let workers = if items.len() < PARALLEL_MIN_ITEMS {
        1
    } else {
        std::thread::available_parallelism()
            .map_or(1, std::num::NonZeroUsize::get)
            .min(MAX_WORKERS)
    };
    if workers <= 1 {
        return items.iter().map(&f).collect();
    }
    let chunk = items.len().div_ceil(workers).max(1);
    let f = &f;
    std::thread::scope(|scope| {
        let handles: Vec<_> = items
            .chunks(chunk)
            .map(|part| {
                let handle = std::thread::Builder::new()
                    .spawn_scoped(scope, move || part.iter().map(f).collect::<Vec<R>>())
                    .ok();
                (part, handle)
            })
            .collect();
        let mut out = Vec::with_capacity(items.len());
        for (part, handle) in handles {
            match handle.map(std::thread::ScopedJoinHandle::join) {
                Some(Ok(results)) => out.extend(results),
                _ => out.extend(part.iter().map(f)),
            }
        }
        out
    })
}

fn join(root: &Path, rel: &str) -> PathBuf {
    rel.split('/').fold(root.to_path_buf(), |p, part| p.join(part))
}

/// Paths (relative) of the files `write_dir` owns that exist now: page, visual, mobile and
/// bookmark documents.
fn owned_files(root: &Path) -> Result<Vec<String>, Error> {
    let mut found = Vec::new();
    for sub in ["definition/pages", "definition/bookmarks"] {
        let dir = join(root, sub);
        if dir.is_dir() {
            let mut files = Vec::new();
            walk_names(&dir, sub, 0, &mut files)?;
            found.extend(
                files
                    .into_iter()
                    .filter(|p| paths::classify(p).is_ok_and(|k| is_owned_kind(&k))),
            );
        }
    }
    Ok(found)
}

fn walk_names(dir: &Path, rel: &str, depth: usize, out: &mut Vec<String>) -> Result<(), Error> {
    if depth > MAX_DEPTH || out.len() >= MAX_FILES {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir).map_err(|e| io(dir, e))? {
        let entry = entry.map_err(|e| io(dir, e))?;
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let child = format!("{rel}/{name}");
        let ty = entry.file_type().map_err(|e| io(&entry.path(), e))?;
        if ty.is_dir() {
            walk_names(&entry.path(), &child, depth + 1, out)?;
        } else if ty.is_file() {
            out.push(child);
        }
    }
    Ok(())
}

/// `StaticResources/RegisteredResources/<item>` files registered by the report.json on disk, and
/// the files under `CustomVisuals/<name>/` of each `CustomVisual` package it registers.
fn registered_resources(root: &Path) -> Vec<String> {
    let Ok(bytes) = std::fs::read(join(root, paths::REPORT)) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for package in value
        .get("resourcePackages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let kind = package.get("type").and_then(Value::as_str);
        let Some(name) = package.get("name").and_then(Value::as_str) else {
            continue;
        };
        // a registered custom visual owns its `CustomVisuals/<name>/` folder
        if kind == Some("CustomVisual") {
            let rel = format!("CustomVisuals/{name}");
            let dir = join(root, &rel);
            if paths::classify(&format!("{rel}/package.json")).is_ok_and(|k| matches!(k, DocKind::Resource))
                && dir.is_dir()
            {
                let mut files = Vec::new();
                if walk_names(&dir, &rel, 0, &mut files).is_ok() {
                    out.extend(
                        files
                            .into_iter()
                            .filter(|p| matches!(paths::classify(p), Ok(DocKind::Resource))),
                    );
                }
            }
            continue;
        }
        if kind != Some("RegisteredResources") {
            continue;
        }
        for item in package
            .get("items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(p) = item.get("path").and_then(Value::as_str) {
                let rel = format!("StaticResources/{name}/{}", paths::normalize(p));
                if matches!(paths::classify(&rel), Ok(DocKind::Resource)) {
                    out.push(rel);
                }
            }
        }
    }
    out
}

/// Writes one document unless the file already holds exactly these bytes.
fn write_document(root: &Path, rel: &str, bytes: &[u8]) -> Result<(), Error> {
    let path = join(root, rel);
    if let Some(parent) = path.parent() {
        // Safe when several threads create the same folders.
        std::fs::create_dir_all(parent).map_err(|e| io(parent, e))?;
    }
    let same = std::fs::read(&path).is_ok_and(|current| current == bytes);
    if !same {
        std::fs::write(&path, bytes).map_err(|e| io(&path, e))?;
    }
    Ok(())
}

pub(crate) fn write(root: &Path, docs: &[(String, Vec<u8>)]) -> Result<(), Error> {
    std::fs::create_dir_all(root).map_err(|e| io(root, e))?;
    let old_registered = registered_resources(root);
    let old_owned = owned_files(root)?;

    let mut written: HashSet<String> = HashSet::new();
    for (rel, _) in docs {
        match paths::classify(rel) {
            Ok(DocKind::Ignored) | Err(_) => {
                return Err(Error::InvalidPath {
                    path: rel.clone(),
                    message: "not a path inside the report content".into(),
                });
            }
            Ok(_) => {}
        }
        written.insert(rel.to_lowercase());
    }
    // Several threads: each document is one open for the comparison and one for the write.
    for result in parallel_map(docs, |(rel, bytes)| write_document(root, rel, bytes)) {
        result?;
    }

    // Stale page / visual / bookmark documents, and resources the old report registered.
    let stale: Vec<&String> = old_owned
        .iter()
        .chain(old_registered.iter())
        .filter(|p| !written.contains(&p.to_lowercase()))
        .collect();
    let mut dirs: Vec<PathBuf> = Vec::new();
    for rel in stale {
        let path = join(root, rel);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io(&path, e)),
        }
        // Folders that may become empty: up to (not including) `definition/pages` and
        // `StaticResources/<package>`.
        let parts: Vec<&str> = rel.split('/').collect();
        let keep = if parts.first() == Some(&"StaticResources") {
            3
        } else {
            2
        };
        for n in (keep..parts.len()).rev() {
            if let Some(prefix) = parts.get(..n)
                && !(n == 2 && prefix == ["definition", "pages"])
            {
                dirs.push(join(root, &prefix.join("/")));
            }
        }
    }
    dirs.sort_by(|a, b| {
        b.components()
            .count()
            .cmp(&a.components().count())
            .then_with(|| a.cmp(b))
    });
    dirs.dedup();
    for d in dirs {
        // Fails (and is ignored) when the folder is not empty.
        let _ = std::fs::remove_dir(&d);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tempdir::TempDir;

    /// A guarded temp folder and a path inside it that does not exist yet.
    fn temp(tag: &str) -> (TempDir, PathBuf) {
        let tmp = TempDir::new(&format!("pbir-fs-{tag}"));
        let dir = tmp.join("out");
        (tmp, dir)
    }

    #[test]
    fn parallel_map_keeps_the_order() {
        let items: Vec<usize> = (0..1000).collect();
        let doubled: Vec<usize> = items.iter().map(|x| x * 2).collect();
        assert_eq!(parallel_map(&items, |x| x * 2), doubled);
        assert_eq!(parallel_map(&items[..3], |x| *x), [0, 1, 2]);
        assert!(parallel_map(&items[..0], |x| *x).is_empty());
    }

    /// G23: the level-by-level, multi-threaded read returns what a sequential walk would.
    #[test]
    fn wide_trees_are_read_completely_and_sorted() {
        let (_tmp, root) = temp("wide");
        for p in 0..40 {
            for v in 0..3 {
                let dir = root.join(format!("definition/pages/p{p:02}/visuals/v{v}"));
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(dir.join("visual.json"), format!("{p}-{v}")).unwrap();
            }
            std::fs::write(root.join(format!("definition/pages/p{p:02}/page.json")), "{}").unwrap();
        }
        std::fs::create_dir_all(root.join("StaticResources")).unwrap();
        std::fs::write(root.join("StaticResources/x.png"), [1u8, 2]).unwrap();
        std::fs::write(root.join("ignored.txt"), "not content").unwrap();
        let docs = collect(&root).unwrap();
        assert_eq!(docs.len(), 40 * 4 + 1);
        assert!(docs.windows(2).all(|w| w[0].0 < w[1].0));
        let v = docs
            .iter()
            .find(|(p, _)| p == "definition/pages/p07/visuals/v2/visual.json")
            .unwrap();
        assert_eq!(v.1, b"7-2");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A custom visual the old report registered and the new one no longer writes leaves its
    /// `CustomVisuals/<name>/` folder; a folder no package registered stays.
    #[test]
    fn a_dropped_custom_visual_loses_its_files() {
        let (_tmp, root) = temp("cv");
        let report = serde_json::json!({"resourcePackages": [
            {"name": "cvA", "type": "CustomVisual", "items": [{"name": "cvA.pbiviz.json", "path": "cvA.pbiviz.json", "type": "CustomVisualMetadata"}]}
        ]});
        std::fs::create_dir_all(root.join("definition")).unwrap();
        std::fs::write(root.join("definition/report.json"), report.to_string()).unwrap();
        for dir in ["CustomVisuals/cvA/resources", "CustomVisuals/stray"] {
            std::fs::create_dir_all(join(&root, dir)).unwrap();
        }
        std::fs::write(root.join("CustomVisuals/cvA/package.json"), "{}").unwrap();
        std::fs::write(root.join("CustomVisuals/cvA/resources/cvA.pbiviz.json"), "{}").unwrap();
        std::fs::write(root.join("CustomVisuals/stray/package.json"), "{}").unwrap();
        write(&root, &[("definition/report.json".into(), b"{}".to_vec())]).unwrap();
        assert!(!root.join("CustomVisuals/cvA").exists());
        assert!(root.join("CustomVisuals/stray/package.json").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn deep_trees_are_refused() {
        let (_tmp, root) = temp("deep");
        let mut dir = root.join("definition");
        for i in 0..=MAX_DEPTH + 1 {
            dir = dir.join(format!("d{i}"));
        }
        std::fs::create_dir_all(&dir).unwrap();
        assert!(matches!(collect(&root), Err(Error::InvalidPath { .. })));
        let _ = std::fs::remove_dir_all(&root);
    }
}
