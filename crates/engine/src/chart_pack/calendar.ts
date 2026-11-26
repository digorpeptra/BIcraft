// BIcraft Calendar Heatmap (docs/formats/chart-pack.md "Calendar heatmap"): one cell per day of
// each year of the data (or of its months only), weeks as columns and weekdays (Monday or Sunday
// first) as rows, coloured from the low to the high colour by the day's value; months and
// weekdays labelled in English or French.
// The TypeScript twin of crates/render/src/chart_pack/calendar.rs.
"use strict";

import { MS_PER_DAY, Out, PAD, Prim, cellNum, civilFromDays, daysFromCivil, est, fmtAuto, highlighted, hlNum, langOf, lerpColor, lookOfInput, optBool, optColor, optionsOf, parseIso, r2, rowsOf, text } from "./kit";
import { Look, color, dim, minimal, mix, part, pushCard, textW, tile } from "./runtime/style";

export const MAX_ROWS = 4000;
/** Most years drawn (the latest ones). */
export const MAX_YEARS = 4;

const MONTHS: { [lang: string]: string[] } = {
    en: ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"],
    fr: ["janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.", "déc."],
};
const WEEKDAYS: { [lang: string]: string[] } = { en: ["Mon", "Wed", "Fri"], fr: ["lun.", "mer.", "ven."] };

/** Monday 0 … Sunday 6 of a day number (1970-01-01 was a Thursday). */
export function weekday(z: number): number {
    return (((z + 3) % 7) + 7) % 7;
}

function dayOf(v: any): number | null {
    let ms: number | null = null;
    if (typeof v === "string") {
        ms = parseIso(v);
    } else if (typeof v === "number" && isFinite(v)) {
        ms = v;
    }
    if (ms === null || !(Math.abs(ms) <= 8.64e15)) {
        return null;
    }
    return Math.floor(ms / MS_PER_DAY);
}

/** What the modern drawing of the days needs. */
interface Grid {
    firstYear: number;
    lastYear: number;
    weeks: number;
    sundayFirst: boolean;
    showLegend: boolean;
    vmin: number;
    vmax: number;
    low: string;
    high: string;
    empty: string;
    /** The highlighted part of each day (null: nothing is highlighted). */
    parts: Map<number, number> | null;
}

/**
 * Day `z`'s cell (`b`: x, y, w, h), coloured `c`, for its first row `row` and value `v`. When the
 * visual is highlighted (`parts`), the full cell is dimmed and the day's highlighted part fills it
 * from its foot in proportion (docs/formats/bicraft-visuals.md "Highlights").
 */
function dayCell(out: Out, lk: Look, b: number[], c: string, corner: number, z: number, row: number, v: number, parts: Map<number, number> | null): void {
    const [x, y, w, h] = b;
    out.hits.push({ row, prim: out.prims.length, day: z, value: v });
    const t = tile(lk, x, y, w, h, c, corner, false);
    if (!parts) {
        out.prims.push(t);
        return;
    }
    out.prims.push(dim(t));
    const hv = parts.get(z);
    const p = part(v, hv === undefined ? null : hv);
    if (p !== null) {
        const ph = (h * p) / v;
        out.hits.push({ row, prim: out.prims.length, day: z, value: v });
        out.prims.push(tile(lk, x, y + h - ph, w, ph, c, corner, false));
    }
}

/** A polyline (a month's separator). */
function pathP(points: number[][], stroke: string, width: number): Prim {
    return { t: "path", points: points.map((q) => [r2(q[0]), r2(q[1])]), stroke, width: r2(width) };
}

/**
 * Modern and Bold: the year's weeks fill the visual's width and its weekdays its height (cells
 * at most 48 px and no more than twice as wide as tall or 1.6 times as tall as wide), centred;
 * each year headed by its number and its months, the months parted by a line along their first
 * days, three weekdays named, and a legend of five steps under the grid.
 */
function modern(out: Out, lk: Look, g: Grid, days: Map<number, number[]>, row: (z: number) => number, span: (y: number) => number[], lang: string, w: number, h: number): void {
    const p = out.prims;
    const years = g.lastYear - g.firstYear + 1;
    const tick = lk.tick;
    const wds = WEEKDAYS[lang];
    let gutter = est(String(g.lastYear), lk.name) + 10;
    for (const s of wds) {
        gutter = Math.max(gutter, est(s, tick - 1) + 10);
    }
    const hdr = tick + 10;
    const yearGap = 12;
    const legendH = g.showLegend ? 30 : 0;
    const aw = w - 2 * PAD - gutter;
    const ah = h - 2 * PAD - legendH - (years - 1) * yearGap;
    let cw = Math.min(aw / g.weeks, 48);
    let ch = Math.min((ah / years - hdr) / 7, 48);
    cw = Math.min(cw, ch * 2);
    ch = Math.min(ch, cw * 1.6);
    if (!(cw >= 3 && ch >= 3)) {
        return;
    }
    const m = Math.min(cw, ch);
    const gap = m >= 12 ? 3 : m >= 6 ? 2 : 1;
    const corner = Math.min((m - gap) * 0.3, 6);
    const gw = g.weeks * cw;
    const blockH = hdr + 7 * ch;
    const total = years * blockH + (years - 1) * yearGap;
    const x0 = PAD + gutter + Math.max((aw - gw) / 2, 0);
    const y0 = PAD + Math.max((h - 2 * PAD - legendH - total) / 2, 0);
    pushCard(p, lk, w, h);
    const months = MONTHS[lang];
    for (let y = g.firstYear; y <= g.lastYear; y++) {
        const top = y0 + (y - g.firstYear) * (blockH + yearGap);
        const ty = top + hdr;
        const [z0, z1] = span(y);
        const start = z0 - row(z0);
        p.push(textW(x0 - 8, top + hdr / 2, String(y), lk.name, lk.label, "end", 0, 600));
        for (let mo = 1; mo <= 12; mo++) {
            const label = months[mo - 1];
            const m0 = daysFromCivil(y, mo, 1);
            const m1 = mo === 12 ? daysFromCivil(y, 12, 31) : daysFromCivil(y, mo + 1, 1) - 1;
            if (m1 < z0 || m0 > z1) {
                continue;
            }
            const col = Math.floor((Math.max(m0, z0) - start) / 7);
            if (cw * 4 >= est(label, tick)) {
                p.push(text(x0 + col * cw, top + hdr / 2, label, tick, lk.text, "start"));
            }
            if (m0 > z0 && m >= 8) {
                // the line between the month before and this one, along the gaps (cells of 8 px or more)
                const r = row(m0);
                const xb = x0 + col * cw - gap / 2;
                const yb = ty + 7 * ch - gap / 2;
                if (r === 0) {
                    p.push(pathP([[xb, ty - gap / 2], [xb, yb]], lk.axis, 1.5));
                } else {
                    const yr = ty + r * ch - gap / 2;
                    p.push(pathP([[xb + cw, ty - gap / 2], [xb + cw, yr], [xb, yr], [xb, yb]], lk.axis, 1.5));
                }
            }
        }
        if (ch >= 9) {
            for (let k = 0; k < 3; k++) {
                const r = 2 * k + (g.sundayFirst ? 1 : 0);
                p.push(text(x0 - 8, ty + r * ch + (ch - gap) / 2, wds[k], tick - 1, lk.text, "end"));
            }
        }
        for (let z = z0; z <= z1; z++) {
            const x = x0 + Math.floor((z - start) / 7) * cw;
            const yy = ty + row(z) * ch;
            const d = days.get(z);
            if (d) {
                const t = g.vmax > g.vmin ? (d[0] - g.vmin) / (g.vmax - g.vmin) : 1;
                dayCell(out, lk, [x, yy, cw - gap, ch - gap], lerpColor(g.low, g.high, t), corner, z, d[1], d[0], g.parts);
            } else {
                p.push(tile(lk, x, yy, cw - gap, ch - gap, g.empty, corner, false));
            }
        }
    }
    if (g.showLegend) {
        // the lowest value, five steps and the highest, right-aligned under the grid
        const minText = fmtAuto(g.vmin);
        const maxText = fmtAuto(g.vmax);
        const s = 12;
        const ly = y0 + total + (legendH - s) / 2;
        const right = Math.min(x0 + gw - gap, w - PAD);
        const xs = right - est(maxText, tick) - 6 - 5 * (s + 3) + 3;
        if (xs - 6 - est(minText, tick) >= PAD) {
            p.push(text(xs - 6, ly + s / 2, minText, tick, lk.text, "end"));
            for (let k = 0; k < 5; k++) {
                p.push(tile(lk, xs + k * (s + 3), ly, s, s, lerpColor(g.low, g.high, k / 4), 3, false));
            }
            p.push(text(xs + 5 * (s + 3) - 3 + 6, ly + s / 2, maxText, tick, lk.text, "start"));
        }
    }
}

/** Lays out `{rows: [[date, value]], options, lang}` in a `w`×`h` visual. */
export function layoutCalendar(input: any, w: number, h: number): Out {
    const o = optionsOf(input);
    const lk = lookOfInput(input);
    // Modern and Bold: one hue, light to dark, from the palette's first colour and its tint
    const high = optColor(o, "high", minimal(lk) ? "#1F6F8B" : color(lk, 0));
    const low = optColor(o, "low", minimal(lk) ? "#D6E6EE" : mix(high, lk.night ? lk.surface : "#FFFFFF", 0.86));
    const empty = optColor(o, "empty", minimal(lk) ? "#EFEFEF" : lk.empty);
    const showLegend = optBool(o, "showLegend", true);
    const monthsOnly = optBool(o, "monthsOnly", false);
    const sundayFirst = optBool(o, "sundayFirst", false);
    // the row of a day: Monday (or Sunday) 0
    const row = (z: number) => (sundayFirst ? (((z + 4) % 7) + 7) % 7 : weekday(z));
    const lang = langOf(input);
    const out: Out = { prims: [], hits: [] };
    // the value (summed) and first row of each day, and its highlighted part
    const days = new Map<number, number[]>();
    const partOf = new Map<number, number>();
    const rows = rowsOf(input, MAX_ROWS);
    let maxDay = -Infinity;
    for (let i = 0; i < rows.length; i++) {
        const z = dayOf(rows[i][0]);
        const v = cellNum(rows[i][1]);
        if (z === null || v === null) {
            continue;
        }
        const d = days.get(z);
        if (d) {
            d[0] += v;
        } else {
            days.set(z, [v, i]);
        }
        const hv = hlNum(input, i);
        if (hv !== null) {
            const q = partOf.get(z);
            partOf.set(z, (q === undefined ? 0 : q) + hv);
        }
        maxDay = Math.max(maxDay, z);
    }
    const parts = highlighted(input) ? partOf : null;
    if (days.size === 0) {
        return out;
    }
    const lastYear = civilFromDays(maxDay)[0];
    let firstYear = lastYear;
    days.forEach((d, z) => {
        firstYear = Math.min(firstYear, civilFromDays(z)[0]);
    });
    firstYear = Math.max(firstYear, lastYear - (MAX_YEARS - 1));
    const from = daysFromCivil(firstYear, 1, 1);
    let vmin = Infinity;
    let vmax = -Infinity;
    let minDay = maxDay;
    days.forEach((d, z) => {
        if (z >= from) {
            vmin = Math.min(vmin, d[0]);
            vmax = Math.max(vmax, d[0]);
            minDay = Math.min(minDay, z);
        }
    });
    // the days drawn: whole years, or from the first month with data to the last
    const a = civilFromDays(minDay);
    const b = civilFromDays(maxDay);
    if (monthsOnly) {
        firstYear = a[0];
    }
    const firstDay = monthsOnly ? daysFromCivil(a[0], a[1], 1) : from;
    const lastDay = monthsOnly ? (b[1] === 12 ? daysFromCivil(b[0], 12, 31) : daysFromCivil(b[0], b[1] + 1, 1) - 1) : daysFromCivil(lastYear, 12, 31);
    const span = (y: number) => [Math.max(daysFromCivil(y, 1, 1), firstDay), Math.min(daysFromCivil(y, 12, 31), lastDay)];
    const years = lastYear - firstYear + 1;
    const legendH = showLegend ? 18 : 0;
    const gutter = 30;
    let weeks = 0;
    for (let y = firstYear; y <= lastYear; y++) {
        const [z0, z1] = span(y);
        const start = z0 - row(z0);
        weeks = Math.max(weeks, Math.floor((z1 - start) / 7) + 1);
    }
    if (!minimal(lk)) {
        // Modern and Bold: the cells fill the visual (calendar.rs `modern`)
        const g: Grid = { firstYear, lastYear, weeks, sundayFirst, showLegend, vmin, vmax, low, high, empty, parts };
        modern(out, lk, g, days, row, span, lang, w, h);
        return out;
    }
    const block = (h - 2 * PAD - legendH) / years;
    const c = Math.min(24, (w - 2 * PAD - gutter) / weeks, (block - 22) / 7);
    if (!(c >= 2)) {
        return out;
    }
    const gap = c >= 10 ? 2 : c >= 4 ? 1 : 0;
    const cs = c - gap;
    const corner = Math.min(cs * 0.25, 3);
    const x0 = PAD + gutter;
    const p = out.prims;
    const months = MONTHS[lang];
    const wds = WEEKDAYS[lang];
    for (let y = firstYear; y <= lastYear; y++) {
        const top = PAD + (y - firstYear) * (14 + 7 * c + 8);
        const [z0, z1] = span(y);
        const start = z0 - row(z0);
        p.push(text(PAD, top + 6, String(y), 11, lk.label, "start"));
        for (let m = 1; m <= 12; m++) {
            const label = months[m - 1];
            const m0 = daysFromCivil(y, m, 1);
            const m1 = m === 12 ? daysFromCivil(y, 12, 31) : daysFromCivil(y, m + 1, 1) - 1;
            if (c * 4 >= est(label, 9) && m1 >= z0 && m0 <= z1) {
                const col = Math.floor((Math.max(m0, z0) - start) / 7);
                p.push(text(x0 + col * c, top + 6, label, 9, lk.text, "start"));
            }
        }
        if (c >= 7) {
            for (let k = 0; k < 3; k++) {
                p.push(text(x0 - 4, top + 14 + (2 * k + (sundayFirst ? 1 : 0)) * c + cs / 2, wds[k], 9, lk.text, "end"));
            }
        }
        for (let z = z0; z <= z1; z++) {
            const col = Math.floor((z - start) / 7);
            const x = x0 + col * c;
            const yy = top + 14 + row(z) * c;
            const d = days.get(z);
            if (d) {
                const t = vmax > vmin ? (d[0] - vmin) / (vmax - vmin) : 1;
                dayCell(out, lk, [x, yy, cs, cs], lerpColor(low, high, t), corner, z, d[1], d[0], parts);
            } else {
                p.push(tile(lk, x, yy, cs, cs, empty, corner, false));
            }
        }
    }
    if (showLegend) {
        const ly = h - PAD - 12;
        const minText = fmtAuto(vmin);
        const maxText = fmtAuto(vmax);
        const xs = w - PAD - est(maxText, 9) - 4 - 5 * 12;
        if (xs - 4 - est(minText, 9) >= PAD) {
            p.push(text(xs - 4, ly + 5, minText, 9, lk.text, "end"));
            for (let k = 0; k < 5; k++) {
                p.push(tile(lk, xs + k * 12, ly, 10, 10, lerpColor(low, high, k / 4), 2, false));
            }
            p.push(text(xs + 5 * 12 + 2, ly + 5, maxText, 9, lk.text, "start"));
        }
    }
    return out;
}
