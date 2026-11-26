//! `Report` → PBIR documents.

use std::collections::HashSet;

use bicraft_report::{Page, Report, Visual};
use bicraft_versions::{self as versions, Schema};
use serde_json::{Map, Value, json};

use crate::fields;
use crate::json::{self, merge, number, schema_url};
use crate::paths::{self, DocKind};
use crate::read::bookmark_order;

/// Schema versions for documents created without a recorded `$schema`: what Power BI Desktop
/// 2.158 writes, or the newest public schema when Desktop's is not published (see pbir.md).
/// The table lives in `bicraft-versions` (checked by `cargo xtask schema-watch`).
pub(crate) const DEFAULT_VERSIONS: [(&str, Schema); 7] = [
    ("version", versions::VERSION_METADATA),
    ("report", versions::REPORT),
    ("pages", versions::PAGES_METADATA),
    ("page", versions::PAGE),
    ("visual", versions::VISUAL_CONTAINER),
    ("bookmarks", versions::BOOKMARKS_METADATA),
    ("bookmark", versions::BOOKMARK),
];

/// `version.json` `version` written when none was read.
const DEFINITION_VERSION: &str = versions::PBIR_DEFINITION_VERSION;

/// Desktop key orders (public schema property order, adjusted to what Desktop writes).
const REPORT_ORDER: [&str; 12] = [
    "$schema",
    "themeCollection",
    "filterConfig",
    "objects",
    "reportSource",
    "publicCustomVisuals",
    "resourcePackages",
    "organizationCustomVisuals",
    "annotations",
    "dataSourceVariables",
    "settings",
    "slowDataSourceSettings",
];
const PAGES_ORDER: [&str; 4] = ["$schema", "pageOrder", "activePageName", "landingPageName"];
const VERSION_ORDER: [&str; 2] = ["$schema", "version"];
const PAGE_ORDER: [&str; 15] = [
    "$schema",
    "name",
    "displayName",
    "displayOption",
    "height",
    "width",
    "filterConfig",
    "pageBinding",
    "objects",
    "type",
    "visibility",
    "visualInteractions",
    "autoPageGenerationConfig",
    "annotations",
    "howCreated",
];
const VISUAL_ORDER: [&str; 10] = [
    "$schema",
    "name",
    "position",
    "visual",
    "visualGroup",
    "parentGroupName",
    "isHidden",
    "annotations",
    "filterConfig",
    "howCreated",
];
const POSITION_ORDER: [&str; 7] = ["x", "y", "z", "height", "width", "tabOrder", "angle"];
const VISUAL_CONFIG_ORDER: [&str; 8] = [
    "visualType",
    "autoSelectVisualType",
    "query",
    "expansionStates",
    "objects",
    "visualContainerObjects",
    "syncGroup",
    "drillFilterOtherVisuals",
];

fn schema_for(report: &Report, kind: &str) -> Value {
    if let Some(Value::String(s)) = report.schemas.get(kind) {
        return Value::String(s.clone());
    }
    let (key, version) = DEFAULT_VERSIONS
        .iter()
        .find(|(k, _)| *k == kind)
        .map(|(_, schema)| (schema.name(), schema.version))
        .unwrap_or((kind, "1.0.0"));
    Value::String(schema_url(key, version))
}

pub(crate) fn write(report: &Report) -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<(String, Vec<u8>)> = Vec::new();
    let mut emitted: HashSet<String> = HashSet::new();
    let mut push = |out: &mut Vec<(String, Vec<u8>)>, path: String, bytes: Vec<u8>| {
        if emitted.insert(path.to_lowercase()) {
            out.push((path, bytes));
        }
    };

    // version.json
    let mut typed = vec![("$schema", schema_for(report, "version"))];
    if !report.version_extra.contains_key("version") {
        typed.push(("version", json!(DEFINITION_VERSION)));
    }
    let doc = merge(&VERSION_ORDER, typed, &report.version_extra, &[]);
    push(
        &mut out,
        paths::VERSION.into(),
        json::to_bytes(&Value::Object(doc)),
    );

    // report.json
    let mut typed = vec![
        ("$schema", schema_for(report, "report")),
        (
            "themeCollection",
            report.theme_collection.clone().unwrap_or_else(|| json!({})),
        ),
    ];
    if let Some(f) = &report.filter_config {
        typed.push(("filterConfig", f.clone()));
    }
    let doc = merge(&REPORT_ORDER, typed, &report.extra, &[]);
    push(
        &mut out,
        paths::REPORT.into(),
        json::to_bytes(&Value::Object(doc)),
    );

    // pages.json
    let pages: Vec<&Page> = report
        .pages
        .iter()
        .filter(|p| paths::check_name(&p.name).is_ok())
        .collect();
    let mut typed = vec![
        ("$schema", schema_for(report, "pages")),
        (
            "pageOrder",
            Value::Array(pages.iter().map(|p| json!(p.name)).collect()),
        ),
    ];
    if let Some(active) = &report.active_page {
        typed.push(("activePageName", json!(active)));
    }
    let doc = merge(&PAGES_ORDER, typed, &report.pages_extra, &[]);
    push(&mut out, paths::PAGES.into(), json::to_bytes(&Value::Object(doc)));

    // Pages and visuals.
    let page_schema = schema_for(report, "page");
    let visual_schema = schema_for(report, "visual");
    let mut live_visuals: HashSet<(String, String)> = HashSet::new();
    for page in &pages {
        let path = format!("definition/pages/{}/page.json", page.name);
        push(
            &mut out,
            path,
            json::to_bytes(&Value::Object(page_json(page, &page_schema))),
        );
        for visual in page.visuals.iter().filter(|v| paths::check_name(&v.name).is_ok()) {
            live_visuals.insert((page.name.clone(), visual.name.clone()));
            let path = format!(
                "definition/pages/{}/visuals/{}/visual.json",
                page.name, visual.name
            );
            push(
                &mut out,
                path,
                json::to_bytes(&Value::Object(visual_json(visual, &visual_schema))),
            );
        }
    }

    // Bookmarks.
    let bookmark_schema = schema_for(report, "bookmark");
    let mut names: Vec<&str> = Vec::new();
    for b in &report.bookmarks {
        if paths::check_name(&format!("{}{}", b.name, paths::BOOKMARK_SUFFIX)).is_err() {
            continue;
        }
        let document = match &b.document {
            Value::Object(map) if !map.contains_key("$schema") => {
                let mut doc = Map::new();
                doc.insert("$schema".into(), bookmark_schema.clone());
                doc.extend(map.clone());
                Value::Object(doc)
            }
            other => other.clone(),
        };
        names.push(&b.name);
        let path = format!("definition/bookmarks/{}{}", b.name, paths::BOOKMARK_SUFFIX);
        push(&mut out, path, json::to_bytes(&document));
    }
    let meta = report
        .resources
        .iter()
        .find(|r| paths::normalize(&r.path) == paths::BOOKMARKS);
    if let Some(bytes) = bookmarks_json(report, meta.map(|r| r.bytes.as_slice()), &names) {
        push(&mut out, paths::BOOKMARKS.into(), bytes);
    }

    // Kept documents and static resources.
    let live_pages: HashSet<&str> = pages.iter().map(|p| p.name.as_str()).collect();
    for resource in &report.resources {
        let path = paths::normalize(&resource.path);
        if path == paths::BOOKMARKS || !paths::resource_path_ok(&path) {
            continue;
        }
        if !belongs(&path, &live_pages, &live_visuals) {
            continue;
        }
        push(&mut out, path, resource.bytes.clone());
    }
    out
}

/// Whether a kept document under `definition/pages/<p>/[visuals/<v>/]` still has its page/visual.
fn belongs(path: &str, pages: &HashSet<&str>, visuals: &HashSet<(String, String)>) -> bool {
    let parts: Vec<&str> = path.split('/').collect();
    match parts.as_slice() {
        ["definition", "pages", page, "visuals", visual, _, ..] => {
            visuals.contains(&((*page).to_string(), (*visual).to_string()))
        }
        ["definition", "pages", page, _, ..] => pages.contains(page),
        _ => true,
    }
}

fn page_json(page: &Page, default_schema: &Value) -> Map<String, Value> {
    let mut typed = vec![
        ("name", json!(page.name)),
        ("displayName", json!(page.display_name)),
        (
            "displayOption",
            json!(page.display_option.as_deref().unwrap_or("FitToPage")),
        ),
    ];
    // Desktop writes no size for the deprecated dynamic option, nor a zero size.
    if page.display_option.as_deref() != Some("DeprecatedDynamic") {
        if page.height != 0.0 {
            typed.push(("height", number(page.height)));
        }
        if page.width != 0.0 {
            typed.push(("width", number(page.width)));
        }
    }
    if !page.extra.contains_key("$schema") {
        typed.push(("$schema", default_schema.clone()));
    }
    if let Some(v) = &page.visibility {
        typed.push(("visibility", json!(v)));
    }
    if let Some(f) = &page.filter_config {
        typed.push(("filterConfig", f.clone()));
    }
    merge(&PAGE_ORDER, typed, &page.extra, &[])
}

fn visual_json(visual: &Visual, default_schema: &Value) -> Map<String, Value> {
    let p = &visual.position;
    let mut pos = vec![
        ("x", number(p.x)),
        ("y", number(p.y)),
        ("z", number(p.z)),
        ("height", number(p.height)),
        ("width", number(p.width)),
    ];
    if let Some(t) = p.tab_order {
        pos.push(("tabOrder", json!(t)));
    }
    let leftover = match visual.extra.get("position") {
        Some(Value::Object(m)) => m.clone(),
        _ => Map::new(),
    };
    let position = merge(&POSITION_ORDER, pos, &leftover, &[]);

    let mut typed = vec![
        ("name", json!(visual.name)),
        ("position", Value::Object(position)),
    ];
    if !visual.extra.contains_key("$schema") {
        typed.push(("$schema", default_schema.clone()));
    }
    if visual.visual_type.is_some() || !visual.visual_extra.is_empty() || !visual.fields.is_empty() {
        typed.push(("visual", Value::Object(visual_config(visual))));
    }
    if let Some(g) = &visual.parent_group {
        typed.push(("parentGroupName", json!(g)));
    }
    if visual.is_hidden {
        typed.push(("isHidden", json!(true)));
    }
    if let Some(f) = &visual.filter_config {
        typed.push(("filterConfig", f.clone()));
    }
    merge(&VISUAL_ORDER, typed, &visual.extra, &[])
}

/// The `visual` object: `visualType`, the raw query (or one regenerated from edited fields), and
/// the unmodelled keys.
fn visual_config(visual: &Visual) -> Map<String, Value> {
    let mut typed = Vec::new();
    if let Some(t) = &visual.visual_type {
        typed.push(("visualType", json!(t)));
    }
    let raw = visual.visual_extra.get("query");
    let mut remove: Vec<&str> = Vec::new();
    if fields::derive(raw) != visual.fields {
        match fields::regenerate(raw, &visual.fields) {
            Some(q) => typed.push(("query", q)),
            None => remove.push("query"),
        }
    }
    merge(&VISUAL_CONFIG_ORDER, typed, &visual.visual_extra, &remove)
}

/// `bookmarks.json`: the kept document reconciled with the bookmark list (missing bookmarks
/// removed, new ones appended), its original bytes when nothing changed.
fn bookmarks_json(report: &Report, raw: Option<&[u8]>, names: &[&str]) -> Option<Vec<u8>> {
    let mut doc = match raw.map(|b| json::parse(paths::BOOKMARKS, b)) {
        None if names.is_empty() => return None,
        None => {
            let mut m = Map::new();
            m.insert("$schema".into(), schema_for(report, "bookmarks"));
            m.insert("items".into(), Value::Array(Vec::new()));
            Value::Object(m)
        }
        Some(Ok(v)) if v.is_object() => v,
        // Not reconcilable: keep it verbatim.
        Some(_) => return raw.map(<[u8]>::to_vec),
    };
    let before = doc.clone();
    let listed: HashSet<String> = bookmark_order(Some(&doc)).into_iter().collect();
    if let Value::Object(map) = &mut doc {
        let mut items: Vec<Value> = match map.get("items") {
            Some(Value::Array(a)) => a.clone(),
            _ => Vec::new(),
        };
        items.retain_mut(|item| {
            if let Some(children) = item.get_mut("children").and_then(Value::as_array_mut) {
                children.retain(|c| c.as_str().is_some_and(|n| names.contains(&n)));
                !children.is_empty()
            } else {
                item.get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|n| names.contains(&n))
            }
        });
        for name in names {
            if !listed.contains(*name) {
                items.push(json!({"name": name}));
            }
        }
        map.insert("items".into(), Value::Array(items));
    }
    match raw {
        Some(bytes) if doc == before => Some(bytes.to_vec()),
        _ => Some(json::to_bytes(&doc)),
    }
}

/// Documents `write_dir` owns under `definition/`: a stale one (not written any more) is deleted.
pub(crate) fn is_owned_kind(kind: &DocKind) -> bool {
    matches!(
        kind,
        DocKind::Page { .. }
            | DocKind::Visual { .. }
            | DocKind::Mobile { .. }
            | DocKind::Bookmark { .. }
            | DocKind::Bookmarks
    )
}
