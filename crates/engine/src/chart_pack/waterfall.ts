// BIcraft Waterfall+ (docs/formats/chart-pack.md "Waterfall+"): a running total drawn step by
// step, each step a bar floating from the total before it to the total after it, with subtotal
// bars (after each group of the group role, after a row whose subtotal flag is set, or after
// every N steps) and a final total; vertical or horizontal; steps coloured by sign, by category or
// by a rule; connectors between bars; value and percent labels; a target drawn as a marker
// across the final bar with the variance of the total from it. Several measures in the value
// role become the steps themselves (each measure's total over the rows). Drawn with BIcraft's
// look (docs/formats/visual-look.md). The TypeScript twin of
// crates/render/src/chart_pack/waterfall.rs.
"use strict";

import { Out, PAD, cellLabel, cellNum, cellText, est, fit, fmtAuto, fmtFixed, highlighted, line, lookOfInput, optBool, optColor, optNum, optionsOf, rowsOf, text } from "./kit";
import { domain, tickDecimals } from "./spec";
import { Look, bar, color, dim, minimal, mix, part, pushCard, valueLabel } from "./runtime/style";
import { formatNumber } from "./runtime/format";

/** Most rows read. */
export const MAX_ROWS = 1000;
/** Most bars drawn (steps, subtotals and the total). */
export const MAX_STEPS = 200;
/** Most measures in the value role. */
export const MAX_MEASURES = 8;
/** Values beyond this size are not read (they would make the scale meaningless). */
export const MAX_ABS = 1e15;
/** The cells before the measures: category, group, subtotal flag, target. */
export const FIRST_MEASURE = 4;
export const INCREASE = "#1F9D55";
export const DECREASE = "#D64545";
export const RULE_ABOVE = "#1C8FCC";
export const RULE_BELOW = "#BF8700";

/** One bar: a step (`step`), a subtotal (`sub`) or the total (`total`). */
interface Item {
    kind: string;
    label: string;
    /** A step's change, a subtotal's or the total's value. */
    value: number;
    start: number;
    end: number;
    /** The row of a step (-1: none, several measures or a total). */
    row: number;
    measure: number;
    /** The step's index among the steps (its colour by category). */
    index: number;
    /** The highlighted part of a step's change (null: none). */
    hl: number | null;
}

/** A finite number of moderate size, else null. */
export function numOf(v: any): number | null {
    const n = cellNum(v);
    return n !== null && Math.abs(n) <= MAX_ABS ? n : null;
}

/** A number written with a column's format string (`f`), or as BIcraft writes numbers without one. */
export function fmtNum(v: number, f: string, lang: string): string {
    const g = Array.from(f).slice(0, 200).join("");
    return g.trim() === "" ? fmtAuto(v) : formatNumber(v, g, lang === "fr" ? "fr-FR" : "en-US");
}

/** `s` with a plus sign when `v` is above zero and the text has no sign of its own. */
export function signed(v: number, s: string): string {
    return v > 0 && s !== "" && s.charAt(0) !== "+" && s.charAt(0) !== "-" ? "+" + s : s;
}

/** A change as a signed percentage with one decimal. */
export function pctText(p: number, lang?: string): string {
    const s = fmtFixed(p * 100, 1);
    return signed(p, String(lang || "").slice(0, 2).toLowerCase() === "fr" ? s.replace(".", ",") + " %" : s + "%");
}

/**
 * A value axis tick with `d` decimals, its digits grouped and its decimals marked by the
 * culture: `1,000.5` in English, `1 000,5` in French (a no-break space, as the runtime's
 * `formatNumber` groups them).
 */
export function tickText(t: number, d: number, lang: string): string {
    const s = fmtFixed(t, d);
    return lang === "fr" ? s.replace(/[,.]/g, (c) => (c === "," ? "\u00a0" : ",")) : s;
}

/** Whether a subtotal flag cell is set: true, a number other than 0, or a text other than "", "0", "false", "no", "non". */
export function flagSet(v: any): boolean {
    if (typeof v === "boolean") {
        return v;
    }
    if (typeof v === "number") {
        return isFinite(v) && v !== 0;
    }
    if (typeof v === "string") {
        const t = v.trim().toLowerCase();
        return t !== "" && t !== "0" && t !== "false" && t !== "no" && t !== "non";
    }
    return false;
}

function formatAt(input: any, k: number): string {
    const l: any[] = input && Array.isArray(input.formats) ? input.formats : [];
    return typeof l[k] === "string" ? l[k] : "";
}

/** The highlighted part of measure `m` of row `i` (null: not highlighted). */
function hlAt(input: any, i: number, m: number): number | null {
    if (!highlighted(input) || i < 0 || i >= input.highlights.length) {
        return null;
    }
    const r = input.highlights[i];
    return Array.isArray(r) && m < r.length ? numOf(r[m]) : null;
}

/** The steps, subtotals and total of the input. */
function itemsOf(input: any, o: any, fr: boolean): Item[] {
    const rows = rowsOf(input, MAX_ROWS);
    let nm = 0;
    for (const r of rows) {
        nm = Math.max(nm, Math.min(r.length - FIRST_MEASURE, MAX_MEASURES));
    }
    const names: any[] = input && Array.isArray(input.names) ? input.names : [];
    const every = Math.floor(optNum(o, "subtotalEvery", 0, 100, 0));
    const subName = fr ? "Sous-total" : "Subtotal";
    const items: Item[] = [];
    let run = 0;
    let steps = 0;
    const room = () => items.length < MAX_STEPS - 1;
    const sub = (label: string) => {
        const last = items.length > 0 ? items[items.length - 1] : null;
        if (last && last.kind === "step" && room()) {
            items.push({ kind: "sub", label, value: run, start: 0, end: run, row: -1, measure: 0, index: -1, hl: null });
        }
    };
    const step = (label: string, v: number, row: number, m: number, hl: number | null) => {
        items.push({ kind: "step", label, value: v, start: run, end: run + v, row, measure: m, index: steps, hl });
        run += v;
        steps += 1;
    };
    if (nm === 1) {
        let group: string | null = null;
        let groupLabel = "";
        for (let i = 0; i < rows.length && room(); i++) {
            const r = rows[i];
            const v = numOf(r[FIRST_MEASURE]);
            if (v === null) {
                continue;
            }
            const g = cellText(r[1]);
            if (group !== null && g !== group) {
                sub(groupLabel);
            }
            if (!room()) {
                break;
            }
            step(cellLabel(input, 0, r[0]), v, i, 0, hlAt(input, i, 0));
            group = g;
            groupLabel = cellLabel(input, 1, r[1]);
            if (flagSet(r[2]) || (every > 0 && steps % every === 0)) {
                sub(subName);
            }
        }
    } else if (nm > 1) {
        for (let m = 0; m < nm && room(); m++) {
            let sum = 0;
            let any = false;
            let hs = 0;
            let lit = false;
            for (let i = 0; i < rows.length; i++) {
                const v = numOf(rows[i][FIRST_MEASURE + m]);
                if (v !== null) {
                    sum += v;
                    any = true;
                }
                const h = hlAt(input, i, m);
                if (h !== null) {
                    hs += h;
                    lit = true;
                }
            }
            if (!any) {
                continue;
            }
            step(typeof names[m] === "string" ? names[m] : "", sum, -1, m, lit ? hs : null);
            if (every > 0 && steps % every === 0) {
                sub(subName);
            }
        }
    }
    if (steps > 0 && optBool(o, "showTotal", true)) {
        const last = items[items.length - 1];
        if (last && last.kind === "sub") {
            items.pop();
        }
        items.push({ kind: "total", label: "Total", value: run, start: 0, end: run, row: -1, measure: 0, index: -1, hl: null });
    }
    return items;
}

/** The fill of an item. */
function fillOf(k: Look, o: any, it: Item): string {
    const total = optColor(o, "totalFill", color(k, 0));
    if (it.kind === "total") {
        return total;
    }
    if (it.kind === "sub") {
        return mix(total, "#FFFFFF", 0.35);
    }
    const by = o.colorBy;
    if (by === "category") {
        return color(k, it.index);
    }
    if (by === "rule") {
        const t = optNum(o, "ruleValue", -MAX_ABS, MAX_ABS, 0);
        return it.value >= t ? optColor(o, "ruleAbove", RULE_ABOVE) : optColor(o, "ruleBelow", RULE_BELOW);
    }
    return it.value >= 0 ? optColor(o, "increaseFill", INCREASE) : optColor(o, "decreaseFill", DECREASE);
}

/** The label of an item for the label mode (`value`, `percent`, `both`, `none`). */
function labelOf(input: any, it: Item, mode: string, lang: string): string {
    if (mode === "none") {
        return "";
    }
    const f = formatAt(input, FIRST_MEASURE + it.measure);
    const v = it.kind === "step" ? signed(it.value, fmtNum(it.value, f, lang)) : fmtNum(it.value, f, lang);
    const p = it.kind === "step" && it.start !== 0 ? pctText(it.value / Math.abs(it.start), lang) : "";
    if (mode === "percent") {
        return p !== "" ? p : v;
    }
    if (mode === "both" && p !== "") {
        return v + " (" + p + ")";
    }
    return v;
}

/** Lays out `{rows: [[category, group, subtotal, target, value…]], names, formats, options}` in a `w`×`h` visual. */
export function layoutWaterfall(input: any, w: number, h: number): Out {
    const o = optionsOf(input);
    const k = lookOfInput(input);
    const lang = input && input.lang === "fr" ? "fr" : "en";
    const out: Out = { prims: [], hits: [] };
    const items = itemsOf(input, o, lang === "fr");
    if (items.length === 0) {
        return out;
    }
    const rows = rowsOf(input, MAX_ROWS);
    let target = 0;
    let hasTarget = false;
    for (const r of rows) {
        const t = numOf(r[3]);
        if (t !== null) {
            target += t;
            hasTarget = true;
        }
    }
    const p = out.prims;
    const size = optNum(o, "fontSize", 8, 20, k.value);
    const mode = o.labelMode === "percent" || o.labelMode === "both" || o.labelMode === "none" ? o.labelMode : "value";
    const horizontal = o.orientation === "horizontal";
    const connectors = optBool(o, "showConnectors", true);
    const lit = highlighted(input);
    const levels: number[] = [];
    for (const it of items) {
        levels.push(it.start, it.end);
    }
    if (hasTarget) {
        levels.push(target);
    }
    const dom = domain(levels, true, null, null, 5);
    const span = dom.hi - dom.lo > 0 ? dom.hi - dom.lo : 1;
    const dec = tickDecimals(dom.step);
    const ticks = dom.ticks.map((t) => tickText(t, dec, lang));
    const labels = items.map((it) => labelOf(input, it, mode, lang));
    pushCard(p, k, w, h);
    const last = items[items.length - 1];
    const fin = last.end;
    // the variance of the total from the target, in the header
    let header = "";
    if (hasTarget) {
        const f = formatAt(input, FIRST_MEASURE);
        const diff = fin - target;
        header = (lang === "fr" ? "Écart à la cible : " : "vs target: ") + signed(diff, fmtNum(diff, f, lang));
        if (target !== 0) {
            header += " (" + pctText(diff / Math.abs(target), lang) + ")";
        }
    }
    const top = PAD + (header !== "" ? k.name + 8 : 0);
    if (header !== "") {
        const s = fit(header, k.name, w - 2 * PAD);
        if (s !== "") {
            out.hits.push({ row: -1, prim: p.length, kind: "variance", value: fin - target, total: fin, target });
            p.push(text(w - PAD, PAD + k.name / 2 + 1, s, k.name, k.label, "end"));
        }
    }
    const n = items.length;
    const lw = (s: string) => est(s, size) + (k.pills ? 10 : 0);
    const targetFill = optColor(o, "targetFill", k.label);
    if (!horizontal) {
        let gutter = 0;
        for (const t of ticks) {
            gutter = Math.max(gutter, est(t, k.tick));
        }
        const x0 = PAD + gutter + 6;
        const x1 = w - PAD;
        const y0 = top + (mode === "none" ? 4 : size + 8);
        const y1 = h - PAD - k.tick - 6;
        const pw = x1 - x0;
        const ph = y1 - y0;
        if (!(pw >= 20 && ph >= 20)) {
            return out;
        }
        const yOf = (v: number) => y1 - ((v - dom.lo) / span) * ph;
        for (let i = 0; i < dom.ticks.length; i++) {
            const y = yOf(dom.ticks[i]);
            p.push(line(x0, y, x1, y, k.grid, 1));
            p.push(text(x0 - 4, y, ticks[i], k.tick, k.text, "end"));
        }
        const slot = pw / n;
        const bw = slot * (minimal(k) ? 0.6 : 0.68);
        for (let i = 0; i < n; i++) {
            const it = items[i];
            const cx = x0 + slot * (i + 0.5);
            const a = yOf(it.start);
            const b = yOf(it.end);
            const y = Math.min(a, b);
            const bh = Math.max(Math.abs(a - b), 1);
            const fill = fillOf(k, o, it);
            const sq = it.kind === "step" ? "" : it.value >= 0 ? "b" : "t";
            const hit = { row: it.row, prim: p.length, item: i, kind: it.kind, label: it.label, value: it.value, start: it.start, end: it.end, measure: it.measure };
            out.hits.push(hit);
            const mark = bar(k, cx - bw / 2, y, bw, bh, fill, sq);
            p.push(lit && it.kind === "step" ? dim(mark) : mark);
            const hv = lit && it.kind === "step" ? part(it.value, it.hl) : null;
            if (hv !== null) {
                const c = yOf(it.start + hv);
                out.hits.push(Object.assign({}, hit, { prim: p.length }));
                p.push(bar(k, cx - bw / 2, Math.min(a, c), bw, Math.max(Math.abs(a - c), 1), fill, ""));
            }
            if (connectors && i + 1 < n) {
                const ly = yOf(it.end);
                p.push(line(cx + bw / 2, ly, cx + slot - bw / 2, ly, k.axis, 1));
            }
            const s = labels[i];
            if (s !== "" && lw(s) <= slot) {
                if (it.value >= 0) {
                    valueLabel(p, k, cx, y, s, "n", 4, size);
                } else {
                    valueLabel(p, k, cx, y + bh, s, "s", 4, size);
                }
            }
            const name = fit(it.label, k.tick, slot - 2);
            if (name !== "") {
                p.push(text(cx, y1 + 4 + k.tick / 2, name, k.tick, k.text, "middle"));
            }
            if (hasTarget && i === n - 1) {
                const ty = yOf(target);
                out.hits.push({ row: -1, prim: p.length, kind: "target", value: target, total: fin });
                p.push(line(cx - bw / 2 - 4, ty, cx + bw / 2 + 4, ty, targetFill, minimal(k) ? 2 : 3));
            }
        }
        const zy = yOf(0);
        p.push(line(x0, zy, x1, zy, k.axis, 1));
        return out;
    }
    // horizontal: the steps top to bottom, the values along x
    let gutter = 0;
    for (const it of items) {
        gutter = Math.max(gutter, est(it.label, k.tick));
    }
    gutter = Math.min(gutter, w * 0.3);
    let right = 0;
    for (const s of labels) {
        right = Math.max(right, lw(s));
    }
    right = Math.min(right, w * 0.25);
    const x0 = PAD + gutter + 6;
    const x1 = w - PAD - (right > 0 ? right + 6 : 0);
    const y0 = top + 2;
    const y1 = h - PAD - k.tick - 6;
    const pw = x1 - x0;
    const ph = y1 - y0;
    if (!(pw >= 20 && ph >= 20)) {
        return out;
    }
    const xOf = (v: number) => x0 + ((v - dom.lo) / span) * pw;
    for (let i = 0; i < dom.ticks.length; i++) {
        const x = xOf(dom.ticks[i]);
        p.push(line(x, y0, x, y1, k.grid, 1));
        const t = ticks[i];
        const half = est(t, k.tick) / 2;
        p.push(text(Math.min(Math.max(x, half), w - half), y1 + 4 + k.tick / 2, t, k.tick, k.text, "middle"));
    }
    const slot = ph / n;
    const bh = slot * (minimal(k) ? 0.6 : 0.68);
    for (let i = 0; i < n; i++) {
        const it = items[i];
        const cy = y0 + slot * (i + 0.5);
        const a = xOf(it.start);
        const b = xOf(it.end);
        const x = Math.min(a, b);
        const bw = Math.max(Math.abs(a - b), 1);
        const fill = fillOf(k, o, it);
        const sq = it.kind === "step" ? "" : it.value >= 0 ? "l" : "r";
        const hit = { row: it.row, prim: p.length, item: i, kind: it.kind, label: it.label, value: it.value, start: it.start, end: it.end, measure: it.measure };
        out.hits.push(hit);
        const mark = bar(k, x, cy - bh / 2, bw, bh, fill, sq);
        p.push(lit && it.kind === "step" ? dim(mark) : mark);
        const hv = lit && it.kind === "step" ? part(it.value, it.hl) : null;
        if (hv !== null) {
            const c = xOf(it.start + hv);
            out.hits.push(Object.assign({}, hit, { prim: p.length }));
            p.push(bar(k, Math.min(a, c), cy - bh / 2, Math.max(Math.abs(a - c), 1), bh, fill, ""));
        }
        if (connectors && i + 1 < n) {
            const lx = xOf(it.end);
            p.push(line(lx, cy + bh / 2, lx, cy + slot - bh / 2, k.axis, 1));
        }
        const s = labels[i];
        // a rise's label after its bar's end; a fall's before its bar, inside the plot so it
        // keeps clear of the category names, else after the bar's other end
        const room = (from: number) => from + 4 + lw(s) <= w - 2;
        if (s !== "" && slot >= size + 2) {
            if (it.value >= 0 && room(x + bw)) {
                valueLabel(p, k, x + bw, cy, s, "e", 4, size);
            } else if (it.value < 0 && x - 4 - lw(s) >= x0 + 2) {
                valueLabel(p, k, x, cy, s, "w", 4, size);
            } else if (it.value < 0 && room(x + bw)) {
                valueLabel(p, k, x + bw, cy, s, "e", 4, size);
            }
        }
        const name = fit(it.label, k.tick, gutter + 2);
        if (name !== "" && slot >= k.tick) {
            p.push(text(x0 - 6, cy, name, k.tick, k.text, "end"));
        }
        if (hasTarget && i === n - 1) {
            const tx = xOf(target);
            out.hits.push({ row: -1, prim: p.length, kind: "target", value: target, total: fin });
            p.push(line(tx, cy - bh / 2 - 4, tx, cy + bh / 2 + 4, targetFill, minimal(k) ? 2 : 3));
        }
    }
    const zx = xOf(0);
    p.push(line(zx, y0, zx, y1, k.axis, 1));
    return out;
}
