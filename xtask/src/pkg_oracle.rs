//! `cargo xtask pkg-oracle`: black-box evidence for the packaging facts
//! (docs/formats/packaging-verification.md).
//!
//! - `observe PATH [--json]`: the structure of a PBIX/PBIT package or a PBIP project (zip header
//!   fields, entry order, compression, `[Content_Types].xml` shape, text encodings, version
//!   strings, `$schema` URIs). Only structure is printed, never content: page, visual, table and
//!   resource names are replaced by placeholders, so it is safe on local client files.
//! - `mutate BASE.pbit BASE.pbix BASE_PROJECT_DIR OUT_DIR`: writes one variant per checked fact
//!   (each changes one thing of a BIcraft-generated file) and `OUT_DIR/cases.json`
//!   (`[{id, path, fact, expect}]`), the input of `tools/packaging-oracle/run.ps1`, which opens
//!   each variant in Power BI Desktop and records whether Desktop accepts it.

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

const USAGE: &str = "usage: cargo xtask pkg-oracle observe PATH [--json]\n       \
                     cargo xtask pkg-oracle mutate BASE.pbit BASE.pbix BASE_PROJECT_DIR OUT_DIR";

pub fn cmd(args: &[String]) -> Result<(), String> {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    match a.as_slice() {
        ["observe", path] => observe_print(Path::new(path), false),
        ["observe", path, "--json"] => observe_print(Path::new(path), true),
        ["mutate", pbit, pbix, project, out] => mutate(
            Path::new(pbit),
            Path::new(pbix),
            Path::new(project),
            Path::new(out),
        ),
        _ => Err(USAGE.into()),
    }
}

fn observe_print(path: &Path, as_json: bool) -> Result<(), String> {
    let v = observe(path)?;
    if as_json {
        println!("{}", serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?);
    } else {
        print_tree(&v, 0);
    }
    Ok(())
}

fn print_tree(v: &Value, depth: usize) {
    let pad = "  ".repeat(depth);
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                if x.is_object() || x.as_array().is_some_and(|a| a.iter().any(|e| !e.is_string())) {
                    println!("{pad}{k}:");
                    print_tree(x, depth + 1);
                } else {
                    println!("{pad}{k}: {}", compact(x));
                }
            }
        }
        Value::Array(a) => {
            for x in a {
                println!("{pad}- {}", compact(x));
            }
        }
        other => println!("{pad}{}", compact(other)),
    }
}

fn compact(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Observes a package (any file that is a zip) or a project (a `.pbip` file, or a folder).
pub fn observe(path: &Path) -> Result<Value, String> {
    if path.is_dir() {
        return Ok(observe_project(path));
    }
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pbip")) {
        let dir = path.parent().unwrap_or(Path::new("."));
        return Ok(observe_project(dir));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    observe_package(&bytes)
}

// ---------------------------------------------------------------------------------------------
// Packages

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    let s = b.get(i..i.checked_add(2)?)?;
    Some(u16::from_le_bytes([*s.first()?, *s.get(1)?]))
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    let s = b.get(i..i.checked_add(4)?)?;
    Some(u32::from_le_bytes([
        *s.first()?,
        *s.get(1)?,
        *s.get(2)?,
        *s.get(3)?,
    ]))
}

/// One central-directory record, as the zip file holds it.
#[derive(Debug, Clone)]
pub struct RawEntry {
    pub name: String,
    pub made_by: u16,
    pub needed: u16,
    pub flags: u16,
    pub method: u16,
    pub extra_central: u16,
    pub extra_local: u16,
    pub dos_date: u16,
}

/// Central directory records in directory order (no Zip64 support needed for observation: an
/// archive that needs it reports `zip64: true` and its records as far as they parse).
pub fn raw_entries(b: &[u8]) -> Result<(Vec<RawEntry>, bool, usize), String> {
    let start = b.len().saturating_sub(65_557);
    let eocd = (start..b.len().saturating_sub(21))
        .rev()
        .find(|&i| u32_at(b, i) == Some(0x0605_4b50))
        .ok_or("no end of central directory record")?;
    let count = usize::from(u16_at(b, eocd + 10).ok_or("short EOCD")?);
    let cd = u32_at(b, eocd + 16).ok_or("short EOCD")?;
    let comment = usize::from(u16_at(b, eocd + 20).ok_or("short EOCD")?);
    let zip64 = cd == u32::MAX
        || count == 0xFFFF
        || eocd
            .checked_sub(20)
            .is_some_and(|l| u32_at(b, l) == Some(0x0706_4b50));
    let mut out = Vec::new();
    let mut i = usize::try_from(cd).map_err(|e| e.to_string())?;
    for _ in 0..count {
        if u32_at(b, i) != Some(0x0201_4b50) {
            break;
        }
        let field = |o: usize| u16_at(b, i + o).ok_or("short central header");
        let name_len = usize::from(field(28)?);
        let extra = field(30)?;
        let comment_len = usize::from(field(32)?);
        let local = u32_at(b, i + 42).ok_or("short central header")?;
        let name_bytes = b.get(i + 46..i + 46 + name_len).ok_or("short central header")?;
        let extra_local = usize::try_from(local)
            .ok()
            .and_then(|l| u16_at(b, l + 28))
            .unwrap_or(0);
        out.push(RawEntry {
            name: String::from_utf8_lossy(name_bytes).into_owned(),
            made_by: field(4)?,
            needed: field(6)?,
            flags: field(8)?,
            method: field(10)?,
            extra_central: extra,
            extra_local,
            dos_date: field(14)?,
        });
        i = i + 46 + name_len + usize::from(extra) + comment_len;
    }
    Ok((out, zip64, comment))
}

/// Reads every entry's data (decompressed) with the `zip` crate.
pub fn read_parts(b: &[u8]) -> Result<Vec<(String, Vec<u8>, bool)>, String> {
    let mut z = zip::ZipArchive::new(Cursor::new(b)).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for i in 0..z.len() {
        let mut f = z.by_index(i).map_err(|e| e.to_string())?;
        let stored = f.compression() == zip::CompressionMethod::Stored;
        let mut data = Vec::new();
        f.read_to_end(&mut data).map_err(|e| e.to_string())?;
        out.push((f.name().map_err(|e| e.to_string())?.into_owned(), data, stored));
    }
    Ok(out)
}

/// Replaces user-chosen names (pages, visuals, bookmarks, resources, tables…) by placeholders.
pub fn normalize(name: &str) -> String {
    let segs: Vec<&str> = name.split('/').collect();
    let mut out: Vec<String> = Vec::with_capacity(segs.len());
    let last = segs.len().saturating_sub(1);
    for (i, s) in segs.iter().enumerate() {
        let prev = if i > 0 { segs.get(i - 1).copied() } else { None };
        let ext = s.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
        let placeholder =
            match prev {
                Some("pages") if i != last => Some("<page>".to_string()),
                Some("visuals") if i != last => Some("<visual>".to_string()),
                Some("bookmarks") if s.ends_with(".bookmark.json") => Some("<bookmark>.bookmark.json".into()),
                Some(
                    "RegisteredResources" | "BaseThemes" | "tables" | "cultures" | "roles" | "perspectives",
                ) if i == last => Some(format!("<item>.{ext}")),
                Some("CustomVisuals") => Some("<guid>".into()),
                Some("resources") if i == last => Some(format!("<file>.{ext}")),
                Some("DAXQueries" | "TMDLScripts" | "definitions") if i == last && !s.starts_with('.') => {
                    Some(format!("<item>.{ext}"))
                }
                _ => None,
            };
        // Item folders of a project (`<Name>.Report`, `<Name>.SemanticModel`, `<Name>.pbip`).
        let item = (i == 0)
            .then(|| {
                ["Report", "SemanticModel", "Dataset", "pbip"]
                    .iter()
                    .find(|k| s.ends_with(&format!(".{k}")))
                    .map(|k| format!("<Name>.{k}"))
            })
            .flatten();
        out.push(item.or(placeholder).unwrap_or_else(|| (*s).to_string()));
    }
    out.join("/")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Enc {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16LeBom,
    Binary,
    Empty,
}

impl Enc {
    fn name(self) -> &'static str {
        match self {
            Enc::Utf8 => "utf-8",
            Enc::Utf8Bom => "utf-8+bom",
            Enc::Utf16Le => "utf-16le",
            Enc::Utf16LeBom => "utf-16le+bom",
            Enc::Binary => "binary",
            Enc::Empty => "empty",
        }
    }
}

fn detect(b: &[u8]) -> (Enc, Option<String>) {
    if b.is_empty() {
        return (Enc::Empty, Some(String::new()));
    }
    if let Some(rest) = b.strip_prefix(b"\xEF\xBB\xBF") {
        return match std::str::from_utf8(rest) {
            Ok(t) => (Enc::Utf8Bom, Some(t.to_string())),
            Err(_) => (Enc::Binary, None),
        };
    }
    let utf16 = |body: &[u8]| -> Option<String> {
        if !body.len().is_multiple_of(2) {
            return None;
        }
        let units: Vec<u16> = body
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        String::from_utf16(&units).ok()
    };
    if let Some(rest) = b.strip_prefix(b"\xFF\xFE") {
        return match utf16(rest) {
            Some(t) => (Enc::Utf16LeBom, Some(t)),
            None => (Enc::Binary, None),
        };
    }
    // UTF-16LE of mostly-ASCII text: NUL in odd positions.
    let odd_nul = b.iter().skip(1).step_by(2).take(64).filter(|&&x| x == 0).count();
    let pairs = b.len().div_ceil(2).min(64);
    if b.len() >= 2 && odd_nul * 2 > pairs {
        return match utf16(b) {
            Some(t) => (Enc::Utf16Le, Some(t)),
            None => (Enc::Binary, None),
        };
    }
    match std::str::from_utf8(b) {
        Ok(t) if !t.contains('\0') => (Enc::Utf8, Some(t.to_string())),
        _ => (Enc::Binary, None),
    }
}

fn line_endings(t: &str) -> &'static str {
    let crlf = t.matches("\r\n").count();
    let lf = t.matches('\n').count();
    match (crlf, lf) {
        (0, 0) => "none",
        (c, l) if c == l => "crlf",
        (0, _) => "lf",
        _ => "mixed",
    }
}

fn json_keys(v: &Value) -> Value {
    match v {
        Value::Object(m) => Value::Array(m.keys().map(|k| Value::String(k.clone())).collect()),
        _ => Value::Null,
    }
}

/// Structure of a JSON text: top-level keys, `$schema`, `version`/`Version`, indentation.
fn json_shape(text: &str) -> Value {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return json!({"json": false});
    };
    let indent = text
        .lines()
        .nth(1)
        .map(|l| l.len() - l.trim_start().len())
        .unwrap_or(0);
    let mut m = Map::new();
    m.insert("keys".into(), json_keys(&v));
    for k in ["$schema", "version", "Version"] {
        if let Some(x) = v.get(k).filter(|x| !x.is_object() && !x.is_array()) {
            m.insert(k.into(), x.clone());
        }
    }
    m.insert("indent".into(), json!(indent));
    m.insert("multiline".into(), json!(text.contains('\n')));
    Value::Object(m)
}

pub fn observe_package(bytes: &[u8]) -> Result<Value, String> {
    let (raw, zip64, comment) = raw_entries(bytes)?;
    let parts = read_parts(bytes)?;
    let mut entries = Vec::new();
    let mut counts: BTreeMap<String, (usize, Value)> = BTreeMap::new();
    let mut order = Vec::new();
    for (r, (name, data, _)) in raw.iter().zip(&parts) {
        let norm = normalize(&r.name);
        let (enc, _) = detect(data);
        let desc = json!({
            "method": match r.method { 0 => "stored", 8 => "deflated", _ => "other" },
            "made_by": format!("{:#06x}", r.made_by),
            "needed": r.needed,
            "flags": r.flags,
            "extra": [r.extra_local, r.extra_central],
            "encoding": if name.ends_with(".png") || name == "DataModel" || name == "SecurityBindings" { "binary" } else { enc.name() },
            "dos_date_1980": r.dos_date == 0x21,
        });
        if !order.contains(&norm) {
            order.push(norm.clone());
        }
        let e = counts.entry(norm).or_insert((0, desc));
        e.0 += 1;
    }
    for n in &order {
        if let Some((c, d)) = counts.get(n) {
            let mut d = d.clone();
            if let Some(m) = d.as_object_mut() {
                m.insert("name".into(), json!(n));
                if *c > 1 {
                    m.insert("count".into(), json!(c));
                }
            }
            entries.push(d);
        }
    }
    let get = |n: &str| {
        parts
            .iter()
            .find(|(p, _, _)| p.eq_ignore_ascii_case(n))
            .map(|(_, d, _)| d)
    };
    let mut out = Map::new();
    out.insert("zip64".into(), json!(zip64));
    out.insert("archive_comment".into(), json!(comment));
    out.insert("entries".into(), Value::Array(entries));
    if let Some(ct) = get("[Content_Types].xml") {
        out.insert("content_types".into(), content_types_shape(ct));
    }
    if let Some(v) = get("Version") {
        let (enc, t) = detect(v);
        out.insert("Version".into(), json!({"encoding": enc.name(), "text": t}));
    }
    for name in [
        "Settings",
        "Metadata",
        "Connections",
        "DiagramLayout",
        "UnappliedChanges",
    ] {
        if let Some(d) = get(name) {
            let (enc, t) = detect(d);
            let mut m = Map::new();
            m.insert("encoding".into(), json!(enc.name()));
            if let Some(v) = t.as_deref().and_then(|t| serde_json::from_str::<Value>(t).ok()) {
                m.insert("keys".into(), json_keys(&v));
                if let Some(x) = v.get("Version").or_else(|| v.get("version")) {
                    m.insert("version".into(), x.clone());
                }
                if let Some(q) = v.get("QueriesSettings") {
                    m.insert("QueriesSettings.keys".into(), json_keys(q));
                    m.insert(
                        "QueriesSettings.Version".into(),
                        q.get("Version").cloned().unwrap_or(Value::Null),
                    );
                }
                if let Some(r) = v.get("ReportSettings") {
                    m.insert("ReportSettings.keys".into(), json_keys(r));
                }
            }
            out.insert(name.into(), Value::Object(m));
        }
    }
    if let Some(d) = get("DataModelSchema") {
        let (enc, t) = detect(d);
        let t = t.unwrap_or_default();
        out.insert(
            "DataModelSchema".into(),
            json!({"encoding": enc.name(), "line_endings": line_endings(&t),
                   "trailing_newline": t.ends_with('\n'),
                   "compatibilityLevel": serde_json::from_str::<Value>(&t).ok().and_then(|v| v.get("compatibilityLevel").cloned())}),
        );
    }
    if let Some(d) = get("Report/Layout") {
        let (enc, _) = detect(d);
        out.insert("Report/Layout".into(), json!({"encoding": enc.name()}));
    }
    let mut pbir: BTreeMap<String, Value> = BTreeMap::new();
    for (name, data, _) in &parts {
        if let Some(rel) = name.strip_prefix("Report/definition/") {
            let (enc, t) = detect(data);
            let mut shape = json_shape(t.as_deref().unwrap_or(""));
            if let Some(m) = shape.as_object_mut() {
                m.insert("encoding".into(), json!(enc.name()));
                m.remove("keys");
            }
            pbir.entry(normalize(rel)).or_insert(shape);
        }
    }
    if !pbir.is_empty() {
        out.insert("Report/definition".into(), json!(pbir));
    }
    Ok(Value::Object(out))
}

fn attr<'a>(el: &'a str, name: &str) -> Option<&'a str> {
    let key = format!("{name}=\"");
    let i = el.find(&key)? + key.len();
    let rest = el.get(i..)?;
    rest.get(..rest.find('"')?)
}

fn content_types_shape(ct: &[u8]) -> Value {
    let (enc, t) = detect(ct);
    let t = t.unwrap_or_default();
    let mut defaults = Vec::new();
    let mut overrides = Vec::new();
    for el in t.split('<').skip(1) {
        if el.starts_with("Default ") {
            defaults.push(json!([attr(el, "Extension"), attr(el, "ContentType")]));
        } else if el.starts_with("Override ") {
            let n = attr(el, "PartName").unwrap_or_default();
            overrides.push(json!([
                normalize(n.trim_start_matches('/')),
                attr(el, "ContentType")
            ]));
        }
    }
    json!({
        "encoding": enc.name(),
        "xml_declaration": t.starts_with("<?xml version=\"1.0\" encoding=\"utf-8\"?>"),
        "single_line": !t.contains('\n'),
        "space_before_slash": t.contains("\" />"),
        "defaults": defaults,
        "overrides": overrides,
    })
}

// ---------------------------------------------------------------------------------------------
// Projects

fn walk(dir: &Path, root: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 16 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut items: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    items.sort();
    for p in items {
        if p.is_dir() {
            walk(&p, root, out, depth + 1);
        } else if let Ok(rel) = p.strip_prefix(root) {
            out.push(rel.to_path_buf());
        }
    }
}

pub fn observe_project(dir: &Path) -> Value {
    let mut files = Vec::new();
    walk(dir, dir, &mut files, 0);
    let mut seen: BTreeMap<String, Value> = BTreeMap::new();
    for rel in files {
        let rel_s = rel.to_string_lossy().replace('\\', "/");
        let norm = normalize(&rel_s);
        if seen.contains_key(&norm) {
            if let Some(c) = seen.get_mut(&norm).and_then(|v| v.as_object_mut()) {
                let n = c.get("count").and_then(Value::as_u64).unwrap_or(1) + 1;
                c.insert("count".into(), json!(n));
            }
            continue;
        }
        let Ok(bytes) = std::fs::read(dir.join(&rel)) else {
            continue;
        };
        let (enc, text) = detect(&bytes);
        let mut m = Map::new();
        m.insert("encoding".into(), json!(enc.name()));
        let binary = rel_s.ends_with(".abf") || rel_s.ends_with(".png");
        if let (false, Some(t)) = (binary, text.as_deref()) {
            m.insert("line_endings".into(), json!(line_endings(t)));
            m.insert("trailing_newline".into(), json!(t.ends_with('\n')));
            let name = rel
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default();
            let metadata = name.ends_with(".pbip")
                || name.ends_with(".pbir")
                || name.ends_with(".pbism")
                || name == ".platform"
                || name.ends_with(".json");
            if metadata {
                if let Value::Object(s) = json_shape(t) {
                    m.extend(s);
                }
                if name == ".platform"
                    && let Ok(v) = serde_json::from_str::<Value>(t)
                {
                    m.insert(
                        "metadata.type".into(),
                        v.pointer("/metadata/type").cloned().unwrap_or(Value::Null),
                    );
                    m.insert(
                        "config.version".into(),
                        v.pointer("/config/version").cloned().unwrap_or(Value::Null),
                    );
                }
                if name.ends_with(".pbir")
                    && let Ok(v) = serde_json::from_str::<Value>(t)
                {
                    m.insert(
                        "datasetReference".into(),
                        json_keys(v.get("datasetReference").unwrap_or(&Value::Null)),
                    );
                }
            }
            if name == ".gitignore" {
                m.insert("lines".into(), json!(t.lines().collect::<Vec<_>>()));
            }
        }
        seen.insert(norm, Value::Object(m));
    }
    json!({"project_files": seen})
}

// ---------------------------------------------------------------------------------------------
// Mutations

type Parts = Vec<(String, Vec<u8>, bool)>;

fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

fn text_of(b: &[u8]) -> String {
    detect(b).1.unwrap_or_default()
}

/// `[Content_Types].xml` as BIcraft's writer lays it out (one `Default` per extension, an
/// `Override` per extension-less part, `application/json` for `Settings` and `Metadata`).
fn content_types(parts: &Parts) -> Vec<u8> {
    let mut defaults: Vec<String> = Vec::new();
    let mut overrides = String::new();
    for (name, _, _) in parts {
        if name == "[Content_Types].xml" {
            continue;
        }
        let last = name.rsplit('/').next().unwrap_or(name);
        match last.rsplit_once('.') {
            Some((_, ext)) if !ext.is_empty() => {
                let e = ext.to_ascii_lowercase();
                if !defaults.contains(&e) {
                    defaults.push(e);
                }
            }
            _ => {
                let ct = if matches!(name.as_str(), "Settings" | "Metadata" | "UnappliedChanges") {
                    "application/json"
                } else {
                    ""
                };
                overrides.push_str(&format!("<Override PartName=\"/{name}\" ContentType=\"{ct}\" />"));
            }
        }
    }
    let d: String = defaults
        .iter()
        .map(|e| format!("<Default Extension=\"{e}\" ContentType=\"\" />"))
        .collect();
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">{d}{overrides}</Types>"
    );
    [&b"\xEF\xBB\xBF"[..], xml.as_bytes()].concat()
}

fn write_package(path: &Path, parts: &Parts, fix_types: bool) -> Result<(), String> {
    let mut parts = parts.clone();
    if fix_types {
        let ct = content_types(&parts);
        if let Some(p) = parts.iter_mut().find(|(n, _, _)| n == "[Content_Types].xml") {
            p.1 = ct;
        }
    }
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data, stored) in &parts {
        let method = if *stored {
            zip::CompressionMethod::Stored
        } else {
            zip::CompressionMethod::Deflated
        };
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(method)
            .large_file(data.len() >= 0xFFFF_FFFF);
        w.start_file(name.as_str(), opts).map_err(|e| e.to_string())?;
        w.write_all(data).map_err(|e| e.to_string())?;
    }
    let bytes = w.finish().map_err(|e| e.to_string())?.into_inner();
    std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}

fn set(parts: &mut Parts, name: &str, data: Vec<u8>) {
    match parts.iter_mut().find(|(n, _, _)| n == name) {
        Some(p) => p.1 = data,
        None => parts.push((name.to_string(), data, false)),
    }
}

fn remove(parts: &mut Parts, name: &str) {
    parts.retain(|(n, _, _)| n != name);
}

fn get<'a>(parts: &'a Parts, name: &str) -> Option<&'a Vec<u8>> {
    parts.iter().find(|(n, _, _)| n == name).map(|(_, d, _)| d)
}

/// Edits the JSON of a UTF-16LE part.
fn edit_json16(parts: &mut Parts, name: &str, f: impl FnOnce(&mut Value)) -> Result<(), String> {
    let text = text_of(get(parts, name).ok_or(format!("base has no {name}"))?);
    let mut v: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    f(&mut v);
    set(parts, name, utf16(&v.to_string()));
    Ok(())
}

struct Case {
    id: &'static str,
    file: String,
    fact: &'static str,
    expect: &'static str,
}

fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    let mut files = Vec::new();
    walk(from, from, &mut files, 0);
    for rel in files {
        let dst = to.join(&rel);
        if let Some(p) = dst.parent() {
            std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        std::fs::copy(from.join(&rel), &dst).map_err(|e| format!("{}: {e}", rel.display()))?;
    }
    Ok(())
}

fn find_file(dir: &Path, f: &dyn Fn(&str) -> bool) -> Option<PathBuf> {
    let mut files = Vec::new();
    walk(dir, dir, &mut files, 0);
    files
        .into_iter()
        .find(|r| f(&r.to_string_lossy().replace('\\', "/")))
        .map(|r| dir.join(r))
}

fn edit_json_file(path: &Path, f: impl FnOnce(&mut Value)) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut v: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    f(&mut v);
    std::fs::write(path, serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

fn mutate(pbit: &Path, pbix: &Path, project: &Path, out: &Path) -> Result<(), String> {
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let t = read_parts(&std::fs::read(pbit).map_err(|e| e.to_string())?)?;
    let x = read_parts(&std::fs::read(pbix).map_err(|e| e.to_string())?)?;
    let mut cases: Vec<Case> = Vec::new();
    let mut pkg = |id: &'static str,
                   base: &Parts,
                   ext: &str,
                   fact: &'static str,
                   expect: &'static str,
                   edit: &dyn Fn(&mut Parts) -> Result<(), String>|
     -> Result<(), String> {
        let mut p = base.clone();
        edit(&mut p)?;
        let file = out.join(format!("{id}.{ext}"));
        write_package(&file, &p, true)?;
        cases.push(Case {
            id,
            file: file.to_string_lossy().into_owned(),
            fact,
            expect,
        });
        Ok(())
    };
    let none = |_: &mut Parts| Ok(());
    pkg(
        "p00",
        &t,
        "pbit",
        "control: BIcraft PBIT re-zipped",
        "accept",
        &none,
    )?;
    pkg(
        "p01",
        &x,
        "pbix",
        "control: BIcraft PBIX re-zipped",
        "accept",
        &none,
    )?;
    pkg("v01", &t, "pbit", "/Version 1.36 is accepted", "accept", &|p| {
        set(p, "Version", utf16("1.36"));
        Ok(())
    })?;
    pkg(
        "v02",
        &t,
        "pbit",
        "/Version 1.37 (above the maximum) is refused",
        "refuse",
        &|p| {
            set(p, "Version", utf16("1.37"));
            Ok(())
        },
    )?;
    pkg(
        "v03",
        &t,
        "pbit",
        "/Version 1.32.0 (three components) is refused",
        "refuse",
        &|p| {
            set(p, "Version", utf16("1.32.0"));
            Ok(())
        },
    )?;
    pkg(
        "v04",
        &t,
        "pbit",
        "/Version with a leading space is refused",
        "refuse",
        &|p| {
            set(p, "Version", utf16(" 1.32"));
            Ok(())
        },
    )?;
    pkg(
        "v05",
        &t,
        "pbit",
        "/Version in UTF-8 instead of UTF-16LE",
        "refuse",
        &|p| {
            set(p, "Version", b"1.32".to_vec());
            Ok(())
        },
    )?;
    pkg(
        "v06",
        &t,
        "pbit",
        "/Version missing (the only required part)",
        "refuse",
        &|p| {
            remove(p, "Version");
            Ok(())
        },
    )?;
    pkg(
        "v07",
        &t,
        "pbit",
        "/Version 1.28 with a PBIR report (below 1.32)",
        "unknown",
        &|p| {
            set(p, "Version", utf16("1.28"));
            Ok(())
        },
    )?;
    pkg(
        "v08",
        &t,
        "pbit",
        "/Version 1.32 followed by CRLF",
        "refuse",
        &|p| {
            set(p, "Version", utf16("1.32\r\n"));
            Ok(())
        },
    )?;
    pkg(
        "k01",
        &t,
        "pbit",
        "a `Version.txt` sibling of a known part",
        "refuse",
        &|p| {
            p.push(("Version.txt".into(), utf16("1.32"), false));
            Ok(())
        },
    )?;
    pkg(
        "k02",
        &t,
        "pbit",
        "a `foo.txt` part that is not a known part's sibling",
        "accept",
        &|p| {
            p.push(("foo.txt".into(), b"x".to_vec(), false));
            Ok(())
        },
    )?;
    pkg(
        "d01",
        &x,
        "pbix",
        "PBIX with both DataModel and DataModelSchema",
        "refuse",
        &|p| {
            let schema = get(&t, "DataModelSchema").cloned().ok_or("no schema")?;
            p.push(("DataModelSchema".into(), schema, false));
            Ok(())
        },
    )?;
    pkg(
        "d02",
        &x,
        "pbix",
        "PBIX with DataModel deflated instead of stored",
        "unknown",
        &|p| {
            if let Some(e) = p.iter_mut().find(|(n, _, _)| n == "DataModel") {
                e.2 = false;
            }
            Ok(())
        },
    )?;
    pkg(
        "d03",
        &x,
        "pbix",
        "PBIX with DataModel as the first entry",
        "accept",
        &|p| {
            if let Some(i) = p.iter().position(|(n, _, _)| n == "DataModel") {
                let e = p.remove(i);
                p.insert(0, e);
            }
            Ok(())
        },
    )?;
    pkg(
        "l01",
        &t,
        "pbit",
        "Report/Layout next to Report/definition/",
        "refuse",
        &|p| {
            p.push((
                "Report/Layout".into(),
                utf16(r#"{"config":"{}","layoutOptimization":0,"sections":[]}"#),
                false,
            ));
            Ok(())
        },
    )?;
    pkg(
        "s01",
        &t,
        "pbit",
        "Settings Version 5 (above 4)",
        "refuse",
        &|p| edit_json16(p, "Settings", |v| v["Version"] = json!(5)),
    )?;
    pkg(
        "s02",
        &t,
        "pbit",
        "Settings without QueriesSettings.Version",
        "refuse",
        &|p| {
            edit_json16(p, "Settings", |v| {
                if let Some(q) = v.get_mut("QueriesSettings").and_then(Value::as_object_mut) {
                    q.remove("Version");
                }
            })
        },
    )?;
    for (id, ver, fact, expect) in [
        (
            "s03",
            "1.0.3206.0",
            "QueriesSettings.Version 1.0.3206.0 (just below the oldest)",
            "refuse",
        ),
        (
            "s04",
            "1.0.3207.0",
            "QueriesSettings.Version 1.0.3207.0 (the oldest supported)",
            "accept",
        ),
        (
            "s05",
            "99.0.0.0",
            "QueriesSettings.Version 99.0.0.0 (future)",
            "refuse",
        ),
        (
            "s06",
            "2.130",
            "QueriesSettings.Version with two components",
            "accept",
        ),
    ] {
        pkg(id, &t, "pbit", fact, expect, &|p| {
            edit_json16(p, "Settings", |v| v["QueriesSettings"]["Version"] = json!(ver))
        })?;
    }
    pkg("s07", &t, "pbit", "Settings part missing", "unknown", &|p| {
        remove(p, "Settings");
        Ok(())
    })?;
    pkg(
        "s08",
        &t,
        "pbit",
        "Settings in UTF-8 instead of UTF-16LE",
        "unknown",
        &|p| {
            let s = text_of(get(p, "Settings").ok_or("no Settings")?);
            set(p, "Settings", s.into_bytes());
            Ok(())
        },
    )?;
    pkg(
        "s09",
        &t,
        "pbit",
        "Settings without a top-level Version",
        "refuse",
        &|p| {
            edit_json16(p, "Settings", |v| {
                if let Some(m) = v.as_object_mut() {
                    m.remove("Version");
                }
            })
        },
    )?;
    pkg(
        "m01",
        &t,
        "pbit",
        "Metadata Version 6 (above 5)",
        "refuse",
        &|p| edit_json16(p, "Metadata", |v| v["Version"] = json!(6)),
    )?;
    pkg("m02", &t, "pbit", "Metadata part missing", "unknown", &|p| {
        remove(p, "Metadata");
        Ok(())
    })?;
    pkg(
        "e01",
        &t,
        "pbit",
        "DataModelSchema in UTF-8 instead of UTF-16LE",
        "unknown",
        &|p| {
            let s = text_of(get(p, "DataModelSchema").ok_or("no schema")?);
            set(p, "DataModelSchema", s.into_bytes());
            Ok(())
        },
    )?;
    pkg("e02", &t, "pbit", "DiagramLayout part missing", "accept", &|p| {
        remove(p, "DiagramLayout");
        Ok(())
    })?;
    pkg(
        "e03",
        &t,
        "pbit",
        "DiagramLayout in UTF-8 instead of UTF-16LE",
        "unknown",
        &|p| {
            let s = text_of(get(p, "DiagramLayout").ok_or("no DiagramLayout")?);
            set(p, "DiagramLayout", s.into_bytes());
            Ok(())
        },
    )?;
    pkg(
        "r01",
        &t,
        "pbit",
        "PBIR report.json with a UTF-8 BOM",
        "refuse",
        &|p| {
            let d = get(p, "Report/definition/report.json")
                .cloned()
                .ok_or("no report.json")?;
            set(
                p,
                "Report/definition/report.json",
                [&b"\xEF\xBB\xBF"[..], &d].concat(),
            );
            Ok(())
        },
    )?;
    pkg(
        "r02",
        &t,
        "pbit",
        "PBIR version.json in UTF-16LE instead of UTF-8",
        "refuse",
        &|p| {
            let d = text_of(get(p, "Report/definition/version.json").ok_or("no version.json")?);
            set(p, "Report/definition/version.json", utf16(&d));
            Ok(())
        },
    )?;
    pkg("r03", &t, "pbit", "PBIR with no page", "refuse", &|p| {
        p.retain(|(n, _, _)| !n.starts_with("Report/definition/pages/") || n.ends_with("pages.json"));
        Ok(())
    })?;
    pkg(
        "r04",
        &t,
        "pbit",
        "PBIR documents indented (multi-line) instead of compact",
        "accept",
        &|p| {
            for e in p
                .iter_mut()
                .filter(|(n, _, _)| n.starts_with("Report/definition/") && n.ends_with(".json"))
            {
                if let Ok(v) = serde_json::from_slice::<Value>(&e.1)
                    && let Ok(s) = serde_json::to_string_pretty(&v)
                {
                    e.1 = s.into_bytes();
                }
            }
            Ok(())
        },
    )?;
    pkg(
        "c01",
        &t,
        "pbit",
        "docProps/custom.xml of 1 KiB (control)",
        "accept",
        &|p| {
            p.push(("docProps/custom.xml".into(), custom_xml(1024), false));
            Ok(())
        },
    )?;
    pkg(
        "c02",
        &t,
        "pbit",
        "docProps/custom.xml over 2 MiB",
        "refuse",
        &|p| {
            p.push((
                "docProps/custom.xml".into(),
                custom_xml(2 * 1024 * 1024 + 1),
                false,
            ));
            Ok(())
        },
    )?;
    pkg(
        "x01",
        &t,
        "pbit",
        "DataMashup that does not start with Int32 0",
        "refuse",
        &|p| {
            p.push((
                "DataMashup".into(),
                [1u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0].to_vec(),
                false,
            ));
            Ok(())
        },
    )?;
    pkg(
        "x02",
        &t,
        "pbit",
        "DataMashup and UnappliedChanges both present",
        "refuse",
        &|p| {
            p.push(("DataMashup".into(), vec![0u8; 20], false));
            p.push((
                "UnappliedChanges".into(),
                utf16(r#"{"version":"1.0","queries":[]}"#),
                false,
            ));
            Ok(())
        },
    )?;
    pkg(
        "x03",
        &t,
        "pbit",
        "control for x01/x02: DataMashup of version 0 and four empty blocks, alone",
        "unknown",
        &|p| {
            p.push(("DataMashup".into(), vec![0u8; 20], false));
            Ok(())
        },
    )?;
    pkg(
        "x04",
        &t,
        "pbit",
        "control for x02: UnappliedChanges alone",
        "unknown",
        &|p| {
            p.push((
                "UnappliedChanges".into(),
                utf16(r#"{"version":"1.0","queries":[]}"#),
                false,
            ));
            Ok(())
        },
    )?;
    pkg(
        "n01",
        &t,
        "pbit",
        "an extra part with a percent-encoded name (Report/StaticResources/RegisteredResources/a%20b.png)",
        "accept",
        &|p| {
            p.push((
                "Report/StaticResources/RegisteredResources/a%20b.png".into(),
                tiny_png(),
                false,
            ));
            Ok(())
        },
    )?;
    pkg(
        "z01",
        &t,
        "pbit",
        "every part stored (no deflate)",
        "accept",
        &|p| {
            for e in p.iter_mut() {
                e.2 = true;
            }
            Ok(())
        },
    )?;

    // Projects.
    let mut proj = |id: &'static str,
                    fact: &'static str,
                    expect: &'static str,
                    edit: &dyn Fn(&Path) -> Result<(), String>|
     -> Result<(), String> {
        let dir = out.join(id);
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
        }
        copy_dir(project, &dir)?;
        edit(&dir)?;
        let pbip = find_file(&dir, &|r| r.ends_with(".pbip") && !r.contains('/')).ok_or("no .pbip")?;
        cases.push(Case {
            id,
            file: pbip.to_string_lossy().into_owned(),
            fact,
            expect,
        });
        Ok(())
    };
    let pbir_file = |d: &Path| {
        find_file(d, &|r| r.ends_with(".Report/definition.pbir")).ok_or("no definition.pbir".to_string())
    };
    let pbism_file = |d: &Path| {
        find_file(d, &|r| r.ends_with(".SemanticModel/definition.pbism"))
            .ok_or("no definition.pbism".to_string())
    };
    let set_version = |f: PathBuf, v: &str| edit_json_file(&f, |j| j["version"] = json!(v));
    proj("j00", "control: BIcraft project", "accept", &|_| Ok(()))?;
    proj(
        "j01",
        "definition.pbir version 3.0 with a PBIR definition/ folder",
        "unknown",
        &|d| set_version(pbir_file(d)?, "3.0"),
    )?;
    proj(
        "j02",
        "definition.pbir version 3.0 and no .platform",
        "unknown",
        &|d| {
            set_version(pbir_file(d)?, "3.0")?;
            let r = pbir_file(d)?;
            let plat = r.with_file_name(".platform");
            std::fs::remove_file(plat).map_err(|e| e.to_string())
        },
    )?;
    proj(
        "j03",
        "definition.pbir version 4.0 and no .platform",
        "accept",
        &|d| {
            let plat = pbir_file(d)?.with_file_name(".platform");
            std::fs::remove_file(plat).map_err(|e| e.to_string())
        },
    )?;
    proj(
        "j04",
        "definition.pbir version 1.0 with a PBIR definition/ folder",
        "refuse",
        &|d| set_version(pbir_file(d)?, "1.0"),
    )?;
    proj(
        "j05",
        "definition.pbism version 3.0 with a TMDL definition/ folder",
        "unknown",
        &|d| set_version(pbism_file(d)?, "3.0"),
    )?;
    proj(
        "j06",
        "definition.pbism version 1.0 with a TMDL definition/ folder",
        "refuse",
        &|d| set_version(pbism_file(d)?, "1.0"),
    )?;
    proj(
        "j07",
        "model.bim next to a TMDL definition/ folder",
        "refuse",
        &|d| {
            let f = pbism_file(d)?.with_file_name("model.bim");
            std::fs::write(f, "{\"name\":\"x\",\"compatibilityLevel\":1606,\"model\":{}}")
                .map_err(|e| e.to_string())
        },
    )?;
    proj(
        "j08",
        "report.json next to a PBIR definition/ folder",
        "refuse",
        &|d| {
            let f = pbir_file(d)?.with_file_name("report.json");
            std::fs::write(f, r#"{"config":"{}","layoutOptimization":0,"sections":[]}"#)
                .map_err(|e| e.to_string())
        },
    )?;
    proj("j09", "a PBIR page.json with a UTF-8 BOM", "refuse", &|d| {
        let f = find_file(d, &|r| r.ends_with("/page.json")).ok_or("no page.json")?;
        let b = std::fs::read(&f).map_err(|e| e.to_string())?;
        std::fs::write(f, [&b"\xEF\xBB\xBF"[..], &b].concat()).map_err(|e| e.to_string())
    })?;
    proj("j10", "a TMDL file with a UTF-8 BOM", "unknown", &|d| {
        let f = find_file(d, &|r| r.ends_with("/model.tmdl")).ok_or("no model.tmdl")?;
        let b = std::fs::read(&f).map_err(|e| e.to_string())?;
        std::fs::write(f, [&b"\xEF\xBB\xBF"[..], &b].concat()).map_err(|e| e.to_string())
    })?;
    let set_schema = |f: PathBuf, from: &'static str, to: &'static str| {
        edit_json_file(&f, |j| {
            if let Some(s) = j.get("$schema").and_then(Value::as_str) {
                j["$schema"] = json!(s.replace(from, to));
            }
        })
    };
    proj(
        "j11",
        "definition.pbir $schema with an unknown major (9.0.0)",
        "refuse",
        &|d| set_schema(pbir_file(d)?, "/2.0.0/", "/9.0.0/"),
    )?;
    proj(
        "j12",
        "definition.pbir $schema with a newer minor (2.9.0)",
        "accept",
        &|d| set_schema(pbir_file(d)?, "/2.0.0/", "/2.9.0/"),
    )?;
    proj(
        "j13",
        "page.json $schema with an unknown major (9.0.0)",
        "refuse",
        &|d| {
            let f = find_file(d, &|r| r.ends_with("/page.json")).ok_or("no page.json")?;
            edit_json_file(&f, |j| {
                if let Some(s) = j.get("$schema").and_then(Value::as_str) {
                    let re: Vec<&str> = s.split('/').collect();
                    let n = re.len();
                    let mut v: Vec<String> = re.iter().map(|x| (*x).to_string()).collect();
                    if let Some(x) = n.checked_sub(2).and_then(|i| v.get_mut(i)) {
                        *x = "9.0.0".into();
                    }
                    j["$schema"] = json!(v.join("/"));
                }
            })
        },
    )?;
    proj(
        "j14",
        "definition.pbir byPath with an absolute path",
        "refuse",
        &|d| {
            let f = pbir_file(d)?;
            let model = find_file(d, &|r| r.ends_with(".SemanticModel/definition.pbism"))
                .and_then(|p| p.parent().map(Path::to_path_buf))
                .ok_or("no model folder")?;
            let abs = model.to_string_lossy().replace('\\', "/");
            edit_json_file(&f, |j| j["datasetReference"]["byPath"]["path"] = json!(abs))
        },
    )?;
    proj(
        "j15",
        "definition.pbir with both byPath and byConnection",
        "refuse",
        &|d| {
            edit_json_file(&pbir_file(d)?, |j| {
                j["datasetReference"]["byConnection"] = json!({"connectionString": "Data Source=powerbi://api.powerbi.com/v1.0/myorg/W;Initial Catalog=M"});
            })
        },
    )?;
    proj(
        "j16",
        "files with CRLF line endings instead of LF",
        "accept",
        &|d| {
            let mut files = Vec::new();
            walk(d, d, &mut files, 0);
            for rel in files {
                let s = rel.to_string_lossy();
                if s.ends_with(".json")
                    || s.ends_with(".tmdl")
                    || s.ends_with(".pbir")
                    || s.ends_with(".pbism")
                    || s.ends_with(".pbip")
                    || s.ends_with(".platform")
                {
                    let p = d.join(&rel);
                    let t = std::fs::read_to_string(&p).map_err(|e| e.to_string())?;
                    std::fs::write(&p, t.replace("\r\n", "\n").replace('\n', "\r\n"))
                        .map_err(|e| e.to_string())?;
                }
            }
            Ok(())
        },
    )?;
    proj(
        "j17",
        ".pbip and definition.pbism without $schema (V1 form)",
        "accept",
        &|d| {
            for f in [
                find_file(d, &|r| r.ends_with(".pbip") && !r.contains('/')).ok_or("no .pbip")?,
                pbism_file(d)?,
            ] {
                edit_json_file(&f, |j| {
                    if let Some(m) = j.as_object_mut() {
                        m.remove("$schema");
                    }
                })?;
            }
            Ok(())
        },
    )?;
    proj(
        "j18",
        "report .platform plus item.config.json / item.metadata.json",
        "unknown",
        &|d| {
            let r = pbir_file(d)?;
            std::fs::write(
                r.with_file_name("item.config.json"),
                r#"{"version":"1.0","logicalId":"00000000-0000-0000-0000-000000000001"}"#,
            )
            .map_err(|e| e.to_string())?;
            std::fs::write(
                r.with_file_name("item.metadata.json"),
                r#"{"type":"report","displayName":"x"}"#,
            )
            .map_err(|e| e.to_string())
        },
    )?;
    proj(
        "j19",
        "semantic model folder with the legacy `.Dataset` suffix",
        "accept",
        &|d| {
            let model = pbism_file(d)?
                .parent()
                .map(Path::to_path_buf)
                .ok_or("no model folder")?;
            let name = model
                .file_name()
                .map(|n| n.to_string_lossy().replace(".SemanticModel", ".Dataset"))
                .ok_or("bad")?;
            let new = model.with_file_name(&name);
            std::fs::rename(&model, &new).map_err(|e| e.to_string())?;
            edit_json_file(&pbir_file(d)?, |j| {
                j["datasetReference"]["byPath"]["path"] = json!(format!("../{name}"))
            })
        },
    )?;
    proj(
        "j20",
        "definition.pbism 4.2 with a broken Copilot/settings.json (folder ignored below 6.0)",
        "accept",
        &|d| {
            let m = pbism_file(d)?;
            let c = m.with_file_name("Copilot");
            std::fs::create_dir_all(&c).map_err(|e| e.to_string())?;
            std::fs::write(c.join("settings.json"), "not json").map_err(|e| e.to_string())
        },
    )?;
    for (id, name, fact, expect) in [
        (
            "j21",
            "pg-1_A",
            "PBIR page name of word characters and a hyphen",
            "accept",
        ),
        (
            "j22",
            "pg.1",
            "PBIR page name with a dot (not a word character or hyphen)",
            "refuse",
        ),
        ("j23", "pg 1", "PBIR page name with a space", "refuse"),
    ] {
        proj(id, fact, expect, &|d| rename_page(d, name))?;
    }
    let list: Vec<Value> = cases
        .iter()
        .map(|c| json!({"id": c.id, "path": c.file, "fact": c.fact, "expect": c.expect}))
        .collect();
    std::fs::write(
        out.join("cases.json"),
        serde_json::to_string_pretty(&list).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    println!("{} cases in {}", list.len(), out.join("cases.json").display());
    Ok(())
}

/// Renames the project's only page (folder, `page.json` `name`, `pages.json` references).
fn rename_page(d: &Path, new: &str) -> Result<(), String> {
    let page_json = find_file(d, &|r| r.ends_with("/page.json")).ok_or("no page.json")?;
    let folder = page_json.parent().ok_or("no page folder")?.to_path_buf();
    let old = folder
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or("bad folder")?;
    edit_json_file(&page_json, |j| j["name"] = json!(new))?;
    let pages = folder.parent().ok_or("no pages folder")?.join("pages.json");
    edit_json_file(&pages, |j| {
        if let Some(a) = j.get_mut("pageOrder").and_then(Value::as_array_mut) {
            for x in a.iter_mut().filter(|x| x.as_str() == Some(old.as_str())) {
                *x = json!(new);
            }
        }
        if j.get("activePageName").and_then(Value::as_str) == Some(old.as_str()) {
            j["activePageName"] = json!(new);
        }
    })?;
    std::fs::rename(&folder, folder.with_file_name(new)).map_err(|e| e.to_string())
}

/// A valid OPC custom-properties part (ECMA-376 Part 1, 22.3) padded to `size` bytes.
fn custom_xml(size: usize) -> Vec<u8> {
    let head = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/custom-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"><property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="2" name="Probe"><vt:lpwstr>x</vt:lpwstr></property></Properties>"#;
    let mut s = head.as_bytes().to_vec();
    if s.len() < size {
        // XML allows trailing whitespace after the root element (Misc*).
        s.resize(size, b' ');
    }
    s
}

/// A 1×1 PNG.
fn tiny_png() -> Vec<u8> {
    vec![
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0x0D, 0x49, 0x48, 0x44, 0x52, 0, 0, 0, 1, 0,
        0, 0, 1, 8, 6, 0, 0, 0, 0x1F, 0x15, 0xC4, 0x89, 0, 0, 0, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C,
        0x63, 0, 1, 0, 0, 5, 0, 1, 0x0D, 0x0A, 0x2D, 0xB4, 0, 0, 0, 0, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42,
        0x60, 0x82,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_normalized() {
        assert_eq!(
            normalize("Report/definition/pages/abc/visuals/def/visual.json"),
            "Report/definition/pages/<page>/visuals/<visual>/visual.json"
        );
        assert_eq!(
            normalize("Report/definition/pages/pages.json"),
            "Report/definition/pages/pages.json"
        );
        assert_eq!(
            normalize("Sales.SemanticModel/definition/tables/Orders.tmdl"),
            "<Name>.SemanticModel/definition/tables/<item>.tmdl"
        );
        assert_eq!(
            normalize("Report/StaticResources/RegisteredResources/logo.png"),
            "Report/StaticResources/RegisteredResources/<item>.png"
        );
        assert_eq!(normalize("Version"), "Version");
    }

    #[test]
    fn encodings_are_detected() {
        assert_eq!(detect(&utf16("1.32")).0, Enc::Utf16Le);
        assert_eq!(detect(b"{\"a\":1}").0, Enc::Utf8);
        assert_eq!(detect(b"\xEF\xBB\xBFx").0, Enc::Utf8Bom);
        assert_eq!(detect(&[0xFF, 0xFE, b'1', 0]).0, Enc::Utf16LeBom);
    }

    #[test]
    fn rewritten_packages_parse_back() {
        let parts: Parts = vec![
            ("Version".into(), utf16("1.32"), false),
            ("[Content_Types].xml".into(), Vec::new(), false),
            ("Settings".into(), utf16("{\"Version\":4}"), false),
            ("Report/definition/report.json".into(), b"{}".to_vec(), false),
            ("DataModel".into(), vec![1, 2, 3], true),
        ];
        let dir = std::env::temp_dir().join(format!("pkg-oracle-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("a.pbix");
        write_package(&f, &parts, true).unwrap();
        let obs = observe_package(&std::fs::read(&f).unwrap()).unwrap();
        assert_eq!(obs["Version"]["text"], "1.32");
        assert_eq!(obs["content_types"]["defaults"], json!([["json", ""]]));
        assert_eq!(
            obs["content_types"]["overrides"],
            json!([
                ["Version", ""],
                ["Settings", "application/json"],
                ["DataModel", ""]
            ])
        );
        assert_eq!(obs["entries"][4]["method"], "stored");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
