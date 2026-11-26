// BIcraft Marimekko (docs/formats/chart-pack.md "Marimekko"): 100 % stacked columns whose width is
// each category's share of the total, so every segment's area is its share of the whole. The
// TypeScript twin of crates/render/src/chart_pack/mekko.rs.
"use strict";

import { Out, PAD, cellLabel, cellNum, cellText, est, fit, fmtFixed, highlighted, hlNum, legend, line, lookOfInput, optBool, optColor, optNum, optionsOf, rowsOf, text } from "./kit";
import { color, dim, minimal, onColor, part, pushCard, textW, tile, valueWeight } from "./runtime/style";

export const MAX_ROWS = 3000;
export const MAX_CATEGORIES = 50;
export const MAX_SERIES = 20;
/** Series with a colour option (`color1` … `color6`). */
export const SERIES_COLORS = 6;

/** Lays out `{rows: [[category, series, value]], options}` in a `w`×`h` visual. */
export function layoutMekko(input: any, w: number, h: number): Out {
    const o = optionsOf(input);
    const showLabels = optBool(o, "showLabels", true);
    const showLegend = optBool(o, "showLegend", true);
    const k = lookOfInput(input);
    const size = optNum(o, "fontSize", 8, 20, k.value);
    const sortColumns = optBool(o, "sortColumns", false);
    const widthAxis = optBool(o, "widthAxis", false);
    const out: Out = { prims: [], hits: [] };
    const cats: string[] = [];
    const series: string[] = [];
    const catIx = new Map<string, number>();
    const serIx = new Map<string, number>();
    // value and first row by category, then series
    const cell: number[][] = [];
    const first: number[][] = [];
    // highlighted: the highlighted part of each segment (undefined: none of its rows is)
    const lit = highlighted(input);
    const hcell: number[][] = [];
    const rows = rowsOf(input, MAX_ROWS);
    for (let i = 0; i < rows.length; i++) {
        const c = cellText(rows[i][0]);
        const s = cellText(rows[i][1]);
        let ci = catIx.get(c);
        if (ci === undefined) {
            if (cats.length >= MAX_CATEGORIES) {
                continue;
            }
            ci = cats.length;
            catIx.set(c, ci);
            cats.push(cellLabel(input, 0, rows[i][0]));
            cell.push([]);
            first.push([]);
            hcell.push([]);
        }
        let si = serIx.get(s);
        if (si === undefined) {
            if (series.length >= MAX_SERIES) {
                continue;
            }
            si = series.length;
            serIx.set(s, si);
            series.push(cellLabel(input, 1, rows[i][1]));
        }
        const v = cellNum(rows[i][2]);
        if (first[ci][si] === undefined) {
            first[ci][si] = i;
            cell[ci][si] = 0;
        }
        if (v !== null && v > 0) {
            cell[ci][si] += v;
        }
        const hv = hlNum(input, i);
        if (hv !== null && hv > 0) {
            hcell[ci][si] = (hcell[ci][si] === undefined ? 0 : hcell[ci][si]) + hv;
        }
    }
    if (cats.length === 0) {
        return out;
    }
    const p = out.prims;
    // a series takes the look's palette colour (the theme's or BIcraft's), or its colour option
    // when one is set (the first six)
    const colors = series.map((s, i) => (i < SERIES_COLORS ? optColor(o, "color" + (i + 1), color(k, i)) : color(k, i)));
    pushCard(p, k, w, h);
    // Modern and Bold part the columns and the segments with a surface gap
    const gap = minimal(k) ? 0 : 2;
    const withLegend = showLegend && !(series.length === 1 && series[0] === "");
    if (withLegend) {
        legend(p, k, series, colors, PAD, PAD + 4, w - PAD);
    }
    const x0 = PAD + 36;
    const x1 = w - PAD;
    const y0 = (withLegend ? PAD + 24 : PAD + 6) + (widthAxis ? 14 : 0);
    const y1 = h - PAD - 18;
    const pw = x1 - x0;
    const ph = y1 - y0;
    if (pw < 8 || ph < 8) {
        return out;
    }
    for (const t of [0, 25, 50, 75, 100]) {
        const y = y1 - (ph * t) / 100;
        p.push(line(x0, y, x1, y, k.grid, 1));
        p.push(text(x0 - 4, y, `${t}%`, k.tick, k.text, "end"));
    }
    if (widthAxis) {
        // the columns' widths: shares of the grand total along the top
        for (const t of [0, 25, 50, 75, 100]) {
            const x = x0 + (pw * t) / 100;
            p.push(line(x, y0 - 3, x, y0, k.axis, 1));
            p.push(text(x, y0 - 9, `${t}%`, k.tick - 1, k.text, t === 0 ? "start" : t === 100 ? "end" : "middle"));
        }
    }
    const totals = cats.map((c, ci) => {
        let t = 0;
        for (let si = 0; si < series.length; si++) {
            const v = cell[ci][si];
            t += v === undefined ? 0 : v;
        }
        return t;
    });
    let grand = 0;
    for (const t of totals) {
        grand += t;
    }
    // the columns in row order, or widest first (ties in row order)
    const order = cats.map((c, ci) => ci);
    if (sortColumns) {
        order.sort((a, b) => totals[b] - totals[a] || a - b);
    }
    if (grand > 0) {
        let x = x0;
        for (const ci of order) {
            const total = totals[ci];
            if (!(total > 0)) {
                continue;
            }
            const cw = (pw * total) / grand;
            const inner = gap > 0 && cw >= 6 ? cw - gap : cw >= 3 ? cw - 1 : cw;
            let base = y1;
            let lowest = true;
            for (let si = 0; si < series.length; si++) {
                const v = cell[ci][si];
                if (v === undefined || !(v > 0)) {
                    continue;
                }
                const share = v / total;
                const sh = ph * share;
                out.hits.push({ row: first[ci][si], prim: p.length, category: ci, series: si, share });
                // the gap under every segment but the lowest, when the segment keeps 2 px
                const g = !lowest && sh - gap >= 2 ? gap : 0;
                const seg = tile(k, x, base - sh, inner, sh - g, colors[si], 3, true);
                const top = seg.grad ? seg.grad : colors[si];
                p.push(lit ? dim(seg) : seg);
                // its highlighted part from its foot, over the dimmed segment
                const hv = lit ? part(v, hcell[ci][si] === undefined ? null : hcell[ci][si]) : null;
                if (hv !== null) {
                    const hp = ((sh - g) * hv) / v;
                    out.hits.push({ row: first[ci][si], prim: p.length, category: ci, series: si, share });
                    p.push(tile(k, x, base - g - hp, inner, hp, colors[si], 3, true));
                }
                if (showLabels) {
                    const label = fmtFixed(share * 100, 0) + "%";
                    if (est(label, size) + 4 <= inner && sh >= size + 4) {
                        p.push(textW(x + inner / 2, base - (sh + g) / 2, label, size, onColor(k, top, colors[si]), "middle", 0, valueWeight(k)));
                    }
                }
                base -= sh;
                lowest = false;
            }
            const name = fit(cats[ci], k.tick, cw - 2);
            if (name !== "") {
                p.push(text(x + cw / 2, y1 + 9, name, k.tick, k.text, "middle"));
            }
            x += cw;
        }
    }
    p.push(line(x0, y1, x1, y1, k.axis, 1));
    return out;
}
