//! The BIcraft Bullet chart (docs/formats/bicraft-visuals.md "Bullet chart"): one horizontal
//! (or vertical) bullet per category: qualitative bands in shades of grey, the value as a bar and the target as
//! a tick, on one scale shared by every row. The same layout is in
//! `crates/engine/src/bvis/bullet.ts`.

use super::{Data, Opts, PAD, Units, fit, label_width, scale, x_axis, y_labels};
use crate::look::Look;
use crate::spec::{Prim, domain, fmt_auto, inside, line, text};

/// Most bullets drawn.
pub const MAX_ROWS: usize = 100;
/// Most band fields.
pub const MAX_BANDS: usize = 4;
/// Which side of band `j` of `n` stays square, from the low end to the high end (`first` and
/// `last`: the sides facing the next band): the outer ends of the track are rounded.
fn band_side(j: usize, n: usize, first: &'static str, last: &'static str) -> &'static str {
    match (j == 0, j + 1 == n) {
        (true, true) => "",
        (true, false) => first,
        (false, true) => last,
        (false, false) => "x",
    }
}

/// The shade of band `j` of `n`: `n` bands take the last `n` of the look's five.
fn shade(k: &Look, j: usize, n: usize) -> &'static str {
    k.shades
        .get(k.shades.len().saturating_sub(n) + j)
        .copied()
        .unwrap_or(k.shades[4])
}

/// The target tick's width.
fn target_width(k: &Look) -> f64 {
    if k.minimal() { 2.0 } else { 3.0 }
}

/// Row `i`'s band thresholds, ascending: the `bands` fields, else the format pane's percentages
/// of the target.
fn thresholds(d: &Data, i: usize, pct: &[f64; 3]) -> Vec<f64> {
    let mut t: Vec<f64> = if d.count("bands") > 0 {
        (0..MAX_BANDS)
            .filter_map(|k| d.col("bands", k).and_then(|c| c.num(i)))
            .collect()
    } else {
        match d.col("target", 0).and_then(|c| c.num(i)) {
            Some(target) => pct
                .iter()
                .filter(|p| **p > 0.0)
                .map(|p| target * p / 100.0)
                .collect(),
            None => Vec::new(),
        }
    };
    t.sort_by(f64::total_cmp);
    t
}

/// Row `i`'s value bar (`bar` draws a value's): highlighted, the full value dimmed and its
/// highlighted part over it (docs/formats/bicraft-visuals.md "Highlights").
fn value_bar(
    d: &Data,
    i: usize,
    v: f64,
    bar: &dyn Fn(f64) -> Prim,
    out: &mut Vec<Prim>,
    marks: &mut Vec<usize>,
) {
    marks.push(out.len());
    if !d.highlighted() {
        out.push(bar(v));
        return;
    }
    out.push(crate::look::dim(bar(v)));
    if let Some(p) = crate::look::part(v, d.col("value", 0).and_then(|c| c.hl(i))) {
        marks.push(out.len());
        out.push(bar(p));
    }
}

/// The vertical bullets: one column per row, the scale on the left, the category names below.
#[allow(clippy::too_many_arguments)]
fn columns(
    d: &Data,
    pct: &[f64; 3],
    dom: &crate::spec::Domain,
    names: &[String],
    has_cat: bool,
    (vcolor, tcolor): (&str, &str),
    labels: bool,
    lk: &Look,
    w: f64,
    h: f64,
    out: &mut Vec<Prim>,
    u: &mut Units,
) {
    let n = names.len();
    let left = PAD + 36.0;
    let right = w - PAD;
    let top = PAD + 6.0;
    let bottom = h - PAD - if has_cat { 18.0 } else { 0.0 };
    if right - left < 8.0 || bottom - top < 8.0 {
        return;
    }
    let cw = (right - left) / n as f64;
    if cw < 3.0 {
        return;
    }
    let bw = (cw * 0.7).min(32.0);
    lk.push_card(out, w, h);
    let ys = |v: f64| scale(dom, bottom, top, v);
    let zero = ys(0.0_f64.clamp(dom.lo, dom.hi));
    for (i, name) in names.iter().enumerate() {
        let x0 = left + cw * i as f64 + (cw - bw) / 2.0;
        let mid = x0 + bw / 2.0;
        let th: Vec<f64> = thresholds(d, i, pct)
            .into_iter()
            .map(|t| t.clamp(dom.lo, dom.hi))
            .collect();
        let end = d
            .col("maximum", 0)
            .and_then(|c| c.num(i))
            .map_or(dom.hi, |m| m.clamp(dom.lo, dom.hi));
        let k = th.len() + 1;
        let mut from = dom.lo;
        let (mut bands, mut marks) = (Vec::new(), Vec::new());
        for (j, edge) in th.iter().copied().chain(std::iter::once(end)).enumerate() {
            let (a, b) = (ys(from), ys(edge.max(from)));
            if a > b {
                bands.push(out.len());
                out.push(lk.band(x0, b, bw, a - b, shade(lk, j, k), band_side(j, k, "t", "b")));
            }
            from = edge.max(from);
        }
        let v = d.col("value", 0).and_then(|c| c.num(i));
        if let Some(v) = v {
            let y = ys(v);
            let sq = if v >= 0.0 { "b" } else { "t" };
            let bar = |v: f64| {
                let y = ys(v);
                lk.bar(x0 + bw / 3.0, y.min(zero), bw / 3.0, (y - zero).abs(), vcolor, sq)
            };
            value_bar(d, i, v, &bar, out, &mut marks);
            if labels {
                let side = if v >= 0.0 { 'n' } else { 's' };
                let first = out.len();
                lk.value_label(out, mid, y, fmt_auto(v), side, 7.0, lk.value);
                marks.extend(first..out.len());
            }
        }
        let t = d.col("target", 0).and_then(|c| c.num(i));
        if let Some(t) = t {
            let y = ys(t.clamp(dom.lo, dom.hi));
            marks.push(out.len());
            out.push(line(
                x0 + bw * 0.15,
                y,
                x0 + bw * 0.85,
                y,
                tcolor,
                target_width(lk),
            ));
        }
        if has_cat {
            let s = fit(name, cw - 4.0, lk.tick);
            out.push(text(
                inside(mid, &s, lk.tick, w),
                bottom + 9.0,
                s,
                lk.tick,
                lk.label,
                "middle",
                0.0,
            ));
        }
        marks.extend(bands);
        u.unit(
            vec![i],
            vec![v.unwrap_or(f64::NAN), t.unwrap_or(f64::NAN)],
            &marks,
        );
    }
    y_labels(out, lk, dom, left - 4.0, top, bottom);
    out.push(line(left, bottom, right, bottom, lk.axis, 1.0));
}

/// Lays the bullets out; `u` gets one unit per row (stats: value, target).
pub(crate) fn layout(d: &Data, o: Opts<'_>, w: f64, h: f64, out: &mut Vec<Prim>, u: &mut Units) {
    let pct = [
        o.num("bands", "first", 60.0, 0.0, 1000.0),
        o.num("bands", "second", 80.0, 0.0, 1000.0),
        o.num("bands", "third", 100.0, 0.0, 1000.0),
    ];
    let lk = o.look();
    let vcolor = o.color("colors", "value", &lk.color(0));
    let tcolor = o.color("colors", "target", lk.label);
    let labels = o.flag("labels", "show", false);
    let vertical = o.flag("style", "vertical", false);
    let n = d.rows().min(MAX_ROWS);
    if n == 0 {
        return;
    }
    let val = |i: usize| d.col("value", 0).and_then(|c| c.num(i));
    let tgt = |i: usize| d.col("target", 0).and_then(|c| c.num(i));
    let max = |i: usize| d.col("maximum", 0).and_then(|c| c.num(i));
    let mut vals = Vec::new();
    for i in 0..n {
        vals.extend(val(i));
        vals.extend(tgt(i));
        vals.extend(max(i));
        vals.extend(thresholds(d, i, &pct));
    }
    let dom = domain(&vals, true, None, None, 5.0);
    let cat = d.col("category", 0);
    let names: Vec<String> = (0..n)
        .map(|i| {
            cat.map(|c| crate::spec::clip(&c.label(i), 24))
                .unwrap_or_default()
        })
        .collect();
    if vertical {
        columns(
            d,
            &pct,
            &dom,
            &names,
            cat.is_some(),
            (vcolor.as_str(), tcolor.as_str()),
            labels,
            &lk,
            w,
            h,
            out,
            u,
        );
        return;
    }
    let lw = if cat.is_some() {
        label_width(&names, lk.name, (w - 2.0 * PAD) * 0.4)
    } else {
        0.0
    };
    let left = PAD + lw;
    let right = w - PAD;
    let top = PAD;
    let bottom = h - PAD - 18.0;
    if right - left < 8.0 || bottom - top < 8.0 {
        return;
    }
    let rh = (bottom - top) / n as f64;
    if rh < 3.0 {
        return;
    }
    let bh = (rh * 0.7).min(32.0);
    lk.push_card(out, w, h);
    let xs = |v: f64| scale(&dom, left, right, v);
    let zero = xs(0.0_f64.clamp(dom.lo, dom.hi));
    for i in 0..n {
        let y0 = top + rh * i as f64 + (rh - bh) / 2.0;
        let mid = y0 + bh / 2.0;
        let th: Vec<f64> = thresholds(d, i, &pct)
            .into_iter()
            .map(|t| t.clamp(dom.lo, dom.hi))
            .collect();
        let end = max(i).map_or(dom.hi, |m| m.clamp(dom.lo, dom.hi));
        let k = th.len() + 1;
        let mut from = dom.lo;
        let (mut bands, mut marks) = (Vec::new(), Vec::new());
        for (j, edge) in th.iter().copied().chain(std::iter::once(end)).enumerate() {
            let a = xs(from);
            let b = xs(edge.max(from));
            if b > a {
                bands.push(out.len());
                out.push(lk.band(a, y0, b - a, bh, shade(&lk, j, k), band_side(j, k, "r", "l")));
            }
            from = edge.max(from);
        }
        if let Some(v) = val(i) {
            let x = xs(v);
            let sq = if v >= 0.0 { "l" } else { "r" };
            let bar = |v: f64| {
                let x = xs(v);
                lk.bar(
                    x.min(zero),
                    y0 + bh / 3.0,
                    (x - zero).abs(),
                    bh / 3.0,
                    &vcolor,
                    sq,
                )
            };
            value_bar(d, i, v, &bar, out, &mut marks);
            if labels {
                let side = if v >= 0.0 { 'e' } else { 'w' };
                let first = out.len();
                lk.value_label(out, x, mid, fmt_auto(v), side, 4.0, lk.value);
                marks.extend(first..out.len());
            }
        }
        if let Some(t) = tgt(i) {
            let x = xs(t.clamp(dom.lo, dom.hi));
            marks.push(out.len());
            out.push(line(
                x,
                y0 + bh * 0.15,
                x,
                y0 + bh * 0.85,
                &tcolor,
                target_width(&lk),
            ));
        }
        if let Some(name) = names.get(i).filter(|_| cat.is_some()) {
            out.push(text(
                left - 6.0,
                mid,
                fit(name, lw - 6.0, lk.name),
                lk.name,
                lk.label,
                "end",
                0.0,
            ));
        }
        marks.extend(bands);
        u.unit(
            vec![i],
            vec![val(i).unwrap_or(f64::NAN), tgt(i).unwrap_or(f64::NAN)],
            &marks,
        );
    }
    x_axis(out, &lk, &dom, left, right, bottom, w);
}
