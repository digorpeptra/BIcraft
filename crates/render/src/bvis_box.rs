//! The BIcraft Box plot (docs/formats/bicraft-visuals.md "Box plot"): for each category, the
//! distribution of a measure over the items of the `sample` role: quartiles (linear
//! interpolation between order statistics), whiskers to the furthest values within 1.5 IQR of
//! the box, the values beyond as outliers, and the mean. The same layout is in
//! `crates/engine/src/bvis/box_plot.ts`.

use super::{Data, Opts, PAD, Units, label_width, path, scale, x_axis, y_grid, y_labels};
use crate::look::mix;
use crate::spec::{Prim, clip, domain, inside, line, text};

/// Most boxes drawn.
pub const MAX_BOXES: usize = 100;
/// Most outliers drawn per box.
pub const MAX_OUTLIERS: usize = 500;

/// One box's statistics.
#[derive(Debug, Clone, PartialEq)]
pub struct Stats {
    pub n: usize,
    pub min: f64,
    pub q1: f64,
    pub median: f64,
    pub q3: f64,
    pub max: f64,
    /// The whiskers' ends: the furthest values within 1.5 IQR of the box.
    pub low: f64,
    pub high: f64,
    pub mean: f64,
    pub outliers: Vec<f64>,
}

/// The `p` quantile of sorted values, interpolated between order statistics.
fn quantile(s: &[f64], p: f64) -> f64 {
    let h = (s.len().saturating_sub(1)) as f64 * p;
    let lo = h.floor() as usize;
    let a = s.get(lo).copied().unwrap_or(0.0);
    let b = s.get(lo + 1).copied().unwrap_or(a);
    a + (h - lo as f64) * (b - a)
}

/// The statistics of `vals` (any order), `None` when empty.
pub fn stats(vals: &[f64]) -> Option<Stats> {
    let mut s = vals.to_vec();
    s.sort_by(f64::total_cmp);
    let (&min, &max) = (s.first()?, s.last()?);
    let q1 = quantile(&s, 0.25);
    let median = quantile(&s, 0.5);
    let q3 = quantile(&s, 0.75);
    let iqr = q3 - q1;
    let (lf, hf) = (q1 - 1.5 * iqr, q3 + 1.5 * iqr);
    let low = s.iter().copied().find(|v| *v >= lf).unwrap_or(min);
    let high = s.iter().rev().copied().find(|v| *v <= hf).unwrap_or(max);
    let mean = s.iter().sum::<f64>() / s.len() as f64;
    let outliers = s.iter().copied().filter(|v| *v < lf || *v > hf).collect();
    Some(Stats {
        n: s.len(),
        min,
        q1,
        median,
        q3,
        max,
        low,
        high,
        mean,
        outliers,
    })
}

/// The boxes: each category's label (by first appearance; dates with the column's format), its
/// values and their rows. Categories are told apart by their value, not their label.
pub fn groups(d: &Data) -> Vec<(String, Vec<f64>, Vec<usize>)> {
    let mut out: Vec<(String, Vec<f64>, Vec<usize>)> = Vec::new();
    let mut keys: Vec<String> = Vec::new();
    let Some(vc) = d.col("value", 0) else {
        return out;
    };
    let cat = d.col("category", 0);
    for i in 0..d.rows() {
        let Some(v) = vc.num(i) else { continue };
        let key = cat.map(|c| c.text(i)).unwrap_or_default();
        match keys.iter().position(|k| *k == key) {
            Some(g) => {
                if let Some(x) = out.get_mut(g) {
                    x.1.push(v);
                    x.2.push(i);
                }
            }
            None if out.len() < MAX_BOXES => {
                keys.push(key);
                out.push((cat.map(|c| c.label(i)).unwrap_or_default(), vec![v], vec![i]));
            }
            None => {}
        }
    }
    out
}

/// Lays the boxes out; `u` gets one unit per box (stats: count, minimum, quartiles, maximum,
/// mean, outliers).
pub(crate) fn layout(d: &Data, o: Opts<'_>, w: f64, h: f64, out: &mut Vec<Prim>, u: &mut Units) {
    let k = o.look();
    let fill = o.color("colors", "fill", &k.color(0));
    let plain = k.minimal() && !k.rounded && !k.gradient;
    let (whisker, median_w, median_c) = if k.minimal() {
        (1.0, 2.0, fill.clone())
    } else {
        (1.5, 2.5, mix(&fill, "#000000", 0.25))
    };
    let show_mean = o.flag("box", "mean", true);
    let show_out = o.flag("box", "outliers", true);
    let horizontal = o.flag("box", "horizontal", false);
    let boxes: Vec<(String, Stats, Vec<usize>)> = groups(d)
        .into_iter()
        .filter_map(|(k, v, r)| stats(&v).map(|s| (k, s, r)))
        .collect();
    if boxes.is_empty() {
        return;
    }
    let has_cat = d.col("category", 0).is_some();
    let mut vals = Vec::new();
    for (_, s, _) in &boxes {
        vals.push(s.low);
        vals.push(s.high);
        if show_mean {
            vals.push(s.mean);
        }
        if show_out {
            vals.extend(s.outliers.iter().take(MAX_OUTLIERS));
        }
    }
    let dom = domain(&vals, false, None, None, 5.0);
    // horizontal: the category names in a column on the left, the values along the bottom
    let names: Vec<String> = boxes.iter().map(|(n, _, _)| clip(n, 24)).collect();
    let lw = if horizontal && has_cat {
        label_width(&names, k.tick, (w - 2.0 * PAD) * 0.4)
    } else {
        0.0
    };
    let left = if horizontal { PAD + lw } else { PAD + 36.0 };
    let right = w - PAD;
    let top = if horizontal { PAD } else { PAD + 6.0 };
    let bottom = if horizontal {
        h - PAD - 18.0
    } else {
        h - PAD - if has_cat { 18.0 } else { 0.0 }
    };
    if right - left < 8.0 || bottom - top < 8.0 {
        return;
    }
    // the point at value `v` and `u` across the value axis
    let pt = |v: f64, u: f64| {
        if horizontal {
            (scale(&dom, left, right, v), u)
        } else {
            (u, scale(&dom, bottom, top, v))
        }
    };
    let seg = |v1: f64, u1: f64, v2: f64, u2: f64, width: f64| {
        let (a, b) = (pt(v1, u1), pt(v2, u2));
        line(a.0, a.1, b.0, b.1, &fill, width)
    };
    k.push_card(out, w, h);
    if horizontal {
        for t in &dom.ticks {
            let x = scale(&dom, left, right, *t);
            out.push(line(x, top, x, bottom, k.grid, 1.0));
        }
    } else {
        y_grid(out, &k, &dom, left, right, top, bottom);
    }
    let step = if horizontal { bottom - top } else { right - left } / boxes.len() as f64;
    let bw = (step * 0.5).min(48.0);
    for (g, (_, s, rows)) in boxes.iter().enumerate() {
        let c = if horizontal { top } else { left } + step * (g as f64 + 0.5);
        let first = out.len();
        out.push(seg(s.low, c, s.q1, c, whisker));
        out.push(seg(s.q3, c, s.high, c, whisker));
        out.push(seg(s.low, c - bw / 4.0, s.low, c + bw / 4.0, whisker));
        out.push(seg(s.high, c - bw / 4.0, s.high, c + bw / 4.0, whisker));
        let (q1, q3) = (pt(s.q1, c - bw / 2.0), pt(s.q3, c + bw / 2.0));
        let bx = out.len();
        out.push(k.boxed(
            q1.0.min(q3.0),
            q1.1.min(q3.1),
            (q3.0 - q1.0).abs(),
            (q3.1 - q1.1).abs(),
            &fill,
        ));
        if plain {
            out.push(path(
                &[
                    pt(s.q3, c - bw / 2.0),
                    pt(s.q3, c + bw / 2.0),
                    pt(s.q1, c + bw / 2.0),
                    pt(s.q1, c - bw / 2.0),
                    pt(s.q3, c - bw / 2.0),
                ],
                &fill,
                1.0,
            ));
        }
        let (a, b) = (pt(s.median, c - bw / 2.0), pt(s.median, c + bw / 2.0));
        out.push(line(a.0, a.1, b.0, b.1, &median_c, median_w));
        if show_mean {
            let m = pt(s.mean, c);
            out.push(k.dot(m.0, m.1, if k.minimal() { 3.0 } else { 3.5 }, k.label, 1.0));
        }
        if show_out {
            for v in s.outliers.iter().take(MAX_OUTLIERS) {
                let q = pt(*v, c);
                let (r, a) = if k.minimal() { (2.5, 0.6) } else { (3.0, 0.75) };
                out.push(k.dot(q.0, q.1, r, &fill, a));
            }
        }
        // the box first: it takes the keyboard focus
        let marks: Vec<usize> = std::iter::once(bx)
            .chain((first..out.len()).filter(|p| *p != bx))
            .collect();
        let stats = vec![
            s.n as f64,
            s.min,
            s.q1,
            s.median,
            s.q3,
            s.max,
            s.mean,
            s.outliers.len() as f64,
        ];
        u.unit(rows.clone(), stats, &marks);
    }
    if horizontal {
        x_axis(out, &k, &dom, left, right, bottom, w);
        if has_cat {
            for (g, name) in names.iter().enumerate() {
                let y = top + step * (g as f64 + 0.5);
                let s = super::fit(name, lw - 6.0, k.tick);
                out.push(text(left - 6.0, y, s, k.tick, k.text, "end", 0.0));
            }
        }
        return;
    }
    y_labels(out, &k, &dom, left - 4.0, top, bottom);
    out.push(line(left, bottom, right, bottom, k.axis, 1.0));
    if has_cat {
        for (g, (name, _, _)) in boxes.iter().enumerate() {
            let cx = left + step * (g as f64 + 0.5);
            let s = super::fit(name, step - 4.0, k.tick);
            out.push(text(
                inside(cx, &s, k.tick, w),
                bottom + 9.0,
                s,
                k.tick,
                k.text,
                "middle",
                0.0,
            ));
        }
    }
}
