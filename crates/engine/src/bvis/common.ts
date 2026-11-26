// BIcraft's own chart visuals (docs/formats/bicraft-visuals.md): the data, options and drawing
// helpers their layouts share, the same as crates/render/src/bvis.rs. Nothing here touches the
// browser, so BIcraft's golden tests run the layouts under Node.
"use strict";

import { Domain, Prim, clip, dateText, est, fmtAuto, fmtFixed, inside, parseIso, r2, text, tickDecimals, line } from "./spec";
import { dateLabel } from "./runtime/format";
import { langOf } from "./runtime/strings";
import { Look, swatch } from "./runtime/style";

export const MAX_ROWS = 30000;
export const MAX_COLUMNS = 16;
const MAX_CELL = 200;

export const PAD = 8;
const MS_PER_DAY = 86400000;

/** A cell: a finite number, a text (dates as `YYYY-MM-DD[ HH:MM]`) or null. */
export type Cell = number | string | null;

export interface Column {
    role: string;
    name: string;
    /** Its format string (the labels of dates are written with it). */
    format: string;
    /** The labels' language: `en` or `fr` (`host.locale`'s). */
    lang: string;
    cells: Cell[];
    /** The highlighted part of each row (a value column's `highlights`: another visual's selection highlights this one, docs/formats/bicraft-visuals.md "Highlights"); null when nothing is highlighted. */
    hl: Cell[] | null;
    /** The data view's column (for selection ids); null in the golden tests. */
    source: any;
}

export interface Data {
    cols: Column[];
}

/** A unit the user selects and hovers (a bin, a bullet, a box, a row): its data rows and numbers. */
export interface Unit {
    rows: number[];
    stats: number[];
}

/** A drawn primitive that belongs to unit `unit`. */
export interface Hit {
    unit: number;
    prim: number;
}

/** What a layout gives the visual besides its primitives. */
export interface Out {
    units: Unit[];
    hits: Hit[];
    /** Primitives drawn as outlines in high contrast (bands, a box's fill). */
    outline?: number[];
    /** The units drawn dimmed because another visual's selection highlights none of their rows (`fade`), ascending. */
    faded?: number[];
}

function cellText(s: string): string {
    return Array.from(s).slice(0, MAX_CELL).join("");
}

/** A data view or JSON value as a cell. */
export function cellOf(v: any): Cell {
    if (typeof v === "number") {
        return isFinite(v) ? v : null;
    }
    if (typeof v === "string") {
        return cellText(v);
    }
    if (typeof v === "boolean") {
        return String(v);
    }
    if (v instanceof Date) {
        const t = v.getTime();
        if (!isFinite(t)) {
            return null;
        }
        // the date on its own clock, as BIcraft reads a date column
        const ms = Date.UTC(v.getFullYear(), v.getMonth(), v.getDate(), v.getHours(), v.getMinutes(), v.getSeconds(), v.getMilliseconds());
        return dateText(ms, Math.floor(ms / MS_PER_DAY) * MS_PER_DAY !== ms);
    }
    return null;
}

/** A format string as the layouts read it (its first 200 characters). */
function formatOf(f: any): string {
    return typeof f === "string" ? cellText(f) : "";
}

/** The golden tests' form: `{locale?, columns: [{role, name?, format?, values}]}`. */
export function dataFromJson(v: any): Data {
    const list: any[] = v && Array.isArray(v.columns) ? v.columns.slice(0, MAX_COLUMNS) : [];
    const lang = langOf(v && typeof v.locale === "string" ? v.locale : "");
    return {
        cols: list.map((c) => ({
            role: c && typeof c.role === "string" ? c.role : "",
            name: c && typeof c.name === "string" ? cellText(c.name) : "",
            format: formatOf(c ? c.format : ""),
            lang,
            cells: c && Array.isArray(c.values) ? c.values.slice(0, MAX_ROWS).map(cellOf) : [],
            hl: c && Array.isArray(c.highlights) ? c.highlights.slice(0, MAX_ROWS).map(cellOf) : null,
            source: null,
        })),
    };
}

/** The categorical data view as columns: one per role each category or value column is bound to; labels in `locale`'s language. */
export function dataOf(dataView: any, locale?: string): Data {
    const lang = langOf(locale);
    const cat = dataView && dataView.categorical ? dataView.categorical : null;
    const cols: Column[] = [];
    const add = (c: any, value: boolean) => {
        const source = c && c.source ? c.source : {};
        const roles = source.roles && typeof source.roles === "object" ? Object.keys(source.roles).filter((k) => !!source.roles[k]) : [];
        const values: any[] = c && Array.isArray(c.values) ? c.values.slice(0, MAX_ROWS) : [];
        // a value column's highlighted parts (`supportsHighlight`)
        const hl: any[] | null = value && c && Array.isArray(c.highlights) ? c.highlights.slice(0, MAX_ROWS) : null;
        for (const role of roles) {
            if (cols.length < MAX_COLUMNS) {
                cols.push({
                    role,
                    name: typeof source.displayName === "string" ? cellText(source.displayName) : "",
                    format: formatOf(source.format),
                    lang,
                    cells: values.map(cellOf),
                    hl: hl ? hl.map(cellOf) : null,
                    source: c,
                });
            }
        }
    };
    const cats: any[] = cat && Array.isArray(cat.categories) ? cat.categories : [];
    for (const c of cats.slice(0, MAX_COLUMNS)) {
        add(c, false);
    }
    const vals: any[] = cat && cat.values && typeof cat.values.length === "number" ? cat.values : [];
    for (let k = 0; k < vals.length && k < MAX_COLUMNS; k++) {
        add(vals[k], true);
    }
    return { cols };
}

export function rows(d: Data): number {
    let n = 0;
    for (const c of d.cols) {
        n = Math.max(n, c.cells.length);
    }
    return Math.min(n, MAX_ROWS);
}

/** The `k`-th column of `role` (roles compared ignoring case). */
export function col(d: Data, role: string, k: number): Column | null {
    const r = role.toLowerCase();
    const l = d.cols.filter((c) => c.role.toLowerCase() === r);
    return k < l.length ? l[k] : null;
}

export function count(d: Data, role: string): number {
    const r = role.toLowerCase();
    return d.cols.filter((c) => c.role.toLowerCase() === r).length;
}

/** Row `i`'s number (a finite one), else null. */
export function num(c: Column | null, i: number): number | null {
    if (!c || i < 0 || i >= c.cells.length) {
        return null;
    }
    const v = c.cells[i];
    return typeof v === "number" && isFinite(v) ? v : null;
}

/** Row `i`'s highlighted part (a finite number), else null (not highlighted). */
export function hlOf(c: Column | null, i: number): number | null {
    if (!c || !c.hl || i < 0 || i >= c.hl.length) {
        return null;
    }
    const v = c.hl[i];
    return typeof v === "number" && isFinite(v) ? v : null;
}

/** Whether another visual's selection highlights this one (a column has `highlights`). */
export function highlighted(d: Data): boolean {
    return d.cols.some((c) => c.hl !== null);
}

/** Whether row `i` is highlighted: a value column has a highlighted part for it. */
export function lit(d: Data, i: number): boolean {
    return d.cols.some((c) => hlOf(c, i) !== null);
}

/** Records in `out.faded` the units none of whose rows is highlighted while the visual is (the visuals whose marks have no highlighted part to draw, as Data::fade in bvis.rs). */
export function fade(d: Data, out: Out): void {
    if (!highlighted(d)) {
        return;
    }
    const f: number[] = [];
    for (let k = 0; k < out.units.length; k++) {
        if (!out.units[k].rows.some((r) => lit(d, r))) {
            f.push(k);
        }
    }
    out.faded = f;
}

/** Row `i` as text (numbers as fmtAuto writes them, blanks empty). */
export function textOf(c: Column | null, i: number): string {
    if (!c || i < 0 || i >= c.cells.length) {
        return "";
    }
    const v = c.cells[i];
    if (typeof v === "number") {
        return fmtAuto(v);
    }
    return typeof v === "string" ? v : "";
}

/** Row `i` as a label: a date written with the column's format string (`dateLabel`, as `Column::label` in bvis.rs), else as textOf writes it. */
export function labelOf(c: Column | null, i: number): string {
    const v = c && i >= 0 && i < c.cells.length ? c.cells[i] : null;
    if (c && typeof v === "string") {
        const ms = parseIso(v);
        const l = ms === null ? null : dateLabel(ms, c.format, c.lang);
        if (l !== null) {
            return l;
        }
    }
    return textOf(c, i);
}

// ---------------------------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------------------------

export function isColor(c: any): boolean {
    return typeof c === "string" && /^#[0-9A-Fa-f]{6}$/.test(c);
}

/** The visual's format objects as plain JSON (a `{solid: {color}}` fill as its colour). */
export function plainObjects(objects: any): any {
    const out: any = {};
    if (!objects || typeof objects !== "object") {
        return out;
    }
    for (const o of Object.keys(objects).slice(0, 64)) {
        const props = objects[o];
        if (!props || typeof props !== "object") {
            continue;
        }
        const p: any = {};
        for (const k of Object.keys(props).slice(0, 64)) {
            const v = props[k];
            p[k] = v && typeof v === "object" && v.solid && typeof v.solid.color === "string" ? v.solid.color : v;
        }
        out[o] = p;
    }
    return out;
}

function optGet(o: any, obj: string, prop: string): any {
    const x = o && typeof o === "object" ? o[obj] : undefined;
    return x && typeof x === "object" ? x[prop] : undefined;
}

/** A number within lo..hi, else `def`. */
export function optNum(o: any, obj: string, prop: string, def: number, lo: number, hi: number): number {
    const v = optGet(o, obj, prop);
    return typeof v === "number" && isFinite(v) ? Math.min(Math.max(v, lo), hi) : def;
}

/** A whole number within lo..hi (rounded down), else `def`. */
export function optInt(o: any, obj: string, prop: string, def: number, lo: number, hi: number): number {
    return Math.floor(optNum(o, obj, prop, def, lo, hi));
}

export function optFlag(o: any, obj: string, prop: string, def: boolean): boolean {
    const v = optGet(o, obj, prop);
    return typeof v === "boolean" ? v : def;
}

export function optColor(o: any, obj: string, prop: string, def: string): string {
    const v = optGet(o, obj, prop);
    return isColor(v) ? String(v).toUpperCase() : def;
}

// ---------------------------------------------------------------------------------------------
// Drawing helpers (the same in bvis.rs)
// ---------------------------------------------------------------------------------------------

export function path(points: number[][], stroke: string, width: number): Prim {
    return { t: "path", points: points.map((p) => [r2(p[0]), r2(p[1])]), stroke, width: r2(width) };
}

export function scale(d: Domain, a: number, b: number, v: number): number {
    return a + ((v - d.lo) / (d.hi - d.lo)) * (b - a);
}

export function yLabels(out: Prim[], k: Look, d: Domain, x: number, top: number, bottom: number): void {
    const dec = tickDecimals(d.step);
    for (const t of d.ticks) {
        out.push(text(x, scale(d, bottom, top, t), fmtFixed(t, dec), k.tick, k.text, "end", 0));
    }
}

export function yGrid(out: Prim[], k: Look, d: Domain, left: number, right: number, top: number, bottom: number): void {
    for (const t of d.ticks) {
        const y = scale(d, bottom, top, t);
        out.push(line(left, y, right, y, k.grid, 1));
    }
}

export function xAxis(out: Prim[], k: Look, d: Domain, left: number, right: number, bottom: number, w: number): void {
    out.push(line(left, bottom, right, bottom, k.axis, 1));
    const dec = tickDecimals(d.step);
    let end = -Infinity;
    for (const t of d.ticks) {
        const s = fmtFixed(t, dec);
        const half = est(s, k.tick) / 2;
        const x = inside(scale(d, left, right, t), s, k.tick, w);
        if (x - half < end + 4) {
            continue;
        }
        end = x + half;
        out.push(text(x, bottom + 9, s, k.tick, k.text, "middle", 0));
    }
}

export function legend(out: Prim[], k: Look, items: string[][], w: number): void {
    let x = PAD;
    for (let j = 0; j < items.length; j++) {
        const name = clip(items[j][0], 30);
        const wd = 14 + est(name, k.name) + 12;
        if (j > 0 && x + wd - 12 > w - PAD) {
            break;
        }
        out.push(swatch(k, x, PAD + 4, 10, items[j][1]));
        out.push(text(x + 14, PAD + 9, name, k.name, k.text, "start", 0));
        x += wd;
    }
}

export function labelWidth(labels: string[], size: number, max: number): number {
    let m = 0;
    for (const l of labels) {
        m = Math.max(m, est(l, size));
    }
    return Math.min(m + 8, max);
}

export function fit(s: string, width: number, size: number): string {
    const n = Math.min(Math.max(Math.floor(width / (size * 0.55)), 1), 60);
    return clip(s, n);
}
