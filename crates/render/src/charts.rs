//! Charts: bar/column (clustered, stacked, 100 %), line/area (stacked, 100 %), combo, pie/donut,
//! gauge, KPI, funnel, waterfall, treemap, scatter. Ported from the live viewer's SVG drawing
//! (`crates/view/src/page.html`), drawn straight onto the canvas.

use serde_json::Value;

use crate::canvas::{Anchor, Base, Canvas, Dash, Rgb, Text, poly};
use crate::data::{
    BAD, ChartData, Ctx, Dataset, Fmt, Rc, Series, chart_data, clampf, cmp_raw, fin, key_of, nk, sk,
};
use crate::fmt::{ColFmt, fmt_cell, fmt_val};
use crate::fonts::Font;
use crate::hits;
use crate::timeaxis::TimeAxis;

/// A data label with the report's label display units and precision (`format.labels`).
pub(crate) fn lbl(x: f64, col: Option<&ColFmt>, f: &Fmt, ctx: &Ctx<'_>) -> String {
    let l = crate::data::get(&f.raw, "labels");
    crate::fmt::fmt_label(
        x,
        col,
        &ctx.loc,
        fin(crate::data::get(l, "display_units")),
        fin(crate::data::get(l, "precision")).map(|p| p.clamp(0.0, 10.0) as usize),
    )
}

/// A box in the canvas (the visual body).
#[derive(Debug, Clone, Copy)]
pub struct Bx {
    pub w: f32,
    pub h: f32,
}

pub fn font(ctx: &Ctx<'_>, px: f32) -> Font {
    Font::new(&ctx.font, px)
}
pub fn tw(ctx: &Ctx<'_>, s: &str, px: f32) -> f32 {
    ctx.fonts.measure(&font(ctx, px), s)
}
pub fn fit(ctx: &Ctx<'_>, s: &str, w: f32, px: f32) -> String {
    ctx.fonts.fit(&font(ctx, px), s, w.max(0.0))
}

/// A text label in the chart font.
#[allow(clippy::too_many_arguments)]
pub fn tx(
    cv: &mut Canvas,
    ctx: &Ctx<'_>,
    x: f32,
    y: f32,
    s: &str,
    px: f32,
    color: Rgb,
    anchor: Anchor,
) -> f32 {
    cv.text(
        ctx.fonts,
        &Text::new(s, font(ctx, px), color)
            .anchor(anchor)
            .base(Base::Central),
        x,
        y,
    )
}
pub(crate) fn tx_w(
    cv: &mut Canvas,
    ctx: &Ctx<'_>,
    x: f32,
    y: f32,
    s: &str,
    px: f32,
    color: Rgb,
    anchor: Anchor,
    weight: u16,
) {
    cv.text(
        ctx.fonts,
        &Text::new(s, font(ctx, px).weight(weight), color)
            .anchor(anchor)
            .base(Base::Central),
        x,
        y,
    );
}
pub(crate) fn tx_rot(
    cv: &mut Canvas,
    ctx: &Ctx<'_>,
    x: f32,
    y: f32,
    s: &str,
    px: f32,
    color: Rgb,
    anchor: Anchor,
    rot: f32,
) {
    cv.text(
        ctx.fonts,
        &Text::new(s, font(ctx, px), color)
            .anchor(anchor)
            .base(Base::Central)
            .rotate(rot),
        x,
        y,
    );
}

/// Gridlines: solid 1 px lines (Desktop 2.158).
pub(crate) const GRID_DASH: Dash = None;

// --- axes and scales -------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Ticks {
    pub lo: f64,
    pub hi: f64,
    pub step: f64,
    pub ticks: Vec<f64>,
    pub base: f64,
    pub log: Option<(f64, f64)>,
}

impl Ticks {
    /// A value's position on the axis, 0..1.
    pub fn f(&self, x: f64) -> f32 {
        if let Some((a, b)) = self.log {
            return if x > 0.0 {
                (((x.log10() - a) / (b - a)).clamp(0.0, 1.0)) as f32
            } else {
                0.0
            };
        }
        let span = if self.hi - self.lo == 0.0 {
            1.0
        } else {
            self.hi - self.lo
        };
        ((x.clamp(self.lo.min(self.hi), self.hi.max(self.lo)) - self.lo) / span) as f32
    }
}

pub fn nice_ticks(lo: f64, hi: f64, n: usize) -> Ticks {
    let (mut lo, mut hi) = if lo.is_finite() && hi.is_finite() {
        (lo, hi)
    } else {
        (0.0, 1.0)
    };
    if hi < lo {
        std::mem::swap(&mut lo, &mut hi);
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
    let raw = (hi - lo) / n.max(1) as f64;
    let p = 10f64.powf(raw.log10().floor());
    let m = raw / p;
    let step = (if m <= 1.0 {
        1.0
    } else if m <= 2.0 {
        2.0
    } else if m <= 2.5 {
        2.5
    } else if m <= 5.0 {
        5.0
    } else {
        10.0
    }) * p;
    let step = if step.is_finite() && step > 0.0 { step } else { 1.0 };
    let a = (lo / step + 1e-9).floor() * step;
    let b = (hi / step - 1e-9).ceil() * step;
    let mut ticks = Vec::new();
    for i in 0..=60 {
        let t = a + i as f64 * step;
        if t > b + step * 1e-6 {
            break;
        }
        ticks.push(if t.abs() < step * 1e-9 { 0.0 } else { t });
    }
    Ticks {
        lo: a,
        hi: if b > a { b } else { a + step },
        step,
        ticks,
        base: 0.0,
        log: None,
    }
}

/// Nice ticks, the axis' explicit min/max, log scale.
pub fn value_scale(lo: f64, hi: f64, n: usize, ax: Option<&crate::data::Axis>) -> Ticks {
    if let Some(ax) = ax
        && ax.log
        && hi > 0.0
    {
        let pos = if lo > 0.0 { lo } else { hi / 1000.0 };
        let mut a = ax.min.filter(|m| *m > 0.0).unwrap_or(pos).log10().floor();
        let mut b = ax.max.filter(|m| *m > 0.0).unwrap_or(hi).log10().ceil();
        if !a.is_finite() || !b.is_finite() {
            a = 0.0;
            b = 1.0;
        }
        if b <= a {
            b = a + 1.0;
        }
        if b - a > 30.0 {
            a = b - 30.0;
        }
        let ticks: Vec<f64> = (0..=((b - a) as usize))
            .map(|e| 10f64.powf(a + e as f64))
            .collect();
        let first = ticks.first().copied().unwrap_or(1.0);
        return Ticks {
            lo: first,
            hi: ticks.last().copied().unwrap_or(10.0),
            step: 0.0,
            ticks,
            base: first,
            log: Some((a, b)),
        };
    }
    let mut tk = nice_ticks(lo, hi, n);
    if let Some(ax) = ax
        && (ax.min.is_some() || ax.max.is_some())
    {
        let a = ax.min.unwrap_or(tk.lo);
        let b = ax.max.unwrap_or(tk.hi);
        if b > a {
            let t2 = nice_ticks(a, b, n);
            let eps = t2.step * 1e-6;
            tk = Ticks {
                lo: a,
                hi: b,
                step: t2.step,
                ticks: t2
                    .ticks
                    .into_iter()
                    .filter(|t| *t >= a - eps && *t <= b + eps)
                    .collect(),
                base: 0.0,
                log: None,
            };
        }
    }
    // Desktop ends the axis a little over the largest value unless the next tick is close
    if ax.is_none_or(|a| a.max.is_none())
        && hi > 0.0
        && tk.step > 0.0
        && tk.hi - hi > tk.step * 0.25
        && hi > tk.lo
    {
        tk.hi = (hi * 1.05).min(tk.hi);
        let eps = tk.step * 1e-6;
        tk.ticks.retain(|t| *t <= tk.hi + eps);
    }
    tk.base = 0f64.clamp(tk.lo, tk.hi.max(tk.lo));
    tk
}

pub(crate) fn tick_fmt<'a>(
    tk: &Ticks,
    col: &ColFmt,
    ctx: &'a Ctx<'_>,
    units: Option<f64>,
) -> Box<dyn Fn(f64) -> String + 'a> {
    if tk.log.is_some() {
        let c = col.clone();
        Box::new(move |x| fmt_val(x, Some(&c), &ctx.loc, true))
    } else {
        Box::new(crate::fmt::axis_fmt_units(
            tk.lo,
            tk.hi,
            tk.step,
            Some(col),
            &ctx.loc,
            units,
        ))
    }
}

/// Category axis under a vertical chart: horizontal labels when they fit, else 45°.
pub(crate) struct CatAxis {
    pub(crate) h: f32,
    pub(crate) rot: bool,
    pub(crate) every: usize,
    pub(crate) len: f32,
}
pub(crate) fn cat_axis(
    ctx: &Ctx<'_>,
    cats: &[String],
    band: f32,
    px: f32,
    max_h: f32,
    continuous: bool,
) -> CatAxis {
    let widest = cats
        .iter()
        .take(400)
        .map(|c| tw(ctx, c, px))
        .fold(0.0f32, f32::max)
        .min(px * 14.0);
    let flat = widest + px * 0.4 <= band;
    // a continuous (date) axis skips labels instead of turning them
    let rot = !flat && !continuous && band >= px * 1.1;
    let len = widest.min((max_h - px) / 0.71).max(0.0);
    let need = if flat || !rot {
        px * 1.8
    } else {
        (px * 1.8).max(len * 0.71 + px)
    };
    let every = if rot {
        ((px * 1.3 / band).ceil() as usize).max(1)
    } else {
        (((widest + px).min(px * 6.0) / band).ceil() as usize).max(1)
    };
    CatAxis {
        h: need,
        rot,
        every,
        len,
    }
}
pub(crate) fn draw_cat_axis(
    cv: &mut Canvas,
    ctx: &Ctx<'_>,
    ax: &CatAxis,
    cats: &[String],
    band: f32,
    px: f32,
    x_of: &dyn Fn(usize) -> f32,
    b: f32,
) {
    let mut i = 0;
    while i < cats.len() {
        let c = cats.get(i).map(String::as_str).unwrap_or("");
        if ax.rot {
            let t = fit(ctx, c, ax.len, px);
            tx_rot(
                cv,
                ctx,
                x_of(i) + px * 0.2,
                b + px * 0.6,
                &t,
                px,
                ctx.muted,
                Anchor::End,
                -45.0,
            );
        } else {
            let t = fit(ctx, c, band * ax.every as f32 - 2.0, px);
            tx(cv, ctx, x_of(i), b + px * 0.85, &t, px, ctx.muted, Anchor::Middle);
        }
        i += ax.every;
    }
}

/// One-row legend; returns (space used at the top, at the bottom).
pub fn place_legend(
    cv: &mut Canvas,
    ctx: &Ctx<'_>,
    f: &Fmt,
    items: &[(String, Rgb)],
    title: &str,
    b: Bx,
) -> (f32, f32) {
    let px = ctx.chart_px;
    // Desktop 2.158: a legend row one line plus 12.7 px high (26.7 px at 8 pt), items centred,
    // 5 px dots
    let lh = crate::axes::line_h(px) + 12.7;
    let bottom = f.legend_pos.contains("bottom");
    let cy = if bottom { b.h - lh / 2.0 } else { lh / 2.0 + 0.55 };
    let mut cx = 0.0f32;
    let title = if f.legend_text.is_empty() {
        title
    } else {
        f.legend_text.as_str()
    };
    // Desktop shows the legend field's name unless the report turns the title off
    if !title.is_empty() && f.legend_title != Some(false) {
        let t = fit(ctx, title, b.w * 0.4, px);
        tx_w(
            cv,
            ctx,
            cx,
            cy,
            &t,
            px,
            f.legend_color.unwrap_or_else(|| legend_color(ctx, ctx.fg2)),
            Anchor::Start,
            600,
        );
        cx += ctx.fonts.measure(&font(ctx, px).weight(600), &t) + px * 0.7;
    }
    for (name, color) in items {
        let label = crate::data::clip(name, 40);
        let need = 12.0 + tw(ctx, &label, px) + 4.5;
        let room = b.w - cx - px * 1.1;
        if room < px * 1.5 {
            break;
        }
        cv.circle(cx + 5.5, cy, 5.0, Some((*color, 1.0)), None);
        let t = if cx + need > b.w {
            fit(ctx, &label, room, px)
        } else {
            label
        };
        tx(
            cv,
            ctx,
            cx + 12.0,
            cy,
            &t,
            px,
            f.legend_color.unwrap_or_else(|| legend_color(ctx, ctx.muted)),
            Anchor::Start,
        );
        if cx + need > b.w {
            break;
        }
        cx += need;
    }
    if bottom { (0.0, lh) } else { (lh, 0.0) }
}

/// Legend text: #666666 in Desktop's classic look (2.158), else the theme's colour.
fn legend_color(ctx: &Ctx<'_>, themed: Rgb) -> Rgb {
    if ctx.classic {
        Rgb(0x66, 0x66, 0x66)
    } else {
        themed
    }
}

pub(crate) fn want_legend(f: &Fmt, cd: &ChartData) -> bool {
    f.legend_show != Some(false) && (cd.series.len() > 1 || !cd.legend_name.is_empty())
}

/// The visual-type icon drawn when a visual has nothing to show.
pub fn blank_visual(cv: &mut Canvas, ctx: &Ctx<'_>, b: Bx, icon: &[&str]) {
    let m = b.w.min(b.h) * 0.42;
    if m < 6.0 {
        return;
    }
    let k = m / 16.0;
    let t = tiny_skia::Transform::from_translate((b.w - m) / 2.0, (b.h - m) / 2.0).pre_scale(k, k);
    for d in icon {
        if let Some(p) = crate::canvas::svg_path(d) {
            cv.stroke_tf(&p, ctx.grid, 1.2, t);
        }
    }
}

// --- reference lines and trends --------------------------------------------------------------

struct RefLine {
    x: f64,
    color: Rgb,
    width: f32,
    dash: Dash,
    label: bool,
}

fn dash_of(style: &str) -> Dash {
    match style.to_lowercase().as_str() {
        "solid" => None,
        "dotted" => Some((1.0, 3.0)),
        _ => Some((5.0, 4.0)),
    }
}

fn ref_values(f: &Fmt, ser: &[Series]) -> Vec<RefLine> {
    let mut out = Vec::new();
    for l in &f.lines {
        let axis = sk(l, "axis");
        if !axis.is_empty() && axis != "y" {
            continue;
        }
        let kind = sk(l, "kind");
        let x = if kind == "constant" {
            nk(l, "value", f64::NAN)
        } else {
            let col = l.get("column").and_then(Value::as_u64).map(|c| c as usize);
            let se = ser.iter().find(|s| s.j.is_some() && s.j == col).or(ser.first());
            let mut xs: Vec<f64> = se
                .map(|s| s.vals.iter().flatten().copied().collect())
                .unwrap_or_default();
            if xs.is_empty() {
                continue;
            }
            xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let q = |p: f64| {
                let k = p.clamp(0.0, 1.0) * (xs.len() - 1) as f64;
                let i = k.floor() as usize;
                let a = xs.get(i).copied().unwrap_or(0.0);
                let b = xs.get(i + 1).copied().unwrap_or(a);
                a + (b - a) * (k - i as f64)
            };
            match kind {
                "average" => xs.iter().sum::<f64>() / xs.len() as f64,
                "min" => xs.first().copied().unwrap_or(0.0),
                "max" => xs.last().copied().unwrap_or(0.0),
                "median" => q(0.5),
                "percentile" => {
                    let p = nk(l, "percentile", 50.0);
                    q(if p > 1.0 { p / 100.0 } else { p })
                }
                _ => continue,
            }
        };
        if !x.is_finite() {
            continue;
        }
        out.push(RefLine {
            x,
            color: Rgb::from_value(crate::data::get(l, "color")).unwrap_or(Rgb(0x60, 0x5e, 0x5c)),
            width: clampf(nk(l, "width", 1.5) as f32, 0.5, 10.0),
            dash: dash_of(sk(l, "style")),
            label: crate::data::get(l, "data_label") == &Value::Bool(true),
        });
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn draw_ref_lines(
    cv: &mut Canvas,
    ctx: &Ctx<'_>,
    lines: &[RefLine],
    at: &dyn Fn(f64) -> f32,
    horiz: bool,
    a0: f32,
    a1: f32,
    fmt: &dyn Fn(f64) -> String,
) {
    let px = ctx.chart_px;
    for l in lines {
        let p = at(l.x);
        if horiz {
            cv.line(p, a0, p, a1, l.color, l.width, l.dash);
        } else {
            cv.line(a0, p, a1, p, l.color, l.width, l.dash);
        }
        if l.label {
            let t = fmt(l.x);
            if horiz {
                tx(
                    cv,
                    ctx,
                    p + px * 0.3,
                    a0 + px * 0.6,
                    &t,
                    px,
                    l.color,
                    Anchor::Start,
                );
            } else {
                tx(cv, ctx, a1 - 2.0, p - px * 0.6, &t, px, l.color, Anchor::End);
            }
        }
    }
}

struct Fit {
    a: f64,
    b: f64,
    t0: f64,
    t1: f64,
    color: Rgb,
    width: f32,
    dash: Dash,
}

/// The trend lines of the first `n` categories, category `i` at `xs(i)` (its index, or its date
/// on a time axis).
fn trend_fits(f: &Fmt, ser: &[Series], n: usize, xs: &dyn Fn(usize) -> f64) -> Vec<Fit> {
    let mut out = Vec::new();
    let (x0, x1) = (0..n)
        .map(xs)
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), x| {
            (a.min(x), b.max(x))
        });
    for l in &f.lines {
        if sk(l, "kind") != "trend" {
            continue;
        }
        for se in ser.iter().take(12) {
            let (mut k, mut sx, mut sy, mut sxx, mut sxy) = (0.0, 0.0, 0.0, 0.0, 0.0);
            for i in 0..n {
                let Some(y) = se.vals.get(i).copied().flatten() else {
                    continue;
                };
                // from the first category, so that dates keep their precision
                let x = xs(i) - x0;
                k += 1.0;
                sx += x;
                sy += y;
                sxx += x * x;
                sxy += x * y;
            }
            let d = k * sxx - sx * sx;
            if k < 2.0 || d == 0.0 {
                continue;
            }
            let b = (k * sxy - sx * sy) / d;
            let a = (sy - b * sx) / k - b * x0;
            if !a.is_finite() || !b.is_finite() {
                continue;
            }
            out.push(Fit {
                a,
                b,
                t0: x0,
                t1: x1,
                color: Rgb::from_value(crate::data::get(l, "color")).unwrap_or(se.color),
                width: clampf(nk(l, "width", 1.5) as f32, 0.5, 10.0),
                dash: dash_of(sk(l, "style")),
            });
        }
    }
    out
}

fn draw_trends(cv: &mut Canvas, fits: &[Fit], p: &dyn Fn(f64, f64) -> (f32, f32), lo: f64, hi: f64) {
    for f in fits {
        let (mut t0, mut t1) = (f.t0, f.t1);
        if f.b != 0.0 {
            let ta = (lo - f.a) / f.b;
            let tb = (hi - f.a) / f.b;
            t0 = t0.max(ta.min(tb));
            t1 = t1.min(ta.max(tb));
        } else if f.a < lo || f.a > hi {
            continue;
        }
        if t1 <= t0 || !t0.is_finite() || !t1.is_finite() {
            continue;
        }
        let (x1, y1) = p(t0, f.a + f.b * t0);
        let (x2, y2) = p(t1, f.a + f.b * t1);
        cv.line_a(x1, y1, x2, y2, f.color, 1.0, f.width, f.dash, true);
    }
}

// --- error bars --------------------------------------------------------------------------------

/// How one series' error bars are drawn (`format.reference_lines` entry of kind `error_bars`).
/// Defaults when the visual sets nothing are BIcraft's (a 2 px dark whisker with flat caps), not
/// measured against Desktop.
#[derive(Debug, Clone)]
pub(crate) struct ErrStyle {
    /// The series' index in the chart's series.
    pub j: usize,
    pub bar: bool,
    pub color: Rgb,
    pub width: f32,
    pub caps: bool,
    pub band: bool,
    /// `fill`, `line` or `fillLine`.
    pub band_style: String,
    pub band_color: Rgb,
    pub band_alpha: f32,
}

/// The error bars of the series `ser` (matched by their data column).
pub(crate) fn err_styles(f: &Fmt, ser: &[Series], ctx: &Ctx<'_>) -> Vec<ErrStyle> {
    let mut out = Vec::new();
    for l in &f.lines {
        if sk(l, "kind") != "error_bars" {
            continue;
        }
        let Some(col) = l.get("column").and_then(Value::as_u64) else {
            continue;
        };
        let Some((j, se)) = ser
            .iter()
            .enumerate()
            .find(|(_, s)| s.j == usize::try_from(col).ok() && !s.err.is_empty())
        else {
            continue;
        };
        let matched = crate::data::get(l, "match_series") == &Value::Bool(true);
        let color = Rgb::from_value(crate::data::get(l, "color"))
            .filter(|_| !matched)
            .unwrap_or(if matched { se.color } else { ctx.fg2 });
        let shape = sk(l, "marker_shape");
        out.push(ErrStyle {
            j,
            bar: crate::data::get(l, "bar") != &Value::Bool(false),
            color,
            width: clampf(nk(l, "width", 2.0) as f32, 0.5, 10.0),
            caps: crate::data::get(l, "marker") != &Value::Bool(false) && shape != "none",
            band: crate::data::get(l, "band") == &Value::Bool(true),
            band_style: sk(l, "band_style").to_string(),
            band_color: Rgb::from_value(crate::data::get(l, "band_color")).unwrap_or(se.color),
            band_alpha: 1.0 - clampf(nk(l, "transparency", 70.0) as f32, 0.0, 100.0) / 100.0,
        });
    }
    out
}

/// Widens `(lo, hi)` to the error bars' bounds.
pub(crate) fn err_range(errs: &[ErrStyle], ser: &[Series], lo: &mut f64, hi: &mut f64) {
    for e in errs {
        let Some(se) = ser.get(e.j) else { continue };
        for (u, l) in &se.err {
            for x in [u, l].into_iter().flatten() {
                *lo = lo.min(*x);
                *hi = hi.max(*x);
            }
        }
    }
}

/// One whisker at `c` (the across-axis position) from `a` to `b` (along the value axis), caps
/// `cap` wide; `horiz`: the value axis is horizontal.
#[allow(clippy::too_many_arguments)]
pub(crate) fn whisker(
    cv: &mut Canvas,
    e: &ErrStyle,
    c: f32,
    a: Option<f32>,
    b: Option<f32>,
    mid: f32,
    cap: f32,
    horiz: bool,
) {
    let seg = |cv: &mut Canvas, p: f32, q: f32| {
        if horiz {
            cv.line(p, c, q, c, e.color, e.width, None);
        } else {
            cv.line(c, p, c, q, e.color, e.width, None);
        }
    };
    let tick = |cv: &mut Canvas, p: f32| {
        if horiz {
            cv.line(p, c - cap / 2.0, p, c + cap / 2.0, e.color, e.width, None);
        } else {
            cv.line(c - cap / 2.0, p, c + cap / 2.0, p, e.color, e.width, None);
        }
    };
    for end in [a, b].into_iter().flatten() {
        if e.bar {
            seg(cv, mid, end);
        }
        if e.caps {
            tick(cv, end);
        }
    }
}

/// The band of a line's error bars: the area between the bounds (`fill`), its edges (`line`) or
/// both, over the points `(x, upper, lower)` of one run.
pub(crate) fn err_band(cv: &mut Canvas, e: &ErrStyle, pts: &[(f32, f32, f32)]) {
    if pts.len() < 2 {
        return;
    }
    let fill = e.band_style != "line";
    let edge = e.band_style == "line" || e.band_style == "fillLine";
    if fill {
        let mut poly: Vec<(f32, f32)> = pts.iter().map(|p| (p.0, p.1)).collect();
        poly.extend(pts.iter().rev().map(|p| (p.0, p.2)));
        cv.polygon(&poly, e.band_color, e.band_alpha);
    }
    if edge {
        let up: Vec<(f32, f32)> = pts.iter().map(|p| (p.0, p.1)).collect();
        let dn: Vec<(f32, f32)> = pts.iter().map(|p| (p.0, p.2)).collect();
        cv.polyline(&up, e.band_color, 1.0, 1.0, None);
        cv.polyline(&dn, e.band_color, 1.0, 1.0, None);
    }
}

// --- anomaly detection --------------------------------------------------------------------------

/// A line chart's anomalies (`format.reference_lines` entry of kind `anomaly_detection`) and how
/// they are drawn: BIcraft's estimate ([`crate::anomaly`]). Defaults when the visual sets none
/// are BIcraft's (a filled band in the series' colour at 80 % transparency, 5 px circles in the
/// secondary text colour), not measured against Desktop.
#[derive(Debug, Clone)]
pub(crate) struct Anomalies {
    /// The series' index in the chart's series.
    pub j: usize,
    pub est: Vec<Option<crate::anomaly::Expected>>,
    /// `fill`, `line` or `none`.
    pub band_style: String,
    pub band: bool,
    pub band_color: Rgb,
    pub band_alpha: f32,
    pub marker: bool,
    pub shape: String,
    pub size: f32,
    pub marker_color: Rgb,
    pub marker_alpha: f32,
}

/// The anomalies of the chart's series `ser`: the first `anomaly_detection` entry, on the series
/// of its data column (else the first series), when the series has enough points.
pub(crate) fn anomalies(f: &Fmt, ser: &[Series], ctx: &Ctx<'_>) -> Option<Anomalies> {
    let l = f.lines.iter().find(|l| sk(l, "kind") == "anomaly_detection")?;
    let col = l.get("column").and_then(Value::as_u64);
    let (j, se) = match col {
        Some(c) => ser
            .iter()
            .enumerate()
            .find(|(_, s)| s.j == usize::try_from(c).ok())?,
        None => (0, ser.first()?),
    };
    let est = crate::anomaly::detect(
        &se.vals,
        nk(l, "sensitivity", crate::anomaly::DEFAULT_SENSITIVITY),
    );
    if est.is_empty() {
        return None;
    }
    let style = match sk(l, "band_style") {
        "line" => "line",
        "none" => "none",
        _ => "fill",
    };
    let alpha = |key: &str, def: f64| 1.0 - clampf(nk(l, key, def) as f32, 0.0, 100.0) / 100.0;
    Some(Anomalies {
        j,
        est,
        band_style: style.to_string(),
        band: crate::data::get(l, "band") != &Value::Bool(false) && style != "none",
        band_color: Rgb::from_value(crate::data::get(l, "band_color")).unwrap_or(se.color),
        band_alpha: alpha("transparency", 80.0),
        marker: crate::data::get(l, "marker") != &Value::Bool(false),
        shape: sk(l, "marker_shape").to_string(),
        size: clampf(nk(l, "marker_size", 5.0) as f32, 1.0, 50.0),
        marker_color: Rgb::from_value(crate::data::get(l, "marker_color")).unwrap_or(ctx.fg2),
        marker_alpha: alpha("marker_transparency", 0.0),
    })
}

/// Widens `(lo, hi)` to the expected range when the band is drawn.
pub(crate) fn anomaly_range(a: &Anomalies, lo: &mut f64, hi: &mut f64) {
    if !a.band {
        return;
    }
    for e in a.est.iter().flatten() {
        *lo = lo.min(e.low);
        *hi = hi.max(e.high);
    }
}

/// The expected range over the points `(x, high, low)` of one run.
pub(crate) fn anomaly_band(cv: &mut Canvas, a: &Anomalies, pts: &[(f32, f32, f32)]) {
    if !a.band || pts.len() < 2 {
        return;
    }
    if a.band_style == "line" {
        let up: Vec<(f32, f32)> = pts.iter().map(|p| (p.0, p.1)).collect();
        let dn: Vec<(f32, f32)> = pts.iter().map(|p| (p.0, p.2)).collect();
        let alpha = a.band_alpha.max(0.35);
        cv.polyline(&up, a.band_color, alpha, 1.0, Some((3.0, 3.0)));
        cv.polyline(&dn, a.band_color, alpha, 1.0, Some((3.0, 3.0)));
        return;
    }
    let mut poly: Vec<(f32, f32)> = pts.iter().map(|p| (p.0, p.1)).collect();
    poly.extend(pts.iter().rev().map(|p| (p.0, p.2)));
    cv.polygon(&poly, a.band_color, a.band_alpha);
}

/// One anomaly's marker at `(x, y)`.
pub(crate) fn anomaly_marker(cv: &mut Canvas, a: &Anomalies, x: f32, y: f32) {
    if !a.marker {
        return;
    }
    let r = a.size;
    let fill = (a.marker_color, a.marker_alpha);
    match a.shape.as_str() {
        "square" => cv.polygon(
            &[(x - r, y - r), (x + r, y - r), (x + r, y + r), (x - r, y + r)],
            fill.0,
            fill.1,
        ),
        "diamond" => cv.polygon(
            &[
                (x, y - r * 1.3),
                (x + r * 1.3, y),
                (x, y + r * 1.3),
                (x - r * 1.3, y),
            ],
            fill.0,
            fill.1,
        ),
        "triangle" => cv.polygon(
            &[(x, y - r * 1.3), (x + r * 1.2, y + r), (x - r * 1.2, y + r)],
            fill.0,
            fill.1,
        ),
        "droplet" => {
            cv.circle(x, y + r * 0.3, r, Some(fill), None);
            cv.polygon(
                &[
                    (x - r * 0.85, y - r * 0.2),
                    (x, y - r * 1.6),
                    (x + r * 0.85, y - r * 0.2),
                ],
                fill.0,
                fill.1,
            );
        }
        _ => cv.circle(x, y, r, Some(fill), None),
    }
}

pub(crate) fn axis_title(ax: &crate::data::Axis, fallback: &str) -> String {
    if ax.title {
        if ax.text.is_empty() {
            fallback.to_string()
        } else {
            ax.text.clone()
        }
    } else {
        String::new()
    }
}

pub(crate) fn items(ser: &[Series]) -> Vec<(String, Rgb)> {
    ser.iter().map(|s| (s.name.clone(), s.color)).collect()
}

pub const ICON_BAR: &[&str] = &[
    "M3 13V8",
    "M6.5 13V4",
    "M10 13V6.5",
    "M13.5 13V9.5",
    "M1.5 13.5h13",
];
pub const ICON_LINE: &[&str] = &["M1.5 13.5h13", "M2 11l3.5-4 3 2.5L14 3"];
pub const ICON_PIE: &[&str] = &[
    "M8 2.5a5.5 5.5 0 1 0 5.5 5.5H8z",
    "M10 1.5a4.5 4.5 0 0 1 4.5 4.5H10z",
];
pub const ICON_GAUGE: &[&str] = &["M2 11.5a6 6 0 0 1 12 0", "M8 11.5l3-4"];
pub const ICON_KPI: &[&str] = &["M2 12l4-4 3 2 5-6", "M10.5 4H14v3.5"];
pub const ICON_SCATTER: &[&str] = &[
    "M1.5 13.5h13",
    "M1.5 13.5v-12",
    "M5 10h.01",
    "M7.5 6h.01",
    "M10 9h.01",
    "M12.5 4h.01",
];
pub const ICON_TABLE: &[&str] = &["M2 3h12v10H2z", "M2 6.3h12", "M2 9.6h12", "M6 3v10"];

fn pct_fmt(ctx: &Ctx<'_>) -> impl Fn(f64) -> String + use<> {
    let loc = ctx.loc.clone();
    move |x| loc.percent(x, 0, 0)
}

// --- bar / column ----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default)]
pub struct BarOpts {
    pub horiz: bool,
    pub stacked: bool,
    pub pct: bool,
}

pub fn bar_chart(cv: &mut Canvas, ds: &Dataset, b: Bx, f: &Fmt, ctx: &Ctx<'_>, o: BarOpts) {
    use crate::axes::{self, Bands, Label, LabelPos, line_h};
    let mut cd = chart_data(ds, ctx, Some(f), true, if o.horiz { 40 } else { 60 });
    crate::timeaxis::clip_to_range(&mut cd, &ds.cols, f);
    let (ser, n, m) = (&cd.series, cd.cats.len(), cd.series.len());
    if m == 0 || n == 0 {
        return blank_visual(cv, ctx, b, ICON_BAR);
    }
    let px = ctx.chart_px;
    let cpx = f.x.px.unwrap_or(px);
    let vpx = f.y.px.unwrap_or(px);
    let (lt, lb) = if want_legend(f, &cd) {
        place_legend(cv, ctx, f, &items(ser), &cd.legend_name, b)
    } else {
        (0.0, 0.0)
    };
    let sum_abs: Vec<f64> = (0..n)
        .map(|i| {
            ser.iter()
                .map(|s| s.vals.get(i).copied().flatten().unwrap_or(0.0).abs())
                .sum()
        })
        .collect();
    let val = |j: usize, i: usize| -> Option<f64> {
        let x = ser.get(j)?.vals.get(i).copied().flatten()?;
        Some(if o.pct {
            let t = sum_abs.get(i).copied().unwrap_or(0.0);
            if t != 0.0 { x / t } else { 0.0 }
        } else {
            x
        })
    };
    let (mut lo, mut hi) = (0.0f64, 0.0f64);
    for i in 0..n {
        let (mut p, mut q) = (0.0, 0.0);
        for j in 0..m {
            let Some(x) = val(j, i) else { continue };
            if o.stacked {
                if x >= 0.0 { p += x } else { q += x }
            } else {
                hi = hi.max(x);
                lo = lo.min(x);
            }
        }
        if o.stacked {
            hi = hi.max(p);
            lo = lo.min(q);
        }
    }
    if o.pct {
        hi = hi.min(1.0);
    }
    let lines = if o.pct { Vec::new() } else { ref_values(f, ser) };
    for l in &lines {
        hi = hi.max(l.x);
        lo = lo.min(l.x);
    }
    // error bars on clustered bars (stacked and 100 % bars draw none)
    let errs = if o.pct || o.stacked {
        Vec::new()
    } else {
        err_styles(f, ser, ctx)
    };
    err_range(&errs, ser, &mut lo, &mut hi);
    let Some(first) = ser.first() else { return };
    let vcol = first.col.cf();
    let vfallback = if o.pct {
        String::new()
    } else if !cd.legend_name.is_empty() {
        first.col.name.clone()
    } else {
        ser.iter()
            .take(3)
            .map(|s| s.name.clone())
            .collect::<Vec<_>>()
            .join(" and ")
    };
    let vtitle = axis_title(&f.y, &vfallback);
    let ctitle = axis_title(&f.x, &cd.cat_name);
    let refs: Vec<&Series> = ser.iter().collect();
    let styles = axes::label_styles(f, &refs);
    let auto_pos = if o.stacked {
        LabelPos::InsideCenter
    } else {
        LabelPos::OutsideEnd
    };
    let pos_of = |j: usize| styles.get(j).and_then(|s| s.pos).unwrap_or(auto_pos);
    let any_out = (0..m).any(|j| styles.get(j).is_some_and(|s| s.show) && pos_of(j) == LabelPos::OutsideEnd);
    let lpx_of = |j: usize| styles.get(j).and_then(|s| s.px).unwrap_or(px);
    let lpx = (0..m).map(lpx_of).fold(px, f32::max);
    let pct_tf = pct_fmt(ctx);
    let explicit = if o.pct { None } else { Some(&f.y) };
    let ticks_for = |len: f32| -> Ticks {
        if o.pct {
            axes::scale(0.0, 1.0, len, !o.horiz, vpx, None)
        } else {
            axes::scale(lo, hi, len, !o.horiz, vpx, explicit)
        }
    };
    let labels_of = |tk: &Ticks| -> Vec<String> {
        let tf: Box<dyn Fn(f64) -> String> = if o.pct {
            Box::new(pct_fmt(ctx))
        } else {
            tick_fmt(tk, &vcol, ctx, f.y.units)
        };
        tk.ticks
            .iter()
            .map(|t| if f.y.show { tf(*t) } else { String::new() })
            .collect()
    };
    let lab_text = |j: usize, i: usize, unit: f64| -> String {
        let Some(st) = styles.get(j) else {
            return String::new();
        };
        if o.pct {
            let p = st.precision.unwrap_or(2);
            ctx.loc.percent(val(j, i).unwrap_or(0.0), p, p)
        } else {
            let s = ser.get(j);
            axes::label_text(
                s.and_then(|s| s.vals.get(i).copied().flatten()).unwrap_or(0.0),
                &s.map(|s| s.col.cf()).unwrap_or_default(),
                st,
                unit,
                ctx,
            )
        }
    };
    let grid = ctx.grid;
    let vcolor = f.y.color.unwrap_or(ctx.muted);
    let ccolor = f.x.color.unwrap_or(ctx.muted);
    let ip = axes::inner_pad(f);
    let minw = axes::min_cat(f);
    let ch = (b.h - lt - lb).max(1.0);
    let mut labels: Vec<Label> = Vec::new();
    if !o.horiz {
        let y_tw = if vtitle.is_empty() { 0.0 } else { px * 1.5 };
        let x_th = if ctitle.is_empty() { 0.0 } else { px * 1.5 };
        let t0 = lt + if f.y.show { line_h(vpx) / 2.0 + 2.5 } else { 4.0 };
        // a continuous date axis keeps 13 px on the right (Desktop 2.158)
        let ta = TimeAxis::of(&cd, &ds.cols, f);
        let r = if ta.is_some() { b.w - 13.0 } else { b.w };
        let reserve = if any_out { line_h(lpx) + 8.0 } else { 0.0 };
        // ticks depend on the plot height, which depends on the category labels, which depend
        // on the category width, which depends on the tick labels' width: two passes
        let mut bb = b.h - lb - x_th - 22.5;
        let mut l = 0.0f32;
        let mut tk = ticks_for(bb - t0 - reserve);
        let mut tl = labels_of(&tk);
        let mut bands = Bands::new(n, 1.0, ip, minw);
        let mut cl = None;
        let mut reach = 0.0f32;
        let mut ticks: Vec<(f32, String)> = Vec::new();
        for _ in 0..3 {
            let tl_w = tl.iter().map(|t| tw(ctx, t, vpx)).fold(0.0f32, f32::max);
            l = y_tw + if f.y.show { tl_w + 10.0 } else { 2.0 };
            bands = match &ta {
                Some(ta) => Bands::time(ta, r - l, ip),
                None => Bands::new(n, r - l, ip, minw),
            };
            if ta.is_none() && reach > l + bands.mid(0) {
                // solve l + mid(0) = reach with mid(0) proportional to the plot width
                let k = bands.mid(0) / (r - l).max(1.0);
                l = ((reach - k * r) / (1.0 - k).max(0.1)).max(l);
                bands = Bands::new(n, r - l, ip, minw);
            }
            let c = match &ta {
                Some(ta) => {
                    let (s, bar) = ta.bars(r - l, ip);
                    ticks = ta.ticks(ctx, r - l, 0.5 + bar / 2.0, s, cpx, &|w| {
                        crate::timeaxis::label_room(ta, w, cpx)
                    });
                    let widest = ticks.iter().map(|t| tw(ctx, &t.1, cpx)).fold(0.0f32, f32::max);
                    f.x.show.then(|| axes::tick_labels(cpx, widest))
                }
                None => {
                    let vis: Vec<String> = cd.cats.iter().take(bands.n).cloned().collect();
                    f.x.show
                        .then(|| axes::cat_labels(ctx, &vis, bands.step, cpx, ch, f))
                }
            };
            let scroll = if bands.scrolls() { axes::SCROLL } else { 0.0 };
            bb = b.h - lb - x_th - scroll - c.as_ref().map_or(4.0, |c| c.h);
            reach = c.as_ref().map_or(0.0, |c| {
                axes::first_label_reach(ctx, c, cd.cats.first().map(String::as_str).unwrap_or(""))
            });
            cl = c;
            tk = ticks_for(bb - t0 - reserve);
            tl = labels_of(&tk);
        }
        if r - l < 8.0 || bb - t0 - reserve < 8.0 {
            return;
        }
        let top = t0 + reserve;
        let y = |x: f64| bb - tk.f(x) * (bb - top);
        for (k, t) in tk.ticks.iter().enumerate() {
            if f.y.gridlines {
                cv.line(l, y(*t), r, y(*t), grid, 1.0, GRID_DASH);
            }
            if let Some(s) = tl.get(k).filter(|s| !s.is_empty()) {
                tx(cv, ctx, l - 10.0, y(*t), s, vpx, vcolor, Anchor::End);
            }
        }
        let unit = axes::axis_unit(&tk, f.y.units);
        let y0 = y(tk.base);
        let lh = line_h(lpx);
        let mut push = |j: usize, i: usize, x: f32, w: f32, ya: f32, yb: f32, c: Rgb| {
            let Some(st) = styles.get(j).filter(|s| s.show) else {
                return;
            };
            let lp = lpx_of(j);
            let t = lab_text(j, i, unit);
            let (top_y, base_y) = (ya.min(yb), ya.max(yb));
            let up = yb <= ya;
            let hgt = base_y - top_y;
            let pos = pos_of(j);
            let inside = pos != LabelPos::OutsideEnd;
            if inside && hgt < lh * 0.85 {
                return;
            }
            let t = if inside { fit(ctx, &t, w - 2.0, lp) } else { t };
            if t.is_empty() || (!inside && tw(ctx, &t, lp) > bands.step * 1.15) {
                return;
            }
            let ly = match (pos, up) {
                (LabelPos::OutsideEnd, true) => top_y - 9.0,
                (LabelPos::OutsideEnd, false) => base_y + 9.0,
                (LabelPos::InsideEnd, true) => top_y + lh / 2.0 + 5.4,
                (LabelPos::InsideEnd, false) => base_y - lh / 2.0 - 5.4,
                (LabelPos::InsideCenter, _) => (top_y + base_y) / 2.0,
                (LabelPos::InsideBase, true) => base_y - 12.5,
                (LabelPos::InsideBase, false) => top_y + 12.5,
            };
            // a conditional label colour (labels.color by rule) first
            let own = ser.get(j).and_then(|s| s.lpts.get(i).copied().flatten());
            let color = if let Some(c) = own {
                c
            } else if inside && st.background.is_none() {
                axes::inside_color(st.color, c)
            } else {
                st.color.unwrap_or(ctx.muted)
            };
            labels.push(Label {
                x: x + w / 2.0,
                y: ly,
                text: t,
                color,
                anchor: Anchor::Middle,
                bg: st.background,
                px: lp,
            });
        };
        for i in 0..bands.n {
            let x0 = l + bands.x(i);
            let gw = bands.bar;
            if o.stacked {
                let (mut p, mut q) = (0.0, 0.0);
                for j in 0..m {
                    let Some(x) = val(j, i).filter(|x| *x != 0.0) else {
                        continue;
                    };
                    let a = if x >= 0.0 { p } else { q };
                    let e = a + x;
                    if x >= 0.0 {
                        p = e
                    } else {
                        q = e
                    }
                    let (ya, yb) = (y(a), y(e));
                    let c = ser.get(j).map_or(ctx.accent, |s| s.ptc(i));
                    let part = hits::hv(&cd, i, j).map(|h0| {
                        let h = match h0 {
                            Some(h) if o.pct => {
                                let t = sum_abs.get(i).copied().unwrap_or(0.0);
                                if t != 0.0 { h / t } else { 0.0 }
                            }
                            h => h.unwrap_or(0.0),
                        };
                        (h != 0.0).then(|| {
                            let yh = y(a + h);
                            (x0, ya.min(yh), gw, (ya - yh).abs().max(0.5))
                        })
                    });
                    hits::bar_mark(
                        cv,
                        ctx,
                        &cd,
                        i,
                        Some(j),
                        (x0, ya.min(yb), gw, (ya - yb).abs().max(0.5)),
                        c,
                        part,
                    );
                    push(j, i, x0, gw, ya, yb, c);
                }
            } else {
                let bw = gw / m as f32;
                for j in 0..m {
                    let Some(x) = val(j, i) else { continue };
                    let yv = y(x);
                    let c = ser.get(j).map_or(ctx.accent, |s| s.ptc(i));
                    let xj = x0 + j as f32 * bw;
                    let part = hits::hv(&cd, i, j).map(|h| {
                        h.filter(|h| *h != 0.0).map(|h| {
                            let yh = y(h);
                            (xj, yh.min(y0), bw.max(0.5), (y0 - yh).abs().max(0.5))
                        })
                    });
                    hits::bar_mark(
                        cv,
                        ctx,
                        &cd,
                        i,
                        Some(j),
                        (xj, yv.min(y0), bw.max(0.5), (y0 - yv).abs().max(0.5)),
                        c,
                        part,
                    );
                    push(j, i, xj, bw, y0, yv, c);
                }
            }
        }
        for e in &errs {
            let Some(se) = ser.get(e.j) else { continue };
            let bw = bands.bar / m as f32;
            for i in 0..bands.n {
                let (Some((u, d)), Some(x)) = (se.err.get(i), val(e.j, i)) else {
                    continue;
                };
                let c = l + bands.x(i) + e.j as f32 * bw + bw / 2.0;
                whisker(
                    cv,
                    e,
                    c,
                    u.map(y),
                    d.map(y),
                    y(x),
                    (bw * 0.3).clamp(3.0, 16.0),
                    false,
                );
            }
        }
        let tf: Box<dyn Fn(f64) -> String> = if o.pct {
            Box::new(&pct_tf)
        } else {
            tick_fmt(&tk, &vcol, ctx, f.y.units)
        };
        draw_ref_lines(cv, ctx, &lines, &y, false, l, r, &tf);
        if !o.stacked && !o.pct {
            match &ta {
                Some(ta) => {
                    let (s, bar) = ta.bars(r - l, ip);
                    let a = l + 0.5 + bar / 2.0;
                    draw_trends(
                        cv,
                        &trend_fits(f, ser, bands.n, &|i| ta.at(i)),
                        &|t, v| (a + ((t - ta.lo) * s) as f32, y(v)),
                        tk.lo,
                        tk.hi,
                    );
                }
                None => draw_trends(
                    cv,
                    &trend_fits(f, ser, bands.n, &|i| i as f64),
                    &|t, v| (l + bands.mid(0) + t as f32 * bands.step, y(v)),
                    tk.lo,
                    tk.hi,
                ),
            }
        }
        axes::draw_labels(cv, ctx, &labels);
        if let Some(cl) = &cl {
            if ta.is_some() {
                let texts: Vec<String> = ticks.iter().map(|t| t.1.clone()).collect();
                axes::draw_cat_labels(
                    cv,
                    ctx,
                    cl,
                    &texts,
                    texts.len(),
                    crate::timeaxis::tick_room(&ticks, r - l),
                    ccolor,
                    &|k| l + ticks.get(k).map_or(0.0, |t| t.0),
                    bb,
                );
            } else {
                axes::draw_cat_labels(
                    cv,
                    ctx,
                    cl,
                    &cd.cats,
                    bands.n,
                    bands.step,
                    ccolor,
                    &|i| l + bands.mid(i),
                    bb,
                );
            }
        }
        if bands.scrolls() {
            axes::scroll_bar(cv, false, l, r, b.h - lb - x_th - axes::SCROLL, &bands);
        }
        if x_th > 0.0 {
            let t = fit(ctx, &ctitle, r - l, px);
            tx(
                cv,
                ctx,
                (l + r) / 2.0,
                b.h - lb - px * 0.55,
                &t,
                px,
                f.x.title_color.unwrap_or(ctx.fg2),
                Anchor::Middle,
            );
        }
        if y_tw > 0.0 {
            let t = fit(ctx, &vtitle, bb - t0, px);
            tx_rot(
                cv,
                ctx,
                px * 0.7,
                (t0 + bb) / 2.0,
                &t,
                px,
                f.y.title_color.unwrap_or(ctx.fg2),
                Anchor::Middle,
                -90.0,
            );
        }
    } else {
        let y_tw = if ctitle.is_empty() { 0.0 } else { px * 1.5 };
        let x_th = if vtitle.is_empty() { 0.0 } else { px * 1.5 };
        let t0 = lt + 8.0;
        let bb = b.h - lb - x_th - if f.y.show { 22.0 } else { 2.0 };
        if bb - t0 < 8.0 {
            return;
        }
        // a continuous date axis runs down, the earliest date on top, labelled at its ticks
        let ta = TimeAxis::of(&cd, &ds.cols, f);
        let (bands, ticks, ta_at) = match &ta {
            Some(ta) => {
                let (s, bar) = ta.bars(bb - t0, ip);
                let lh = line_h(cpx);
                let a = 0.5 + bar / 2.0;
                let ticks = ta.ticks(ctx, bb - t0, a, s, cpx, &|_| lh * 1.6);
                (Bands::time(ta, bb - t0, ip), ticks, Some((a, s)))
            }
            None => (Bands::new(n, bb - t0, ip, minw), Vec::new(), None),
        };
        let cw = if f.x.show {
            let widest = match ta {
                Some(_) => ticks.iter().map(|t| tw(ctx, &t.1, cpx)).fold(0.0f32, f32::max),
                None => cd
                    .cats
                    .iter()
                    .take(400)
                    .map(|c| tw(ctx, c, cpx))
                    .fold(0.0f32, f32::max),
            };
            (widest + 4.4).min(b.w * f.x.max_margin.unwrap_or(axes::MAX_MARGIN))
        } else {
            0.0
        };
        let l = y_tw + if f.x.show { cw + 9.5 } else { 2.0 };
        let scroll = if bands.scrolls() { axes::SCROLL } else { 0.0 };
        // the room outside the bars' ends for their labels
        let reserve_for = |tk: &Ticks| -> f32 {
            if !any_out {
                return 0.0;
            }
            let unit = axes::axis_unit(tk, f.y.units);
            let mut w = 0.0f32;
            for j in 0..m {
                if styles.get(j).is_some_and(|s| s.show) && pos_of(j) == LabelPos::OutsideEnd {
                    for i in 0..bands.n {
                        w = w.max(tw(ctx, &lab_text(j, i, unit), lpx_of(j)));
                    }
                }
            }
            w + 19.0
        };
        let mut r = b.w - scroll - 12.0;
        let mut tk = ticks_for(r - l);
        let mut tl = labels_of(&tk);
        for _ in 0..2 {
            let last = tw(ctx, tl.last().map(String::as_str).unwrap_or(""), vpx) / 2.0;
            r = b.w - scroll - last.max(2.0);
            let res = reserve_for(&tk);
            tk = ticks_for(r - l - res);
            tl = labels_of(&tk);
        }
        let res = reserve_for(&tk);
        let xr = r - res;
        if xr - l < 8.0 {
            return;
        }
        let x = |v: f64| l + tk.f(v) * (xr - l);
        let every_t = (((tl.iter().map(|t| tw(ctx, t, vpx)).fold(0.0f32, f32::max) + vpx)
            / ((xr - l) / (tl.len().max(2) - 1) as f32))
            .ceil() as usize)
            .max(1);
        for (k, t) in tk.ticks.iter().enumerate() {
            if f.y.gridlines {
                cv.line(x(*t), t0, x(*t), bb, grid, 1.0, GRID_DASH);
            }
            if k % every_t == 0
                && let Some(s) = tl.get(k).filter(|s| !s.is_empty())
            {
                tx(
                    cv,
                    ctx,
                    x(*t),
                    bb + 2.3 + line_h(vpx) / 2.0,
                    s,
                    vpx,
                    vcolor,
                    Anchor::Middle,
                );
            }
        }
        let unit = axes::axis_unit(&tk, f.y.units);
        let x0 = x(tk.base);
        let mut push = |j: usize, i: usize, y: f32, h: f32, xa: f32, xb: f32, c: Rgb| {
            let Some(st) = styles.get(j).filter(|s| s.show) else {
                return;
            };
            let lp = lpx_of(j);
            let pos = pos_of(j);
            let inside = pos != LabelPos::OutsideEnd;
            // a label outside a thin bar may use the gap to the next band (Desktop 2.158 labels
            // short bar charts whose bars are thinner than the text)
            let slots = if o.stacked { 1.0 } else { m.max(1) as f32 };
            let room = if inside { h } else { h.max(bands.step / slots) };
            if room < line_h(lp) * 0.7 {
                return;
            }
            let t = lab_text(j, i, unit);
            let (left, right) = (xa.min(xb), xa.max(xb));
            let fwd = xb >= xa;
            // a label that does not fit inside its bar is left out (Desktop 2.158)
            if inside && tw(ctx, &t, lp) > right - left - 4.0 {
                return;
            }
            if t.is_empty() {
                return;
            }
            let (lx, anchor) = match (pos, fwd) {
                (LabelPos::OutsideEnd, true) => (right + 7.5, Anchor::Start),
                (LabelPos::OutsideEnd, false) => (left - 7.5, Anchor::End),
                (LabelPos::InsideEnd, true) => (right - 6.0, Anchor::End),
                (LabelPos::InsideEnd, false) => (left + 6.0, Anchor::Start),
                (LabelPos::InsideCenter, _) => ((left + right) / 2.0, Anchor::Middle),
                (LabelPos::InsideBase, true) => (left + 6.0, Anchor::Start),
                (LabelPos::InsideBase, false) => (right - 6.0, Anchor::End),
            };
            // a conditional label colour (labels.color by rule) first
            let own = ser.get(j).and_then(|s| s.lpts.get(i).copied().flatten());
            let color = if let Some(c) = own {
                c
            } else if inside && st.background.is_none() {
                axes::inside_color(st.color, c)
            } else {
                st.color.unwrap_or(ctx.muted)
            };
            labels.push(Label {
                x: lx,
                y: y + h / 2.0,
                text: t,
                color,
                anchor,
                bg: st.background,
                px: lp,
            });
        };
        for i in 0..bands.n {
            let y0 = t0 + bands.x(i);
            let gh = bands.bar;
            if o.stacked {
                let (mut p, mut q) = (0.0, 0.0);
                for j in 0..m {
                    let Some(v) = val(j, i).filter(|x| *x != 0.0) else {
                        continue;
                    };
                    let a = if v >= 0.0 { p } else { q };
                    let e = a + v;
                    if v >= 0.0 {
                        p = e
                    } else {
                        q = e
                    }
                    let (xa, xb) = (x(a), x(e));
                    let c = ser.get(j).map_or(ctx.accent, |s| s.ptc(i));
                    let part = hits::hv(&cd, i, j).map(|h0| {
                        let h = match h0 {
                            Some(h) if o.pct => {
                                let t = sum_abs.get(i).copied().unwrap_or(0.0);
                                if t != 0.0 { h / t } else { 0.0 }
                            }
                            h => h.unwrap_or(0.0),
                        };
                        (h != 0.0).then(|| {
                            let xh = x(a + h);
                            (xa.min(xh), y0, (xa - xh).abs().max(0.5), gh)
                        })
                    });
                    hits::bar_mark(
                        cv,
                        ctx,
                        &cd,
                        i,
                        Some(j),
                        (xa.min(xb), y0, (xa - xb).abs().max(0.5), gh),
                        c,
                        part,
                    );
                    push(j, i, y0, gh, xa, xb, c);
                }
            } else {
                let bh = gh / m as f32;
                for j in 0..m {
                    let Some(v) = val(j, i) else { continue };
                    let xv = x(v);
                    let c = ser.get(j).map_or(ctx.accent, |s| s.ptc(i));
                    let yj = y0 + j as f32 * bh;
                    let part = hits::hv(&cd, i, j).map(|h| {
                        h.filter(|h| *h != 0.0).map(|h| {
                            let xh = x(h);
                            (xh.min(x0), yj, (xh - x0).abs().max(0.5), bh.max(0.5))
                        })
                    });
                    hits::bar_mark(
                        cv,
                        ctx,
                        &cd,
                        i,
                        Some(j),
                        (xv.min(x0), yj, (xv - x0).abs().max(0.5), bh.max(0.5)),
                        c,
                        part,
                    );
                    push(j, i, yj, bh, x0, xv, c);
                }
            }
        }
        for e in &errs {
            let Some(se) = ser.get(e.j) else { continue };
            let bh = bands.bar / m as f32;
            for i in 0..bands.n {
                let (Some((u, d)), Some(v)) = (se.err.get(i), val(e.j, i)) else {
                    continue;
                };
                let c = t0 + bands.x(i) + e.j as f32 * bh + bh / 2.0;
                whisker(
                    cv,
                    e,
                    c,
                    u.map(x),
                    d.map(x),
                    x(v),
                    (bh * 0.3).clamp(3.0, 16.0),
                    true,
                );
            }
        }
        let tf: Box<dyn Fn(f64) -> String> = if o.pct {
            Box::new(&pct_tf)
        } else {
            tick_fmt(&tk, &vcol, ctx, f.y.units)
        };
        draw_ref_lines(cv, ctx, &lines, &x, true, t0, bb, &tf);
        if !o.stacked && !o.pct {
            match (&ta, ta_at) {
                (Some(ta), Some((a, s))) => draw_trends(
                    cv,
                    &trend_fits(f, ser, bands.n, &|i| ta.at(i)),
                    &|t, v| (x(v), t0 + a + ((t - ta.lo) * s) as f32),
                    tk.lo,
                    tk.hi,
                ),
                _ => draw_trends(
                    cv,
                    &trend_fits(f, ser, bands.n, &|i| i as f64),
                    &|t, v| (x(v), t0 + bands.mid(0) + t as f32 * bands.step),
                    tk.lo,
                    tk.hi,
                ),
            }
        }
        axes::draw_labels(cv, ctx, &labels);
        if f.x.show && ta.is_some() {
            for (at, label) in &ticks {
                let t = fit(ctx, label, (cw - 4.0).max(cpx), cpx);
                tx(cv, ctx, l - 9.5, t0 + at, &t, cpx, ccolor, Anchor::End);
            }
        } else if f.x.show {
            let lh = line_h(cpx);
            let every = ((lh * 0.8 / bands.step).ceil() as usize).max(1);
            let mut i = 0;
            while i < bands.n {
                let t = fit(
                    ctx,
                    cd.cats.get(i).map(String::as_str).unwrap_or(""),
                    (cw - 4.0).max(cpx),
                    cpx,
                );
                tx(cv, ctx, l - 9.5, t0 + bands.mid(i), &t, cpx, ccolor, Anchor::End);
                i += every;
            }
        }
        if bands.scrolls() {
            axes::scroll_bar(cv, true, t0, bb, b.w - axes::SCROLL, &bands);
        }
        if x_th > 0.0 {
            let t = fit(ctx, &vtitle, xr - l, px);
            tx(
                cv,
                ctx,
                (l + xr) / 2.0,
                b.h - lb - px * 0.55,
                &t,
                px,
                f.y.title_color.unwrap_or(ctx.fg2),
                Anchor::Middle,
            );
        }
        if y_tw > 0.0 {
            let t = fit(ctx, &ctitle, bb - t0, px);
            tx_rot(
                cv,
                ctx,
                px * 0.7,
                (t0 + bb) / 2.0,
                &t,
                px,
                f.x.title_color.unwrap_or(ctx.fg2),
                Anchor::Middle,
                -90.0,
            );
        }
    }
}

// --- line / area / combo ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default)]
pub struct LineOpts {
    pub area: bool,
    pub stacked: bool,
    pub pct: bool,
    pub combo: bool,
}

fn range(list: &[&Series], n: usize, stacked: bool, zero: bool) -> (f64, f64) {
    let (mut lo, mut hi) = if zero {
        (0.0f64, 0.0f64)
    } else {
        (f64::INFINITY, f64::NEG_INFINITY)
    };
    for i in 0..n {
        let (mut p, mut q) = (0.0, 0.0);
        for se in list {
            let Some(x) = se.vals.get(i).copied().flatten() else {
                continue;
            };
            if stacked {
                if x >= 0.0 { p += x } else { q += x }
            } else {
                hi = hi.max(x);
                lo = lo.min(x);
            }
        }
        if stacked {
            hi = hi.max(p);
            lo = lo.min(q);
        }
    }
    if hi < lo || !hi.is_finite() || !lo.is_finite() {
        return (0.0, 1.0);
    }
    // Desktop starts a mostly positive line axis at zero; otherwise the axis is the data range
    // made nice (axes::scale)
    if !zero && lo >= 0.0 && lo <= hi * 0.5 {
        lo = 0.0;
    }
    (lo, hi)
}

/// Whether a combo's line values are drawn on the columns' axis (no secondary axis): the
/// secondary axis is turned off, or the two domains overlap on at least a tenth of their union.
/// Desktop 2.158 draws a line of the columns' magnitude (even with another format string, e.g.
/// signed) on the primary axis, and gives a line of another magnitude its own axis.
fn merge_axes(f: &Fmt, l: (f64, f64), r: (f64, f64)) -> bool {
    if f.raw.get("y2_axis").and_then(|a| a.get("show")) == Some(&Value::Bool(false)) {
        return true;
    }
    let union = l.1.max(r.1) - l.0.min(r.0);
    let overlap = l.1.min(r.1) - l.0.max(r.0);
    union.is_finite() && union > 0.0 && overlap >= union * 0.1
}

pub fn line_chart(cv: &mut Canvas, ds: &Dataset, b: Bx, f: &Fmt, ctx: &Ctx<'_>, o: LineOpts) {
    use crate::axes::{self, Bands, Label, LabelPos, line_h};
    let mut cd = chart_data(ds, ctx, Some(f), false, 400);
    crate::timeaxis::clip_to_range(&mut cd, &ds.cols, f);
    let n = cd.cats.len();
    if cd.series.is_empty() || n == 0 {
        return blank_visual(cv, ctx, b, ICON_LINE);
    }
    let px = ctx.chart_px;
    let (lt, lb) = if want_legend(f, &cd) {
        place_legend(cv, ctx, f, &items(&cd.series), &cd.legend_name, b)
    } else {
        (0.0, 0.0)
    };
    // which series are columns (combo) and which are lines
    let mut is_col: Vec<bool> = vec![false; cd.series.len()];
    if o.combo {
        is_col = cd.series.iter().map(|s| !s.y2).collect();
        let lines_n = is_col.iter().filter(|c| !**c).count();
        let cols_n = is_col.len() - lines_n;
        if lines_n == 0
            && cols_n > 1
            && cd.legend_name.is_empty()
            && let Some(last) = is_col.iter().rposition(|c| *c)
            && let Some(c) = is_col.get_mut(last)
        {
            *c = false;
        }
    }
    // stacked areas: each series between the running totals (100 %: shares)
    if o.area && o.stacked {
        let line_idx: Vec<usize> = (0..cd.series.len())
            .filter(|i| !is_col.get(*i).copied().unwrap_or(false))
            .collect();
        let tot: Vec<f64> = (0..n)
            .map(|i| {
                line_idx
                    .iter()
                    .map(|j| {
                        cd.series
                            .get(*j)
                            .and_then(|s| s.vals.get(i).copied().flatten())
                            .unwrap_or(0.0)
                            .abs()
                    })
                    .sum()
            })
            .collect();
        let mut acc = vec![0.0f64; n];
        for j in line_idx {
            let Some(se) = cd.series.get_mut(j) else { continue };
            let base = acc.clone();
            se.vals = se
                .vals
                .iter()
                .enumerate()
                .map(|(i, x)| {
                    x.map(|x| {
                        let t = tot.get(i).copied().unwrap_or(0.0);
                        base.get(i).copied().unwrap_or(0.0)
                            + if o.pct {
                                if t != 0.0 { x / t } else { 0.0 }
                            } else {
                                x
                            }
                    })
                })
                .collect();
            acc = se
                .vals
                .iter()
                .enumerate()
                .map(|(i, y)| y.unwrap_or(base.get(i).copied().unwrap_or(0.0)))
                .collect();
            se.base = Some(base);
        }
    }
    let col_s: Vec<&Series> = cd
        .series
        .iter()
        .zip(&is_col)
        .filter(|(_, c)| **c)
        .map(|(s, _)| s)
        .collect();
    let line_s: Vec<&Series> = cd
        .series
        .iter()
        .zip(&is_col)
        .filter(|(_, c)| !**c)
        .map(|(s, _)| s)
        .collect();
    let left_s: Vec<&Series> = if col_s.is_empty() {
        line_s.clone()
    } else {
        col_s.clone()
    };
    let mut dual = !col_s.is_empty() && !line_s.is_empty();
    let vpx = f.y.px.unwrap_or(px);
    let cpx = f.x.px.unwrap_or(px);
    let rpx = f.y2.px.unwrap_or(vpx);
    let left_owned: Vec<Series> = left_s.iter().map(|s| (*s).clone()).collect();
    let lines = if o.pct {
        Vec::new()
    } else {
        ref_values(f, &left_owned)
    };
    let (mut l0, mut h0) = if o.pct {
        (0.0, 1.0)
    } else {
        range(
            &left_s,
            n,
            o.stacked && !col_s.is_empty() && !o.area,
            !col_s.is_empty(),
        )
    };
    for l in &lines {
        l0 = l0.min(l.x);
        h0 = h0.max(l.x);
    }
    let (mut r0, mut r1) = range(&line_s, n, false, false);
    // error bars on the lines (not on a combo chart's columns, nor stacked or 100 % areas)
    let errs: Vec<ErrStyle> = if o.stacked || o.pct {
        Vec::new()
    } else {
        err_styles(f, &cd.series, ctx)
            .into_iter()
            .filter(|e| !is_col.get(e.j).copied().unwrap_or(false))
            .collect()
    };
    if col_s.is_empty() {
        err_range(&errs, &cd.series, &mut l0, &mut h0);
    } else {
        err_range(&errs, &cd.series, &mut r0, &mut r1);
    }
    // anomalies: a plain line chart's one series (Power BI offers none with a legend, columns,
    // stacks or several values)
    let anom = if o.stacked || o.pct || o.area || !col_s.is_empty() || !cd.legend_name.is_empty() {
        None
    } else {
        anomalies(f, &cd.series, ctx).filter(|a| !cd.series.get(a.j).is_some_and(|s| s.y2))
    };
    if let Some(a) = &anom {
        anomaly_range(a, &mut l0, &mut h0);
    }
    // one value axis for both when the line values share the columns' format and range
    if dual && !o.pct && merge_axes(f, (l0, h0), (r0, r1)) {
        dual = false;
        l0 = l0.min(r0);
        h0 = h0.max(r1);
    }
    let Some(lf) = left_s.first() else { return };
    let lcol = lf.col.cf();
    let rcol = line_s.first().map(|s| s.col.cf()).unwrap_or_default();
    let pctf = pct_fmt(ctx);
    let left_ticks =
        |len: f32| -> Ticks { axes::scale(l0, h0, len, true, vpx, if o.pct { None } else { Some(&f.y) }) };
    let right_ticks = |len: f32| -> Ticks { axes::scale(r0, r1, len, true, rpx, Some(&f.y2)) };
    let left_labels = |tk: &Ticks| -> Vec<String> {
        let tf: Box<dyn Fn(f64) -> String> = if o.pct {
            Box::new(pct_fmt(ctx))
        } else {
            tick_fmt(tk, &lcol, ctx, f.y.units)
        };
        tk.ticks
            .iter()
            .map(|t| if f.y.show { tf(*t) } else { String::new() })
            .collect()
    };
    let show_r = dual && f.y.show && f.y2.show;
    let right_labels = |tk: &Ticks| -> Vec<String> {
        if !show_r {
            return Vec::new();
        }
        let tf = tick_fmt(tk, &rcol, ctx, f.y2.units.or(f.y.units));
        tk.ticks.iter().map(|t| tf(*t)).collect()
    };
    let all: Vec<&Series> = cd.series.iter().collect();
    let styles = axes::label_styles(f, &all);
    let lstyles = axes::line_styles(f, &all);
    let col_idx: Vec<usize> = (0..cd.series.len())
        .filter(|i| is_col.get(*i).copied().unwrap_or(false))
        .collect();
    let line_idx: Vec<usize> = (0..cd.series.len())
        .filter(|i| !is_col.get(*i).copied().unwrap_or(false))
        .collect();
    let col_out = col_idx.iter().any(|j| {
        styles
            .get(*j)
            .is_some_and(|s| s.show && s.pos.is_none_or(|p| p == LabelPos::OutsideEnd) && !o.stacked)
    });
    let line_lab = line_idx.iter().any(|j| styles.get(*j).is_some_and(|s| s.show));
    let ctitle = axis_title(&f.x, &cd.cat_name);
    let yfb = if !cd.legend_name.is_empty() {
        lf.col.name.clone()
    } else {
        left_s
            .iter()
            .take(3)
            .map(|s| s.name.clone())
            .collect::<Vec<_>>()
            .join(" and ")
    };
    let ytitle = axis_title(&f.y, &yfb);
    let x_th = if ctitle.is_empty() { 0.0 } else { px * 1.5 };
    let y_tw = if ytitle.is_empty() { 0.0 } else { px * 1.5 };
    let lpx = px;
    let lh = line_h(lpx);
    let t0 = lt
        + if f.y.show { line_h(vpx) / 2.0 + 2.5 } else { 4.0 }
        + if line_lab && col_idx.is_empty() { lh } else { 0.0 };
    let reserve = if col_out { lh + 8.0 } else { 0.0 };
    let ch = (b.h - lt - lb).max(1.0);
    // a continuous date axis: points at their dates from edge to edge, 10.5 px in, or columns
    // at their dates (combo)
    let ip = axes::inner_pad(f);
    let ta = TimeAxis::of(&cd, &ds.cols, f);
    let cont = ta.is_some() && col_s.is_empty();
    // where the first date sits and the px per day: (a, s), date t at l + a + (t - lo) * s
    let tmap = |l: f32, r: f32| -> Option<(f32, f64)> {
        let ta = ta.as_ref()?;
        Some(if cont {
            (10.5, f64::from((r - l - 21.0).max(1.0)) / (ta.hi - ta.lo))
        } else {
            let (s, bar) = ta.bars(r - l, ip);
            (0.5 + bar / 2.0, s)
        })
    };
    let mut ticks: Vec<(f32, String)> = Vec::new();
    let minw = if col_s.is_empty() { 0.0 } else { axes::min_cat(f) };
    // two passes: tick labels' width → plot width → category labels → plot height → ticks
    let mut bb = b.h - lb - x_th - 22.5;
    let mut tk_l = left_ticks(bb - t0 - reserve);
    let mut tl_l = left_labels(&tk_l);
    let mut tk_r = dual.then(|| right_ticks(bb - t0));
    let mut tl_r = tk_r.as_ref().map(right_labels).unwrap_or_default();
    let (mut l, mut r) = (0.0f32, b.w);
    let mut bands = Bands::new(n, 1.0, ip, minw);
    let mut cl = None;
    for _ in 0..2 {
        l = y_tw
            + if f.y.show {
                tl_l.iter().map(|t| tw(ctx, t, vpx)).fold(0.0f32, f32::max) + 10.0
            } else {
                2.0
            };
        r = b.w
            - if show_r {
                tl_r.iter().map(|t| tw(ctx, t, rpx)).fold(0.0f32, f32::max) + 10.0
            } else if ta.is_some() {
                // a continuous date axis keeps 13 px on the right (Desktop 2.158)
                13.0
            } else if col_s.is_empty() {
                px * 0.6
            } else {
                0.0
            };
        bands = match &ta {
            Some(_) if cont => Bands::new(n, 1.0, ip, 0.0),
            Some(ta) => Bands::time(ta, r - l, ip),
            None => Bands::new(n, r - l, ip, minw),
        };
        let c = match (&ta, tmap(l, r)) {
            (Some(ta), Some((a, s))) => {
                ticks = ta.ticks(ctx, r - l, a, s, cpx, &|w| {
                    crate::timeaxis::label_room(ta, w, cpx)
                });
                let widest = ticks.iter().map(|t| tw(ctx, &t.1, cpx)).fold(0.0f32, f32::max);
                f.x.show.then(|| axes::tick_labels(cpx, widest))
            }
            _ => {
                let vis: Vec<String> = cd.cats.iter().take(bands.n).cloned().collect();
                f.x.show
                    .then(|| axes::cat_labels(ctx, &vis, bands.step, cpx, ch, f))
            }
        };
        let scroll = if !cont && bands.scrolls() {
            axes::SCROLL
        } else {
            0.0
        };
        bb = b.h - lb - x_th - scroll - c.as_ref().map_or(4.0, |c| c.h);
        cl = c;
        tk_l = left_ticks(bb - t0 - reserve);
        tl_l = left_labels(&tk_l);
        tk_r = dual.then(|| right_ticks(bb - t0));
        tl_r = tk_r.as_ref().map(right_labels).unwrap_or_default();
    }
    if r - l < 8.0 || bb - t0 - reserve < 8.0 {
        return;
    }
    let shown = if cont { n } else { bands.n };
    let tm = tmap(l, r);
    // the distance between two categories (on a time axis: the smallest one)
    let band = match (&ta, tm) {
        (Some(ta), Some((_, s))) if cont => ((ta.gap * s) as f32).max(0.1),
        _ => bands.step,
    };
    let top_l = t0 + reserve;
    let yl = |x: f64| bb - tk_l.f(x) * (bb - top_l);
    let yr = |x: f64| match &tk_r {
        Some(tk) => bb - tk.f(x) * (bb - t0),
        None => yl(x),
    };
    let vcolor = f.y.color.unwrap_or(ctx.muted);
    for (k, t) in tk_l.ticks.iter().enumerate() {
        if f.y.gridlines {
            cv.line(l, yl(*t), r, yl(*t), ctx.grid, 1.0, GRID_DASH);
        }
        if let Some(s) = tl_l.get(k).filter(|s| !s.is_empty()) {
            tx(cv, ctx, l - 10.0, yl(*t), s, vpx, vcolor, Anchor::End);
        }
    }
    if let Some(tk) = &tk_r {
        let rcolor = f.y2.color.unwrap_or(vcolor);
        for (k, t) in tk.ticks.iter().enumerate() {
            if let Some(s) = tl_r.get(k).filter(|s| !s.is_empty()) {
                // Desktop marks the secondary axis' ticks with 6 px strokes inside the plot
                cv.line(r - 6.0, yr(*t), r, yr(*t), ctx.grid, 1.0, None);
                tx(cv, ctx, r + 9.0, yr(*t), s, rpx, rcolor, Anchor::Start);
            }
        }
    }
    let xi = |i: usize| match (&ta, tm) {
        (Some(ta), Some((a, s))) if cont => l + a + ((ta.at(i) - ta.lo) * s) as f32,
        _ => l + bands.mid(i),
    };
    // lines join their points in time order
    let order: Vec<usize> = match &ta {
        Some(ta) => ta.order().into_iter().filter(|i| *i < shown).collect(),
        None => (0..shown).collect(),
    };
    let mut labels: Vec<Label> = Vec::new();
    let unit_l = axes::axis_unit(&tk_l, f.y.units);
    let unit_r = tk_r
        .as_ref()
        .map_or(unit_l, |t| axes::axis_unit(t, f.y2.units.or(f.y.units)));
    if !col_s.is_empty() {
        let gw = bands.bar;
        let y0 = yl(tk_l.base);
        let m = col_s.len();
        for i in 0..shown {
            let x0 = l + bands.x(i);
            let (mut p, mut q) = (0.0, 0.0);
            for (k, j) in col_idx.iter().enumerate() {
                let Some(se) = cd.series.get(*j) else { continue };
                let Some(x) = se.vals.get(i).copied().flatten() else {
                    continue;
                };
                let (bx, bw, ya, yb) = if o.stacked {
                    let a = if x >= 0.0 { p } else { q };
                    let e = a + x;
                    if x >= 0.0 {
                        p = e
                    } else {
                        q = e
                    }
                    (x0, gw, yl(a), yl(e))
                } else {
                    let bw = gw / m as f32;
                    (x0 + k as f32 * bw, bw, y0, yl(x))
                };
                let part = hits::hv(&cd, i, *j).map(|h| {
                    let base = if o.stacked {
                        if x >= 0.0 { p - x } else { q - x }
                    } else {
                        tk_l.base
                    };
                    h.filter(|h| *h != 0.0).map(|h| {
                        let (yh, yb0) = (yl(base + h), yl(base));
                        (bx, yb0.min(yh), bw.max(0.5), (yb0 - yh).abs().max(0.5))
                    })
                });
                hits::bar_mark(
                    cv,
                    ctx,
                    &cd,
                    i,
                    Some(*j),
                    (bx, ya.min(yb), bw.max(0.5), (ya - yb).abs().max(0.5)),
                    se.ptc(i),
                    part,
                );
                if let Some(st) = styles.get(*j).filter(|s| s.show) {
                    let t = axes::label_text(x, &se.col.cf(), st, unit_l, ctx);
                    let (top_y, base_y) = (ya.min(yb), ya.max(yb));
                    let pos = st.pos.unwrap_or(if o.stacked {
                        LabelPos::InsideCenter
                    } else {
                        LabelPos::OutsideEnd
                    });
                    let inside = pos != LabelPos::OutsideEnd;
                    if inside && base_y - top_y < lh * 0.85 {
                        continue;
                    }
                    let t = if inside { fit(ctx, &t, bw - 2.0, lpx) } else { t };
                    let ly = match pos {
                        LabelPos::OutsideEnd => top_y - 9.0,
                        LabelPos::InsideEnd => top_y + lh / 2.0 + 5.4,
                        LabelPos::InsideCenter => (top_y + base_y) / 2.0,
                        LabelPos::InsideBase => base_y - 12.5,
                    };
                    labels.push(Label {
                        x: bx + bw / 2.0,
                        y: ly,
                        text: t,
                        color: if let Some(c) = se.lpts.get(i).copied().flatten() {
                            c
                        } else if inside {
                            axes::inside_color(st.color, se.ptc(i))
                        } else {
                            st.color.unwrap_or(ctx.muted)
                        },
                        anchor: Anchor::Middle,
                        bg: st.background,
                        px: lpx,
                    });
                }
            }
        }
    }
    // areas first, then lines over them
    for pass in 0..2 {
        for j in &line_idx {
            let Some(se) = cd.series.get(*j) else { continue };
            let ls = lstyles.get(*j).copied().unwrap_or(axes::LineStyle {
                width: 2.0,
                dash: None,
                marker: None,
                marker_color: None,
                show: true,
            });
            let y = |x: f64| if dual { yr(x) } else { yl(x) };
            let base_y = if dual {
                yr(tk_r.as_ref().map_or(0.0, |t| t.base))
            } else {
                yl(tk_l.base)
            };
            let mut segs: Vec<Vec<(f32, f32, usize)>> = Vec::new();
            let mut cur = Vec::new();
            for &i in &order {
                match se.vals.get(i).copied().flatten() {
                    Some(x) => cur.push((xi(i), y(x), i)),
                    None => {
                        if !cur.is_empty() {
                            segs.push(std::mem::take(&mut cur));
                        }
                    }
                }
            }
            if !cur.is_empty() {
                segs.push(cur);
            }
            // this line's error bars: the band under the lines, the whiskers over them
            if let Some(e) = errs.iter().find(|e| e.j == *j) {
                for sg in &segs {
                    let at = |p: &(f32, f32, usize)| se.err.get(p.2).copied().unwrap_or((None, None));
                    if pass == 0 && e.band {
                        let pts: Vec<(f32, f32, f32)> = sg
                            .iter()
                            .map(|p| {
                                let (u, d) = at(p);
                                (p.0, u.map_or(p.1, y), d.map_or(p.1, y))
                            })
                            .collect();
                        err_band(cv, e, &pts);
                    }
                    if pass == 1 && (e.bar || e.caps) {
                        for p in sg {
                            let (u, d) = at(p);
                            let cap = (band * 0.3).clamp(3.0, 12.0);
                            whisker(cv, e, p.0, u.map(y), d.map(y), p.1, cap, false);
                        }
                    }
                }
            }
            // this line's expected range (under the lines)
            if pass == 0
                && let Some(a) = anom.as_ref().filter(|a| a.j == *j)
            {
                for sg in &segs {
                    let pts: Vec<(f32, f32, f32)> = sg
                        .iter()
                        .filter_map(|p| {
                            let e = a.est.get(p.2).copied().flatten()?;
                            Some((p.0, y(e.high), y(e.low)))
                        })
                        .collect();
                    anomaly_band(cv, a, &pts);
                }
            }
            for sg in &segs {
                let pts: Vec<(f32, f32)> = sg.iter().map(|p| (p.0, p.1)).collect();
                if pass == 0 && o.area {
                    let mut poly_pts = pts.clone();
                    match &se.base {
                        Some(base) => {
                            for p in sg.iter().rev() {
                                poly_pts.push((p.0, y(base.get(p.2).copied().unwrap_or(0.0))));
                            }
                            cv.polygon(&poly_pts, se.color, 0.4);
                        }
                        None => {
                            if let (Some(last), Some(first)) = (pts.last(), pts.first()) {
                                poly_pts.push((last.0, base_y));
                                poly_pts.push((first.0, base_y));
                            }
                            cv.polygon(&poly_pts, se.color, 0.3);
                        }
                    }
                }
                if pass == 1 {
                    if !ls.show {
                    } else if pts.len() == 1 {
                        if let Some(p) = pts.first() {
                            cv.circle(p.0, p.1, ls.width * 1.4, Some((se.color, 1.0)), None);
                        }
                    } else if let Some(p) = poly(&pts, false) {
                        cv.stroke(&p, se.color, 1.0, ls.width, ls.dash);
                    }
                    if let Some(mr) = ls.marker {
                        for p in sg {
                            let a = ctx.ix.alpha(&hits::cd_point(&cd, Some(p.2), Some(*j)));
                            cv.circle(
                                p.0,
                                p.1,
                                mr * 0.8,
                                Some((ls.marker_color.unwrap_or(se.color), a)),
                                None,
                            );
                        }
                    }
                    // the anomalies, over the line
                    if let Some(a) = anom.as_ref().filter(|a| a.j == *j) {
                        for p in sg {
                            if a.est.get(p.2).copied().flatten().is_some_and(|e| e.anomaly) {
                                anomaly_marker(cv, a, p.0, p.1);
                            }
                        }
                    }
                    // every point is clickable and has its tooltip (a disc around it)
                    if ctx.ix.on() {
                        for p in sg {
                            let pt = hits::cd_point(&cd, Some(p.2), Some(*j));
                            let shape = hits::HitShape::Circle {
                                cx: p.0,
                                cy: p.1,
                                r: 7.0,
                            };
                            let tip = hits::cd_tip(&cd, p.2, *j, ctx);
                            cv.hit(ctx.ix.hit(shape, ctx.ix.select(&pt), tip));
                        }
                    }
                }
            }
            if pass == 1
                && let Some(st) = styles.get(*j).filter(|s| s.show)
                && !o.stacked
                && band > lpx * 2.0
            {
                let unit = if dual { unit_r } else { unit_l };
                for i in 0..shown {
                    if let Some(x) = se.vals.get(i).copied().flatten() {
                        labels.push(Label {
                            x: xi(i),
                            y: y(x) - 9.0 - ls.marker.unwrap_or(0.0) * 0.5,
                            text: axes::label_text(x, &se.col.cf(), st, unit, ctx),
                            color: se
                                .lpts
                                .get(i)
                                .copied()
                                .flatten()
                                .or(st.color)
                                .unwrap_or(ctx.muted),
                            anchor: Anchor::Middle,
                            bg: st.background,
                            px: lpx,
                        });
                    }
                }
            }
        }
    }
    let rf: Box<dyn Fn(f64) -> String> = if o.pct {
        Box::new(&pctf)
    } else {
        tick_fmt(&tk_l, &lcol, ctx, f.y.units)
    };
    draw_ref_lines(cv, ctx, &lines, &yl, false, l, r, &rf);
    if !o.stacked && !o.pct && tk_l.log.is_none() {
        match (&ta, tm) {
            (Some(ta), Some((a, s))) => draw_trends(
                cv,
                &trend_fits(f, &left_owned, shown, &|i| ta.at(i)),
                &|t, v| (l + a + ((t - ta.lo) * s) as f32, yl(v)),
                tk_l.lo,
                tk_l.hi,
            ),
            _ => draw_trends(
                cv,
                &trend_fits(f, &left_owned, shown, &|i| i as f64),
                &|t, v| (xi(0) + t as f32 * band, yl(v)),
                tk_l.lo,
                tk_l.hi,
            ),
        }
    }
    axes::draw_labels(cv, ctx, &labels);
    if let Some(cl) = &cl {
        if ta.is_some() {
            let texts: Vec<String> = ticks.iter().map(|t| t.1.clone()).collect();
            axes::draw_cat_labels(
                cv,
                ctx,
                cl,
                &texts,
                texts.len(),
                crate::timeaxis::tick_room(&ticks, r - l),
                f.x.color.unwrap_or(ctx.muted),
                &|k| l + ticks.get(k).map_or(0.0, |t| t.0),
                bb,
            );
        } else {
            axes::draw_cat_labels(
                cv,
                ctx,
                cl,
                &cd.cats,
                shown,
                band,
                f.x.color.unwrap_or(ctx.muted),
                &xi,
                bb,
            );
        }
    }
    if !cont && bands.scrolls() {
        axes::scroll_bar(cv, false, l, r, b.h - lb - x_th - axes::SCROLL, &bands);
    }
    if x_th > 0.0 {
        let t = fit(ctx, &ctitle, r - l, px);
        tx(
            cv,
            ctx,
            (l + r) / 2.0,
            b.h - lb - px * 0.55,
            &t,
            px,
            f.x.title_color.unwrap_or(ctx.fg2),
            Anchor::Middle,
        );
    }
    if y_tw > 0.0 {
        let t = fit(ctx, &ytitle, bb - t0, px);
        tx_rot(
            cv,
            ctx,
            px * 0.7,
            (t0 + bb) / 2.0,
            &t,
            px,
            f.y.title_color.unwrap_or(ctx.fg2),
            Anchor::Middle,
            -90.0,
        );
    }
}

// --- waterfall, scatter (pie, donut, gauge, KPI, funnel, treemap: parts.rs) -----------------------------------------------------

pub fn waterfall_chart(cv: &mut Canvas, ds: &Dataset, b: Bx, f: &Fmt, ctx: &Ctx<'_>) {
    let cd = chart_data(ds, ctx, Some(f), false, 60);
    let Some(se) = cd.series.first().filter(|_| !cd.cats.is_empty()) else {
        return blank_visual(cv, ctx, b, ICON_BAR);
    };
    let mut steps: Vec<(String, f64, f64, f64, bool)> = Vec::new();
    let (mut run, mut lo, mut hi) = (0.0f64, 0.0f64, 0.0f64);
    for (i, c) in cd.cats.iter().enumerate() {
        let x = se.vals.get(i).copied().flatten().unwrap_or(0.0);
        steps.push((c.clone(), run, run + x, x, false));
        run += x;
        lo = lo.min(run);
        hi = hi.max(run);
    }
    steps.push((ctx.t("total", "Total"), 0.0, run, run, true));
    let px = ctx.chart_px;
    let n = steps.len();
    // Desktop's waterfall legend: increase, decrease, total
    let (lt, _) = if f.legend_show != Some(false) {
        let items = [
            (ctx.t("increase", "Increase"), ctx.good()),
            (ctx.t("decrease", "Decrease"), BAD),
            (ctx.t("total", "Total"), ctx.color(0)),
        ];
        place_legend(cv, ctx, f, &items, "", b)
    } else {
        (0.0, 0.0)
    };
    use crate::axes::{self, Bands, line_h};
    let vpx = f.y.px.unwrap_or(px);
    let cpx = f.x.px.unwrap_or(px);
    let names: Vec<String> = steps.iter().map(|s| s.0.clone()).collect();
    let t0 = lt + if f.y.show { line_h(vpx) / 2.0 + 2.5 } else { 4.0 };
    let ch = (b.h - lt).max(1.0);
    let ip = axes::inner_pad(f);
    let mut bb = b.h - 22.5;
    let mut tk = axes::scale(lo, hi, bb - t0, true, vpx, Some(&f.y));
    let mut tl: Vec<String>;
    let mut l = 0.0f32;
    let r = b.w;
    let mut bands = Bands::new(n, 1.0, ip, 0.0);
    let mut cl = None;
    for _ in 0..2 {
        let af = tick_fmt(&tk, &se.col.cf(), ctx, f.y.units);
        tl = tk
            .ticks
            .iter()
            .map(|t| if f.y.show { af(*t) } else { String::new() })
            .collect();
        l = if f.y.show {
            tl.iter().map(|t| tw(ctx, t, vpx)).fold(0.0f32, f32::max) + 10.0
        } else {
            2.0
        };
        bands = Bands::new(n, r - l, ip, 0.0);
        let c =
            f.x.show
                .then(|| axes::cat_labels(ctx, &names, bands.step, cpx, ch, f));
        bb = b.h - c.as_ref().map_or(4.0, |c| c.h);
        cl = c;
        tk = axes::scale(lo, hi, bb - t0, true, vpx, Some(&f.y));
    }
    let af = tick_fmt(&tk, &se.col.cf(), ctx, f.y.units);
    tl = tk
        .ticks
        .iter()
        .map(|t| if f.y.show { af(*t) } else { String::new() })
        .collect();
    if r - l < 8.0 || bb - t0 < 8.0 {
        return;
    }
    let y = |x: f64| bb - tk.f(x) * (bb - t0);
    for (k, t) in tk.ticks.iter().enumerate() {
        if f.y.gridlines {
            cv.line(l, y(*t), r, y(*t), ctx.grid, 1.0, GRID_DASH);
        }
        if let Some(s) = tl.get(k).filter(|s| !s.is_empty()) {
            tx(
                cv,
                ctx,
                l - 10.0,
                y(*t),
                s,
                vpx,
                f.y.color.unwrap_or(ctx.muted),
                Anchor::End,
            );
        }
    }
    let gw = bands.bar;
    for (i, (_, a, e, x, total)) in steps.iter().enumerate() {
        let (ya, yb) = (y(*a), y(*e));
        let c = if *total {
            ctx.color(0)
        } else if *x >= 0.0 {
            ctx.good()
        } else {
            BAD
        };
        let x0 = l + bands.x(i);
        cv.rect(x0, ya.min(yb), gw, (ya - yb).abs().max(0.5), c, 1.0);
        // connector from this bar's start to the next bar's end, at this bar's end
        if i + 1 < n {
            cv.line(x0, yb, x0 + bands.step + gw, yb, Rgb(0xa6, 0xa6, 0xa6), 1.0, None);
        }
    }
    if let Some(cl) = &cl {
        axes::draw_cat_labels(
            cv,
            ctx,
            cl,
            &names,
            n,
            bands.step,
            f.x.color.unwrap_or(ctx.muted),
            &|i| l + bands.mid(i),
            bb,
        );
    }
}

pub fn scatter_chart(cv: &mut Canvas, ds: &Dataset, b: Bx, f: &Fmt, ctx: &Ctx<'_>) {
    let (cols, rows) = (&ds.cols, &ds.rows);
    let at = |r: &str| cols.iter().position(|c| c.role.to_lowercase() == r);
    let (Some(xi), Some(yi)) = (at("x"), at("y")) else {
        return blank_visual(cv, ctx, b, ICON_SCATTER);
    };
    let si = at("size");
    let li = cols.iter().position(|c| c.rc == Rc::Leg);
    let mut pts: Vec<(f64, f64, Option<f64>, Value)> = Vec::new();
    for r in rows {
        if pts.len() >= 3000 {
            break;
        }
        if let (Some(x), Some(y)) = (r.get(xi).and_then(fin), r.get(yi).and_then(fin)) {
            let z = si.and_then(|s| r.get(s)).and_then(fin);
            let l = li.and_then(|l| r.get(l)).cloned().unwrap_or(Value::Null);
            pts.push((x, y, z, l));
        }
    }
    if pts.is_empty() {
        return blank_visual(cv, ctx, b, ICON_SCATTER);
    }
    let px = ctx.chart_px;
    let mut legs: Vec<Value> = Vec::new();
    if li.is_some() {
        let mut seen = std::collections::HashSet::new();
        for p in &pts {
            if seen.insert(key_of(&p.3)) {
                legs.push(p.3.clone());
            }
        }
        legs.sort_by(cmp_raw);
        legs.truncate(12);
    }
    let leg_at: std::collections::HashMap<String, usize> =
        legs.iter().enumerate().map(|(i, l)| (key_of(l), i)).collect();
    let its: Vec<(String, Rgb)> = legs
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let t = fmt_cell(
                l,
                li.and_then(|li| cols.get(li)).map(|c| c.cf()).as_ref(),
                &ctx.loc,
                false,
            );
            (if t.is_empty() { ctx.blank() } else { t }, ctx.color(i))
        })
        .collect();
    let (lt, lb) = if !its.is_empty() && f.legend_show != Some(false) {
        place_legend(cv, ctx, f, &its, "", b)
    } else {
        (0.0, 0.0)
    };
    // Desktop's scatter axes hug the data: from half a step under the smallest value to the
    // largest, with axis titles under and beside them
    let ext = |k: usize| {
        let vals = pts.iter().map(|p| if k == 0 { p.0 } else { p.1 });
        (
            vals.clone().fold(f64::INFINITY, f64::min),
            vals.fold(f64::NEG_INFINITY, f64::max),
        )
    };
    let tight = |lo: f64, hi: f64, n: usize| {
        let mut t = nice_ticks(lo, hi, n);
        let half = t.step / 2.0;
        let a = if half > 0.0 {
            (lo / half).floor() * half
        } else {
            lo
        };
        let z = hi.max(a + t.step);
        t.ticks
            .retain(|x| *x >= a - t.step * 1e-6 && *x <= z + t.step * 1e-6);
        t.lo = a;
        t.hi = if z > a { z } else { a + 1.0 };
        t
    };
    let titles = f.x.show && f.y.show;
    // each axis at its own font size (`x_axis` / `y_axis` `font_size`, as bar and line charts)
    let (xpx, ypx) = (f.x.px.unwrap_or(px), f.y.px.unwrap_or(px));
    let x_th = if titles { xpx * 1.5 } else { 0.0 };
    let y_tw = if titles { ypx * 1.5 } else { 0.0 };
    let (ylo, yhi) = ext(1);
    let tky = tight(
        ylo,
        yhi,
        (((b.h - lt - lb) / (ypx * 6.0)).floor() as usize).clamp(2, 8),
    );
    let ycol = cols.get(yi).map(|c| c.cf());
    let fy = crate::fmt::axis_fmt_units(tky.lo, tky.hi, tky.step, ycol.as_ref(), &ctx.loc, f.y.units);
    let tly: Vec<String> = tky.ticks.iter().map(|t| fy(*t)).collect();
    let l = y_tw + tly.iter().map(|t| tw(ctx, t, ypx)).fold(0.0f32, f32::max) + ypx * 0.5;
    let (xlo, xhi) = ext(0);
    let tkx = tight(xlo, xhi, (((b.w - l) / (xpx * 7.0)).floor() as usize).clamp(2, 8));
    let xcol = cols.get(xi).map(|c| c.cf());
    let fx = crate::fmt::axis_fmt_units(tkx.lo, tkx.hi, tkx.step, xcol.as_ref(), &ctx.loc, f.x.units);
    let (r, t0, bb) = (b.w - xpx, lt + ypx * 0.6, b.h - lb - xpx * 1.5 - x_th);
    if r - l < 8.0 || bb - t0 < 8.0 {
        return;
    }
    if titles {
        let xt = fit(ctx, cols.get(xi).map_or("", |c| c.name.as_str()), r - l, xpx);
        tx(
            cv,
            ctx,
            (l + r) / 2.0,
            b.h - lb - xpx * 0.55,
            &xt,
            xpx,
            f.x.title_color.unwrap_or(ctx.fg2),
            Anchor::Middle,
        );
        let yt = fit(ctx, cols.get(yi).map_or("", |c| c.name.as_str()), bb - t0, ypx);
        tx_rot(
            cv,
            ctx,
            ypx * 0.7,
            (t0 + bb) / 2.0,
            &yt,
            ypx,
            f.y.title_color.unwrap_or(ctx.fg2),
            Anchor::Middle,
            -90.0,
        );
    }
    let xf = |x: f64| l + (((x.clamp(tkx.lo, tkx.hi) - tkx.lo) / (tkx.hi - tkx.lo)) as f32) * (r - l);
    let yf = |y: f64| bb - (((y.clamp(tky.lo, tky.hi) - tky.lo) / (tky.hi - tky.lo)) as f32) * (bb - t0);
    for (k, t) in tky.ticks.iter().enumerate() {
        if f.y.gridlines {
            cv.line(l, yf(*t), r, yf(*t), ctx.grid, 1.0, GRID_DASH);
        }
        if let Some(s) = tly.get(k) {
            tx(cv, ctx, l - ypx * 0.4, yf(*t), s, ypx, ctx.muted, Anchor::End);
        }
    }
    for t in &tkx.ticks {
        if f.y.gridlines {
            cv.line(xf(*t), t0, xf(*t), bb, ctx.grid, 1.0, GRID_DASH);
        }
        tx(
            cv,
            ctx,
            xf(*t),
            bb + xpx * 0.85,
            &fx(*t),
            xpx,
            ctx.muted,
            Anchor::Middle,
        );
    }
    // symmetry shading: the plot above y = x and below it, under the points
    if let Some(sh) = f.lines.iter().find(|l| sk(l, "kind") == "symmetry_shading") {
        let alpha = 1.0 - clampf(nk(sh, "transparency", 70.0) as f32, 0.0, 100.0) / 100.0;
        let rect = [
            (tkx.lo, tky.lo),
            (tkx.hi, tky.lo),
            (tkx.hi, tky.hi),
            (tkx.lo, tky.hi),
        ];
        for (key, sign, fallback) in [
            ("upper_color", 1.0, ctx.color(0)),
            ("lower_color", -1.0, ctx.color(1)),
        ] {
            let poly = clip_half(&rect, &|x, y| sign * (y - x));
            let c = Rgb::from_value(crate::data::get(sh, key)).unwrap_or(fallback);
            let px_pts: Vec<(f32, f32)> = poly.iter().map(|(x, y)| (xf(*x), yf(*y))).collect();
            if px_pts.len() >= 3 {
                cv.polygon(&px_pts, c, alpha);
            }
        }
    }
    let zmax = pts.iter().map(|p| p.2.unwrap_or(0.0).abs()).fold(0.0, f64::max);
    for p in &pts {
        let rad = match p.2 {
            Some(z) if zmax > 0.0 => 3.0 + ((z.abs() / zmax).sqrt() as f32) * (18.0f32).min(b.h / 12.0),
            _ => 6.5,
        };
        let li2 = if li.is_some() {
            leg_at.get(&key_of(&p.3)).copied().unwrap_or(0)
        } else {
            0
        };
        cv.circle(xf(p.0), yf(p.1), rad, Some((ctx.color(li2), 0.85)), None);
    }
    if let Some(l) = f.lines.iter().find(|l| sk(l, "kind") == "trend") {
        let (mut k, mut sx, mut sy, mut sxx, mut sxy) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for p in &pts {
            k += 1.0;
            sx += p.0;
            sy += p.1;
            sxx += p.0 * p.0;
            sxy += p.0 * p.1;
        }
        let d = k * sxx - sx * sx;
        if k >= 2.0 && d != 0.0 {
            let bb2 = (k * sxy - sx * sy) / d;
            let a = (sy - bb2 * sx) / k;
            if a.is_finite() && bb2.is_finite() {
                let fit = Fit {
                    a,
                    b: bb2,
                    t0: tkx.lo,
                    t1: tkx.hi,
                    color: Rgb::from_value(crate::data::get(l, "color")).unwrap_or(ctx.fg2),
                    width: clampf(nk(l, "width", 1.5) as f32, 0.5, 10.0),
                    dash: dash_of(sk(l, "style")),
                };
                draw_trends(cv, &[fit], &|t, v| (xf(t), yf(v)), tky.lo, tky.hi);
            }
        }
    }
    // constant and statistical lines: on y (horizontal) or x (vertical, `axis: "x"` or a line
    // following the X measure)
    let in_x = |v: f64| v >= tkx.lo && v <= tkx.hi;
    let in_y = |v: f64| v >= tky.lo && v <= tky.hi;
    for (vertical, x, rl) in scatter_lines(f, &pts, xi) {
        if vertical && in_x(x) {
            cv.line(xf(x), t0, xf(x), bb, rl.color, rl.width, rl.dash);
            if rl.label {
                tx(
                    cv,
                    ctx,
                    xf(x) + px * 0.3,
                    t0 + px * 0.6,
                    &fx(x),
                    px,
                    rl.color,
                    Anchor::Start,
                );
            }
        } else if !vertical && in_y(x) {
            cv.line(l, yf(x), r, yf(x), rl.color, rl.width, rl.dash);
            if rl.label {
                tx(
                    cv,
                    ctx,
                    r - 2.0,
                    yf(x) - px * 0.6,
                    &fy(x),
                    px,
                    rl.color,
                    Anchor::End,
                );
            }
        }
    }
    // the ratio line: from the origin through (Σx, Σy) (BIcraft's reading of Desktop's)
    if let Some(rl) = f.lines.iter().find(|l| sk(l, "kind") == "ratio_line") {
        let (sx, sy) = pts.iter().fold((0.0, 0.0), |a, p| (a.0 + p.0, a.1 + p.1));
        if sx != 0.0 && (sy / sx).is_finite() {
            let k = sy / sx;
            let fit = Fit {
                a: 0.0,
                b: k,
                t0: tkx.lo,
                t1: tkx.hi,
                color: Rgb::from_value(crate::data::get(rl, "color")).unwrap_or(ctx.fg2),
                width: clampf(nk(rl, "width", 1.5) as f32, 0.5, 10.0),
                dash: dash_of(sk(rl, "style")),
            };
            draw_trends(cv, &[fit], &|t, v| (xf(t), yf(v)), tky.lo, tky.hi);
        }
    }
}

/// The scatter chart's constant and statistical lines: (vertical, value, style).
fn scatter_lines(f: &Fmt, pts: &[(f64, f64, Option<f64>, Value)], xi: usize) -> Vec<(bool, f64, RefLine)> {
    let mut out = Vec::new();
    for l in &f.lines {
        let kind = sk(l, "kind");
        let col = l.get("column").and_then(Value::as_u64);
        let vertical = match kind {
            "constant" => sk(l, "axis") == "x",
            "average" | "min" | "max" | "median" | "percentile" => col == u64::try_from(xi).ok(),
            _ => continue,
        };
        let x = if kind == "constant" {
            nk(l, "value", f64::NAN)
        } else {
            let mut xs: Vec<f64> = pts.iter().map(|p| if vertical { p.0 } else { p.1 }).collect();
            if xs.is_empty() {
                continue;
            }
            xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let q = |p: f64| {
                let k = p.clamp(0.0, 1.0) * (xs.len() - 1) as f64;
                let i = k.floor() as usize;
                let a = xs.get(i).copied().unwrap_or(0.0);
                let b = xs.get(i + 1).copied().unwrap_or(a);
                a + (b - a) * (k - i as f64)
            };
            match kind {
                "average" => xs.iter().sum::<f64>() / xs.len() as f64,
                "min" => xs.first().copied().unwrap_or(0.0),
                "max" => xs.last().copied().unwrap_or(0.0),
                "median" => q(0.5),
                _ => {
                    let p = nk(l, "percentile", 50.0);
                    q(if p > 1.0 { p / 100.0 } else { p })
                }
            }
        };
        if !x.is_finite() {
            continue;
        }
        out.push((
            vertical,
            x,
            RefLine {
                x,
                color: Rgb::from_value(crate::data::get(l, "color")).unwrap_or(Rgb(0x60, 0x5e, 0x5c)),
                width: clampf(nk(l, "width", 1.5) as f32, 0.5, 10.0),
                dash: dash_of(sk(l, "style")),
                label: crate::data::get(l, "data_label") == &Value::Bool(true),
            },
        ));
    }
    out
}

/// The part of the polygon `poly` where `side(x, y) >= 0` (one Sutherland–Hodgman pass).
fn clip_half(poly: &[(f64, f64)], side: &dyn Fn(f64, f64) -> f64) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    let n = poly.len();
    for i in 0..n {
        let (Some(&a), Some(&b)) = (poly.get(i), poly.get((i + 1) % n)) else {
            continue;
        };
        let (sa, sb) = (side(a.0, a.1), side(b.0, b.1));
        if sa >= 0.0 {
            out.push(a);
        }
        if (sa >= 0.0) != (sb >= 0.0) && sa != sb {
            let t = sa / (sa - sb);
            out.push((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_are_nice() {
        let t = nice_ticks(0.0, 87.0, 5);
        assert_eq!(t.ticks, vec![0.0, 20.0, 40.0, 60.0, 80.0, 100.0]);
        let t = nice_ticks(f64::NAN, 3.0, 5);
        assert_eq!(t.lo, 0.0);
        let t = nice_ticks(5.0, 5.0, 4);
        assert_eq!(t.lo, 0.0);
        assert!(t.hi >= 5.0);
    }

    #[test]
    fn log_scale_positions() {
        let ax = crate::data::Axis {
            show: true,
            log: true,
            ..Default::default()
        };
        let t = value_scale(1.0, 1000.0, 4, Some(&ax));
        assert_eq!(t.ticks.len(), 4);
        assert!((t.f(10.0) - 1.0 / 3.0).abs() < 1e-6);
        assert_eq!(t.f(-5.0), 0.0);
    }
}
