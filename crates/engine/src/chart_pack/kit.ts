// BIcraft's chart pack (ROADMAP M12.3, docs/formats/chart-pack.md): what the layouts of the
// organization chart, Venn diagram, Marimekko and calendar heatmap share. This is the TypeScript
// twin of crates/render/src/chart_pack.rs: both compute the same primitives from the same input
// (`{rows, options, lang}`), held equal by the engine's golden tests
// (crates/engine/tests/report/chart_pack_tests.rs). A change here goes to the Rust too. Nothing
// here touches the browser, so the tests run the layouts under Node.
"use strict";

import { dateText, fmtAuto, fmtFixed, parseIso, r2 } from "./spec";
import { Look, lookFlat, swatch } from "./runtime/style";
import { dateLabel } from "./runtime/format";

export { dateText, fmtAuto, fmtFixed, parseIso, r2 };

/** A drawing primitive (`t`: rect, line, poly, circle, text), in the visual's pixels. */
export type Prim = { [key: string]: any };
/** A drawn element that belongs to data: `prim` its index, `row` its first data row (or -1). */
export type Hit = { [key: string]: any };
/** A layout's result. */
export interface Out {
    prims: Prim[];
    hits: Hit[];
}

export const PAD = 8;
export const MS_PER_DAY = 86400000;

export function isObj(v: any): boolean {
    return v !== null && typeof v === "object" && !Array.isArray(v);
}

export function chars(s: string): number {
    return Array.from(s).length;
}

/** A text's estimated width (no font is measured, so both drawers agree). */
export function est(s: string, size: number): number {
    return chars(s) * size * 0.55;
}

export function clip(s: string, n: number): string {
    const a = Array.from(s);
    if (a.length <= n) {
        return s;
    }
    return a.slice(0, Math.max(n - 1, 0)).join("") + "…";
}

/** `s` shortened with an ellipsis so its estimated width fits `width` ("" when not 2 characters fit). */
export function fit(s: string, size: number, width: number): string {
    if (est(s, size) <= width) {
        return s;
    }
    const n = Math.floor(width / (size * 0.55));
    return n < 2 ? "" : clip(s, n);
}

/** A cell as text: numbers as BIcraft writes them, booleans as true / false, else "". */
export function cellText(v: any): string {
    if (typeof v === "string") {
        return v;
    }
    if (typeof v === "number") {
        return isFinite(v) ? fmtAuto(v) : "";
    }
    if (typeof v === "boolean") {
        return v ? "true" : "false";
    }
    return "";
}

/** Cell `k` of a row as a label: a date written with its column's format string (the input's `formats[k]`, `dateLabel`), else as cellText writes it. */
export function cellLabel(input: any, k: number, v: any): string {
    const formats: any[] = input && Array.isArray(input.formats) ? input.formats : [];
    const f = typeof formats[k] === "string" ? Array.from(formats[k] as string).slice(0, 200).join("") : "";
    if (typeof v === "string" && f !== "") {
        const ms = parseIso(v);
        const l = ms === null ? null : dateLabel(ms, f, langOf(input));
        if (l !== null) {
            return l;
        }
    }
    return cellText(v);
}

export function cellNum(v: any): number | null {
    return typeof v === "number" && isFinite(v) ? v : null;
}

/** Whether another visual's selection highlights the visual: the input has `highlights`, the highlighted part of each row's measure (docs/formats/bicraft-visuals.md "Highlights"). */
export function highlighted(input: any): boolean {
    return !!input && Array.isArray(input.highlights);
}

/** Row `i`'s highlighted part (null: not highlighted). */
export function hlNum(input: any, i: number): number | null {
    return highlighted(input) && i >= 0 && i < input.highlights.length ? cellNum(input.highlights[i]) : null;
}

/** The input's rows (arrays), at most `max`. */
export function rowsOf(input: any, max: number): any[][] {
    const l: any[] = input && Array.isArray(input.rows) ? input.rows : [];
    const out: any[][] = [];
    for (let i = 0; i < l.length && i < max; i++) {
        out.push(Array.isArray(l[i]) ? l[i] : []);
    }
    return out;
}

export function optionsOf(input: any): any {
    return input && isObj(input.options) ? input.options : {};
}

/** The look of the visual (docs/formats/visual-look.md): its flat Style options and the input's `theme`. */
export function lookOfInput(input: any): Look {
    return lookFlat(optionsOf(input), input ? input.theme : undefined);
}

export function langOf(input: any): string {
    return input && input.lang === "fr" ? "fr" : "en";
}

export function optNum(o: any, k: string, lo: number, hi: number, def: number): number {
    const v = o[k];
    return typeof v === "number" && isFinite(v) ? Math.min(Math.max(v, lo), hi) : def;
}

export function optBool(o: any, k: string, def: boolean): boolean {
    const v = o[k];
    return typeof v === "boolean" ? v : def;
}

export function optColor(o: any, k: string, def: string): string {
    const v = o[k];
    return typeof v === "string" && /^#[0-9A-Fa-f]{6}$/.test(v) ? v.toUpperCase() : def;
}

/** A list of texts (at most `max`). */
export function optTexts(o: any, k: string, max: number): string[] {
    const v = o[k];
    const out: string[] = [];
    if (!Array.isArray(v)) {
        return out;
    }
    for (let i = 0; i < v.length && out.length < max; i++) {
        if (typeof v[i] === "string") {
            out.push(v[i]);
        }
    }
    return out;
}

export function rect(x: number, y: number, w: number, h: number, fill: string, opacity: number): Prim {
    return { t: "rect", x: r2(x), y: r2(y), w: r2(w), h: r2(h), fill, opacity };
}

export function line(x1: number, y1: number, x2: number, y2: number, stroke: string, width: number): Prim {
    return { t: "line", x1: r2(x1), y1: r2(y1), x2: r2(x2), y2: r2(y2), stroke, width: r2(width) };
}

/** A filled polygon (a Sankey link's ribbon). */
export function poly(points: number[][], fill: string, opacity: number): Prim {
    return { t: "poly", points: points.map((q) => [r2(q[0]), r2(q[1])]), fill, opacity };
}

export function circle(cx: number, cy: number, r: number, fill: string, opacity: number): Prim {
    return { t: "circle", cx: r2(cx), cy: r2(cy), r: r2(r), fill, opacity };
}

export function text(x: number, y: number, s: string, size: number, fill: string, anchor: string): Prim {
    return { t: "text", x: r2(x), y: r2(y), text: s, size, fill, anchor, rotate: 0 };
}

function hexOf(c: string): number[] {
    const p = (i: number) => {
        const v = parseInt(c.slice(i, i + 2), 16);
        return isNaN(v) ? 0 : v;
    };
    return [p(1), p(3), p(5)];
}

export function lerpColor(a: string, b: string, t: number): string {
    const x = hexOf(a);
    const y = hexOf(b);
    const ch = (p: number, q: number) => {
        const v = Math.min(Math.max(Math.floor(p + (q - p) * t + 0.5), 0), 255);
        const h = v.toString(16).toUpperCase();
        return h.length < 2 ? "0" + h : h;
    };
    return "#" + ch(x[0], y[0]) + ch(x[1], y[1]) + ch(x[2], y[2]);
}

/** Days since 1970-01-01 of a proleptic Gregorian date. */
export function daysFromCivil(y: number, m: number, d: number): number {
    y = m <= 2 ? y - 1 : y;
    const era = Math.trunc((y >= 0 ? y : y - 399) / 400);
    const yoe = y - era * 400;
    const mp = m > 2 ? m - 3 : m + 9;
    const doy = Math.trunc((153 * mp + 2) / 5) + d - 1;
    const doe = yoe * 365 + Math.trunc(yoe / 4) - Math.trunc(yoe / 100) + doy;
    return era * 146097 + doe - 719468;
}

/** [year, month, day] of a day number. */
export function civilFromDays(z: number): number[] {
    z = z + 719468;
    const era = Math.trunc((z >= 0 ? z : z - 146096) / 146097);
    const doe = z - era * 146097;
    const yoe = Math.trunc((doe - Math.trunc(doe / 1460) + Math.trunc(doe / 36524) - Math.trunc(doe / 146096)) / 365);
    const doy = doe - (365 * yoe + Math.trunc(yoe / 4) - Math.trunc(yoe / 100));
    const mp = Math.trunc((5 * doy + 2) / 153);
    const d = doy - Math.trunc((153 * mp + 2) / 5) + 1;
    const m = mp < 10 ? mp + 3 : mp - 9;
    return [yoe + era * 400 + (m <= 2 ? 1 : 0), m, d];
}

/**
 * A legend row at `y` (swatch 10×10 and name in 11 px), from `x0`, items dropped once the row is
 * full. Returns the primitives' count.
 */
export function legend(out: Prim[], k: Look, names: string[], colors: string[], x0: number, y: number, right: number): number {
    let x = x0;
    let n = 0;
    for (let i = 0; i < names.length; i++) {
        const name = fit(names[i], k.name, 140);
        const wd = 14 + est(name, k.name);
        if (x + wd > right) {
            break;
        }
        out.push(swatch(k, x, y, 10, colors[i]));
        out.push(text(x + 14, y + 5, name, k.name, k.label, "start"));
        x += wd + 14;
        n += 2;
    }
    return n;
}

/** One of `items` (`def` when the option is missing or not one of them). */
export function optEnum(o: any, k: string, items: string[], def: string): string {
    const v = o[k];
    return typeof v === "string" && items.indexOf(v) >= 0 ? v : def;
}

/** A text option, at most `max` characters ("" when missing). */
export function optText(o: any, k: string, max: number): string {
    const v = o[k];
    return typeof v === "string" ? Array.from(v).slice(0, max).join("") : "";
}

/** A number short enough for an axis or a label: thousands as K, millions as M, billions as B (one decimal when not whole), else with `d` decimals (null: 0 or 2, as fmtAuto). */
export function compact(v: number, d: number | null): string {
    const a = Math.abs(v);
    const scaled = (x: number, unit: string) => fmtFixed(x, Math.abs(x - Math.floor(x)) < 1e-9 ? 0 : 1) + unit;
    if (a >= 1e9) {
        return scaled(v / 1e9, "B");
    }
    if (a >= 1e6) {
        return scaled(v / 1e6, "M");
    }
    if (a >= 1e4) {
        return scaled(v / 1e3, "K");
    }
    return d === null ? fmtAuto(v) : fmtFixed(v, d);
}

/** Most dashes one call draws. */
export const MAX_DASHES = 4000;

/**
 * A dashed polyline as line primitives (`on` pixels drawn, `off` skipped, the pattern carried
 * across corners), at most MAX_DASHES; pushed onto `out`.
 */
export function dashes(out: Prim[], points: number[][], stroke: string, width: number, on: number, off: number): void {
    let phase = 0;
    let n = 0;
    const period = on + off;
    if (!(on > 0) || !(off >= 0)) {
        return;
    }
    for (let i = 1; i < points.length && n < MAX_DASHES; i++) {
        const a = points[i - 1];
        const b = points[i];
        const dx = b[0] - a[0];
        const dy = b[1] - a[1];
        const len = Math.sqrt(dx * dx + dy * dy);
        if (!(len > 0) || !isFinite(len)) {
            continue;
        }
        let s = 0;
        while (s < len && n < MAX_DASHES) {
            const inOn = phase < on;
            const step = Math.min(inOn ? on - phase : period - phase, len - s);
            if (inOn) {
                const t0 = s / len;
                const t1 = (s + step) / len;
                out.push(line(a[0] + dx * t0, a[1] + dy * t0, a[0] + dx * t1, a[1] + dy * t1, stroke, width));
                n++;
            }
            s += step;
            phase += step;
            if (phase >= period - 1e-9) {
                phase = 0;
            }
        }
    }
}
