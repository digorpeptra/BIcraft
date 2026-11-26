//! `project.test`: CI quality gates for a Power BI project (`docs/ci-tests.md`).
//!
//! A declarative test file (`bicraft.tests.yaml`, `.yml` or `.json` next to the project; schema
//! in `citest_spec.rs`) lists DAX assertions evaluated over the model's data by bicraft's own
//! engine, model rules (`model.validate`, the BPA rules of `model.analyze`, declarative naming /
//! required-property / forbidden-function / expression-length rules), report rules
//! (`report.validate` findings by category plus declarative page, visual and theme rules) and
//! usage rules. Every check becomes a test case with a status (passed, failed, error, skipped)
//! and a severity; the run fails when a failed or errored case reaches `fail_on`.
//!
//! The command only reads: with `path` it opens the file in a private session (the caller's
//! session is left alone), without it it tests the open file. Results come as JSON, and also as
//! text, JUnit XML or SARIF 2.1.0 (`citest_out.rs`), with findings located in the project's
//! TMDL / model.bim / PBIR files where possible (`citest_loc.rs`).

use std::path::{Path, PathBuf};
use std::time::Instant;

use serde_json::{Value, json};

use crate::citest_spec::Spec;
use crate::commands::source_path;
use crate::params::{allow, opt_str};
use crate::{Session, Source};

/// The test file names looked for next to the project, in this order.
pub(crate) const TEST_FILES: &[&str] = &["bicraft.tests.yaml", "bicraft.tests.yml", "bicraft.tests.json"];

/// Most findings kept per test case (the count stays exact).
pub(crate) const MAX_FINDINGS: usize = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Level {
    Info,
    Warning,
    Error,
}

impl Level {
    pub fn name(self) -> &'static str {
        match self {
            Level::Info => "info",
            Level::Warning => "warning",
            Level::Error => "error",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "error" => Some(Level::Error),
            "warning" => Some(Level::Warning),
            "info" | "note" => Some(Level::Info),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Status {
    Passed,
    /// The check ran and found problems.
    Failed,
    /// The check could not run (an invalid query, a missing report, an unsupported function).
    Error,
    Skipped,
}

impl Status {
    pub fn name(self) -> &'static str {
        match self {
            Status::Passed => "passed",
            Status::Failed => "failed",
            Status::Error => "error",
            Status::Skipped => "skipped",
        }
    }
}

/// What a finding is about, for locating it in the project's files.
#[derive(Debug, Clone, Default)]
pub(crate) enum Target {
    #[default]
    None,
    /// A model object: `kind` is a [`crate::citest_spec::Scope`] name (or `relationship`).
    Model {
        kind: String,
        table: Option<String>,
        name: String,
    },
    /// A report page or visual (page by name or display name).
    Report {
        page: Option<String>,
        visual: Option<String>,
        bookmark: Option<String>,
    },
    /// A file of the report folder, relative to it (`definition/pages/x/page.json`).
    ReportFile(String),
    /// The test file, at the line holding this text.
    TestFile(String),
}

#[derive(Debug, Clone)]
pub(crate) struct Finding {
    pub level: Level,
    pub message: String,
    pub target: Target,
    /// Located file (relative, `/`-separated) and 1-based line.
    pub location: Option<(String, Option<usize>)>,
}

impl Finding {
    pub fn new(level: Level, message: impl Into<String>, target: Target) -> Self {
        Self {
            level,
            message: message.into(),
            target,
            location: None,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Case {
    /// `dax`, `model`, `bpa`, `report` or `usage`.
    pub suite: &'static str,
    pub id: String,
    pub name: String,
    /// What kind of check (`measure`, `query`, `naming`, `validate`, a BPA category, ...).
    pub kind: String,
    pub status: Status,
    pub level: Level,
    pub message: String,
    pub findings: Vec<Finding>,
    /// Findings beyond [`MAX_FINDINGS`], not listed.
    pub omitted: usize,
    pub elapsed_ms: u64,
    /// Kind-specific details (actual values, the BPA rule, ...).
    pub details: Option<Value>,
}

impl Case {
    pub fn new(
        suite: &'static str,
        id: impl Into<String>,
        name: impl Into<String>,
        kind: impl Into<String>,
    ) -> Self {
        Self {
            suite,
            id: id.into(),
            name: name.into(),
            kind: kind.into(),
            status: Status::Passed,
            level: Level::Error,
            message: String::new(),
            findings: Vec::new(),
            omitted: 0,
            elapsed_ms: 0,
            details: None,
        }
    }

    /// Adds findings and sets the outcome: failed at `level` when there are any, else passed.
    pub fn conclude(mut self, findings: Vec<Finding>, what: &str) -> Self {
        let n = findings.len();
        if n == 0 {
            self.status = Status::Passed;
            self.message = format!("no {what}");
        } else {
            self.status = Status::Failed;
            self.message = if n == 1 {
                findings.first().map(|f| f.message.clone()).unwrap_or_default()
            } else {
                format!("{n} {what}")
            };
        }
        self.omitted = n.saturating_sub(MAX_FINDINGS);
        self.findings = findings;
        self.findings.truncate(MAX_FINDINGS);
        self
    }

    pub fn skip(mut self, why: impl Into<String>) -> Self {
        self.status = Status::Skipped;
        self.message = why.into();
        self
    }

    pub fn error(mut self, why: impl Into<String>) -> Self {
        self.status = Status::Error;
        self.message = why.into();
        self
    }

    pub fn at(mut self, level: Level) -> Self {
        self.level = level;
        self
    }

    /// Whether this case fails a run that fails on `fail_on`.
    pub fn gates(&self, fail_on: Level) -> bool {
        matches!(self.status, Status::Failed | Status::Error) && self.level >= fail_on
    }
}

/// Output formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Format {
    Json,
    Text,
    Junit,
    Sarif,
}

impl Format {
    fn parse(s: &str) -> Result<Self, String> {
        match s {
            "json" => Ok(Format::Json),
            "text" => Ok(Format::Text),
            "junit" => Ok(Format::Junit),
            "sarif" => Ok(Format::Sarif),
            other => Err(format!(
                "parameter `format`: `{other}` is not json, text, junit or sarif"
            )),
        }
    }
}

/// Where the test file came from.
pub(crate) struct TestSource {
    /// The file (absolute, as read), `None` for inline tests.
    pub path: Option<PathBuf>,
    /// The folder relative paths in the file are resolved against.
    pub dir: PathBuf,
    /// The file's text (for locating tests in it).
    pub text: Option<String>,
}

/// Everything the run produced.
pub(crate) struct Run {
    pub spec_name: Option<String>,
    pub cases: Vec<Case>,
    pub notes: Vec<String>,
    pub fail_on: Level,
    /// Paths shown in results (relative to the base folder).
    pub tests_file: Option<String>,
    pub project: Option<String>,
}

impl Run {
    pub fn count(&self, status: Status) -> usize {
        self.cases.iter().filter(|c| c.status == status).count()
    }
    pub fn passed(&self) -> bool {
        !self.cases.iter().any(|c| c.gates(self.fail_on))
    }
    /// Failed or errored cases below `fail_on`: reported, but they do not fail the run.
    pub fn warnings(&self) -> usize {
        self.cases
            .iter()
            .filter(|c| matches!(c.status, Status::Failed | Status::Error) && !c.gates(self.fail_on))
            .count()
    }
}

/// `project.test`.
pub(crate) fn project_test(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["path", "tests", "format", "fail_on", "base_dir"])?;
    let formats = formats(p.get("format"))?;
    let fail_on = match opt_str(p, "fail_on")? {
        None => Level::Error,
        Some("error") => Level::Error,
        Some("warning") => Level::Warning,
        Some(other) => return Err(format!("parameter `fail_on`: `{other}` is not error or warning")),
    };
    let base_dir = opt_str(p, "base_dir")?
        .filter(|b| !b.trim().is_empty())
        .map(PathBuf::from);
    match opt_str(p, "path")?.filter(|x| !x.trim().is_empty()) {
        Some(raw) => {
            // a private session: the caller's open file, edits and caches are left alone; in
            // server mode it resolves `path` under the root and counts against the same
            // process memory limit as every session
            let mut private = s.private();
            private.set_cancel(s.cancel.clone());
            private.execute("file.open", &json!({ "path": raw }))?;
            run(&mut private, p, &formats, fail_on, base_dir.as_deref())
        }
        None => run(s, p, &formats, fail_on, base_dir.as_deref()),
    }
}

fn run(
    s: &mut Session,
    p: &Value,
    formats: &[Format],
    fail_on: Level,
    base_dir: Option<&Path>,
) -> Result<Value, String> {
    let Some(source) = s.source.as_ref() else {
        return Err("no file is open: pass `path` (a .pbip, project folder, .pbix, .pbit or model.bim) or run file.open first".into());
    };
    let project_path = source_path(source).to_path_buf();
    let project_dir = project_folder(source);
    let (spec, tests) = load_spec(s, p.get("tests"), &project_dir)?;
    let base = match base_dir {
        Some(b) => b.to_path_buf(),
        None => project_dir.clone(),
    };

    let mut cases = Vec::new();
    let mut notes = Vec::new();
    cases.extend(crate::citest_dax::run(s, &spec.dax));
    if let Some(m) = &spec.model {
        cases.extend(crate::citest_rules::model_cases(s, m, &tests)?);
    }
    if let Some(r) = &spec.report {
        cases.extend(crate::citest_rules::report_cases(s, r));
    }
    if let Some(u) = &spec.usage {
        let (c, note) = crate::citest_rules::usage_cases(s, u);
        cases.extend(c);
        notes.extend(note);
    }
    if spec.dax.is_empty() && spec.model.is_none() && spec.report.is_none() && spec.usage.is_none() {
        notes.push("the test file defines no test".into());
    }
    crate::citest_loc::locate(s, &mut cases, &tests, &base);

    let shown = |path: &Path| crate::citest_loc::relative(path, &base, s.server_config());
    let run = Run {
        spec_name: spec.name.clone(),
        cases,
        notes,
        fail_on,
        tests_file: tests.path.as_deref().map(shown),
        project: Some(shown(&project_path)),
    };
    let mut out = crate::citest_out::json(&run);
    let render = |f: Format| match f {
        Format::Text => crate::citest_out::text(&run),
        Format::Junit => crate::citest_out::junit(&run),
        Format::Sarif => crate::citest_out::sarif(&run),
        Format::Json => String::new(),
    };
    let rendered: Vec<Format> = formats.iter().copied().filter(|f| *f != Format::Json).collect();
    if let Some(o) = out.as_object_mut() {
        match rendered.as_slice() {
            [] => {}
            [one] if formats.len() == 1 => {
                o.insert("format".into(), json!(format_name(*one)));
                o.insert("output".into(), json!(render(*one)));
            }
            many => {
                let mut outputs = serde_json::Map::new();
                for f in many {
                    outputs.insert(format_name(*f).into(), json!(render(*f)));
                }
                o.insert("outputs".into(), Value::Object(outputs));
            }
        }
    }
    Ok(out)
}

/// The `format` parameter: one name, or a list of names (each rendered once, in `outputs`).
fn formats(v: Option<&Value>) -> Result<Vec<Format>, String> {
    match v {
        None | Some(Value::Null) => Ok(vec![Format::Json]),
        Some(Value::String(s)) => Ok(vec![Format::parse(s)?]),
        Some(Value::Array(items)) if !items.is_empty() && items.len() <= 4 => {
            let mut out = Vec::new();
            for x in items {
                let f = Format::parse(x.as_str().unwrap_or_default())?;
                if !out.contains(&f) {
                    out.push(f);
                }
            }
            Ok(out)
        }
        Some(_) => Err("parameter `format` must be json, text, junit, sarif or a list of them".into()),
    }
}

fn format_name(f: Format) -> &'static str {
    match f {
        Format::Json => "json",
        Format::Text => "text",
        Format::Junit => "junit",
        Format::Sarif => "sarif",
    }
}

/// The folder holding the project (where the test file is looked for).
pub(crate) fn project_folder(source: &Source) -> PathBuf {
    let path = source_path(source);
    match source {
        Source::Project { project, .. } => project.root.clone(),
        _ if path.is_dir() => path.to_path_buf(),
        _ => path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(".")),
    }
}

/// Reads the `tests` parameter (a path, inline tests) or the test file next to the project.
fn load_spec(s: &Session, tests: Option<&Value>, project_dir: &Path) -> Result<(Spec, TestSource), String> {
    let (path, doc, text) = match tests {
        Some(v @ Value::Object(_)) => (None, v.clone(), None),
        Some(Value::String(raw)) if !raw.trim().is_empty() => {
            let path = PathBuf::from(raw);
            let (doc, text) = read_file(&path)?;
            (Some(path), doc, Some(text))
        }
        None | Some(Value::Null) => {
            let found = TEST_FILES
                .iter()
                .map(|n| project_dir.join(n))
                .find(|p| p.is_file())
                .ok_or_else(|| {
                    format!(
                        "no test file: put {} next to the project, or pass `tests` (a file or inline tests)",
                        TEST_FILES.join(", ")
                    )
                })?;
            if let Some(server) = s.server_config()
                && !server.contains(&found)
            {
                return Err(
                    "server mode: the test file next to the project leaves the workspace root".into(),
                );
            }
            let (doc, text) = read_file(&found)?;
            (Some(found), doc, Some(text))
        }
        Some(_) => {
            return Err("parameter `tests` must be a test file path or inline tests (an object)".into());
        }
    };
    let spec = crate::citest_spec::parse(&doc).map_err(|e| match &path {
        Some(p) => format!("{}: {e}", file_name(p)),
        None => format!("tests: {e}"),
    })?;
    let dir = path
        .as_deref()
        .and_then(Path::parent)
        .map_or_else(|| project_dir.to_path_buf(), Path::to_path_buf);
    Ok((spec, TestSource { path, dir, text }))
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// Reads and parses a test file: `.json` as JSON, anything else as YAML.
pub(crate) fn read_file(path: &Path) -> Result<(Value, String), String> {
    let name = file_name(path);
    let meta = std::fs::metadata(path).map_err(|e| format!("{name}: {e}"))?;
    if !meta.is_file() {
        return Err(format!("{name}: not a file"));
    }
    if meta.len() > crate::citest_yaml::MAX_TEXT_BYTES as u64 {
        return Err(format!(
            "{name}: larger than {} bytes",
            crate::citest_yaml::MAX_TEXT_BYTES
        ));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{name}: {e}"))?;
    let text = String::from_utf8(bytes).map_err(|_| format!("{name}: not UTF-8 text"))?;
    let json = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("json"));
    let doc = if json {
        serde_json::from_str(text.trim_start_matches('\u{feff}'))
            .map_err(|e| format!("{name}: not valid JSON: {e}"))?
    } else {
        crate::citest_yaml::parse(&text).map_err(|e| format!("{name}: not valid YAML: {e}"))?
    };
    Ok((doc, text))
}

/// Milliseconds since `t`.
pub(crate) fn elapsed(t: Instant) -> u64 {
    u64::try_from(t.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
#[path = "citest_tests.rs"]
mod tests;
