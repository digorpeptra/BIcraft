//! `file.redact` / `bicraft redact` (ROADMAP M11.37): a copy of a PBIX, PBIT or PBIP with its
//! connection strings, credentials, server and database names, file paths and URLs (and, with
//! `data`, the stored text values) replaced by stable placeholders, so a file can be shared in a
//! bug report. The idea comes from DashCraft's `redact.rs`; this is our own code.
//!
//! 1. **Collect.** The model's M (partitions and shared expressions) is scanned token by token
//!    (`bicraft_data::m::scan`): the text arguments of data source functions (any call outside
//!    the standard library's namespaces, `Sql.Database("srv", "db")`), the values of credential
//!    record fields (`[Password = "…"]`), the parameters passed to a source function, and any
//!    literal that reads as a URL, an absolute path or a connection string. The model's
//!    `connectionString`, `server`, `database`, `url`, `path`, `account` and credential
//!    properties (legacy data sources), and strings of the model, the report and the package's
//!    other text parts that read as a URL or an absolute path, are collected too.
//! 2. **Replace.** Every string of the model, the report and the other text parts has each
//!    collected value replaced by its placeholder (`server_1`, `database_1`, `path_1.csv`,
//!    `https://redacted.invalid/url_1`, `connection_1`, `credential_1`): the same value always
//!    gets the same placeholder, so the queries keep their shape. Values of fewer than 4
//!    characters are replaced only where they are the whole string. With `data`, every stored
//!    text value of the import tables becomes `value_N` (numbers and dates are kept), and the
//!    report's literals of those values too. The `SecurityBindings` part (stored credentials) is
//!    left out.
//! 3. **Write and check.** The copy is written by `file.save_as` in a private session (PBIX:
//!    the tables keep their stored data, read back instead of running their redacted queries;
//!    PBIP: a new project, without the `.pbi` cache), opened again, and every byte of it (the
//!    decompressed `DataModel` too, in UTF-8 and UTF-16LE) is searched for the collected
//!    values: if one survives, the copy is deleted and the command fails.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bicraft_model::SemanticModel;
use bicraft_pbix::structural::{DataModelEditor, StructuralData};
use bicraft_vertipaq::CellValue;
use serde_json::{Map, Value, json};

use crate::params::{allow, opt_bool, str_param};
use crate::{Session, Source};

/// Shorter values are replaced only where they are a whole string, and not searched for in the
/// written bytes (they would match by chance).
const MIN_SUBSTRING: usize = 4;
/// Most distinct values collected (secrets) or data values replaced.
const MAX_VALUES: usize = 5_000_000;
/// Most files of a written project checked.
const MAX_PROJECT_FILES: usize = 100_000;

/// Namespaces of the M standard library: their functions read no data source.
const STD_NAMESPACES: &[&str] = &[
    "Table",
    "List",
    "Text",
    "Number",
    "Date",
    "DateTime",
    "DateTimeZone",
    "Time",
    "Duration",
    "Record",
    "Value",
    "Splitter",
    "Combiner",
    "Comparer",
    "Replacer",
    "Binary",
    "BinaryFormat",
    "Json",
    "Csv",
    "Lines",
    "Character",
    "Byte",
    "Int8",
    "Int16",
    "Int32",
    "Int64",
    "Single",
    "Double",
    "Decimal",
    "Currency",
    "Percentage",
    "Logical",
    "Expression",
    "Function",
    "Type",
    "Uri",
    "Xml",
    "Excel",
    "Parquet",
    "Pdf",
    "Cube",
    "Error",
    "Diagnostics",
    "Guid",
    "Variable",
    "Precision",
    "Order",
    "JoinKind",
    "JoinAlgorithm",
    "MissingField",
    "QuoteStyle",
    "Occurrence",
    "RoundingMode",
    "ExtraValues",
    "GroupKind",
    "Compression",
    "BinaryEncoding",
    "TextEncoding",
    "Culture",
    "Embedded",
    "Html",
];

/// Record fields whose text is a credential.
const CREDENTIAL_FIELDS: &[&str] = &[
    "password",
    "pwd",
    "secret",
    "clientsecret",
    "token",
    "accesstoken",
    "apikey",
    "key",
    "authorization",
    "username",
    "user",
    "uid",
    "userid",
    "accountkey",
    "sharedaccesssignature",
    "sas",
];

/// Model properties whose strings are redacted, with the kind of their values.
const PROPERTY_KINDS: &[(&str, &str)] = &[
    ("connectionstring", "connection"),
    ("server", "server"),
    ("database", "database"),
    ("url", ""),
    ("path", ""),
    ("account", "server"),
    ("domain", "server"),
    ("username", "credential"),
    ("password", "credential"),
    ("credential", "credential"),
    ("credentials", "credential"),
];

/// The collected values and their placeholders.
#[derive(Default)]
struct Redactor {
    /// value → placeholder.
    map: HashMap<String, String>,
    /// values by length, longest first (rebuilt before replacing).
    ordered: Vec<String>,
    counters: HashMap<&'static str, usize>,
    by_kind: Vec<(&'static str, String)>,
    /// data values (whole strings only).
    data: HashMap<String, String>,
}

fn is_url(s: &str) -> bool {
    let l = s.trim().to_ascii_lowercase();
    [
        "http://", "https://", "ftp://", "sftp://", "abfss://", "wasbs://", "adl://", "jdbc:", "file://",
    ]
    .iter()
    .any(|p| l.starts_with(p))
        && !l.starts_with("https://developer.microsoft.com/json-schemas/")
        && !l.starts_with("http://www.w3.org/")
        && !l.starts_with("http://schemas.")
}

/// The URLs written inside a longer text (a text box's sentence, a tooltip): from a scheme to
/// the next space, quote or bracket.
fn embedded_urls(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    for scheme in ["http://", "https://", "ftp://", "sftp://", "file://"] {
        let mut from = 0;
        while let Some(at) = s.get(from..).and_then(|rest| rest.find(scheme)) {
            let start = from + at;
            let tail = s.get(start..).unwrap_or_default();
            let end = tail
                .find(|c: char| {
                    c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | ')' | ']' | '}' | ',')
                })
                .unwrap_or(tail.len());
            let url = tail
                .get(..end)
                .unwrap_or_default()
                .trim_end_matches(['.', ';', ':']);
            if url.len() > scheme.len() && is_url(url) && out.len() < 64 {
                out.push(url);
            }
            from = start + scheme.len();
        }
    }
    out
}

fn is_path(s: &str) -> bool {
    let b = s.as_bytes();
    let drive = matches!(b, [d, b':', b'\\' | b'/', ..] if d.is_ascii_alphabetic());
    let unix = s
        .strip_prefix('/')
        .is_some_and(|rest| rest.contains('/') && !s.contains(' ') && !s.contains("//"));
    drive || s.starts_with("\\\\") || unix
}

fn is_connection_string(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    s.contains('=')
        && s.contains(';')
        && [
            "server=",
            "data source=",
            "database=",
            "initial catalog=",
            "uid=",
            "user id=",
            "pwd=",
            "password=",
            "dsn=",
            "provider=",
            "host=",
        ]
        .iter()
        .any(|k| l.contains(k))
}

impl Redactor {
    fn add(&mut self, value: &str, kind: &'static str) {
        let v = value.trim();
        if v.len() < 2 || self.map.contains_key(v) || self.map.len() >= MAX_VALUES || self.is_placeholder(v) {
            return;
        }
        let kind = if kind.is_empty() { classify(v, None) } else { kind };
        let n = self.counters.entry(kind).or_insert(0);
        *n += 1;
        let ph = match kind {
            "server" => format!("server_{n}.redacted.invalid"),
            "database" => format!("database_{n}"),
            "url" => format!("https://redacted.invalid/url_{n}"),
            "path" => {
                let ext = v
                    .rsplit_once('.')
                    .map(|(_, e)| e)
                    .filter(|e| (1..=5).contains(&e.len()) && e.chars().all(|c| c.is_ascii_alphanumeric()));
                match ext {
                    Some(e) => format!("C:\\redacted\\path_{n}.{e}"),
                    None => format!("C:\\redacted\\path_{n}"),
                }
            }
            "connection" => format!("connection_{n}"),
            "credential" => format!("credential_{n}"),
            _ => format!("text_{n}"),
        };
        self.by_kind.push((kind, ph.clone()));
        self.map.insert(v.to_string(), ph);
    }

    fn is_placeholder(&self, v: &str) -> bool {
        v.contains("redacted.invalid") || v.starts_with("C:\\redacted\\")
    }

    fn data_value(&mut self, v: &str) -> String {
        if v.is_empty() {
            return String::new();
        }
        if let Some(p) = self.data.get(v) {
            return p.clone();
        }
        let p = format!("value_{}", self.data.len() + 1);
        if self.data.len() < MAX_VALUES {
            self.data.insert(v.to_string(), p.clone());
        }
        p
    }

    fn freeze(&mut self) {
        let mut v: Vec<String> = self.map.keys().cloned().collect();
        v.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        self.ordered = v;
    }

    /// `s` with every collected value replaced.
    fn text(&self, s: &str) -> String {
        if let Some(p) = self.map.get(s.trim()).filter(|_| s.trim().len() == s.len()) {
            return p.clone();
        }
        if let Some(p) = self.data.get(s) {
            return p.clone();
        }
        // a DAX/report literal of a data value: 'value'
        if let Some(inner) = s.strip_prefix('\'').and_then(|x| x.strip_suffix('\''))
            && let Some(p) = self.data.get(&inner.replace("''", "'"))
        {
            return format!("'{p}'");
        }
        let mut out = s.to_string();
        for k in &self.ordered {
            if k.len() < MIN_SUBSTRING || !out.contains(k.as_str()) {
                continue;
            }
            if let Some(p) = self.map.get(k) {
                out = out.replace(k.as_str(), p);
                // the M spelling of a value holding quotes
                if k.contains('"') {
                    out = out.replace(&k.replace('"', "\"\""), &p.replace('"', "\"\""));
                }
            }
        }
        out
    }

    fn value(&self, v: &mut Value) {
        match v {
            Value::String(s) => *s = self.text(s),
            Value::Array(a) => a.iter_mut().for_each(|x| self.value(x)),
            Value::Object(o) => {
                for (k, x) in o.iter_mut() {
                    if k == "$schema" {
                        continue;
                    }
                    self.value(x);
                }
            }
            _ => {}
        }
    }

    /// Collects the URLs and absolute paths among the strings of `v`.
    fn collect_strings(&mut self, v: &Value, depth: usize) {
        if depth > 200 {
            return;
        }
        match v {
            Value::String(s) if is_url(s) || is_path(s) => self.add(s, ""),
            Value::String(s) => {
                for u in embedded_urls(s) {
                    self.add(u, "url");
                }
            }
            Value::Array(a) => a.iter().for_each(|x| self.collect_strings(x, depth + 1)),
            Value::Object(o) => {
                for (k, x) in o {
                    if k != "$schema" {
                        self.collect_strings(x, depth + 1);
                    }
                }
            }
            _ => {}
        }
    }

    /// Collects the strings under the model's connection and credential properties.
    fn collect_properties(&mut self, v: &Value, depth: usize) {
        if depth > 200 {
            return;
        }
        match v {
            Value::Array(a) => a.iter().for_each(|x| self.collect_properties(x, depth + 1)),
            Value::Object(o) => {
                for (k, x) in o {
                    let lk = k.to_ascii_lowercase();
                    match PROPERTY_KINDS.iter().find(|(p, _)| *p == lk) {
                        Some((_, kind)) => self.collect_all(x, kind, depth + 1),
                        None => self.collect_properties(x, depth + 1),
                    }
                }
            }
            _ => {}
        }
    }

    fn collect_all(&mut self, v: &Value, kind: &'static str, depth: usize) {
        if depth > 200 {
            return;
        }
        match v {
            Value::String(s) => {
                let k = if kind.is_empty() || is_connection_string(s) {
                    classify(s, None)
                } else {
                    kind
                };
                self.add(s, k);
            }
            Value::Array(a) => a.iter().for_each(|x| self.collect_all(x, kind, depth + 1)),
            Value::Object(o) => o.values().for_each(|x| self.collect_all(x, kind, depth + 1)),
            _ => {}
        }
    }

    /// Collects the values of one M expression; `params`: the model's shared expressions by
    /// lower-case name.
    fn collect_m(&mut self, expr: &str, params: &HashMap<String, String>) {
        let limits = bicraft_data::Limits::default();
        let Ok((lits, idents)) = bicraft_data::m::scan::text_literals(expr, &limits) else {
            // not M we can scan: the content heuristics only
            return;
        };
        for l in lits {
            if let Some(f) = &l.field
                && CREDENTIAL_FIELDS.contains(&f.to_ascii_lowercase().as_str())
            {
                self.add(&l.value, "credential");
            } else if let Some((call, pos)) = l.call.as_ref().filter(|(c, _)| is_source(c)) {
                self.add(&l.value, classify(&l.value, Some((call, *pos))));
            } else if is_url(&l.value) || is_path(&l.value) || is_connection_string(&l.value) {
                self.add(&l.value, "");
            }
        }
        for i in idents.iter().filter(|i| is_source(&i.call)) {
            let Some(p) = params.get(&i.name.to_lowercase()) else {
                continue;
            };
            // the parameter's value: its first text literal
            if let Ok((plits, _)) = bicraft_data::m::scan::text_literals(p, &limits)
                && let Some(first) = plits.first()
            {
                self.add(&first.value, classify(&first.value, Some((&i.call, i.position))));
            }
        }
    }

    fn collect_model(&mut self, m: &SemanticModel) {
        let params: HashMap<String, String> = m
            .expressions
            .iter()
            .map(|e| (e.name.to_lowercase(), e.expression.clone()))
            .collect();
        for e in &m.expressions {
            self.collect_m(&e.expression, &params);
        }
        for t in &m.tables {
            for p in &t.partitions {
                if let Some(e) = &p.expression
                    && crate::mquery::is_m(&p.source_type)
                {
                    self.collect_m(e, &params);
                }
            }
        }
        if let Ok(v) = serde_json::to_value(m) {
            self.collect_properties(&v, 0);
            self.collect_strings(&v, 0);
        }
    }
}

/// A function outside the standard library (a data source).
fn is_source(call: &str) -> bool {
    match call.split_once('.') {
        Some((ns, _)) => !STD_NAMESPACES.iter().any(|s| s.eq_ignore_ascii_case(ns)),
        None => false,
    }
}

/// The kind of a value, by its content, else by its place among the source function's
/// arguments (the first: a server, a URL or a path; the second: a database).
fn classify(v: &str, call: Option<(&str, usize)>) -> &'static str {
    if is_connection_string(v) {
        "connection"
    } else if is_url(v) {
        "url"
    } else if is_path(v) {
        "path"
    } else {
        match call {
            Some((c, _))
                if c.to_ascii_lowercase().starts_with("odbc.")
                    || c.to_ascii_lowercase().starts_with("oledb.") =>
            {
                "connection"
            }
            Some((_, 0)) => "server",
            Some((_, 1)) => "database",
            _ => "text",
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The command
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Package,
    Project,
}

fn ext(p: &Path) -> Option<String> {
    p.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
}

fn redacted<T: serde::Serialize + serde::de::DeserializeOwned>(r: &Redactor, x: &T) -> Result<T, String> {
    let mut v = serde_json::to_value(x).map_err(|e| e.to_string())?;
    r.value(&mut v);
    serde_json::from_value(v).map_err(|e| format!("internal: the redacted copy does not read back: {e}"))
}

pub(crate) fn file_redact(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["path", "out", "data", "overwrite"])?;
    let path = str_param(p, "path")?.to_string();
    let out = str_param(p, "out")?.to_string();
    let data = opt_bool(p, "data")?.unwrap_or(false);
    let overwrite = opt_bool(p, "overwrite")?.unwrap_or(false);
    if path.trim().is_empty() || out.trim().is_empty() {
        return Err("`path` and `out` must not be empty".into());
    }
    let (pin, pout) = (PathBuf::from(&path), PathBuf::from(&out));
    let kind = match ext(&pin).as_deref() {
        Some(e @ ("pbix" | "pbit")) => {
            if ext(&pout).as_deref() != Some(e) {
                return Err(format!("`out` must be a .{e} file, as `path`"));
            }
            Kind::Package
        }
        Some("pbip") => Kind::Project,
        _ if pin.is_dir() => Kind::Project,
        _ => return Err("`path` must be a .pbix, .pbit or .pbip (or a PBIP project folder)".into()),
    };
    if kind == Kind::Project && ext(&pout).as_deref() != Some("pbip") {
        return Err("`out` must be a .pbip path (the new project's name and folder)".into());
    }
    if pin == pout {
        return Err("`out` must differ from `path`".into());
    }

    let mut private = s.private();
    private.execute("file.open", &json!({"path": path}))?;
    let model = private.model.clone().ok_or_else(|| {
        private
            .model_error
            .clone()
            .unwrap_or_else(|| "the file has no model".into())
    })?;
    let report = private.report.clone();

    // 1. collect
    let mut r = Redactor::default();
    r.collect_model(&model);
    if let Some(rep) = &report
        && let Ok(v) = serde_json::to_value(rep)
    {
        r.collect_strings(&v, 0);
    }
    let mut removed = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    if let Some(Source::Package { package, .. }) = private.source.as_ref() {
        for part in &package.parts {
            if is_data_model(&part.name) || part.is_deferred() {
                continue;
            }
            if let Some(v) = json_part(&part.name, &part.data) {
                r.collect_properties(&v, 0);
                r.collect_strings(&v, 0);
            }
        }
    }
    r.freeze();

    // 2. replace
    let model2 = redacted(&r, &model)?;
    let report2 = match &report {
        Some(rep) => Some(redacted(&r, rep)?),
        None => None,
    };
    let mut data_values = 0usize;
    match kind {
        Kind::Package => {
            let Some(Source::Package { package, .. }) = private.source.as_mut() else {
                return Err("internal: the package went away".into());
            };
            // the other parts first (the report parts are written again below)
            let mut parts = Vec::new();
            for part in std::mem::take(&mut package.parts) {
                if part.name.eq_ignore_ascii_case(bicraft_pbix::SECURITY_BINDINGS) {
                    removed.push(part.name.clone());
                    continue;
                }
                if is_data_model(&part.name) || part.is_deferred() {
                    parts.push(part);
                    continue;
                }
                let mut part = part;
                if let Some(bytes) = redact_text_part(&r, &part.name, &part.data) {
                    part.data = bytes;
                }
                parts.push(part);
            }
            package.parts = parts;
            // the model
            let is_pbix = package.get(bicraft_pbix::DATA_MODEL).is_some()
                && package.get(bicraft_pbix::DATA_MODEL_SCHEMA).is_none();
            if is_pbix {
                let (name, bytes, n) = write_data_model(package, &model2, data, &mut r)?;
                data_values = n;
                package.put(&name, bytes, true);
            } else {
                *package = bicraft_pbix::write_model(package, &model2).map_err(|e| e.to_string())?;
                if data {
                    notes.push("a PBIT holds no data: `data` changes nothing".into());
                }
            }
            private.model = Some(model2);
            private.dirty = false;
        }
        Kind::Project => {
            private.source = None;
            private.cache_abf = None;
            private.model = Some(model2);
            private.dirty = true;
            private.diagram_layout = private.diagram_layout.as_deref().map(|d| r.text(d));
            notes.push(
                "the copy is a new project without its .pbi folder: no cached data, no local settings".into(),
            );
            if data {
                notes.push("a project copy holds no data: `data` changes nothing".into());
            }
        }
    }
    // the report's data literals once the data values are known
    let report2 = match (report2, data) {
        (Some(rep), true) => Some(redacted(&r, &rep)?),
        (rep, _) => rep,
    };
    let has_report = report2.is_some();
    private.report = report2;
    private.report_dirty = has_report;

    // 3. write, reopen, check
    let saved = private.execute("file.save_as", &json!({"path": out, "overwrite": overwrite}))?;
    let saved_path = saved
        .get("saved")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .or_else(|| {
            private
                .source
                .as_ref()
                .map(|s| crate::commands::source_path(s).to_path_buf())
        })
        .ok_or("internal: the copy's path is unknown")?;
    let written = written_files(kind, &saved_path, &model_name_of(&saved_path));
    let check = (|| -> Result<u64, String> {
        let mut again = s.private();
        again
            .execute("file.open", &json!({"path": out}))
            .map_err(|e| format!("the redacted copy does not open again: {e}"))?;
        verify(&r, &written)
    })();
    let scanned = match check {
        Ok(n) => n,
        Err(e) => {
            for f in &written {
                let _ = if f.is_dir() {
                    std::fs::remove_dir_all(f)
                } else {
                    std::fs::remove_file(f)
                };
            }
            return Err(format!("{e}; the copy was deleted"));
        }
    };
    let mut kinds: Map<String, Value> = Map::new();
    for (k, _) in &r.by_kind {
        let n = kinds.get(*k).and_then(Value::as_u64).unwrap_or(0);
        kinds.insert((*k).to_string(), json!(n + 1));
    }
    Ok(json!({
        "from": path,
        "out": saved_path.display().to_string(),
        "redacted": r.by_kind.len(),
        "kinds": kinds,
        "placeholders": r.by_kind.iter().map(|(k, p)| json!({"kind": k, "placeholder": p})).collect::<Vec<_>>(),
        "data": data,
        "data_values": data_values,
        "removed_parts": removed,
        "verified": {"bytes_scanned": scanned, "survivors": 0},
        "notes": notes,
    }))
}

fn is_data_model(name: &str) -> bool {
    name.eq_ignore_ascii_case(bicraft_pbix::DATA_MODEL)
}

fn model_name_of(p: &Path) -> String {
    p.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string()
}

/// The files and folders the save wrote: the package, or the project's `.pbip` and its two
/// folders.
fn written_files(kind: Kind, saved: &Path, name: &str) -> Vec<PathBuf> {
    match kind {
        Kind::Package => vec![saved.to_path_buf()],
        Kind::Project => {
            let dir = saved.parent().unwrap_or(Path::new("."));
            vec![
                saved.to_path_buf(),
                dir.join(format!("{name}.SemanticModel")),
                dir.join(format!("{name}.Report")),
            ]
        }
    }
}

/// A text part read as JSON (UTF-8 or UTF-16), when it is JSON.
fn json_part(name: &str, bytes: &[u8]) -> Option<Value> {
    let (text, _) = bicraft_opc::decode_text(name, bytes).ok()?;
    let t = text.trim_start_matches('\u{feff}').trim_start();
    if !(t.starts_with('{') || t.starts_with('[')) {
        return None;
    }
    serde_json::from_str(t).ok()
}

/// A text part with the values replaced (JSON through its strings, other text as is); `None`
/// for a binary part or one that does not change.
fn redact_text_part(r: &Redactor, name: &str, bytes: &[u8]) -> Option<Vec<u8>> {
    let lname = name.to_ascii_lowercase();
    let texty = [".json", ".xml", ".txt", ".m", ".tmdl", ".bim", ".pq"]
        .iter()
        .any(|e| lname.ends_with(e))
        || !lname.contains('.')
        || lname.ends_with("layout");
    if !texty {
        return None;
    }
    let (text, enc) = bicraft_opc::decode_text(name, bytes).ok()?;
    if text.chars().any(|c| c == '\0') {
        return None;
    }
    let new = match json_part(name, bytes) {
        Some(mut v) => {
            r.value(&mut v);
            let new = serde_json::to_string(&v).ok()?;
            if new == serde_json::to_string(&json_part(name, bytes)?).ok()? {
                return None;
            }
            new
        }
        None => {
            let new = r.text(&text);
            if new == text {
                return None;
            }
            new
        }
    };
    Some(bicraft_opc::encode_text(&new, enc))
}

/// The PBIX `DataModel` with the redacted model: each table whose (redacted) query changed
/// keeps its stored rows; with `data`, every import table's stored text values are replaced.
/// Returns the part's name, its bytes and the number of distinct data values replaced.
fn write_data_model(
    package: &bicraft_opc::Package,
    model: &SemanticModel,
    data: bool,
    r: &mut Redactor,
) -> Result<(String, Vec<u8>, usize), String> {
    let part = package
        .get(bicraft_pbix::DATA_MODEL)
        .ok_or("internal: no DataModel part")?;
    let name = part.name.clone();
    let bytes = part.bytes().map_err(|e| e.to_string())?;
    let editor = DataModelEditor::open(&bytes, &bicraft_pbix::DataModelLimits::default())
        .map_err(|e| e.to_string())?;
    let import: Vec<String> = model
        .tables
        .iter()
        .filter(|t| {
            !t.partitions.is_empty()
                && t.partitions.iter().all(|p| crate::mquery::is_m(&p.source_type))
                && editor.row_count(&t.name).ok().flatten().is_some()
        })
        .map(|t| t.name.clone())
        .collect();
    let refresh: Vec<String> = if data { import.clone() } else { Vec::new() };
    let changes = editor.changes_refreshing(model, &refresh);
    if changes.is_empty() {
        let out = editor
            .write(model, &StructuralData::default())
            .map_err(|e| e.to_string())?;
        return Ok((name, out, 0));
    }
    let stored: Vec<String> = changes
        .replaced_tables
        .iter()
        .filter(|t| import.iter().any(|i| i.eq_ignore_ascii_case(t)))
        .cloned()
        .collect();
    let ld = crate::pbix_struct::Load {
        stored,
        ..crate::pbix_struct::Load::files(None)
    };
    let mut g = crate::pbix_struct::gather(&changes, model, &editor, &bytes, &ld)
        .map_err(|e| format!("the redacted model cannot be written into this PBIX: {e}"))?;
    if data {
        for t in &mut g.data.tables {
            for c in &mut t.columns {
                for v in &mut c.values {
                    if let CellValue::Text(s) = v {
                        *s = r.data_value(s);
                    }
                }
            }
        }
        for c in &mut g.data.columns {
            for v in &mut c.values {
                if let CellValue::Text(s) = v {
                    *s = r.data_value(s);
                }
            }
        }
    }
    let out = editor
        .write_refreshed(&g.model, &g.data, &g.partitions, &refresh, None)
        .map_err(|e| e.to_string())?;
    Ok((name, out, r.data.len()))
}

// ---------------------------------------------------------------------------------------------
// The check of the written copy
// ---------------------------------------------------------------------------------------------

/// Searches every written byte for the collected values; returns the bytes scanned.
fn verify(r: &Redactor, written: &[PathBuf]) -> Result<u64, String> {
    let needles: Vec<(&str, Vec<u8>, Vec<u8>)> = r
        .map
        .keys()
        .filter(|k| k.len() >= MIN_SUBSTRING)
        .map(|k| {
            let utf16: Vec<u8> = k.encode_utf16().flat_map(u16::to_le_bytes).collect();
            (k.as_str(), k.as_bytes().to_vec(), utf16)
        })
        .collect();
    let finders: Vec<(&str, memchr::memmem::Finder<'_>, memchr::memmem::Finder<'_>)> = needles
        .iter()
        .map(|(k, a, b)| (*k, memchr::memmem::Finder::new(a), memchr::memmem::Finder::new(b)))
        .collect();
    let mut scanned = 0u64;
    let mut scan = |what: &str, bytes: &[u8]| -> Result<(), String> {
        scanned = scanned.saturating_add(bytes.len() as u64);
        for (k, a, b) in &finders {
            if a.find(bytes).is_some() || b.find(bytes).is_some() {
                let kind = r
                    .map
                    .get(*k)
                    .and_then(|p| r.by_kind.iter().find(|(_, x)| x == p))
                    .map_or("value", |(k, _)| *k);
                let hint = if what.contains("DataModel") {
                    " (it is in the stored data: run with data: true)"
                } else {
                    ""
                };
                return Err(format!("a redacted {kind} survives in {what}{hint}"));
            }
        }
        Ok(())
    };
    let mut files = Vec::new();
    for w in written {
        collect_files(w, &mut files);
    }
    if files.len() > MAX_PROJECT_FILES {
        return Err(format!("more than {MAX_PROJECT_FILES} files written"));
    }
    for f in &files {
        let bytes = std::fs::read(f).map_err(|e| format!("{}: {e}", f.display()))?;
        let label = f
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("a file")
            .to_string();
        scan(&label, &bytes)?;
        let is_pkg = matches!(ext(f).as_deref(), Some("pbix" | "pbit"));
        if is_pkg {
            let pkg = bicraft_opc::Package::read_bytes(&bytes).map_err(|e| e.to_string())?;
            for part in &pkg.parts {
                scan(&format!("part {}", part.name), &part.data)?;
                if is_data_model(&part.name)
                    && let Ok((_, Some(abf))) = bicraft_pbix::read_data_model_keeping(
                        &part.data,
                        &bicraft_pbix::DataModelLimits::default(),
                    )
                {
                    scan("the decompressed DataModel", &abf)?;
                }
            }
        }
    }
    Ok(scanned)
}

fn collect_files(p: &Path, out: &mut Vec<PathBuf>) {
    if out.len() > MAX_PROJECT_FILES {
        return;
    }
    if p.is_dir() {
        if let Ok(rd) = std::fs::read_dir(p) {
            for e in rd.flatten() {
                collect_files(&e.path(), out);
            }
        }
    } else if p.is_file() {
        out.push(p.to_path_buf());
    }
}
