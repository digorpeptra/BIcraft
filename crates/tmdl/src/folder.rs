//! Writing TMDL documents into a `definition/` folder.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::par::parallel_map;
use crate::{Error, MAX_FILE_BYTES, collect_files};

pub(crate) fn write(docs: &[(String, String)], dir: &Path) -> Result<(), Error> {
    for (relative, _) in docs {
        check_relative(relative)?;
    }
    std::fs::create_dir_all(dir).map_err(|e| Error::Io(dir.to_path_buf(), e))?;

    // Sub-folders once each (paths have at most two components), then the files on several
    // threads: on Windows each one costs a few opens.
    let mut folders = HashSet::new();
    for (relative, _) in docs {
        if let Some((folder, _)) = relative.split_once('/')
            && folders.insert(folder)
        {
            ensure_dir(&dir.join(folder))?;
        }
    }
    for result in parallel_map(docs, |(relative, text)| {
        write_file(&join(dir, relative), text.as_bytes())
    }) {
        result?;
    }

    // Remove `.tmdl` files the model no longer produces. A file listed under a produced name is
    // kept; one whose name matches a produced one only when case is ignored is compared by
    // identity (canonical paths), so a case-only rename on a case-insensitive file system is
    // kept while a stale file on a case-sensitive one is removed.
    let written: HashSet<&str> = docs.iter().map(|(r, _)| r.as_str()).collect();
    let mut by_lowercase: HashMap<String, Vec<&str>> = HashMap::new();
    for (relative, _) in docs {
        by_lowercase
            .entry(relative.to_lowercase())
            .or_default()
            .push(relative);
    }
    let mut existing = Vec::new();
    collect_files(dir, dir, 0, &mut existing)?;
    for (relative, path, _) in existing {
        if written.contains(relative.as_str()) {
            continue;
        }
        let same_file = match by_lowercase.get(&relative.to_lowercase()) {
            None => false,
            Some(candidates) => {
                let this = canonical(&path)?;
                let mut same = false;
                for c in candidates {
                    same |= canonical(&join(dir, c))? == this;
                }
                same
            }
        };
        if !same_file {
            std::fs::remove_file(&path).map_err(|e| Error::Io(path.clone(), e))?;
        }
    }
    Ok(())
}

fn join(dir: &Path, relative: &str) -> PathBuf {
    relative
        .split('/')
        .fold(dir.to_path_buf(), |p, part| p.join(part))
}

/// A produced path must be one or two plain components ending in `.tmdl`.
fn check_relative(relative: &str) -> Result<(), Error> {
    let parts: Vec<&str> = relative.split('/').collect();
    let ok = (1..=2).contains(&parts.len())
        && parts.iter().all(|p| {
            !p.is_empty()
                && *p != "."
                && *p != ".."
                && !p.contains(['\\', ':', '/'])
                && !p.chars().any(char::is_control)
        })
        && relative.ends_with(".tmdl");
    if ok {
        Ok(())
    } else {
        Err(Error::UnsafePath(relative.to_string()))
    }
}

/// Creates a sub-folder, refusing to write through a symbolic link.
fn ensure_dir(path: &Path) -> Result<(), Error> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => Err(Error::Symlink(path.to_path_buf())),
        Ok(meta) if meta.is_dir() => Ok(()),
        Ok(_) => Err(Error::Io(
            path.to_path_buf(),
            std::io::Error::new(std::io::ErrorKind::AlreadyExists, "not a directory"),
        )),
        Err(_) => std::fs::create_dir(path).map_err(|e| Error::Io(path.to_path_buf(), e)),
    }
}

/// Writes through a temporary file renamed into place; an identical file is left untouched.
fn write_file(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            return Err(Error::Symlink(path.to_path_buf()));
        }
        let same_len = u64::try_from(bytes.len()).is_ok_and(|n| n == meta.len());
        if meta.is_file()
            && same_len
            && meta.len() <= MAX_FILE_BYTES
            && std::fs::read(path).is_ok_and(|old| old == bytes)
        {
            return Ok(());
        }
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.bicraft-tmp"));
    std::fs::write(&tmp, bytes).map_err(|e| Error::Io(tmp.clone(), e))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        Error::Io(path.to_path_buf(), e)
    })
}

fn canonical(path: &Path) -> Result<PathBuf, Error> {
    std::fs::canonicalize(path).map_err(|e| Error::Io(path.to_path_buf(), e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsafe_relative_paths_are_refused() {
        for bad in [
            "../x.tmdl",
            "tables/../x.tmdl",
            "/x.tmdl",
            "a/b/c.tmdl",
            "x.txt",
            "tables\\x.tmdl",
            "c:x.tmdl",
            "",
            "tables/",
            "a\u{0}.tmdl",
        ] {
            assert!(check_relative(bad).is_err(), "{bad:?}");
        }
        for good in ["model.tmdl", "tables/a%2Fb.tmdl", "tables/...tmdl"] {
            assert!(check_relative(good).is_ok(), "{good:?}");
        }
    }
}
