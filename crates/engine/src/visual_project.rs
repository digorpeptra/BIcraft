//! Custom visual projects written in Rust, no Node (ROADMAP M11.30, docs/formats/pbiviz.md
//! "Visual projects"): `visual.scaffold` writes a project (`pbiviz.json`, `capabilities.json`
//! with the data roles and format options asked for, a TypeScript visual, its CSS and an icon we
//! draw), `visual.build` compiles its TypeScript and bundles it into one script with swc
//! (`bicraft_tsbuild`), and `visual.pack` writes the `.pbiviz` package that
//! `visual.import_custom` reads.
//!
//! A project folder is hostile input: every file read is capped, links and special files are
//! skipped, paths named in `pbiviz.json` stay inside the folder, and the sources reach the
//! bundler in memory (it never reads the disk).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::Session;
use crate::params::{allow, opt_bool, opt_str, str_param};

/// Most bytes of `pbiviz.json`, `capabilities.json`, `tsconfig.json`, the CSS or the icon.
const MAX_SMALL_FILE: u64 = 1 << 20;
/// Deepest folder under `src/` that is read.
const MAX_SRC_DEPTH: usize = 8;
/// Most data roles and format options a scaffold writes.
const MAX_ROLES: usize = 32;
const MAX_OPTIONS: usize = 64;
/// Longest name (visual, role, object, property) and display name.
const MAX_NAME: usize = 64;
const MAX_DISPLAY: usize = 200;
/// The API version a scaffold targets (powerbi-visuals-api, MIT).
pub(crate) const API_VERSION: &str = "5.3.0";
/// The generated module that registers the visual with the host page.
const PLUGIN_MODULE: &str = "__bicraft_plugin.js";
/// The data role kinds of capabilities.json.
const ROLE_KINDS: &[&str] = &["Grouping", "Measure", "GroupingOrMeasure"];
/// The format option kinds a scaffold writes, with their capabilities.json type.
const OPTION_KINDS: &[&str] = &["bool", "color", "numeric", "text"];

// ---------------------------------------------------------------------------------------------
// Reading a project
// ---------------------------------------------------------------------------------------------

/// A visual project read from its folder.
#[derive(Debug, Clone)]
pub(crate) struct Project {
    pub folder: PathBuf,
    /// `pbiviz.json`.
    pub pbiviz: Value,
    pub guid: String,
    pub display_name: String,
    pub version: String,
    pub class_name: String,
    pub api_version: String,
    pub capabilities: Value,
    pub css: String,
    pub icon: Option<Vec<u8>>,
    /// The sources under `src/` (project paths), and the entry among them.
    pub sources: BTreeMap<String, String>,
    pub entry: String,
    /// The localized strings (`stringResources/<locale>/resources.resjson`) by locale (`en-US`,
    /// `fr-FR`): each an object of texts by the `displayNameKey` capabilities.json names.
    pub string_resources: BTreeMap<String, Value>,
    pub warnings: Vec<String>,
}

/// Reads a small file of the project, capped and refusing links and special files.
fn read_small(path: &Path) -> Result<Vec<u8>, String> {
    let meta = fs::symlink_metadata(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if !meta.is_file() {
        return Err(format!("{} is not a plain file", path.display()));
    }
    if meta.len() > MAX_SMALL_FILE {
        return Err(format!(
            "{} is {} bytes; at most {MAX_SMALL_FILE} are read",
            path.display(),
            meta.len()
        ));
    }
    fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

fn read_text(path: &Path) -> Result<String, String> {
    let bytes = read_small(path)?;
    let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(&bytes);
    String::from_utf8(body.to_vec()).map_err(|_| format!("{} is not UTF-8 text", path.display()))
}

fn read_json(path: &Path) -> Result<Value, String> {
    let text = read_text(path)?;
    serde_json::from_str(&text).map_err(|e| format!("{}: not valid JSON ({e})", path.display()))
}

/// A path named in `pbiviz.json`: relative, without `..`, so it stays inside the folder.
fn inside(folder: &Path, rel: &str, what: &str) -> Result<PathBuf, String> {
    let p = Path::new(rel);
    if rel.trim().is_empty()
        || !p
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Err(format!(
            "pbiviz.json `{what}` is `{rel}`: a path inside the project folder is needed"
        ));
    }
    no_links(folder, p)?;
    Ok(folder.join(p))
}

/// Refuses a link among the parts of `rel` that exist under `base` (a link could lead out of
/// the project folder, and out of a server's workspace).
fn no_links(base: &Path, rel: &Path) -> Result<(), String> {
    let mut cur = base.to_path_buf();
    for c in rel.components() {
        cur.push(c);
        match fs::symlink_metadata(&cur) {
            Ok(m) if m.file_type().is_symlink() => {
                return Err(format!("{} is a link: links are not followed", cur.display()));
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    Ok(())
}

fn text_at<'a>(v: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut cur = v;
    for k in path {
        cur = cur.get(k)?;
    }
    cur.as_str().map(str::trim).filter(|s| !s.is_empty())
}

fn is_identifier(s: &str) -> bool {
    let mut b = s.bytes();
    b.next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_' || c == b'$')
        && b.all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'$')
        && s.len() <= MAX_NAME
}

/// The `.ts` / `.js` files under `src/` (project paths), skipping links and `node_modules`.
fn read_sources(folder: &Path) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    let mut total = 0usize;
    no_links(folder, Path::new("src"))?;
    let mut stack = vec![(folder.join("src"), "src".to_string(), 0usize)];
    while let Some((dir, rel, depth)) = stack.pop() {
        let entries = fs::read_dir(&dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
            let name = entry.file_name().to_string_lossy().to_string();
            let meta = fs::symlink_metadata(entry.path())
                .map_err(|e| format!("cannot read {}: {e}", entry.path().display()))?;
            let child = format!("{rel}/{name}");
            if meta.is_dir() {
                if name == "node_modules" || name.starts_with('.') || depth >= MAX_SRC_DEPTH {
                    continue;
                }
                stack.push((entry.path(), child, depth + 1));
            } else if meta.is_file() && bicraft_tsbuild::is_source(&name) && !name.contains('\\') {
                if out.len() >= bicraft_tsbuild::MAX_FILES {
                    return Err(format!(
                        "src/ holds more than {} source files",
                        bicraft_tsbuild::MAX_FILES
                    ));
                }
                if meta.len() > bicraft_tsbuild::MAX_FILE_BYTES as u64 {
                    return Err(format!(
                        "{child} is {} bytes; at most {} per source file",
                        meta.len(),
                        bicraft_tsbuild::MAX_FILE_BYTES
                    ));
                }
                let text = read_text_capped(&entry.path(), bicraft_tsbuild::MAX_FILE_BYTES as u64)?;
                total = total.saturating_add(text.len());
                if total > bicraft_tsbuild::MAX_TOTAL_BYTES {
                    return Err(format!(
                        "the sources under src/ pass {} bytes",
                        bicraft_tsbuild::MAX_TOTAL_BYTES
                    ));
                }
                out.insert(child, text);
            }
        }
    }
    Ok(out)
}

fn read_text_capped(path: &Path, cap: u64) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if bytes.len() as u64 > cap {
        return Err(format!("{} passes {cap} bytes", path.display()));
    }
    let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(&bytes);
    String::from_utf8(body.to_vec()).map_err(|_| format!("{} is not UTF-8 text", path.display()))
}

/// The entry module: `tsconfig.json` `files[0]` when it names a source under `src/`, else
/// `src/visual.ts`.
fn entry_of(
    folder: &Path,
    sources: &BTreeMap<String, String>,
    warnings: &mut Vec<String>,
) -> Result<String, String> {
    let tsconfig = folder.join("tsconfig.json");
    if fs::symlink_metadata(&tsconfig).is_ok_and(|m| m.is_file()) {
        match read_json(&tsconfig) {
            Ok(v) => {
                if let Some(first) = v
                    .get("files")
                    .and_then(Value::as_array)
                    .and_then(|a| a.first())
                    .and_then(Value::as_str)
                {
                    let rel = first.trim_start_matches("./").replace('\\', "/");
                    if sources.contains_key(&rel) {
                        return Ok(rel);
                    }
                    return Err(format!(
                        "tsconfig.json `files` names `{first}`, which is not a .ts or .js file under src/"
                    ));
                }
            }
            // tsconfig.json may hold comments (JSONC): fall back to the default entry
            Err(_) => warnings.push(
                "tsconfig.json is not plain JSON: its `files` are not read; the entry is src/visual.ts"
                    .into(),
            ),
        }
    }
    if sources.contains_key("src/visual.ts") {
        Ok("src/visual.ts".into())
    } else {
        Err("the project has no src/visual.ts (and tsconfig.json names no other entry)".into())
    }
}

/// Reads the project in `folder` (module docs).
pub(crate) fn read_project(folder: &Path) -> Result<Project, String> {
    if !fs::symlink_metadata(folder).is_ok_and(|m| m.is_dir()) {
        return Err(format!("{} is not a folder", folder.display()));
    }
    let manifest = folder.join("pbiviz.json");
    if !manifest.exists() {
        return Err(format!(
            "{} has no pbiviz.json: not a visual project (visual.scaffold writes one)",
            folder.display()
        ));
    }
    let pbiviz = read_json(&manifest)?;
    let visual = pbiviz
        .get("visual")
        .filter(|v| v.is_object())
        .ok_or("pbiviz.json has no `visual` object")?;
    let guid = text_at(visual, &["guid"]).ok_or("pbiviz.json has no `visual.guid`")?;
    if !bicraft_pbix::pbiviz::valid_guid(guid) {
        return Err(format!(
            "pbiviz.json `visual.guid` `{guid}`: ASCII letters, digits and _ only (at most {})",
            bicraft_pbix::pbiviz::MAX_GUID
        ));
    }
    let display_name = text_at(visual, &["displayName"])
        .or_else(|| text_at(visual, &["name"]))
        .unwrap_or(guid)
        .to_string();
    let version = text_at(visual, &["version"]).unwrap_or("1.0.0.0").to_string();
    if version.len() > 32 || !version.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return Err(format!(
            "pbiviz.json `visual.version` `{version}`: digits and dots, as 1.0.0.0"
        ));
    }
    let class_name = text_at(visual, &["visualClassName"])
        .unwrap_or("Visual")
        .to_string();
    if !is_identifier(&class_name) {
        return Err(format!(
            "pbiviz.json `visual.visualClassName` `{class_name}` is not a JavaScript name"
        ));
    }
    let api_version = text_at(&pbiviz, &["apiVersion"])
        .unwrap_or(API_VERSION)
        .to_string();
    if api_version.len() > 32 || !api_version.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return Err(format!(
            "pbiviz.json `apiVersion` `{api_version}`: digits and dots, as {API_VERSION}"
        ));
    }
    let mut warnings = Vec::new();
    let caps_rel = text_at(&pbiviz, &["capabilities"]).unwrap_or("capabilities.json");
    let capabilities = read_json(&inside(folder, caps_rel, "capabilities")?)?;
    if !capabilities.is_object() {
        return Err(format!("{caps_rel} is not a JSON object"));
    }
    if capabilities
        .get("dataRoles")
        .and_then(Value::as_array)
        .is_none_or(Vec::is_empty)
    {
        warnings.push(format!(
            "{caps_rel} declares no data roles: the visual takes no fields"
        ));
    }
    let style_rel = text_at(&pbiviz, &["style"]).unwrap_or("style/visual.css");
    if !style_rel.to_ascii_lowercase().ends_with(".css") {
        return Err(format!(
            "pbiviz.json `style` is `{style_rel}`: BIcraft packages plain CSS only (LESS is not \
             compiled); write the styles to a .css file and name it there"
        ));
    }
    let css = read_text(&inside(folder, style_rel, "style")?)?;
    let icon = match text_at(&pbiviz, &["assets", "icon"]) {
        None => {
            warnings.push("pbiviz.json names no icon (`assets.icon`): the package has none".into());
            None
        }
        Some(rel) => {
            let bytes = read_small(&inside(folder, rel, "assets.icon")?)?;
            if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
                return Err(format!("{rel} is not a PNG image"));
            }
            Some(bytes)
        }
    };
    let sources = read_sources(folder)?;
    let entry = entry_of(folder, &sources, &mut warnings)?;
    let string_resources = read_string_resources(folder, &mut warnings)?;
    Ok(Project {
        folder: folder.to_path_buf(),
        guid: guid.to_string(),
        display_name,
        version,
        class_name,
        api_version,
        capabilities,
        css,
        icon,
        sources,
        entry,
        string_resources,
        warnings,
        pbiviz,
    })
}

/// Most locales of `stringResources/` read, and most texts of one.
const MAX_LOCALES: usize = 64;
const MAX_STRINGS: usize = 2_000;

/// A locale folder's name: ASCII letters, digits and `-`, starting with a letter (`fr-FR`).
fn is_locale_name(s: &str) -> bool {
    s.len() <= 20
        && s.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// The project's `stringResources/<locale>/resources.resjson` files (Microsoft Learn, "Add the
/// locale in Power BI for Power BI visuals"): each locale's texts by key; other values are left
/// out, and a locale folder without the file with a warning.
fn read_string_resources(
    folder: &Path,
    warnings: &mut Vec<String>,
) -> Result<BTreeMap<String, Value>, String> {
    let mut out = BTreeMap::new();
    no_links(folder, Path::new("stringResources"))?;
    let dir = folder.join("stringResources");
    if !fs::symlink_metadata(&dir).is_ok_and(|m| m.is_dir()) {
        return Ok(out);
    }
    let mut names = Vec::new();
    for entry in fs::read_dir(&dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))? {
        let entry = entry.map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if fs::symlink_metadata(entry.path()).is_ok_and(|m| m.is_dir()) && is_locale_name(&name) {
            names.push(name);
        }
    }
    names.sort();
    for name in names.into_iter().take(MAX_LOCALES) {
        let rel = format!("stringResources/{name}/resources.resjson");
        let path = folder.join(&rel);
        if !fs::symlink_metadata(&path).is_ok_and(|m| m.is_file()) {
            warnings.push(format!("{rel} is missing: the locale `{name}` is left out"));
            continue;
        }
        let Value::Object(map) = read_json(&path)? else {
            return Err(format!("{rel} is not a JSON object of texts"));
        };
        let texts: Map<String, Value> = map
            .into_iter()
            .filter(|(_, v)| v.is_string())
            .take(MAX_STRINGS)
            .collect();
        out.insert(name, Value::Object(texts));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Building and packaging
// ---------------------------------------------------------------------------------------------

/// The registration module: imports the visual's class and registers a plugin for it with the
/// host page's `powerbi.visuals.plugins`, under the visual's guid (the plugin object's fields
/// are those of `IVisualPlugin` in the MIT powerbi-visuals-api), and exports it as `default`.
fn plugin_module(p: &Project) -> String {
    let import = p
        .entry
        .strip_suffix(".ts")
        .or_else(|| p.entry.strip_suffix(".js"))
        .unwrap_or(&p.entry);
    let q = |s: &str| serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into());
    let class = &p.class_name;
    format!(
        "import {{ {class} }} from {import};\n\
         var host = typeof window !== \"undefined\" ? window[\"powerbi\"] : undefined;\n\
         var plugin = {{\n\
         \x20   name: {guid},\n\
         \x20   displayName: {display},\n\
         \x20   class: {class_q},\n\
         \x20   apiVersion: {api},\n\
         \x20   create: function (options) {{ return new {class}(options); }},\n\
         \x20   custom: true\n\
         }};\n\
         if (host) {{\n\
         \x20   host.visuals = host.visuals || {{}};\n\
         \x20   host.visuals.plugins = host.visuals.plugins || {{}};\n\
         \x20   host.visuals.plugins[{guid}] = plugin;\n\
         }}\n\
         export default plugin;\n",
        import = q(&format!("./{import}")),
        guid = q(&p.guid),
        display = q(&p.display_name),
        class_q = q(class),
        api = q(&p.api_version),
    )
}

/// Whether the entry module exports the visual's class (checked on its text; the bundler does
/// not refuse a missing named import).
fn exports_class(p: &Project) -> bool {
    let Some(text) = p.sources.get(&p.entry) else {
        return false;
    };
    let class = &p.class_name;
    text.lines().any(|l| {
        let l = l.trim_start();
        l.starts_with(&format!("export class {class} "))
            || l.starts_with(&format!("export class {class}{{"))
            || l == format!("export class {class}")
            || (l.starts_with("export {") && l.contains(class.as_str()))
            || l.starts_with(&format!("export const {class} "))
            || l.starts_with(&format!("export function {class}("))
    })
}

/// Compiles the project's TypeScript with its registration module into one script.
pub(crate) fn build(p: &Project) -> Result<bicraft_tsbuild::Output, String> {
    if !exports_class(p) {
        return Err(format!(
            "{} does not export the class `{}` (pbiviz.json `visual.visualClassName`)",
            p.entry, p.class_name
        ));
    }
    let mut files = p.sources.clone();
    files.insert(PLUGIN_MODULE.into(), plugin_module(p));
    let mut out = bicraft_tsbuild::bundle(&files, PLUGIN_MODULE)?;
    out.modules.retain(|m| m != PLUGIN_MODULE);
    // The bundle's exports (`default`: the plugin) as a global named after the guid, as the
    // `pbiviz` tools' webpack build names its library: Power BI Desktop 2.159 registers a visual
    // without it but never draws it (docs/formats/pbiviz.md "Seen in Desktop").
    if p.guid.as_bytes().first().is_none_or(|b| b.is_ascii_digit()) {
        return Err(format!(
            "the guid `{}` must start with a letter or `_` (it names the visual's script)",
            p.guid
        ));
    }
    out.js = format!("var {} = {}", p.guid, out.js);
    Ok(out)
}

/// The `.pbiviz` bytes of a built project: `package.json` and `resources/<guid>.pbiviz.json`
/// (docs/formats/pbiviz.md "The file").
pub(crate) fn package(p: &Project, js: &str) -> Result<Vec<u8>, String> {
    let mut pkg = bicraft_opc::Package::default();
    for (name, bytes) in package_files(p, js)? {
        pkg.put(&name, bytes, false);
    }
    let bytes = pkg
        .to_bytes()
        .map_err(|e| format!("cannot write the package: {e}"))?;
    if bytes.len() > bicraft_pbix::pbiviz::MAX_PBIVIZ_BYTES {
        return Err(format!(
            "the package is {} bytes; a .pbiviz is read up to {} MB",
            bytes.len(),
            bicraft_pbix::pbiviz::MAX_PBIVIZ_BYTES >> 20
        ));
    }
    Ok(bytes)
}

/// The entries of a built project's `.pbiviz`: `package.json`, then the `.pbiviz.json`.
pub(crate) fn package_files(p: &Project, js: &str) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut visual = p.pbiviz.get("visual").cloned().unwrap_or_else(|| json!({}));
    if let Value::Object(v) = &mut visual {
        v.insert("guid".into(), json!(p.guid));
        v.insert("displayName".into(), json!(p.display_name));
        v.insert("version".into(), json!(p.version));
        v.insert("visualClassName".into(), json!(p.class_name));
    }
    let author = p.pbiviz.get("author").cloned().unwrap_or_else(|| json!({}));
    let icon = p
        .icon
        .as_ref()
        .map(|b| format!("data:image/png;base64,{}", crate::render::base64(b)))
        .unwrap_or_default();
    let file = format!("resources/{}.pbiviz.json", p.guid);
    let mut body = json!({
        "visual": visual,
        "apiVersion": p.api_version,
        "author": author,
        "capabilities": p.capabilities,
        "content": {"js": js, "css": p.css, "iconBase64": icon},
    });
    // the localized strings by locale, under `stringResources` as the open-source
    // powerbi-visuals-webpack-plugin writes them (docs/formats/pbiviz.md "Localized strings")
    if !p.string_resources.is_empty()
        && let Value::Object(o) = &mut body
    {
        o.insert("stringResources".into(), json!(p.string_resources));
    }
    let manifest = json!({
        "version": p.version,
        "visual": visual,
        "author": author,
        "resources": [{"resourceId": "rId0", "sourceType": 5, "file": file}],
        "metadata": {"pbivizjson": {"resourceId": "rId0"}},
    });
    let json_bytes = |v: &Value| serde_json::to_vec_pretty(v).map_err(|e| format!("cannot write JSON: {e}"));
    Ok(vec![
        ("package.json".to_string(), json_bytes(&manifest)?),
        (file, json_bytes(&body)?),
    ])
}

pub(crate) fn write_out(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    if fs::symlink_metadata(path).is_ok_and(|m| m.is_dir()) {
        return Err(format!("{} is a folder", path.display()));
    }
    fs::write(path, bytes).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// `out`, else `default` (a path under the project folder, its parts checked for links).
fn out_path(p: &Value, folder: &Path, default: &str) -> Result<PathBuf, String> {
    match opt_str(p, "out")?.filter(|s| !s.trim().is_empty()) {
        Some(out) => Ok(PathBuf::from(out)),
        None => {
            no_links(folder, Path::new(default))?;
            Ok(folder.join(default))
        }
    }
}

/// `visual.build {folder, out?}`: compiles the project to one JavaScript file.
pub(crate) fn visual_build(_s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["folder", "out"])?;
    let project = read_project(Path::new(str_param(p, "folder")?))?;
    let out = out_path(p, &project.folder, "dist/visual.js")?;
    let built = build(&project)?;
    write_out(&out, built.js.as_bytes())?;
    Ok(json!({
        "guid": project.guid,
        "entry": project.entry,
        "modules": built.modules,
        "out": out.display().to_string(),
        "js_bytes": built.js.len(),
        "warnings": project.warnings,
    }))
}

/// `visual.pack {folder, out?}`: builds the project and writes its `.pbiviz`.
pub(crate) fn visual_pack(_s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["folder", "out"])?;
    let project = read_project(Path::new(str_param(p, "folder")?))?;
    let default = format!("dist/{}.{}.pbiviz", project.guid, project.version);
    let out = out_path(p, &project.folder, &default)?;
    let built = build(&project)?;
    let bytes = package(&project, &built.js)?;
    write_out(&out, &bytes)?;
    Ok(json!({
        "guid": project.guid,
        "display_name": project.display_name,
        "version": project.version,
        "api_version": project.api_version,
        "entry": project.entry,
        "modules": built.modules,
        "out": out.display().to_string(),
        "bytes": bytes.len(),
        "js_bytes": built.js.len(),
        "has_icon": project.icon.is_some(),
        "warnings": project.warnings,
    }))
}

// ---------------------------------------------------------------------------------------------
// Scaffolding
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
struct Role {
    name: String,
    kind: &'static str,
    display: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FormatOption {
    object: String,
    property: String,
    kind: &'static str,
    display: String,
}

/// `category` → `Category`, `fontSize` → `Font size`.
fn humanize(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if i == 0 {
            out.extend(c.to_uppercase());
        } else if c.is_ascii_uppercase() {
            out.push(' ');
            out.push(c.to_ascii_lowercase());
        } else if c == '_' {
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out
}

fn display_of(given: Option<&str>, name: &str, what: &str) -> Result<String, String> {
    match given.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(humanize(name)),
        Some(d) if d.chars().count() > MAX_DISPLAY || d.chars().any(char::is_control) => Err(format!(
            "{what} `{name}`: its display name is too long or holds control characters"
        )),
        Some(d) => Ok(d.to_string()),
    }
}

fn pick_kind(given: &str, kinds: &[&'static str], what: &str) -> Result<&'static str, String> {
    kinds
        .iter()
        .find(|k| k.eq_ignore_ascii_case(given.trim()))
        .copied()
        .ok_or_else(|| format!("{what} kind `{given}`: one of {}", kinds.join(", ")))
}

/// The items of a list parameter: an array, or text with one item per line.
fn items<'a>(p: &'a Value, key: &str) -> Result<Option<Vec<&'a Value>>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(a)) => Ok(Some(a.iter().collect())),
        Some(_) => Err(format!("`{key}` must be a list")),
    }
}

fn parse_roles(p: &Value) -> Result<Vec<Role>, String> {
    let Some(list) = items(p, "data_roles")? else {
        return Ok(vec![
            Role {
                name: "category".into(),
                kind: "Grouping",
                display: "Category".into(),
            },
            Role {
                name: "measure".into(),
                kind: "Measure",
                display: "Measure".into(),
            },
        ]);
    };
    if list.len() > MAX_ROLES {
        return Err(format!("{} data roles: at most {MAX_ROLES}", list.len()));
    }
    let mut out: Vec<Role> = Vec::new();
    for item in list {
        let (name, kind, display) = match item {
            Value::String(s) => {
                let mut parts = s.splitn(3, ':');
                let name = parts.next().unwrap_or_default().trim().to_string();
                (
                    name,
                    parts.next().map(str::to_string),
                    parts.next().map(str::to_string),
                )
            }
            Value::Object(o) => {
                let get = |k: &str| -> Result<Option<String>, String> {
                    match o.get(k) {
                        None | Some(Value::Null) => Ok(None),
                        Some(Value::String(s)) => Ok(Some(s.clone())),
                        Some(_) => Err(format!("a data role's `{k}` must be a string")),
                    }
                };
                (
                    get("name")?.unwrap_or_default(),
                    get("kind")?,
                    get("display_name")?,
                )
            }
            _ => {
                return Err("a data role is `name:Kind[:Display name]` or {name, kind, display_name}".into());
            }
        };
        if !is_identifier(&name) {
            return Err(format!(
                "data role `{name}`: a name of letters, digits and _ is needed"
            ));
        }
        let kind = pick_kind(kind.as_deref().unwrap_or("Grouping"), ROLE_KINDS, "data role")?;
        if out.iter().any(|r| r.name.eq_ignore_ascii_case(&name)) {
            return Err(format!("data role `{name}` is given twice"));
        }
        let display = display_of(display.as_deref(), &name, "data role")?;
        out.push(Role { name, kind, display });
    }
    Ok(out)
}

fn parse_options(p: &Value) -> Result<Vec<FormatOption>, String> {
    let Some(list) = items(p, "format_options")? else {
        return Ok(vec![
            FormatOption {
                object: "dataPoint".into(),
                property: "fill".into(),
                kind: "color",
                display: "Color".into(),
            },
            FormatOption {
                object: "dataPoint".into(),
                property: "fontSize".into(),
                kind: "numeric",
                display: "Text size".into(),
            },
        ]);
    };
    if list.len() > MAX_OPTIONS {
        return Err(format!("{} format options: at most {MAX_OPTIONS}", list.len()));
    }
    let mut out: Vec<FormatOption> = Vec::new();
    for item in list {
        let (path, kind, display) = match item {
            Value::String(s) => {
                let mut parts = s.splitn(3, ':');
                let path = parts.next().unwrap_or_default().trim().to_string();
                (
                    path,
                    parts.next().map(str::to_string),
                    parts.next().map(str::to_string),
                )
            }
            Value::Object(o) => {
                let get = |k: &str| -> Result<Option<String>, String> {
                    match o.get(k) {
                        None | Some(Value::Null) => Ok(None),
                        Some(Value::String(s)) => Ok(Some(s.clone())),
                        Some(_) => Err(format!("a format option's `{k}` must be a string")),
                    }
                };
                let path = format!(
                    "{}.{}",
                    get("object")?.unwrap_or_default(),
                    get("property")?.unwrap_or_default()
                );
                (path, get("kind")?, get("display_name")?)
            }
            _ => {
                return Err("a format option is `object.property:kind[:Display name]` or {object, property, kind, display_name}".into());
            }
        };
        let Some((object, property)) = path.split_once('.') else {
            return Err(format!("format option `{path}`: `object.property` is needed"));
        };
        if !is_identifier(object) || !is_identifier(property) {
            return Err(format!(
                "format option `{path}`: object and property are names of letters, digits and _"
            ));
        }
        let kind = pick_kind(kind.as_deref().unwrap_or("text"), OPTION_KINDS, "format option")?;
        if out.iter().any(|o| o.object == object && o.property == property) {
            return Err(format!("format option `{path}` is given twice"));
        }
        let display = display_of(display.as_deref(), property, "format option")?;
        out.push(FormatOption {
            object: object.into(),
            property: property.into(),
            kind,
            display,
        });
    }
    Ok(out)
}

/// The `capabilities.json` of a scaffold: its roles, a categorical mapping, its format objects.
fn capabilities(roles: &[Role], options: &[FormatOption]) -> Value {
    let data_roles: Vec<Value> = roles
        .iter()
        .map(|r| json!({"displayName": r.display, "name": r.name, "kind": r.kind}))
        .collect();
    let grouping = roles.iter().find(|r| r.kind != "Measure");
    let measures: Vec<Value> = roles
        .iter()
        .filter(|r| r.kind == "Measure")
        .map(|r| json!({"bind": {"to": r.name}}))
        .collect();
    let mut categorical = Map::new();
    if let Some(g) = grouping {
        categorical.insert(
            "categories".into(),
            json!({"for": {"in": g.name}, "dataReductionAlgorithm": {"top": {"count": 1000}}}),
        );
    }
    if !measures.is_empty() {
        categorical.insert("values".into(), json!({"select": measures}));
    }
    let mut objects = Map::new();
    for o in options {
        let ty = match o.kind {
            "bool" => json!({"bool": true}),
            "color" => json!({"fill": {"solid": {"color": true}}}),
            "numeric" => json!({"numeric": true}),
            _ => json!({"text": true}),
        };
        let entry = objects
            .entry(o.object.clone())
            .or_insert_with(|| json!({"displayName": humanize(&o.object), "properties": {}}));
        if let Some(props) = entry.get_mut("properties").and_then(Value::as_object_mut) {
            props.insert(o.property.clone(), json!({"displayName": o.display, "type": ty}));
        }
    }
    let mut caps = Map::new();
    caps.insert("dataRoles".into(), Value::Array(data_roles));
    if !categorical.is_empty() {
        caps.insert("dataViewMappings".into(), json!([{"categorical": categorical}]));
    }
    caps.insert("objects".into(), Value::Object(objects));
    caps.insert("privileges".into(), json!([]));
    Value::Object(caps)
}

/// The scaffold's TypeScript visual: a line per category (`label: value`), the first colour
/// option colouring the text and the first number sizing it; its format options in the format
/// pane through `getFormattingModel`.
fn visual_ts(options: &[FormatOption]) -> String {
    let q = |s: &str| serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into());
    let mut list = String::new();
    for o in options {
        let value = match o.kind {
            "bool" => "true".to_string(),
            "color" => q("#118DFF"),
            "numeric" => "14".to_string(),
            _ => q(""),
        };
        list.push_str(&format!(
            "    {{ object: {}, property: {}, kind: {}, displayName: {}, value: {value} }},\n",
            q(&o.object),
            q(&o.property),
            q(o.kind),
            q(&o.display)
        ));
    }
    let mut names = String::new();
    let mut seen: Vec<&str> = Vec::new();
    for o in options {
        if !seen.contains(&o.object.as_str()) {
            seen.push(&o.object);
            names.push_str(&format!("    {}: {},\n", q(&o.object), q(&humanize(&o.object))));
        }
    }
    VISUAL_TS
        .replace("/*OPTIONS*/\n", &list)
        .replace("/*OBJECTS*/\n", &names)
}

const VISUAL_TS: &str = r#"// A custom visual written by BIcraft's visual.scaffold. Edit it, then run
// `bicraft visual pack <project folder>` to compile it and write its .pbiviz (no Node needed).
// Only the types of powerbi-visuals-api are used: the host page provides the API at run time.
"use strict";

import powerbi from "powerbi-visuals-api";
import VisualConstructorOptions = powerbi.extensibility.visual.VisualConstructorOptions;
import VisualUpdateOptions = powerbi.extensibility.visual.VisualUpdateOptions;
import IVisual = powerbi.extensibility.visual.IVisual;
import DataView = powerbi.DataView;

type OptionKind = "bool" | "color" | "numeric" | "text";

/** A format option of capabilities.json and its current value. */
interface FormatOption {
    object: string;
    property: string;
    kind: OptionKind;
    displayName: string;
    value: boolean | number | string;
}

const OPTIONS: FormatOption[] = [
/*OPTIONS*/
];

const OBJECT_NAMES: { [name: string]: string } = {
/*OBJECTS*/
};

/** The format pane control of each option kind (powerbi.visuals.FormattingComponent). */
const CONTROLS: { [kind: string]: string } = {
    bool: "ToggleSwitch",
    color: "ColorPicker",
    numeric: "NumUpDown",
    text: "TextInput",
};

/** Most lines drawn. */
const MAX_LINES = 100;

export class Visual implements IVisual {
    private readonly root: HTMLElement;
    private readonly options: FormatOption[] = OPTIONS.map((o) => ({ ...o }));

    constructor(options: VisualConstructorOptions) {
        this.root = document.createElement("div");
        this.root.className = "bicraft-visual";
        options.element.appendChild(this.root);
    }

    public update(options: VisualUpdateOptions): void {
        const dataView: DataView | undefined = options.dataViews ? options.dataViews[0] : undefined;
        this.readOptions(dataView);
        while (this.root.firstChild) {
            this.root.removeChild(this.root.firstChild);
        }
        const categorical = dataView ? dataView.categorical : undefined;
        const categories = categorical && categorical.categories ? categorical.categories[0] : undefined;
        const values = categorical && categorical.values ? categorical.values[0] : undefined;
        const count = categories ? categories.values.length : values ? values.values.length : 0;
        for (let i = 0; i < Math.min(count, MAX_LINES); i++) {
            const label = categories ? String(categories.values[i]) : "";
            const value = values ? String(values.values[i]) : "";
            this.line(label && value ? `${label}: ${value}` : label || value);
        }
        if (count === 0) {
            this.line("Add fields to the visual's data roles.");
        }
    }

    public getFormattingModel(): powerbi.visuals.FormattingModel {
        const cards: any[] = [];
        for (const o of this.options) {
            let card = cards.find((c) => c.uid === o.object);
            if (!card) {
                const name = OBJECT_NAMES[o.object] || o.object;
                card = { displayName: name, uid: o.object, groups: [{ displayName: name, uid: `${o.object}_group`, slices: [] }] };
                cards.push(card);
            }
            card.groups[0].slices.push({
                uid: `${o.object}_${o.property}`,
                displayName: o.displayName,
                control: {
                    type: CONTROLS[o.kind],
                    properties: {
                        descriptor: { objectName: o.object, propertyName: o.property },
                        value: o.kind === "color" ? { value: o.value } : o.value,
                    },
                },
            });
        }
        return { cards };
    }

    private readOptions(dataView: DataView | undefined): void {
        const objects: any = dataView && dataView.metadata ? dataView.metadata.objects : undefined;
        for (const o of this.options) {
            const object = objects ? objects[o.object] : undefined;
            const v = object ? object[o.property] : undefined;
            if (v === undefined || v === null) {
                continue;
            }
            if (o.kind === "color") {
                if (v.solid && typeof v.solid.color === "string") {
                    o.value = v.solid.color;
                }
            } else {
                o.value = v;
            }
        }
    }

    private line(text: string): void {
        const p = document.createElement("p");
        p.textContent = text;
        const color = this.options.find((o) => o.kind === "color");
        const size = this.options.find((o) => o.kind === "numeric");
        if (color) {
            p.style.color = String(color.value);
        }
        if (size) {
            p.style.fontSize = `${Number(size.value)}px`;
        }
        this.root.appendChild(p);
    }
}
"#;

const VISUAL_CSS: &str = ".bicraft-visual {\n    font-family: \"Segoe UI\", sans-serif;\n    overflow: auto;\n    height: 100%;\n}\n\n.bicraft-visual p {\n    margin: 4px 8px;\n}\n";

/// The scaffold's 20×20 icon: three rising bars on a rounded tile (drawn here, our own).
pub(crate) fn icon_png() -> Result<Vec<u8>, String> {
    tile_icon(&[
        (4.0, 11.0, 3.0, 5.0),
        (8.5, 8.0, 3.0, 8.0),
        (13.0, 5.0, 3.0, 11.0),
    ])
}

/// A 20×20 icon of our own: white `(x, y, w, h)` bars on a rounded tile.
pub(crate) fn tile_icon(bars: &[(f32, f32, f32, f32)]) -> Result<Vec<u8>, String> {
    use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Rect, Transform};
    let mut pm = Pixmap::new(20, 20).ok_or("cannot draw the icon")?;
    // tiny-skia's paint is anti-aliased by default
    let mut paint = Paint::default();
    paint.set_color(Color::from_rgba8(0x1F, 0x6F, 0x8B, 0xFF));
    let tile = {
        let mut b = PathBuilder::new();
        let (x0, y0, x1, y1, r) = (1.0f32, 1.0f32, 19.0f32, 19.0f32, 4.0f32);
        b.move_to(x0 + r, y0);
        b.line_to(x1 - r, y0);
        b.quad_to(x1, y0, x1, y0 + r);
        b.line_to(x1, y1 - r);
        b.quad_to(x1, y1, x1 - r, y1);
        b.line_to(x0 + r, y1);
        b.quad_to(x0, y1, x0, y1 - r);
        b.line_to(x0, y0 + r);
        b.quad_to(x0, y0, x0 + r, y0);
        b.close();
        b.finish().ok_or("cannot draw the icon")?
    };
    pm.fill_path(&tile, &paint, FillRule::Winding, Transform::identity(), None);
    paint.set_color(Color::from_rgba8(0xFF, 0xFF, 0xFF, 0xFF));
    for (x, y, w, h) in bars.iter().copied() {
        if let Some(r) = Rect::from_xywh(x, y, w, h) {
            pm.fill_rect(r, &paint, Transform::identity(), None);
        }
    }
    pm.encode_png().map_err(|e| format!("cannot write the icon: {e}"))
}

/// The ink of a drawn package icon: the theme's text colour cannot follow the host's theme in a
/// PNG, so a dark slate grey near the ink of Power BI Desktop's own pane icons, which keep their
/// dark ink in both themes (7.5:1 on its light pane, 2.3:1 on its dark one; G111).
pub(crate) const ICON_INK: [u8; 3] = [0x4a, 0x50, 0x58];

/// How much bolder a package icon's strokes are than the UI's stroke rule: Desktop shows the PNG
/// at 20 × 20 CSS pixels, upscaled on a scaled screen, beside its own heavier icons (G111).
const ICON_WEIGHT: f32 = 1.3;

/// A tone's opacity in a package icon, fuller than in the UI so the PNG does not wash out once
/// upscaled (G111): ink `1 - (1 - a)³`, near the solid ink of Desktop's own icons, and the
/// accent `1 - (1 - a)²`, which keeps overlapping and shaded fills apart. Both keep the tones'
/// order.
fn icon_opacity(a: f32, power: i32) -> f32 {
    1.0 - (1.0 - a.clamp(0.0, 1.0)).powi(power)
}

/// A `side` × `side` PNG of a drawing on the UI icons' 24 × 24 grid (`bicraft_visuals::icons`),
/// with the UI's stroke rule (≈ side/16 at 16 px, side/20 at 32 px, never under 1 px) made
/// [`ICON_WEIGHT`] bolder, tones made fuller ([`icon_opacity`]), on a transparent background.
pub(crate) fn marks_png(marks: &[bicraft_visuals::icons::Mark], side: u32) -> Result<Vec<u8>, String> {
    use bicraft_visuals::icons::{ACCENT_RGB, GRID, Mark, Tone};
    use tiny_skia::{Color, FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform};
    let side = side.clamp(8, 256);
    let mut pm = Pixmap::new(side, side).ok_or("cannot draw the icon")?;
    let size = side as f32;
    let k = size / GRID;
    let t = ((size - 16.0) / 16.0).clamp(0.0, 1.0);
    let sw = (size / (16.0 + 4.0 * t)).max(1.0) * ICON_WEIGHT;
    let paint = |tone: Tone| {
        let (rgb, a) = match tone {
            Tone::Ink(a) => (ICON_INK, icon_opacity(a, 3)),
            Tone::Accent(a) => (ACCENT_RGB, icon_opacity(a, 2)),
        };
        let mut p = Paint::default();
        p.set_color(Color::from_rgba8(
            rgb[0],
            rgb[1],
            rgb[2],
            (a.clamp(0.0, 1.0) * 255.0).round() as u8,
        ));
        p
    };
    let stroke = |w: f32| Stroke {
        width: sw * w,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Stroke::default()
    };
    let poly = |pts: &[(f32, f32)], close: bool| {
        let mut b = PathBuilder::new();
        for (i, &(x, y)) in pts.iter().take(512).enumerate() {
            if i == 0 {
                b.move_to(x * k, y * k);
            } else {
                b.line_to(x * k, y * k);
            }
        }
        if close {
            b.close();
        }
        b.finish()
    };
    let id = Transform::identity();
    for m in marks.iter().take(256) {
        match m {
            Mark::Line { pts, width, tone } => {
                if let Some(path) = poly(pts, false) {
                    pm.stroke_path(&path, &paint(*tone), &stroke(*width), id, None);
                }
            }
            Mark::Fill { pts, tone } => {
                if let Some(path) = poly(pts, true) {
                    pm.fill_path(&path, &paint(*tone), FillRule::Winding, id, None);
                }
            }
            Mark::Rect {
                min,
                max,
                radius,
                fill,
                stroke: outline,
            } => {
                let (x0, y0, x1, y1) = (min.0 * k, min.1 * k, max.0 * k, max.1 * k);
                let r = (radius * k).min((x1 - x0) * 0.5).min((y1 - y0) * 0.5).max(0.0);
                let mut b = PathBuilder::new();
                b.move_to(x0 + r, y0);
                b.line_to(x1 - r, y0);
                b.quad_to(x1, y0, x1, y0 + r);
                b.line_to(x1, y1 - r);
                b.quad_to(x1, y1, x1 - r, y1);
                b.line_to(x0 + r, y1);
                b.quad_to(x0, y1, x0, y1 - r);
                b.line_to(x0, y0 + r);
                b.quad_to(x0, y0, x0 + r, y0);
                b.close();
                if let Some(path) = b.finish() {
                    if let Some(f) = fill {
                        pm.fill_path(&path, &paint(*f), FillRule::Winding, id, None);
                    }
                    if let Some(s) = outline {
                        pm.stroke_path(&path, &paint(*s), &stroke(1.0), id, None);
                    }
                }
            }
            Mark::Circle {
                center,
                radius,
                fill,
                stroke: outline,
            } => {
                if let Some(path) = PathBuilder::from_circle(center.0 * k, center.1 * k, radius * k) {
                    if let Some(f) = fill {
                        pm.fill_path(&path, &paint(*f), FillRule::Winding, id, None);
                    }
                    if let Some(s) = outline {
                        pm.stroke_path(&path, &paint(*s), &stroke(1.0), id, None);
                    }
                }
            }
        }
    }
    pm.encode_png().map_err(|e| format!("cannot write the icon: {e}"))
}

/// A guid for `name`: the name and 32 hexadecimal digits, as `pbiviz new` makes them.
fn new_guid(name: &str) -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut hex = String::new();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    for i in 0..2u64 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(now);
        h.write_u64(i);
        h.write(name.as_bytes());
        hex.push_str(&format!("{:016X}", h.finish()));
    }
    format!("{name}{hex}")
}

/// `visual.scaffold {folder, name, display_name?, guid?, description?, author?, data_roles?,
/// format_options?, overwrite?, template?}`: writes a visual project.
pub(crate) fn visual_scaffold(_s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "folder",
            "name",
            "display_name",
            "guid",
            "description",
            "author",
            "data_roles",
            "format_options",
            "overwrite",
            "template",
        ],
    )?;
    let folder = PathBuf::from(str_param(p, "folder")?);
    let name = str_param(p, "name")?.trim();
    if !is_identifier(name) || name.contains('$') {
        return Err(format!(
            "visual name `{name}`: letters, digits and _, starting with a letter (at most {MAX_NAME})"
        ));
    }
    let display = display_of(opt_str(p, "display_name")?, name, "visual")?;
    let guid = match opt_str(p, "guid")?.map(str::trim).filter(|g| !g.is_empty()) {
        Some(g) if bicraft_pbix::pbiviz::valid_guid(g) => g.to_string(),
        Some(g) => {
            return Err(format!(
                "guid `{g}`: ASCII letters, digits and _ only (at most {})",
                bicraft_pbix::pbiviz::MAX_GUID
            ));
        }
        None => new_guid(name),
    };
    let description = opt_str(p, "description")?.unwrap_or_default();
    let author = opt_str(p, "author")?.unwrap_or_default();
    if description.chars().count() > 1000 || author.chars().count() > MAX_DISPLAY {
        return Err("`description` (1000 characters) or `author` (200) is too long".into());
    }
    let overwrite = opt_bool(p, "overwrite")?.unwrap_or(false);
    // `spec`: the BIcraft Spec visual's own sources (docs/formats/spec-visual.md), to edit
    let spec = match opt_str(p, "template")?.map(str::trim).filter(|t| !t.is_empty()) {
        None | Some("default") => false,
        Some("spec") => true,
        Some(t) => {
            return Err(format!(
                "template `{}`: `default` (a line per category) or `spec` (the BIcraft Spec visual)",
                t.chars().take(40).collect::<String>()
            ));
        }
    };
    let given = |k: &str| p.get(k).is_some_and(|v| !v.is_null());
    if spec && (given("data_roles") || given("format_options")) {
        return Err(
            "the spec template has its own data roles (category, values) and format option (spec.json)"
                .into(),
        );
    }
    let roles = parse_roles(p)?;
    let options = parse_options(p)?;
    if folder.join("pbiviz.json").exists() && !overwrite {
        return Err(format!(
            "{} already holds a visual project (pass overwrite: true to replace its files)",
            folder.display()
        ));
    }
    if fs::symlink_metadata(&folder).is_ok_and(|m| !m.is_dir()) {
        return Err(format!("{} is not a folder", folder.display()));
    }
    let pbiviz = json!({
        "visual": {
            "name": name,
            "displayName": display,
            "guid": guid,
            "visualClassName": "Visual",
            "version": "1.0.0.0",
            "description": description,
            "supportUrl": "",
            "gitHubUrl": "",
        },
        "apiVersion": API_VERSION,
        "author": {"name": author, "email": ""},
        "assets": {"icon": "assets/icon.png"},
        "style": "style/visual.css",
        "capabilities": "capabilities.json",
    });
    let tsconfig = json!({
        "compilerOptions": {
            "target": "es2020",
            "module": "es2020",
            "moduleResolution": "node",
            "lib": ["es2020", "dom"],
            "strict": false,
            "noEmit": true,
        },
        "files": ["src/visual.ts"],
    });
    // for editors only (the types of powerbi-visuals-api); BIcraft's build needs no npm install
    let npm = json!({
        "name": name.to_ascii_lowercase(),
        "private": true,
        "devDependencies": {"powerbi-visuals-api": format!("~{API_VERSION}")},
    });
    let pretty = |v: &Value| -> Result<Vec<u8>, String> {
        let mut b = serde_json::to_vec_pretty(v).map_err(|e| format!("cannot write JSON: {e}"))?;
        b.push(b'\n');
        Ok(b)
    };
    let mut strings = BTreeMap::new();
    let (caps, ts, css) = if spec {
        let mut caps = crate::spec_visual::capabilities();
        strings = crate::visual_runtime::localize(&mut caps)?;
        (
            caps,
            crate::spec_visual::VISUAL_TS.to_string(),
            crate::spec_visual::css(),
        )
    } else {
        (
            capabilities(&roles, &options),
            visual_ts(&options),
            VISUAL_CSS.to_string(),
        )
    };
    let mut files: Vec<(String, Vec<u8>)> = vec![
        ("pbiviz.json".into(), pretty(&pbiviz)?),
        ("capabilities.json".into(), pretty(&caps)?),
        ("tsconfig.json".into(), pretty(&tsconfig)?),
        ("package.json".into(), pretty(&npm)?),
        ("src/visual.ts".into(), ts.into_bytes()),
        ("style/visual.css".into(), css.into_bytes()),
        ("assets/icon.png".into(), icon_png()?),
    ];
    if spec {
        let mut extra = vec![(
            "src/spec.ts".to_string(),
            crate::spec_visual::SPEC_TS.as_bytes().to_vec(),
        )];
        // the Spec visual imports BIcraft's visual runtime
        for (name, text) in crate::visual_runtime::FILES {
            extra.push((
                format!("{}/{name}", crate::visual_runtime::FOLDER),
                text.as_bytes().to_vec(),
            ));
        }
        files.splice(5..5, extra);
    }
    // its data role and format option names in English and French
    for (locale, texts) in &strings {
        files.push((
            format!("stringResources/{locale}/resources.resjson"),
            pretty(texts)?,
        ));
    }
    for (rel, bytes) in &files {
        no_links(&folder, Path::new(rel))?;
        write_out(&folder.join(rel), bytes)?;
    }
    Ok(json!({
        "folder": folder.display().to_string(),
        "name": name,
        "display_name": display,
        "guid": guid,
        "template": if spec { "spec" } else { "default" },
        "data_roles": if spec { crate::spec_visual::roles_json() } else { roles.iter().map(|r| json!({"name": r.name, "kind": r.kind, "display_name": r.display})).collect::<Vec<_>>() },
        "format_options": if spec { vec![json!({"object": "spec", "property": "json", "kind": "text", "display_name": "Spec (JSON)"})] } else { options.iter().map(|o| json!({"object": o.object, "property": o.property, "kind": o.kind, "display_name": o.display})).collect::<Vec<_>>() },
        "files": files.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
    }))
}
