//! BIcraft Scatter+ (docs/formats/chart-pack.md "Scatter+"): a scatter chart that draws every
//! point up to [`MAX_POINTS`] (none sampled), switches to a hexagonal density view above a
//! threshold, can add marginal histograms or box strips on both axes, a least-squares trend
//! line, quadrant lines at averages or given values with their labels, and labels on the top N
//! or the selected points. Markers can be filled, bordered or both, sized by a measure and
//! coloured by a legend. The Rust twin of `crates/engine/src/chart_pack/scatter.ts`.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};

use super::{
    Out, PAD, cell_label, cell_num, cell_text, compact, dashes, est, fit, highlighted, hl_num, is_fr, legend,
    line, look_of, opt_bool, opt_enum, opt_num, opt_text, options_of, poly, rows_of, text,
};
use crate::look::{Look, dim, mix, text_w};
use crate::spec::{Deco, Domain, Prim, domain, fmt_fixed, inside, r2, tick_decimals};

/// Most points drawn (every one of them; the rows after it are left out and the visual says
/// so).
pub const MAX_POINTS: usize = 10_000;
/// Most legend groups with their own colour (later groups take the last one's).
pub const MAX_GROUPS: usize = 12;
/// Most point labels.
pub const MAX_LABELS: usize = 50;
/// Most rows a density cell selects.
pub const MAX_CELL_ROWS: usize = 1000;
/// Bins of a marginal histogram.
const BINS: usize = 20;
/// The side of a marginal strip.
const STRIP: f64 = 40.0;

struct Pt {
    row: usize,
    x: f64,
    y: f64,
    s: Option<f64>,
    g: usize,
    /// Not left out by another visual's highlight.
    lit: bool,
}

/// The plot's frame: its domains and pixel bounds.
struct Frame {
    xd: Domain,
    yd: Domain,
    x0: f64,
    x1: f64,
    y0: f64,
    y1: f64,
}

impl Frame {
    fn sx(&self, v: f64) -> f64 {
        self.x0 + ((v - self.xd.lo) / (self.xd.hi - self.xd.lo)) * (self.x1 - self.x0)
    }

    fn sy(&self, v: f64) -> f64 {
        self.y1 - ((v - self.yd.lo) / (self.yd.hi - self.yd.lo)) * (self.y1 - self.y0)
    }
}

fn name_of(input: &Value, k: usize) -> String {
    input
        .get("names")
        .and_then(|l| l.get(k))
        .and_then(Value::as_str)
        .map(|s| s.chars().take(80).collect())
        .unwrap_or_default()
}

fn tick_text(t: f64, d: &Domain) -> String {
    compact(t, Some(tick_decimals(d.step)))
}

fn cmp(a: f64, b: f64) -> Ordering {
    a.partial_cmp(&b).unwrap_or(Ordering::Equal)
}

/// The linear interpolation quantile `q` of sorted values.
fn quantile(s: &[f64], q: f64) -> f64 {
    let n = s.len();
    if n == 0 {
        return 0.0;
    }
    let pos = q * (n - 1) as f64;
    let lo = pos.floor();
    let li = (lo as usize).min(n - 1);
    let hi = (li + 1).min(n - 1);
    let (a, b) = (
        s.get(li).copied().unwrap_or(0.0),
        s.get(hi).copied().unwrap_or(0.0),
    );
    a + (b - a) * (pos - lo)
}

/// The five numbers of a box strip: whisker ends (the extreme values within 1.5 IQR),
/// quartiles, median.
fn five_of(vals: &[f64]) -> [f64; 5] {
    let mut s = vals.to_vec();
    s.sort_by(|a, b| cmp(*a, *b));
    let q1 = quantile(&s, 0.25);
    let med = quantile(&s, 0.5);
    let q3 = quantile(&s, 0.75);
    let iqr = q3 - q1;
    let (mut lo, mut hi) = (q1, q3);
    for &v in &s {
        if v >= q1 - 1.5 * iqr && v < lo {
            lo = v;
        }
        if v <= q3 + 1.5 * iqr && v > hi {
            hi = v;
        }
    }
    [lo, q1, med, q3, hi]
}

fn bin_of(v: f64, d: &Domain) -> usize {
    let b = (((v - d.lo) / (d.hi - d.lo)) * BINS as f64).floor();
    b.clamp(0.0, (BINS - 1) as f64) as usize
}

/// 0 for an even row of hexagons, 1 for an odd one.
fn odd(j: f64) -> f64 {
    j - 2.0 * (j / 2.0).floor()
}

/// Lays out `{rows: [[details, legend, x, y, size]], names, options, selected?}` in a `w`×`h`
/// visual.
pub fn layout(input: &Value, w: f64, h: f64) -> Out {
    let o = options_of(input);
    let k = look_of(input);
    let fr = is_fr(input);
    let mut out = Out::default();
    let rows = rows_of(input, MAX_POINTS);
    let total = input.get("rows").and_then(Value::as_array).map_or(0, Vec::len);
    let lit = highlighted(input);
    let mut groups: Vec<String> = Vec::new();
    let mut group_ix: HashMap<String, usize> = HashMap::new();
    let mut pts: Vec<Pt> = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        let (Some(x), Some(y)) = (cell_num(r.get(2)), cell_num(r.get(3))) else {
            continue;
        };
        let key = cell_text(r.get(1));
        let g = match group_ix.get(&key) {
            Some(&g) => g,
            None if groups.len() < MAX_GROUPS => {
                group_ix.insert(key, groups.len());
                groups.push(cell_label(input, 1, r.get(1)));
                groups.len() - 1
            }
            None => MAX_GROUPS - 1,
        };
        pts.push(Pt {
            row: i,
            x,
            y,
            s: cell_num(r.get(4)),
            g,
            lit: !lit || hl_num(input, i).is_some(),
        });
    }
    if pts.is_empty() {
        return out;
    }
    let size = opt_num(o, "markerSize", 2.0, 20.0, 5.0);
    let mut fill = opt_bool(o, "markerFill", true);
    let border = opt_bool(o, "markerBorder", true);
    if !fill && !border {
        fill = true;
    }
    let fill_opacity = opt_num(o, "fillOpacity", 10.0, 100.0, 70.0) / 100.0;
    let density = opt_enum(o, "density", &["auto", "on", "off"], "auto");
    let threshold = opt_num(o, "densityThreshold", 100.0, MAX_POINTS as f64, 2000.0);
    let hex_size = opt_num(o, "hexSize", 4.0, 40.0, 10.0);
    let marginals = opt_enum(o, "marginals", &["none", "histogram", "box"], "none");
    let trend = opt_bool(o, "trendLine", false);
    let quadrants = opt_bool(o, "quadrants", false);
    let quadrant_at = opt_enum(o, "quadrantAt", &["average", "value"], "average");
    let label_mode = opt_enum(o, "labelMode", &["none", "top", "selected"], "none");
    let label_count = opt_num(o, "labelCount", 1.0, MAX_LABELS as f64, 5.0).floor() as usize;
    let show_legend = opt_bool(o, "showLegend", true);
    let font_size = opt_num(o, "fontSize", 8.0, 20.0, k.tick);
    let (mut mx, mut my) = (0.0, 0.0);
    for q in &pts {
        mx += q.x;
        my += q.y;
    }
    mx /= pts.len() as f64;
    my /= pts.len() as f64;
    let (qx, qy) = if quadrant_at == "value" {
        (
            opt_num(o, "quadrantX", -1e15, 1e15, 0.0),
            opt_num(o, "quadrantY", -1e15, 1e15, 0.0),
        )
    } else {
        (mx, my)
    };
    let mut xs: Vec<f64> = pts.iter().map(|q| q.x).collect();
    let mut ys: Vec<f64> = pts.iter().map(|q| q.y).collect();
    if quadrants {
        xs.push(qx);
        ys.push(qy);
    }
    let xd = domain(&xs, false, None, None, 5.0);
    let yd = domain(&ys, false, None, None, 5.0);
    let colors: Vec<String> = (0..groups.len()).map(|i| k.color(i)).collect();
    k.push_card(&mut out.prims, w, h);
    let with_legend = show_legend && !(groups.len() == 1 && groups.first().is_some_and(String::is_empty));
    let mut top = PAD;
    if with_legend {
        legend(&mut out.prims, &k, &groups, &colors, PAD, PAD + 4.0, w - PAD);
        top = PAD + 20.0;
    }
    let y_labels: Vec<String> = yd.ticks.iter().map(|t| tick_text(*t, &yd)).collect();
    let mut lw: f64 = 0.0;
    for l in &y_labels {
        lw = lw.max(est(l, k.tick));
    }
    let x0 = PAD + lw.max(10.0) + 6.0;
    let y_title = name_of(input, 3);
    let x_title = name_of(input, 2);
    if !y_title.is_empty() {
        let s = fit(&y_title, k.tick, w - 2.0 * PAD);
        if !s.is_empty() {
            out.prims.push(text(PAD, top + 6.0, s, k.tick, k.text, "start"));
        }
        top += 16.0;
    } else {
        top += 6.0;
    }
    let strips = marginals != "none";
    let y0 = top + if strips { STRIP } else { 0.0 };
    let x1 = w - PAD - if strips { STRIP } else { 0.0 };
    let y1 = h - PAD - 18.0 - if x_title.is_empty() { 0.0 } else { 14.0 };
    if x1 - x0 < 20.0 || y1 - y0 < 20.0 {
        return out;
    }
    let f = Frame {
        xd,
        yd,
        x0,
        x1,
        y0,
        y1,
    };
    for (t, l) in f.yd.ticks.iter().zip(&y_labels) {
        let yy = f.sy(*t);
        out.prims.push(line(x0, yy, x1, yy, k.grid, 1.0));
        out.prims
            .push(text(x0 - 4.0, yy, l.clone(), k.tick, k.text, "end"));
    }
    let x_labels: Vec<String> = f.xd.ticks.iter().map(|t| tick_text(*t, &f.xd)).collect();
    let mut widest: f64 = 0.0;
    for l in &x_labels {
        widest = widest.max(est(l, k.tick));
    }
    let n_ticks = f.xd.ticks.len();
    let room = if n_ticks > 1 {
        (x1 - x0) / (n_ticks - 1) as f64
    } else {
        x1 - x0
    };
    let every = ((widest + 6.0) / room.max(1.0)).ceil().max(1.0) as usize;
    for (i, (t, l)) in f.xd.ticks.iter().zip(&x_labels).enumerate() {
        let xx = f.sx(*t);
        out.prims.push(line(xx, y0, xx, y1, k.grid, 1.0));
        if i % every == 0 {
            out.prims.push(text(
                inside(xx, l, k.tick, w),
                y1 + 9.0,
                l.clone(),
                k.tick,
                k.text,
                "middle",
            ));
        }
    }
    out.prims.push(line(x0, y1, x1, y1, k.axis, 1.0));
    out.prims.push(line(x0, y0, x0, y1, k.axis, 1.0));
    // the notes: the points left out past the cap, the density view
    let mut notes: Vec<String> = Vec::new();
    if total > MAX_POINTS {
        let (a, b) = (fmt_fixed(MAX_POINTS as f64, 0), fmt_fixed(total as f64, 0));
        notes.push(if fr {
            format!("{a} premiers points sur {b}")
        } else {
            format!("First {a} of {b} points")
        });
    }
    let use_density = density == "on" || (density == "auto" && pts.len() as f64 > threshold);
    if use_density {
        let n = fmt_fixed(pts.len() as f64, 0);
        notes.push(if fr {
            format!("{n} points, vue de densité")
        } else {
            format!("{n} points, density view")
        });
    }
    if !notes.is_empty() {
        let room = if x_title.is_empty() {
            x1 - x0
        } else {
            (x1 - x0) / 2.0
        };
        let s = fit(&notes.join(" · "), k.tick, room);
        if !s.is_empty() {
            out.prims
                .push(text(x0, h - PAD - 5.0, s, k.tick, k.text, "start"));
        }
    }
    if !x_title.is_empty() {
        let room = if notes.is_empty() {
            x1 - PAD
        } else {
            (x1 - x0) / 2.0 - 6.0
        };
        let s = fit(&x_title, k.tick, room);
        if !s.is_empty() {
            out.prims.push(text(x1, h - PAD - 5.0, s, k.tick, k.text, "end"));
        }
    }
    if quadrants {
        quadrant_marks(&mut out.prims, &k, o, &pts, qx, qy, &f);
    }
    // the marks: points (largest behind) or density cells
    let mut radius: Vec<f64> = Vec::new();
    if use_density {
        density_cells(&mut out, &k, &pts, hex_size, &f);
        radius.resize(pts.len(), 0.0);
    } else {
        let (mut smin, mut smax) = (f64::INFINITY, f64::NEG_INFINITY);
        for s in pts.iter().filter_map(|q| q.s) {
            smin = smin.min(s);
            smax = smax.max(s);
        }
        let sized = smin <= smax;
        for q in &pts {
            let r = match (sized, q.s) {
                (false, _) => size,
                (true, None) => size * 0.6,
                (true, Some(s)) if smax > smin => {
                    size * 0.6 + size * 1.8 * ((s - smin) / (smax - smin)).sqrt()
                }
                (true, Some(_)) => size,
            };
            radius.push(r);
        }
        let mut order: Vec<usize> = (0..pts.len()).collect();
        let rad = |i: usize| radius.get(i).copied().unwrap_or(0.0);
        order.sort_by(|a, b| cmp(rad(*b), rad(*a)).then(a.cmp(b)));
        for i in order {
            let Some(q) = pts.get(i) else { continue };
            let c = k.color(q.g);
            let m = Prim::Circle {
                cx: r2(f.sx(q.x)),
                cy: r2(f.sy(q.y)),
                r: r2(rad(i)),
                fill: c.clone(),
                opacity: if fill { fill_opacity } else { 0.0 },
                deco: Deco {
                    stroke: if border {
                        mix(&c, "#000000", 0.3)
                    } else {
                        String::new()
                    },
                    sw: 1.25,
                    ..Deco::default()
                },
            };
            let mut extra = json!({"group": q.g});
            if !q.lit {
                extra["fade"] = json!(true);
            }
            out.hit(q.row as i64, extra);
            out.prims.push(if q.lit { m } else { dim(m) });
        }
    }
    if strips {
        strip_marks(&mut out.prims, &k, marginals, &pts, &f);
    }
    // the point labels: the top N by y, or the selected rows
    let mut chosen: Vec<usize> = Vec::new();
    if label_mode == "top" {
        chosen = (0..pts.len()).collect();
        let y = |i: usize| pts.get(i).map_or(0.0, |q| q.y);
        chosen.sort_by(|a, b| cmp(y(*b), y(*a)).then(a.cmp(b)));
        chosen.truncate(label_count);
    } else if label_mode == "selected"
        && let Some(sel) = input.get("selected").and_then(Value::as_array)
    {
        let want: HashSet<i64> = sel
            .iter()
            .take(1000)
            .filter_map(Value::as_f64)
            .filter(|r| r.is_finite() && r.fract() == 0.0 && r.abs() < 1e15)
            .map(|r| r as i64)
            .collect();
        for (i, q) in pts.iter().enumerate() {
            if chosen.len() >= MAX_LABELS {
                break;
            }
            if want.contains(&(q.row as i64)) {
                chosen.push(i);
            }
        }
    }
    let mut tags: Vec<Tag> = Vec::new();
    for i in chosen {
        let (Some(q), Some(r)) = (pts.get(i), radius.get(i).copied()) else {
            continue;
        };
        let cell = rows.get(q.row).and_then(|x| x.first());
        let mut s = fit(&cell_label(input, 0, cell), font_size, 140.0);
        if s.is_empty() {
            continue;
        }
        let cx = f.sx(q.x);
        let ty = f.sy(q.y).max(y0 + font_size / 2.0).min(y1 - font_size / 2.0);
        let mut gx = cx + r + 3.0;
        let mut anchor = "start";
        if gx + est(&s, font_size) > x1 {
            gx = cx - r - 3.0;
            anchor = "end";
            s = fit(&s, font_size, gx - 1.0);
            if s.is_empty() {
                continue;
            }
        }
        let tw = est(&s, font_size);
        let bx0 = if anchor == "start" { gx } else { gx - tw };
        let half = font_size / 2.0;
        tags.push(Tag {
            x: gx,
            y: ty,
            s,
            anchor,
            boxed: [bx0, ty - half, bx0 + tw, ty + half],
        });
    }
    // the trend's R² text keeps clear of the point labels; a label it cannot avoid is left out
    let boxes: Vec<[f64; 4]> = tags.iter().map(|t| t.boxed).collect();
    let r2box = if trend {
        trend_marks(&mut out.prims, &k, &pts, mx, my, &f, &boxes)
    } else {
        None
    };
    for t in tags {
        if r2box.is_some_and(|b| meets(&t.boxed, &b)) {
            continue;
        }
        out.prims.push(text_w(
            t.x,
            t.y,
            t.s,
            font_size,
            k.label,
            t.anchor,
            0.0,
            k.value_weight(),
        ));
    }
    out
}

/// A point's label: its anchor point, text, anchor and box (`[x0, y0, x1, y1]`).
struct Tag {
    x: f64,
    y: f64,
    s: String,
    anchor: &'static str,
    boxed: [f64; 4],
}

/// Whether two boxes (`[x0, y0, x1, y1]`) come within 1 px of each other.
pub(crate) fn meets(a: &[f64; 4], b: &[f64; 4]) -> bool {
    a[0] < b[2] + 1.0 && b[0] < a[2] + 1.0 && a[1] < b[3] + 1.0 && b[1] < a[3] + 1.0
}

fn quadrant_marks(p: &mut Vec<Prim>, k: &Look, o: &Value, pts: &[Pt], qx: f64, qy: f64, f: &Frame) {
    if qx >= f.xd.lo && qx <= f.xd.hi {
        dashes(p, &[(f.sx(qx), f.y0), (f.sx(qx), f.y1)], k.axis, 1.0, 4.0, 3.0);
    }
    if qy >= f.yd.lo && qy <= f.yd.hi {
        dashes(p, &[(f.x0, f.sy(qy)), (f.x1, f.sy(qy))], k.axis, 1.0, 4.0, 3.0);
    }
    if f.y1 - f.y0 < 40.0 {
        return;
    }
    // top right, top left, bottom left, bottom right
    let mut n = [0usize; 4];
    for q in pts {
        let (right, up) = (q.x >= qx, q.y >= qy);
        let i = match (up, right) {
            (true, true) => 0,
            (true, false) => 1,
            (false, false) => 2,
            (false, true) => 3,
        };
        if let Some(c) = n.get_mut(i) {
            *c += 1;
        }
    }
    let keys = [
        "labelTopRight",
        "labelTopLeft",
        "labelBottomLeft",
        "labelBottomRight",
    ];
    let at = [
        (f.x1 - 6.0, f.y0 + 8.0, "end"),
        (f.x0 + 6.0, f.y0 + 8.0, "start"),
        (f.x0 + 6.0, f.y1 - 8.0, "start"),
        (f.x1 - 6.0, f.y1 - 8.0, "end"),
    ];
    for ((key, (x, y, anchor)), c) in keys.iter().zip(at).zip(n) {
        let label = opt_text(o, key, 60);
        let share = format!("{}%", fmt_fixed((c as f64 / pts.len() as f64) * 100.0, 0));
        let full = if label.is_empty() {
            share
        } else {
            format!("{label} · {share}")
        };
        let s = fit(&full, k.tick, (f.x1 - f.x0) / 2.0 - 8.0);
        if !s.is_empty() {
            p.push(text_w(x, y, s, k.tick, k.text, anchor, 0.0, 600));
        }
    }
}

struct Cell {
    i: f64,
    j: f64,
    rows: Vec<usize>,
    n: usize,
    lit: bool,
}

fn density_cells(out: &mut Out, k: &Look, pts: &[Pt], r: f64, f: &Frame) {
    let dx = r * 3f64.sqrt();
    let dy = r * 1.5;
    let mut ix: HashMap<String, usize> = HashMap::new();
    let mut cells: Vec<Cell> = Vec::new();
    for q in pts {
        let px = f.sx(q.x) - f.x0;
        let py = f.sy(q.y) - f.y0;
        // the nearest centre among the two nearest rows of hexagons
        let fj = py / dy;
        let j0 = (fj + 0.5).floor();
        let j1 = if fj >= j0 { j0 + 1.0 } else { j0 - 1.0 };
        let (mut bi, mut bj, mut best) = (0.0, j0, f64::INFINITY);
        for j in [j0, j1] {
            let i = (px / dx - odd(j) / 2.0 + 0.5).floor();
            let cx = (i + odd(j) / 2.0) * dx;
            let cy = j * dy;
            let d = (px - cx) * (px - cx) + (py - cy) * (py - cy);
            if d < best {
                best = d;
                bi = i;
                bj = j;
            }
        }
        let key = format!("{},{}", bi as i64, bj as i64);
        let c = match ix.get(&key) {
            Some(&c) => c,
            None => {
                ix.insert(key, cells.len());
                cells.push(Cell {
                    i: bi,
                    j: bj,
                    rows: Vec::new(),
                    n: 0,
                    lit: false,
                });
                cells.len() - 1
            }
        };
        let Some(cell) = cells.get_mut(c) else { continue };
        cell.n += 1;
        if cell.rows.len() < MAX_CELL_ROWS {
            cell.rows.push(q.row);
        }
        cell.lit = cell.lit || q.lit;
    }
    let most = cells.iter().map(|c| c.n).max().unwrap_or(1).max(1);
    let base = k.color(0);
    let light = mix(&base, &k.surface, 0.82);
    let dark = mix(&base, "#000000", 0.15);
    let rr = r * 0.95;
    let s = 3f64.sqrt() / 2.0;
    for (b, c) in cells.iter().enumerate() {
        let cx = f.x0 + (c.i + odd(c.j) / 2.0) * dx;
        let cy = f.y0 + c.j * dy;
        let shape = [
            (cx, cy - rr),
            (cx + rr * s, cy - rr / 2.0),
            (cx + rr * s, cy + rr / 2.0),
            (cx, cy + rr),
            (cx - rr * s, cy + rr / 2.0),
            (cx - rr * s, cy - rr / 2.0),
        ];
        let m = poly(
            &shape,
            &mix(&light, &dark, (c.n as f64 / most as f64).sqrt()),
            1.0,
        );
        let first = c.rows.first().map_or(-1, |r| *r as i64);
        let mut extra = json!({"cell": b, "n": c.n, "rows": c.rows});
        if !c.lit {
            extra["fade"] = json!(true);
        }
        out.hit(first, extra);
        out.prims.push(if c.lit { m } else { dim(m) });
    }
}

fn strip_marks(p: &mut Vec<Prim>, k: &Look, mode: &str, pts: &[Pt], f: &Frame) {
    let tone = mix(&k.color(0), &k.surface, 0.35);
    let (x0, x1, y0, y1) = (f.x0, f.x1, f.y0, f.y1);
    if mode == "histogram" {
        let mut cx = [0usize; BINS];
        let mut cy = [0usize; BINS];
        for q in pts {
            if let Some(c) = cx.get_mut(bin_of(q.x, &f.xd)) {
                *c += 1;
            }
            if let Some(c) = cy.get_mut(bin_of(q.y, &f.yd)) {
                *c += 1;
            }
        }
        let mut most = 1;
        for b in 0..BINS {
            most = most.max(cx[b]).max(cy[b]);
        }
        let tall = STRIP - 8.0;
        let bw = (x1 - x0) / BINS as f64;
        let bh = (y1 - y0) / BINS as f64;
        for b in 0..BINS {
            let bf = b as f64;
            if cx[b] > 0 {
                let hh = (tall * cx[b] as f64) / most as f64;
                p.push(k.bar(
                    x0 + bf * bw + 0.5,
                    y0 - 4.0 - hh,
                    (bw - 1.0).max(0.5),
                    hh,
                    &tone,
                    "b",
                ));
            }
            if cy[b] > 0 {
                let ww = (tall * cy[b] as f64) / most as f64;
                p.push(k.bar(
                    x1 + 4.0,
                    y1 - (bf + 1.0) * bh + 0.5,
                    ww,
                    (bh - 1.0).max(0.5),
                    &tone,
                    "l",
                ));
            }
        }
        return;
    }
    let xs: Vec<f64> = pts.iter().map(|q| q.x).collect();
    let ys: Vec<f64> = pts.iter().map(|q| q.y).collect();
    let fx = five_of(&xs);
    let fy = five_of(&ys);
    let ink = k.color(0);
    // along x, over the plot
    let cy = y0 - STRIP / 2.0;
    p.push(line(f.sx(fx[0]), cy, f.sx(fx[1]), cy, k.axis, 1.0));
    p.push(line(f.sx(fx[3]), cy, f.sx(fx[4]), cy, k.axis, 1.0));
    p.push(line(f.sx(fx[0]), cy - 4.0, f.sx(fx[0]), cy + 4.0, k.axis, 1.0));
    p.push(line(f.sx(fx[4]), cy - 4.0, f.sx(fx[4]), cy + 4.0, k.axis, 1.0));
    p.push(k.boxed(f.sx(fx[1]), cy - 7.0, f.sx(fx[3]) - f.sx(fx[1]), 14.0, &ink));
    p.push(line(f.sx(fx[2]), cy - 7.0, f.sx(fx[2]), cy + 7.0, &ink, 2.0));
    // along y, beside the plot
    let cx = x1 + STRIP / 2.0;
    p.push(line(cx, f.sy(fy[0]), cx, f.sy(fy[1]), k.axis, 1.0));
    p.push(line(cx, f.sy(fy[3]), cx, f.sy(fy[4]), k.axis, 1.0));
    p.push(line(cx - 4.0, f.sy(fy[0]), cx + 4.0, f.sy(fy[0]), k.axis, 1.0));
    p.push(line(cx - 4.0, f.sy(fy[4]), cx + 4.0, f.sy(fy[4]), k.axis, 1.0));
    p.push(k.boxed(cx - 7.0, f.sy(fy[3]), 14.0, f.sy(fy[1]) - f.sy(fy[3]), &ink));
    p.push(line(cx - 7.0, f.sy(fy[2]), cx + 7.0, f.sy(fy[2]), &ink, 2.0));
}

/// The least-squares line and its R² text, placed clear of the boxes `avoid` where it can;
/// returns the text's box (`None`: no line).
fn trend_marks(
    p: &mut Vec<Prim>,
    k: &Look,
    pts: &[Pt],
    mx: f64,
    my: f64,
    f: &Frame,
    avoid: &[[f64; 4]],
) -> Option<[f64; 4]> {
    let (mut sxx, mut sxy, mut syy) = (0.0, 0.0, 0.0);
    for q in pts {
        sxx += (q.x - mx) * (q.x - mx);
        sxy += (q.x - mx) * (q.y - my);
        syy += (q.y - my) * (q.y - my);
    }
    if pts.len() < 2 || sxx.partial_cmp(&0.0) != Some(Ordering::Greater) {
        return None;
    }
    let b = sxy / sxx;
    let a = my - b * mx;
    let (xa, xb) = (f.xd.lo, f.xd.hi);
    let ya = a + b * xa;
    let dyv = a + b * xb - ya;
    let (mut t0, mut t1): (f64, f64) = (0.0, 1.0);
    if dyv != 0.0 {
        let tl = (f.yd.lo - ya) / dyv;
        let th = (f.yd.hi - ya) / dyv;
        t0 = t0.max(tl.min(th));
        t1 = t1.min(tl.max(th));
    }
    if t0.partial_cmp(&t1) != Some(Ordering::Less) {
        return None;
    }
    let ax = f.sx(xa + (xb - xa) * t0);
    let ay = f.sy(ya + dyv * t0);
    let bx = f.sx(xa + (xb - xa) * t1);
    let by = f.sy(ya + dyv * t1);
    p.push(line(ax, ay, bx, by, k.label, 1.5));
    let r2v = if syy > 0.0 { (sxy * sxy) / (sxx * syy) } else { 1.0 };
    let s = format!("R² = {}", fmt_fixed(r2v, 2));
    let tw = est(&s, k.tick);
    // above the line's end, below it, a line further up and down, above and below its start,
    // then the plot's top left and bottom right corners: the first place clear of every point
    // label (`avoid`), else the first
    let far = 8.0 + k.tick + 3.0;
    let places = [
        (bx - 4.0, by - 8.0),
        (bx - 4.0, by + 8.0),
        (bx - 4.0, by - far),
        (bx - 4.0, by + far),
        (ax + 4.0 + tw, ay - 8.0),
        (ax + 4.0 + tw, ay + 8.0),
        (f.x0 + 4.0 + tw, f.y0 + 6.0),
        (f.x1 - 4.0, f.y1 - 6.0),
    ];
    let place = |(px, py): (f64, f64)| {
        let tx = px.max(f.x0 + tw + 2.0);
        let ty = py.max(f.y0 + 6.0).min(f.y1 - 6.0);
        (tx, ty, [tx - tw, ty - k.tick / 2.0, tx, ty + k.tick / 2.0])
    };
    let (tx, ty, boxed) = places
        .iter()
        .map(|q| place(*q))
        .find(|(_, _, b)| !avoid.iter().any(|a| meets(a, b)))
        .unwrap_or_else(|| place((bx - 4.0, by - 8.0)));
    p.push(text_w(tx, ty, s, k.tick, k.label, "end", 0.0, 600));
    Some(boxed)
}
