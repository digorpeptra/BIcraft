// The BIcraft Histogram's layout (docs/formats/bicraft-visuals.md "Histogram"), the same as
// crates/render/src/bvis_histogram.rs.
"use strict";

import { Prim, domain, fmtAuto } from "./spec";
import { Data, Out, fade, PAD, col, num, optColor, optFlag, optInt, optNum, rows, scale, xAxis, yGrid, yLabels } from "./common";
import { bar, color, lookOf, minimal, pushCard, valueLabel } from "./runtime/style";

export const MAX_BINS = 200;
/** Most rows a cumulative bin selects. */
const MAX_UNIT_ROWS = 5000;

export interface Bins {
    start: number;
    size: number;
    counts: number[];
    /** The data rows counted in each bin. */
    members: number[][];
}

/** `[value, weight, row]` for rows with a value and a positive weight. */
function values(d: Data): number[][] {
    const vc = col(d, "values", 0);
    const out: number[][] = [];
    if (!vc) {
        return out;
    }
    const fc = col(d, "frequency", 0);
    const n = rows(d);
    for (let i = 0; i < n; i++) {
        const v = num(vc, i);
        if (v === null) {
            continue;
        }
        let f = 1;
        if (fc) {
            const x = num(fc, i);
            if (x === null || !(x > 0)) {
                continue;
            }
            f = x;
        }
        out.push([v, f, i]);
    }
    return out;
}

export function bins(vals: number[][], count: number, size: number): Bins | null {
    let lo = Infinity;
    let hi = -Infinity;
    for (const p of vals) {
        lo = Math.min(lo, p[0]);
        hi = Math.max(hi, p[0]);
    }
    if (!(isFinite(lo) && isFinite(hi))) {
        return null;
    }
    let start: number;
    let n: number;
    if (size > 0) {
        start = Math.floor(lo / size) * size;
        const k = Math.floor((hi - start) / size) + 1;
        if (k > MAX_BINS) {
            size = (hi - start) / MAX_BINS;
            n = MAX_BINS;
        } else {
            n = Math.max(k, 1);
        }
    } else if (hi === lo) {
        start = lo - 0.5;
        size = 1;
        n = 1;
    } else {
        n = Math.min(Math.max(count, 1), MAX_BINS);
        start = lo;
        size = (hi - lo) / n;
    }
    if (!(size > 0 && isFinite(size) && isFinite(start) && isFinite(start + size * n))) {
        return null;
    }
    const counts: number[] = [];
    const members: number[][] = [];
    for (let k = 0; k < n; k++) {
        counts.push(0);
        members.push([]);
    }
    for (const p of vals) {
        const k = Math.min(Math.max(Math.floor((p[0] - start) / size), 0), n - 1);
        counts[k] += p[1];
        members[k].push(p[2]);
    }
    return { start, size, counts, members };
}

/** Lays the histogram out (`draw`). Another visual's selection dims the bins none of whose rows it highlights (`fade`; docs/formats/bicraft-visuals.md "Highlights"). */
export function layout(d: Data, o: any, w: number, h: number, out?: Out): Prim[] {
    const prims = draw(d, o, w, h, out);
    if (out) {
        fade(d, out);
    }
    return prims;
}

/** Lays the histogram out; `out` gets one unit per bin (stats: from, to, count). */
function draw(d: Data, o: any, w: number, h: number, out?: Out): Prim[] {
    const prims: Prim[] = [];
    const count = optInt(o, "bins", "count", 10, 1, MAX_BINS);
    const size = optNum(o, "bins", "size", 0, 0, 1e15);
    const k = lookOf(o && o.style, o && o.$theme);
    const fill = optColor(o, "colors", "fill", color(k, 0));
    const labels = optFlag(o, "labels", "show", false);
    const cumulative = optFlag(o, "bins", "cumulative", false);
    const b = bins(values(d), count, size);
    if (!b) {
        return prims;
    }
    if (cumulative) {
        // each bar counts its bin and every bin before it
        for (let k = 1; k < b.counts.length; k++) {
            b.counts[k] += b.counts[k - 1];
        }
        let acc: number[] = [];
        for (let k = 0; k < b.members.length; k++) {
            acc = acc.concat(b.members[k].slice(0, Math.max(MAX_UNIT_ROWS - acc.length, 0)));
            b.members[k] = acc;
        }
    }
    const left = PAD + 36;
    const right = w - PAD;
    const top = PAD + 6;
    const bottom = h - PAD - 18;
    if (right - left < 8 || bottom - top < 8) {
        return prims;
    }
    const n = b.counts.length;
    const end = b.start + b.size * n;
    const xd = domain([], false, b.start, end, 5);
    const yd = domain(b.counts, true, null, null, 5);
    pushCard(prims, k, w, h);
    yGrid(prims, k, yd, left, right, top, bottom);
    for (let j = 0; j < n; j++) {
        const c = b.counts[j];
        if (out) {
            out.units.push({ rows: b.members[j], stats: [cumulative ? b.start : b.start + b.size * j, b.start + b.size * (j + 1), c, cumulative ? 1 : 0] });
        }
        if (c <= 0) {
            continue;
        }
        const x0 = scale(xd, left, right, b.start + b.size * j);
        const x1 = scale(xd, left, right, b.start + b.size * (j + 1));
        const bw = x1 - x0;
        // a surface gap between touching bars: 2 px in the modern looks when they are wide
        const gap = !minimal(k) && bw > 6 ? 2 : bw > 3 ? 1 : 0;
        const y = scale(yd, bottom, top, c);
        if (out) {
            out.hits.push({ unit: j, prim: prims.length });
        }
        prims.push(bar(k, x0 + gap / 2, y, bw - gap, bottom - y, fill, "b"));
        if (labels) {
            const t = valueLabel(prims, k, (x0 + x1) / 2, y, fmtAuto(c), "n", 7, k.value);
            if (out) {
                out.hits.push({ unit: j, prim: t });
            }
        }
    }
    yLabels(prims, k, yd, left - 4, top, bottom);
    xAxis(prims, k, xd, left, right, bottom, w);
    return prims;
}
