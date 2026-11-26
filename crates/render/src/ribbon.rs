//! Ribbon chart (`ribbonChart`): per category a column whose segments are stacked by rank (the
//! largest value on top), and between adjacent columns a translucent band (ribbon) per series
//! joining its two segments with horizontal-tangent curves. Layout measured on Desktop 2.158
//! (`tools/render-oracle`): no value axis or gridlines by default, the vertical layout and value
//! scale of the stacked column chart, columns about 35 % of the category step with 40 % of a
//! step before the first and after the last column, ribbons at 70 % opacity under the columns.
//! A date category is a continuous axis: columns sit at their dates.

use crate::canvas::{Anchor, Canvas, Rgb};
use crate::charts::{
    Bx, ICON_BAR, axis_title, blank_visual, cat_axis, draw_cat_axis, fit, items, nice_ticks, place_legend,
    tw, tx, value_scale, want_legend,
};
use crate::data::{Ctx, Dataset, Fmt, chart_data, fin, get};
use crate::hits;

/// Column width as a share of the category step.
const COL_FILL: f32 = 0.35;
/// Space before the first and after the last column, in steps (categorical axis).
const OUTER: f32 = 0.4;
/// The same for a continuous (date) axis.
const OUTER_DATE: f32 = 0.25;
/// Ribbon opacity (Desktop's default ribbon transparency is 30 %).
const RIBBON_ALPHA: f32 = 0.7;

/// Days since 1970-01-01 of an ISO date (`2025-03-01…`).
fn iso_days(s: &str) -> Option<i64> {
    let y: i64 = s.get(0..4)?.parse().ok()?;
    let m: i64 = s.get(5..7)?.parse().ok()?;
    let d: i64 = s.get(8..10)?.parse().ok()?;
    if s.get(4..5) != Some("-")
        || !(1..=12).contains(&m)
        || !(1..=31).contains(&d)
        || !(1..=9999).contains(&y)
    {
        return None;
    }
    Some(bicraft_locale::calendar::days_from_civil(y, m, d))
}

/// Category positions on 0..1 of the span between the first and last column centres: their
/// dates when every category is an increasing date, else evenly spaced.
fn positions(raw: &[serde_json::Value], n: usize, date: bool) -> (Vec<f32>, bool) {
    let even = |n: usize| -> Vec<f32> {
        (0..n)
            .map(|i| if n > 1 { i as f32 / (n - 1) as f32 } else { 0.5 })
            .collect()
    };
    if !date || n < 2 {
        return (even(n), false);
    }
    let days: Option<Vec<i64>> = (0..n)
        .map(|i| raw.get(i).and_then(serde_json::Value::as_str).and_then(iso_days))
        .collect();
    let Some(days) = days else {
        return (even(n), false);
    };
    let (Some(a), Some(b)) = (days.first(), days.last()) else {
        return (even(n), false);
    };
    let span = b.saturating_sub(*a);
    if span <= 0 || days.windows(2).any(|w| w.get(1) <= w.first()) {
        return (even(n), false);
    }
    (
        days.iter()
            .map(|d| (d.saturating_sub(*a)) as f32 / span as f32)
            .collect(),
        true,
    )
}

/// Ribbons' grey when Match series color is off (Learn: "ribbons are gray").
const RIBBON_GRAY: Rgb = Rgb(0xA0, 0xA0, 0xA0);

/// The Ribbons card (`format.ribbon`: `ribbonBands` and `layout.ribbonGapSize`), Learn's
/// "Format a ribbon chart": Spacing, Match series color, Transparency (30 by default), Border
/// (off by default).
struct RibbonFmt {
    show: bool,
    /// Ribbon colour when Match series color is off (grey unless set).
    color: Option<Rgb>,
    match_color: bool,
    alpha: f32,
    border: bool,
    border_color: Option<Rgb>,
    border_match: bool,
    border_width: f32,
    border_alpha: f32,
    /// Spacing between the segments of a column, a share of the tallest column (0..1).
    gap: f64,
}

impl RibbonFmt {
    fn of(f: &Fmt) -> RibbonFmt {
        let r = get(&f.raw, "ribbon");
        let b = |k: &str, d: bool| r.get(k).and_then(serde_json::Value::as_bool).unwrap_or(d);
        let pct = |k: &str, d: f64| fin(get(r, k)).unwrap_or(d).clamp(0.0, 100.0);
        RibbonFmt {
            show: b("show", true),
            color: Rgb::from_value(get(r, "color")),
            match_color: b("match_color", true),
            alpha: (1.0 - pct("transparency", (1.0 - f64::from(RIBBON_ALPHA)) * 100.0) / 100.0) as f32,
            border: b("border", false),
            border_color: Rgb::from_value(get(r, "border_color")),
            border_match: b("border_match", false),
            border_width: fin(get(r, "border_width")).unwrap_or(1.0).clamp(0.0, 10.0) as f32,
            border_alpha: (1.0 - pct("border_transparency", 0.0) / 100.0) as f32,
            gap: pct("gap", 0.0) / 100.0,
        }
    }
}

/// Total labels (`format.totals`): shown when `show` is true.
struct TotalsFmt {
    color: Option<Rgb>,
    px: Option<f32>,
    units: Option<f64>,
    precision: Option<usize>,
}

impl TotalsFmt {
    fn of(f: &Fmt) -> Option<TotalsFmt> {
        let t = get(&f.raw, "totals");
        if t.get("show").and_then(serde_json::Value::as_bool) != Some(true) {
            return None;
        }
        Some(TotalsFmt {
            color: Rgb::from_value(get(t, "color")),
            px: fin(get(t, "font_size")).map(|s| s.clamp(5.0, 72.0) as f32 * crate::data::PT),
            units: fin(get(t, "display_units")).filter(|u| *u > 0.0),
            precision: fin(get(t, "precision")).map(|p| p.clamp(0.0, 10.0) as usize),
        })
    }
}

/// One column's segments: (series, bottom value, top value), by rank.
#[cfg(test)]
fn stack(vals: &[Option<f64>]) -> Vec<(usize, f64, f64)> {
    stack_gap(vals, 0.0)
}

/// [`stack`] with `gap` (value units) between adjacent segments.
fn stack_gap(vals: &[Option<f64>], gap: f64) -> Vec<(usize, f64, f64)> {
    let mut pos: Vec<(usize, f64)> = Vec::new();
    let mut neg: Vec<(usize, f64)> = Vec::new();
    for (j, v) in vals.iter().enumerate() {
        match v {
            Some(x) if *x > 0.0 => pos.push((j, *x)),
            Some(x) if *x < 0.0 => neg.push((j, *x)),
            _ => {}
        }
    }
    // smallest at the bottom, largest on top (stable: ties keep the series order)
    pos.sort_by(|a, b| a.1.total_cmp(&b.1));
    // below zero: the smallest magnitude next to the axis
    neg.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut out = Vec::with_capacity(pos.len() + neg.len());
    let gap = if gap.is_finite() { gap.max(0.0) } else { 0.0 };
    let mut p = 0.0;
    for (k, (j, x)) in pos.into_iter().enumerate() {
        if k > 0 {
            p += gap;
        }
        out.push((j, p, p + x));
        p += x;
    }
    let mut q = 0.0;
    for (k, (j, x)) in neg.into_iter().enumerate() {
        if k > 0 {
            q -= gap;
        }
        out.push((j, q + x, q));
        q += x;
    }
    out
}

/// The top and bottom edges of a band (for its border).
fn edge_path(x0: f32, a0: f32, x1: f32, a1: f32) -> Option<tiny_skia::Path> {
    let xm = (x0 + x1) / 2.0;
    let mut pb = tiny_skia::PathBuilder::new();
    pb.move_to(x0, a0);
    pb.cubic_to(xm, a0, xm, a1, x1, a1);
    pb.finish()
}

/// A band from the edge `x0` (spanning `a0..b0` vertically) to `x1` (`a1..b1`).
fn ribbon_path(x0: f32, a0: f32, b0: f32, x1: f32, a1: f32, b1: f32) -> Option<tiny_skia::Path> {
    let xm = (x0 + x1) / 2.0;
    let mut pb = tiny_skia::PathBuilder::new();
    pb.move_to(x0, a0);
    pb.cubic_to(xm, a0, xm, a1, x1, a1);
    pb.line_to(x1, b1);
    pb.cubic_to(xm, b1, xm, b0, x0, b0);
    pb.close();
    pb.finish()
}

pub fn ribbon_chart(cv: &mut Canvas, ds: &Dataset, b: Bx, f: &Fmt, ctx: &Ctx<'_>) {
    let cd = chart_data(ds, ctx, Some(f), true, 60);
    let (ser, n, m) = (&cd.series, cd.cats.len(), cd.series.len());
    if m == 0 || n == 0 {
        return blank_visual(cv, ctx, b, ICON_BAR);
    }
    let px = ctx.chart_px;
    let lpx = px * 0.92;
    let (lt, lb) = if want_legend(f, &cd) {
        place_legend(cv, ctx, f, &items(ser), &cd.legend_name, b)
    } else {
        (0.0, 0.0)
    };
    let rf = RibbonFmt::of(f);
    let totals = TotalsFmt::of(f);
    let vals: Vec<Vec<Option<f64>>> = (0..n)
        .map(|i| ser.iter().map(|s| s.vals.get(i).copied().flatten()).collect())
        .collect();
    // Spacing: a share of the tallest column (its positive or negative part)
    let tallest = vals
        .iter()
        .map(|v| {
            let (p, q) =
                v.iter().flatten().fold(
                    (0.0f64, 0.0f64),
                    |(p, q), x| {
                        if *x > 0.0 { (p + x, q) } else { (p, q - x) }
                    },
                );
            p.max(q)
        })
        .fold(0.0f64, f64::max);
    let gap = tallest * rf.gap;
    let cols: Vec<Vec<(usize, f64, f64)>> = vals.iter().map(|v| stack_gap(v, gap)).collect();
    let (mut lo, mut hi) = (0.0f64, 0.0f64);
    for c in &cols {
        for (_, a, e) in c {
            lo = lo.min(*a);
            hi = hi.max(*e);
        }
    }
    let span = b.h - lt - lb;
    let nt = ((span / (px * 6.0)).floor() as usize).clamp(2, 8);
    // Desktop rounds the hidden axis up to the next nice tick (156K -> 200K), without the
    // column chart's tightening; explicit bounds and log scales go through the shared scale
    let tk = if f.y.min.is_some() || f.y.max.is_some() || f.y.log {
        value_scale(lo, hi, nt, Some(&f.y))
    } else {
        nice_ticks(lo, hi, nt)
    };
    let show_lab = f.labels == Some(true);
    let ctitle = axis_title(&f.x, &cd.cat_name);
    let x_th = if ctitle.is_empty() { 0.0 } else { px * 1.5 };
    // Desktop hides the value axis of a ribbon chart: the plot spans the whole body
    let (l, r) = (0.0f32, b.w);
    let tpx = totals.as_ref().and_then(|t| t.px).unwrap_or(lpx);
    let t0 = lt
        + if totals.is_some() {
            tpx * 1.8
        } else if show_lab {
            px * 1.4
        } else {
            px
        };
    let (pos, continuous) = positions(&cd.raw, n, cd.cat_date);
    let outer = if continuous { OUTER_DATE } else { OUTER };
    // the step: n - 1 steps between column centres, a column and the two outer spaces
    let steps = (n.saturating_sub(1)) as f32 + COL_FILL + 2.0 * outer;
    let step = (r - l) / steps.max(0.1);
    let cw = step * COL_FILL;
    let first = l + step * outer + cw / 2.0;
    let centre_span = step * (n.saturating_sub(1)) as f32;
    let cx = |i: usize| first + pos.get(i).copied().unwrap_or(0.0) * centre_span;
    let band = if n > 1 { centre_span / (n - 1) as f32 } else { step };
    let ax = if f.x.show {
        Some(cat_axis(ctx, &cd.cats, band, px, span * 0.35, continuous))
    } else {
        None
    };
    let bb = b.h - lb - ax.as_ref().map_or(px * 0.3, |a| a.h) - x_th;
    if r - l < 8.0 || bb - t0 < 8.0 || cw <= 0.0 {
        return;
    }
    let y = |v: f64| bb - tk.f(v) * (bb - t0);
    // ribbons first, under the columns
    for i in 1..n {
        if !rf.show {
            break;
        }
        let (Some(pc), Some(nc)) = (cols.get(i - 1), cols.get(i)) else {
            continue;
        };
        let (x0, x1) = (cx(i - 1) + cw / 2.0, cx(i) - cw / 2.0);
        if x1 <= x0 {
            continue;
        }
        for (j, a0, e0) in pc {
            let Some((_, a1, e1)) = nc.iter().find(|s| s.0 == *j) else {
                continue;
            };
            let series = ser.get(*j).map_or(ctx.accent, |s| s.color);
            let c = if rf.match_color {
                series
            } else {
                rf.color.unwrap_or(RIBBON_GRAY)
            };
            // a ribbon fades with its two ends when a selection leaves them out
            let dim = ctx
                .ix
                .alpha(&hits::cd_point(&cd, Some(i - 1), Some(*j)))
                .max(ctx.ix.alpha(&hits::cd_point(&cd, Some(i), Some(*j))));
            if let Some(p) = ribbon_path(x0, y(*e0), y(*a0), x1, y(*e1), y(*a1)) {
                cv.fill_path(&p, c, rf.alpha * dim);
            }
            if rf.border && rf.border_width > 0.0 {
                let bc = match (rf.border_match, rf.border_color) {
                    (true, _) => c.mix(Rgb(0, 0, 0), 0.35),
                    (false, Some(b)) => b,
                    (false, None) => ctx.fg,
                };
                for (a, e) in [(*e0, *e1), (*a0, *a1)] {
                    if let Some(p) = edge_path(x0, y(a), x1, y(e)) {
                        cv.stroke(&p, bc, rf.border_alpha * dim, rf.border_width, None);
                    }
                }
            }
        }
    }
    let mut labels: Vec<(f32, f32, String, Rgb)> = Vec::new();
    for (i, c) in cols.iter().enumerate() {
        let x0 = cx(i) - cw / 2.0;
        for (j, a, e) in c {
            let (ya, ye) = (y(*a), y(*e));
            let hgt = (ya - ye).abs();
            let s = ser.get(*j);
            let col = s.map_or(ctx.accent, |s| s.ptc(i));
            // the segment, faded by a selection, with its hit (tooltip, cross-filter on click)
            hits::bar_mark(
                cv,
                ctx,
                &cd,
                i,
                Some(*j),
                (x0, ya.min(ye), cw, hgt.max(0.5)),
                col,
                None,
            );
            if show_lab && hgt > lpx * 1.2 {
                // the segment's own value (negative below the axis)
                let x = if *e <= 0.0 { a - e } else { e - a };
                let t = crate::fmt::fmt_val(x, s.map(|s| s.col.cf()).as_ref(), &ctx.loc, true);
                if tw(ctx, &t, lpx) < cw + px {
                    labels.push((cx(i), (ya + ye) / 2.0, t, col.on()));
                }
            }
        }
    }
    for (x, yy, t, c) in &labels {
        tx(cv, ctx, *x, *yy, t, lpx, *c, Anchor::Middle);
    }
    // total labels: each column's total above it
    if let Some(t) = &totals {
        let cf = ser.first().map(|s| s.col.cf());
        for (i, v) in vals.iter().enumerate() {
            if v.iter().all(Option::is_none) {
                continue;
            }
            let total: f64 = v.iter().flatten().sum();
            let top = cols
                .get(i)
                .map_or(0.0, |c| c.iter().map(|s| s.2).fold(0.0f64, f64::max));
            let text = crate::fmt::fmt_label(total, cf.as_ref(), &ctx.loc, t.units, t.precision);
            let yy = y(top) - tpx * 0.75;
            tx(
                cv,
                ctx,
                cx(i),
                yy,
                &text,
                tpx,
                t.color.unwrap_or(ctx.fg),
                Anchor::Middle,
            );
        }
    }
    if let Some(ax) = &ax {
        draw_cat_axis(cv, ctx, ax, &cd.cats, band, px, &cx, bb);
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
            ctx.fg2,
            Anchor::Middle,
        );
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::fonts::FontBook;

    #[test]
    fn ranks_each_column() {
        let s = stack(&[Some(10.0), Some(30.0), None, Some(20.0), Some(-5.0), Some(-1.0)]);
        let order: Vec<usize> = s.iter().map(|x| x.0).collect();
        assert_eq!(order, vec![0, 3, 1, 5, 4]);
        assert_eq!(s.get(2).map(|x| (x.1, x.2)), Some((30.0, 60.0)));
        assert_eq!(s.get(4).map(|x| (x.1, x.2)), Some((-6.0, -1.0)));
    }

    #[test]
    fn dates_are_continuous() {
        let raw = vec![json!("2025-01-01"), json!("2025-02-01"), json!("2025-03-01")];
        let (p, cont) = positions(&raw, 3, true);
        assert!(cont);
        // January has 31 days, February 28
        let mid = p.get(1).copied().unwrap_or(0.0);
        assert!((mid - 31.0 / 59.0).abs() < 1e-4, "{mid}");
        let (p, cont) = positions(&[json!("x"), json!(3)], 2, true);
        assert!(!cont);
        assert_eq!(p, vec![0.0, 1.0]);
        assert_eq!(iso_days("1970-01-01"), Some(0));
        assert_eq!(iso_days("2025-13-01"), None);
    }

    #[test]
    fn draws_columns_and_ribbons() {
        let fonts = FontBook::bundled_only();
        let snap = json!({"version": 1, "model": {"culture": "en-US"}, "report": {"pages": []}});
        let ctx = Ctx::new(&snap, &fonts, None);
        let v = json!({
            "type": "ribbonChart",
            "data": {"status": "ok",
                "columns": [{"name": "Region", "role": "Category", "kind": "column", "data_type": "string"},
                            {"name": "Cat", "role": "Series", "kind": "column", "data_type": "string"},
                            {"name": "Total", "role": "Y", "kind": "measure"}],
                "rows": [["A", "x", 10.0], ["A", "y", 30.0], ["B", "x", 40.0], ["B", "y", 20.0]]}
        });
        let ds = crate::data::dataset(&v, "ribbon", &ctx);
        let f = crate::data::vis_format(&v, "ribbon", &ctx);
        let (w, h) = (400.0, 300.0);
        let Some(mut cv) = Canvas::new(w, h, 1.0) else {
            panic!("canvas")
        };
        ribbon_chart(&mut cv, &ds, Bx { w, h }, &f, &ctx);
        let px = |x: f32, y: f32| {
            cv.pm
                .pixel(x as u32, y as u32)
                .map(|p| (p.red(), p.green(), p.blue(), p.alpha()))
        };
        // the column centres: step = 400 / (1 + 0.35 + 0.8)
        let step = w / (1.0 + COL_FILL + 2.0 * OUTER);
        let c0 = step * OUTER + step * COL_FILL / 2.0;
        // opaque columns low in the plot, a translucent ribbon midway between them
        let col = px(c0, h * 0.75);
        assert_eq!(col.map(|p| p.3), Some(255), "{col:?}");
        let mid = px(c0 + step / 2.0, h * 0.7);
        let a = mid.map_or(0, |p| p.3);
        assert!(a > 100 && a < 255, "{mid:?}");
        // hostile: nothing to draw, a tiny box, huge values
        let empty = Dataset::default();
        ribbon_chart(&mut cv, &empty, Bx { w, h }, &f, &ctx);
        ribbon_chart(&mut cv, &ds, Bx { w: 3.0, h: 2.0 }, &f, &ctx);
    }

    #[test]
    fn spacing_puts_gaps_between_ranked_segments() {
        let s = stack_gap(&[Some(10.0), Some(30.0)], 4.0);
        assert_eq!(s, vec![(0, 0.0, 10.0), (1, 14.0, 44.0)]);
        let s = stack_gap(&[Some(-10.0), Some(-30.0)], 4.0);
        assert_eq!(s, vec![(0, -10.0, 0.0), (1, -44.0, -14.0)]);
        assert_eq!(
            stack_gap(&[Some(1.0), Some(2.0)], f64::NAN),
            stack(&[Some(1.0), Some(2.0)])
        );
    }

    /// Draws the two-column sample with `format`, hits on; the canvas, its column centres.
    fn draw(format: serde_json::Value) -> (Canvas, f32, f32) {
        let fonts = FontBook::bundled_only();
        let snap = json!({"version": 1, "model": {"culture": "en-US"}, "report": {"pages": []}});
        let mut ctx = Ctx::new(&snap, &fonts, None);
        ctx.ix = crate::hits::Ix::new(true);
        let v = json!({
            "name": "r", "type": "ribbonChart", "format": format,
            "data": {"status": "ok",
                "columns": [{"name": "Region", "role": "Category", "kind": "column", "data_type": "string"},
                            {"name": "Cat", "role": "Series", "kind": "column", "data_type": "string"},
                            {"name": "Total", "role": "Y", "kind": "measure"}],
                "rows": [["A", "x", 10.0], ["A", "y", 30.0], ["B", "x", 40.0], ["B", "y", 20.0]]}
        });
        ctx.ix.enter("p", &v);
        let ds = crate::data::dataset(&v, "ribbon", &ctx);
        let f = crate::data::vis_format(&v, "ribbon", &ctx);
        let (w, h) = (400.0, 300.0);
        let Some(mut cv) = Canvas::new(w, h, 1.0) else {
            panic!("canvas")
        };
        ribbon_chart(&mut cv, &ds, Bx { w, h }, &f, &ctx);
        let step = w / (1.0 + COL_FILL + 2.0 * OUTER);
        let c0 = step * OUTER + step * COL_FILL / 2.0;
        (cv, c0, c0 + step)
    }

    fn pixel(cv: &Canvas, x: f32, y: f32) -> (u8, u8, u8, u8) {
        cv.pm
            .pixel(x as u32, y as u32)
            .map_or((0, 0, 0, 0), |p| (p.red(), p.green(), p.blue(), p.alpha()))
    }

    #[test]
    fn ribbons_card_colour_transparency_and_off() {
        let (plain, c0, c1) = draw(json!({}));
        let mid = (c0 + c1) / 2.0;
        // the midway ribbon column: a coloured ribbon at the default 30 % transparency
        let col = |cv: &Canvas| {
            (0..300)
                .map(|y| pixel(cv, mid, y as f32))
                .filter(|p| p.3 > 0)
                .collect::<Vec<_>>()
        };
        let base = col(&plain);
        assert!(!base.is_empty());
        assert!(
            base.iter().any(|p| p.0 != p.1 || p.1 != p.2),
            "a series colour: {base:?}"
        );
        // Match series color off: grey ribbons
        let (grey, ..) = draw(json!({"ribbon": {"match_color": false}}));
        let g = col(&grey);
        assert!(
            !g.is_empty() && g.iter().all(|p| p.0 == p.1 && p.1 == p.2),
            "{g:?}"
        );
        // more transparent
        let (light, ..) = draw(json!({"ribbon": {"transparency": 80}}));
        let max_a = |v: &[(u8, u8, u8, u8)]| v.iter().map(|p| p.3).max().unwrap_or(0);
        assert!(max_a(&col(&light)) < max_a(&base), "{:?}", col(&light));
        // ribbons off: nothing between the columns
        let (off, ..) = draw(json!({"ribbon": {"show": false}}));
        assert!(col(&off).is_empty());
    }

    #[test]
    fn total_labels_and_hits() {
        let (plain, c0, _) = draw(json!({}));
        let (tot, ..) = draw(json!({"totals": {"show": true}}));
        // something is written over the first column (total 40) that is not there without totals
        let changed = (0..300)
            .flat_map(|y| ((c0 - 25.0) as i32..(c0 + 25.0) as i32).map(move |x| (x as f32, y as f32)))
            .filter(|(x, y)| pixel(&tot, *x, *y) != pixel(&plain, *x, *y))
            .count();
        assert!(changed > 20, "{changed}");
        // every segment has a hit with a tooltip and a select action
        assert_eq!(plain.hits.len(), 4);
        assert!(plain.hits.iter().all(|h| !h.tip.is_empty()));
    }
}
