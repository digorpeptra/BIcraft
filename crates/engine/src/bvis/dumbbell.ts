// The BIcraft Dumbbell and slope chart's layout (docs/formats/bicraft-visuals.md "Dumbbell and
// slope chart"), the same as crates/render/src/bvis_dumbbell.rs.
"use strict";

import { Prim, clip, domain, est, fmtAuto, inside, line, text } from "./spec";
import { Data, Out, fade, PAD, col, fit, labelOf, labelWidth, legend, num, optColor, optFlag, path, rows, scale, xAxis } from "./common";
import { Look, color, dot, lineWidth, lookOf, minimal, mix, pushCard, stemWidth, valueLabel } from "./runtime/style";

export const MAX_ROWS = 200;
const LINE = "#A6A6A6";

interface Colors {
    start: string;
    end: string;
    line: string;
}

class Rows {
    constructor(public d: Data, public n: number) {}
    start(i: number): number | null {
        return num(col(this.d, "start", 0), i);
    }
    end(i: number): number | null {
        return num(col(this.d, "end", 0), i);
    }
    name(i: number): string {
        const c = col(this.d, "category", 0);
        return c ? clip(labelOf(c, i), 24) : "";
    }
    values(): number[] {
        const out: number[] = [];
        for (let i = 0; i < this.n; i++) {
            const a = this.start(i);
            const b = this.end(i);
            if (a !== null) {
                out.push(a);
            }
            if (b !== null) {
                out.push(b);
            }
        }
        return out;
    }
    field(role: string, def: string): string {
        const c = col(this.d, role, 0);
        return c && c.name !== "" ? c.name : def;
    }
}

/**
 * The start, end and line colours: the options', else the palette's first two and a line in
 * their mixed hue lightened towards the surface (Minimal: a grey line).
 */
export function colorsOf(o: any, k: Look): Colors {
    const start = optColor(o, "colors", "start", color(k, 0));
    const end = optColor(o, "colors", "end", color(k, 1));
    return { start, end, line: optColor(o, "colors", "line", minimal(k) ? LINE : mix(mix(start, end, 0.5), k.surface, 0.55)) };
}

/** Lays the dumbbells or slopes out (`draw`). Another visual's selection dims the rows it does not highlight (`fade`; docs/formats/bicraft-visuals.md "Highlights"). */
export function layout(d: Data, o: any, w: number, h: number, out?: Out): Prim[] {
    const prims = draw(d, o, w, h, out);
    if (out) {
        fade(d, out);
    }
    return prims;
}

/** Lays the chart out; `out` gets one unit per row (stats: start, end). */
function draw(d: Data, o: any, w: number, h: number, out?: Out): Prim[] {
    const prims: Prim[] = [];
    const slopeMode = optFlag(o, "style", "slope", false);
    const r = new Rows(d, Math.min(rows(d), MAX_ROWS));
    if (r.n === 0) {
        return prims;
    }
    const k = lookOf(o && o.style, o && o.$theme);
    const c = colorsOf(o, k);
    const labels = optFlag(o, "labels", "show", true);
    if (slopeMode) {
        slope(r, c, k, labels, w, h, prims, out);
    } else {
        dumbbell(r, c, k, labels, w, h, prims, out);
    }
    return prims;
}

/** A line joining two points: Minimal's plain line, else a path with round caps. */
function join(k: Look, x1: number, y1: number, x2: number, y2: number, stroke: string, width: number): Prim {
    return minimal(k) ? line(x1, y1, x2, y2, stroke, width) : path([[x1, y1], [x2, y2]], stroke, width);
}

function unit(r: Rows, i: number, marks: number[], out?: Out): void {
    if (out) {
        const a = r.start(i);
        const b = r.end(i);
        out.units.push({ rows: [i], stats: [a === null ? NaN : a, b === null ? NaN : b] });
        for (const p of marks) {
            out.hits.push({ unit: i, prim: p });
        }
    }
}

function dumbbell(r: Rows, c: Colors, k: Look, labels: boolean, w: number, h: number, prims: Prim[], out?: Out): void {
    const names: string[] = [];
    for (let i = 0; i < r.n; i++) {
        names.push(r.name(i));
    }
    const hasCat = col(r.d, "category", 0) !== null;
    const lw = hasCat ? labelWidth(names, k.name, (w - 2 * PAD) * 0.4) : 0;
    const top = PAD + 18;
    const bottom = h - PAD - 18;
    if (bottom - top < 8) {
        return;
    }
    const rh = (bottom - top) / r.n;
    if (rh < 3) {
        return;
    }
    const rad = Math.min(Math.max(rh * 0.3, 2), minimal(k) ? 6 : 7);
    const stem = Math.min(stemWidth(k), rad * 1.2);
    const gap = rad + 4;
    const padX = k.pills ? 5 : 0;
    // each row's smaller and larger value (one alone counts as the larger)
    const ends = (i: number): (number | null)[] => {
        const a = r.start(i);
        const b = r.end(i);
        if (a !== null && b !== null) {
            return a > b ? [b, a] : [a, b];
        }
        return [null, a !== null ? a : b];
    };
    const width = (v: number) => est(fmtAuto(v), k.value) + 2 * padX;
    // the margins beside the scale: room for the widest smaller value's label on the left and
    // the widest larger one's on the right (at least the look's margin, at most 30% of the plot)
    let wLo = 0;
    let wHi = 0;
    if (labels) {
        for (let i = 0; i < r.n; i++) {
            const [lo, hi] = ends(i);
            if (lo !== null) {
                wLo = Math.max(wLo, width(lo));
            }
            if (hi !== null) {
                wHi = Math.max(wHi, width(hi));
            }
        }
    }
    const base = 28 + (k.value - 10) * 3 + (k.pills ? 10 : 0);
    const room = (w - 2 * PAD - lw) * 0.3;
    const margin = (wd: number) => (labels ? Math.max(base, Math.min(gap + wd + 2, room)) : 0);
    const left = PAD + lw + margin(wLo);
    const right = w - PAD - margin(wHi);
    if (right - left < 8) {
        return;
    }
    pushCard(prims, k, w, h);
    const dom = domain(r.values(), false, null, null, 5);
    const xs = (v: number) => scale(dom, left, right, v);
    for (const t of dom.ticks) {
        prims.push(line(xs(t), top, xs(t), bottom, k.grid, 1));
    }
    // the value labels: the smaller value on the left of its dot, the larger on the right (on
    // the other side where a margin capped at 30% leaves it no room, left out where neither side
    // has room), moved apart where they would overlap (`spreadBoxes`, kept above the axis),
    // those that cannot fit in the plot left out (`fitInside`)
    const loX = PAD + lw;
    const hiX = w - 2;
    const tags: Tag[] = [];
    if (labels) {
        for (let i = 0; i < r.n; i++) {
            const y = top + rh * (i + 0.5);
            const [lo, hi] = ends(i);
            const sides: [number | null, string][] = [[lo, "w"], [hi, "e"]];
            for (const [v, want] of sides) {
                if (v === null) {
                    continue;
                }
                const x = xs(v);
                const wd = width(v);
                const fitsW = x - gap - wd >= loX;
                const fitsE = x + gap + wd <= hiX;
                let side = want;
                if (side === "w" && !fitsW && fitsE) {
                    side = "e";
                } else if (side === "e" && !fitsE && fitsW) {
                    side = "w";
                }
                if ((side === "w" && !fitsW) || (side === "e" && !fitsE)) {
                    continue;
                }
                tags.push({ row: i, x, y, s: fmtAuto(v), side });
            }
        }
    }
    const spans = tags.map((t) => {
        const wd = est(t.s, k.value) + 2 * padX;
        return t.side === "w" ? [t.x - gap - wd, t.x - gap] : [t.x + gap, t.x + gap + wd];
    });
    const ys = tags.map((t) => t.y);
    const gapY = k.value + 1 + (k.pills ? 6 : 0);
    const lo = Math.min(top + k.value / 2, top + rh / 2);
    const at = fitInside(ys, lo, (keep) =>
        spreadBoxes(
            keep.map((i) => ys[i]),
            keep.map((i) => spans[i]),
            gapY,
            bottom - k.value / 2
        )
    );
    for (let i = 0; i < r.n; i++) {
        const y = top + rh * (i + 0.5);
        const a = r.start(i);
        const b = r.end(i);
        const marks: number[] = [];
        const dots: number[] = [];
        if (a !== null && b !== null) {
            marks.push(prims.length);
            prims.push(join(k, xs(a), y, xs(b), y, c.line, stem));
        }
        if (a !== null) {
            dots.push(prims.length);
            prims.push(dot(k, xs(a), y, rad, c.start, 1));
        }
        if (b !== null) {
            dots.push(prims.length);
            prims.push(dot(k, xs(b), y, rad, c.end, 1));
        }
        for (let j = 0; j < tags.length; j++) {
            const t = tags[j];
            if (t.row !== i || at[j] === null) {
                continue;
            }
            const ty = at[j] as number;
            if (Math.abs(ty - y) > 1) {
                marks.push(prims.length);
                prims.push(leader(k, t.x, y, rad, t.side === "w" ? t.x - gap : t.x + gap, ty));
            }
            marks.push(valueLabel(prims, k, t.x, ty, t.s, t.side, gap, k.value));
        }
        if (hasCat) {
            prims.push(text(PAD + lw - 6, y, fit(names[i], lw - 6, k.name), k.name, k.label, "end", 0));
        }
        // a dot first: it takes the keyboard focus
        unit(r, i, dots.concat(marks), out);
    }
    xAxis(prims, k, dom, left, right, bottom, w);
    legend(prims, k, [[r.field("start", "Start"), c.start], [r.field("end", "End"), c.end]], w);
}

/** A dumbbell's value label: its row, its dot's centre, its text and its side (`w` or `e`). */
interface Tag {
    row: number;
    x: number;
    y: number;
    s: string;
    side: string;
}

/** A thin line from the edge of the dot at (`cx`, `cy`) of radius `rad` to a label moved away from it, at (`x`, `y`). */
export function leader(k: Look, cx: number, cy: number, rad: number, x: number, y: number): Prim {
    const dx = x - cx;
    const dy = y - cy;
    const len = Math.sqrt(dx * dx + dy * dy);
    const f = len > rad ? rad / len : 0;
    return line(cx + dx * f, cy + dy * f, x, y, k.axis, 0.75);
}

/**
 * Label positions `ys` (any order) moved apart as `spread` moves them, but only between labels
 * whose horizontal spans (`spans`, `[from, to]`) come within 2 px of each other.
 */
export function spreadBoxes(ys: number[], spans: number[][], gap: number, hi: number): number[] {
    const order = ys.map((y, i) => i);
    order.sort((a, b) => ys[a] - ys[b] || a - b);
    const near = (a: number, b: number) => spans[a][0] < spans[b][1] + 2 && spans[b][0] < spans[a][1] + 2;
    const out = ys.slice();
    for (let p = 0; p < order.length; p++) {
        const i = order[p];
        for (let q = 0; q < p; q++) {
            if (near(order[q], i)) {
                out[i] = Math.max(out[i], out[order[q]] + gap);
            }
        }
    }
    for (let p = order.length - 1; p >= 0; p--) {
        const i = order[p];
        out[i] = Math.min(out[i], hi);
        for (let q = p + 1; q < order.length; q++) {
            if (near(order[q], i)) {
                out[i] = Math.min(out[i], out[order[q]] - gap);
            }
        }
    }
    return out;
}

/**
 * Label positions `ys` (any order) placed by `place` (given the indices of the labels kept, it
 * returns their positions in that order), keeping only those it leaves at `lo` or below: a label
 * placed above `lo` is left out (null) and the others placed again, until all of them fit.
 */
export function fitInside(ys: number[], lo: number, place: (keep: number[]) => number[]): (number | null)[] {
    let keep = ys.map((y, i) => i);
    // each round leaves one label out at least, so it ends with every label kept placed
    for (let round = 0; round <= ys.length; round++) {
        const at = place(keep);
        const fits = (p: number) => p < at.length && at[p] >= lo - 1e-9;
        if (keep.every((i, p) => fits(p))) {
            const out: (number | null)[] = ys.map(() => null);
            keep.forEach((i, p) => {
                out[i] = at[p];
            });
            return out;
        }
        keep = keep.filter((i, p) => fits(p));
    }
    return ys.map(() => null);
}

/**
 * Label positions `ys` (any order) moved apart to at least `gap` in their order from the top
 * (ties in list order), pushed down, then up where they would pass `hi`.
 */
export function spread(ys: number[], gap: number, hi: number): number[] {
    const order = ys.map((y, i) => i);
    order.sort((a, b) => ys[a] - ys[b] || a - b);
    const out = ys.slice();
    let prev = -Infinity;
    for (const i of order) {
        out[i] = Math.max(out[i], prev + gap);
        prev = out[i];
    }
    let next = hi + gap;
    for (let j = order.length - 1; j >= 0; j--) {
        const i = order[j];
        out[i] = Math.min(out[i], next - gap);
        next = out[i];
    }
    return out;
}

function slope(r: Rows, c: Colors, k: Look, labels: boolean, w: number, h: number, prims: Prim[], out?: Out): void {
    const leftText = (i: number) => {
        const v = r.start(i);
        return v === null ? null : `${r.name(i)} ${fmtAuto(v)}`;
    };
    const rightText = (i: number) => {
        const v = r.end(i);
        return v === null ? null : `${fmtAuto(v)} ${r.name(i)}`;
    };
    let lw = 0;
    if (labels) {
        const texts: string[] = [];
        for (let i = 0; i < r.n; i++) {
            for (const s of [leftText(i), rightText(i)]) {
                if (s !== null) {
                    texts.push(s);
                }
            }
        }
        lw = labelWidth(texts, k.tick, (w - 2 * PAD) * 0.3);
    }
    const x0 = PAD + Math.max(lw, 4);
    const x1 = w - PAD - Math.max(lw, 4);
    const top = PAD + 24;
    const bottom = h - PAD - 6;
    if (x1 - x0 < 8 || bottom - top < 8) {
        return;
    }
    pushCard(prims, k, w, h);
    const dom = domain(r.values(), false, null, null, 5);
    const ys = (v: number) => scale(dom, bottom, top, v);
    // each side's labels moved apart so they do not overlap
    const at: (number | null)[][] = [[], []];
    for (let side = 0; side < 2; side++) {
        const want: number[] = [];
        const which: number[] = [];
        for (let i = 0; i < r.n; i++) {
            const v = side === 0 ? r.start(i) : r.end(i);
            at[side].push(null);
            if (labels && v !== null) {
                want.push(ys(v));
                which.push(i);
            }
        }
        // those that cannot fit below the axes' top are left out
        const got = fitInside(want, top, (keep) => spread(keep.map((i) => want[i]), k.tick + 1, bottom));
        for (let j = 0; j < which.length; j++) {
            at[side][which[j]] = got[j];
        }
    }
    const heads: [number, string, string][] = [[x0, "start", "Start"], [x1, "end", "End"]];
    for (const [x, role, def] of heads) {
        const name = clip(r.field(role, def), 30);
        prims.push(text(inside(x, name, k.name, w), PAD + 7, name, k.name, k.text, "middle", 0));
        prims.push(line(x, top, x, bottom, k.axis, 1));
    }
    const dotR = minimal(k) ? 4 : 4.5;
    for (let i = 0; i < r.n; i++) {
        const a = r.start(i);
        const b = r.end(i);
        const marks: number[] = [];
        const dots: number[] = [];
        if (a !== null && b !== null) {
            marks.push(prims.length);
            prims.push(join(k, x0, ys(a), x1, ys(b), c.line, lineWidth(k)));
        }
        if (a !== null) {
            dots.push(prims.length);
            prims.push(dot(k, x0, ys(a), dotR, c.start, 1));
            const s = leftText(i);
            const ly = at[0][i];
            if (labels && s !== null && ly !== null) {
                if (Math.abs(ly - ys(a)) > 1) {
                    marks.push(prims.length);
                    prims.push(leader(k, x0, ys(a), dotR, x0 - 8, ly));
                }
                marks.push(prims.length);
                prims.push(text(x0 - 8, ly, fit(s, lw - 8, k.tick), k.tick, k.label, "end", 0));
            }
        }
        if (b !== null) {
            dots.push(prims.length);
            prims.push(dot(k, x1, ys(b), dotR, c.end, 1));
            const s = rightText(i);
            const ry = at[1][i];
            if (labels && s !== null && ry !== null) {
                if (Math.abs(ry - ys(b)) > 1) {
                    marks.push(prims.length);
                    prims.push(leader(k, x1, ys(b), dotR, x1 + 8, ry));
                }
                marks.push(prims.length);
                prims.push(text(x1 + 8, ry, fit(s, lw - 8, k.tick), k.tick, k.label, "start", 0));
            }
        }
        unit(r, i, dots.concat(marks), out);
    }
}
