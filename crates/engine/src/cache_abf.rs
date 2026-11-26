//! Where a PBIP project's stored data comes from: its own `<Name>.SemanticModel/.pbi/cache.abf`,
//! or an explicit file named by the caller (`file.open {cache_abf}`, `file.set_cache_abf`, or a
//! `cache_abf` parameter on the commands that read the data).
//!
//! The explicit file lets a throwaway copy of a project read the original's cache without copying
//! or linking it. It wins over the discovered one, is opened read-only and never written: saves
//! only write the project's model and report, and `file.save_as` refuses a project whose cache
//! would land on it. A missing or unreadable explicit file is an error, never a silent fallback.

use std::fs::File;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::params::allow;
use crate::{Session, Source};

/// The cache file the session reads: the explicit one when set (an error when it cannot be read),
/// else the project's `.pbi/cache.abf` when it exists. `Ok(None)`: no project open, or no cache.
pub(crate) fn path(s: &Session) -> Result<Option<PathBuf>, String> {
    let Some(Source::Project { project, .. }) = &s.source else {
        return Ok(None);
    };
    if let Some(explicit) = &s.cache_abf {
        return check(explicit).map(Some);
    }
    Ok(discovered(project).filter(|p| p.is_file()))
}

/// The bytes of the cache [`path`] names (capped like a project's own cache).
pub(crate) fn read(s: &Session) -> Result<Option<Vec<u8>>, String> {
    let Some(p) = path(s)? else {
        return Ok(None);
    };
    let len = std::fs::metadata(&p)
        .map_err(|e| format!("cache_abf {}: {e}", p.display()))?
        .len();
    if len > bicraft_pbip::MAX_CACHE_BYTES {
        return Err(format!("cache_abf {}: {len} bytes is too large", p.display()));
    }
    std::fs::read(&p)
        .map(Some)
        .map_err(|e| format!("cache_abf {}: {e}", p.display()))
}

/// Where the project's own cache is (or would be).
fn discovered(project: &bicraft_pbip::Project) -> Option<PathBuf> {
    bicraft_pbip::cache_abf_path(project)
}

/// How a result names the cache it read: `.pbi/cache.abf`, or `cache_abf` for an explicit one.
pub(crate) fn label(s: &Session) -> &'static str {
    if s.cache_abf.is_some() {
        "cache_abf"
    } else {
        ".pbi/cache.abf"
    }
}

/// The explicit file, opened read-only to prove it can be read.
fn check(path: &Path) -> Result<PathBuf, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("cache_abf {}: {e}", path.display()))?;
    if !meta.is_file() {
        return Err(format!("cache_abf {}: not a file", path.display()));
    }
    File::open(path).map_err(|e| format!("cache_abf {}: cannot be read: {e}", path.display()))?;
    Ok(path.to_path_buf())
}

/// The `cache_abf` parameter: `None` when absent, `Some(None)` for null or "", else the checked
/// absolute path.
pub(crate) fn param(p: &Value) -> Result<Option<Option<PathBuf>>, String> {
    match p.get("cache_abf") {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(t)) if t.trim().is_empty() => Ok(Some(None)),
        Some(Value::String(t)) => {
            let abs = std::path::absolute(t).map_err(|e| format!("cache_abf `{t}`: {e}"))?;
            check(&abs).map(|p| Some(Some(p)))
        }
        Some(_) => Err("parameter `cache_abf` must be a file path (or null)".into()),
    }
}

/// An explicit cache only applies to a PBIP project.
pub(crate) fn require_project(source: Option<&Source>) -> Result<(), String> {
    match source {
        Some(Source::Project { .. }) => Ok(()),
        _ => Err("`cache_abf` applies to a PBIP project only (a PBIX keeps its data in the file)".into()),
    }
}

/// The parameters without `cache_abf` (for code that checks its own allowed keys).
fn without(p: &Value) -> Value {
    match p {
        Value::Object(o) if o.contains_key("cache_abf") => {
            let mut o = o.clone();
            o.remove("cache_abf");
            Value::Object(o)
        }
        other => other.clone(),
    }
}

/// Refuses a write of a project cache onto the session's explicit `cache_abf`.
pub(crate) fn refuse_overwrite(s: &Session, target: &Path) -> Result<(), String> {
    let Some(explicit) = &s.cache_abf else {
        return Ok(());
    };
    let same = |a: &Path, b: &Path| match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => std::path::absolute(a).ok() == std::path::absolute(b).ok(),
    };
    if same(explicit, target) {
        return Err(format!(
            "{} is the session's cache_abf, which bicraft never writes: save to another folder",
            target.display()
        ));
    }
    Ok(())
}

/// Runs a data-reading command with its `cache_abf` parameter (when given) as the session's for
/// that command only; an explicit cache, given or set, is checked first.
fn with_param(
    s: &mut Session,
    p: &Value,
    run: fn(&mut Session, &Value) -> Result<Value, String>,
) -> Result<Value, String> {
    let Some(given) = param(p)? else {
        path(s)?;
        return run(s, p);
    };
    if given.is_some() {
        require_project(s.source.as_ref())?;
    }
    let rest = without(p);
    let before = std::mem::replace(&mut s.cache_abf, given);
    s.refresh = Default::default();
    let out = run(s, &rest);
    s.cache_abf = before;
    s.refresh = Default::default();
    out
}

pub(crate) fn dax_evaluate(s: &mut Session, p: &Value) -> Result<Value, String> {
    with_param(s, p, |s, p| crate::preview::dax_evaluate(s, p))
}

pub(crate) fn report_render(s: &mut Session, p: &Value) -> Result<Value, String> {
    with_param(s, p, |s, p| crate::render::report_render(s, p))
}

pub(crate) fn storage_stats(s: &mut Session, p: &Value) -> Result<Value, String> {
    with_param(s, p, |s, p| crate::storage_stats::storage_stats(s, p))
}

pub(crate) fn export_vpax(s: &mut Session, p: &Value) -> Result<Value, String> {
    with_param(s, p, crate::vpax::export_vpax)
}

/// `file.set_cache_abf {cache_abf}`: the file the open project's data is read from (null: the
/// project's own `.pbi/cache.abf` again).
pub(crate) fn file_set_cache_abf(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["cache_abf"])?;
    let Some(given) = param(p)? else {
        return Err("`cache_abf` is required (a cache.abf file path, or null for the project's own)".into());
    };
    require_project(s.source.as_ref())?;
    s.cache_abf = given;
    s.refresh = Default::default();
    Ok(status(s))
}

/// `{cache_abf, project_cache_abf}` for results.
pub(crate) fn status(s: &Session) -> Value {
    let own = match &s.source {
        Some(Source::Project { project, .. }) => discovered(project)
            .filter(|p| p.is_file())
            .map(|p| p.display().to_string()),
        _ => None,
    };
    json!({
        "cache_abf": s.cache_abf.as_ref().map(|p| p.display().to_string()),
        "project_cache_abf": own,
    })
}

/// A data-reading command's shared run (`Session::execute_shared`, under a session read lock):
/// the same as [`with_param`] without a `cache_abf` parameter, which swaps session fields and so
/// always runs exclusively (`Session::runs_shared` routes it to [`Session::execute`]).
fn shared(
    s: &Session,
    p: &Value,
    run: fn(&Session, &Value) -> Result<Value, String>,
) -> Result<Value, String> {
    if param(p)?.is_some() {
        return Err("internal error: a `cache_abf` parameter runs exclusively".into());
    }
    path(s)?;
    run(s, p)
}

pub(crate) fn dax_evaluate_shared(s: &Session, p: &Value) -> Result<Value, String> {
    shared(s, p, crate::preview::dax_evaluate)
}

pub(crate) fn report_render_shared(s: &Session, p: &Value) -> Result<Value, String> {
    shared(s, p, crate::render::report_render)
}

pub(crate) fn storage_stats_shared(s: &Session, p: &Value) -> Result<Value, String> {
    shared(s, p, crate::storage_stats::storage_stats)
}
