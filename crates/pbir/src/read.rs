//! PBIR documents → `Report`.

use std::collections::{BTreeMap, HashSet};

use bicraft_report::{NamedDocument, Page, Position, Report, Resource, Visual};
use serde_json::{Map, Value};

use crate::Error;
use crate::fields;
use crate::json::{self, parse_object, parse_schema_url};
use crate::paths::{self, DocKind};

type Object = Map<String, Value>;

fn invalid(path: &str, message: impl Into<String>) -> Error {
    Error::Invalid {
        path: path.to_string(),
        message: message.into(),
    }
}

/// Removes `key` (keeping the order of the rest) when it is a string; errors on another type.
fn take_str(map: &mut Object, key: &str, path: &str) -> Result<Option<String>, Error> {
    match map.get(key) {
        None => Ok(None),
        Some(Value::String(_)) => match map.shift_remove(key) {
            Some(Value::String(s)) => Ok(Some(s)),
            _ => Ok(None),
        },
        Some(other) => Err(invalid(
            path,
            format!("`{key}` must be a string, found {}", json::type_name(other)),
        )),
    }
}

fn take_f64(map: &mut Object, key: &str, path: &str) -> Result<Option<f64>, Error> {
    match map.get(key) {
        None => Ok(None),
        Some(Value::Number(n)) => {
            let v = n.as_f64();
            map.shift_remove(key);
            Ok(v)
        }
        Some(other) => Err(invalid(
            path,
            format!("`{key}` must be a number, found {}", json::type_name(other)),
        )),
    }
}

/// One parsed per-object document (page, visual, bookmark) with its folder/file name.
struct Doc {
    path: String,
    map: Object,
}

#[derive(Default)]
struct Collected {
    version: Option<Doc>,
    report: Option<Doc>,
    pages_meta: Option<Doc>,
    pages: BTreeMap<String, Doc>,
    visuals: BTreeMap<String, BTreeMap<String, Doc>>,
    bookmarks: BTreeMap<String, Doc>,
    bookmarks_meta: Option<Value>,
    resources: Vec<Resource>,
    /// Highest `$schema` seen per document kind: (version, url).
    schemas: BTreeMap<&'static str, ((u64, u64, u64), String)>,
}

impl Collected {
    fn note_schema(&mut self, kind: &DocKind, map: &Object) {
        let (Some((key, _)), Some(url)) = (kind.schema_kind(), map.get("$schema").and_then(Value::as_str))
        else {
            return;
        };
        let version = parse_schema_url(url).map(|(_, v)| v).unwrap_or_default();
        match self.schemas.get(key) {
            Some((seen, _)) if *seen >= version => {}
            _ => {
                self.schemas.insert(key, (version, url.to_string()));
            }
        }
    }
}

pub(crate) fn read(docs: &[(String, Vec<u8>)]) -> Result<Report, Error> {
    let mut c = Collected::default();
    let mut seen = HashSet::new();
    for (raw_path, bytes) in docs {
        let path = paths::normalize(raw_path);
        let kind = paths::classify(&path)?;
        if kind == DocKind::Ignored {
            continue;
        }
        if !seen.insert(path.to_lowercase()) {
            return Err(Error::InvalidPath {
                path,
                message: "duplicate document (paths differ only by case or separators)".into(),
            });
        }
        let doc = |c: &mut Collected| -> Result<Doc, Error> {
            let map = parse_object(&path, bytes)?;
            c.note_schema(&kind, &map);
            Ok(Doc {
                path: path.clone(),
                map,
            })
        };
        match &kind {
            DocKind::Version => c.version = Some(doc(&mut c)?),
            DocKind::Report => c.report = Some(doc(&mut c)?),
            DocKind::Pages => c.pages_meta = Some(doc(&mut c)?),
            DocKind::Page { page } => {
                let d = doc(&mut c)?;
                c.pages.insert(page.clone(), d);
            }
            DocKind::Visual { page, visual } => {
                let d = doc(&mut c)?;
                c.visuals
                    .entry(page.clone())
                    .or_default()
                    .insert(visual.clone(), d);
            }
            DocKind::Bookmark { stem } => {
                let d = doc(&mut c)?;
                c.bookmarks.insert(stem.clone(), d);
            }
            DocKind::Bookmarks => {
                let map = parse_object(&path, bytes)?;
                c.note_schema(&kind, &map);
                c.bookmarks_meta = Some(Value::Object(map));
                c.resources.push(Resource {
                    path,
                    bytes: bytes.clone(),
                });
            }
            DocKind::Mobile { .. } | DocKind::ReportExtensions => {
                if let Ok(map) = parse_object(&path, bytes) {
                    c.note_schema(&kind, &map);
                }
                c.resources.push(Resource {
                    path,
                    bytes: bytes.clone(),
                });
            }
            DocKind::OtherDefinition | DocKind::Resource => c.resources.push(Resource {
                path,
                bytes: bytes.clone(),
            }),
            DocKind::Ignored => {}
        }
    }
    build(c)
}

fn build(mut c: Collected) -> Result<Report, Error> {
    let mut report = Report::default();
    for (key, (_, url)) in &c.schemas {
        report
            .schemas
            .insert((*key).to_string(), Value::String(url.clone()));
    }
    let schema_of = |kind: &str| c.schemas.get(kind).map(|(_, u)| u.clone());

    if let Some(Doc { path, mut map }) = c.version.take() {
        take_str(&mut map, "$schema", &path)?;
        report.version_extra = map;
    }
    if let Some(Doc { path, mut map }) = c.report.take() {
        take_str(&mut map, "$schema", &path)?;
        report.theme_collection = map.shift_remove("themeCollection");
        report.filter_config = map.shift_remove("filterConfig");
        report.extra = map;
    }
    let mut page_order: Vec<String> = Vec::new();
    if let Some(Doc { path, mut map }) = c.pages_meta.take() {
        take_str(&mut map, "$schema", &path)?;
        if let Some(order) = map.get("pageOrder") {
            let Some(list) = order.as_array() else {
                return Err(invalid(&path, "`pageOrder` must be an array"));
            };
            for name in list {
                let Some(name) = name.as_str() else {
                    return Err(invalid(&path, "`pageOrder` entries must be strings"));
                };
                page_order.push(name.to_string());
            }
            map.shift_remove("pageOrder");
        }
        report.active_page = take_str(&mut map, "activePageName", &path)?;
        report.pages_extra = map;
    }

    // Pages, keyed by their `name`.
    let page_schema = schema_of("page");
    let visual_schema = schema_of("visual");
    let mut pages: Vec<Page> = Vec::new();
    let mut page_names = HashSet::new();
    for (folder, doc) in std::mem::take(&mut c.pages) {
        let visuals = c.visuals.remove(&folder).unwrap_or_default();
        let page = read_page(
            &folder,
            doc,
            visuals,
            page_schema.as_deref(),
            visual_schema.as_deref(),
        )?;
        if !page_names.insert(page.name.clone()) {
            return Err(invalid(
                &format!("definition/pages/{folder}/page.json"),
                format!("duplicate page name `{}`", page.name),
            ));
        }
        pages.push(page);
    }
    if let Some((folder, visuals)) = c.visuals.iter().next() {
        let path = visuals
            .values()
            .next()
            .map(|d| d.path.clone())
            .unwrap_or_default();
        return Err(invalid(
            &path,
            format!("visual without a page: no definition/pages/{folder}/page.json"),
        ));
    }
    // `pageOrder` first, then the rest by name (pages are already sorted by folder).
    let mut ordered = Vec::with_capacity(pages.len());
    for name in &page_order {
        if let Some(i) = pages.iter().position(|p| &p.name == name) {
            ordered.push(pages.remove(i));
        }
    }
    pages.sort_by(|a, b| a.name.cmp(&b.name));
    ordered.extend(pages);
    report.pages = ordered;

    // Bookmarks: `bookmarks.json` order (groups flattened), then the rest by name.
    let mut bookmarks: Vec<NamedDocument> = Vec::new();
    for (stem, Doc { path, map }) in std::mem::take(&mut c.bookmarks) {
        let name = match map.get("name") {
            Some(Value::String(s)) => s.clone(),
            _ => stem,
        };
        paths::check_name(&format!("{name}{}", paths::BOOKMARK_SUFFIX)).map_err(|m| Error::InvalidPath {
            path: path.clone(),
            message: m,
        })?;
        if bookmarks.iter().any(|b| b.name == name) {
            return Err(invalid(&path, format!("duplicate bookmark name `{name}`")));
        }
        bookmarks.push(NamedDocument {
            name,
            document: Value::Object(map),
        });
    }
    let mut ordered = Vec::with_capacity(bookmarks.len());
    for name in bookmark_order(c.bookmarks_meta.as_ref()) {
        if let Some(i) = bookmarks.iter().position(|b| b.name == name) {
            ordered.push(bookmarks.remove(i));
        }
    }
    ordered.extend(bookmarks);
    report.bookmarks = ordered;

    c.resources.sort_by(|a, b| a.path.cmp(&b.path));
    report.resources = c.resources;
    Ok(report)
}

/// Bookmark names in `bookmarks.json` order, group children flattened.
pub(crate) fn bookmark_order(meta: Option<&Value>) -> Vec<String> {
    let mut out = Vec::new();
    for item in meta
        .and_then(|m| m.get("items"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(children) = item.get("children").and_then(Value::as_array) {
            out.extend(children.iter().filter_map(Value::as_str).map(str::to_string));
        } else if let Some(name) = item.get("name").and_then(Value::as_str) {
            out.push(name.to_string());
        }
    }
    out
}

/// Keeps a per-object `$schema` only when it differs from the report-level one.
fn strip_common_schema(map: &mut Object, common: Option<&str>) {
    if let (Some(Value::String(s)), Some(common)) = (map.get("$schema"), common)
        && s == common
    {
        map.shift_remove("$schema");
    }
}

fn read_page(
    folder: &str,
    Doc { path, mut map }: Doc,
    visual_docs: BTreeMap<String, Doc>,
    page_schema: Option<&str>,
    visual_schema: Option<&str>,
) -> Result<Page, Error> {
    strip_common_schema(&mut map, page_schema);
    let name = take_str(&mut map, "name", &path)?.unwrap_or_else(|| folder.to_string());
    paths::check_name(&name).map_err(|m| Error::InvalidPath {
        path: path.clone(),
        message: format!("page name: {m}"),
    })?;
    let mut page = Page {
        name,
        display_name: take_str(&mut map, "displayName", &path)?.unwrap_or_default(),
        display_option: take_str(&mut map, "displayOption", &path)?,
        height: take_f64(&mut map, "height", &path)?.unwrap_or_default(),
        width: take_f64(&mut map, "width", &path)?.unwrap_or_default(),
        visibility: take_str(&mut map, "visibility", &path)?,
        filter_config: map.shift_remove("filterConfig"),
        ..Default::default()
    };
    // Keep the order of the remaining keys, but `height`/`width` were typed: nothing else to do.
    page.extra = map;
    let mut names = HashSet::new();
    for (vfolder, doc) in visual_docs {
        let visual = read_visual(&vfolder, doc, visual_schema)?;
        if !names.insert(visual.name.clone()) {
            return Err(invalid(
                &format!("definition/pages/{folder}/visuals/{vfolder}/visual.json"),
                format!("duplicate visual name `{}`", visual.name),
            ));
        }
        page.visuals.push(visual);
    }
    Ok(page)
}

const POSITION_KEYS: [&str; 5] = ["x", "y", "z", "height", "width"];

fn read_visual(
    folder: &str,
    Doc { path, mut map }: Doc,
    visual_schema: Option<&str>,
) -> Result<Visual, Error> {
    strip_common_schema(&mut map, visual_schema);
    let name = take_str(&mut map, "name", &path)?.unwrap_or_else(|| folder.to_string());
    paths::check_name(&name).map_err(|m| Error::InvalidPath {
        path: path.clone(),
        message: format!("visual name: {m}"),
    })?;
    let mut visual = Visual {
        name,
        ..Default::default()
    };

    // Position: typed numbers; anything else stays in `extra.position` (at its original place).
    if let Some(Value::Object(pos)) = map.get_mut("position") {
        let mut p = Position::default();
        for key in POSITION_KEYS {
            if let Some(v) = pos.get(key).and_then(Value::as_f64) {
                pos.shift_remove(key);
                match key {
                    "x" => p.x = v,
                    "y" => p.y = v,
                    "z" => p.z = v,
                    "height" => p.height = v,
                    _ => p.width = v,
                }
            }
        }
        if let Some(t) = pos.get("tabOrder").and_then(Value::as_i64) {
            pos.shift_remove("tabOrder");
            p.tab_order = Some(t);
        }
        visual.position = p;
        if pos.is_empty() {
            map.shift_remove("position");
        }
    }

    match map.shift_remove("visual") {
        None => {}
        Some(Value::Object(mut v)) => {
            visual.visual_type = take_str(&mut v, "visualType", &path)?;
            visual.fields = fields::derive(v.get("query"));
            visual.visual_extra = v;
        }
        Some(other) => {
            return Err(invalid(
                &path,
                format!("`visual` must be an object, found {}", json::type_name(&other)),
            ));
        }
    }
    visual.parent_group = take_str(&mut map, "parentGroupName", &path)?;
    if map.get("isHidden") == Some(&Value::Bool(true)) {
        map.shift_remove("isHidden");
        visual.is_hidden = true;
    }
    visual.filter_config = map.shift_remove("filterConfig");
    visual.extra = map;
    Ok(visual)
}
