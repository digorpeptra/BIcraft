// BIcraft Gauge / KPI+ (docs/formats/chart-pack.md "Gauge / KPI+"): one value against its range
// (minimum, up to three thresholds, maximum, each from a role or the format pane), the range
// drawn as free colour bands, a target marker and the variance from the target or the previous
// value, in three styles: a radial arc, a linear bullet, or a KPI card. With a trend field the
// value is the latest point's and a sparkline of every point runs along the foot. Values are
// written with their field's format string. Drawn with BIcraft's look
// (docs/formats/visual-look.md). The TypeScript twin of crates/render/src/chart_pack/gauge.rs.
"use strict";

import { Out, PAD, Prim, cellLabel, cellNum, est, fit, fmtAuto, fmtFixed, hlNum, highlighted, line, lookOfInput, optBool, optColor, optionsOf, poly, rowsOf, text } from "./kit";
import { domain, inside } from "./spec";
import { Look, band, bar, color, dim, dot, lineWidth, minimal, mix, part, pushCard, textW } from "./runtime/style";
import { formatNumber } from "./runtime/format";

/** Most rows (trend points) read. */
export const MAX_ROWS = 1000;
/** Most sparkline points drawn as dots (each a data point); beyond, only the latest. */
export const MAX_DOTS = 60;
/** Values beyond this size are not read. */
export const MAX_ABS = 1e15;
/** The band colours' bases, worst to best (drawn softened unless a band colour is set). */
export const BAND_BASES = ["#D64545", "#E0A100", "#2E9E5B", "#1C7A45"];
/** The bases each band count takes, worst to best. */
const BAND_PICKS = [[0], [0, 2], [0, 1, 2], [0, 1, 2, 3]];
const GOOD = ["#137A3F", "#5BD08A"];
const BAD = ["#C0392B", "#FF7B6E"];

interface Pt {
    row: number;
    v: number;
    label: string;
}

function numOf(v: any): number | null {
    const n = cellNum(v);
    return n !== null && Math.abs(n) <= MAX_ABS ? n : null;
}

function optVal(o: any, k: string): number | null {
    return numOf(o[k]);
}

/** A section's text with the commas right after its last decimal placeholder moved before its decimal point. */
function scaleSection(sec: [string, boolean][]): string {
    const text = (r: [string, boolean][]) => r.map((x) => x[0]).join("");
    let d = -1;
    let l = -1;
    let sci = false;
    for (let i = 0; i < sec.length; i++) {
        const c = sec[i][0];
        if (!sec[i][1]) {
            continue;
        }
        if (c === "." && d < 0) {
            d = i;
        }
        if (c === "0" || c === "#") {
            l = i;
        }
        if (c === "E" || c === "e") {
            sci = true;
        }
    }
    let n = 0;
    while (l >= 0 && l + 1 + n < sec.length && sec[l + 1 + n][1] && sec[l + 1 + n][0] === ",") {
        n++;
    }
    if (d < 0 || l < 0 || sci || l < d || n === 0) {
        return text(sec);
    }
    return text(sec.slice(0, d)) + ",".repeat(n) + text(sec.slice(d, l + 1)) + text(sec.slice(l + 1 + n));
}

/**
 * Format string `f` with the commas right after a section's last decimal placeholder moved before its decimal point
 * (`$#,0.00,,"M"` → `$#,0,,.00"M"`): read as scaling commas, ÷1000 each, as before the point. Quoted and escaped text
 * and scientific sections stay as they are. The twin of gauge.rs `scale_commas`.
 */
export function scaleCommas(f: string): string {
    let sec: [string, boolean][] = [];
    let out = "";
    let quoted = false;
    let esc = false;
    for (const c of Array.from(f)) {
        if (esc || quoted) {
            quoted = quoted && c !== "\"";
            esc = false;
            sec.push([c, false]);
            continue;
        }
        if (c === "\"") {
            quoted = true;
        } else if (c === "\\") {
            esc = true;
        } else if (c === ";") {
            out += scaleSection(sec) + ";";
            sec = [];
            continue;
        }
        sec.push([c, c !== "\"" && c !== "\\"]);
    }
    return out + scaleSection(sec);
}

/** A value written with its field's whole format string (after scaleCommas); without one, as the pack writes numbers. */
function fmtNum(v: number, f: string, lang: string): string {
    const g = Array.from(f).slice(0, 200).join("");
    return g.trim() === "" ? fmtAuto(v) : formatNumber(v, scaleCommas(g.trim()), lang === "fr" ? "fr-FR" : "en-US");
}

export function signed(v: number, s: string): string {
    return v > 0 && s !== "" && s.charAt(0) !== "+" && s.charAt(0) !== "-" ? "+" + s : s;
}

export function pctText(p: number, lang?: string): string {
    const s = fmtFixed(p * 100, 1);
    return signed(p, String(lang || "").slice(0, 2).toLowerCase() === "fr" ? s.replace(".", ",") + " %" : s + "%");
}

/** sin and cos of `t` in [0, π] by their series (the same arithmetic in the Rust, so both drawers agree to the last bit). */
export function sinCos(t: number): number[] {
    let x = t;
    let flip = false;
    if (x > Math.PI / 2) {
        x = Math.PI - x;
        flip = true;
    }
    const x2 = x * x;
    const s = x * (1 - (x2 / 6) * (1 - (x2 / 20) * (1 - (x2 / 42) * (1 - (x2 / 72) * (1 - (x2 / 110) * (1 - (x2 / 156) * (1 - x2 / 210)))))));
    const c = 1 - (x2 / 2) * (1 - (x2 / 12) * (1 - (x2 / 30) * (1 - (x2 / 56) * (1 - (x2 / 90) * (1 - (x2 / 132) * (1 - x2 / 182))))));
    return [s, flip ? -c : c];
}

/** A ring sector of the half-circle gauge from fraction `f0` to `f1` (0 at the left end, 1 at the right), between radii `r0` and `r1`. */
function arc(cx: number, cy: number, r0: number, r1: number, f0: number, f1: number, fill: string): Prim {
    const segs = Math.max(2, Math.ceil(48 * (f1 - f0)));
    const pts: number[][] = [];
    for (let i = 0; i <= segs; i++) {
        const sc = sinCos(Math.PI * (1 - (f0 + ((f1 - f0) * i) / segs)));
        pts.push([cx + r1 * sc[1], cy - r1 * sc[0]]);
    }
    for (let i = segs; i >= 0; i--) {
        const sc = sinCos(Math.PI * (1 - (f0 + ((f1 - f0) * i) / segs)));
        pts.push([cx + r0 * sc[1], cy - r0 * sc[0]]);
    }
    return poly(pts, fill, 1);
}

/** What every style draws from. */
interface G {
    pts: Pt[];
    row: number;
    val: number;
    target: number | null;
    min: number;
    max: number;
    edges: number[];
    bands: string[];
    valueFill: string;
    targetFill: string;
    valText: string;
    minText: string;
    maxText: string;
    /** The variance line ("" without a target or previous value) and its ink. */
    varText: string;
    /** The variance line with less and less of it, for the arc's narrow middle: the whole line, then without its words, then the difference alone. */
    varTexts: string[];
    varInk: string;
    refText: string;
    ref: number | null;
    refKind: string;
    /** The highlighted part of the value (null: none). */
    hv: number | null;
    lit: boolean;
}

function frac(g: G, v: number): number {
    return Math.min(Math.max((v - g.min) / (g.max - g.min), 0), 1);
}

function prepare(input: any, k: Look): G | null {
    const o = optionsOf(input);
    const lang = input && input.lang === "fr" ? "fr" : "en";
    const rows = rowsOf(input, MAX_ROWS);
    const pts: Pt[] = [];
    for (let i = 0; i < rows.length; i++) {
        const v = numOf(rows[i][1]);
        if (v !== null) {
            pts.push({ row: i, v, label: cellLabel(input, 0, rows[i][0]) });
        }
    }
    if (pts.length === 0) {
        return null;
    }
    const cur = pts[pts.length - 1];
    const r = rows[cur.row];
    const val = cur.v;
    const at = (c: number, key: string): number | null => {
        const x = numOf(r[c]);
        return x !== null ? x : optVal(o, key);
    };
    const target = at(2, "target");
    const lo = at(3, "min");
    const min = lo !== null ? lo : 0;
    const given: number[] = [];
    for (let j = 0; j < 3; j++) {
        const t = at(5 + j, "threshold" + (j + 1));
        if (t !== null) {
            given.push(t);
        }
    }
    let max = at(4, "max");
    if (max === null || !(max > min)) {
        const vals = [min, val].concat(given);
        if (target !== null) {
            vals.push(target);
        }
        max = domain(vals, false, min, null, 4).hi;
        if (!(max > min)) {
            max = min + 1;
        }
    }
    const hi = max;
    const ts = given.filter((t) => t > min && t < hi).sort((a, b) => a - b);
    const edges = [min];
    for (const t of ts) {
        if (t !== edges[edges.length - 1]) {
            edges.push(t);
        }
    }
    edges.push(max);
    const nb = edges.length - 1;
    const lower = optBool(o, "lowerIsBetter", false);
    const picks = BAND_PICKS[Math.min(nb, 4) - 1];
    const bases: string[] = [];
    for (let j = 0; j < nb; j++) {
        bases.push(BAND_BASES[picks[lower ? nb - 1 - j : j]]);
    }
    const bands = bases.map((b, j) => optColor(o, "band" + (j + 1), nb === 1 ? k.empty : mix(b, k.surface, 0.62)));
    let valueFill = optColor(o, "valueFill", color(k, 0));
    if (optBool(o, "valueByBand", false) && nb > 1) {
        let j = 0;
        while (j < nb - 1 && val >= edges[j + 1]) {
            j++;
        }
        valueFill = typeof o["band" + (j + 1)] === "string" ? bands[j] : bases[j];
    }
    let prev = numOf(r[8]);
    if (prev === null && pts.length >= 2) {
        prev = pts[pts.length - 2].v;
    }
    const want = o.compareTo === "previous" ? "previous" : "target";
    let ref: number | null = null;
    let refKind = "";
    if (want === "previous" && prev !== null) {
        ref = prev;
        refKind = "previous";
    } else if (target !== null) {
        ref = target;
        refKind = "target";
    } else if (prev !== null) {
        ref = prev;
        refKind = "previous";
    }
    const f = typeof input.formats === "object" && Array.isArray(input.formats) && typeof input.formats[1] === "string" ? input.formats[1] : "";
    const fr = lang === "fr";
    let varText = "";
    let varInk = k.label;
    let refText = "";
    let varTexts: string[] = [];
    if (ref !== null) {
        const diff = val - ref;
        const diffText = signed(diff, fmtNum(diff, f, lang));
        varText = diffText;
        if (ref !== 0) {
            varText += " (" + pctText(diff / Math.abs(ref), lang) + ")";
        }
        const bare = varText;
        varText += refKind === "target" ? (fr ? " vs cible" : " vs target") : fr ? " vs précédent" : " vs previous";
        const good = lower ? diff <= 0 : diff >= 0;
        varInk = (good ? GOOD : BAD)[k.night ? 1 : 0];
        refText = (refKind === "target" ? (fr ? "Cible : " : "Target: ") : fr ? "Précédent : " : "Previous: ") + fmtNum(ref, f, lang);
        varTexts = [varText, bare, diffText];
    }
    const lit = highlighted(input);
    return {
        pts,
        row: cur.row,
        val,
        target,
        min,
        max,
        edges,
        bands,
        valueFill,
        targetFill: optColor(o, "targetFill", k.label),
        valText: fmtNum(val, f, lang),
        minText: fmtNum(min, f, lang),
        maxText: fmtNum(max, f, lang),
        varText,
        varTexts,
        varInk,
        refText,
        ref,
        refKind,
        hv: lit ? part(val, hlNum(input, cur.row)) : null,
        lit,
    };
}

function valueHit(g: G, prim: number): any {
    return { row: g.row, prim, part: "value", value: g.val, target: g.target, ref: g.ref, refKind: g.refKind, min: g.min, max: g.max };
}

/** The trend's sparkline in the box (`x0`, `y0`)–(`x1`, `y1`): a line through every point, each a dot (only the latest past MAX_DOTS points). */
function spark(out: Out, k: Look, g: G, input: any, x0: number, y0: number, x1: number, y1: number): void {
    const p = out.prims;
    const n = g.pts.length;
    let lo = Infinity;
    let hi = -Infinity;
    for (const q of g.pts) {
        lo = Math.min(lo, q.v);
        hi = Math.max(hi, q.v);
    }
    const xs = (i: number) => (n === 1 ? (x0 + x1) / 2 : x0 + ((x1 - x0) * i) / (n - 1));
    const ys = (v: number) => (hi > lo ? y1 - ((v - lo) / (hi - lo)) * (y1 - y0) : (y0 + y1) / 2);
    const lw = lineWidth(k);
    for (let i = 1; i < n; i++) {
        p.push(line(xs(i - 1), ys(g.pts[i - 1].v), xs(i), ys(g.pts[i].v), g.valueFill, lw));
    }
    for (let i = n <= MAX_DOTS ? 0 : n - 1; i < n; i++) {
        const q = g.pts[i];
        out.hits.push({ row: q.row, prim: p.length, part: "point", value: q.v, label: q.label });
        const d = dot(k, xs(i), ys(q.v), i === n - 1 ? 3.5 : 2.5, g.valueFill, 1);
        p.push(g.lit && part(q.v, hlNum(input, q.row)) === null ? dim(d) : d);
    }
}

function arcStyle(out: Out, k: Look, g: G, input: any, trend: boolean, w: number, h: number): void {
    const p = out.prims;
    const trendH = trend && h >= 120 ? Math.min(40, h * 0.22) : 0;
    const ah = h - 2 * PAD - (trendH > 0 ? trendH + 6 : 0) - (k.tick + 6);
    const R = Math.min((w - 2 * PAD) / 2 - 4, ah - 4);
    if (!(R >= 16)) {
        const vs = Math.min(Math.max(h * 0.3, 10), 28);
        const s = fit(g.valText, vs, w - 2 * PAD);
        if (s !== "") {
            out.hits.push(valueHit(g, p.length));
            p.push(textW(w / 2, h / 2, s, vs, k.label, "middle", 0, 700));
        }
        return;
    }
    const cx = w / 2;
    const cy = PAD + 4 + R;
    const thick = R * 0.26;
    for (let j = 0; j < g.bands.length; j++) {
        p.push(arc(cx, cy, R - thick, R, frac(g, g.edges[j]), frac(g, g.edges[j + 1]), g.bands[j]));
    }
    const fv = frac(g, g.val);
    const r0 = R - thick * 0.8;
    const r1 = R - thick * 0.2;
    if (fv > 0) {
        out.hits.push(valueHit(g, p.length));
        const a = arc(cx, cy, r0, r1, 0, fv, g.valueFill);
        p.push(g.lit ? dim(a) : a);
        if (g.hv !== null) {
            const fh = frac(g, g.hv);
            if (fh > 0) {
                out.hits.push(valueHit(g, p.length));
                p.push(arc(cx, cy, r0, r1, 0, fh, g.valueFill));
            }
        }
    }
    if (g.target !== null) {
        const sc = sinCos(Math.PI * (1 - frac(g, g.target)));
        const a = R - thick - 3;
        const b = R + 3;
        out.hits.push({ row: g.row, prim: p.length, part: "target", value: g.target });
        p.push(line(cx + a * sc[1], cy - a * sc[0], cx + b * sc[1], cy - b * sc[0], g.targetFill, minimal(k) ? 2 : 3));
    }
    const ly = cy + 4 + k.tick / 2;
    const minX = inside(cx - R + thick / 2, g.minText, k.tick, w);
    const maxX = inside(cx + R - thick / 2, g.maxText, k.tick, w);
    p.push(text(minX, ly, g.minText, k.tick, k.text, "middle"));
    p.push(text(maxX, ly, g.maxText, k.tick, k.text, "middle"));
    const inner = R - thick;
    const vs = Math.min(Math.max(R * 0.3, 12), 40);
    const vy = cy - inner * 0.35;
    const s = fit(g.valText, vs, 2 * inner - 8);
    if (s !== "") {
        out.hits.push(valueHit(g, p.length));
        p.push(textW(cx, vy, s, vs, k.label, "middle", 0, 700));
    }
    // the variance under the value: the most of it that fits whole inside the ring, and between
    // the minimum and the maximum when it reaches their line, at the name size, else the tick size
    const gap = Math.min(cx - minX - est(g.minText, k.tick) / 2, maxX - est(g.maxText, k.tick) / 2 - cx);
    const varY = (size: number) => vy + vs / 2 + 3 + size / 2;
    const room = (size: number) => (varY(size) + size / 2 > ly - k.tick / 2 ? Math.min(2 * inner - 8, 2 * gap - 8) : 2 * inner - 8);
    let picked = false;
    for (const t of g.varTexts) {
        for (const size of [k.name, k.tick]) {
            if (!picked && t !== "" && est(t, size) <= room(size)) {
                p.push(textW(cx, varY(size), t, size, g.varInk, "middle", 0, 600));
                picked = true;
            }
        }
    }
    if (trendH > 0) {
        spark(out, k, g, input, PAD + 4, h - PAD - trendH, w - PAD - 4, h - PAD - 4);
    }
}

function bulletStyle(out: Out, k: Look, g: G, input: any, trend: boolean, w: number, h: number): void {
    const p = out.prims;
    const vs = Math.min(Math.max(h * 0.2, 12), 30);
    const hy = PAD + vs / 2 + 2;
    const s = fit(g.valText, vs, (w - 2 * PAD) * 0.6);
    if (s !== "") {
        out.hits.push(valueHit(g, p.length));
        p.push(textW(PAD + 2, hy, s, vs, k.label, "start", 0, 700));
    }
    const room = w - 2 * PAD - est(s, vs) - 12;
    const v = fit(g.varText, k.name, room);
    if (v !== "") {
        p.push(textW(w - PAD - 2, hy, v, k.name, g.varInk, "end", 0, 600));
    }
    const trendH = trend && h >= 120 ? Math.min(36, h * 0.22) : 0;
    const by0 = PAD + vs + 10;
    const avail = h - by0 - PAD - (k.tick + 8) - (trendH > 0 ? trendH + 6 : 0);
    const bh = Math.min(Math.max(avail, 0), 40);
    const bx0 = PAD + 2;
    const bx1 = w - PAD - 2;
    const bw = bx1 - bx0;
    if (bh >= 6 && bw >= 20) {
        const xOf = (x: number) => bx0 + frac(g, x) * bw;
        const nb = g.bands.length;
        for (let j = 0; j < nb; j++) {
            const a = xOf(g.edges[j]);
            const sq = nb === 1 ? "" : j === 0 ? "r" : j === nb - 1 ? "l" : "x";
            p.push(band(k, a, by0, xOf(g.edges[j + 1]) - a, bh, g.bands[j], sq));
        }
        const vh = bh * 0.42;
        const vy = by0 + (bh - vh) / 2;
        const fv = frac(g, g.val);
        if (fv > 0) {
            out.hits.push(valueHit(g, p.length));
            const m = bar(k, bx0, vy, fv * bw, vh, g.valueFill, "l");
            p.push(g.lit ? dim(m) : m);
            if (g.hv !== null && frac(g, g.hv) > 0) {
                out.hits.push(valueHit(g, p.length));
                p.push(bar(k, bx0, vy, frac(g, g.hv) * bw, vh, g.valueFill, "l"));
            }
        }
        if (g.target !== null) {
            const tx = xOf(g.target);
            out.hits.push({ row: g.row, prim: p.length, part: "target", value: g.target });
            p.push(line(tx, by0 - 3, tx, by0 + bh + 3, g.targetFill, minimal(k) ? 2 : 3));
        }
        const ly = by0 + bh + 4 + k.tick / 2;
        p.push(text(bx0, ly, fit(g.minText, k.tick, bw / 2 - 4), k.tick, k.text, "start"));
        p.push(text(bx1, ly, fit(g.maxText, k.tick, bw / 2 - 4), k.tick, k.text, "end"));
    }
    if (trendH > 0) {
        spark(out, k, g, input, PAD + 4, h - PAD - trendH, w - PAD - 4, h - PAD - 4);
    }
}

function kpiStyle(out: Out, k: Look, g: G, input: any, trend: boolean, w: number, h: number): void {
    const p = out.prims;
    let trendH = trend && h >= 80 ? Math.min(h * 0.3, 60) : 0;
    const areaOf = (t: number) => h - 2 * PAD - (t > 0 ? t + 6 : 0);
    const vs = Math.min(Math.max(Math.min(areaOf(trendH) * 0.42, w * 0.16), 12), 48);
    let showVar = g.varText !== "";
    let showRef = g.refText !== "";
    const blockOf = (v: boolean, r: boolean) => vs + (v ? k.name + 6 : 0) + (r ? k.tick + 4 : 0);
    // the text first: the sparkline takes what it leaves (none under 16 px), then the reference
    // and the variance lines give way when even that is too little
    if (trendH > 0 && blockOf(showVar, showRef) > areaOf(trendH)) {
        trendH = h - 2 * PAD - 6 - blockOf(showVar, showRef);
        if (trendH < 16) {
            trendH = 0;
        }
    }
    const area = areaOf(trendH);
    if (blockOf(showVar, showRef) > area) {
        showRef = false;
    }
    if (blockOf(showVar, showRef) > area) {
        showVar = false;
    }
    const block = blockOf(showVar, showRef);
    let y = PAD + Math.max((area - block) / 2, 0);
    const cx = w / 2;
    const s = fit(g.valText, vs, w - 2 * PAD);
    if (s !== "") {
        out.hits.push(valueHit(g, p.length));
        p.push(textW(cx, y + vs / 2, s, vs, g.lit && g.hv === null ? mix(k.label, k.surface, 0.6) : k.label, "middle", 0, 700));
    }
    y += vs;
    // the most of the variance that fits whole, else the whole line shortened
    const whole = g.varTexts.filter((t) => t !== "" && est(t, k.name) <= w - 2 * PAD);
    const v = whole.length > 0 ? whole[0] : fit(g.varText, k.name, w - 2 * PAD);
    if (showVar && v !== "") {
        p.push(textW(cx, y + 6 + k.name / 2, v, k.name, g.varInk, "middle", 0, 600));
        y += k.name + 6;
    }
    const r = fit(g.refText, k.tick, w - 2 * PAD);
    if (showRef && r !== "") {
        p.push(text(cx, y + 4 + k.tick / 2, r, k.tick, k.text, "middle"));
    }
    if (trendH > 0) {
        spark(out, k, g, input, PAD + 6, h - PAD - trendH, w - PAD - 6, h - PAD - 4);
    }
}

/** Lays out `{rows: [[trend, value, target, min, max, threshold1, threshold2, threshold3, previous]], formats, options}` in a `w`×`h` visual. */
export function layoutGauge(input: any, w: number, h: number): Out {
    const o = optionsOf(input);
    const k = lookOfInput(input);
    const out: Out = { prims: [], hits: [] };
    const g = prepare(input, k);
    if (!g || !(w >= 24 && h >= 24)) {
        return out;
    }
    pushCard(out.prims, k, w, h);
    const trend = g.pts.length >= 2 && optBool(o, "showTrend", true);
    const style = o.gaugeStyle === "bullet" || o.gaugeStyle === "kpi" ? o.gaugeStyle : "arc";
    if (style === "bullet") {
        bulletStyle(out, k, g, input, trend, w, h);
    } else if (style === "kpi") {
        kpiStyle(out, k, g, input, trend, w, h);
    } else {
        arcStyle(out, k, g, input, trend, w, h);
    }
    return out;
}
