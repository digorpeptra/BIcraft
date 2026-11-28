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

/// Format string `f` with the commas right after a section's last decimal placeholder moved
/// before its decimal point (`$#,0.00,,"M"` → `$#,0,,.00"M"`): read as scaling commas, ÷1000
/// each, as before the point. Quoted and escaped text and scientific sections stay as they are.
/// The twin of `scaleCommas` in `crates/engine/src/visual_runtime/format.ts`.
pub fn scale_commas(f: &str) -> String {
    // each character, and whether it is outside quotes and escapes
    let mut sec: Vec<(char, bool)> = Vec::new();
    let mut out = String::new();
    let (mut quoted, mut esc) = (false, false);
    for c in f.chars() {
        if esc || quoted {
            quoted = quoted && c != '"';
            esc = false;
            sec.push((c, false));
            continue;
        }
        match c {
            '"' => quoted = true,
            '\\' => esc = true,
            ';' => {
                out.push_str(&scale_section(&sec));
                out.push(';');
                sec.clear();
                continue;
            }
            _ => {}
        }
        sec.push((c, c != '"' && c != '\\'));
    }
    out.push_str(&scale_section(&sec));
    out
}

fn scale_section(sec: &[(char, bool)]) -> String {
    let text = |r: Option<&[(char, bool)]>| r.unwrap_or(&[]).iter().map(|(c, _)| *c).collect::<String>();
    let dot = sec.iter().position(|&(c, p)| p && c == '.');
    let last = sec.iter().rposition(|&(c, p)| p && (c == '0' || c == '#'));
    let sci = sec.iter().any(|&(c, p)| p && (c == 'E' || c == 'e'));
    let (Some(d), Some(l)) = (dot, last) else {
        return text(Some(sec));
    };
    let n = sec
        .get(l + 1..)
        .map_or(0, |r| r.iter().take_while(|&&(c, p)| p && c == ',').count());
    if sci || l < d || n == 0 {
        return text(Some(sec));
    }
    format!(
        "{}{}{}{}",
        text(sec.get(..d)),
        ",".repeat(n),
        text(sec.get(d..=l)),
        text(sec.get(l + 1 + n..))
    )
}

/// `v` written with the whole format string `f`, as BIcraft's DAX `FORMAT` reads it
/// (`bicraft_locale::vb`: scaling commas, quoted and escaped text, sections), after
/// [`scale_commas`], grouped with the culture's display separator; `None` without a format string
/// or for one `FORMAT` refuses.
pub(crate) fn full_format(v: f64, f: &str, loc: &crate::fmt::Locale) -> Option<String> {
    let g: String = f.chars().take(200).collect();
    if g.trim().is_empty() || !v.is_finite() {
        return None;
    }
    let s = bicraft_locale::vb::format(
        bicraft_locale::vb::Value::Number(v),
        &scale_commas(g.trim()),
        loc.culture,
    )?;
    let vb = loc.culture.group;
    Some(match loc.group {
        Some(g) if g != vb => s.replace(vb, &g.to_string()),
        _ => s,
    })
}

/// A value of the gauge written with its field's format string ([`full_format`]); without one,
/// or with one `FORMAT` refuses, as the pack writes numbers.
fn fmt_value(v: f64, f: &str, fr: bool) -> String {
    let loc = crate::fmt::Locale::new(if fr { "fr-FR" } else { "en-US" });
    full_format(v, f, &loc).unwrap_or_else(|| fmt_num(v, f, fr))
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
    /// The variance line with less and less of it, for the arc's narrow middle: the whole line,
    /// then without its words (`-8 (-10.0%)`), then the difference alone.
    var_texts: Vec<String>,
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
    let mut var_texts = Vec::new();
    if let Some(rf) = reference {
        let diff = val - rf;
        let diff_text = signed(diff, fmt_value(diff, &f, fr));
        var_text = diff_text.clone();
        if rf != 0.0 {
            var_text.push_str(&format!(" ({})", pct_text(diff / rf.abs(), fr)));
        }
        let bare = var_text.clone();
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
            fmt_value(rf, &f, fr)
        );
        var_texts = vec![var_text.clone(), bare, diff_text];
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
        val_text: fmt_value(val, &f, fr),
        min_text: fmt_value(min, &f, fr),
        max_text: fmt_value(max, &f, fr),
        var_text,
        var_texts,
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
    let min_x = inside(cx - big_r + thick / 2.0, &g.min_text, k.tick, w);
    let max_x = inside(cx + big_r - thick / 2.0, &g.max_text, k.tick, w);
    out.prims
        .push(text(min_x, ly, g.min_text.clone(), k.tick, k.text, "middle"));
    out.prims
        .push(text(max_x, ly, g.max_text.clone(), k.tick, k.text, "middle"));
    let inner = big_r - thick;
    let vs = (big_r * 0.3).clamp(12.0, 40.0);
    let vy = cy - inner * 0.35;
    let s = fit(&g.val_text, vs, 2.0 * inner - 8.0);
    if !s.is_empty() {
        out.hit(g.row as i64, g.value_hit());
        out.prims.push(text_w(cx, vy, s, vs, k.label, "middle", 0.0, 700));
    }
    // the variance under the value: the most of it that fits whole inside the ring, and between
    // the minimum and the maximum when it reaches their line, at the name size, else the tick size
    let gap = (cx - min_x - est(&g.min_text, k.tick) / 2.0).min(max_x - est(&g.max_text, k.tick) / 2.0 - cx);
    let var_y = |size: f64| vy + vs / 2.0 + 3.0 + size / 2.0;
    let room = |size: f64| {
        let r = 2.0 * inner - 8.0;
        if var_y(size) + size / 2.0 > ly - k.tick / 2.0 {
            r.min(2.0 * gap - 8.0)
        } else {
            r
        }
    };
    let pick = g
        .var_texts
        .iter()
        .flat_map(|t| [(t, k.name), (t, k.tick)])
        .find(|(t, size)| !t.is_empty() && est(t, *size) <= room(*size));
    if let Some((v, size)) = pick {
        out.prims.push(text_w(
            cx,
            var_y(size),
            v.clone(),
            size,
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
    let mut trend_h = if trend && h >= 80.0 {
        (h * 0.3).min(60.0)
    } else {
        0.0
    };
    let area_of = |t: f64| h - 2.0 * PAD - if t > 0.0 { t + 6.0 } else { 0.0 };
    let vs = (area_of(trend_h) * 0.42).min(w * 0.16).clamp(12.0, 48.0);
    let mut show_var = !g.var_text.is_empty();
    let mut show_ref = !g.ref_text.is_empty();
    let block_of =
        |var: bool, rf: bool| vs + if var { k.name + 6.0 } else { 0.0 } + if rf { k.tick + 4.0 } else { 0.0 };
    // the text first: the sparkline takes what it leaves (none under 16 px), then the reference
    // and the variance lines give way when even that is too little
    if trend_h > 0.0 && block_of(show_var, show_ref) > area_of(trend_h) {
        trend_h = h - 2.0 * PAD - 6.0 - block_of(show_var, show_ref);
        if trend_h < 16.0 {
            trend_h = 0.0;
        }
    }
    let area = area_of(trend_h);
    if block_of(show_var, show_ref) > area {
        show_ref = false;
    }
    if block_of(show_var, show_ref) > area {
        show_var = false;
    }
    let block = block_of(show_var, show_ref);
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
    // the most of the variance that fits whole, else the whole line shortened
    let v = g
        .var_texts
        .iter()
        .find(|t| !t.is_empty() && est(t, k.name) <= w - 2.0 * PAD)
        .cloned()
        .unwrap_or_else(|| fit(&g.var_text, k.name, w - 2.0 * PAD));
    if show_var && !v.is_empty() {
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
    if show_ref && !r.is_empty() {
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
    let fmt = vc
        .and_then(|c| ds.cols.get(c))
        .map(|c| c.fmt.clone())
        .unwrap_or_default();
    let value = |x: f64| full_format(x, &fmt, &ctx.loc).unwrap_or_else(|| num_text(ds, vc, x, ctx));
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

    /// Each text's box as the layouts estimate it: (left, top, right, bottom).
    fn text_boxes(out: &Out) -> Vec<(f64, f64, f64, f64, String)> {
        out.prims
            .iter()
            .filter_map(|p| match p {
                Prim::Text {
                    x,
                    y,
                    text,
                    size,
                    anchor,
                    ..
                } => {
                    let w = est(text, *size);
                    let x0 = match *anchor {
                        "middle" => x - w / 2.0,
                        "end" => x - w,
                        _ => *x,
                    };
                    Some((x0, y - size / 2.0, x0 + w, y + size / 2.0, text.clone()))
                }
                _ => None,
            })
            .collect()
    }

    fn overlap(a: &(f64, f64, f64, f64, String), b: (f64, f64, f64, f64)) -> bool {
        a.0 < b.2 && b.0 < a.2 && a.1 < b.3 && b.1 < a.3
    }

    /// Every text whole (no ellipsis), inside the `w`×`h` visual and clear of the other texts.
    fn texts_fit(out: &Out, w: f64, h: f64, at: &str) {
        let boxes = text_boxes(out);
        for (i, a) in boxes.iter().enumerate() {
            assert!(!a.4.contains('…'), "{at}: {:?} is cut", a.4);
            assert!(
                a.0 >= 0.0 && a.1 >= 0.0 && a.2 <= w && a.3 <= h,
                "{at}: {a:?} outside {w}x{h}"
            );
            for b in boxes.iter().skip(i + 1) {
                assert!(!overlap(a, (b.0, b.1, b.2, b.3)), "{at}: {a:?} over {b:?}");
            }
        }
    }

    /// The KPI card's numbers are written with the whole format string: scaling commas after
    /// the decimals and quoted text.
    #[test]
    fn the_value_takes_its_whole_format_string() {
        let input = json!({"rows": [[null, 7_433_915.0, null, null, null, null, null, null, 6_900_000.0]],
                           "formats": ["", "$#,0.00,,\"M\""], "options": {"gaugeStyle": "kpi"}});
        let texts: Vec<String> = text_boxes(&layout(&input, 300.0, 168.0))
            .into_iter()
            .map(|b| b.4)
            .collect();
        assert!(texts.iter().any(|t| t == "$7.43M"), "{texts:?}");
        assert!(texts.iter().any(|t| t == "Previous: $6.90M"), "{texts:?}");
        assert!(texts.iter().any(|t| t.starts_with("+$0.53M (")), "{texts:?}");
        let fr = json!({"rows": [[null, 7_433_915.0]], "formats": ["", "#,0.0,,\" M€\""],
                        "lang": "fr", "options": {"gaugeStyle": "kpi"}});
        let texts: Vec<String> = text_boxes(&layout(&fr, 300.0, 168.0))
            .into_iter()
            .map(|b| b.4)
            .collect();
        assert!(texts.iter().any(|t| t == "7,4 M€"), "{texts:?}");
    }

    /// The arc's variance line fits its narrow middle whole, with less of it when needed, and
    /// no text runs over another or out of the visual.
    #[test]
    fn the_arc_variance_fits_a_short_arc() {
        let one = json!({"rows": [[null, 0.375, 0.38]], "formats": ["", "0.0%"]});
        let trend = json!({"rows": [["a", 0.31, 0.38], ["b", 0.36, 0.38], ["c", 0.375, 0.38]],
                           "formats": ["", "0.0%"], "options": {"min": 0.3, "max": 0.45}});
        for input in [&one, &trend] {
            for (w, h) in [
                (300.0, 136.0),
                (276.0, 120.0),
                (240.0, 110.0),
                (300.0, 168.0),
                (360.0, 240.0),
            ] {
                let out = layout(input, w, h);
                let at = format!("{w}x{h} {input}");
                texts_fit(&out, w, h, &at);
                let boxes = text_boxes(&out);
                assert!(
                    boxes.iter().any(|b| b.4.starts_with("-0.5%")),
                    "{at}: no variance in {boxes:?}"
                );
            }
        }
    }

    /// The KPI card keeps its sparkline below the reference line at every height.
    #[test]
    fn the_kpi_sparkline_stays_clear_of_the_text() {
        let input = json!({"rows": [["a", 52, 70], ["b", 61, 70], ["c", 58, 75], ["d", 66, 75], ["e", 79, 80]],
                           "formats": ["", "#,0"], "options": {"gaugeStyle": "kpi", "compareTo": "previous"}});
        for w in [200.0, 300.0] {
            for h in (80..=240).step_by(4).map(f64::from) {
                let out = layout(&input, w, h);
                let at = format!("{w}x{h}");
                texts_fit(&out, w, h, &at);
                let mut spark: Option<(f64, f64, f64, f64)> = None;
                let mut add = |x0: f64, y0: f64, x1: f64, y1: f64| {
                    let b = spark.get_or_insert((x0, y0, x1, y1));
                    *b = (b.0.min(x0), b.1.min(y0), b.2.max(x1), b.3.max(y1));
                };
                for p in &out.prims {
                    match p {
                        Prim::Line { x1, y1, x2, y2, .. } => {
                            add(x1.min(*x2), y1.min(*y2), x1.max(*x2), y1.max(*y2))
                        }
                        Prim::Circle { cx, cy, r, .. } => add(cx - r, cy - r, cx + r, cy + r),
                        _ => {}
                    }
                }
                for t in text_boxes(&out) {
                    assert!(
                        !spark.is_some_and(|s| overlap(&t, s)),
                        "{at}: {t:?} over the sparkline {spark:?}"
                    );
                }
                assert!(
                    text_boxes(&out).iter().any(|t| t.4.starts_with("Previous: ")),
                    "{at}: no reference line"
                );
            }
        }
    }
}
