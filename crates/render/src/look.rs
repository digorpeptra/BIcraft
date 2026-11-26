//! The look every BIcraft visual shares (docs/formats/visual-look.md): from a visual's Style
//! options (a preset, Modern, Minimal or Bold, and switches for rounded corners, gradients,
//! shadows and animation) and the report theme's colours, the colour tokens and the marks (bars,
//! tiles, cards, dots, value labels) as drawing primitives with their decorations
//! ([`crate::spec::Deco`]).
//!
//! The TypeScript twin is `crates/engine/src/visual_runtime/style.ts`; both build the same
//! primitives, which the engine's golden tests compare. Animation is the TypeScript's alone: the
//! renderer draws the last frame.

use serde_json::Value;

use crate::spec::{Deco, Prim, r2};

/// BIcraft's own categorical palette (no theme of the report's own, or `palette: bicraft`), in
/// its fixed order (indigo, vermilion, teal, amber, magenta, sky, violet, crimson): each colour
/// at least 3:1 on white and on a dark surface, adjacent pairs apart for colour-vision
/// deficiencies, the first three apart in every pair (checked with the dataviz method's
/// validator, docs/formats/visual-look.md).
pub const BICRAFT_PALETTE: [&str; 8] = [
    "#4A5BE8", "#EA5A28", "#0E9F84", "#BF8700", "#D6408E", "#1C8FCC", "#8A4FE0", "#E23D4F",
];
/// The first data colours of Power BI's stock themes (the default theme since 2020 and the
/// classic one): a report on one of them has no palette of its own, so Modern and Bold draw with
/// BIcraft's.
const STOCK_THEMES: [[&str; 8]; 2] = [
    [
        "#118DFF", "#12239E", "#E66C37", "#6B007B", "#E044A7", "#744EC2", "#D9B300", "#D64550",
    ],
    [
        "#01B8AA", "#374649", "#FD625E", "#F2C80F", "#5F6B6D", "#8AD4EB", "#FE9666", "#A66999",
    ],
];
/// The light card's fill behind the plot (5.9:1 under the axis text).
const CARD: &str = "#F7F6F3";
/// The palette of the 1.x visuals: the Minimal preset's when the report gives no theme.
pub const LEGACY_PALETTE: [&str; 8] = crate::spec::DEFAULT_PALETTE;
/// The presets, the first the default.
pub const PRESETS: [&str; 3] = ["modern", "minimal", "bold"];
/// Where the colours come from, the first the default.
pub const PALETTES: [&str; 2] = ["theme", "bicraft"];
/// Most theme colours read.
const MAX_THEME: usize = 16;
const WHITE: &str = "#FFFFFF";
const BLACK: &str = "#000000";

/// A preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    Modern,
    Minimal,
    Bold,
}

/// The colours, sizes and switches a visual draws with.
#[derive(Debug, Clone, PartialEq)]
pub struct Look {
    pub preset: Preset,
    pub rounded: bool,
    pub gradient: bool,
    pub shadow: bool,
    /// The entry animation (the TypeScript's; the renderer draws the last frame).
    pub motion: bool,
    /// The categorical colours, in order (the theme's or BIcraft's).
    pub palette: Vec<String>,
    /// Axis tick and category text (at least 4.5:1 on white).
    pub text: &'static str,
    /// Values and names.
    pub label: &'static str,
    pub grid: &'static str,
    pub axis: &'static str,
    /// The surface the marks sit on (gaps and rings): the card's, else the report's.
    pub surface: String,
    /// The dark ink of a label inside a fill.
    pub dark: &'static str,
    /// A cell without data.
    pub empty: &'static str,
    /// Qualitative bands, darkest first.
    pub shades: [&'static str; 5],
    /// The report's surface is dark (a dark theme): light inks on dark greys.
    pub night: bool,
    /// A rounded card behind the plot.
    pub card: bool,
    /// Value labels on a rounded chip.
    pub pills: bool,
    /// Sizes: axis ticks, names (categories, legends), value labels.
    pub tick: f64,
    pub name: f64,
    pub value: f64,
}

fn is_color(c: &str) -> bool {
    c.len() == 7 && c.starts_with('#') && c.bytes().skip(1).all(|b| b.is_ascii_hexdigit())
}

/// The theme's data colours (`{colors: ["#RRGGBB", …]}`), upper case, at most 16.
pub fn theme_colors(theme: Option<&Value>) -> Vec<String> {
    theme
        .and_then(|t| t.get("colors"))
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .filter_map(Value::as_str)
                .filter(|c| is_color(c))
                .take(MAX_THEME)
                .map(str::to_ascii_uppercase)
                .collect()
        })
        .unwrap_or_default()
}

/// The theme of a layout's input as `{colors}` from the renderer's theme colours.
pub fn theme_json(colors: &[String]) -> Value {
    serde_json::json!({"colors": colors})
}

/// The theme of a layout's input as `{colors, background}`: the renderer's theme colours and
/// the report's background (`#RRGGBB`).
pub fn theme_json_bg(colors: &[String], background: &str) -> Value {
    serde_json::json!({"colors": colors, "background": background})
}

/// Whether `colors` start with a stock Power BI theme's data colours.
fn is_stock(colors: &[String]) -> bool {
    STOCK_THEMES
        .iter()
        .any(|t| colors.len() >= t.len() && t.iter().zip(colors).all(|(a, b)| *a == b.as_str()))
}

/// The report's background (`theme.background`) when it is dark, upper case.
fn dark_background(theme: Option<&Value>) -> Option<String> {
    let bg = theme?.get("background")?.as_str()?;
    (is_color(bg) && lum(bg) < 0.07).then(|| bg.to_ascii_uppercase())
}

/// The Style options a look is made from (each may be missing or invalid: the preset's
/// default).
#[derive(Debug, Clone, Copy, Default)]
pub struct Options<'a> {
    pub preset: Option<&'a Value>,
    pub palette: Option<&'a Value>,
    pub rounded: Option<&'a Value>,
    pub gradient: Option<&'a Value>,
    pub shadow: Option<&'a Value>,
    pub motion: Option<&'a Value>,
    pub card: Option<&'a Value>,
    pub pills: Option<&'a Value>,
}

impl Look {
    /// A look from the Style options and the theme (`{colors, background?}`).
    pub fn new(o: Options<'_>, theme: Option<&Value>) -> Look {
        let p = match o.preset.and_then(Value::as_str) {
            Some("minimal") => Preset::Minimal,
            Some("bold") => Preset::Bold,
            _ => Preset::Modern,
        };
        let minimal = p == Preset::Minimal;
        let colors = theme_colors(theme);
        let own_asked = o.palette.and_then(Value::as_str) == Some("bicraft");
        // Modern and Bold take BIcraft's colours over a stock theme's
        let pal: Vec<String> = if own_asked || colors.is_empty() || (!minimal && is_stock(&colors)) {
            let l = if minimal && !own_asked {
                LEGACY_PALETTE
            } else {
                BICRAFT_PALETTE
            };
            l.iter().map(|c| (*c).to_string()).collect()
        } else {
            colors
        };
        let flag = |v: Option<&Value>, def: bool| v.and_then(Value::as_bool).unwrap_or(def);
        let card = flag(o.card, !minimal);
        let bg = dark_background(theme);
        let night = bg.is_some();
        let surface = match (card, bg) {
            (true, Some(b)) => mix(&b, WHITE, 0.06),
            (true, None) => CARD.to_string(),
            (false, Some(b)) => b,
            (false, None) => WHITE.to_string(),
        };
        let pick = |n: &'static str, m: &'static str, d: &'static str| {
            if night {
                n
            } else if minimal {
                m
            } else {
                d
            }
        };
        Look {
            preset: p,
            rounded: flag(o.rounded, !minimal),
            gradient: flag(o.gradient, !minimal),
            shadow: flag(o.shadow, !minimal),
            motion: flag(o.motion, !minimal),
            palette: pal,
            text: pick("#CFCEC4", "#555555", "#5F5E5A"),
            label: pick("#F0EFEC", "#333333", "#1F1E1C"),
            grid: pick("#3B3A37", "#E5E5E5", "#E8E7E2"),
            axis: pick("#5E5D58", "#B3B3B3", "#C8C7C0"),
            surface,
            dark: if minimal { "#252423" } else { BLACK },
            empty: pick("#34332F", "#EFEFEF", "#F0EFEB"),
            shades: if night {
                ["#6E6D68", "#5E5D59", "#4F4E4A", "#43423F", "#393835"]
            } else if minimal {
                ["#9E9E9E", "#BDBDBD", "#D3D3D3", "#E4E4E4", "#F0F0F0"]
            } else {
                ["#C4C3BC", "#D5D4CE", "#E2E1DC", "#ECEBE7", "#F5F4F1"]
            },
            night,
            card,
            pills: flag(o.pills, p == Preset::Bold),
            tick: if minimal { 10.0 } else { 11.0 },
            name: if minimal { 11.0 } else { 12.0 },
            value: match p {
                Preset::Minimal => 10.0,
                Preset::Modern => 11.0,
                Preset::Bold => 12.0,
            },
        }
    }

    /// The look of a visual whose Style options are an object `{preset, palette, rounded,
    /// gradient, shadow, animation, card, pills}`.
    pub fn of(style: Option<&Value>, theme: Option<&Value>) -> Look {
        let g = |k: &str| style.and_then(|s| s.get(k));
        Look::new(
            Options {
                preset: g("preset"),
                palette: g("palette"),
                rounded: g("rounded"),
                gradient: g("gradient"),
                shadow: g("shadow"),
                motion: g("animation"),
                card: g("card"),
                pills: g("pills"),
            },
            theme,
        )
    }

    /// The look of a chart pack visual, whose options are flat (`stylePreset`, `styleRounded`…).
    pub fn flat(o: &Value, theme: Option<&Value>) -> Look {
        Look::new(
            Options {
                preset: o.get("stylePreset"),
                palette: o.get("stylePalette"),
                rounded: o.get("styleRounded"),
                gradient: o.get("styleGradient"),
                shadow: o.get("styleShadow"),
                motion: o.get("styleAnimation"),
                card: o.get("styleCard"),
                pills: o.get("stylePills"),
            },
            theme,
        )
    }

    pub fn minimal(&self) -> bool {
        self.preset == Preset::Minimal
    }

    pub fn bold(&self) -> bool {
        self.preset == Preset::Bold
    }

    /// The `i`-th categorical colour.
    pub fn color(&self, i: usize) -> String {
        let n = self.palette.len().max(1);
        self.palette
            .get(i % n)
            .cloned()
            .unwrap_or_else(|| BICRAFT_PALETTE[0].to_string())
    }

    /// The corner radius of bars and boxes (0: square).
    pub fn radius(&self) -> f64 {
        match (self.rounded, self.bold()) {
            (false, _) => 0.0,
            (true, true) => 6.0,
            (true, false) => 4.0,
        }
    }

    /// The weight of value labels.
    pub fn value_weight(&self) -> u16 {
        match self.preset {
            Preset::Minimal => 400,
            Preset::Bold => 700,
            Preset::Modern => 600,
        }
    }

    /// The width of a connecting stem (a dumbbell's).
    pub fn stem_width(&self) -> f64 {
        match self.preset {
            Preset::Minimal => 2.0,
            Preset::Bold => 5.0,
            Preset::Modern => 4.0,
        }
    }

    /// The width of a data line (a slope's).
    pub fn line_width(&self) -> f64 {
        match self.preset {
            Preset::Minimal => 2.0,
            Preset::Bold => 3.0,
            Preset::Modern => 2.5,
        }
    }

    /// The ink of a label inside a fill (from `a` to `b` when it is a gradient): white or the
    /// look's dark, whichever reads better on both.
    pub fn on(&self, a: &str, b: &str) -> String {
        let w = contrast(WHITE, a).min(contrast(WHITE, b));
        let d = contrast(self.dark, a).min(contrast(self.dark, b));
        if w >= d { WHITE } else { self.dark }.to_string()
    }

    /// The lighter top colour of a fill drawn as a gradient (empty when gradients are off).
    pub fn grad_top(&self, fill: &str) -> String {
        if self.gradient {
            mix(fill, WHITE, if self.bold() { 0.32 } else { 0.22 })
        } else {
            String::new()
        }
    }

    /// A bar from a baseline: its corners rounded except on the side `sq` (`b`, `t`, `l`, `r`:
    /// the baseline's; empty rounds all four), a gradient, Bold's crisp edge and shadow.
    pub fn bar(&self, x: f64, y: f64, w: f64, h: f64, fill: &str, sq: &'static str) -> Prim {
        let r = self.radius().min(w / 2.0).min(h / 2.0);
        let bold = self.bold();
        rect_d(
            x,
            y,
            w,
            h,
            fill,
            1.0,
            Deco {
                rx: if r >= 0.5 { r } else { 0.0 },
                sq,
                grad: self.grad_top(fill),
                stroke: if bold {
                    mix(fill, BLACK, 0.22)
                } else {
                    String::new()
                },
                sw: 1.0,
                shadow: if bold && self.shadow { 3.0 } else { 0.0 },
                role: "",
            },
        )
    }

    /// A band behind a bar (a bullet's): no gradient, its corners rounded except on side `sq`
    /// (`x`: none rounded).
    pub fn band(&self, x: f64, y: f64, w: f64, h: f64, fill: &str, sq: &'static str) -> Prim {
        let r = if sq == "x" {
            0.0
        } else {
            self.radius().min(w / 2.0).min(h / 2.0)
        };
        rect_d(
            x,
            y,
            w,
            h,
            fill,
            1.0,
            Deco {
                rx: if r >= 0.5 { r } else { 0.0 },
                sq,
                ..Deco::default()
            },
        )
    }

    /// A box (a box plot's quartiles): translucent; Modern and Bold round it, shade it and draw
    /// its edge in its colour (Minimal draws the edge as a separate path).
    pub fn boxed(&self, x: f64, y: f64, w: f64, h: f64, fill: &str) -> Prim {
        if self.minimal() && !self.rounded && !self.gradient {
            return rect_d(x, y, w, h, fill, 0.3, Deco::default());
        }
        let r = self.radius().min(w / 2.0).min(h / 2.0);
        rect_d(
            x,
            y,
            w,
            h,
            fill,
            0.22,
            Deco {
                rx: if r >= 0.5 { r } else { 0.0 },
                grad: self.grad_top(fill),
                stroke: fill.to_string(),
                sw: if self.bold() { 2.0 } else { 1.5 },
                ..Deco::default()
            },
        )
    }

    /// A tile of a grid or a mosaic: all corners rounded up to `max`, a softer gradient when
    /// `soft`.
    #[allow(clippy::too_many_arguments)]
    pub fn tile(&self, x: f64, y: f64, w: f64, h: f64, fill: &str, max: f64, soft: bool) -> Prim {
        let r = if self.rounded {
            max.min(w / 2.0).min(h / 2.0)
        } else {
            0.0
        };
        rect_d(
            x,
            y,
            w,
            h,
            fill,
            1.0,
            Deco {
                rx: if r >= 0.5 { r } else { 0.0 },
                grad: if soft && self.gradient {
                    mix(fill, WHITE, 0.14)
                } else {
                    String::new()
                },
                ..Deco::default()
            },
        )
    }

    fn card_radius(&self) -> f64 {
        if self.bold() { 10.0 } else { 8.0 }
    }

    /// A card (an organization chart's node): rounded, its hairline edge, a gentle shadow.
    #[allow(clippy::too_many_arguments)]
    pub fn card(&self, x: f64, y: f64, w: f64, h: f64, fill: &str, edge: &str) -> Prim {
        let r = if self.rounded {
            self.card_radius().min(w / 2.0).min(h / 2.0)
        } else {
            0.0
        };
        rect_d(
            x,
            y,
            w,
            h,
            fill,
            1.0,
            Deco {
                rx: if r >= 0.5 { r } else { 0.0 },
                stroke: edge.to_string(),
                sw: 1.0,
                shadow: match (self.shadow, self.bold()) {
                    (false, _) => 0.0,
                    (true, true) => 8.0,
                    (true, false) => 6.0,
                },
                ..Deco::default()
            },
        )
    }

    /// An accent strip down a card's left edge, its left corners rounded as the card's.
    pub fn accent(&self, x: f64, y: f64, w: f64, h: f64, fill: &str) -> Prim {
        let r = if self.rounded {
            self.card_radius().min(w).min(h / 2.0)
        } else {
            0.0
        };
        rect_d(
            x,
            y,
            w,
            h,
            fill,
            1.0,
            Deco {
                rx: if r >= 0.5 { r } else { 0.0 },
                sq: "r",
                ..Deco::default()
            },
        )
    }

    /// A dot: Modern and Bold ring it with the surface so it reads where it crosses other marks.
    pub fn dot(&self, cx: f64, cy: f64, r: f64, fill: &str, opacity: f64) -> Prim {
        let ring = !self.minimal();
        circle_d(
            cx,
            cy,
            r,
            fill,
            opacity,
            Deco {
                stroke: if ring { self.surface.clone() } else { String::new() },
                sw: 1.5,
                ..Deco::default()
            },
        )
    }

    /// A large translucent disc (a Venn set): a gradient, its edge in its colour, a gentle shadow.
    pub fn blob(&self, cx: f64, cy: f64, r: f64, fill: &str, opacity: f64) -> Prim {
        let m = self.minimal();
        circle_d(
            cx,
            cy,
            r,
            fill,
            if m { opacity } else { opacity * 0.8 },
            Deco {
                grad: self.grad_top(fill),
                stroke: if m { String::new() } else { fill.to_string() },
                sw: if self.bold() { 2.0 } else { 1.5 },
                shadow: match (self.shadow, self.bold()) {
                    (false, _) => 0.0,
                    (true, true) => 5.0,
                    (true, false) => 4.0,
                },
                ..Deco::default()
            },
        )
    }

    /// A legend swatch.
    pub fn swatch(&self, x: f64, y: f64, s: f64, fill: &str) -> Prim {
        rect_d(
            x,
            y,
            s,
            s,
            fill,
            1.0,
            Deco {
                rx: if self.rounded { s / 2.0 - 2.0 } else { 0.0 },
                ..Deco::default()
            },
        )
    }

    /// The card behind a `w`×`h` visual's plot (`None` without one): a rounded surface with a
    /// hairline edge (`role: card`).
    pub fn card_of(&self, w: f64, h: f64) -> Option<Prim> {
        if !self.card || w < 24.0 || h < 24.0 {
            return None;
        }
        let edge = if self.night {
            mix(&self.surface, WHITE, 0.08)
        } else {
            "#E9E8E3".to_string()
        };
        let rx = match (self.rounded, self.bold()) {
            (false, _) => 0.0,
            (true, true) => 14.0,
            (true, false) => 12.0,
        };
        Some(rect_d(
            1.0,
            1.0,
            w - 2.0,
            h - 2.0,
            &self.surface,
            1.0,
            Deco {
                rx,
                stroke: edge,
                sw: 1.0,
                role: "card",
                ..Deco::default()
            },
        ))
    }

    /// Pushes the card of a `w`×`h` visual (when the look has one) onto `out`.
    pub fn push_card(&self, out: &mut Vec<Prim>, w: f64, h: f64) {
        if let Some(c) = self.card_of(w, h) {
            out.push(c);
        }
    }

    /// Pushes a value label for the point (`x`, `y`) of a mark onto `out`, `gap` pixels away on
    /// side `side` (`n` above, `s` below, `e` right, `w` left, `c` on it), at the look's value
    /// size and weight; with pills, on a rounded chip of the label ink (`role: pill`) in the ink
    /// that reads on it. Returns the index of the text.
    #[allow(clippy::too_many_arguments)]
    pub fn value_label(
        &self,
        out: &mut Vec<Prim>,
        x: f64,
        y: f64,
        s: String,
        side: char,
        gap: f64,
        size: f64,
    ) -> usize {
        let pad_x = if self.pills { 5.0 } else { 0.0 };
        let v = if side == 'n' || side == 's' {
            gap + (size - 10.0) / 2.0 + if self.pills { 3.0 } else { 0.0 }
        } else {
            0.0
        };
        let tx = match side {
            'e' => x + gap + pad_x,
            'w' => x - gap - pad_x,
            _ => x,
        };
        // Modern and Bold keep a label above its mark inside the visual
        let floor = if self.minimal() {
            f64::NEG_INFINITY
        } else {
            size / 2.0 + if self.pills { 3.0 } else { 0.0 } + 1.0
        };
        let ty = match side {
            'n' => (y - v).max(floor),
            's' => y + v,
            _ => y,
        };
        let anchor = match side {
            'e' => "start",
            'w' => "end",
            _ => "middle",
        };
        let mut ink = self.label.to_string();
        if self.pills {
            let wd = s.chars().count() as f64 * size * 0.55 + 10.0;
            let ht = size + 6.0;
            let px = match anchor {
                "start" => tx - 5.0,
                "end" => tx - wd + 5.0,
                _ => tx - wd / 2.0,
            };
            out.push(rect_d(
                px,
                ty - ht / 2.0,
                wd,
                ht,
                self.label,
                1.0,
                Deco {
                    rx: ht / 2.0,
                    role: "pill",
                    ..Deco::default()
                },
            ));
            ink = self.on(self.label, self.label);
        }
        out.push(text_w(tx, ty, s, size, &ink, anchor, 0.0, self.value_weight()));
        out.len() - 1
    }
}

/// A text primitive with its weight.
#[allow(clippy::too_many_arguments)]
pub fn text_w(
    x: f64,
    y: f64,
    s: String,
    size: f64,
    fill: &str,
    anchor: &'static str,
    rotate: f64,
    weight: u16,
) -> Prim {
    Prim::Text {
        x: r2(x),
        y: r2(y),
        text: s,
        size,
        fill: fill.to_string(),
        anchor,
        rotate,
        weight,
        font: crate::spec::TextFont::default(),
    }
}

/// How much of its opacity a mark keeps behind its highlighted part, or when another visual's
/// selection highlights none of its rows (docs/formats/bicraft-visuals.md "Highlights"; the
/// same as the selection's dimming).
pub const HIGHLIGHT_DIM: f64 = 0.4;

/// `p` drawn as the full value behind its highlighted part: a rectangle, circle or polygon at
/// [`HIGHLIGHT_DIM`] of its opacity (other primitives as they are).
pub fn dim(mut p: Prim) -> Prim {
    match &mut p {
        Prim::Rect { opacity, .. } | Prim::Circle { opacity, .. } | Prim::Poly { opacity, .. } => {
            *opacity = r2(*opacity * HIGHLIGHT_DIM);
        }
        _ => {}
    }
    p
}

/// The highlighted part of value `v` to draw over it (the data view's `highlights` entry `h`),
/// kept between 0 and `v`; `None` without a highlight or when nothing of `v` is highlighted.
pub fn part(v: f64, h: Option<f64>) -> Option<f64> {
    let h = h.filter(|x| x.is_finite())?;
    if !v.is_finite() {
        return None;
    }
    let p = if v >= 0.0 {
        h.clamp(0.0, v)
    } else {
        h.clamp(v, 0.0)
    };
    if p == 0.0 { None } else { Some(p) }
}

fn rect_d(x: f64, y: f64, w: f64, h: f64, fill: &str, opacity: f64, mut deco: Deco) -> Prim {
    deco.rx = r2(deco.rx);
    deco.sw = r2(deco.sw);
    Prim::Rect {
        x: r2(x),
        y: r2(y),
        w: r2(w),
        h: r2(h),
        fill: fill.to_string(),
        opacity,
        deco,
    }
}

fn circle_d(cx: f64, cy: f64, r: f64, fill: &str, opacity: f64, mut deco: Deco) -> Prim {
    deco.sw = r2(deco.sw);
    Prim::Circle {
        cx: r2(cx),
        cy: r2(cy),
        r: r2(r),
        fill: fill.to_string(),
        opacity,
        deco,
    }
}

fn hex(c: &str) -> (f64, f64, f64) {
    let p = |i: usize| {
        c.get(i..i + 2)
            .and_then(|h| u8::from_str_radix(h, 16).ok())
            .map_or(0.0, f64::from)
    };
    (p(1), p(3), p(5))
}

/// `a` moved towards `b` by `t` (0..1), each channel rounded half up.
pub fn mix(a: &str, b: &str, t: f64) -> String {
    let (a, b) = (hex(a), hex(b));
    let ch = |x: f64, y: f64| (x + (y - x) * t + 0.5).floor().clamp(0.0, 255.0) as u8;
    format!("#{:02X}{:02X}{:02X}", ch(a.0, b.0), ch(a.1, b.1), ch(a.2, b.2))
}

fn lum(c: &str) -> f64 {
    let (r, g, b) = hex(c);
    let f = |x: f64| {
        let s = x / 255.0;
        if s <= 0.03928 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b)
}

/// The WCAG contrast ratio of two colours.
pub fn contrast(a: &str, b: &str) -> f64 {
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn presets_and_switches() {
        let m = Look::of(None, None);
        assert_eq!(m.preset, Preset::Modern);
        assert!(m.rounded && m.gradient && m.shadow && m.motion);
        assert_eq!(m.palette[0], BICRAFT_PALETTE[0]);
        let min = Look::of(Some(&json!({"preset": "minimal"})), None);
        assert!(!min.rounded && !min.gradient && !min.shadow && !min.motion);
        assert_eq!(min.palette[0], LEGACY_PALETTE[0]);
        let mixed = Look::of(
            Some(&json!({"preset": "minimal", "rounded": true, "palette": "bicraft"})),
            None,
        );
        assert!(mixed.rounded && !mixed.gradient);
        assert_eq!(mixed.palette[0], BICRAFT_PALETTE[0]);
        // the theme wins unless BIcraft's palette is asked for; bad colours are dropped
        let theme = json!({"colors": ["#118dff", "red", 3, "#12239E"]});
        let t = Look::of(None, Some(&theme));
        assert_eq!(t.palette, vec!["#118DFF", "#12239E"]);
        assert_eq!(t.color(3), "#12239E");
        let own = Look::flat(
            &json!({"stylePalette": "bicraft", "stylePreset": "bold"}),
            Some(&theme),
        );
        assert_eq!(own.palette[0], BICRAFT_PALETTE[0]);
        assert_eq!(own.preset, Preset::Bold);
        assert_eq!(Look::of(Some(&json!({"preset": 7})), None).preset, Preset::Modern);
    }

    /// Text tokens read at 4.5:1 or more on white; labels inside any palette or theme colour too.
    #[test]
    fn text_contrast() {
        for k in [
            Look::of(None, None),
            Look::of(Some(&json!({"preset": "minimal"})), None),
        ] {
            assert!(contrast(k.text, WHITE) >= 4.5, "{}", k.text);
            assert!(contrast(k.label, WHITE) >= 4.5, "{}", k.label);
        }
        let k = Look::of(None, None);
        for c in BICRAFT_PALETTE
            .iter()
            .chain(LEGACY_PALETTE.iter())
            .chain(["#118DFF", "#12239E", "#E66C37", "#D9B300", "#FFFFFF", "#000000"].iter())
        {
            let ink = k.on(c, c);
            assert!(contrast(&ink, c) >= 4.5, "{ink} on {c}");
        }
    }

    /// A dark report background turns the look's text dark-surface aware (4.5:1 or more on the
    /// surface, with and without the card); a light one does not.
    #[test]
    fn dark_backgrounds_get_night_text() {
        for bg in ["#000000", "#1F1F1F", "#252423", "#333333", "#474747"] {
            let theme = theme_json_bg(&[], bg);
            for style in [json!({}), json!({"card": false}), json!({"preset": "bold"})] {
                let k = Look::of(Some(&style), Some(&theme));
                assert!(k.night, "{bg}");
                assert!(contrast(k.text, &k.surface) >= 4.5, "{} on {}", k.text, k.surface);
                assert!(
                    contrast(k.label, &k.surface) >= 4.5,
                    "{} on {}",
                    k.label,
                    k.surface
                );
            }
        }
        for bg in ["#FFFFFF", "#F3F2F1", "#808080"] {
            assert!(!Look::of(None, Some(&theme_json_bg(&[], bg))).night, "{bg}");
        }
    }

    /// Modern and Bold read Power BI's stock themes as no palette of the report's own (they
    /// draw BIcraft's); Minimal keeps them. The card and the value chips' defaults.
    #[test]
    fn stock_themes_card_and_chips() {
        let stock: Vec<String> = STOCK_THEMES[0].iter().map(|s| (*s).to_string()).collect();
        let theme = theme_json_bg(&stock, "#FFFFFF");
        assert_eq!(Look::of(None, Some(&theme)).palette[0], BICRAFT_PALETTE[0]);
        let min = Look::of(Some(&json!({"preset": "minimal"})), Some(&theme));
        assert_eq!(min.palette[0], STOCK_THEMES[0][0]);
        assert!(!min.card && !min.pills);
        let m = Look::of(None, None);
        assert!(m.card && !m.pills);
        let b = Look::of(Some(&json!({"preset": "bold"})), None);
        assert!(b.card && b.pills);
        let off = Look::of(
            Some(&json!({"preset": "bold", "card": false, "pills": false})),
            None,
        );
        assert!(!off.card && !off.pills);
        // the chip's ink reads on the chip
        let mut out = Vec::new();
        let t = b.value_label(&mut out, 50.0, 50.0, "12".into(), 'n', 6.0, b.value);
        let (chip, text) = (out[0].json(), out[t].json());
        assert_eq!(chip["role"], json!("pill"));
        assert!(contrast(text["fill"].as_str().unwrap(), chip["fill"].as_str().unwrap()) >= 4.5);
    }

    #[test]
    fn decorations_are_written_only_when_set() {
        let min = Look::of(Some(&json!({"preset": "minimal"})), None);
        assert_eq!(
            min.bar(1.0, 2.0, 3.0, 4.0, "#123456", "b").json(),
            json!({"t": "rect", "x": 1.0, "y": 2.0, "w": 3.0, "h": 4.0, "fill": "#123456", "opacity": 1.0})
        );
        let m = Look::of(None, None);
        let b = m.bar(0.0, 0.0, 20.0, 50.0, "#2A78D6", "b").json();
        assert_eq!(b["rx"], json!(4.0));
        assert_eq!(b["sq"], json!("b"));
        assert_eq!(b["grad"], json!(mix("#2A78D6", WHITE, 0.22)));
        assert!(b.get("stroke").is_none() && b.get("shadow").is_none());
        // a bar thinner than its radius is rounded to half its width
        assert_eq!(
            m.bar(0.0, 0.0, 3.0, 50.0, "#2A78D6", "b").json()["rx"],
            json!(1.5)
        );
        let mut out = Vec::new();
        let t = m.value_label(&mut out, 0.0, 0.0, "1".into(), 'c', 0.0, 10.0);
        assert_eq!(out[t].json()["weight"], json!(600));
        let mut out = Vec::new();
        let t = min.value_label(&mut out, 0.0, 0.0, "1".into(), 'c', 0.0, 10.0);
        assert!(out[t].json().get("weight").is_none());
    }
}
