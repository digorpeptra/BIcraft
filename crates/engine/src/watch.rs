//! Watching an opened file or project for changes made by other tools (`bicraft-cli view`, the
//! desktop app's live reload): which paths to watch, a fingerprint of their files, and whether a
//! snapshot still waits for data. Polling with `std::fs` only (no notification API), so it
//! behaves the same on Windows, Linux and network shares.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crate::{Session, Source};

/// How often front ends check the files.
pub const POLL: Duration = Duration::from_millis(150);
/// How long the files must stay unchanged before they are re-read (editors and Desktop write
/// projects file by file).
pub const SETTLE: Duration = Duration::from_millis(300);
/// Most files fingerprinted in a folder, and how deep it looks.
const MAX_WATCHED_FILES: usize = 20_000;
const MAX_WATCH_DEPTH: usize = 16;
/// Extensions of the files that make up a model or a report.
const WATCHED_EXTENSIONS: &[&str] = &["tmdl", "bim", "json", "pbir", "pbism", "pbip", "pbix", "pbit"];
/// Folders never descended into: Desktop's local cache and VCS metadata.
const SKIPPED_DIRS: &[&str] = &[".pbi", ".git", "node_modules", "StaticResources", "CustomVisuals"];

/// What to watch for `path`: the project's item folders once it is open, otherwise the path
/// itself (plus `<stem>.SemanticModel` / `<stem>.Report` next to a `.pbip` file).
pub fn watch_roots(path: &Path, session: &Session) -> Vec<PathBuf> {
    let mut roots = vec![path.to_path_buf()];
    if path.is_dir() {
        return roots;
    }
    if let Some(Source::Project { project, .. }) = &session.source {
        roots.extend(project.semantic_model.iter().cloned());
        roots.extend(project.report.iter().cloned());
    } else if path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("pbip"))
        && let (Some(parent), Some(stem)) = (path.parent(), path.file_stem().and_then(|s| s.to_str()))
    {
        roots.push(parent.join(format!("{stem}.SemanticModel")));
        roots.push(parent.join(format!("{stem}.Report")));
    }
    roots
}

/// A hash of the paths, sizes and modification times of the watched files.
pub fn fingerprint(roots: &[PathBuf]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut files = Vec::new();
    for root in roots {
        collect(root, 0, &mut files);
    }
    files.sort();
    files.hash(&mut hasher);
    hasher.finish()
}

fn collect(path: &Path, depth: usize, out: &mut Vec<(PathBuf, u64, u128)>) {
    if out.len() >= MAX_WATCHED_FILES {
        return;
    }
    let Ok(meta) = std::fs::metadata(path) else {
        // Missing (e.g. deleted mid-save) counts as a state of its own.
        out.push((path.to_path_buf(), u64::MAX, 0));
        return;
    };
    if meta.is_dir() {
        if depth >= MAX_WATCH_DEPTH {
            return;
        }
        let Ok(entries) = std::fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            let child = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
            if is_dir {
                if !SKIPPED_DIRS.iter().any(|s| name.eq_ignore_ascii_case(s)) {
                    collect(&child, depth + 1, out);
                }
            } else if watched(&child) {
                collect(&child, depth + 1, out);
            }
            if out.len() >= MAX_WATCHED_FILES {
                return;
            }
        }
    } else {
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos());
        out.push((path.to_path_buf(), meta.len(), modified));
    }
}

/// Model and report files, plus `.platform` item files.
fn watched(path: &Path) -> bool {
    path.file_name().and_then(|n| n.to_str()) == Some(".platform")
        || path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| WATCHED_EXTENSIONS.iter().any(|w| e.eq_ignore_ascii_case(w)))
}

/// Whether a visual with bound fields has no `data` yet (left over by the snapshot's time
/// budget): front ends take another snapshot until none is left.
pub fn data_pending(snap: &serde_json::Value) -> bool {
    let pages = snap.pointer("/report/pages").and_then(|p| p.as_array());
    pages.into_iter().flatten().any(|p| {
        let visuals = p.get("visuals").and_then(|v| v.as_array());
        visuals.into_iter().flatten().any(|v| {
            let bound = v
                .get("fields")
                .and_then(|f| f.as_array())
                .is_some_and(|f| !f.is_empty());
            bound && v.get("data").is_none_or(serde_json::Value::is_null)
        })
    })
}

/// Waits for the watched files to change and settle: call [`Watcher::poll`] every [`POLL`];
/// it answers `true` once a change has stayed the same for [`SETTLE`].
#[derive(Debug)]
pub struct Watcher {
    roots: Vec<PathBuf>,
    loaded: u64,
    pending: Option<(u64, Instant)>,
}

impl Watcher {
    /// Watches `path` as `session` opened it.
    pub fn new(path: &Path, session: &Session) -> Watcher {
        let roots = watch_roots(path, session);
        let loaded = fingerprint(&roots);
        Watcher {
            roots,
            loaded,
            pending: None,
        }
    }

    /// `true` when the files changed and settled since the last load: the caller re-reads them,
    /// then calls [`Watcher::reloaded`].
    pub fn poll(&mut self) -> bool {
        let now = fingerprint(&self.roots);
        if now == self.loaded {
            self.pending = None;
            return false;
        }
        match self.pending {
            Some((seen, since)) if seen == now => since.elapsed() >= SETTLE,
            _ => {
                self.pending = Some((now, Instant::now()));
                false
            }
        }
    }

    /// After a re-read: watch what `session` has open now.
    pub fn reloaded(&mut self, path: &Path, session: &Session) {
        *self = Watcher::new(path, session);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bicraft-watch-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn fingerprint_follows_model_and_report_files_only() {
        let d = temp("fingerprint");
        std::fs::create_dir_all(d.join("M.SemanticModel/definition")).unwrap();
        std::fs::create_dir_all(d.join("M.SemanticModel/.pbi")).unwrap();
        std::fs::write(d.join("M.SemanticModel/definition/model.tmdl"), "model Model\n").unwrap();
        let roots = vec![d.clone()];
        let a = fingerprint(&roots);
        assert_eq!(a, fingerprint(&roots));
        // Ignored: Desktop's cache and unrelated files.
        std::fs::write(d.join("M.SemanticModel/.pbi/cache.abf"), "x").unwrap();
        std::fs::write(d.join("notes.txt"), "x").unwrap();
        assert_eq!(a, fingerprint(&roots));
        // Followed: a new table file, then a changed size.
        std::fs::write(d.join("M.SemanticModel/definition/t.tmdl"), "table T\n").unwrap();
        let b = fingerprint(&roots);
        assert_ne!(a, b);
        std::fs::write(d.join("M.SemanticModel/definition/t.tmdl"), "table Tx\n").unwrap();
        assert_ne!(b, fingerprint(&roots));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn pbip_files_watch_their_item_folders() {
        let roots = watch_roots(Path::new("x/Sales.pbip"), &Session::new());
        assert_eq!(
            roots,
            vec![
                PathBuf::from("x/Sales.pbip"),
                PathBuf::from("x").join("Sales.SemanticModel"),
                PathBuf::from("x").join("Sales.Report"),
            ]
        );
    }

    #[test]
    fn a_change_is_reported_once_it_settles() {
        let d = temp("settle");
        let f = d.join("m.bim");
        std::fs::write(&f, "{}").unwrap();
        let s = Session::new();
        let mut w = Watcher::new(&f, &s);
        assert!(!w.poll());
        std::fs::write(&f, "{  }").unwrap();
        assert!(!w.poll(), "the first sight of a change waits for it to settle");
        w.pending = w
            .pending
            .map(|(h, t)| (h, t.checked_sub(SETTLE * 2).unwrap_or(t)));
        assert!(w.poll());
        w.reloaded(&f, &s);
        assert!(!w.poll());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn visuals_without_data_are_pending() {
        // a bound visual left `data: null` by the snapshot budget is still pending; field-less
        // visuals and computed ones are not; hostile shapes are not pending
        let snap = |data: serde_json::Value, fields: serde_json::Value| json!({"report": {"pages": [{"visuals": [{"fields": fields, "data": data}]}]}});
        let f = json!([{"name": "Total"}]);
        assert!(data_pending(&snap(json!(null), f.clone())));
        assert!(!data_pending(&snap(json!({"status": "ok"}), f.clone())));
        assert!(!data_pending(&snap(json!({"status": "unsupported"}), f)));
        assert!(!data_pending(&snap(json!(null), json!([]))));
        assert!(!data_pending(&json!({"report": {"pages": "x"}})));
        assert!(!data_pending(&json!(null)));
    }
}
