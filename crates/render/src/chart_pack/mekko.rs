//! BIcraft Marimekko (docs/formats/chart-pack.md "Marimekko"): 100 % stacked columns whose width
//! is each category's share of the total, so every segment's area is its share of the whole.
//! The Rust twin of `crates/engine/src/chart_pack/mekko.ts`.

use std::collections::HashMap;

use serde_json::{Value, json};

use super::{
    Out, PAD, cell_label, cell_num, cell_text, est, fit, highlighted, hl_num, legend, line, look_of,
    opt_bool, opt_color, opt_num, options_of, rows_of, text,
};
use crate::look::text_w;
use crate::spec::{Prim, fmt_fixed};

pub const MAX_ROWS: usize = 3000;
pub const MAX_CATEGORIES: usize = 50;
pub const MAX_SERIES: usize = 20;
/// Series with a colour option (`color1` … `color6`).
pub const SERIES_COLORS: usize = 6;

/// Lays out `{rows: [[category, series, value]], options}` in a `w`×`h` visual.
pub fn layout(input: &Value, w: f64, h: f64) -> Out {
    let o = options_of(input);
    let show_labels = opt_bool(o, "showLabels", true);
    let show_legend = opt_bool(o, "showLegend", true);
    let k = look_of(input);
    let size = opt_num(o, "fontSize", 8.0, 20.0, k.value);
    let sort_columns = opt_bool(o, "sortColumns", false);
    let width_axis = opt_bool(o, "widthAxis", false);
    let mut out = Out::default();
    let mut cats: Vec<String> = Vec::new();
    let mut series: Vec<String> = Vec::new();
    let mut cat_ix: HashMap<String, usize> = HashMap::new();
    let mut ser_ix: HashMap<String, usize> = HashMap::new();
    // (value, first row, highlighted part) by category, then series; the part is `None` when
    // none of its rows is highlighted
    let lit = highlighted(input);
    let mut cell: Vec<Vec<Option<(f64, usize, Option<f64>)>>> = Vec::new();
    for (i, r) in rows_of(input, MAX_ROWS).into_iter().enumerate() {
        let c = cell_text(r.first());
        let s = cell_text(r.get(1));
        let ci = match cat_ix.get(&c) {
            Some(&ci) => ci,
            None => {
                if cats.len() >= MAX_CATEGORIES {
                    continue;
                }
                cat_ix.insert(c, cats.len());
                cats.push(cell_label(input, 0, r.first()));
                cell.push(Vec::new());
                cats.len() - 1
            }
        };
        let si = match ser_ix.get(&s) {
            Some(&si) => si,
            None => {
                if series.len() >= MAX_SERIES {
                    continue;
                }
                ser_ix.insert(s, series.len());
                series.push(cell_label(input, 1, r.get(1)));
                series.len() - 1
            }
        };
        let v = cell_num(r.get(2));
        let Some(row) = cell.get_mut(ci) else { continue };
        if row.len() <= si {
            row.resize(si + 1, None);
        }
        let Some(slot) = row.get_mut(si) else { continue };
        let entry = slot.get_or_insert((0.0, i, None));
        if let Some(v) = v
            && v > 0.0
        {
            entry.0 += v;
        }
        if let Some(hv) = hl_num(input, i)
            && hv > 0.0
        {
            entry.2 = Some(entry.2.unwrap_or(0.0) + hv);
        }
    }
    if cats.is_empty() {
        return out;
    }
    // a series takes the look's palette colour (the theme's or BIcraft's), or its colour option
    // when one is set (the first six)
    let colors: Vec<String> = (0..series.len())
        .map(|i| {
            if i < SERIES_COLORS {
                opt_color(o, &format!("color{}", i + 1), &k.color(i))
            } else {
                k.color(i)
            }
        })
        .collect();
    k.push_card(&mut out.prims, w, h);
    // Modern and Bold part the columns and the segments with a surface gap
    let gap = if k.minimal() { 0.0 } else { 2.0 };
    let with_legend = show_legend && !(series.len() == 1 && series.first().is_some_and(String::is_empty));
    if with_legend {
        legend(&mut out.prims, &k, &series, &colors, PAD, PAD + 4.0, w - PAD);
    }
    let x0 = PAD + 36.0;
    let x1 = w - PAD;
    let y0 = if with_legend { PAD + 24.0 } else { PAD + 6.0 } + if width_axis { 14.0 } else { 0.0 };
    let y1 = h - PAD - 18.0;
    let pw = x1 - x0;
    let ph = y1 - y0;
    if pw < 8.0 || ph < 8.0 {
        return out;
    }
    for t in [0.0, 25.0, 50.0, 75.0, 100.0] {
        let y = y1 - (ph * t) / 100.0;
        out.prims.push(line(x0, y, x1, y, k.grid, 1.0));
        out.prims
            .push(text(x0 - 4.0, y, format!("{t}%"), k.tick, k.text, "end"));
    }
    if width_axis {
        // the columns' widths: shares of the grand total along the top
        for t in [0.0, 25.0, 50.0, 75.0, 100.0] {
            let x = x0 + (pw * t) / 100.0;
            let anchor = if t == 0.0 {
                "start"
            } else if t == 100.0 {
                "end"
            } else {
                "middle"
            };
            out.prims.push(line(x, y0 - 3.0, x, y0, k.axis, 1.0));
            out.prims
                .push(text(x, y0 - 9.0, format!("{t}%"), k.tick - 1.0, k.text, anchor));
        }
    }
    let value = |ci: usize, si: usize| cell.get(ci).and_then(|r| r.get(si)).copied().flatten();
    let totals: Vec<f64> = (0..cats.len())
        .map(|ci| {
            let mut t = 0.0;
            for si in 0..series.len() {
                t += value(ci, si).map_or(0.0, |v| v.0);
            }
            t
        })
        .collect();
    let mut grand = 0.0;
    for t in &totals {
        grand += t;
    }
    // the columns in row order, or widest first (ties in row order)
    let mut order: Vec<usize> = (0..cats.len()).collect();
    if sort_columns {
        let total = |i: &usize| totals.get(*i).copied().unwrap_or(0.0);
        order.sort_by(|a, b| total(b).total_cmp(&total(a)).then(a.cmp(b)));
    }
    if grand > 0.0 {
        let mut x = x0;
        for ci in order {
            let (Some(&total), Some(name)) = (totals.get(ci), cats.get(ci)) else {
                continue;
            };
            if total.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
                continue;
            }
            let cw = (pw * total) / grand;
            let inner = if gap > 0.0 && cw >= 6.0 {
                cw - gap
            } else if cw >= 3.0 {
                cw - 1.0
            } else {
                cw
            };
            let mut base = y1;
            let mut lowest = true;
            for si in 0..series.len() {
                let Some((v, first, hsum)) = value(ci, si) else {
                    continue;
                };
                if v.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
                    continue;
                }
                let share = v / total;
                let sh = ph * share;
                out.hit(
                    first as i64,
                    json!({"category": ci, "series": si, "share": share}),
                );
                let color = colors.get(si).cloned().unwrap_or_else(|| k.color(0));
                // the gap under every segment but the lowest, when the segment keeps 2 px
                let g = if !lowest && sh - gap >= 2.0 { gap } else { 0.0 };
                let tile = k.tile(x, base - sh, inner, sh - g, &color, 3.0, true);
                let top = match &tile {
                    Prim::Rect { deco, .. } if !deco.grad.is_empty() => deco.grad.clone(),
                    _ => color.clone(),
                };
                out.prims.push(if lit { crate::look::dim(tile) } else { tile });
                // its highlighted part from its foot, over the dimmed segment
                if let Some(hv) = crate::look::part(v, hsum.filter(|_| lit)) {
                    let hp = ((sh - g) * hv) / v;
                    out.hit(
                        first as i64,
                        json!({"category": ci, "series": si, "share": share}),
                    );
                    out.prims
                        .push(k.tile(x, base - g - hp, inner, hp, &color, 3.0, true));
                }
                if show_labels {
                    let label = format!("{}%", fmt_fixed(share * 100.0, 0));
                    if est(&label, size) + 4.0 <= inner && sh >= size + 4.0 {
                        let ink = k.on(&top, &color);
                        out.prims.push(text_w(
                            x + inner / 2.0,
                            base - (sh + g) / 2.0,
                            label,
                            size,
                            &ink,
                            "middle",
                            0.0,
                            k.value_weight(),
                        ));
                    }
                }
                base -= sh;
                lowest = false;
            }
            let name = fit(name, k.tick, cw - 2.0);
            if !name.is_empty() {
                out.prims
                    .push(text(x + cw / 2.0, y1 + 9.0, name, k.tick, k.text, "middle"));
            }
            x += cw;
        }
    }
    out.prims.push(line(x0, y1, x1, y1, k.axis, 1.0));
    out
}
