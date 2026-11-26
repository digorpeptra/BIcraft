//! The BIcraft Histogram (docs/formats/bicraft-visuals.md "Histogram"): the values of a numeric
//! column, each weighted by its `frequency` (else 1), counted in bins of equal width (or
//! cumulatively). The same layout is in `crates/engine/src/bvis/histogram.ts`.

use super::{Data, Opts, PAD, Unit, Units, scale, x_axis, y_grid, y_labels};
use crate::spec::{Prim, domain, fmt_auto};

/// Most bins drawn.
pub const MAX_BINS: usize = 200;
/// Most rows a cumulative bin selects.
const MAX_UNIT_ROWS: usize = 5_000;

/// The bins: their start, width, count and the counted weight of each.
#[derive(Debug, Clone, PartialEq)]
pub struct Bins {
    pub start: f64,
    pub size: f64,
    pub counts: Vec<f64>,
}

/// The weighted values: `(value, weight, row)` for rows with a value and a positive weight.
fn rows_values(d: &Data) -> Vec<(f64, f64, usize)> {
    let Some(vc) = d.col("values", 0) else {
        return Vec::new();
    };
    let fc = d.col("frequency", 0);
    (0..d.rows())
        .filter_map(|i| {
            let v = vc.num(i)?;
            let f = match fc {
                Some(c) => c.num(i).filter(|f| *f > 0.0)?,
                None => 1.0,
            };
            Some((v, f, i))
        })
        .collect()
}

/// The data rows counted in each bin of `b` (as [`bins`] counts them).
fn members(vals: &[(f64, f64, usize)], b: &Bins) -> Vec<Vec<usize>> {
    let n = b.counts.len();
    let mut out = vec![Vec::new(); n];
    if n == 0 {
        return out;
    }
    for (v, _, i) in vals {
        let k = ((v - b.start) / b.size).floor().clamp(0.0, (n - 1) as f64) as usize;
        if let Some(m) = out.get_mut(k) {
            m.push(*i);
        }
    }
    out
}

/// The bins of `vals`: `count` bins from the smallest value to the largest, or bins of `size`
/// (when positive) starting at a multiple of it, at most [`MAX_BINS`].
pub fn bins(vals: &[(f64, f64)], count: usize, size: f64) -> Option<Bins> {
    let lo = vals.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let hi = vals.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
    if !(lo.is_finite() && hi.is_finite()) {
        return None;
    }
    let (start, size, n) = if size > 0.0 {
        let start = (lo / size).floor() * size;
        let n = ((hi - start) / size).floor() + 1.0;
        if n > MAX_BINS as f64 {
            // too many: as many bins as allowed over the same range
            (start, (hi - start) / MAX_BINS as f64, MAX_BINS)
        } else {
            (start, size, n.max(1.0) as usize)
        }
    } else if hi == lo {
        (lo - 0.5, 1.0, 1)
    } else {
        let n = count.clamp(1, MAX_BINS);
        (lo, (hi - lo) / n as f64, n)
    };
    if !(size > 0.0 && size.is_finite() && start.is_finite() && (start + size * n as f64).is_finite()) {
        return None;
    }
    let mut counts = vec![0.0; n];
    for (v, f) in vals {
        let k = ((v - start) / size).floor().clamp(0.0, (n - 1) as f64) as usize;
        if let Some(c) = counts.get_mut(k) {
            *c += f;
        }
    }
    Some(Bins { start, size, counts })
}

/// Lays the histogram out; `u` gets one unit per bin (stats: from, to, count, cumulative).
pub(crate) fn layout(d: &Data, o: Opts<'_>, w: f64, h: f64, out: &mut Vec<Prim>, u: &mut Units) {
    let count = o.int("bins", "count", 10.0, 1.0, MAX_BINS as f64) as usize;
    let size = o.num("bins", "size", 0.0, 0.0, 1e15);
    let k = o.look();
    let fill = o.color("colors", "fill", &k.color(0));
    let labels = o.flag("labels", "show", false);
    let cumulative = o.flag("bins", "cumulative", false);
    let vals = rows_values(d);
    let weighted: Vec<(f64, f64)> = vals.iter().map(|(v, f, _)| (*v, *f)).collect();
    let Some(mut b) = bins(&weighted, count, size) else {
        return;
    };
    let mut rows = members(&vals, &b);
    if cumulative {
        // each bar counts its bin and every bin before it
        let mut acc = 0.0;
        for c in &mut b.counts {
            acc += *c;
            *c = acc;
        }
        let mut acc: Vec<usize> = Vec::new();
        for m in &mut rows {
            let room = MAX_UNIT_ROWS.saturating_sub(acc.len());
            acc.extend(m.iter().take(room));
            *m = acc.clone();
        }
    }
    let left = PAD + 36.0;
    let right = w - PAD;
    let top = PAD + 6.0;
    let bottom = h - PAD - 18.0;
    if right - left < 8.0 || bottom - top < 8.0 {
        return;
    }
    let n = b.counts.len();
    let end = b.start + b.size * n as f64;
    let xd = domain(&[], false, Some(b.start), Some(end), 5.0);
    let yd = domain(&b.counts, true, None, None, 5.0);
    k.push_card(out, w, h);
    y_grid(out, &k, &yd, left, right, top, bottom);
    for (j, c) in b.counts.iter().enumerate() {
        let from = if cumulative {
            b.start
        } else {
            b.start + b.size * j as f64
        };
        u.units.push(Unit {
            rows: rows.get(j).cloned().unwrap_or_default(),
            stats: vec![
                from,
                b.start + b.size * (j + 1) as f64,
                *c,
                if cumulative { 1.0 } else { 0.0 },
            ],
        });
        if *c <= 0.0 {
            continue;
        }
        let x0 = scale(&xd, left, right, b.start + b.size * j as f64);
        let x1 = scale(&xd, left, right, b.start + b.size * (j + 1) as f64);
        let bw = x1 - x0;
        // a surface gap between touching bars: 2 px in the modern looks when they are wide
        let gap = if !k.minimal() && bw > 6.0 {
            2.0
        } else if bw > 3.0 {
            1.0
        } else {
            0.0
        };
        let y = scale(&yd, bottom, top, *c);
        u.hits.push((j, out.len()));
        out.push(k.bar(x0 + gap / 2.0, y, bw - gap, bottom - y, &fill, "b"));
        if labels {
            let first = out.len();
            k.value_label(out, (x0 + x1) / 2.0, y, fmt_auto(*c), 'n', 7.0, k.value);
            u.hits.extend((first..out.len()).map(|p| (j, p)));
        }
    }
    y_labels(out, &k, &yd, left - 4.0, top, bottom);
    x_axis(out, &k, &xd, left, right, bottom, w);
}
