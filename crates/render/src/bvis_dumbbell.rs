//! The BIcraft Dumbbell and slope chart (docs/formats/bicraft-visuals.md "Dumbbell and slope
//! chart"): two measures per category, `start` and `end`. As a dumbbell, one row per category
//! with a dot at each value joined by a line, on one horizontal scale; as a slope chart, two
//! vertical axes (start on the left, end on the right) with a line per category. The same
//! layout is in `crates/engine/src/bvis/dumbbell.ts`.

use super::{Data, Opts, PAD, Units, fit, label_width, legend, path, scale, x_axis};
use crate::look::{Look, mix};
use crate::spec::{Prim, clip, domain, est, fmt_auto, inside, line, text};

/// Most rows drawn.
pub const MAX_ROWS: usize = 200;
/// The line joining the two values.
const LINE: &str = "#A6A6A6";

struct Rows<'a> {
    d: &'a Data,
    n: usize,
}

impl Rows<'_> {
    fn start(&self, i: usize) -> Option<f64> {
        self.d.col("start", 0).and_then(|c| c.num(i))
    }
    fn end(&self, i: usize) -> Option<f64> {
        self.d.col("end", 0).and_then(|c| c.num(i))
    }
    fn name(&self, i: usize) -> String {
        self.d
            .col("category", 0)
            .map(|c| clip(&c.label(i), 24))
            .unwrap_or_default()
    }
    fn values(&self) -> Vec<f64> {
        (0..self.n)
            .flat_map(|i| self.start(i).into_iter().chain(self.end(i)))
            .collect()
    }
    /// Row `i`'s unit: `marks` drawn for it (a dot first), stats: start, end.
    fn unit(&self, i: usize, marks: &[usize], u: &mut Units) {
        let stats = vec![self.start(i).unwrap_or(f64::NAN), self.end(i).unwrap_or(f64::NAN)];
        u.unit(vec![i], stats, marks);
    }
    fn field(&self, role: &str, def: &str) -> String {
        self.d
            .col(role, 0)
            .map(|c| c.name.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| def.to_string())
    }
}

/// Lays the dumbbell or slope chart out; `u` gets one unit per row.
pub(crate) fn layout(d: &Data, o: Opts<'_>, w: f64, h: f64, out: &mut Vec<Prim>, u: &mut Units) {
    let slope_mode = o.flag("style", "slope", false);
    let r = Rows {
        d,
        n: d.rows().min(MAX_ROWS),
    };
    if r.n == 0 {
        return;
    }
    let k = o.look();
    // the line: Minimal's grey, else the two colours' mixed hue lightened towards the surface
    let start = o.color("colors", "start", &k.color(0));
    let end = o.color("colors", "end", &k.color(1));
    let line_def = if k.minimal() {
        LINE.to_string()
    } else {
        mix(&mix(&start, &end, 0.5), &k.surface, 0.55)
    };
    let c = Colors {
        line: o.color("colors", "line", &line_def),
        start,
        end,
    };
    let labels = o.flag("labels", "show", true);
    if slope_mode {
        slope(&r, &c, &k, labels, w, h, out, u);
    } else {
        dumbbell(&r, &c, &k, labels, w, h, out, u);
    }
}

/// A line joining two points: Minimal's plain line, else a path with round caps.
#[allow(clippy::too_many_arguments)]
fn join(k: &Look, x1: f64, y1: f64, x2: f64, y2: f64, stroke: &str, width: f64) -> Prim {
    if k.minimal() {
        line(x1, y1, x2, y2, stroke, width)
    } else {
        path(&[(x1, y1), (x2, y2)], stroke, width)
    }
}

struct Colors {
    start: String,
    end: String,
    line: String,
}

#[allow(clippy::too_many_arguments)]
fn dumbbell(
    r: &Rows<'_>,
    c: &Colors,
    k: &Look,
    labels: bool,
    w: f64,
    h: f64,
    out: &mut Vec<Prim>,
    u: &mut Units,
) {
    let names: Vec<String> = (0..r.n).map(|i| r.name(i)).collect();
    let has_cat = r.d.col("category", 0).is_some();
    let lw = if has_cat {
        label_width(&names, k.name, (w - 2.0 * PAD) * 0.4)
    } else {
        0.0
    };
    let top = PAD + 18.0;
    let bottom = h - PAD - 18.0;
    if bottom - top < 8.0 {
        return;
    }
    let rh = (bottom - top) / r.n as f64;
    if rh < 3.0 {
        return;
    }
    let rad = (rh * 0.3).clamp(2.0, if k.minimal() { 6.0 } else { 7.0 });
    let stem = k.stem_width().min(rad * 1.2);
    let gap = rad + 4.0;
    let pad_x = if k.pills { 5.0 } else { 0.0 };
    // each row's smaller and larger value (one alone counts as the larger)
    let ends = |i: usize| match (r.start(i), r.end(i)) {
        (Some(a), Some(b)) if a > b => (Some(b), Some(a)),
        (Some(a), Some(b)) => (Some(a), Some(b)),
        (x, None) | (None, x) => (None, x),
    };
    let width = |v: f64| est(&fmt_auto(v), k.value) + 2.0 * pad_x;
    // the margins beside the scale: room for the widest smaller value's label on the left and
    // the widest larger one's on the right (at least the look's margin, at most 30% of the plot)
    let (mut w_lo, mut w_hi) = (0.0f64, 0.0f64);
    if labels {
        for i in 0..r.n {
            let (lo, hi) = ends(i);
            w_lo = lo.map_or(w_lo, |v| w_lo.max(width(v)));
            w_hi = hi.map_or(w_hi, |v| w_hi.max(width(v)));
        }
    }
    let base = 28.0 + (k.value - 10.0) * 3.0 + if k.pills { 10.0 } else { 0.0 };
    let room = (w - 2.0 * PAD - lw) * 0.3;
    let margin = |wd: f64| {
        if labels {
            base.max((gap + wd + 2.0).min(room))
        } else {
            0.0
        }
    };
    let left = PAD + lw + margin(w_lo);
    let right = w - PAD - margin(w_hi);
    if right - left < 8.0 {
        return;
    }
    k.push_card(out, w, h);
    let dom = domain(&r.values(), false, None, None, 5.0);
    let xs = |v: f64| scale(&dom, left, right, v);
    for t in &dom.ticks {
        out.push(line(xs(*t), top, xs(*t), bottom, k.grid, 1.0));
    }
    // the value labels: the smaller value on the left of its dot, the larger on the right (on
    // the other side where a margin capped at 30% leaves it no room, left out where neither side
    // has room), moved apart where they would overlap (`spread_boxes`, kept above the axis),
    // those that cannot fit in the plot left out (`fit_inside`)
    let (lo_x, hi_x) = (PAD + lw, w - 2.0);
    let mut tags: Vec<Tag> = Vec::new();
    if labels {
        for i in 0..r.n {
            let y = top + rh * (i as f64 + 0.5);
            let (lo, hi) = ends(i);
            for (v, side) in [(lo, 'w'), (hi, 'e')] {
                let Some(v) = v else {
                    continue;
                };
                let (x, wd) = (xs(v), width(v));
                let fits_w = x - gap - wd >= lo_x;
                let fits_e = x + gap + wd <= hi_x;
                let side = match side {
                    'w' if !fits_w && fits_e => 'e',
                    'e' if !fits_e && fits_w => 'w',
                    s => s,
                };
                if (side == 'w' && !fits_w) || (side == 'e' && !fits_e) {
                    continue;
                }
                tags.push(Tag {
                    row: i,
                    x,
                    y,
                    s: fmt_auto(v),
                    side,
                });
            }
        }
    }
    let spans: Vec<(f64, f64)> = tags
        .iter()
        .map(|t| {
            let wd = est(&t.s, k.value) + 2.0 * pad_x;
            if t.side == 'w' {
                (t.x - gap - wd, t.x - gap)
            } else {
                (t.x + gap, t.x + gap + wd)
            }
        })
        .collect();
    let ys: Vec<f64> = tags.iter().map(|t| t.y).collect();
    let pill_gap = if k.pills { 6.0 } else { 0.0 };
    let (gap_y, hi) = (k.value + 1.0 + pill_gap, bottom - k.value / 2.0);
    let lo = (top + k.value / 2.0).min(top + rh / 2.0);
    let at = fit_inside(&ys, lo, &|keep: &[usize]| {
        let sub: Vec<f64> = keep.iter().map(|i| ys.get(*i).copied().unwrap_or(0.0)).collect();
        let spans: Vec<(f64, f64)> = keep
            .iter()
            .map(|i| spans.get(*i).copied().unwrap_or((0.0, 0.0)))
            .collect();
        spread_boxes(&sub, &spans, gap_y, hi)
    });
    for i in 0..r.n {
        let y = top + rh * (i as f64 + 0.5);
        let (a, b) = (r.start(i), r.end(i));
        let (mut dots, mut marks) = (Vec::new(), Vec::new());
        if let (Some(a), Some(b)) = (a, b) {
            marks.push(out.len());
            out.push(join(k, xs(a), y, xs(b), y, &c.line, stem));
        }
        if let Some(a) = a {
            dots.push(out.len());
            out.push(k.dot(xs(a), y, rad, &c.start, 1.0));
        }
        if let Some(b) = b {
            dots.push(out.len());
            out.push(k.dot(xs(b), y, rad, &c.end, 1.0));
        }
        for (t, ty) in tags.iter().zip(&at).filter(|(t, _)| t.row == i) {
            let Some(ty) = ty else {
                continue;
            };
            if (ty - y).abs() > 1.0 {
                let ex = if t.side == 'w' { t.x - gap } else { t.x + gap };
                marks.push(out.len());
                out.push(leader(k, t.x, y, rad, ex, *ty));
            }
            let first = out.len();
            k.value_label(out, t.x, *ty, t.s.clone(), t.side, gap, k.value);
            marks.extend(first..out.len());
        }
        if has_cat {
            let s = names.get(i).map(|s| fit(s, lw - 6.0, k.name)).unwrap_or_default();
            out.push(text(PAD + lw - 6.0, y, s, k.name, k.label, "end", 0.0));
        }
        // a dot first: it takes the keyboard focus
        dots.extend(marks);
        r.unit(i, &dots, u);
    }
    x_axis(out, k, &dom, left, right, bottom, w);
    legend(
        out,
        k,
        &[
            (r.field("start", "Start"), c.start.clone()),
            (r.field("end", "End"), c.end.clone()),
        ],
        w,
    );
}

/// A dumbbell's value label: its row, its dot's centre, its text and its side (`w` or `e`).
struct Tag {
    row: usize,
    x: f64,
    y: f64,
    s: String,
    side: char,
}

/// A thin line from the edge of the dot at (`cx`, `cy`) of radius `rad` to a label moved away
/// from it, at (`x`, `y`).
pub(crate) fn leader(k: &Look, cx: f64, cy: f64, rad: f64, x: f64, y: f64) -> Prim {
    let (dx, dy) = (x - cx, y - cy);
    let len = (dx * dx + dy * dy).sqrt();
    let f = if len > rad { rad / len } else { 0.0 };
    line(cx + dx * f, cy + dy * f, x, y, k.axis, 0.75)
}

/// Label positions `ys` (any order) moved apart as [`spread`] moves them, but only between
/// labels whose horizontal spans (`spans`, `(from, to)`) come within 2 px of each other.
pub fn spread_boxes(ys: &[f64], spans: &[(f64, f64)], gap: f64, hi: f64) -> Vec<f64> {
    let mut order: Vec<usize> = (0..ys.len()).collect();
    let y = |i: &usize| ys.get(*i).copied().unwrap_or(0.0);
    order.sort_by(|a, b| y(a).total_cmp(&y(b)).then(a.cmp(b)));
    let span = |i: usize| spans.get(i).copied().unwrap_or((0.0, 0.0));
    let near = |a: usize, b: usize| {
        let (p, q) = (span(a), span(b));
        p.0 < q.1 + 2.0 && q.0 < p.1 + 2.0
    };
    let mut out = ys.to_vec();
    for (p, &i) in order.iter().enumerate() {
        let mut v = out.get(i).copied().unwrap_or(0.0);
        for &j in order.iter().take(p) {
            if near(j, i) {
                v = v.max(out.get(j).copied().unwrap_or(0.0) + gap);
            }
        }
        if let Some(s) = out.get_mut(i) {
            *s = v;
        }
    }
    for (p, &i) in order.iter().enumerate().rev() {
        let mut v = out.get(i).copied().unwrap_or(0.0).min(hi);
        for &j in order.iter().skip(p + 1) {
            if near(j, i) {
                v = v.min(out.get(j).copied().unwrap_or(0.0) - gap);
            }
        }
        if let Some(s) = out.get_mut(i) {
            *s = v;
        }
    }
    out
}

/// Label positions `ys` (any order) placed by `place` (given the indices of the labels kept, it
/// returns their positions in that order), keeping only those it leaves at `lo` or below: a label
/// placed above `lo` is left out (`None`) and the others placed again, until all of them fit.
pub fn fit_inside(ys: &[f64], lo: f64, place: &dyn Fn(&[usize]) -> Vec<f64>) -> Vec<Option<f64>> {
    let mut keep: Vec<usize> = (0..ys.len()).collect();
    // each round leaves one label out at least, so it ends with every label kept placed
    for _ in 0..=ys.len() {
        let at = place(&keep);
        let fits = |p: usize| at.get(p).is_some_and(|y| *y >= lo - 1e-9);
        if (0..keep.len()).all(fits) {
            let mut out = vec![None; ys.len()];
            for (p, i) in keep.iter().enumerate() {
                if let Some(s) = out.get_mut(*i) {
                    *s = at.get(p).copied();
                }
            }
            return out;
        }
        keep = keep
            .iter()
            .enumerate()
            .filter(|(p, _)| fits(*p))
            .map(|(_, i)| *i)
            .collect();
    }
    vec![None; ys.len()]
}

/// Label positions `ys` (any order) moved apart to at least `gap` in their order from the top
/// (ties in list order), pushed down, then up where they would pass `hi`.
pub fn spread(ys: &[f64], gap: f64, hi: f64) -> Vec<f64> {
    let mut order: Vec<usize> = (0..ys.len()).collect();
    let y = |i: &usize| ys.get(*i).copied().unwrap_or(0.0);
    order.sort_by(|a, b| y(a).total_cmp(&y(b)).then(a.cmp(b)));
    let mut out = ys.to_vec();
    let mut prev = f64::NEG_INFINITY;
    for i in &order {
        if let Some(v) = out.get_mut(*i) {
            *v = v.max(prev + gap);
            prev = *v;
        }
    }
    let mut next = hi + gap;
    for i in order.iter().rev() {
        if let Some(v) = out.get_mut(*i) {
            *v = v.min(next - gap);
            next = *v;
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn slope(
    r: &Rows<'_>,
    c: &Colors,
    k: &Look,
    labels: bool,
    w: f64,
    h: f64,
    out: &mut Vec<Prim>,
    u: &mut Units,
) {
    let left_text = |i: usize| r.start(i).map(|v| format!("{} {}", r.name(i), fmt_auto(v)));
    let right_text = |i: usize| r.end(i).map(|v| format!("{} {}", fmt_auto(v), r.name(i)));
    let lw = if labels {
        let texts: Vec<String> = (0..r.n)
            .flat_map(|i| left_text(i).into_iter().chain(right_text(i)))
            .collect();
        label_width(&texts, k.tick, (w - 2.0 * PAD) * 0.3)
    } else {
        0.0
    };
    let x0 = PAD + lw.max(4.0);
    let x1 = w - PAD - lw.max(4.0);
    let top = PAD + 24.0;
    let bottom = h - PAD - 6.0;
    if x1 - x0 < 8.0 || bottom - top < 8.0 {
        return;
    }
    k.push_card(out, w, h);
    let dom = domain(&r.values(), false, None, None, 5.0);
    let ys = |v: f64| scale(&dom, bottom, top, v);
    // each side's labels moved apart so they do not overlap
    let mut at: [Vec<Option<f64>>; 2] = [vec![None; r.n], vec![None; r.n]];
    for (side, slots) in at.iter_mut().enumerate() {
        let (mut want, mut which) = (Vec::new(), Vec::new());
        for i in 0..r.n {
            let v = if side == 0 { r.start(i) } else { r.end(i) };
            if let Some(v) = v.filter(|_| labels) {
                want.push(ys(v));
                which.push(i);
            }
        }
        // those that cannot fit below the axes' top are left out
        let got = fit_inside(&want, top, &|keep: &[usize]| {
            let sub: Vec<f64> = keep
                .iter()
                .map(|i| want.get(*i).copied().unwrap_or(0.0))
                .collect();
            spread(&sub, k.tick + 1.0, bottom)
        });
        for (i, y) in which.into_iter().zip(got) {
            if let Some(s) = slots.get_mut(i) {
                *s = y;
            }
        }
    }
    for (x, role, def) in [(x0, "start", "Start"), (x1, "end", "End")] {
        let name = clip(&r.field(role, def), 30);
        let cx = inside(x, &name, k.name, w);
        out.push(text(cx, PAD + 7.0, name, k.name, k.text, "middle", 0.0));
        out.push(line(x, top, x, bottom, k.axis, 1.0));
    }
    let dot_r = if k.minimal() { 4.0 } else { 4.5 };
    for i in 0..r.n {
        let (a, b) = (r.start(i), r.end(i));
        let (mut dots, mut marks) = (Vec::new(), Vec::new());
        if let (Some(a), Some(b)) = (a, b) {
            marks.push(out.len());
            out.push(join(k, x0, ys(a), x1, ys(b), &c.line, k.line_width()));
        }
        if let Some(a) = a {
            dots.push(out.len());
            out.push(k.dot(x0, ys(a), dot_r, &c.start, 1.0));
            if let Some(s) = left_text(i).filter(|_| labels)
                && let Some(y) = at[0].get(i).copied().flatten()
            {
                let s = fit(&s, lw - 8.0, k.tick);
                if (y - ys(a)).abs() > 1.0 {
                    marks.push(out.len());
                    out.push(leader(k, x0, ys(a), dot_r, x0 - 8.0, y));
                }
                marks.push(out.len());
                out.push(text(x0 - 8.0, y, s, k.tick, k.label, "end", 0.0));
            }
        }
        if let Some(b) = b {
            dots.push(out.len());
            out.push(k.dot(x1, ys(b), dot_r, &c.end, 1.0));
            if let Some(s) = right_text(i).filter(|_| labels)
                && let Some(y) = at[1].get(i).copied().flatten()
            {
                let s = fit(&s, lw - 8.0, k.tick);
                if (y - ys(b)).abs() > 1.0 {
                    marks.push(out.len());
                    out.push(leader(k, x1, ys(b), dot_r, x1 + 8.0, y));
                }
                marks.push(out.len());
                out.push(text(x1 + 8.0, y, s, k.tick, k.label, "start", 0.0));
            }
        }
        dots.extend(marks);
        r.unit(i, &dots, u);
    }
}
