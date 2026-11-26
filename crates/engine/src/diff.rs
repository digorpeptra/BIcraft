//! `file.diff` and `file.textconv` (`docs/diff.md`): a semantic diff of two files of any format,
//! read metadata only. A PBIX `DataModel` stays in the file: only its `metadata.sqlitedb` is
//! decompressed, as a stream ([`bicraft_pbix::large`]), and the data snapshot is compared by
//! the size and CRC-32 the ZIP directory already holds. A PBIP project's `.pbi/cache.abf` is
//! never read.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use bicraft_diff::{DataStamp, Input, Options, Scope};
use bicraft_model::SemanticModel;
use bicraft_opc::Package;
use bicraft_report::Report;
use serde_json::{Value, json};

use crate::Session;
use crate::params::{allow, opt_bool, opt_str, str_param};

/// One side, as read for a diff.
pub(crate) struct Side {
    pub label: String,
    pub kind: &'static str,
    pub report_format: Option<&'static str>,
    pub model: Option<SemanticModel>,
    pub report: Option<Report>,
    pub report_error: Option<String>,
    pub data: Option<DataStamp>,
}

impl Side {
    fn empty() -> Side {
        Side {
            label: "(none)".into(),
            kind: "none",
            report_format: None,
            model: None,
            report: None,
            report_error: None,
            data: None,
        }
    }

    fn info(&self) -> Value {
        let mut o = serde_json::Map::new();
        o.insert("path".into(), json!(self.label));
        o.insert("kind".into(), json!(self.kind));
        if let Some(f) = self.report_format {
            o.insert("report_format".into(), json!(f));
        }
        Value::Object(o)
    }
}

/// Whether the file starts like a ZIP archive (a PBIX/PBIT whose name lost its extension, as
/// git's temporary files may).
fn looks_like_zip(path: &Path) -> bool {
    let mut magic = [0u8; 4];
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut magic))
        .is_ok()
        && magic == *b"PK\x03\x04"
}

fn report_format_name(f: bicraft_pbix::ReportFormat) -> Option<&'static str> {
    match f {
        bicraft_pbix::ReportFormat::Legacy => Some("legacy"),
        bicraft_pbix::ReportFormat::Pbir => Some("pbir"),
        bicraft_pbix::ReportFormat::None => None,
    }
}

/// Reads one side, metadata only. `label` is what the output shows.
pub(crate) fn load(s: &Session, path: &Path, label: &str) -> Result<Side, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    let is_package = matches!(ext.as_deref(), Some("pbix" | "pbit"))
        || (path.is_file() && !matches!(ext.as_deref(), Some("bim" | "pbip")) && looks_like_zip(path));
    let mut side = Side::empty();
    side.label = label.to_string();
    if is_package {
        // the DataModel stays in the file; everything else is small metadata
        let package = Package::open_deferring(
            path,
            &bicraft_opc::ReadLimits::default(),
            &[bicraft_pbix::DATA_MODEL],
        )
        .map_err(|e| format!("{}: {e}", path.display()))?;
        let inv = bicraft_pbix::inventory(&package);
        side.kind = match inv.kind {
            bicraft_pbix::Kind::Pbit => "pbit",
            _ => "pbix",
        };
        side.report_format = report_format_name(inv.report_format);
        if inv.kind != bicraft_pbix::Kind::Thin {
            side.model = Some(
                bicraft_pbix::read_model_with(&package, &bicraft_pbix::DataModelLimits::default())
                    .map_err(|e| format!("{}: {e}", path.display()))?,
            );
        }
        if package.get(bicraft_pbix::DATA_MODEL_SCHEMA).is_none() {
            side.data = package.get(bicraft_pbix::DATA_MODEL).and_then(|p| {
                p.deferred.as_ref().map(|d| DataStamp {
                    size: d.size,
                    crc32: d.crc32,
                })
            });
        }
        match bicraft_pbix::read_report(&package) {
            Ok(r) => side.report = Some(r),
            Err(e) => side.report_error = Some(e.to_string()),
        }
        return Ok(side);
    }
    if ext.as_deref() == Some("bim") {
        side.kind = "bim";
        side.model = Some(bicraft_pbip::read_bim(path).map_err(|e| e.to_string())?);
        return Ok(side);
    }
    if path.is_dir() || ext.as_deref() == Some("pbip") {
        let project = bicraft_pbip::locate(path).map_err(|e| e.to_string())?;
        if let Some(server) = s.server_config() {
            server.check_project(&project)?;
        }
        side.kind = "pbip";
        side.report_format = match project.report_format {
            bicraft_pbip::ReportFormat::Legacy => Some("legacy"),
            bicraft_pbip::ReportFormat::Pbir => Some("pbir"),
            _ => None,
        };
        if project.semantic_model.is_some() {
            side.model = Some(bicraft_pbip::read_model(&project).map_err(|e| e.to_string())?);
        }
        if project.report.is_some() {
            match bicraft_pbip::read_report(&project) {
                Ok(mut r) => {
                    // a legacy report.json lists no resources: its StaticResources files are
                    // the report's, as a PBIX's `Report/StaticResources/` parts are
                    if project.report_format == bicraft_pbip::ReportFormat::Legacy && r.resources.is_empty() {
                        let files = bicraft_pbip::read_report_files(&project).map_err(|e| e.to_string())?;
                        r.resources = files
                            .into_iter()
                            .filter(|(p, _)| p.starts_with("StaticResources/"))
                            .map(|(path, bytes)| bicraft_report::Resource { path, bytes })
                            .collect();
                    }
                    side.report = Some(r);
                }
                Err(e) => side.report_error = Some(e.to_string()),
            }
        }
        return Ok(side);
    }
    Err(format!(
        "{}: expected a .pbix, .pbit, .pbip, model.bim or project folder",
        path.display()
    ))
}

/// A side parameter: a path, or null (nothing: a file added or deleted).
fn side(s: &Session, p: &Value, key: &str) -> Result<Side, String> {
    match opt_str(p, key)? {
        None => {
            let mut side = Side::empty();
            if let Some(l) = p.get(format!("{key}_label")).and_then(Value::as_str) {
                side.label = l.to_string();
            }
            Ok(side)
        }
        Some(path) => {
            let path = PathBuf::from(path);
            // server mode: the path was resolved under the workspace root, which answers never show
            let shown = match s.server_config() {
                Some(server) => path
                    .strip_prefix(server.root())
                    .map(|r| r.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_else(|_| {
                        path.file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default()
                    }),
                None => path.to_string_lossy().into_owned(),
            };
            let label = p
                .get(format!("{key}_label"))
                .and_then(Value::as_str)
                .map_or(shown, str::to_string);
            load(s, &path, &label)
        }
    }
}

pub(crate) fn options(p: &Value) -> Result<Options, String> {
    let scope = match opt_str(p, "scope")? {
        None => Scope::All,
        Some(x) => {
            Scope::parse(x).ok_or_else(|| format!("scope must be all, model or report (got `{x}`)"))?
        }
    };
    Ok(Options {
        scope,
        ignore_formatting: opt_bool(p, "ignore_formatting")?.unwrap_or(false),
        ignore_positions: opt_bool(p, "ignore_positions")?.unwrap_or(false),
        ignore_whitespace: opt_bool(p, "ignore_whitespace")?.unwrap_or(false),
    })
}

pub(crate) fn file_diff(s: &Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "a",
            "b",
            "a_label",
            "b_label",
            "scope",
            "ignore_formatting",
            "ignore_positions",
            "ignore_whitespace",
            "format",
        ],
    )?;
    if p.get("a").is_none() || p.get("b").is_none() {
        return Err("file.diff needs `a` and `b` (a path, or null for nothing)".into());
    }
    for key in ["a_label", "b_label"] {
        opt_str(p, key)?;
    }
    let format = opt_str(p, "format")?.unwrap_or("json");
    if !matches!(format, "json" | "text" | "markdown") {
        return Err(format!("format must be json, text or markdown (got `{format}`)"));
    }
    let opts = options(p)?;
    let a = side(s, p, "a")?;
    let b = side(s, p, "b")?;
    fn input(x: &Side) -> Input<'_> {
        Input {
            model: x.model.as_ref(),
            report: x.report.as_ref(),
            data: x.data,
        }
    }
    let (ia, ib) = (input(&a), input(&b));
    // a report one side could not read is not compared at all
    let mut skip_report = false;
    let mut notes = Vec::new();
    for (name, x) in [("a", &a), ("b", &b)] {
        if let Some(e) = &x.report_error {
            // asked for the report only: no silent empty answer
            if opts.scope == Scope::Report {
                return Err(format!("report of {name} ({}) cannot be read: {e}", x.label));
            }
            skip_report = true;
            notes.push(format!("report of {name} not read ({e}): reports not compared"));
        }
        let other = if name == "a" { b.kind } else { a.kind };
        if x.kind == "bim" && !matches!(other, "bim" | "none") {
            if opts.scope == Scope::Report {
                return Err(format!("{name} ({}) is a model.bim: it holds no report", x.label));
            }
            skip_report = true;
            notes.push(format!(
                "{name} is a model.bim, which holds no report: reports not compared"
            ));
        }
    }
    let (ia, ib) = if skip_report {
        (Input { report: None, ..ia }, Input { report: None, ..ib })
    } else {
        (ia, ib)
    };
    let mut d = bicraft_diff::diff(&ia, &ib, &opts);
    d.notes.splice(0..0, notes);
    let mut out = serde_json::to_value(&d).map_err(|e| e.to_string())?;
    if let Some(o) = out.as_object_mut() {
        let mut head = serde_json::Map::new();
        head.insert("a".into(), a.info());
        head.insert("b".into(), b.info());
        head.extend(std::mem::take(o));
        *o = head;
        match format {
            "text" => {
                o.insert(
                    "text".into(),
                    json!(bicraft_diff::render::text(&d, &a.label, &b.label)),
                );
            }
            "markdown" => {
                o.insert(
                    "markdown".into(),
                    json!(bicraft_diff::render::markdown(&d, &a.label, &b.label)),
                );
            }
            _ => {}
        }
    }
    Ok(out)
}

pub(crate) fn file_textconv(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &["file"])?;
    let path = str_param(p, "file")?;
    let side = load(s, Path::new(path), path)?;
    Ok(json!({
        "kind": side.kind,
        "text": bicraft_diff::dump::dump(side.model.as_ref(), side.report.as_ref()),
    }))
}
