//! L3: TMDL (Tabular Model Definition Language), the folder-of-text form of a tabular model used
//! by PBIP `*.SemanticModel/definition/`. Reading and writing, losslessly.
//!
//! Reading happens in two steps: [`parse`] turns each document into a generic tree of entries
//! nested by indentation, then every object is converted into the JSON object TMSL would hold for
//! it and the properties [`SemanticModel`] types are taken out; everything else stays in the
//! owning object's `extra` map (unmodelled child objects as TMSL-named JSON arrays). Malformed
//! syntax is an error naming file and line.
//!
//! Writing reverses this and produces the files Power BI Desktop writes, in its style: tabs,
//! CRLF line endings on every platform, Desktop's property order, quoting and expression fences.
//! A canonical TMDL folder reads and writes back byte for byte. See `docs/formats/tmdl.md`.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

mod folder;
mod map;
mod par;
mod parse;
mod schema;
mod script;
mod write;

use std::path::{Path, PathBuf};

use bicraft_model::SemanticModel;

pub use parse::{Document, Node, ValueKind, parse};
pub use script::{CREATE_OR_REPLACE, Script, ScriptEntry, ScriptMember, Scripted, read_script, script};

/// Largest single `.tmdl` file read. Input is hostile.
pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
/// Largest total size of all `.tmdl` files of one model.
pub const MAX_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
/// Most `.tmdl` files read from one folder tree.
pub const MAX_FILES: usize = 100_000;
/// Deepest sub-folder searched below `definition/` (the standard layout uses one level).
const MAX_DIR_DEPTH: usize = 4;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("{0} is larger than {MAX_FILE_BYTES} bytes")]
    TooLarge(PathBuf),
    #[error("{0}: the model's TMDL files exceed {MAX_TOTAL_BYTES} bytes or {MAX_FILES} files")]
    TooMuch(PathBuf),
    #[error("{0}: not valid UTF-8 text")]
    Encoding(String),
    #[error("{file}:{line}: {message}")]
    Syntax {
        file: String,
        line: usize,
        message: String,
    },
    #[error("refusing to write the unsafe path `{0}`")]
    UnsafePath(String),
    #[error("{0} is a symbolic link; refusing to write through it")]
    Symlink(PathBuf),
    #[error(
        "the object name {0:?} contains a control character or line break, which TMDL cannot hold \
         (Power BI Desktop refuses it); rename the object or save the model as TMSL (model.bim)"
    )]
    UnwritableName(String),
}

/// Reads a TMDL `definition/` folder: every `*.tmdl` file below it, in sorted path order.
pub fn read_folder(definition_dir: &Path) -> Result<SemanticModel, Error> {
    let mut files = Vec::new();
    collect_files(definition_dir, definition_dir, 0, &mut files)?;
    files.sort();
    // Sizes from the folder listing bound the work before anything is read.
    let mut total = 0u64;
    for (_, path, len) in &files {
        if *len > MAX_FILE_BYTES {
            return Err(Error::TooLarge(path.clone()));
        }
        total = total.saturating_add(*len);
        if total > MAX_TOTAL_BYTES {
            return Err(Error::TooMuch(definition_dir.to_path_buf()));
        }
    }
    let docs = par::parallel_map(&files, |(relative, path, len)| read_text(relative, path, *len))
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
    read_documents(&docs)
}

/// One `.tmdl` file as text: one open, at most [`MAX_FILE_BYTES`] read (the file may have grown
/// since it was listed).
fn read_text(relative: &str, path: &Path, len: u64) -> Result<(String, String), Error> {
    use std::io::Read as _;
    let file = std::fs::File::open(path).map_err(|e| Error::Io(path.to_path_buf(), e))?;
    let mut bytes = Vec::with_capacity(usize::try_from(len.min(MAX_FILE_BYTES)).unwrap_or(0));
    file.take(MAX_FILE_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| Error::Io(path.to_path_buf(), e))?;
    if u64::try_from(bytes.len()).map_or(true, |n| n > MAX_FILE_BYTES) {
        return Err(Error::TooLarge(path.to_path_buf()));
    }
    let text = String::from_utf8(bytes).map_err(|_| Error::Encoding(relative.to_string()))?;
    Ok((relative.to_string(), text))
}

/// Reads a model from in-memory documents given as `(relative path, text)` pairs, in order.
pub fn read_documents(docs: &[(String, String)]) -> Result<SemanticModel, Error> {
    // Documents parse independently (one per table), so on several threads; the first error
    // in document order is reported.
    let parsed = par::parallel_map(docs, |(file, text)| {
        parse(file, text).map(|doc| (file.clone(), doc))
    })
    .into_iter()
    .collect::<Result<Vec<_>, _>>()?;
    map::build(&parsed)
}

/// Collects `(relative path with '/' separators, full path, length)` for each `.tmdl` file; the
/// length comes from the folder listing. Symbolic links are not followed, so a link loop cannot
/// make the walk endless.
pub(crate) fn collect_files(
    root: &Path,
    dir: &Path,
    depth: usize,
    out: &mut Vec<(String, PathBuf, u64)>,
) -> Result<(), Error> {
    let entries = std::fs::read_dir(dir).map_err(|e| Error::Io(dir.to_path_buf(), e))?;
    for entry in entries {
        let entry = entry.map_err(|e| Error::Io(dir.to_path_buf(), e))?;
        let path = entry.path();
        let kind = entry.file_type().map_err(|e| Error::Io(path.clone(), e))?;
        if kind.is_dir() {
            if depth < MAX_DIR_DEPTH {
                collect_files(root, &path, depth.saturating_add(1), out)?;
            }
        } else if kind.is_file()
            && path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("tmdl"))
        {
            if out.len() >= MAX_FILES {
                return Err(Error::TooMuch(root.to_path_buf()));
            }
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            let len = entry.metadata().map_err(|e| Error::Io(path.clone(), e))?.len();
            out.push((relative, path, len));
        }
    }
    Ok(())
}

/// Serializes a model as TMDL documents: `(path relative to definition/, text)`, sorted by path,
/// e.g. `("tables/Sales.tmdl", …)`. The layout is Desktop's: `database.tmdl`, `model.tmdl`,
/// `relationships.tmdl`, `expressions.tmdl`, `dataSources.tmdl`, `functions.tmdl` (when not
/// empty) and one file per object in `tables/`, `roles/`, `perspectives/` and `cultures/`, named
/// by [`file_stem`]. Text is UTF-8 without BOM, tab-indented, with CRLF line endings.
pub fn write_documents(model: &SemanticModel) -> Vec<(String, String)> {
    write::documents(model)
}

/// Writes a model into a `definition/` folder: each file is written to a temporary name and
/// renamed into place (unchanged files are not touched), then `.tmdl` files the model no longer
/// produces are deleted. Other files are never touched, and nothing is written through a
/// symbolic link.
///
/// Fails without writing anything when an object name contains a control character or a line
/// break: TMDL has no escape for them and Desktop's reader refuses them (see [`unwritable_name`]).
pub fn write_folder(model: &SemanticModel, definition_dir: &Path) -> Result<(), Error> {
    if let Some(name) = unwritable_name(model) {
        return Err(Error::UnwritableName(name));
    }
    folder::write(&write_documents(model), definition_dir)
}

/// The first object name (table, column, measure, annotation, role member, ...) that TMDL cannot
/// hold: one containing a control character such as a tab or a line break. [`write_documents`]
/// still writes such names (bicraft reads them back) but Power BI Desktop refuses the files.
pub fn unwritable_name(model: &SemanticModel) -> Option<String> {
    write::unwritable_name(model)
}

/// The file name (without `.tmdl`) used for an object name: Desktop's rule, percent-encoding
/// `" * / : < > ? \ |` and a leading space, plus control characters and Windows device names;
/// stems are capped at 120 bytes. Clashes within a folder get a `~2`, `~3`… suffix.
pub fn file_stem(name: &str) -> String {
    write::file_stem(name)
}

#[cfg(test)]
#[path = "../../../testsupport/tempdir.rs"]
mod tempdir;
#[cfg(test)]
mod tests;
