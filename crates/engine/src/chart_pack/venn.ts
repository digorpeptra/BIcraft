// BIcraft Venn (docs/formats/chart-pack.md "Venn diagram"): two or three sets read from an item
// and set membership table, drawn as fixed overlapping circles (the layout does not depend on
// the counts, so it never moves; two sets can be drawn area-proportional) with the count of
// items in each region. The TypeScript twin of
// crates/render/src/chart_pack/venn.rs.
"use strict";

import { Out, PAD, cellLabel, cellText, fmtFixed, legend, lookOfInput, optBool, optColor, optNum, optionsOf, rowsOf, text } from "./kit";
import { blob, color, pushCard, valueLabel } from "./runtime/style";

/** Most membership rows read. */
export const MAX_ROWS = 5000;
/** Most distinct sets counted (three are drawn). */
export const MAX_SETS = 50;

interface VennSet {
    name: string;
    first: number;
    items: Set<string>;
}

/** The area of the overlap of two circles of radii `a` and `b` whose centres are `d` apart. */
function lens(a: number, b: number, d: number): number {
    const ca = Math.min(Math.max((d * d + a * a - b * b) / (2 * d * a), -1), 1);
    const cb = Math.min(Math.max((d * d + b * b - a * a) / (2 * d * b), -1), 1);
    const k = Math.max((-d + a + b) * (d + a - b) * (d - a + b) * (d + a + b), 0);
    return a * a * Math.acos(ca) + b * b * Math.acos(cb) - 0.5 * Math.sqrt(k);
}

/**
 * The distance between the centres of two circles of radii `a` and `b` whose overlap has the
 * area π·`ab` (60 halvings): touching when `ab` is 0, one inside the other when it is the
 * smaller circle's whole area.
 */
export function apart(a: number, b: number, ab: number): number {
    let lo = Math.abs(a - b);
    let hi = a + b;
    if (ab <= 0) {
        return hi;
    }
    if (ab >= Math.min(a * a, b * b)) {
        return lo;
    }
    const target = Math.PI * ab;
    for (let i = 0; i < 60; i++) {
        const d = (lo + hi) / 2;
        if (lens(a, b, d) > target) {
            lo = d;
        } else {
            hi = d;
        }
    }
    return (lo + hi) / 2;
}

/** Lays out `{rows: [[item, set]], options, lang}` in a `w`×`h` visual. */
export function layoutVenn(input: any, w: number, h: number): Out {
    const o = optionsOf(input);
    const lk = lookOfInput(input);
    const colors = [optColor(o, "color1", color(lk, 0)), optColor(o, "color2", color(lk, 1)), optColor(o, "color3", color(lk, 2))];
    const showCounts = optBool(o, "showCounts", true);
    const proportional = optBool(o, "proportional", false);
    const size = optNum(o, "fontSize", 8, 24, 12 + lk.value - 10);
    const fr = input && input.lang === "fr";
    const out: Out = { prims: [], hits: [] };
    const sets: VennSet[] = [];
    const byName = new Map<string, number>();
    const rows = rowsOf(input, MAX_ROWS);
    for (let i = 0; i < rows.length; i++) {
        const item = cellText(rows[i][0]);
        const name = cellText(rows[i][1]);
        if (item === "" || name === "") {
            continue;
        }
        let k = byName.get(name);
        if (k === undefined) {
            if (sets.length >= MAX_SETS) {
                continue;
            }
            k = sets.length;
            byName.set(name, k);
            sets.push({ name: cellLabel(input, 1, rows[i][1]), first: i, items: new Set<string>() });
        }
        sets[k].items.add(item);
    }
    if (sets.length === 0) {
        return out;
    }
    // the three largest sets (ties: the first seen), in the order they were seen
    const ranked = sets.map((s, i) => i);
    ranked.sort((a, b) => sets[b].items.size - sets[a].items.size || a - b);
    const drawn = ranked.slice(0, 3).sort((a, b) => a - b);
    const more = sets.length - drawn.length;
    const k = drawn.length;
    const masks = new Map<string, number>();
    for (let j = 0; j < k; j++) {
        sets[drawn[j]].items.forEach((item) => {
            masks.set(item, (masks.get(item) || 0) | (1 << j));
        });
    }
    const counts = [0, 0, 0, 0, 0, 0, 0, 0];
    masks.forEach((m) => {
        counts[m] += 1;
    });
    const p = out.prims;
    pushCard(p, lk, w, h);
    legend(p, lk, drawn.map((i) => sets[i].name), colors, PAD, PAD + 4, w - PAD);
    if (more > 0) {
        const note = fr ? (more === 1 ? "1 autre ensemble non dessiné" : `${more} autres ensembles non dessinés`) : more === 1 ? "1 more set not drawn" : `${more} more sets not drawn`;
        p.push(text(PAD, h - PAD - 6, note, lk.tick, lk.text, "start"));
    }
    const ax = PAD;
    const aw = w - 2 * PAD;
    const ay = PAD + 22;
    const ah = h - ay - PAD - (more > 0 ? 14 : 0);
    if (aw < 16 || ah < 16) {
        return out;
    }
    const cx = ax + aw / 2;
    const cy = ay + ah / 2;
    const s3 = Math.sqrt(3);
    const rs: number[] = [];
    const cs: number[][] = [];
    // region label points by mask (bit j: drawn set j)
    const at: number[][] = [[], [], [], [], [], [], [], []];
    if (k === 1) {
        rs.push(Math.min(aw, ah) / 2);
        cs.push([cx, cy]);
        at[1] = [cx, cy];
    } else if (k === 2 && proportional) {
        // areas as the counts: radii √n, the overlap's area the shared items', scaled to fit
        const a = Math.sqrt(sets[drawn[0]].items.size);
        const b = Math.sqrt(sets[drawn[1]].items.size);
        const d = apart(a, b, counts[3]);
        const lo = Math.min(-a, d - b);
        const hi = Math.max(a, d + b);
        const f = Math.min(aw / (hi - lo), ah / (2 * Math.max(a, b)));
        const xa = cx - ((lo + hi) / 2) * f;
        const xb = xa + d * f;
        rs.push(a * f, b * f);
        cs.push([xa, cy], [xb, cy]);
        const al = xa - a * f;
        const ar = xa + a * f;
        const bl = xb - b * f;
        const br = xb + b * f;
        at[1] = [(al + Math.max(al, Math.min(bl, ar))) / 2, cy];
        at[2] = [(br + Math.min(br, Math.max(ar, bl))) / 2, cy];
        at[3] = [(Math.max(al, bl) + Math.min(ar, br)) / 2, cy];
    } else if (k === 2) {
        const r = Math.min(aw / 3, ah / 2);
        rs.push(r, r);
        cs.push([cx - r / 2, cy], [cx + r / 2, cy]);
        at[1] = [cx - r, cy];
        at[2] = [cx + r, cy];
        at[3] = [cx, cy];
    } else {
        const r = Math.min(aw / 3, ah / (2 + s3 / 2));
        rs.push(r, r, r);
        const gy = cy - (r * s3) / 12;
        cs.push([cx - r / 2, gy - r / (2 * s3)], [cx + r / 2, gy - r / (2 * s3)], [cx, gy + r / s3]);
        for (let j = 0; j < 3; j++) {
            at[1 << j] = [cs[j][0] + (cs[j][0] - cx) * s3 * 0.45, cs[j][1] + (cs[j][1] - gy) * s3 * 0.45];
        }
        const pairs = [[0, 1], [0, 2], [1, 2]];
        for (const pr of pairs) {
            const mx = (cs[pr[0]][0] + cs[pr[1]][0]) / 2;
            const my = (cs[pr[0]][1] + cs[pr[1]][1]) / 2;
            at[(1 << pr[0]) | (1 << pr[1])] = [mx + (mx - cx) * s3 * 0.5, my + (my - gy) * s3 * 0.5];
        }
        at[7] = [cx, gy];
    }
    for (let j = 0; j < k; j++) {
        out.hits.push({ row: sets[drawn[j]].first, prim: p.length, set: j, n: sets[drawn[j]].items.size });
        p.push(blob(lk, cs[j][0], cs[j][1], rs[j], colors[j], 0.35));
    }
    if (showCounts) {
        for (let m = 1; m < 8; m++) {
            if (at[m].length === 2) {
                const t = valueLabel(p, lk, at[m][0], at[m][1], fmtFixed(counts[m], 0), "c", 0, size);
                out.hits.push({ row: -1, prim: t, region: m, n: counts[m] });
            }
        }
    }
    return out;
}
