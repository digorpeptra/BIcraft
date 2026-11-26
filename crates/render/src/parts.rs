//! Part-of-whole and indicator charts: pie, donut, gauge, KPI, funnel and treemap, laid out like
//! Power BI Desktop 2.158 (measured with `cargo xtask oracle-gen parts`; docs/render.md).

use serde_json::Value;

use crate::canvas::{Anchor, Base, Canvas, Rgb, Text, annulus};
use crate::charts::{
    Bx, ICON_BAR, ICON_GAUGE, ICON_KPI, ICON_PIE, ICON_TABLE, blank_visual, fit, font, lbl, place_legend, tx,
    tx_w,
};
use crate::data::{BAD, Ctx, Dataset, Fmt, PT, Rc, chart_data, clampf, fin, get, is_num_col, sk};
use crate::fmt::{ColFmt, fmt_kpi, fmt_label, fmt_val};
use crate::hits;

/// Desktop's pie, donut and funnel labels: 12 px, in `wf_standard-font` (DIN) in the classic
/// look (Desktop 2.158 DOM; DIN is not a system font, so Segoe UI stands in). Themed reports
/// label in the theme's font.
const LABEL_PX: f32 = 12.0;

fn label_face(ctx: &Ctx<'_>, px: f32) -> crate::fonts::Font {
    if ctx.classic {
        crate::fonts::Font::new(&format!("DIN, {}", ctx.font), px)
    } else {
        font(ctx, px)
    }
}

/// A label in [`label_face`], centred vertically on `y`.
#[allow(clippy::too_many_arguments)]
fn label_tx(cv: &mut Canvas, ctx: &Ctx<'_>, x: f32, y: f32, s: &str, px: f32, color: Rgb, anchor: Anchor) {
    cv.text(
        ctx.fonts,
        &Text::new(s, label_face(ctx, px), color)
            .anchor(anchor)
            .base(Base::Central),
        x,
        y,
    );
}

/// Desktop's plot area in body coordinates: the body itself (the container already places it
/// where Desktop's plot area is: classic charts 5 px padding, 4 px under the title).
fn viewport(_f: &Fmt, _ctx: &Ctx<'_>, b: Bx) -> (f32, f32, f32, f32) {
    (0.0, 0.0, b.w, b.h)
}

/// The `format.labels` font: size (pt), colour, bold.
fn label_font(f: &Fmt, default_px: f32, default_color: Rgb) -> (f32, Rgb, u16) {
    let l = get(&f.raw, "labels");
    let px = fin(get(l, "font_size")).map_or(default_px, |s| clampf(s as f32, 5.0, 40.0) * PT);
    let color = Rgb::from_value(get(l, "color")).unwrap_or(default_color);
    let bold = get(l, "bold").as_bool() == Some(true);
    (px, color, if bold { 700 } else { 400 })
}

// --- pie / donut -----------------------------------------------------------------------------

struct Slice {
    name: String,
    v: f64,
    col: ColFmt,
    color: Rgb,
    /// The category and series it comes from (hits, highlights).
    i: Option<usize>,
    j: Option<usize>,
}

/// A slice label: its parts (first line, rest) per Desktop's label style.
fn slice_text(s: &Slice, share: f64, style: &str, f: &Fmt, ctx: &Ctx<'_>) -> (String, String) {
    let l = get(&f.raw, "labels");
    let pp = fin(get(l, "percent_precision")).map_or(2, |p| p.clamp(0.0, 6.0) as usize);
    let pct = || ctx.loc.percent(share, pp, pp);
    let val = || lbl(s.v, Some(&s.col), f, ctx);
    let name = s.name.clone();
    match style.to_ascii_lowercase().as_str() {
        "data" => (val(), String::new()),
        "percent of total" => (pct(), String::new()),
        "both" => (name, val()),
        "category, percent of total" => (name, pct()),
        "data value, percent of total" => (val(), format!("({})", pct())),
        "category, data value, percent of total" => (name, format!("{} ({})", val(), pct())),
        _ => (name, String::new()),
    }
}

struct PieLabel {
    /// Slice edge point and wanted elbow.
    p1: (f32, f32),
    p2: (f32, f32),
    right: bool,
    lines: Vec<String>,
    y: f32,
    h: f32,
    v: f64,
}

pub fn pie_chart(cv: &mut Canvas, ds: &Dataset, b: Bx, f: &Fmt, ctx: &Ctx<'_>, donut: bool) {
    let cd = chart_data(ds, ctx, Some(f), true, 60);
    let by_series = cd.cats.len() <= 1 && cd.series.len() > 1;
    let mut sl: Vec<Slice> = if by_series {
        cd.series
            .iter()
            .enumerate()
            .map(|(j, s)| Slice {
                name: s.name.clone(),
                v: s.vals.first().copied().flatten().unwrap_or(f64::NAN),
                col: s.col.cf(),
                color: s.color,
                i: None,
                j: Some(j),
            })
            .collect()
    } else if let Some(s0) = cd.series.first() {
        // Desktop colours slices in the categories' own order (its query order), whatever the
        // sort that lays them out
        let mut order: Vec<usize> = (0..cd.cats.len()).collect();
        order.sort_by(|a, b| match (cd.raw.get(*a), cd.raw.get(*b)) {
            (Some(x), Some(y)) => crate::data::cmp_raw(x, y),
            _ => a.cmp(b),
        });
        let mut rank = vec![0usize; cd.cats.len()];
        for (k, i) in order.iter().enumerate() {
            if let Some(r) = rank.get_mut(*i) {
                *r = k;
            }
        }
        cd.cats
            .iter()
            .enumerate()
            .map(|(i, c)| Slice {
                name: c.clone(),
                v: s0.vals.get(i).copied().flatten().unwrap_or(f64::NAN),
                col: s0.col.cf(),
                color: s0
                    .pts
                    .get(i)
                    .copied()
                    .flatten()
                    .unwrap_or_else(|| ctx.color(rank.get(i).copied().unwrap_or(i))),
                i: Some(i),
                j: None,
            })
            .collect()
    } else {
        Vec::new()
    };
    sl.retain(|x| x.v.is_finite() && x.v > 0.0);
    if sl.is_empty() {
        return blank_visual(cv, ctx, b, ICON_PIE);
    }
    let lab = get(&f.raw, "labels");
    let its: Vec<(String, Rgb)> = sl.iter().map(|x| (x.name.clone(), x.color)).collect();
    // Desktop 2.158: no legend unless the report turns it on; the labels name the slices
    let (lt, lb) = if f.legend_show == Some(true) {
        place_legend(cv, ctx, f, &its, &cd.cat_name, b)
    } else {
        (0.0, 0.0)
    };
    let total: f64 = sl.iter().map(|x| x.v).sum();
    let (vx, vy0, vw, vh0) = viewport(f, ctx, b);
    let vy = if lt > 0.0 { lt } else { vy0 };
    let vh = (vy0 + vh0 - lb - vy).max(1.0);
    let show_lab = f.labels != Some(false) && vw > 40.0 && vh > 30.0;
    // radius: 80 % of min(w, h) / 2, or with labels / (2 + a logistic of the aspect): BIcraft's
    // empirical formula, checked against the parts render oracle's pie captures (docs/render.md)
    let small = vw.min(vh);
    let denom = if show_lab {
        let hw = vh / vw.max(1.0);
        2.0 + 1.0 / (1.0 + (-5.0 * (hw - 1.0)).exp())
    } else {
        2.0
    };
    let r = small / denom * 0.8;
    if r.is_nan() || r < 4.0 {
        return;
    }
    let (cx, cy) = (vx + vw / 2.0, vy + vh / 2.0);
    // `format.pie.inner_radius`: the donut hole in percent of the radius (Desktop: 60)
    let pie = get(&f.raw, "pie");
    let ratio = fin(get(pie, "inner_radius"))
        .map(|x| if x > 1.0 { x / 100.0 } else { x })
        .map(|x| clampf(x as f32, 0.0, 0.95));
    let ri = if donut { r * ratio.unwrap_or(0.6) } else { 0.0 };
    // `format.pie.start_angle`: degrees clockwise from 12 o'clock
    let start = fin(get(pie, "start_angle")).map_or(0.0, |a| (a.clamp(-3600.0, 3600.0) as f32).to_radians());
    let mut a = -std::f32::consts::FRAC_PI_2 + start;
    let mut mids = Vec::with_capacity(sl.len());
    for s in &sl {
        let da = (s.v / total) as f32 * std::f32::consts::TAU;
        if da > 0.0 {
            let pt = hits::cd_point(&cd, s.i, s.j);
            // highlighted: the slice's highlighted share as that share of its area, from the centre
            let h = s.i.and_then(|i| hits::hv(&cd, i, 0));
            let alpha = if h.is_some() { hits::DIM } else { ctx.ix.alpha(&pt) };
            if let Some(p) = annulus(cx, cy, ri, r, a, a + da) {
                cv.fill_path(&p, s.color, alpha);
            }
            if let Some(h) = h {
                let ratio = match h {
                    Some(h) if s.v > 0.0 && h.is_finite() => (h / s.v).clamp(0.0, 1.0) as f32,
                    _ => 0.0,
                };
                if ratio > 0.0
                    && let Some(p) = annulus(
                        cx,
                        cy,
                        ri,
                        (ri * ri + ratio * (r * r - ri * ri)).sqrt(),
                        a,
                        a + da,
                    )
                {
                    cv.fill_path(&p, s.color, 1.0);
                }
            }
            if ctx.ix.on() {
                let q = std::f32::consts::FRAC_PI_2;
                let shape = hits::HitShape::Sector {
                    cx,
                    cy,
                    r0: ri,
                    r1: r,
                    a0: a + q,
                    a1: a + da + q,
                };
                let tip = hits::cd_tip(&cd, s.i.unwrap_or(0), s.j.unwrap_or(0), ctx);
                cv.hit(ctx.ix.hit(shape, ctx.ix.select(&pt), tip));
            }
        }
        mids.push(a + da / 2.0);
        a += da;
    }
    if !show_lab {
        return;
    }
    let style = {
        let s = sk(lab, "label_style");
        if s.is_empty() && !ctx.classic {
            "Category, percent of total"
        } else {
            s
        }
    };
    let pos = sk(lab, "position");
    let inside = matches!(pos, "inside" | "insidecenter" | "preferinside");
    let (lpx, lcolor, lweight) = label_font(f, LABEL_PX, ctx.muted);
    let line_h = lpx * 1.05;
    if inside {
        // inside labels: white on a half-transparent black box at the slice's centroid
        let rc = (ri + r) / 2.0;
        let ring = r - ri;
        // Desktop 2.158 draws them at the label weight (400 unless `labels.bold`)
        let bold = label_face(ctx, lpx).weight(lweight);
        for (s, m) in sl.iter().zip(&mids) {
            let (l1, l2) = slice_text(s, s.v / total, style, f, ctx);
            let one = if l2.is_empty() {
                l1.clone()
            } else {
                format!("{l1} {l2}")
            };
            let lines = if !l2.is_empty() && ctx.fonts.measure(&bold, &one) > ring {
                vec![l1, l2]
            } else {
                vec![one]
            };
            let w = lines
                .iter()
                .map(|t| ctx.fonts.measure(&bold, t))
                .fold(0.0f32, f32::max);
            let h = lines.len() as f32 * lpx * 1.1 + 2.0;
            let (x, y) = (cx + m.cos() * rc, cy + m.sin() * rc);
            cv.round_rect(
                x - w / 2.0 - 4.0,
                y - h / 2.0,
                w + 8.0,
                h,
                3.0,
                Some((Rgb(0, 0, 0), 0.5)),
                None,
            );
            for (k, t) in lines.iter().enumerate() {
                let ly = y - h / 2.0 + 1.0 + (k as f32 + 0.5) * lpx * 1.1;
                cv.text(
                    ctx.fonts,
                    &Text::new(t, bold.clone(), Rgb(255, 255, 255))
                        .anchor(Anchor::Middle)
                        .base(Base::Central),
                    x,
                    ly,
                );
            }
        }
        return;
    }
    // outside labels with leader lines: a radial segment to 1.13 r, then 13 px across
    let lf = label_face(ctx, lpx).weight(lweight);
    let mut labels: Vec<PieLabel> = Vec::new();
    for (s, m) in sl.iter().zip(&mids) {
        let (c, si) = (m.cos(), m.sin());
        let right = c >= 0.0;
        let p1 = (cx + c * r, cy + si * r);
        let p2 = (cx + c * r * 1.13, cy + si * r * 1.13);
        let tx0 = p2.0 + if right { 17.0 } else { -17.0 };
        // the text may run to the visual's edge (the body's padding)
        let room = if right { b.w + 5.0 - tx0 } else { tx0 + 5.0 }.max(0.0);
        let (l1, l2) = slice_text(s, s.v / total, style, f, ctx);
        let one = if l2.is_empty() {
            l1.clone()
        } else {
            format!("{l1} {l2}")
        };
        let lines = if ctx.fonts.measure(&lf, &one) <= room || l2.is_empty() {
            vec![ctx.fonts.fit(&lf, &one, room)]
        } else {
            vec![ctx.fonts.fit(&lf, &l1, room), ctx.fonts.fit(&lf, &l2, room)]
        };
        if lines.iter().all(|t| t.is_empty()) {
            continue;
        }
        let h = lines.len() as f32 * line_h;
        labels.push(PieLabel {
            p1,
            p2,
            right,
            lines,
            y: p2.1,
            h,
            v: s.v,
        });
    }
    // collision avoidance per side: drop the smallest slices' labels until they fit, then spread
    let (top, bottom) = (vy - 2.0, vy + vh + 2.0);
    for right in [false, true] {
        loop {
            let need: f32 = labels
                .iter()
                .filter(|l| l.right == right)
                .map(|l| l.h + 1.0)
                .sum();
            if need <= bottom - top {
                break;
            }
            let Some(k) = labels
                .iter()
                .enumerate()
                .filter(|(_, l)| l.right == right)
                .min_by(|a, b| a.1.v.partial_cmp(&b.1.v).unwrap_or(std::cmp::Ordering::Equal))
                .map(|(k, _)| k)
            else {
                break;
            };
            labels.remove(k);
        }
        let mut idx: Vec<usize> = (0..labels.len())
            .filter(|k| labels.get(*k).is_some_and(|l| l.right == right))
            .collect();
        idx.sort_by(|a, b| {
            let ya = labels.get(*a).map_or(0.0, |l| l.y);
            let yb = labels.get(*b).map_or(0.0, |l| l.y);
            ya.partial_cmp(&yb).unwrap_or(std::cmp::Ordering::Equal)
        });
        // downwards: no overlap
        let mut prev: Option<(f32, f32)> = None;
        for k in &idx {
            if let Some(l) = labels.get_mut(*k) {
                let lo = match prev {
                    Some((py, ph)) => py + (ph + l.h) / 2.0 + 1.0,
                    None => top + l.h / 2.0,
                };
                l.y = l.y.max(lo);
                prev = Some((l.y, l.h));
            }
        }
        // upwards: inside the plot
        let mut next: Option<(f32, f32)> = None;
        for k in idx.iter().rev() {
            if let Some(l) = labels.get_mut(*k) {
                let hi = match next {
                    Some((ny, nh)) => ny - (nh + l.h) / 2.0 - 1.0,
                    None => bottom - l.h / 2.0,
                };
                l.y = l.y.min(hi);
                next = Some((l.y, l.h));
            }
        }
    }
    let line_c = if ctx.classic {
        Rgb(0x66, 0x66, 0x66)
    } else {
        ctx.muted
    };
    for l in &labels {
        let dir = if l.right { 1.0 } else { -1.0 };
        let p2 = (l.p2.0, l.y);
        let p3 = (p2.0 + dir * 13.0, l.y);
        cv.polyline(&[l.p1, p2, p3], line_c, 1.0, 1.0, None);
        let x = p3.0 + dir * 4.0;
        let n = l.lines.len() as f32;
        for (k, t) in l.lines.iter().enumerate() {
            let y = l.y + (k as f32 - (n - 1.0) / 2.0) * line_h;
            cv.text(
                ctx.fonts,
                &Text::new(t, lf.clone(), lcolor)
                    .anchor(if l.right { Anchor::Start } else { Anchor::End })
                    .base(Base::Central),
                x,
                y,
            );
        }
    }
}

// --- gauge -----------------------------------------------------------------------------------

/// A gauge number: the format string for percentages, else auto display units (picked from the
/// value, or from `scale` for zero) with no decimals, unless `fmt` (a `callout`/`labels`-like
/// object) sets units or precision.
fn gauge_num(v: f64, scale: f64, col: Option<&ColFmt>, fmt: &Value, ctx: &Ctx<'_>) -> String {
    let units = fin(get(fmt, "display_units")).filter(|u| *u != 0.0);
    let prec = fin(get(fmt, "precision")).map(|p| p.clamp(0.0, 10.0) as usize);
    let pct = col.is_some_and(|c| crate::fmt::fmt_parts(&c.fmt).pct);
    // percentages keep their format string (Desktop ignores label units and precision here)
    if pct {
        return fmt_val(v, col, &ctx.loc, true);
    }
    let auto = |x: f64| {
        let a = x.abs();
        if a >= 1e12 {
            1e12
        } else if a >= 1e9 {
            1e9
        } else if a >= 1e6 {
            1e6
        } else if a >= 1e3 {
            1e3
        } else {
            1.0
        }
    };
    let u = units.unwrap_or_else(|| auto(if v == 0.0 { scale } else { v }));
    let u = if u < 0.0 {
        auto(if v == 0.0 { scale } else { v })
    } else {
        u
    };
    fmt_label(v, col, &ctx.loc, Some(u), Some(prec.unwrap_or(0)))
}

pub fn gauge_chart(cv: &mut Canvas, ds: &Dataset, b: Bx, f: &Fmt, ctx: &Ctx<'_>) {
    // the report's gauge axis (`format.gauge`: literal min, max, target) wins over the fields
    let g = get(&f.raw, "gauge");
    let (cols, rows) = (&ds.cols, &ds.rows);
    let empty = Vec::new();
    let r0 = rows.first().unwrap_or(&empty);
    let at = |rc: Rc| {
        cols.iter()
            .enumerate()
            .position(|(i, c)| c.rc == rc && is_num_col(c, rows, i))
    };
    let vi = at(Rc::Val).or_else(|| {
        cols.iter().enumerate().position(|(i, c)| {
            is_num_col(c, rows, i) && !matches!(c.rc, Rc::Min | Rc::Max | Rc::Goal | Rc::Tip)
        })
    });
    let Some(vi) = vi else {
        return blank_visual(cv, ctx, b, ICON_GAUGE);
    };
    let cell = |i: Option<usize>| i.and_then(|i| r0.get(i)).and_then(fin);
    // a bound is a literal, or `{field, column}` (Desktop's fx binding) read from the data
    let bound = |k: &str| {
        let x = get(g, k);
        fin(x).or_else(|| fin(get(x, "column")).and_then(|c| cell(Some(c.clamp(0.0, 4096.0) as usize))))
    };
    let col = cols.get(vi).map(|c| c.cf());
    let value = cell(Some(vi)).unwrap_or(0.0);
    let lo = bound("min").or(cell(at(Rc::Min))).unwrap_or(0.0);
    let mut hi = bound("max").or(cell(at(Rc::Max))).unwrap_or(f64::NAN);
    let pct = col.as_ref().is_some_and(|c| crate::fmt::fmt_parts(&c.fmt).pct);
    if (hi.is_nan() || hi <= lo) && pct && (0.0..=1.0).contains(&value) && lo < 1.0 {
        // a percentage gauge without a maximum runs to 100 % (Desktop 2.158)
        hi = 1.0;
    }
    if hi.is_nan() || hi <= lo {
        hi = if value > lo {
            lo + (value - lo) * 2.0
        } else {
            lo + 1.0
        };
    }
    let target = bound("target").or(cell(at(Rc::Goal)));
    let pi = std::f32::consts::PI;
    let ang = |x: f64| pi + pi * (((x - lo) / (hi - lo)).clamp(0.0, 1.0) as f32);
    let callout = get(g, "callout");
    let tl = get(g, "target_label");
    let show_target_label = tl.get("show").and_then(Value::as_bool) != Some(false);
    let (vx, vy, vw, vh) = viewport(f, ctx, b);
    // room the target label needs above the arc's top
    let lift = |r: f32| match target {
        Some(t) if show_target_label => ((r + 8.0) * ang(t).sin().abs() + 13.0 - r).max(0.0),
        _ => 0.0,
    };
    let below = 21.0; // min / max labels under the ends
    let half = vw / 2.0;
    let (r, cy, beside) = if vh - lift(half) - below >= half {
        // width-bound: the arc and its labels centred
        let t = lift(half);
        let top = vy + (vh - (t + half + below)) / 2.0 + t;
        (half, top + half, false)
    } else {
        // height-bound: min and max beside the ends
        let mut r = vh - 6.0;
        for _ in 0..3 {
            r = (vh - lift(r) - 6.0).min(half - 30.0);
        }
        let t = lift(r);
        (r, vy + t + 0.7 + r, true)
    };
    if r.is_nan() || r < 6.0 {
        return;
    }
    let th = r * 0.3;
    let inner = r - th;
    let cx = vx + vw / 2.0;
    if let Some(p) = annulus(cx, cy, inner, r, pi, pi * 2.0) {
        cv.fill_path(&p, ctx.track, 1.0);
    }
    if ang(value) > pi + 1e-3
        && let Some(p) = annulus(cx, cy, inner, r, pi, ang(value))
    {
        cv.fill_path(&p, ctx.color(0), 1.0);
    }
    let spx = 10.67 * 0.92;
    let lab = get(&f.raw, "labels");
    let lcolor = Rgb::from_value(get(lab, "color")).unwrap_or(ctx.muted);
    let scale = lo.abs().max(hi.abs());
    if let Some(t) = target {
        let a = ang(t);
        let (c, si) = (a.cos(), a.sin());
        let tcol = Rgb::from_value(get(g, "target_color")).unwrap_or(if ctx.classic {
            Rgb(0x37, 0x46, 0x49)
        } else {
            ctx.fg
        });
        cv.line(
            cx + inner * c,
            cy + inner * si,
            cx + r * c,
            cy + r * si,
            tcol,
            2.0,
            None,
        );
        if show_target_label {
            let text = gauge_num(t, scale, col.as_ref(), tl, ctx);
            let tpx = fin(get(tl, "font_size")).map_or(spx, |s| clampf(s as f32, 5.0, 40.0) * PT);
            let color = Rgb::from_value(get(tl, "color")).unwrap_or(lcolor);
            let (ox, oy) = (cx + r * c, cy + r * si);
            let (x, anchor) = if c > 0.1 {
                (ox + 5.0, Anchor::Start)
            } else {
                (ox + 2.0, Anchor::End)
            };
            let y = oy - if si.abs() > 0.5 { 8.0 } else { 5.0 };
            tx(cv, ctx, x, y, &text, tpx, color, anchor);
        }
    }
    // the callout: light digits in the hole, sized by the hole
    if callout.get("show").and_then(Value::as_bool) != Some(false) {
        let text = if callout.get("display_units").is_some() || callout.get("precision").is_some() {
            gauge_num(value, value, col.as_ref(), callout, ctx)
        } else {
            fmt_val(value, col.as_ref(), &ctx.loc, true)
        };
        let gw = if callout.get("bold").and_then(Value::as_bool) == Some(true) {
            700
        } else if ctx.classic {
            300
        } else {
            600
        };
        let per_px = ctx.fonts.measure(&font(ctx, 10.0).weight(gw), &text).max(1.0) / 10.0;
        let want = fin(get(callout, "font_size")).map(|s| clampf(s as f32, 5.0, 200.0) * PT);
        let vpx = clampf(
            want.unwrap_or((inner * 0.51).min(57.0)).min(inner * 1.8 / per_px),
            6.0,
            120.0,
        );
        let color = Rgb::from_value(get(callout, "color")).unwrap_or(ctx.fg);
        cv.text(
            ctx.fonts,
            &Text::new(&text, font(ctx, vpx).weight(gw), color)
                .anchor(Anchor::Middle)
                .base(Base::Alphabetic),
            cx,
            cy - 0.263 * inner + vpx * 0.35,
        );
    }
    if lab.get("show").and_then(Value::as_bool) != Some(false) {
        let lpx = fin(get(lab, "font_size")).map_or(spx, |s| clampf(s as f32, 5.0, 40.0) * PT);
        let (lt, ht) = (
            gauge_num(lo, scale, col.as_ref(), lab, ctx),
            gauge_num(hi, scale, col.as_ref(), lab, ctx),
        );
        if beside {
            tx(cv, ctx, cx - r - 4.0, cy - 3.4, &lt, lpx, lcolor, Anchor::End);
            tx(cv, ctx, cx + r + 4.0, cy - 3.4, &ht, lpx, lcolor, Anchor::Start);
        } else {
            tx(cv, ctx, cx - r, cy + 13.3, &lt, lpx, lcolor, Anchor::Start);
            tx(cv, ctx, cx + r, cy + 13.3, &ht, lpx, lcolor, Anchor::End);
        }
    }
}

// --- KPI -------------------------------------------------------------------------------------

/// A classic KPI's value size without `indicator.fontSize` (page px).
const KPI_VALUE_PX: f32 = 59.7;

pub fn kpi_chart(cv: &mut Canvas, ds: &Dataset, b: Bx, f: &Fmt, ctx: &Ctx<'_>) {
    let cd = chart_data(ds, ctx, Some(f), false, 200);
    let Some(se) = cd.series.first() else {
        return blank_visual(cv, ctx, b, ICON_KPI);
    };
    // Desktop draws nothing without a trend axis
    if !ds.cols.iter().any(|c| c.rc == Rc::Cat) && cd.cats.len() <= 1 {
        return;
    }
    let value = se.vals.iter().rev().find_map(|x| *x);
    let gi = ds.cols.iter().position(|c| c.rc == Rc::Goal);
    let goal = gi.and_then(|gi| ds.rows.iter().rev().find_map(|r| r.get(gi).and_then(fin)));
    // the status colours (`status.goodColor` / `badColor`, or their conditional formats)
    let kpi_color =
        |k: &str| Rgb::from_value(f.raw.get("kpi").and_then(|x| x.get(k)).unwrap_or(&Value::Null));
    let bad = kpi_color("bad_color").unwrap_or(if ctx.classic { Rgb(0xe8, 0x11, 0x23) } else { BAD });
    let good = kpi_color("good_color").unwrap_or_else(|| ctx.good());
    let status = match (goal, value) {
        (Some(g), Some(v)) => Some(v >= g),
        _ => None,
    };
    let (color, area_alpha) = match status {
        Some(true) => (good, 0.2),
        Some(false) => (bad, 0.2),
        None => (ctx.fg, 0.15),
    };
    let px = 10.67;
    let n = se.vals.len();
    let pts: Vec<(usize, f64)> = se
        .vals
        .iter()
        .enumerate()
        .filter_map(|(i, x)| x.map(|x| (i, x)))
        .collect();
    let (_, vy, _, vh) = viewport(f, ctx, b);
    let lo = pts.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let hi = pts.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
    // a flat trend draws no area (Desktop 2.158 on `oracle-gen kpis`, constant measures)
    if pts.len() > 1 && hi > lo {
        // Desktop's trend area fills the plot under the indicator, the maximum at the top
        let bottom = vy + vh;
        let y = |x: f64| {
            bottom
                - (if hi > lo {
                    ((x - lo) / (hi - lo)) as f32
                } else {
                    0.5
                }) * (bottom - vy)
        };
        let x = |i: usize| {
            if n > 1 {
                i as f32 / (n - 1) as f32 * b.w
            } else {
                0.0
            }
        };
        let line: Vec<(f32, f32)> = pts.iter().map(|p| (x(p.0), y(p.1))).collect();
        let mut area = line.clone();
        if let (Some(l), Some(fp)) = (pts.last(), pts.first()) {
            area.push((x(l.0), b.h));
            area.push((x(fp.0), b.h));
        }
        cv.polygon(&area, color, area_alpha);
    }
    // the indicator's display units and precision (`indicator` object), Desktop's rule
    let ind = f.raw.get("kpi");
    let units = ind.and_then(|k| k.get("display_units")).and_then(Value::as_f64);
    let precision = ind
        .and_then(|k| k.get("precision"))
        .and_then(Value::as_f64)
        .filter(|p| p.is_finite() && *p >= 0.0)
        .map(|p| p.min(10.0) as usize);
    let text = match value {
        Some(v) => fmt_kpi(v, Some(&se.col.cf()), &ctx.loc, units, precision),
        None => ctx.blank(),
    };
    let weight = if ctx.classic { 300 } else { 600 };
    // classic: a fixed 45 pt (Desktop 2.158 draws 59.7 page px in 200 x 165 to 400 x 330 KPIs
    // and lets a long value overflow); themed: fitted to the plot
    let per_px = ctx.fonts.measure(&font(ctx, 10.0).weight(weight), &text).max(1.0) / 10.0;
    let size_pt = ind
        .and_then(|k| k.get("font_size"))
        .and_then(Value::as_f64)
        .filter(|x| x.is_finite());
    let vpx = match size_pt {
        Some(pt) => clampf(pt as f32 * crate::data::PT, 6.0, 200.0),
        None if ctx.classic => KPI_VALUE_PX,
        None => clampf((vh * 0.19).min(b.w * 0.8 / per_px), 6.0, 72.0),
    };
    // classic: the value's box centred at 0.519 of the visual's height (-1.7 px), raised 0.12 of
    // its size above a goal, and shifted left by half the status icon (Desktop 2.158: 200 x 165
    // and 400 x 330 KPIs)
    let mid = if ctx.classic && size_pt.is_none() {
        b.h * 0.519 - 1.7
    } else {
        vy + vh / 2.0
    };
    let vc = if goal.is_some() { mid - vpx * 0.12 } else { mid };
    let cx = if ctx.classic && status.is_some() {
        b.w / 2.0 - vpx * 0.097
    } else {
        b.w / 2.0
    };
    // the indicator's own colour (`indicator.fontColor`) over the status colour
    let tcol = kpi_color("color").unwrap_or(if status.is_some() { color } else { ctx.fg });
    cv.text(
        ctx.fonts,
        &Text::new(&text, font(ctx, vpx).weight(weight), tcol)
            .anchor(Anchor::Middle)
            .base(Base::Alphabetic),
        cx,
        vc + vpx * 0.35,
    );
    // the status icon after the value: a check mark, or "!"
    if let Some(ok) = status {
        let tw_ = ctx.fonts.measure(&font(ctx, vpx).weight(weight), &text);
        let (ix, iy, s) = (cx + tw_ / 2.0 + vpx * 0.06, vc - vpx * 0.2, vpx * 0.2);
        if ok {
            cv.polyline(
                &[(ix, iy), (ix + s * 0.35, iy + s * 0.35), (ix + s, iy - s * 0.3)],
                color,
                1.0,
                (vpx * 0.035).max(1.0),
                None,
            );
        } else {
            // "!" drawn as a shape (Desktop's is an icon, no text)
            let (bx, bw, gh) = (ix + s * 0.2, (vpx * 0.035).max(1.0), s * 0.9);
            cv.rect(bx - bw / 2.0, iy - gh / 2.0, bw, gh * 0.68, color, 1.0);
            cv.rect(bx - bw / 2.0, iy + gh * 0.32, bw, bw, color, 1.0);
        }
    }
    if let Some(g) = goal
        && vh > vpx + px * 2.4
    {
        // the goal in its own format string ("Goal: 74 360" for a `#,0` goal beside a `#,0.00`
        // indicator, Desktop 2.158)
        let gcol = gi.and_then(|gi| ds.cols.get(gi)).unwrap_or(&se.col);
        let head = format!(
            "{}: {}",
            ctx.t("goal", "Goal"),
            fmt_val(g, Some(&gcol.cf()), &ctx.loc, gcol.fmt.is_empty())
        );
        let mut dist = String::new();
        if let Some(v) = value
            && g != 0.0
        {
            let d = (v - g) / g.abs();
            // the distance: the culture's digits; the percent pattern of the UI language: French
            // UI "(+5,26 %)", English UI "(+5,26%)" on the same fr-FR report (Desktop 2.158)
            let p = &ctx.loc.culture.display.percent;
            let (pre, suf) = if ctx.ui_lang.eq_ignore_ascii_case("en") {
                ("", "%")
            } else {
                (p.prefix, p.suffix)
            };
            dist = format!(
                "({}{}{}{})",
                if d >= 0.0 { "+" } else { "" },
                pre,
                ctx.loc.number(d * 100.0, 2, 2, true),
                suf
            );
        }
        // two runs, "Goal: …" and "(±x %)", centred together (Desktop's two text nodes)
        let gf = font(ctx, px).weight(600);
        let whole = if dist.is_empty() {
            head.clone()
        } else {
            format!("{head} {dist}")
        };
        let y = vc + vpx * 0.57;
        let full_w = ctx.fonts.measure(&gf, &whole);
        // the goal's and distance's colours (`goals.goalFontColor` / `distanceFontColor`)
        let gcolor = kpi_color("goal_color").unwrap_or(ctx.fg);
        let dcolor = kpi_color("distance_color").unwrap_or(ctx.fg);
        if dist.is_empty() || full_w > b.w - 4.0 {
            let t = ctx.fonts.fit(&gf, &whole, b.w - 4.0);
            tx_w(cv, ctx, b.w / 2.0, y, &t, px, gcolor, Anchor::Middle, 600);
        } else {
            let x0 = b.w / 2.0 - full_w / 2.0;
            let dx = ctx.fonts.measure(&gf, &format!("{head} "));
            tx_w(cv, ctx, x0, y, &head, px, gcolor, Anchor::Start, 600);
            tx_w(cv, ctx, x0 + dx, y, &dist, px, dcolor, Anchor::Start, 600);
        }
    }
}

// --- funnel ----------------------------------------------------------------------------------

/// Funnel, Desktop style: category labels on the left, centred bars with value labels, and
/// the conversion rates (first stage = 100 %, last / first) in brackets above and below, each
/// taking one stage's height (at least 22.7 px).
pub fn funnel_chart(cv: &mut Canvas, ds: &Dataset, b: Bx, f: &Fmt, ctx: &Ctx<'_>) {
    let cd = chart_data(ds, ctx, Some(f), true, 60);
    let n = cd.cats.len();
    let Some(se) = cd.series.first().filter(|_| n > 0) else {
        return blank_visual(cv, ctx, b, ICON_BAR);
    };
    let px = 12.0;
    let (vx, vy, vw, vh) = viewport(f, ctx, b);
    let cpx = LABEL_PX;
    let cat_w = (vw * 0.3).min(
        cd.cats
            .iter()
            .map(|c| ctx.fonts.measure(&label_face(ctx, cpx), c))
            .fold(0.0f32, f32::max)
            + 18.0,
    );
    let x0 = vx + cat_w;
    let w_all = (vw - cat_w - 4.0).max(1.0);
    let rates = vh > px * 6.0 && n > 1;
    let slot = vh / (n as f32 + 2.0);
    let reg = if rates { slot.max(22.7) } else { 0.0 };
    let band = (vh - 2.0 * reg) / n as f32;
    let gh = band * 0.89;
    let max = se
        .vals
        .iter()
        .map(|x| x.unwrap_or(0.0).abs())
        .fold(1e-9, f64::max);
    let width =
        |i: usize| ((se.vals.get(i).copied().flatten().unwrap_or(0.0).abs() / max) as f32 * w_all).max(1.0);
    // value labels at the label weight (Desktop 2.158: 400 unless `labels.bold`)
    let bold = label_face(ctx, cpx).weight(label_font(f, cpx, ctx.muted).2);
    let lh = cpx * 1.2;
    for (i, c) in cd.cats.iter().enumerate() {
        let x = se.vals.get(i).copied().flatten().unwrap_or(0.0);
        let w = width(i);
        let y = vy + reg + i as f32 * band + (band - gh) / 2.0;
        let part = hits::hv(&cd, i, 0).map(|h| {
            h.filter(|h| *h != 0.0).map(|h| {
                let hw = ((h.abs() / max) as f32 * w_all).max(1.0);
                (x0 + (w_all - hw) / 2.0, y, hw, gh)
            })
        });
        hits::bar_mark(
            cv,
            ctx,
            &cd,
            i,
            None,
            (x0 + (w_all - w) / 2.0, y, w, gh),
            se.ptc(i),
            part,
        );
        if band > cpx * 0.8 {
            let t = ctx.fonts.fit(&label_face(ctx, cpx), c, (cat_w - 4.0).max(0.0));
            label_tx(cv, ctx, vx, y + gh / 2.0, &t, cpx, ctx.muted, Anchor::Start);
        }
        if f.labels == Some(false) {
            continue;
        }
        // auto display units are the visual's (from the largest stage: "0K" next to "78K")
        let t = if fin(get(get(&f.raw, "labels"), "display_units")).is_some_and(|u| u != 0.0) {
            lbl(x, Some(&se.col.cf()), f, ctx)
        } else {
            let u = [1e12, 1e9, 1e6, 1e3]
                .into_iter()
                .find(|u| max >= *u)
                .unwrap_or(1.0);
            let prec = fin(get(get(&f.raw, "labels"), "precision")).map(|p| p.clamp(0.0, 10.0) as usize);
            fmt_label(x, Some(&se.col.cf()), &ctx.loc, Some(u), prec)
        };
        let tw_ = ctx.fonts.measure(&bold, &t);
        if gh >= lh && tw_ + 6.0 < w {
            cv.text(
                ctx.fonts,
                &Text::new(&t, bold.clone(), se.ptc(i).on())
                    .anchor(Anchor::Middle)
                    .base(Base::Central),
                x0 + w_all / 2.0,
                y + gh / 2.0,
            );
        } else if band >= cpx && x0 + (w_all + w) / 2.0 + 8.0 + tw_ < vx + vw {
            // a bar too thin for its label: the label after it
            label_tx(
                cv,
                ctx,
                x0 + (w_all + w) / 2.0 + 8.0,
                y + gh / 2.0,
                &t,
                cpx,
                ctx.muted,
                Anchor::Start,
            );
        }
    }
    if rates {
        let first = se.vals.first().copied().flatten().unwrap_or(0.0);
        let last = se.vals.get(n - 1).copied().flatten().unwrap_or(0.0);
        let cx = x0 + w_all / 2.0;
        let tick = (reg * 0.2).min(13.3);
        let bracket = |cv: &mut Canvas, w: f32, y: f32| {
            let (a, z) = (cx - w / 2.0, cx + w / 2.0);
            cv.line(a, y, z, y, ctx.faint, 1.0, None);
            cv.line(a, y - tick, a, y + tick, ctx.faint, 1.0, None);
            cv.line(z, y - tick, z, y + tick, ctx.faint, 1.0, None);
        };
        bracket(cv, width(0), vy + reg * 0.62);
        label_tx(
            cv,
            ctx,
            cx,
            vy + reg * 0.28,
            &ctx.loc.percent(1.0, 0, 1),
            cpx,
            ctx.muted,
            Anchor::Middle,
        );
        let bottom = vy + vh;
        bracket(cv, width(n - 1), bottom - reg * 0.62);
        if first != 0.0 {
            label_tx(
                cv,
                ctx,
                cx,
                bottom - reg * 0.28,
                &ctx.loc.percent(last / first, 0, 1),
                cpx,
                ctx.muted,
                Anchor::Middle,
            );
        }
    }
}

// --- treemap ---------------------------------------------------------------------------------

/// Treemap: d3 v3's squarified layout (golden-ratio aspect target, rows along the shorter side,
/// largest first), which matches the parts render oracle's treemap captures; colours follow the
/// data's order.
pub fn treemap_chart(cv: &mut Canvas, ds: &Dataset, b: Bx, f: &Fmt, ctx: &Ctx<'_>) {
    let cd = chart_data(ds, ctx, Some(f), true, 80);
    let Some(se) = cd.series.first() else {
        return blank_visual(cv, ctx, b, ICON_TABLE);
    };
    let mut items: Vec<(String, f64, Rgb, f64, usize)> = cd
        .cats
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let v = se.vals.get(i).copied().flatten()?;
            (v.is_finite() && v > 0.0).then(|| {
                (
                    c.clone(),
                    v,
                    se.pts.get(i).copied().flatten().unwrap_or_else(|| ctx.color(i)),
                    0.0,
                    i,
                )
            })
        })
        .collect();
    let total: f64 = items.iter().map(|x| x.1).sum();
    if items.is_empty() || total <= 0.0 {
        return blank_visual(cv, ctx, b, ICON_TABLE);
    }
    // stable descending sort (d3 pops the largest of an ascending sort)
    items.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let (vx, vy, vw, vh) = viewport(f, ctx, b);
    let area = f64::from(vw) * f64::from(vh);
    for it in &mut items {
        it.3 = it.1 / total * area;
    }
    let areas: Vec<f64> = items.iter().map(|it| it.3).collect();
    let rects = squarify(&areas, f64::from(vx), f64::from(vy), f64::from(vw), f64::from(vh));
    let px = 12.0;
    let lab = get(&f.raw, "labels");
    let vcolor = Rgb::from_value(get(lab, "color"));
    for (k, rx, ry, rw, rh) in rects {
        let Some(it) = items.get(k) else { continue };
        let (rx, ry, rw, rh) = (rx as f32, ry as f32, rw as f32, rh as f32);
        let (tw_, th_) = ((rw - 1.5).max(0.5), (rh - 1.5).max(0.5));
        let part = hits::hv(&cd, it.4, 0).map(|h| {
            let f = match h {
                Some(h) if it.1 > 0.0 && h.is_finite() => (h / it.1).clamp(0.0, 1.0) as f32,
                _ => 0.0,
            };
            (f > 0.0).then_some((rx, ry + th_ - th_ * f, tw_, th_ * f))
        });
        hits::bar_mark(cv, ctx, &cd, it.4, None, (rx, ry, tw_, th_), it.2, part);
        if rw > px * 3.0 && rh > px * 1.6 {
            let fg = Rgb(255, 255, 255);
            let lw = if ctx.classic { 500 } else { 600 };
            let t = ctx.fonts.fit(&font(ctx, px).weight(lw), &it.0, rw - px * 1.1);
            tx_w(
                cv,
                ctx,
                rx + px * 0.55,
                ry + px * 1.05,
                &t,
                px,
                fg,
                Anchor::Start,
                lw,
            );
            if rh > px * 3.0 && f.labels == Some(true) {
                let vpx = px * 0.85;
                let t = fit(ctx, &lbl(it.1, Some(&se.col.cf()), f, ctx), rw - px * 1.1, vpx);
                tx(
                    cv,
                    ctx,
                    rx + px * 0.55,
                    ry + rh - 9.0,
                    &t,
                    vpx,
                    vcolor.unwrap_or(fg),
                    Anchor::Start,
                );
            }
        }
    }
}

/// `squarify` and `position` of `d3.layout.treemap`, ported from d3 v3.5.17 (`src/layout/treemap.js`
/// @9cc9a875e636a1dcf36cc1e07bdf77e1ad6e2c74, BSD-3-Clause, Copyright (c) 2010-2016 Michael Bostock;
/// see `crates/render/LICENSE-BSD-d3` and THIRD_PARTY_NOTICES.md): `areas` sorted largest first, already
/// scaled to the rectangle; rows along the shorter side while the golden-ratio aspect score improves.
/// Returns (index, x, y, w, h) per tile.
fn squarify(areas: &[f64], x0: f64, y0: f64, w0: f64, h0: f64) -> Vec<(usize, f64, f64, f64, f64)> {
    let phi = 0.5 * (1.0 + 5f64.sqrt());
    let worst = |row: &[usize], s: f64, u: f64| -> f64 {
        let (mut rmin, mut rmax) = (f64::INFINITY, 0.0f64);
        for k in row {
            if let Some(it) = areas.get(*k) {
                rmin = rmin.min(*it);
                rmax = rmax.max(*it);
            }
        }
        let (s2, u2) = (s * s, u * u);
        if s2 > 0.0 && rmin > 0.0 {
            ((u2 * rmax * phi) / s2).max(s2 / (u2 * rmin * phi))
        } else {
            f64::INFINITY
        }
    };
    let (mut x, mut y, mut w, mut h) = (x0, y0, w0, h0);
    let mut rects: Vec<(usize, f64, f64, f64, f64)> = Vec::new();
    let mut row: Vec<usize> = Vec::new();
    let mut row_area = 0.0;
    let mut best = f64::INFINITY;
    let mut i = 0;
    let mut guard = 0;
    // d3.layout.treemap's `position`: lay one row along the shorter side
    let mut place =
        |row: &[usize], row_area: f64, flush: bool, x: &mut f64, y: &mut f64, w: &mut f64, h: &mut f64| {
            let u = w.min(*h);
            let mut v = if u > 0.0 { row_area / u } else { 0.0 };
            if u == *w {
                if flush || v > *h {
                    v = *h;
                }
                let mut ox = *x;
                for (k, it) in row.iter().enumerate() {
                    let a = areas.get(*it).copied().unwrap_or(0.0);
                    let mut dw = (*x + *w - ox).min(if v > 0.0 { a / v } else { 0.0 });
                    if k + 1 == row.len() {
                        dw = *x + *w - ox;
                    }
                    rects.push((*it, ox, *y, dw, v));
                    ox += dw;
                }
                *y += v;
                *h -= v;
            } else {
                if flush || v > *w {
                    v = *w;
                }
                let mut oy = *y;
                for (k, it) in row.iter().enumerate() {
                    let a = areas.get(*it).copied().unwrap_or(0.0);
                    let mut dh = (*y + *h - oy).min(if v > 0.0 { a / v } else { 0.0 });
                    if k + 1 == row.len() {
                        dh = *y + *h - oy;
                    }
                    rects.push((*it, *x, oy, v, dh));
                    oy += dh;
                }
                *x += v;
                *w -= v;
            }
        };
    while i < areas.len() && guard < 10_000 {
        guard += 1;
        let u = w.min(h);
        let a = areas.get(i).copied().unwrap_or(0.0);
        row.push(i);
        row_area += a;
        let score = worst(&row, row_area, u);
        if score <= best {
            best = score;
            i += 1;
        } else {
            row.pop();
            row_area -= a;
            place(&row, row_area, false, &mut x, &mut y, &mut w, &mut h);
            row.clear();
            row_area = 0.0;
            best = f64::INFINITY;
        }
    }
    if !row.is_empty() {
        place(&row, row_area, true, &mut x, &mut y, &mut w, &mut h);
    }
    rects
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn squarify_keeps_desktop_rows() {
        // Desktop 2.158 (389 x 303): the two largest of three share the left column
        let (w, h) = (389.0, 303.0);
        let vals = [411.0, 303.0, 197.0];
        let total: f64 = vals.iter().sum();
        let areas: Vec<f64> = vals.iter().map(|v| v / total * w * h).collect();
        let r = squarify(&areas, 0.0, 0.0, w, h);
        assert_eq!(r.len(), 3);
        let (a, b, c) = (r[0], r[1], r[2]);
        assert!((a.1 - b.1).abs() < 1e-9 && a.3 > 300.0 && a.3 < 310.0, "{r:?}");
        assert!(c.1 > 300.0 && (c.4 - h).abs() < 1e-6, "{r:?}");
        let sum: f64 = r.iter().map(|t| t.3 * t.4).sum();
        assert!((sum - w * h).abs() < 1e-6);
    }

    #[test]
    fn squarify_survives_hostile_areas() {
        for areas in [
            vec![],
            vec![0.0, 0.0],
            vec![f64::MAX, 1.0],
            vec![1.0; 500],
            vec![1e-300, 5.0],
        ] {
            let r = squarify(&areas, 0.0, 0.0, 100.0, 50.0);
            assert!(r.len() <= areas.len());
        }
        let r = squarify(&[1.0, 2.0], 0.0, 0.0, 0.0, 0.0);
        assert!(r.len() <= 2);
    }
}
