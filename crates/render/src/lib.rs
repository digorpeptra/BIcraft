//! L4: renders report pages to PNG from a viewer snapshot (`docs/view-snapshot.md`) in pure
//! Rust, so agents can look at a page without Power BI Desktop (`report.render`,
//! `bicraft-cli render`). The drawing is a port of the live viewer's (`crates/view/src/page.html`):
//! same layout, theme, evaluated data, slicer selections, images, text boxes, titles, conditional
//! formatting and backgrounds. Rasterization: tiny-skia; glyphs: skrifa over the system's
//! fonts (Segoe UI…) with the bundled Selawik (SIL OFL) as fallback. See `docs/render.md`.
//!
//! The renderer only reads JSON: it depends on no other bicraft crate.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
// Drawing calls take positions, sizes and styles: long argument lists are the clearer form here.
#![allow(clippy::too_many_arguments, clippy::type_complexity)]

pub mod anomaly;
mod axes;
mod bodies;
pub mod bvis;
mod canvas;
pub mod chart_pack;
mod charts;
mod data;
pub mod decomp;
mod fmt;
mod fonts;
pub mod gantt;
pub mod geo;
pub mod hits;
mod icc;
mod imgs;
mod keyinf;
pub mod look;
mod maps;
pub mod own_hits;
mod parts;
mod pdf;
pub mod pdf_read;
pub mod phone;
mod ribbon;
mod shapes;
mod slicers;
pub mod spec;
mod state;
pub mod states;
mod tables;
mod timeaxis;
mod ttf_subset;

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;

pub use crate::canvas::TextRun;
use crate::canvas::{Anchor, Base, Canvas, Rgb, Text};
use crate::charts::{BarOpts, Bx, LineOpts};
use crate::data::{
    Ctx, Fmt, PT, arr, auto_title, clampf, cmp_raw, fit_of, get, img_of, key_of, nk, opacity_of, sk,
};
/// The date labels of BIcraft's custom visuals (twin of their TypeScript `dateLabel`).
pub use crate::fmt::date_label;
pub use crate::fonts::{
    BUNDLED_FALLBACKS, CARLITO_BOLD, CARLITO_BOLD_ITALIC, CARLITO_ITALIC, CARLITO_REGULAR, Font, FontBook,
    NOTO_EMOJI, NOTO_SANS_BOLD, NOTO_SANS_MATH, NOTO_SANS_MONO, NOTO_SANS_REGULAR, NOTO_SANS_SYMBOLS,
    NOTO_SANS_SYMBOLS2, SELAWIK_BOLD, SELAWIK_LIGHT, SELAWIK_REGULAR, SELAWIK_SEMIBOLD, system_font_dirs,
};
pub use crate::state::{DataKind, DataState, data_state, data_state_of};

/// A conditional-formatting icon (`CircleHigh`, `TrafficLow`, `ColoredArrowUp`…) drawn as the
/// tables draw it, in a `px`×`px` RGBA image (not premultiplied, rows top to bottom): the
/// conditional-formatting dialog's icon lists and preview show the same icons. `None` for a
/// size outside 1–256.
pub fn icon_rgba(name: &str, px: u32) -> Option<Vec<u8>> {
    if !(1..=256).contains(&px) {
        return None;
    }
    let size = px as f32;
    let mut cv = canvas::Canvas::new(size, size, 1.0)?;
    tables::draw_icon(&mut cv, name, 0.0, 0.0, size);
    let mut out = Vec::with_capacity((px * px * 4) as usize);
    for p in cv.pm.pixels() {
        let c = p.demultiply();
        out.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    Some(out)
}

/// The colours the pages of snapshot `snap` draw their series and categories with, in order, as
/// `#RRGGBB`: the report theme's data colours, else the palette Desktop uses for that report (the
/// Format visual pane shows a series' colour with it before one is set).
pub fn data_colors(snap: &Value) -> Vec<String> {
    let theme = snap
        .get("report")
        .and_then(|r| r.get("theme"))
        .unwrap_or(&Value::Null);
    data::palette(theme)
        .iter()
        .map(|c| format!("#{:02X}{:02X}{:02X}", c.0, c.1, c.2))
        .collect()
}

/// Image bytes by snapshot image id.
pub trait Assets {
    fn get(&self, id: &str) -> Option<&[u8]>;
}
impl Assets for HashMap<String, Vec<u8>> {
    fn get(&self, id: &str) -> Option<&[u8]> {
        HashMap::get(self, id).map(Vec::as_slice)
    }
}
impl Assets for HashMap<String, Arc<[u8]>> {
    fn get(&self, id: &str) -> Option<&[u8]> {
        HashMap::get(self, id).map(|b| &b[..])
    }
}

/// Rendering options.
#[derive(Debug, Clone)]
pub struct RenderOptions {
    /// Device pixels per report pixel (default 1).
    pub scale: f32,
    /// Index the system's font directories (else only `font_dirs` and the bundled fallback).
    pub system_fonts: bool,
    /// More font directories.
    pub font_dirs: Vec<PathBuf>,
    /// Largest image (device pixels); the scale is lowered to fit.
    pub max_pixels: u64,
    /// Draw the "no data" / "filters ignored" badges.
    pub badges: bool,
    /// Formatting culture override (numbers, dates, automatic titles); `None` formats in the
    /// snapshot's model culture (`model.culture`, as Desktop and the live viewer do), en-US when
    /// the model has none. Not the translation culture (`display_culture`, M10.4).
    pub culture: Option<String>,
    /// Return every drawn text run in [`PageImage::texts`] (render oracle text check).
    pub text_dump: bool,
    /// Record the hit map in [`PageImage::hits`]: where clicks land and the `view.interact`
    /// action each sends (a native front end, `apps/bicraft-desktop`).
    pub hits: bool,
    /// Return the pixels in [`PageImage::rgba`] (premultiplied RGBA, row-major) instead of a PNG.
    pub raw: bool,
    /// Draw upright text with subpixel (LCD, RGB stripes) antialiasing where it lands on opaque
    /// pixels, for a page shown 1:1 on a screen of low DPI (the app's pages, `crates/ui`); off by
    /// default: files, goldens and PDF keep grayscale text (docs/render.md, "Subpixel text").
    pub lcd_text: bool,
    /// Draw the page with the high-contrast palette (`Rgb::high_contrast`: black page, white
    /// text, bright series colours; images unchanged): the app's view-only option, never
    /// written to a file (docs/render.md, "High contrast").
    pub high_contrast: bool,
    /// Where the reader's pointer is: that button, shape or navigator button is drawn in its
    /// hover (or press) state ([`states::Pointer`]); `None` for files and exports.
    pub pointer: Option<states::Pointer>,
    /// The reader's view of each Gantt on the page, by visual name: its first line shown and
    /// its time line's zoom ([`gantt::View`]); the app's view state, never written to a file.
    /// Empty: every Gantt from its first line over its whole time line.
    pub gantt: HashMap<String, gantt::View>,
    /// The author's grid (the app's View › Gridlines): dots over the page background and image,
    /// under the visuals; `None` for files and exports, never written to a file.
    pub grid: Option<GridDots>,
}

/// The dots of the author's grid ([`RenderOptions::grid`]), in report pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridDots {
    /// Distance between dots, across and down (at least 1).
    pub step: (f32, f32),
    /// Side of a dot.
    pub dot: f32,
    /// sRGB colour and alpha of the dots.
    pub rgba: [u8; 4],
}

/// Most grid dots drawn on a page; a finer grid is not drawn.
pub const MAX_GRID_DOTS: usize = 20_000;

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            scale: 1.0,
            system_fonts: true,
            font_dirs: Vec::new(),
            max_pixels: 40_000_000,
            badges: true,
            culture: None,
            text_dump: false,
            hits: false,
            raw: false,
            lcd_text: false,
            high_contrast: false,
            pointer: None,
            gantt: HashMap::new(),
            grid: None,
        }
    }
}

/// What happened to one visual.
#[derive(Debug, Clone, PartialEq)]
pub struct VisualNote {
    pub name: String,
    pub visual_type: String,
    /// `drawn`, `preview` (made-up values: the snapshot has no data), `placeholder` (a type the
    /// renderer does not draw), `blank` (a data visual with no field bound: the empty-visual prompt)
    /// `estimate` (a key influencers visual drawn from BIcraft's own estimate, never Power BI's result) or `skipped` (outside the page, too large). A visual drawn from an empty dataset is still
    /// `drawn`: see [`VisualNote::data`].
    pub status: &'static str,
    pub note: Option<String>,
    /// What the evaluated dataset holds, independent of what was drawn ([`data_state`]).
    pub data: DataState,
    /// How long drawing it took, in milliseconds (a monotonic clock, the browser's too; `None` for a
    /// visual not drawn): the app's Performance analyzer reports it as BIcraft's render time.
    pub render_ms: Option<f64>,
}

/// The monotonic clock of [`VisualNote::render_ms`]: `std::time::Instant`, and in the browser
/// (wasm32, where `std`'s panics) `web-time`'s, over the page's `performance.now()`.
mod clock {
    #[cfg(not(target_arch = "wasm32"))]
    use std::time::Instant;
    #[cfg(target_arch = "wasm32")]
    use web_time::Instant;

    pub fn start() -> Instant {
        Instant::now()
    }

    pub fn elapsed_ms(s: Instant) -> f64 {
        (s.elapsed().as_secs_f64() * 1_000_000.0).round() / 1000.0
    }
}

/// A rendered page.
#[derive(Debug, Clone)]
pub struct PageImage {
    pub page: String,
    pub display_name: String,
    pub width: u32,
    pub height: u32,
    pub scale: f32,
    pub png: Vec<u8>,
    pub visuals: Vec<VisualNote>,
    /// The drawn text runs, when [`RenderOptions::text_dump`] is set.
    pub texts: Vec<TextRun>,
    /// The hit map, when [`RenderOptions::hits`] is set (back to front).
    pub hits: Vec<hits::Hit>,
    /// Premultiplied RGBA pixels (`width` × `height`), when [`RenderOptions::raw`] is set; `png` is
    /// then empty.
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderError {
    NoReport,
    PageNotFound(String),
    TooLarge,
    Encode(String),
    Internal,
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RenderError::NoReport => write!(f, "the snapshot has no report"),
            RenderError::PageNotFound(p) => write!(
                f,
                "no page `{p}` (use a page name, display name or 0-based index)"
            ),
            RenderError::TooLarge => write!(f, "the page is too large to render"),
            RenderError::Encode(e) => write!(f, "cannot encode the PNG: {e}"),
            RenderError::Internal => write!(f, "internal error while rendering"),
        }
    }
}
impl std::error::Error for RenderError {}

/// Decodes image bytes as the renderer draws them (PNG, JPEG, GIF, WebP, SVG; sniffed by
/// content, size-capped, under `catch_unwind`, SVG references outside the bytes ignored) and
/// returns the natural size in CSS pixels. `None`: the renderer would not draw them.
pub fn image_size(bytes: &[u8]) -> Option<(f32, f32)> {
    let img = imgs::decode(bytes)?;
    let (w, h) = img.size();
    (w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0).then_some((w, h))
}

/// The index of a page by name, display name (case-insensitive) or 0-based index.
pub fn find_page(snapshot: &Value, sel: &str) -> Option<usize> {
    let pages = arr(get(get(snapshot, "report"), "pages"));
    if let Some(i) = pages.iter().position(|p| sk(p, "name") == sel) {
        return Some(i);
    }
    if let Some(i) = pages.iter().position(|p| sk(p, "display_name") == sel) {
        return Some(i);
    }
    if let Some(i) = pages
        .iter()
        .position(|p| sk(p, "display_name").eq_ignore_ascii_case(sel))
    {
        return Some(i);
    }
    sel.trim().parse::<usize>().ok().filter(|i| *i < pages.len())
}

/// Number of pages in the snapshot.
pub fn page_count(snapshot: &Value) -> usize {
    arr(get(get(snapshot, "report"), "pages")).len()
}

/// Renders page `index` of `snapshot` to PNG.
pub fn render_page(
    snapshot: &Value,
    index: usize,
    assets: &dyn Assets,
    fonts: &FontBook,
    opts: &RenderOptions,
) -> Result<PageImage, RenderError> {
    match catch_unwind(AssertUnwindSafe(|| {
        render_inner(snapshot, index, assets, fonts, opts)
    })) {
        Ok(r) => r,
        Err(_) => Err(RenderError::Internal),
    }
}

pub use crate::pdf::{PdfConformance, PdfFont};

/// One page of a PDF ([`render_pdf`]).
#[derive(Debug, Clone)]
pub struct PdfPageNote {
    pub index: usize,
    pub page: String,
    pub display_name: String,
    /// The PDF page's size in points (report pixels × 0.75).
    pub width_pt: f32,
    pub height_pt: f32,
    pub visuals: Vec<VisualNote>,
    /// Parts placed as images because their drawing is too large to record as vectors (a visual
    /// at a time; the whole page when the page itself is too large).
    pub raster_parts: usize,
}

/// A PDF of report pages.
#[derive(Debug, Clone)]
pub struct PdfDocument {
    pub bytes: Vec<u8>,
    pub pages: Vec<PdfPageNote>,
    pub fonts: Vec<PdfFont>,
}

/// Renders pages `pages` (indexes, in order) of `snapshot` to one PDF titled `title`: one PDF
/// page per report page at its size, vector paths and real text with embedded font subsets
/// (`docs/render.md`, "PDF"). `opts.scale` is the resolution of the images and of the parts
/// placed as images (2 when 1 or less is given; at most 4).
pub fn render_pdf(
    snapshot: &Value,
    pages: &[usize],
    assets: &dyn Assets,
    fonts: &FontBook,
    opts: &RenderOptions,
    title: &str,
) -> Result<PdfDocument, RenderError> {
    render_pdf_as(
        snapshot,
        pages,
        assets,
        fonts,
        opts,
        title,
        PdfConformance::default(),
    )
}

/// [`render_pdf`], identified as `conf` says (PDF/UA-1, PDF/A-2b).
pub fn render_pdf_as(
    snapshot: &Value,
    pages: &[usize],
    assets: &dyn Assets,
    fonts: &FontBook,
    opts: &RenderOptions,
    title: &str,
    conf: PdfConformance,
) -> Result<PdfDocument, RenderError> {
    pdf_with_cap(
        snapshot,
        pages,
        assets,
        fonts,
        opts,
        &PdfJob {
            title,
            cap: canvas::MAX_VECTOR_OPS,
            conf,
        },
    )
}

/// What a PDF is written as: its title, the most vector operations per canvas, what it
/// claims to conform to.
struct PdfJob<'a> {
    title: &'a str,
    cap: usize,
    conf: PdfConformance,
}

/// [`render_pdf`] as `job` says.
fn pdf_with_cap(
    snap: &Value,
    pages: &[usize],
    assets: &dyn Assets,
    fonts: &FontBook,
    opts: &RenderOptions,
    job: &PdfJob<'_>,
) -> Result<PdfDocument, RenderError> {
    match catch_unwind(AssertUnwindSafe(|| {
        pdf_inner(snap, pages, assets, fonts, opts, job)
    })) {
        Ok(r) => r,
        Err(_) => Err(RenderError::Internal),
    }
}

fn pdf_inner(
    snap: &Value,
    pages: &[usize],
    assets: &dyn Assets,
    fonts: &FontBook,
    opts: &RenderOptions,
    job: &PdfJob<'_>,
) -> Result<PdfDocument, RenderError> {
    let _vector = canvas::VectorScope::with_cap(job.cap);
    let scale = if opts.scale <= 1.0 {
        2.0
    } else {
        opts.scale.min(4.0)
    };
    let po = RenderOptions {
        scale,
        hits: false,
        text_dump: false,
        raw: true,
        // a file: grayscale text
        lcd_text: false,
        ..opts.clone()
    };
    let mut w = pdf::Writer::new(fonts);
    let mut notes = Vec::new();
    for &i in pages {
        // a canvas side stays under the raster's limit (the page is still drawn in full)
        let page = arr(get(get(snap, "report"), "pages")).get(i);
        let side = page.map_or(1280.0, |p| {
            clampf(nk(p, "width", 1280.0) as f32, 1.0, 20_000.0).max(clampf(
                nk(p, "height", 720.0) as f32,
                1.0,
                20_000.0,
            ))
        });
        let po = RenderOptions {
            scale: po.scale.min(16_000.0 / side),
            ..po.clone()
        };
        let d = draw_page(snap, i, assets, fonts, &po)?;
        let (lw, lh) = (d.cv.w, d.cv.h);
        let (ops, raster_parts) = match d.cv.take_ops() {
            Some((canvas::LayerBody::Vector(ops), _)) => {
                let n = ops
                    .iter()
                    .filter(|o| {
                        matches!(
                            o,
                            canvas::Op::Layer {
                                body: canvas::LayerBody::Raster(_),
                                ..
                            }
                        )
                    })
                    .count();
                (ops, n)
            }
            Some((canvas::LayerBody::Raster(pm), _)) => (
                vec![canvas::Op::Image {
                    img: pm,
                    x: 0.0,
                    y: 0.0,
                    w: lw,
                    h: lh,
                    opacity: 1.0,
                }],
                1,
            ),
            None => (Vec::new(), 0),
        };
        // the page's outline entry and structure element: its display name, else its name
        let label = if d.display_name.trim().is_empty() {
            d.page.clone()
        } else {
            d.display_name.clone()
        };
        w.page(&ops, lw, lh, &label);
        drop(ops);
        notes.push(PdfPageNote {
            index: i,
            page: d.page,
            display_name: d.display_name,
            width_pt: lw * pdf::PT_PER_PX,
            height_pt: lh * pdf::PT_PER_PX,
            visuals: d.notes,
            raster_parts,
        });
    }
    // the document's language: the formatting culture, else the model's (as the text is formatted)
    let lang = opts
        .culture
        .clone()
        .or_else(|| {
            get(snap, "model")
                .get("culture")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .filter(|c| !c.trim().is_empty())
        .unwrap_or_else(|| "en-US".to_string());
    let (bytes, fonts) = w.finish(job.title, &lang, job.conf);
    Ok(PdfDocument {
        bytes,
        pages: notes,
        fonts,
    })
}

/// Visual types drawn; anything else becomes a labelled placeholder.
fn render_kind(t: &str) -> Option<&'static str> {
    Some(match t.to_lowercase().as_str() {
        "card" => "card",
        "cardvisual" => "cardNew",
        "multirowcard" => "multiRow",
        "clusteredcolumnchart" => "col",
        "columnchart" | "stackedcolumnchart" => "colS",
        "ribbonchart" => "ribbon",
        "hundredpercentstackedcolumnchart" => "colP",
        "clusteredbarchart" => "bar",
        "barchart" | "stackedbarchart" => "barS",
        "hundredpercentstackedbarchart" => "barP",
        "linechart" => "line",
        "areachart" => "area",
        "stackedareachart" => "areaS",
        "hundredpercentstackedareachart" => "areaP",
        "lineclusteredcolumncombochart" => "combo",
        "linestackedcolumncombochart" => "comboS",
        "funnel" => "funnel",
        "pagenavigator" => "nav",
        "bookmarknavigator" => "bmnav",
        "treemap" => "treemap",
        "scatterchart" => "scatter",
        "waterfallchart" => "waterfall",
        "listslicer" => "slicerList",
        "advancedslicervisual" => "slicerTiles",
        "textslicer" => "slicerText",
        "piechart" => "pie",
        "donutchart" => "donut",
        "tableex" => "table",
        "pivottable" => "matrix",
        "slicer" => "slicer",
        "textbox" => "text",
        "shape" | "basicshape" => "shape",
        "image" => "image",
        "actionbutton" => "button",
        "gauge" => "gauge",
        "kpi" => "kpi",
        "keydriversvisual" => "keyInf",
        "decompositiontreevisual" => "decomp",
        "ganttchart" => "gantt",
        "map" => "mapBubble",
        "filledmap" => "mapFilled",
        "azuremap" => "mapAzure",
        "shapemap" => "mapShape",
        _ => return None,
    })
}

const CHARTS: [&str; 25] = [
    "mapBubble",
    "mapFilled",
    "mapAzure",
    "mapShape",
    "ribbon",
    "col",
    "colS",
    "colP",
    "bar",
    "barS",
    "barP",
    "line",
    "area",
    "areaS",
    "areaP",
    "combo",
    "comboS",
    "pie",
    "donut",
    "gauge",
    "kpi",
    "funnel",
    "treemap",
    "scatter",
    "waterfall",
];
const SM_KINDS: [&str; 12] = [
    "col", "colS", "colP", "bar", "barS", "barP", "line", "area", "areaS", "areaP", "combo", "comboS",
];

struct Images<'a> {
    assets: &'a dyn Assets,
    cache: HashMap<String, Option<Arc<imgs::Image>>>,
}

impl Images<'_> {
    fn get(&mut self, id: &str) -> Option<Arc<imgs::Image>> {
        if let Some(x) = self.cache.get(id) {
            return x.clone();
        }
        let img = self.assets.get(id).and_then(imgs::decode).map(Arc::new);
        if self.cache.len() < 256 {
            self.cache.insert(id.to_string(), img.clone());
        }
        img
    }
}

/// Draws an image layer the way Desktop draws backgrounds: `normal` = natural size centred,
/// `fit` = stretched, `fill` = full width centred.
#[allow(clippy::too_many_arguments)]
fn image_layer(
    cv: &mut Canvas,
    img: &imgs::Image,
    nat: (f32, f32),
    fit: &str,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    alpha: f32,
) {
    let (iw, ih) = if nat.0 > 0.0 && nat.1 > 0.0 {
        nat
    } else {
        img.size()
    };
    if iw <= 0.0 || ih <= 0.0 {
        return;
    }
    let (dw, dh) = match fit {
        "fit" => (w, h),
        "fill" => (w, w * ih / iw),
        _ => (iw, ih),
    };
    let (dx, dy) = (x + (w - dw) / 2.0, y + (h - dh) / 2.0);
    draw_image(cv, img, dx, dy, dw, dh, alpha, Some((x, y, w, h)));
}

/// Draws `img` into the rectangle, clipped to `clip`.
#[allow(clippy::too_many_arguments)]
fn draw_image(
    cv: &mut Canvas,
    img: &imgs::Image,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    alpha: f32,
    clip: Option<(f32, f32, f32, f32)>,
) {
    let s = cv.scale;
    let Some(pm) = img.pixmap((w * s).ceil().max(1.0) as u32, (h * s).ceil().max(1.0) as u32) else {
        return;
    };
    match clip {
        Some((cx, cy, cw, ch)) if x < cx || y < cy || x + w > cx + cw || y + h > cy + ch => {
            // draw on a clipped layer
            let Some(mut layer) = Canvas::new(cw, ch, s) else {
                return;
            };
            layer.image(&pm, x - cx, y - cy, w, h, alpha);
            cv.draw(&layer, cx, cy, 1.0);
        }
        _ => cv.image(&pm, x, y, w, h, alpha),
    }
}

/// A page drawn on its canvas, before encoding.
struct Drawn {
    cv: Canvas,
    page: String,
    display_name: String,
    scale: f32,
    notes: Vec<VisualNote>,
}

fn render_inner(
    snap: &Value,
    index: usize,
    assets: &dyn Assets,
    fonts: &FontBook,
    opts: &RenderOptions,
) -> Result<PageImage, RenderError> {
    let _contrast = canvas::ContrastScope::new(opts.high_contrast);
    let Drawn {
        mut cv,
        page,
        display_name,
        scale,
        notes,
    } = draw_page(snap, index, assets, fonts, opts)?;
    let (png, rgba) = if opts.raw {
        (Vec::new(), cv.pm.data().to_vec())
    } else {
        let png = cv
            .pm
            .encode_png()
            .map_err(|e| RenderError::Encode(e.to_string()))?;
        (png, Vec::new())
    };
    Ok(PageImage {
        page,
        display_name,
        width: cv.pm.width(),
        height: cv.pm.height(),
        scale,
        png,
        visuals: notes,
        texts: if opts.text_dump {
            std::mem::take(&mut cv.texts)
        } else {
            Vec::new()
        },
        hits: std::mem::take(&mut cv.hits),
        rgba,
    })
}

/// The author's grid over a `pw` × `ph` page: a dot centred on every step, none when more than
/// [`MAX_GRID_DOTS`] would be drawn.
fn grid_dots(cv: &mut Canvas, g: &GridDots, pw: f32, ph: f32) {
    let (sx, sy) = g.step;
    if !(sx.is_finite() && sy.is_finite() && g.dot.is_finite()) || sx < 1.0 || sy < 1.0 {
        return;
    }
    let (nx, ny) = ((pw / sx) as usize, (ph / sy) as usize);
    if nx.saturating_add(1).saturating_mul(ny.saturating_add(1)) > MAX_GRID_DOTS {
        return;
    }
    let [r, gr, b, a] = g.rgba;
    let (c, alpha) = (Rgb(r, gr, b), f32::from(a) / 255.0);
    let d = g.dot.clamp(0.01, sx.min(sy));
    for i in 0..=nx {
        for j in 0..=ny {
            let (x, y) = (i as f32 * sx, j as f32 * sy);
            cv.rect(x - d / 2.0, y - d / 2.0, d, d, c, alpha);
        }
    }
}

/// Opacity of the visuals a spotlight dims: Microsoft Learn says the page's other visuals "fade
/// to near transparency" ("Add spotlights to Power BI reports"); the amount is BIcraft's choice,
/// not measured against Power BI.
pub const SPOTLIGHT_OPACITY: f32 = 0.15;

/// Opacity of a disabled button that sets no disabled look of its own (BIcraft's choice).
pub const DISABLED_ALPHA: f32 = 0.45;

fn draw_page(
    snap: &Value,
    index: usize,
    assets: &dyn Assets,
    fonts: &FontBook,
    opts: &RenderOptions,
) -> Result<Drawn, RenderError> {
    let report = get(snap, "report");
    if !report.is_object() {
        return Err(RenderError::NoReport);
    }
    let page = arr(get(report, "pages"))
        .get(index)
        .ok_or_else(|| RenderError::PageNotFound(index.to_string()))?;
    let pw = clampf(nk(page, "width", 1280.0) as f32, 1.0, 20_000.0);
    let ph = clampf(nk(page, "height", 720.0) as f32, 1.0, 20_000.0);
    let mut scale = clampf(opts.scale, 0.05, 8.0);
    let max_px = opts.max_pixels.clamp(1, 200_000_000) as f32;
    if pw * ph * scale * scale > max_px {
        // a little under the cap: the canvas rounds its size up
        scale = (max_px / (pw * ph)).sqrt() * 0.995;
    }
    let mut cv = Canvas::new(pw, ph, scale).ok_or(RenderError::TooLarge)?;
    cv.set_lcd_text(opts.lcd_text);
    let mut ctx = Ctx::new(snap, fonts, opts.culture.as_deref());
    ctx.pointer = opts.pointer.clone();
    ctx.gantt = opts.gantt.clone();
    let ctx = ctx;
    ctx.ix.scope.borrow_mut().on = opts.hits;
    let mut images = Images {
        assets,
        cache: HashMap::new(),
    };

    // wallpaper, page background colour and image
    cv.fill(Rgb::WHITE, 1.0);
    let wp = get(page, "wallpaper");
    let wp_alpha = opacity_of(get(wp, "transparency"));
    if let Some(c) = Rgb::from_value(get(wp, "color")) {
        cv.fill(c.mix(Rgb::WHITE, 1.0 - wp_alpha), 1.0);
    }
    if let Some(r) = img_of(get(wp, "image"))
        && let Some(img) = images.get(&r.id)
    {
        image_layer(
            &mut cv,
            &img,
            (r.w, r.h),
            fit_of(sk(wp, "fit")),
            0.0,
            0.0,
            pw,
            ph,
            wp_alpha,
        );
    }
    let page_bg = Rgb::from_value(get(page, "background")).unwrap_or(ctx.bg);
    let bg_alpha = opacity_of(get(page, "background_transparency"));
    cv.rect(0.0, 0.0, pw, ph, page_bg, bg_alpha);
    let bi = get(page, "background_image");
    if let Some(r) = img_of(get(bi, "image"))
        && let Some(img) = images.get(&r.id)
    {
        image_layer(
            &mut cv,
            &img,
            (r.w, r.h),
            fit_of(sk(bi, "fit")),
            0.0,
            0.0,
            pw,
            ph,
            opacity_of(get(bi, "transparency")),
        );
    }
    if let Some(g) = &opts.grid {
        grid_dots(&mut cv, g, pw, ph);
    }

    // visuals, back to front
    let visuals: Vec<&Value> = arr(get(page, "visuals"))
        .iter()
        .filter(|v| v.is_object())
        .collect();
    let groups: std::collections::HashSet<&str> = visuals
        .iter()
        .map(|v| sk(v, "parent_group"))
        .filter(|s| !s.is_empty())
        .collect();
    // A hidden group hides its members: the visuals under it are not drawn either.
    let parents: std::collections::HashMap<&str, &str> = visuals
        .iter()
        .filter_map(|v| {
            let p = sk(v, "parent_group");
            (!p.is_empty()).then_some((sk(v, "name"), p))
        })
        .collect();
    let hidden_groups: std::collections::HashSet<&str> = visuals
        .iter()
        .filter(|v| get(v, "hidden") == &Value::Bool(true) && groups.contains(sk(v, "name")))
        .map(|v| sk(v, "name"))
        .collect();
    let hidden_by_group = |v: &Value| {
        let mut cur = parents.get(sk(v, "name")).copied();
        for _ in 0..32 {
            let Some(p) = cur else { return false };
            if hidden_groups.contains(p) {
                return true;
            }
            cur = parents.get(p).copied();
        }
        false
    };
    let mut order: Vec<(usize, &Value)> = visuals.iter().copied().enumerate().collect();
    order.sort_by(|a, b| {
        nk(a.1, "z", 0.0)
            .partial_cmp(&nk(b.1, "z", 0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    let skip = order.len().saturating_sub(500);
    let shown = |v: &Value| {
        let is_group = (v.get("type").is_some_and(Value::is_null)) || groups.contains(sk(v, "name"));
        !(is_group || get(v, "hidden") == &Value::Bool(true) || hidden_by_group(v))
    };
    // Spotlight (the reader's `interaction.spotlight`): the page's other visuals fade, when the
    // spotlight visual is drawn on this page
    let spot = get(get(snap, "interaction"), "spotlight");
    let spotlight = (sk(spot, "page") == sk(page, "name") && !sk(spot, "visual").is_empty())
        .then(|| sk(spot, "visual"))
        .filter(|name| visuals.iter().any(|v| sk(v, "name") == *name && shown(v)));
    let mut notes = Vec::new();
    for (_, v) in order.into_iter().skip(skip) {
        if !shown(v) {
            continue;
        }
        let opacity = match spotlight {
            Some(name) if sk(v, "name") != name => SPOTLIGHT_OPACITY,
            _ => 1.0,
        };
        let start = clock::start();
        // the visual's part of the PDF's structure tree: its title (or automatic title, or type)
        // and its alt text
        let title = [sk(v, "title"), sk(v, "auto_title"), sk(v, "type")]
            .into_iter()
            .find(|t| !t.trim().is_empty())
            .unwrap_or("");
        let alt = Some(sk(v, "alt_text")).filter(|a| !a.trim().is_empty());
        // a picture-like visual without alt text is still a figure, labelled with its title
        // (text boxes, tables, slicers, buttons and cards are read as their text)
        let textual = matches!(
            render_kind(sk(v, "type")),
            Some(
                "text"
                    | "table"
                    | "matrix"
                    | "slicer"
                    | "slicerList"
                    | "slicerTiles"
                    | "slicerText"
                    | "nav"
                    | "bmnav"
                    | "button"
                    | "shape"
                    | "card"
                    | "cardNew"
                    | "multiRow"
            )
        );
        let label = [sk(v, "title"), sk(v, "auto_title")]
            .into_iter()
            .find(|t| !t.trim().is_empty())
            .filter(|_| !textual);
        // the reading order: the visuals' tab order (the keyboard's), else top to bottom
        // and left to right
        let order = match get(v, "tab_order").as_f64() {
            Some(t) => (0.0, t as f32, 0.0),
            None => (1.0, nk(v, "y", 0.0) as f32, nk(v, "x", 0.0) as f32),
        };
        cv.begin_part(canvas::Part::Visual {
            title: title.to_string(),
            alt: alt.map(str::to_string),
            label: label.map(str::to_string),
            order,
        });
        let mut note = draw_visual(&mut cv, v, &ctx, &mut images, opts, sk(page, "name"), opacity);
        cv.end_part();
        note.render_ms = Some(clock::elapsed_ms(start));
        notes.push(note);
    }
    Ok(Drawn {
        cv,
        page: sk(page, "name").to_string(),
        display_name: sk(page, "display_name").to_string(),
        scale,
        notes,
    })
}

fn font_from(spec: &data::FontSpec, ctx: &Ctx<'_>, size: f32, default_weight: u16) -> Font {
    Font::new(spec.family.as_deref().unwrap_or(&ctx.font), size)
        .weight(spec.weight.unwrap_or(default_weight))
        .italic(spec.italic)
}

fn draw_header_line(
    cv: &mut Canvas,
    ctx: &Ctx<'_>,
    t: &data::TitleF,
    text: &str,
    y: f32,
    h: f32,
    left: f32,
    right: f32,
    default_weight: u16,
) {
    let f = font_from(&t.font, ctx, t.size, default_weight);
    let s = ctx.fonts.fit(&f, text, (right - left).max(0.0));
    let (x, a) = match t.align {
        data::Align::Left => (left, Anchor::Start),
        data::Align::Center => ((left + right) / 2.0, Anchor::Middle),
        data::Align::Right => (right, Anchor::End),
    };
    cv.text(
        ctx.fonts,
        &Text::new(&s, f, t.color)
            .anchor(a)
            .base(Base::Central)
            .underline(t.font.underline),
        x,
        y + h / 2.0,
    );
}

fn draw_visual(
    page: &mut Canvas,
    v: &Value,
    ctx: &Ctx<'_>,
    images: &mut Images<'_>,
    opts: &RenderOptions,
    page_name: &str,
    opacity: f32,
) -> VisualNote {
    let typ = sk(v, "type").to_string();
    let mut note = VisualNote {
        name: sk(v, "name").to_string(),
        visual_type: typ.clone(),
        status: "drawn",
        note: None,
        data: data_state(v),
        render_ms: None,
    };
    let (pw, ph) = (page.w, page.h);
    let w = clampf(nk(v, "width", 1.0) as f32, 1.0, (pw * 4.0).max(1.0));
    let h = clampf(nk(v, "height", 1.0) as f32, 1.0, (ph * 4.0).max(1.0));
    let (x, y) = (nk(v, "x", 0.0) as f32, nk(v, "y", 0.0) as f32);
    if x >= pw || y >= ph || x + w <= 0.0 || y + h <= 0.0 {
        note.status = "skipped";
        note.note = Some("outside the page".into());
        return note;
    }
    let Some(mut cv) = page.layer(x, y, w, h) else {
        note.status = "skipped";
        note.note = Some("too large".into());
        return note;
    };
    ctx.ix.enter(page_name, v);
    background_hit(&mut cv, ctx, v, w, h);
    let kind = render_kind(&typ);
    let kname = kind.unwrap_or("other");
    let mut f = data::vis_format(v, kname, ctx);
    if kind.is_none()
        && !get(v, "spec").is_object()
        && !get(v, "chart_pack").is_object()
        && !get(v, "bicraft_visual").is_object()
    {
        // not drawn: the container (background, border, title) and a labelled box; BIcraft's
        // own visuals are drawn and keep their default title (seen in Desktop 2.159.1231.0 over
        // all nine, 2026-11-24)
        f.title.show = false;
    }
    if kind == Some("keyInf") && sk(v, "title").is_empty() {
        // Desktop draws no title over a key influencers visual unless it has one
        f.title.show = false;
    }
    // background, border
    let radius = f.border.2;
    if f.bg_show {
        cv.round_rect(0.0, 0.0, w, h, radius, Some((f.bg, f.bg_alpha)), None);
        if let Some(r) = &f.bg_img
            && let Some(img) = images.get(&r.id)
        {
            image_layer(&mut cv, &img, (r.w, r.h), f.bg_fit, 0.0, 0.0, w, h, f.bg_alpha);
        }
    }
    if f.border.0 {
        cv.round_rect(
            0.5,
            0.5,
            w - 1.0,
            h - 1.0,
            radius,
            None,
            Some((f.border.1, 1.0, None)),
        );
    }
    // container padding: the report's (or theme's), else the viewer's defaults
    let (pt, pb, pl, pr) = f.padding.unwrap_or_else(|| {
        let p = if CHARTS.contains(&kname) {
            if ctx.classic { 5.0 } else { 8.0 }
        } else if matches!(kname, "shape" | "image" | "button") {
            0.0
        } else {
            5.0
        };
        (if f.title.show { 0.0 } else { p }, p, p, p)
    });
    let mut top = 0.0f32;
    let mut hy = pt;
    if f.title.show {
        // a title from a field (conditional formatting) comes first
        let t = if !f.title.text.is_empty() {
            f.title.text.clone()
        } else if sk(v, "title").is_empty() {
            auto_title(v, &ctx.loc)
        } else {
            sk(v, "title").to_string()
        };
        if !t.is_empty() {
            let th = (f.title.size * 1.42).round();
            draw_header_line(
                &mut cv,
                ctx,
                &f.title,
                &data::clip(&t, 300),
                hy,
                th,
                pl,
                w - pr,
                600,
            );
            hy += th;
            top = hy;
        }
    }
    if f.sub.show {
        // an empty subtitle shows the automatic title ("Sales by Region") on charts, as Desktop
        // does; tables, cards and slicers have none
        let text = if f.sub.text.is_empty() && CHARTS.contains(&kname) {
            auto_title(v, &ctx.loc)
        } else if f.sub.text.is_empty() {
            String::new()
        } else {
            f.sub.text.clone()
        };
        if !text.is_empty() {
            let sh = (f.sub.size * 1.4 + 2.0).round();
            let sub = f.sub.clone();
            draw_header_line(
                &mut cv,
                ctx,
                &sub,
                &data::clip(&text, 300),
                hy,
                sh,
                pl,
                w - pr,
                400,
            );
            hy += sh;
            top = hy;
        }
    }
    if f.divider.0 && top > 0.0 {
        let (_, c, width, style) = &f.divider;
        let dash = match style.as_str() {
            "dashed" => Some((width * 3.0, width * 3.0)),
            "dotted" => Some((*width, *width)),
            _ => None,
        };
        cv.line(pl, top + width / 2.0, w - pr, top + width / 2.0, *c, *width, dash);
        top += width + 2.0;
    }
    // the body starts under the header (with Desktop's gap) or at the top padding
    // (classic charts: Desktop 2.158 starts the plot area 4 px under a 17 px title)
    // BIcraft's own visuals: Desktop 2.159.1231.0 places their frame 21 px down under a classic
    // title, as a classic chart's plot (2026-11-24)
    let own = get(v, "spec").is_object()
        || get(v, "chart_pack").is_object()
        || get(v, "bicraft_visual").is_object();
    let gap = if ctx.classic && (CHARTS.contains(&kname) || own) {
        4.0
    } else {
        8.0
    };
    let by = if top > 0.0 { top + gap } else { pt };
    let b = Bx {
        w: w - pl - pr,
        h: h - by - pb,
    };
    f.titled = top > 0.0;
    let mut body_alpha = 1.0;
    let mut empty_prompt = false;
    // a button or shape in its hover, press or disabled state: the state's formatting over the
    // default one; a disabled button without a look of its own is dimmed
    let state = matches!(kind, Some("button" | "shape"))
        .then(|| states::visual_state(v, ctx))
        .flatten();
    let staged = state.and_then(|st| {
        let key = if kind == Some("shape") { "shape" } else { "button" };
        states::with_state(v, key, st)
    });
    if state == Some("disabled") && !states::has_disabled_look(v) {
        body_alpha = DISABLED_ALPHA;
    }
    let v = staged.as_ref().unwrap_or(v);
    if b.w >= 4.0
        && b.h >= 4.0
        && let Some(mut body) = cv.layer(pl, by, b.w, b.h)
    {
        match kind {
            // the BIcraft Spec visual: its declarative spec drawn as its TypeScript draws it in
            // Power BI (docs/formats/spec-visual.md)
            None if get(v, "spec").is_object() => {
                let ds = data::dataset(v, "spec", ctx);
                match spec::spec_body(&mut body, v, &ds, b, ctx) {
                    Ok(()) if ds.fake => {
                        note.status = "preview";
                        note.note = ds.reason.clone();
                        body_alpha = 0.5;
                    }
                    Ok(()) => {}
                    Err(e) => {
                        bodies::placeholder(&mut body, v, b, ctx);
                        note.status = "placeholder";
                        note.note = Some(format!("BIcraft Spec visual: {}", data::clip(&e, 300)));
                    }
                }
            }
            // BIcraft's chart pack (organization chart, Venn, Marimekko, calendar heatmap): drawn
            // as their TypeScript draws them in Power BI (docs/formats/chart-pack.md)
            None if get(v, "chart_pack").is_object() => {
                let ds = data::dataset(v, "chart_pack", ctx);
                match chart_pack::body(&mut body, v, &ds, b, ctx) {
                    Ok(()) if ds.fake => {
                        note.status = "preview";
                        note.note = ds.reason.clone();
                        body_alpha = 0.5;
                    }
                    Ok(()) => {}
                    Err(e) => {
                        bodies::placeholder(&mut body, v, b, ctx);
                        note.status = "placeholder";
                        note.note = Some(format!("BIcraft visual: {}", data::clip(&e, 300)));
                    }
                }
            }
            // BIcraft's own chart visuals (histogram, bullet, box plot, dumbbell and slope;
            // docs/formats/bicraft-visuals.md), laid out as their TypeScript lays them out
            None if get(v, "bicraft_visual").is_object() => {
                let ds = data::dataset(v, "bicraft_visual", ctx);
                match bvis::body(&mut body, v, &ds, b, ctx) {
                    Ok(()) if ds.fake => {
                        note.status = "preview";
                        note.note = ds.reason.clone();
                        body_alpha = 0.5;
                    }
                    Ok(()) => {}
                    Err(e) => {
                        bodies::placeholder(&mut body, v, b, ctx);
                        note.status = "placeholder";
                        note.note = Some(format!("BIcraft visual: {}", data::clip(&e, 300)));
                    }
                }
            }
            None if get(v, "custom_visual").is_object() => {
                let cvis = get(v, "custom_visual");
                let icon = img_of(get(cvis, "icon")).and_then(|r| images.get(&r.id));
                custom_visual_placeholder(&mut body, v, cvis, icon.as_deref(), b, ctx);
                note.status = "placeholder";
                note.note = Some(format!(
                    "custom visual `{typ}`: BIcraft never runs its code; drawn as its icon and name"
                ));
            }
            None => {
                bodies::placeholder(&mut body, v, b, ctx);
                note.status = "placeholder";
                note.note = Some(format!("`{typ}` is drawn as a labelled box"));
            }
            Some("text") => bodies::text_body(&mut body, v, b, ctx),
            Some("button") if get(v, "button").is_object() => {
                // the state's fill image and custom icon (snapshot `button.fill_image`,
                // `button.icon_image`)
                let bt = get(v, "button");
                let fill = img_of(get(bt, "fill_image")).and_then(|r| images.get(&r.id));
                let icon = img_of(get(bt, "icon_image")).and_then(|r| images.get(&r.id));
                let imgs = bodies::ButtonImages {
                    fill: fill.as_deref(),
                    icon: icon.as_deref(),
                };
                bodies::button_body(&mut body, v, b, ctx, imgs);
                // its shadow and glow, under the visual on the page: they reach past its box
                bodies::button_effects(page, v, x + pl, y + by, b, ctx);
            }
            Some("button") => button_fallback(&mut body, v, b, ctx),
            Some("image") => match img_of(get(v, "image")).and_then(|r| images.get(&r.id).map(|i| (r, i))) {
                Some((_, img)) => draw_image_visual(&mut body, &img, sk(get(v, "image"), "scaling"), b),
                None => bodies::image_placeholder(&mut body, b, ctx),
            },
            Some("nav") => bodies::nav_body(&mut body, v, b, ctx, page_name),
            Some("bmnav") => bodies::bookmark_nav_body(&mut body, v, b, ctx, page_name),
            Some("keyInf") => {
                if keyinf::key_influencers_body(&mut body, v, b, ctx) {
                    note.status = "estimate";
                    note.note = Some(
                        "BIcraft's own estimate, not Power BI's result (docs/key-influencers.md)".into(),
                    );
                } else {
                    note.status = "placeholder";
                    note.note = Some("key influencers are computed by Power BI: not reproduced".into());
                }
            }
            Some("shape") => shapes::shape_body(&mut body, v, b, ctx),
            // the Gantt (docs/formats/gantt.md): the task list and its time line
            Some("gantt") => {
                let ds = data::dataset(v, "gantt", ctx);
                if ds.empty {
                    charts::blank_visual(&mut body, ctx, b, charts::ICON_BAR);
                    note.status = "blank";
                    empty_prompt = true;
                } else {
                    if ds.fake {
                        note.status = "preview";
                        note.note = ds.reason.clone();
                        body_alpha = 0.5;
                    } else if ds.note.is_some() {
                        note.note = ds.note.clone().map(|n| format!("filters ignored: {n}"));
                    }
                    match gantt::gantt_body(&mut body, v, &ds, b, ctx) {
                        None => {
                            bodies::placeholder(&mut body, v, b, ctx);
                            note.status = "placeholder";
                            note.note = Some("a Gantt needs a field in Task".into());
                        }
                        Some(cut) if cut > 0 && note.note.is_none() => {
                            note.note = Some(format!(
                                "{cut} rows outside the visual are not drawn (the reader scrolls to them)"
                            ));
                        }
                        Some(_) => {}
                    }
                }
            }
            // maps, drawn offline (docs/formats/maps.md): bubbles at their latitude and
            // longitude, a shape map's custom areas, else a placeholder saying why
            Some(k) if maps::MAP_KINDS.contains(&k) => {
                let ds = data::dataset(v, k, ctx);
                let out = if k == "mapShape" {
                    maps::shape_map(&mut body, v, &ds, b, &f, ctx)
                } else {
                    maps::bubble_map(&mut body, &ds, b, &f, ctx, k)
                };
                note.status = out.status;
                note.note = out.note;
                if ds.fake && out.status == "drawn" {
                    note.status = "preview";
                    note.note = ds.reason.clone();
                } else if out.status == "blank" {
                    empty_prompt = v
                        .get("fields")
                        .and_then(Value::as_array)
                        .is_none_or(Vec::is_empty);
                }
            }
            Some(k) => {
                let ds = data::dataset(v, k, ctx);
                if ds.empty {
                    charts::blank_visual(&mut body, ctx, b, charts::ICON_BAR);
                    note.status = "blank";
                    empty_prompt = v
                        .get("fields")
                        .and_then(Value::as_array)
                        .is_none_or(Vec::is_empty);
                } else {
                    if ds.fake {
                        note.status = "preview";
                        note.note = ds.reason.clone();
                        body_alpha = 0.5;
                    } else if let Some(u) = &ds.unformatted {
                        // conditional formatting reading a measure the engine could not compute
                        note.status = "preview";
                        note.note = Some(format!("conditional formatting not applied: {u}"));
                    } else if ds.note.is_some() {
                        note.note = ds.note.clone().map(|n| format!("filters ignored: {n}"));
                    } else if k == "line" && f.lines.iter().any(|l| sk(l, "kind") == "anomaly_detection") {
                        note.note = Some(
                            "anomalies and their expected range are BIcraft's own estimate, not Power BI's result"
                                .into(),
                        );
                    }
                    match f.sm {
                        Some(_) if !ds.fake && SM_KINDS.contains(&k) => {
                            // no point selection in small-multiple panels (the viewer's `IXB.off`)
                            ctx.ix.set_off(true);
                            small_multiples(&mut body, &ds, b, &f, ctx, k);
                            ctx.ix.set_off(false);
                        }
                        _ => draw_body(&mut body, &ds, b, &f, ctx, v, k),
                    }
                }
            }
        }
        // a button, image or shape with an action the reader runs (navigation, Back, drill through,
        // web URL, Apply / Clear all slicers…)
        let ib = get(get(v, "interaction"), "button");
        if matches!(kind, Some("button" | "image" | "shape")) && !sk(ib, "kind").is_empty() {
            let mut a = serde_json::json!({"action": "button", "page": page_name, "visual": sk(v, "name")});
            // a conditional (fx) destination: the value the snapshot evaluated
            if get(ib, "conditional") == &Value::Bool(true) && !sk(ib, "destination").is_empty() {
                a["destination"] = serde_json::json!(sk(ib, "destination"));
            }
            // a disabled button (a drillthrough button without a selection) takes no click
            let action = if get(ib, "enabled") == &Value::Bool(false) {
                hits::HitAction::None
            } else {
                ctx.ix.plain(a)
            };
            let tip: Vec<(String, String)> = Some(sk(ib, "tooltip"))
                .filter(|t| !t.is_empty())
                .map(|t| vec![(String::new(), data::clip(t, 500))])
                .unwrap_or_default();
            if action != hits::HitAction::None || !tip.is_empty() {
                body.hit(ctx.ix.hit(
                    hits::HitShape::Rect {
                        x: 0.0,
                        y: 0.0,
                        w: b.w,
                        h: b.h,
                    },
                    action,
                    tip,
                ));
            }
        }
        cv.draw(&body, pl, by, body_alpha);
    }
    if empty_prompt {
        empty_visual_prompt(&mut cv, ctx, w);
    }
    // a measure-bound text, fill or title the engine could not compute: not what Desktop shows
    let unbound = sk(v, "preview_reason");
    if note.status == "drawn" && !unbound.is_empty() {
        note.status = "preview";
        note.note = Some(data::clip(unbound, 600));
    }
    if opts.badges
        && (note.status == "preview"
            || note
                .note
                .as_deref()
                .is_some_and(|n| n.starts_with("filters ignored")))
    {
        let label = if note.status == "preview" {
            "no data · preview"
        } else {
            "filters ignored"
        };
        let f = Font::new(&ctx.font, 9.0 * PT * 0.8);
        let tw = ctx.fonts.measure(&f, label);
        let (bw, bh) = (tw + 10.0, 14.0);
        let (bx, by) = (w - bw - 3.0, h - bh - 3.0);
        if bx > 0.0 && by > 0.0 {
            cv.round_rect(bx, by, bw, bh, 7.0, Some((Rgb(0x60, 0x5e, 0x5c), 0.85)), None);
            cv.text(
                ctx.fonts,
                &Text::new(label, f, Rgb::WHITE)
                    .anchor(Anchor::Middle)
                    .base(Base::Central),
                bx + bw / 2.0,
                by + bh / 2.0,
            );
        }
    }
    page.draw(&cv, x, y, opacity);
    note
}

/// The visual's own area in the hit map: it catches clicks like the viewer's visual box (a
/// click there reaches no visual below); a click on a visual that is the source of a selection
/// clears it (Desktop; the viewer's background click).
fn background_hit(cv: &mut Canvas, ctx: &Ctx<'_>, v: &Value, w: f32, h: f32) {
    if !ctx.ix.on() {
        return;
    }
    let (page, visual) = ctx.ix.page_visual();
    let action = if get(get(v, "interaction"), "selected").is_object() {
        hits::HitAction::Send {
            action: serde_json::json!({"action": "select", "page": page, "visual": visual, "point": null}),
            ctrl: false,
        }
    } else {
        hits::HitAction::None
    };
    cv.hit(Some(hits::Hit {
        visual,
        shape: hits::HitShape::Rect { x: 0.0, y: 0.0, w, h },
        action,
        tip: Vec::new(),
        item: None,
    }));
}

fn draw_body(cv: &mut Canvas, ds: &data::Dataset, b: Bx, f: &Fmt, ctx: &Ctx<'_>, v: &Value, k: &str) {
    let bar = |horiz, stacked, pct| BarOpts { horiz, stacked, pct };
    let line = |area, stacked, pct, combo| LineOpts {
        area,
        stacked,
        pct,
        combo,
    };
    match k {
        "card" => bodies::card_body(cv, ds, b, f, ctx),
        "cardNew" => bodies::card_new_body(cv, ds, b, f, ctx),
        "multiRow" => bodies::multi_row_body(cv, ds, b, ctx),
        "table" => tables::table_body(cv, ds, b, f, ctx),
        "matrix" => tables::matrix_body(cv, ds, b, f, ctx),
        "slicer" => slicers::slicer_v2(cv, ds, b, f, ctx, v, "slicer"),
        "slicerList" => slicers::slicer_v2(cv, ds, b, f, ctx, v, "list"),
        "slicerTiles" => slicers::slicer_v2(cv, ds, b, f, ctx, v, "tiles"),
        "slicerText" => slicers::slicer_v2(cv, ds, b, f, ctx, v, "text"),
        "col" => charts::bar_chart(cv, ds, b, f, ctx, bar(false, false, false)),
        "colS" => charts::bar_chart(cv, ds, b, f, ctx, bar(false, true, false)),
        "colP" => charts::bar_chart(cv, ds, b, f, ctx, bar(false, true, true)),
        "bar" => charts::bar_chart(cv, ds, b, f, ctx, bar(true, false, false)),
        "barS" => charts::bar_chart(cv, ds, b, f, ctx, bar(true, true, false)),
        "barP" => charts::bar_chart(cv, ds, b, f, ctx, bar(true, true, true)),
        "line" => charts::line_chart(cv, ds, b, f, ctx, line(false, false, false, false)),
        "area" => charts::line_chart(cv, ds, b, f, ctx, line(true, false, false, false)),
        "areaS" => charts::line_chart(cv, ds, b, f, ctx, line(true, true, false, false)),
        "areaP" => charts::line_chart(cv, ds, b, f, ctx, line(true, true, true, false)),
        "combo" => charts::line_chart(cv, ds, b, f, ctx, line(false, false, false, true)),
        "comboS" => charts::line_chart(cv, ds, b, f, ctx, line(false, true, false, true)),
        "pie" => parts::pie_chart(cv, ds, b, f, ctx, false),
        "donut" => parts::pie_chart(cv, ds, b, f, ctx, true),
        "gauge" => parts::gauge_chart(cv, ds, b, f, ctx),
        "kpi" => parts::kpi_chart(cv, ds, b, f, ctx),
        "funnel" => parts::funnel_chart(cv, ds, b, f, ctx),
        "waterfall" => charts::waterfall_chart(cv, ds, b, f, ctx),
        "ribbon" => ribbon::ribbon_chart(cv, ds, b, f, ctx),
        "treemap" => parts::treemap_chart(cv, ds, b, f, ctx),
        "scatter" => charts::scatter_chart(cv, ds, b, f, ctx),
        "decomp" => decomp::decomp_body(cv, v, ds, b, ctx),
        _ => bodies::placeholder(cv, v, b, ctx),
    }
}

/// A custom visual (the snapshot's `custom_visual`): BIcraft never runs a visual's JavaScript,
/// so it draws the labelled box of [`bodies::placeholder`] with the package's icon above its
/// display name (docs/render.md, "Custom visuals").
fn custom_visual_placeholder(
    cv: &mut Canvas,
    v: &Value,
    cvis: &Value,
    icon: Option<&imgs::Image>,
    b: Bx,
    ctx: &Ctx<'_>,
) {
    let name = match sk(cvis, "display_name") {
        "" => sk(cvis, "name"),
        d => d,
    };
    let lines = [name.to_string(), sk(v, "title").to_string()];
    bodies::placeholder_frame(cv, b, ctx);
    // the icon at most 32 px, when the box has room for it above the lines
    let k = (b.w.min(b.h) * 0.3).min(32.0);
    let text_h = ctx.px * 3.2;
    let icon = icon.filter(|img| {
        let (iw, ih) = img.size();
        iw > 0.0 && ih > 0.0 && k >= 8.0 && b.h >= k + text_h + 4.0
    });
    let Some(img) = icon else {
        bodies::placeholder_lines(cv, &lines, b, ctx);
        return;
    };
    let (iw, ih) = img.size();
    let s = (k / iw).min(k / ih);
    let (w, h) = (iw * s, ih * s);
    let top = (b.h - k - text_h) / 2.0;
    draw_image(
        cv,
        img,
        (b.w - w) / 2.0,
        top + (k - h) / 2.0,
        w,
        h,
        1.0,
        Some((0.0, 0.0, b.w, b.h)),
    );
    let rest_h = (b.h - top - k).max(0.0);
    if let Some(mut rest) = cv.layer(0.0, top + k, b.w, rest_h) {
        let rb = Bx {
            w: b.w,
            h: text_h.min(rest_h),
        };
        bodies::placeholder_lines(&mut rest, &lines, rb, ctx);
        cv.draw(&rest, 0.0, top + k, 1.0);
    }
}

/// Image visuals: `normal` keeps the aspect ratio inside the box (Desktop's "Fit"), `fit`
/// stretches ("Stretch"), `fill` covers.
fn draw_image_visual(cv: &mut Canvas, img: &imgs::Image, scaling: &str, b: Bx) {
    draw_image_in(cv, img, scaling, 0.0, 0.0, b.w, b.h);
}

/// Draws `img` in the `bw` × `bh` box at (`bx`, `by`) as the image visual scales it: `fit`
/// stretched, `fill` covering the box (cut to it), else (`normal`) whole and centred.
fn draw_image_in(cv: &mut Canvas, img: &imgs::Image, scaling: &str, bx: f32, by: f32, bw: f32, bh: f32) {
    let (iw, ih) = img.size();
    if iw <= 0.0 || ih <= 0.0 || bw <= 0.0 || bh <= 0.0 {
        return;
    }
    let (w, h) = match fit_of(scaling) {
        "fit" => (bw, bh),
        "fill" => {
            let k = (bw / iw).max(bh / ih);
            (iw * k, ih * k)
        }
        _ => {
            let k = (bw / iw).min(bh / ih);
            (iw * k, ih * k)
        }
    };
    draw_image(
        cv,
        img,
        bx + (bw - w) / 2.0,
        by + (bh - h) / 2.0,
        w,
        h,
        1.0,
        Some((bx, by, bw, bh)),
    );
}

/// A button without button details: Desktop's blank button (a 2 px grey outline) and the
/// visual's title, if any.
fn button_fallback(cv: &mut Canvas, v: &Value, b: Bx, ctx: &Ctx<'_>) {
    cv.round_rect(
        1.0,
        1.0,
        (b.w - 2.0).max(0.0),
        (b.h - 2.0).max(0.0),
        0.0,
        None,
        Some((Rgb(0x66, 0x66, 0x66), 2.0, None)),
    );
    let t = data::clip(sk(v, "title"), 60);
    if !t.is_empty() {
        let px = ctx.px;
        let s = charts::fit(ctx, &t, b.w - px, px);
        charts::tx(cv, ctx, b.w / 2.0, b.h / 2.0, &s, px, ctx.fg2, Anchor::Middle);
    }
}

/// Small multiples: one panel per value of the split column, each drawn by the chart's renderer.
fn small_multiples(cv: &mut Canvas, ds: &data::Dataset, b: Bx, f: &Fmt, ctx: &Ctx<'_>, k: &str) {
    let Some((ci, sm_rows, sm_cols)) = f.sm else {
        return;
    };
    if ci >= ds.cols.len() {
        return draw_body(cv, ds, b, f, ctx, &Value::Null, k);
    }
    let mut groups: Vec<(Value, Vec<Vec<Value>>, Vec<usize>)> = Vec::new();
    let mut at: HashMap<String, usize> = HashMap::new();
    for (ri, r) in ds.rows.iter().enumerate() {
        let x = r.get(ci).cloned().unwrap_or(Value::Null);
        let key = key_of(&x);
        let gi = match at.get(&key) {
            Some(g) => *g,
            None => {
                if groups.len() >= 36 {
                    continue;
                }
                groups.push((x, Vec::new(), Vec::new()));
                at.insert(key, groups.len() - 1);
                groups.len() - 1
            }
        };
        if let Some(g) = groups.get_mut(gi) {
            g.1.push(
                r.iter()
                    .enumerate()
                    .filter(|(i, _)| *i != ci)
                    .map(|(_, c)| c.clone())
                    .collect(),
            );
            g.2.push(ri);
        }
    }
    groups.sort_by(|a, b| cmp_raw(&a.0, &b.0));
    let n = groups.len().max(1);
    let mut cols = if sm_cols > 0 {
        sm_cols
    } else {
        ((n as f32 * b.w / b.h.max(1.0)).sqrt().ceil() as usize).max(1)
    };
    cols = cols.clamp(1, n);
    let rows_n = if sm_rows > 0 {
        sm_rows.min(n.div_ceil(cols))
    } else {
        n.div_ceil(cols)
    };
    let gap = 6.0;
    let px = ctx.px;
    let pw = (b.w - gap * (cols - 1) as f32) / cols as f32;
    let ph = (b.h - gap * (rows_n.max(1) - 1) as f32) / rows_n.max(1) as f32;
    let th = px * 1.5;
    let cols2: Vec<data::Col> = ds
        .cols
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != ci)
        .map(|(_, c)| c.clone())
        .collect();
    let shift = |c: usize| if c > ci { c - 1 } else { c };
    let mut sub = f.clone();
    sub.sm = None;
    sub.legend_show = Some(false);
    sub.colors.by_col = f
        .colors
        .by_col
        .iter()
        .filter(|(c, _)| **c != ci)
        .map(|(c, x)| (shift(*c), *x))
        .collect();
    sub.col_styles = f
        .col_styles
        .iter()
        .filter(|(c, _)| **c != ci)
        .map(|(c, x)| (shift(*c), *x))
        .collect();
    sub.lines = f
        .lines
        .iter()
        .map(|l| {
            let mut l = l.clone();
            if let Some(c) = l.get("column").and_then(Value::as_u64)
                && let Some(o) = l.as_object_mut()
            {
                o.insert("column".into(), Value::from(shift(c as usize) as u64));
            }
            l
        })
        .collect();
    // per-series label and line settings follow their data column too (the split one dropped)
    for key in ["series_labels", "series_lines"] {
        if let Some(list) = sub.raw.get_mut(key).and_then(Value::as_array_mut) {
            list.retain(|e| e.get("column").and_then(Value::as_u64) != Some(ci as u64));
            for e in list.iter_mut() {
                if let Some(c) = e.get("column").and_then(Value::as_u64)
                    && let Some(o) = e.as_object_mut()
                {
                    o.insert("column".into(), Value::from(shift(c as usize) as u64));
                }
            }
        }
    }
    let ccol = ds.cols.get(ci).map(|c| c.cf());
    for (i, (value, rows, at_rows)) in groups.iter().take(cols * rows_n).enumerate() {
        let (gx, gy) = ((i % cols) as f32 * (pw + gap), (i / cols) as f32 * (ph + gap));
        let label = crate::fmt::fmt_cell(value, ccol.as_ref(), &ctx.loc, false);
        let label = if label.is_empty() { ctx.blank() } else { label };
        let t = charts::fit(ctx, &label, pw, px);
        charts::tx(
            cv,
            ctx,
            gx + pw / 2.0,
            gy + th / 2.0,
            &t,
            px,
            ctx.fg2,
            Anchor::Middle,
        );
        let pb = Bx {
            w: pw.max(4.0),
            h: (ph - th).max(4.0),
        };
        let mut styles = HashMap::new();
        for (r, k0) in at_rows.iter().enumerate() {
            for c in 0..ds.cols.len() {
                if c != ci
                    && let Some(s) = ds.styles.get(&(*k0, c))
                {
                    styles.insert((r, shift(c)), s.clone());
                }
            }
        }
        let pds = data::Dataset {
            cols: cols2.clone(),
            rows: rows.clone(),
            truncated: ds.truncated,
            styles,
            ..data::Dataset::default()
        };
        if let Some(mut panel) = cv.layer(gx, gy + th, pb.w, pb.h) {
            draw_body(&mut panel, &pds, pb, &sub, ctx, &Value::Null, k);
            cv.draw(&panel, gx, gy + th, 1.0);
        }
    }
}

#[cfg(test)]
mod analytics_tests;
#[cfg(test)]
mod maps_tests;
#[cfg(test)]
mod numaxis_tests;
#[cfg(test)]
mod pdf_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod timeaxis_tests;

/// The prompt on a visual that binds no field: an info icon and the `empty_visual` text (UI
/// language) at the top left, 12 px #252423, where Desktop 2.158 draws its own prompt.
fn empty_visual_prompt(cv: &mut Canvas, ctx: &Ctx<'_>, w: f32) {
    if w < 60.0 {
        return;
    }
    let ink = Rgb(0x25, 0x24, 0x23);
    cv.circle(16.0, 17.0, 6.5, None, Some((ink, 1.0)));
    // the "i": a dot over a stem
    cv.circle(16.0, 14.0, 0.7, Some((ink, 1.0)), None);
    cv.line(16.0, 16.0, 16.0, 20.5, ink, 1.0, None);
    let text = ctx.t("empty_visual", "Add fields to show data in this visual");
    let f = Font::new(&ctx.font, 12.0);
    let t = ctx.fonts.fit(&f, &text, (w - 40.0).max(10.0));
    cv.text(
        ctx.fonts,
        &Text::new(&t, f, ink).anchor(Anchor::Start).base(Base::Alphabetic),
        34.0,
        22.0,
    );
}

#[cfg(test)]
mod keyinf_tests;

#[cfg(test)]
mod gantt_tests;
