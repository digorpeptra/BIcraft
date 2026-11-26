//! The UI's icons, drawn in code.
//!
//! Ported from DeckCraft `crates/ui-egui/src/icons.rs` @ 0ec0cc5 (MIT OR Apache-2.0, © 2026
//! ArtCraft Team and the DeckCraft contributors): the drawing toolkit (a pen on a 24 × 24 design
//! grid, line icons in the theme's ink with fixed accent colours, the stroke width rule, the
//! disabled look) and the drawings of the icons DeckCraft and BIcraft share. Rewritten for
//! BIcraft's never-crash rules (no direct indexing, bounded loops), with DeckCraft's orange
//! replaced by BIcraft's amber. The icons marked "BIcraft" are original drawings made the same
//! way. Nothing is traced from or modelled on any vendor's artwork (AGENTS.md §2).

use std::f32::consts::PI;

use egui::epaint::{Mesh, PathShape};
use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, pos2, vec2};

use crate::theme;

/// Accent blue.
pub const BLUE: Color32 = Color32::from_rgb(0x2e, 0x6f, 0xd8);
/// Accent amber (DeckCraft: orange).
pub const AMBER: Color32 = theme::AMBER;
/// Accent green.
pub const GREEN: Color32 = Color32::from_rgb(0x2e, 0xa3, 0x6f);
/// Accent red.
pub const RED: Color32 = Color32::from_rgb(0xd8, 0x3f, 0x6b);
/// Accent gold.
pub const GOLD: Color32 = Color32::from_rgb(0xe3, 0xb2, 0x1c);
/// Accent teal (the logo's).
pub const TEAL: Color32 = theme::TEAL;
/// Fixed dark ink for marks on gold fills (readable in both themes).
const DARK: Color32 = Color32::from_rgb(0x2b, 0x2b, 0x2b);

macro_rules! icons {
    ($($(#[$m:meta])* $v:ident => $n:literal,)*) => {
        /// Every icon the UI draws.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Icon {
            $($(#[$m])* $v,)*
        }

        impl Icon {
            /// All icons, in declaration order.
            pub const ALL: &'static [Icon] = &[$(Icon::$v,)*];

            /// Stable kebab-case name, e.g. `"zoom-in"`.
            pub fn name(self) -> &'static str {
                match self {
                    $(Icon::$v => $n,)*
                }
            }

            /// Inverse of [`Icon::name`].
            pub fn from_name(s: &str) -> Option<Icon> {
                match s {
                    $($n => Some(Icon::$v),)*
                    _ => None,
                }
            }
        }
    };
}

icons! {
    // from DeckCraft
    Home => "home",
    Search => "search",
    File => "file",
    Folder => "folder",
    Export => "export",
    Settings => "settings",
    Help => "help",
    Info => "info",
    Warning => "warning",
    Lock => "lock",
    Eye => "eye",
    EyeOff => "eye-off",
    Close => "close",
    Check => "check",
    Plus => "plus",
    Minus => "minus",
    ChevronDown => "chevron-down",
    ChevronUp => "chevron-up",
    ChevronLeft => "chevron-left",
    ChevronRight => "chevron-right",
    More => "more",
    Table => "table",
    Chart => "chart",
    Grid => "grid",
    List => "list",
    FitSmall => "fit-small",
    ZoomIn => "zoom-in",
    ZoomOut => "zoom-out",
    // BIcraft
    Reload => "reload",
    Page => "page",
    Model => "model",
    Validate => "validate",
    Measure => "measure",
    Column => "column",
    Relationship => "relationship",
    Server => "server",
    Sun => "sun",
    Moon => "moon",
    Contrast => "contrast",
    OpenInDesktop => "open-in-desktop",
    FitWidth => "fit-width",
    Language => "language",
    Save => "save",
    Back => "back",
    Pin => "pin",
    Copy => "copy",
    Cut => "cut",
    // BIcraft, for the command registry's buttons (ROADMAP M11)
    Calendar => "calendar",
    Compare => "compare",
    Delete => "delete",
    Edit => "edit",
    Filter => "filter",
    Format => "format",
    Hierarchy => "hierarchy",
    Import => "import",
    Layers => "layers",
    Link => "link",
    Mobile => "mobile",
    Palette => "palette",
    Play => "play",
    Redo => "redo",
    Rename => "rename",
    SaveAs => "save-as",
    Sort => "sort",
    Undo => "undo",
    Calculator => "calculator",
    // BIcraft, for the Edit interactions mode (ROADMAP M11.15)
    Highlight => "highlight",
    NoInteraction => "no-interaction",
    // BIcraft, for canvas editing (ROADMAP M11.16)
    Paste => "paste",
    Duplicate => "duplicate",
    Align => "align",
    Group => "group",
    Ungroup => "ungroup",
    Move => "move",
    // BIcraft, Insert tab (M11.20)
    TextBox => "text-box",
    Image => "image",
    Button => "button",
    Shapes => "shapes",
    Logo => "logo",
    // BIcraft, the visual header's focus mode (M11.19)
    Focus => "focus",
    // BIcraft, the Visualizations pane's Analytics tab (G56)
    Analytics => "analytics",
    // BIcraft, File › Export › Print (M11.21)
    Print => "print",
}

/// Stroke width (points) for an icon drawn at `size` points: ≈ size/16 at 16 pt, size/20 at 32 pt
/// and above, never thinner than 1.0.
fn stroke_width(size: f32) -> f32 {
    let t = ((size - 16.0) / 16.0).clamp(0.0, 1.0);
    (size / (16.0 + 4.0 * t)).max(1.0)
}

/// Paints `icon` centred in the largest square of `rect`. `ink` is the theme's text colour;
/// accents are fixed colours, except when `disabled`: then everything is ink at 35 %.
pub fn paint(painter: &Painter, rect: Rect, icon: Icon, ink: Color32, disabled: bool) {
    let size = rect.width().min(rect.height());
    if !size.is_finite() || size <= 0.5 || size > 4096.0 {
        return;
    }
    if icon == Icon::Logo {
        theme::paint_logo(painter, Rect::from_center_size(rect.center(), vec2(size, size)));
        return;
    }
    let origin = rect.center() - vec2(size * 0.5, size * 0.5);
    let pen = Pen {
        p: painter,
        o: origin,
        k: size / 24.0,
        sw: stroke_width(size),
        ink,
        dis: disabled,
    };
    draw(&pen, icon);
}

/// A logical colour, resolved against the ink and the disabled flag.
#[derive(Clone, Copy)]
enum C {
    Ink(f32),
    Acc(Color32, f32),
    White,
    None,
}

impl C {
    fn a(self, f: f32) -> C {
        match self {
            C::Ink(a) => C::Ink(a * f),
            C::Acc(c, a) => C::Acc(c, a * f),
            other => other,
        }
    }
}

const K: C = C::Ink(1.0);
const B: C = C::Acc(BLUE, 1.0);
const O: C = C::Acc(AMBER, 1.0);
const G: C = C::Acc(GREEN, 1.0);
const Y: C = C::Acc(GOLD, 1.0);
const T: C = C::Acc(TEAL, 1.0);
const D: C = C::Acc(DARK, 1.0);
const N: C = C::None;

/// Light tint of an accent, for fills behind outlines.
fn tint(c: C) -> C {
    c.a(0.25)
}

type P2 = (f32, f32);

fn rad(deg: f32) -> f32 {
    deg * PI / 180.0
}

fn arc_pts(c: P2, rx: f32, ry: f32, a0: f32, a1: f32) -> Vec<P2> {
    let n = (((a1 - a0).abs() / 9.0).ceil() as usize).clamp(4, 80);
    (0..=n)
        .map(|i| {
            let a = rad(a0 + (a1 - a0) * i as f32 / n as f32);
            (c.0 + rx * a.cos(), c.1 + ry * a.sin())
        })
        .collect()
}

fn quad_pts(p0: P2, p1: P2, p2: P2, n: usize) -> Vec<P2> {
    let n = n.clamp(1, 64);
    (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            (
                u * u * p0.0 + 2.0 * u * t * p1.0 + t * t * p2.0,
                u * u * p0.1 + 2.0 * u * t * p1.1 + t * t * p2.1,
            )
        })
        .collect()
}

/// Rounds the corners of a polyline by `r` design units.
fn round_pts(pts: &[P2], r: f32, closed: bool) -> Vec<P2> {
    let n = pts.len();
    if n < 3 || r <= 0.0 || n > 256 {
        return pts.to_vec();
    }
    let mut out = Vec::with_capacity(n * 6);
    for (i, &cur) in pts.iter().enumerate() {
        let interior = closed || (i > 0 && i + 1 < n);
        let (Some(&prev), Some(&next)) = (pts.get((i + n - 1) % n), pts.get((i + 1) % n)) else {
            continue;
        };
        if !interior {
            out.push(cur);
            continue;
        }
        let d1 = ((prev.0 - cur.0).powi(2) + (prev.1 - cur.1).powi(2)).sqrt();
        let d2 = ((next.0 - cur.0).powi(2) + (next.1 - cur.1).powi(2)).sqrt();
        if d1 < 1e-4 || d2 < 1e-4 {
            out.push(cur);
            continue;
        }
        let rr = r.min(d1 * 0.5).min(d2 * 0.5);
        let a = (
            cur.0 + (prev.0 - cur.0) / d1 * rr,
            cur.1 + (prev.1 - cur.1) / d1 * rr,
        );
        let b = (
            cur.0 + (next.0 - cur.0) / d2 * rr,
            cur.1 + (next.1 - cur.1) / d2 * rr,
        );
        out.extend(quad_pts(a, cur, b, 4));
    }
    out
}

fn point_in_tri(p: Pos2, a: Pos2, b: Pos2, c: Pos2) -> bool {
    let d1 = (p.x - b.x) * (a.y - b.y) - (a.x - b.x) * (p.y - b.y);
    let d2 = (p.x - c.x) * (b.y - c.y) - (b.x - c.x) * (p.y - c.y);
    let d3 = (p.x - a.x) * (c.y - a.y) - (c.x - a.x) * (p.y - a.y);
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

/// Ear-clipping triangulation of a simple polygon (at most 512 points; degenerate input drops
/// vertices rather than looping).
fn triangulate(pts: &[Pos2]) -> Vec<[u32; 3]> {
    let n = pts.len();
    let mut tris = Vec::new();
    if !(3..=512).contains(&n) || pts.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
        return tris;
    }
    let area: f32 = pts
        .iter()
        .zip(pts.iter().cycle().skip(1))
        .map(|(a, b)| a.x * b.y - b.x * a.y)
        .sum();
    let mut idx: Vec<usize> = (0..n).collect();
    if area < 0.0 {
        idx.reverse();
    }
    let p = |i: usize| pts.get(i).copied().unwrap_or(Pos2::ZERO);
    while idx.len() > 3 {
        let m = idx.len();
        let mut clipped = false;
        for i in 0..m {
            let at = |k: usize| idx.get(k % m).copied().unwrap_or(0);
            let (ia, ib, ic) = (at(i + m - 1), at(i), at(i + 1));
            let (a, b, c) = (p(ia), p(ib), p(ic));
            let cross = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
            if cross <= 1e-7 {
                continue;
            }
            if idx
                .iter()
                .any(|&j| j != ia && j != ib && j != ic && point_in_tri(p(j), a, b, c))
            {
                continue;
            }
            tris.push([ia as u32, ib as u32, ic as u32]);
            idx.remove(i);
            clipped = true;
            break;
        }
        if !clipped {
            idx.remove(0);
        }
    }
    if let [a, b, c] = idx[..] {
        tris.push([a as u32, b as u32, c as u32]);
    }
    tris
}

struct Pen<'a> {
    p: &'a Painter,
    o: Pos2,
    /// Points per design unit.
    k: f32,
    /// Base stroke width in points.
    sw: f32,
    ink: Color32,
    dis: bool,
}

impl Pen<'_> {
    fn pt(&self, (x, y): P2) -> Pos2 {
        pos2(self.o.x + x * self.k, self.o.y + y * self.k)
    }

    fn v(&self, pts: &[P2]) -> Vec<Pos2> {
        pts.iter().map(|&p| self.pt(p)).collect()
    }

    fn col(&self, c: C) -> Color32 {
        let dim = if self.dis { 0.35 } else { 1.0 };
        match c {
            C::Ink(a) => self.ink.gamma_multiply(a * dim),
            C::Acc(col, a) => {
                if self.dis {
                    self.ink.gamma_multiply(a * dim)
                } else {
                    col.gamma_multiply(a)
                }
            }
            C::White => {
                if self.dis {
                    self.ink.gamma_multiply(0.6)
                } else {
                    Color32::WHITE
                }
            }
            C::None => Color32::TRANSPARENT,
        }
    }

    fn stroke(&self, c: C, w: f32) -> Stroke {
        Stroke::new(self.sw * w, self.col(c))
    }

    fn line_w(&self, pts: &[P2], c: C, w: f32) {
        if pts.len() < 2 {
            return;
        }
        let col = self.col(c);
        if col == Color32::TRANSPARENT {
            return;
        }
        let v = self.v(pts);
        if let [a, b] = v[..] {
            // single segments get round caps: one convex capsule
            let d = b - a;
            let len = d.length();
            let r = self.sw * w * 0.5;
            if len < 1e-3 {
                self.p.circle_filled(a, r, col);
                return;
            }
            let ang = d.y.atan2(d.x);
            let mut cap = Vec::with_capacity(20);
            for (c, start) in [(b, ang - PI * 0.5), (a, ang + PI * 0.5)] {
                for i in 0..=8 {
                    let t = start + PI * i as f32 / 8.0;
                    cap.push(c + vec2(t.cos(), t.sin()) * r);
                }
            }
            self.p.add(PathShape::convex_polygon(cap, col, Stroke::NONE));
            return;
        }
        self.p.add(PathShape::line(v, Stroke::new(self.sw * w, col)));
    }

    fn line(&self, pts: &[P2], c: C) {
        self.line_w(pts, c, 1.0);
    }

    fn seg(&self, a: P2, b: P2, c: C) {
        self.line_w(&[a, b], c, 1.0);
    }

    fn seg_w(&self, a: P2, b: P2, c: C, w: f32) {
        self.line_w(&[a, b], c, w);
    }

    fn rline(&self, pts: &[P2], r: f32, c: C) {
        self.line(&round_pts(pts, r, false), c);
    }

    /// Any simple polygon: filled by triangulation, stroked as a closed path.
    fn shape(&self, pts: &[P2], fill: C, stroke: C) {
        if pts.len() < 3 {
            return;
        }
        let v = self.v(pts);
        let fc = self.col(fill);
        if fc != Color32::TRANSPARENT {
            let mut mesh = Mesh::default();
            for &p in &v {
                mesh.colored_vertex(p, fc);
            }
            for [a, b, c] in triangulate(&v) {
                mesh.add_triangle(a, b, c);
            }
            self.p.add(Shape::mesh(mesh));
            if matches!(stroke, C::None) {
                self.p.add(PathShape::closed_line(
                    v.clone(),
                    Stroke::new(0.6_f32.min(self.sw * 0.5), fc),
                ));
            }
        }
        let sc = self.col(stroke);
        if sc != Color32::TRANSPARENT {
            self.p.add(PathShape::closed_line(v, Stroke::new(self.sw, sc)));
        }
    }

    fn rshape(&self, pts: &[P2], r: f32, fill: C, stroke: C) {
        self.shape(&round_pts(pts, r, true), fill, stroke);
    }

    /// Convex polygon.
    fn poly(&self, pts: &[P2], fill: C, stroke: C) {
        let v = self.v(pts);
        self.p.add(PathShape::convex_polygon(
            v,
            self.col(fill),
            self.stroke(stroke, 1.0),
        ));
    }

    #[allow(clippy::too_many_arguments)]
    fn rect(&self, x0: f32, y0: f32, x1: f32, y1: f32, r: f32, fill: C, stroke: C) {
        let rect = Rect::from_min_max(self.pt((x0, y0)), self.pt((x1, y1)));
        self.p.rect(
            rect,
            r * self.k,
            self.col(fill),
            self.stroke(stroke, 1.0),
            StrokeKind::Middle,
        );
    }

    fn circle(&self, c: P2, r: f32, fill: C, stroke: C) {
        self.p
            .circle(self.pt(c), r * self.k, self.col(fill), self.stroke(stroke, 1.0));
    }

    fn dot(&self, c: P2, r: f32, fill: C) {
        self.p.circle_filled(self.pt(c), r * self.k, self.col(fill));
    }

    /// Filled arrow head with its tip at `tip`, pointing along `dir` degrees.
    fn head(&self, tip: P2, dir: f32, size: f32, c: C) {
        let (s, co) = rad(dir).sin_cos();
        let hw = size * 0.62;
        let back = (tip.0 - co * size, tip.1 - s * size);
        self.poly(
            &[
                tip,
                (back.0 - s * hw, back.1 + co * hw),
                (back.0 + s * hw, back.1 - co * hw),
            ],
            c,
            N,
        );
    }

    fn head_size(&self) -> f32 {
        (2.2 * self.sw / self.k).max(3.6)
    }

    fn arrow(&self, a: P2, b: P2, c: C) {
        let dir = (b.1 - a.1).atan2(b.0 - a.0);
        let hs = self.head_size();
        let end = (b.0 - dir.cos() * hs * 0.7, b.1 - dir.sin() * hs * 0.7);
        self.seg(a, end, c);
        self.head(b, dir.to_degrees(), hs, c);
    }

    fn magnifier(&self, c: P2, r: f32, col: C) {
        self.circle(c, r, N, col);
        let d = r * 0.707;
        self.seg_w(
            (c.0 + d + 0.6, c.1 + d + 0.6),
            (c.0 + r * 1.55, c.1 + r * 1.55),
            col,
            1.5,
        );
    }

    /// Document page with a folded top-right corner.
    fn page(&self, x0: f32, y0: f32, x1: f32, y1: f32, fill: C, stroke: C) {
        let f = ((x1 - x0) * 0.32).min(5.0);
        self.rshape(
            &[(x0, y0), (x1 - f, y0), (x1, y0 + f), (x1, y1), (x0, y1)],
            1.2,
            fill,
            stroke,
        );
        self.line(&[(x1 - f, y0), (x1 - f, y0 + f), (x1, y0 + f)], stroke);
    }

    fn gear(&self, c: P2, ro: f32, ri: f32, teeth: usize, stroke: C) {
        let teeth = teeth.clamp(3, 24);
        let step = 360.0 / teeth as f32;
        let mut pts = Vec::with_capacity(teeth * 4);
        for i in 0..teeth {
            let base = i as f32 * step;
            for (a, r) in [(-0.3, ri), (-0.17, ro), (0.17, ro), (0.3, ri)] {
                let ang = rad(base + a * step);
                pts.push((c.0 + r * ang.cos(), c.1 + r * ang.sin()));
            }
        }
        self.shape(&pts, N, stroke);
    }

    /// Table grid; `header` fills the first row.
    #[allow(clippy::too_many_arguments)]
    fn table(&self, x0: f32, y0: f32, x1: f32, y1: f32, rows: usize, cols: usize, header: C) {
        let (rows, cols) = (rows.clamp(1, 12), cols.clamp(1, 12));
        let rh = (y1 - y0) / rows as f32;
        let cw = (x1 - x0) / cols as f32;
        if !matches!(header, C::None) {
            self.rect(x0, y0, x1, y0 + rh, 1.2, header, N);
        }
        for i in 1..rows {
            let y = y0 + rh * i as f32;
            self.seg_w((x0, y), (x1, y), K, 0.8);
        }
        for j in 1..cols {
            let x = x0 + cw * j as f32;
            self.seg_w((x, y0), (x, y1), K, 0.8);
        }
        self.rect(x0, y0, x1, y1, 1.2, N, K);
    }

    /// Bar chart in the box (the logo's colours).
    fn bars(&self, x0: f32, y0: f32, x1: f32, y1: f32) {
        let w = (x1 - x0) / 3.0;
        let h = y1 - y0;
        for (i, (frac, c)) in [(0.5, T), (0.75, O), (1.0, C::Acc(theme::DARK_AMBER, 1.0))]
            .into_iter()
            .enumerate()
        {
            let bx = x0 + w * i as f32;
            self.rect(bx + w * 0.16, y1 - h * frac, bx + w * 0.84, y1, 0.6, c, N);
        }
        self.seg((x0 - 0.5, y1 + 0.2), (x1 + 0.5, y1 + 0.2), K);
    }

    /// A small table card of the model diagram (BIcraft).
    fn card(&self, x0: f32, y0: f32, x1: f32, y1: f32, header: C) {
        self.rect(x0, y0, x1, y0 + 3.0, 1.0, header, N);
        self.rect(x0, y0, x1, y1, 1.0, N, K);
    }
}

fn draw(p: &Pen<'_>, icon: Icon) {
    use Icon as I;
    match icon {
        // ----- from DeckCraft -----
        I::Home => {
            p.rect(10.0, 14.5, 14.0, 21.0, 0.8, O, N);
            p.line(&[(2.5, 11.5), (12.0, 3.0), (21.5, 11.5)], K);
            p.rline(&[(5.0, 9.5), (5.0, 21.0), (19.0, 21.0), (19.0, 9.5)], 1.2, K);
        }
        I::Search => p.magnifier((10.0, 10.0), 6.5, K),
        I::File => {
            p.page(4.5, 2.0, 19.5, 22.0, N, K);
            p.seg((8.0, 12.5), (16.0, 12.5), B);
            p.seg((8.0, 16.5), (14.0, 16.5), B);
        }
        I::Folder => {
            p.rshape(
                &[
                    (2.5, 4.5),
                    (9.0, 4.5),
                    (11.0, 7.0),
                    (21.5, 7.0),
                    (21.5, 19.5),
                    (2.5, 19.5),
                ],
                1.2,
                tint(Y),
                K,
            );
            p.rshape(
                &[(2.5, 10.0), (21.5, 10.0), (21.5, 19.5), (2.5, 19.5)],
                1.2,
                Y.a(0.6),
                K,
            );
        }
        I::Export => {
            p.rline(
                &[
                    (15.0, 8.5),
                    (15.0, 2.5),
                    (3.0, 2.5),
                    (3.0, 21.5),
                    (15.0, 21.5),
                    (15.0, 15.5),
                ],
                1.5,
                K,
            );
            p.arrow((8.5, 12.0), (22.0, 12.0), O);
        }
        I::Settings => {
            p.gear((12.0, 12.0), 9.8, 7.3, 8, K);
            p.circle((12.0, 12.0), 3.0, N, K);
        }
        I::Help => {
            p.circle((12.0, 12.0), 9.8, N, K);
            let mut q = arc_pts((12.0, 9.4), 3.2, 3.0, 180.0, 400.0);
            q.push((12.0, 13.0));
            q.push((12.0, 14.6));
            p.line(&q, B);
            p.dot((12.0, 17.8), 1.25, B);
        }
        I::Info => {
            p.circle((12.0, 12.0), 9.8, N, K);
            p.dot((12.0, 7.3), 1.3, B);
            p.seg((12.0, 10.8), (12.0, 17.5), B);
        }
        I::Warning => {
            p.rshape(&[(12.0, 2.5), (22.5, 20.5), (1.5, 20.5)], 2.0, Y, N);
            p.seg((12.0, 9.0), (12.0, 14.0), D);
            p.dot((12.0, 17.2), 1.2, D);
        }
        I::Lock => {
            let mut sh = vec![(7.5, 10.5), (7.5, 8.0)];
            sh.extend(arc_pts((12.0, 8.0), 4.5, 4.5, 180.0, 360.0));
            sh.push((16.5, 10.5));
            p.line(&sh, K);
            p.rect(4.5, 10.5, 19.5, 21.5, 1.8, Y, K);
            p.dot((12.0, 15.0), 1.5, D);
            p.seg((12.0, 15.5), (12.0, 18.3), D);
        }
        I::Eye | I::EyeOff => {
            let mut e = quad_pts((2.0, 12.0), (12.0, 2.5), (22.0, 12.0), 16);
            e.extend(
                quad_pts((22.0, 12.0), (12.0, 21.5), (2.0, 12.0), 16)
                    .into_iter()
                    .skip(1),
            );
            e.pop();
            p.shape(&e, N, K);
            p.circle((12.0, 12.0), 3.4, B, N);
            p.dot((12.0, 12.0), 1.3, if p.dis { N } else { C::White });
            if icon == I::EyeOff {
                p.seg_w((4.0, 21.0), (20.0, 3.0), K, 1.1);
            }
        }
        I::Close => {
            p.seg_w((5.5, 5.5), (18.5, 18.5), K, 1.1);
            p.seg_w((18.5, 5.5), (5.5, 18.5), K, 1.1);
        }
        I::Check => p.line_w(&[(4.0, 12.5), (9.5, 18.0), (20.0, 6.5)], G, 1.3),
        I::Plus => {
            p.seg((12.0, 4.5), (12.0, 19.5), K);
            p.seg((4.5, 12.0), (19.5, 12.0), K);
        }
        I::Minus => p.seg((4.5, 12.0), (19.5, 12.0), K),
        I::ChevronDown => p.line(&[(6.0, 9.0), (12.0, 15.0), (18.0, 9.0)], K),
        I::ChevronUp => p.line(&[(6.0, 15.0), (12.0, 9.0), (18.0, 15.0)], K),
        I::ChevronLeft => p.line(&[(15.0, 6.0), (9.0, 12.0), (15.0, 18.0)], K),
        I::ChevronRight => p.line(&[(9.0, 6.0), (15.0, 12.0), (9.0, 18.0)], K),
        I::More => {
            for x in [5.0, 12.0, 19.0] {
                p.dot((x, 12.0), 1.9, K);
            }
        }
        I::Table => p.table(2.5, 4.0, 21.5, 20.0, 3, 3, B),
        I::Chart => p.bars(2.5, 4.0, 21.5, 20.5),
        I::Grid => {
            for (x, y) in [(3.0, 3.0), (13.0, 3.0), (3.0, 13.0), (13.0, 13.0)] {
                p.rect(x, y, x + 8.0, y + 8.0, 1.5, N, K);
            }
        }
        I::List => {
            for y in [5.5, 12.0, 18.5] {
                p.dot((4.5, y), 1.5, K);
                p.seg((9.0, y), (21.0, y), K);
            }
        }
        I::FitSmall => {
            for (cx, cy, sx, sy) in [
                (3.0, 3.0, 1.0, 1.0),
                (21.0, 3.0, -1.0, 1.0),
                (3.0, 21.0, 1.0, -1.0),
                (21.0, 21.0, -1.0, -1.0),
            ] {
                p.rline(&[(cx, cy + 5.0 * sy), (cx, cy), (cx + 5.0 * sx, cy)], 0.8, K);
            }
            p.rect(8.0, 8.5, 16.0, 15.5, 0.8, N, K);
        }
        I::ZoomIn | I::ZoomOut => {
            p.magnifier((10.0, 10.0), 7.0, K);
            p.seg((7.0, 10.0), (13.0, 10.0), K);
            if icon == I::ZoomIn {
                p.seg((10.0, 7.0), (10.0, 13.0), K);
            }
        }
        // ----- BIcraft -----
        I::Reload => {
            let pts = arc_pts((12.0, 12.0), 8.0, 8.0, -60.0, 250.0);
            p.line(&pts, K);
            p.head((16.0, 5.1), -30.0, p.head_size(), O);
        }
        I::TextBox => {
            p.rect(2.5, 4.5, 21.5, 19.5, 1.5, N, K.a(0.6));
            p.seg((7.5, 8.0), (16.5, 8.0), K);
            p.seg((12.0, 8.0), (12.0, 16.5), K);
            p.seg((10.0, 16.5), (14.0, 16.5), O);
        }
        I::Image => {
            p.rect(2.5, 4.5, 21.5, 19.5, 1.5, N, K);
            p.poly(
                &[(4.5, 17.5), (9.5, 11.0), (13.0, 15.0), (15.5, 12.5), (19.5, 17.5)],
                tint(B),
                B,
            );
            p.dot((16.5, 8.5), 1.6, O);
        }
        I::Button => {
            p.rect(2.5, 6.5, 21.5, 15.5, 4.0, tint(O), K);
            p.seg((7.0, 11.0), (14.0, 11.0), K.a(0.7));
            p.poly(
                &[
                    (15.5, 12.5),
                    (15.5, 21.0),
                    (17.6, 19.0),
                    (19.2, 22.0),
                    (20.4, 21.3),
                    (18.8, 18.4),
                    (21.5, 18.0),
                ],
                C::White,
                K,
            );
        }
        I::Cut => {
            p.circle((6.5, 18.0), 3.0, N, K);
            p.circle((17.5, 18.0), 3.0, N, K);
            p.seg((8.5, 15.8), (17.0, 3.0), K);
            p.seg((15.5, 15.8), (7.0, 3.0), K);
            p.dot((12.0, 9.8), 1.2, O);
        }
        I::Shapes => {
            p.rect(2.5, 2.5, 12.0, 12.0, 0.8, tint(B), B);
            p.circle((16.5, 16.5), 5.0, tint(O), O);
            p.poly(&[(6.5, 13.5), (11.0, 21.5), (2.0, 21.5)], N, K);
        }
        I::Page => {
            p.rect(2.5, 4.5, 21.5, 19.5, 1.5, N, K);
            p.rect(5.5, 7.5, 11.5, 11.5, 0.6, tint(O), O);
            p.bars(13.5, 7.5, 18.5, 16.5);
            p.seg((5.5, 14.5), (11.5, 14.5), K.a(0.6));
        }
        I::Model => {
            p.card(2.0, 3.0, 10.0, 11.0, O);
            p.card(14.0, 3.0, 22.0, 11.0, T);
            p.card(8.0, 14.0, 16.0, 22.0, B);
            p.seg((10.0, 7.0), (14.0, 7.0), K);
            p.seg((6.0, 11.0), (10.0, 14.0), K);
        }
        I::Validate => {
            p.rshape(
                &[
                    (12.0, 2.0),
                    (20.5, 5.0),
                    (20.0, 13.0),
                    (12.0, 22.0),
                    (4.0, 13.0),
                    (3.5, 5.0),
                ],
                1.5,
                tint(G),
                K,
            );
            p.line_w(&[(8.0, 12.0), (11.0, 15.0), (16.5, 8.5)], G, 1.2);
        }
        I::Measure => {
            p.line(
                &[(18.0, 4.0), (6.0, 4.0), (12.5, 12.0), (6.0, 20.0), (18.0, 20.0)],
                O,
            );
        }
        I::Column => {
            p.rect(4.0, 3.0, 20.0, 21.0, 1.2, N, K);
            p.rect(9.0, 3.0, 15.0, 21.0, 0.0, tint(O), O);
        }
        I::Relationship => {
            p.card(2.0, 8.0, 9.0, 16.0, O);
            p.card(15.0, 8.0, 22.0, 16.0, T);
            p.seg((9.0, 12.0), (15.0, 12.0), K);
            p.dot((12.0, 12.0), 1.6, K);
        }
        I::Server => {
            for y in [3.0, 10.0, 17.0] {
                p.rect(3.5, y, 20.5, y + 5.0, 1.2, N, K);
                p.dot((7.0, y + 2.5), 1.0, T);
            }
        }
        I::Sun => {
            p.circle((12.0, 12.0), 4.5, tint(Y), Y);
            for i in 0..8 {
                let a = rad(i as f32 * 45.0);
                let (s, c) = a.sin_cos();
                p.seg(
                    (12.0 + 7.5 * c, 12.0 + 7.5 * s),
                    (12.0 + 10.0 * c, 12.0 + 10.0 * s),
                    K,
                );
            }
        }
        I::Moon => {
            // a crescent: the outer circle from one intersection to the other, back along the
            // inside of the cutting circle
            let mut pts = arc_pts((12.0, 12.0), 9.0, 9.0, -0.6, 258.0);
            pts.extend(arc_pts((16.0, 7.0), 7.0, 7.0, 213.0, 44.4));
            p.shape(&pts, tint(B), K);
        }
        I::Contrast => {
            // a disc, its left half filled with ink: high contrast
            let mut half = arc_pts((12.0, 12.0), 9.0, 9.0, 90.0, 270.0);
            half.push((12.0, 3.0));
            p.shape(&half, K, N);
            p.circle((12.0, 12.0), 9.0, N, K);
        }
        I::OpenInDesktop => {
            p.rect(2.5, 4.0, 18.5, 16.5, 1.2, N, K);
            p.seg((2.5, 7.0), (18.5, 7.0), K);
            p.seg((10.5, 16.5), (10.5, 20.5), K);
            p.seg((6.5, 20.5), (14.5, 20.5), K);
            p.arrow((11.0, 13.0), (22.0, 3.0), O);
        }
        I::FitWidth => {
            p.rect(2.5, 5.0, 21.5, 19.0, 1.5, N, K);
            p.arrow((12.0, 12.0), (4.5, 12.0), O);
            p.arrow((12.0, 12.0), (19.5, 12.0), O);
        }
        I::Language => {
            p.circle((12.0, 12.0), 9.5, N, K);
            p.seg((2.5, 12.0), (21.5, 12.0), K.a(0.7));
            let half = |a0: f32, a1: f32| arc_pts((12.0, 12.0), 4.5, 9.5, a0, a1);
            p.line(&half(-90.0, 90.0), K.a(0.7));
            p.line(&half(90.0, 270.0), K.a(0.7));
        }
        I::Save => {
            // a disk: the body, the label window, the shutter
            p.rshape(
                &[(3.5, 3.5), (17.5, 3.5), (20.5, 6.5), (20.5, 20.5), (3.5, 20.5)],
                1.5,
                N,
                K,
            );
            p.rect(7.0, 3.5, 15.0, 8.5, 0.6, tint(B), K);
            p.rect(7.0, 13.0, 17.0, 20.5, 0.8, N, K);
            p.seg((9.5, 16.5), (14.5, 16.5), B);
        }
        I::Back => {
            p.circle((12.0, 12.0), 9.8, N, K);
            p.arrow((17.0, 12.0), (6.5, 12.0), K);
        }
        I::Pin => {
            p.rshape(
                &[
                    (9.0, 3.0),
                    (15.0, 3.0),
                    (14.0, 10.0),
                    (17.5, 14.0),
                    (6.5, 14.0),
                    (10.0, 10.0),
                ],
                0.8,
                tint(O),
                K,
            );
            p.seg((12.0, 14.0), (12.0, 21.5), K);
        }
        I::Copy => {
            p.page(8.5, 6.5, 20.5, 22.0, N, K);
            p.rline(
                &[(15.5, 4.5), (15.5, 2.0), (3.5, 2.0), (3.5, 17.5), (6.0, 17.5)],
                1.2,
                K,
            );
            p.seg((11.5, 13.0), (17.5, 13.0), B);
            p.seg((11.5, 16.5), (16.0, 16.5), B);
        }
        // ----- BIcraft, for the command registry's buttons -----
        I::Calendar => {
            p.rect(3.0, 5.0, 21.0, 21.0, 1.5, N, K);
            p.rect(3.0, 5.0, 21.0, 9.5, 1.5, tint(O), K);
            p.seg((8.0, 3.0), (8.0, 7.0), K);
            p.seg((16.0, 3.0), (16.0, 7.0), K);
            for (x, y) in [(8.0, 13.5), (12.0, 13.5), (16.0, 13.5), (8.0, 17.5), (12.0, 17.5)] {
                p.dot((x, y), 1.1, K);
            }
            p.dot((16.0, 17.5), 1.4, O);
        }
        I::Compare => {
            p.rect(2.5, 3.0, 10.5, 21.0, 1.2, N, K);
            p.rect(13.5, 3.0, 21.5, 21.0, 1.2, N, K);
            for (y, c) in [(8.0, K), (12.0, O), (16.0, K)] {
                p.seg((5.0, y), (8.0, y), c);
                p.seg((16.0, y), (19.0, y), c);
            }
        }
        I::Delete => {
            p.seg((3.5, 6.0), (20.5, 6.0), K);
            p.rline(&[(9.0, 6.0), (9.0, 3.0), (15.0, 3.0), (15.0, 6.0)], 0.8, K);
            p.rshape(&[(5.5, 6.0), (18.5, 6.0), (17.0, 21.0), (7.0, 21.0)], 1.2, N, K);
            let r = C::Acc(RED, 1.0);
            p.seg((10.0, 10.0), (10.3, 17.5), r);
            p.seg((14.0, 10.0), (13.7, 17.5), r);
        }
        I::Edit => {
            p.shape(
                &[(3.5, 20.5), (4.5, 15.5), (15.5, 4.5), (19.5, 8.5), (8.5, 19.5)],
                tint(O),
                K,
            );
            p.seg((13.0, 7.0), (17.0, 11.0), K);
            p.seg((4.5, 15.5), (8.5, 19.5), O);
        }
        I::Filter => {
            p.rshape(
                &[
                    (2.5, 4.0),
                    (21.5, 4.0),
                    (14.0, 12.5),
                    (14.0, 19.0),
                    (10.0, 21.0),
                    (10.0, 12.5),
                ],
                1.0,
                tint(B),
                K,
            );
        }
        I::Format => {
            p.rect(3.0, 3.0, 17.5, 8.5, 1.2, tint(O), K);
            p.rline(
                &[
                    (17.5, 5.75),
                    (20.5, 5.75),
                    (20.5, 11.5),
                    (12.0, 11.5),
                    (12.0, 14.0),
                ],
                1.0,
                K,
            );
            p.rect(10.5, 14.0, 13.5, 21.5, 0.8, K, K);
        }
        I::Paste => {
            p.rect(4.5, 4.0, 19.5, 22.0, 1.5, N, K);
            p.rect(8.5, 2.0, 15.5, 6.0, 1.0, tint(O), K);
            p.seg((8.0, 11.0), (16.0, 11.0), B);
            p.seg((8.0, 15.0), (14.0, 15.0), B);
            p.seg((8.0, 18.5), (12.0, 18.5), B);
        }
        I::Duplicate => {
            p.rect(2.5, 2.5, 14.0, 14.0, 1.2, N, K);
            p.rect(9.5, 9.5, 21.5, 21.5, 1.2, tint(O), K);
            p.seg((15.5, 12.5), (15.5, 18.5), K);
            p.seg((12.5, 15.5), (18.5, 15.5), K);
        }
        I::Align => {
            p.seg((3.5, 2.5), (3.5, 21.5), O);
            p.rect(6.5, 5.0, 20.5, 10.0, 1.0, tint(B), K);
            p.rect(6.5, 14.0, 14.5, 19.0, 1.0, N, K);
        }
        I::Group => {
            p.rect(2.5, 2.5, 21.5, 21.5, 2.0, N, O);
            p.rect(6.0, 6.0, 13.5, 13.5, 1.0, tint(B), K);
            p.rect(10.5, 10.5, 18.0, 18.0, 1.0, N, K);
        }
        I::Print => {
            // a printer: the paper fed in at the top, the body, the printed sheet coming out
            p.rect(7.0, 2.5, 17.0, 8.0, 0.6, N, K);
            p.rect(2.5, 8.0, 21.5, 17.0, 2.0, tint(B), K);
            p.rect(7.0, 13.5, 17.0, 21.5, 0.6, if p.dis { N } else { C::White }, K);
            p.seg((9.5, 16.5), (14.5, 16.5), B);
            p.seg((9.5, 19.0), (13.0, 19.0), B);
            p.dot((18.5, 11.0), 0.9, O);
        }
        I::Ungroup => {
            p.rect(2.5, 2.5, 10.5, 10.5, 1.0, tint(B), K);
            p.rect(13.5, 13.5, 21.5, 21.5, 1.0, N, K);
            p.seg((13.5, 10.5), (10.5, 13.5), O);
        }
        I::Analytics => {
            // a rising line over a dashed reference line, a magnifier at the bottom right
            p.line(&[(2.5, 15.0), (7.0, 10.0), (11.0, 12.5), (17.0, 4.0)], B);
            for x in [2.5f32, 7.0, 11.5] {
                p.seg((x, 8.0), (x + 2.5, 8.0), O);
            }
            p.magnifier((14.5, 15.5), 4.0, K);
        }
        I::Focus => {
            // a frame with two arrows pointing out of it to opposite corners
            p.rect(3.0, 5.0, 21.0, 19.0, 1.5, N, K.a(0.6));
            p.arrow((12.5, 11.5), (19.0, 5.5), K);
            p.arrow((11.5, 12.5), (5.0, 18.5), O);
        }
        I::Move => {
            p.arrow((12.0, 12.0), (12.0, 2.5), K);
            p.arrow((12.0, 12.0), (12.0, 21.5), K);
            p.arrow((12.0, 12.0), (2.5, 12.0), K);
            p.arrow((12.0, 12.0), (21.5, 12.0), K);
            p.dot((12.0, 12.0), 2.0, O);
        }
        I::Calculator => {
            p.rect(5.0, 2.5, 19.0, 21.5, 1.5, N, K);
            p.rect(7.5, 5.0, 16.5, 8.5, 0.6, tint(O), O);
            for (i, y) in [11.5f32, 15.0, 18.5].into_iter().enumerate() {
                for x in [8.5f32, 12.0, 15.5] {
                    let c = if i == 2 && x > 15.0 { O } else { K };
                    p.dot((x, y), 1.1, c);
                }
            }
        }
        I::Hierarchy => {
            p.rect(9.0, 2.5, 15.0, 7.5, 1.0, tint(O), O);
            p.rect(3.0, 16.5, 9.0, 21.5, 1.0, N, K);
            p.rect(15.0, 16.5, 21.0, 21.5, 1.0, N, K);
            p.seg((12.0, 7.5), (12.0, 12.0), K);
            p.rline(&[(6.0, 16.5), (6.0, 12.0), (18.0, 12.0), (18.0, 16.5)], 0.8, K);
        }
        I::Import => {
            p.rline(
                &[
                    (9.0, 8.5),
                    (9.0, 2.5),
                    (21.0, 2.5),
                    (21.0, 21.5),
                    (9.0, 21.5),
                    (9.0, 15.5),
                ],
                1.5,
                K,
            );
            p.arrow((2.0, 12.0), (15.5, 12.0), O);
        }
        I::Layers => {
            p.line(&[(3.0, 16.0), (12.0, 21.0), (21.0, 16.0)], K);
            p.line(&[(3.0, 12.0), (12.0, 17.0), (21.0, 12.0)], K);
            p.shape(&[(12.0, 3.0), (21.0, 8.0), (12.0, 13.0), (3.0, 8.0)], tint(O), K);
        }
        I::Highlight => {
            // three bars, the middle one drawn full over its dimmed part
            p.rect(3.0, 10.0, 8.0, 21.0, 0.8, tint(B), N);
            p.rect(9.5, 3.0, 14.5, 21.0, 0.8, tint(B), N);
            p.rect(9.5, 11.0, 14.5, 21.0, 0.8, B, N);
            p.rect(16.0, 7.0, 21.0, 21.0, 0.8, tint(B), N);
            p.seg((2.0, 21.5), (22.0, 21.5), K);
        }
        I::NoInteraction => {
            p.circle((12.0, 12.0), 8.5, N, K);
            p.seg_w((6.0, 18.0), (18.0, 6.0), K, 1.1);
        }
        I::Link => {
            p.rect(2.5, 8.0, 13.5, 16.0, 4.0, N, K);
            p.rect(10.5, 8.0, 21.5, 16.0, 4.0, N, B);
            p.seg((8.0, 12.0), (16.0, 12.0), K);
        }
        I::Mobile => {
            p.rect(6.5, 2.5, 17.5, 21.5, 2.0, N, K);
            p.rect(8.5, 6.0, 15.5, 16.0, 0.5, tint(O), N);
            p.seg((10.5, 4.25), (13.5, 4.25), K);
            p.dot((12.0, 18.75), 1.1, K);
        }
        I::Palette => {
            let mut pts = arc_pts((12.0, 12.0), 9.5, 9.0, 20.0, 340.0);
            pts.extend(arc_pts((17.5, 12.0), 2.6, 2.6, 270.0, 90.0));
            p.shape(&pts, N, K);
            for (x, y, c) in [(7.5, 9.0, B), (11.5, 6.5, O), (7.5, 14.5, G), (12.0, 17.0, Y)] {
                p.dot((x, y), 1.8, c);
            }
        }
        I::Play => p.rshape(&[(7.0, 4.0), (19.5, 12.0), (7.0, 20.0)], 1.5, G, N),
        I::Undo | I::Redo => {
            let redo = icon == I::Redo;
            let m = |x: f32| if redo { 24.0 - x } else { x };
            let mut pts = vec![(m(6.5), 9.0), (m(14.0), 9.0)];
            pts.extend(
                arc_pts((14.0, 14.0), 5.0, 5.0, -90.0, 90.0)
                    .into_iter()
                    .map(|(x, y)| (m(x), y)),
            );
            pts.push((m(8.0), 19.0));
            p.line(&pts, K);
            p.head((m(3.5), 9.0), if redo { 0.0 } else { 180.0 }, p.head_size(), O);
        }
        I::Rename => {
            p.rect(2.5, 7.0, 21.5, 17.0, 1.5, N, K);
            p.seg((5.5, 12.0), (11.0, 12.0), K);
            p.seg((15.0, 4.5), (15.0, 19.5), O);
            p.seg((13.0, 4.5), (17.0, 4.5), O);
            p.seg((13.0, 19.5), (17.0, 19.5), O);
        }
        I::SaveAs => {
            p.rshape(
                &[
                    (3.5, 3.5),
                    (15.5, 3.5),
                    (18.0, 6.0),
                    (18.0, 12.0),
                    (12.0, 18.0),
                    (3.5, 18.0),
                ],
                1.5,
                N,
                K,
            );
            p.rect(6.5, 3.5, 13.0, 7.5, 0.6, tint(B), K);
            p.shape(
                &[
                    (12.5, 21.5),
                    (13.0, 18.5),
                    (19.0, 12.5),
                    (21.5, 15.0),
                    (15.5, 21.0),
                ],
                tint(O),
                K,
            );
        }
        I::Sort => {
            p.arrow((7.0, 20.0), (7.0, 4.0), K);
            p.arrow((17.0, 4.0), (17.0, 20.0), O);
        }
        I::Logo => {}
    }
}

// ----- the Visualizations pane's gallery (M11.26) -----

/// Paints the gallery's illustration of a visual type in `rect`: a small picture of the visual,
/// drawn like Desktop's (a blue chart on a grey baseline), under our own shapes and colours.
pub fn paint_visual_type(painter: &Painter, rect: Rect, ty: &str, ink: Color32, disabled: bool) {
    let size = rect.width().min(rect.height());
    if !size.is_finite() || size <= 0.5 || size > 4096.0 {
        return;
    }
    let origin = rect.center() - vec2(size * 0.5, size * 0.5);
    let pen = Pen {
        p: painter,
        o: origin,
        k: size / 24.0,
        sw: stroke_width(size),
        ink,
        dis: disabled,
    };
    draw_visual_type(&pen, ty);
}

/// A pie slice: the centre and the arc from `a0` to `a1` degrees.
fn sector(c: P2, r: f32, a0: f32, a1: f32) -> Vec<P2> {
    let mut v = vec![c];
    v.extend(arc_pts(c, r, r, a0, a1));
    v
}

/// An annular slice between `r0` and `r1` from `a0` to `a1` degrees.
fn ring_sector(c: P2, r0: f32, r1: f32, a0: f32, a1: f32) -> Vec<P2> {
    let mut v = arc_pts(c, r1, r1, a0, a1);
    v.extend(arc_pts(c, r0, r0, a1, a0));
    v
}

/// The light baseline of a chart illustration.
fn axis(p: &Pen) {
    p.line(&[(4.0, 3.5), (4.0, 20.5), (21.0, 20.5)], K.a(0.45));
}

/// The drawing of one of BIcraft's visuals (`bicraft_visuals::icons`, the same marks its package's
/// icon is rasterized from), in the pen's ink and blue accent.
fn draw_marks(p: &Pen, marks: &[bicraft_visuals::icons::Mark]) {
    use bicraft_visuals::icons::{Mark, Tone};
    let c = |t: Option<Tone>| match t {
        Some(Tone::Ink(a)) => C::Ink(a),
        Some(Tone::Accent(a)) => C::Acc(BLUE, a),
        None => N,
    };
    for m in marks.iter().take(256) {
        match m {
            Mark::Line { pts, width, tone } => p.line_w(pts, c(Some(*tone)), *width),
            Mark::Fill { pts, tone } => p.shape(pts, c(Some(*tone)), N),
            Mark::Rect {
                min,
                max,
                radius,
                fill,
                stroke,
            } => p.rect(min.0, min.1, max.0, max.1, *radius, c(*fill), c(*stroke)),
            Mark::Circle {
                center,
                radius,
                fill,
                stroke,
            } => p.circle(*center, *radius, c(*fill), c(*stroke)),
        }
    }
}

/// The gallery's pictures, one per family of built-in visual (Desktop's grid), and one per
/// BIcraft visual (by its name).
fn draw_visual_type(p: &Pen, ty: &str) {
    if let Some(marks) = bicraft_visuals::icons::bicraft_visual_icon(ty) {
        draw_marks(p, &marks);
        return;
    }
    match ty {
        "tableEx" => p.table(3.0, 4.0, 21.0, 20.0, 4, 3, B),
        "pivotTable" => {
            p.table(3.0, 4.0, 21.0, 20.0, 4, 3, B);
            p.rect(3.0, 9.0, 9.0, 20.0, 0.6, tint(B), N);
        }
        "card" => {
            p.rect(3.0, 5.0, 21.0, 19.5, 2.0, N, K.a(0.6));
            p.rect(7.0, 10.0, 17.0, 14.0, 0.8, B, N);
        }
        "cardVisual" => {
            p.rect(3.0, 5.0, 21.0, 19.5, 2.0, N, K.a(0.6));
            p.rect(6.0, 9.0, 16.0, 12.5, 0.8, B, N);
            p.seg((6.0, 15.5), (14.0, 15.5), K.a(0.5));
        }
        "multiRowCard" => {
            for (y, w) in [(5.5, 9.0), (10.5, 12.0), (15.5, 7.5)] {
                p.rect(3.5, y, 3.5 + w, y + 2.2, 0.6, B, N);
                p.seg((3.5, y + 3.6), (20.5, y + 3.6), K.a(0.45));
            }
        }
        "slicer" | "listSlicer" | "textSlicer" | "advancedSlicerVisual" => {
            p.rect(3.0, 4.0, 21.0, 20.0, 1.5, N, K.a(0.6));
            for (y, on) in [(7.0, true), (11.5, false), (15.5, false)] {
                p.rect(4.8, y, 7.0, y + 1.8, 0.4, if on { B } else { N }, K.a(0.45));
                p.seg((8.6, y + 0.9), (19.0, y + 0.9), K.a(0.45));
            }
        }
        "clusteredColumnChart" => {
            axis(p);
            for (x, top) in [(6.0, 9.0), (11.0, 5.5), (16.0, 12.0)] {
                p.rect(x, top, x + 3.6, 20.0, 0.4, B, N);
            }
        }
        "columnChart" => {
            axis(p);
            for x in [6.5, 13.5] {
                p.rect(x, 8.0, x + 4.0, 20.0, 0.4, B, N);
                p.rect(x, 4.5, x + 4.0, 8.0, 0.4, B.a(0.55), N);
            }
        }
        "hundredPercentStackedColumnChart" => {
            axis(p);
            for x in [6.5, 13.5] {
                p.rect(x, 11.0, x + 4.0, 20.0, 0.4, B, N);
                p.rect(x, 4.5, x + 4.0, 11.0, 0.4, B.a(0.55), N);
            }
        }
        "clusteredBarChart" => {
            p.line(&[(4.0, 3.5), (4.0, 20.5), (21.0, 20.5)], K.a(0.45));
            for (y, right) in [(5.5, 18.0), (10.5, 12.5), (15.5, 15.5)] {
                p.rect(4.0, y, right, y + 2.8, 0.4, B, N);
            }
        }
        "barChart" => {
            p.line(&[(4.0, 3.5), (4.0, 20.5), (21.0, 20.5)], K.a(0.45));
            for (y, split) in [(5.5, 12.0), (12.0, 9.0)] {
                p.rect(4.0, y, split, y + 3.5, 0.4, B, N);
                p.rect(split, y, 20.0, y + 3.5, 0.4, B.a(0.55), N);
            }
        }
        "hundredPercentStackedBarChart" => {
            p.line(&[(4.0, 3.5), (4.0, 20.5), (21.0, 20.5)], K.a(0.45));
            for y in [5.5, 12.0] {
                p.rect(4.0, y, 13.0, y + 3.5, 0.4, B, N);
                p.rect(13.0, y, 20.0, y + 3.5, 0.4, B.a(0.55), N);
            }
        }
        "lineChart" => {
            axis(p);
            let pts = [(5.0, 15.0), (9.5, 9.0), (14.0, 12.0), (19.5, 5.5)];
            p.line_w(&pts, B, 1.5);
            for q in pts {
                p.dot(q, 1.2, B);
            }
        }
        "areaChart" | "stackedAreaChart" | "hundredPercentStackedAreaChart" => {
            axis(p);
            let top = [(5.0, 13.0), (9.5, 7.5), (14.0, 10.5), (19.5, 5.0)];
            if ty == "areaChart" {
                let mut poly = top.to_vec();
                poly.extend([(19.5, 20.0), (5.0, 20.0)]);
                p.shape(&poly, B.a(0.45), N);
            } else {
                let back = [(5.0, 6.0), (9.5, 4.5), (14.0, 6.5), (19.5, 4.0)];
                let mut poly = back.to_vec();
                poly.extend([(19.5, 20.0), (5.0, 20.0)]);
                p.shape(&poly, B.a(0.35), N);
                let mut front = top.to_vec();
                front.extend([(19.5, 20.0), (5.0, 20.0)]);
                p.shape(&front, B.a(0.65), N);
            }
            p.line_w(&top, B, 1.3);
        }
        "lineClusteredColumnComboChart" | "lineStackedColumnComboChart" => {
            axis(p);
            for (x, top) in [(6.0, 13.0), (11.0, 10.0), (16.0, 14.5)] {
                p.rect(x, top, x + 3.4, 20.0, 0.4, B.a(0.7), N);
            }
            p.line_w(&[(5.0, 9.0), (9.5, 5.5), (14.0, 8.0), (19.5, 4.0)], B, 1.5);
        }
        "pieChart" => {
            let c = (12.0, 12.0);
            p.shape(&sector(c, 8.0, -90.0, 45.0), B, N);
            p.shape(&sector(c, 8.0, 45.0, 160.0), B.a(0.55), N);
            p.shape(&sector(c, 8.0, 160.0, 270.0), tint(B), N);
            p.circle(c, 8.0, N, K.a(0.55));
        }
        "donutChart" => {
            let c = (12.0, 12.0);
            p.shape(&ring_sector(c, 4.0, 8.5, -90.0, 60.0), B, N);
            p.shape(&ring_sector(c, 4.0, 8.5, 60.0, 190.0), B.a(0.55), N);
            p.shape(&ring_sector(c, 4.0, 8.5, 190.0, 270.0), tint(B), N);
            p.circle(c, 8.5, N, K.a(0.55));
            p.circle(c, 4.0, N, K.a(0.55));
        }
        "scatterChart" => {
            axis(p);
            for (x, y, c) in [
                (6.5, 15.0, B),
                (10.0, 9.5, B),
                (14.0, 13.0, B.a(0.55)),
                (17.5, 7.5, B.a(0.55)),
                (9.0, 17.0, B),
            ] {
                p.dot((x, y), 1.6, c);
            }
        }
        "treemap" => {
            p.rect(3.5, 4.0, 12.5, 20.0, 0.4, B, N);
            p.rect(12.5, 4.0, 20.5, 11.5, 0.4, B.a(0.7), N);
            p.rect(12.5, 11.5, 20.5, 15.5, 0.4, B.a(0.5), N);
            p.rect(12.5, 15.5, 20.5, 20.0, 0.4, tint(B), N);
            p.rect(3.5, 4.0, 20.5, 20.0, 0.4, N, K.a(0.55));
        }
        "funnel" => {
            for (pts, c) in [
                ([(4.0, 4.0), (20.0, 4.0), (17.0, 9.5), (7.0, 9.5)], B),
                ([(7.6, 10.5), (16.4, 10.5), (14.8, 15.5), (9.2, 15.5)], B.a(0.7)),
                (
                    [(10.2, 16.5), (13.8, 16.5), (13.0, 20.0), (11.0, 20.0)],
                    B.a(0.45),
                ),
            ] {
                p.poly(&pts, c, K.a(0.45));
            }
        }
        "waterfallChart" => {
            axis(p);
            p.rect(5.0, 13.0, 8.2, 20.0, 0.4, B, N);
            p.rect(9.6, 8.0, 12.8, 20.0, 0.4, B, N);
            p.rect(14.2, 11.0, 17.4, 20.0, 0.4, B, N);
            p.seg((5.0, 13.0), (8.2, 13.0), K.a(0.5));
            p.seg((9.6, 8.0), (12.8, 8.0), K.a(0.5));
            p.seg((14.2, 11.0), (17.4, 11.0), K.a(0.5));
        }
        "ribbonChart" => {
            axis(p);
            for (dy, c, h) in [(0.0, B, 4.0), (4.6, B.a(0.6), 3.4), (8.6, tint(B), 3.0)] {
                let mut band = Vec::with_capacity(28);
                for i in 0..=13 {
                    let x = 4.5 + 16.0 * i as f32 / 13.0;
                    band.push((x, 5.0 + dy + h * 0.5 + (i as f32 * 0.6).sin() * 1.6));
                }
                for i in (0..=13).rev() {
                    let x = 4.5 + 16.0 * i as f32 / 13.0;
                    band.push((x, 5.0 + dy + h * 0.5 + (i as f32 * 0.6).sin() * 1.6 + h * 0.45));
                }
                p.shape(&band, c, N);
            }
        }
        "gauge" => {
            let (c, r) = ((12.0, 16.5), 8.5);
            let mut arc = Vec::with_capacity(13);
            for i in 0..=12 {
                let a = std::f32::consts::PI + std::f32::consts::PI * i as f32 / 12.0;
                arc.push((c.0 + r * a.cos(), c.1 + r * a.sin()));
            }
            p.line_w(&arc, B, 1.9);
            p.seg_w(c, (18.5, 8.0), K.a(0.8), 1.9);
            p.dot(c, 1.4, K);
        }
        "kpi" => {
            p.rect(3.0, 4.5, 21.0, 19.5, 2.0, N, K.a(0.6));
            p.line_w(&[(6.5, 15.0), (10.0, 11.5), (13.0, 13.5)], B, 1.6);
            p.arrow((13.0, 13.5), (18.0, 8.5), B);
        }
        "map" | "filledMap" => {
            p.rect(3.0, 4.5, 21.0, 19.5, 1.0, N, K.a(0.55));
            p.seg((10.0, 4.5), (10.0, 19.5), K.a(0.3));
            p.seg((16.0, 4.5), (16.0, 19.5), K.a(0.3));
            p.seg((3.0, 12.0), (21.0, 12.0), K.a(0.3));
            if ty == "filledMap" {
                p.shape(&[(4.8, 7.0), (9.0, 5.8), (9.4, 12.6), (5.4, 14.5)], B.a(0.7), N);
                p.shape(
                    &[(11.4, 8.0), (15.0, 6.5), (15.0, 12.0), (11.4, 13.5)],
                    B.a(0.45),
                    N,
                );
            } else {
                p.dot((13.2, 12.2), 1.7, B);
            }
        }
        "azureMap" => {
            p.rect(3.0, 6.0, 21.0, 20.0, 1.0, N, K.a(0.55));
            p.seg((10.0, 6.0), (10.0, 20.0), K.a(0.3));
            p.seg((16.0, 6.0), (16.0, 20.0), K.a(0.3));
            // a location pin
            p.circle((13.0, 9.0), 3.5, tint(B), B);
            p.poly(&[(13.0, 20.0), (10.6, 11.6), (15.4, 11.6)], B, N);
            p.dot((13.0, 9.0), 1.2, K);
        }
        "shapeMap" => {
            p.shape(
                &[
                    (4.0, 12.0),
                    (6.5, 5.0),
                    (13.0, 4.0),
                    (20.0, 8.0),
                    (19.0, 16.0),
                    (10.0, 20.0),
                ],
                tint(B),
                K.a(0.55),
            );
            p.shape(
                &[(9.0, 9.0), (14.0, 7.5), (15.5, 13.0), (10.5, 15.0)],
                B.a(0.7),
                N,
            );
        }
        "decompositionTreeVisual" => {
            p.circle((5.5, 12.0), 2.3, B, N);
            p.seg((7.8, 12.0), (10.5, 12.0), K.a(0.55));
            p.seg((10.5, 6.5), (10.5, 17.5), K.a(0.55));
            p.seg((10.5, 6.5), (13.0, 6.5), K.a(0.55));
            p.seg((10.5, 17.5), (13.0, 17.5), K.a(0.55));
            p.circle((15.5, 6.5), 2.3, B.a(0.7), N);
            p.circle((15.5, 17.5), 2.3, B.a(0.7), N);
        }
        "keyDriversVisual" => {
            p.circle((14.5, 12.0), 6.5, N, K.a(0.55));
            p.circle((14.5, 12.0), 2.6, N, K.a(0.55));
            p.arrow((3.5, 19.0), (12.4, 13.6), B);
        }
        "ganttChart" => {
            // task bars staggered along a time axis
            p.seg((4.0, 4.0), (4.0, 20.0), K.a(0.55));
            for (x0, x1, y) in [(5.5, 12.0, 6.0), (9.0, 17.0, 10.5), (14.0, 21.0, 15.0)] {
                p.rect(x0, y, x1, y + 3.0, 0.6, B, N);
            }
        }
        "pythonVisual" | "scriptVisual" => {
            p.rect(3.0, 4.5, 21.0, 19.5, 1.5, N, K.a(0.6));
            p.seg_w((6.5, 9.0), (10.5, 9.0), B, 1.4);
            p.seg((6.5, 12.5), (16.5, 12.5), K.a(0.45));
            p.seg((6.5, 16.0), (12.5, 16.0), K.a(0.45));
        }
        "rdlVisual" => {
            p.page(3.5, 3.5, 15.0, 20.5, N, K.a(0.6));
            p.table(8.5, 10.5, 20.5, 17.5, 3, 2, B);
        }
        // a type with no picture of its own: the column chart stands for the family
        _ => {
            axis(p);
            for (x, top) in [(6.0, 9.0), (11.0, 6.0), (16.0, 12.0)] {
                p.rect(x, top, x + 3.6, 20.0, 0.4, B, N);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn names_round_trip_and_are_unique() {
        let mut names = HashSet::new();
        for &icon in Icon::ALL {
            let name = icon.name();
            assert!(names.insert(name), "duplicate name {name}");
            assert_eq!(Icon::from_name(name), Some(icon));
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "{name} is not kebab-case"
            );
        }
        assert_eq!(Icon::from_name("no-such-icon"), None);
    }

    #[test]
    fn stroke_width_scales() {
        assert!((stroke_width(16.0) - 1.0).abs() < 1e-6);
        assert!((stroke_width(32.0) - 1.6).abs() < 1e-6);
        assert!((stroke_width(8.0) - 1.0).abs() < 1e-6);
    }

    /// Every icon paints at every size, enabled and disabled, and odd rectangles are no-ops.
    #[test]
    fn every_icon_paints_at_any_size() {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            for &icon in Icon::ALL {
                for s in [8.0, 16.0, 17.0, 30.0, 64.0] {
                    for dis in [false, true] {
                        paint(
                            ui.painter(),
                            Rect::from_min_size(pos2(4.0, 4.0), vec2(s, s)),
                            icon,
                            Color32::BLACK,
                            dis,
                        );
                    }
                }
                for r in [
                    Rect::NOTHING,
                    Rect::EVERYTHING,
                    Rect::from_min_size(pos2(0.0, 0.0), vec2(f32::NAN, 10.0)),
                    Rect::from_min_size(pos2(0.0, 0.0), vec2(0.1, 0.1)),
                    Rect::from_min_size(pos2(0.0, 0.0), vec2(1e9, 1e9)),
                ] {
                    paint(ui.painter(), r, icon, Color32::BLACK, false);
                }
            }
        });
        out.textures_delta.clear();
    }

    /// What `paint_visual_type` paints for `ty` in a 22 pt cell (the gallery's).
    fn visual_type_shapes(ty: &str, ink: Color32, disabled: bool) -> String {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            paint_visual_type(
                ui.painter(),
                Rect::from_min_size(pos2(10.0, 10.0), vec2(22.0, 22.0)),
                ty,
                ink,
                disabled,
            );
        });
        out.textures_delta.clear();
        format!("{:?}", out.shapes)
    }

    /// BIcraft's own visuals each have their own picture in the gallery (ROADMAP M12.4), drawn
    /// like the built-in types' (ink and blue, the ink following the theme), never the generic
    /// column chart of an unknown type.
    #[test]
    fn bicraft_visuals_have_distinct_pictures() {
        let generic = visual_type_shapes("noSuchVisualType", Color32::BLACK, false);
        let mut seen = vec![generic.clone()];
        for &n in bicraft_visuals::icons::BICRAFT_VISUAL_ICONS {
            let light = visual_type_shapes(n, Color32::from_rgb(0x24, 0x24, 0x24), false);
            assert!(!seen.contains(&light), "{n}: the same picture as another type");
            let dark = visual_type_shapes(n, Color32::from_rgb(0xee, 0xee, 0xee), false);
            let inked = bicraft_visuals::icons::bicraft_visual_icon(n)
                .unwrap_or_default()
                .iter()
                .any(|m| format!("{m:?}").contains("Ink("));
            if inked {
                assert_ne!(light, dark, "{n}: its ink follows the theme");
            }
            let disabled = visual_type_shapes(n, Color32::from_rgb(0x24, 0x24, 0x24), true);
            assert_ne!(light, disabled, "{n}: dimmed when disabled");
            seen.push(light);
        }
        assert_eq!(seen.len(), bicraft_visuals::icons::BICRAFT_VISUAL_ICONS.len() + 1);
    }

    #[test]
    fn helpers_take_hostile_input() {
        assert!(triangulate(&[]).is_empty());
        assert!(triangulate(&[Pos2::ZERO, Pos2::ZERO]).is_empty());
        assert_eq!(
            triangulate(&[Pos2::ZERO, pos2(1.0, 0.0), pos2(0.0, 1.0)]).len(),
            1
        );
        // collinear and repeated points: no endless loop, no panic
        let line: Vec<Pos2> = (0..50).map(|i| pos2(i as f32, i as f32)).collect();
        let _ = triangulate(&line);
        let nan = [pos2(f32::NAN, 0.0), pos2(1.0, 0.0), pos2(0.0, 1.0)];
        assert!(triangulate(&nan).is_empty());
        let big: Vec<Pos2> = (0..10_000).map(|i| pos2(i as f32, 0.0)).collect();
        assert!(triangulate(&big).is_empty());
        // a concave polygon (the folder's tab): n − 2 triangles
        let folder: Vec<Pos2> = [
            (2.5, 4.5),
            (9.0, 4.5),
            (11.0, 7.0),
            (21.5, 7.0),
            (21.5, 19.5),
            (2.5, 19.5),
        ]
        .iter()
        .map(|&(x, y)| pos2(x, y))
        .collect();
        assert_eq!(triangulate(&folder).len(), 4);
        assert!(round_pts(&[], 1.0, true).is_empty());
        assert_eq!(
            round_pts(&[(0.0, 0.0), (0.0, 0.0), (0.0, 0.0)], 1.0, true).len(),
            3
        );
        assert_eq!(arc_pts((0.0, 0.0), 1.0, 1.0, 0.0, 1e9).len(), 81);
    }
}
