// BIcraft Scatter+ (docs/formats/chart-pack.md "Scatter+"): a scatter chart that draws every
// point up to MAX_POINTS (none sampled), switches to a hexagonal density view above a threshold,
// can add marginal histograms or box strips on both axes, a least-squares trend line, quadrant
// lines at averages or given values with their labels, and labels on the top N or the selected
// points. Markers can be filled, bordered or both, sized by a measure and coloured by a legend.
// The TypeScript twin of crates/render/src/chart_pack/scatter.rs.
"use strict";

import { Out, PAD, Prim, cellLabel, cellNum, cellText, circle, compact, dashes, est, fit, fmtFixed, highlighted, hlNum, langOf, legend, line, lookOfInput, optBool, optEnum, optNum, optText, optionsOf, poly, rowsOf, text } from "./kit";
import { Domain, domain, inside, tickDecimals } from "./spec";
import { Look, bar, boxed, color, dim, mix, pushCard, textW, valueWeight } from "./runtime/style";

/** Most points drawn (every one of them; the rows after it are left out and the visual says so). */
export const MAX_POINTS = 10000;
/** Most legend groups with their own colour (later groups take the last one's). */
export const MAX_GROUPS = 12;
/** Most point labels. */
export const MAX_LABELS = 50;
/** Most rows a density cell selects. */
export const MAX_CELL_ROWS = 1000;
/** Bins of a marginal histogram. */
const BINS = 20;
/** The side of a marginal strip. */
const STRIP = 40;

interface Pt {
    row: number;
    x: number;
    y: number;
    s: number | null;
    g: number;
    /** Not left out by another visual's highlight. */
    lit: boolean;
}

function nameOf(input: any, k: number): string {
    const l: any[] = input && Array.isArray(input.names) ? input.names : [];
    return typeof l[k] === "string" ? Array.from(l[k] as string).slice(0, 80).join("") : "";
}

function tickText(t: number, d: Domain): string {
    return compact(t, tickDecimals(d.step));
}

/** The linear interpolation quantile `q` of sorted values. */
function quantile(s: number[], q: number): number {
    const pos = q * (s.length - 1);
    const lo = Math.floor(pos);
    const hi = Math.min(lo + 1, s.length - 1);
    return s[lo] + (s[hi] - s[lo]) * (pos - lo);
}

/** The five numbers of a box strip: whisker ends (the extreme values within 1.5 IQR), quartiles, median. */
function fiveOf(vals: number[]): number[] {
    const s = vals.slice().sort((a, b) => a - b);
    const q1 = quantile(s, 0.25);
    const med = quantile(s, 0.5);
    const q3 = quantile(s, 0.75);
    const iqr = q3 - q1;
    let lo = q1;
    let hi = q3;
    for (const v of s) {
        if (v >= q1 - 1.5 * iqr && v < lo) {
            lo = v;
        }
        if (v <= q3 + 1.5 * iqr && v > hi) {
            hi = v;
        }
    }
    return [lo, q1, med, q3, hi];
}

function binOf(v: number, d: Domain): number {
    const b = Math.floor(((v - d.lo) / (d.hi - d.lo)) * BINS);
    return Math.min(Math.max(b, 0), BINS - 1);
}

/** 0 for an even row of hexagons, 1 for an odd one. */
function odd(j: number): number {
    return j - 2 * Math.floor(j / 2);
}

/** Lays out `{rows: [[details, legend, x, y, size]], names, options, selected?}` in a `w`×`h` visual. */
export function layoutScatter(input: any, w: number, h: number): Out {
    const o = optionsOf(input);
    const k = lookOfInput(input);
    const fr = langOf(input) === "fr";
    const out: Out = { prims: [], hits: [] };
    const p = out.prims;
    const rows = rowsOf(input, MAX_POINTS);
    const total = input && Array.isArray(input.rows) ? input.rows.length : 0;
    const lit = highlighted(input);
    const groups: string[] = [];
    const groupIx = new Map<string, number>();
    const pts: Pt[] = [];
    for (let i = 0; i < rows.length; i++) {
        const x = cellNum(rows[i][2]);
        const y = cellNum(rows[i][3]);
        if (x === null || y === null) {
            continue;
        }
        const key = cellText(rows[i][1]);
        let g = groupIx.get(key);
        if (g === undefined) {
            if (groups.length < MAX_GROUPS) {
                g = groups.length;
                groupIx.set(key, g);
                groups.push(cellLabel(input, 1, rows[i][1]));
            } else {
                g = MAX_GROUPS - 1;
            }
        }
        pts.push({ row: i, x, y, s: cellNum(rows[i][4]), g, lit: !lit || hlNum(input, i) !== null });
    }
    if (pts.length === 0) {
        return out;
    }
    const size = optNum(o, "markerSize", 2, 20, 5);
    let fill = optBool(o, "markerFill", true);
    const border = optBool(o, "markerBorder", true);
    if (!fill && !border) {
        fill = true;
    }
    const fillOpacity = optNum(o, "fillOpacity", 10, 100, 70) / 100;
    const density = optEnum(o, "density", ["auto", "on", "off"], "auto");
    const threshold = optNum(o, "densityThreshold", 100, MAX_POINTS, 2000);
    const hexSize = optNum(o, "hexSize", 4, 40, 10);
    const marginals = optEnum(o, "marginals", ["none", "histogram", "box"], "none");
    const trend = optBool(o, "trendLine", false);
    const quadrants = optBool(o, "quadrants", false);
    const quadrantAt = optEnum(o, "quadrantAt", ["average", "value"], "average");
    const labelMode = optEnum(o, "labelMode", ["none", "top", "selected"], "none");
    const labelCount = Math.floor(optNum(o, "labelCount", 1, MAX_LABELS, 5));
    const showLegend = optBool(o, "showLegend", true);
    const fontSize = optNum(o, "fontSize", 8, 20, k.tick);
    let mx = 0;
    let my = 0;
    for (const q of pts) {
        mx += q.x;
        my += q.y;
    }
    mx /= pts.length;
    my /= pts.length;
    const qx = quadrantAt === "value" ? optNum(o, "quadrantX", -1e15, 1e15, 0) : mx;
    const qy = quadrantAt === "value" ? optNum(o, "quadrantY", -1e15, 1e15, 0) : my;
    const xs = pts.map((q) => q.x);
    const ys = pts.map((q) => q.y);
    if (quadrants) {
        xs.push(qx);
        ys.push(qy);
    }
    const xd = domain(xs, false, null, null, 5);
    const yd = domain(ys, false, null, null, 5);
    const colors = groups.map((g, i) => color(k, i));
    pushCard(p, k, w, h);
    const withLegend = showLegend && !(groups.length === 1 && groups[0] === "");
    let top = PAD;
    if (withLegend) {
        legend(p, k, groups, colors, PAD, PAD + 4, w - PAD);
        top = PAD + 20;
    }
    const yLabels = yd.ticks.map((t) => tickText(t, yd));
    let lw = 0;
    for (const l of yLabels) {
        lw = Math.max(lw, est(l, k.tick));
    }
    const x0 = PAD + Math.max(lw, 10) + 6;
    const yTitle = nameOf(input, 3);
    const xTitle = nameOf(input, 2);
    if (yTitle !== "") {
        const s = fit(yTitle, k.tick, w - 2 * PAD);
        if (s !== "") {
            p.push(text(PAD, top + 6, s, k.tick, k.text, "start"));
        }
        top += 16;
    } else {
        top += 6;
    }
    const strips = marginals !== "none";
    const y0 = top + (strips ? STRIP : 0);
    const x1 = w - PAD - (strips ? STRIP : 0);
    const y1 = h - PAD - 18 - (xTitle !== "" ? 14 : 0);
    if (x1 - x0 < 20 || y1 - y0 < 20) {
        return out;
    }
    const sx = (v: number) => x0 + ((v - xd.lo) / (xd.hi - xd.lo)) * (x1 - x0);
    const sy = (v: number) => y1 - ((v - yd.lo) / (yd.hi - yd.lo)) * (y1 - y0);
    for (let i = 0; i < yd.ticks.length; i++) {
        const yy = sy(yd.ticks[i]);
        p.push(line(x0, yy, x1, yy, k.grid, 1));
        p.push(text(x0 - 4, yy, yLabels[i], k.tick, k.text, "end"));
    }
    const xLabels = xd.ticks.map((t) => tickText(t, xd));
    let widest = 0;
    for (const l of xLabels) {
        widest = Math.max(widest, est(l, k.tick));
    }
    const room = xd.ticks.length > 1 ? (x1 - x0) / (xd.ticks.length - 1) : x1 - x0;
    const every = Math.max(1, Math.ceil((widest + 6) / Math.max(room, 1)));
    for (let i = 0; i < xd.ticks.length; i++) {
        const xx = sx(xd.ticks[i]);
        p.push(line(xx, y0, xx, y1, k.grid, 1));
        if (i % every === 0) {
            p.push(text(inside(xx, xLabels[i], k.tick, w), y1 + 9, xLabels[i], k.tick, k.text, "middle"));
        }
    }
    p.push(line(x0, y1, x1, y1, k.axis, 1));
    p.push(line(x0, y0, x0, y1, k.axis, 1));
    // the notes: the points left out past the cap, the density view
    const notes: string[] = [];
    if (total > MAX_POINTS) {
        notes.push(fr ? `${fmtFixed(MAX_POINTS, 0)} premiers points sur ${fmtFixed(total, 0)}` : `First ${fmtFixed(MAX_POINTS, 0)} of ${fmtFixed(total, 0)} points`);
    }
    const useDensity = density === "on" || (density === "auto" && pts.length > threshold);
    if (useDensity) {
        notes.push(fr ? `${fmtFixed(pts.length, 0)} points, vue de densité` : `${fmtFixed(pts.length, 0)} points, density view`);
    }
    if (notes.length > 0) {
        const s = fit(notes.join(" · "), k.tick, xTitle !== "" ? (x1 - x0) / 2 : x1 - x0);
        if (s !== "") {
            p.push(text(x0, h - PAD - 5, s, k.tick, k.text, "start"));
        }
    }
    if (xTitle !== "") {
        const s = fit(xTitle, k.tick, notes.length > 0 ? (x1 - x0) / 2 - 6 : x1 - PAD);
        if (s !== "") {
            p.push(text(x1, h - PAD - 5, s, k.tick, k.text, "end"));
        }
    }
    if (quadrants) {
        quadrantMarks(p, k, o, pts, qx, qy, xd, yd, sx, sy, x0, x1, y0, y1);
    }
    // the marks: points (largest behind) or density cells
    const radius: number[] = [];
    if (useDensity) {
        densityCells(out, k, pts, hexSize, sx, sy, x0, y0);
        for (let i = 0; i < pts.length; i++) {
            radius.push(0);
        }
    } else {
        let smin = Infinity;
        let smax = -Infinity;
        for (const q of pts) {
            if (q.s !== null) {
                smin = Math.min(smin, q.s);
                smax = Math.max(smax, q.s);
            }
        }
        const sized = smin <= smax;
        for (const q of pts) {
            let r = size;
            if (sized) {
                r = q.s === null ? size * 0.6 : smax > smin ? size * 0.6 + size * 1.8 * Math.sqrt((q.s - smin) / (smax - smin)) : size;
            }
            radius.push(r);
        }
        const order = pts.map((q, i) => i);
        order.sort((a, b) => radius[b] - radius[a] || a - b);
        for (const i of order) {
            const q = pts[i];
            const c = color(k, q.g);
            const m = circle(sx(q.x), sy(q.y), radius[i], c, fill ? fillOpacity : 0);
            if (border) {
                m.stroke = mix(c, "#000000", 0.3);
                m.sw = 1.25;
            }
            const hit: any = { row: q.row, prim: p.length, group: q.g };
            if (!q.lit) {
                hit.fade = true;
            }
            out.hits.push(hit);
            p.push(q.lit ? m : dim(m));
        }
    }
    if (strips) {
        stripMarks(p, k, marginals, pts, xd, yd, sx, sy, x0, x1, y0, y1);
    }
    // the point labels: the top N by y, or the selected rows
    let chosen: number[] = [];
    if (labelMode === "top") {
        chosen = pts.map((q, i) => i);
        chosen.sort((a, b) => pts[b].y - pts[a].y || a - b);
        chosen = chosen.slice(0, labelCount);
    } else if (labelMode === "selected" && input && Array.isArray(input.selected)) {
        const want: { [r: number]: boolean } = {};
        for (let i = 0; i < input.selected.length && i < 1000; i++) {
            const r = input.selected[i];
            if (typeof r === "number" && Number.isInteger(r)) {
                want[r] = true;
            }
        }
        for (let i = 0; i < pts.length && chosen.length < MAX_LABELS; i++) {
            if (want[pts[i].row]) {
                chosen.push(i);
            }
        }
    }
    const tags: Tag[] = [];
    for (const i of chosen) {
        const q = pts[i];
        let s = fit(cellLabel(input, 0, rows[q.row][0]), fontSize, 140);
        if (s === "") {
            continue;
        }
        const cx = sx(q.x);
        const r = radius[i];
        const ty = Math.min(Math.max(sy(q.y), y0 + fontSize / 2), y1 - fontSize / 2);
        let gx = cx + r + 3;
        let anchor = "start";
        if (gx + est(s, fontSize) > x1) {
            gx = cx - r - 3;
            anchor = "end";
            s = fit(s, fontSize, gx - 1);
            if (s === "") {
                continue;
            }
        }
        const tw = est(s, fontSize);
        const bx0 = anchor === "start" ? gx : gx - tw;
        tags.push({ x: gx, y: ty, s, anchor, box: [bx0, ty - fontSize / 2, bx0 + tw, ty + fontSize / 2] });
    }
    // the trend's R² text keeps clear of the point labels; a label it cannot avoid is left out
    let r2box: number[] | null = null;
    if (trend) {
        r2box = trendMarks(p, k, pts, mx, my, xd, yd, sx, sy, x0, x1, y0, y1, tags.map((t) => t.box));
    }
    for (const t of tags) {
        if (r2box !== null && meets(t.box, r2box)) {
            continue;
        }
        p.push(textW(t.x, t.y, t.s, fontSize, k.label, t.anchor, 0, valueWeight(k)));
    }
    return out;
}

/** A point's label: its anchor point, text, anchor and box (`[x0, y0, x1, y1]`). */
interface Tag {
    x: number;
    y: number;
    s: string;
    anchor: string;
    box: number[];
}

/** Whether two boxes (`[x0, y0, x1, y1]`) come within 1 px of each other. */
export function meets(a: number[], b: number[]): boolean {
    return a[0] < b[2] + 1 && b[0] < a[2] + 1 && a[1] < b[3] + 1 && b[1] < a[3] + 1;
}

function quadrantMarks(p: Prim[], k: Look, o: any, pts: Pt[], qx: number, qy: number, xd: Domain, yd: Domain, sx: (v: number) => number, sy: (v: number) => number, x0: number, x1: number, y0: number, y1: number): void {
    if (qx >= xd.lo && qx <= xd.hi) {
        dashes(p, [[sx(qx), y0], [sx(qx), y1]], k.axis, 1, 4, 3);
    }
    if (qy >= yd.lo && qy <= yd.hi) {
        dashes(p, [[x0, sy(qy)], [x1, sy(qy)]], k.axis, 1, 4, 3);
    }
    if (y1 - y0 < 40) {
        return;
    }
    // top right, top left, bottom left, bottom right
    const n = [0, 0, 0, 0];
    for (const q of pts) {
        const right = q.x >= qx;
        const up = q.y >= qy;
        n[up ? (right ? 0 : 1) : right ? 3 : 2]++;
    }
    const keys = ["labelTopRight", "labelTopLeft", "labelBottomLeft", "labelBottomRight"];
    const at: [number, number, string][] = [
        [x1 - 6, y0 + 8, "end"],
        [x0 + 6, y0 + 8, "start"],
        [x0 + 6, y1 - 8, "start"],
        [x1 - 6, y1 - 8, "end"],
    ];
    for (let i = 0; i < 4; i++) {
        const label = optText(o, keys[i], 60);
        const share = fmtFixed((n[i] / pts.length) * 100, 0) + "%";
        const s = fit(label !== "" ? `${label} · ${share}` : share, k.tick, (x1 - x0) / 2 - 8);
        if (s !== "") {
            p.push(textW(at[i][0], at[i][1], s, k.tick, k.text, at[i][2], 0, 600));
        }
    }
}

function densityCells(out: Out, k: Look, pts: Pt[], r: number, sx: (v: number) => number, sy: (v: number) => number, x0: number, y0: number): void {
    const p = out.prims;
    const dx = r * Math.sqrt(3);
    const dy = r * 1.5;
    const ix = new Map<string, number>();
    const cells: { i: number; j: number; rows: number[]; n: number; lit: boolean }[] = [];
    for (const q of pts) {
        const px = sx(q.x) - x0;
        const py = sy(q.y) - y0;
        // the nearest centre among the two nearest rows of hexagons
        const fj = py / dy;
        const j0 = Math.floor(fj + 0.5);
        const j1 = fj >= j0 ? j0 + 1 : j0 - 1;
        let bi = 0;
        let bj = j0;
        let best = Infinity;
        for (const j of [j0, j1]) {
            const i = Math.floor(px / dx - odd(j) / 2 + 0.5);
            const cx = (i + odd(j) / 2) * dx;
            const cy = j * dy;
            const d = (px - cx) * (px - cx) + (py - cy) * (py - cy);
            if (d < best) {
                best = d;
                bi = i;
                bj = j;
            }
        }
        const key = `${bi},${bj}`;
        let c = ix.get(key);
        if (c === undefined) {
            c = cells.length;
            ix.set(key, c);
            cells.push({ i: bi, j: bj, rows: [], n: 0, lit: false });
        }
        const cell = cells[c];
        cell.n++;
        if (cell.rows.length < MAX_CELL_ROWS) {
            cell.rows.push(q.row);
        }
        cell.lit = cell.lit || q.lit;
    }
    let most = 1;
    for (const c of cells) {
        most = Math.max(most, c.n);
    }
    const base = color(k, 0);
    const light = mix(base, k.surface, 0.82);
    const dark = mix(base, "#000000", 0.15);
    const rr = r * 0.95;
    const s = Math.sqrt(3) / 2;
    for (let b = 0; b < cells.length; b++) {
        const c = cells[b];
        const cx = x0 + (c.i + odd(c.j) / 2) * dx;
        const cy = y0 + c.j * dy;
        const shape = [
            [cx, cy - rr],
            [cx + rr * s, cy - rr / 2],
            [cx + rr * s, cy + rr / 2],
            [cx, cy + rr],
            [cx - rr * s, cy + rr / 2],
            [cx - rr * s, cy - rr / 2],
        ];
        const m = poly(shape, mix(light, dark, Math.sqrt(c.n / most)), 1);
        const hit: any = { row: c.rows[0], prim: p.length, cell: b, n: c.n, rows: c.rows };
        if (!c.lit) {
            hit.fade = true;
        }
        out.hits.push(hit);
        p.push(c.lit ? m : dim(m));
    }
}

function stripMarks(p: Prim[], k: Look, mode: string, pts: Pt[], xd: Domain, yd: Domain, sx: (v: number) => number, sy: (v: number) => number, x0: number, x1: number, y0: number, y1: number): void {
    const tone = mix(color(k, 0), k.surface, 0.35);
    const xs = pts.map((q) => q.x);
    const ys = pts.map((q) => q.y);
    if (mode === "histogram") {
        const cx: number[] = [];
        const cy: number[] = [];
        for (let b = 0; b < BINS; b++) {
            cx.push(0);
            cy.push(0);
        }
        for (const q of pts) {
            cx[binOf(q.x, xd)]++;
            cy[binOf(q.y, yd)]++;
        }
        let most = 1;
        for (let b = 0; b < BINS; b++) {
            most = Math.max(most, cx[b], cy[b]);
        }
        const tall = STRIP - 8;
        const bw = (x1 - x0) / BINS;
        const bh = (y1 - y0) / BINS;
        for (let b = 0; b < BINS; b++) {
            if (cx[b] > 0) {
                const hh = (tall * cx[b]) / most;
                p.push(bar(k, x0 + b * bw + 0.5, y0 - 4 - hh, Math.max(bw - 1, 0.5), hh, tone, "b"));
            }
            if (cy[b] > 0) {
                const ww = (tall * cy[b]) / most;
                p.push(bar(k, x1 + 4, y1 - (b + 1) * bh + 0.5, ww, Math.max(bh - 1, 0.5), tone, "l"));
            }
        }
        return;
    }
    const fx = fiveOf(xs);
    const fy = fiveOf(ys);
    const ink = color(k, 0);
    // along x, over the plot
    const cy = y0 - STRIP / 2;
    p.push(line(sx(fx[0]), cy, sx(fx[1]), cy, k.axis, 1));
    p.push(line(sx(fx[3]), cy, sx(fx[4]), cy, k.axis, 1));
    p.push(line(sx(fx[0]), cy - 4, sx(fx[0]), cy + 4, k.axis, 1));
    p.push(line(sx(fx[4]), cy - 4, sx(fx[4]), cy + 4, k.axis, 1));
    p.push(boxed(k, sx(fx[1]), cy - 7, sx(fx[3]) - sx(fx[1]), 14, ink));
    p.push(line(sx(fx[2]), cy - 7, sx(fx[2]), cy + 7, ink, 2));
    // along y, beside the plot
    const cx = x1 + STRIP / 2;
    p.push(line(cx, sy(fy[0]), cx, sy(fy[1]), k.axis, 1));
    p.push(line(cx, sy(fy[3]), cx, sy(fy[4]), k.axis, 1));
    p.push(line(cx - 4, sy(fy[0]), cx + 4, sy(fy[0]), k.axis, 1));
    p.push(line(cx - 4, sy(fy[4]), cx + 4, sy(fy[4]), k.axis, 1));
    p.push(boxed(k, cx - 7, sy(fy[3]), 14, sy(fy[1]) - sy(fy[3]), ink));
    p.push(line(cx - 7, sy(fy[2]), cx + 7, sy(fy[2]), ink, 2));
}

/** The least-squares line and its R² text, placed clear of the boxes `avoid` where it can; returns the text's box (null: no line). */
function trendMarks(p: Prim[], k: Look, pts: Pt[], mx: number, my: number, xd: Domain, yd: Domain, sx: (v: number) => number, sy: (v: number) => number, x0: number, x1: number, y0: number, y1: number, avoid: number[][]): number[] | null {
    let sxx = 0;
    let sxy = 0;
    let syy = 0;
    for (const q of pts) {
        sxx += (q.x - mx) * (q.x - mx);
        sxy += (q.x - mx) * (q.y - my);
        syy += (q.y - my) * (q.y - my);
    }
    if (pts.length < 2 || !(sxx > 0)) {
        return null;
    }
    const b = sxy / sxx;
    const a = my - b * mx;
    const xa = xd.lo;
    const xb = xd.hi;
    const ya = a + b * xa;
    const dyv = a + b * xb - ya;
    let t0 = 0;
    let t1 = 1;
    if (dyv !== 0) {
        const tl = (yd.lo - ya) / dyv;
        const th = (yd.hi - ya) / dyv;
        t0 = Math.max(t0, Math.min(tl, th));
        t1 = Math.min(t1, Math.max(tl, th));
    }
    if (!(t0 < t1)) {
        return null;
    }
    const ax = sx(xa + (xb - xa) * t0);
    const ay = sy(ya + dyv * t0);
    const bx = sx(xa + (xb - xa) * t1);
    const by = sy(ya + dyv * t1);
    p.push(line(ax, ay, bx, by, k.label, 1.5));
    const r2v = syy > 0 ? (sxy * sxy) / (sxx * syy) : 1;
    const s = `R² = ${fmtFixed(r2v, 2)}`;
    const tw = est(s, k.tick);
    // above the line's end, below it, a line further up and down, above and below its start,
    // then the plot's top left and bottom right corners: the first place clear of every point
    // label (`avoid`), else the first
    const far = 8 + k.tick + 3;
    const places = [
        [bx - 4, by - 8],
        [bx - 4, by + 8],
        [bx - 4, by - far],
        [bx - 4, by + far],
        [ax + 4 + tw, ay - 8],
        [ax + 4 + tw, ay + 8],
        [x0 + 4 + tw, y0 + 6],
        [x1 - 4, y1 - 6],
    ];
    const boxOf = (q: number[]) => {
        const tx = Math.max(q[0], x0 + tw + 2);
        const ty = Math.min(Math.max(q[1], y0 + 6), y1 - 6);
        return [tx, ty, tx - tw, ty - k.tick / 2, tx, ty + k.tick / 2];
    };
    let at = boxOf(places[0]);
    for (const q of places) {
        const c = boxOf(q);
        if (!avoid.some((b) => meets(b, c.slice(2)))) {
            at = c;
            break;
        }
    }
    p.push(textW(at[0], at[1], s, k.tick, k.label, "end", 0, 600));
    return at.slice(2);
}
