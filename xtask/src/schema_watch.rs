//! `cargo xtask schema-watch [--json] [--no-diff]`: tells when Microsoft publishes newer versions
//! of the formats bicraft writes or reads (docs/schema-watch.md).
//!
//! Ours: the table in `bicraft-versions` (what the writers use), the PBIR schemas vendored under
//! `crates/pbir/schemas/` (what the validator knows) and the vendored report theme schema.
//! Theirs: one GitHub API call for the tree of microsoft/json-schemas (`fabric/…`) and one for
//! the "Report Theme JSON Schema" folder of microsoft/powerbi-desktop-samples; schema bodies for
//! the diffs come from raw.githubusercontent.com.
//!
//! Exit codes: 0 nothing needs action, 1 action needed (newer major, a written schema no longer
//! published, a new Power BI schema bicraft does not know), 2 network or setup failure.
//!
//! The network client (`ureq`) sits behind the `net` feature, so `cargo xtask ci` and every other
//! command build without it; without the feature this command re-runs xtask with it. The
//! comparison logic below is pure and tested offline.

// Without `net`, only the pure part is compiled for the tests.
#![cfg_attr(not(feature = "net"), allow(dead_code))]

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::ExitCode;

use serde_json::{Value, json};

const JSON_SCHEMAS_TREE: &str =
    "https://api.github.com/repos/microsoft/json-schemas/git/trees/main?recursive=1";
const JSON_SCHEMAS_RAW: &str = "https://raw.githubusercontent.com/microsoft/json-schemas/main/fabric/";
const THEMES_DIR: &str =
    "https://api.github.com/repos/microsoft/powerbi-desktop-samples/contents/Report%20Theme%20JSON%20Schema";
const THEMES_RAW: &str = "https://raw.githubusercontent.com/microsoft/powerbi-desktop-samples/main/Report%20Theme%20JSON%20Schema/";
const THEME_PREFIX: &str = "reportThemeSchema-";
/// Pseudo key of the report theme schema in the report.
const THEME_KEY: &str = "powerbi-desktop-samples/reportThemeSchema";
/// Most changes listed per schema diff (the count is always exact).
const MAX_DIFF_LINES: usize = 40;
/// Deepest schema nesting walked by the diff.
const MAX_DEPTH: usize = 64;

pub const EXIT_OK: u8 = 0;
pub const EXIT_ACTION: u8 = 1;
pub const EXIT_FAILURE: u8 = 2;

// --- Versions ---------------------------------------------------------------------------------

/// A dotted version (`2.12.0`, `2.157`), compared numerically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version(Vec<u64>);

impl Version {
    pub fn parse(s: &str) -> Option<Version> {
        let parts: Option<Vec<u64>> = s.split('.').map(|p| p.parse().ok()).collect();
        let parts = parts?;
        (!parts.is_empty() && parts.len() <= 4).then_some(Version(parts))
    }

    fn major(&self) -> u64 {
        self.0.first().copied().unwrap_or(0)
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        let n = self.0.len().max(other.0.len());
        (0..n)
            .map(|i| {
                let a = self.0.get(i).copied().unwrap_or(0);
                let b = other.0.get(i).copied().unwrap_or(0);
                a.cmp(&b)
            })
            .find(|o| o.is_ne())
            .unwrap_or(Ordering::Equal)
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let parts: Vec<String> = self.0.iter().map(u64::to_string).collect();
        f.write_str(&parts.join("."))
    }
}

// --- Inputs -----------------------------------------------------------------------------------

/// Published schemas: key (folder under `fabric/`) → versions, sorted ascending.
pub type Published = BTreeMap<String, Vec<Version>>;

/// Parses a GitHub git-tree listing of microsoft/json-schemas.
pub fn parse_tree(tree: &Value) -> Result<(Published, String), String> {
    let entries = tree
        .get("tree")
        .and_then(Value::as_array)
        .ok_or("the GitHub tree listing has no `tree` array")?;
    if tree.get("truncated").and_then(Value::as_bool) == Some(true) {
        return Err("the GitHub tree listing is truncated".into());
    }
    let sha = tree.get("sha").and_then(Value::as_str).unwrap_or("").to_string();
    let mut out = Published::new();
    for e in entries {
        if e.get("type").and_then(Value::as_str) != Some("blob") {
            continue;
        }
        let Some(path) = e.get("path").and_then(Value::as_str) else {
            continue;
        };
        let Some(rest) = path
            .strip_prefix("fabric/")
            .and_then(|p| p.strip_suffix("/schema.json"))
        else {
            continue;
        };
        let Some((key, version)) = rest.rsplit_once('/') else {
            continue;
        };
        let Some(version) = Version::parse(version) else {
            continue;
        };
        out.entry(key.to_string()).or_default().push(version);
    }
    for versions in out.values_mut() {
        versions.sort();
        versions.dedup();
    }
    Ok((out, sha))
}

/// Report theme schema versions from a GitHub contents listing (`reportThemeSchema-2.157.json`).
pub fn parse_theme_listing(listing: &Value) -> Result<Vec<Version>, String> {
    let entries = listing
        .as_array()
        .ok_or("the theme schema listing is not an array")?;
    let mut out: Vec<Version> = entries
        .iter()
        .filter_map(|e| e.get("name").and_then(Value::as_str))
        .filter_map(|n| n.strip_prefix(THEME_PREFIX)?.strip_suffix(".json"))
        .filter_map(Version::parse)
        .collect();
    out.sort();
    out.dedup();
    Ok(out)
}

/// What bicraft has for one schema key.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ours {
    pub key: String,
    pub files: String,
    /// Version written for new files (`bicraft_versions::WRITTEN`).
    pub written: Option<Version>,
    /// Newest version vendored for validation (`crates/pbir/schemas/`, the theme schema).
    pub vendored: Option<Version>,
    /// Known but not written (`bicraft_versions::KNOWN`): newest version seen at the last review.
    pub known: Option<Version>,
}

/// Ours, from the versions table plus the vendored schema names (`(name, versions)` of
/// `crates/pbir/schemas/<name>/<version>/`).
pub fn ours(vendored_pbir: &[(String, Vec<Version>)]) -> Vec<Ours> {
    let mut out: Vec<Ours> = Vec::new();
    for s in bicraft_versions::WRITTEN {
        out.push(Ours {
            key: s.key.into(),
            files: s.files.into(),
            written: Version::parse(s.version),
            ..Ours::default()
        });
    }
    for s in bicraft_versions::KNOWN {
        out.push(Ours {
            key: s.key.into(),
            files: s.files.into(),
            known: Version::parse(s.version),
            ..Ours::default()
        });
    }
    for (name, versions) in vendored_pbir {
        let key = format!("item/report/definition/{name}");
        let newest = versions.iter().max().cloned();
        match out.iter_mut().find(|o| o.key == key) {
            Some(o) => o.vendored = newest,
            None => out.push(Ours {
                key,
                files: "vendored for validation".into(),
                vendored: newest,
                ..Ours::default()
            }),
        }
    }
    out
}

fn theme_ours() -> Ours {
    Ours {
        key: THEME_KEY.into(),
        files: "report theme JSON (crates/visuals/schemas/)".into(),
        vendored: Version::parse(bicraft_versions::REPORT_THEME_SCHEMA_VERSION),
        ..Ours::default()
    }
}

// --- Comparison -------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    UpToDate,
    /// Ours is newer than anything published (Desktop ships schemas before they are public).
    Ahead,
    NewerMinor,
    /// The key, or the exact version written, is not published.
    NotPublished,
    NewerMajor,
}

impl Status {
    fn label(self) -> &'static str {
        match self {
            Status::UpToDate => "up to date",
            Status::Ahead => "ahead of published",
            Status::NewerMinor => "newer minor",
            Status::NotPublished => "not published",
            Status::NewerMajor => "NEWER MAJOR",
        }
    }

    fn id(self) -> &'static str {
        match self {
            Status::UpToDate => "up_to_date",
            Status::Ahead => "ahead",
            Status::NewerMinor => "newer_minor",
            Status::NotPublished => "not_published",
            Status::NewerMajor => "newer_major",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub ours: Ours,
    pub latest: Option<Version>,
    pub status: Status,
    pub action: bool,
}

fn compare(ours: &Version, latest: &Version) -> Status {
    match ours.cmp(latest) {
        Ordering::Equal => Status::UpToDate,
        Ordering::Greater => Status::Ahead,
        Ordering::Less if latest.major() > ours.major() => Status::NewerMajor,
        Ordering::Less => Status::NewerMinor,
    }
}

/// Status of one of our keys against the published versions.
pub fn assess(ours: &Ours, published: Option<&[Version]>) -> Row {
    let latest = published.and_then(|v| v.iter().max()).cloned();
    let Some(latest_v) = &latest else {
        return Row {
            ours: ours.clone(),
            latest,
            status: Status::NotPublished,
            action: ours.written.is_some(),
        };
    };
    let mut status = Status::UpToDate;
    let mut action = false;
    if let Some(w) = &ours.written {
        let s = compare(w, latest_v);
        let unpublished = !published.is_some_and(|p| p.contains(w));
        let s = if unpublished { Status::NotPublished } else { s };
        action |= matches!(s, Status::NewerMajor | Status::NotPublished);
        status = status.max(s);
    }
    if let Some(v) = &ours.vendored {
        let s = compare(v, latest_v);
        action |= s == Status::NewerMajor;
        status = status.max(s);
    }
    if ours.written.is_none()
        && ours.vendored.is_none()
        && let Some(k) = &ours.known
    {
        status = compare(k, latest_v);
    }
    Row {
        ours: ours.clone(),
        latest,
        status,
        action,
    }
}

#[derive(Debug, Clone, Default)]
pub struct Report {
    pub source_sha: String,
    pub rows: Vec<Row>,
    /// Published under a watched folder, unknown to bicraft (action).
    pub new: Vec<(String, Version)>,
    /// Other published schemas (other Fabric item types), for information.
    pub unused: Vec<(String, Version)>,
    pub theme: Option<Row>,
    pub diffs: Vec<Diff>,
}

#[derive(Debug, Clone, Default)]
pub struct Diff {
    pub key: String,
    pub from: String,
    pub to: String,
    pub changes: Vec<String>,
    pub error: Option<String>,
}

pub fn build_report(
    ours: &[Ours],
    published: &Published,
    theme: Option<&[Version]>,
    watched: &[&str],
) -> Report {
    let rows = ours
        .iter()
        .map(|o| assess(o, published.get(&o.key).map(Vec::as_slice)))
        .collect();
    let mut new = Vec::new();
    let mut unused = Vec::new();
    for (key, versions) in published {
        if ours.iter().any(|o| &o.key == key) {
            continue;
        }
        let Some(latest) = versions.iter().max().cloned() else {
            continue;
        };
        if watched.iter().any(|p| key.starts_with(p)) {
            new.push((key.clone(), latest));
        } else {
            unused.push((key.clone(), latest));
        }
    }
    Report {
        rows,
        new,
        unused,
        theme: theme.map(|t| assess(&theme_ours(), Some(t))),
        ..Report::default()
    }
}

pub fn exit_code(report: &Report) -> u8 {
    let action = report.rows.iter().any(|r| r.action)
        || !report.new.is_empty()
        || report.theme.as_ref().is_some_and(|t| t.action);
    if action { EXIT_ACTION } else { EXIT_OK }
}

/// `(key, ours, latest)` of the rows whose published version is newer than ours.
fn newer(report: &Report) -> Vec<(String, Version, Version)> {
    let mut out = Vec::new();
    for row in report.rows.iter().chain(report.theme.as_ref()) {
        let Some(latest) = &row.latest else { continue };
        let base = row
            .ours
            .written
            .as_ref()
            .or(row.ours.vendored.as_ref())
            .or(row.ours.known.as_ref());
        if let Some(base) = base.filter(|b| *b < latest) {
            out.push((row.ours.key.clone(), base.clone(), latest.clone()));
        }
    }
    out
}

// --- Schema diff ------------------------------------------------------------------------------

#[derive(Default)]
struct Shape {
    /// Property paths (`Definition.prop.sub`).
    props: BTreeSet<String>,
    /// Required property paths.
    required: BTreeSet<String>,
    /// `path = value` of enum members.
    enums: BTreeSet<String>,
    /// External `$ref` targets (other schema files).
    refs: BTreeSet<String>,
}

fn shape(schema: &Value) -> Shape {
    let mut s = Shape::default();
    walk(schema, "", 0, &mut s);
    s
}

fn join(path: &str, name: &str) -> String {
    if path.is_empty() {
        name.to_string()
    } else {
        format!("{path}.{name}")
    }
}

fn walk(v: &Value, path: &str, depth: usize, s: &mut Shape) {
    if depth > MAX_DEPTH {
        return;
    }
    match v {
        Value::Object(map) => {
            for (k, child) in map {
                match k.as_str() {
                    "properties" => {
                        if let Some(props) = child.as_object() {
                            for (name, p) in props {
                                let at = join(path, name);
                                s.props.insert(at.clone());
                                walk(p, &at, depth + 1, s);
                            }
                        }
                    }
                    "definitions" | "$defs" => {
                        if let Some(defs) = child.as_object() {
                            for (name, d) in defs {
                                // Definitions are named from the root, wherever they sit.
                                s.props.insert(format!("#{name}"));
                                walk(d, &format!("#{name}"), depth + 1, s);
                            }
                        }
                    }
                    "required" => {
                        for r in child.as_array().into_iter().flatten() {
                            if let Some(r) = r.as_str() {
                                s.required.insert(join(path, r));
                            }
                        }
                    }
                    "enum" => {
                        for e in child.as_array().into_iter().flatten() {
                            s.enums
                                .insert(format!("{} = {e}", if path.is_empty() { "(root)" } else { path }));
                        }
                    }
                    "$ref" => {
                        if let Some(r) = child.as_str().filter(|r| !r.starts_with('#')) {
                            s.refs.insert(strip_versions(r));
                        }
                    }
                    _ => walk(child, path, depth + 1, s),
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                walk(item, path, depth + 1, s);
            }
        }
        _ => {}
    }
}

/// Keeps a `$ref` as written; the version inside it is what changes.
fn strip_versions(r: &str) -> String {
    r.split('#').next().unwrap_or(r).to_string()
}

/// Added / removed properties, required changes, enum members and referenced schema versions.
pub fn diff_schemas(old: &Value, new: &Value) -> Vec<String> {
    let (a, b) = (shape(old), shape(new));
    let mut out = Vec::new();
    let mut set = |what: &str, a: &BTreeSet<String>, b: &BTreeSet<String>| {
        for x in b.difference(a) {
            out.push(format!("+ {what} {x}"));
        }
        for x in a.difference(b) {
            out.push(format!("- {what} {x}"));
        }
    };
    set("property", &a.props, &b.props);
    set("required", &a.required, &b.required);
    set("enum", &a.enums, &b.enums);
    set("$ref", &a.refs, &b.refs);
    out
}

// --- Output -----------------------------------------------------------------------------------

fn opt(v: &Option<Version>) -> String {
    v.as_ref().map(Version::to_string).unwrap_or_else(|| "-".into())
}

fn row_json(r: &Row) -> Value {
    json!({
        "key": r.ours.key,
        "files": r.ours.files,
        "written": r.ours.written.as_ref().map(Version::to_string),
        "vendored": r.ours.vendored.as_ref().map(Version::to_string),
        "known": r.ours.known.as_ref().map(Version::to_string),
        "latest": r.latest.as_ref().map(Version::to_string),
        "status": r.status.id(),
        "action": r.action,
    })
}

pub fn to_json(report: &Report, code: u8) -> Value {
    let pairs = |v: &[(String, Version)]| -> Vec<Value> {
        v.iter()
            .map(|(k, ver)| json!({"key": k, "latest": ver.to_string()}))
            .collect()
    };
    json!({
        "source": {
            "json_schemas": "https://github.com/microsoft/json-schemas (fabric/)",
            "tree_sha": report.source_sha,
            "theme_schemas": "https://github.com/microsoft/powerbi-desktop-samples (Report Theme JSON Schema/)",
        },
        "schemas": report.rows.iter().map(row_json).collect::<Vec<_>>(),
        "theme": report.theme.as_ref().map(row_json),
        "new": pairs(&report.new),
        "unused": pairs(&report.unused),
        "file_versions": bicraft_versions::FILE_VERSIONS
            .iter()
            .map(|v| json!({"what": v.what, "value": v.value}))
            .collect::<Vec<_>>(),
        "desktop": {
            "baseline": bicraft_versions::DESKTOP_BASELINE,
            "latest": null,
            "note": "not checked online: no stable public source (the Download Center page refuses scripted requests); the newest report theme schema tracks Desktop's monthly releases",
        },
        "diffs": report.diffs.iter().map(|d| json!({
            "key": d.key, "from": d.from, "to": d.to, "changes": d.changes, "error": d.error,
        })).collect::<Vec<_>>(),
        "exit_code": code,
    })
}

pub fn to_text(report: &Report, code: u8) -> String {
    let mut s = String::new();
    let mut line = |t: String| {
        s.push_str(&t);
        s.push('\n');
    };
    line(format!(
        "Power BI schema watch: bicraft vs microsoft/json-schemas (tree {})\n",
        report.source_sha.get(..12).unwrap_or(&report.source_sha)
    ));
    line(format!(
        "{:<52} {:>8} {:>8} {:>8} {:>8}  status",
        "schema (fabric/...)", "writes", "vendored", "known", "latest"
    ));
    for r in report.rows.iter().chain(report.theme.as_ref()) {
        line(format!(
            "{:<52} {:>8} {:>8} {:>8} {:>8}  {}{}",
            r.ours.key,
            opt(&r.ours.written),
            opt(&r.ours.vendored),
            opt(&r.ours.known),
            opt(&r.latest),
            r.status.label(),
            if r.action { "  <- ACTION" } else { "" }
        ));
    }
    if !report.new.is_empty() {
        line(String::new());
        line(
            "New Power BI schemas bicraft does not know (ACTION: review, then add to bicraft-versions):"
                .into(),
        );
        for (k, v) in &report.new {
            line(format!("  {k} {v}"));
        }
    }
    let mut items: BTreeSet<&str> = BTreeSet::new();
    for (k, _) in &report.unused {
        let mut parts = k.split('/');
        let item = match (parts.next(), parts.next()) {
            (Some("item"), Some(t)) => t,
            (Some(first), _) => first,
            _ => k.as_str(),
        };
        items.insert(item);
    }
    if !items.is_empty() {
        line(String::new());
        line(format!(
            "Other published schemas, not Power BI Desktop files ({} schemas; --json lists them): {}",
            report.unused.len(),
            items.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    line(String::new());
    line(
        "Versions not published as schemas (bicraft-versions; checked against files Desktop writes, not online):"
            .into(),
    );
    for v in bicraft_versions::FILE_VERSIONS {
        line(format!("  {:<48} {}", v.what, v.value));
    }
    line("  latest Power BI Desktop: not checked (no stable public source)".into());
    for d in &report.diffs {
        line(String::new());
        line(format!("Changes in {} {} -> {}:", d.key, d.from, d.to));
        if let Some(e) = &d.error {
            line(format!("  (diff unavailable: {e})"));
        } else if d.changes.is_empty() {
            line("  no property, required, enum or $ref change (descriptions/constraints only)".into());
        }
        for c in d.changes.iter().take(MAX_DIFF_LINES) {
            line(format!("  {c}"));
        }
        if d.changes.len() > MAX_DIFF_LINES {
            line(format!(
                "  ... {} more (--json lists them all)",
                d.changes.len() - MAX_DIFF_LINES
            ));
        }
    }
    line(String::new());
    line(match code {
        EXIT_OK => "Result: nothing needs action (exit 0).".into(),
        _ => "Result: ACTION NEEDED (exit 1), see docs/schema-watch.md.".into(),
    });
    s
}

// --- Command ----------------------------------------------------------------------------------

/// `crates/pbir/schemas/<name>/<version>/schema.json`.
fn vendored_pbir(root: &Path) -> Result<Vec<(String, Vec<Version>)>, String> {
    let dir = root.join("crates").join("pbir").join("schemas");
    let read = |d: &Path| std::fs::read_dir(d).map_err(|e| format!("{}: {e}", d.display()));
    let mut out = Vec::new();
    for entry in read(&dir)?.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let mut versions: Vec<Version> = read(&path)?
            .flatten()
            .filter(|v| v.path().join("schema.json").is_file())
            .filter_map(|v| Version::parse(&v.file_name().to_string_lossy()))
            .collect();
        versions.sort();
        out.push((name, versions));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

pub fn cmd(args: &[String]) -> ExitCode {
    let mut as_json = false;
    let mut with_diff = true;
    for a in args {
        match a.as_str() {
            "--json" => as_json = true,
            "--no-diff" => with_diff = false,
            other => {
                eprintln!("error: schema-watch: unknown argument `{other}` (expected --json, --no-diff)");
                return ExitCode::from(EXIT_FAILURE);
            }
        }
    }
    run(as_json, with_diff, args)
}

#[cfg(not(feature = "net"))]
fn run(_: bool, _: bool, args: &[String]) -> ExitCode {
    // Re-run with the HTTP client compiled in, in its own target dir: Windows cannot replace the
    // running xtask.exe.
    let base = std::env::var_os("CARGO_TARGET_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| crate::root().join("target"));
    let target = base.join("xtask-net");
    let mut c = crate::cargo();
    c.args([
        "run",
        "--quiet",
        "-p",
        "xtask",
        "--features",
        "net",
        "--target-dir",
    ]);
    c.arg(&target);
    c.args(["--", "schema-watch"]);
    c.args(args);
    match c.status() {
        Ok(s) => match s.code() {
            Some(0) => ExitCode::from(EXIT_OK),
            Some(1) => ExitCode::from(EXIT_ACTION),
            // The child already said why.
            Some(2) => ExitCode::from(EXIT_FAILURE),
            _ => {
                eprintln!(
                    "error: schema-watch failed ({s}); a build failure here usually means the `net` dependencies could not be downloaded"
                );
                ExitCode::from(EXIT_FAILURE)
            }
        },
        Err(e) => {
            eprintln!("error: schema-watch: cannot run cargo: {e}");
            ExitCode::from(EXIT_FAILURE)
        }
    }
}

#[cfg(feature = "net")]
fn run(as_json: bool, with_diff: bool, _: &[String]) -> ExitCode {
    match fetch_and_report(with_diff) {
        Ok(report) => {
            let code = exit_code(&report);
            if as_json {
                let text = serde_json::to_string_pretty(&to_json(&report, code))
                    .unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"));
                println!("{text}");
            } else {
                print!("{}", to_text(&report, code));
            }
            ExitCode::from(code)
        }
        Err(e) => {
            eprintln!("error: schema-watch: {e}");
            ExitCode::from(EXIT_FAILURE)
        }
    }
}

#[cfg(feature = "net")]
fn fetch_and_report(with_diff: bool) -> Result<Report, String> {
    let vendored = vendored_pbir(&crate::root())?;
    let tree: Value = serde_json::from_str(&net::get(JSON_SCHEMAS_TREE, net::API_LIMIT)?)
        .map_err(|e| format!("{JSON_SCHEMAS_TREE}: bad JSON: {e}"))?;
    let (published, sha) = parse_tree(&tree)?;
    let themes: Value = serde_json::from_str(&net::get(THEMES_DIR, net::API_LIMIT)?)
        .map_err(|e| format!("{THEMES_DIR}: bad JSON: {e}"))?;
    let themes = parse_theme_listing(&themes)?;
    let mut report = build_report(
        &ours(&vendored),
        &published,
        Some(&themes),
        bicraft_versions::WATCHED_PREFIXES,
    );
    report.source_sha = sha;
    if with_diff {
        for (key, from, to) in newer(&report) {
            let url = |v: &Version| {
                if key == THEME_KEY {
                    format!("{THEMES_RAW}{THEME_PREFIX}{v}.json")
                } else {
                    format!("{JSON_SCHEMAS_RAW}{key}/{v}/schema.json")
                }
            };
            let fetch = |v: &Version| -> Result<Value, String> {
                let u = url(v);
                serde_json::from_str(&net::get(&u, net::SCHEMA_LIMIT)?)
                    .map_err(|e| format!("{u}: bad JSON: {e}"))
            };
            let mut d = Diff {
                key: key.clone(),
                from: from.to_string(),
                to: to.to_string(),
                ..Diff::default()
            };
            match (fetch(&from), fetch(&to)) {
                (Ok(a), Ok(b)) => d.changes = diff_schemas(&a, &b),
                (Err(e), _) | (_, Err(e)) => d.error = Some(e),
            }
            report.diffs.push(d);
        }
    }
    Ok(report)
}

#[cfg(feature = "net")]
mod net {
    use std::io::Read as _;
    use std::time::Duration;

    /// Largest API listing read (the json-schemas tree is ~0.3 MB).
    pub const API_LIMIT: u64 = 16 << 20;
    /// Largest schema read (the report theme schema is ~1.2 MB).
    pub const SCHEMA_LIMIT: u64 = 32 << 20;

    pub fn get(url: &str, limit: u64) -> Result<String, String> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(60)))
            .build()
            .into();
        let response = agent
            .get(url)
            .header("User-Agent", "bicraft-xtask-schema-watch")
            .header("Accept", "application/vnd.github+json")
            .call()
            .map_err(|e| match e {
                ureq::Error::StatusCode(403 | 429) => format!(
                    "{url}: HTTP rate limit reached (GitHub allows 60 unauthenticated API calls per hour; retry later)"
                ),
                e => format!("{url}: {e}"),
            })?;
        let mut text = String::new();
        response
            .into_body()
            .into_reader()
            .take(limit)
            .read_to_string(&mut text)
            .map_err(|e| format!("{url}: {e}"))?;
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    /// A canned git-tree listing shaped like GitHub's.
    fn canned_tree(paths: &[&str]) -> Value {
        let tree: Vec<Value> = paths
            .iter()
            .map(|p| json!({"path": p, "type": "blob", "mode": "100644"}))
            .chain(std::iter::once(json!({"path": "fabric/item", "type": "tree"})))
            .collect();
        json!({"sha": "abc123", "truncated": false, "tree": tree})
    }

    fn row<'a>(r: &'a Report, key: &str) -> &'a Row {
        r.rows.iter().find(|r| r.ours.key == key).unwrap()
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(v("2.12.0") > v("2.9.0"));
        assert!(v("2.157") < v("2.158"));
        assert_eq!(v("1.0"), v("1.0"));
        assert_eq!(v("1.0").cmp(&v("1.0.0")), Ordering::Equal);
        assert!(Version::parse("1.x").is_none());
        assert!(Version::parse("").is_none());
        assert_eq!(v("2.10.0").to_string(), "2.10.0");
    }

    #[test]
    fn tree_listing_is_parsed_and_embedded_variants_ignored() {
        let tree = canned_tree(&[
            "fabric/item/report/definition/page/2.1.0/schema.json",
            "fabric/item/report/definition/page/2.0.0/schema.json",
            "fabric/item/report/definition/filterConfiguration/1.3.0/schema-embedded.json",
            "fabric/item/report/definition/page/CHANGELOG.md",
            "other/thing/1.0.0/schema.json",
        ]);
        let (p, sha) = parse_tree(&tree).unwrap();
        assert_eq!(sha, "abc123");
        assert_eq!(p.len(), 1);
        assert_eq!(p["item/report/definition/page"], vec![v("2.0.0"), v("2.1.0")]);
        let truncated = json!({"truncated": true, "tree": []});
        assert!(parse_tree(&truncated).is_err());
        assert!(parse_tree(&json!({"message": "rate limited"})).is_err());
    }

    #[test]
    fn theme_listing_is_parsed() {
        let listing = json!([
            {"name": "reportThemeSchema-2.157.json"},
            {"name": "reportThemeSchema-2.99.json"},
            {"name": "README.md"},
        ]);
        assert_eq!(
            parse_theme_listing(&listing).unwrap(),
            vec![v("2.99"), v("2.157")]
        );
        assert!(parse_theme_listing(&json!({"message": "x"})).is_err());
    }

    /// Every written and known key published at exactly our version.
    fn all_current() -> Vec<String> {
        bicraft_versions::WRITTEN
            .iter()
            .chain(bicraft_versions::KNOWN)
            .map(|s| format!("fabric/{}/{}/schema.json", s.key, s.version))
            .collect()
    }

    fn report_for(extra: &[&str], drop_key: Option<&str>, theme: &[&str]) -> Report {
        let mut paths = all_current();
        if let Some(k) = drop_key {
            paths.retain(|p| !p.contains(&format!("/{k}/")));
        }
        paths.extend(extra.iter().map(|s| s.to_string()));
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        let (published, _) = parse_tree(&canned_tree(&refs)).unwrap();
        let theme: Vec<Version> = theme.iter().map(|t| v(t)).collect();
        build_report(
            &ours(&[]),
            &published,
            Some(&theme),
            bicraft_versions::WATCHED_PREFIXES,
        )
    }

    #[test]
    fn everything_current_needs_no_action() {
        let r = report_for(
            &["fabric/item/plan/definition/cube/1.0.0/schema.json"],
            None,
            &[bicraft_versions::REPORT_THEME_SCHEMA_VERSION],
        );
        assert!(
            r.rows.iter().all(|r| r.status == Status::UpToDate),
            "{:?}",
            r.rows
        );
        assert!(r.new.is_empty());
        assert_eq!(
            r.unused,
            vec![("item/plan/definition/cube".to_string(), v("1.0.0"))]
        );
        assert_eq!(exit_code(&r), EXIT_OK);
        assert!(newer(&r).is_empty());
        let text = to_text(&r, exit_code(&r));
        assert!(text.contains("nothing needs action"), "{text}");
        assert_eq!(to_json(&r, 0)["exit_code"], 0);
    }

    #[test]
    fn newer_minor_is_reported_without_action() {
        let r = report_for(
            &[
                "fabric/gitIntegration/platformProperties/2.1.0/schema.json",
                "fabric/item/report/localSettings/1.1.0/schema.json",
            ],
            None,
            &["2.157", "2.158"],
        );
        let p = row(&r, "gitIntegration/platformProperties");
        assert_eq!((p.status, p.action), (Status::NewerMinor, false));
        assert_eq!(p.latest, Some(v("2.1.0")));
        // Known-only key: information.
        let l = row(&r, "item/report/localSettings");
        assert_eq!((l.status, l.action), (Status::NewerMinor, false));
        let t = r.theme.as_ref().unwrap();
        assert_eq!((t.status, t.action), (Status::NewerMinor, false));
        assert_eq!(exit_code(&r), EXIT_OK);
        let n = newer(&r);
        assert!(n.contains(&("gitIntegration/platformProperties".into(), v("2.0.0"), v("2.1.0"))));
        assert!(n.iter().any(|(k, _, to)| k == THEME_KEY && *to == v("2.158")));
    }

    #[test]
    fn newer_major_of_a_written_schema_needs_action() {
        let r = report_for(
            &["fabric/item/report/definition/report/4.0.0/schema.json"],
            None,
            &[],
        );
        let p = row(&r, "item/report/definition/report");
        assert_eq!((p.status, p.action), (Status::NewerMajor, true));
        assert_eq!(exit_code(&r), EXIT_ACTION);
        assert!(to_text(&r, EXIT_ACTION).contains("ACTION"));
    }

    #[test]
    fn newer_major_of_a_known_only_schema_is_information() {
        let r = report_for(
            &["fabric/item/semanticModel/editorSettings/2.0.0/schema.json"],
            None,
            &[],
        );
        let p = row(&r, "item/semanticModel/editorSettings");
        assert_eq!((p.status, p.action), (Status::NewerMajor, false));
        assert_eq!(exit_code(&r), EXIT_OK);
    }

    #[test]
    fn vendored_newer_major_needs_action() {
        let paths = all_current();
        let mut refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        refs.push("fabric/item/report/definition/semanticQuery/2.0.0/schema.json");
        let (published, _) = parse_tree(&canned_tree(&refs)).unwrap();
        let vendored = vec![("semanticQuery".to_string(), vec![v("1.3.0"), v("1.4.0")])];
        let r = build_report(
            &ours(&vendored),
            &published,
            None,
            bicraft_versions::WATCHED_PREFIXES,
        );
        let p = row(&r, "item/report/definition/semanticQuery");
        assert_eq!(p.ours.vendored, Some(v("1.4.0")));
        assert_eq!((p.status, p.action), (Status::NewerMajor, true));
        assert_eq!(exit_code(&r), EXIT_ACTION);
    }

    #[test]
    fn unknown_power_bi_schema_needs_action() {
        let r = report_for(
            &["fabric/item/report/definition/newThing/1.0.0/schema.json"],
            None,
            &[],
        );
        assert_eq!(
            r.new,
            vec![("item/report/definition/newThing".to_string(), v("1.0.0"))]
        );
        assert_eq!(exit_code(&r), EXIT_ACTION);
    }

    #[test]
    fn written_schema_missing_upstream_needs_action() {
        let r = report_for(&[], Some("pagesMetadata"), &[]);
        let p = row(&r, "item/report/definition/pagesMetadata");
        assert_eq!((p.status, p.action), (Status::NotPublished, true));
        assert_eq!(exit_code(&r), EXIT_ACTION);
        // Our exact version withdrawn while a lower one stays.
        let ours = Ours {
            key: "k".into(),
            written: Some(v("1.2.0")),
            ..Ours::default()
        };
        let row = assess(&ours, Some(&[v("1.1.0")]));
        assert_eq!((row.status, row.action), (Status::NotPublished, true));
    }

    #[test]
    fn schema_diff_lists_properties_required_enums_and_refs() {
        let old = json!({
            "properties": {"a": {"type": "string"}, "b": {"$ref": "#/definitions/D"}},
            "required": ["a"],
            "definitions": {"D": {"properties": {"x": {"enum": ["One", "Two"]}}},
                            "Old": {"type": "string"}},
            "allOf": [{"$ref": "../../semanticQuery/1.3.0/schema.json#/definitions/Q"}]
        });
        let new = json!({
            "properties": {"a": {"type": "string"}, "c": {"type": "number"}, "b": {"$ref": "#/definitions/D"}},
            "required": ["a", "c"],
            "definitions": {"D": {"properties": {"x": {"enum": ["One", "Three"]}, "y": {}}}},
            "allOf": [{"$ref": "../../semanticQuery/1.4.0/schema.json#/definitions/Q"}]
        });
        let d = diff_schemas(&old, &new);
        for expected in [
            "+ property c",
            "+ property #D.y",
            "- property #Old",
            "+ required c",
            "+ enum #D.x = \"Three\"",
            "- enum #D.x = \"Two\"",
            "+ $ref ../../semanticQuery/1.4.0/schema.json",
            "- $ref ../../semanticQuery/1.3.0/schema.json",
        ] {
            assert!(d.iter().any(|l| l == expected), "missing {expected}: {d:?}");
        }
        assert!(diff_schemas(&old, &old).is_empty());
    }

    #[test]
    fn deep_schemas_do_not_overflow() {
        // Built by hand: `json!` would serialize the nested value recursively.
        let mut deep = json!({});
        for _ in 0..10_000 {
            let mut props = serde_json::Map::new();
            props.insert("p".into(), deep);
            let mut outer = serde_json::Map::new();
            outer.insert("properties".into(), Value::Object(props));
            deep = Value::Object(outer);
        }
        // Bounded walk: no stack overflow, a capped number of paths.
        let s = shape(&deep);
        assert!(s.props.len() <= MAX_DEPTH + 1);
        // serde_json's drop of a deep value is recursive; leak it.
        std::mem::forget(deep);
    }

    #[test]
    fn vendored_pbir_schemas_are_found() {
        let v = vendored_pbir(&crate::root()).unwrap();
        let page = v.iter().find(|(n, _)| n == "page").unwrap();
        assert!(page.1.contains(&Version::parse("2.1.0").unwrap()));
    }
}
