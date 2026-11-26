//! PDF output (`report.export_pdf`): the vector operations the canvases record under a
//! [`crate::canvas::VectorScope`] written as one PDF page per report page, at the page's aspect
//! ratio (one report pixel = 0.75 pt, 96 px per inch).
//!
//! - Shapes, chart marks, lines and borders are vector paths; transparency is an `ExtGState`.
//! - Text is real text: each face used is embedded as a TrueType subset (`ttf_subset.rs`) under
//!   a `Type0`/`CIDFontType2` font with `Identity-H` encoding (glyph ids as codes) and a
//!   `ToUnicode` map, so it can be selected, searched and extracted. A face whose `OS/2`
//!   `fsType` forbids embedding a subset is drawn as glyph outlines with invisible Helvetica
//!   text over it, so it is still searchable.
//! - Images (and canvases too large to record, a visual at a time) are Flate-compressed RGB
//!   images with a soft mask for their transparency.
//!
//! The file layout follows ISO 32000-1 (PDF 1.7): header, numbered objects, a classic
//! cross-reference table and trailer. The output is deterministic: the same input gives the same
//! bytes (no dates, no random ids).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::Write as _;

use tiny_skia::{PathSegment, Pixmap, Transform};

use crate::canvas::{LayerBody, Op, Rgb, TextOp};
use crate::fonts::FontBook;
use crate::ttf_subset::{self, FaceInfo};

/// Points per report pixel (72 / 96).
pub const PT_PER_PX: f32 = 0.75;

/// A font of the document: the face it embeds, or why it is drawn as outlines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfFont {
    /// PostScript name, with the subset tag when embedded (`ABCDEF+Selawik`).
    pub name: String,
    /// Embedded as a subset (else drawn as outlines: the licence forbids embedding, or the
    /// outlines are not TrueType).
    pub embedded: bool,
    /// Glyphs used.
    pub glyphs: usize,
}

/// A face in use: its font object and the glyphs drawn.
struct FaceUse {
    id: usize,
    name: String,
    info: FaceInfo,
    gids: BTreeSet<u16>,
    /// What each glyph stands for (the ToUnicode map).
    uni: BTreeMap<u16, String>,
    /// Advance widths (font units), for the TJ adjustments.
    adv: BTreeMap<u16, u16>,
}

struct Doc<'a> {
    objs: Vec<Option<Vec<u8>>>,
    fonts: &'a FontBook,
    faces: BTreeMap<usize, Option<FaceUse>>,
    gstates: BTreeMap<(u16, u16), String>,
    /// Axial shadings by their end colours (top, bottom), from (0, 0) to (0, 1).
    shadings: BTreeMap<(Rgb, Rgb), String>,
    xobjects: Vec<(String, usize)>,
    resources: usize,
    helvetica: Option<usize>,
    /// The structure tree's elements (`elems[0]` is the `Document`).
    elems: Vec<Elem>,
    /// The elements open while a page is written (its section, then a visual's).
    stack: Vec<Frame>,
    /// The content stream being written: where its marked content's ids go.
    mark: Option<Mark>,
    /// The parent tree: for each content stream's `StructParents` key, the element of each
    /// marked-content id.
    parent_tree: Vec<(usize, Vec<usize>)>,
    next_key: usize,
}

/// A structure element (ISO 32000-1 §14.7.2).
struct Elem {
    id: usize,
    /// The standard structure type (`Document`, `Sect`, `P`, `Figure`).
    role: &'static str,
    parent: Option<usize>,
    title: Option<String>,
    alt: Option<String>,
    /// The page object it is on.
    page: Option<usize>,
    kids: Vec<Kid>,
}

enum Kid {
    Elem(usize),
    /// Marked content `mcid` of the page's contents, or of form XObject `stm` drawn on it.
    Content {
        page: usize,
        stm: Option<usize>,
        mcid: usize,
    },
}

/// An open element: the visual's alt text, and its `Figure` once its graphics need one.
struct Frame {
    elem: usize,
    alt: Option<String>,
    figure: Option<usize>,
    /// The paragraph the last text run went to: its element, baseline and where the run ended.
    para: Option<(usize, f32, f32)>,
}

/// The content stream being written: its `StructParents` key, the page it is on, the form
/// XObject it is (if any), and the element of each marked-content id so far.
struct Mark {
    key: usize,
    page: usize,
    stm: Option<usize>,
    parents: Vec<usize>,
}

/// Most structure elements a document gets; past it, text and graphics are artifacts.
const MAX_ELEMS: usize = 500_000;

/// A number as PDF writes it: up to 4 decimals, trailing zeros dropped, never NaN.
fn num(x: f32) -> String {
    let x = if x.is_finite() { x.clamp(-1e7, 1e7) } else { 0.0 };
    let s = format!("{x:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" || s.is_empty() {
        "0".into()
    } else {
        s.to_string()
    }
}

fn matrix(t: &Transform) -> String {
    format!(
        "{} {} {} {} {} {}",
        num(t.sx),
        num(t.ky),
        num(t.kx),
        num(t.sy),
        num(t.tx),
        num(t.ty)
    )
}

fn color(c: Rgb) -> String {
    format!(
        "{} {} {}",
        num(f32::from(c.0) / 255.0),
        num(f32::from(c.1) / 255.0),
        num(f32::from(c.2) / 255.0)
    )
}

fn deflate(b: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    if e.write_all(b).is_err() {
        return Vec::new();
    }
    e.finish().unwrap_or_default()
}

/// A PDF text string: UTF-16BE with its byte order mark, in hex.
fn text_string(s: &str) -> String {
    let mut out = String::from("<FEFF");
    for u in s.encode_utf16().take(4000) {
        let _ = write!(out, "{u:04X}");
    }
    out.push('>');
    out
}

/// A 6-letter subset tag from the glyphs used (deterministic).
fn subset_tag(face: usize, gids: &BTreeSet<u16>) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in face
        .to_le_bytes()
        .into_iter()
        .chain(gids.iter().flat_map(|g| g.to_le_bytes()))
    {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    (0..6)
        .map(|i| char::from(b'A' + ((h >> (i * 5)) % 26) as u8))
        .collect()
}

/// Appends a path's segments.
fn path_ops(out: &mut String, p: &tiny_skia::Path) {
    let mut last = (0.0f32, 0.0f32);
    for seg in p.segments() {
        match seg {
            PathSegment::MoveTo(a) => {
                let _ = write!(out, "{} {} m ", num(a.x), num(a.y));
                last = (a.x, a.y);
            }
            PathSegment::LineTo(a) => {
                let _ = write!(out, "{} {} l ", num(a.x), num(a.y));
                last = (a.x, a.y);
            }
            PathSegment::QuadTo(c, a) => {
                let (c1x, c1y) = (
                    last.0 + 2.0 / 3.0 * (c.x - last.0),
                    last.1 + 2.0 / 3.0 * (c.y - last.1),
                );
                let (c2x, c2y) = (a.x + 2.0 / 3.0 * (c.x - a.x), a.y + 2.0 / 3.0 * (c.y - a.y));
                let _ = write!(
                    out,
                    "{} {} {} {} {} {} c ",
                    num(c1x),
                    num(c1y),
                    num(c2x),
                    num(c2y),
                    num(a.x),
                    num(a.y)
                );
                last = (a.x, a.y);
            }
            PathSegment::CubicTo(c1, c2, a) => {
                let _ = write!(
                    out,
                    "{} {} {} {} {} {} c ",
                    num(c1.x),
                    num(c1.y),
                    num(c2.x),
                    num(c2.y),
                    num(a.x),
                    num(a.y)
                );
                last = (a.x, a.y);
            }
            PathSegment::Close => out.push_str("h "),
        }
    }
}

/// Glyph space → run space for a glyph at (`x`, `y` up) of a `size` px face with `upem` units.
fn glyph_tf(op: &TextOp, x: f32, y: f32, scale: f32) -> Transform {
    let k = if op.oblique { 0.2 } else { 0.0 };
    op.tf
        .pre_translate(x, -y)
        .pre_concat(Transform::from_row(scale, 0.0, k * scale, -scale, 0.0, 0.0))
}

impl<'a> Doc<'a> {
    fn reserve(&mut self) -> usize {
        self.objs.push(None);
        self.objs.len()
    }

    fn set(&mut self, id: usize, body: Vec<u8>) {
        if let Some(slot) = self.objs.get_mut(id.wrapping_sub(1)) {
            *slot = Some(body);
        }
    }

    fn add(&mut self, body: Vec<u8>) -> usize {
        let id = self.reserve();
        self.set(id, body);
        id
    }

    fn stream(&mut self, dict: &str, data: &[u8]) -> usize {
        let id = self.reserve();
        self.stream_at(id, dict, data);
        id
    }

    /// A stream written as object `id` (reserved earlier).
    fn stream_at(&mut self, id: usize, dict: &str, data: &[u8]) {
        let z = deflate(data);
        let mut b = format!("<< {dict} /Filter /FlateDecode /Length {} >>\nstream\n", z.len()).into_bytes();
        b.extend_from_slice(&z);
        b.extend_from_slice(b"\nendstream");
        self.set(id, b);
    }

    /// The name of the graphics state with these fill and stroke opacities.
    fn gs(&mut self, fill: f32, stroke: f32) -> Option<String> {
        let q = |a: f32| (a.clamp(0.0, 1.0) * 1000.0).round() as u16;
        let key = (q(fill), q(stroke));
        if key == (1000, 1000) {
            return None;
        }
        let n = self.gstates.len();
        Some(
            self.gstates
                .entry(key)
                .or_insert_with(|| format!("GS{n}"))
                .clone(),
        )
    }

    fn with_gs(&mut self, out: &mut String, fill: f32, stroke: f32) {
        if let Some(g) = self.gs(fill, stroke) {
            let _ = write!(out, "/{g} gs ");
        }
    }

    /// A new structure element under `parent`, on the page being written.
    fn elem(
        &mut self,
        role: &'static str,
        parent: usize,
        title: Option<String>,
        alt: Option<String>,
    ) -> Option<usize> {
        if self.elems.len() >= MAX_ELEMS {
            return None;
        }
        let id = self.reserve();
        let i = self.elems.len();
        self.elems.push(Elem {
            id,
            role,
            parent: Some(parent),
            title,
            alt,
            page: self.mark.as_ref().map(|m| m.page),
            kids: Vec::new(),
        });
        self.elems.get_mut(parent)?.kids.push(Kid::Elem(i));
        Some(i)
    }

    /// The next marked-content id of the stream being written, as content of element `elem`.
    fn mcid(&mut self, elem: usize) -> Option<usize> {
        let m = self.mark.as_mut()?;
        let n = m.parents.len();
        m.parents.push(elem);
        let kid = Kid::Content {
            page: m.page,
            stm: m.stm,
            mcid: n,
        };
        self.elems.get_mut(elem)?.kids.push(kid);
        Some(n)
    }

    /// Opens the marked content of text run `t`: part of the visual's figure when the visual has
    /// alt text (the alt text stands for it), else a paragraph of the open element; a run that
    /// goes on where the last one ended, on its baseline, joins its paragraph (a text box's
    /// words). An artifact when there is no structure to put it in. Closed with `EMC`.
    fn open_text(&mut self, out: &mut String, t: &TextOp) {
        let (ty, x0) = (t.tf.ty, t.tf.tx);
        let near = t.size.max(1.0) * 0.5;
        let para = match self.stack.last() {
            Some(Frame { alt: Some(_), .. }) => return self.open_graphic(out),
            Some(Frame {
                para: Some((p, y, end)),
                ..
            }) if (y - ty).abs() < 0.5 && (end - x0).abs() < near => Some(*p),
            Some(Frame { elem, .. }) => {
                let parent = *elem;
                self.elem("P", parent, None, None)
            }
            None => None,
        };
        if let Some(top) = self.stack.last_mut() {
            top.para = para.map(|p| (p, ty, x0 + t.width * t.tf.sx));
        }
        match para.and_then(|p| self.mcid(p)) {
            Some(n) => {
                let _ = writeln!(out, "/P <</MCID {n}>> BDC");
            }
            None => out.push_str("/Artifact BMC\n"),
        }
    }

    /// Opens the marked content of a graphic: part of the visual's figure when the visual has
    /// alt text, else an artifact (backgrounds, borders, the marks of a visual without alt text).
    fn open_graphic(&mut self, out: &mut String) {
        if let Some(top) = self.stack.last_mut() {
            top.para = None;
        }
        let fig = match self.stack.last() {
            Some(Frame { figure: Some(f), .. }) => Some(*f),
            Some(Frame {
                elem,
                alt: Some(a),
                figure: None,
                ..
            }) => {
                let (elem, alt) = (*elem, a.clone());
                let f = self.elem("Figure", elem, None, Some(alt));
                if let Some(top) = self.stack.last_mut() {
                    top.figure = f;
                }
                f
            }
            _ => None,
        };
        match fig.and_then(|f| self.mcid(f)) {
            Some(n) => {
                let _ = writeln!(out, "/Figure <</MCID {n}>> BDC");
            }
            None => out.push_str("/Artifact BMC\n"),
        }
    }

    /// Starts writing a content stream (a page, or a form XObject `stm` on it): its key.
    fn begin_stream(&mut self, page: usize, stm: Option<usize>) -> (usize, Option<Mark>) {
        let key = self.next_key;
        self.next_key += 1;
        let prev = self.mark.replace(Mark {
            key,
            page,
            stm,
            parents: Vec::new(),
        });
        (key, prev)
    }

    /// Ends the stream [`Doc::begin_stream`] started: its parent tree entry.
    fn end_stream(&mut self, prev: Option<Mark>) {
        if let Some(m) = std::mem::replace(&mut self.mark, prev)
            && !m.parents.is_empty()
        {
            self.parent_tree.push((m.key, m.parents));
        }
    }

    /// The face's use record (`None` when it cannot be embedded).
    fn face(&mut self, i: usize) -> bool {
        if let Some(f) = self.faces.get(&i) {
            return f.is_some();
        }
        let use_ = self.fonts.face_bytes(i).and_then(|(data, index)| {
            let info = ttf_subset::info(&data, index)?;
            (info.glyf && info.embeddable()).then_some(info)
        });
        let rec = use_.map(|info| {
            let id = self.reserve();
            FaceUse {
                id,
                name: format!("F{}", self.faces.len()),
                info,
                gids: BTreeSet::new(),
                uni: BTreeMap::new(),
                adv: BTreeMap::new(),
            }
        });
        let ok = rec.is_some();
        self.faces.insert(i, rec);
        ok
    }

    fn ops(&mut self, out: &mut String, ops: &[Op]) {
        for op in ops {
            self.op(out, op);
        }
    }

    fn fill(&mut self, out: &mut String, path: &tiny_skia::Path, tf: &Transform, c: Rgb, alpha: f32) {
        out.push_str("q ");
        if !tf.is_identity() {
            let _ = write!(out, "{} cm ", matrix(tf));
        }
        self.with_gs(out, alpha, 1.0);
        let _ = write!(out, "{} rg ", color(c));
        path_ops(out, path);
        out.push_str("f Q\n");
    }

    fn op(&mut self, out: &mut String, op: &Op) {
        match op {
            Op::Fill {
                path,
                tf,
                color: c,
                alpha,
            } => {
                self.open_graphic(out);
                self.fill(out, path, tf, *c, *alpha);
                out.push_str("EMC\n");
            }
            Op::Gradient {
                path,
                tf,
                top,
                bottom,
                y0,
                y1,
                alpha,
            } => {
                self.open_graphic(out);
                let n = self.shadings.len();
                let name = self
                    .shadings
                    .entry((*top, *bottom))
                    .or_insert_with(|| format!("Sh{n}"))
                    .clone();
                out.push_str("q ");
                if !tf.is_identity() {
                    let _ = write!(out, "{} cm ", matrix(tf));
                }
                self.with_gs(out, *alpha, 1.0);
                path_ops(out, path);
                let _ = writeln!(out, "W n 1 0 0 {} 0 {} cm /{name} sh Q", num(*y1 - *y0), num(*y0));
                out.push_str(
                    "EMC
",
                );
            }
            Op::Stroke {
                path,
                tf,
                color: c,
                alpha,
                width,
                dash,
                round,
            } => {
                self.open_graphic(out);
                out.push_str("q ");
                if !tf.is_identity() {
                    let _ = write!(out, "{} cm ", matrix(tf));
                }
                self.with_gs(out, 1.0, *alpha);
                let _ = write!(out, "{} RG {} w ", color(*c), num(*width));
                out.push_str(if *round { "1 J 1 j " } else { "0 J 0 j 4 M " });
                if let Some((a, b)) = dash {
                    let _ = write!(out, "[{} {}] 0 d ", num(*a), num(*b));
                }
                path_ops(out, path);
                out.push_str("S Q\nEMC\n");
            }
            Op::Text(t) => {
                if t.glyphs.is_empty() || t.alpha <= 0.0 {
                    return;
                }
                self.open_text(out, t);
                self.text(out, t);
                out.push_str("EMC\n");
            }
            Op::Image {
                img,
                x,
                y,
                w,
                h,
                opacity,
            } => {
                self.open_graphic(out);
                self.image(out, img, *x, *y, *w, *h, *opacity);
                out.push_str("EMC\n");
            }
            Op::Layer {
                x,
                y,
                w,
                h,
                opacity,
                body,
            } => {
                // another coordinate space: no paragraph goes on across it
                if let Some(top) = self.stack.last_mut() {
                    top.para = None;
                }
                let _ = writeln!(
                    out,
                    "q 1 0 0 1 {} {} cm 0 0 {} {} re W n",
                    num(*x),
                    num(*y),
                    num(*w),
                    num(*h)
                );
                match body {
                    LayerBody::Raster(pm) => {
                        self.open_graphic(out);
                        self.image(out, pm, 0.0, 0.0, *w, *h, *opacity);
                        out.push_str("EMC\n");
                    }
                    LayerBody::Vector(inner) if *opacity >= 0.999 => self.ops(out, inner),
                    LayerBody::Vector(inner) => {
                        // a transparency group, painted at the layer's opacity; its marked
                        // content belongs to the page's structure through a key of its own
                        let id = self.reserve();
                        let page = self.mark.as_ref().map_or(0, |m| m.page);
                        let (key, prev) = self.begin_stream(page, Some(id));
                        let mut sub = String::new();
                        self.ops(&mut sub, inner);
                        let marked = self.mark.as_ref().is_some_and(|m| !m.parents.is_empty());
                        self.end_stream(prev);
                        let mut dict = format!(
                            "/Type /XObject /Subtype /Form /BBox [0 0 {} {}] /Group << /S /Transparency >> /Resources {} 0 R",
                            num(*w),
                            num(*h),
                            self.resources
                        );
                        if marked {
                            let _ = write!(dict, " /StructParents {key}");
                        }
                        self.stream_at(id, &dict, sub.as_bytes());
                        let name = format!("X{}", self.xobjects.len());
                        self.xobjects.push((name.clone(), id));
                        self.with_gs(out, *opacity, *opacity);
                        let _ = writeln!(out, "/{name} Do");
                    }
                }
                out.push_str("Q\n");
            }
            Op::Begin { title, alt } => {
                let t = Some(title.clone()).filter(|t| !t.trim().is_empty());
                let e = self
                    .stack
                    .last()
                    .map(|f| f.elem)
                    .and_then(|p| self.elem("Sect", p, t, None));
                match (e, self.stack.last().map(|f| f.elem)) {
                    (Some(elem), _) => self.stack.push(Frame {
                        elem,
                        alt: alt.clone(),
                        figure: None,
                        para: None,
                    }),
                    // no room for one more element: the visual's content goes to the one around it
                    (None, Some(elem)) => self.stack.push(Frame {
                        elem,
                        alt: None,
                        figure: None,
                        para: None,
                    }),
                    (None, None) => {}
                }
            }
            Op::End => {
                // the page's own section stays open
                if self.stack.len() > 1 {
                    self.stack.pop();
                }
            }
        }
    }

    fn image(&mut self, out: &mut String, img: &Pixmap, x: f32, y: f32, w: f32, h: f32, opacity: f32) {
        let (iw, ih) = (img.width(), img.height());
        if iw == 0 || ih == 0 {
            return;
        }
        let px = img.pixels();
        let mut rgb = Vec::with_capacity(px.len().saturating_mul(3));
        let mut alpha = Vec::with_capacity(px.len());
        for p in px {
            let c = p.demultiply();
            rgb.extend_from_slice(&[c.red(), c.green(), c.blue()]);
            alpha.push(c.alpha());
        }
        let smask = if alpha.iter().any(|a| *a < 255) {
            let d = format!(
                "/Type /XObject /Subtype /Image /Width {iw} /Height {ih} /ColorSpace /DeviceGray /BitsPerComponent 8"
            );
            Some(self.stream(&d, &alpha))
        } else {
            None
        };
        let mut d = format!(
            "/Type /XObject /Subtype /Image /Width {iw} /Height {ih} /ColorSpace /DeviceRGB /BitsPerComponent 8"
        );
        if let Some(m) = smask {
            let _ = write!(d, " /SMask {m} 0 R");
        }
        let id = self.stream(&d, &rgb);
        let name = format!("X{}", self.xobjects.len());
        self.xobjects.push((name.clone(), id));
        out.push_str("q ");
        self.with_gs(out, opacity, opacity);
        let _ = writeln!(
            out,
            "{} 0 0 {} {} {} cm /{name} Do Q",
            num(w),
            num(-h),
            num(x),
            num(y + h)
        );
    }

    fn text(&mut self, out: &mut String, t: &TextOp) {
        if t.glyphs.is_empty() || t.alpha <= 0.0 {
            return;
        }
        let size = t.size.max(0.0);
        // segments of one face and one baseline offset
        let mut i = 0usize;
        let mut outlined = false;
        while let Some(first) = t.glyphs.get(i) {
            let mut j = i + 1;
            while let Some(g) = t.glyphs.get(j) {
                if g.face != first.face || (g.y - first.y).abs() > 1e-3 {
                    break;
                }
                j += 1;
            }
            let seg = t.glyphs.get(i..j).unwrap_or(&[]);
            match first.face {
                Some(f) if self.face(f) => self.text_segment(out, t, f, seg, size),
                face => {
                    outlined |= face.is_some();
                    self.outlines(out, t, face, seg, size);
                }
            }
            i = j;
        }
        if outlined {
            self.invisible(out, t, size);
        }
    }

    fn text_segment(
        &mut self,
        out: &mut String,
        t: &TextOp,
        face: usize,
        seg: &[crate::fonts::VecGlyph],
        size: f32,
    ) {
        let Some(first) = seg.first() else { return };
        // the characters of the run that these glyphs stand for
        let mut uni: Vec<(u16, char)> = Vec::new();
        for c in t.text.chars().take(4000) {
            let c = if matches!(c, '\u{a0}' | '\u{202f}' | '\t') {
                ' '
            } else {
                c
            };
            if let Some(g) = self.fonts.char_gid(face, c) {
                uni.push((g, c));
            }
        }
        let Some(Some(fu)) = self.faces.get_mut(&face) else {
            return;
        };
        let upem = f32::from(fu.info.upem.max(1));
        let (data, index) = match self.fonts.face_bytes(face) {
            Some(x) => x,
            None => return,
        };
        let new: BTreeSet<u16> = seg
            .iter()
            .map(|g| g.gid)
            .filter(|g| !fu.adv.contains_key(g))
            .collect();
        for (g, w) in ttf_subset::advances(&data, index, &new) {
            fu.adv.insert(g, w);
        }
        for g in seg {
            fu.gids.insert(g.gid);
        }
        for (g, c) in uni {
            if seg.iter().any(|x| x.gid == g) {
                fu.uni.entry(g).or_insert_with(|| c.to_string());
            }
        }
        let name = fu.name.clone();
        let mut tj = String::new();
        let mut prev: Option<(f32, u16)> = None;
        for g in seg {
            if let Some((px, pg)) = prev {
                let font_adv = f32::from(fu.adv.get(&pg).copied().unwrap_or(0)) / upem;
                let want = if size > 0.0 { (g.x - px) / size } else { 0.0 };
                let adj = -(want - font_adv) * 1000.0;
                if adj.abs() > 0.05 {
                    let _ = write!(tj, "> {} <", num(adj));
                }
            }
            let _ = write!(tj, "{:04X}", g.gid);
            prev = Some((g.x, g.gid));
        }
        let tm = glyph_tf(t, first.x, first.y, size);
        out.push_str("q ");
        self.with_gs(out, t.alpha, t.alpha);
        let _ = writeln!(
            out,
            "{} rg BT /{name} 1 Tf {} Tm [<{tj}>] TJ ET Q",
            color(t.color),
            matrix(&tm)
        );
    }

    /// Glyphs drawn as filled outlines (the missing-glyph box, faces that cannot be embedded).
    fn outlines(
        &mut self,
        out: &mut String,
        t: &TextOp,
        face: Option<usize>,
        seg: &[crate::fonts::VecGlyph],
        size: f32,
    ) {
        for g in seg {
            let Some((path, upem)) = self.fonts.glyph_outline(face, g.gid) else {
                continue;
            };
            let tf = glyph_tf(t, g.x, g.y, size / upem.max(1.0));
            // inside the run's own marked content
            self.fill(out, &path, &tf, t.color, t.alpha);
        }
    }

    /// Invisible Helvetica text over a run drawn as outlines, so it can still be searched.
    fn invisible(&mut self, out: &mut String, t: &TextOp, size: f32) {
        if self.helvetica.is_none() {
            let id = self.add(
                b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec(),
            );
            self.helvetica = Some(id);
        }
        let mut s = String::new();
        let mut n = 0usize;
        for c in t.text.chars().take(4000) {
            n += 1;
            match c {
                '(' | ')' | '\\' => {
                    s.push('\\');
                    s.push(c);
                }
                ' '..='~' => s.push(c),
                c if (0xa0..=0xff).contains(&u32::from(c)) => {
                    let _ = write!(s, "\\{:03o}", u32::from(c));
                }
                _ => s.push('?'),
            }
        }
        if n == 0 || size <= 0.0 {
            return;
        }
        // Helvetica's average advance is about half an em: stretch it to the run's width
        let tz = (t.width / (0.5 * size * n as f32) * 100.0).clamp(1.0, 1000.0);
        let tm = glyph_tf(t, 0.0, 0.0, size);
        let _ = writeln!(
            out,
            "q BT 3 Tr /FH 1 Tf {} Tz {} Tm ({s}) Tj ET Q",
            num(tz),
            matrix(&tm)
        );
    }

    /// Writes the embedded fonts (after every page: the glyph sets are known).
    fn finish_fonts(&mut self) -> Vec<PdfFont> {
        let mut notes = Vec::new();
        let faces = std::mem::take(&mut self.faces);
        for (i, f) in &faces {
            let Some(f) = f else {
                if let Some((data, index)) = self.fonts.face_bytes(*i)
                    && let Some(info) = ttf_subset::info(&data, index)
                {
                    notes.push(PdfFont {
                        name: info.ps_name,
                        embedded: false,
                        glyphs: 0,
                    });
                }
                continue;
            };
            let Some((data, index)) = self.fonts.face_bytes(*i) else {
                continue;
            };
            let sub = ttf_subset::subset(&data, index, &f.gids).unwrap_or_default();
            let base = format!("{}+{}", subset_tag(*i, &f.gids), f.info.ps_name);
            let file = {
                let z = deflate(&sub);
                let mut b = format!(
                    "<< /Length1 {} /Filter /FlateDecode /Length {} >>\nstream\n",
                    sub.len(),
                    z.len()
                )
                .into_bytes();
                b.extend_from_slice(&z);
                b.extend_from_slice(b"\nendstream");
                self.add(b)
            };
            let k = 1000.0 / f32::from(f.info.upem.max(1));
            let sc = |v: i16| num(f32::from(v) * k);
            let flags = if f.info.italic_angle != 0.0 { 4 | 64 } else { 4 };
            let desc = self.add(
                format!(
                    "<< /Type /FontDescriptor /FontName /{base} /Flags {flags} /FontBBox [{} {} {} {}] /ItalicAngle {} /Ascent {} /Descent {} /CapHeight {} /StemV 80 /FontFile2 {file} 0 R >>",
                    sc(f.info.bbox[0]),
                    sc(f.info.bbox[1]),
                    sc(f.info.bbox[2]),
                    sc(f.info.bbox[3]),
                    num(f.info.italic_angle),
                    sc(f.info.ascent),
                    sc(f.info.descent),
                    sc(f.info.cap_height)
                )
                .into_bytes(),
            );
            let mut w = String::new();
            for (g, a) in &f.adv {
                let _ = write!(w, "{g} [{}] ", num(f32::from(*a) * k));
            }
            let cid = self.add(
                format!(
                    "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /{base} /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor {desc} 0 R /DW 0 /W [{w}] /CIDToGIDMap /Identity >>"
                )
                .into_bytes(),
            );
            let cmap = self.stream("", to_unicode(&f.uni).as_bytes());
            self.set(
                f.id,
                format!(
                    "<< /Type /Font /Subtype /Type0 /BaseFont /{base} /Encoding /Identity-H /DescendantFonts [{cid} 0 R] /ToUnicode {cmap} 0 R >>"
                )
                .into_bytes(),
            );
            notes.push(PdfFont {
                name: base,
                embedded: true,
                glyphs: f.gids.len(),
            });
        }
        self.faces = faces;
        notes
    }
}

/// A ToUnicode CMap mapping glyph ids (2-byte codes) to their text.
fn to_unicode(uni: &BTreeMap<u16, String>) -> String {
    let mut s = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    let entries: Vec<(&u16, &String)> = uni.iter().collect();
    for chunk in entries.chunks(100) {
        let _ = writeln!(s, "{} beginbfchar", chunk.len());
        for (g, t) in chunk {
            let mut hex = String::new();
            for u in t.encode_utf16() {
                let _ = write!(hex, "{u:04X}");
            }
            let _ = writeln!(s, "<{g:04X}> <{hex}>");
        }
        s.push_str("endbfchar\n");
    }
    s.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    s
}

/// A document being written, a page at a time (a page's operations can be dropped once it is
/// written).
pub(crate) struct Writer<'a> {
    d: Doc<'a>,
    catalog: usize,
    tree: usize,
    info: usize,
    /// The structure tree's root.
    struct_root: usize,
    /// The pages written, with their titles (the outline's entries).
    kids: Vec<(usize, String)>,
}

impl<'a> Writer<'a> {
    pub(crate) fn new(fonts: &'a FontBook) -> Writer<'a> {
        let mut d = Doc {
            objs: Vec::new(),
            fonts,
            faces: BTreeMap::new(),
            gstates: BTreeMap::new(),
            shadings: BTreeMap::new(),
            xobjects: Vec::new(),
            resources: 0,
            helvetica: None,
            elems: Vec::new(),
            stack: Vec::new(),
            mark: None,
            parent_tree: Vec::new(),
            next_key: 0,
        };
        let catalog = d.reserve();
        let tree = d.reserve();
        let info = d.reserve();
        d.resources = d.reserve();
        let struct_root = d.reserve();
        let doc = d.reserve();
        d.elems.push(Elem {
            id: doc,
            role: "Document",
            parent: None,
            title: None,
            alt: None,
            page: None,
            kids: Vec::new(),
        });
        Writer {
            d,
            catalog,
            tree,
            info,
            struct_root,
            kids: Vec::new(),
        }
    }

    /// Writes one page of `w`×`h` report pixels titled `title` (its outline entry and section).
    pub(crate) fn page(&mut self, ops: &[Op], w: f32, h: f32, title: &str) {
        let (d, tree) = (&mut self.d, self.tree);
        let (w, h) = (w.max(1.0), h.max(1.0));
        let page = d.reserve();
        let (key, prev) = d.begin_stream(page, None);
        let title: String = title.chars().take(500).collect();
        d.stack.clear();
        if let Some(sect) = d.elem("Sect", 0, Some(title.clone()), None) {
            d.stack.push(Frame {
                elem: sect,
                alt: None,
                figure: None,
                para: None,
            });
        }
        let mut c = format!("{PT_PER_PX} 0 0 -{PT_PER_PX} 0 {} cm\n", num(h * PT_PER_PX));
        d.ops(&mut c, ops);
        d.stack.clear();
        let marked = d.mark.as_ref().is_some_and(|m| !m.parents.is_empty());
        d.end_stream(prev);
        let content = d.stream("", c.as_bytes());
        let structure = if marked {
            format!(" /StructParents {key} /Tabs /S")
        } else {
            String::new()
        };
        d.set(
            page,
            format!(
                "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 {} {}] /Resources {} 0 R /Contents {content} 0 R{structure} >>",
                num(w * PT_PER_PX),
                num(h * PT_PER_PX),
                d.resources
            )
            .into_bytes(),
        );
        self.kids.push((page, title));
    }

    /// The outline: an entry per page, titled as the page, that goes to it. Its object.
    fn outline(d: &mut Doc<'_>, kids: &[(usize, String)]) -> Option<usize> {
        if kids.is_empty() {
            return None;
        }
        let root = d.reserve();
        let ids: Vec<usize> = kids.iter().map(|_| d.reserve()).collect();
        for (i, ((page, title), id)) in kids.iter().zip(&ids).enumerate() {
            let mut b = format!(
                "<< /Title {} /Parent {root} 0 R /Dest [{page} 0 R /Fit]",
                text_string(if title.trim().is_empty() { "Page" } else { title })
            );
            if let Some(p) = i.checked_sub(1).and_then(|j| ids.get(j)) {
                let _ = write!(b, " /Prev {p} 0 R");
            }
            if let Some(n) = ids.get(i + 1) {
                let _ = write!(b, " /Next {n} 0 R");
            }
            b.push_str(" >>");
            d.set(*id, b.into_bytes());
        }
        let (first, last) = (ids.first().copied()?, ids.last().copied()?);
        d.set(
            root,
            format!(
                "<< /Type /Outlines /First {first} 0 R /Last {last} 0 R /Count {} >>",
                ids.len()
            )
            .into_bytes(),
        );
        Some(root)
    }

    /// The structure tree: the elements that hold content (others are left out), the parent
    /// tree, and the root.
    fn structure(d: &mut Doc<'_>, root: usize) {
        // an element is kept when it or an element under it holds content (children always
        // come after their parent)
        let n = d.elems.len();
        let mut keep = vec![false; n];
        for i in (0..n).rev() {
            let k = d.elems.get(i).is_some_and(|e| {
                e.kids.iter().any(|k| match k {
                    Kid::Content { .. } => true,
                    Kid::Elem(c) => keep.get(*c).copied().unwrap_or(false),
                })
            });
            if let Some(slot) = keep.get_mut(i) {
                *slot = k || i == 0;
            }
        }
        let mut bodies: Vec<(usize, Vec<u8>)> = Vec::new();
        for e in &d.elems {
            let parent = e.parent.and_then(|p| d.elems.get(p)).map_or(root, |p| p.id);
            let mut b = format!("<< /Type /StructElem /S /{} /P {parent} 0 R", e.role);
            if let Some(pg) = e.page {
                let _ = write!(b, " /Pg {pg} 0 R");
            }
            if let Some(t) = &e.title {
                let _ = write!(b, " /T {}", text_string(t));
            }
            if let Some(a) = &e.alt {
                let _ = write!(b, " /Alt {}", text_string(a));
            }
            b.push_str(" /K [");
            for k in &e.kids {
                match k {
                    Kid::Elem(c) => {
                        if keep.get(*c).copied().unwrap_or(false)
                            && let Some(ce) = d.elems.get(*c)
                        {
                            let _ = write!(b, "{} 0 R ", ce.id);
                        }
                    }
                    Kid::Content { page, stm, mcid } => {
                        let _ = write!(b, "<< /Type /MCR /Pg {page} 0 R /MCID {mcid}");
                        if let Some(s) = stm {
                            let _ = write!(b, " /Stm {s} 0 R");
                        }
                        b.push_str(" >> ");
                    }
                }
            }
            b.push_str("] >>");
            bodies.push((e.id, b.into_bytes()));
        }
        for (i, (id, b)) in bodies.into_iter().enumerate() {
            // an element left out is written as null (its object number is reserved)
            if keep.get(i).copied().unwrap_or(false) {
                d.set(id, b);
            }
        }
        let mut entries = std::mem::take(&mut d.parent_tree);
        entries.sort_by_key(|e| e.0);
        let mut nums = String::new();
        for (key, parents) in &entries {
            let refs: Vec<String> = parents
                .iter()
                .filter_map(|p| d.elems.get(*p))
                .map(|e| format!("{} 0 R", e.id))
                .collect();
            let arr = d.add(format!("[{}]", refs.join(" ")).into_bytes());
            let _ = write!(nums, "{key} {arr} 0 R ");
        }
        let tree = d.add(format!("<< /Nums [{nums}] >>").into_bytes());
        let doc = d.elems.first().map_or(0, |e| e.id);
        d.set(
            root,
            format!(
                "<< /Type /StructTreeRoot /K [{doc} 0 R] /ParentTree {tree} 0 R /ParentTreeNextKey {} >>",
                d.next_key
            )
            .into_bytes(),
        );
    }

    /// The file, and the fonts it embeds. `lang` is the document's language (`en-US`).
    pub(crate) fn finish(self, title: &str, lang: &str) -> (Vec<u8>, Vec<PdfFont>) {
        let Writer {
            mut d,
            catalog,
            tree,
            info,
            struct_root,
            kids,
        } = self;
        let notes = d.finish_fonts();
        Self::structure(&mut d, struct_root);
        let outline = Self::outline(&mut d, &kids);
        let kids: Vec<usize> = kids.into_iter().map(|k| k.0).collect();
        // the shared resources
        let mut r = String::from("<< /ProcSet [/PDF /Text /ImageB /ImageC] /Font << ");
        for f in d.faces.values().flatten() {
            let _ = write!(r, "/{} {} 0 R ", f.name, f.id);
        }
        if let Some(h) = d.helvetica {
            let _ = write!(r, "/FH {h} 0 R ");
        }
        r.push_str(">> /ExtGState << ");
        for ((f, s), name) in &d.gstates {
            let _ = write!(
                r,
                "/{name} << /ca {} /CA {} >> ",
                num(f32::from(*f) / 1000.0),
                num(f32::from(*s) / 1000.0)
            );
        }
        r.push_str(">> /Shading << ");
        for ((top, bottom), name) in &d.shadings {
            let _ = write!(
                r,
                "/{name} << /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 0 0 1] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [{}] /C1 [{}] /N 1 >> >> ",
                color(*top),
                color(*bottom)
            );
        }
        r.push_str(">> /XObject << ");
        for (name, id) in &d.xobjects {
            let _ = write!(r, "/{name} {id} 0 R ");
        }
        r.push_str(">> >>");
        let res = d.resources;
        d.set(res, r.into_bytes());
        let kids_s: Vec<String> = kids.iter().map(|k| format!("{k} 0 R")).collect();
        d.set(
            tree,
            format!(
                "<< /Type /Pages /Kids [{}] /Count {} >>",
                kids_s.join(" "),
                kids.len()
            )
            .into_bytes(),
        );
        // tagged (ISO 32000-1 §14.8): marked, a structure tree, the language, the title shown;
        // the outline open beside the pages when there are several
        let mut cat = format!(
            "<< /Type /Catalog /Pages {tree} 0 R /MarkInfo << /Marked true >> /StructTreeRoot {struct_root} 0 R /Lang {} /ViewerPreferences << /DisplayDocTitle true >>",
            text_string(lang.trim())
        );
        if let Some(o) = outline {
            let _ = write!(cat, " /Outlines {o} 0 R");
            if kids.len() > 1 {
                cat.push_str(" /PageMode /UseOutlines");
            }
        }
        cat.push_str(" >>");
        d.set(catalog, cat.into_bytes());
        d.set(
            info,
            format!(
                "<< /Title {} /Producer (BIcraft {}) /Creator (BIcraft) >>",
                text_string(title),
                env!("CARGO_PKG_VERSION")
            )
            .into_bytes(),
        );
        // the file
        let mut out: Vec<u8> = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
        let mut offsets = Vec::with_capacity(d.objs.len());
        for (i, o) in d.objs.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            out.extend_from_slice(o.as_deref().unwrap_or(b"null"));
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", d.objs.len() + 1).as_bytes());
        for o in offsets {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root {catalog} 0 R /Info {info} 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                d.objs.len() + 1
            )
            .as_bytes(),
        );
        (out, notes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_pdf_numbers() {
        assert_eq!(num(1.0), "1");
        assert_eq!(num(0.5), "0.5");
        assert_eq!(num(-0.00001), "0");
        assert_eq!(num(f32::NAN), "0");
        assert_eq!(num(1e30), "10000000");
        assert_eq!(text_string("Aé"), "<FEFF004100E9>");
        let t = subset_tag(3, &[1u16, 2].into_iter().collect());
        assert!(t.len() == 6 && t.chars().all(|c| c.is_ascii_uppercase()));
    }
}
