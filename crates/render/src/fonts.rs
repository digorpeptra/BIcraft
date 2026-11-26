//! Fonts: the system's font files (indexed from their `name` and `OS/2` tables only) plus the
//! bundled Selawik fallback (SIL OFL 1.1, metric-compatible with Segoe UI; see
//! `THIRD_PARTY_NOTICES.md`). Glyph outlines come from `skrifa` and are drawn as tiny-skia
//! paths, so text scales and rotates like any other shape.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{
    DrawSettings, Engine, HintingInstance, HintingOptions, OutlinePen, SmoothMode, Target,
};
use skrifa::raw::TableProvider;
use skrifa::raw::tables::cmap::{CmapSubtable, PlatformId};
use skrifa::raw::tables::os2::SelectionFlags;
use skrifa::{FontRef, GlyphId, MetadataProvider};
use tiny_skia::{Path as SkPath, PathBuilder};

/// The bundled Selawik faces (SIL OFL 1.1, `crates/render/fonts/OFL-Selawik.txt`), shared with the
/// app's own text (`crates/ui/src/fonts.rs`) so both use one copy.
pub static SELAWIK_REGULAR: &[u8] = include_bytes!("../fonts/selawk.ttf");
pub static SELAWIK_SEMIBOLD: &[u8] = include_bytes!("../fonts/selawksb.ttf");
pub static SELAWIK_BOLD: &[u8] = include_bytes!("../fonts/selawkb.ttf");
pub static SELAWIK_LIGHT: &[u8] = include_bytes!("../fonts/selawkl.ttf");

/// The bundled fallback faces after Selawik (SIL OFL 1.1): glyph subsets of Noto fonts made by
/// `cargo xtask font-subset bundle` (`crates/render/fonts/SOURCE.txt`): Greek, Cyrillic, Latin
/// extensions, punctuation, superscripts, currency and letterlike symbols (Noto Sans, regular and
/// bold), box drawing and block elements (Noto Sans Mono), geometric shapes, dingbats and other
/// symbols (Noto Sans Symbols 2), arrows and mathematical operators (Noto Sans Math), what Noto
/// Sans Symbols adds, and common emoji (Noto Emoji, monochrome).
pub static NOTO_SANS_REGULAR: &[u8] = include_bytes!("../fonts/NotoSans-Regular-subset.ttf");
pub static NOTO_SANS_BOLD: &[u8] = include_bytes!("../fonts/NotoSans-Bold-subset.ttf");
pub static NOTO_SANS_MONO: &[u8] = include_bytes!("../fonts/NotoSansMono-Regular-subset.ttf");
pub static NOTO_SANS_SYMBOLS2: &[u8] = include_bytes!("../fonts/NotoSansSymbols2-Regular-subset.ttf");
pub static NOTO_SANS_MATH: &[u8] = include_bytes!("../fonts/NotoSansMath-Regular-subset.ttf");
pub static NOTO_SANS_SYMBOLS: &[u8] = include_bytes!("../fonts/NotoSansSymbols-Regular-subset.ttf");
pub static NOTO_EMOJI: &[u8] = include_bytes!("../fonts/NotoEmoji-Regular-subset.ttf");

/// The bundled stand-in for Calibri (SIL OFL 1.1, `crates/render/fonts/carlito/`): glyph subsets
/// of Carlito, which has Calibri's advance widths, made by `cargo xtask font-subset carlito`
/// and renamed "BIcraft Metric Sans" as Carlito's Reserved Font Name requires. Text asking for
/// Calibri takes these faces when no Calibri (or Carlito) face is installed, so it keeps
/// Calibri's widths ([`METRIC_TWINS`]).
pub static CARLITO_REGULAR: &[u8] = include_bytes!("../fonts/carlito/Carlito-Regular-subset.ttf");
pub static CARLITO_BOLD: &[u8] = include_bytes!("../fonts/carlito/Carlito-Bold-subset.ttf");
pub static CARLITO_ITALIC: &[u8] = include_bytes!("../fonts/carlito/Carlito-Italic-subset.ttf");
pub static CARLITO_BOLD_ITALIC: &[u8] = include_bytes!("../fonts/carlito/Carlito-BoldItalic-subset.ttf");

/// The family name of the bundled Carlito subsets in the book.
const METRIC_SANS: &str = "bicraft metric sans";

/// Families a missing family is replaced by, in order: faces with the same advance widths
/// (Carlito for Calibri, as LibreOffice and the Linux font configurations substitute it), so
/// text measures and wraps as with the font asked for.
const METRIC_TWINS: &[(&str, &[&str])] = &[("calibri", &["carlito", METRIC_SANS])];

/// The family names of the bundled fallback faces, in chain order (each face's `bytes`, weight).
pub const BUNDLED_FALLBACKS: &[(&str, &[u8], u16)] = &[
    ("bicraft noto sans", NOTO_SANS_REGULAR, 400),
    ("bicraft noto sans", NOTO_SANS_BOLD, 700),
    ("bicraft noto sans mono", NOTO_SANS_MONO, 400),
    ("bicraft noto sans symbols 2", NOTO_SANS_SYMBOLS2, 400),
    ("bicraft noto sans math", NOTO_SANS_MATH, 400),
    ("bicraft noto sans symbols", NOTO_SANS_SYMBOLS, 400),
    ("bicraft noto emoji", NOTO_EMOJI, 400),
];

/// Advance of the box drawn for a character no face has (ems).
const TOFU_ADVANCE: f32 = 0.6;
/// The face index of a box glyph (no face).
const TOFU: usize = usize::MAX;
/// Characters remembered as missing per book.
const MAX_MISSING: usize = 4096;

/// Largest font file read (bytes).
const MAX_FONT_FILE: u64 = 64 << 20;
/// Font files indexed per directory tree.
const MAX_FONT_FILES: usize = 4000;
/// Directory depth scanned under a font directory.
const MAX_DEPTH: usize = 6;
/// Faces kept loaded per render.
const MAX_LOADED: usize = 64;
/// Glyph outlines cached per face.
const MAX_GLYPHS: usize = 8192;
/// Hinting instances kept per face (one per pixel size); the map is emptied when full.
const MAX_HINT_SIZES: usize = 16;
/// Hinted glyph outlines cached per face, all sizes together; emptied when full.
const MAX_HINTED_GLYPHS: usize = 8192;
/// Pixel sizes (pixels per em on the device) whose outlines are hinted: above the range the
/// grid no longer matters, below it there is nothing to snap.
pub(crate) const HINT_PPEM: std::ops::RangeInclusive<f32> = 2.0..=48.0;
/// Hinted sizes are quantised to 1/64 pixel (as FreeType's 26.6 sizes).
const HINT_STEPS: f32 = 64.0;
/// Shaped lines cached per render.
const MAX_SHAPES: usize = 16_384;
/// Longest text (bytes) whose shaping is cached.
const MAX_CACHED_TEXT: usize = 512;
/// Characters laid out on one line.
const MAX_LINE_CHARS: usize = 20_000;

#[derive(Debug, Clone)]
enum FaceSource {
    Bundled(&'static [u8]),
    File(PathBuf, u32),
}

/// One indexed face.
#[derive(Debug, Clone)]
struct FaceEntry {
    /// Lower-cased family names (name IDs 1 and 16).
    families: Vec<String>,
    weight: u16,
    italic: bool,
    source: FaceSource,
}

/// A loaded face: its bytes plus cached metrics and glyph outlines (font units, y up).
struct LoadedFace {
    data: Arc<Vec<u8>>,
    index: u32,
    upem: f32,
    ascent: f32,
    descent: f32,
    glyphs: RefCell<HashMap<u16, Option<SkPath>>>,
    advances: RefCell<HashMap<char, Option<(u16, f32)>>>,
    /// Hinting instances per quantised pixel size (`None`: the face could not be hinted there).
    hinters: RefCell<HashMap<u32, Option<HintingInstance>>>,
    /// Hinted outlines (pixels, y up) per glyph and quantised pixel size.
    hinted: RefCell<HashMap<(u16, u32), Option<SkPath>>>,
}

/// The hinting of raster text: vertical only ("light", as Chromium with DirectWrite or FreeType's
/// light target draws text on Windows and Linux), so baselines, x-heights, cap heights and
/// horizontal stems sit on whole pixel rows while horizontal positions and advances keep their
/// unhinted values. The font's own instructions where it has them (TrueType in its
/// backward-compatible mode, which ignores horizontal moves; PostScript hints), else the
/// automatic hinter.
fn hint_options() -> HintingOptions {
    HintingOptions {
        engine: Engine::AutoFallback,
        target: Target::Smooth {
            mode: SmoothMode::Light,
            symmetric_rendering: true,
            preserve_linear_metrics: true,
        },
    }
}

impl LoadedFace {
    fn new(data: Arc<Vec<u8>>, index: u32) -> Option<Self> {
        let face = FontRef::from_index(&data, index).ok()?;
        // the tables every face needs (a face without them is not loaded)
        let head = face.head().ok()?;
        let hhea = face.hhea().ok()?;
        face.maxp().ok()?;
        let upem = f32::from(head.units_per_em().max(1));
        let os2 = face.os2().ok();
        // OS/2's typographic metrics when the face asks for them (USE_TYPO_METRICS, OS/2 version
        // 4 and later), else `hhea`'s; a zero `hhea` value falls back to OS/2's typographic, then
        // Windows, value
        let (asc, desc) = match &os2 {
            Some(o) if o.version() >= 4 && o.fs_selection().contains(SelectionFlags::USE_TYPO_METRICS) => {
                (o.s_typo_ascender(), o.s_typo_descender())
            }
            _ => {
                let pick = |hhea_value: i16, typo: i16, win: i16| match &os2 {
                    Some(_) if hhea_value == 0 => {
                        if typo == 0 {
                            win
                        } else {
                            typo
                        }
                    }
                    _ => hhea_value,
                };
                let (typo_a, typo_d, win_a, win_d) = os2.as_ref().map_or((0, 0, 0, 0), |o| {
                    (
                        o.s_typo_ascender(),
                        o.s_typo_descender(),
                        // the unsigned Windows values read as signed, the descent negated
                        o.us_win_ascent() as i16,
                        (o.us_win_descent() as i16).wrapping_neg(),
                    )
                });
                (
                    pick(hhea.ascender().to_i16(), typo_a, win_a),
                    pick(hhea.descender().to_i16(), typo_d, win_d),
                )
            }
        };
        Some(Self {
            upem,
            ascent: f32::from(asc) / upem,
            descent: (f32::from(desc) / upem).abs(),
            data,
            index,
            glyphs: RefCell::new(HashMap::new()),
            advances: RefCell::new(HashMap::new()),
            hinters: RefCell::new(HashMap::new()),
            hinted: RefCell::new(HashMap::new()),
        })
    }

    fn face(&self) -> Option<FontRef<'_>> {
        FontRef::from_index(&self.data, self.index).ok()
    }

    /// Glyph id and advance (in ems) of `c`, `None` when the face lacks it.
    fn glyph(&self, c: char) -> Option<(u16, f32)> {
        if let Some(x) = self.advances.borrow().get(&c) {
            return *x;
        }
        let r = self.face().and_then(|f| {
            let g = glyph_index(&f, c)?;
            let adv = f
                .hmtx()
                .ok()
                .and_then(|h| h.advance(GlyphId::from(g)))
                .unwrap_or(0);
            Some((g, f32::from(adv) / self.upem))
        });
        let mut m = self.advances.borrow_mut();
        if m.len() < MAX_GLYPHS {
            m.insert(c, r);
        }
        r
    }

    /// Shapes `text` (one face, one line) with OpenType shaping (harfrust: GSUB ligatures and
    /// contextual forms, GPOS and `kern` kerning, marks), HarfBuzz's default features as Chromium
    /// applies them. Glyph ids with their advance and offsets, in ems, and cluster (byte offset in
    /// `text`); `None` when the face's layout tables cannot be read.
    fn shape(&self, text: &str) -> Option<Vec<(u16, f32, f32, f32, usize)>> {
        let face = self.face()?;
        let data = harfrust::ShaperData::new(&face);
        let shaper = data.shaper(&face).build();
        let mut buf = harfrust::UnicodeBuffer::new();
        buf.push_str(text);
        buf.guess_segment_properties();
        let out = shaper.shape(buf, harfrust::ShapeOptions::new());
        let k = 1.0 / self.upem;
        Some(
            out.glyph_infos()
                .iter()
                .zip(out.glyph_positions())
                .map(|(i, p)| {
                    (
                        u16::try_from(i.glyph_id).unwrap_or(0),
                        p.x_advance as f32 * k,
                        p.x_offset as f32 * k,
                        p.y_offset as f32 * k,
                        i.cluster as usize,
                    )
                })
                .collect(),
        )
    }

    fn outline(&self, g: u16) -> Option<SkPath> {
        if let Some(p) = self.glyphs.borrow().get(&g) {
            return p.clone();
        }
        // skrifa and read-fonts are not panic-free on every hostile outline (a simple glyph with
        // flags but no points indexes an empty array): such a glyph is drawn as nothing
        let p = catch_unwind(AssertUnwindSafe(|| {
            self.face().and_then(|f| {
                let glyph = f.outline_glyphs().get(GlyphId::from(g))?;
                let mut b = Outline(PathBuilder::new());
                let unscaled = DrawSettings::unhinted(Size::unscaled(), LocationRef::default());
                glyph.draw(unscaled, &mut b).ok()?;
                b.0.finish()
            })
        }))
        .ok()
        .flatten();
        let mut m = self.glyphs.borrow_mut();
        if m.len() < MAX_GLYPHS {
            m.insert(g, p.clone());
        }
        p
    }

    /// The outline of glyph `g` hinted at `key` / [`HINT_STEPS`] pixels per em, in pixels (y up);
    /// `None` when the face cannot be hinted at that size or the glyph fails to hint (the caller
    /// draws the unhinted outline instead). Both caches are bounded.
    fn hinted_outline(&self, g: u16, key: u32) -> Option<SkPath> {
        if let Some(p) = self.hinted.borrow().get(&(g, key)) {
            return p.clone();
        }
        // a panic while hinting (see `outline`) drops the hinter of that size: unhinted from then on
        let p = match catch_unwind(AssertUnwindSafe(|| self.hint_glyph(g, key))) {
            Ok(p) => p,
            Err(_) => {
                if let Ok(mut m) = self.hinters.try_borrow_mut() {
                    m.insert(key, None);
                }
                None
            }
        };
        let mut m = self.hinted.borrow_mut();
        if m.len() >= MAX_HINTED_GLYPHS {
            m.clear();
        }
        m.insert((g, key), p.clone());
        p
    }

    fn hint_glyph(&self, g: u16, key: u32) -> Option<SkPath> {
        let face = self.face()?;
        let outlines = face.outline_glyphs();
        if !self.hinters.borrow().contains_key(&key) {
            let ppem = key as f32 / HINT_STEPS;
            let h =
                HintingInstance::new(&outlines, Size::new(ppem), LocationRef::default(), hint_options()).ok();
            let mut m = self.hinters.borrow_mut();
            if m.len() >= MAX_HINT_SIZES {
                m.clear();
            }
            m.insert(key, h);
        }
        let hinters = self.hinters.borrow();
        let h = hinters.get(&key)?.as_ref()?;
        let glyph = outlines.get(GlyphId::from(g))?;
        let mut b = Outline(PathBuilder::new());
        glyph.draw(DrawSettings::hinted(h, false), &mut b).ok()?;
        let p = b.0.finish()?;
        // instructions that throw points far off the em square are not trusted
        let r = p.bounds();
        let lim = 16.0 * (key as f32 / HINT_STEPS) + 16.0;
        [r.left(), r.right(), r.top(), r.bottom()]
            .iter()
            .all(|v| v.abs() <= lim)
            .then_some(p)
    }

    #[cfg(test)]
    fn hint_cache_sizes(&self) -> (usize, usize) {
        (self.hinters.borrow().len(), self.hinted.borrow().len())
    }
}

struct Outline(PathBuilder);

/// The glyph of `c` in `face`, read as ttf-parser did: the first Unicode `cmap` subtable (platform
/// 0, Windows BMP, or a Windows full-repertoire table in format 12 or 13) that maps it, in the
/// table's order.
fn glyph_index(face: &FontRef<'_>, c: char) -> Option<u16> {
    let cmap = face.cmap().ok()?;
    let data = cmap.offset_data();
    for rec in cmap.encoding_records() {
        let Ok(sub) = rec.subtable(data) else { continue };
        let unicode = match rec.platform_id() {
            PlatformId::Unicode => true,
            PlatformId::Windows => {
                rec.encoding_id() == 1
                    || (rec.encoding_id() == 10
                        && matches!(sub, CmapSubtable::Format12(_) | CmapSubtable::Format13(_)))
            }
            _ => false,
        };
        if !unicode {
            continue;
        }
        if let Some(g) = sub.map_codepoint(c) {
            return u16::try_from(g.to_u32()).ok();
        }
    }
    None
}

impl OutlinePen for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to(x, y);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.0.quad_to(x1, y1, x, y);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.0.cubic_to(x1, y1, x2, y2, x, y);
    }
    fn close(&mut self) {
        self.0.close();
    }
}

/// A text style: CSS-like family list, size in pixels, weight, italic.
#[derive(Debug, Clone, PartialEq)]
pub struct Font {
    pub family: String,
    pub size: f32,
    pub weight: u16,
    pub italic: bool,
}

impl Font {
    pub fn new(family: &str, size: f32) -> Self {
        Self {
            family: family.to_string(),
            size,
            weight: 400,
            italic: false,
        }
    }
    pub fn weight(mut self, w: u16) -> Self {
        self.weight = w;
        self
    }
    pub fn size(mut self, s: f32) -> Self {
        self.size = s;
        self
    }
    pub fn italic(mut self, i: bool) -> Self {
        self.italic = i;
        self
    }
}

/// One positioned glyph of a laid-out run.
pub(crate) struct Glyph {
    pub path: SkPath,
    /// Pen position (pixels from the run start), vertical offset (pixels, y up) and the scale
    /// (pixels per font unit).
    pub x: f32,
    pub y: f32,
    pub scale: f32,
    /// Whether `path` is hinted at the device size (see [`FontBook::layout_at`]; read by tests).
    #[cfg_attr(not(test), allow(dead_code))]
    pub hinted: bool,
}

/// One glyph of a run laid out for vector output ([`FontBook::layout_vector`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct VecGlyph {
    /// The face (an index in the book); `None` for the missing-glyph box.
    pub face: Option<usize>,
    pub gid: u16,
    /// Pen position and vertical offset (pixels from the run start, y up).
    pub x: f32,
    pub y: f32,
}

/// Families tried after the requested ones, in order: Segoe UI and its metric-compatible Selawik,
/// common sans fonts, then symbol, emoji and other-script faces for characters the first ones
/// lack (Windows names first, then the usual Linux ones).
const FALLBACK_FAMILIES: &[&str] = &[
    "segoe ui",
    "selawik",
    "arial",
    "liberation sans",
    "dejavu sans",
    "noto sans",
    "segoe ui symbol",
    "segoe ui emoji",
    "segoe ui historic",
    "nirmala ui",
    "leelawadee ui",
    "ebrima",
    "gadugi",
    "microsoft yahei",
    "microsoft jhenghei",
    "yu gothic ui",
    "malgun gothic",
    "noto sans cjk sc",
    "noto sans cjk jp",
    "noto sans cjk kr",
    "noto sans arabic",
    "noto sans hebrew",
    "noto sans devanagari",
    "noto sans thai",
    "noto sans symbols",
    "noto sans symbols2",
    "noto emoji",
    "noto color emoji",
    "droid sans fallback",
    "wenquanyi micro hei",
    // the bundled fallback faces: last, so the system's own fonts win where they are installed
    "bicraft noto sans",
    "bicraft noto sans mono",
    "bicraft noto sans symbols 2",
    "bicraft noto sans math",
    "bicraft noto sans symbols",
    "bicraft noto emoji",
];

/// The faces available to one render.
pub struct FontBook {
    entries: Arc<Vec<FaceEntry>>,
    loaded: RefCell<HashMap<usize, Option<Arc<LoadedFace>>>>,
    resolved: RefCell<HashMap<(String, u16, bool), Vec<usize>>>,
    shapes: RefCell<HashMap<ShapeKey, Arc<Shaped>>>,
    /// Characters drawn as a box because no face has them (see [`FontBook::missing_glyphs`]).
    missing: RefCell<std::collections::BTreeSet<char>>,
    /// Whether a real DIN face is present (else Segoe UI stands in, see [`DIN_STAND_IN`]).
    din: bool,
    /// Whether raster text is hinted at the device size (on by default; see
    /// [`FontBook::set_hinting`]).
    hinting: std::cell::Cell<bool>,
}

/// Power BI's DIN (`wf_standard-font`) is about 0.92 times as wide as Segoe UI at the same size
/// (Desktop 2.158: "100%" at 12 px is 26.95 px wide in DIN, 29.2 px in Segoe UI). Segoe UI
/// stands in for it at this fraction of the requested size, like a CSS `size-adjust` on a
/// fallback face; the text keeps its requested size.
const DIN_STAND_IN: f32 = 0.92;

/// A shaping cache key: family list, weight, italic, text.
type ShapeKey = (String, u16, bool, String);

/// One glyph of a shaped line: its face (index in the book), glyph id, pen position and vertical
/// offset (ems, y up).
#[derive(Debug, Clone, Copy)]
struct ShapedGlyph {
    face: usize,
    gid: u16,
    x: f32,
    y: f32,
}

/// A shaped line: its glyphs and total advance (ems).
#[derive(Debug, Default)]
struct Shaped {
    glyphs: Vec<ShapedGlyph>,
    width: f32,
}

/// How far a face of weight `have` is from the `want`ed weight, in CSS Fonts' matching order:
/// 400–500 try up to 500, then lighter, then heavier; below 400 lighter first, then heavier;
/// above 500 heavier first, then lighter.
fn weight_distance(want: u16, have: u16) -> u32 {
    let (w, h) = (u32::from(want), u32::from(have));
    if (400..=500).contains(&w) {
        if h >= w && h <= 500 {
            h - w
        } else if h < w {
            1000 + w - h
        } else {
            2000 + h - 500
        }
    } else if w < 400 {
        if h <= w { w - h } else { 1000 + h - w }
    } else if h >= w {
        h - w
    } else {
        1000 + w - h
    }
}

/// A font's size in pixels, bounded (0 when not a number).
fn px_size(font: &Font) -> f32 {
    if font.size.is_finite() {
        font.size.clamp(0.0, 4000.0)
    } else {
        0.0
    }
}

/// Combining marks, joiners, variation selectors and emoji modifiers: shaped with the character
/// before them.
fn is_mark_like(c: char) -> bool {
    matches!(u32::from(c),
        0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0xFE20..=0xFE2F
        | 0x200C | 0x200D | 0xFE00..=0xFE0F | 0xE0100..=0xE01EF | 0x1F3FB..=0x1F3FF)
}

/// Default-ignorable characters: soft hyphen, zero-width spaces and joiners, bidi controls,
/// word joiners, variation selectors, the BOM and tags (drawn as nothing, never as a box).
fn is_ignorable(c: char) -> bool {
    matches!(u32::from(c),
        0x00AD | 0x034F | 0x061C | 0x115F | 0x1160 | 0x17B4 | 0x17B5 | 0x180B..=0x180F
        | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x206F | 0x3164 | 0xFE00..=0xFE0F
        | 0xFEFF | 0xFFA0 | 0xFFF0..=0xFFF8 | 0x1BCA0..=0x1BCA3 | 0x1D173..=0x1D17A
        | 0xE0000..=0xE0FFF)
}

impl std::fmt::Debug for FontBook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FontBook")
            .field("faces", &self.entries.len())
            .finish()
    }
}

fn bundled() -> Vec<FaceEntry> {
    [
        (SELAWIK_REGULAR, 400),
        (SELAWIK_SEMIBOLD, 600),
        (SELAWIK_BOLD, 700),
        (SELAWIK_LIGHT, 300),
    ]
    .into_iter()
    .map(|(b, w)| ("selawik", b, w, false))
    .chain([
        (METRIC_SANS, CARLITO_REGULAR, 400, false),
        (METRIC_SANS, CARLITO_BOLD, 700, false),
        (METRIC_SANS, CARLITO_ITALIC, 400, true),
        (METRIC_SANS, CARLITO_BOLD_ITALIC, 700, true),
    ])
    .chain(BUNDLED_FALLBACKS.iter().map(|(f, b, w)| (*f, *b, *w, false)))
    .map(|(f, b, w, italic)| FaceEntry {
        families: vec![f.into()],
        weight: w,
        italic,
        source: FaceSource::Bundled(b),
    })
    .collect()
}

/// The box drawn for a character no face has (font units of a 1000-unit em, y up): an outlined
/// rectangle the height of a capital, like a browser's missing-glyph box.
fn tofu_path() -> Option<SkPath> {
    static P: OnceLock<Option<SkPath>> = OnceLock::new();
    P.get_or_init(|| {
        let (l, r, b, t, w) = (70.0, 530.0, 0.0, 700.0, 60.0);
        let mut p = PathBuilder::new();
        // outer clockwise, inner counter-clockwise: a hole under the non-zero rule
        p.move_to(l, b);
        p.line_to(l, t);
        p.line_to(r, t);
        p.line_to(r, b);
        p.close();
        p.move_to(l + w, b + w);
        p.line_to(r - w, b + w);
        p.line_to(r - w, t - w);
        p.line_to(l + w, t - w);
        p.close();
        p.finish()
    })
    .clone()
}

/// The platform's font directories.
pub fn system_font_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    if cfg!(windows) {
        let win = std::env::var_os("WINDIR")
            .or_else(|| std::env::var_os("SystemRoot"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:\\Windows"));
        out.push(win.join("Fonts"));
        if let Some(l) = std::env::var_os("LOCALAPPDATA") {
            out.push(PathBuf::from(l).join("Microsoft").join("Windows").join("Fonts"));
        }
    } else if cfg!(target_os = "macos") {
        out.push(PathBuf::from("/System/Library/Fonts"));
        out.push(PathBuf::from("/Library/Fonts"));
        if let Some(h) = &home {
            out.push(h.join("Library/Fonts"));
        }
    } else {
        out.push(PathBuf::from("/usr/share/fonts"));
        out.push(PathBuf::from("/usr/local/share/fonts"));
        if let Some(d) = std::env::var_os("XDG_DATA_HOME") {
            out.push(PathBuf::from(d).join("fonts"));
        } else if let Some(h) = &home {
            out.push(h.join(".local/share/fonts"));
        }
        if let Some(h) = &home {
            out.push(h.join(".fonts"));
        }
    }
    out
}

type IndexCache = Mutex<HashMap<PathBuf, Arc<Vec<FaceEntry>>>>;

fn index_cache() -> &'static IndexCache {
    static CACHE: OnceLock<IndexCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The faces of one directory tree, indexed once per process.
fn index_dir(dir: &Path) -> Arc<Vec<FaceEntry>> {
    if let Ok(c) = index_cache().lock()
        && let Some(v) = c.get(dir)
    {
        return Arc::clone(v);
    }
    let mut files = Vec::new();
    collect_files(dir, 0, &mut files);
    files.sort();
    let mut out = Vec::new();
    for f in files {
        index_file(&f, &mut out);
    }
    let v = Arc::new(out);
    if let Ok(mut c) = index_cache().lock() {
        c.insert(dir.to_path_buf(), Arc::clone(&v));
    }
    v
}

fn collect_files(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > MAX_DEPTH || out.len() >= MAX_FONT_FILES {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        if out.len() >= MAX_FONT_FILES {
            return;
        }
        let p = e.path();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            collect_files(&p, depth + 1, out);
        } else {
            let ext = p
                .extension()
                .and_then(|x| x.to_str())
                .map(str::to_ascii_lowercase)
                .unwrap_or_default();
            if matches!(ext.as_str(), "ttf" | "otf" | "ttc" | "otc") {
                out.push(p);
            }
        }
    }
}

fn be_u16(b: &[u8], at: usize) -> Option<u16> {
    let s = b.get(at..at.checked_add(2)?)?;
    Some(u16::from_be_bytes([*s.first()?, *s.get(1)?]))
}
fn be_u32(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes([
        *s.first()?,
        *s.get(1)?,
        *s.get(2)?,
        *s.get(3)?,
    ]))
}

fn read_at(f: &mut File, off: u64, len: usize) -> Option<Vec<u8>> {
    if len > (4 << 20) {
        return None;
    }
    f.seek(SeekFrom::Start(off)).ok()?;
    let mut buf = vec![0u8; len];
    f.read_exact(&mut buf).ok()?;
    Some(buf)
}

/// Indexes one font file from its table directory, `name` and `OS/2` tables only.
fn index_file(path: &Path, out: &mut Vec<FaceEntry>) {
    let Ok(mut f) = File::open(path) else { return };
    let Some(head) = read_at(&mut f, 0, 12) else {
        return;
    };
    let offsets: Vec<u64> = if head.get(0..4) == Some(b"ttcf") {
        let n = be_u32(&head, 8).unwrap_or(0).min(64) as usize;
        let Some(dir) = read_at(&mut f, 12, n * 4) else {
            return;
        };
        (0..n)
            .filter_map(|i| be_u32(&dir, i * 4).map(u64::from))
            .collect()
    } else {
        vec![0]
    };
    for (i, off) in offsets.into_iter().enumerate() {
        if let Some(e) = index_face(&mut f, off, path, i as u32) {
            out.push(e);
        }
    }
}

fn index_face(f: &mut File, off: u64, path: &Path, index: u32) -> Option<FaceEntry> {
    let hdr = read_at(f, off, 12)?;
    let n = usize::from(be_u16(&hdr, 4)?).min(128);
    let dir = read_at(f, off.checked_add(12)?, n * 16)?;
    let mut name = None;
    let mut os2 = None;
    for i in 0..n {
        let rec = i * 16;
        let tag = dir.get(rec..rec + 4)?;
        let t_off = u64::from(be_u32(&dir, rec + 8)?);
        let t_len = be_u32(&dir, rec + 12)? as usize;
        if tag == b"name" {
            name = Some((t_off, t_len));
        } else if tag == b"OS/2" {
            os2 = Some((t_off, t_len));
        }
    }
    let (no, nl) = name?;
    let name_data = read_at(f, no, nl)?;
    let mut families = Vec::new();
    let mut sub = String::new();
    for n in name_records(&name_data)? {
        let id = n.name_id;
        if id != 1 && id != 16 && id != 2 {
            continue;
        }
        let Some(s) = n.text else { continue };
        if id == 2 {
            if sub.is_empty() || n.language_id == 0x0409 {
                sub = s.to_ascii_lowercase();
            }
            continue;
        }
        let s = s.trim().to_lowercase();
        if !s.is_empty() && !families.contains(&s) {
            families.push(s);
        }
    }
    if families.is_empty() {
        return None;
    }
    let (mut weight, mut italic) = (400u16, sub.contains("italic") || sub.contains("oblique"));
    if let Some((oo, ol)) = os2
        && let Some(d) = read_at(f, oo, ol.min(128))
    {
        if let Some(w) = be_u16(&d, 4) {
            weight = w.clamp(1, 1000);
        }
        if let Some(sel) = be_u16(&d, 62) {
            italic = italic || sel & 1 != 0;
        }
    }
    let len = f.metadata().map(|m| m.len()).unwrap_or(u64::MAX);
    if len > MAX_FONT_FILE {
        return None;
    }
    Some(FaceEntry {
        families,
        weight,
        italic,
        source: FaceSource::File(path.to_path_buf(), index),
    })
}

/// One record of a `name` table: its IDs and its text when it is Unicode (UTF-16BE: platform 0, or
/// Windows symbol or BMP encodings).
struct NameRecord {
    name_id: u16,
    language_id: u16,
    text: Option<String>,
}

/// The records of a `name` table (versions 0 and 1), read up to the first one that is malformed
/// (an unknown platform or a string outside the storage); `None` when the header is.
fn name_records(data: &[u8]) -> Option<Vec<NameRecord>> {
    let version = be_u16(data, 0)?;
    let count = usize::from(be_u16(data, 2)?);
    let storage_offset = usize::from(be_u16(data, 4)?);
    let mut pos = 6;
    match version {
        0 => {}
        1 => {
            let lang_tags = usize::from(be_u16(data, 6)?);
            pos += 2 + lang_tags * 4;
        }
        _ => return None,
    }
    let records = data.get(pos..pos.checked_add(count * 12)?)?;
    let storage = data.get(storage_offset.max(pos + count * 12)..)?;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let r = i * 12;
        let platform = be_u16(records, r)?;
        if platform > 4 {
            break;
        }
        let encoding = be_u16(records, r + 2)?;
        let language_id = be_u16(records, r + 4)?;
        let name_id = be_u16(records, r + 6)?;
        let len = usize::from(be_u16(records, r + 8)?);
        let off = usize::from(be_u16(records, r + 10)?);
        let Some(bytes) = storage.get(off..off + len) else {
            break;
        };
        let unicode = platform == 0 || (platform == 3 && encoding <= 1);
        let text = unicode
            .then(|| {
                let units: Vec<u16> = bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|p| u16::from_be_bytes(*p))
                    .collect();
                String::from_utf16(&units).ok()
            })
            .flatten();
        out.push(NameRecord {
            name_id,
            language_id,
            text,
        });
    }
    Some(out)
}

/// Splits a CSS-like family list into lower-cased names, mapping Power BI's web font aliases
/// (`wf_segoe-ui_semibold`…) to a family and a weight.
pub(crate) fn family_list(css: &str) -> Vec<(String, Option<u16>)> {
    let mut out = Vec::new();
    for raw in css.split(',').take(8) {
        let n = raw
            .trim()
            .trim_matches(|c| c == '\'' || c == '"')
            .trim()
            .to_lowercase();
        if n.is_empty() {
            continue;
        }
        let mapped: (String, Option<u16>) = match n.as_str() {
            "wf_segoe-ui_normal" | "wf_standard-font" | "segoe ui normal" => ("segoe ui".into(), None),
            "wf_segoe-ui_semibold" => ("segoe ui".into(), Some(600)),
            "wf_segoe-ui_bold" => ("segoe ui".into(), Some(700)),
            "wf_segoe-ui_light" => ("segoe ui".into(), Some(300)),
            "wf_segoe-ui_semilight" => ("segoe ui".into(), Some(350)),
            "wf_din" | "din" => ("din".into(), None),
            "wf_standard-font_light" => ("segoe ui".into(), Some(300)),
            "sans-serif" | "helvetica" | "helvetica neue" | "system-ui" => ("arial".into(), None),
            _ => (n.clone(), None),
        };
        out.push(mapped);
    }
    out
}

/// A weight suffix of a family name ("Segoe UI Semibold" → ("segoe ui", 600)).
fn split_weight(name: &str) -> Option<(String, u16)> {
    const SUFFIXES: &[(&str, u16)] = &[
        (" semibold", 600),
        (" semilight", 350),
        (" light", 300),
        (" bold", 700),
        (" black", 900),
        (" thin", 100),
        (" demibold", 600),
        (" medium", 500),
    ];
    SUFFIXES.iter().find_map(|(s, w)| {
        name.strip_suffix(s)
            .filter(|b| !b.is_empty())
            .map(|b| (b.to_string(), *w))
    })
}

impl FontBook {
    /// The bundled faces only (deterministic: what the tests use).
    pub fn bundled_only() -> Self {
        Self::with(Vec::new(), false)
    }

    /// The bundled faces, the system's fonts (when `system`) and the faces under `dirs`.
    pub fn with(dirs: Vec<PathBuf>, system: bool) -> Self {
        let mut all: Vec<FaceEntry> = Vec::new();
        let mut scan = dirs;
        if system {
            scan.extend(system_font_dirs());
        }
        for d in scan {
            all.extend(index_dir(&d).iter().cloned());
        }
        all.extend(bundled());
        let din = all.iter().any(|e| e.families.iter().any(|f| f == "din"));
        Self {
            din,
            entries: Arc::new(all),
            loaded: RefCell::new(HashMap::new()),
            resolved: RefCell::new(HashMap::new()),
            shapes: RefCell::new(HashMap::new()),
            missing: RefCell::new(std::collections::BTreeSet::new()),
            hinting: std::cell::Cell::new(true),
        }
    }

    /// Turns the hinting of raster text on or off (on by default). Off, glyphs are the unhinted
    /// outlines scaled to the device, as vector output always draws them.
    pub fn set_hinting(&self, on: bool) {
        self.hinting.set(on);
    }

    /// Whether a system or configured face of this family is present.
    pub fn has_family(&self, family: &str) -> bool {
        let f = family.to_lowercase();
        self.entries.iter().any(|e| e.families.contains(&f))
    }

    fn load(&self, i: usize) -> Option<Arc<LoadedFace>> {
        if let Some(x) = self.loaded.borrow().get(&i) {
            return x.clone();
        }
        let e = self.entries.get(i)?;
        let lf = match &e.source {
            FaceSource::Bundled(b) => LoadedFace::new(Arc::new(b.to_vec()), 0),
            FaceSource::File(p, idx) => {
                let ok = std::fs::metadata(p).is_ok_and(|m| m.len() <= MAX_FONT_FILE);
                if ok {
                    std::fs::read(p)
                        .ok()
                        .and_then(|d| LoadedFace::new(Arc::new(d), *idx))
                } else {
                    None
                }
            }
        }
        .map(Arc::new);
        let mut m = self.loaded.borrow_mut();
        if m.len() >= MAX_LOADED {
            m.clear();
        }
        m.insert(i, lf.clone());
        lf
    }

    /// The best face of `family` for `weight`/`italic`.
    fn best_in(&self, family: &str, weight: u16, italic: bool) -> Option<usize> {
        let mut best: Option<(u32, usize)> = None;
        for (i, e) in self.entries.iter().enumerate() {
            if !e.families.iter().any(|f| f == family) {
                continue;
            }
            let score = weight_distance(weight, e.weight) + if e.italic != italic { 5000 } else { 0 };
            if best.is_none_or(|(s, _)| score < s) {
                best = Some((score, i));
            }
        }
        best.map(|(_, i)| i)
    }

    /// Face indices to try for a font, best first: the requested families, then Segoe UI, the
    /// bundled Selawik, then common sans fonts (for glyphs the first faces lack).
    fn chain(&self, font: &Font) -> Vec<usize> {
        let key = (font.family.clone(), font.weight, font.italic);
        if let Some(c) = self.resolved.borrow().get(&key) {
            return c.clone();
        }
        let mut out: Vec<usize> = Vec::new();
        let push = |i: Option<usize>, out: &mut Vec<usize>| {
            if let Some(i) = i
                && !out.contains(&i)
            {
                out.push(i);
            }
        };
        // the weight the fallback faces use: the first family's (an alias or a weight suffix)
        let mut fallback_weight = None;
        for (fam, w) in family_list(&font.family) {
            // an alias weight (wf_segoe-ui_semibold) wins unless the text asks for bolder
            let weight = match w {
                Some(w) if font.weight > 400 => w.max(font.weight),
                Some(w) => w,
                None => font.weight,
            };
            let direct = self.best_in(&fam, weight, font.italic);
            let mut used = weight;
            let mut base = fam.clone();
            let mut found = direct.is_some();
            if direct.is_some() {
                push(direct, &mut out);
            } else if let Some((b, w2)) = split_weight(&fam) {
                used = if font.weight >= 600 {
                    font.weight.max(w2)
                } else {
                    w2
                };
                let i = self.best_in(&b, used, font.italic);
                found = i.is_some();
                push(i, &mut out);
                base = b;
            }
            if !found
                && let Some((_, twins)) = METRIC_TWINS.iter().find(|(f, _)| *f == base)
                && let Some(i) = twins.iter().find_map(|t| self.best_in(t, used, font.italic))
            {
                push(Some(i), &mut out);
            }
            if fam == "din" && direct.is_none() {
                // Power BI's DIN (card callouts) is not a system font: Segoe UI stands in.
                push(self.best_in("segoe ui", font.weight, font.italic), &mut out);
            }
            fallback_weight.get_or_insert(used);
        }
        let fw = fallback_weight.unwrap_or(font.weight);
        for fam in FALLBACK_FAMILIES {
            push(self.best_in(fam, fw, font.italic), &mut out);
        }
        if out.is_empty() {
            let selawik = self
                .entries
                .iter()
                .position(|e| e.families.iter().any(|f| f == "selawik"));
            push(selawik, &mut out);
        }
        self.resolved.borrow_mut().insert(key, out.clone());
        out
    }

    /// Whether italic text must be slanted by hand (the face found is upright).
    pub fn needs_oblique(&self, font: &Font) -> bool {
        font.italic
            && self
                .chain(font)
                .first()
                .and_then(|i| self.entries.get(*i))
                .is_some_and(|e| !e.italic)
    }

    /// The primary face of a font, loaded (the first face of its chain that loads).
    fn primary(&self, font: &Font) -> Option<Arc<LoadedFace>> {
        self.chain(font).into_iter().take(8).find_map(|i| self.load(i))
    }

    /// Ascent and descent of the font's primary face, in pixels.
    pub fn metrics(&self, font: &Font) -> (f32, f32) {
        let size = self.size_px(font);
        match self.primary(font) {
            Some(f) => (f.ascent * size, f.descent * size),
            None => (size * 0.93, size * 0.23),
        }
    }

    /// The pixel size the faces are drawn at: the font's, scaled for the DIN stand-in.
    fn size_px(&self, font: &Font) -> f32 {
        let size = px_size(font);
        let first_din = !self.din
            && family_list(font.family.split(',').next().unwrap_or(""))
                .first()
                .is_some_and(|(f, _)| f == "din");
        if first_din { size * DIN_STAND_IN } else { size }
    }

    /// `text` on one line, shaped and positioned in ems (cached per font and text).
    fn shaped(&self, font: &Font, text: &str) -> Arc<Shaped> {
        let cacheable = text.len() <= MAX_CACHED_TEXT;
        let key = (font.family.clone(), font.weight, font.italic, text.to_string());
        if cacheable && let Some(s) = self.shapes.borrow().get(&key) {
            return Arc::clone(s);
        }
        let s = Arc::new(self.shape_line(font, text));
        if cacheable {
            let mut m = self.shapes.borrow_mut();
            if m.len() >= MAX_SHAPES {
                m.clear();
            }
            m.insert(key, Arc::clone(&s));
        }
        s
    }

    /// Splits `text` into runs of one face each (per-character fallback along the font's chain:
    /// a character the first faces lack comes from the next face that has it, as browsers do),
    /// then shapes each run.
    fn shape_line(&self, font: &Font, text: &str) -> Shaped {
        let chain = self.chain(font);
        let mut found: HashMap<char, Option<usize>> = HashMap::new();
        let mut runs: Vec<(Option<usize>, String)> = Vec::new();
        for c in text.chars().take(MAX_LINE_CHARS) {
            let c = match c {
                '\u{a0}' | '\u{202f}' | '\t' => ' ',
                c if c.is_control() => continue,
                c => c,
            };
            let cur = runs.last().and_then(|r| r.0);
            if is_ignorable(c) && cur.is_none_or(|f| !self.has_glyph(f, c)) {
                // a joiner or selector no face of the run has draws nothing (never a box)
                continue;
            }
            // a combining mark the run's face lacks: the base moves to a face that has both
            if is_mark_like(c)
                && let Some(f) = cur
                && !self.has_glyph(f, c)
                && let Some(base) = runs.last().and_then(|r| r.1.chars().last())
                && let Some(g) = chain
                    .iter()
                    .copied()
                    .find(|&i| self.has_glyph(i, c) && self.has_glyph(i, base))
            {
                if let Some((_, s)) = runs.last_mut() {
                    s.pop();
                }
                if runs.last().is_some_and(|r| r.1.is_empty()) {
                    runs.pop();
                }
                match runs.last_mut() {
                    Some((Some(h), s)) if *h == g => {
                        s.push(base);
                        s.push(c);
                    }
                    _ => runs.push((Some(g), [base, c].into_iter().collect())),
                }
                continue;
            }
            // marks, joiners and variation selectors stay with the run they follow when its face
            // has them; spaces too
            let face = match cur {
                Some(f) if (is_mark_like(c) || c == ' ') && self.has_glyph(f, c) => Some(f),
                _ => *found
                    .entry(c)
                    .or_insert_with(|| chain.iter().copied().find(|&i| self.has_glyph(i, c))),
            };
            match runs.last_mut() {
                Some((f, s)) if *f == face && face.is_some() => s.push(c),
                _ => runs.push((face, c.to_string())),
            }
        }
        let mut out = Shaped::default();
        let mut x = 0.0f32;
        let tofu = |c: char, x: &mut f32, out: &mut Shaped| {
            out.glyphs.push(ShapedGlyph {
                face: TOFU,
                gid: 0,
                x: *x,
                y: 0.0,
            });
            *x += TOFU_ADVANCE;
            let mut m = self.missing.borrow_mut();
            if m.len() < MAX_MISSING {
                m.insert(c);
            }
        };
        for (face, s) in runs {
            let shaped = face.and_then(|i| self.load(i).and_then(|f| f.shape(&s)).map(|g| (i, g)));
            match shaped {
                Some((i, glyphs)) => {
                    for (gid, adv, dx, dy, cluster) in glyphs {
                        if gid == 0 {
                            // the face's .notdef: a box like any missing character
                            let c = s
                                .get(cluster..)
                                .and_then(|t| t.chars().next())
                                .unwrap_or('\u{fffd}');
                            if !is_ignorable(c) {
                                tofu(c, &mut x, &mut out);
                            }
                            continue;
                        }
                        out.glyphs.push(ShapedGlyph {
                            face: i,
                            gid,
                            x: x + dx,
                            y: dy,
                        });
                        x += adv;
                    }
                }
                // no face has these characters: a box each
                None => {
                    for c in s.chars() {
                        tofu(c, &mut x, &mut out);
                    }
                }
            }
        }
        out.width = x;
        out
    }

    /// The characters this book has drawn as a box so far (no face has them), sorted: what
    /// `report.render` lists once per render.
    pub fn missing_glyphs(&self) -> Vec<char> {
        self.missing.borrow().iter().copied().collect()
    }

    /// The characters of `text` that no face of `font`'s chain has (the coverage query; each
    /// one is drawn as a box and reported). Controls and default-ignorable characters (joiners,
    /// variation selectors) are not counted: they draw nothing.
    pub fn missing(&self, font: &Font, text: &str) -> Vec<char> {
        let chain = self.chain(font);
        let mut out: Vec<char> = Vec::new();
        for c in text.chars().take(MAX_LINE_CHARS) {
            if c.is_control() || is_ignorable(c) || out.contains(&c) {
                continue;
            }
            let c = if matches!(c, '\u{a0}' | '\u{202f}') {
                ' '
            } else {
                c
            };
            if !chain.iter().any(|&i| self.has_glyph(i, c)) {
                out.push(c);
            }
        }
        out
    }

    fn has_glyph(&self, i: usize, c: char) -> bool {
        self.load(i).is_some_and(|f| f.glyph(c).is_some())
    }

    /// Lays out `text` on one line: glyphs and the total advance (pixels).
    #[cfg(test)]
    pub(crate) fn layout(&self, font: &Font, text: &str) -> (Vec<Glyph>, f32) {
        self.layout_at(font, text, None)
    }

    /// Lays out `text` on one line for a raster drawn at `device` pixels per logical pixel
    /// (`None`: no device grid, e.g. rotated text): where the device size is in [`HINT_PPEM`] and
    /// hinting is on, each glyph is the outline hinted at that size (pixels, y up, so its
    /// `scale` is `1 / device`) and vertical offsets fall on whole device pixels. Positions
    /// and the total advance are those of the unhinted layout, so widths never change.
    pub(crate) fn layout_at(&self, font: &Font, text: &str, device: Option<f32>) -> (Vec<Glyph>, f32) {
        let size = self.size_px(font);
        let shaped = self.shaped(font, text);
        let mut out = Vec::with_capacity(shaped.glyphs.len());
        let hint = device
            .filter(|d| self.hinting.get() && d.is_finite() && *d > 0.0)
            .and_then(|d| {
                let ppem = size * d;
                HINT_PPEM
                    .contains(&ppem)
                    .then(|| (d, (ppem * HINT_STEPS).round() as u32))
            });
        for g in &shaped.glyphs {
            if g.face == TOFU {
                if let Some(path) = tofu_path() {
                    out.push(Glyph {
                        path,
                        x: g.x * size,
                        y: 0.0,
                        scale: size / 1000.0,
                        hinted: false,
                    });
                }
                continue;
            }
            let Some(face) = self.load(g.face) else { continue };
            if let Some((d, key)) = hint
                && let Some(path) = face.hinted_outline(g.gid, key)
            {
                out.push(Glyph {
                    path,
                    x: g.x * size,
                    y: (g.y * size * d).round() / d,
                    scale: 1.0 / d,
                    hinted: true,
                });
            } else if let Some(path) = face.outline(g.gid) {
                out.push(Glyph {
                    path,
                    x: g.x * size,
                    y: g.y * size,
                    scale: size / face.upem,
                    hinted: false,
                });
            }
        }
        (out, shaped.width * size)
    }

    /// Lays out `text` on one line for vector output (PDF): every shaped glyph, spaces included,
    /// with its face (`None`: the missing-glyph box) and glyph id, pen position and vertical
    /// offset in pixels (y up); and the pixel size the faces are drawn at.
    pub(crate) fn layout_vector(&self, font: &Font, text: &str) -> (Vec<VecGlyph>, f32) {
        let size = self.size_px(font);
        let shaped = self.shaped(font, text);
        let glyphs = shaped
            .glyphs
            .iter()
            .map(|g| VecGlyph {
                face: (g.face != TOFU).then_some(g.face),
                gid: g.gid,
                x: g.x * size,
                y: g.y * size,
            })
            .collect();
        (glyphs, size)
    }

    /// The bytes of face `i` and its index in its collection file.
    pub(crate) fn face_bytes(&self, i: usize) -> Option<(Arc<Vec<u8>>, u32)> {
        self.load(i).map(|f| (Arc::clone(&f.data), f.index))
    }

    /// The outline of glyph `gid` of `face` (`None`: the missing-glyph box) in font units (y up),
    /// with the face's units per em.
    pub(crate) fn glyph_outline(&self, face: Option<usize>, gid: u16) -> Option<(SkPath, f32)> {
        match face {
            None => tofu_path().map(|p| (p, 1000.0)),
            Some(i) => {
                let f = self.load(i)?;
                f.outline(gid).map(|p| (p, f.upem))
            }
        }
    }

    /// The glyph id of `c` in face `i` (what a PDF's ToUnicode map reverses).
    pub(crate) fn char_gid(&self, i: usize, c: char) -> Option<u16> {
        self.load(i).and_then(|f| f.glyph(c)).map(|g| g.0)
    }

    /// Width of `text` in pixels.
    pub fn measure(&self, font: &Font, text: &str) -> f32 {
        self.shaped(font, text).width * self.size_px(font)
    }

    /// `text` cut with an ellipsis to fit `max` pixels ("" when not even one character fits).
    pub fn fit(&self, font: &Font, text: &str, max: f32) -> String {
        if self.measure(font, text) <= max {
            return text.to_string();
        }
        let ell = self.measure(font, "…");
        let chars: Vec<char> = text.chars().take(4000).collect();
        // binary search the longest prefix that fits with the ellipsis
        let (mut lo, mut hi) = (0usize, chars.len());
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            let s: String = chars.iter().take(mid).collect();
            if self.measure(font, s.trim_end()) + ell <= max {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        if lo == 0 {
            return if ell <= max { "…".into() } else { String::new() };
        }
        let s: String = chars.iter().take(lo).collect();
        format!("{}…", s.trim_end())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_fonts_measure_and_fit() {
        let b = FontBook::bundled_only();
        let f = Font::new("Segoe UI", 12.0);
        let w = b.measure(&f, "Hello");
        assert!(w > 20.0 && w < 40.0, "{w}");
        let bold = b.measure(&f.clone().weight(700), "Hello");
        assert!(bold >= w, "{bold} {w}");
        let s = b.fit(&f, "A long label that does not fit", 60.0);
        assert!(s.ends_with('…') && b.measure(&f, &s) <= 60.0, "{s}");
        assert_eq!(b.fit(&f, "x", 0.5), "");
        let (g, adv) = b.layout(&f, "Ab");
        assert_eq!(g.len(), 2);
        assert!((adv - b.measure(&f, "Ab")).abs() < 1e-3);
    }

    /// The width of `text` at `px` from a face's own advance widths (no kerning).
    fn advance_sum(face: &[u8], text: &str, px: f32) -> f32 {
        let f = FontRef::new(face).unwrap();
        let upem = f32::from(f.head().unwrap().units_per_em());
        let m = f.glyph_metrics(Size::unscaled(), LocationRef::default());
        let cmap = f.charmap();
        text.chars()
            .map(|c| m.advance_width(cmap.map(c).unwrap()).unwrap() / upem * px)
            .sum()
    }

    #[test]
    fn calibri_text_takes_carlito_widths() {
        let b = FontBook::bundled_only();
        let text = "Total Sales 2024 (€) — Région";
        for (fam, w, italic, face) in [
            ("Calibri", 400, false, CARLITO_REGULAR),
            ("Calibri", 700, false, CARLITO_BOLD),
            ("Calibri", 400, true, CARLITO_ITALIC),
            ("Calibri", 700, true, CARLITO_BOLD_ITALIC),
            ("'Calibri', sans-serif", 400, false, CARLITO_REGULAR),
            ("Calibri Light", 400, false, CARLITO_REGULAR),
        ] {
            let f = Font::new(fam, 14.0).weight(w).italic(italic);
            let got = b.measure(&f, text);
            let want = advance_sum(face, text, 14.0);
            assert!(
                (got - want).abs() < 1e-3,
                "{fam} {w} {italic}: {got} vs Carlito {want}"
            );
            let selawik = b.measure(&Font::new("Segoe UI", 14.0).weight(w).italic(italic), text);
            assert!((got - selawik).abs() > 1.0, "{fam}: {got} vs Selawik {selawik}");
            // real italics: no slant by hand
            assert!(!b.needs_oblique(&f), "{fam} {w} {italic}");
        }
        // characters the subset lacks still come from the fallback chain, not as boxes
        let f = Font::new("Calibri", 14.0);
        assert!(b.missing(&f, "Δ → ✓").is_empty());
        // other families keep their chain: Segoe UI text never takes Carlito
        let seg = b.measure(&Font::new("Segoe UI", 14.0), text);
        assert!((seg - advance_sum(SELAWIK_REGULAR, text, 14.0)).abs() < 1e-3);
    }

    #[test]
    fn family_aliases() {
        let l = family_list("'Segoe UI Semibold', wf_segoe-ui_semibold, helvetica, arial, sans-serif");
        assert_eq!(l.first().map(|x| x.0.as_str()), Some("segoe ui semibold"));
        assert_eq!(l.get(1), Some(&("segoe ui".to_string(), Some(600))));
        assert_eq!(split_weight("segoe ui semibold"), Some(("segoe ui".into(), 600)));
    }

    #[test]
    fn shaping_kerns_falls_back_and_keeps_marks() {
        let b = FontBook::bundled_only();
        let f = Font::new("Segoe UI", 20.0);
        // Selawik has no kerning pairs; Segoe UI's GPOS pulls "To" together (Windows only)
        let sys = FontBook::with(Vec::new(), true);
        if sys.has_family("segoe ui") {
            let to = sys.measure(&f, "To");
            assert!(to < sys.measure(&f, "T") + sys.measure(&f, "o") - 1.0, "{to}");
        }
        // a combining accent has no advance of its own and stays in the run
        let e = b.measure(&f, "e");
        let e_acute = b.measure(&f, "e\u{301}");
        assert!((e_acute - e).abs() < 0.5, "{e} {e_acute}");
        // characters no face has (CJK, a documented gap): a visible box each, and reported
        let (g, w) = b.layout(&f, "\u{4e2d}\u{6587}");
        assert!(
            g.len() == 2 && (w - 2.0 * TOFU_ADVANCE * 20.0).abs() < 1e-3,
            "{w}"
        );
        assert_eq!(b.missing_glyphs(), vec!['\u{4e2d}', '\u{6587}']);
        // mixed runs: Latin glyphs and the box drawn
        let (g, _) = b.layout(&f, "a\u{4e2d}b");
        assert_eq!(g.len(), 3);
        // joiners and selectors no face has draw nothing, never a box
        let (g, _) = b.layout(&f, "a\u{200b}\u{fe0f}b");
        assert_eq!(g.len(), 2);
        // controls are dropped, no-break spaces measure as spaces
        assert!((b.measure(&f, "a\u{0}\u{7}b") - b.measure(&f, "ab")).abs() < 1e-3);
        assert!((b.measure(&f, "a\u{a0}b") - b.measure(&f, "a b")).abs() < 1e-3);
    }

    #[test]
    fn hostile_text_is_bounded() {
        let b = FontBook::bundled_only();
        let f = Font::new("Segoe UI", 12.0);
        let long = "W\u{301}\u{200d}".repeat(30_000);
        let w = b.measure(&f, &long);
        assert!(w.is_finite() && w > 0.0);
        let (g, _) = b.layout(&f, &long);
        assert!(g.len() <= 2 * MAX_LINE_CHARS);
        for s in ["\u{fe0f}", "\u{200d}\u{200d}", "\u{1f600}\u{1f3fb}", "\u{10ffff}"] {
            assert!(b.measure(&f, s).is_finite());
        }
        assert_eq!(b.measure(&f.clone().size(f32::NAN), "abc"), 0.0);
    }

    #[test]
    fn regular_text_gets_the_regular_face() {
        // regression: a 400 request used to pick the Light (bundled) or Semilight (Windows) face
        assert!(weight_distance(400, 400) < weight_distance(400, 300));
        assert!(weight_distance(400, 500) < weight_distance(400, 350));
        assert!(weight_distance(400, 350) < weight_distance(400, 600));
        assert!(weight_distance(700, 900) < weight_distance(700, 600));
        assert!(weight_distance(300, 200) < weight_distance(300, 400));
        let b = FontBook::bundled_only();
        let pick = |w: u16| {
            b.chain(&Font::new("Segoe UI", 12.0).weight(w))
                .first()
                .and_then(|i| b.entries.get(*i))
                .map(|e| e.weight)
        };
        assert_eq!(
            [pick(400), pick(600), pick(700), pick(300), pick(500)],
            [Some(400), Some(600), Some(700), Some(300), Some(400)]
        );
    }

    #[test]
    fn standard_light_font_is_segoe_ui_light() {
        assert_eq!(
            family_list("wf_standard-font_light"),
            vec![("segoe ui".to_string(), Some(300))]
        );
        // with only the bundled faces, the light alias lands on Selawik Light
        let b = FontBook::bundled_only();
        let light = b.measure(&Font::new("wf_standard-font_light", 40.0), "Total");
        let regular = b.measure(&Font::new("Segoe UI", 40.0), "Total");
        assert!(light < regular, "{light} {regular}");
    }

    #[test]
    fn greek_delta_has_a_glyph_in_every_weight() {
        // regression: "CA Δ N-1" drew "CA   N-1" (Selawik has no Greek, nothing after it did)
        let b = FontBook::bundled_only();
        for w in [300, 400, 600, 700] {
            let f = Font::new("Segoe UI", 12.0).weight(w);
            assert_eq!(
                b.missing(&f, "CA Δ N-1, CA Δ% N-1"),
                Vec::<char>::new(),
                "weight {w}"
            );
            let (g, _) = b.layout(&f, "CAΔN-1");
            assert_eq!(g.len(), 6, "weight {w}");
        }
    }

    #[test]
    fn fallback_faces_match_selawik_metrics() {
        let cap = |b: &[u8]| {
            let f = FontRef::new(b).unwrap();
            f32::from(f.os2().unwrap().s_cap_height().unwrap()) / f32::from(f.head().unwrap().units_per_em())
        };
        let s = cap(SELAWIK_REGULAR);
        for b in [NOTO_SANS_REGULAR, NOTO_SANS_BOLD] {
            assert!((cap(b) / s - 1.0).abs() < 0.03, "{} vs {s}", cap(b));
        }
        // every bundled face parses and maps characters
        for (fam, b, _) in BUNDLED_FALLBACKS {
            let f = FontRef::new(b).unwrap();
            assert!(f.maxp().unwrap().num_glyphs() > 10, "{fam}");
            assert!(LoadedFace::new(Arc::new(b.to_vec()), 0).is_some(), "{fam}");
        }
    }

    #[test]
    fn hinted_layout_keeps_positions_and_snaps_to_the_device_grid() {
        let b = FontBook::bundled_only();
        let f = Font::new("Segoe UI", 9.0);
        let (plain, w0) = b.layout_at(&f, "Hxg 1,234.5K", None);
        let (hinted, w1) = b.layout_at(&f, "Hxg 1,234.5K", Some(0.68));
        assert_eq!(w0, w1);
        assert_eq!(plain.len(), hinted.len());
        for (p, h) in plain.iter().zip(&hinted) {
            assert!(!p.hinted && h.hinted);
            assert_eq!((p.x, p.y), (h.x, h.y));
            assert!((h.scale - 1.0 / 0.68).abs() < 1e-6);
        }
        // the cap height of H lands on a whole device row
        let top = hinted.first().map(|g| g.path.bounds().bottom()).unwrap_or(0.0);
        assert!(top > 3.0 && (top - top.round()).abs() < 0.02, "{top}");
        // too large on the device, or hinting off: unhinted
        assert!(b.layout_at(&f, "H", Some(8.0)).0.iter().all(|g| !g.hinted));
        b.set_hinting(false);
        assert!(b.layout_at(&f, "H", Some(0.68)).0.iter().all(|g| !g.hinted));
        // the vector (PDF) outlines stay in font units, unhinted
        b.set_hinting(true);
        let _ = b.layout_at(&f, "H", Some(0.68));
        let (vg, _) = b.layout_vector(&f, "H");
        let g0 = vg.first().copied().unwrap();
        let (path, upem) = b.glyph_outline(g0.face, g0.gid).unwrap();
        let face = b.load(g0.face.unwrap()).unwrap();
        assert_eq!(upem, face.upem);
        assert_eq!(path.bounds(), face.outline(g0.gid).unwrap().bounds());
        assert!(path.bounds().bottom() > 100.0, "font units");
    }

    #[test]
    fn hint_caches_are_bounded() {
        let face = LoadedFace::new(Arc::new(SELAWIK_REGULAR.to_vec()), 0).unwrap();
        for key in 0..(MAX_HINT_SIZES as u32 * 3) {
            for g in 0..40u16 {
                let _ = face.hinted_outline(g, 256 + key * 16);
            }
            let (sizes, glyphs) = face.hint_cache_sizes();
            assert!(
                sizes <= MAX_HINT_SIZES && glyphs <= MAX_HINTED_GLYPHS,
                "{sizes} {glyphs}"
            );
        }
        for g in 0..(MAX_HINTED_GLYPHS as u16 + 500) {
            let _ = face.hinted_outline(g % 600, 640 + u32::from(g / 600));
        }
        assert!(face.hint_cache_sizes().1 <= MAX_HINTED_GLYPHS);
    }

    #[test]
    fn hostile_fonts_never_panic_when_hinted() {
        // bytes flipped inside the outline and instruction tables of the bundled faces
        let mut seed: u64 = 0x5eed;
        let mut next = move || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) as usize
        };
        for base in [
            SELAWIK_REGULAR,
            SELAWIK_BOLD,
            NOTO_SANS_REGULAR,
            NOTO_SANS_MATH,
            CARLITO_REGULAR,
        ] {
            let tables: Vec<(usize, usize)> = (0..usize::from(be_u16(base, 4).unwrap_or(0)))
                .filter_map(|i| {
                    let at = 12 + 16 * i;
                    let tag = base.get(at..at + 4)?;
                    let off = be_u32(base, at + 8)? as usize;
                    let len = be_u32(base, at + 12)? as usize;
                    [&b"glyf"[..], b"loca", b"fpgm", b"prep", b"cvt ", b"maxp", b"hmtx"]
                        .contains(&tag)
                        .then_some((off, len.max(1)))
                })
                .collect();
            for round in 0..60 {
                let mut data = base.to_vec();
                for _ in 0..(1 + round % 12) {
                    let (off, len) = tables[next() % tables.len().max(1)];
                    if let Some(b) = data.get_mut(off + next() % len) {
                        *b = next() as u8;
                    }
                }
                if round % 10 == 9 {
                    data.truncate(next() % data.len());
                }
                let Some(face) = LoadedFace::new(Arc::new(data), 0) else {
                    continue;
                };
                for g in 0..48u16 {
                    for key in [130, 520, 3072] {
                        let _ = face.hinted_outline(g * 7, key);
                    }
                }
            }
        }
    }

    #[test]
    fn missing_font_dir_is_ignored() {
        let b = FontBook::with(vec![PathBuf::from("/definitely/not/here")], false);
        assert!(b.measure(&Font::new("Nope", 10.0), "abc") > 0.0);
    }
}
