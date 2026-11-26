//! Report images for the live viewer (M8, `docs/view-snapshot.md`, "Images").
//!
//! The snapshot names each image by a content hash (`{id, mime, width, height}`); the bytes are
//! served apart by the viewer ([`Session::snapshot_assets`]), never inlined in the snapshot.
//!
//! Images come from:
//! - the page wallpaper (`objects.outspace`) and page background (`objects.background`):
//!   `image.image.{url, scaling}`, falling back to the theme's `visualStyles.page.*`;
//! - image visuals: `objects.general.imageUrl` and `objects.imageScaling.imageScalingType`;
//! - visual backgrounds: `visualContainerObjects.background.image.image.{url, scaling}`.
//!
//! A `url` is a `ResourcePackageItem` (a file under `StaticResources/<package>/`, found through
//! `report.json` `resourcePackages`) or a `data:` URL literal. Other URLs (web links) are not
//! fetched. The type comes from the bytes (PNG, JPEG, GIF, WebP, SVG), never from the name.
//!
//! Results are cached per session, keyed by `revision` and a fingerprint of the report.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use bicraft_report::{Page, Report, Visual};
use serde_json::{Map, Value, json};

use crate::Session;
use crate::theme::{self, Theme};

/// Largest image referenced (bigger ones are left out of the snapshot).
pub const MAX_IMAGE_BYTES: usize = 16 << 20;
/// Total bytes of the distinct images one snapshot references.
pub const MAX_IMAGES_TOTAL_BYTES: usize = 128 << 20;
/// Distinct images one snapshot references.
pub const MAX_IMAGES: usize = 1024;
/// Longest `data:` URL decoded.
const MAX_DATA_URL_CHARS: usize = 23 << 20;
/// Hex digits of the SHA-256 kept as the image id (128 bits).
const ID_HEX: usize = 32;

/// An image the current snapshot references, for the viewer to serve at `/api/asset/<id>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotAsset {
    /// The first 32 hex digits of the SHA-256 of `bytes`.
    pub id: String,
    /// `image/png`, `image/jpeg`, `image/gif`, `image/webp` or `image/svg+xml`.
    pub mime: &'static str,
    pub bytes: Arc<[u8]>,
}

/// One resolved image.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ImageInfo {
    pub id: String,
    pub mime: &'static str,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

impl ImageInfo {
    fn json(&self) -> Map<String, Value> {
        let mut o = Map::new();
        o.insert("id".into(), json!(self.id));
        o.insert("mime".into(), json!(self.mime));
        if let (Some(w), Some(h)) = (self.width, self.height) {
            o.insert("width".into(), json!(w));
            o.insert("height".into(), json!(h));
        }
        o
    }
}

/// An image placed somewhere, with its scaling (`normal`, `fit`, `fill`; lower case).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Placed {
    pub image: Arc<ImageInfo>,
    pub fit: Option<String>,
}

#[derive(Debug, Default)]
struct PageImages {
    wallpaper: Option<Placed>,
    background: Option<Placed>,
}

#[derive(Debug, Default)]
struct VisualImages {
    image: Option<Placed>,
    background: Option<Placed>,
}

/// Every image the report references, resolved once per revision.
#[derive(Debug, Default)]
pub(crate) struct ReportImages {
    pages: HashMap<String, PageImages>,
    visuals: HashMap<(String, String), VisualImages>,
    /// The embedded custom visuals (`custom_visuals::gallery`) with their icon.
    custom: Vec<CustomVisual>,
    /// BIcraft's own visuals (`builtin_visuals::gallery`) with their icon.
    bicraft: Vec<BicraftVisual>,
    assets: Vec<SnapshotAsset>,
}

/// An embedded custom visual as the snapshot shows it.
#[derive(Debug)]
struct CustomVisual {
    name: String,
    display_name: Option<String>,
    icon: Option<Arc<ImageInfo>>,
}

impl CustomVisual {
    fn json(&self) -> Value {
        json!({
            "name": self.name,
            "display_name": self.display_name,
            "icon": self.icon.as_ref().map(|i| Value::Object(i.json())),
        })
    }
}

/// One of BIcraft's visuals as the snapshot shows it.
#[derive(Debug)]
struct BicraftVisual {
    name: &'static str,
    guid: &'static str,
    display_name: &'static str,
    version: &'static str,
    embedded: bool,
    icon: Option<Arc<ImageInfo>>,
}

/// The snapshot's `report.bicraft_visuals`: BIcraft's own visuals, which the Visualizations
/// pane lists after the built-in types whether the report embeds them or not,
/// `{name, guid, display_name, version, embedded, icon: {id, mime, width, height} | null}`.
pub(crate) fn bicraft_visuals_json(images: Option<&ReportImages>) -> Value {
    Value::Array(
        images
            .map(|i| {
                i.bicraft
                    .iter()
                    .map(|b| {
                        json!({
                            "name": b.name,
                            "guid": b.guid,
                            "display_name": b.display_name,
                            "version": b.version,
                            "embedded": b.embedded,
                            "icon": b.icon.as_ref().map(|i| Value::Object(i.json())),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
    )
}

/// The snapshot's `report.custom_visuals`:the embedded custom visuals the Visualizations pane
/// lists, `{name, display_name, icon: {id, mime, width, height} | null}`.
pub(crate) fn custom_visuals_json(images: Option<&ReportImages>) -> Value {
    Value::Array(
        images
            .map(|i| i.custom.iter().map(CustomVisual::json).collect())
            .unwrap_or_default(),
    )
}

/// The per-session cache ([`Session::snapshot`] and [`Session::snapshot_assets`] share it).
#[derive(Default)]
pub(crate) struct ImageCache {
    state: Mutex<Option<(CacheKey, Arc<ReportImages>)>>,
}

impl std::fmt::Debug for ImageCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageCache").finish_non_exhaustive()
    }
}

/// Revision plus a cheap fingerprint of the report (guards against callers that reset
/// `revision`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CacheKey {
    revision: u64,
    resources: usize,
    resource_bytes: usize,
    pages: usize,
    visuals: usize,
}

impl ImageCache {
    fn lock(&self) -> MutexGuard<'_, Option<(CacheKey, Arc<ReportImages>)>> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Session {
    /// The images the current [`Session::snapshot`] references (wallpaper, page backgrounds,
    /// image visuals, visual backgrounds), deduplicated, for the live viewer to serve. Never
    /// fails: unreadable or unsupported images are simply absent (and absent from the snapshot).
    pub fn snapshot_assets(&self) -> Vec<SnapshotAsset> {
        self.report_images().map(|i| i.assets.clone()).unwrap_or_default()
    }

    /// The resolved images of the open report, cached per revision.
    pub(crate) fn report_images(&self) -> Option<Arc<ReportImages>> {
        let r = self.report.as_ref()?;
        let key = CacheKey {
            revision: self.revision,
            resources: r.resources.len(),
            resource_bytes: r
                .resources
                .iter()
                .fold(0usize, |n, x| n.saturating_add(x.bytes.len())),
            pages: r.pages.len(),
            visuals: r
                .pages
                .iter()
                .fold(0usize, |n, p| n.saturating_add(p.visuals.len())),
        };
        let mut guard = self.images.lock();
        if let Some((k, images)) = guard.as_ref()
            && *k == key
        {
            return Some(Arc::clone(images));
        }
        let theme = theme::resolve(r);
        let images = Arc::new(collect(r, theme.as_ref()));
        *guard = Some((key, Arc::clone(&images)));
        Some(images)
    }
}

// ---------------------------------------------------------------------------------------------
// snapshot fields
// ---------------------------------------------------------------------------------------------

fn placed_json(p: &Placed) -> Map<String, Value> {
    let mut o = Map::new();
    o.insert("image".into(), Value::Object(p.image.json()));
    o.insert("fit".into(), json!(p.fit.as_deref().unwrap_or("normal")));
    o
}

/// Adds `wallpaper`, `background_transparency` and `background_image` to a page object.
pub(crate) fn page_fields(
    o: &mut Map<String, Value>,
    p: &Page,
    theme: Option<&Theme>,
    images: Option<&ReportImages>,
) {
    let found = images.and_then(|i| i.pages.get(&p.name));
    let (color, transparency) = theme::page_wallpaper(p, theme);
    let wallpaper_image = found.and_then(|f| f.wallpaper.as_ref());
    if color.is_some() || wallpaper_image.is_some() {
        let mut w = Map::new();
        w.insert("color".into(), json!(color));
        w.insert("transparency".into(), number(transparency.unwrap_or(0.0)));
        match wallpaper_image {
            Some(img) => {
                w.extend(placed_json(img));
            }
            None => {
                w.insert("image".into(), Value::Null);
            }
        }
        o.insert("wallpaper".into(), Value::Object(w));
    }
    let bg_transparency = theme::page_background_transparency(p, theme);
    if let Some(t) = bg_transparency {
        o.insert("background_transparency".into(), number(t));
    }
    if let Some(img) = found.and_then(|f| f.background.as_ref()) {
        let mut b = placed_json(img);
        b.insert("transparency".into(), number(bg_transparency.unwrap_or(0.0)));
        o.insert("background_image".into(), Value::Object(b));
    }
}

/// Adds `image` (image visuals) and `format.background.image` / `image_fit` to a visual object.
pub(crate) fn visual_fields(
    o: &mut Map<String, Value>,
    page: &Page,
    v: &Visual,
    images: Option<&ReportImages>,
) {
    // the BIcraft Spec visual: its spec, drawn by the renderer (docs/formats/spec-visual.md)
    crate::spec_visual::snapshot_fields(o, v);
    // BIcraft's chart pack: its layout's kind and options (docs/formats/chart-pack.md)
    crate::chart_pack::snapshot_fields(o, v);
    // BIcraft's own chart visuals: their kind and options (docs/formats/bicraft-visuals.md)
    crate::bvis::snapshot_fields(o, v);
    // a custom visual: drawn as a labelled placeholder with its own icon and name
    if let (Some(ty), Some(i)) = (v.visual_type.as_deref(), images)
        && let Some(c) = i.custom.iter().find(|c| c.name.eq_ignore_ascii_case(ty))
    {
        o.insert("custom_visual".into(), c.json());
    }
    let Some(found) = images.and_then(|i| i.visuals.get(&(page.name.clone(), v.name.clone()))) else {
        return;
    };
    if let Some(img) = &found.image {
        let mut x = img.image.json();
        x.insert("scaling".into(), json!(img.fit.as_deref().unwrap_or("normal")));
        o.insert("image".into(), Value::Object(x));
    }
    if let Some(img) = &found.background {
        let format = o.entry("format").or_insert_with(|| Value::Object(Map::new()));
        if !format.is_object() {
            *format = Value::Object(Map::new());
        }
        if let Some(format) = format.as_object_mut() {
            let bg = format
                .entry("background")
                .or_insert_with(|| Value::Object(Map::new()));
            if let Some(bg) = bg.as_object_mut() {
                bg.insert("image".into(), Value::Object(img.image.json()));
                bg.insert("image_fit".into(), json!(img.fit.as_deref().unwrap_or("normal")));
            }
        }
    }
}

fn number(x: f64) -> Value {
    if x.is_finite() && x.fract() == 0.0 && x.abs() < 9.0e15 {
        json!(x as i64)
    } else if x.is_finite() {
        json!(x)
    } else {
        Value::Null
    }
}

// ---------------------------------------------------------------------------------------------
// collection
// ---------------------------------------------------------------------------------------------

/// Where an image's bytes come from.
enum Source<'s> {
    Resource(usize),
    Data(&'s str),
}

struct Collector<'a> {
    report: &'a Report,
    /// Lower-cased, `/`-separated resource path → index in `report.resources`.
    by_path: HashMap<String, usize>,
    /// (package, item name), lower-cased → item path, from `resourcePackages`.
    items: HashMap<(String, String), String>,
    /// Resolved sources: resource index or `data:` URL fingerprint → image (or `None`).
    by_resource: HashMap<usize, Option<Arc<ImageInfo>>>,
    by_data: HashMap<(usize, u64), Option<Arc<ImageInfo>>>,
    by_id: HashMap<String, Arc<ImageInfo>>,
    assets: Vec<SnapshotAsset>,
    total: usize,
}

fn norm(path: &str) -> String {
    path.replace('\\', "/")
        .trim_start_matches('/')
        .to_ascii_lowercase()
}

impl<'a> Collector<'a> {
    fn new(report: &'a Report) -> Self {
        let by_path = report
            .resources
            .iter()
            .enumerate()
            .map(|(i, r)| (norm(&r.path), i))
            .collect();
        let mut items = HashMap::new();
        for pkg in report
            .extra
            .get("resourcePackages")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .take(1000)
        {
            // PBIR shape `{name, items}`; tolerate the legacy wrapper `{resourcePackage: {…}}`.
            let pkg = pkg.get("resourcePackage").unwrap_or(pkg);
            let Some(name) = pkg.get("name").and_then(Value::as_str) else {
                continue;
            };
            for item in pkg
                .get("items")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default()
                .iter()
                .take(100_000)
            {
                let (Some(item_name), Some(path)) = (
                    item.get("name").and_then(Value::as_str),
                    item.get("path").and_then(Value::as_str),
                ) else {
                    continue;
                };
                items.insert(
                    (name.to_ascii_lowercase(), item_name.to_ascii_lowercase()),
                    path.to_string(),
                );
            }
        }
        Self {
            report,
            by_path,
            items,
            by_resource: HashMap::new(),
            by_data: HashMap::new(),
            by_id: HashMap::new(),
            assets: Vec::new(),
            total: 0,
        }
    }

    /// The resource a `ResourcePackageItem` names.
    fn find_item(&self, package: &str, item: &str) -> Option<usize> {
        if package.contains("..") || item.contains("..") {
            return None;
        }
        let pkg = package.to_ascii_lowercase();
        let path = self
            .items
            .get(&(pkg.clone(), item.to_ascii_lowercase()))
            .map_or(item, String::as_str);
        let candidates = [
            format!("staticresources/{pkg}/{}", norm(path)),
            format!("staticresources/{pkg}/{}", norm(item)),
        ];
        candidates.iter().find_map(|c| self.by_path.get(c).copied())
    }

    /// The image a url value names (`ResourcePackageItem`, a `data:` literal, or a theme string).
    fn url(&mut self, url: &Value) -> Option<Arc<ImageInfo>> {
        let expr = url.get("expr").unwrap_or(url);
        let source = if let Some(item) = expr.get("ResourcePackageItem") {
            let package = item.get("PackageName").and_then(Value::as_str)?;
            let name = item.get("ItemName").and_then(Value::as_str)?;
            Source::Resource(self.find_item(package, name)?)
        } else {
            let text = match expr.get("Literal").and_then(|l| l.get("Value")) {
                Some(v) => v.as_str()?,
                None => expr.as_str()?,
            };
            let text = text.trim();
            let text = text
                .strip_prefix('\'')
                .and_then(|t| t.strip_suffix('\''))
                .unwrap_or(text);
            if !text.get(..5).is_some_and(|p| p.eq_ignore_ascii_case("data:")) {
                // Web links are never fetched.
                return None;
            }
            Source::Data(text)
        };
        self.resolve(source)
    }

    fn resolve(&mut self, source: Source<'_>) -> Option<Arc<ImageInfo>> {
        match source {
            Source::Resource(i) => {
                if let Some(found) = self.by_resource.get(&i) {
                    return found.clone();
                }
                let report = self.report;
                let found = report.resources.get(i).and_then(|r| self.register(&r.bytes));
                self.by_resource.insert(i, found.clone());
                found
            }
            Source::Data(text) => {
                let key = (text.len(), fnv64(text.as_bytes()));
                if let Some(found) = self.by_data.get(&key) {
                    return found.clone();
                }
                let found = decode_data_url(text).and_then(|bytes| self.register(&bytes));
                self.by_data.insert(key, found.clone());
                found
            }
        }
    }

    /// Hashes, sniffs and records an image; `None` past the caps or for an unsupported type.
    fn register(&mut self, bytes: &[u8]) -> Option<Arc<ImageInfo>> {
        let len = bytes.len();
        if len == 0 || len > MAX_IMAGE_BYTES {
            return None;
        }
        let (mime, size) = sniff(bytes)?;
        let digest = sha256(bytes);
        let mut id = String::with_capacity(ID_HEX);
        for b in digest.iter().take(ID_HEX / 2) {
            id.push_str(&format!("{b:02x}"));
        }
        if let Some(found) = self.by_id.get(&id) {
            return Some(Arc::clone(found));
        }
        if self.assets.len() >= MAX_IMAGES {
            return None;
        }
        let total = self
            .total
            .checked_add(len)
            .filter(|t| *t <= MAX_IMAGES_TOTAL_BYTES)?;
        self.total = total;
        let info = Arc::new(ImageInfo {
            id: id.clone(),
            mime,
            width: size.map(|s| s.0),
            height: size.map(|s| s.1),
        });
        self.by_id.insert(id.clone(), Arc::clone(&info));
        self.assets.push(SnapshotAsset {
            id,
            mime,
            bytes: Arc::from(bytes),
        });
        Some(info)
    }

    /// An `image` property: `{image: {url, scaling}}` (report objects) or `{url, scaling}`
    /// (theme styles).
    fn image_prop(&mut self, entry: &Value, from_theme: bool) -> Option<Placed> {
        let prop = if from_theme {
            entry.get("image")?
        } else {
            entry.get("properties")?.get("image")?
        };
        let inner = prop.get("image").unwrap_or(prop);
        let image = self.url(inner.get("url")?)?;
        Some(Placed {
            image,
            fit: inner.get("scaling").and_then(text_value).and_then(fit_name),
        })
    }

    /// The highest-priority entry (entries are lowest priority first) that sets an image.
    fn first_image(&mut self, entries: &[(&Value, bool)]) -> Option<Placed> {
        entries
            .iter()
            .rev()
            .find_map(|(entry, from_theme)| self.image_prop(entry, *from_theme))
    }
}

/// A literal (`{"expr": {"Literal": {"Value": "'Fit'"}}}`) or a plain theme string.
fn text_value(v: &Value) -> Option<&str> {
    let s = match v
        .get("expr")
        .and_then(|e| e.get("Literal"))
        .and_then(|l| l.get("Value"))
    {
        Some(x) => x.as_str()?,
        None => v.as_str()?,
    };
    let s = s.trim();
    Some(
        s.strip_prefix('\'')
            .and_then(|t| t.strip_suffix('\''))
            .unwrap_or(s),
    )
}

/// `Normal` / `Fit` / `Fill` (as stored) → lower case; anything else is dropped.
fn fit_name(s: &str) -> Option<String> {
    ["normal", "fit", "fill"]
        .into_iter()
        .find(|n| n.eq_ignore_ascii_case(s))
        .map(str::to_string)
}

fn collect(r: &Report, theme: Option<&Theme>) -> ReportImages {
    let mut c = Collector::new(r);
    let mut out = ReportImages::default();
    for p in &r.pages {
        let wallpaper = c.first_image(&theme::page_entries(p, theme, "outspace"));
        let background = c.first_image(&theme::page_entries(p, theme, "background"));
        if wallpaper.is_some() || background.is_some() {
            out.pages.insert(
                p.name.clone(),
                PageImages {
                    wallpaper,
                    background,
                },
            );
        }
        for v in &p.visuals {
            let image = if v
                .visual_type
                .as_deref()
                .is_some_and(|t| t.eq_ignore_ascii_case("image"))
            {
                visual_image(&mut c, v, theme)
            } else {
                None
            };
            let background = c.first_image(&theme::visual_sources(v, theme, "background"));
            if image.is_some() || background.is_some() {
                out.visuals.insert(
                    (p.name.clone(), v.name.clone()),
                    VisualImages { image, background },
                );
            }
        }
    }
    for (name, display_name, icon) in crate::custom_visuals::gallery(r) {
        let icon = icon.and_then(|url| c.resolve(Source::Data(&url)));
        out.custom.push(CustomVisual {
            name,
            display_name,
            icon,
        });
    }
    for (b, embedded) in crate::builtin_visuals::gallery(r) {
        let icon = (b.icon)(b.guid).ok().and_then(|png| c.register(&png));
        out.bicraft.push(BicraftVisual {
            name: b.name,
            guid: b.guid,
            display_name: b.display_name,
            version: b.version,
            embedded,
            icon,
        });
    }
    out.assets = c.assets;
    out
}

/// An image visual's picture: `general.imageUrl`, scaled by `imageScaling.imageScalingType`.
fn visual_image(c: &mut Collector<'_>, v: &Visual, theme: Option<&Theme>) -> Option<Placed> {
    let url = theme::visual_sources(v, theme, "general")
        .into_iter()
        .rev()
        .find_map(|(entry, from_theme)| {
            if from_theme {
                entry.get("imageUrl")
            } else {
                entry.get("properties")?.get("imageUrl")
            }
        })?;
    let image = c.url(url)?;
    let fit = theme::visual_sources(v, theme, "imageScaling")
        .into_iter()
        .rev()
        .find_map(|(entry, from_theme)| {
            let x = if from_theme {
                entry.get("imageScalingType")
            } else {
                entry.get("properties")?.get("imageScalingType")
            }?;
            text_value(x).and_then(fit_name)
        });
    Some(Placed { image, fit })
}

// ---------------------------------------------------------------------------------------------
// bytes
// ---------------------------------------------------------------------------------------------

/// The image type from the bytes, and the pixel size when the header gives it.
pub(crate) fn sniff(b: &[u8]) -> Option<(&'static str, Option<(u32, u32)>)> {
    let be16 = |i: usize| -> Option<u32> { Some(u32::from(*b.get(i)?) << 8 | u32::from(*b.get(i + 1)?)) };
    let le16 = |i: usize| -> Option<u32> { Some(u32::from(*b.get(i)?) | u32::from(*b.get(i + 1)?) << 8) };
    let be32 = |i: usize| -> Option<u32> { Some(be16(i)? << 16 | be16(i + 2)?) };
    let le24 = |i: usize| -> Option<u32> { Some(le16(i)? | u32::from(*b.get(i + 2)?) << 16) };
    let sane = |w: u32, h: u32| (w > 0 && h > 0 && w <= 1 << 20 && h <= 1 << 20).then_some((w, h));
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        let size = (b.get(12..16) == Some(b"IHDR"))
            .then(|| Some((be32(16)?, be32(20)?)))
            .flatten()
            .and_then(|(w, h)| sane(w, h));
        return Some(("image/png", size));
    }
    if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
        return Some(("image/gif", le16(6).zip(le16(8)).and_then(|(w, h)| sane(w, h))));
    }
    if b.starts_with(b"\xFF\xD8\xFF") {
        return Some(("image/jpeg", jpeg_size(b).and_then(|(w, h)| sane(w, h))));
    }
    if b.starts_with(b"RIFF") && b.get(8..12) == Some(b"WEBP") {
        let size = match b.get(12..16) {
            Some(b"VP8X") => le24(24)
                .zip(le24(27))
                .map(|(w, h)| (w.saturating_add(1), h.saturating_add(1))),
            Some(b"VP8 ") if b.get(23..26) == Some(b"\x9d\x01\x2a") => {
                le16(26).zip(le16(28)).map(|(w, h)| (w & 0x3fff, h & 0x3fff))
            }
            Some(b"VP8L") if b.get(20) == Some(&0x2f) => (|| {
                let (b0, b1, b2, b3) = (
                    u32::from(*b.get(21)?),
                    u32::from(*b.get(22)?),
                    u32::from(*b.get(23)?),
                    u32::from(*b.get(24)?),
                );
                Some((
                    1 + (b0 | (b1 & 0x3f) << 8),
                    1 + (b1 >> 6 | b2 << 2 | (b3 & 0x0f) << 10),
                ))
            })(),
            _ => None,
        };
        return Some(("image/webp", size.and_then(|(w, h)| sane(w, h))));
    }
    is_svg(b).then_some(("image/svg+xml", None))
}

/// Width and height from the first JPEG start-of-frame segment.
fn jpeg_size(b: &[u8]) -> Option<(u32, u32)> {
    let mut i = 2usize;
    for _ in 0..10_000 {
        if *b.get(i)? != 0xFF {
            return None;
        }
        let mut m = *b.get(i + 1)?;
        while m == 0xFF {
            i += 1;
            m = *b.get(i + 1)?;
        }
        if m == 0x01 || (0xD0..=0xD9).contains(&m) {
            i += 2;
            continue;
        }
        let len = usize::from(*b.get(i + 2)?) << 8 | usize::from(*b.get(i + 3)?);
        if (0xC0..=0xCF).contains(&m) && !matches!(m, 0xC4 | 0xC8 | 0xCC) {
            let h = u32::from(*b.get(i + 5)?) << 8 | u32::from(*b.get(i + 6)?);
            let w = u32::from(*b.get(i + 7)?) << 8 | u32::from(*b.get(i + 8)?);
            return Some((w, h));
        }
        if len < 2 {
            return None;
        }
        i = i.checked_add(2)?.checked_add(len)?;
    }
    None
}

/// An SVG document: after an optional BOM, whitespace, XML declaration, comments and doctype,
/// the root element is `<svg`.
fn is_svg(b: &[u8]) -> bool {
    let b = b.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(b);
    let mut rest = b.get(..b.len().min(64 * 1024)).unwrap_or(b);
    for _ in 0..64 {
        while let Some((first, tail)) = rest.split_first() {
            if first.is_ascii_whitespace() {
                rest = tail;
            } else {
                break;
            }
        }
        let skip_to = |rest: &[u8], end: &[u8]| -> Option<usize> {
            rest.windows(end.len())
                .position(|w| w == end)
                .map(|p| p + end.len())
        };
        let next = if rest.starts_with(b"<?") {
            skip_to(rest, b"?>")
        } else if rest.starts_with(b"<!--") {
            skip_to(rest, b"-->")
        } else if rest.starts_with(b"<!") {
            skip_to(rest, b">")
        } else {
            return rest.len() > 4
                && rest.get(..4).is_some_and(|p| p.eq_ignore_ascii_case(b"<svg"))
                && rest
                    .get(4)
                    .is_some_and(|c| c.is_ascii_whitespace() || *c == b'>' || *c == b'/');
        };
        match next.and_then(|n| rest.get(n..)) {
            Some(r) => rest = r,
            None => return false,
        }
    }
    false
}

/// The bytes of a `data:` URL (`;base64` or percent-encoded).
fn decode_data_url(url: &str) -> Option<Vec<u8>> {
    if url.len() > MAX_DATA_URL_CHARS {
        return None;
    }
    let (head, data) = url.get(5..)?.split_once(',')?;
    if head
        .rsplit(';')
        .next()
        .is_some_and(|p| p.trim().eq_ignore_ascii_case("base64"))
    {
        base64(data)
    } else {
        Some(percent_decode(data))
    }
}

pub(crate) fn base64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut acc = 0u32;
    let mut bits = 0u32;
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            b' ' | b'\t' | b'\r' | b'\n' => continue,
            _ => return None,
        };
        acc = (acc << 6 | u32::from(v)) & 0xFF_FFFF;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xFF) as u8);
        }
    }
    Some(out)
}

fn percent_decode(text: &str) -> Vec<u8> {
    let b = text.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0usize;
    while let Some(&c) = b.get(i) {
        let hex = |x: u8| (x as char).to_digit(16);
        if c == b'%'
            && let (Some(h), Some(l)) = (
                b.get(i + 1).and_then(|x| hex(*x)),
                b.get(i + 2).and_then(|x| hex(*x)),
            )
        {
            out.push((h << 4 | l) as u8);
            i += 3;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

fn fnv64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// SHA-256 (FIPS 180-4).
/// The id an image is served under: the first 32 hex digits of its SHA-256.
pub(crate) fn asset_id(bytes: &[u8]) -> String {
    let mut id = String::with_capacity(ID_HEX);
    for b in sha256(bytes).iter().take(ID_HEX / 2) {
        id.push_str(&format!("{b:02x}"));
    }
    id
}

fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut tail = Vec::with_capacity(128);
    let full = data.len() - data.len() % 64;
    tail.extend_from_slice(data.get(full..).unwrap_or_default());
    tail.push(0x80);
    while tail.len() % 64 != 56 {
        tail.push(0);
    }
    tail.extend_from_slice(&bit_len.to_be_bytes());
    let blocks = data
        .get(..full)
        .unwrap_or_default()
        .as_chunks::<64>()
        .0
        .iter()
        .chain(tail.as_chunks::<64>().0.iter());
    let mut w = [0u32; 64];
    for block in blocks {
        for (slot, word) in w.iter_mut().zip(block.as_chunks::<4>().0) {
            *slot = u32::from_be_bytes(*word);
        }
        for i in 16..64 {
            let at = |j: usize| w.get(j).copied().unwrap_or(0);
            let (w15, w2) = (at(i - 15), at(i - 2));
            let s0 = w15.rotate_right(7) ^ w15.rotate_right(18) ^ (w15 >> 3);
            let s1 = w2.rotate_right(17) ^ w2.rotate_right(19) ^ (w2 >> 10);
            let next = at(i - 16)
                .wrapping_add(s0)
                .wrapping_add(at(i - 7))
                .wrapping_add(s1);
            if let Some(slot) = w.get_mut(i) {
                *slot = next;
            }
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for (k, wi) in K.iter().zip(w.iter()) {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(*k)
                .wrapping_add(*wi);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 32];
    for (chunk, x) in out.as_chunks_mut::<4>().0.iter_mut().zip(h) {
        *chunk = x.to_be_bytes();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(d: &[u8]) -> String {
        d.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn sha256_vectors() {
        assert_eq!(
            hex(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex(&sha256(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        let million = vec![b'a'; 1_000_000];
        assert_eq!(
            hex(&sha256(&million)),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn sniffing_and_sizes() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&[0, 0, 0, 7, 0, 0, 0, 3]);
        assert_eq!(sniff(&png), Some(("image/png", Some((7, 3)))));
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n"), Some(("image/png", None)));
        assert_eq!(
            sniff(b"GIF89a\x10\x00\x20\x00"),
            Some(("image/gif", Some((16, 32))))
        );
        // SOI, APP0 (len 4), SOF0 with height 0x0102 and width 0x0304
        let jpeg = b"\xFF\xD8\xFF\xE0\x00\x04ab\xFF\xC0\x00\x11\x08\x01\x02\x03\x04";
        assert_eq!(sniff(jpeg), Some(("image/jpeg", Some((0x0304, 0x0102)))));
        assert_eq!(sniff(b"\xFF\xD8\xFF\xE0\x00\x01"), Some(("image/jpeg", None)));
        let mut webp = b"RIFF\0\0\0\0WEBPVP8X".to_vec();
        webp.extend_from_slice(&[0; 8]);
        webp.extend_from_slice(&[9, 0, 0, 4, 0, 0]);
        assert_eq!(sniff(&webp), Some(("image/webp", Some((10, 5)))));
        for svg in [
            &b"<svg xmlns='http://www.w3.org/2000/svg'/>"[..],
            b"\xEF\xBB\xBF<?xml version='1.0'?>\n<!-- c --><!DOCTYPE svg><SVG width='1'>",
        ] {
            assert_eq!(sniff(svg), Some(("image/svg+xml", None)));
        }
        for not in [
            &b"<html><svg></svg></html>"[..],
            b"<svgx>",
            b"<!-- never closed",
            b"",
            b"BM\0\0",
            b"{\"name\": \"theme\"}",
        ] {
            assert_eq!(sniff(not), None);
        }
    }

    #[test]
    fn data_urls() {
        assert_eq!(
            decode_data_url("data:image/png;base64,iVBORw0KGgo=").unwrap(),
            b"\x89PNG\r\n\x1a\n"
        );
        assert_eq!(decode_data_url("data:image/svg+xml,%3Csvg%3E").unwrap(), b"<svg>");
        assert_eq!(decode_data_url("data:;base64,@@"), None);
        assert_eq!(decode_data_url("data:nocomma"), None);
        assert_eq!(percent_decode("%zz%4"), b"%zz%4");
    }
}

#[cfg(test)]
#[path = "images_tests.rs"]
mod session_tests;
