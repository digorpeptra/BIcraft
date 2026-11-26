//! BIcraft Gauge / KPI+ (docs/formats/chart-pack.md "Gauge / KPI+"): one value against its
//! range (minimum, up to three thresholds, maximum, each from a role or the format pane), the
//! range drawn as free colour bands, a target marker and the variance from the target or the
//! previous value, in three styles: a radial arc, a linear bullet, or a KPI card. With a trend
//! field the value is the latest point's and a sparkline of every point runs along the foot.
//! Values are written with their field's format string. Drawn with BIcraft's look
//! (docs/formats/visual-look.md). The Rust twin of `crates/engine/src/chart_pack/gauge.ts`.

use std::f64::consts::PI;

use serde_json::{Value, json};

use super::waterfall::{fmt_num, format_at, pct_text, signed};
use super::{
    Out, PAD, cell_label, cell_num, est, fit, highlighted, hl_num, is_fr, line, look_of, opt_bool, opt_color,
    options_of, poly, rows_of, text,
};
use crate::data::{Ctx, Dataset};
use crate::look::{Look, dim, mix, part, text_w};
use crate::own_hits::{Target, num_text, role_col, tip_line, word};
use crate::spec::{Prim, domain, inside};

/// Most rows (trend points) read.
pub const MAX_ROWS: usize = 1000;
/// Most sparkline points drawn as dots (each a data point); beyond, only the latest.
pub const MAX_DOTS: usize = 60;
/// Values beyond this size are not read.
pub const MAX_ABS: f64 = 1e15;
/// The band colours' bases, worst to best (drawn softened unless a band colour is set).
pub const BAND_BASES: [&str; 4] = ["#D64545", "#E0A100", "#2E9E5B", "#1C7A45"];
/// The bases each band count takes, worst to best.
const BAND_PICKS: [&[usize]; 4] = [&[0], &[0, 2], &[0, 1, 2], &[0, 1, 2, 3]];
const GOOD: [&str; 2] = ["#137A3F", "#5BD08A"];
const BAD: [&str; 2] = ["#C0392B", "#FF7B6E"];

struct Pt {
    row: usize,
    v: f64,
    label: String,
}

fn num_of(v: Option<&Value>) -> Option<f64> {
    cell_num(v).filter(|n| n.abs() <= MAX_ABS)
}

/// sin and cos of `t` in [0, π] by their series (the same arithmetic as the TypeScript, so both
/// drawers agree to the last bit).
pub fn sin_cos(t: f64) -> (f64, f64) {
    let mut x = t;
    let mut flip = false;
    if x > PI / 2.0 {
        x = PI - x;
        flip = true;
    }
    let x2 = x * x;
    let s = x
        * (1.0
            - (x2 / 6.0)
                * (1.0
                    - (x2 / 20.0)
                        * (1.0
                            - (x2 / 42.0)
                                * (1.0
                                    - (x2 / 72.0)
                                        * (1.0
                                            - (x2 / 110.0) * (1.0 - (x2 / 156.0) * (1.0 - x2 / 210.0)))))));
    let c = 1.0
        - (x2 / 2.0)
            * (1.0
                - (x2 / 12.0)
                    * (1.0
                        - (x2 / 30.0)
                            * (1.0
                                - (x2 / 56.0)
                                    * (1.0 - (x2 / 90.0) * (1.0 - (x2 / 132.0) * (1.0 - x2 / 182.0))))));
    (s, if flip { -c } else { c })
}

/// A ring sector of the half-circle gauge from fraction `f0` to `f1` (0 at the left end, 1 at
/// the right), between radii `r0` and `r1`.
#[allow(clippy::too_many_arguments)]
fn arc(cx: f64, cy: f64, r0: f64, r1: f64, f0: f64, f1: f64, fill: &str) -> Prim {
    let segs = (48.0 * (f1 - f0)).ceil().clamp(2.0, 48.0) as usize;
    let at = |r: f64, i: usize| {
        let (s, c) = sin_cos(PI * (1.0 - (f0 + ((f1 - f0) * i as f64) / segs as f64)));
        (cx + r * c, cy - r * s)
    };
    let mut pts: Vec<(f64, f64)> = (0..=segs).map(|i| at(r1, i)).collect();
    pts.extend((0..=segs).rev().map(|i| at(r0, i)));
    poly(&pts, fill, 1.0)
}

/// What every style draws from.
struct G {
    pts: Vec<Pt>,
    row: usize,
    val: f64,
    target: Option<f64>,
    min: f64,
    max: f64,
    edges: Vec<f64>,
    bands: Vec<String>,
    value_fill: String,
    target_fill: String,
    val_text: String,
    min_text: String,
    max_text: String,
    /// The variance line (empty without a target or previous value) and its ink.
    var_text: String,
    var_ink: String,
    ref_text: String,
    reference: Option<f64>,
    ref_kind: &'static str,
    /// The highlighted part of the value (`None`: none).
    hv: Option<f64>,
    lit: bool,
}

impl G {
    fn frac(&self, v: f64) -> f64 {
        ((v - self.min) / (self.max - self.min)).clamp(0.0, 1.0)
    }

    fn value_hit(&self) -> Value {
        json!({"part": "value", "value": self.val, "target": self.target, "ref": self.reference,
               "refKind": self.ref_kind, "min": self.min, "max": self.max})
    }
}

fn prepare(input: &Value, k: &Look) -> Option<G> {
    let o = options_of(input);
    let fr = is_fr(input);
    let rows = rows_of(input, MAX_ROWS);
    let pts: Vec<Pt> = rows
        .iter()
        .enumerate()
        .filter_map(|(i, r)| {
            num_of(r.get(1)).map(|v| Pt {
                row: i,
                v,
                label: cell_label(input, 0, r.first()),
            })
        })
        .collect();
    let cur = pts.last()?;
    let r = rows.get(cur.row)?;
    let val = cur.v;
    let at = |c: usize, key: &str| num_of(r.get(c)).or_else(|| num_of(o.get(key)));
    let target = at(2, "target");
    let min = at(3, "min").unwrap_or(0.0);
    let given: Vec<f64> = (0..3)
        .filter_map(|j| at(5 + j, &format!("threshold{}", j + 1)))
        .collect();
    let max = match at(4, "max") {
        Some(m) if m > min => m,
        _ => {
            let mut vals = vec![min, val];
            vals.extend(&given);
            vals.extend(target);
            let hi = domain(&vals, false, Some(min), None, 4.0).hi;
            if hi > min { hi } else { min + 1.0 }
        }
    };
    let mut ts: Vec<f64> = given.into_iter().filter(|t| *t > min && *t < max).collect();
    ts.sort_by(f64::total_cmp);
    let mut edges = vec![min];
    for t in ts {
        if edges.last() != Some(&t) {
            edges.push(t);
        }
    }
    edges.push(max);
    let nb = edges.len() - 1;
    let lower = opt_bool(o, "lowerIsBetter", false);
    let picks = BAND_PICKS.get(nb.clamp(1, 4) - 1).copied().unwrap_or(&[0]);
    let bases: Vec<&str> = (0..nb)
        .map(|j| {
            let i = if lower { nb - 1 - j } else { j };
            BAND_BASES
                .get(picks.get(i).copied().unwrap_or(0))
                .copied()
                .unwrap_or(BAND_BASES[0])
        })
        .collect();
    let bands: Vec<String> = bases
        .iter()
        .enumerate()
        .map(|(j, b)| {
            let def = if nb == 1 {
                k.empty.to_string()
            } else {
                mix(b, &k.surface, 0.62)
            };
            opt_color(o, &format!("band{}", j + 1), &def)
        })
        .collect();
    let mut value_fill = opt_color(o, "valueFill", &k.color(0));
    if opt_bool(o, "valueByBand", false) && nb > 1 {
        let mut j = 0;
        while j < nb - 1 && edges.get(j + 1).is_some_and(|e| val >= *e) {
            j += 1;
        }
        value_fill = if o.get(format!("band{}", j + 1)).is_some_and(Value::is_string) {
            bands.get(j).cloned().unwrap_or_default()
        } else {
            bases.get(j).map(|b| (*b).to_string()).unwrap_or_default()
        };
    }
    let prev = num_of(r.get(8)).or_else(|| pts.len().checked_sub(2).and_then(|i| pts.get(i)).map(|p| p.v));
    let want_prev = o.get("compareTo").and_then(Value::as_str) == Some("previous");
    let (reference, ref_kind) = match (want_prev, prev, target) {
        (true, Some(p), _) => (Some(p), "previous"),
        (_, _, Some(t)) => (Some(t), "target"),
        (_, Some(p), None) => (Some(p), "previous"),
        _ => (None, ""),
    };
    let f = format_at(input, 1);
    let (mut var_text, mut var_ink, mut ref_text) = (String::new(), k.label.to_string(), String::new());
    if let Some(rf) = reference {
        let diff = val - rf;
        var_text = signed(diff, fmt_num(diff, &f, fr));
        if rf != 0.0 {
            var_text.push_str(&format!(" ({})", pct_text(diff / rf.abs(), fr)));
        }
        var_text.push_str(match (ref_kind == "target", fr) {
            (true, true) => " vs cible",
            (true, false) => " vs target",
            (false, true) => " vs précédent",
            (false, false) => " vs previous",
        });
        let good = if lower { diff <= 0.0 } else { diff >= 0.0 };
        let i = usize::from(k.night);
        var_ink = (if good { GOOD } else { BAD })[i].to_string();
        ref_text = format!(
            "{}{}",
            match (ref_kind == "target", fr) {
                (true, true) => "Cible : ",
                (true, false) => "Target: ",
                (false, true) => "Précédent : ",
                (false, false) => "Previous: ",
            },
            fmt_num(rf, &f, fr)
        );
    }
    let lit = highlighted(input);
    let row = cur.row;
    Some(G {
        row,
        val,
        target,
        min,
        max,
        edges,
        bands,
        value_fill,
        target_fill: opt_color(o, "targetFill", k.label),
        val_text: fmt_num(val, &f, fr),
        min_text: fmt_num(min, &f, fr),
        max_text: fmt_num(max, &f, fr),
        var_text,
        var_ink,
        ref_text,
        reference,
        ref_kind,
        hv: if lit { part(val, hl_num(input, row)) } else { None },
        lit,
        pts,
    })
}

/// The trend's sparkline in the box (`x0`, `y0`)–(`x1`, `y1`): a line through every point, each
/// a dot (only the latest past [`MAX_DOTS`] points).
#[allow(clippy::too_many_arguments)]
fn spark(out: &mut Out, k: &Look, g: &G, input: &Value, x0: f64, y0: f64, x1: f64, y1: f64) {
    let n = g.pts.len();
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for q in &g.pts {
        lo = lo.min(q.v);
        hi = hi.max(q.v);
    }
    let xs = |i: usize| {
        if n == 1 {
            (x0 + x1) / 2.0
        } else {
            x0 + ((x1 - x0) * i as f64) / (n - 1) as f64
        }
    };
    let ys = |v: f64| {
        if hi > lo {
            y1 - ((v - lo) / (hi - lo)) * (y1 - y0)
        } else {
            (y0 + y1) / 2.0
        }
    };
    let lw = k.line_width();
    for (i, w) in g.pts.windows(2).enumerate() {
        if let [a, b] = w {
            out.prims
                .push(line(xs(i), ys(a.v), xs(i + 1), ys(b.v), &g.value_fill, lw));
        }
    }
    let from = if n <= MAX_DOTS { 0 } else { n - 1 };
    for (i, q) in g.pts.iter().enumerate().skip(from) {
        out.hit(
            q.row as i64,
            json!({"part": "point", "value": q.v, "label": q.label}),
        );
        let r = if i == n - 1 { 3.5 } else { 2.5 };
        let d = k.dot(xs(i), ys(q.v), r, &g.value_fill, 1.0);
        let faded = g.lit && part(q.v, hl_num(input, q.row)).is_none();
        out.prims.push(if faded { dim(d) } else { d });
    }
}

fn arc_style(out: &mut Out, k: &Look, g: &G, input: &Value, trend: bool, w: f64, h: f64) {
    let trend_h = if trend && h >= 120.0 {
        (h * 0.22).min(40.0)
    } else {
        0.0
    };
    let ah = h - 2.0 * PAD - if trend_h > 0.0 { trend_h + 6.0 } else { 0.0 } - (k.tick + 6.0);
    let big_r = ((w - 2.0 * PAD) / 2.0 - 4.0).min(ah - 4.0);
    if big_r.is_nan() || big_r < 16.0 {
        let vs = (h * 0.3).clamp(10.0, 28.0);
        let s = fit(&g.val_text, vs, w - 2.0 * PAD);
        if !s.is_empty() {
            out.hit(g.row as i64, g.value_hit());
            out.prims
                .push(text_w(w / 2.0, h / 2.0, s, vs, k.label, "middle", 0.0, 700));
        }
        return;
    }
    let cx = w / 2.0;
    let cy = PAD + 4.0 + big_r;
    let thick = big_r * 0.26;
    for (j, fill) in g.bands.iter().enumerate() {
        let (Some(a), Some(b)) = (g.edges.get(j), g.edges.get(j + 1)) else {
            continue;
        };
        out.prims
            .push(arc(cx, cy, big_r - thick, big_r, g.frac(*a), g.frac(*b), fill));
    }
    let fv = g.frac(g.val);
    let r0 = big_r - thick * 0.8;
    let r1 = big_r - thick * 0.2;
    if fv > 0.0 {
        out.hit(g.row as i64, g.value_hit());
        let a = arc(cx, cy, r0, r1, 0.0, fv, &g.value_fill);
        out.prims.push(if g.lit { dim(a) } else { a });
        if let Some(hv) = g.hv {
            let fh = g.frac(hv);
            if fh > 0.0 {
                out.hit(g.row as i64, g.value_hit());
                out.prims.push(arc(cx, cy, r0, r1, 0.0, fh, &g.value_fill));
            }
        }
    }
    if let Some(t) = g.target {
        let (s, c) = sin_cos(PI * (1.0 - g.frac(t)));
        let a = big_r - thick - 3.0;
        let b = big_r + 3.0;
        out.hit(g.row as i64, json!({"part": "target", "value": t}));
        out.prims.push(line(
            cx + a * c,
            cy - a * s,
            cx + b * c,
            cy - b * s,
            &g.target_fill,
            if k.minimal() { 2.0 } else { 3.0 },
        ));
    }
    let ly = cy + 4.0 + k.tick / 2.0;
    out.prims.push(text(
        inside(cx - big_r + thick / 2.0, &g.min_text, k.tick, w),
        ly,
        g.min_text.clone(),
        k.tick,
        k.text,
        "middle",
    ));
    out.prims.push(text(
        inside(cx + big_r - thick / 2.0, &g.max_text, k.tick, w),
        ly,
        g.max_text.clone(),
        k.tick,
        k.text,
        "middle",
    ));
    let inner = big_r - thick;
    let vs = (big_r * 0.3).clamp(12.0, 40.0);
    let vy = cy - inner * 0.35;
    let s = fit(&g.val_text, vs, 2.0 * inner - 8.0);
    if !s.is_empty() {
        out.hit(g.row as i64, g.value_hit());
        out.prims.push(text_w(cx, vy, s, vs, k.label, "middle", 0.0, 700));
    }
    let v = fit(&g.var_text, k.name, 2.0 * inner - 8.0);
    if !v.is_empty() {
        out.prims.push(text_w(
            cx,
            vy + vs / 2.0 + 3.0 + k.name / 2.0,
            v,
            k.name,
            &g.var_ink,
            "middle",
            0.0,
            600,
        ));
    }
    if trend_h > 0.0 {
        spark(
            out,
            k,
            g,
            input,
            PAD + 4.0,
            h - PAD - trend_h,
            w - PAD - 4.0,
            h - PAD - 4.0,
        );
    }
}

fn bullet_style(out: &mut Out, k: &Look, g: &G, input: &Value, trend: bool, w: f64, h: f64) {
    let vs = (h * 0.2).clamp(12.0, 30.0);
    let hy = PAD + vs / 2.0 + 2.0;
    let s = fit(&g.val_text, vs, (w - 2.0 * PAD) * 0.6);
    let room = w - 2.0 * PAD - est(&s, vs) - 12.0;
    if !s.is_empty() {
        out.hit(g.row as i64, g.value_hit());
        out.prims
            .push(text_w(PAD + 2.0, hy, s, vs, k.label, "start", 0.0, 700));
    }
    let v = fit(&g.var_text, k.name, room);
    if !v.is_empty() {
        out.prims
            .push(text_w(w - PAD - 2.0, hy, v, k.name, &g.var_ink, "end", 0.0, 600));
    }
    let trend_h = if trend && h >= 120.0 {
        (h * 0.22).min(36.0)
    } else {
        0.0
    };
    let by0 = PAD + vs + 10.0;
    let avail = h - by0 - PAD - (k.tick + 8.0) - if trend_h > 0.0 { trend_h + 6.0 } else { 0.0 };
    let bh = avail.clamp(0.0, 40.0);
    let bx0 = PAD + 2.0;
    let bx1 = w - PAD - 2.0;
    let bw = bx1 - bx0;
    if bh >= 6.0 && bw >= 20.0 {
        let x_of = |x: f64| bx0 + g.frac(x) * bw;
        let nb = g.bands.len();
        for (j, fill) in g.bands.iter().enumerate() {
            let (Some(e0), Some(e1)) = (g.edges.get(j), g.edges.get(j + 1)) else {
                continue;
            };
            let a = x_of(*e0);
            let sq = if nb == 1 {
                ""
            } else if j == 0 {
                "r"
            } else if j == nb - 1 {
                "l"
            } else {
                "x"
            };
            out.prims.push(k.band(a, by0, x_of(*e1) - a, bh, fill, sq));
        }
        let vh = bh * 0.42;
        let vy = by0 + (bh - vh) / 2.0;
        let fv = g.frac(g.val);
        if fv > 0.0 {
            out.hit(g.row as i64, g.value_hit());
            let m = k.bar(bx0, vy, fv * bw, vh, &g.value_fill, "l");
            out.prims.push(if g.lit { dim(m) } else { m });
            if let Some(hv) = g.hv
                && g.frac(hv) > 0.0
            {
                out.hit(g.row as i64, g.value_hit());
                out.prims
                    .push(k.bar(bx0, vy, g.frac(hv) * bw, vh, &g.value_fill, "l"));
            }
        }
        if let Some(t) = g.target {
            let tx = x_of(t);
            out.hit(g.row as i64, json!({"part": "target", "value": t}));
            out.prims.push(line(
                tx,
                by0 - 3.0,
                tx,
                by0 + bh + 3.0,
                &g.target_fill,
                if k.minimal() { 2.0 } else { 3.0 },
            ));
        }
        let ly = by0 + bh + 4.0 + k.tick / 2.0;
        out.prims.push(text(
            bx0,
            ly,
            fit(&g.min_text, k.tick, bw / 2.0 - 4.0),
            k.tick,
            k.text,
            "start",
        ));
        out.prims.push(text(
            bx1,
            ly,
            fit(&g.max_text, k.tick, bw / 2.0 - 4.0),
            k.tick,
            k.text,
            "end",
        ));
    }
    if trend_h > 0.0 {
        spark(
            out,
            k,
            g,
            input,
            PAD + 4.0,
            h - PAD - trend_h,
            w - PAD - 4.0,
            h - PAD - 4.0,
        );
    }
}

fn kpi_style(out: &mut Out, k: &Look, g: &G, input: &Value, trend: bool, w: f64, h: f64) {
    let trend_h = if trend && h >= 80.0 {
        (h * 0.3).min(60.0)
    } else {
        0.0
    };
    let area = h - 2.0 * PAD - if trend_h > 0.0 { trend_h + 6.0 } else { 0.0 };
    let vs = (area * 0.42).min(w * 0.16).clamp(12.0, 48.0);
    let block = vs
        + if g.var_text.is_empty() { 0.0 } else { k.name + 6.0 }
        + if g.ref_text.is_empty() { 0.0 } else { k.tick + 4.0 };
    let mut y = PAD + ((area - block) / 2.0).max(0.0);
    let cx = w / 2.0;
    let s = fit(&g.val_text, vs, w - 2.0 * PAD);
    if !s.is_empty() {
        out.hit(g.row as i64, g.value_hit());
        let ink = if g.lit && g.hv.is_none() {
            mix(k.label, &k.surface, 0.6)
        } else {
            k.label.to_string()
        };
        out.prims
            .push(text_w(cx, y + vs / 2.0, s, vs, &ink, "middle", 0.0, 700));
    }
    y += vs;
    let v = fit(&g.var_text, k.name, w - 2.0 * PAD);
    if !v.is_empty() {
        out.prims.push(text_w(
            cx,
            y + 6.0 + k.name / 2.0,
            v,
            k.name,
            &g.var_ink,
            "middle",
            0.0,
            600,
        ));
        y += k.name + 6.0;
    }
    let r = fit(&g.ref_text, k.tick, w - 2.0 * PAD);
    if !r.is_empty() {
        out.prims
            .push(text(cx, y + 4.0 + k.tick / 2.0, r, k.tick, k.text, "middle"));
    }
    if trend_h > 0.0 {
        spark(
            out,
            k,
            g,
            input,
            PAD + 6.0,
            h - PAD - trend_h,
            w - PAD - 6.0,
            h - PAD - 4.0,
        );
    }
}

/// Lays out `{rows: [[trend, value, target, min, max, threshold1, threshold2, threshold3,
/// previous]], formats, options}` in a `w`×`h` visual.
pub fn layout(input: &Value, w: f64, h: f64) -> Out {
    let o = options_of(input);
    let k = look_of(input);
    let mut out = Out::default();
    let Some(g) = prepare(input, &k) else {
        return out;
    };
    if !(w >= 24.0 && h >= 24.0) {
        return out;
    }
    k.push_card(&mut out.prims, w, h);
    let trend = g.pts.len() >= 2 && opt_bool(o, "showTrend", true);
    match o.get("gaugeStyle").and_then(Value::as_str) {
        Some("bullet") => bullet_style(&mut out, &k, &g, input, trend, w, h),
        Some("kpi") => kpi_style(&mut out, &k, &g, input, trend, w, h),
        _ => arc_style(&mut out, &k, &g, input, trend, w, h),
    }
    out
}

/// What each hit selects and shows, as `gauge_visual.ts` `point` does: the value, its target
/// and a sparkline point their row (the latest for the value).
pub(crate) fn targets(out: &Out, ds: &Dataset, ctx: &Ctx<'_>) -> Vec<Target> {
    let fr = ctx.ui_lang.to_ascii_lowercase().starts_with("fr");
    let vc = role_col(ds, "value");
    let value = |x: f64| num_text(ds, vc, x, ctx);
    let value_name = vc
        .and_then(|c| ds.cols.get(c))
        .map(|c| c.name.clone())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| word(ctx, "Value", "Valeur"));
    let num = |h: &Value, k: &str| h.get(k).and_then(Value::as_f64);
    let mut list = Vec::new();
    for h in &out.hits {
        let Some(prim) = h
            .get("prim")
            .and_then(Value::as_u64)
            .and_then(|p| usize::try_from(p).ok())
        else {
            continue;
        };
        let row = h
            .get("row")
            .and_then(Value::as_i64)
            .and_then(|r| usize::try_from(r).ok());
        let mut tip: Vec<(String, String)> = Vec::new();
        match h.get("part").and_then(Value::as_str) {
            Some("value") => {
                if let Some(r) = row {
                    tip.extend(tip_line(ds, role_col(ds, "trend"), r, ctx));
                }
                let v = num(h, "value").unwrap_or(f64::NAN);
                tip.push((value_name.clone(), value(v)));
                if let Some(t) = num(h, "target") {
                    tip.push((word(ctx, "Target", "Cible"), value(t)));
                }
                if let Some(rf) = num(h, "ref") {
                    let diff = v - rf;
                    let mut s = signed(diff, value(diff));
                    if rf != 0.0 {
                        s.push_str(&format!(" ({})", pct_text(diff / rf.abs(), fr)));
                    }
                    let name = if h.get("refKind").and_then(Value::as_str) == Some("previous") {
                        word(ctx, "Variance vs previous", "Écart au précédent")
                    } else {
                        word(ctx, "Variance vs target", "Écart à la cible")
                    };
                    tip.push((name, s));
                }
            }
            Some("target") => {
                tip.push((
                    word(ctx, "Target", "Cible"),
                    value(num(h, "value").unwrap_or(f64::NAN)),
                ));
            }
            Some("point") => {
                if let Some(r) = row {
                    tip.extend(tip_line(ds, role_col(ds, "trend"), r, ctx));
                }
                tip.push((value_name.clone(), value(num(h, "value").unwrap_or(f64::NAN))));
            }
            _ => continue,
        }
        list.push(Target {
            rows: row.map(|r| vec![r]).unwrap_or_default(),
            prims: vec![prim],
            at: Vec::new(),
            tip,
            faded: false,
            send: None,
        });
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The series agree with the platform's sine and cosine over the half turn.
    #[test]
    fn the_series_are_sine_and_cosine() {
        for i in 0..=1000 {
            let t = PI * f64::from(i) / 1000.0;
            let (s, c) = sin_cos(t);
            assert!(
                (s - t.sin()).abs() < 1e-9 && (c - t.cos()).abs() < 1e-9,
                "{t}: {s} {c}"
            );
        }
    }

    fn edges(input: Value) -> Vec<f64> {
        let k = look_of(&input);
        prepare(&input, &k).map(|g| g.edges).unwrap_or_default()
    }

    /// The range: roles before the format pane, a maximum above every value when none is given,
    /// thresholds outside the range or repeated left out.
    #[test]
    fn the_range_and_its_bands() {
        let row = |v: Value| json!({"rows": [v]});
        assert_eq!(edges(row(json!([null, 72, null, null, null]))), [0.0, 80.0]);
        assert_eq!(
            edges(row(json!([null, 72, 90, 0, 100, 70, 40, 40]))),
            [0.0, 40.0, 70.0, 100.0]
        );
        let input =
            json!({"rows": [[null, 5, null, null, null, 150, -3]], "options": {"max": 10, "threshold3": 6}});
        assert_eq!(edges(input), [0.0, 6.0, 10.0]);
        // a maximum at or below the minimum is not one: the range is found again
        let e = edges(json!({"rows": [[null, 5, null, 10, 2]]}));
        assert!(
            e.first() == Some(&10.0) && e.last().is_some_and(|m| *m > 10.0),
            "{e:?}"
        );
        assert!(edges(json!({"rows": [["a", "x"]]})).is_empty());
    }
}
