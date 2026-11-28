//! A tiny-skia pixmap with a logical-pixel coordinate system (report pixels × scale), plus the
//! shapes and text the visual renderers draw.
//!
//! Under a [`VectorScope`] every canvas also records what it draws as vector operations
//! ([`Op`], logical pixels): the PDF export (`pdf.rs`) writes them as paths, text and images.
//! The raster is drawn too, so a canvas whose recording is too large can still be placed as an
//! image (the raster fallback, one visual at a time).

use std::cell::{Cell, RefCell};
use std::sync::Arc;

use tiny_skia::{
    FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, PixmapPaint, Rect, Stroke, StrokeDash, Transform,
};

use crate::fonts::{Font, FontBook, VecGlyph};

thread_local! {
    /// The vector operations a canvas created on this thread records at most (0: none).
    static VECTOR: Cell<usize> = const { Cell::new(0) };
    /// Canvases on this thread paint with the high-contrast palette ([`ContrastScope`]).
    static HIGH_CONTRAST: Cell<bool> = const { Cell::new(false) };
}

/// While this lives (on this thread), every colour a canvas paints goes through
/// [`Rgb::high_contrast`], and text takes black or white by what it is drawn over: the app's
/// view-only high-contrast rendering of a report (`RenderOptions::high_contrast`). Images keep
/// their pixels.
pub(crate) struct ContrastScope {
    was: bool,
}

impl ContrastScope {
    pub(crate) fn new(on: bool) -> ContrastScope {
        ContrastScope {
            was: HIGH_CONTRAST.with(|h| h.replace(on)),
        }
    }
}

impl Drop for ContrastScope {
    fn drop(&mut self) {
        let was = self.was;
        HIGH_CONTRAST.with(|h| h.set(was));
    }
}

pub(crate) fn high_contrast_on() -> bool {
    HIGH_CONTRAST.with(Cell::get)
}

/// The high-contrast palette: black, white, and bright colours that read at 7:1 or more on black
/// (yellow, the hyperlink colour of common high-contrast schemes, first).
pub const HIGH_CONTRAST_COLORS: [Rgb; 7] = [
    Rgb(0xFF, 0xFF, 0x00),
    Rgb(0x1A, 0xEB, 0xFF),
    Rgb(0x3F, 0xF2, 0x3F),
    Rgb(0xFF, 0x8C, 0x8C),
    Rgb(0xFF, 0x80, 0xFF),
    Rgb(0xFF, 0xB0, 0x00),
    Rgb(0xA0, 0xC4, 0xFF),
];

/// Canvases created while this lives (on this thread) record vector operations.
pub(crate) struct VectorScope {
    was: usize,
}

impl VectorScope {
    #[cfg(test)]
    pub(crate) fn new() -> VectorScope {
        Self::with_cap(MAX_VECTOR_OPS)
    }

    /// With a cap of `cap` operations per canvas (tests use a small one).
    pub(crate) fn with_cap(cap: usize) -> VectorScope {
        VectorScope {
            was: VECTOR.with(|v| v.replace(cap.max(1))),
        }
    }
}

impl Drop for VectorScope {
    fn drop(&mut self) {
        let was = self.was;
        VECTOR.with(|v| v.set(was));
    }
}

/// Most vector operations one canvas records, nested layers included; past it the canvas is
/// placed as an image (a scatter with a million points, a very long table).
pub(crate) const MAX_VECTOR_OPS: usize = 200_000;

/// A recorded drawing operation, in the canvas's logical pixels (y down).
#[derive(Debug, Clone)]
pub(crate) enum Op {
    /// A path filled (non-zero rule) after `tf`.
    Fill {
        path: tiny_skia::Path,
        tf: Transform,
        color: Rgb,
        alpha: f32,
    },
    /// A path stroked after `tf` (the width in the path's space); `round` caps and joins, else
    /// butt caps and miter joins.
    Stroke {
        path: tiny_skia::Path,
        tf: Transform,
        color: Rgb,
        alpha: f32,
        width: f32,
        dash: Dash,
        round: bool,
    },
    /// A path filled after `tf` with a vertical gradient: `top` at `y0`, `bottom` at `y1` (the
    /// path's space), padded beyond.
    Gradient {
        path: tiny_skia::Path,
        tf: Transform,
        top: Rgb,
        bottom: Rgb,
        y0: f32,
        y1: f32,
        alpha: f32,
    },
    /// A shaped run of text.
    Text(TextOp),
    /// An image stretched over the rectangle.
    Image {
        img: Arc<Pixmap>,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        opacity: f32,
    },
    /// Another canvas composited at (`x`, `y`), clipped to its `w`×`h`.
    Layer {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        opacity: f32,
        body: LayerBody,
    },
    /// The start of a part of the page for the PDF's structure tree (a visual, a table, a row
    /// or a cell of it); it ends at the matching [`Op::End`]. Draws nothing.
    Begin(Part),
    /// The end of the part opened by the last [`Op::Begin`].
    End,
}

/// A part of the page for the PDF's structure tree ([`Op::Begin`]).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Part {
    /// A visual: its title, its alternative text (which stands for all its content), the label
    /// of its graphics when it has no alternative text (its title, for a chart), and its place
    /// in the reading order (smaller first).
    Visual {
        title: String,
        alt: Option<String>,
        label: Option<String>,
        order: (f32, f32, f32),
    },
    /// A table of rows and cells.
    Table,
    /// A row of the open table.
    Row,
    /// A header cell (a column's header) of the open row.
    HeadCell,
    /// A data cell of the open row.
    Cell,
}

/// What a composited canvas holds: its operations, or its raster when they are not available.
#[derive(Debug, Clone)]
pub(crate) enum LayerBody {
    Vector(Vec<Op>),
    Raster(Arc<Pixmap>),
}

/// A run of text as glyphs: `tf` maps the run's space (pixels, y down, the pen at the origin on
/// the baseline) to the canvas.
#[derive(Debug, Clone)]
pub(crate) struct TextOp {
    pub tf: Transform,
    /// Pixels per em.
    pub size: f32,
    /// A synthetic slant (x' = x + 0.2·y in glyph space).
    pub oblique: bool,
    pub color: Rgb,
    pub alpha: f32,
    pub glyphs: Vec<VecGlyph>,
    pub text: String,
    /// The run's advance (pixels).
    pub width: f32,
}

/// The operations recorded by a canvas and their count, nested layers included.
#[derive(Debug, Default)]
struct Recording {
    ops: Vec<Op>,
    count: usize,
    /// The most it records ([`MAX_VECTOR_OPS`]).
    cap: usize,
    /// More than `cap`: the canvas is placed as an image.
    overflow: bool,
}

/// An sRGB color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub const WHITE: Rgb = Rgb(255, 255, 255);

    /// `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa` (alpha dropped), with or without `#`.
    pub fn parse(s: &str) -> Option<Rgb> {
        let t = s.trim();
        let t = t.strip_prefix('#').unwrap_or(t);
        if !t.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let full: String = match t.len() {
            3 | 4 => t.chars().take(3).flat_map(|c| [c, c]).collect(),
            6 | 8 => t.chars().take(6).collect(),
            _ => return None,
        };
        let n = u32::from_str_radix(&full, 16).ok()?;
        Some(Rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
    }

    pub fn from_value(v: &serde_json::Value) -> Option<Rgb> {
        v.as_str().and_then(Rgb::parse)
    }

    /// `self` moved towards `other` by `t` (0..1).
    pub fn mix(self, other: Rgb, t: f32) -> Rgb {
        let t = t.clamp(0.0, 1.0);
        let m = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
        Rgb(m(self.0, other.0), m(self.1, other.1), m(self.2, other.2))
    }

    /// WCAG relative luminance.
    pub fn lum(self) -> f32 {
        let c = |x: u8| {
            let v = f32::from(x) / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * c(self.0) + 0.7152 * c(self.1) + 0.0722 * c(self.2)
    }

    /// Text color readable on `self`.
    pub fn on(self) -> Rgb {
        if self.lum() > 0.4 {
            Rgb(0x25, 0x24, 0x23)
        } else {
            Rgb::WHITE
        }
    }

    /// The colour drawn for `self` in high contrast: light neutrals (backgrounds, bands, light
    /// grid lines) become black, dark neutrals (text, axes, dark fills) white, and a colour
    /// with a hue the bright colour of [`HIGH_CONTRAST_COLORS`] for that hue, so that series
    /// keep apart and read on black.
    pub fn high_contrast(self) -> Rgb {
        let [r, g, b] = [self.0, self.1, self.2].map(|c| f32::from(c) / 255.0);
        let (max, min) = (r.max(g).max(b), r.min(g).min(b));
        let chroma = max - min;
        let light = (max + min) / 2.0;
        if chroma < 0.2 || light > 0.9 || light < 0.1 {
            return if self.lum() > 0.4 {
                Rgb(0, 0, 0)
            } else {
                Rgb::WHITE
            };
        }
        let hue = if max == r {
            60.0 * ((g - b) / chroma).rem_euclid(6.0)
        } else if max == g {
            60.0 * ((b - r) / chroma + 2.0)
        } else {
            60.0 * ((r - g) / chroma + 4.0)
        };
        let [yellow, cyan, green, red, magenta, orange, blue] = HIGH_CONTRAST_COLORS;
        match hue {
            h if h < 20.0 => red,
            h if h < 48.0 => orange,
            h if h < 75.0 => yellow,
            h if h < 165.0 => green,
            h if h < 200.0 => cyan,
            h if h < 255.0 => blue,
            h if h < 335.0 => magenta,
            _ => red,
        }
    }

    /// `self` as painted: [`Rgb::high_contrast`] under a [`ContrastScope`].
    pub(crate) fn shown(self) -> Rgb {
        if high_contrast_on() {
            self.high_contrast()
        } else {
            self
        }
    }

    fn paint(self, alpha: f32) -> Paint<'static> {
        self.shown().paint_raw(alpha)
    }

    /// A paint of exactly `self` (a colour already shown).
    fn paint_raw(self, alpha: f32) -> Paint<'static> {
        let mut p = Paint::default();
        p.set_color_rgba8(
            self.0,
            self.1,
            self.2,
            (alpha.clamp(0.0, 1.0) * 255.0).round() as u8,
        );
        p.anti_alias = true;
        p
    }
}

/// Horizontal text anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    Start,
    Middle,
    End,
}

/// Vertical reference of a text position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Base {
    /// `y` is the baseline.
    Alphabetic,
    /// `y` is the middle of the line (SVG `dominant-baseline: central`).
    Central,
}

/// A text drawing request.
#[derive(Debug, Clone)]
pub struct Text<'a> {
    pub font: Font,
    pub color: Rgb,
    pub alpha: f32,
    pub anchor: Anchor,
    pub base: Base,
    /// Rotation in degrees around the anchor point.
    pub rotate: f32,
    pub underline: bool,
    pub text: &'a str,
}

impl<'a> Text<'a> {
    pub fn new(text: &'a str, font: Font, color: Rgb) -> Self {
        Self {
            font,
            color,
            alpha: 1.0,
            anchor: Anchor::Start,
            base: Base::Central,
            rotate: 0.0,
            underline: false,
            text,
        }
    }
    pub fn anchor(mut self, a: Anchor) -> Self {
        self.anchor = a;
        self
    }
    pub fn base(mut self, b: Base) -> Self {
        self.base = b;
        self
    }
    pub fn rotate(mut self, r: f32) -> Self {
        self.rotate = r;
        self
    }
    pub fn underline(mut self, u: bool) -> Self {
        self.underline = u;
        self
    }
}

/// Stroke dash pattern in logical pixels.
pub type Dash = Option<(f32, f32)>;

/// A text run as drawn (`RenderOptions::text_dump`): logical pixels of the page; the box is the
/// axis-aligned bounds of the run's line box (ascent + descent), after rotation.
#[derive(Debug, Clone, PartialEq)]
pub struct TextRun {
    pub text: String,
    pub family: String,
    pub size: f32,
    pub weight: u16,
    pub italic: bool,
    pub color: Rgb,
    pub alpha: f32,
    pub rotate: f32,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Most text runs a canvas records.
const MAX_TEXT_RUNS: usize = 100_000;

/// A drawing surface in logical (report) pixels.
pub struct Canvas {
    pub pm: Pixmap,
    pub scale: f32,
    /// Logical size.
    pub w: f32,
    pub h: f32,
    /// The text drawn on this canvas (and on canvases composited into it).
    pub texts: Vec<TextRun>,
    /// The hit regions recorded on this canvas (and on canvases composited into it).
    pub hits: Vec<crate::hits::Hit>,
    /// The vector operations, when the canvas was created under a [`VectorScope`].
    rec: RefCell<Option<Recording>>,
    /// Upright text gets subpixel (LCD) antialiasing where it lands on opaque pixels
    /// ([`Canvas::set_lcd_text`]).
    lcd: bool,
    /// With `lcd`, a layer's knowledge of what it will be composited over: per device pixel of
    /// this canvas, the opaque colour beneath it (alpha 255), or nothing known (alpha 0).
    backdrop: Option<Pixmap>,
    /// The LCD filter (tests compare it with others).
    lcd_taps: [u32; 5],
}

/// Largest coverage mask of one subpixel run (bytes, 3 per device pixel): a larger run is drawn
/// in grayscale.
const LCD_MAX_MASK: u64 = 8 << 20;
/// Largest layer that keeps a copy of its backdrop (device pixels); a larger one draws its text
/// in grayscale where it lands on transparent pixels.
const LCD_MAX_BACKDROP: u64 = 16 << 20;
/// The 5-tap LCD filter over the subpixel coverage (FreeType's default weights, sum 256): it
/// spreads each subpixel's ink over its neighbours, so that the colour fringes stay faint.
const LCD_FILTER: [u32; 5] = [0x08, 0x4D, 0x56, 0x4D, 0x08];

fn finite(x: f32) -> f32 {
    if x.is_finite() { x.clamp(-1e6, 1e6) } else { 0.0 }
}

impl Canvas {
    /// A transparent canvas of `w`×`h` logical pixels; `None` when too large or empty.
    pub fn new(w: f32, h: f32, scale: f32) -> Option<Canvas> {
        let pw = (finite(w) * scale).ceil().max(1.0);
        let ph = (finite(h) * scale).ceil().max(1.0);
        if pw > 30_000.0 || ph > 30_000.0 || pw * ph > 200_000_000.0 {
            return None;
        }
        let pm = Pixmap::new(pw as u32, ph as u32)?;
        Some(Canvas {
            pm,
            scale,
            w: finite(w).max(0.0),
            h: finite(h).max(0.0),
            texts: Vec::new(),
            hits: Vec::new(),
            rec: RefCell::new(match VECTOR.with(Cell::get) {
                0 => None,
                cap => Some(Recording {
                    cap,
                    ..Recording::default()
                }),
            }),
            lcd: false,
            backdrop: None,
            lcd_taps: LCD_FILTER,
        })
    }

    /// Turns subpixel (LCD, RGB horizontal stripes) antialiasing of upright text on or off (off
    /// by default): the on-screen pages only, never files (docs/render.md, "Subpixel text").
    /// Text gets it where it lands on opaque pixels (on a layer from [`Canvas::layer`], on the
    /// opaque pixels the layer will be composited over), else it is drawn in grayscale.
    pub fn set_lcd_text(&mut self, on: bool) {
        self.lcd = on;
        if !on {
            self.backdrop = None;
        }
    }

    /// A transparent canvas of `w`×`h` logical pixels that will be composited at (`x`, `y`) on
    /// this one ([`Canvas::draw`] with opacity 1): [`Canvas::new`] at this scale, and with
    /// subpixel text on, it keeps a copy of the opaque pixels it will be drawn over, so that
    /// its text is antialiased against them (this canvas must not change under it before the
    /// layer is drawn).
    pub fn layer(&self, x: f32, y: f32, w: f32, h: f32) -> Option<Canvas> {
        let mut c = Canvas::new(w, h, self.scale)?;
        if !self.lcd {
            return Some(c);
        }
        c.lcd = true;
        c.lcd_taps = self.lcd_taps;
        let (cw, ch) = (c.pm.width(), c.pm.height());
        if u64::from(cw) * u64::from(ch) > LCD_MAX_BACKDROP {
            return Some(c);
        }
        let Some(mut bd) = Pixmap::new(cw, ch) else {
            return Some(c);
        };
        // the offset `draw` uses
        let px = (finite(x) * self.scale).round() as i64;
        let py = (finite(y) * self.scale).round() as i64;
        let (pw, ph) = (i64::from(self.pm.width()), i64::from(self.pm.height()));
        let src = self.pm.pixels();
        let under = self.backdrop.as_ref().map(|b| b.pixels());
        let dst = bd.pixels_mut();
        for j in 0..i64::from(ch) {
            let sy = py + j;
            if !(0..ph).contains(&sy) {
                continue;
            }
            for i in 0..i64::from(cw) {
                let sx = px + i;
                if !(0..pw).contains(&sx) {
                    continue;
                }
                let (Ok(si), Ok(di)) = (
                    usize::try_from(sy * pw + sx),
                    usize::try_from(j * i64::from(cw) + i),
                ) else {
                    continue;
                };
                let (Some(d), Some(out)) = (src.get(si).copied(), dst.get_mut(di)) else {
                    continue;
                };
                *out = match d.alpha() {
                    255 => d,
                    da => match under.and_then(|u| u.get(si)).copied() {
                        Some(b) if b.alpha() == 255 => {
                            let k = |dc: u8, bc: u8| {
                                (f32::from(dc) + f32::from(bc) * (1.0 - f32::from(da) / 255.0))
                                    .round()
                                    .clamp(0.0, 255.0) as u8
                            };
                            tiny_skia::PremultipliedColorU8::from_rgba(
                                k(d.red(), b.red()),
                                k(d.green(), b.green()),
                                k(d.blue(), b.blue()),
                                255,
                            )
                            .unwrap_or(tiny_skia::PremultipliedColorU8::TRANSPARENT)
                        }
                        _ => tiny_skia::PremultipliedColorU8::TRANSPARENT,
                    },
                };
            }
        }
        c.backdrop = Some(bd);
        Some(c)
    }

    fn tf(&self) -> Transform {
        Transform::from_scale(self.scale, self.scale)
    }

    /// Whether this canvas records vector operations.
    pub(crate) fn recording(&self) -> bool {
        self.rec.borrow().is_some()
    }

    /// Opens a part of the page for the PDF's structure tree ([`Op::Begin`]); nothing when the
    /// canvas does not record.
    pub(crate) fn begin_part(&self, part: Part) {
        if self.recording() {
            let cap = |s: String, n: usize| s.chars().take(n).collect::<String>();
            let part = match part {
                Part::Visual {
                    title,
                    alt,
                    label,
                    order,
                } => Part::Visual {
                    title: cap(title, 500),
                    alt: alt.map(|a| cap(a, 2000)),
                    label: label.map(|a| cap(a, 2000)),
                    order,
                },
                p => p,
            };
            self.push(Op::Begin(part), 0);
        }
    }

    /// Closes the part [`Canvas::begin_part`] opened.
    pub(crate) fn end_part(&self) {
        if self.recording() {
            self.push(Op::End, 0);
        }
    }

    /// Records `op` (counting `n` operations) when this canvas records.
    fn push(&self, op: Op, n: usize) {
        if let Some(r) = self.rec.borrow_mut().as_mut() {
            r.count = r.count.saturating_add(n);
            if r.count > r.cap {
                r.overflow = true;
                r.ops.clear();
            } else if !r.overflow {
                r.ops.push(op);
            }
        }
    }

    /// The recorded operations, taken out of the canvas, and how many they count (`None` when
    /// it did not record).
    pub(crate) fn take_ops(&self) -> Option<(LayerBody, usize)> {
        let r = self.rec.borrow_mut().take()?;
        Some(if r.overflow {
            (LayerBody::Raster(Arc::new(self.pm.clone())), 1)
        } else {
            (LayerBody::Vector(r.ops), r.count.max(1))
        })
    }

    fn push_fill(&self, path: &tiny_skia::Path, tf: Transform, color: Rgb, alpha: f32) {
        if self.recording() && alpha > 0.0 {
            self.push(
                Op::Fill {
                    path: path.clone(),
                    tf,
                    color,
                    alpha: alpha.clamp(0.0, 1.0),
                },
                1,
            );
        }
    }

    pub fn fill(&mut self, c: Rgb, alpha: f32) {
        if let Some(r) = self.rec.borrow_mut().as_mut() {
            // a fill replaces every pixel: what was recorded before is covered
            r.ops.clear();
        }
        if let Some(p) =
            Rect::from_xywh(0.0, 0.0, self.w.max(0.01), self.h.max(0.01)).map(PathBuilder::from_rect)
        {
            self.push_fill(&p, Transform::identity(), c, alpha);
        }
        let r = alpha.clamp(0.0, 1.0);
        let c = c.shown();
        self.pm.fill(tiny_skia::Color::from_rgba8(
            c.0,
            c.1,
            c.2,
            (r * 255.0).round() as u8,
        ));
    }

    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, c: Rgb, alpha: f32) {
        if alpha <= 0.0 {
            return;
        }
        if let Some(r) = Rect::from_xywh(finite(x), finite(y), finite(w).max(0.01), finite(h).max(0.01)) {
            if self.recording() {
                self.push_fill(&PathBuilder::from_rect(r), Transform::identity(), c, alpha);
            }
            self.pm.fill_rect(r, &c.paint(alpha), self.tf(), None);
        }
    }

    pub fn round_rect(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        r: f32,
        fill: Option<(Rgb, f32)>,
        stroke: Option<(Rgb, f32, Dash)>,
    ) {
        let r = r.clamp(0.0, w.min(h) / 2.0).max(0.0);
        let Some(path) = round_rect_path(finite(x), finite(y), finite(w), finite(h), r) else {
            return;
        };
        if let Some((c, a)) = fill
            && a > 0.0
        {
            self.push_fill(&path, Transform::identity(), c, a);
            self.pm
                .fill_path(&path, &c.paint(a), FillRule::Winding, self.tf(), None);
        }
        if let Some((c, wd, dash)) = stroke {
            self.stroke_path(&path, c, 1.0, wd, dash, false);
        }
    }

    fn stroke_path(
        &mut self,
        path: &tiny_skia::Path,
        c: Rgb,
        alpha: f32,
        width: f32,
        dash: Dash,
        round: bool,
    ) {
        let mut st = Stroke {
            width: finite(width).max(0.05),
            ..Stroke::default()
        };
        if round {
            st.line_cap = LineCap::Round;
            st.line_join = LineJoin::Round;
        }
        if let Some((on, off)) = dash {
            st.dash = StrokeDash::new(vec![on.max(0.1), off.max(0.1)], 0.0);
        }
        self.push_stroke(path, Transform::identity(), c, alpha, st.width, dash, round);
        self.pm.stroke_path(path, &c.paint(alpha), &st, self.tf(), None);
    }

    fn push_stroke(
        &self,
        path: &tiny_skia::Path,
        tf: Transform,
        color: Rgb,
        alpha: f32,
        width: f32,
        dash: Dash,
        round: bool,
    ) {
        if self.recording() && alpha > 0.0 {
            self.push(
                Op::Stroke {
                    path: path.clone(),
                    tf,
                    color,
                    alpha: alpha.clamp(0.0, 1.0),
                    width,
                    dash: dash.map(|(a, b)| (a.max(0.1), b.max(0.1))),
                    round,
                },
                1,
            );
        }
    }

    pub fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, c: Rgb, width: f32, dash: Dash) {
        self.line_a(x1, y1, x2, y2, c, 1.0, width, dash, false);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn line_a(
        &mut self,
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        c: Rgb,
        alpha: f32,
        width: f32,
        dash: Dash,
        round: bool,
    ) {
        let mut pb = PathBuilder::new();
        pb.move_to(finite(x1), finite(y1));
        pb.line_to(finite(x2), finite(y2));
        if let Some(p) = pb.finish() {
            self.stroke_path(&p, c, alpha, width, dash, round);
        }
    }

    pub fn polyline(&mut self, pts: &[(f32, f32)], c: Rgb, alpha: f32, width: f32, dash: Dash) {
        if let Some(p) = poly(pts, false) {
            self.stroke_path(&p, c, alpha, width, dash, true);
        }
    }

    pub fn polygon(&mut self, pts: &[(f32, f32)], c: Rgb, alpha: f32) {
        if let Some(p) = poly(pts, true) {
            self.push_fill(&p, Transform::identity(), c, alpha);
            self.pm
                .fill_path(&p, &c.paint(alpha), FillRule::Winding, self.tf(), None);
        }
    }

    pub fn fill_path(&mut self, p: &tiny_skia::Path, c: Rgb, alpha: f32) {
        self.push_fill(p, Transform::identity(), c, alpha);
        self.pm
            .fill_path(p, &c.paint(alpha), FillRule::Winding, self.tf(), None);
    }

    /// Fills `p` with a vertical gradient from `top` (at `y0`) to `bottom` (at `y1`); in high
    /// contrast both ends take the same colour, so it is a plain fill.
    #[allow(clippy::too_many_arguments)]
    pub fn fill_gradient(
        &mut self,
        p: &tiny_skia::Path,
        top: Rgb,
        bottom: Rgb,
        y0: f32,
        y1: f32,
        alpha: f32,
    ) {
        let (y0, y1) = (finite(y0), finite(y1));
        if alpha <= 0.0 {
            return;
        }
        if high_contrast_on() || top == bottom || (y1 - y0).abs() < 0.01 {
            self.fill_path(p, bottom, alpha);
            return;
        }
        if self.recording() {
            self.push(
                Op::Gradient {
                    path: p.clone(),
                    tf: Transform::identity(),
                    top,
                    bottom,
                    y0,
                    y1,
                    alpha: alpha.clamp(0.0, 1.0),
                },
                1,
            );
        }
        let a = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
        let stop = |c: Rgb, at: f32| {
            tiny_skia::GradientStop::new(at, tiny_skia::Color::from_rgba8(c.0, c.1, c.2, a))
        };
        let shader = tiny_skia::LinearGradient::new(
            tiny_skia::Point::from_xy(0.0, y0),
            tiny_skia::Point::from_xy(0.0, y1),
            vec![stop(top, 0.0), stop(bottom, 1.0)],
            tiny_skia::SpreadMode::Pad,
            Transform::identity(),
        );
        match shader {
            Some(sh) => {
                let paint = Paint {
                    shader: sh,
                    anti_alias: true,
                    ..Paint::default()
                };
                self.pm.fill_path(p, &paint, FillRule::Winding, self.tf(), None);
            }
            None => {
                self.pm
                    .fill_path(p, &bottom.paint(alpha), FillRule::Winding, self.tf(), None);
            }
        }
    }

    pub fn fill_path_tf(&mut self, p: &tiny_skia::Path, c: Rgb, alpha: f32, t: Transform) {
        self.push_fill(p, t, c, alpha);
        self.pm.fill_path(
            p,
            &c.paint(alpha),
            FillRule::Winding,
            self.tf().pre_concat(t),
            None,
        );
    }

    pub fn stroke(&mut self, p: &tiny_skia::Path, c: Rgb, alpha: f32, width: f32, dash: Dash) {
        self.stroke_path(p, c, alpha, width, dash, true);
    }

    pub fn stroke_tf(&mut self, p: &tiny_skia::Path, c: Rgb, width: f32, t: Transform) {
        let st = Stroke {
            width: finite(width).max(0.05),
            line_cap: LineCap::Round,
            line_join: LineJoin::Round,
            ..Stroke::default()
        };
        self.push_stroke(p, t, c, 1.0, st.width, None, true);
        self.pm
            .stroke_path(p, &c.paint(1.0), &st, self.tf().pre_concat(t), None);
    }

    pub fn circle(&mut self, cx: f32, cy: f32, r: f32, fill: Option<(Rgb, f32)>, stroke: Option<(Rgb, f32)>) {
        let Some(p) = PathBuilder::from_circle(finite(cx), finite(cy), finite(r).max(0.05)) else {
            return;
        };
        if let Some((c, a)) = fill {
            self.fill_path(&p, c, a);
        }
        if let Some((c, w)) = stroke {
            self.stroke_path(&p, c, 1.0, w, None, false);
        }
    }

    /// Draws text; returns its advance width.
    pub fn text(&mut self, fonts: &FontBook, t: &Text<'_>, x: f32, y: f32) -> f32 {
        if t.text.is_empty() || t.alpha <= 0.0 {
            return 0.0;
        }
        // upright raster text is hinted at the device size (vertical hinting: stems and
        // x-heights on whole rows); rotated text keeps the unhinted outlines, and so does the
        // vector recording below (PDF: glyph ids drawn from the unhinted, resolution-independent
        // outlines), so recording never changes the pixels
        let upright = finite(t.rotate) == 0.0;
        let device = (upright && self.scale.is_finite() && self.scale > 0.0).then_some(self.scale);
        let (glyphs, width) = fonts.layout_at(&t.font, t.text, device);
        let (asc, desc) = fonts.metrics(&t.font);
        let dx = match t.anchor {
            Anchor::Start => 0.0,
            Anchor::Middle => -width / 2.0,
            Anchor::End => -width,
        };
        let dy = match t.base {
            Base::Alphabetic => 0.0,
            Base::Central => (asc - desc) / 2.0,
        };
        let mut base = self
            .tf()
            .pre_translate(finite(x), finite(y))
            .pre_concat(Transform::from_rotate(finite(t.rotate)))
            .pre_translate(dx, dy);
        // horizontal text sits on a whole device pixel row, as Chromium places it: a baseline
        // between two rows smears every horizontal stroke over both (soft text at 100 % scale)
        if base.kx == 0.0 && base.ky == 0.0 && base.ty.is_finite() {
            base.ty = base.ty.round();
        }
        // high contrast: the text's colour mapped, then black or white by what it lands on
        let color = if high_contrast_on() {
            self.contrast_text(t.color, base, width, asc, desc)
        } else {
            t.color
        };
        let paint = color.paint_raw(t.alpha);
        // synthetic oblique (x' = x + 0.2·y in font units, y up)
        let italic = if fonts.needs_oblique(&t.font) { 0.2 } else { 0.0 };
        if self.recording() {
            let lbase = Transform::from_translate(finite(x), finite(y))
                .pre_concat(Transform::from_rotate(finite(t.rotate)))
                .pre_translate(dx, dy);
            let (vg, size) = fonts.layout_vector(&t.font, t.text);
            let n = vg.len().max(1);
            self.push(
                Op::Text(TextOp {
                    tf: lbase,
                    size,
                    oblique: italic > 0.0,
                    color: t.color,
                    alpha: t.alpha.clamp(0.0, 1.0),
                    glyphs: vg,
                    text: t.text.to_string(),
                    width,
                }),
                n,
            );
            if t.underline
                && let Some(r) = Rect::from_xywh(
                    0.0,
                    t.font.size * 0.12,
                    width.max(0.1),
                    (t.font.size / 14.0).max(0.6),
                )
            {
                self.push_fill(&PathBuilder::from_rect(r), lbase, t.color, t.alpha);
            }
        }
        let placed: Vec<(&tiny_skia::Path, Transform)> = glyphs
            .iter()
            .map(|g| {
                let gt = base.pre_translate(g.x, -g.y).pre_concat(Transform::from_row(
                    g.scale,
                    0.0,
                    italic * g.scale,
                    -g.scale,
                    0.0,
                    0.0,
                ));
                (&g.path, gt)
            })
            .collect();
        // subpixel antialiasing needs horizontal text: the stripes run along x
        let horizontal = upright && base.kx == 0.0 && base.ky == 0.0;
        self.glyphs(&placed, color, t.alpha, horizontal);
        self.record_text(t, color, x, y, dx, dy, width, asc, desc);
        if t.underline
            && let Some(r) = Rect::from_xywh(
                0.0,
                t.font.size * 0.12,
                width.max(0.1),
                (t.font.size / 14.0).max(0.6),
            )
        {
            self.pm.fill_rect(r, &paint, base, None);
        }
        width
    }

    /// The high-contrast colour of text `c` laid out at device transform `base` (`width` wide,
    /// `asc` above and `desc` below its baseline): black over light pixels, else its mapped
    /// colour when it reads (white otherwise). What lies beneath is sampled at three points of
    /// the run; transparent pixels (a layer's, without a known backdrop) count as the black page.
    fn contrast_text(&self, c: Rgb, base: Transform, width: f32, asc: f32, desc: f32) -> Rgb {
        let mid = (desc - asc) / 2.0;
        let mut light = 0;
        let mut seen = 0;
        for f in [0.2, 0.5, 0.8] {
            let mut pt = [tiny_skia::Point::from_xy(width * f, mid)];
            base.map_points(&mut pt);
            let (px, py) = (pt[0].x, pt[0].y);
            if !(px.is_finite() && py.is_finite()) || px < 0.0 || py < 0.0 {
                continue;
            }
            let (ix, iy) = (px as u32, py as u32);
            let under = self.pm.pixel(ix, iy).filter(|p| p.alpha() >= 128).or_else(|| {
                self.backdrop
                    .as_ref()
                    .and_then(|b| b.pixel(ix, iy))
                    .filter(|p| p.alpha() >= 128)
            });
            seen += 1;
            if let Some(p) = under.map(|p| p.demultiply())
                && Rgb(p.red(), p.green(), p.blue()).lum() > 0.4
            {
                light += 1;
            }
        }
        if seen > 0 && light * 2 > seen {
            return Rgb(0, 0, 0);
        }
        let m = c.high_contrast();
        if m.lum() > 0.4 { m } else { Rgb::WHITE }
    }

    /// Notes a drawn run (its rotated line box, in logical pixels) for the text dump.
    fn record_text(
        &mut self,
        t: &Text<'_>,
        color: Rgb,
        x: f32,
        y: f32,
        dx: f32,
        dy: f32,
        width: f32,
        asc: f32,
        desc: f32,
    ) {
        if self.texts.len() >= MAX_TEXT_RUNS {
            return;
        }
        let (sin, cos) = finite(t.rotate).to_radians().sin_cos();
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for (cx, cy) in [
            (dx, dy - asc),
            (dx + width, dy - asc),
            (dx, dy + desc),
            (dx + width, dy + desc),
        ] {
            let (px, py) = (finite(x) + cx * cos - cy * sin, finite(y) + cx * sin + cy * cos);
            (x0, y0, x1, y1) = (x0.min(px), y0.min(py), x1.max(px), y1.max(py));
        }
        self.texts.push(TextRun {
            text: t.text.to_string(),
            family: t.font.family.clone(),
            size: t.font.size,
            weight: t.font.weight,
            italic: t.font.italic,
            color,
            alpha: t.alpha,
            rotate: t.rotate,
            x: x0,
            y: y0,
            w: (x1 - x0).max(0.0),
            h: (y1 - y0).max(0.0),
        });
    }

    /// Fills glyph outlines (device transforms) the way Chromium's text rasterizer does: the
    /// coverage of the whole run is computed first, then lifted by the text contrast curve (see
    /// [`text_coverage`]), then composited in one pass.
    ///
    /// With subpixel text on ([`Canvas::set_lcd_text`]), a `horizontal` opaque run over opaque
    /// pixels is drawn by [`Canvas::lcd_glyphs`] instead.
    fn glyphs(&mut self, placed: &[(&tiny_skia::Path, Transform)], color: Rgb, alpha: f32, horizontal: bool) {
        let Some((x0, y0, x1, y1)) = run_bounds(placed) else {
            return;
        };
        if self.lcd && horizontal && alpha >= 1.0 && self.lcd_glyphs(placed, color, (x0, y0, x1, y1)) {
            return;
        }
        let (pw, ph) = (self.pm.width() as f32, self.pm.height() as f32);
        let ox = x0.floor().clamp(0.0, pw);
        let oy = y0.floor().clamp(0.0, ph);
        let ex = (x1.ceil() + 1.0).clamp(0.0, pw);
        let ey = (y1.ceil() + 1.0).clamp(0.0, ph);
        if ex <= ox || ey <= oy {
            return;
        }
        let (w, h) = ((ex - ox) as u32, (ey - oy) as u32);
        let (Some(mut mask), Some(mut run)) = (tiny_skia::Mask::new(w, h), Pixmap::new(w, h)) else {
            return;
        };
        let shift = Transform::from_translate(-ox, -oy);
        for (p, t) in placed {
            mask.fill_path(p, FillRule::Winding, true, t.post_concat(shift));
        }
        let lut = text_coverage(color);
        let a = alpha.clamp(0.0, 1.0);
        for (px, m) in run.pixels_mut().iter_mut().zip(mask.data()) {
            let cov = f32::from(lut.get(usize::from(*m)).copied().unwrap_or(0)) / 255.0 * a;
            if cov <= 0.0 {
                continue;
            }
            let k = |c: u8| (f32::from(c) * cov).round() as u8;
            let al = (cov * 255.0).round() as u8;
            if let Some(c) = tiny_skia::PremultipliedColorU8::from_rgba(
                k(color.0).min(al),
                k(color.1).min(al),
                k(color.2).min(al),
                al,
            ) {
                *px = c;
            }
        }
        self.pm.draw_pixmap(
            ox as i32,
            oy as i32,
            run.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            None,
        );
    }

    /// Subpixel (LCD) text, as Chromium draws it on Windows: the run's coverage is rasterized
    /// at 3× the horizontal resolution, spread by the 5-tap LCD filter, lifted by the text
    /// contrast curve, and each device pixel takes its red, green and blue coverage from its
    /// three subpixels (RGB stripes, left to right). It needs opaque pixels under the whole run
    /// (this canvas's, or its backdrop's); `false` when the run is not drawn (then it is drawn
    /// in grayscale): pixels not opaque, run too large, nothing to draw.
    fn lcd_glyphs(
        &mut self,
        placed: &[(&tiny_skia::Path, Transform)],
        color: Rgb,
        (x0, y0, x1, y1): (f32, f32, f32, f32),
    ) -> bool {
        let (pw, ph) = (self.pm.width() as f32, self.pm.height() as f32);
        // a pixel more on each side for the filter's spread
        let ox = (x0.floor() - 1.0).clamp(0.0, pw);
        let oy = y0.floor().clamp(0.0, ph);
        let ex = (x1.ceil() + 2.0).clamp(0.0, pw);
        let ey = (y1.ceil() + 1.0).clamp(0.0, ph);
        if !(ex > ox && ey > oy) {
            return false;
        }
        let (w, h) = ((ex - ox) as usize, (ey - oy) as usize);
        let (ox, oy, cw) = (ox as usize, oy as usize, self.pm.width() as usize);
        if (w as u64).saturating_mul(3).saturating_mul(h as u64) > LCD_MAX_MASK {
            return false;
        }
        // every pixel under the run is opaque, here or on the backdrop
        let opaque = |i: usize| {
            self.pm.pixels().get(i).is_some_and(|p| p.alpha() == 255)
                || self
                    .backdrop
                    .as_ref()
                    .and_then(|b| b.pixels().get(i))
                    .is_some_and(|p| p.alpha() == 255)
        };
        for j in oy..oy + h {
            if !(ox..ox + w).all(|i| opaque(j * cw + i)) {
                return false;
            }
        }
        let Ok(w3) = u32::try_from(w * 3) else {
            return false;
        };
        let Ok(hu) = u32::try_from(h) else { return false };
        let Some(mut mask) = tiny_skia::Mask::new(w3, hu) else {
            return false;
        };
        let shift = Transform::from_translate(-(ox as f32), -(oy as f32)).post_scale(3.0, 1.0);
        for (p, t) in placed {
            mask.fill_path(p, FillRule::Winding, true, t.post_concat(shift));
        }
        let lut = text_coverage(color);
        let m = mask.data();
        let taps = self.lcd_taps;
        let tc = [color.0, color.1, color.2].map(|c| f32::from(c) / 255.0);
        let backdrop = self.backdrop.as_ref().map(|b| b.pixels());
        let pixels = self.pm.pixels_mut();
        let w3 = w * 3;
        for j in 0..h {
            let row = m.get(j * w3..(j + 1) * w3).unwrap_or(&[]);
            for x in 0..w {
                // the filtered, contrast-lifted coverage of the pixel's red, green, blue
                let mut cov = [0f32; 3];
                for (c, v) in cov.iter_mut().enumerate() {
                    let s = 3 * x + c;
                    let mut acc = 0u32;
                    for (k, wk) in taps.iter().enumerate() {
                        if let Some(q) = (s + k).checked_sub(2).and_then(|i| row.get(i)) {
                            acc += u32::from(*q) * wk;
                        }
                    }
                    let f = ((acc + 128) >> 8).min(255) as usize;
                    *v = f32::from(lut.get(f).copied().unwrap_or(0)) / 255.0;
                }
                let cmax = cov[0].max(cov[1]).max(cov[2]);
                if cmax <= 0.0 {
                    continue;
                }
                let i = (oy + j) * cw + ox + x;
                let b = backdrop.and_then(|b| b.get(i)).copied();
                let Some(px) = pixels.get_mut(i) else { continue };
                let d = [px.red(), px.green(), px.blue()].map(|c| f32::from(c) / 255.0);
                let da = f32::from(px.alpha()) / 255.0;
                let bc = b.map_or([0.0; 3], |b| {
                    [b.red(), b.green(), b.blue()].map(|c| f32::from(c) / 255.0)
                });
                // over the opaque colour E = D + B·(1 − Da) the text gives T·c + E·(1 − c) per
                // channel; a pixel not opaque here keeps that result when composited over B with
                // alpha A = Da + cmax·(1 − Da): S = T·c + D·(1 − c) + B·(1 − Da)·(cmax − c),
                // which is a valid premultiplied colour (0 ≤ S ≤ A)
                let a = da + cmax * (1.0 - da);
                let s = |c: usize| {
                    let (t, dc, bc, cv) = (
                        tc.get(c).copied().unwrap_or(0.0),
                        d.get(c).copied().unwrap_or(0.0),
                        bc.get(c).copied().unwrap_or(0.0),
                        cov.get(c).copied().unwrap_or(0.0),
                    );
                    let v = t * cv + dc * (1.0 - cv) + bc * (1.0 - da) * (cmax - cv);
                    (v.min(a) * 255.0).round().clamp(0.0, 255.0) as u8
                };
                let al = (a * 255.0).round().clamp(0.0, 255.0) as u8;
                if let Some(c) =
                    tiny_skia::PremultipliedColorU8::from_rgba(s(0).min(al), s(1).min(al), s(2).min(al), al)
                {
                    *px = c;
                }
            }
        }
        true
    }

    /// Records a hit region (`None`: nothing, e.g. hits are off).
    pub fn hit(&mut self, h: Option<crate::hits::Hit>) {
        if let Some(h) = h
            && self.hits.len() < crate::hits::MAX_HITS
        {
            self.hits.push(h);
        }
    }

    /// Composites `other` at logical position (`x`, `y`) with `opacity`.
    pub fn draw(&mut self, other: &Canvas, x: f32, y: f32, opacity: f32) {
        let px = (finite(x) * self.scale).round() as i32;
        let py = (finite(y) * self.scale).round() as i32;
        if self.recording()
            && opacity > 0.0
            && let Some((body, n)) = other.take_ops()
        {
            let s = self.scale.max(1e-6);
            self.push(
                Op::Layer {
                    x: px as f32 / s,
                    y: py as f32 / s,
                    w: other.w,
                    h: other.h,
                    opacity: opacity.clamp(0.0, 1.0),
                    body,
                },
                n,
            );
        }
        let paint = PixmapPaint {
            opacity: opacity.clamp(0.0, 1.0),
            ..PixmapPaint::default()
        };
        self.pm
            .draw_pixmap(px, py, other.pm.as_ref(), &paint, Transform::identity(), None);
        // the composited canvas's text, where it is visible (centre inside that canvas)
        if opacity > 0.0 {
            let (ox, oy) = (px as f32 / self.scale.max(1e-6), py as f32 / self.scale.max(1e-6));
            let room = MAX_TEXT_RUNS.saturating_sub(self.texts.len());
            let shown = other.texts.iter().filter(|r| {
                let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
                (0.0..=other.w).contains(&cx) && (0.0..=other.h).contains(&cy)
            });
            self.texts.extend(shown.take(room).map(|r| TextRun {
                x: r.x + ox,
                y: r.y + oy,
                alpha: r.alpha * opacity.min(1.0),
                ..r.clone()
            }));
            let room = crate::hits::MAX_HITS.saturating_sub(self.hits.len());
            self.hits.extend(
                other
                    .hits
                    .iter()
                    .filter(|h| {
                        let (x, y, w, hh) = h.shape.bounds();
                        x < other.w && y < other.h && x + w > 0.0 && y + hh > 0.0
                    })
                    .take(room)
                    .map(|h| h.moved(ox, oy)),
            );
        }
    }

    /// Draws an image into the logical rectangle with `opacity`, clipped to this canvas.
    pub fn image(&mut self, img: &Pixmap, x: f32, y: f32, w: f32, h: f32, opacity: f32) {
        let iw = img.width() as f32;
        let ih = img.height() as f32;
        if iw <= 0.0 || ih <= 0.0 || w <= 0.0 || h <= 0.0 {
            return;
        }
        if self.recording() && opacity > 0.0 {
            self.push(
                Op::Image {
                    img: Arc::new(img.clone()),
                    x: finite(x),
                    y: finite(y),
                    w: finite(w),
                    h: finite(h),
                    opacity: opacity.clamp(0.0, 1.0),
                },
                1,
            );
        }
        let paint = PixmapPaint {
            opacity: opacity.clamp(0.0, 1.0),
            quality: tiny_skia::FilterQuality::Bicubic,
            ..PixmapPaint::default()
        };
        let t = self
            .tf()
            .pre_translate(finite(x), finite(y))
            .pre_scale(finite(w) / iw, finite(h) / ih);
        self.pm.draw_pixmap(0, 0, img.as_ref(), &paint, t, None);
    }
}

/// The device bounds of a run of placed glyphs (`None`: nothing to draw).
fn run_bounds(placed: &[(&tiny_skia::Path, Transform)]) -> Option<(f32, f32, f32, f32)> {
    let mut bb: Option<(f32, f32, f32, f32)> = None;
    for (p, t) in placed {
        let Some(r) = (*p).clone().transform(*t).map(|q| q.bounds()) else {
            continue;
        };
        let (l, tp, rt, bt) = (r.left(), r.top(), r.right(), r.bottom());
        bb = Some(match bb {
            None => (l, tp, rt, bt),
            Some((a, b, c, d)) => (a.min(l), b.min(tp), c.max(rt), d.max(bt)),
        });
    }
    bb
}

/// The coverage curve of text: Chromium (Skia) rasterizes glyphs with a contrast and gamma
/// boost so that small text reads darker than its geometric coverage; dark text gains more than
/// light text. A table from coverage (0..255) to the alpha painted.
fn text_coverage(color: Rgb) -> [u8; 256] {
    // exponent of 1 - (1 - c)^k: 1 = geometric coverage
    let k = TEXT_BOOST_DARK + (TEXT_BOOST_LIGHT - TEXT_BOOST_DARK) * color.lum().clamp(0.0, 1.0);
    let mut lut = [0u8; 256];
    for (i, v) in lut.iter_mut().enumerate() {
        let c = i as f32 / 255.0;
        *v = ((1.0 - (1.0 - c).powf(k)) * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    lut
}

/// Coverage boost of dark text (black) and of light text (white): with these the ink of table
/// text matches Desktop 2.158's captures (render oracle, docs/render.md).
const TEXT_BOOST_DARK: f32 = 1.6;
const TEXT_BOOST_LIGHT: f32 = 1.2;

pub fn poly(pts: &[(f32, f32)], close: bool) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    let mut it = pts.iter();
    let (x0, y0) = it.next()?;
    pb.move_to(finite(*x0), finite(*y0));
    for (x, y) in it {
        pb.line_to(finite(*x), finite(*y));
    }
    if close {
        pb.close();
    }
    pb.finish()
}

pub fn round_rect_path(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<tiny_skia::Path> {
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    if r <= 0.0 {
        return Rect::from_xywh(x, y, w, h).map(PathBuilder::from_rect);
    }
    let k = 0.552_284_8 * r;
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
    pb.finish()
}

/// A rectangle whose corners are rounded by `r` except those on side `sq` (`b`, `t`, `l`, `r`;
/// anything else rounds all four).
pub fn corner_rect_path(x: f32, y: f32, w: f32, h: f32, r: f32, sq: &str) -> Option<tiny_skia::Path> {
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let r = r.clamp(0.0, w.min(h) / 2.0);
    let on = |side: &str| if sq == side { 0.0 } else { r };
    // top left, top right, bottom right, bottom left
    let tl = if sq == "t" || sq == "l" { 0.0 } else { r };
    let tr = if sq == "t" { 0.0 } else { on("r") };
    let br = if sq == "b" { 0.0 } else { on("r") };
    let bl = if sq == "b" || sq == "l" { 0.0 } else { r };
    if tl + tr + br + bl <= 0.0 {
        return Rect::from_xywh(x, y, w, h).map(PathBuilder::from_rect);
    }
    const K: f32 = 0.552_284_8;
    let mut pb = PathBuilder::new();
    pb.move_to(x + tl, y);
    pb.line_to(x + w - tr, y);
    if tr > 0.0 {
        pb.cubic_to(x + w - tr + K * tr, y, x + w, y + tr - K * tr, x + w, y + tr);
    }
    pb.line_to(x + w, y + h - br);
    if br > 0.0 {
        pb.cubic_to(
            x + w,
            y + h - br + K * br,
            x + w - br + K * br,
            y + h,
            x + w - br,
            y + h,
        );
    }
    pb.line_to(x + bl, y + h);
    if bl > 0.0 {
        pb.cubic_to(x + bl - K * bl, y + h, x, y + h - bl + K * bl, x, y + h - bl);
    }
    pb.line_to(x, y + tl);
    if tl > 0.0 {
        pb.cubic_to(x, y + tl - K * tl, x + tl - K * tl, y, x + tl, y);
    }
    pb.close();
    pb.finish()
}

/// An annulus sector (`r0` = 0: a pie slice) from angle `a0` to `a1` (radians, clockwise from +x).
pub fn annulus(cx: f32, cy: f32, r0: f32, r1: f32, a0: f32, a1: f32) -> Option<tiny_skia::Path> {
    let span = (a1 - a0).clamp(0.0, std::f32::consts::TAU);
    let steps = ((span / std::f32::consts::TAU) * 180.0).ceil().clamp(2.0, 360.0) as usize;
    let mut pb = PathBuilder::new();
    let at = |r: f32, a: f32| (cx + r * a.cos(), cy + r * a.sin());
    let (sx, sy) = at(r1, a0);
    if r0 <= 0.0 {
        pb.move_to(cx, cy);
        pb.line_to(sx, sy);
    } else {
        pb.move_to(sx, sy);
    }
    for i in 1..=steps {
        let (x, y) = at(r1, a0 + span * i as f32 / steps as f32);
        pb.line_to(x, y);
    }
    if r0 > 0.0 {
        for i in (0..=steps).rev() {
            let (x, y) = at(r0, a0 + span * i as f32 / steps as f32);
            pb.line_to(x, y);
        }
    }
    pb.close();
    pb.finish()
}

/// An SVG-like path in a 16-unit box ("M2 8h10M8.5 4l4 4-4 4"): M/L/H/V/Z/A(circles only as
/// line segments)/C, absolute and relative.
pub fn svg_path(d: &str) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    let toks = tokenize(d);
    let mut i = 0usize;
    let (mut x, mut y) = (0.0f32, 0.0f32);
    let (mut sx, mut sy) = (0.0f32, 0.0f32);
    let mut cmd = 'M';
    let num = |toks: &[Tok], i: &mut usize| -> Option<f32> {
        match toks.get(*i) {
            Some(Tok::Num(n)) => {
                *i += 1;
                Some(*n)
            }
            _ => None,
        }
    };
    let mut guard = 0usize;
    while i < toks.len() {
        guard += 1;
        if guard > 10_000 {
            break;
        }
        if let Some(Tok::Cmd(c)) = toks.get(i) {
            cmd = *c;
            i += 1;
        }
        let rel = cmd.is_ascii_lowercase();
        let (ox, oy) = if rel { (x, y) } else { (0.0, 0.0) };
        match cmd.to_ascii_uppercase() {
            'M' => {
                let (a, b) = (num(&toks, &mut i)?, num(&toks, &mut i)?);
                x = ox + a;
                y = oy + b;
                sx = x;
                sy = y;
                pb.move_to(x, y);
                cmd = if rel { 'l' } else { 'L' };
            }
            'L' => {
                let (a, b) = (num(&toks, &mut i)?, num(&toks, &mut i)?);
                x = ox + a;
                y = oy + b;
                pb.line_to(x, y);
            }
            'H' => {
                let a = num(&toks, &mut i)?;
                x = ox + a;
                pb.line_to(x, y);
            }
            'V' => {
                let b = num(&toks, &mut i)?;
                y = oy + b;
                pb.line_to(x, y);
            }
            'C' => {
                let v: Vec<f32> = (0..6).filter_map(|_| num(&toks, &mut i)).collect();
                let [a, b, c, d, e, f] = v.as_slice() else {
                    return pb.finish();
                };
                pb.cubic_to(ox + a, oy + b, ox + c, oy + d, ox + e, oy + f);
                x = ox + e;
                y = oy + f;
            }
            'A' => {
                let v: Vec<f32> = (0..7).filter_map(|_| num(&toks, &mut i)).collect();
                let [rx, _ry, _rot, large, sweep, ex, ey] = v.as_slice() else {
                    return pb.finish();
                };
                let (tx, ty) = (ox + ex, oy + ey);
                arc_to(&mut pb, x, y, *rx, *large != 0.0, *sweep != 0.0, tx, ty);
                x = tx;
                y = ty;
            }
            'Z' => {
                pb.close();
                x = sx;
                y = sy;
            }
            _ => break,
        }
    }
    pb.finish()
}

/// A circular arc as line segments (SVG endpoint parameterization, rx = ry).
#[allow(clippy::too_many_arguments)]
fn arc_to(pb: &mut PathBuilder, x0: f32, y0: f32, r: f32, large: bool, sweep: bool, x1: f32, y1: f32) {
    let dx = (x1 - x0) / 2.0;
    let dy = (y1 - y0) / 2.0;
    let d2 = dx * dx + dy * dy;
    if d2 <= 1e-9 || r <= 0.0 {
        pb.line_to(x1, y1);
        return;
    }
    let r = r.max(d2.sqrt());
    let h = (r * r - d2).max(0.0).sqrt() / d2.sqrt();
    let s = if large == sweep { -1.0 } else { 1.0 };
    let cx = x0 + dx - s * h * dy;
    let cy = y0 + dy + s * h * dx;
    let a0 = (y0 - cy).atan2(x0 - cx);
    let mut a1 = (y1 - cy).atan2(x1 - cx);
    if sweep && a1 < a0 {
        a1 += std::f32::consts::TAU;
    }
    if !sweep && a1 > a0 {
        a1 -= std::f32::consts::TAU;
    }
    let n = 24;
    for k in 1..=n {
        let a = a0 + (a1 - a0) * k as f32 / n as f32;
        pb.line_to(cx + r * a.cos(), cy + r * a.sin());
    }
}

#[derive(Debug, Clone, Copy)]
enum Tok {
    Cmd(char),
    Num(f32),
}

fn tokenize(d: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let b: Vec<char> = d.chars().take(10_000).collect();
    let mut i = 0usize;
    while i < b.len() {
        let c = b.get(i).copied().unwrap_or(' ');
        if c.is_ascii_alphabetic() {
            out.push(Tok::Cmd(c));
            i += 1;
        } else if c.is_ascii_digit() || c == '-' || c == '.' || c == '+' {
            let start = i;
            let mut seen_dot = c == '.';
            i += 1;
            while let Some(&n) = b.get(i) {
                if n.is_ascii_digit() || (n == '.' && !seen_dot) || n == 'e' {
                    if n == '.' {
                        seen_dot = true;
                    }
                    i += 1;
                } else {
                    break;
                }
            }
            let s: String = b.get(start..i).map(|x| x.iter().collect()).unwrap_or_default();
            if let Ok(n) = s.parse::<f32>() {
                out.push(Tok::Num(n));
            }
        } else {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_parse_and_mix() {
        assert_eq!(Rgb::parse("#118DFF"), Some(Rgb(0x11, 0x8d, 0xff)));
        assert_eq!(Rgb::parse("fff"), Some(Rgb::WHITE));
        assert_eq!(Rgb::parse("#11223344"), Some(Rgb(0x11, 0x22, 0x33)));
        assert_eq!(Rgb::parse("red"), None);
        assert_eq!(Rgb(0, 0, 0).mix(Rgb::WHITE, 0.5), Rgb(128, 128, 128));
        assert_eq!(Rgb::WHITE.on(), Rgb(0x25, 0x24, 0x23));
    }

    #[test]
    fn svg_paths_parse() {
        assert!(svg_path("M2 8h10M8.5 4l4 4-4 4").is_some());
        assert!(svg_path("M8 1.5a6.5 6.5 0 1 0 0 13 6.5 6.5 0 0 0 0-13z").is_some());
        assert!(svg_path("").is_none());
        assert!(svg_path("M1").is_none());
    }

    #[test]
    fn text_draws_with_boost_and_survives_edges() {
        let fonts = FontBook::bundled_only();
        let mut cv = Canvas::new(120.0, 40.0, 1.0).expect("canvas");
        cv.fill(Rgb::WHITE, 1.0);
        let t = Text::new("Total 910K", Font::new("Segoe UI", 12.0), Rgb(0x33, 0x33, 0x33));
        let w = cv.text(&fonts, &t, 4.0, 20.0);
        assert!(w > 30.0, "{w}");
        let dark = cv.pm.pixels().iter().filter(|p| p.red() < 128).count();
        assert!(dark > 20, "{dark}");
        // half off the canvas, rotated, transparent, NaN positions: no panic
        let _ = cv.text(&fonts, &t.clone().rotate(-45.0), -30.0, -5.0);
        let _ = cv.text(&fonts, &t.clone().anchor(Anchor::End), 500.0, 500.0);
        let _ = cv.text(&fonts, &t, f32::NAN, f32::INFINITY);
        let mut faint = t.clone();
        faint.alpha = 0.0;
        assert_eq!(cv.text(&fonts, &faint, 4.0, 20.0), 0.0);
        // the contrast curve keeps its ends and lifts dark text more than light text
        let (d, l) = (text_coverage(Rgb(0, 0, 0)), text_coverage(Rgb::WHITE));
        assert_eq!((d[0], d[255], l[0], l[255]), (0, 255, 0, 255));
        assert!(d[64] > l[64] && l[64] >= 64);
    }

    #[test]
    fn text_runs_are_recorded_and_follow_composited_canvases() {
        let fonts = FontBook::bundled_only();
        let mut sub = Canvas::new(100.0, 30.0, 2.0).expect("canvas");
        let t = Text::new(
            "North",
            Font::new("Segoe UI", 12.0).weight(600),
            Rgb(0x66, 0x66, 0x66),
        );
        let w = sub.text(&fonts, &t, 10.0, 15.0);
        let _ = sub.text(&fonts, &t, 500.0, 15.0); // outside the sub canvas: not shown
        let r = sub.texts.first().expect("run").clone();
        assert_eq!(
            (r.text.as_str(), r.size, r.weight, r.color),
            ("North", 12.0, 600, Rgb(0x66, 0x66, 0x66))
        );
        assert!((r.x - 10.0).abs() < 0.01 && (r.w - w).abs() < 0.01);
        // Base::Central: the line box is centred on y
        assert!((r.y + r.h / 2.0 - 15.0).abs() < 0.01 && r.h > 12.0, "{r:?}");
        let mut page = Canvas::new(400.0, 300.0, 2.0).expect("canvas");
        page.draw(&sub, 50.0, 100.0, 1.0);
        assert_eq!(page.texts.len(), 1);
        assert!((page.texts[0].x - 60.0).abs() < 0.01 && (page.texts[0].y - (r.y + 100.0)).abs() < 0.01);
        // rotated by 90°: the box turns upright
        let _ = page.text(&fonts, &t.clone().rotate(-90.0), 200.0, 200.0);
        let v = page.texts.last().expect("run");
        assert!(v.h > v.w && (v.h - w).abs() < 0.01, "{v:?}");
    }

    /// Lines of report-sized text (8 to 14 px, regular and semibold, dark on white) drawn at
    /// `scale` with hinting on or off.
    fn sample(scale: f32, hinting: bool) -> Canvas {
        sample_lcd(scale, hinting, false)
    }

    /// [`sample`] with subpixel text on or off.
    fn sample_lcd(scale: f32, hinting: bool, lcd: bool) -> Canvas {
        let fonts = FontBook::bundled_only();
        fonts.set_hinting(hinting);
        let mut cv = Canvas::new(420.0, 260.0, scale).expect("canvas");
        cv.set_lcd_text(lcd);
        cv.fill(Rgb::WHITE, 1.0);
        let mut y = 14.0;
        for size in [8.0, 9.0, 10.0, 10.67, 12.0, 14.0] {
            for weight in [400, 600] {
                let t = Text::new(
                    "Sales by Region  North 1,234.5K  Total 910,047  Margin 38.0 %",
                    Font::new("Segoe UI", size).weight(weight),
                    Rgb(0x25, 0x25, 0x25),
                );
                cv.text(&fonts, &t, 6.3, y);
                y += size * 1.45;
            }
        }
        cv
    }

    /// Vertical and horizontal gradient energy of the luminance per unit of ink: how sharply
    /// the text's horizontal and vertical edges change from one device pixel to the next.
    fn edge_energy(cv: &Canvas) -> (f64, f64) {
        let (w, h) = (cv.pm.width() as usize, cv.pm.height() as usize);
        let lum: Vec<f64> = cv
            .pm
            .pixels()
            .iter()
            .map(|p| f64::from(p.green()) / 255.0)
            .collect();
        let at = |x: usize, y: usize| lum.get(y * w + x).copied().unwrap_or(1.0);
        let (mut v, mut hz, mut ink) = (0.0, 0.0, 0.0);
        for y in 0..h {
            for x in 0..w {
                ink += 1.0 - at(x, y);
                if y + 1 < h {
                    v += (at(x, y + 1) - at(x, y)).powi(2);
                }
                if x + 1 < w {
                    hz += (at(x + 1, y) - at(x, y)).powi(2);
                }
            }
        }
        (v / ink.max(1e-9), hz / ink.max(1e-9))
    }

    #[test]
    fn hinting_sharpens_small_upright_text() {
        // `BICRAFT_HINT_SHOTS=DIR` writes the samples for a look
        let shots = std::env::var_os("BICRAFT_HINT_SHOTS").map(std::path::PathBuf::from);
        for scale in [0.68, 1.0] {
            let (off, on) = (sample(scale, false), sample(scale, true));
            let ((v0, h0), (v1, h1)) = (edge_energy(&off), edge_energy(&on));
            println!(
                "scale {scale}: vertical edge energy ×{:.2} ({v0:.3} → {v1:.3}), horizontal ×{:.2}",
                v1 / v0,
                h1 / h0
            );
            assert!(v1 > v0 * 1.1, "scale {scale}: {v0} → {v1}");
            // horizontal hinting is off: the horizontal edges barely change
            assert!((h1 / h0 - 1.0).abs() < 0.15, "scale {scale}: {h0} → {h1}");
            if let Some(d) = &shots {
                let _ = std::fs::create_dir_all(d);
                let _ = off.pm.save_png(d.join(format!("text-{scale}-unhinted.png")));
                let _ = on.pm.save_png(d.join(format!("text-{scale}-hinted.png")));
            }
        }
    }

    #[test]
    fn rotated_text_is_not_hinted_and_widths_do_not_change() {
        let draw = |hinting: bool, rotate: f32| {
            let fonts = FontBook::bundled_only();
            fonts.set_hinting(hinting);
            let mut cv = Canvas::new(200.0, 200.0, 0.68).expect("canvas");
            let t = Text::new("Margin 38.0 %", Font::new("Segoe UI", 9.0), Rgb(0, 0, 0)).rotate(rotate);
            let w = cv.text(&fonts, &t, 100.0, 100.0);
            (w, cv.pm.data().to_vec())
        };
        let ((w0, up0), (w1, up1)) = (draw(false, 0.0), draw(true, 0.0));
        assert_eq!(w0, w1, "hinting keeps the advance");
        assert_ne!(up0, up1, "upright text is hinted");
        for r in [-90.0, 30.0, 180.0] {
            assert_eq!(draw(false, r), draw(true, r), "rotated by {r}: unhinted");
        }
    }

    /// Pixel-level luminance edge energy (vertical, horizontal), horizontal energy at subpixel
    /// resolution (the R, G, B values of a row in turn), and the colour fringes (mean and
    /// largest spread between the channels of an inked pixel, 0..1).
    fn lcd_metrics(cv: &Canvas) -> (f64, f64, f64, f64, f64) {
        let (w, h) = (cv.pm.width() as usize, cv.pm.height() as usize);
        let px = cv.pm.pixels();
        let ch = |p: &tiny_skia::PremultipliedColorU8| {
            [p.red(), p.green(), p.blue()].map(|c| f64::from(c) / 255.0)
        };
        let lum = |x: usize, y: usize| {
            px.get(y * w + x).map_or(1.0, |p| {
                ch(p)
                    .iter()
                    .zip([0.2126, 0.7152, 0.0722])
                    .map(|(c, k)| c * k)
                    .sum()
            })
        };
        let (mut v, mut hz, mut sub, mut ink, mut fr, mut fmax, mut n) = (0.0, 0.0, 0.0, 0.0, 0.0, 0f64, 0.0);
        for y in 0..h {
            let mut prev: Option<f64> = None;
            for x in 0..w {
                let a = lum(x, y);
                ink += 1.0 - a;
                if y + 1 < h {
                    v += (lum(x, y + 1) - a).powi(2);
                }
                if x + 1 < w {
                    hz += (lum(x + 1, y) - a).powi(2);
                }
                let c = px.get(y * w + x).map_or([1.0; 3], ch);
                for s in c {
                    if let Some(p) = prev {
                        sub += (s - p).powi(2);
                    }
                    prev = Some(s);
                }
                if a < 0.95 {
                    let spread =
                        c.iter().copied().fold(0.0, f64::max) - c.iter().copied().fold(1.0, f64::min);
                    fr += spread;
                    fmax = fmax.max(spread);
                    n += 1.0;
                }
            }
        }
        let k = ink.max(1e-9);
        (v / k, hz / k, sub / k, fr / f64::max(n, 1.0), fmax)
    }

    /// `cv` enlarged `k` times without smoothing (a crop to look at).
    fn zoomed(cv: &Canvas, x0: u32, y0: u32, w: u32, h: u32, k: u32) -> Option<Pixmap> {
        let mut out = Pixmap::new(w * k, h * k)?;
        for y in 0..h * k {
            for x in 0..w * k {
                let p = cv.pm.pixel(x0 + x / k, y0 + y / k)?;
                if let Some(o) = out.pixels_mut().get_mut((y * w * k + x) as usize) {
                    *o = p;
                }
            }
        }
        Some(out)
    }

    #[test]
    fn lcd_text_colours_upright_text_on_opaque_pixels() {
        // `BICRAFT_HINT_SHOTS=DIR` writes the samples and zoomed crops for a look
        let shots = std::env::var_os("BICRAFT_HINT_SHOTS").map(std::path::PathBuf::from);
        for scale in [0.6, 1.0] {
            let (gray, lcd) = (sample_lcd(scale, true, false), sample_lcd(scale, true, true));
            assert_eq!(
                gray.pm.data(),
                sample(scale, true).pm.data(),
                "off: today's pixels"
            );
            assert_eq!(gray.texts, lcd.texts, "text runs");
            // the ideal at subpixel resolution: each subpixel shows its own coverage (no filter)
            let ideal = sample_taps(scale, [0, 0, 256, 0, 0]);
            if let Some(d) = &shots {
                let _ = std::fs::create_dir_all(d);
                let _ = gray.pm.save_png(d.join(format!("text-{scale}-gray.png")));
                let _ = lcd.pm.save_png(d.join(format!("text-{scale}-lcd.png")));
                let (cw, chh) = ((80.0 * scale) as u32, (36.0 * scale) as u32);
                for (n, c) in [("gray", &gray), ("lcd", &lcd), ("unfiltered", &ideal)] {
                    if let Some(z) = zoomed(c, 0, 0, cw, chh, 12) {
                        let _ = z.save_png(d.join(format!("crop-{scale}-{n}.png")));
                    }
                }
            }
            let (g, l) = (lcd_metrics(&gray), lcd_metrics(&lcd));
            let (eg, el) = (subpixel_error(&gray, &ideal), subpixel_error(&lcd, &ideal));
            println!(
                "scale {scale}: subpixel error ×{:.2} smaller ({eg:.4} → {el:.4}); vertical edge energy \
                 ×{:.2}, horizontal ×{:.2} (pixels), ×{:.2} (subpixels); fringes mean {:.3} max {:.3}",
                eg / el,
                l.0 / g.0,
                l.1 / g.1,
                l.2 / g.2,
                l.3,
                l.4,
            );
            // the stripes differ per channel and follow the outlines more closely than whole
            // pixels; the rows (vertical hinting) are unchanged
            assert!(g.3 < 0.01 && l.3 > 0.05, "scale {scale}: fringes {g:?} → {l:?}");
            assert!(el < eg * 0.85, "scale {scale}: subpixel error {eg} → {el}");
            assert!(
                (l.0 / g.0 - 1.0).abs() < 0.2,
                "scale {scale}: vertical {g:?} → {l:?}"
            );
            // ClearType-like fringes, not garish: well under those of the unfiltered stripes
            assert!(l.3 < lcd_metrics(&ideal).3 * 0.8, "scale {scale}: fringes {l:?}");
        }
    }

    /// [`sample_lcd`] (hinted, subpixel) with other LCD filter taps.
    fn sample_taps(scale: f32, taps: [u32; 5]) -> Canvas {
        let fonts = FontBook::bundled_only();
        let mut cv = Canvas::new(420.0, 260.0, scale).expect("canvas");
        cv.set_lcd_text(true);
        cv.lcd_taps = taps;
        cv.fill(Rgb::WHITE, 1.0);
        let mut y = 14.0;
        for size in [8.0, 9.0, 10.0, 10.67, 12.0, 14.0] {
            for weight in [400, 600] {
                let t = Text::new(
                    "Sales by Region  North 1,234.5K  Total 910,047  Margin 38.0 %",
                    Font::new("Segoe UI", size).weight(weight),
                    Rgb(0x25, 0x25, 0x25),
                );
                cv.text(&fonts, &t, 6.3, y);
                y += size * 1.45;
            }
        }
        cv
    }

    /// Root mean square distance between the subpixel values (R, G, B in turn) of two canvases,
    /// over the pixels either inks.
    fn subpixel_error(a: &Canvas, b: &Canvas) -> f64 {
        let (mut e, mut n) = (0.0, 0.0);
        for (p, q) in a.pm.pixels().iter().zip(b.pm.pixels()) {
            let (x, y) = ([p.red(), p.green(), p.blue()], [q.red(), q.green(), q.blue()]);
            if x.iter().chain(&y).all(|c| *c >= 250) {
                continue;
            }
            for (u, v) in x.iter().zip(&y) {
                e += ((f64::from(*u) - f64::from(*v)) / 255.0).powi(2);
                n += 1.0;
            }
        }
        (e / f64::max(n, 1.0)).sqrt()
    }

    #[test]
    fn lcd_text_falls_back_to_grayscale() {
        let fonts = FontBook::bundled_only();
        let draw = |lcd: bool, t: &Text<'_>, opaque: bool| {
            let mut cv = Canvas::new(200.0, 200.0, 0.68).expect("canvas");
            cv.set_lcd_text(lcd);
            if opaque {
                cv.fill(Rgb::WHITE, 1.0);
            }
            let w = cv.text(&fonts, t, 100.0, 100.0);
            (w, cv.pm.data().to_vec(), cv.texts)
        };
        let t = Text::new("Margin 38.0 %", Font::new("Segoe UI", 9.0), Rgb(0, 0, 0));
        let (w0, up0, r0) = draw(false, &t, true);
        let (w1, up1, r1) = draw(true, &t, true);
        assert_eq!((w0, &r0), (w1, &r1), "same advance and run");
        assert_ne!(up0, up1, "upright text on white is subpixel");
        for r in [-90.0, 30.0, 180.0] {
            let tr = t.clone().rotate(r);
            assert_eq!(
                draw(false, &tr, true),
                draw(true, &tr, true),
                "rotated by {r}: grayscale"
            );
        }
        // over transparent pixels (no backdrop known), and translucent text: grayscale
        assert_eq!(draw(false, &t, false), draw(true, &t, false), "transparent");
        let mut faded = t.clone();
        faded.alpha = 0.5;
        assert_eq!(draw(false, &faded, true), draw(true, &faded, true), "translucent");
        // a layer of a canvas without subpixel text is a plain canvas
        let mut page = Canvas::new(100.0, 100.0, 1.0).expect("canvas");
        page.fill(Rgb::WHITE, 1.0);
        let layer = page.layer(10.0, 10.0, 50.0, 50.0).expect("layer");
        assert!(!layer.lcd && layer.backdrop.is_none());
    }

    #[test]
    fn lcd_text_on_a_layer_is_antialiased_against_what_it_is_drawn_over() {
        let fonts = FontBook::bundled_only();
        let scale = 0.6;
        let t = Text::new(
            "Sales by Region 1,234.5K",
            Font::new("Segoe UI", 10.0),
            Rgb(0x25, 0x25, 0x25),
        );
        let ground = |cv: &mut Canvas| {
            cv.fill(Rgb::WHITE, 1.0);
            cv.rect(0.0, 40.0, 300.0, 60.0, Rgb(0xE0, 0xF0, 0xFF), 1.0);
        };
        // drawn on the page itself
        let mut direct = Canvas::new(300.0, 120.0, scale).expect("canvas");
        direct.set_lcd_text(true);
        ground(&mut direct);
        // on a layer (a visual), then on a layer of the layer (its body), composited back
        let mut page = Canvas::new(300.0, 120.0, scale).expect("canvas");
        page.set_lcd_text(true);
        ground(&mut page);
        let (lx, ly) = (20.0, 30.0);
        let mut visual = page.layer(lx, ly, 250.0, 80.0).expect("layer");
        let (bx, by) = (5.0, 10.0);
        let mut body = visual.layer(bx, by, 240.0, 60.0).expect("layer");
        // whole device pixels, so both draw the same glyph positions
        let (ox, oy) = (
            ((lx * scale).round() + (bx * scale).round()) / scale,
            ((ly * scale).round() + (by * scale).round()) / scale,
        );
        let _ = direct.text(&fonts, &t, ox + 4.0, oy + 20.0);
        let _ = body.text(&fonts, &t, 4.0, 20.0);
        assert!(
            body.pm
                .pixels()
                .iter()
                .any(|p| p.alpha() > 0 && p.red() != p.blue()),
            "subpixel on the layer"
        );
        visual.draw(&body, bx, by, 1.0);
        page.draw(&visual, lx, ly, 1.0);
        let worst = direct
            .pm
            .pixels()
            .iter()
            .zip(page.pm.pixels())
            .map(|(a, b)| {
                [(a.red(), b.red()), (a.green(), b.green()), (a.blue(), b.blue())]
                    .iter()
                    .map(|(x, y)| x.abs_diff(*y))
                    .max()
                    .unwrap_or(0)
            })
            .max()
            .unwrap_or(0);
        assert!(
            worst <= 2,
            "the layered text is the text drawn on the page (worst {worst})"
        );
    }

    #[test]
    fn hostile_lcd_text_never_panics() {
        let fonts = FontBook::bundled_only();
        let mut cv = Canvas::new(400.0, 300.0, 1.0).expect("canvas");
        cv.set_lcd_text(true);
        cv.fill(Rgb::WHITE, 1.0);
        for (size, x, y) in [
            (4000.0, 0.0, 200.0),
            (9.0, f32::NAN, f32::INFINITY),
            (9.0, -1e9, 1e9),
            (0.01, 10.0, 10.0),
            (12.0, 395.0, 299.0),
        ] {
            let t = Text::new("Wg Total 910,047 ████", Font::new("Segoe UI", size), Rgb(0, 0, 0));
            let _ = cv.text(&fonts, &t, x, y);
        }
        for (x, y, w, h) in [
            (f32::NAN, 0.0, 10.0, 10.0),
            (1e9, -1e9, 50.0, 50.0),
            (390.0, 290.0, 100.0, 100.0),
        ] {
            if let Some(mut l) = cv.layer(x, y, w, h) {
                let _ = l.text(
                    &fonts,
                    &Text::new("x", Font::new("Segoe UI", 9.0), Rgb(0, 0, 0)),
                    1.0,
                    8.0,
                );
                cv.draw(&l, x, y, 1.0);
            }
        }
        // a run bigger than the subpixel mask allows is drawn in grayscale
        let mut big = Canvas::new(5_600.0, 2_000.0, 1.0).expect("canvas");
        big.set_lcd_text(true);
        big.fill(Rgb::WHITE, 1.0);
        let _ = big.text(
            &fonts,
            &Text::new("WWWWW", Font::new("Segoe UI", 1150.0), Rgb(0, 0, 0)),
            0.0,
            1500.0,
        );
        let bad = big
            .pm
            .pixels()
            .iter()
            .enumerate()
            .find(|(_, p)| p.red() != p.blue())
            .map(|(i, p)| (i % 5600, i / 5600, *p));
        assert!(bad.is_none(), "grayscale {bad:?}");
    }

    #[test]
    fn huge_canvas_is_refused() {
        assert!(Canvas::new(1e9, 10.0, 1.0).is_none());
        assert!(Canvas::new(f32::NAN, 10.0, 1.0).is_some());
    }
}
