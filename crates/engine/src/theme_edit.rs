//! Report theme editing (M2.5 f): the `theme.*` commands.
//!
//! A report's look comes from `report.json` `themeCollection`: a `baseTheme` (one of Desktop's
//! monthly themes, embedded under `StaticResources/SharedResources/BaseThemes/`) and an optional
//! `customTheme` layered on top of it. Desktop stores a custom theme as a registered resource:
//!
//! - the file `StaticResources/RegisteredResources/<name><digits>.json`;
//! - an item `{name, path, type: "CustomTheme"}` in the `RegisteredResources` package of
//!   `report.json` `resourcePackages`;
//! - `themeCollection.customTheme = {name, reportVersionAtImport, type: "RegisteredResources"}`.
//!
//! The file name is the theme name (letters, digits, `-`, `_`) followed by 16 digits derived from
//! the file's bytes, so installing the same theme twice yields the same file. Replacing or
//! removing the custom theme drops its old file and package item unless something else still
//! registers them, so no resource is left dangling.
//!
//! Every command plans on a shared borrow and touches the report only once nothing can fail any
//! more; an edit that leaves the theme as it was changes nothing (not even the unsaved-changes
//! flag). Themes are validated against Microsoft's report theme JSON schema (the copy the visual
//! catalog vendors under `crates/visuals/schemas/`, MIT) and, for `visualStyles`, against the
//! visual catalog when it knows the visual type. The live viewer reads the result through
//! [`theme::resolve`].

use std::path::Path;
use std::sync::OnceLock;

use bicraft_pbir::JsonSchema;
use bicraft_report::{Report, Resource};
use bicraft_visuals::PropType;
use serde_json::{Map, Value, json};

use crate::params::{allow, force, opt_bool, opt_str, str_param};
use crate::report::{KNOWN_VISUAL_TYPES, MAX_VISUAL_TYPE, report_ref};
use crate::{Session, theme};

/// Largest theme document read, written or imported (Desktop's base themes are ~100 KB).
const MAX_THEME_BYTES: usize = 4 << 20;
/// Most data colors a theme may set.
const MAX_DATA_COLORS: usize = 200;
/// Longest object, property, state or font name accepted.
const MAX_NAME: usize = 200;
/// Schema problems quoted in an error message; warnings list at most `MAX_LISTED`.
const MAX_IN_MESSAGE: usize = 10;
const MAX_LISTED: usize = 100;
/// Longest theme-name prefix of a resource file name, in characters.
const MAX_SLUG: usize = 60;

const REGISTERED: &str = "RegisteredResources";
const SHARED: &str = "SharedResources";
const CUSTOM_ITEM: &str = "CustomTheme";
/// `name` of a custom theme started by an edit command.
const DEFAULT_THEME_NAME: &str = "Custom theme";

/// The report theme schema the visual catalog vendors (`crates/visuals/schemas/SOURCE.txt`).
const SCHEMA_TEXT: &str = bicraft_visuals::THEME_SCHEMA;
const SCHEMA_VERSION: &str = bicraft_visuals::THEME_SCHEMA_VERSION;

/// `reportVersionAtImport` (report schema 3.x) when the report records no `$schema` versions:
/// what the PBIR writer emits for new documents.
const DEFAULT_IMPORT_VERSIONS: [(&str, &str); 3] =
    [("visual", "2.12.0"), ("report", "3.3.0"), ("page", "2.1.0")];
/// `reportVersionAtImport` (report schema 1.x/2.x, a string) when no theme of the report has one.
const LEGACY_IMPORT_VERSION: &str = "5.61";

/// `theme.set_colors` parameters → theme keys (besides `data_colors` → `dataColors`).
const COLOR_KEYS: [(&str, &str); 9] = [
    ("background", "background"),
    ("foreground", "foreground"),
    ("table_accent", "tableAccent"),
    ("good", "good"),
    ("neutral", "neutral"),
    ("bad", "bad"),
    ("maximum", "maximum"),
    ("center", "center"),
    ("minimum", "minimum"),
];

/// Base theme keys copied into a custom theme an edit command starts.
const SEED_KEYS: [&str; 10] = [
    "dataColors",
    "background",
    "foreground",
    "tableAccent",
    "good",
    "neutral",
    "bad",
    "maximum",
    "center",
    "minimum",
];

/// The text classes of the report theme schema.
const TEXT_CLASSES: [&str; 14] = [
    "callout",
    "title",
    "header",
    "label",
    "largeTitle",
    "dataTitle",
    "boldLabel",
    "largeLabel",
    "largeLightLabel",
    "lightLabel",
    "semiboldLabel",
    "smallLabel",
    "smallLightLabel",
    "smallDataLabel",
];

/// `theme.set_text_classes` class properties → theme keys.
const TEXT_PROPS: [(&str, &str); 4] = [
    ("font_face", "fontFace"),
    ("font_size", "fontSize"),
    ("color", "color"),
    ("font_weight", "fontWeight"),
];

// ---------------------------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------------------------

fn schema() -> Option<&'static JsonSchema> {
    static SCHEMA: OnceLock<Option<JsonSchema>> = OnceLock::new();
    SCHEMA
        .get_or_init(|| {
            serde_json::from_str::<Value>(SCHEMA_TEXT)
                .ok()
                .map(JsonSchema::new)
        })
        .as_ref()
}

/// Problems of `doc` against the report theme schema, as `<pointer>: <message>`.
fn schema_issues(doc: &Value) -> Vec<String> {
    match schema() {
        None => vec!["the report theme schema could not be loaded".into()],
        Some(s) => s
            .validate(doc)
            .into_iter()
            .map(|(p, m)| if p.is_empty() { m } else { format!("{p}: {m}") })
            .collect(),
    }
}

/// Follows local `$ref`s (`#/definitions/x`) of the theme schema.
fn deref<'a>(root: &'a Value, mut v: &'a Value) -> &'a Value {
    for _ in 0..8 {
        let Some(name) = v
            .get("$ref")
            .and_then(Value::as_str)
            .and_then(|r| r.strip_prefix("#/definitions/"))
        else {
            break;
        };
        match root.get("definitions").and_then(|d| d.get(name)) {
            Some(target) => v = target,
            None => break,
        }
    }
    v
}

/// The schema of `object` in a visual style node (its `properties`, or those of its `allOf`).
fn schema_object<'a>(root: &'a Value, node: &'a Value, object: &str, depth: u32) -> Option<&'a Value> {
    if depth > 6 {
        return None;
    }
    let node = deref(root, node);
    if let Some(o) = node.get("properties").and_then(|p| p.get(object)) {
        return Some(deref(root, o));
    }
    node.get("allOf")
        .and_then(Value::as_array)?
        .iter()
        .find_map(|n| schema_object(root, n, object, depth + 1))
}

/// What the theme schema says about `visualStyles.<visual_type>.*.<object>`: `None` when it
/// does not describe the visual type; `Some(None)` when it describes the type but not the
/// object; `Some(Some(names))` with the object's property names.
fn schema_style_props(visual_type: &str, object: &str) -> Option<Option<Vec<String>>> {
    let root = schema()?.document()?;
    let vt = root
        .get("properties")?
        .get("visualStyles")?
        .get("properties")?
        .get(visual_type)?;
    let star = vt.get("properties").and_then(|p| p.get("*"))?;
    let Some(obj) = schema_object(root, star, object, 0) else {
        return Some(None);
    };
    let items = deref(root, obj.get("items").unwrap_or(&Value::Null));
    let names = items
        .get("properties")
        .and_then(Value::as_object)
        .map(|p| p.keys().cloned().collect())
        .unwrap_or_default();
    Some(Some(names))
}

fn schema_knows_visual(visual_type: &str) -> bool {
    schema()
        .and_then(JsonSchema::document)
        .and_then(|root| root.pointer("/properties/visualStyles/properties"))
        .and_then(|p| p.get(visual_type))
        .is_some()
}

// ---------------------------------------------------------------------------------------------
// Reading the report's themes
// ---------------------------------------------------------------------------------------------

fn norm(path: &str) -> String {
    path.replace('\\', "/")
        .trim_start_matches('/')
        .to_ascii_lowercase()
}

fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// `themeCollection.<key>`.
fn theme_entry<'a>(r: &'a Report, key: &str) -> Option<&'a Map<String, Value>> {
    r.theme_collection.as_ref()?.get(key)?.as_object()
}

fn packages(r: &Report) -> &[Value] {
    r.extra
        .get("resourcePackages")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn items(pkg: &Value) -> &[Value] {
    pkg.get("items")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn str_of<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

/// `StaticResources/<package>/<item path>`, normalized.
fn item_path(pkg: &Value, item: &Value) -> Option<String> {
    let pkg_name = str_of(pkg, "name")?;
    let path = str_of(item, "path")?;
    Some(norm(&format!("StaticResources/{pkg_name}/{path}")))
}

/// The resource holding the theme `name` of package type `package_type`: the `resourcePackages`
/// item of that name, else (as the viewer does) a file named `name` or `name.json`, preferring
/// one under that package's folder.
fn theme_resource(r: &Report, name: &str, package_type: &str) -> Option<usize> {
    for pkg in packages(r) {
        if str_of(pkg, "type") != Some(package_type) {
            continue;
        }
        for item in items(pkg) {
            if !str_of(item, "name").is_some_and(|n| n.eq_ignore_ascii_case(name)) {
                continue;
            }
            if let Some(want) = item_path(pkg, item)
                && let Some(i) = r.resources.iter().position(|res| norm(&res.path) == want)
            {
                return Some(i);
            }
        }
    }
    let wanted = name.to_ascii_lowercase();
    let with_json = format!("{wanted}.json");
    let matches = |res: &Resource| {
        let f = file_name(&res.path).to_ascii_lowercase();
        f == wanted || f == with_json
    };
    let folder = format!("staticresources/{}/", package_type.to_ascii_lowercase());
    r.resources
        .iter()
        .position(|res| matches(res) && norm(&res.path).starts_with(&folder))
        .or_else(|| r.resources.iter().position(matches))
}

/// A theme document: a JSON object of at most [`MAX_THEME_BYTES`], UTF-8 BOM tolerated.
fn parse_doc(bytes: &[u8]) -> Result<Map<String, Value>, String> {
    if bytes.len() > MAX_THEME_BYTES {
        return Err(format!(
            "the theme is {} bytes; the limit is {MAX_THEME_BYTES}",
            bytes.len()
        ));
    }
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    match serde_json::from_slice::<Value>(bytes) {
        Ok(Value::Object(m)) => Ok(m),
        Ok(_) => Err("a theme must be a JSON object".into()),
        Err(e) => Err(format!("the theme is not valid JSON: {e}")),
    }
}

/// The report's custom theme reference.
struct Custom {
    /// `themeCollection.customTheme.name` (the resource item name).
    name: String,
    package_type: String,
    /// The resource holding it, when found.
    resource: Option<usize>,
    /// Its parsed document, when found and readable.
    doc: Option<Map<String, Value>>,
}

fn custom(r: &Report) -> Option<Custom> {
    let e = theme_entry(r, "customTheme")?;
    let name = e.get("name").and_then(Value::as_str).filter(|n| !n.is_empty())?;
    let package_type = e
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or(REGISTERED)
        .to_string();
    let resource = theme_resource(r, name, &package_type);
    let doc = resource
        .and_then(|i| r.resources.get(i))
        .and_then(|res| parse_doc(&res.bytes).ok());
    Some(Custom {
        name: name.to_string(),
        package_type,
        resource,
        doc,
    })
}

/// The embedded base theme document, when the report carries it.
fn base_doc(r: &Report) -> Option<(usize, Map<String, Value>)> {
    let name = theme_entry(r, "baseTheme")?.get("name")?.as_str()?;
    let i = theme_resource(r, name, SHARED)?;
    let doc = parse_doc(&r.resources.get(i)?.bytes).ok()?;
    Some((i, doc))
}

/// Whether a `CustomTheme` item named `name` is registered.
fn has_custom_item(r: &Report, name: &str) -> bool {
    packages(r).iter().any(|pkg| {
        items(pkg).iter().any(|item| {
            str_of(item, "type") == Some(CUSTOM_ITEM)
                && str_of(item, "name").is_some_and(|n| n.eq_ignore_ascii_case(name))
        })
    })
}

// ---------------------------------------------------------------------------------------------
// Planning and applying
// ---------------------------------------------------------------------------------------------

/// The report changes of one theme command, computed before anything is touched.
struct Plan {
    theme_collection: Value,
    /// New `resourcePackages`; `None` removes the key.
    packages: Option<Vec<Value>>,
    /// Resource path to drop.
    remove: Option<String>,
    add: Option<Resource>,
}

fn apply_plan(r: &mut Report, plan: Plan) {
    r.theme_collection = Some(plan.theme_collection);
    match plan.packages {
        Some(p) if r.extra.contains_key("resourcePackages") => {
            r.extra.insert("resourcePackages".into(), Value::Array(p));
        }
        Some(p) => {
            // A new key goes where Desktop writes it: before these report.json properties.
            const AFTER: [&str; 5] = [
                "organizationCustomVisuals",
                "annotations",
                "dataSourceVariables",
                "settings",
                "slowDataSourceSettings",
            ];
            let old = std::mem::take(&mut r.extra);
            let mut value = Some(Value::Array(p));
            for (k, v) in old {
                if AFTER.contains(&k.as_str())
                    && let Some(p) = value.take()
                {
                    r.extra.insert("resourcePackages".into(), p);
                }
                r.extra.insert(k, v);
            }
            if let Some(p) = value {
                r.extra.insert("resourcePackages".into(), p);
            }
        }
        None => {
            r.extra.shift_remove("resourcePackages");
        }
    }
    if let Some(path) = plan.remove {
        let path = norm(&path);
        r.resources.retain(|res| norm(&res.path) != path);
    }
    if let Some(res) = plan.add {
        let path = norm(&res.path);
        match r.resources.iter_mut().find(|x| norm(&x.path) == path) {
            Some(x) => x.bytes = res.bytes,
            None => r.resources.push(res),
        }
    }
}

fn current_packages(r: &Report) -> Result<Vec<Value>, String> {
    match r.extra.get("resourcePackages") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(a)) => Ok(a.clone()),
        Some(_) => Err("report.json `resourcePackages` is not a list; the theme cannot be registered".into()),
    }
}

fn current_collection(r: &Report) -> Result<Map<String, Value>, String> {
    match &r.theme_collection {
        None | Some(Value::Null) => Ok(Map::new()),
        Some(Value::Object(m)) => Ok(m.clone()),
        Some(_) => Err("report.json `themeCollection` is not an object".into()),
    }
}

/// Removes the `CustomTheme` items named `name` from the registered packages; returns whether a
/// package was emptied by it (those packages are dropped when `drop_empty`).
fn drop_custom_items(packages: &mut Vec<Value>, name: &str, drop_empty: bool) {
    let mut emptied = Vec::new();
    for (i, pkg) in packages.iter_mut().enumerate() {
        let Some(list) = pkg.get_mut("items").and_then(Value::as_array_mut) else {
            continue;
        };
        let before = list.len();
        list.retain(|item| {
            !(str_of(item, "type") == Some(CUSTOM_ITEM)
                && str_of(item, "name").is_some_and(|n| n.eq_ignore_ascii_case(name)))
        });
        if list.is_empty() && before > 0 {
            emptied.push(i);
        }
    }
    if drop_empty {
        for i in emptied.into_iter().rev() {
            if i < packages.len() {
                packages.remove(i);
            }
        }
    }
}

/// Whether any package item still registers `path` (normalized).
fn registered(packages: &[Value], path: &str) -> bool {
    packages.iter().any(|pkg| {
        items(pkg)
            .iter()
            .any(|item| item_path(pkg, item).as_deref() == Some(path))
    })
}

/// The old custom theme file to drop once its reference is gone: only a registered resource
/// that nothing else registers.
fn stale_resource(r: &Report, c: &Custom, packages: &[Value], keep: Option<&str>) -> Option<String> {
    if c.package_type != REGISTERED {
        return None;
    }
    let path = &r.resources.get(c.resource?)?.path;
    let n = norm(path);
    if keep.is_some_and(|k| norm(k) == n) || registered(packages, &n) {
        return None;
    }
    Some(path.clone())
}

/// `x.y.z` of the `$schema` URL the report recorded for document kind `kind`.
fn schema_version(r: &Report, kind: &str) -> Option<String> {
    let url = r.schemas.get(kind)?.as_str()?;
    let mut parts = url.rsplit('/');
    parts.next()?;
    let v = parts.next()?;
    let ok = v.split('.').count() == 3
        && v.split('.')
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    ok.then(|| v.to_string())
}

/// `reportVersionAtImport` for a theme imported now: the report, page and visual schema versions
/// (report schema 3.x), or the version string of report schema 1.x/2.x.
fn import_version(r: &Report) -> Value {
    let major =
        schema_version(r, "report").and_then(|v| v.split('.').next().and_then(|m| m.parse::<u64>().ok()));
    if major.is_some_and(|m| m < 3) {
        let existing = ["customTheme", "baseTheme"].iter().find_map(|k| {
            theme_entry(r, k)
                .and_then(|e| e.get("reportVersionAtImport"))
                .and_then(Value::as_str)
                .map(str::to_string)
        });
        return json!(existing.unwrap_or_else(|| LEGACY_IMPORT_VERSION.to_string()));
    }
    let mut m = Map::new();
    for (kind, default) in DEFAULT_IMPORT_VERSIONS {
        let v = schema_version(r, kind).unwrap_or_else(|| default.to_string());
        m.insert(kind.into(), json!(v));
    }
    Value::Object(m)
}

/// 64-bit FNV-1a.
fn fnv64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// `<theme name without other characters than letters, digits, - and _><16 digits>.json`.
fn resource_file(theme_name: &str, bytes: &[u8]) -> String {
    let slug: String = theme_name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(MAX_SLUG)
        .collect();
    let slug = if slug.is_empty() {
        "CustomTheme".to_string()
    } else {
        slug
    };
    let digits = fnv64(bytes) % 10_000_000_000_000_000;
    format!("{slug}{digits:016}.json")
}

fn plan_install(r: &Report, file: &str, bytes: Vec<u8>) -> Result<Plan, String> {
    let mut packages = current_packages(r)?;
    let mut tc = current_collection(r)?;
    let current = custom(r);
    if let Some(c) = &current {
        drop_custom_items(&mut packages, &c.name, false);
    }
    // The registered package (created when missing).
    let idx = match packages
        .iter()
        .position(|p| str_of(p, "type") == Some(REGISTERED) && str_of(p, "name").is_some())
    {
        Some(i) => i,
        None => {
            packages.push(json!({"name": REGISTERED, "type": REGISTERED, "items": []}));
            packages.len().saturating_sub(1)
        }
    };
    let pkg = packages
        .get_mut(idx)
        .ok_or("internal error: missing resource package")?;
    let pkg_name = str_of(pkg, "name").unwrap_or(REGISTERED).to_string();
    let item = json!({"name": file, "path": file, "type": CUSTOM_ITEM});
    match pkg.get_mut("items") {
        Some(Value::Array(list)) => list.push(item),
        Some(_) => return Err(format!("resource package `{pkg_name}` has no item list")),
        None => {
            if let Some(o) = pkg.as_object_mut() {
                o.insert("items".into(), json!([item]));
            }
        }
    }
    let path = format!("StaticResources/{pkg_name}/{file}");
    let remove = current
        .as_ref()
        .and_then(|c| stale_resource(r, c, &packages, Some(&path)));
    tc.insert(
        "customTheme".into(),
        json!({"name": file, "reportVersionAtImport": import_version(r), "type": REGISTERED}),
    );
    Ok(Plan {
        theme_collection: Value::Object(tc),
        packages: Some(packages),
        remove,
        add: Some(Resource { path, bytes }),
    })
}

fn plan_remove(r: &Report, c: &Custom) -> Result<Plan, String> {
    let mut packages = current_packages(r)?;
    let mut tc = current_collection(r)?;
    drop_custom_items(&mut packages, &c.name, true);
    let remove = stale_resource(r, c, &packages, None);
    tc.shift_remove("customTheme");
    let had_packages = r.extra.contains_key("resourcePackages");
    Ok(Plan {
        theme_collection: Value::Object(tc),
        packages: (had_packages && !packages.is_empty()).then_some(packages),
        remove,
        add: None,
    })
}

fn listed(items: &[String], max: usize) -> String {
    let mut text = items.iter().take(max).cloned().collect::<Vec<_>>().join("; ");
    if items.len() > max {
        text.push_str(&format!("; and {} more", items.len() - max));
    }
    text
}

/// The theme's `name`: required, a non-empty string.
fn check_theme_name(doc: &Map<String, Value>) -> Result<&str, String> {
    match doc.get("name") {
        Some(Value::String(s)) if !s.trim().is_empty() => Ok(s),
        Some(Value::String(_)) => Err("the theme's `name` is empty".into()),
        Some(_) => Err("the theme's `name` must be a string".into()),
        None => Err("the theme has no `name` (Desktop lists custom themes by name)".into()),
    }
}

/// Installs `doc` as the report's custom theme. Schema problems not in `baseline` (those the
/// theme already had) fail unless `force`, which turns them into warnings.
fn install(
    s: &mut Session,
    doc: Map<String, Value>,
    force: bool,
    baseline: &[String],
    mut warnings: Vec<String>,
) -> Result<Map<String, Value>, String> {
    let theme_name = check_theme_name(&doc)?.to_string();
    let doc = Value::Object(doc);
    let issues: Vec<String> = schema_issues(&doc)
        .into_iter()
        .filter(|i| !baseline.contains(i))
        .collect();
    if !issues.is_empty() {
        if !force {
            return Err(format!(
                "the theme does not match the report theme schema {SCHEMA_VERSION} (force: true installs it anyway): {}",
                listed(&issues, MAX_IN_MESSAGE)
            ));
        }
        warnings.extend(
            issues
                .into_iter()
                .take(MAX_LISTED)
                .map(|i| format!("theme schema: {i}")),
        );
    }
    let bytes = serde_json::to_vec_pretty(&doc).map_err(|e| format!("cannot serialize the theme: {e}"))?;
    if bytes.len() > MAX_THEME_BYTES {
        return Err(format!(
            "the theme is {} bytes; the limit is {MAX_THEME_BYTES}",
            bytes.len()
        ));
    }
    let r = report_ref(s)?;
    let current = custom(r);
    let unchanged = current.as_ref().is_some_and(|c| {
        c.doc.as_ref().is_some_and(|d| doc.as_object() == Some(d)) && has_custom_item(r, &c.name)
    });
    let mut out = Map::new();
    if unchanged {
        let c = current.as_ref();
        out.insert("changed".into(), json!(false));
        out.insert("name".into(), json!(c.map(|c| c.name.clone())));
        out.insert(
            "resource".into(),
            json!(
                c.and_then(|c| c.resource)
                    .and_then(|i| r.resources.get(i))
                    .map(|x| x.path.clone())
            ),
        );
    } else {
        let file = resource_file(&theme_name, &bytes);
        let plan = plan_install(r, &file, bytes)?;
        let path = plan.add.as_ref().map(|a| a.path.clone());
        out.insert("changed".into(), json!(true));
        out.insert("name".into(), json!(file));
        out.insert("resource".into(), json!(path));
        if let Some(old) = &plan.remove {
            out.insert("removed_resource".into(), json!(old));
        }
        if let Some(rep) = s.report.as_mut() {
            apply_plan(rep, plan);
        }
        s.report_dirty = true;
    }
    out.insert("theme_name".into(), json!(theme_name));
    let resolved = s.report.as_ref().and_then(theme::resolve);
    out.insert("resolved".into(), theme::theme_json(resolved.as_ref()));
    out.insert("warnings".into(), json!(warnings));
    Ok(out)
}

/// A new custom theme: a name, and the essentials of the embedded base theme when the report
/// carries it (the base theme still supplies everything else).
fn seed(r: &Report) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("name".into(), json!(DEFAULT_THEME_NAME));
    if let Some((_, base)) = base_doc(r) {
        for key in SEED_KEYS {
            if let Some(v) = base.get(key) {
                m.insert(key.into(), v.clone());
            }
        }
    }
    m
}

/// Runs an edit on the custom theme (started from [`seed`] when the report has none) and
/// installs the result.
fn edit(
    s: &mut Session,
    force: bool,
    f: impl FnOnce(&mut Map<String, Value>, &mut Vec<String>) -> Result<(), String>,
) -> Result<Value, String> {
    let r = report_ref(s)?;
    let current = custom(r);
    let mut warnings = Vec::new();
    let existing = current.as_ref().and_then(|c| c.doc.clone());
    if let Some(c) = &current
        && c.doc.is_none()
    {
        warnings.push(format!(
            "the custom theme `{}` could not be read; a new one is started from the base theme",
            c.name
        ));
    }
    let created = existing.is_none();
    let start = existing.unwrap_or_else(|| seed(r));
    let baseline = if created {
        Vec::new()
    } else {
        schema_issues(&Value::Object(start.clone()))
    };
    let mut doc = start.clone();
    f(&mut doc, &mut warnings)?;
    if created && doc == start {
        // Nothing to set and no custom theme: do not create one.
        let resolved = theme::resolve(r);
        return Ok(json!({
            "changed": false,
            "created": false,
            "resolved": theme::theme_json(resolved.as_ref()),
            "warnings": warnings,
        }));
    }
    let mut out = install(s, doc, force, &baseline, warnings)?;
    let changed = out.get("changed").and_then(Value::as_bool).unwrap_or(false);
    out.insert("created".into(), json!(created && changed));
    Ok(Value::Object(out))
}

// ---------------------------------------------------------------------------------------------
// Parameter checks
// ---------------------------------------------------------------------------------------------

/// `#RGB`, `#RRGGBB` or `#RRGGBBAA` (the theme schema's color pattern).
fn is_color(s: &str) -> bool {
    s.strip_prefix('#')
        .is_some_and(|h| matches!(h.len(), 3 | 6 | 8) && h.bytes().all(|b| b.is_ascii_hexdigit()))
}

const COLOR_HINT: &str = "a color such as \"#118DFF\" (#RGB, #RRGGBB or #RRGGBBAA)";

/// A color parameter: `null` → `None` (remove), a color → `Some`.
fn color_param(v: &Value, what: &str) -> Result<Option<Value>, String> {
    match v {
        Value::Null => Ok(None),
        Value::String(s) if is_color(s) => Ok(Some(json!(s))),
        _ => Err(format!("`{what}` must be {COLOR_HINT}, or null")),
    }
}

fn check_name_text(s: &str, what: &str) -> Result<(), String> {
    if s.is_empty() || s.chars().count() > MAX_NAME || s.chars().any(char::is_control) {
        return Err(format!(
            "`{what}` must be a non-empty name of at most {MAX_NAME} characters"
        ));
    }
    Ok(())
}

/// Sets (`Some`) or removes (`None`) `key`, keeping the key order.
fn put(m: &mut Map<String, Value>, key: &str, v: Option<Value>) {
    match v {
        Some(v) => {
            m.insert(key.into(), v);
        }
        None => {
            m.shift_remove(key);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Visual styles
// ---------------------------------------------------------------------------------------------

/// A `theme.set_visual_style` value: `null` (remove), a boolean, number or string, `{color}`
/// (→ `{"solid": {"color": …}}`), or any theme JSON value (checked by the schema).
fn style_value(v: &Value) -> Result<Option<Value>, String> {
    match v {
        Value::Null => Ok(None),
        Value::Object(o) if o.len() == 1 && o.contains_key("color") => match o.get("color") {
            Some(Value::String(c)) if is_color(c) => Ok(Some(json!({"solid": {"color": c}}))),
            _ => Err(format!("`value.color` must be {COLOR_HINT}")),
        },
        other => Ok(Some(other.clone())),
    }
}

fn num_text(x: f64) -> String {
    if x.fract() == 0.0 && x.abs() < 9.0e15 {
        format!("{}", x as i64)
    } else {
        format!("{x}")
    }
}

/// A theme value in the PBIR form the visual catalog checks (`{"expr": {"Literal": …}}`).
fn to_pbir(v: &Value, ty: &PropType) -> Option<Value> {
    let lit = |text: String| json!({"expr": {"Literal": {"Value": text}}});
    match v {
        Value::Bool(b) => Some(lit(b.to_string())),
        Value::Number(n) => {
            let x = n.as_f64().filter(|x| x.is_finite())?;
            let suffix = if matches!(ty, PropType::Number { integer: true, .. }) {
                "L"
            } else {
                "D"
            };
            Some(lit(format!("{}{suffix}", num_text(x))))
        }
        Value::String(s) => Some(lit(format!("'{}'", s.replace('\'', "''")))),
        Value::Object(o) => {
            let c = o.get("solid")?.get("color")?.as_str()?;
            Some(json!({"solid": {"color": lit(format!("'{}'", c.replace('\'', "''")))}}))
        }
        _ => None,
    }
}

/// Checks a visual style against the catalog (when it knows the visual type) and the theme
/// schema. Errors are returned; `warnings` get the softer findings.
fn check_style(
    visual_type: &str,
    object: &str,
    property: &str,
    value: Option<&Value>,
    warnings: &mut Vec<String>,
) -> Result<(), String> {
    if visual_type == "*" {
        return Ok(());
    }
    let catalog = bicraft_visuals::visual(visual_type);
    let in_schema = schema_knows_visual(visual_type);
    if catalog.is_none() && !in_schema && !KNOWN_VISUAL_TYPES.contains(&visual_type) {
        warnings.push(format!(
            "`{visual_type}` is not a built-in visual type (a custom visual?); its style is kept as given"
        ));
    }
    if let Some(vt) = catalog {
        let Some(obj) = vt.objects.iter().find(|o| o.name == object) else {
            let names: Vec<&str> = vt.objects.iter().map(|o| o.name.as_str()).take(60).collect();
            return Err(format!(
                "`{visual_type}` has no formatting object `{object}` (objects: {})",
                names.join(", ")
            ));
        };
        let Some(prop) = obj.properties.iter().find(|p| p.name == property) else {
            let names: Vec<&str> = obj.properties.iter().map(|p| p.name.as_str()).take(60).collect();
            return Err(format!(
                "`{visual_type}.{object}` has no property `{property}` (properties: {})",
                names.join(", ")
            ));
        };
        if let Some(pbir) = value.and_then(|v| to_pbir(v, &prop.ty)) {
            bicraft_visuals::check_value(prop, &pbir)
                .map_err(|e| format!("`{visual_type}.{object}.{property}`: {e}"))?;
        }
    } else {
        match schema_style_props(visual_type, object) {
            Some(None) => warnings.push(format!(
                "`{object}` is not a formatting object of `{visual_type}` in the report theme schema {SCHEMA_VERSION}"
            )),
            Some(Some(names)) if !names.iter().any(|n| n == property) => warnings.push(format!(
                "`{property}` is not a property of `{visual_type}.{object}` in the report theme schema {SCHEMA_VERSION}"
            )),
            _ => {}
        }
    }
    Ok(())
}

fn object_mut<'a>(
    parent: &'a mut Map<String, Value>,
    key: &str,
    what: &str,
) -> Result<&'a mut Map<String, Value>, String> {
    parent
        .entry(key.to_string())
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| format!("the theme's `{what}` is not an object"))
}

/// Whether a `visualStyles` entry is the one for `state` (`None`: the entry without `$id`).
fn entry_is(entry: &Value, state: Option<&str>) -> bool {
    entry.get("$id").and_then(Value::as_str) == state
}

/// Sets `visualStyles.<vt>.*.<object>[state].<property>` in `doc`.
fn set_style(
    doc: &mut Map<String, Value>,
    vt: &str,
    object: &str,
    property: &str,
    state: Option<&str>,
    value: Value,
) -> Result<(), String> {
    let styles = object_mut(doc, "visualStyles", "visualStyles")?;
    let by_type = object_mut(styles, vt, &format!("visualStyles.{vt}"))?;
    let star = object_mut(by_type, "*", &format!("visualStyles.{vt}.*"))?;
    let list = star
        .entry(object.to_string())
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or_else(|| format!("the theme's `visualStyles.{vt}.*.{object}` is not a list"))?;
    let idx = match list.iter().position(|e| entry_is(e, state)) {
        Some(i) => i,
        None => {
            match state {
                Some(st) => list.push(json!({"$id": st})),
                // the viewer and Desktop read the stateless entry first
                None => list.insert(0, json!({})),
            }
            if state.is_some() {
                list.len().saturating_sub(1)
            } else {
                0
            }
        }
    };
    let entry = list
        .get_mut(idx)
        .and_then(Value::as_object_mut)
        .ok_or_else(|| format!("the theme's `visualStyles.{vt}.*.{object}` holds a non-object entry"))?;
    entry.insert(property.to_string(), value);
    Ok(())
}

/// Removes `visualStyles.<vt>.*.<object>[state].<property>` and the containers it empties.
fn remove_style(doc: &mut Map<String, Value>, vt: &str, object: &str, property: &str, state: Option<&str>) {
    let Some(styles) = doc.get_mut("visualStyles").and_then(Value::as_object_mut) else {
        return;
    };
    let Some(by_type) = styles.get_mut(vt).and_then(Value::as_object_mut) else {
        return;
    };
    let Some(star) = by_type.get_mut("*").and_then(Value::as_object_mut) else {
        return;
    };
    let Some(list) = star.get_mut(object).and_then(Value::as_array_mut) else {
        return;
    };
    let Some(idx) = list.iter().position(|e| entry_is(e, state)) else {
        return;
    };
    let Some(entry) = list.get_mut(idx).and_then(Value::as_object_mut) else {
        return;
    };
    if entry.shift_remove(property).is_none() {
        return;
    }
    if entry.keys().all(|k| k == "$id") {
        list.remove(idx);
    }
    if list.is_empty() {
        star.shift_remove(object);
    }
    if star.is_empty() {
        by_type.shift_remove("*");
    }
    if by_type.is_empty() {
        styles.shift_remove(vt);
    }
    if styles.is_empty() {
        doc.shift_remove("visualStyles");
    }
}

// ---------------------------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------------------------

pub(crate) fn theme_get(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &[])?;
    let r = report_ref(s)?;
    let custom_json = custom(r).map(|c| {
        let entry = theme_entry(r, "customTheme");
        json!({
            "name": c.name,
            "type": c.package_type,
            "report_version_at_import": entry.and_then(|e| e.get("reportVersionAtImport")),
            "resource": c.resource.and_then(|i| r.resources.get(i)).map(|x| x.path.clone()),
            "theme_name": c.doc.as_ref().and_then(|d| d.get("name")),
            "theme": c.doc.map(Value::Object),
            "issues": c.resource.and_then(|i| r.resources.get(i)).map(|res| match parse_doc(&res.bytes) {
                Ok(d) => schema_issues(&Value::Object(d)).into_iter().take(MAX_LISTED).collect::<Vec<_>>(),
                Err(e) => vec![e],
            }).unwrap_or_else(|| vec!["the theme file is missing".to_string()]),
        })
    });
    let base_json = theme_entry(r, "baseTheme").map(|e| {
        let embedded = base_doc(r);
        json!({
            "name": e.get("name"),
            "type": e.get("type"),
            "report_version_at_import": e.get("reportVersionAtImport"),
            "resource": embedded.as_ref().and_then(|(i, _)| r.resources.get(*i)).map(|x| x.path.clone()),
        })
    });
    let resolved = theme::resolve(r);
    Ok(json!({
        "custom": custom_json,
        "base": base_json,
        "resolved": theme::theme_json(resolved.as_ref()),
        "builtin": bicraft_visuals::BUILTIN_THEMES.iter().map(|t| t.id).collect::<Vec<_>>(),
        "schema_version": SCHEMA_VERSION,
    }))
}

pub(crate) fn theme_set(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["theme", "builtin", "force"])?;
    let force = force(p)?;
    let doc = match (p.get("theme"), p.get("builtin")) {
        (Some(_), Some(_)) => return Err("pass `theme` or `builtin`, not both".into()),
        (None, Some(b)) => {
            let id = b.as_str().ok_or("`builtin` is a built-in theme id (a string)")?;
            let t = bicraft_visuals::builtin_theme(id).ok_or_else(|| {
                let ids: Vec<&str> = bicraft_visuals::BUILTIN_THEMES.iter().map(|t| t.id).collect();
                format!("no built-in theme `{id}` (built-in themes: {})", ids.join(", "))
            })?;
            parse_doc(t.json.as_bytes())?
        }
        (Some(Value::Object(o)), None) => o.clone(),
        (Some(Value::String(text)), None) => parse_doc(text.as_bytes())?,
        _ => {
            return Err(
                "missing parameter `theme`: the theme JSON object (or `builtin`: a built-in theme id)".into(),
            );
        }
    };
    report_ref(s)?;
    install(s, doc, force, &[], Vec::new()).map(Value::Object)
}

pub(crate) fn theme_import(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["path", "force"])?;
    let force = force(p)?;
    let path = Path::new(str_param(p, "path")?);
    report_ref(s)?;
    let len = std::fs::metadata(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?
        .len();
    if len > MAX_THEME_BYTES as u64 {
        return Err(format!(
            "{} is {len} bytes; the theme limit is {MAX_THEME_BYTES}",
            path.display()
        ));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let doc = parse_doc(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    install(s, doc, force, &[], Vec::new()).map(Value::Object)
}

pub(crate) fn theme_export(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["path", "overwrite"])?;
    let path = Path::new(str_param(p, "path")?);
    let overwrite = opt_bool(p, "overwrite")?.unwrap_or(false);
    let r = report_ref(s)?;
    let (source, res) = match custom(r).and_then(|c| c.resource) {
        Some(i) => ("custom", r.resources.get(i)),
        None => {
            match base_doc(r) {
                Some((i, _)) => ("base", r.resources.get(i)),
                None => {
                    return Err("the report has no custom theme, and its base theme is not embedded: nothing to export".into());
                }
            }
        }
    };
    let res = res.ok_or("internal error: missing theme resource")?;
    if path.exists() && !overwrite {
        return Err(format!(
            "{} exists (pass overwrite: true to replace it)",
            path.display()
        ));
    }
    std::fs::write(path, &res.bytes).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(json!({
        "path": path.display().to_string(),
        "source": source,
        "resource": res.path,
        "bytes": res.bytes.len(),
    }))
}

pub(crate) fn theme_remove(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &[])?;
    let r = report_ref(s)?;
    let Some(c) = custom(r) else {
        return Ok(json!({"changed": false, "removed_resource": null}));
    };
    let plan = plan_remove(r, &c)?;
    let removed = plan.remove.clone();
    if let Some(rep) = s.report.as_mut() {
        apply_plan(rep, plan);
    }
    s.report_dirty = true;
    let resolved = s.report.as_ref().and_then(theme::resolve);
    Ok(json!({
        "changed": true,
        "name": c.name,
        "removed_resource": removed,
        "resolved": theme::theme_json(resolved.as_ref()),
    }))
}

/// The colours of `theme.set_colors` (and `theme.customize`) in `p`: (theme key, `None` =
/// remove | the value). Empty when `p` gives none.
fn parse_colors(p: &Value) -> Result<Vec<(&'static str, Option<Value>)>, String> {
    let mut sets: Vec<(&'static str, Option<Value>)> = Vec::new();
    if let Some(v) = p.get("data_colors") {
        let colors = match v {
            Value::Null => None,
            Value::Array(a) if !a.is_empty() && a.len() <= MAX_DATA_COLORS => {
                let mut out = Vec::with_capacity(a.len());
                for (i, c) in a.iter().enumerate() {
                    match c {
                        Value::String(t) if is_color(t) => out.push(json!(t)),
                        _ => return Err(format!("`data_colors[{i}]` must be {COLOR_HINT}")),
                    }
                }
                Some(Value::Array(out))
            }
            _ => {
                return Err(format!(
                    "`data_colors` must be a list of 1 to {MAX_DATA_COLORS} colors, or null"
                ));
            }
        };
        sets.push(("dataColors", colors));
    }
    for (param, key) in COLOR_KEYS {
        if let Some(v) = p.get(param) {
            sets.push((key, color_param(v, param)?));
        }
    }
    Ok(sets)
}

pub(crate) fn theme_set_colors(s: &mut Session, p: &Value) -> Result<Value, String> {
    let mut allowed: Vec<&str> = vec!["data_colors", "force"];
    allowed.extend(COLOR_KEYS.iter().map(|(k, _)| *k));
    allow(p, &allowed)?;
    let force = force(p)?;
    let sets = parse_colors(p)?;
    if sets.is_empty() {
        return Err(
            "give at least one color: data_colors, background, foreground, table_accent, good, \
                    neutral, bad, maximum, center or minimum"
                .into(),
        );
    }
    edit(s, force, |doc, _| {
        for (key, v) in sets {
            put(doc, key, v);
        }
        Ok(())
    })
}

/// A text class edit: (class, `None` = remove the class | its properties: (theme key, `None` =
/// remove | the value)).
type ClassEdit = (&'static str, Option<Vec<(&'static str, Option<Value>)>>);

/// The text classes of `theme.set_text_classes` (and `theme.customize`'s `text_classes`) in `p`.
fn parse_text_classes(p: &Value) -> Result<Vec<ClassEdit>, String> {
    let mut edits: Vec<ClassEdit> = Vec::new();
    for class in TEXT_CLASSES {
        let Some(v) = p.get(class) else { continue };
        let props = match v {
            Value::Null => None,
            Value::Object(o) => {
                let names: Vec<&str> = TEXT_PROPS.iter().map(|(k, _)| *k).collect();
                allow(v, &names).map_err(|e| format!("`{class}`: {e}"))?;
                let mut props = Vec::new();
                for (param, key) in TEXT_PROPS {
                    let Some(x) = o.get(param) else { continue };
                    let value = match (param, x) {
                        (_, Value::Null) => None,
                        ("color", _) => color_param(x, &format!("{class}.color"))?,
                        ("font_size", Value::Number(n)) => {
                            let size = n.as_f64().filter(|f| f.is_finite() && *f > 0.0 && *f <= 1000.0);
                            if size.is_none() {
                                return Err(format!(
                                    "`{class}.font_size` must be a positive number of points"
                                ));
                            }
                            Some(x.clone())
                        }
                        ("font_size", _) => {
                            return Err(format!("`{class}.font_size` must be a number of points"));
                        }
                        (_, Value::String(t)) => {
                            check_name_text(t, &format!("{class}.{param}"))?;
                            Some(x.clone())
                        }
                        _ => return Err(format!("`{class}.{param}` must be a string, or null")),
                    };
                    props.push((key, value));
                }
                Some(props)
            }
            _ => {
                return Err(format!(
                    "`{class}` must be {{font_face?, font_size?, color?, font_weight?}} or null"
                ));
            }
        };
        edits.push((class, props));
    }
    Ok(edits)
}

/// Applies text class edits to a theme document.
fn apply_text_classes(doc: &mut Map<String, Value>, edits: Vec<ClassEdit>) -> Result<(), String> {
    for (class, props) in edits {
        match props {
            None => {
                if let Some(tc) = doc.get_mut("textClasses").and_then(Value::as_object_mut) {
                    tc.shift_remove(class);
                }
            }
            Some(props) => {
                let tc = object_mut(doc, "textClasses", "textClasses")?;
                let entry = object_mut(tc, class, &format!("textClasses.{class}"))?;
                for (key, v) in props {
                    put(entry, key, v);
                }
                if entry.is_empty() {
                    tc.shift_remove(class);
                }
            }
        }
    }
    if doc
        .get("textClasses")
        .and_then(Value::as_object)
        .is_some_and(Map::is_empty)
    {
        doc.shift_remove("textClasses");
    }
    Ok(())
}

pub(crate) fn theme_set_text_classes(s: &mut Session, p: &Value) -> Result<Value, String> {
    let mut allowed: Vec<&str> = TEXT_CLASSES.to_vec();
    allowed.push("force");
    allow(p, &allowed)?;
    let force = force(p)?;
    let edits = parse_text_classes(p)?;
    if edits.is_empty() {
        return Err(format!(
            "give at least one text class: {}",
            TEXT_CLASSES.join(", ")
        ));
    }
    edit(s, force, |doc, _| apply_text_classes(doc, edits))
}

/// The parameters of one visual style (`theme.set_visual_style`, a `theme.customize` style).
const STYLE_PARAMS: [&str; 5] = ["visual_type", "object", "property", "value", "state"];
/// Most styles one `theme.customize` sets.
const MAX_STYLES: usize = 256;

/// One visual style to set (`value` `Some`) or remove.
struct StyleEdit {
    visual_type: String,
    object: String,
    property: String,
    state: Option<String>,
    value: Option<Value>,
}

/// Reads and checks one visual style; a catalog mismatch fails unless `force` (then it is a
/// warning).
fn parse_style(p: &Value, force: bool, warnings: &mut Vec<String>) -> Result<StyleEdit, String> {
    let vt = str_param(p, "visual_type")?;
    if vt != "*" && (vt.is_empty() || vt.len() > MAX_VISUAL_TYPE || vt.chars().any(char::is_control)) {
        return Err(format!(
            "`visual_type` must be \"*\" (every visual) or a visual type name of at most {MAX_VISUAL_TYPE} characters"
        ));
    }
    let object = str_param(p, "object")?;
    check_name_text(object, "object")?;
    let property = str_param(p, "property")?;
    check_name_text(property, "property")?;
    let state = opt_str(p, "state")?;
    if let Some(st) = state {
        check_name_text(st, "state")?;
    }
    let value = match p.get("value") {
        Some(v) => style_value(v)?,
        None => return Err("missing parameter `value` (null removes the style)".into()),
    };
    if let Err(e) = check_style(vt, object, property, value.as_ref(), warnings) {
        if !force {
            return Err(format!("{e} (force: true writes it anyway)"));
        }
        warnings.push(e);
    }
    Ok(StyleEdit {
        visual_type: vt.to_string(),
        object: object.to_string(),
        property: property.to_string(),
        state: state.map(str::to_string),
        value,
    })
}

fn apply_style(doc: &mut Map<String, Value>, e: StyleEdit) -> Result<(), String> {
    match e.value {
        Some(v) => set_style(doc, &e.visual_type, &e.object, &e.property, e.state.as_deref(), v),
        None => {
            remove_style(doc, &e.visual_type, &e.object, &e.property, e.state.as_deref());
            Ok(())
        }
    }
}

pub(crate) fn theme_set_visual_style(s: &mut Session, p: &Value) -> Result<Value, String> {
    let mut allowed = STYLE_PARAMS.to_vec();
    allowed.push("force");
    allow(p, &allowed)?;
    let force = force(p)?;
    let mut warnings = Vec::new();
    let style = parse_style(p, force, &mut warnings)?;
    edit(s, force, move |doc, w| {
        w.extend(warnings);
        apply_style(doc, style)
    })
}

/// Customize current theme: the theme's name, colours, text classes and visual styles in one
/// edit (one undo step), each part checked as `theme.set_colors`, `theme.set_text_classes` and
/// `theme.set_visual_style` check theirs.
pub(crate) fn theme_customize(s: &mut Session, p: &Value) -> Result<Value, String> {
    let mut allowed: Vec<&str> = vec!["name", "data_colors", "text_classes", "styles", "force"];
    allowed.extend(COLOR_KEYS.iter().map(|(k, _)| *k));
    allow(p, &allowed)?;
    let force = force(p)?;
    let name = match p.get("name") {
        None => None,
        Some(Value::String(t)) => {
            let t = t.trim();
            check_name_text(t, "name")?;
            Some(t.to_string())
        }
        Some(_) => return Err("`name` must be the theme's name (a string)".into()),
    };
    let colors = parse_colors(p)?;
    let classes = match p.get("text_classes") {
        None => Vec::new(),
        Some(tc @ Value::Object(_)) => {
            allow(tc, &TEXT_CLASSES).map_err(|e| format!("`text_classes`: {e}"))?;
            parse_text_classes(tc).map_err(|e| format!("`text_classes`: {e}"))?
        }
        Some(_) => {
            return Err("`text_classes` must be an object: {label?, title?, callout?, header?, …}".into());
        }
    };
    let mut warnings = Vec::new();
    let styles = match p.get("styles") {
        None => Vec::new(),
        Some(Value::Array(a)) if a.len() <= MAX_STYLES => {
            let mut out = Vec::with_capacity(a.len());
            for (i, e) in a.iter().enumerate() {
                if !e.is_object() {
                    return Err(format!(
                        "`styles[{i}]` must be {{visual_type, object, property, value, state?}}"
                    ));
                }
                allow(e, &STYLE_PARAMS).map_err(|x| format!("`styles[{i}]`: {x}"))?;
                out.push(parse_style(e, force, &mut warnings).map_err(|x| format!("`styles[{i}]`: {x}"))?);
            }
            out
        }
        Some(_) => {
            return Err(format!(
                "`styles` must be a list of at most {MAX_STYLES} {{visual_type, object, property, value, state?}}"
            ));
        }
    };
    if name.is_none() && colors.is_empty() && classes.is_empty() && styles.is_empty() {
        return Err(
            "give at least one setting: name, data_colors, a named color, text_classes or styles".into(),
        );
    }
    edit(s, force, move |doc, w| {
        w.extend(warnings);
        if let Some(n) = name {
            doc.insert("name".into(), json!(n));
        }
        for (key, v) in colors {
            put(doc, key, v);
        }
        apply_text_classes(doc, classes)?;
        for st in styles {
            apply_style(doc, st)?;
        }
        Ok(())
    })
}

#[cfg(test)]
#[path = "theme_edit_tests.rs"]
mod tests;
