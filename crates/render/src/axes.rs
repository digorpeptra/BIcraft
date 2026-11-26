//! Cartesian layout fitted to what Power BI Desktop 2.158 draws (measured on its report view's SVG;
//! docs/render.md "Cartesian charts"): value axis domains and ticks, category bands (inner and
//! outer padding, minimum category width, scrolling), category labels (flat, word-wrapped,
//! turned 35° or 90°, cut to the label room), data labels (position, display units, background,
//! per-series settings) and line styles per series.

use serde_json::Value;

use crate::canvas::{Anchor, Canvas, Dash, Rgb};
use crate::charts::{Ticks, fit, tw, tx, tx_rot};
use crate::data::{Axis, Ctx, Fmt, Series, clampf, fin, get, sk};
use crate::fmt::{ColFmt, fmt_label};
use crate::timeaxis::TimeAxis;

/// Outer padding of the category axis, in category widths (each side).
pub(crate) const OUTER_PAD: f32 = 0.4;
/// Default gap between bars, in category widths (`innerPadding` 20 %).
pub(crate) const INNER_PAD: f32 = 0.2;
/// A drawn bar is this share of its slot (Desktop leaves a hairline on each side).
const BAR_SLOT: f32 = 0.966;
/// Narrowest category before the chart scrolls (`preferredCategoryWidth`, px).
pub(crate) const MIN_CATEGORY: f32 = 20.0;
/// Most of the chart the category labels may take (`maxMarginFactor` 25 %).
pub(crate) const MAX_MARGIN: f32 = 0.25;
/// Thickness of the scroll bar of a chart with more categories than room.
pub(crate) const SCROLL: f32 = 9.0;

/// Text line height for a font size (Segoe UI: 14 px at 8 pt).
pub(crate) fn line_h(px: f32) -> f32 {
    px * 1.3125
}

/// d3's tick step for `span` split in about `m` intervals (1, 2, 5 × 10ⁿ). Ported from d3 v3.5.17
/// `d3_scale_linearTickRange` (`src/scale/linear.js`@9cc9a875e636a1dcf36cc1e07bdf77e1ad6e2c74),
/// BSD-3-Clause, Copyright (c) 2010-2016 Michael Bostock; see `crates/render/LICENSE-BSD-d3` and
/// THIRD_PARTY_NOTICES.md.
pub(crate) fn d3_step(span: f64, m: f64) -> f64 {
    if !(span.is_finite() && span > 0.0 && m > 0.0) {
        return 1.0;
    }
    let mut step = 10f64.powf((span / m).log10().floor());
    let err = m / span * step;
    if err <= 0.15 {
        step *= 10.0;
    } else if err <= 0.35 {
        step *= 5.0;
    } else if err <= 0.75 {
        step *= 2.0;
    }
    if step.is_finite() && step > 0.0 { step } else { 1.0 }
}

/// d3's tick increment for `span` split in about `m` intervals. Ported from d3-array v1.2.4
/// `tickIncrement` (`src/ticks.js`@fda02e084d50c880c695a3fe0e59cdf1a1ba5ea9), BSD-3-Clause,
/// Copyright 2010-2016 Mike Bostock; see `crates/render/LICENSE-BSD-d3-array` and
/// THIRD_PARTY_NOTICES.md.
pub(crate) fn d3_increment(span: f64, m: f64) -> f64 {
    if !(span.is_finite() && span > 0.0 && m > 0.0) {
        return 1.0;
    }
    let step = 10f64.powf((span / m).log10().floor());
    let e = span / m / step;
    let k = if e >= 50f64.sqrt() {
        10.0
    } else if e >= 10f64.sqrt() {
        5.0
    } else if e >= 2f64.sqrt() {
        2.0
    } else {
        1.0
    };
    let s = step * k;
    if s.is_finite() && s > 0.0 { s } else { 1.0 }
}

/// Desktop's value axis over the data range `lo..hi` drawn on `len` pixels.
///
/// BIcraft's fit to 15 value axes captured in Desktop 2.158 (docs/render.md "Cartesian charts"):
/// the domain is made nice with [`d3_step`] for 3, 5 or 8 intervals by
/// axis length (a vertical axis under 150 / 300 px, a horizontal one under 300 / 500 px); the
/// ticks are [`d3_increment`] for 2, 3 or 5 intervals of that domain. The axis' own start and end
/// (`ax.min` / `ax.max`) replace the domain ends; log axes keep [`crate::charts::value_scale`].
pub(crate) fn scale(lo: f64, hi: f64, len: f32, vertical: bool, _px: f32, ax: Option<&Axis>) -> Ticks {
    if let Some(a) = ax
        && a.log
    {
        return crate::charts::value_scale(lo, hi, 4, ax);
    }
    let (mut lo, mut hi) = if lo.is_finite() && hi.is_finite() {
        (lo.min(hi), hi.max(lo))
    } else {
        (0.0, 1.0)
    };
    if let Some(a) = ax {
        if let Some(m) = a.min {
            lo = m;
        }
        if let Some(m) = a.max {
            hi = m;
        }
        if hi <= lo {
            hi = lo + 1.0;
        }
    }
    if hi == lo {
        if hi == 0.0 {
            hi = 1.0;
        } else if hi > 0.0 {
            lo = 0.0;
        } else {
            hi = 0.0;
        }
    }
    let len = len.max(1.0);
    let m = if vertical {
        if len < 150.0 {
            3.0
        } else if len < 300.0 {
            5.0
        } else {
            8.0
        }
    } else if len < 300.0 {
        3.0
    } else if len < 500.0 {
        5.0
    } else {
        8.0
    };
    let sd = d3_step(hi - lo, m);
    let a = if ax.and_then(|a| a.min).is_some() {
        lo
    } else {
        (lo / sd + 1e-9).floor() * sd
    };
    let b = if ax.and_then(|a| a.max).is_some() {
        hi
    } else {
        (hi / sd - 1e-9).ceil() * sd
    };
    let b = if b > a { b } else { a + sd };
    let mt = if m < 4.0 {
        2.0
    } else if m < 6.0 {
        3.0
    } else {
        5.0
    };
    let st = d3_increment(b - a, mt).max(sd);
    let mut ticks = Vec::new();
    let first = (a / st - 1e-9).ceil() * st;
    for i in 0..=60 {
        let t = first + i as f64 * st;
        if t > b + st * 1e-6 {
            break;
        }
        ticks.push(if t.abs() < st * 1e-9 { 0.0 } else { t });
    }
    Ticks {
        lo: a,
        hi: b,
        step: st,
        ticks,
        base: 0f64.clamp(a, b),
        log: None,
    }
}

/// The display unit an axis writes its ticks in (`axis_fmt_units`'s rule): 1, 1e3, 1e6…
pub(crate) fn axis_unit(tk: &Ticks, units: Option<f64>) -> f64 {
    let m = tk.lo.abs().max(tk.hi.abs());
    match units {
        Some(x) if x >= 1e12 => 1e12,
        Some(x) if x >= 1e9 => 1e9,
        Some(x) if x >= 1e6 => 1e6,
        Some(x) if x >= 1e3 => 1e3,
        Some(x) if x >= 1.0 => 1.0,
        _ if m >= 1e11 => 1e12,
        _ if m >= 1e8 => 1e9,
        _ if m >= 1e5 => 1e6,
        _ if m >= 1e4 => 1e3,
        _ => 1.0,
    }
}

// --- category bands --------------------------------------------------------------------------

/// Categories along an axis: d3 v3's `rangeBands(len, inner, 0.4)` (ported from
/// `src/scale/ordinal.js`@9cc9a875e636a1dcf36cc1e07bdf77e1ad6e2c74, BSD-3-Clause, Copyright (c)
/// 2010-2016 Michael Bostock; see `crates/render/LICENSE-BSD-d3`), with a minimum category width
/// beyond which the chart shows the first categories and a scroll bar; or, on a continuous date
/// axis, each category at its date ([`Bands::time`]).
#[derive(Debug, Clone)]
pub(crate) struct Bands {
    /// Categories drawn.
    pub n: usize,
    /// All categories.
    pub total: usize,
    /// The distance between two categories (on a time axis: the smallest one).
    pub step: f32,
    /// Width of a category's bars.
    pub bar: f32,
    /// Offset of the first bar from the start of the axis.
    first: f32,
    /// Offset of a category's centre from its first bar.
    half: f32,
    /// On a time axis, each category's first bar offset (empty: evenly spaced).
    at: Vec<f32>,
}

impl Bands {
    pub(crate) fn new(total: usize, len: f32, inner: f32, min_w: f32) -> Bands {
        let inner = clampf(inner, 0.0, 0.9);
        let total = total.max(1);
        let pad = 2.0 * OUTER_PAD - inner;
        let mut n = total;
        let mut step = len.max(1.0) / (n as f32 + pad);
        if step < min_w && n > 1 {
            // Desktop 2.158 keeps one category fewer than fit (572 px: 27 at 20 px, not 28)
            n = (((len / min_w - pad).floor() - 1.0).max(1.0) as usize).min(total);
            step = len.max(1.0) / (n as f32 + pad);
        }
        Bands {
            n,
            total,
            step,
            bar: step * (1.0 - inner) * BAR_SLOT,
            first: OUTER_PAD * step,
            half: step * (1.0 - inner) / 2.0,
            at: Vec::new(),
        }
    }
    /// Columns over a continuous date axis: each at its date, the first and last half a pixel
    /// inside the ends, bars as wide as the smallest gap less the inner padding
    /// ([`TimeAxis::bars`]); never scrolls.
    pub(crate) fn time(ta: &TimeAxis, len: f32, inner: f32) -> Bands {
        let (s, bar) = ta.bars(len, clampf(inner, 0.0, 0.9));
        let at: Vec<f32> = ta.t.iter().map(|t| 0.5 + ((t - ta.lo) * s) as f32).collect();
        let total = at.len().max(1);
        Bands {
            n: total,
            total,
            step: ((ta.gap * s) as f32).max(0.1),
            bar,
            first: 0.5,
            half: bar / 2.0,
            at,
        }
    }
    /// Offset of category `i`'s first bar from the start of the axis.
    pub(crate) fn x(&self, i: usize) -> f32 {
        self.at
            .get(i)
            .copied()
            .unwrap_or(self.first + i as f32 * self.step)
    }
    /// Centre of category `i` (its label).
    pub(crate) fn mid(&self, i: usize) -> f32 {
        self.x(i) + self.half
    }
    pub(crate) fn scrolls(&self) -> bool {
        self.n < self.total
    }
}

/// The category gap of a chart (`categoryAxis.innerPadding`, default 20 %).
pub(crate) fn inner_pad(f: &Fmt) -> f32 {
    f.x.inner_padding.unwrap_or(INNER_PAD)
}

/// Minimum category width (`preferredCategoryWidth`, default 20 px).
pub(crate) fn min_cat(f: &Fmt) -> f32 {
    f.x.category_width.unwrap_or(MIN_CATEGORY)
}

/// The scroll bar: a light track along the plot and a darker thumb for the categories shown.
pub(crate) fn scroll_bar(cv: &mut Canvas, horiz: bool, a0: f32, a1: f32, at: f32, b: &Bands) {
    let frac = b.n as f32 / b.total.max(1) as f32;
    let len = (a1 - a0).max(0.0);
    if horiz {
        cv.rect(at, a0, SCROLL, len, Rgb(0xf4, 0xf4, 0xf4), 1.0);
        cv.rect(at, a0, SCROLL, len * frac, Rgb(0xc4, 0xc4, 0xc4), 1.0);
    } else {
        cv.rect(a0, at, len, SCROLL, Rgb(0xf4, 0xf4, 0xf4), 1.0);
        cv.rect(a0, at, len * frac, SCROLL, Rgb(0xc4, 0xc4, 0xc4), 1.0);
    }
}

// --- category labels -------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CatMode {
    Flat,
    /// Word-wrapped, at most this many lines.
    Wrap(Vec<Vec<String>>),
    /// Turned by this many degrees (35 or 90), cut to `len` pixels.
    Turn(f32),
}

/// Labels under a vertical chart's categories.
#[derive(Debug, Clone)]
pub(crate) struct CatLabels {
    pub mode: CatMode,
    /// Room under the plot.
    pub h: f32,
    /// Longest label (turned labels).
    pub len: f32,
    pub px: f32,
}

fn words_wrap(ctx: &Ctx<'_>, s: &str, w: f32, px: f32) -> Option<Vec<String>> {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in s.split_whitespace().take(40) {
        if tw(ctx, word, px) > w {
            return None;
        }
        let cand = if cur.is_empty() {
            word.to_string()
        } else {
            format!("{cur} {word}")
        };
        if tw(ctx, &cand, px) <= w || cur.is_empty() {
            cur = cand;
        } else {
            lines.push(std::mem::take(&mut cur));
            cur = word.to_string();
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    Some(lines)
}

/// How Desktop lays out category labels under columns: flat when every label fits its
/// category, else word-wrapped when every word fits (within the label room), else turned 35°
/// (90° when categories are narrower than two label heights), cut with an ellipsis to the label
/// room (`maxMarginFactor` of the chart height).
pub(crate) fn cat_labels(
    ctx: &Ctx<'_>,
    cats: &[String],
    step: f32,
    px: f32,
    chart_h: f32,
    f: &Fmt,
) -> CatLabels {
    let lh = line_h(px);
    // a quarter of the chart (Desktop 2.158 keeps 25 % even when a theme's `maxMarginFactor`
    // says 50)
    let room = (MAX_MARGIN * chart_h).max(lh);
    let widest = cats
        .iter()
        .take(400)
        .map(|c| tw(ctx, c, px))
        .fold(0.0f32, f32::max);
    let flat_h = 2.8 + lh + 5.7;
    // labels keep a gap between them (Desktop 2.158: 43.6 px labels stay flat on 54 px
    // categories and turn on 47 px ones)
    let fit_w = step - px * 0.4;
    if widest <= fit_w {
        return CatLabels {
            mode: CatMode::Flat,
            h: flat_h,
            len: widest,
            px,
        };
    }
    let max_lines = ((room / lh).floor() as usize).clamp(1, 4);
    let wrapped: Option<Vec<Vec<String>>> = cats
        .iter()
        .take(400)
        .map(|c| words_wrap(ctx, c, fit_w, px).filter(|l| l.len() <= max_lines))
        .collect();
    if let Some(w) = wrapped {
        let lines = w.iter().map(Vec::len).max().unwrap_or(1).max(1);
        return CatLabels {
            mode: CatMode::Wrap(w),
            h: flat_h + (lines - 1) as f32 * (lh + 5.0),
            len: widest,
            px,
        };
    }
    // 90° on 20 px categories at 8 pt, still 35° on 24.2 px ones at 9 pt (Desktop 2.158): the
    // switch lies between 1.88 and 2.02 font sizes. A scrolling chart decides on its preferred
    // category width (20 px), not on the slightly wider step it lays the bands out with (a
    // 28-product chart at 8 pt turns its labels 90°).
    let min_w = min_cat(f);
    let w = if step < min_w * 1.1 { step.min(min_w) } else { step };
    let deg = if w < px * 1.9 { 90.0f32 } else { 35.0 };
    let (s, c) = deg.to_radians().sin_cos();
    // the turned label's height: its length times sin, plus its own height times cos
    let len = widest.min(((room - lh * c) / s).max(lh));
    CatLabels {
        mode: CatMode::Turn(deg),
        // Desktop reserves the whole label room for turned labels (2.5 px more at 35°, 14.5 at 90°)
        h: room + if deg >= 90.0 { 14.5 } else { 2.5 },
        len,
        px,
    }
}

/// The flat labels of a time axis' ticks, the widest `widest` px.
pub(crate) fn tick_labels(px: f32, widest: f32) -> CatLabels {
    CatLabels {
        mode: CatMode::Flat,
        h: 2.8 + line_h(px) + 5.7,
        len: widest,
        px,
    }
}

#[allow(clippy::too_many_arguments)]
/// Draws [`cat_labels`] under the plot (`bottom`), category `i` centred at `x_of(i)`.
pub(crate) fn draw_cat_labels(
    cv: &mut Canvas,
    ctx: &Ctx<'_>,
    cl: &CatLabels,
    cats: &[String],
    n: usize,
    step: f32,
    color: Rgb,
    x_of: &dyn Fn(usize) -> f32,
    bottom: f32,
) {
    let px = cl.px;
    let lh = line_h(px);
    for i in 0..n.min(cats.len()) {
        let c = cats.get(i).map(String::as_str).unwrap_or("");
        let x = x_of(i);
        match &cl.mode {
            CatMode::Flat => {
                let t = fit(ctx, c, step.max(px), px);
                tx(cv, ctx, x, bottom + 2.8 + lh / 2.0, &t, px, color, Anchor::Middle);
            }
            CatMode::Wrap(all) => {
                let lines = all.get(i).cloned().unwrap_or_default();
                let k = lines.len().max(1) as f32;
                let top = bottom + 2.8 + lh / 2.0 + (k - 1.0) * 7.7 - (k - 1.0) * lh / 2.0;
                for (j, l) in lines.iter().enumerate() {
                    tx(cv, ctx, x, top + j as f32 * lh, l, px, color, Anchor::Middle);
                }
            }
            CatMode::Turn(deg) => {
                let t = fit(ctx, c, cl.len, px);
                if *deg >= 90.0 {
                    tx_rot(cv, ctx, x, bottom + 6.0, &t, px, color, Anchor::End, -90.0);
                } else {
                    tx_rot(cv, ctx, x, bottom + lh * 0.45, &t, px, color, Anchor::End, -deg);
                }
            }
        }
    }
}

/// How far left of its category centre the first turned label reaches (35° labels hang to the
/// left; Desktop moves the plot right so that it stays inside the visual).
pub(crate) fn first_label_reach(ctx: &Ctx<'_>, cl: &CatLabels, first: &str) -> f32 {
    match cl.mode {
        CatMode::Turn(deg) if deg < 90.0 => {
            let lh = line_h(cl.px);
            let (s, c) = deg.to_radians().sin_cos();
            let w = tw(ctx, first, cl.px).min(cl.len);
            w * c + lh * 0.5 * s + 8.0
        }
        _ => 0.0,
    }
}

// --- data labels -----------------------------------------------------------------------------

/// Where a data label goes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum LabelPos {
    OutsideEnd,
    InsideEnd,
    InsideCenter,
    InsideBase,
}

/// One series' data label settings: the report's `labels`, then its `series_labels` entry.
#[derive(Debug, Clone)]
pub(crate) struct LabelStyle {
    pub show: bool,
    pub pos: Option<LabelPos>,
    pub color: Option<Rgb>,
    pub units: Option<f64>,
    pub precision: Option<usize>,
    pub background: Option<(Rgb, f32)>,
    pub px: Option<f32>,
}

fn merge_label(st: &mut LabelStyle, l: &Value) {
    if let Some(b) = get(l, "show").as_bool() {
        st.show = b;
    }
    let pos = sk(l, "position").to_ascii_lowercase();
    match pos.as_str() {
        "outsideend" | "outside" | "above" => st.pos = Some(LabelPos::OutsideEnd),
        "insideend" => st.pos = Some(LabelPos::InsideEnd),
        "insidecenter" | "inside" => st.pos = Some(LabelPos::InsideCenter),
        "insidebase" | "under" => st.pos = Some(LabelPos::InsideBase),
        "auto" => st.pos = None,
        _ => {}
    }
    if let Some(c) = Rgb::from_value(get(l, "color")) {
        st.color = Some(c);
    }
    if let Some(u) = fin(get(l, "display_units")) {
        st.units = (u > 0.0).then_some(u);
    }
    if let Some(p) = fin(get(l, "precision")) {
        st.precision = Some(p.clamp(0.0, 10.0) as usize);
    }
    if let Some(p) = fin(get(l, "font_size")) {
        st.px = Some(clampf(p as f32, 6.0, 40.0) * crate::data::PT);
    }
    if let Some(on) = get(l, "background").as_bool() {
        if on {
            let c = Rgb::from_value(get(l, "background_color"))
                .or(st.background.map(|b| b.0))
                .unwrap_or(Rgb(0xff, 0xff, 0xff));
            let t = fin(get(l, "background_transparency")).map_or(0.0, |t| t.clamp(0.0, 100.0)) as f32;
            st.background = Some((c, 1.0 - t / 100.0));
        } else {
            st.background = None;
        }
    }
}

/// The label style of every series (`format.labels`, `format.series_labels` by data column).
pub(crate) fn label_styles(f: &Fmt, ser: &[&Series]) -> Vec<LabelStyle> {
    let mut base = LabelStyle {
        show: f.labels == Some(true),
        pos: None,
        color: None,
        units: None,
        precision: None,
        background: None,
        px: None,
    };
    let l = get(&f.raw, "labels");
    merge_label(&mut base, l);
    if let Some(b) = f.labels {
        base.show = b;
    }
    let per = crate::data::arr(get(&f.raw, "series_labels"));
    ser.iter()
        .map(|s| {
            let mut st = base.clone();
            for e in per.iter().take(60) {
                let col = get(e, "column").as_u64().map(|c| c as usize);
                if col.is_some() && col == s.j {
                    merge_label(&mut st, e);
                }
            }
            st
        })
        .collect()
}

/// A data label's text: the series' display units and precision, else (auto) the value axis'
/// unit with two decimals, else the measure's format string.
pub(crate) fn label_text(x: f64, col: &ColFmt, st: &LabelStyle, axis_unit: f64, ctx: &Ctx<'_>) -> String {
    match (st.units, st.precision) {
        (None, None) if axis_unit > 1.0 => fmt_label(x, Some(col), &ctx.loc, Some(axis_unit), Some(2)),
        (None, None) => fmt_label(x, Some(col), &ctx.loc, Some(1.0), None),
        (u, p) => fmt_label(x, Some(col), &ctx.loc, u.or(Some(axis_unit)), p),
    }
}

/// A placed data label, drawn after the bars.
#[derive(Debug, Clone)]
pub(crate) struct Label {
    pub x: f32,
    pub y: f32,
    pub text: String,
    pub color: Rgb,
    pub anchor: Anchor,
    pub bg: Option<(Rgb, f32)>,
    pub px: f32,
}

/// An inside label's colour: the report's when it reads on the bar, else white or black (Desktop
/// keeps inside labels legible over a theme's grey).
pub(crate) fn inside_color(set: Option<Rgb>, bar: Rgb) -> Rgb {
    match set {
        Some(c) if (c.lum() - bar.lum()).abs() >= 0.35 => c,
        _ => bar.on(),
    }
}

pub(crate) fn draw_labels(cv: &mut Canvas, ctx: &Ctx<'_>, labels: &[Label]) {
    for l in labels {
        if let Some((c, a)) = l.bg {
            let w = tw(ctx, &l.text, l.px) + 16.0;
            let h = line_h(l.px) + 2.4;
            let x0 = match l.anchor {
                Anchor::Start => l.x - 8.0,
                Anchor::End => l.x - w + 8.0,
                Anchor::Middle => l.x - w / 2.0,
            };
            cv.round_rect(x0, l.y - h / 2.0, w, h, 3.0, Some((c, a)), None);
        }
        tx(cv, ctx, l.x, l.y, &l.text, l.px, l.color, l.anchor);
    }
}

// --- line styles -----------------------------------------------------------------------------

/// A line series' stroke and markers (`format.lines`, then its `series_lines` entry).
#[derive(Debug, Clone, Copy)]
pub(crate) struct LineStyle {
    pub width: f32,
    pub dash: Dash,
    pub marker: Option<f32>,
    pub marker_color: Option<Rgb>,
    pub show: bool,
}

fn merge_line(st: &mut LineStyle, l: &Value, style: &mut String) {
    if let Some(w) = fin(get(l, "width")) {
        st.width = clampf(w as f32, 0.5, 10.0);
    }
    if !sk(l, "style").is_empty() {
        *style = sk(l, "style").to_ascii_lowercase();
    }
    if let Some(b) = get(l, "show_marker").as_bool() {
        st.marker = b.then_some(st.marker.unwrap_or(0.0));
    }
    if let Some(s) = fin(get(l, "marker_size"))
        && st.marker.is_some()
    {
        st.marker = Some(clampf(s as f32, 1.0, 20.0));
    }
    if let Some(c) = Rgb::from_value(get(l, "marker_color")) {
        st.marker_color = Some(c);
    }
    if let Some(b) = get(l, "show").as_bool() {
        st.show = b;
    }
}

/// The line style of every series.
pub(crate) fn line_styles(f: &Fmt, ser: &[&Series]) -> Vec<LineStyle> {
    let mut base = LineStyle {
        width: 2.0,
        dash: None,
        marker: None,
        marker_color: None,
        show: true,
    };
    let mut style = String::new();
    merge_line(&mut base, get(&f.raw, "lines"), &mut style);
    let per = crate::data::arr(get(&f.raw, "series_lines"));
    ser.iter()
        .map(|s| {
            let mut st = base;
            let mut sty = style.clone();
            for e in per.iter().take(60) {
                let col = get(e, "column").as_u64().map(|c| c as usize);
                if col.is_some() && col == s.j {
                    merge_line(&mut st, e, &mut sty);
                }
            }
            let w = st.width;
            st.dash = match sty.as_str() {
                "dashed" => Some((w * 3.0, w * 2.0)),
                "dotted" => Some((w * 0.01, w * 2.0)),
                _ => None,
            };
            // Desktop's default marker: 5 px (`markerSize`), a circle of that radius
            st.marker = st.marker.map(|m| if m <= 0.0 { 5.0 } else { m });
            st
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn d3_steps_follow_one_two_five() {
        assert_eq!(d3_step(299_600.0, 5.0), 50_000.0);
        assert_eq!(d3_step(209_700.0, 3.0), 100_000.0);
        assert_eq!(d3_step(1230.0, 5.0), 200.0);
        assert_eq!(d3_step(16.0, 5.0), 5.0);
        assert_eq!(d3_step(0.0, 5.0), 1.0);
        assert_eq!(d3_step(f64::NAN, 5.0), 1.0);
        assert_eq!(d3_increment(250_000.0, 3.0), 100_000.0);
        assert_eq!(d3_increment(200_000.0, 3.0), 50_000.0);
    }

    #[test]
    fn value_axes_match_desktop() {
        // measured on Desktop 2.158 (synthetic report)
        let px = 32.0 / 3.0;
        let t = scale(0.0, 155_800.0, 272.0, true, px, None);
        assert_eq!((t.hi, t.step), (200_000.0, 50_000.0));
        let t = scale(0.0, 224_700.0, 272.0, true, px, None);
        assert_eq!((t.hi, t.step), (250_000.0, 100_000.0));
        assert_eq!(t.ticks, vec![0.0, 100_000.0, 200_000.0]);
        let t = scale(0.0, 209_700.0, 142.0, true, px, None);
        assert_eq!((t.hi, t.step), (300_000.0, 200_000.0));
        let t = scale(0.0, 173_800.0, 142.0, true, px, None);
        assert_eq!((t.hi, t.step), (200_000.0, 100_000.0));
        let t = scale(0.0, 197_700.0, 231.0, false, px, None);
        assert_eq!((t.hi, t.step), (200_000.0, 100_000.0));
        let t = scale(0.0, 299_600.0, 272.0, true, px, None);
        assert_eq!((t.hi, t.step), (300_000.0, 100_000.0));
        let t = scale(0.0, 35_200.0, 195.0, true, px, None);
        assert_eq!((t.hi, t.step), (40_000.0, 10_000.0));
        let t = scale(0.0, 1230.0, 202.0, true, px, None);
        assert_eq!((t.hi, t.step), (1400.0, 500.0));
        let t = scale(0.0, 35_200.0, 181.0, false, px, None);
        assert_eq!((t.hi, t.step), (40_000.0, 20_000.0));
        // a line axis over 45.3K..78.5K: 45K..80K every 10K
        let t = scale(45_300.0, 78_500.0, 245.0, true, px, None);
        assert_eq!((t.lo, t.hi, t.step), (45_000.0, 80_000.0, 10_000.0));
        // hostile input
        let t = scale(f64::NAN, f64::INFINITY, 0.0, true, px, None);
        assert!(t.hi > t.lo && !t.ticks.is_empty());
        let t = scale(5.0, 5.0, 100.0, true, px, None);
        assert!(t.hi > t.lo);
    }

    #[test]
    fn bands_scroll_at_the_minimum_width() {
        let b = Bands::new(5, 356.0, 0.2, 20.0);
        assert!((b.step - 63.57).abs() < 0.01);
        assert!((b.x(0) - 25.43).abs() < 0.01);
        assert!((b.bar - 49.1).abs() < 0.1);
        let b = Bands::new(36, 572.0, 0.2, 20.0);
        assert_eq!(b.n, 27);
        assert!(b.scrolls());
        assert!((b.step - 20.72).abs() < 0.01);
        assert_eq!(Bands::new(36, 274.0, 0.2, 20.0).n, 12);
        let b = Bands::new(0, -5.0, 9.0, 0.0);
        assert!(b.step.is_finite() && b.n >= 1);
    }
}
