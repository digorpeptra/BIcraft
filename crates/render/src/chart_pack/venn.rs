//! BIcraft Venn (docs/formats/chart-pack.md "Venn diagram"): two or three sets read from an
//! item and set membership table, drawn as fixed overlapping circles (the layout does not depend
//! on the counts, so it never moves; two sets can be drawn area-proportional) with the count of
//! items in each region. The Rust twin of
//! `crates/engine/src/chart_pack/venn.ts`.

use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};

use super::{
    Out, PAD, cell_label, cell_text, is_fr, legend, look_of, opt_bool, opt_color, opt_num, options_of,
    rows_of, text,
};
use crate::spec::fmt_fixed;

/// Most membership rows read.
pub const MAX_ROWS: usize = 5000;
/// Most distinct sets counted (three are drawn).
pub const MAX_SETS: usize = 50;

struct VennSet {
    name: String,
    first: usize,
    items: HashSet<String>,
}

/// The area of the overlap of two circles of radii `a` and `b` whose centres are `d` apart.
fn lens(a: f64, b: f64, d: f64) -> f64 {
    let ca = ((d * d + a * a - b * b) / (2.0 * d * a)).clamp(-1.0, 1.0);
    let cb = ((d * d + b * b - a * a) / (2.0 * d * b)).clamp(-1.0, 1.0);
    let k = ((-d + a + b) * (d + a - b) * (d - a + b) * (d + a + b)).max(0.0);
    a * a * ca.acos() + b * b * cb.acos() - 0.5 * k.sqrt()
}

/// The distance between the centres of two circles of radii `a` and `b` whose overlap has the
/// area π·`ab` (60 halvings): touching when `ab` is 0, one inside the other when it is the
/// smaller circle's whole area.
pub fn apart(a: f64, b: f64, ab: f64) -> f64 {
    let mut lo = (a - b).abs();
    let mut hi = a + b;
    if ab <= 0.0 {
        return hi;
    }
    if ab >= (a * a).min(b * b) {
        return lo;
    }
    let target = std::f64::consts::PI * ab;
    for _ in 0..60 {
        let d = (lo + hi) / 2.0;
        if lens(a, b, d) > target {
            lo = d;
        } else {
            hi = d;
        }
    }
    (lo + hi) / 2.0
}

/// Lays out `{rows: [[item, set]], options, lang}` in a `w`×`h` visual.
pub fn layout(input: &Value, w: f64, h: f64) -> Out {
    let o = options_of(input);
    let lk = look_of(input);
    let colors = [
        opt_color(o, "color1", &lk.color(0)),
        opt_color(o, "color2", &lk.color(1)),
        opt_color(o, "color3", &lk.color(2)),
    ];
    let show_counts = opt_bool(o, "showCounts", true);
    let proportional = opt_bool(o, "proportional", false);
    let size = opt_num(o, "fontSize", 8.0, 24.0, 12.0 + lk.value - 10.0);
    let fr = is_fr(input);
    let mut out = Out::default();
    let mut sets: Vec<VennSet> = Vec::new();
    let mut by_name: HashMap<String, usize> = HashMap::new();
    for (i, r) in rows_of(input, MAX_ROWS).into_iter().enumerate() {
        let item = cell_text(r.first());
        let name = cell_text(r.get(1));
        if item.is_empty() || name.is_empty() {
            continue;
        }
        let k = match by_name.get(&name) {
            Some(&k) => k,
            None => {
                if sets.len() >= MAX_SETS {
                    continue;
                }
                by_name.insert(name, sets.len());
                sets.push(VennSet {
                    name: cell_label(input, 1, r.get(1)),
                    first: i,
                    items: HashSet::new(),
                });
                sets.len() - 1
            }
        };
        if let Some(s) = sets.get_mut(k) {
            s.items.insert(item);
        }
    }
    if sets.is_empty() {
        return out;
    }
    // the three largest sets (ties: the first seen), in the order they were seen
    let mut ranked: Vec<usize> = (0..sets.len()).collect();
    ranked.sort_by(|a, b| {
        let n = |i: &usize| sets.get(*i).map_or(0, |s| s.items.len());
        n(b).cmp(&n(a)).then(a.cmp(b))
    });
    let mut drawn: Vec<usize> = ranked.into_iter().take(3).collect();
    drawn.sort_unstable();
    let more = sets.len() - drawn.len();
    let k = drawn.len();
    let mut masks: HashMap<&str, usize> = HashMap::new();
    for (j, &d) in drawn.iter().enumerate() {
        if let Some(s) = sets.get(d) {
            for item in &s.items {
                *masks.entry(item.as_str()).or_insert(0) |= 1 << j;
            }
        }
    }
    let mut counts = [0usize; 8];
    for m in masks.values() {
        if let Some(c) = counts.get_mut(*m) {
            *c += 1;
        }
    }
    let names: Vec<String> = drawn
        .iter()
        .filter_map(|d| sets.get(*d).map(|s| s.name.clone()))
        .collect();
    lk.push_card(&mut out.prims, w, h);
    legend(&mut out.prims, &lk, &names, &colors, PAD, PAD + 4.0, w - PAD);
    if more > 0 {
        let note = match (fr, more) {
            (true, 1) => "1 autre ensemble non dessiné".to_string(),
            (true, n) => format!("{n} autres ensembles non dessinés"),
            (false, 1) => "1 more set not drawn".to_string(),
            (false, n) => format!("{n} more sets not drawn"),
        };
        out.prims
            .push(text(PAD, h - PAD - 6.0, note, lk.tick, lk.text, "start"));
    }
    let ax = PAD;
    let aw = w - 2.0 * PAD;
    let ay = PAD + 22.0;
    let ah = h - ay - PAD - if more > 0 { 14.0 } else { 0.0 };
    if aw < 16.0 || ah < 16.0 {
        return out;
    }
    let cx = ax + aw / 2.0;
    let cy = ay + ah / 2.0;
    let s3 = 3f64.sqrt();
    let mut rs: Vec<f64> = Vec::new();
    let mut cs: Vec<(f64, f64)> = Vec::new();
    // region label points by mask (bit j: drawn set j)
    let mut at: [Option<(f64, f64)>; 8] = [None; 8];
    match k {
        1 => {
            rs.push(aw.min(ah) / 2.0);
            cs.push((cx, cy));
            at[1] = Some((cx, cy));
        }
        2 if proportional => {
            // areas as the counts: radii √n, the overlap's area the shared items', scaled to fit
            let n = |j: usize| {
                drawn
                    .get(j)
                    .and_then(|d| sets.get(*d))
                    .map_or(1, |s| s.items.len())
            };
            let a = (n(0) as f64).sqrt();
            let b = (n(1) as f64).sqrt();
            let d = apart(a, b, counts[3] as f64);
            let lo = (-a).min(d - b);
            let hi = a.max(d + b);
            let f = (aw / (hi - lo)).min(ah / (2.0 * a.max(b)));
            let xa = cx - ((lo + hi) / 2.0) * f;
            let xb = xa + d * f;
            rs.push(a * f);
            rs.push(b * f);
            cs.push((xa, cy));
            cs.push((xb, cy));
            let (al, ar) = (xa - a * f, xa + a * f);
            let (bl, br) = (xb - b * f, xb + b * f);
            at[1] = Some(((al + al.max(bl.min(ar))) / 2.0, cy));
            at[2] = Some(((br + br.min(ar.max(bl))) / 2.0, cy));
            at[3] = Some(((al.max(bl) + ar.min(br)) / 2.0, cy));
        }
        2 => {
            let r = (aw / 3.0).min(ah / 2.0);
            rs.push(r);
            rs.push(r);
            cs.push((cx - r / 2.0, cy));
            cs.push((cx + r / 2.0, cy));
            at[1] = Some((cx - r, cy));
            at[2] = Some((cx + r, cy));
            at[3] = Some((cx, cy));
        }
        _ => {
            let r = (aw / 3.0).min(ah / (2.0 + s3 / 2.0));
            rs.extend([r, r, r]);
            let gy = cy - (r * s3) / 12.0;
            cs.push((cx - r / 2.0, gy - r / (2.0 * s3)));
            cs.push((cx + r / 2.0, gy - r / (2.0 * s3)));
            cs.push((cx, gy + r / s3));
            for (j, c) in cs.iter().enumerate() {
                if let Some(slot) = at.get_mut(1 << j) {
                    *slot = Some((c.0 + (c.0 - cx) * s3 * 0.45, c.1 + (c.1 - gy) * s3 * 0.45));
                }
            }
            for (a, b) in [(0usize, 1usize), (0, 2), (1, 2)] {
                if let (Some(p), Some(q)) = (cs.get(a), cs.get(b)) {
                    let mx = (p.0 + q.0) / 2.0;
                    let my = (p.1 + q.1) / 2.0;
                    if let Some(slot) = at.get_mut((1 << a) | (1 << b)) {
                        *slot = Some((mx + (mx - cx) * s3 * 0.5, my + (my - gy) * s3 * 0.5));
                    }
                }
            }
            at[7] = Some((cx, gy));
        }
    }
    for (j, c) in cs.iter().enumerate() {
        let (Some(&d), Some(color), Some(&r)) = (drawn.get(j), colors.get(j), rs.get(j)) else {
            continue;
        };
        let Some(s) = sets.get(d) else { continue };
        out.hit(s.first as i64, json!({"set": j, "n": s.items.len()}));
        out.prims.push(lk.blob(c.0, c.1, r, color, 0.35));
    }
    if show_counts {
        for (m, p) in at.iter().enumerate().skip(1) {
            if let Some((x, y)) = p {
                let n = counts.get(m).copied().unwrap_or(0);
                let t = lk.value_label(&mut out.prims, *x, *y, fmt_fixed(n as f64, 0), 'c', 0.0, size);
                out.hit_at(-1, t, json!({"region": m, "n": n}));
            }
        }
    }
    out
}
