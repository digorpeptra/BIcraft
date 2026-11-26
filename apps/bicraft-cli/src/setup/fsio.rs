//! Reading, backing up and atomically replacing client configuration files.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Configuration files larger than this are refused (Claude Code's `~/.claude.json` is a few MB).
const MAX_BYTES: u64 = 64 * 1024 * 1024;

/// The file's bytes, or `None` if it does not exist.
pub fn read(path: &Path) -> io::Result<Option<Vec<u8>>> {
    let meta = match fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    if !meta.is_file() {
        return Err(io::Error::other("not a regular file"));
    }
    if meta.len() > MAX_BYTES {
        return Err(io::Error::other(format!("larger than {MAX_BYTES} bytes")));
    }
    fs::read(path).map(Some)
}

/// `<file>.bicraft.bak`.
pub fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".bicraft.bak");
    path.with_file_name(name)
}

/// Copies the current bytes to `<file>.bicraft.bak` (replacing an older backup).
pub fn backup(path: &Path, bytes: &[u8]) -> io::Result<PathBuf> {
    let to = backup_path(path);
    write_atomic(&to, bytes)?;
    Ok(to)
}

/// Writes `bytes` to a temporary file next to `path`, then renames it over `path`. A symbolic
/// link is followed, so the file it points to is replaced, not the link. The permissions of an
/// existing file are kept.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let target = match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => fs::canonicalize(path)?,
        _ => path.to_path_buf(),
    };
    let dir = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    fs::create_dir_all(&dir)?;
    let mut tmp_name = std::ffi::OsString::from(".");
    tmp_name.push(target.file_name().unwrap_or_default());
    tmp_name.push(format!(".{}.bicraft.tmp", std::process::id()));
    let tmp = dir.join(tmp_name);
    let result = (|| {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        if let Ok(meta) = fs::metadata(&target) {
            fs::set_permissions(&tmp, meta.permissions())?;
        }
        fs::rename(&tmp, &target)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}
