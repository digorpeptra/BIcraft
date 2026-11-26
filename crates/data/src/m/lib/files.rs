//! File access, sandboxed by the allow-list `SourceOptions::file_roots`: `File.Contents`,
//! `Folder.Files`, `Folder.Contents`. Also the list of external connectors, which are never
//! run (`Error::ExternalSource`).
//!
//! Every path is resolved (`..`, symbolic links) and must lie under one of the roots; a
//! listing never follows a link out of them and never recurses into a linked folder. Folder
//! listings are capped (entries count as rows, recursion by `max_depth`); file contents are
//! read only when used, each capped at `max_bytes`.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use super::{Builtin, arg, b, text};
use crate::m::eval::{Interp, MS_PER_DAY};
use crate::m::lib::table::make;
use crate::m::value::{Bin, Column, Kind, LazyFile, MType, Res, Value, err, fatal, unsupported};

pub(crate) static FUNCS: &[Builtin] = &[
    b("File.Contents", 1, 2, file_contents),
    b("Folder.Files", 1, 2, folder_files),
    b("Folder.Contents", 1, 2, folder_contents),
];

/// Days from 1899-12-30 (M's epoch) to 1970-01-01.
const UNIX_EPOCH_DAYS: i64 = 25_569;

fn outside(call: &str, path: &str, roots: &[PathBuf]) -> crate::Error {
    let shown: Vec<String> = roots.iter().map(|r| format!("`{}`", r.display())).collect();
    crate::Error::UnsupportedSource(format!(
        "`{call}(\"{path}\")` is outside the allowed file roots ({})",
        shown.join(", ")
    ))
}

/// The roots, resolved; a root that does not exist is refused.
fn resolved_roots(roots: &[PathBuf]) -> Res<Vec<PathBuf>> {
    let mut out = Vec::with_capacity(roots.len());
    for r in roots {
        match std::fs::canonicalize(r) {
            Ok(c) => out.push(c),
            Err(_) => {
                return fatal(crate::Error::UnsupportedSource(format!(
                    "the file root `{}` does not exist",
                    r.display()
                )));
            }
        }
    }
    Ok(out)
}

/// Resolves `path` (for function `call`) inside the allow-list. Returns the path as given
/// (joined to the first root when relative) and the resolved path. Anything outside the roots
/// (after resolving `..` and symbolic links) is a fatal `UnsupportedSource`; a missing path
/// inside a root is a catchable `DataSource.Error`.
fn sandboxed(interp: &Interp, call: &str, path: &str) -> Res<(PathBuf, PathBuf)> {
    let roots = interp.file_roots.clone();
    let Some(first) = roots.first() else {
        return unsupported(format!(
            "`{call}(\"{path}\")`: file access is disabled (no file root was given)"
        ));
    };
    let p = Path::new(path);
    let joined = if p.is_absolute() {
        p.to_path_buf()
    } else {
        first.join(p)
    };
    let resolved = resolved_roots(&roots)?;
    match std::fs::canonicalize(&joined) {
        Ok(c) if resolved.iter().any(|r| c.starts_with(r)) => Ok((joined, c)),
        Ok(_) => fatal(outside(call, path, &roots)),
        Err(e) => {
            // A missing path: refuse what could not be inside a root at all.
            let lexical_inside =
                !path.contains("..") && roots.iter().chain(resolved.iter()).any(|r| joined.starts_with(r));
            if lexical_inside {
                err("DataSource.Error", format!("Could not find '{path}': {e}."))
            } else {
                fatal(outside(call, path, &roots))
            }
        }
    }
}

/// The resolved path and size of the file `File.Contents(path)` would read, with the same
/// sandbox checks and errors, without reading it (the fast CSV path streams it).
pub(crate) fn file_to_stream(interp: &Interp, path: &str) -> Res<(PathBuf, u64)> {
    let (_, full) = sandboxed(interp, "File.Contents", path)?;
    let meta = match std::fs::metadata(&full) {
        Ok(m) => m,
        Err(e) => return err("DataSource.Error", format!("Could not read file '{path}': {e}.")),
    };
    if !meta.is_file() {
        return err("DataSource.Error", format!("'{path}' is not a file."));
    }
    Ok((full, meta.len()))
}

fn file_contents(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let path = text(arg(a, 0))?;
    let (_, full) = sandboxed(interp, "File.Contents", path)?;
    let max = interp.limits.max_bytes;
    let meta = match std::fs::metadata(&full) {
        Ok(m) => m,
        Err(e) => return err("DataSource.Error", format!("Could not read file '{path}': {e}.")),
    };
    if !meta.is_file() {
        return err("DataSource.Error", format!("'{path}' is not a file."));
    }
    if meta.len() > max as u64 {
        return fatal(crate::Error::LimitExceeded(format!(
            "file `{path}` is larger than {max} bytes"
        )));
    }
    match std::fs::read(&full) {
        Ok(bytes) => {
            interp.tick((bytes.len() / 64) as u64)?;
            Ok(Value::bytes(bytes))
        }
        Err(e) => err("DataSource.Error", format!("Could not read file '{path}': {e}.")),
    }
}

// ---------------------------------------------------------------------------
// Folders.

/// One directory entry, after the sandbox checks.
struct Entry {
    name: String,
    /// The path as listed (under the path the query gave).
    shown: PathBuf,
    /// The resolved path (for reading).
    resolved: PathBuf,
    meta: std::fs::Metadata,
    is_dir: bool,
    /// A symbolic link (or junction).
    link: bool,
}

/// The entries of `dir` (shown as `shown`), sorted by name ignoring case. Links that resolve
/// outside the roots are skipped; linked folders are listed as entries but never entered.
fn entries(interp: &mut Interp, roots: &[PathBuf], dir: &Path, shown: &Path) -> Res<Vec<(Entry, bool)>> {
    let read = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(e) => {
            return err(
                "DataSource.Error",
                format!("Could not read folder '{}': {e}.", shown.display()),
            );
        }
    };
    let mut out = Vec::new();
    for item in read {
        interp.tick(1)?;
        let Ok(item) = item else { continue };
        let name = item.file_name().to_string_lossy().into_owned();
        let path = item.path();
        let Ok(lmeta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        let link = lmeta.file_type().is_symlink();
        let Ok(resolved) = std::fs::canonicalize(&path) else {
            continue;
        };
        if !roots.iter().any(|r| resolved.starts_with(r)) {
            continue;
        }
        let Ok(meta) = std::fs::metadata(&resolved) else {
            continue;
        };
        let is_dir = meta.is_dir();
        out.push((
            Entry {
                shown: shown.join(&name),
                name,
                resolved,
                meta,
                is_dir,
                link,
            },
            link,
        ));
        interp.check_rows(out.len())?;
    }
    out.sort_by(|(x, _), (y, _)| {
        x.name
            .to_lowercase()
            .cmp(&y.name.to_lowercase())
            .then_with(|| x.name.cmp(&y.name))
    });
    Ok(out)
}

/// A file time as an M `datetime` (UTC; bicraft reads no time zone).
fn file_time(t: std::io::Result<std::time::SystemTime>) -> Value {
    let Ok(t) = t else { return Value::Null };
    let ms = match t.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_millis()).ok(),
        Err(e) => i64::try_from(e.duration().as_millis()).ok().map(|m| -m),
    };
    match ms.and_then(|m| UNIX_EPOCH_DAYS.checked_mul(MS_PER_DAY)?.checked_add(m)) {
        Some(ms) => Value::DateTime(ms),
        None => Value::Null,
    }
}

/// The extension as Power Query reports it: from the last dot of the name, in lower case (`b.CSV`
/// → `.csv`, Desktop 2.158); empty for folders and names without a dot.
fn extension(name: &str, is_dir: bool) -> String {
    if is_dir {
        return String::new();
    }
    match name.rfind('.') {
        Some(i) => name.get(i..).unwrap_or("").to_lowercase(),
        None => String::new(),
    }
}

/// `Content Type` and `Kind` of the `Attributes` record, from the extension.
fn content_kind(ext: &str, is_dir: bool) -> (&'static str, &'static str) {
    if is_dir {
        return ("", "Folder");
    }
    // Desktop 2.158: `.csv`, `.txt`, `.json` and names without an extension as below; the
    // others follow the same pattern (not checked).
    match ext {
        ".csv" => ("text/csv", "CSV File"),
        "" | ".txt" | ".log" => ("text/plain", "Text File"),
        ".json" => ("application/json", "JSON File"),
        ".xml" => ("text/xml", "XML File"),
        ".htm" | ".html" => ("text/html", "HTML File"),
        ".xlsx" => (
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "Excel File",
        ),
        ".xls" => ("application/vnd.ms-excel", "Excel File"),
        ".pdf" => ("application/pdf", "PDF File"),
        _ => ("", "File"),
    }
}

/// File attributes: Windows' own flags; elsewhere, read-only from the permissions and hidden
/// for dot files.
#[cfg(windows)]
fn flags(e: &Entry) -> [bool; 14] {
    use std::os::windows::fs::MetadataExt;
    let a = e.meta.file_attributes();
    let bit = |m: u32| a & m != 0;
    [
        bit(0x1),
        bit(0x2),
        bit(0x4),
        bit(0x10),
        bit(0x20),
        bit(0x40),
        bit(0x80),
        bit(0x100),
        bit(0x200),
        bit(0x400),
        bit(0x800),
        bit(0x1000),
        bit(0x2000),
        bit(0x4000),
    ]
}

#[cfg(not(windows))]
fn flags(e: &Entry) -> [bool; 14] {
    let ro = e.meta.permissions().readonly();
    let hidden = e.name.starts_with('.');
    [
        ro, hidden, false, e.is_dir, !e.is_dir, false, false, false, false, false, false, false, false, false,
    ]
}

fn attributes(e: &Entry) -> Value {
    let ext = extension(&e.name, e.is_dir);
    let (ctype, kind) = content_kind(&ext, e.is_dir);
    let f = flags(e);
    let size = if e.is_dir {
        Value::Null
    } else {
        Value::num(e.meta.len() as f64)
    };
    let names = [
        "ReadOnly",
        "Hidden",
        "System",
        "Directory",
        "Archive",
        "Device",
        "Normal",
        "Temporary",
        "SparseFile",
        "ReparsePoint",
        "Compressed",
        "Offline",
        "NotContentIndexed",
        "Encrypted",
    ];
    let mut fields: Vec<(&str, Value)> = vec![
        ("Content Type", Value::text(ctype)),
        ("Kind", Value::text(kind)),
        ("Size", size),
    ];
    for (n, v) in names.iter().zip(f) {
        fields.push((n, Value::Logical(v)));
    }
    // As Desktop 2.158 lists them; the change time is read as the modification time.
    fields.push(("ChangeTime", file_time(e.meta.modified())));
    fields.push(("SymbolicLink", Value::Logical(e.link)));
    fields.push(("MountPoint", Value::Logical(false)));
    Value::record(fields)
}

fn folder_columns() -> Vec<Column> {
    let col = |name: &str, k: Kind| Column {
        name: Rc::from(name),
        ty: MType::prim(k),
    };
    vec![
        col("Content", Kind::Binary),
        col("Name", Kind::Text),
        col("Extension", Kind::Text),
        col("Date accessed", Kind::DateTime),
        col("Date modified", Kind::DateTime),
        col("Date created", Kind::DateTime),
        col("Attributes", Kind::Record),
        col("Folder Path", Kind::Text),
    ]
}

/// The `Folder Path` text of a folder: its path with a trailing separator.
fn folder_path(dir: &Path) -> String {
    let mut s = dir.display().to_string();
    // Windows paths are spelled with backslashes, whatever separator the query used.
    if cfg!(windows) {
        s = s.replace('/', "\\");
    }
    if !s.ends_with(std::path::MAIN_SEPARATOR) && !s.ends_with('/') {
        s.push(std::path::MAIN_SEPARATOR);
    }
    s
}

fn row(e: &Entry, content: Value, parent: &Path) -> Vec<Value> {
    vec![
        content,
        Value::text(e.name.as_str()),
        Value::text(extension(&e.name, e.is_dir)),
        file_time(e.meta.accessed()),
        file_time(e.meta.modified()),
        file_time(e.meta.created()),
        attributes(e),
        Value::text(folder_path(parent)),
    ]
}

fn lazy(interp: &Interp, e: &Entry) -> Value {
    Value::Binary(Rc::new(Bin::file(LazyFile {
        path: e.resolved.clone(),
        max: interp.limits.max_bytes,
    })))
}

/// The folder a `Folder.*` call lists: resolved inside the roots, and a folder.
fn folder_arg(interp: &Interp, call: &str, a: &[Value]) -> Res<(PathBuf, PathBuf, Vec<PathBuf>)> {
    let path = text(arg(a, 0))?;
    let (shown, resolved) = sandboxed(interp, call, path)?;
    if !resolved.is_dir() {
        return err("DataSource.Error", format!("'{path}' is not a folder."));
    }
    let roots = resolved_roots(&interp.file_roots)?;
    Ok((shown, resolved, roots))
}

/// `Folder.Files(path)`: every file under the folder and its subfolders, with a link to its
/// content (read when used).
fn folder_files(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let (shown, resolved, roots) = folder_arg(interp, "Folder.Files", a)?;
    let mut rows = Vec::new();
    // Depth-first, files of a folder before its subfolders, each sorted by name.
    let mut stack: Vec<(PathBuf, PathBuf, usize)> = vec![(resolved, shown, 0)];
    while let Some((dir, shown_dir, depth)) = stack.pop() {
        let list = entries(interp, &roots, &dir, &shown_dir)?;
        let mut subdirs = Vec::new();
        for (e, link) in list {
            if e.is_dir {
                if !link {
                    subdirs.push((e.resolved, e.shown));
                }
                continue;
            }
            let content = lazy(interp, &e);
            rows.push(row(&e, content, &shown_dir));
            interp.check_rows(rows.len())?;
        }
        if !subdirs.is_empty() && depth >= interp.limits.max_depth {
            return fatal(crate::Error::LimitExceeded(format!(
                "`Folder.Files` nests deeper than {} folders",
                interp.limits.max_depth
            )));
        }
        for (d, s) in subdirs.into_iter().rev() {
            stack.push((d, s, depth.saturating_add(1)));
        }
    }
    make(interp, folder_columns(), rows)
}

/// `Folder.Contents(path)`: the files and folders directly in the folder; a folder's
/// `Content` is its own contents table.
fn folder_contents(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let (shown, resolved, roots) = folder_arg(interp, "Folder.Contents", a)?;
    contents(interp, &roots, &resolved, &shown, 0)
}

fn contents(interp: &mut Interp, roots: &[PathBuf], dir: &Path, shown: &Path, depth: usize) -> Res<Value> {
    if depth > interp.limits.max_depth {
        return fatal(crate::Error::LimitExceeded(format!(
            "`Folder.Contents` nests deeper than {} folders",
            interp.limits.max_depth
        )));
    }
    let list = entries(interp, roots, dir, shown)?;
    let mut rows = Vec::with_capacity(list.len());
    for (e, link) in list {
        let content = if !e.is_dir {
            lazy(interp, &e)
        } else if link {
            Value::Null
        } else {
            contents(interp, roots, &e.resolved, &e.shown, depth.saturating_add(1))?
        };
        rows.push(row(&e, content, shown));
    }
    make(interp, folder_columns(), rows)
}

// ---------------------------------------------------------------------------
// External connectors.

/// What an external connector reaches, by its library namespace: `Some` for the connectors
/// bicraft never runs (databases, services, the web, scripts).
pub(crate) fn external(name: &str) -> Option<&'static str> {
    let ns = name.split('.').next().unwrap_or(name);
    Some(match ns {
        "Sql"
        | "Oracle"
        | "PostgreSQL"
        | "MySQL"
        | "MariaDB"
        | "Odbc"
        | "OleDb"
        | "AdoDotNet"
        | "DB2"
        | "Teradata"
        | "Sybase"
        | "Informix"
        | "SapHana"
        | "SapBusinessWarehouse"
        | "Snowflake"
        | "GoogleBigQuery"
        | "AmazonRedshift"
        | "AmazonAthena"
        | "Databricks"
        | "DatabricksMultiCloud"
        | "Impala"
        | "Vertica"
        | "Spark"
        | "Hive"
        | "Exasol"
        | "Denodo"
        | "Netezza"
        | "AnalysisServices"
        | "Essbase"
        | "AzureDataExplorer"
        | "Kusto"
        | "AzureSynapse"
        | "Fabric"
        | "Lakehouse"
        | "Warehouse" => "a database or analytical server",
        "Web"
        | "OData"
        | "SharePoint"
        | "AzureStorage"
        | "AzureDataLakeStorage"
        | "AzureBlobs"
        | "AzureTables"
        | "Salesforce"
        | "SalesforceObjects"
        | "SalesforceReports"
        | "Exchange"
        | "ActiveDirectory"
        | "PowerBI"
        | "PowerPlatform"
        | "Dataflows"
        | "CommonDataService"
        | "Dataverse"
        | "Cds"
        | "Dynamics365BusinessCentral"
        | "DynamicsNav"
        | "GoogleAnalytics"
        | "GoogleSheets"
        | "Hdfs"
        | "HdInsight"
        | "Facebook"
        | "GitHub"
        | "Smartsheet"
        | "Marketo"
        | "Zendesk"
        | "Dynamics"
        | "Kql"
        | "Cdm"
        | "ApacheSpark"
        | "Sap" => "a web service or online store",
        "Python" | "R" => "a script runtime",
        _ => return None,
    })
}

/// The error for an external connector: never evaluated.
pub(crate) fn external_error(name: &str, what: &str) -> crate::Error {
    crate::Error::ExternalSource(format!(
        "`{name}` reads {what}; bicraft never connects to external sources (the query is kept as \
         written)"
    ))
}
