//! Conversions between file kinds (gap G3, M3.3): `file.save_as` to a kind other than the open
//! file's, and `file.convert {from, to}` (the same in a private session).
//!
//! Every conversion goes through the session's model and report; no format crate talks to
//! another. What each target gets:
//!
//! | Target | Model | Report | Data |
//! |---|---|---|---|
//! | `.pbit` | TMSL `DataModelSchema` | as the source holds it (PBIR, or the legacy layout) | none |
//! | `.pbix` | the source's `DataModel` / `cache.abf`, metadata edits written in; else built from scratch | as the source holds it | the source's, else computed (`crate::pbix_new`) |
//! | `.pbip` | TMDL (default) or TMSL | PBIR, or `report.json` for a legacy source | `.pbi/cache.abf` from the source's data (`cache`, default on) |
//!
//! - A legacy report stays legacy unless `report_format: "pbir"` asks for the conversion (never
//!   silently). The conversion keeps bookmarks metadata and phone layouts as PBIR documents and
//!   drops the legacy-only properties, which are counted in the warnings.
//! - A PBIX needs table data: it comes from an open PBIX or from a PBIP whose
//!   `.pbi/cache.abf` holds the data (same XPress9 framing as a PBIX `DataModel`, see
//!   `docs/formats/pbip.md`). A new model, a `model.bim`, a PBIT or a project without a cache
//!   gets a `DataModel` built from scratch (`crate::pbix_new`): tables whose source bicraft
//!   cannot run are refused by name, or written without rows with `empty_tables: true`.
//! - The model diagram (`DiagramLayout` / `diagramLayout.json`) and the report's
//!   `StaticResources/` and `CustomVisuals/` files are carried over; parts with no counterpart in
//!   the target are listed in `warnings`.
//!
//! The written file is read back before the session switches to it, like every `file.save_as`.

use std::path::{Path, PathBuf};

use bicraft_model::SemanticModel;
use bicraft_pbip::ModelFormat;
use bicraft_report::{Report, Resource};
use serde_json::{Map, Value, json};

use crate::params::{allow, opt_bool, opt_str, str_param};
use crate::{Session, Source};

/// What `file.save_as` should do with a request.
pub(crate) enum Route {
    /// The conversion ran; this is the command's result.
    Done(Value),
    /// Not a conversion: `file.save_as`'s own path handles it, with these params (conversion-only
    /// keys removed).
    Default(Value),
}

/// The keys only conversions accept.
const CONVERSION_KEYS: [&str; 3] = ["report_format", "cache", "empty_tables"];

/// Where a conversion writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Pbix,
    Pbit,
    Pbip(ModelFormat),
    Bim,
}

impl Target {
    fn name(self) -> &'static str {
        match self {
            Target::Pbix => "pbix",
            Target::Pbit => "pbit",
            Target::Pbip(ModelFormat::Tmsl) => "pbip-tmsl",
            Target::Pbip(_) => "pbip-tmdl",
            Target::Bim => "bim",
        }
    }
}

/// What the session holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Origin {
    Pbix,
    Pbit,
    /// A package without a model (a thin report).
    Thin,
    Project,
    Bim,
    /// `model.new`, never saved.
    New,
}

impl Origin {
    fn of(s: &Session) -> Origin {
        match &s.source {
            None => Origin::New,
            Some(Source::Bim { .. }) => Origin::Bim,
            Some(Source::Project { .. }) => Origin::Project,
            Some(Source::Package { package, .. }) => match bicraft_pbix::inventory(package).kind {
                bicraft_pbix::Kind::Pbix => Origin::Pbix,
                bicraft_pbix::Kind::Pbit => Origin::Pbit,
                bicraft_pbix::Kind::Thin => Origin::Thin,
            },
        }
    }

    fn name(self) -> &'static str {
        match self {
            Origin::Pbix => "pbix",
            Origin::Pbit => "pbit",
            Origin::Thin => "package",
            Origin::Project => "pbip",
            Origin::Bim => "bim",
            Origin::New => "new",
        }
    }
}

/// `report_format`: keep the source's report form, or convert a legacy report to PBIR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReportMode {
    Keep,
    Pbir,
}

fn extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
}

/// The target named by `format` (or, without it, by the extension); `None` when neither names
/// one (`file.save_as` then explains).
fn target_of(format: Option<&str>, ext: Option<&str>) -> Result<Option<Target>, String> {
    let from_ext = match ext {
        Some("pbix") => Some(Target::Pbix),
        Some("pbit") => Some(Target::Pbit),
        Some("bim") => Some(Target::Bim),
        None | Some("pbip") => Some(Target::Pbip(ModelFormat::Tmdl)),
        Some(_) => None,
    };
    let Some(f) = format else {
        return Ok(from_ext);
    };
    let named = match f.to_ascii_lowercase().as_str() {
        "pbix" => Target::Pbix,
        "pbit" => Target::Pbit,
        "pbip" | "pbip-tmdl" => Target::Pbip(ModelFormat::Tmdl),
        "pbip-tmsl" => Target::Pbip(ModelFormat::Tmsl),
        "bim" => Target::Bim,
        _ => return Ok(None),
    };
    // A package or model.bim target must carry its extension, so Desktop and file.open recognize
    // it; a project path must not name another kind of file.
    let clash = match named {
        Target::Pbix | Target::Pbit => from_ext != Some(named),
        Target::Pbip(_) => matches!(ext, Some("pbix" | "pbit" | "bim")),
        Target::Bim => matches!(ext, Some("pbix" | "pbit" | "pbip")),
    };
    if clash {
        return Err(format!(
            "format `{f}` does not match the path's extension (.{})",
            ext.unwrap_or_default()
        ));
    }
    Ok(Some(named))
}

/// Whether the open file's report is a legacy layout.
pub(crate) fn source_is_legacy(s: &Session) -> bool {
    match &s.source {
        Some(Source::Package { package, .. }) => {
            bicraft_pbix::inventory(package).report_format == bicraft_pbix::ReportFormat::Legacy
        }
        Some(Source::Project { project, .. }) => project.report_format == bicraft_pbip::ReportFormat::Legacy,
        _ => false,
    }
}

/// The `file.save_as` hook: runs the request when it is a conversion, else hands it back.
pub(crate) fn save_as(s: &mut Session, p: &Value) -> Result<Route, String> {
    let Some(path) = p.get("path").and_then(Value::as_str) else {
        return Ok(Route::Default(without_conversion_keys(p)));
    };
    let path = PathBuf::from(path);
    let ext = extension(&path);
    let target = match target_of(opt_str(p, "format")?, ext.as_deref())? {
        Some(t) => t,
        None => return Ok(Route::Default(without_conversion_keys(p))),
    };
    let mode = match opt_str(p, "report_format")? {
        None => None,
        Some(f) if f.eq_ignore_ascii_case("keep") => Some(ReportMode::Keep),
        Some(f) if f.eq_ignore_ascii_case("pbir") => Some(ReportMode::Pbir),
        Some(f) => {
            return Err(format!(
                "unknown report_format `{f}` (expected keep or pbir; a PBIR report cannot be turned into a \
                 legacy layout)"
            ));
        }
    };
    let cache = opt_bool(p, "cache")?;
    let empty_tables = opt_bool(p, "empty_tables")?;
    let origin = Origin::of(s);
    let legacy = source_is_legacy(s);
    let handled = match (target, origin) {
        (Target::Bim, _)
        | (Target::Pbix | Target::Pbit, Origin::Thin)
        | (Target::Pbip(_), Origin::Bim | Origin::New) => false,
        (Target::Pbix, Origin::Pbix) | (Target::Pbit, Origin::Pbit) => {
            mode == Some(ReportMode::Pbir) && legacy
        }
        _ => true,
    };
    if !handled {
        if target == Target::Bim && (mode.is_some() || cache.is_some()) {
            return Err(
                "`report_format` and `cache` do not apply to a model.bim (it holds no report or data)".into(),
            );
        }
        if cache == Some(true) && matches!(origin, Origin::Bim | Origin::New) {
            return Err(format!(
                "`cache`: the open model ({}) has no table data to put in .pbi/cache.abf",
                origin.name()
            ));
        }
        if cache.is_some() && !matches!(target, Target::Pbip(_)) {
            return Err("`cache` applies to .pbip targets".into());
        }
        if empty_tables.is_some() {
            return Err("`empty_tables` applies to .pbix targets built without an existing DataModel".into());
        }
        let mut rest = without_conversion_keys(p);
        if let Some(o) = rest.as_object_mut()
            && matches!(target, Target::Pbix | Target::Pbit)
        {
            o.remove("format");
        }
        return Ok(Route::Default(rest));
    }
    allow(
        p,
        &[
            "path",
            "format",
            "overwrite",
            "strict",
            "report_format",
            "cache",
            "empty_tables",
        ],
    )?;
    if cache.is_some() && !matches!(target, Target::Pbip(_)) {
        return Err("`cache` applies to .pbip targets".into());
    }
    if empty_tables.is_some() && target != Target::Pbix {
        return Err("`empty_tables` applies to .pbix targets".into());
    }
    let plan = Plan {
        path,
        target,
        origin,
        mode: mode.unwrap_or(ReportMode::Keep),
        cache,
        empty_tables: empty_tables.unwrap_or(false),
        overwrite: opt_bool(p, "overwrite")?.unwrap_or(false),
        strict: opt_bool(p, "strict")?.unwrap_or(false),
    };
    convert(s, &plan).map(Route::Done)
}

/// `p` without the keys only conversions know (`file.save_as`'s own path rejects them).
fn without_conversion_keys(p: &Value) -> Value {
    let mut rest = p.clone();
    if let Some(o) = rest.as_object_mut() {
        for k in CONVERSION_KEYS {
            o.remove(k);
        }
    }
    rest
}

/// `file.convert {from, to, ...}`: opens `from` in a private session and saves it as `to`. The
/// open file and its unsaved edits are left alone.
pub(crate) fn file_convert(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "from",
            "to",
            "format",
            "report_format",
            "cache",
            "empty_tables",
            "overwrite",
            "strict",
            "data_root",
        ],
    )?;
    let from = str_param(p, "from")?;
    let to = str_param(p, "to")?;
    // same mode: in server mode the private session resolves `from`, `to` and `data_root` itself
    let mut private = s.private();
    let mut open = Map::new();
    open.insert("path".into(), json!(from));
    if let Some(root) = p.get("data_root").filter(|v| !v.is_null()) {
        open.insert("data_root".into(), root.clone());
    }
    private
        .execute("file.open", &Value::Object(open))
        .map_err(|e| format!("cannot open {from}: {e}"))?;
    let mut params = Map::new();
    params.insert("path".into(), json!(to));
    for key in [
        "format",
        "report_format",
        "cache",
        "empty_tables",
        "overwrite",
        "strict",
    ] {
        if let Some(v) = p.get(key).filter(|v| !v.is_null()) {
            params.insert(key.into(), v.clone());
        }
    }
    let mut out = private.execute("file.save_as", &Value::Object(params))?;
    if let Some(o) = out.as_object_mut() {
        o.insert("from".into(), json!(from));
    }
    Ok(out)
}

struct Plan {
    path: PathBuf,
    target: Target,
    origin: Origin,
    mode: ReportMode,
    cache: Option<bool>,
    /// `.pbix` built from scratch: tables whose source cannot run are written without rows.
    empty_tables: bool,
    overwrite: bool,
    strict: bool,
}

/// The report as the target gets it.
enum ReportOut {
    Pbir {
        report: Box<Report>,
        /// A legacy report converted on request.
        converted: bool,
    },
    Legacy {
        layout: String,
        resources: Vec<(String, Vec<u8>)>,
    },
}

/// What the session switches to once the target is written.
struct Written {
    source: Source,
    model: SemanticModel,
    report: Result<Report, String>,
}

fn convert(s: &mut Session, plan: &Plan) -> Result<Value, String> {
    let model = crate::commands::model(s)?;
    let export = model;
    let mut warnings = Vec::new();
    let report = report_out(s, plan.mode, &mut warnings)?;
    let diagram = diagram_layout(s, &mut warnings);
    let copilot = crate::copilot::source_files(s, &mut warnings);
    let mut cache_written = false;
    let mut built_from_scratch = None;
    let mut long_paths = None;
    let written = match plan.target {
        Target::Pbit | Target::Pbix => {
            check_file_target(&plan.path, plan.overwrite)?;
            let extras = bicraft_pbixwriter::Extras {
                diagram_layout: diagram.as_deref(),
                copilot: &copilot,
            };
            let docs;
            let parts = match &report {
                ReportOut::Pbir { report, .. } => {
                    docs = bicraft_pbir::write_documents(report);
                    bicraft_pbixwriter::ReportParts::Pbir(&docs)
                }
                ReportOut::Legacy { layout, resources } => {
                    bicraft_pbixwriter::ReportParts::Legacy { layout, resources }
                }
            };
            let package = if plan.target == Target::Pbit {
                warnings.push(
                    "a PBIT holds the model without its data: Power BI Desktop loads the data when the \
                     template is opened"
                        .into(),
                );
                bicraft_pbixwriter::write_pbit_with(export, parts, &extras)
            } else {
                let bytes = match source_data(s, model)? {
                    Some(data) => {
                        if plan.empty_tables {
                            return Err(
                                "`empty_tables` applies to a .pbix built without an existing DataModel; \
                                 this one keeps the source's data"
                                    .into(),
                            );
                        }
                        data.bytes
                    }
                    None => {
                        let (bytes, emptied) =
                            crate::pbix_new::data_model(model, s.data_root.as_deref(), plan.empty_tables)
                                .map_err(|e| from_scratch_error(plan.origin, &e))?;
                        if !emptied.is_empty() {
                            warnings.push(format!(
                                "written without rows (bicraft cannot run their source; Power BI Desktop \
                                 loads them on refresh): {}",
                                emptied.join(", ")
                            ));
                        }
                        built_from_scratch = Some(emptied);
                        bytes
                    }
                };
                bicraft_pbixwriter::write_pbix(bytes, parts, &extras)
            }
            .map_err(|e| e.to_string())?;
            if let Some(Source::Package { package: src, .. }) = &s.source {
                dropped_parts(src, &mut warnings);
            }
            write_package(&plan.path, &package)?
        }
        Target::Pbip(format) => {
            let (dir, name) = project_target(&plan.path)?;
            if let Some(server) = s.server_config() {
                server.check_project_target(&dir, &name)?;
            }
            let data = match plan.cache {
                Some(false) => None,
                _ => match source_data(s, model) {
                    Ok(d) => d,
                    Err(e) if plan.cache.is_none() => {
                        warnings.push(format!("no .pbi/cache.abf was written: {e}"));
                        None
                    }
                    Err(e) => return Err(e),
                },
            };
            if plan.cache == Some(true) && data.is_none() {
                return Err(format!(
                    "`cache`: the open {} has no table data to put in .pbi/cache.abf",
                    plan.origin.name()
                ));
            }
            if data.is_none() {
                warnings.push(
                    "the project has no .pbi/cache.abf: Power BI Desktop opens it without data until it is \
                     refreshed"
                        .into(),
                );
            }
            let mut files =
                project_file_list(&name, export, format, &report, data.is_some(), diagram.is_some());
            files.extend(copilot.iter().map(|(p, _)| format!("{name}.SemanticModel/{p}")));
            let long = crate::paths::long_paths(&dir, files.iter().map(String::as_str));
            if let Some((message, details)) = crate::paths::describe(&long) {
                if plan.strict {
                    return Err(format!("{message} (nothing was written)"));
                }
                warnings.push(message);
                long_paths = Some(details);
            }
            if let Some(Source::Package { package: src, .. }) = &s.source {
                dropped_parts(src, &mut warnings);
            }
            if data.is_some() {
                let target = dir
                    .join(format!("{name}.SemanticModel"))
                    .join(".pbi")
                    .join("cache.abf");
                crate::cache_abf::refuse_overwrite(s, &target)?;
            }
            cache_written = data.is_some();
            write_project(
                &dir,
                &name,
                export,
                format,
                &report,
                &ProjectExtras {
                    cache: data.as_ref().map(|d| d.bytes.as_slice()),
                    diagram: diagram.as_deref(),
                    copilot: &copilot,
                },
            )?
        }
        Target::Bim => return Err("internal error: a model.bim is not a conversion".into()),
    };
    let (report_format, converted) = match &report {
        ReportOut::Pbir { converted, .. } => ("pbir", *converted),
        ReportOut::Legacy { .. } => ("legacy", false),
    };
    let saved = crate::commands::source_path(&written.source)
        .display()
        .to_string();
    let measures: usize = written.model.tables.iter().map(|t| t.measures.len()).sum();
    let mut out = json!({
        "saved": saved,
        "format": plan.target.name(),
        "converted_from": plan.origin.name(),
        "report_saved": true,
        "report_format": report_format,
        "report_converted_to_pbir": converted,
        "model": {"tables": written.model.tables.len(), "measures": measures, "relationships": written.model.relationships.len()},
        "warnings": warnings,
    });
    if let Some(o) = out.as_object_mut() {
        match plan.target {
            Target::Pbix => {
                o.insert("data".into(), json!(true));
                if let Some(emptied) = &built_from_scratch {
                    o.insert("built_from_scratch".into(), json!(true));
                    o.insert("empty_tables".into(), json!(emptied));
                }
            }
            Target::Pbip(_) => {
                o.insert("cache".into(), json!(cache_written));
            }
            _ => {}
        }
        if let Ok(r) = &written.report {
            o.insert(
                "report".into(),
                json!({"pages": r.pages.len(), "visuals": r.pages.iter().map(|p| p.visuals.len()).sum::<usize>()}),
            );
        }
        if let Some(details) = long_paths {
            o.insert("long_paths".into(), details);
        }
    }
    s.source = Some(written.source);
    s.model = Some(written.model);
    s.model_error = None;
    s.dirty = false;
    match written.report {
        Ok(r) => {
            s.report = Some(r);
            s.report_error = None;
        }
        Err(e) => {
            s.report = None;
            s.report_error = Some(e);
        }
    }
    s.report_dirty = false;
    Ok(out)
}

/// The error of a `DataModel` built from scratch, with where the model came from.
fn from_scratch_error(origin: Origin, e: &str) -> String {
    let what = match origin {
        Origin::Pbit => "the PBIT holds no table data, so the PBIX data is computed by bicraft",
        Origin::Project => "the project has no .pbi/cache.abf, so the PBIX data is computed by bicraft",
        _ => "the model has no table data yet, so the PBIX data is computed by bicraft",
    };
    format!(
        "{what}, and {e}. Pass `empty_tables: true` to write tables whose source bicraft cannot run \
         without rows (Power BI Desktop loads them on refresh), or save as .pbit or .pbip instead"
    )
}

/// The report the target gets, from the session's report and the source's files.
fn report_out(s: &Session, mode: ReportMode, warnings: &mut Vec<String>) -> Result<ReportOut, String> {
    let raw = source_resources(s)?;
    if source_is_legacy(s) && mode == ReportMode::Keep {
        let original = legacy_text(s)?;
        let layout = match (&s.report, s.report_dirty) {
            (Some(r), true) => bicraft_layout::write_layout(&original, r).map_err(|e| {
                format!(
                    "the report edits cannot be written into the legacy layout: {e}; pass report_format: \
                     \"pbir\" to convert the report to PBIR"
                )
            })?,
            _ => original,
        };
        return Ok(ReportOut::Legacy {
            layout,
            resources: raw,
        });
    }
    let Some(report) = &s.report else {
        if let Some(e) = &s.report_error {
            return Err(format!(
                "the report of the open file cannot be read ({e}); nothing was converted, so it is not lost"
            ));
        }
        // No report (model.bim, model.new): a blank one-page report.
        let page = bicraft_pbip::ProjectIds::random().page_name;
        return Ok(ReportOut::Pbir {
            report: Box::new(bicraft_pbip::blank_report(&page)),
            converted: false,
        });
    };
    let mut report = report.clone();
    let converted = source_is_legacy(s);
    let dropped = legacy_to_pbir(&mut report);
    if let Some(w) = dropped_warning(&dropped) {
        warnings.push(w);
    }
    if matches!(s.source, Some(Source::Package { .. })) {
        for r in &mut report.resources {
            r.path = percent_decode(&r.path);
        }
    }
    add_missing_resources(&mut report, raw);
    Ok(ReportOut::Pbir {
        report: Box::new(report),
        converted,
    })
}

/// The original legacy layout text of the open file.
fn legacy_text(s: &Session) -> Result<String, String> {
    match &s.source {
        Some(Source::Package { package, .. }) => {
            let part = package
                .get(bicraft_pbix::LEGACY_LAYOUT)
                .ok_or("the package has no Report/Layout")?;
            bicraft_opc::decode_text(&part.name, &part.data)
                .map(|(t, _)| t)
                .map_err(|e| format!("Report/Layout: {e}"))
        }
        Some(Source::Project { project, .. }) => bicraft_pbip::read_legacy_layout(project)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "the project has no report.json".to_string()),
        _ => Err("the open file has no legacy report".into()),
    }
}

/// The source's raw report files (`StaticResources/…`, `CustomVisuals/…`), relative to the report
/// root, with package part names decoded.
fn source_resources(s: &Session) -> Result<Vec<(String, Vec<u8>)>, String> {
    match &s.source {
        Some(Source::Package { package, .. }) => Ok(package
            .parts
            .iter()
            .filter_map(|p| {
                let rel = p.name.strip_prefix(bicraft_pbix::REPORT_PREFIX)?;
                let resource = rel.starts_with("StaticResources/") || rel.starts_with("CustomVisuals/");
                resource.then(|| (percent_decode(rel), p.data.clone()))
            })
            .collect()),
        Some(Source::Project { project, .. }) => {
            bicraft_pbip::read_report_files(project).map_err(|e| e.to_string())
        }
        _ => Ok(Vec::new()),
    }
}

/// Adds the raw files the report does not hold yet (a PBIR package's `CustomVisuals/`, a legacy
/// project's resources).
fn add_missing_resources(report: &mut Report, raw: Vec<(String, Vec<u8>)>) {
    let mut known: std::collections::HashSet<String> =
        report.resources.iter().map(|r| r.path.to_lowercase()).collect();
    for (path, bytes) in raw {
        if known.insert(path.to_lowercase()) {
            report.resources.push(Resource { path, bytes });
        }
    }
}

/// `visualContainerMobileState` version for phone layouts converted from a legacy layout.
const MOBILE_SCHEMA: bicraft_versions::Schema = bicraft_versions::VISUAL_MOBILE_STATE;
const BOOKMARKS_SCHEMA: bicraft_versions::Schema = bicraft_versions::BOOKMARKS_METADATA;
const EXTENSIONS_SCHEMA: bicraft_versions::Schema = bicraft_versions::REPORT_EXTENSION;

/// Most dropped legacy properties named in a warning; the count is always exact.
const MAX_DROPPED_LISTED: usize = 20;

/// The report in the form the PBIR writer may write: a report read from a legacy layout carries
/// legacy-only keys (`legacy:…`) in its `extra` maps, which are translated (bookmarks metadata →
/// `bookmarks/bookmarks.json`, phone layouts → `visuals/<v>/mobile.json`) or dropped, never written
/// raw. Returns the report (borrowed when it has no such key) and what was dropped, as
/// `<object>: <legacy property>` lines.
pub(crate) fn pbir_form(report: &Report) -> (std::borrow::Cow<'_, Report>, Vec<String>) {
    if !has_legacy_keys(report) {
        return (std::borrow::Cow::Borrowed(report), Vec::new());
    }
    let mut owned = report.clone();
    let dropped = legacy_to_pbir(&mut owned);
    (std::borrow::Cow::Owned(owned), dropped)
}

/// The warning listing what [`pbir_form`] dropped, if anything.
pub(crate) fn dropped_warning(dropped: &[String]) -> Option<String> {
    if dropped.is_empty() {
        return None;
    }
    let listed: Vec<&str> = dropped
        .iter()
        .take(MAX_DROPPED_LISTED)
        .map(String::as_str)
        .collect();
    let more = dropped.len().saturating_sub(listed.len());
    let tail = if more > 0 {
        format!("; and {more} more")
    } else {
        String::new()
    };
    Some(format!(
        "the report was written as PBIR: {} legacy-only properties have no PBIR form and were dropped          (cached query data, internal ids, …): {}{tail}",
        dropped.len(),
        listed.join("; ")
    ))
}

fn has_legacy_keys(report: &Report) -> bool {
    let any = |m: &bicraft_report::Properties| m.keys().any(|k| k.starts_with(bicraft_layout::LEGACY_PREFIX));
    any(&report.extra)
        || any(&report.pages_extra)
        || any(&report.version_extra)
        || report
            .pages
            .iter()
            .any(|p| any(&p.extra) || p.visuals.iter().any(|v| any(&v.extra) || any(&v.visual_extra)))
}

/// Turns a report read from a legacy layout into a pure PBIR one: the bookmarks metadata and the
/// phone layouts become their PBIR documents (unless the report already holds them), every other
/// legacy-only key is removed. Returns what was dropped.
fn legacy_to_pbir(report: &mut Report) -> Vec<String> {
    let with_schema = |value: Value, schema: bicraft_versions::Schema| -> Option<Vec<u8>> {
        let Value::Object(map) = value else {
            return None;
        };
        let mut doc = Map::new();
        if !map.contains_key("$schema") {
            doc.insert("$schema".into(), json!(schema.url()));
        }
        doc.extend(map);
        serde_json::to_vec_pretty(&Value::Object(doc)).ok()
    };
    let mut docs = Vec::new();
    let mut dropped = Vec::new();
    if let Some(meta) = report.extra.shift_remove(bicraft_layout::BOOKMARKS_METADATA_KEY) {
        match with_schema(meta, BOOKMARKS_SCHEMA) {
            Some(bytes) => docs.push(("definition/bookmarks/bookmarks.json".to_string(), bytes)),
            None => dropped.push(format!("report: {}", bicraft_layout::BOOKMARKS_METADATA_KEY)),
        }
    }
    for page in &mut report.pages {
        for visual in &mut page.visuals {
            if let Some(mut state) = visual.extra.shift_remove(bicraft_layout::MOBILE_STATE_KEY) {
                // Desktop takes the phone formatting of PBIR `mobile.json` from the mobile state
                // document, not from the layout's `mobileObjects` / `mobileVcObjects`: only the
                // phone position is converted (docs/formats/legacy-layout-bb.md, visuals table).
                if let Value::Object(m) = &mut state {
                    for key in ["objects", "visualContainerObjects"] {
                        if m.shift_remove(key).is_some() {
                            dropped.push(format!(
                                "visual {}/{}: phone {key} (mobileObjects in the layout)",
                                page.name, visual.name
                            ));
                        }
                    }
                }
                match with_schema(state, MOBILE_SCHEMA) {
                    Some(bytes) => docs.push((
                        format!(
                            "definition/pages/{}/visuals/{}/mobile.json",
                            page.name, visual.name
                        ),
                        bytes,
                    )),
                    None => dropped.push(format!(
                        "visual {}/{}: {}",
                        page.name,
                        visual.name,
                        bicraft_layout::MOBILE_STATE_KEY
                    )),
                }
            }
        }
    }
    // Report-level measures: legacy `config.modelExtensions` → `reportExtensions.json`.
    let extensions = report
        .extra
        .get_mut(bicraft_layout::UNMAPPED_KEY)
        .and_then(Value::as_object_mut)
        .and_then(|u| u.shift_remove("config.modelExtensions"));
    if let Some(ext) = extensions {
        match bicraft_layout::report_extensions(&ext).and_then(|doc| with_schema(doc, EXTENSIONS_SCHEMA)) {
            Some(bytes) => docs.push(("definition/reportExtensions.json".to_string(), bytes)),
            None => dropped.push("report: config.modelExtensions".into()),
        }
    }
    list_legacy(&report.extra, "report", &mut dropped);
    list_legacy(&report.pages_extra, "pages", &mut dropped);
    list_legacy(&report.version_extra, "version", &mut dropped);
    for page in &report.pages {
        list_legacy(&page.extra, &format!("page {}", page.name), &mut dropped);
        for visual in &page.visuals {
            let at = format!("visual {}/{}", page.name, visual.name);
            list_legacy(&visual.extra, &at, &mut dropped);
            list_legacy(&visual.visual_extra, &at, &mut dropped);
        }
    }
    bicraft_layout::strip_legacy(report);
    let mut known: std::collections::HashSet<String> =
        report.resources.iter().map(|r| r.path.to_lowercase()).collect();
    for (path, bytes) in docs {
        if known.insert(path.to_lowercase()) {
            report.resources.push(Resource { path, bytes });
        }
    }
    dropped
}

/// Names the legacy-only properties of one `extra` map: each entry of a `legacy:unmapped` map
/// (keyed by its legacy path) and every other `legacy:` key.
fn list_legacy(map: &bicraft_report::Properties, at: &str, out: &mut Vec<String>) {
    for (k, v) in map
        .iter()
        .filter(|(k, _)| k.starts_with(bicraft_layout::LEGACY_PREFIX))
    {
        match v {
            Value::Object(inner) if k == bicraft_layout::UNMAPPED_KEY => {
                out.extend(inner.keys().map(|legacy| format!("{at}: {legacy}")));
            }
            _ => out.push(format!("{at}: {k}")),
        }
    }
}

/// The source's model diagram, when it has a usable one.
fn diagram_layout(s: &Session, warnings: &mut Vec<String>) -> Option<String> {
    // the diagram as edited in the session (`diagram.move`), else the file's
    if let Some(t) = &s.diagram_layout {
        return Some(t.clone());
    }
    let text = match &s.source {
        Some(Source::Package { package, .. }) => {
            let part = package.get("DiagramLayout")?;
            bicraft_opc::decode_text(&part.name, &part.data).ok()?.0
        }
        Some(Source::Project { project, .. }) => match bicraft_pbip::read_diagram_layout(project) {
            Ok(t) => t?,
            Err(e) => {
                warnings.push(format!("the model diagram was not carried over: {e}"));
                return None;
            }
        },
        _ => return None,
    };
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(_)) => Some(text),
        _ => {
            warnings.push("the model diagram was not carried over: it is not a JSON object".into());
            None
        }
    }
}

/// The table data of the open file, in the `DataModel` framing, with the session's metadata
/// edits written in.
struct Data {
    bytes: Vec<u8>,
}

fn source_data(s: &Session, model: &SemanticModel) -> Result<Option<Data>, String> {
    match &s.source {
        Some(Source::Package { package, .. }) => {
            let Some(part) = package.get(bicraft_pbix::DATA_MODEL) else {
                return Ok(None);
            };
            if part.len() > bicraft_opc::MAX_PART_BYTES {
                return Err(format!(
                    "the {}-byte DataModel is too large to convert (it is only copied by file.save /\
                     file.save_as of the same format)",
                    part.len()
                ));
            }
            // a part left in the file by choice (memory budget) is read now
            let bytes = part.bytes().map_err(|e| e.to_string())?;
            if !s.dirty {
                return Ok(Some(Data {
                    bytes: bytes.into_owned(),
                }));
            }
            bicraft_pbix::write_data_model(&bytes, model)
                .map(|bytes| Some(Data { bytes }))
                .map_err(|e| format!("the model edits cannot be written into the data: {e}"))
        }
        Some(Source::Project { project, .. }) => {
            let cache = match &s.cache_abf {
                Some(_) => crate::cache_abf::read(s)?,
                None => bicraft_pbip::read_cache_abf(project).map_err(|e| e.to_string())?,
            };
            let Some(cache) = cache else {
                return Ok(None);
            };
            let cached = bicraft_pbix::read_data_model(&cache)
                .map_err(|e| format!(".pbi/cache.abf cannot be read: {e}"))?;
            let model = align_with_cache(model, &cached);
            if cached == model {
                return Ok(Some(Data { bytes: cache }));
            }
            bicraft_pbix::write_data_model(&cache, &model)
                .map(|bytes| Some(Data { bytes }))
                .map_err(|e| {
                    format!(
                        "the project's model differs from the one in its .pbi/cache.abf in ways the data \
                         cannot follow ({e}); open the project in Power BI Desktop, refresh and save it so \
                         the cache matches, then convert again"
                    )
                })
        }
        _ => Ok(None),
    }
}

/// The project's model with what a project file does not record taken from the cache's model:
/// the database and model object names (a project names them after itself), the
/// engine-internal `RowNumber-…` columns (TMDL leaves them out) and the spelling of property
/// names that differ only by letter case between serializations. None of it is user metadata.
fn align_with_cache(model: &SemanticModel, cached: &SemanticModel) -> SemanticModel {
    let mut out = model.clone();
    out.name.clone_from(&cached.name);
    match cached.extra.get("name") {
        Some(v) => {
            out.extra.insert("name".into(), v.clone());
        }
        None => {
            out.extra.shift_remove("name");
        }
    }
    align_keys(&mut out.extra, &cached.extra);
    for table in &mut out.tables {
        let Some(c) = cached.tables.iter().find(|t| t.name == table.name) else {
            continue;
        };
        align_keys(&mut table.extra, &c.extra);
        for column in &mut table.columns {
            if let Some(cc) = c.columns.iter().find(|x| x.name == column.name) {
                align_keys(&mut column.extra, &cc.extra);
            }
        }
        for measure in &mut table.measures {
            if let Some(cm) = c.measures.iter().find(|x| x.name == measure.name) {
                align_keys(&mut measure.extra, &cm.extra);
            }
        }
        if table.columns.iter().any(bicraft_model::Column::is_row_number) {
            continue;
        }
        let internal: Vec<_> = c
            .columns
            .iter()
            .filter(|col| col.is_row_number())
            .cloned()
            .collect();
        if !internal.is_empty() {
            table.columns.splice(0..0, internal);
        }
    }
    out
}

/// Renames the keys of `props` that `reference` spells with another letter case.
fn align_keys(props: &mut Map<String, Value>, reference: &Map<String, Value>) {
    let renames: Vec<(String, String)> = props
        .keys()
        .filter(|k| !reference.contains_key(*k))
        .filter_map(|k| {
            reference
                .keys()
                .find(|r| r.eq_ignore_ascii_case(k) && !props.contains_key(*r))
                .map(|r| (k.clone(), r.clone()))
        })
        .collect();
    for (from, to) in renames {
        if let Some(v) = props.shift_remove(&from) {
            props.insert(to, v);
        }
    }
}

/// Lists the source package parts the target does not carry (names only).
fn dropped_parts(package: &bicraft_opc::Package, warnings: &mut Vec<String>) {
    const CARRIED: [&str; 7] = [
        "Version",
        "[Content_Types].xml",
        "DataModel",
        "DataModelSchema",
        "DiagramLayout",
        "Settings",
        "Metadata",
    ];
    let mut dropped: Vec<&str> = package
        .parts
        .iter()
        .map(|p| p.name.as_str())
        .filter(|n| {
            !CARRIED.iter().any(|c| c.eq_ignore_ascii_case(n))
                && *n != bicraft_pbix::LEGACY_LAYOUT
                && !n.starts_with(bicraft_pbix::PBIR_PREFIX)
                && !n.starts_with(bicraft_pbix::STATIC_RESOURCES_PREFIX)
                && !n.starts_with("Report/CustomVisuals/")
                && !bicraft_pbix::is_copilot_part(n)
        })
        .collect();
    dropped.sort_unstable();
    dropped.dedup();
    if dropped.contains(&bicraft_pbix::SECURITY_BINDINGS) {
        warnings.push(
            "SecurityBindings was not carried over: it seals the parts of the last Desktop save; Desktop \
             writes a new one when it saves"
                .into(),
        );
        dropped.retain(|n| *n != bicraft_pbix::SECURITY_BINDINGS);
    }
    if !dropped.is_empty() {
        warnings.push(format!(
            "parts with no counterpart in the target were not carried over: {}",
            dropped.join(", ")
        ));
    }
}

fn check_file_target(path: &Path, overwrite: bool) -> Result<(), String> {
    if path.is_dir() {
        return Err(format!("{} is a folder", path.display()));
    }
    if path.exists() && !overwrite {
        return Err(format!(
            "{} already exists; pass \"overwrite\": true to replace it",
            path.display()
        ));
    }
    Ok(())
}

/// Writes the package (temporary sibling, then rename) and reads it back.
fn write_package(path: &Path, package: &bicraft_opc::Package) -> Result<Written, String> {
    let bytes = package.to_bytes().map_err(|e| e.to_string())?;
    let back = bicraft_opc::Package::read_bytes(&bytes)
        .map_err(|e| format!("internal check failed: the written package does not read back: {e}"))?;
    let model = bicraft_pbix::read_model(&back)
        .map_err(|e| format!("internal check failed: the written model does not read back: {e}"))?;
    let report = bicraft_pbix::read_report(&back).map_err(|e| e.to_string());
    if let Err(e) = &report {
        return Err(format!(
            "internal check failed: the written report does not read back: {e}"
        ));
    }
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| format!("{}: not a file path", path.display()))?;
    let tmp = path.with_file_name(format!("{file_name}.bicraft-tmp"));
    std::fs::write(&tmp, &bytes).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {e}", tmp.display())
    })?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {e}", path.display())
    })?;
    Ok(Written {
        source: Source::Package {
            path: path.to_path_buf(),
            package: back,
        },
        model,
        report,
    })
}

/// `out/Sales.pbip` (or `out/Sales`) → (`out`, `Sales`).
fn project_target(path: &Path) -> Result<(PathBuf, String), String> {
    let name = path
        .file_stem()
        .and_then(|n| n.to_str())
        .ok_or_else(|| format!("{}: no project name in this path", path.display()))?
        .to_string();
    bicraft_pbip::validate_project_name(&name).map_err(|e| e.to_string())?;
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("."),
    };
    Ok((dir, name))
}

/// Every file the new project will hold, relative to its folder (for the MAX_PATH guard).
fn project_file_list(
    name: &str,
    model: &SemanticModel,
    format: ModelFormat,
    report: &ReportOut,
    cache: bool,
    diagram: bool,
) -> Vec<String> {
    let report_dir = format!("{name}.Report");
    let model_dir = format!("{name}.SemanticModel");
    let mut files = match report {
        ReportOut::Pbir { report, .. } => crate::paths::project_files(name, model, format, Some(report)),
        ReportOut::Legacy { resources, .. } => {
            let mut f = crate::paths::project_files(name, model, format, None);
            f.retain(|p| !p.starts_with(&format!("{report_dir}/definition/")));
            f.push(format!("{report_dir}/report.json"));
            f.extend(resources.iter().map(|(p, _)| format!("{report_dir}/{p}")));
            f
        }
    };
    if cache {
        files.push(format!("{model_dir}/{}", bicraft_pbip::CACHE_ABF));
    }
    if diagram {
        files.push(format!("{model_dir}/{}", bicraft_pbip::DIAGRAM_LAYOUT));
    }
    files
}

/// What a new project carries besides the model and the report.
#[derive(Clone, Copy)]
struct ProjectExtras<'a> {
    /// `.pbi/cache.abf`.
    cache: Option<&'a [u8]>,
    /// `diagramLayout.json`.
    diagram: Option<&'a str>,
    /// The Copilot container files (`crate::copilot`).
    copilot: &'a [(String, Vec<u8>)],
}

/// Creates the project; on any failure after the scaffold, the new items are removed again.
fn write_project(
    dir: &Path,
    name: &str,
    model: &SemanticModel,
    format: ModelFormat,
    report: &ReportOut,
    extras: &ProjectExtras<'_>,
) -> Result<Written, String> {
    let ProjectExtras {
        cache,
        diagram,
        copilot,
    } = *extras;
    let project = bicraft_pbip::create_project(dir, name, model, format).map_err(|e| e.to_string())?;
    let fill = || -> Result<bicraft_pbip::Project, String> {
        let mut project = project.clone();
        match report {
            ReportOut::Pbir { report, .. } => {
                bicraft_pbip::save_report(&project, report).map_err(|e| e.to_string())?;
            }
            ReportOut::Legacy { layout, resources } => {
                project = bicraft_pbip::write_legacy_report(&project, layout, resources)
                    .map_err(|e| e.to_string())?;
            }
        }
        if let Some(text) = diagram {
            bicraft_pbip::write_diagram_layout(&project, text).map_err(|e| e.to_string())?;
        }
        if let Some(bytes) = cache {
            bicraft_pbip::write_cache_abf(&project, bytes).map_err(|e| e.to_string())?;
        }
        if !copilot.is_empty() {
            bicraft_pbip::write_copilot_files(&project, copilot).map_err(|e| e.to_string())?;
        }
        bicraft_pbip::locate(&dir.join(format!("{name}.pbip"))).map_err(|e| e.to_string())
    };
    let project = match fill() {
        Ok(p) => p,
        Err(e) => {
            let _ = std::fs::remove_file(dir.join(format!("{name}.pbip")));
            for item in [".SemanticModel", ".Report"] {
                let _ = std::fs::remove_dir_all(dir.join(format!("{name}{item}")));
            }
            return Err(format!(
                "the project could not be written (nothing was kept): {e}"
            ));
        }
    };
    let report = bicraft_pbip::read_report(&project).map_err(|e| e.to_string());
    Ok(Written {
        source: Source::Project {
            path: dir.join(format!("{name}.pbip")),
            project,
        },
        model: model.clone(),
        report,
    })
}

/// Decodes `%XX` escapes (package part names are percent-encoded URIs). Text that does not
/// decode to UTF-8 is returned unchanged.
fn percent_decode(text: &str) -> String {
    if !text.contains('%') {
        return text.to_string();
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while let Some(&b) = bytes.get(i) {
        let hex = bytes
            .get(i.saturating_add(1)..i.saturating_add(3))
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (b, hex) {
            (b'%', Some(v)) => {
                out.push(v);
                i = i.saturating_add(3);
            }
            _ => {
                out.push(b);
                i = i.saturating_add(1);
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_escapes_decode() {
        assert_eq!(percent_decode("a%20b/%C3%A9%25.png"), "a b/é%.png");
        assert_eq!(percent_decode("plain"), "plain");
        assert_eq!(percent_decode("bad%zz%2"), "bad%zz%2");
        assert_eq!(percent_decode("%FF"), "%FF");
    }

    #[test]
    fn targets_follow_format_then_extension() {
        let t = |f: Option<&str>, e: Option<&str>| target_of(f, e);
        assert_eq!(t(None, Some("pbix")).unwrap(), Some(Target::Pbix));
        assert_eq!(t(None, Some("pbit")).unwrap(), Some(Target::Pbit));
        assert_eq!(t(None, None).unwrap(), Some(Target::Pbip(ModelFormat::Tmdl)));
        assert_eq!(
            t(Some("PBIP-TMSL"), Some("pbip")).unwrap(),
            Some(Target::Pbip(ModelFormat::Tmsl))
        );
        assert_eq!(t(None, Some("txt")).unwrap(), None);
        assert_eq!(t(Some("xlsx"), Some("pbip")).unwrap(), None);
        assert!(t(Some("pbit"), Some("pbip")).is_err());
        assert!(t(Some("pbip"), Some("pbix")).is_err());
        assert!(t(Some("bim"), Some("pbit")).is_err());
        assert_eq!(t(Some("bim"), Some("json")).unwrap(), Some(Target::Bim));
        assert_eq!(t(Some("pbit"), Some("pbit")).unwrap(), Some(Target::Pbit));
    }
}

#[cfg(test)]
#[path = "convert_tests.rs"]
mod convert_tests;
