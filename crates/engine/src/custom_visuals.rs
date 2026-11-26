//! `report.custom_visuals`: the custom visuals a report carries or references (read-only);
//! `visual.import_custom` and `visual.remove_custom`: a `.pbiviz` package imported into the report
//! or removed from it (docs/formats/pbiviz.md); the Visualizations pane's list ([`gallery`]).
//! A custom visual's JavaScript is never run: BIcraft draws its visuals as a placeholder.
//!
//! Three kinds (`docs/formats/pbix-package.md` §2, "Custom visuals"):
//!
//! - **embedded**: a `CustomVisual` resource package in `report.json` (`resourcePackages`) whose
//!   files sit under `CustomVisuals/<package name>/`: `package.json` (the `.pbiviz` manifest) and
//!   `resources/<guid>.pbiviz.json` (the visual: metadata, capabilities, JavaScript, CSS, icon),
//!   which the package's `CustomVisualMetadata` item names. A folder under `CustomVisuals/`
//!   without a package entry is listed too (Desktop does not load it).
//! - **public**: AppSource visuals listed in `publicCustomVisuals`, fetched by Desktop, not stored.
//! - **organization**: `organizationCustomVisuals` entries (the organization's store).
//!
//! Each kind says which visuals use it (`visualType` equal to the visual's name).

use std::collections::BTreeMap;

use bicraft_report::Report;
use serde_json::{Value, json};

use crate::Session;
use crate::params::allow;
use crate::report::report_ref;

const PREFIX: &str = "CustomVisuals/";

/// A file of a visual's folder: path relative to `CustomVisuals/<dir>/`, and its bytes.
type File<'a> = (String, &'a [u8]);
/// A visual's folder: its spelling and its files.
type Folder<'a> = (String, Vec<File<'a>>);

/// Largest `package.json` / `.pbiviz.json` parsed for the listing (a visual's bundle with its
/// JavaScript is usually well under this).
const MAX_JSON_BYTES: usize = 64 << 20;

pub(crate) fn report_custom_visuals(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &[])?;
    Ok(list(report_ref(s)?))
}

/// `(page name, visual name)` of every visual whose type is `visual_type` (case-insensitive,
/// like Desktop's plugin lookup).
pub(crate) fn used_by(report: &Report, visual_type: &str) -> Vec<Value> {
    let mut out = Vec::new();
    for page in &report.pages {
        for v in &page.visuals {
            if v.visual_type
                .as_deref()
                .is_some_and(|t| t.eq_ignore_ascii_case(visual_type))
            {
                out.push(
                    json!({"page": page.name, "page_display_name": page.display_name, "visual": v.name}),
                );
            }
        }
    }
    out
}

fn parse_json(bytes: &[u8]) -> Option<Value> {
    if bytes.len() > MAX_JSON_BYTES {
        return None;
    }
    let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(bytes);
    serde_json::from_slice(body).ok()
}

fn str_at<'a>(v: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut cur = v;
    for key in path {
        cur = cur.get(key)?;
    }
    cur.as_str()
}

pub(crate) fn list(report: &Report) -> Value {
    // Files under CustomVisuals/<dir>/, by lower-case directory name.
    let mut dirs: BTreeMap<String, Folder<'_>> = BTreeMap::new();
    for r in &report.resources {
        let path = r.path.replace('\\', "/");
        let Some(rest) = path.strip_prefix(PREFIX) else {
            continue;
        };
        let Some((dir, file)) = rest.split_once('/') else {
            continue;
        };
        let entry = dirs
            .entry(dir.to_lowercase())
            .or_insert_with(|| (dir.to_string(), Vec::new()));
        entry.1.push((file.to_string(), r.bytes.as_slice()));
    }
    let empty = Vec::new();
    let packages = report
        .extra
        .get("resourcePackages")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    let mut embedded = Vec::new();
    let mut seen_dirs = Vec::new();
    for package in packages {
        let kind = package.get("type").and_then(Value::as_str).unwrap_or_default();
        if kind != "CustomVisual" {
            continue;
        }
        let name = package.get("name").and_then(Value::as_str).unwrap_or_default();
        let files = dirs.get(&name.to_lowercase());
        if files.is_some() {
            seen_dirs.push(name.to_lowercase());
        }
        let items: Vec<Value> = package
            .get("items")
            .and_then(Value::as_array)
            .unwrap_or(&empty)
            .iter()
            .map(|item| {
                let path = item.get("path").and_then(Value::as_str).unwrap_or_default();
                json!({
                    "name": item.get("name").cloned().unwrap_or(Value::Null),
                    "type": item.get("type").cloned().unwrap_or(Value::Null),
                    "path": path,
                    "file": files.and_then(|(_, f)| find_item(f, path)).map(|(rel, _)| format!("{PREFIX}{name}/{rel}")),
                })
            })
            .collect();
        let metadata_path = package
            .get("items")
            .and_then(Value::as_array)
            .unwrap_or(&empty)
            .iter()
            .find(|i| i.get("type").and_then(Value::as_str) == Some("CustomVisualMetadata"))
            .and_then(|i| i.get("path").and_then(Value::as_str));
        let mut entry = describe(
            report,
            name,
            files.map(|(d, f)| (d.as_str(), f.as_slice())),
            metadata_path,
        );
        if let Value::Object(m) = &mut entry {
            m.insert("registered".into(), Value::Bool(true));
            m.insert(
                "disabled".into(),
                Value::Bool(package.get("disabled").and_then(Value::as_bool).unwrap_or(false)),
            );
            m.insert("items".into(), Value::Array(items));
        }
        embedded.push(entry);
    }
    // Folders without a resource package: carried, but not loaded by Desktop.
    for (key, (dir, files)) in &dirs {
        if seen_dirs.contains(key) {
            continue;
        }
        let mut entry = describe(report, dir, Some((dir.as_str(), files.as_slice())), None);
        if let Value::Object(m) = &mut entry {
            m.insert("registered".into(), Value::Bool(false));
            if let Some(Value::Array(issues)) = m.get_mut("issues") {
                issues.push(Value::String(
                    "no `CustomVisual` entry in report.json `resourcePackages`: Desktop does not load these files"
                        .into(),
                ));
            }
        }
        embedded.push(entry);
    }
    let public: Vec<Value> = report
        .extra
        .get("publicCustomVisuals")
        .and_then(Value::as_array)
        .unwrap_or(&empty)
        .iter()
        .filter_map(Value::as_str)
        .map(|name| json!({"name": name, "used_by": used_by(report, name)}))
        .collect();
    let organization: Vec<Value> = report
        .extra
        .get("organizationCustomVisuals")
        .and_then(Value::as_array)
        .unwrap_or(&empty)
        .iter()
        .map(|o| {
            let name = o.get("name").and_then(Value::as_str).unwrap_or_default();
            json!({
                "name": name,
                "path": o.get("path").cloned().unwrap_or(Value::Null),
                "disabled": o.get("disabled").and_then(Value::as_bool).unwrap_or(false),
                "used_by": used_by(report, name),
            })
        })
        .collect();
    // an embedded package of one of BIcraft's visuals says so (its registry name)
    for e in &mut embedded {
        let own = e
            .get("name")
            .and_then(Value::as_str)
            .and_then(|n| crate::builtin_visuals::find(n).filter(|b| b.guid.eq_ignore_ascii_case(n)))
            .map(|b| b.name);
        if let Value::Object(m) = e {
            m.insert("bicraft".into(), json!(own));
        }
    }
    json!({
        "embedded": embedded,
        "public": public,
        "organization": organization,
        "count": embedded.len() + public.len() + organization.len(),
        "bicraft": crate::builtin_visuals::report_json(report),
    })
}

/// The file an item path names. Desktop 2.158 resolves item paths under the visual's
/// `resources/` folder: a `CustomVisualMetadata` item `<guid>.pbiviz.json` is the file
/// `CustomVisuals/<name>/resources/<guid>.pbiviz.json` (checked on Desktop: a path of
/// `resources/<guid>.pbiviz.json`, or a missing file, leaves the visual unregistered).
fn find_item<'a>(files: &'a [File<'a>], path: &str) -> Option<&'a File<'a>> {
    let under = format!("resources/{}", path.replace('\\', "/"));
    files.iter().find(|(f, _)| f.eq_ignore_ascii_case(&under))
}

/// One embedded visual: what its manifest and `.pbiviz.json` say, its files and its users.
fn describe(
    report: &Report,
    name: &str,
    files: Option<(&str, &[File<'_>])>,
    metadata_path: Option<&str>,
) -> Value {
    let mut issues: Vec<Value> = Vec::new();
    let list: &[File<'_>] = files.map(|(_, f)| f).unwrap_or(&[]);
    let dir = files.map(|(d, _)| d).unwrap_or(name);
    let manifest = list
        .iter()
        .find(|(f, _)| f.eq_ignore_ascii_case("package.json"))
        .and_then(|(_, b)| parse_json(b));
    if files.is_none() {
        issues.push(Value::String(format!("no files under `{PREFIX}{name}/`")));
    } else if manifest.is_none() {
        issues.push(Value::String("`package.json` is missing or not JSON".into()));
    }
    let pbiviz_file = match metadata_path {
        Some(p) => find_item(list, p),
        None => list
            .iter()
            .find(|(f, _)| f.starts_with("resources/") && f.to_ascii_lowercase().ends_with(".pbiviz.json")),
    };
    if let Some(p) = metadata_path
        && pbiviz_file.is_none()
        && files.is_some()
    {
        issues.push(Value::String(format!(
            "the `CustomVisualMetadata` item `{p}` names no file under `resources/`: Desktop does not load the visual"
        )));
    }
    let pbiviz = pbiviz_file.and_then(|(_, b)| parse_json(b));
    if pbiviz_file.is_some() && pbiviz.is_none() {
        issues.push(Value::String("the `.pbiviz.json` file is not JSON".into()));
    }
    let visual = pbiviz
        .as_ref()
        .and_then(|v| v.get("visual"))
        .or_else(|| manifest.as_ref().and_then(|m| m.get("visual")));
    let field = |key: &str| visual.and_then(|v| v.get(key)).and_then(Value::as_str);
    let guid = field("guid").unwrap_or(name);
    if !guid.eq_ignore_ascii_case(name) {
        issues.push(Value::String(format!(
            "the visual's guid `{guid}` differs from its package name `{name}`"
        )));
    }
    let content = pbiviz.as_ref().and_then(|v| v.get("content"));
    let len_of = |key: &str| {
        content
            .and_then(|c| c.get(key))
            .and_then(Value::as_str)
            .map(str::len)
    };
    let mut files_out: Vec<Value> = list
        .iter()
        .map(|(f, b)| json!({"path": format!("{PREFIX}{dir}/{f}"), "bytes": b.len()}))
        .collect();
    files_out.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    let mut users = used_by(report, guid);
    if !guid.eq_ignore_ascii_case(name) {
        users.extend(used_by(report, name));
    }
    json!({
        "name": name,
        "guid": guid,
        "display_name": field("displayName"),
        "version": field("version").or_else(|| manifest.as_ref().and_then(|m| str_at(m, &["version"]))),
        "api_version": pbiviz.as_ref().and_then(|v| str_at(v, &["apiVersion"])),
        "author": pbiviz.as_ref().and_then(|v| str_at(v, &["author", "name"]))
            .or_else(|| manifest.as_ref().and_then(|m| str_at(m, &["author", "name"]))),
        "description": field("description").filter(|d| !d.is_empty()),
        "data_roles": pbiviz.as_ref()
            .and_then(|v| v.pointer("/capabilities/dataRoles"))
            .and_then(Value::as_array)
            .map(|roles| roles.iter().filter_map(|r| r.get("name").and_then(Value::as_str)).collect::<Vec<_>>()),
        "privileges": pbiviz.as_ref()
            .and_then(|v| v.pointer("/capabilities/privileges"))
            .and_then(Value::as_array)
            .map(|p| p.iter().filter_map(|x| x.get("name").and_then(Value::as_str)).collect::<Vec<_>>()),
        "javascript_bytes": len_of("js"),
        "css_bytes": len_of("css"),
        "has_icon": len_of("iconBase64").is_some_and(|n| n > 0),
        "metadata_file": pbiviz_file.map(|(f, _)| format!("{PREFIX}{dir}/{f}")),
        "files": files_out,
        "used_by": users,
        "issues": issues,
    })
}

// ---------------------------------------------------------------------------------------------
// The Visualizations pane's custom visuals (the snapshot's `report.custom_visuals`)
// ---------------------------------------------------------------------------------------------

/// Most custom visuals listed in the pane.
const MAX_GALLERY: usize = 200;

/// The registered, enabled embedded custom visuals (a `CustomVisual` package whose metadata
/// file is present) as `(name, display name, icon)`: the name is the visual type its visuals
/// carry, the icon the `content.iconBase64` `data:` URL of its `.pbiviz.json`. Never fails: what
/// does not read is left out.
pub(crate) fn gallery(report: &Report) -> Vec<(String, Option<String>, Option<String>)> {
    let mut out: Vec<(String, Option<String>, Option<String>)> = Vec::new();
    let Some(packages) = report.extra.get("resourcePackages").and_then(Value::as_array) else {
        return out;
    };
    for package in packages.iter().take(1000) {
        if out.len() >= MAX_GALLERY {
            break;
        }
        if package.get("type").and_then(Value::as_str) != Some("CustomVisual")
            || package.get("disabled").and_then(Value::as_bool) == Some(true)
        {
            continue;
        }
        let Some(name) = package.get("name").and_then(Value::as_str) else {
            continue;
        };
        if out.iter().any(|(n, _, _)| n.eq_ignore_ascii_case(name)) {
            continue;
        }
        let Some(item) = package
            .get("items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|i| i.get("type").and_then(Value::as_str) == Some("CustomVisualMetadata"))
            .and_then(|i| i.get("path").and_then(Value::as_str))
        else {
            continue;
        };
        let want = format!("{PREFIX}{name}/resources/{}", item.replace('\\', "/"));
        let Some(doc) = report
            .resources
            .iter()
            .find(|r| r.path.replace('\\', "/").eq_ignore_ascii_case(&want))
            .and_then(|r| parse_json(&r.bytes))
        else {
            continue;
        };
        let display = str_at(&doc, &["visual", "displayName"])
            .filter(|d| !d.trim().is_empty())
            .map(|d| d.chars().take(200).collect());
        let icon = str_at(&doc, &["content", "iconBase64"])
            .filter(|i| i.get(..5).is_some_and(|h| h.eq_ignore_ascii_case("data:")))
            .map(str::to_string);
        out.push((name.to_string(), display, icon));
    }
    out
}

// ---------------------------------------------------------------------------------------------
// visual.import_custom / visual.remove_custom
// ---------------------------------------------------------------------------------------------

/// Longest `data` parameter: base64 of the largest `.pbiviz` read, or a `data:` URL of it.
const MAX_DATA_CHARS: usize = bicraft_pbix::pbiviz::MAX_PBIVIZ_BYTES / 3 * 4 + 256;

/// The `.pbiviz` bytes of `p`: `path` (a file) or `data` (base64 or a `data:` URL).
fn package_bytes(p: &Value) -> Result<Vec<u8>, String> {
    let path = crate::params::opt_str(p, "path")?.filter(|s| !s.trim().is_empty());
    let data = crate::params::opt_str(p, "data")?.filter(|s| !s.trim().is_empty());
    let cap = bicraft_pbix::pbiviz::MAX_PBIVIZ_BYTES;
    match (path, data) {
        (Some(_), Some(_)) => Err("give `path` or `data`, not both".into()),
        (None, None) => {
            Err("visual.import_custom needs `path` (a .pbiviz file) or `data` (its bytes in base64)".into())
        }
        (Some(path), None) => {
            let fp = std::path::Path::new(path);
            let len = std::fs::metadata(fp)
                .map_err(|e| format!("cannot read {}: {e}", fp.display()))?
                .len();
            if len > cap as u64 {
                return Err(format!(
                    "{} is {len} bytes; a .pbiviz is read up to {} MB",
                    fp.display(),
                    cap >> 20
                ));
            }
            std::fs::read(fp).map_err(|e| format!("cannot read {}: {e}", fp.display()))
        }
        (None, Some(d)) => {
            if d.len() > MAX_DATA_CHARS {
                return Err(format!(
                    "`data` is longer than {MAX_DATA_CHARS} characters (a .pbiviz up to {} MB)",
                    cap >> 20
                ));
            }
            let body = if d.get(..5).is_some_and(|h| h.eq_ignore_ascii_case("data:")) {
                d.split_once(',')
                    .map(|(_, b)| b)
                    .ok_or("a `data:` URL needs a comma")?
            } else {
                d
            };
            crate::images::base64(body.trim()).ok_or_else(|| "`data` is not valid base64".into())
        }
    }
}

fn packages_of(r: &Report) -> Result<Vec<Value>, String> {
    match r.extra.get("resourcePackages") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(a)) => Ok(a.clone()),
        Some(_) => {
            Err("report.json `resourcePackages` is not a list; the visual cannot be registered".into())
        }
    }
}

pub(crate) fn is_package_of(p: &Value, name: &str) -> bool {
    p.get("type").and_then(Value::as_str) == Some("CustomVisual")
        && p.get("name")
            .and_then(Value::as_str)
            .is_some_and(|n| n.eq_ignore_ascii_case(name))
}

/// Whether resource `path` is under `CustomVisuals/<name>/`.
pub(crate) fn in_folder(path: &str, name: &str) -> bool {
    let path = path.replace('\\', "/");
    let head = format!("{PREFIX}{name}/");
    path.get(..head.len())
        .is_some_and(|h| h.eq_ignore_ascii_case(&head))
}

/// `visual.import_custom {path | data}`: registers a `.pbiviz` package in the report.
pub(crate) fn visual_import_custom(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["path", "data"])?;
    let bytes = package_bytes(p)?;
    report_ref(s)?;
    if crate::convert::source_is_legacy(s) {
        return Err(
            "the report is a legacy layout (Report/Layout): custom visuals are imported into PBIR \
             reports only; convert it first (file.save_as with report_format: pbir)"
                .into(),
        );
    }
    let viz = bicraft_pbix::pbiviz::read_pbiviz(&bytes)?;
    let (_, r) = crate::report::parts(s)?;
    let mut list = packages_of(r)?;
    let guid = viz.guid.clone();
    let replaced =
        list.iter().any(|x| is_package_of(x, &guid)) || r.resources.iter().any(|x| in_folder(&x.path, &guid));
    list.retain(|x| !is_package_of(x, &guid));
    // the item is named as its file: Desktop 2.159's save writes an item's name as its path
    // (pbix-package.md, "Custom visuals")
    list.push(json!({
        "name": guid,
        "type": "CustomVisual",
        "items": [{"name": viz.metadata_item, "path": viz.metadata_item, "type": "CustomVisualMetadata"}],
    }));
    crate::rich::set_packages(r, list);
    r.resources.retain(|x| !in_folder(&x.path, &guid));
    let mut files = Vec::new();
    for (rel, data) in viz.files {
        let path = format!("{PREFIX}{guid}/{rel}");
        files.push(json!({"path": path, "bytes": data.len()}));
        r.resources.push(bicraft_report::Resource { path, bytes: data });
    }
    let mut warnings = viz.warnings;
    if replaced {
        warnings.push(format!(
            "custom visual `{guid}` was already in the report: replaced"
        ));
    }
    let users: Vec<Value> = used_by(r, &guid);
    s.report_dirty = true;
    Ok(json!({
        "name": guid,
        "display_name": viz.display_name,
        "version": viz.version,
        "api_version": viz.api_version,
        "author": viz.author,
        "description": viz.description,
        "has_icon": viz.has_icon,
        "has_javascript": viz.has_js,
        "metadata_item": viz.metadata_item,
        "files": files,
        "skipped": viz.skipped,
        "replaced": replaced,
        "used_by": users,
        "warnings": warnings,
    }))
}

/// `visual.remove_custom {name, force?}`: drops a custom visual from the report.
pub(crate) fn visual_remove_custom(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["name", "force"])?;
    let name = crate::params::str_param(p, "name")?.trim();
    if name.is_empty() || name.len() > 400 {
        return Err("`name` must be a custom visual's name (its guid)".into());
    }
    let force = crate::params::force(p)?;
    let r = report_ref(s)?;
    let packages = packages_of(r)?;
    let embedded = packages.iter().any(|x| is_package_of(x, name));
    let files = r.resources.iter().filter(|x| in_folder(&x.path, name)).count();
    let public = r
        .extra
        .get("publicCustomVisuals")
        .and_then(Value::as_array)
        .is_some_and(|a| {
            a.iter()
                .any(|v| v.as_str().is_some_and(|n| n.eq_ignore_ascii_case(name)))
        });
    if !embedded && files == 0 && !public {
        return Err(format!(
            "the report has no custom visual `{}` (report.custom_visuals lists them)",
            name.chars().take(80).collect::<String>()
        ));
    }
    let users = used_by(r, name);
    if !users.is_empty() && !force {
        return Err(format!(
            "{} visual(s) of the report use `{}`: remove them or change their type first, or pass \
             force: true to keep them (they are then drawn as placeholders)",
            users.len(),
            name.chars().take(80).collect::<String>()
        ));
    }
    if crate::convert::source_is_legacy(s) && embedded {
        return Err(
            "the report is a legacy layout (Report/Layout): its resource packages cannot be \
             rewritten; convert it to PBIR first"
                .into(),
        );
    }
    let (_, r) = crate::report::parts(s)?;
    if embedded {
        let mut list = packages_of(r)?;
        list.retain(|x| !is_package_of(x, name));
        crate::rich::set_packages(r, list);
    }
    r.resources.retain(|x| !in_folder(&x.path, name));
    if public && let Some(Value::Array(a)) = r.extra.get_mut("publicCustomVisuals") {
        a.retain(|v| !v.as_str().is_some_and(|n| n.eq_ignore_ascii_case(name)));
    }
    s.report_dirty = true;
    Ok(json!({
        "name": name,
        "removed_package": embedded,
        "removed_files": files,
        "removed_public_reference": public,
        "kept_visuals": users,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bicraft_report::{Page, Position, Resource, Visual};

    const GUID: &str = "bicraftHello7C3E9A41B2D54F60";

    fn visual(name: &str, ty: &str) -> Visual {
        Visual {
            name: name.into(),
            position: Position::default(),
            visual_type: Some(ty.into()),
            fields: vec![],
            parent_group: None,
            is_hidden: false,
            filter_config: None,
            visual_extra: Default::default(),
            extra: Default::default(),
        }
    }

    /// A report with our own minimal custom visual (built from the public .pbiviz format), an
    /// AppSource reference and a stray folder.
    fn report() -> Report {
        let mut r = Report::default();
        r.pages.push(Page {
            name: "p1".into(),
            display_name: "Page 1".into(),
            visuals: vec![
                visual("v1", GUID),
                visual("v2", "card"),
                visual("v3", "wordCloudXYZ"),
            ],
            ..Default::default()
        });
        let manifest = json!({
            "build": "6.1.0", "version": "1.0.0.0",
            "resources": [{"resourceId": "rId0", "sourceType": 5, "file": format!("resources/{GUID}.pbiviz.json")}],
            "visual": {"name": "bicraftHello", "displayName": "bicraft Hello", "guid": GUID, "version": "1.0.0.0"},
            "author": {"name": "bicraft"}
        });
        let pbiviz = json!({
            "visual": {"name": "bicraftHello", "displayName": "bicraft Hello", "guid": GUID, "version": "1.0.0.0",
                       "description": "test visual"},
            "apiVersion": "5.3.0",
            "author": {"name": "bicraft"},
            "capabilities": {"dataRoles": [{"name": "value", "kind": "Measure"}], "privileges": []},
            "content": {"js": "var x=1;", "css": ".a{}", "iconBase64": "data:image/png;base64,AA=="}
        });
        r.resources = vec![
            Resource {
                path: format!("CustomVisuals/{GUID}/package.json"),
                bytes: manifest.to_string().into_bytes(),
            },
            Resource {
                path: format!("CustomVisuals/{GUID}/resources/{GUID}.pbiviz.json"),
                bytes: pbiviz.to_string().into_bytes(),
            },
            Resource {
                path: "CustomVisuals/stray/package.json".into(),
                bytes: b"not json".to_vec(),
            },
            Resource {
                path: "StaticResources/RegisteredResources/x.png".into(),
                bytes: vec![1],
            },
        ];
        r.extra.insert(
            "resourcePackages".into(),
            json!([
                {"name": "RegisteredResources", "type": "RegisteredResources", "items": []},
                {"name": GUID, "type": "CustomVisual",
                 "items": [{"name": GUID, "path": format!("{GUID}.pbiviz.json"), "type": "CustomVisualMetadata"}]}
            ]),
        );
        r.extra
            .insert("publicCustomVisuals".into(), json!(["wordCloudXYZ"]));
        r
    }

    #[test]
    fn lists_embedded_public_and_stray_visuals() {
        let out = list(&report());
        assert_eq!(out["count"], 3);
        let e = &out["embedded"][0];
        assert_eq!(e["name"], GUID);
        assert_eq!(e["display_name"], "bicraft Hello");
        assert_eq!(e["version"], "1.0.0.0");
        assert_eq!(e["api_version"], "5.3.0");
        assert_eq!(e["author"], "bicraft");
        assert_eq!(e["data_roles"], json!(["value"]));
        assert_eq!(e["javascript_bytes"], 8);
        assert_eq!(e["has_icon"], true);
        assert_eq!(e["registered"], true);
        assert_eq!(
            e["metadata_file"],
            format!("CustomVisuals/{GUID}/resources/{GUID}.pbiviz.json")
        );
        assert_eq!(e["items"][0]["file"], e["metadata_file"]);
        assert_eq!(e["files"].as_array().unwrap().len(), 2);
        assert_eq!(
            e["used_by"],
            json!([{"page": "p1", "page_display_name": "Page 1", "visual": "v1"}])
        );
        assert_eq!(e["issues"], json!([]));
        let stray = &out["embedded"][1];
        assert_eq!(stray["name"], "stray");
        assert_eq!(stray["registered"], false);
        assert_eq!(stray["issues"].as_array().unwrap().len(), 2, "{stray}");
        assert_eq!(out["public"][0]["name"], "wordCloudXYZ");
        assert_eq!(out["public"][0]["used_by"][0]["visual"], "v3");
    }

    #[test]
    fn a_missing_metadata_file_is_an_issue_not_an_error() {
        let mut r = report();
        r.resources.retain(|x| !x.path.ends_with(".pbiviz.json"));
        let out = list(&r);
        let e = &out["embedded"][0];
        assert_eq!(e["metadata_file"], Value::Null);
        assert_eq!(e["display_name"], "bicraft Hello", "falls back to package.json");
        assert!(e["issues"][0].as_str().unwrap().contains("CustomVisualMetadata"));
        // Hostile shapes: wrong types everywhere still list.
        r.extra.insert(
            "resourcePackages".into(),
            json!([{"type": "CustomVisual", "items": "x"}, 3]),
        );
        r.extra.insert("publicCustomVisuals".into(), json!({"a": 1}));
        let out = list(&r);
        assert_eq!(out["public"], json!([]));
        assert!(out["embedded"].as_array().is_some());
    }

    /// Desktop resolves item paths under `resources/`; a path that already starts with it does
    /// not load (Desktop 2.158 left such a visual unregistered).
    #[test]
    fn item_paths_resolve_under_resources() {
        let mut r = report();
        r.extra.insert(
            "resourcePackages".into(),
            json!([{"name": GUID, "type": "CustomVisual",
                    "items": [{"name": GUID, "path": format!("resources/{GUID}.pbiviz.json"), "type": "CustomVisualMetadata"}]}]),
        );
        let e = &list(&r)["embedded"][0];
        assert_eq!(e["items"][0]["file"], Value::Null);
        assert!(e["issues"][0].as_str().unwrap().contains("does not load"), "{e}");
    }

    #[test]
    fn the_command_needs_a_report_and_no_parameters() {
        let mut s = Session::new();
        assert!(s.execute("report.custom_visuals", &json!({})).is_err());
        s.report = Some(report());
        let out = s.execute("report.custom_visuals", &json!({})).unwrap();
        assert_eq!(out["count"], 3);
        assert!(s.execute("report.custom_visuals", &json!({"x": 1})).is_err());
    }
}
