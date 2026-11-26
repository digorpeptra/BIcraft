// BIcraft Line / Area+ (docs/formats/chart-pack.md "Line / Area+"): lines or areas by series on
// a continuous date axis (or a numeric or category axis), with a band between two measures
// coloured by which one is above, shading beyond a threshold, an area fill formatted apart from
// its line, labels on all, the last, or the lowest and highest points, line colours and widths
// by series or by rule, a dashed stretch from a forecast date on, and series named at the end of
// their line instead of in a legend. The TypeScript twin of
// crates/render/src/chart_pack/linearea.rs; its date ticks follow the steps of
// crates/render/src/timeaxis.rs (`stepsOf` here is the twin of `steps_of` there).
"use strict";

import { MS_PER_DAY, Out, PAD, Prim, cellLabel, cellNum, cellText, circle, civilFromDays, compact, dashes, daysFromCivil, est, fit, highlighted, hlNum, langOf, legend, lookOfInput, line, optBool, optColor, optEnum, optNum, optText, optionsOf, parseIso, poly, r2, rowsOf, text, dateText } from "./kit";
import { Domain, domain, inside, tickDecimals } from "./spec";
import { color, dim, dot, lineWidth, mix, pushCard, textW, valueLabel } from "./runtime/style";
import { dateLabel } from "./runtime/format";

export const MAX_ROWS = 20000;
export const MAX_SERIES = 12;
/** Most points of one series (distinct axis values). */
export const MAX_POINTS = 2000;
/** Most points with their own hit target (above it, each line is one target). */
export const MAX_HIT_POINTS = 3000;
/** Most labels of one series in the `all` mode (above it, none). */
export const MAX_ALL_LABELS = 60;

// ---------------------------------------------------------------------------------------------
// Date ticks: the steps of crates/render/src/timeaxis.rs, over OLE serials (days)

/** The OLE serial of 1970-01-01. */
const EPOCH = 25569;
/** Most ticks one step may generate. */
const MAX_TICKS = 400;
const STEPS: [string, number][] = [
    ["minute", 1], ["minute", 5], ["minute", 15], ["minute", 30],
    ["hour", 1], ["hour", 3], ["hour", 6], ["hour", 12],
    ["day", 1], ["day", 2], ["week", 1],
    ["month", 1], ["month", 2], ["month", 3], ["month", 6],
    ["year", 1], ["year", 2], ["year", 5], ["year", 10], ["year", 20], ["year", 50], ["year", 100], ["year", 200], ["year", 500], ["year", 1000],
];
const MONTHS_EN = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const MONTHS_FR = ["janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.", "déc."];

function approxDays(u: string, k: number): number {
    const per = u === "minute" ? 1 / 1440 : u === "hour" ? 1 / 24 : u === "day" ? 1 : u === "week" ? 7 : u === "month" ? 30.44 : 365.25;
    return k * per;
}

function civilOf(serial: number): number[] {
    return civilFromDays(serial - EPOCH);
}

function serialOf(y: number, m: number, d: number): number {
    return daysFromCivil(y, m, d) + EPOCH;
}

function mod(a: number, b: number): number {
    return ((a % b) + b) % b;
}

/** The serials of step (`u`, `k`) within [`a`, `b`] (the twin of timeaxis.rs `steps_of`). */
export function stepsOf(u: string, k: number, a: number, b: number): number[] {
    const out: number[] = [];
    if (!isFinite(a) || !isFinite(b) || b < a || k <= 0 || b - a > 4e6) {
        return out;
    }
    if (u === "minute" || u === "hour") {
        const per = u === "minute" ? k : k * 60;
        let m = Math.ceil((a * 1440) / per);
        while (out.length < MAX_TICKS) {
            const x = (m * per) / 1440;
            if (x > b + 1e-9) {
                break;
            }
            out.push(x);
            m += 1;
        }
    } else if (u === "day" || u === "week") {
        const d0 = Math.ceil(a);
        const end = Math.floor(b);
        let d = d0;
        while (d <= end && out.length < MAX_TICKS && d - d0 < 8 * MAX_TICKS) {
            // serial 1 (1899-12-31) was a Sunday
            const keep = u === "week" ? mod(d, 7) === 1 : (civilOf(d)[2] - 1) % k === 0;
            if (keep) {
                out.push(d);
            }
            d += 1;
        }
    } else if (u === "month") {
        const c = civilOf(Math.floor(a));
        let y = c[0];
        let m = c[1];
        const n = Math.min(MAX_TICKS * k + 2, 20000);
        for (let i = 0; i < n; i++) {
            const x = serialOf(y, m, 1);
            if (x > b) {
                break;
            }
            if (x >= a && (m - 1) % k === 0) {
                out.push(x);
            }
            if (m >= 12) {
                y += 1;
                m = 1;
            } else {
                m += 1;
            }
        }
    } else {
        let y = civilOf(Math.floor(a))[0];
        for (let i = 0; i < 20000; i++) {
            const x = serialOf(y, 1, 1);
            if (x > b || out.length >= MAX_TICKS) {
                break;
            }
            if (x >= a && mod(y, k) === 0) {
                out.push(x);
            }
            y += 1;
        }
    }
    return out;
}

/** A day tick: the column's date format when it writes at most 12 characters, else ISO; `short`, the day and the month's short name ("Jan 5", "5 janv."). */
function dayText(x: number, fmt: string, lang: string, short: boolean): string {
    const day = Math.floor(x);
    if (short) {
        const c = civilOf(day);
        return lang === "fr" ? `${c[2]} ${MONTHS_FR[c[1] - 1]}` : `${MONTHS_EN[c[1] - 1]} ${c[2]}`;
    }
    const ms = (day - EPOCH) * MS_PER_DAY;
    const l = fmt !== "" ? dateLabel(ms, fmt, lang) : null;
    return l !== null && Array.from(l).length <= 12 ? l : dateText(ms, false);
}

function tickLabel(u: string, x: number, fmt: string, lang: string, short: boolean): string {
    const c = civilOf(Math.floor(x));
    if (u === "year") {
        return String(c[0]);
    }
    if (u === "month") {
        return `${(lang === "fr" ? MONTHS_FR : MONTHS_EN)[c[1] - 1]} ${c[0]}`;
    }
    if (u === "day" || u === "week") {
        return dayText(x, fmt, lang, short);
    }
    const whole = Math.floor(x + 0.5);
    if (Math.abs(x - whole) < 1e-7) {
        return dayText(whole, fmt, lang, short);
    }
    return Array.from(dateText((x - EPOCH) * MS_PER_DAY, true)).slice(11).join("");
}

/** The ticks of a date axis from serial `lo` to `hi` over [`x0`, `x1`]: the finest step whose labels keep 8 px apart (steps finer than the data's smallest `gap` skipped). */
function timeTicks(lo: number, hi: number, gap: number, x0: number, x1: number, size: number, fmt: string, lang: string): [number, string][] {
    const s = (x1 - x0) / (hi - lo);
    const pos = (t: number) => x0 + (t - lo) * s;
    let last: [number, string][] = [];
    for (const [u, k] of STEPS) {
        const d = approxDays(u, k);
        if (d < gap * 0.9 || (hi - lo) / d > MAX_TICKS) {
            continue;
        }
        const ts = stepsOf(u, k, lo, hi);
        if (ts.length === 0) {
            continue;
        }
        const shorts = u === "month" || u === "year" ? [false] : [false, true];
        for (const short of shorts) {
            const ticks: [number, string][] = ts.map((t) => [pos(t), tickLabel(u, t, fmt, lang, short)]);
            let widest = 0;
            let room = x1 - x0;
            for (let i = 0; i < ticks.length; i++) {
                widest = Math.max(widest, est(ticks[i][1], size));
                if (i > 0) {
                    room = Math.min(room, ticks[i][0] - ticks[i - 1][0]);
                }
            }
            if (room >= widest + 8) {
                return ticks;
            }
            last = ticks;
        }
    }
    if (last.length > 0) {
        return [last[0]];
    }
    return [[x0, dayText(lo, fmt, lang, false)]];
}

// ---------------------------------------------------------------------------------------------
// The layout

interface LP {
    key: number;
    v: number | null;
    v2: number | null;
    hl: number | null;
    row: number;
    /** The point's own value (`v` is its top when stacked). */
    own: number | null;
    /** Stacked: the top of the series below at this point (0 for the first). */
    base: number;
}

function pathP(points: number[][], stroke: string, width: number): Prim {
    return { t: "path", points: points.map((q) => [r2(q[0]), r2(q[1])]), stroke, width: r2(width) };
}

/**
 * Shades between `a` and `b` (per point, null breaks a run) over the points' `xs`: each run where
 * `a - b` keeps its sign is one polygon, split where the two cross, in `colorOf(sign)` (null: not
 * shaded).
 */
function shade(p: Prim[], xs: number[], a: (number | null)[], b: (number | null)[], sy: (v: number) => number, colorOf: (sign: number) => string | null, opacity: number): void {
    let top: number[][] = [];
    let bot: number[][] = [];
    let sign = 0;
    const flush = () => {
        const c = sign !== 0 && top.length >= 2 ? colorOf(sign) : null;
        if (c !== null) {
            p.push(poly(top.concat(bot.slice().reverse()), c, opacity));
        }
        top = [];
        bot = [];
        sign = 0;
    };
    let px = 0;
    let pa = 0;
    let pb = 0;
    for (let i = 0; i < xs.length; i++) {
        const va = a[i];
        const vb = b[i];
        if (va === null || vb === null) {
            flush();
            continue;
        }
        const d = va - vb;
        const s = d > 0 ? 1 : d < 0 ? -1 : 0;
        if (top.length > 0 && s !== 0 && sign !== 0 && s !== sign) {
            const d0 = pa - pb;
            const t = d0 / (d0 - d);
            const xc = px + (xs[i] - px) * t;
            const yc = sy(pa + (va - pa) * t);
            top.push([xc, yc]);
            bot.push([xc, yc]);
            flush();
            top.push([xc, yc]);
            bot.push([xc, yc]);
        }
        top.push([xs[i], sy(va)]);
        bot.push([xs[i], sy(vb)]);
        if (s !== 0) {
            sign = s;
        }
        px = xs[i];
        pa = va;
        pb = vb;
    }
    flush();
}

/** Lays out `{rows: [[axis, series, value, value2]], formats, options, today?}` in a `w`×`h` visual. */
export function layoutLineArea(input: any, w: number, h: number): Out {
    const o = optionsOf(input);
    const k = lookOfInput(input);
    const lang = langOf(input);
    const out: Out = { prims: [], hits: [] };
    const p = out.prims;
    const rows = rowsOf(input, MAX_ROWS);
    // the axis: dates, numbers, else categories
    const axisType = optEnum(o, "axisType", ["auto", "category"], "auto");
    let any = false;
    let dates = true;
    let nums = true;
    for (const r of rows) {
        const c = r[0];
        if (c === null || c === undefined) {
            continue;
        }
        any = true;
        if (!(typeof c === "string" && parseIso(c) !== null)) {
            dates = false;
        }
        if (cellNum(c) === null) {
            nums = false;
        }
    }
    const kind = axisType === "category" || !any ? "category" : dates ? "time" : nums ? "number" : "category";
    const cats: string[] = [];
    const catIx = new Map<string, number>();
    const series: string[] = [];
    const serIx = new Map<string, number>();
    const pts: LP[][] = [];
    const keyIx: Map<number, number>[] = [];
    const lit = highlighted(input);
    for (let i = 0; i < rows.length; i++) {
        const c = rows[i][0];
        let key: number;
        if (kind === "time") {
            const ms = typeof c === "string" ? parseIso(c) : null;
            if (ms === null) {
                continue;
            }
            key = ms + 0;
        } else if (kind === "number") {
            const n = cellNum(c);
            if (n === null) {
                continue;
            }
            key = n + 0;
        } else {
            const t = cellText(c);
            let ci = catIx.get(t);
            if (ci === undefined) {
                if (cats.length >= MAX_POINTS) {
                    continue;
                }
                ci = cats.length;
                catIx.set(t, ci);
                cats.push(cellLabel(input, 0, c));
            }
            key = ci;
        }
        const sk = cellText(rows[i][1]);
        let si = serIx.get(sk);
        if (si === undefined) {
            if (series.length >= MAX_SERIES) {
                continue;
            }
            si = series.length;
            serIx.set(sk, si);
            series.push(cellLabel(input, 1, rows[i][1]));
            pts.push([]);
            keyIx.push(new Map());
        }
        let pi = keyIx[si].get(key);
        if (pi === undefined) {
            if (pts[si].length >= MAX_POINTS) {
                continue;
            }
            pi = pts[si].length;
            keyIx[si].set(key, pi);
            pts[si].push({ key, v: null, v2: null, hl: null, row: i, own: null, base: 0 });
        }
        const q = pts[si][pi];
        const v = cellNum(rows[i][2]);
        const v2 = cellNum(rows[i][3]);
        const hv = hlNum(input, i);
        if (v !== null) {
            q.v = (q.v === null ? 0 : q.v) + v;
        }
        if (v2 !== null) {
            q.v2 = (q.v2 === null ? 0 : q.v2) + v2;
        }
        if (hv !== null) {
            q.hl = (q.hl === null ? 0 : q.hl) + hv;
        }
    }
    for (const l of pts) {
        l.sort((a, b) => a.key - b.key);
    }
    // stacked: each series' values on the ones before it at the same axis value (rises on the
    // rises, falls under the falls), no second measure; labels keep each point's own value
    const stacked = optBool(o, "stacked", false) && pts.length > 1;
    const ups = new Map<number, number>();
    const downs = new Map<number, number>();
    for (const l of pts) {
        for (const q of l) {
            q.own = q.v;
            if (!stacked) {
                continue;
            }
            q.v2 = null;
            if (q.v === null) {
                continue;
            }
            const m = q.v >= 0 ? ups : downs;
            const b = m.has(q.key) ? (m.get(q.key) as number) : 0;
            q.base = b;
            q.v = b + q.v;
            if (q.hl !== null) {
                q.hl = b + q.hl;
            }
            m.set(q.key, q.v);
        }
    }
    const vals: number[] = [];
    let hasV2 = false;
    let count = 0;
    for (const l of pts) {
        for (const q of l) {
            if (q.v !== null) {
                vals.push(q.v);
                count++;
            }
            if (q.v2 !== null) {
                vals.push(q.v2);
                hasV2 = true;
            }
        }
    }
    if (count === 0) {
        return out;
    }
    const area = optBool(o, "area", false);
    const areaOpacity = optNum(o, "areaOpacity", 0, 100, 25) / 100;
    const areaFill = optColor(o, "areaFill", "");
    const lineColor = optColor(o, "lineColor", "");
    const width = optNum(o, "lineWidth", 0.5, 8, lineWidth(k));
    const markers = optBool(o, "markers", false);
    const labelMode = optEnum(o, "labelMode", ["last", "minmax", "all", "none"], "last");
    const fontSize = optNum(o, "fontSize", 8, 20, k.value);
    const endLabels = optBool(o, "endLabels", true);
    const showLegend = optBool(o, "showLegend", true);
    const band = optBool(o, "band", true);
    const bandUp = optColor(o, "bandUp", "#0E9F84");
    const bandDown = optColor(o, "bandDown", "#E23D4F");
    const bandOpacity = optNum(o, "bandOpacity", 5, 100, 30) / 100;
    const threshold = optBool(o, "threshold", false);
    const tv = optNum(o, "thresholdValue", -1e15, 1e15, 0);
    const thresholdSide = optEnum(o, "thresholdSide", ["above", "below"], "above");
    const thresholdColor = optColor(o, "thresholdColor", "#E23D4F");
    const colorRule = optEnum(o, "colorRule", ["none", "above", "below"], "none");
    const rv = optNum(o, "ruleValue", -1e15, 1e15, 0);
    const ruleColor = optColor(o, "ruleColor", "#E23D4F");
    const emphasize = optText(o, "emphasize", 200);
    const forecastFrom = optText(o, "forecastFrom", 40);
    const yZero = optBool(o, "yZero", area || stacked);
    if (threshold) {
        vals.push(tv);
    }
    const yd = domain(vals, yZero, null, null, 5);
    const named = !(series.length === 1 && series[0] === "");
    const colors = series.map((s, i) => (lineColor !== "" && !named ? lineColor : color(k, i)));
    pushCard(p, k, w, h);
    let top = PAD;
    if (named && !endLabels && showLegend) {
        legend(p, k, series, colors, PAD, PAD + 4, w - PAD);
        top = PAD + 20;
    }
    top += labelMode !== "none" ? fontSize + 6 : 6;
    const yLabels = yd.ticks.map((t) => compact(t, tickDecimals(yd.step)));
    let lw = 0;
    for (const l of yLabels) {
        lw = Math.max(lw, est(l, k.tick));
    }
    const x0 = PAD + Math.max(lw, 10) + 6;
    // the end labels' room
    const ends: string[] = series.map((s) => fit(s, k.name, 110));
    let endW = 0;
    if (named && endLabels) {
        for (const s of ends) {
            endW = Math.max(endW, est(s, k.name));
        }
        endW = endW > 0 ? Math.min(endW + 20, 130) : 0;
    }
    const x1 = w - PAD - (endW > 0 ? endW : 4);
    const y0 = top;
    const y1 = h - PAD - 18;
    if (x1 - x0 < 20 || y1 - y0 < 20) {
        return out;
    }
    const sy = (v: number) => y1 - ((v - yd.lo) / (yd.hi - yd.lo)) * (y1 - y0);
    // the x scale
    let lo = 0;
    let hi = 1;
    let gap = 1;
    let xd: Domain | null = null;
    if (kind === "time") {
        const keys: number[] = [];
        for (const l of pts) {
            for (const q of l) {
                keys.push(q.key / MS_PER_DAY + EPOCH);
            }
        }
        keys.sort((a, b) => a - b);
        lo = keys[0];
        hi = keys[keys.length - 1];
        gap = Infinity;
        for (let i = 1; i < keys.length; i++) {
            if (keys[i] - keys[i - 1] > 1e-9) {
                gap = Math.min(gap, keys[i] - keys[i - 1]);
            }
        }
        if (!(hi > lo)) {
            lo -= 1;
            hi += 1;
        }
        if (!isFinite(gap)) {
            gap = 1;
        }
    } else if (kind === "number") {
        const keys: number[] = [];
        for (const l of pts) {
            for (const q of l) {
                keys.push(q.key);
            }
        }
        xd = domain(keys, false, null, null, 5);
    }
    const n = cats.length;
    const sx = (key: number): number => {
        if (kind === "time") {
            return x0 + ((key / MS_PER_DAY + EPOCH - lo) * (x1 - x0)) / (hi - lo);
        }
        if (xd !== null) {
            return x0 + ((key - xd.lo) / (xd.hi - xd.lo)) * (x1 - x0);
        }
        return n <= 1 ? (x0 + x1) / 2 : x0 + (key * (x1 - x0)) / (n - 1);
    };
    // grid, y ticks, x ticks
    for (let i = 0; i < yd.ticks.length; i++) {
        const yy = sy(yd.ticks[i]);
        p.push(line(x0, yy, x1, yy, k.grid, 1));
        p.push(text(x0 - 4, yy, yLabels[i], k.tick, k.text, "end"));
    }
    let xt: [number, string][] = [];
    if (kind === "time") {
        const fmts: any[] = input && Array.isArray(input.formats) ? input.formats : [];
        const fmt = typeof fmts[0] === "string" ? Array.from(fmts[0] as string).slice(0, 200).join("") : "";
        xt = timeTicks(lo, hi, gap, x0, x1, k.tick, fmt, lang);
    } else if (xd !== null) {
        const d = xd;
        xt = d.ticks.map((t) => [sx(t), compact(t, tickDecimals(d.step))]);
    } else {
        xt = cats.map((c, i) => [sx(i), c]);
    }
    let widest = 0;
    for (const t of xt) {
        widest = Math.max(widest, est(t[1], k.tick));
    }
    const room = xt.length > 1 ? (x1 - x0) / (xt.length - 1) : x1 - x0;
    const every = kind === "time" ? 1 : Math.max(1, Math.ceil((widest + 6) / Math.max(room, 1)));
    for (let i = 0; i < xt.length; i++) {
        if (i % every !== 0) {
            continue;
        }
        const s = kind === "category" ? fit(xt[i][1], k.tick, Math.max(room * every - 4, 0)) : xt[i][1];
        if (s === "") {
            continue;
        }
        p.push(line(xt[i][0], y1, xt[i][0], y1 + 3, k.axis, 1));
        p.push(text(inside(xt[i][0], s, k.tick, w), y1 + 10, s, k.tick, k.text, "middle"));
    }
    p.push(line(x0, y1, x1, y1, k.axis, 1));
    // the forecast cutoff (dates only, inside the data's range)
    let cutoff: number | null = null;
    if (kind === "time" && forecastFrom !== "") {
        const ms = forecastFrom === "today" ? (input && typeof input.today === "number" && isFinite(input.today) ? input.today : null) : parseIso(forecastFrom);
        if (ms !== null) {
            const sr = ms / MS_PER_DAY + EPOCH;
            if (sr > lo && sr <= hi) {
                cutoff = sx(ms);
            }
        }
    }
    // emphasis: one series wider, the others muted
    let hot = -1;
    if (emphasize !== "") {
        hot = series.indexOf(emphasize);
    }
    const xs = pts.map((l) => l.map((q) => sx(q.key)));
    // shading beyond the threshold and between the two measures
    if (threshold) {
        for (let si = 0; si < pts.length; si++) {
            shade(p, xs[si], pts[si].map((q) => q.v), pts[si].map(() => tv), sy, (s) => ((thresholdSide === "above" ? s > 0 : s < 0) ? thresholdColor : null), 0.25);
        }
    }
    if (hasV2 && band) {
        for (let si = 0; si < pts.length; si++) {
            shade(p, xs[si], pts[si].map((q) => q.v), pts[si].map((q) => q.v2), sy, (s) => (s > 0 ? bandUp : bandDown), bandOpacity);
        }
    }
    // areas, under each run of the line
    const base = sy(Math.min(Math.max(0, yd.lo), yd.hi));
    if (area) {
        for (let si = 0; si < pts.length; si++) {
            let run: number[][] = [];
            // stacked: down to the series below, else to zero
            let low: number[][] = [];
            const flush = () => {
                if (run.length >= 2) {
                    const shape = stacked ? run.concat(low.slice().reverse()) : run.concat([[run[run.length - 1][0], base], [run[0][0], base]]);
                    const m = poly(shape, areaFill !== "" ? areaFill : colors[si], areaOpacity);
                    p.push(lit ? dim(m) : m);
                }
                run = [];
                low = [];
            };
            for (let i = 0; i < pts[si].length; i++) {
                const v = pts[si][i].v;
                if (v === null) {
                    flush();
                } else {
                    run.push([xs[si][i], sy(v)]);
                    low.push([xs[si][i], sy(Math.min(Math.max(pts[si][i].base, yd.lo), yd.hi))]);
                }
            }
            flush();
        }
    }
    // the second measure's lines
    if (hasV2) {
        for (let si = 0; si < pts.length; si++) {
            let run: number[][] = [];
            const flush = () => {
                if (run.length >= 2) {
                    p.push(pathP(run, mix(colors[si], k.surface, 0.45), Math.max(width * 0.6, 1)));
                }
                run = [];
            };
            for (let i = 0; i < pts[si].length; i++) {
                const v2 = pts[si][i].v2;
                if (v2 === null) {
                    flush();
                } else {
                    run.push([xs[si][i], sy(v2)]);
                }
            }
            flush();
        }
    }
    // the lines: runs of points with a value, split where the rule or the forecast changes
    const firstPrim: number[] = [];
    for (let si = 0; si < pts.length; si++) {
        const muted = lit || (hot >= 0 && hot !== si);
        const c = muted ? mix(colors[si], k.surface, lit ? 0.6 : 0.55) : colors[si];
        const wd = hot === si ? width * 1.6 : hot >= 0 ? Math.max(width * 0.7, 1) : width;
        const rc = muted ? mix(ruleColor, k.surface, lit ? 0.6 : 0.55) : ruleColor;
        firstPrim.push(-1);
        const emit = (pp: number[][], dashed: boolean, ruled: boolean) => {
            if (firstPrim[si] < 0) {
                firstPrim[si] = p.length;
            }
            if (dashed) {
                dashes(p, pp, ruled ? rc : c, wd, 6, 4);
            } else {
                p.push(pathP(pp, ruled ? rc : c, wd));
            }
        };
        let run: number[] = [];
        const flush = () => {
            if (run.length === 1) {
                const q = pts[si][run[0]];
                if (!markers && q.v !== null) {
                    if (firstPrim[si] < 0) {
                        firstPrim[si] = p.length;
                    }
                    p.push(circle(xs[si][run[0]], sy(q.v), wd / 2 + 1.5, c, 1));
                }
            }
            let cur: number[][] = [];
            let curDashed = false;
            let curRuled = false;
            for (let j = 1; j < run.length; j++) {
                const a = run[j - 1];
                const b = run[j];
                const ax = xs[si][a];
                const bx = xs[si][b];
                const va = pts[si][a].v as number;
                const vb = pts[si][b].v as number;
                const ts: number[] = [0, 1];
                if (cutoff !== null && ax < cutoff && bx > cutoff) {
                    ts.push((cutoff - ax) / (bx - ax));
                }
                if (colorRule !== "none" && (va - rv) * (vb - rv) < 0) {
                    ts.push((rv - va) / (vb - va));
                }
                ts.sort((x, y) => x - y);
                for (let t = 1; t < ts.length; t++) {
                    const ta = ts[t - 1];
                    const tb = ts[t];
                    if (!(tb > ta)) {
                        continue;
                    }
                    const tm = (ta + tb) / 2;
                    const dashed = cutoff !== null && ax + (bx - ax) * tm >= cutoff;
                    const mv = va + (vb - va) * tm;
                    const ruled = colorRule === "above" ? mv > rv : colorRule === "below" ? mv < rv : false;
                    const pa = [ax + (bx - ax) * ta, sy(va + (vb - va) * ta)];
                    const pb = [ax + (bx - ax) * tb, sy(va + (vb - va) * tb)];
                    if (cur.length > 0 && (dashed !== curDashed || ruled !== curRuled)) {
                        emit(cur, curDashed, curRuled);
                        cur = [];
                    }
                    if (cur.length === 0) {
                        cur.push(pa);
                        curDashed = dashed;
                        curRuled = ruled;
                    }
                    cur.push(pb);
                }
            }
            if (cur.length >= 2) {
                emit(cur, curDashed, curRuled);
            }
            run = [];
        };
        for (let i = 0; i < pts[si].length; i++) {
            if (pts[si][i].v === null) {
                flush();
            } else {
                run.push(i);
            }
        }
        flush();
        // another visual's selection: the highlighted parts over the muted line
        // (a lone highlighted point is a dot, as a lone point of the line is)
        if (lit) {
            let hr: number[][] = [];
            const hflush = () => {
                if (hr.length >= 2) {
                    p.push(pathP(hr, colors[si], wd));
                } else if (hr.length === 1 && !markers) {
                    p.push(circle(hr[0][0], hr[0][1], wd / 2 + 1.5, colors[si], 1));
                }
                hr = [];
            };
            for (let i = 0; i < pts[si].length; i++) {
                const hv = pts[si][i].hl;
                if (hv === null || pts[si][i].v === null) {
                    hflush();
                } else {
                    hr.push([xs[si][i], sy(hv)]);
                }
            }
            hflush();
        }
    }
    if (threshold && tv >= yd.lo && tv <= yd.hi) {
        const ty = sy(tv);
        dashes(p, [[x0, ty], [x1, ty]], thresholdColor, 1.5, 6, 4);
        const s = compact(tv, null);
        p.push(text(x1 - 2, Math.max(ty - 8, y0 + 4), s, k.tick, k.text, "end"));
    }
    if (cutoff !== null) {
        dashes(p, [[cutoff, y0], [cutoff, y1]], k.axis, 1, 4, 3);
        const s = fit(lang === "fr" ? "Prévision" : "Forecast", k.tick, x1 - cutoff - 6);
        if (s !== "") {
            p.push(text(cutoff + 4, y0 + 6, s, k.tick, k.text, "start"));
        }
    }
    // the points' targets: markers (or invisible ones), else each line
    const hitPoints = count <= MAX_HIT_POINTS;
    for (let si = 0; si < pts.length; si++) {
        if (!hitPoints) {
            if (firstPrim[si] >= 0 && pts[si].length > 0) {
                out.hits.push({ row: pts[si][0].row, prim: firstPrim[si], series: si, line: true });
            }
            continue;
        }
        for (let i = 0; i < pts[si].length; i++) {
            const q = pts[si][i];
            if (q.v === null) {
                continue;
            }
            const x = xs[si][i];
            const y = sy(q.v);
            const m = markers ? dot(k, x, y, Math.max(width + 1, 3), colors[si], 1) : circle(x, y, 4, colors[si], 0);
            const fade = lit && q.hl === null;
            const hit: any = { row: q.row, prim: p.length, series: si, point: i };
            if (fade) {
                hit.fade = true;
            }
            out.hits.push(hit);
            p.push(fade ? dim(m) : m);
        }
    }
    // value labels
    if (labelMode !== "none") {
        for (let si = 0; si < pts.length; si++) {
            const l = pts[si];
            const at: [number, string][] = [];
            let first = -1;
            let last = -1;
            let lo2 = -1;
            let hi2 = -1;
            let nv = 0;
            for (let i = 0; i < l.length; i++) {
                const v = l[i].own;
                if (v === null) {
                    continue;
                }
                nv++;
                if (first < 0) {
                    first = i;
                }
                last = i;
                if (lo2 < 0 || v < (l[lo2].own as number)) {
                    lo2 = i;
                }
                if (hi2 < 0 || v > (l[hi2].own as number)) {
                    hi2 = i;
                }
            }
            if (labelMode === "last" && last >= 0) {
                at.push([last, "n"]);
            } else if (labelMode === "minmax" && hi2 >= 0) {
                at.push([hi2, "n"]);
                if (lo2 !== hi2) {
                    at.push([lo2, "s"]);
                }
            } else if (labelMode === "all" && nv <= MAX_ALL_LABELS) {
                for (let i = 0; i < l.length; i++) {
                    if (l[i].v !== null) {
                        at.push([i, "n"]);
                    }
                }
            }
            for (const [i, side] of at) {
                const s = compact(l[i].own as number, null);
                const half = est(s, fontSize) / 2 + (k.pills ? 5 : 0);
                const x = Math.max(Math.min(xs[si][i], w - half), half);
                valueLabel(p, k, x, sy(l[i].v as number), s, side, 6, fontSize);
            }
        }
    }
    // the series' names at the end of their lines
    if (named && endLabels && endW > 0) {
        const ends2: [number, number][] = [];
        for (let si = 0; si < pts.length; si++) {
            for (let i = pts[si].length - 1; i >= 0; i--) {
                const v = pts[si][i].v;
                if (v !== null) {
                    ends2.push([si, sy(v)]);
                    break;
                }
            }
        }
        ends2.sort((a, b) => a[1] - b[1] || a[0] - b[0]);
        const gapY = k.name + 3;
        for (let i = 1; i < ends2.length; i++) {
            ends2[i][1] = Math.max(ends2[i][1], ends2[i - 1][1] + gapY);
        }
        for (let i = ends2.length - 1; i >= 0; i--) {
            const lim = i === ends2.length - 1 ? y1 : ends2[i + 1][1] - gapY;
            ends2[i][1] = Math.min(ends2[i][1], lim);
        }
        for (const [si, y] of ends2) {
            const yy = Math.max(y, k.name / 2 + 1);
            const s = fit(ends[si], k.name, w - x1 - 16);
            if (s === "") {
                continue;
            }
            p.push(circle(x1 + 8, yy, 3.5, colors[si], 1));
            p.push(textW(x1 + 15, yy, s, k.name, k.label, "start", 0, 600));
        }
    }
    return out;
}

