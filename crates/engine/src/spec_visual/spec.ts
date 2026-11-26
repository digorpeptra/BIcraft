// The BIcraft Spec visual's chart spec (docs/formats/spec-visual.md): parsing, defaults and the
// layout into drawing primitives. This is the TypeScript twin of BIcraft's renderer
// (crates/render/src/spec.rs): both compute the same primitives from the same spec and data, and
// the engine's golden tests compare them (crates/engine/tests/report/spec_visual_tests.rs). A
// change here goes to the Rust too. Widths are estimated from character counts, numbers and dates
// are formatted by our own code, so both agree to the hundredth of a pixel.
"use strict";

import { dateLabel } from "./runtime/format";
import { langOf } from "./runtime/strings";
import { Look, PALETTES, PRESETS, bar, dim, dot, lookOf, minimal, part, pushCard, radius, swatch, tile, valueLabel } from "./runtime/style";

export const SPEC_VERSION = 1;
export const MAX_SPEC_BYTES = 32 * 1024;
const MAX_MARKS = 16;
const MAX_VALUES = 16;
const MAX_ROWS = 1000;
const MAX_PALETTE = 32;
const MAX_TEXT = 200;
const MARK_TYPES = ["bar", "line", "point", "area", "text", "rule", "rect"];
const DEFAULT_PALETTE = ["#1F6F8B", "#E07A1F", "#5A9E3A", "#C23B5A", "#7A5BA8", "#2E9C9C", "#B08A1E", "#6B6B6B"];
const VALUE_RANGE = ["#D6E6EE", "#1F6F8B"];
/** The colour of a palette slot that is missing (never drawn: palettes are not empty). */
const TEXT = "#555555";
const MS_PER_DAY = 86400000;
const MAX_TICKS = 50;
/** Most points of a line drawn with a marker on each (Modern and Bold). */
const MAX_LINE_DOTS = 40;
/** Most marks once every bound measure is drawn as its own series ("Series" in spec-visual.md). */
const MAX_SERIES_MARKS = 64;
/** The text elements of `spec.text` and their settings (docs/formats/spec-visual.md "Text"). */
const TEXT_PARTS = ["labels", "x", "y", "xTitle", "yTitle", "legend"];
const TEXT_KEYS = ["show", "size", "family", "color", "bold", "italic", "underline"];
/** Display units: 0 automatic, 1 none, then thousands to trillions. */
export const UNITS = [0, 1, 1000, 1000000, 1000000000, 1000000000000];

// ---------------------------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------------------------

/** A field: the category (`k: "c"`) or the measure at index `i` of the values role. */
export interface Ref { k: "c" | "v"; i: number; }
/** A position encoding: a field, a number or a text. */
export interface Enc { f?: Ref; n?: number; s?: string; }
export interface Color { fixed?: string; byCategory?: boolean; byValue?: Ref; range?: string[]; }
export interface Size { fixed?: number; byValue?: Ref; range?: number[]; }
export interface Mark {
    kind: string;
    name: string | null;
    x: Enc | null;
    y: Enc | null;
    x2: Enc | null;
    y2: Enc | null;
    color: Color | null;
    size: Size | null;
    label: Ref | null;
    opacity: number;
    decimals: number | null;
}
export interface Axis {
    scale: string;
    axis: boolean;
    title: string | null;
    grid: boolean;
    min: number | null;
    max: number | null;
    zero: boolean;
    padding: number;
}
export interface Spec {
    padding: number;
    palette: string[];
    /** The spec gives its palette (else the look's: the report theme's or BIcraft's). */
    ownPalette: boolean;
    /** The Style options (docs/formats/visual-look.md), checked; `{}` when not given. */
    style: any;
    /** The text settings by element (`labels`, `x`, `y`, `xTitle`, `yTitle`, `legend`), checked; `{}` when not given. */
    text: any;
    x: Axis;
    y: Axis;
    legend: boolean | null;
    marks: Mark[];
}
/** A category value: text, number or date (milliseconds, the value's own clock). */
export interface Cat { kind: "text" | "num" | "date"; s: string; n: number; }
/** The rows, the category column's format string (its dates are labelled with it) and the labels' language (`en`, `fr`). */
export interface SpecData {
    cats: Cat[];
    values: (number | null)[][];
    catFormat: string;
    lang: string;
    /** The highlighted part of each value, by value column then row (the data view's `highlights`: docs/formats/bicraft-visuals.md "Highlights"); absent or empty when nothing is highlighted. */
    highlights?: (number | null)[][];
    /** The category field's name and each value field's name (series names and default axis titles). */
    catName?: string;
    names?: string[];
}
export type Prim = { [key: string]: any };
/**
 * A data point the layout drew, for the visual's interactions only (not part of the layout the
 * Rust twin computes): its row (category index) and the index of its primitive in the output, or
 * for a vertex of a line or area (`prim` −1) its position; `label` for a value label.
 */
export interface Hit { row: number; prim: number; x?: number; y?: number; label?: boolean; fade?: boolean; }

// ---------------------------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------------------------

function isObj(v: any): boolean {
    return v !== null && typeof v === "object" && !Array.isArray(v);
}

function chars(s: string): number {
    return Array.from(s).length;
}

function keys(o: any, allowed: string[], at: string): void {
    for (const k of Object.keys(o)) {
        if (allowed.indexOf(k) < 0) {
            throw new Error(`${at}: unknown key \`${clip(k, 40)}\` (allowed: ${allowed.join(", ")})`);
        }
    }
}

function textOf(v: any, at: string): string {
    if (typeof v !== "string") {
        throw new Error(`${at}: a string is needed`);
    }
    if (chars(v) > MAX_TEXT || /[\u0000-\u001f\u007f-\u009f]/.test(v)) {
        throw new Error(`${at}: at most ${MAX_TEXT} characters, without control characters`);
    }
    return v;
}

function numOf(v: any, at: string, lo: number, hi: number): number {
    if (typeof v !== "number" || !isFinite(v) || v < lo || v > hi) {
        throw new Error(`${at}: a number from ${lo} to ${hi} is needed`);
    }
    return v;
}

function boolOf(v: any, at: string): boolean {
    if (typeof v !== "boolean") {
        throw new Error(`${at}: true or false is needed`);
    }
    return v;
}

function colorOf(v: any, at: string): string {
    if (typeof v === "string" && /^#[0-9A-Fa-f]{6}$/.test(v)) {
        return v.toUpperCase();
    }
    throw new Error(`${at}: a colour as #RRGGBB is needed`);
}

export function parseRef(s: string): Ref | null {
    if (s === "category") {
        return { k: "c", i: 0 };
    }
    if (s === "value") {
        return { k: "v", i: 0 };
    }
    const m = /^value\.([0-9]{1,2})$/.exec(s);
    if (!m) {
        return null;
    }
    const i = parseInt(m[1], 10);
    return i < MAX_VALUES ? { k: "v", i } : null;
}

function refOf(v: any, at: string): Ref {
    const r = typeof v === "string" ? parseRef(v) : null;
    if (!r) {
        throw new Error(`${at}: a field is \`category\`, \`value\` (the first measure) or \`value.N\` (N from 0 to ${MAX_VALUES - 1})`);
    }
    return r;
}

function encOf(v: any, at: string): Enc {
    if (typeof v === "string") {
        return { f: refOf(v, at) };
    }
    if (isObj(v)) {
        keys(v, ["field", "value"], at);
        const hasF = v.field !== undefined;
        const hasV = v.value !== undefined;
        if (hasF && !hasV) {
            return { f: refOf(v.field, `${at}.field`) };
        }
        if (!hasF && hasV) {
            if (typeof v.value === "string") {
                return { s: textOf(v.value, at) };
            }
            return { n: numOf(v.value, `${at}.value`, -1e15, 1e15) };
        }
        throw new Error(`${at}: give \`field\` or \`value\``);
    }
    throw new Error(`${at}: a field name or {field} / {value} is needed`);
}

function markOf(v: any, i: number): Mark {
    const at = `marks[${i}]`;
    if (!isObj(v)) {
        throw new Error(`${at}: an object is needed`);
    }
    keys(v, ["type", "name", "x", "y", "x2", "y2", "color", "size", "label", "opacity", "decimals"], at);
    const kind = v.type;
    if (typeof kind !== "string" || MARK_TYPES.indexOf(kind) < 0) {
        throw new Error(`${at}.type: one of ${MARK_TYPES.join(", ")}`);
    }
    const enc = (k: string): Enc | null => (v[k] === undefined ? null : encOf(v[k], `${at}.${k}`));
    const rule = kind === "rule";
    let x = enc("x");
    if (x === null && !rule) {
        x = { f: { k: "c", i: 0 } };
    }
    let y = enc("y");
    if (y === null && !rule) {
        y = { f: { k: "v", i: 0 } };
    }
    if (rule && (x !== null) === (y !== null)) {
        throw new Error(`${at}: a rule takes \`x\` (a vertical line) or \`y\` (a horizontal one)`);
    }
    let color: Color | null = null;
    if (v.color !== undefined) {
        const cat = `${at}.color`;
        if (typeof v.color === "string") {
            color = { fixed: colorOf(v.color, cat) };
        } else if (isObj(v.color)) {
            keys(v.color, ["field", "range"], cat);
            const f = refOf(v.color.field === undefined ? null : v.color.field, `${cat}.field`);
            if (f.k === "c") {
                if (v.color.range !== undefined) {
                    throw new Error(`${cat}: colours by category come from the palette (no range)`);
                }
                color = { byCategory: true };
            } else {
                let range = VALUE_RANGE.slice();
                if (v.color.range !== undefined) {
                    const l = v.color.range;
                    if (!Array.isArray(l) || l.length !== 2) {
                        throw new Error(`${cat}.range: two colours are needed`);
                    }
                    range = [colorOf(l[0], `${cat}.range[0]`), colorOf(l[1], `${cat}.range[1]`)];
                }
                color = { byValue: f, range };
            }
        } else {
            throw new Error(`${at}.color: #RRGGBB or {field, range?} is needed`);
        }
    }
    let size: Size | null = null;
    if (v.size !== undefined) {
        const sat = `${at}.size`;
        if (isObj(v.size)) {
            keys(v.size, ["field", "range"], sat);
            const r = refOf(v.size.field === undefined ? null : v.size.field, `${sat}.field`);
            if (r.k === "c") {
                throw new Error(`${sat}.field: a size comes from a measure`);
            }
            let range = [3, 12];
            if (v.size.range !== undefined) {
                const l = v.size.range;
                if (!Array.isArray(l) || l.length !== 2) {
                    throw new Error(`${sat}.range: two numbers are needed`);
                }
                range = [numOf(l[0], `${sat}.range[0]`, 0, 200), numOf(l[1], `${sat}.range[1]`, 0, 200)];
            }
            size = { byValue: r, range };
        } else {
            size = { fixed: numOf(v.size, sat, 0, 200) };
        }
    }
    const label = v.label === undefined ? null : refOf(v.label, `${at}.label`);
    if (kind === "text" && label === null) {
        throw new Error(`${at}: a text mark needs \`label\` (the field it writes)`);
    }
    let opacity = kind === "area" ? 0.35 : 1;
    if (v.opacity !== undefined) {
        opacity = numOf(v.opacity, `${at}.opacity`, 0, 1);
    }
    const decimals = v.decimals === undefined ? null : Math.floor(numOf(v.decimals, `${at}.decimals`, 0, 6));
    let name = v.name === undefined ? null : textOf(v.name, `${at}.name`);
    if (name !== null && name.trim() === "") {
        name = null;
    }
    return { kind, name, x, y, x2: enc("x2"), y2: enc("y2"), color, size, label, opacity, decimals };
}

function axisOf(v: any, at: string, isX: boolean): Axis {
    const a: Axis = {
        scale: isX ? "band" : "linear",
        axis: true,
        title: null,
        grid: !isX,
        min: null,
        max: null,
        zero: !isX,
        padding: 0.2,
    };
    if (v === undefined) {
        return a;
    }
    if (!isObj(v)) {
        throw new Error(`${at}: an object is needed`);
    }
    keys(v, ["scale", "axis", "title", "grid", "min", "max", "zero", "padding"], at);
    if (v.scale !== undefined) {
        const s = v.scale;
        if (isX && (s === "band" || s === "linear" || s === "time")) {
            a.scale = s;
        } else if (!isX && s === "linear") {
            a.scale = s;
        } else if (isX) {
            throw new Error(`${at}.scale: band, linear or time`);
        } else {
            throw new Error(`${at}.scale: the y scale is linear`);
        }
    }
    if (v.axis !== undefined) {
        a.axis = boolOf(v.axis, `${at}.axis`);
    }
    if (v.title !== undefined) {
        const t = textOf(v.title, `${at}.title`);
        a.title = t.trim() === "" ? null : t;
    }
    if (v.grid !== undefined) {
        a.grid = boolOf(v.grid, `${at}.grid`);
    }
    if (v.zero !== undefined) {
        a.zero = boolOf(v.zero, `${at}.zero`);
    }
    if (v.min !== undefined) {
        a.min = numOf(v.min, `${at}.min`, -1e15, 1e15);
    }
    if (v.max !== undefined) {
        a.max = numOf(v.max, `${at}.max`, -1e15, 1e15);
    }
    if (v.padding !== undefined) {
        a.padding = numOf(v.padding, `${at}.padding`, 0, 0.9);
    }
    return a;
}

/** Parses and checks a spec; throws an Error naming the faulty place. */
export function parse(v: any): Spec {
    if (!isObj(v)) {
        throw new Error("the spec must be a JSON object");
    }
    keys(v, ["$schema", "version", "padding", "palette", "style", "text", "x", "y", "legend", "marks"], "spec");
    if (v.version !== undefined && v.version !== SPEC_VERSION) {
        throw new Error(`spec.version: ${SPEC_VERSION} is the version BIcraft reads`);
    }
    const padding = v.padding === undefined ? 8 : numOf(v.padding, "spec.padding", 0, 64);
    let palette = DEFAULT_PALETTE.slice();
    if (v.palette !== undefined) {
        if (!Array.isArray(v.palette) || v.palette.length === 0 || v.palette.length > MAX_PALETTE) {
            throw new Error(`spec.palette: 1 to ${MAX_PALETTE} colours as #RRGGBB`);
        }
        palette = v.palette.map((c: any, i: number) => colorOf(c, `spec.palette[${i}]`));
    }
    const ownPalette = v.palette !== undefined;
    const style = styleOf(v.style);
    const text = textSetOf(v.text);
    const x = axisOf(v.x, "spec.x", true);
    const y = axisOf(v.y, "spec.y", false);
    const legend = v.legend === undefined ? null : boolOf(v.legend, "spec.legend");
    if (!Array.isArray(v.marks) || v.marks.length === 0 || v.marks.length > MAX_MARKS) {
        throw new Error(`spec.marks: a list of 1 to ${MAX_MARKS} marks is needed`);
    }
    const marks: Mark[] = v.marks.map((m: any, i: number) => markOf(m, i));
    marks.forEach((m, i) => {
        const at = `marks[${i}]`;
        const xs: [string, Enc | null][] = [["x", m.x], ["x2", m.x2]];
        for (const [k, e] of xs) {
            if (e === null) {
                continue;
            }
            if (x.scale === "band" && e.n !== undefined) {
                throw new Error(`${at}.${k}: the x scale is a band (categories): give \`category\` or a category's text, or make spec.x.scale linear or time`);
            }
            if (x.scale === "band" && e.f && e.f.k === "v") {
                throw new Error(`${at}.${k}: a measure on x needs spec.x.scale linear (a band scale places categories)`);
            }
            if (x.scale === "linear" && e.s !== undefined) {
                throw new Error(`${at}.${k}: the linear x scale takes numbers`);
            }
            if (x.scale === "time" && e.s !== undefined && parseIso(e.s) === null) {
                throw new Error(`${at}.${k}: the time scale takes ISO dates (2024-01-31)`);
            }
        }
        const ys: [string, Enc | null][] = [["y", m.y], ["y2", m.y2]];
        for (const [k, e] of ys) {
            if (e !== null && e.f && e.f.k === "c") {
                throw new Error(`${at}.${k}: y is a measure (\`value\`, \`value.N\`) or a number`);
            }
            if (e !== null && e.s !== undefined) {
                throw new Error(`${at}.${k}: y takes numbers`);
            }
        }
        if (m.kind === "bar" && x.scale !== "band") {
            throw new Error(`${at}: bars need the band x scale (use rect on a linear scale)`);
        }
        if (m.kind === "rect" && x.scale !== "band" && m.x2 === null) {
            throw new Error(`${at}: a rect on a linear or time x scale needs \`x2\``);
        }
    });
    return { padding, palette, ownPalette, style, text, x, y, legend, marks };
}

/** The spec's `text` (font and number settings of its text elements), checked; colours upper case, decimals whole. */
function textSetOf(v: any): any {
    if (v === undefined) {
        return {};
    }
    if (!isObj(v)) {
        throw new Error("spec.text: an object is needed");
    }
    keys(v, TEXT_PARTS, "spec.text");
    const out: any = {};
    for (const part of TEXT_PARTS) {
        const o = v[part];
        if (o === undefined) {
            continue;
        }
        const at = `spec.text.${part}`;
        if (!isObj(o)) {
            throw new Error(`${at}: an object is needed`);
        }
        const numeric = part === "labels" || part === "y";
        keys(o, numeric ? TEXT_KEYS.concat(["units", "decimals"]) : TEXT_KEYS, at);
        const s: any = {};
        for (const k of ["show", "bold", "italic", "underline"]) {
            if (o[k] !== undefined) {
                s[k] = boolOf(o[k], `${at}.${k}`);
            }
        }
        if (o.size !== undefined) {
            s.size = numOf(o.size, `${at}.size`, 6, 60);
        }
        if (o.family !== undefined) {
            s.family = textOf(o.family, `${at}.family`);
        }
        if (o.color !== undefined) {
            s.color = colorOf(o.color, `${at}.color`);
        }
        if (o.units !== undefined) {
            if (!(typeof o.units === "number" && UNITS.indexOf(o.units) >= 0)) {
                throw new Error(`${at}.units: one of 0 (automatic), 1 (none), 1000, 1000000, 1000000000, 1000000000000`);
            }
            s.units = o.units;
        }
        if (o.decimals !== undefined) {
            s.decimals = Math.floor(numOf(o.decimals, `${at}.decimals`, 0, 6));
        }
        out[part] = s;
    }
    return out;
}

/** The measures the spec reads (highest index plus one), as `Spec::values_used` in spec.rs. */
export function valuesUsed(spec: Spec): number {
    let n = 0;
    const see = (r: Ref | null | undefined) => {
        if (r && r.k === "v") {
            n = Math.max(n, r.i + 1);
        }
    };
    for (const m of spec.marks) {
        for (const e of [m.x, m.y, m.x2, m.y2]) {
            if (e && e.f) {
                see(e.f);
            }
        }
        if (m.color && m.color.byValue) {
            see(m.color.byValue);
        }
        if (m.size && m.size.byValue) {
            see(m.size.byValue);
        }
        see(m.label);
    }
    return n;
}

/** The spec's `style` (the Style options of BIcraft's look), checked. */
function styleOf(v: any): any {
    if (v === undefined) {
        return {};
    }
    if (!isObj(v)) {
        throw new Error("spec.style: an object is needed");
    }
    keys(v, ["preset", "palette", "rounded", "gradient", "shadow", "animation", "card", "pills"], "spec.style");
    const lists: [string, string[]][] = [["preset", PRESETS], ["palette", PALETTES]];
    for (const [k, list] of lists) {
        if (v[k] !== undefined && !(typeof v[k] === "string" && list.indexOf(v[k]) >= 0)) {
            throw new Error(`spec.style.${k}: one of ${list.join(", ")}`);
        }
    }
    for (const k of ["rounded", "gradient", "shadow", "animation", "card", "pills"]) {
        if (v[k] !== undefined) {
            boolOf(v[k], `spec.style.${k}`);
        }
    }
    return v;
}

/** Parses spec text. */
export function parseText(text: string): Spec {
    if (text.length > MAX_SPEC_BYTES) {
        throw new Error(`the spec is ${text.length} characters; at most ${MAX_SPEC_BYTES}`);
    }
    let v: any;
    try {
        v = JSON.parse(text);
    } catch (e) {
        throw new Error(`the spec is not valid JSON: ${e && (e as any).message ? (e as any).message : e}`);
    }
    return parse(v);
}

// ---------------------------------------------------------------------------------------------
// Data
// ---------------------------------------------------------------------------------------------

function catFromJson(v: any): Cat {
    if (typeof v === "number") {
        return isFinite(v) ? { kind: "num", s: "", n: v } : { kind: "text", s: "", n: 0 };
    }
    if (isObj(v)) {
        const ms = typeof v.date === "string" ? parseIso(v.date) : null;
        return ms === null ? { kind: "text", s: "", n: 0 } : { kind: "date", s: "", n: ms };
    }
    if (typeof v === "string") {
        return { kind: "text", s: v, n: 0 };
    }
    if (typeof v === "boolean") {
        return { kind: "text", s: String(v), n: 0 };
    }
    return { kind: "text", s: "", n: 0 };
}

function numCell(v: any): number | null {
    return typeof v === "number" && isFinite(v) ? v : null;
}

/** The golden tests' data form (as SpecData::from_json). */
export function dataFromJson(v: any): SpecData {
    const cats = v && Array.isArray(v.categories) ? v.categories.slice(0, MAX_ROWS).map(catFromJson) : [];
    const values = v && Array.isArray(v.values)
        ? v.values.slice(0, MAX_VALUES).map((c: any) => (Array.isArray(c) ? c.slice(0, MAX_ROWS).map(numCell) : []))
        : [];
    const highlights = v && Array.isArray(v.highlights)
        ? v.highlights.slice(0, MAX_VALUES).map((c: any) => (Array.isArray(c) ? c.slice(0, MAX_ROWS).map(numCell) : []))
        : [];
    const name = (x: any) => (typeof x === "string" ? clip(x, MAX_TEXT) : "");
    const names = v && Array.isArray(v.names) ? v.names.slice(0, MAX_VALUES).map(name) : [];
    return { cats, values, highlights, catFormat: formatOf(v ? v.categoryFormat : ""), lang: langOf(v && typeof v.locale === "string" ? v.locale : ""), catName: name(v ? v.categoryName : ""), names };
}

/** A format string as the layout reads it (its first 200 characters). */
export function formatOf(f: any): string {
    return typeof f === "string" ? Array.from(f).slice(0, 200).join("") : "";
}

/** A category value Power BI hands over (a Date is read on its own clock). */
export function catOf(v: any): Cat {
    if (v instanceof Date) {
        const t = v.getTime();
        if (!isFinite(t)) {
            return { kind: "text", s: "", n: 0 };
        }
        const ms = Date.UTC(v.getFullYear(), v.getMonth(), v.getDate(), v.getHours(), v.getMinutes(), v.getSeconds(), v.getMilliseconds());
        return { kind: "date", s: "", n: ms };
    }
    if (v === null || v === undefined) {
        return { kind: "text", s: "", n: 0 };
    }
    return catFromJson(v);
}

function rows(d: SpecData): number {
    let n = d.cats.length;
    for (const c of d.values) {
        n = Math.max(n, c.length);
    }
    return Math.min(n, MAX_ROWS);
}

function value(d: SpecData, k: number, i: number): number | null {
    const c = d.values[k];
    if (!c) {
        return null;
    }
    const v = c[i];
    return typeof v === "number" && isFinite(v) ? v : null;
}

/** Whether another visual's selection highlights this one. */
function highlighted(d: SpecData): boolean {
    return !!d.highlights && d.highlights.length > 0;
}

/** The highlighted part of value `k` at row `i` (null: not highlighted). */
function highlightOf(d: SpecData, k: number, i: number): number | null {
    const c = d.highlights ? d.highlights[k] : undefined;
    const v = c ? c[i] : null;
    return typeof v === "number" && isFinite(v) ? v : null;
}

/** Whether row `i`'s marks are drawn dimmed: the visual is highlighted and no value of the row is. */
function faded(d: SpecData, i: number): boolean {
    if (!highlighted(d)) {
        return false;
    }
    const n = d.highlights ? d.highlights.length : 0;
    for (let k = 0; k < n; k++) {
        if (highlightOf(d, k, i) !== null) {
            return false;
        }
    }
    return true;
}

// ---------------------------------------------------------------------------------------------
// Numbers, dates and text (the same in spec.rs)
// ---------------------------------------------------------------------------------------------

export function r2(v: number): number {
    return Math.floor(v * 100 + 0.5) / 100;
}

export function fmtFixed(v: number, d: number): string {
    if (!isFinite(v)) {
        return "";
    }
    d = Math.min(d, 6);
    const scale = Math.pow(10, d);
    const a = Math.floor(Math.abs(v) * scale + 0.5);
    if (a > 9.0e15) {
        return "#";
    }
    const s = Math.pow(10, d);
    const int = String(Math.floor(a / s));
    let out = "";
    if (v < 0 && a !== 0) {
        out += "-";
    }
    const len = int.length;
    for (let i = 0; i < len; i++) {
        if (i > 0 && (len - i) % 3 === 0) {
            out += ",";
        }
        out += int.charAt(i);
    }
    if (d > 0) {
        const frac = String(a % s);
        out += ".";
        for (let i = frac.length; i < d; i++) {
            out += "0";
        }
        out += frac;
    }
    return out;
}

export function fmtAuto(v: number): string {
    return fmtFixed(v, Math.abs(v - Math.floor(v)) < 1e-9 ? 0 : 2);
}

/** `s` (written by fmtFixed) with the culture's marks: `1 000,5` in French (a no-break space), unchanged in English. */
export function localNum(s: string, lang: string): string {
    return lang === "fr" ? s.replace(/[,.]/g, (c) => (c === "," ? " " : ",")) : s;
}

/** The display unit an automatic setting takes for values up to `maxAbs`. */
export function autoUnit(maxAbs: number): number {
    return maxAbs >= 1e12 ? 1e12 : maxAbs >= 1e9 ? 1e9 : maxAbs >= 1e6 ? 1e6 : maxAbs >= 1e3 ? 1e3 : 1;
}

/** The unit drawn for setting `units` (null: none, 0: automatic from `maxAbs`). */
export function unitOf(units: number | null, maxAbs: number): number {
    return units === null ? 1 : units === 0 ? autoUnit(maxAbs) : units;
}

/** `v` in unit `u` with `d` decimals (null: none for whole numbers, else 2), in the culture of `lang`, with the unit's suffix. */
export function numText(v: number, d: number | null, u: number, lang: string): string {
    const x = u > 1 ? v / u : v;
    const s = localNum(d !== null ? fmtFixed(x, d) : fmtAuto(x), lang);
    const fr = lang === "fr";
    const suffix = u === 1e3 ? (fr ? " k" : "K") : u === 1e6 ? (fr ? " M" : "M") : u === 1e9 ? (fr ? " Md" : "bn") : u === 1e12 ? (fr ? " Bn" : "T") : "";
    return s + suffix;
}

export function tickDecimals(step: number): number {
    if (step >= 1 || step <= 0) {
        return 0;
    }
    return Math.min(Math.max(Math.ceil(-Math.log10(step) - 1e-9), 0), 6);
}

function daysFromCivil(y: number, m: number, d: number): number {
    y = m <= 2 ? y - 1 : y;
    const era = Math.trunc((y >= 0 ? y : y - 399) / 400);
    const yoe = y - era * 400;
    const mp = m > 2 ? m - 3 : m + 9;
    const doy = Math.trunc((153 * mp + 2) / 5) + d - 1;
    const doe = yoe * 365 + Math.trunc(yoe / 4) - Math.trunc(yoe / 100) + doy;
    return era * 146097 + doe - 719468;
}

function civilFromDays(z: number): number[] {
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

function digits(s: string | undefined, n: number): number | null {
    if (s === undefined || s.length !== n || !/^[0-9]+$/.test(s)) {
        return null;
    }
    return parseInt(s, 10);
}

/** `YYYY-MM-DD[THH:MM[:SS[.fff]]][zone]` as milliseconds since 1970-01-01 (zone ignored). */
export function parseIso(s: string): number | null {
    s = s.trim();
    if (s.length > 40 || !/^[\x00-\x7f]*$/.test(s)) {
        return null;
    }
    let date = s;
    let time = "";
    const cut = s.search(/[T ]/);
    if (cut >= 0) {
        date = s.slice(0, cut);
        time = s.slice(cut + 1);
    }
    const dp = date.split("-");
    if (dp.length !== 3) {
        return null;
    }
    const y = digits(dp[0], 4);
    const m = digits(dp[1], 2);
    const d = digits(dp[2], 2);
    if (y === null || m === null || d === null || m < 1 || m > 12 || d < 1 || d > 31) {
        return null;
    }
    time = time.replace(/Z+$/, "");
    const z = time.search(/[+-]/);
    if (z >= 0) {
        time = time.slice(0, z);
    }
    let hh = 0, mi = 0, ss = 0, ms = 0;
    if (time !== "") {
        let hms = time;
        let frac: string | null = null;
        const dot = time.indexOf(".");
        if (dot >= 0) {
            hms = time.slice(0, dot);
            frac = time.slice(dot + 1);
        }
        const tp = hms.split(":");
        if (tp.length < 2 || tp.length > 3) {
            return null;
        }
        const h1 = digits(tp[0], 2);
        const m1 = digits(tp[1], 2);
        const s1 = tp.length === 3 ? digits(tp[2], 2) : 0;
        if (h1 === null || m1 === null || s1 === null || h1 > 23 || m1 > 59 || s1 > 59) {
            return null;
        }
        hh = h1;
        mi = m1;
        ss = s1;
        if (frac !== null) {
            if (frac.length === 0 || frac.length > 9 || !/^[0-9]+$/.test(frac)) {
                return null;
            }
            ms = parseInt((frac + "00").slice(0, 3), 10);
        }
    }
    return daysFromCivil(y, m, d) * MS_PER_DAY + (hh * 3600000 + mi * 60000 + ss * 1000 + ms);
}

function pad(n: number, w: number): string {
    let s = String(Math.abs(n));
    while (s.length < w) {
        s = "0" + s;
    }
    return n < 0 ? "-" + s : s;
}

export function dateText(ms: number, time: boolean): string {
    if (!isFinite(ms)) {
        return "";
    }
    ms = Math.min(Math.max(ms, -8.64e15), 8.64e15);
    const days = Math.floor(ms / MS_PER_DAY);
    const ymd = civilFromDays(days);
    let out = `${pad(ymd[0], 4)}-${pad(ymd[1], 2)}-${pad(ymd[2], 2)}`;
    if (time) {
        const rem = ms - days * MS_PER_DAY;
        const h = Math.floor(rem / 3600000);
        const mi = Math.floor((rem - h * 3600000) / 60000);
        out += ` ${pad(h, 2)}:${pad(mi, 2)}`;
    }
    return out;
}

export function clip(s: string, n: number): string {
    const a = Array.from(s);
    if (a.length <= n) {
        return s;
    }
    return a.slice(0, Math.max(n - 1, 0)).join("") + "…";
}

export function est(s: string, size: number): number {
    return chars(s) * size * 0.55;
}

// The centre of a centred text moved so its estimated width stays within [0, w] (the first and
// last x tick labels; a text wider than the visual starts at 0).
export function inside(x: number, s: string, size: number, w: number): number {
    const half = est(s, size) / 2;
    return Math.max(Math.min(x, w - half), half);
}

function catText(c: Cat | undefined): string {
    if (!c) {
        return "";
    }
    if (c.kind === "num") {
        return fmtAuto(c.n);
    }
    if (c.kind === "date") {
        return dateText(c.n, Math.floor(c.n / MS_PER_DAY) * MS_PER_DAY !== c.n);
    }
    return c.s;
}

/** A category as a label: a date written with the category's format string (`dateLabel`, as `cat_label` in spec.rs), else as catText writes it. */
function catLabel(d: SpecData, i: number): string {
    const c = d.cats[i];
    if (c && c.kind === "date") {
        const l = dateLabel(c.n, d.catFormat, d.lang);
        if (l !== null) {
            return l;
        }
    }
    if (c && c.kind === "num") {
        return localNum(fmtAuto(c.n), d.lang);
    }
    return catText(c);
}

function hexOf(c: string): number[] {
    const p = (i: number) => {
        const v = parseInt(c.slice(i, i + 2), 16);
        return isNaN(v) ? 0 : v;
    };
    return [p(1), p(3), p(5)];
}

function lerpColor(a: string, b: string, t: number): string {
    const x = hexOf(a);
    const y = hexOf(b);
    const ch = (p: number, q: number) => {
        const v = Math.min(Math.max(Math.floor(p + (q - p) * t + 0.5), 0), 255);
        const h = v.toString(16).toUpperCase();
        return h.length < 2 ? "0" + h : h;
    };
    return "#" + ch(x[0], y[0]) + ch(x[1], y[1]) + ch(x[2], y[2]);
}

// ---------------------------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------------------------

export interface Domain { lo: number; hi: number; step: number; ticks: number[]; }

function niceStep(raw: number): number {
    if (!(raw > 0) || !isFinite(raw)) {
        return 1;
    }
    const p = Math.pow(10, Math.floor(Math.log10(raw)));
    const f = raw / p;
    const m = f <= 1 ? 1 : f <= 2 ? 2 : f <= 5 ? 5 : 10;
    return m * p;
}

export function domain(vals: number[], zero: boolean, min: number | null, max: number | null, count: number): Domain {
    let lo = Infinity;
    let hi = -Infinity;
    for (const v of vals) {
        lo = Math.min(lo, v);
        hi = Math.max(hi, v);
    }
    if (zero) {
        lo = Math.min(lo, 0);
        hi = Math.max(hi, 0);
    }
    if (!isFinite(lo) || !isFinite(hi)) {
        lo = 0;
        hi = 1;
    }
    if (min !== null) {
        lo = min;
    }
    if (max !== null) {
        hi = max;
    }
    if (hi < lo) {
        const t = lo;
        lo = hi;
        hi = t;
    }
    if (hi === lo) {
        if (lo === 0) {
            hi = 1;
        } else {
            const d = Math.abs(lo) * 0.1;
            lo -= d;
            hi += d;
        }
    }
    const step = niceStep((hi - lo) / count);
    if (min === null) {
        lo = Math.floor(lo / step) * step;
    }
    if (max === null) {
        hi = Math.ceil(hi / step) * step;
    }
    const k0 = Math.ceil(lo / step - 1e-9);
    const k1 = Math.floor(hi / step + 1e-9);
    const ticks: number[] = [];
    for (let k = k0; k <= k1 && ticks.length < MAX_TICKS; k += 1) {
        ticks.push(k * step);
    }
    return { lo, hi, step, ticks };
}

class Frame {
    constructor(
        public spec: Spec,
        public data: SpecData,
        public n: number,
        public left: number,
        public right: number,
        public top: number,
        public bottom: number,
        public y: Domain,
        public x: Domain | null,
        public look: Look,
        /** The palette drawn with: the spec's, else the look's. */
        public pal: string[],
    ) {}

    ys(v: number): number {
        return this.bottom - (v - this.y.lo) / (this.y.hi - this.y.lo) * (this.bottom - this.top);
    }

    xs(v: number): number {
        const d = this.x;
        return d ? this.left + (v - d.lo) / (d.hi - d.lo) * (this.right - this.left) : this.left;
    }

    band(i: number): number[] {
        const step = (this.right - this.left) / Math.max(this.n, 1);
        const p = this.spec.x.padding;
        return [this.left + step * i + step * p / 2, step * (1 - p)];
    }

    bandOfText(t: string): number | null {
        for (let i = 0; i < this.n; i++) {
            if (catText(this.data.cats[i]) === t) {
                return i;
            }
        }
        return null;
    }

    catNum(i: number): number | null {
        const c = this.data.cats[i];
        if (!c) {
            return null;
        }
        if (c.kind === "num" || c.kind === "date") {
            return c.n;
        }
        return this.spec.x.scale === "time" ? parseIso(c.s) : null;
    }

    xValue(e: Enc, i: number): number | null {
        if (e.f) {
            return e.f.k === "c" ? this.catNum(i) : value(this.data, e.f.i, i);
        }
        if (e.n !== undefined) {
            return e.n;
        }
        return e.s !== undefined ? parseIso(e.s) : null;
    }

    xPos(e: Enc, i: number): number | null {
        if (this.spec.x.scale === "band") {
            let row = i;
            if (e.s !== undefined) {
                const b = this.bandOfText(e.s);
                if (b === null) {
                    return null;
                }
                row = b;
            }
            const [x0, bw] = this.band(row);
            return x0 + bw / 2;
        }
        const v = this.xValue(e, i);
        return v === null ? null : this.xs(v);
    }

    yValue(e: Enc, i: number): number | null {
        if (e.f && e.f.k === "v") {
            return value(this.data, e.f.i, i);
        }
        return e.n !== undefined ? e.n : null;
    }

    base(m: Mark, i: number): number {
        let v = m.y2 ? this.yValue(m.y2, i) : null;
        if (v === null) {
            v = 0;
        }
        return this.ys(Math.min(Math.max(v, this.y.lo), this.y.hi));
    }

    refValue(r: Ref, i: number): number | null {
        return r.k === "v" ? value(this.data, r.i, i) : this.catNum(i);
    }

    tOf(r: Ref, i: number): number | null {
        const v = this.refValue(r, i);
        if (v === null) {
            return null;
        }
        let lo = Infinity;
        let hi = -Infinity;
        for (let j = 0; j < this.n; j++) {
            const x = this.refValue(r, j);
            if (x !== null) {
                lo = Math.min(lo, x);
                hi = Math.max(hi, x);
            }
        }
        return hi > lo ? (v - lo) / (hi - lo) : 1;
    }

    palette(i: number): string {
        const p = this.pal;
        return p[i % Math.max(p.length, 1)] || TEXT;
    }

    markColor(m: Mark, mi: number): string {
        return m.color && m.color.fixed ? m.color.fixed : this.palette(mi);
    }

    colorAt(m: Mark, mi: number, i: number): string {
        const c = m.color;
        if (!c) {
            return this.palette(mi);
        }
        if (c.fixed) {
            return c.fixed;
        }
        if (c.byCategory) {
            return this.palette(i);
        }
        const range = c.range || VALUE_RANGE;
        const t = c.byValue ? this.tOf(c.byValue, i) : null;
        return t === null ? range[0] : lerpColor(range[0], range[1], t);
    }

    sizeAt(m: Mark, i: number, dflt: number): number {
        const s = m.size;
        if (!s) {
            return dflt;
        }
        if (s.fixed !== undefined) {
            return s.fixed;
        }
        const range = s.range || [3, 12];
        const t = s.byValue ? this.tOf(s.byValue, i) : null;
        return t === null ? range[0] : range[0] + t * (range[1] - range[0]);
    }

    labelText(m: Mark, i: number): string | null {
        const r = m.label;
        if (!r) {
            return null;
        }
        if (r.k === "c") {
            const s = catLabel(this.data, i);
            return s === "" ? null : s;
        }
        const v = value(this.data, r.i, i);
        if (v === null) {
            return null;
        }
        // the text settings' display units and decimals over the mark's decimals
        const t = this.spec.text.labels || {};
        let maxAbs = 0;
        if (t.units === 0) {
            for (let j = 0; j < this.n; j++) {
                const x = value(this.data, r.i, j);
                if (x !== null) {
                    maxAbs = Math.max(maxAbs, Math.abs(x));
                }
            }
        }
        const d = typeof t.decimals === "number" ? t.decimals : m.decimals;
        return numText(v, d, unitOf(typeof t.units === "number" ? t.units : null, maxAbs), this.data.lang);
    }
}

export function text(x: number, y: number, s: string, size: number, fill: string, anchor: string, rotate: number): Prim {
    return { t: "text", x: r2(x), y: r2(y), text: s, size, fill, anchor, rotate };
}

export function line(x1: number, y1: number, x2: number, y2: number, stroke: string, width: number): Prim {
    return { t: "line", x1: r2(x1), y1: r2(y1), x2: r2(x2), y2: r2(y2), stroke, width: r2(width) };
}

export function rect(x: number, y: number, w: number, h: number, fill: string, opacity: number): Prim {
    return { t: "rect", x: r2(x), y: r2(y), w: r2(w), h: r2(h), fill, opacity };
}

/** A text element's drawing settings: the spec's `text` over the look's defaults. */
export interface TextSet { size: number; family: string; color: string; bold: boolean | null; italic: boolean; underline: boolean; }

/** The settings of text element `part` (`labels`, `x`, `y`, `xTitle`, `yTitle`, `legend`), `size` and `color` by default. */
export function textSet(spec: Spec, part: string, size: number, color: string): TextSet {
    const o = spec.text && spec.text[part] ? spec.text[part] : {};
    return {
        size: typeof o.size === "number" ? o.size : size,
        family: typeof o.family === "string" ? o.family : "",
        color: typeof o.color === "string" ? o.color : color,
        bold: typeof o.bold === "boolean" ? o.bold : null,
        italic: o.italic === true,
        underline: o.underline === true,
    };
}

/** `p` (a text primitive) with the font of `t`: its weight when bold is set, its family, italic and underline when set. */
export function styled(p: Prim, t: TextSet): Prim {
    if (t.bold === true) {
        p.weight = 700;
    } else if (t.bold === false) {
        delete p.weight;
    }
    if (t.family !== "") {
        p.family = t.family;
    }
    if (t.italic) {
        p.italic = true;
    }
    if (t.underline) {
        p.underline = true;
    }
    return p;
}

/** Whether mark `m` draws the first measure as one plain series (so it can be repeated for each other measure). */
function plainSeries(m: Mark): boolean {
    return (m.kind === "bar" || m.kind === "line" || m.kind === "area" || m.kind === "point")
        && !!m.y && !!m.y.f && m.y.f.k === "v" && m.y.f.i === 0
        && (m.x === null || (!!m.x.f && m.x.f.k === "c"))
        && m.x2 === null
        && (m.y2 === null || m.y2.n !== undefined)
        && (m.color === null || m.color.fixed !== undefined)
        && (m.size === null || m.size.fixed !== undefined)
        && (m.label === null || (m.label.k === "v" && m.label.i === 0));
}

/** Whether the spec's marks are plain series of the first measure, or marks that read no measure (rules at a value), with at least one series. */
function oneSeries(spec: Spec): boolean {
    if (spec.x.scale === "linear") {
        return false;
    }
    const reads = (m: Mark) => valuesUsed(Object.assign({}, spec, { marks: [m] }));
    return spec.marks.some(plainSeries) && spec.marks.every((m) => plainSeries(m) || reads(m) === 0);
}

/** A value field's name (its series name), else `value.N`. */
function seriesName(d: SpecData, k: number): string {
    const n = d.names && typeof d.names[k] === "string" ? d.names[k] : "";
    return n.trim() !== "" ? n : k === 0 ? "value" : `value.${k}`;
}

/**
 * The spec as drawn over `data` (docs/formats/spec-visual.md "Series" and "Text"): every bound
 * measure as its own series when the spec draws only the first one as one plain series; the
 * text settings' `show` over the axes, the axis titles, the legend and the value labels. Also
 * how many bound measures stay undrawn (the visual says so).
 */
export function resolve(spec: Spec, data: SpecData, pal: string[]): { spec: Spec; hidden: number } {
    const cols = Math.min(data.values.length, MAX_VALUES);
    const used = valuesUsed(spec);
    let marks = spec.marks;
    let hidden = Math.max(cols - used, 0);
    if (cols > 1 && used === 1 && oneSeries(spec)) {
        const plain = spec.marks.filter(plainSeries).length;
        const first = spec.marks.findIndex(plainSeries);
        const series = Math.min(cols, Math.max(Math.floor(MAX_SERIES_MARKS / plain), 1));
        const out: Mark[] = [];
        for (let k = 0; k < series; k++) {
            const c = pal[k % Math.max(pal.length, 1)] || TEXT;
            spec.marks.forEach((m, j) => {
                if (!plainSeries(m)) {
                    // a mark that reads no measure (a rule at a value) is drawn once
                    if (k === 0) {
                        out.push(m);
                    }
                    return;
                }
                out.push({
                    kind: m.kind,
                    name: j === first ? clip(seriesName(data, k), MAX_TEXT) : null,
                    x: m.x,
                    y: { f: { k: "v", i: k } },
                    x2: m.x2,
                    y2: m.y2,
                    color: k === 0 && m.color !== null ? m.color : { fixed: c },
                    size: m.size,
                    label: m.label !== null ? { k: "v", i: k } : null,
                    opacity: m.opacity,
                    decimals: m.decimals,
                });
            });
        }
        marks = out;
        hidden = cols - series;
    }
    const t = spec.text || {};
    const show = (part: string): boolean | null => (t[part] && typeof t[part].show === "boolean" ? t[part].show : null);
    const labels = show("labels");
    if (labels !== null) {
        marks = marks.map((m) => {
            if (m.kind === "text" || m.kind === "rule" || m.kind === "rect") {
                return m;
            }
            if (labels === false) {
                return Object.assign({}, m, { label: null });
            }
            const y = m.y && m.y.f && m.y.f.k === "v" ? m.y.f : null;
            return m.label === null && y !== null ? Object.assign({}, m, { label: y }) : m;
        });
    }
    const xs = show("x");
    const ys = show("y");
    const xt = show("xTitle");
    const yt = show("yTitle");
    const valueNames: string[] = [];
    for (let k = 0; k < Math.min(cols, Math.max(used, cols - hidden)); k++) {
        valueNames.push(seriesName(data, k));
    }
    const xTitle = xt === false ? null : spec.x.title !== null ? spec.x.title : xt === true && data.catName && data.catName.trim() !== "" ? data.catName : null;
    const yTitle = yt === false ? null : spec.y.title !== null ? spec.y.title : yt === true && valueNames.length > 0 ? valueNames.join(", ") : null;
    const x: Axis = Object.assign({}, spec.x, { axis: xs !== null ? xs : spec.x.axis, title: xTitle });
    const y: Axis = Object.assign({}, spec.y, { axis: ys !== null ? ys : spec.y.axis, title: yTitle });
    const lg = show("legend");
    return { spec: Object.assign({}, spec, { marks, x, y, legend: lg !== null ? lg : spec.legend }), hidden };
}

/** The line saying how many bound measures the spec leaves undrawn. */
function noticeText(hidden: number, cols: number, lang: string): string {
    return lang === "fr"
        ? `${hidden} mesure(s) sur ${cols} non dessinée(s) : la spécification ne les lit pas`
        : `${hidden} of ${cols} measures not drawn: the spec does not read them`;
}

/**
 * Lays out `spec` over `data` in a `w`×`h` visual (docs/formats/spec-visual.md "Layout"). With
 * `hits`, also lists the data points drawn (the primitives are the same with or without it).
 * `theme` is the report theme's colours, or `{colors, background}`.
 */
export function layout(given: Spec, data: SpecData, w: number, h: number, hits?: Hit[], theme?: any): Prim[] {
    const out: Prim[] = [];
    if (!isFinite(w) || !isFinite(h)) {
        return out;
    }
    const look = lookOf(given.style, Array.isArray(theme) ? { colors: theme } : theme && typeof theme === "object" ? theme : { colors: [] });
    const tick = look.tick;
    const name = look.name;
    const palette = given.ownPalette ? given.palette : look.palette;
    const { spec, hidden } = resolve(given, data, palette);
    const TX = textSet(spec, "x", tick, look.text);
    const TY = textSet(spec, "y", tick, look.text);
    const TXT = textSet(spec, "xTitle", name, look.text);
    const TYT = textSet(spec, "yTitle", name, look.text);
    const TL = textSet(spec, "legend", name, look.text);
    const n = rows(data);
    const pad = spec.padding;
    const pal = (i: number) => palette[i % Math.max(palette.length, 1)] || "";
    let items: string[][] = [];
    spec.marks.forEach((m, mi) => {
        if (m.name !== null) {
            items.push([m.name, m.color && m.color.fixed ? m.color.fixed : pal(mi)]);
        }
    });
    if (items.length === 0 && spec.marks.some((m) => !!(m.color && m.color.byCategory))) {
        items = [];
        for (let i = 0; i < Math.min(n, 20); i++) {
            items.push([catLabel(data, i), pal(i)]);
        }
    }
    let legend = (spec.legend === null ? true : spec.legend) && items.length > 0;
    // a line saying which measures are not drawn, then the legend row, then half the top y tick
    // label (10 px text centred on its tick) and 1 px; larger text takes more room
    let notice = hidden > 0 ? noticeText(hidden, Math.min(data.values.length, MAX_VALUES), data.lang) : null;
    const headOf = (n: boolean) => pad + (n ? 14 : 0);
    const topOf = (n: boolean) => headOf(n) + (legend ? 18 + (TL.size - name) : 0) + ((legend || n) && spec.y.axis ? 6 + (TY.size - tick) / 2 : 0);
    let head = headOf(notice !== null);
    let top = topOf(notice !== null);
    const left = pad + (spec.y.axis ? 36 * TY.size / tick : 0) + (spec.y.title !== null ? 14 + (TYT.size - name) : 0);
    const bottom = h - pad - (spec.x.axis ? 18 + (TX.size - tick) : 0) - (spec.x.title !== null ? 14 + (TXT.size - name) : 0);
    const right = w - pad;
    if (notice !== null && (right - left < 8 || bottom - top < 8)) {
        // a visual too small for the notice still draws its plot
        notice = null;
        head = headOf(false);
        top = topOf(false);
    }
    if (legend && (right - left < 8 || bottom - top < 8)) {
        // nor for the legend
        legend = false;
        top = topOf(false);
    }
    if (right - left < 8 || bottom - top < 8) {
        return out;
    }
    pushCard(out, look, w, h);
    const yv: number[] = [];
    for (const m of spec.marks) {
        for (const e of [m.y, m.y2]) {
            if (e === null) {
                continue;
            }
            if (e.f && e.f.k === "v") {
                for (let i = 0; i < n; i++) {
                    const v = value(data, e.f.i, i);
                    if (v !== null) {
                        yv.push(v);
                    }
                }
            } else if (e.n !== undefined) {
                yv.push(e.n);
            }
        }
    }
    const f = new Frame(spec, data, n, left, right, top, bottom, domain(yv, spec.y.zero, spec.y.min, spec.y.max, 5), null, look, palette);
    if (spec.x.scale !== "band") {
        const xv: number[] = [];
        for (const m of spec.marks) {
            for (const e of [m.x, m.x2]) {
                if (e === null) {
                    continue;
                }
                for (let i = 0; i < n; i++) {
                    const v = f.xValue(e, i);
                    if (v !== null) {
                        xv.push(v);
                    }
                }
            }
        }
        f.x = domain(xv, spec.x.zero, spec.x.min, spec.x.max, spec.x.scale === "time" ? 4 : 5);
    }
    if (spec.y.grid) {
        for (const t of f.y.ticks) {
            const yy = f.ys(t);
            out.push(line(left, yy, right, yy, look.grid, 1));
        }
    }
    if (spec.x.grid && f.x) {
        for (const t of f.x.ticks) {
            const xx = f.xs(t);
            out.push(line(xx, top, xx, bottom, look.grid, 1));
        }
    }
    const bars: number[] = [];
    spec.marks.forEach((m, i) => {
        if (m.kind === "bar") {
            bars.push(i);
        }
    });
    spec.marks.forEach((m, mi) => mark(f, m, mi, bars, out, hits));
    if (spec.y.axis) {
        const ty = spec.text.y || {};
        const u = unitOf(typeof ty.units === "number" ? ty.units : null, Math.max(Math.abs(f.y.lo), Math.abs(f.y.hi)));
        const d = typeof ty.decimals === "number" ? ty.decimals : tickDecimals(f.y.step / u);
        for (const t of f.y.ticks) {
            out.push(styled(text(left - 4, f.ys(t), numText(t, d, u, data.lang), TY.size, TY.color, "end", 0), TY));
        }
    }
    if (spec.x.axis) {
        out.push(line(left, bottom, right, bottom, look.axis, 1));
        const ly = bottom + 9 + (TX.size - tick) / 2;
        const dx = f.x;
        if (dx === null) {
            const labels: string[] = [];
            for (let i = 0; i < n; i++) {
                labels.push(clip(catLabel(data, i), 20));
            }
            let maxw = 0;
            for (const l of labels) {
                maxw = Math.max(maxw, est(l, TX.size));
            }
            maxw += 6;
            const k = Math.max(Math.ceil(n * maxw / (right - left)), 1);
            labels.forEach((l, i) => {
                if (i % k === 0) {
                    const [x0, bw] = f.band(i);
                    out.push(styled(text(inside(x0 + bw / 2, l, TX.size, w), ly, l, TX.size, TX.color, "middle", 0), TX));
                }
            });
        } else {
            const d = tickDecimals(dx.step);
            for (const t of dx.ticks) {
                // a tick of days or more: in the category's date format
                const label = spec.x.scale === "time" && dx.step >= MS_PER_DAY ? dateLabel(t, data.catFormat, data.lang) : null;
                const s = spec.x.scale === "time" ? (label !== null ? label : dateText(t, dx.step < MS_PER_DAY)) : localNum(fmtFixed(t, d), data.lang);
                out.push(styled(text(inside(f.xs(t), s, TX.size, w), ly, s, TX.size, TX.color, "middle", 0), TX));
            }
        }
    }
    if (spec.y.title !== null) {
        out.push(styled(text(pad + 7 + (TYT.size - name) / 2, (top + bottom) / 2, clip(spec.y.title, 60), TYT.size, TYT.color, "middle", -90), TYT));
    }
    if (spec.x.title !== null) {
        out.push(styled(text((left + right) / 2, h - pad - 7 - (TXT.size - name) / 2, clip(spec.x.title, 60), TXT.size, TXT.color, "middle", 0), TXT));
    }
    if (notice !== null) {
        out.push(text(pad, pad + 6, clip(notice, Math.max(Math.floor((w - 2 * pad) / (tick * 0.55)), 1)), tick, look.text, "start", 0));
    }
    if (legend) {
        let x = pad;
        const ly = head + (TL.size - name) / 2;
        for (let j = 0; j < items.length; j++) {
            const label = clip(items[j][0], 30);
            const wd = 14 + est(label, TL.size) + 12;
            if (j > 0 && x + wd - 12 > w - pad) {
                break;
            }
            out.push(swatch(look, x, ly + 4, 10, items[j][1]));
            out.push(styled(text(x + 14, ly + 9, label, TL.size, TL.color, "start", 0), TL));
            x += wd;
        }
    }
    return out;
}

function mark(f: Frame, m: Mark, mi: number, bars: number[], out: Prim[], hits?: Hit[]): void {
    const n = f.n;
    // each label's primitives (a pill, then its text), the index of its text and its row
    const labels: [Prim[], number, number][] = [];
    // the data labels' text settings (their colour by default: the look's ink, on a chip when it has one)
    const tl = textSet(f.spec, "labels", f.look.value, "");
    const label = (i: number, x: number, y: number, side: string, gap: number) => {
        const s = f.labelText(m, i);
        if (s !== null) {
            const ps: Prim[] = [];
            const t = valueLabel(ps, f.look, x, y, s, side, gap, tl.size);
            if (tl.color !== "") {
                ps[t].fill = tl.color;
            }
            styled(ps[t], tl);
            labels.push([ps, t, i]);
        }
    };
    // the primitive just pushed is row i's (`fade`: dimmed when the row is not highlighted)
    const hitAs = (i: number, fade: boolean) => {
        if (hits) {
            hits.push({ row: i, prim: out.length - 1, fade });
        }
    };
    const hit = (i: number) => hitAs(i, faded(f.data, i));
    if (m.kind === "bar" || m.kind === "rect") {
        let nb = 1;
        let j = 0;
        if (m.kind === "bar") {
            nb = Math.max(bars.length, 1);
            j = Math.max(bars.indexOf(mi), 0);
        }
        for (let i = 0; i < n; i++) {
            const v = m.y ? f.yValue(m.y, i) : null;
            if (v === null) {
                continue;
            }
            let x0: number;
            let x1: number;
            if (f.x === null) {
                const [b0, bw] = f.band(i);
                const sub = bw / nb;
                x0 = b0 + sub * j;
                x1 = b0 + sub * (j + 1);
            } else if (m.x && m.x2) {
                const a = f.xValue(m.x, i);
                const b = f.xValue(m.x2, i);
                if (a === null || b === null) {
                    continue;
                }
                const pa = f.xs(a);
                const pb = f.xs(b);
                x0 = Math.min(pa, pb);
                x1 = Math.max(pa, pb);
            } else {
                continue;
            }
            const base = f.base(m, i);
            const c = f.colorAt(m, mi, i);
            const draw = (val: number): Prim => {
                const yv = f.ys(val);
                const y0 = Math.min(yv, base);
                const y1 = Math.max(yv, base);
                const p = m.kind === "bar" ? bar(f.look, x0, y0, x1 - x0, y1 - y0, c, yv <= base ? "b" : "t") : tile(f.look, x0, y0, x1 - x0, y1 - y0, c, radius(f.look) / 2, false);
                p.opacity = m.opacity;
                return p;
            };
            // highlighted: a bar of a value from zero draws its highlighted part over the dimmed
            // full value; other bars are dimmed with their row
            const k = m.y && m.y.f && m.y.f.k === "v" && !m.y2 && highlighted(f.data) ? m.y.f.i : -1;
            if (k >= 0) {
                out.push(dim(draw(v)));
                hitAs(i, false);
                const pv = part(v, highlightOf(f.data, k, i));
                if (pv !== null) {
                    out.push(draw(pv));
                    hitAs(i, false);
                }
            } else {
                out.push(draw(v));
                hit(i);
            }
            const yv = f.ys(v);
            const y0 = Math.min(yv, base);
            const y1 = Math.max(yv, base);
            if (yv <= base) {
                label(i, (x0 + x1) / 2, y0, "n", 7);
            } else {
                label(i, (x0 + x1) / 2, y1, "s", 7);
            }
        }
    } else if (m.kind === "line" || m.kind === "area") {
        const runs: number[][][] = [[]];
        for (let i = 0; i < n; i++) {
            let p: number[] | null = null;
            if (m.x && m.y) {
                const x = f.xPos(m.x, i);
                const v = f.yValue(m.y, i);
                if (x !== null && v !== null) {
                    p = [x, f.ys(v)];
                }
            }
            const last = runs[runs.length - 1];
            if (p !== null) {
                last.push([r2(p[0]), r2(p[1]), i]);
            } else if (last.length > 0) {
                runs.push([]);
            }
        }
        const c = f.markColor(m, mi);
        for (const r of runs) {
            if (r.length === 0) {
                continue;
            }
            if (m.kind === "line") {
                const modern = !minimal(f.look);
                if (r.length >= 2) {
                    out.push({ t: "path", points: r.map((q) => [q[0], q[1]]), stroke: c, width: r2(f.sizeAt(m, 0, modern ? 2.5 : 2)) });
                }
                // Modern and Bold: a marker on each point of a short line, ringed in the surface
                if (modern && r.length <= MAX_LINE_DOTS) {
                    for (const q of r) {
                        out.push(dot(f.look, q[0], q[1], 3.5, c, 1));
                    }
                }
            } else {
                const pts = r.map((q) => [q[0], q[1]]);
                for (let k = r.length - 1; k >= 0; k--) {
                    pts.push([r[k][0], r2(f.base(m, r[k][2]))]);
                }
                out.push({ t: "poly", points: pts, fill: c, opacity: m.opacity });
            }
            for (const q of r) {
                if (hits) {
                    hits.push({ row: q[2], prim: -1, x: q[0], y: q[1], fade: faded(f.data, q[2]) });
                }
                label(q[2], q[0], q[1], "n", 8);
            }
        }
    } else if (m.kind === "point" || m.kind === "text") {
        for (let i = 0; i < n; i++) {
            if (!m.x || !m.y) {
                break;
            }
            const x = f.xPos(m.x, i);
            const v = f.yValue(m.y, i);
            if (x === null || v === null) {
                continue;
            }
            const y = f.ys(v);
            if (m.kind === "point") {
                const r = f.sizeAt(m, i, 4);
                out.push(dot(f.look, x, y, r, f.colorAt(m, mi, i), m.opacity));
                hit(i);
                label(i, x, y, "n", r + 6);
            } else {
                const s = f.labelText(m, i);
                if (s !== null) {
                    out.push(text(x, y, s, r2(f.sizeAt(m, i, 11)), f.colorAt(m, mi, i), "middle", 0));
                    hit(i);
                }
            }
        }
    } else if (m.kind === "rule") {
        const c = f.markColor(m, mi);
        const wd = f.sizeAt(m, 0, 1);
        if (m.x === null && m.y !== null && m.y.n !== undefined) {
            const y = f.ys(m.y.n);
            out.push(line(f.left, y, f.right, y, c, wd));
        } else if (m.x === null && m.y !== null) {
            for (let i = 0; i < n; i++) {
                const v = f.yValue(m.y, i);
                if (v === null) {
                    continue;
                }
                const y = f.ys(v);
                let x0: number;
                let x1: number;
                if (f.x === null) {
                    const [b0, bw] = f.band(i);
                    x0 = b0;
                    x1 = b0 + bw;
                } else {
                    const x = f.xPos({ f: { k: "c", i: 0 } }, i);
                    if (x === null) {
                        continue;
                    }
                    x0 = x - 5;
                    x1 = x + 5;
                }
                out.push(line(x0, y, x1, y, c, wd));
                hit(i);
            }
        } else if (m.x !== null && m.y === null && (m.x.n !== undefined || m.x.s !== undefined)) {
            const x = f.xPos(m.x, 0);
            if (x !== null) {
                out.push(line(x, f.top, x, f.bottom, c, wd));
            }
        } else if (m.x !== null && m.y === null) {
            for (let i = 0; i < n; i++) {
                const x = f.xPos(m.x, i);
                if (x !== null) {
                    out.push(line(x, f.top, x, f.bottom, c, wd));
                }
            }
        }
    }
    for (const [ps, t, row] of labels) {
        const at = out.length;
        out.push(...ps);
        if (hits) {
            hits.push({ row, prim: at + t, label: true, fade: faded(f.data, row) });
        }
    }
}
