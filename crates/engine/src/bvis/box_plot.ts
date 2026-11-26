// The BIcraft Box plot's layout (docs/formats/bicraft-visuals.md "Box plot"), the same as
// crates/render/src/bvis_box.rs.
"use strict";

import { Prim, clip, domain, inside, line, text } from "./spec";
import { Data, Out, fade, PAD, col, fit, labelOf, labelWidth, num, optColor, optFlag, path, rows, scale, textOf, xAxis, yGrid, yLabels } from "./common";
import { boxPlain, boxed, color, dot, lookOf, minimal, mix, pushCard } from "./runtime/style";

export const MAX_BOXES = 100;
export const MAX_OUTLIERS = 500;

export interface Stats {
    n: number;
    min: number;
    q1: number;
    median: number;
    q3: number;
    max: number;
    low: number;
    high: number;
    mean: number;
    outliers: number[];
}

function quantile(s: number[], p: number): number {
    const h = Math.max(s.length - 1, 0) * p;
    const lo = Math.floor(h);
    const a = lo < s.length ? s[lo] : 0;
    const b = lo + 1 < s.length ? s[lo + 1] : a;
    return a + (h - lo) * (b - a);
}

export function stats(vals: number[]): Stats | null {
    if (vals.length === 0) {
        return null;
    }
    const s = vals.slice().sort((a, b) => a - b);
    const min = s[0];
    const max = s[s.length - 1];
    const q1 = quantile(s, 0.25);
    const median = quantile(s, 0.5);
    const q3 = quantile(s, 0.75);
    const iqr = q3 - q1;
    const lf = q1 - 1.5 * iqr;
    const hf = q3 + 1.5 * iqr;
    let low = min;
    for (const v of s) {
        if (v >= lf) {
            low = v;
            break;
        }
    }
    let high = max;
    for (let i = s.length - 1; i >= 0; i--) {
        if (s[i] <= hf) {
            high = s[i];
            break;
        }
    }
    let sum = 0;
    for (const v of s) {
        sum += v;
    }
    return { n: s.length, min, q1, median, q3, max, low, high, mean: sum / s.length, outliers: s.filter((v) => v < lf || v > hf) };
}

export interface Group {
    name: string;
    values: number[];
    rows: number[];
}

export function groups(d: Data): Group[] {
    const out: Group[] = [];
    const vc = col(d, "value", 0);
    if (!vc) {
        return out;
    }
    const cat = col(d, "category", 0);
    const index: { [k: string]: number } = {};
    const n = rows(d);
    for (let i = 0; i < n; i++) {
        const v = num(vc, i);
        if (v === null) {
            continue;
        }
        const key = cat ? textOf(cat, i) : "";
        const at = Object.prototype.hasOwnProperty.call(index, key) ? index[key] : -1;
        if (at >= 0) {
            out[at].values.push(v);
            out[at].rows.push(i);
        } else if (out.length < MAX_BOXES) {
            index[key] = out.length;
            out.push({ name: cat ? labelOf(cat, i) : "", values: [v], rows: [i] });
        }
    }
    return out;
}

/** Lays the box plot out (`draw`). Another visual's selection dims the boxes none of whose rows it highlights (`fade`; docs/formats/bicraft-visuals.md "Highlights"). */
export function layout(d: Data, o: any, w: number, h: number, out?: Out): Prim[] {
    const prims = draw(d, o, w, h, out);
    if (out) {
        fade(d, out);
    }
    return prims;
}

/** Lays the boxes out; `out` gets one unit per box (stats: n, min, q1, median, q3, max, mean, outliers). */
function draw(d: Data, o: any, w: number, h: number, out?: Out): Prim[] {
    const prims: Prim[] = [];
    const k = lookOf(o && o.style, o && o.$theme);
    const fill = optColor(o, "colors", "fill", color(k, 0));
    const plain = boxPlain(k);
    const whisker = minimal(k) ? 1 : 1.5;
    const medianW = minimal(k) ? 2 : 2.5;
    const medianC = minimal(k) ? fill : mix(fill, "#000000", 0.25);
    const showMean = optFlag(o, "box", "mean", true);
    const showOut = optFlag(o, "box", "outliers", true);
    const horizontal = optFlag(o, "box", "horizontal", false);
    const boxes: { g: Group; s: Stats }[] = [];
    for (const g of groups(d)) {
        const s = stats(g.values);
        if (s) {
            boxes.push({ g, s });
        }
    }
    if (boxes.length === 0) {
        return prims;
    }
    const hasCat = col(d, "category", 0) !== null;
    const vals: number[] = [];
    for (const b of boxes) {
        vals.push(b.s.low, b.s.high);
        if (showMean) {
            vals.push(b.s.mean);
        }
        if (showOut) {
            for (const v of b.s.outliers.slice(0, MAX_OUTLIERS)) {
                vals.push(v);
            }
        }
    }
    const dom = domain(vals, false, null, null, 5);
    // horizontal: the category names in a column on the left, the values along the bottom
    const names = boxes.map((b) => clip(b.g.name, 24));
    const lw = horizontal && hasCat ? labelWidth(names, k.tick, (w - 2 * PAD) * 0.4) : 0;
    const left = horizontal ? PAD + lw : PAD + 36;
    const right = w - PAD;
    const top = horizontal ? PAD : PAD + 6;
    const bottom = horizontal ? h - PAD - 18 : h - PAD - (hasCat ? 18 : 0);
    if (right - left < 8 || bottom - top < 8) {
        return prims;
    }
    // the point at value `v` and `u` across the value axis
    const pt = (v: number, u: number) => (horizontal ? [scale(dom, left, right, v), u] : [u, scale(dom, bottom, top, v)]);
    const seg = (v1: number, u1: number, v2: number, u2: number, width: number) => {
        const a = pt(v1, u1);
        const b = pt(v2, u2);
        return line(a[0], a[1], b[0], b[1], fill, width);
    };
    pushCard(prims, k, w, h);
    if (horizontal) {
        for (const t of dom.ticks) {
            const x = scale(dom, left, right, t);
            prims.push(line(x, top, x, bottom, k.grid, 1));
        }
    } else {
        yGrid(prims, k, dom, left, right, top, bottom);
    }
    const step = (horizontal ? bottom - top : right - left) / boxes.length;
    const bw = Math.min(step * 0.5, 48);
    for (let g = 0; g < boxes.length; g++) {
        const s = boxes[g].s;
        const c = (horizontal ? top : left) + step * (g + 0.5);
        const first = prims.length;
        prims.push(seg(s.low, c, s.q1, c, whisker));
        prims.push(seg(s.q3, c, s.high, c, whisker));
        prims.push(seg(s.low, c - bw / 4, s.low, c + bw / 4, whisker));
        prims.push(seg(s.high, c - bw / 4, s.high, c + bw / 4, whisker));
        const box = prims.length;
        const q1 = pt(s.q1, c - bw / 2);
        const q3 = pt(s.q3, c + bw / 2);
        prims.push(boxed(k, Math.min(q1[0], q3[0]), Math.min(q1[1], q3[1]), Math.abs(q3[0] - q1[0]), Math.abs(q3[1] - q1[1]), fill));
        if (plain) {
            prims.push(path([pt(s.q3, c - bw / 2), pt(s.q3, c + bw / 2), pt(s.q1, c + bw / 2), pt(s.q1, c - bw / 2), pt(s.q3, c - bw / 2)], fill, 1));
        }
        const ma = pt(s.median, c - bw / 2);
        const mb = pt(s.median, c + bw / 2);
        prims.push(line(ma[0], ma[1], mb[0], mb[1], medianC, medianW));
        if (showMean) {
            const m = pt(s.mean, c);
            prims.push(dot(k, m[0], m[1], minimal(k) ? 3 : 3.5, k.label, 1));
        }
        if (showOut) {
            for (const v of s.outliers.slice(0, MAX_OUTLIERS)) {
                const q = pt(v, c);
                prims.push(dot(k, q[0], q[1], minimal(k) ? 2.5 : 3, fill, minimal(k) ? 0.6 : 0.75));
            }
        }
        if (out) {
            out.units.push({ rows: boxes[g].g.rows, stats: [s.n, s.min, s.q1, s.median, s.q3, s.max, s.mean, s.outliers.length] });
            // the box first: it takes the keyboard focus
            out.hits.push({ unit: g, prim: box });
            out.outline = (out.outline || []).concat([box]);
            for (let p = first; p < prims.length; p++) {
                if (p !== box) {
                    out.hits.push({ unit: g, prim: p });
                }
            }
        }
    }
    if (horizontal) {
        xAxis(prims, k, dom, left, right, bottom, w);
        if (hasCat) {
            for (let g = 0; g < boxes.length; g++) {
                prims.push(text(left - 6, top + step * (g + 0.5), fit(names[g], lw - 6, k.tick), k.tick, k.text, "end", 0));
            }
        }
        return prims;
    }
    yLabels(prims, k, dom, left - 4, top, bottom);
    prims.push(line(left, bottom, right, bottom, k.axis, 1));
    if (hasCat) {
        for (let g = 0; g < boxes.length; g++) {
            const cx = left + step * (g + 0.5);
            const s = fit(boxes[g].g.name, step - 4, k.tick);
            prims.push(text(inside(cx, s, k.tick, w), bottom + 9, s, k.tick, k.text, "middle", 0));
        }
    }
    return prims;
}
