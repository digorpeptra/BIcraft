//! Saving: `file.save`, `file.save_as`, `project.new`. The writers live in the format crates
//! (`bicraft_tmsl::write`, `bicraft_pbip::{create_project, save_model, save_report}`); this
//! module picks the target, refuses to clobber existing projects, and updates the session only
//! after a write succeeded.
//!
//! Reports: `file.save` writes the report of a PBIP project when it has unsaved edits (as PBIR,
//! converting a legacy `report.json` project). A new project (`file.save_as`, `project.new`) gets
//! the open report, or the blank report `create_project` writes, which is then read back.
//!
//! Packages: `file.save` on an open PBIX/PBIT (and `file.save_as` to a `.pbix`/`.pbit` of the
//! same kind) writes the unsaved edits back into the package: the model through
//! `bicraft_pbix::write_model` (metadata-only edits for a PBIX, the TMSL text for a PBIT) when
//! it has edits, the report through `bicraft_pbix::write_report` (in the form the package holds:
//! PBIR parts or the legacy `Report/Layout`) when it has edits. Every other part keeps its bytes;
//! an unedited model keeps its compressed `DataModel` as is. The file is replaced atomically
//! (temporary sibling, then rename) and `file.save` keeps the previous file as `<name>.bak`
//! unless `backup` is false. Any error leaves the file as it was.

use std::path::{Path, PathBuf};

use bicraft_model::SemanticModel;
use bicraft_pbip::{ModelFormat, ReportFormat};
use bicraft_report::Report;
use serde_json::{Map, Value, json};

use crate::commands::{file_inspect, model, source_path};
use crate::edit::empty_model;
use crate::params::{allow, opt_bool, opt_str, str_param};
use crate::{Session, Source};

/// Suffix of the temporary sibling a package is written to before it replaces the target.
const TMP_SUFFIX: &str = ".bicraft-tmp";
/// Suffix of the copy `file.save` keeps of the package it replaces.
const BACKUP_SUFFIX: &str = ".bak";

/// Where `file.save_as` writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Pbip(ModelFormat),
    Bim,
}

fn format_name(t: Target) -> &'static str {
    match t {
        Target::Pbip(ModelFormat::Tmsl) => "pbip-tmsl",
        Target::Pbip(_) => "pbip-tmdl",
        Target::Bim => "bim",
    }
}

fn extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
}

/// Writes `model` as TMSL to `path` through a temporary sibling, so a failed write never leaves a
/// half-written model.bim behind.
fn write_bim(path: &Path, model: &SemanticModel) -> Result<(), String> {
    let text = bicraft_tmsl::write(model);
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| format!("{}: not a file path", path.display()))?;
    let tmp = path.with_file_name(format!("{file_name}.bicraft-tmp"));
    std::fs::write(&tmp, text.as_bytes()).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {e}", path.display())
    })
}

/// `path` with `suffix` appended to its file name.
fn sibling(path: &Path, suffix: &str) -> Result<PathBuf, String> {
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| format!("{}: not a file path", path.display()))?;
    Ok(path.with_file_name(format!("{file_name}{suffix}")))
}

/// Replaces (or creates) `path` with `bytes` through a temporary sibling and a rename, so a
/// failure never leaves a half-written file. With `backup`, the current file is first copied
/// there. On error the target is untouched and the temporary file removed.
///
/// The package is written straight to the temporary file, so a part left in the source file
/// (too large for memory, `bicraft_opc::Part::deferred`) is copied from there piece by piece;
/// afterwards such parts are pointed at the new file.
fn replace_file(
    path: &Path,
    package: &mut bicraft_opc::Package,
    backup: Option<&Path>,
) -> Result<(), String> {
    replace_file_checked(path, package, backup, None)
}

/// Where `file.save` keeps the previous file (`<name>.bak`).
pub(crate) fn backup_path(path: &Path) -> Result<PathBuf, String> {
    sibling(path, BACKUP_SUFFIX)
}

/// A check of the written temporary file ([`replace_file_checked`]).
pub(crate) type Check<'a> = &'a dyn Fn(&Path) -> Result<(), String>;

/// [`replace_file`] with `check` run on the written temporary file before it replaces `path`
/// (`model.refresh` reopens it with bicraft's reader): an error there removes the temporary
/// file and leaves `path` untouched.
pub(crate) fn replace_file_checked(
    path: &Path,
    package: &mut bicraft_opc::Package,
    backup: Option<&Path>,
    check: Option<Check<'_>>,
) -> Result<(), String> {
    let tmp = sibling(path, TMP_SUFFIX)?;
    let cleanup = |e: String| {
        let _ = std::fs::remove_file(&tmp);
        e
    };
    let write = || -> Result<(), String> {
        let file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        let out = package
            .write_with(
                std::io::BufWriter::new(file),
                &bicraft_opc::WriteOptions::default(),
            )
            .map_err(|e| e.to_string())?;
        let file = out.into_inner().map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())
    };
    write().map_err(|e| cleanup(format!("{}: {e}", tmp.display())))?;
    if let Some(check) = check {
        check(&tmp).map_err(cleanup)?;
    }
    if let Some(bak) = backup {
        std::fs::copy(path, bak)
            .map_err(|e| cleanup(format!("cannot keep a backup in {}: {e}", bak.display())))?;
    }
    std::fs::rename(&tmp, path).map_err(|e| cleanup(format!("{}: {e}", path.display())))?;
    if package.has_deferred() {
        let source = bicraft_opc::FileSource::new(path).map_err(|e| format!("{}: {e}", path.display()))?;
        package
            .rebind_deferred(std::sync::Arc::new(source))
            .map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(())
}

/// What a package save writes: the model when it has unsaved edits, the report when it has
/// unsaved edits. A part nobody edited keeps its bytes; in particular an unedited model keeps
/// its compressed `DataModel` part as is (no XPress9 round trip).
pub(crate) struct Built {
    package: bicraft_opc::Package,
    /// The model as it reads back (new measures get their generated lineage tags), when written.
    model: Option<SemanticModel>,
    /// The report as it reads back, and what changed, when written.
    report: Option<(Report, bicraft_pbix::ReportWrite)>,
}

/// Builds the package with the session's unsaved edits written into it. Nothing is written to
/// disk.
#[cfg(test)]
pub(crate) fn build_package(
    package: &bicraft_opc::Package,
    model: Option<&SemanticModel>,
    report: Option<&Report>,
) -> Result<Built, String> {
    build_package_with(package, model, report, None)
}

/// [`build_package`] with the folder Power Query sources may read (`data_root`).
pub(crate) fn build_package_with(
    package: &bicraft_opc::Package,
    model: Option<&SemanticModel>,
    report: Option<&Report>,
    root: Option<&std::path::Path>,
) -> Result<Built, String> {
    let mut written = match model {
        Some(m) => crate::pbix_struct::write_model(package, m, root)?,
        None => package.clone(),
    };
    let mut report_out = None;
    if let Some(r) = report {
        let (next, change) = bicraft_pbix::write_report(&written, r).map_err(|e| {
            let also = if model.is_some() {
                " (nothing was saved, the model edits neither)"
            } else {
                ""
            };
            format!("the report edits cannot be written into this package{also}: {e}")
        })?;
        let back = bicraft_pbix::read_report(&next)
            .map_err(|e| format!("internal check failed: the written report does not read back: {e}"))?;
        written = next;
        report_out = Some((back, change));
    }
    let model_out = match model {
        Some(_) => Some(
            bicraft_pbix::read_model(&written)
                .map_err(|e| format!("internal check failed: the written package does not read back: {e}"))?,
        ),
        None => None,
    };
    Ok(Built {
        package: written,
        model: model_out,
        report: report_out,
    })
}

fn package_format(package: &bicraft_opc::Package) -> &'static str {
    match bicraft_pbix::inventory(package).kind {
        bicraft_pbix::Kind::Pbit => "pbit",
        bicraft_pbix::Kind::Pbix => "pbix",
        bicraft_pbix::Kind::Thin => "package",
    }
}

/// The session's unsaved model and report: what [`build_package`] writes.
fn pending(s: &Session) -> (Option<&SemanticModel>, Option<&Report>) {
    let model = s.model.as_ref().filter(|_| s.dirty);
    let report = s.report.as_ref().filter(|_| s.report_dirty);
    (model, report)
}

/// Commits a written package to the session; returns the save's `model_saved` / `report_*`
/// fields.
fn adopt_package(s: &mut Session, path: PathBuf, built: Built) -> Map<String, Value> {
    s.source = Some(Source::Package {
        path,
        package: built.package,
    });
    let mut out = Map::new();
    out.insert("model_saved".into(), json!(built.model.is_some()));
    if let Some(m) = built.model {
        s.model = Some(m);
        s.model_error = None;
        s.dirty = false;
    }
    out.insert("report_saved".into(), json!(built.report.is_some()));
    out.insert("report_converted_to_pbir".into(), json!(false));
    if let Some((report, change)) = built.report {
        s.report = Some(report);
        s.report_error = None;
        s.report_dirty = false;
        out.insert("report_format".into(), json!(change.format));
        out.insert(
            "report_parts".into(),
            json!({
                "changed": change.changed.len(),
                "added": change.added.len(),
                "removed": change.removed.len(),
            }),
        );
        out.insert(
            "security_bindings_removed".into(),
            json!(change.security_bindings_removed),
        );
    }
    out
}

fn save_warnings(built: &Built) -> Vec<String> {
    let mut warnings = Vec::new();
    if built
        .report
        .as_ref()
        .is_some_and(|(_, c)| c.security_bindings_removed)
    {
        warnings.push(
            "SecurityBindings was removed: it seals the report parts of the last Desktop save and no \
             longer matches them; Desktop opens the file without it and writes a new one when it saves"
                .to_string(),
        );
    }
    warnings
}

/// `file.save` for an open PBIX/PBIT.
fn save_package(s: &mut Session, backup: bool) -> Result<Value, String> {
    if s.dirty && s.model.is_none() {
        model(s)?;
    }
    let Some(Source::Package { path, package }) = s.source.as_ref() else {
        return Err("no package is open".into());
    };
    if crate::workbook_import::is_workbook(path) {
        return Err(crate::workbook_import::save_refused(path));
    }
    let (m, r) = pending(s);
    let mut built = build_package_with(package, m, r, s.data_root.as_deref())?;
    crate::diagram_layout::put_in_package(s, &mut built.package);
    let bak = if backup {
        Some(sibling(path, BACKUP_SUFFIX)?)
    } else {
        None
    };
    replace_file(path, &mut built.package, bak.as_deref())?;
    let path = path.clone();
    let mut warnings = save_warnings(&built);
    warnings.extend(crate::dax_queries::save(s)?);
    let mut out = Map::new();
    out.insert("saved".into(), json!(path.display().to_string()));
    out.insert("format".into(), json!(package_format(&built.package)));
    out.insert("backup".into(), json!(bak.map(|b| b.display().to_string())));
    out.insert("warnings".into(), json!(warnings));
    out.extend(adopt_package(s, path, built));
    Ok(Value::Object(out))
}

/// `file.save_as` to a `.pbix`/`.pbit`: the open package of the same kind, with the unsaved
/// model and report edits written into it, saved under a new name.
fn save_package_as(s: &mut Session, path: PathBuf, ext: &str, overwrite: bool) -> Result<Value, String> {
    let Some(Source::Package { package, .. }) = s.source.as_ref() else {
        return Err(format!(
            "saving as .{ext} needs an open .{ext}: the model is written into that package (file.save_as \
             does not build a PBIX or PBIT from scratch); use a .pbip or .bim target"
        ));
    };
    let kind = package_format(package);
    if kind != ext {
        return Err(format!(
            "the open file is a {kind}, so it can only be saved as a .{kind} (converting between PBIX and \
             PBIT is not supported by file.save_as)"
        ));
    }
    if path.is_dir() {
        return Err(format!("{} is a folder", path.display()));
    }
    if path.exists() && !overwrite {
        return Err(format!(
            "{} already exists; pass \"overwrite\": true to replace it",
            path.display()
        ));
    }
    if s.dirty && s.model.is_none() {
        model(s)?;
    }
    let Some(Source::Package { package, .. }) = s.source.as_ref() else {
        return Err("no package is open".into());
    };
    let (m, r) = pending(s);
    let mut built = build_package_with(package, m, r, s.data_root.as_deref())?;
    crate::diagram_layout::put_in_package(s, &mut built.package);
    create_dir(path.parent().unwrap_or(Path::new("")))?;
    replace_file(&path, &mut built.package, None)?;
    let mut out = Map::new();
    out.insert("saved".into(), json!(path.display().to_string()));
    out.insert("format".into(), json!(kind));
    out.insert("warnings".into(), json!(save_warnings(&built)));
    out.extend(adopt_package(s, path, built));
    Ok(Value::Object(out))
}

/// Adds `w` to the result's `warnings`.
fn push_warning(out: &mut Value, w: String) {
    let Some(o) = out.as_object_mut() else { return };
    let list = o.entry("warnings").or_insert_with(|| json!([]));
    if let Some(a) = list.as_array_mut() {
        a.push(json!(w));
    } else {
        *list = json!([w]);
    }
}

/// A project name becomes file and folder names: it must be valid on Windows and Linux.
fn check_project_name(name: &str) -> Result<(), String> {
    const RESERVED: &[&str] = &["CON", "PRN", "AUX", "NUL"];
    let upper = name.to_ascii_uppercase();
    let reserved = RESERVED.contains(&upper.as_str())
        || ((upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.len() == 4
            && upper.ends_with(|c: char| c.is_ascii_digit()));
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.ends_with(['.', ' '])
        || name.starts_with(' ')
        || name.chars().count() > 200
        || name
            .chars()
            .any(|c| c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'))
        || reserved
    {
        return Err(format!(
            "`{name}` cannot be a project name: it becomes file names, so it must be a valid file name \
             on Windows and Linux"
        ));
    }
    Ok(())
}

/// Refuses to create a project over an existing one.
fn check_project_free(dir: &Path, name: &str) -> Result<(), String> {
    for item in [
        format!("{name}.pbip"),
        format!("{name}.SemanticModel"),
        format!("{name}.Report"),
    ] {
        let path = dir.join(&item);
        if path.exists() {
            return Err(format!(
                "{} already exists; open that project and use file.save, or pick another name",
                path.display()
            ));
        }
    }
    Ok(())
}

fn create_dir(dir: &Path) -> Result<(), String> {
    if dir.as_os_str().is_empty() {
        return Ok(());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))
}

/// Creates the project and returns the session source for it.
fn create_project(
    dir: &Path,
    name: &str,
    model: &SemanticModel,
    format: ModelFormat,
) -> Result<Source, String> {
    check_project_name(name)?;
    check_project_free(dir, name)?;
    create_dir(dir)?;
    let project = bicraft_pbip::create_project(dir, name, model, format).map_err(|e| e.to_string())?;
    Ok(Source::Project {
        path: dir.join(format!("{name}.pbip")),
        project,
    })
}

/// What happens to the report of a project that `file.save_as` / `project.new` just created.
enum NewReport {
    /// The session's report was written into it.
    Written,
    /// The session had none: the blank report on disk, as read back (or why it cannot be read).
    Blank(Result<Box<Report>, String>),
}

/// Writes the session's report into a freshly created project, or reads the blank one back.
fn report_for_new_project(report: Option<&Report>, source: &Source) -> Result<NewReport, String> {
    let Source::Project { project, path } = source else {
        return Ok(NewReport::Blank(Err("not a project".into())));
    };
    match report {
        Some(r) => {
            // Legacy-only keys are translated or dropped, never written (convert.rs).
            bicraft_pbip::save_report(project, &crate::convert::pbir_form(r).0).map_err(|e| {
                format!(
                    "{} was created, but its report could not be written: {e}",
                    path.display()
                )
            })?;
            Ok(NewReport::Written)
        }
        None => Ok(NewReport::Blank(
            bicraft_pbip::read_report(project)
                .map(Box::new)
                .map_err(|e| e.to_string()),
        )),
    }
}

/// Commits a new project's report state to the session.
fn adopt_report(s: &mut Session, report: NewReport) {
    match report {
        NewReport::Written => {}
        NewReport::Blank(Ok(r)) => {
            s.report = Some(*r);
            s.report_error = None;
        }
        NewReport::Blank(Err(e)) => {
            s.report = None;
            s.report_error = Some(e);
        }
    }
    s.report_dirty = false;
}

/// Runs a save with the command's `data_root` (when given) as the session's data root; it stays
/// set only when the save succeeds.
fn with_data_root(
    s: &mut Session,
    p: &Value,
    run: fn(&mut Session, &Value) -> Result<Value, String>,
) -> Result<Value, String> {
    let Some(root) = crate::datasrc::data_root_param(p)? else {
        return run(s, p);
    };
    let rest = crate::datasrc::without_data_root(p);
    let before = std::mem::replace(&mut s.data_root, root);
    let out = run(s, &rest);
    if out.is_err() {
        s.data_root = before;
    }
    out
}

pub(crate) fn file_save(s: &mut Session, p: &Value) -> Result<Value, String> {
    // a BIcraft visual the edited report no longer uses leaves with its package (ROADMAP M12.4)
    crate::builtin_visuals::drop_unused(s);
    with_data_root(s, p, file_save_inner)
}

pub(crate) fn file_save_as(s: &mut Session, p: &Value) -> Result<Value, String> {
    crate::builtin_visuals::drop_unused(s);
    let mut out = with_data_root(s, p, file_save_as_inner)?;
    // The new file holds its own data (or none): an explicit cache_abf belonged to the old one.
    s.cache_abf = None;
    // a new project gets the DAX query tabs (a project saved under another name included)
    if let Some(w) = crate::dax_queries::into_new_project(s) {
        push_warning(&mut out, w);
    }
    Ok(out)
}

fn file_save_inner(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["backup"])?;
    let backup = opt_bool(p, "backup")?.unwrap_or(true);
    if matches!(s.source, Some(Source::Package { .. })) {
        return save_package(s, backup);
    }
    if s.source.is_none() && s.report.is_none() {
        model(s)?;
    }
    let source = s
        .source
        .as_ref()
        .ok_or("the model has not been saved yet: use file.save_as {path} or project.new {dir, name}")?;
    let mut warnings = Vec::new();
    let mut report_saved = false;
    let mut converted = false;
    let format = match source {
        Source::Package { .. } => return save_package(s, backup),
        Source::Project { project, .. } => {
            let report = s.report.as_ref().filter(|_| s.report_dirty);
            let long = crate::paths::project_long_paths(project, s.model.as_ref(), report);
            if let Some((message, _)) = crate::paths::describe(&long) {
                warnings.push(message);
            }
            match (&s.model, report) {
                (None, None) => {
                    model(s)?;
                }
                (Some(m), _) => {
                    bicraft_pbip::save_model(project, m).map_err(|e| e.to_string())?;
                }
                (None, Some(_)) => {}
            }
            if let Some(text) = &crate::diagram_layout::to_write(s) {
                bicraft_pbip::write_diagram_layout(project, text)
                    .map_err(|e| format!("the model was saved, but its diagram could not be: {e}"))?;
            }
            if let Some(r) = report {
                let (r, dropped) = crate::convert::pbir_form(r);
                warnings.extend(crate::convert::dropped_warning(&dropped));
                bicraft_pbip::save_report(project, &r).map_err(|e| {
                    if s.model.is_some() {
                        format!("the model was saved, but the report could not be: {e}")
                    } else {
                        e.to_string()
                    }
                })?;
                report_saved = true;
                converted = project.report_format != ReportFormat::Pbir;
            }
            if s.dax_queries.dirty {
                bicraft_pbip::write_dax_queries(project, &s.dax_queries.queries).map_err(|e| {
                    format!("the model and the report were saved, but the DAX queries could not be: {e}")
                })?;
            }
            match project.model_format {
                ModelFormat::Tmsl => "pbip-tmsl",
                _ => "pbip-tmdl",
            }
        }
        Source::Bim { path } => {
            write_bim(path, model(s)?)?;
            if crate::diagram_layout::to_write(s).is_some() {
                warnings
                    .push("a model.bim holds no model diagram: the moved tables are not saved".to_string());
            }
            if s.report_dirty && s.report.is_some() {
                warnings.push(
                    "a model.bim holds no report: the report edits are not saved (use file.save_as to a .pbip)"
                        .to_string(),
                );
            }
            "bim"
        }
    };
    let path = source_path(source).display().to_string();
    if converted && let Some(Source::Project { project, .. }) = s.source.as_mut() {
        project.report_format = ReportFormat::Pbir;
    }
    if s.model.is_some() {
        s.dirty = false;
    }
    if report_saved {
        s.report_dirty = false;
    }
    if matches!(s.source, Some(Source::Project { .. })) {
        s.dax_queries.dirty = false;
    } else {
        warnings.extend(crate::dax_queries::save(s)?);
    }
    Ok(json!({
        "saved": path,
        "format": format,
        "report_saved": report_saved,
        "report_converted_to_pbir": converted,
        "warnings": warnings,
    }))
}

fn file_save_as_inner(s: &mut Session, p: &Value) -> Result<Value, String> {
    // Conversions to another kind of file (and legacy → PBIR) live in convert.rs.
    let p = &match crate::convert::save_as(s, p)? {
        crate::convert::Route::Done(out) => return Ok(out),
        crate::convert::Route::Default(p) => p,
    };
    allow(p, &["path", "format", "overwrite", "strict"])?;
    let strict = opt_bool(p, "strict")?.unwrap_or(false);
    let mut long_paths = None;
    let path = PathBuf::from(str_param(p, "path")?);
    let overwrite = opt_bool(p, "overwrite")?.unwrap_or(false);
    let ext = extension(&path);
    if let Some(ext @ ("pbix" | "pbit")) = ext.as_deref() {
        if opt_str(p, "format")?.is_some() {
            return Err(format!("`format` does not apply to a .{ext} target"));
        }
        return save_package_as(s, path, ext, overwrite);
    }
    let target = match opt_str(p, "format")? {
        Some(f) if f.eq_ignore_ascii_case("pbip-tmdl") || f.eq_ignore_ascii_case("pbip") => {
            Target::Pbip(ModelFormat::Tmdl)
        }
        Some(f) if f.eq_ignore_ascii_case("pbip-tmsl") => Target::Pbip(ModelFormat::Tmsl),
        Some(f) if f.eq_ignore_ascii_case("bim") => Target::Bim,
        Some(f) => {
            return Err(format!(
                "unknown format `{f}` (expected pbip-tmdl, pbip-tmsl or bim)"
            ));
        }
        None => match ext.as_deref() {
            Some("bim") => Target::Bim,
            None | Some("pbip") => Target::Pbip(ModelFormat::Tmdl),
            Some(other) => {
                return Err(format!(
                    "cannot tell the format of `.{other}`: use a .pbip or .bim path, or pass `format`"
                ));
            }
        },
    };
    let model = model(s)?;
    let mut warnings = Vec::new();
    let mut new_report = None;
    let source = match target {
        Target::Bim => {
            if path.is_dir() {
                return Err(format!("{} is a folder", path.display()));
            }
            if path.exists() && !overwrite {
                return Err(format!(
                    "{} already exists; pass \"overwrite\": true to replace it",
                    path.display()
                ));
            }
            create_dir(path.parent().unwrap_or(Path::new("")))?;
            write_bim(&path, model)?;
            if crate::diagram_layout::to_write(s).is_some() {
                warnings
                    .push("a model.bim holds no model diagram: the moved tables were not saved".to_string());
            }
            if s.report.is_some() {
                warnings.push(
                    "a model.bim holds no report: the report was not saved (save to a .pbip to keep it)"
                        .to_string(),
                );
            }
            Source::Bim { path }
        }
        Target::Pbip(format) => {
            let name = path
                .file_stem()
                .and_then(|n| n.to_str())
                .ok_or_else(|| format!("{}: no project name in this path", path.display()))?;
            let dir = path.parent().unwrap_or(Path::new(""));
            let dir = if dir.as_os_str().is_empty() {
                Path::new(".")
            } else {
                dir
            };
            if let Some(server) = s.server_config() {
                server.check_project_target(dir, name)?;
            }
            if let Some((message, details)) =
                check_new_project_paths(dir, name, model, format, s.report.as_ref(), strict)?
            {
                warnings.push(message);
                long_paths = Some(details);
            }
            let source = create_project(dir, name, model, format)?;
            new_report = Some(report_for_new_project(s.report.as_ref(), &source)?);
            if let (Some(text), Source::Project { project, .. }) = (&s.diagram_layout, &source) {
                bicraft_pbip::write_diagram_layout(project, text).map_err(|e| {
                    format!("the project was created, but its model diagram could not be written: {e}")
                })?;
            }
            source
        }
    };
    let saved = source_path(&source).display().to_string();
    s.source = Some(source);
    s.model_error = None;
    s.dirty = false;
    let report_saved = matches!(new_report, Some(NewReport::Written));
    if let Some(r) = new_report {
        adopt_report(s, r);
    }
    let mut out = json!({
        "saved": saved,
        "format": format_name(target),
        "report_saved": report_saved,
        "warnings": warnings,
    });
    if let (Some(details), Some(o)) = (long_paths, out.as_object_mut()) {
        o.insert("long_paths".into(), details);
    }
    Ok(out)
}

pub(crate) fn project_new(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["dir", "name", "format", "strict"])?;
    let strict = opt_bool(p, "strict")?.unwrap_or(false);
    let dir = PathBuf::from(str_param(p, "dir")?);
    let name = str_param(p, "name")?;
    let format = match opt_str(p, "format")? {
        None => ModelFormat::Tmdl,
        Some(f) if f.eq_ignore_ascii_case("tmdl") => ModelFormat::Tmdl,
        Some(f) if f.eq_ignore_ascii_case("tmsl") || f.eq_ignore_ascii_case("bim") => ModelFormat::Tmsl,
        Some(f) => return Err(format!("unknown format `{f}` (expected tmdl or tmsl)")),
    };
    check_project_name(name)?;
    if let Some(server) = s.server_config() {
        server.check_project_target(&dir, name)?;
    }
    let model = match (&s.model, &s.source) {
        (Some(m), _) => m.clone(),
        (None, None) => empty_model(name, "en-US"),
        (None, Some(_)) => {
            let why = s.model_error.as_deref().unwrap_or("no model");
            return Err(format!(
                "the open file's model cannot be read ({why}); run model.new first to start an empty project"
            ));
        }
    };
    let path_warning = check_new_project_paths(&dir, name, &model, format, s.report.as_ref(), strict)?;
    let dropped = s
        .report
        .as_ref()
        .and_then(|r| crate::convert::dropped_warning(&crate::convert::pbir_form(r).1));
    // A model read from a PBIX keeps engine spellings that TMSL/TMDL do not accept.
    let source = create_project(&dir, name, &model, format)?;
    let report = report_for_new_project(s.report.as_ref(), &source)?;
    s.source = Some(source);
    s.cache_abf = None;
    s.model = Some(model);
    s.model_error = None;
    s.dirty = false;
    adopt_report(s, report);
    let queries_warning = crate::dax_queries::into_new_project(s);
    let mut out = file_inspect(s, &Value::Null)?;
    if let (Some((message, details)), Some(o)) = (path_warning, out.as_object_mut()) {
        o.insert("warnings".into(), json!([message]));
        o.insert("long_paths".into(), details);
    }
    if let (Some(w), Some(o)) = (dropped, out.as_object_mut()) {
        let mut warnings = o.get("warnings").cloned().unwrap_or_else(|| json!([]));
        if let Some(list) = warnings.as_array_mut() {
            list.push(json!(w));
        }
        o.insert("warnings".into(), warnings);
    }
    if let Some(w) = queries_warning {
        push_warning(&mut out, w);
    }
    Ok(out)
}

/// MAX_PATH guard for a project about to be created: an error with `strict`, else a warning.
fn check_new_project_paths(
    dir: &Path,
    name: &str,
    model: &SemanticModel,
    format: ModelFormat,
    report: Option<&Report>,
    strict: bool,
) -> Result<Option<(String, Value)>, String> {
    let files = crate::paths::project_files(name, model, format, report);
    let long = crate::paths::long_paths(dir, files.iter().map(String::as_str));
    match crate::paths::describe(&long) {
        Some((message, _)) if strict => Err(format!("{message} (nothing was written)")),
        other => Ok(other),
    }
}
