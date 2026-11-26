// The BIcraft Bullet chart's layout (docs/formats/bicraft-visuals.md "Bullet chart"), the same
// as crates/render/src/bvis_bullet.rs.
"use strict";

import { Prim, clip, domain, fmtAuto, inside, line, text } from "./spec";
import { Data, Out, PAD, col, count, fit, highlighted, hlOf, labelOf, labelWidth, num, optColor, optFlag, optNum, rows, scale, xAxis, yLabels } from "./common";
import { Look, band, bar, color, dim, lookOf, minimal, part, pushCard, valueLabel } from "./runtime/style";

export const MAX_ROWS = 100;
export const MAX_BANDS = 4;

/** Which side of band `j` of `n` stays square, from the low end (the outer ends of the track are rounded). */
function bandSide(j: number, n: number, first: string, last: string): string {
    if (j === 0) {
        return j + 1 === n ? "" : first;
    }
    return j + 1 === n ? last : "x";
}

/** The shade of band `j` of `n`: `n` bands take the last `n` of the look's five. */
function shade(k: Look, j: number, n: number): string {
    const idx = Math.max(k.shades.length - n, 0) + j;
    return idx < k.shades.length ? k.shades[idx] : k.shades[4];
}

function targetWidth(k: Look): number {
    return minimal(k) ? 2 : 3;
}

function thresholds(d: Data, i: number, pct: number[]): number[] {
    let t: number[] = [];
    if (count(d, "bands") > 0) {
        for (let k = 0; k < MAX_BANDS; k++) {
            const v = num(col(d, "bands", k), i);
            if (v !== null) {
                t.push(v);
            }
        }
    } else {
        const target = num(col(d, "target", 0), i);
        if (target !== null) {
            t = pct.filter((p) => p > 0).map((p) => (target * p) / 100);
        }
    }
    t.sort((a, b) => a - b);
    return t;
}

/** Row `i`'s value bar (`draw` draws a value's): highlighted, the full value dimmed and its highlighted part over it (docs/formats/bicraft-visuals.md "Highlights"). */
function valueBar(d: Data, i: number, v: number, draw: (v: number) => Prim, prims: Prim[], marks: number[]): void {
    marks.push(prims.length);
    if (!highlighted(d)) {
        prims.push(draw(v));
        return;
    }
    prims.push(dim(draw(v)));
    const p = part(v, hlOf(col(d, "value", 0), i));
    if (p !== null) {
        marks.push(prims.length);
        prims.push(draw(p));
    }
}

function clamp(v: number, lo: number, hi: number): number {
    return Math.min(Math.max(v, lo), hi);
}

/** Lays the bullets out; `out` gets one unit per row (stats: value, target). */
export function layout(d: Data, o: any, w: number, h: number, out?: Out): Prim[] {
    const prims: Prim[] = [];
    const pct = [optNum(o, "bands", "first", 60, 0, 1000), optNum(o, "bands", "second", 80, 0, 1000), optNum(o, "bands", "third", 100, 0, 1000)];
    const lk = lookOf(o && o.style, o && o.$theme);
    const vcolor = optColor(o, "colors", "value", color(lk, 0));
    const tcolor = optColor(o, "colors", "target", lk.label);
    const labels = optFlag(o, "labels", "show", false);
    const vertical = optFlag(o, "style", "vertical", false);
    const n = Math.min(rows(d), MAX_ROWS);
    if (n === 0) {
        return prims;
    }
    const val = (i: number) => num(col(d, "value", 0), i);
    const tgt = (i: number) => num(col(d, "target", 0), i);
    const max = (i: number) => num(col(d, "maximum", 0), i);
    const vals: number[] = [];
    for (let i = 0; i < n; i++) {
        for (const v of [val(i), tgt(i), max(i)]) {
            if (v !== null) {
                vals.push(v);
            }
        }
        for (const v of thresholds(d, i, pct)) {
            vals.push(v);
        }
    }
    const dom = domain(vals, true, null, null, 5);
    const cat = col(d, "category", 0);
    const names: string[] = [];
    for (let i = 0; i < n; i++) {
        names.push(cat ? clip(labelOf(cat, i), 24) : "");
    }
    if (vertical) {
        // one column per row: the scale on the left, the category names below
        const left = PAD + 36;
        const right = w - PAD;
        const top = PAD + 6;
        const bottom = h - PAD - (cat ? 18 : 0);
        if (right - left < 8 || bottom - top < 8) {
            return prims;
        }
        const cw = (right - left) / n;
        if (cw < 3) {
            return prims;
        }
        const bw = Math.min(cw * 0.7, 32);
        pushCard(prims, lk, w, h);
        const ys = (v: number) => scale(dom, bottom, top, v);
        const zero = ys(clamp(0, dom.lo, dom.hi));
        for (let i = 0; i < n; i++) {
            const x0 = left + cw * i + (cw - bw) / 2;
            const mid = x0 + bw / 2;
            const th = thresholds(d, i, pct).map((t) => clamp(t, dom.lo, dom.hi));
            const m = max(i);
            const end = m === null ? dom.hi : clamp(m, dom.lo, dom.hi);
            const k = th.length + 1;
            const bands: number[] = [];
            let from = dom.lo;
            const edges = th.concat([end]);
            for (let j = 0; j < edges.length; j++) {
                const a = ys(from);
                const b = ys(Math.max(edges[j], from));
                if (a > b) {
                    bands.push(prims.length);
                    prims.push(band(lk, x0, b, bw, a - b, shade(lk, j, k), bandSide(j, k, "t", "b")));
                }
                from = Math.max(edges[j], from);
            }
            const v = val(i);
            const marks: number[] = [];
            if (v !== null) {
                const y = ys(v);
                const sq = v >= 0 ? "b" : "t";
                valueBar(d, i, v, (q) => bar(lk, x0 + bw / 3, Math.min(ys(q), zero), bw / 3, Math.abs(ys(q) - zero), vcolor, sq), prims, marks);
                if (labels) {
                    marks.push(valueLabel(prims, lk, mid, y, fmtAuto(v), v >= 0 ? "n" : "s", 7, lk.value));
                }
            }
            const t = tgt(i);
            if (t !== null) {
                const y = ys(clamp(t, dom.lo, dom.hi));
                marks.push(prims.length);
                prims.push(line(x0 + bw * 0.15, y, x0 + bw * 0.85, y, tcolor, targetWidth(lk)));
            }
            if (cat) {
                const s = fit(names[i], cw - 4, lk.tick);
                prims.push(text(inside(mid, s, lk.tick, w), bottom + 9, s, lk.tick, lk.label, "middle", 0));
            }
            if (out) {
                out.units.push({ rows: [i], stats: [v === null ? NaN : v, t === null ? NaN : t] });
                for (const p of marks.concat(bands)) {
                    out.hits.push({ unit: i, prim: p });
                }
                out.outline = (out.outline || []).concat(bands);
            }
        }
        yLabels(prims, lk, dom, left - 4, top, bottom);
        prims.push(line(left, bottom, right, bottom, lk.axis, 1));
        return prims;
    }
    const lw = cat ? labelWidth(names, lk.name, (w - 2 * PAD) * 0.4) : 0;
    const left = PAD + lw;
    const right = w - PAD;
    const top = PAD;
    const bottom = h - PAD - 18;
    if (right - left < 8 || bottom - top < 8) {
        return prims;
    }
    const rh = (bottom - top) / n;
    if (rh < 3) {
        return prims;
    }
    const bh = Math.min(rh * 0.7, 32);
    pushCard(prims, lk, w, h);
    const xs = (v: number) => scale(dom, left, right, v);
    const zero = xs(clamp(0, dom.lo, dom.hi));
    for (let i = 0; i < n; i++) {
        const y0 = top + rh * i + (rh - bh) / 2;
        const mid = y0 + bh / 2;
        const th = thresholds(d, i, pct).map((t) => clamp(t, dom.lo, dom.hi));
        const m = max(i);
        const end = m === null ? dom.hi : clamp(m, dom.lo, dom.hi);
        const k = th.length + 1;
        const bands: number[] = [];
        let from = dom.lo;
        const edges = th.concat([end]);
        for (let j = 0; j < edges.length; j++) {
            const a = xs(from);
            const b = xs(Math.max(edges[j], from));
            if (b > a) {
                bands.push(prims.length);
                prims.push(band(lk, a, y0, b - a, bh, shade(lk, j, k), bandSide(j, k, "r", "l")));
            }
            from = Math.max(edges[j], from);
        }
        const v = val(i);
        const marks: number[] = [];
        if (v !== null) {
            const x = xs(v);
            const sq = v >= 0 ? "l" : "r";
            valueBar(d, i, v, (q) => bar(lk, Math.min(xs(q), zero), y0 + bh / 3, Math.abs(xs(q) - zero), bh / 3, vcolor, sq), prims, marks);
            if (labels) {
                marks.push(valueLabel(prims, lk, x, mid, fmtAuto(v), v >= 0 ? "e" : "w", 4, lk.value));
            }
        }
        const t = tgt(i);
        if (t !== null) {
            const x = xs(clamp(t, dom.lo, dom.hi));
            marks.push(prims.length);
            prims.push(line(x, y0 + bh * 0.15, x, y0 + bh * 0.85, tcolor, targetWidth(lk)));
        }
        if (cat) {
            prims.push(text(left - 6, mid, fit(names[i], lw - 6, lk.name), lk.name, lk.label, "end", 0));
        }
        if (out) {
            out.units.push({ rows: [i], stats: [v === null ? NaN : v, t === null ? NaN : t] });
            // the value bar first: it takes the keyboard focus
            for (const p of marks.concat(bands)) {
                out.hits.push({ unit: i, prim: p });
            }
            out.outline = (out.outline || []).concat(bands);
        }
    }
    xAxis(prims, lk, dom, left, right, bottom, w);
    return prims;
}
