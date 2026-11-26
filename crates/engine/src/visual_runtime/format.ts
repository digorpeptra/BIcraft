// BIcraft's visual runtime (docs/community-visuals.md "Runtime"): values written with the format
// strings of the data view's columns (`source.format`), for tooltips and labels. Our own code for
// the common custom number and date formats of Power BI's format strings (Microsoft Learn's
// "Custom format strings" page): digit placeholders `0` and `#`, `.`, `,` (grouping, or ÷1000
// before the decimal point), `%`, `‰`, exponents `E+0`/`E-0`, sections `positive;negative;zero`,
// quoted and escaped literals; the date and time format strings of DAX `FORMAT` (`dd/mm/yyyy`,
// `mmmm yyyy`, `hh:nn`, `AM/PM`, …) and the named formats General Number, Fixed, Standard,
// Percent, Short Date, Long Date, General Date… English and French separators and names.
"use strict";

import { Lang, langOf, t } from "./strings";

/** Separators and names of a language. */
interface Culture {
    lang: Lang;
    group: string;
    decimal: string;
    months: string[];
    days: string[];
    am: string;
    pm: string;
}

const EN_CULTURE: Culture = {
    lang: "en",
    group: ",",
    decimal: ".",
    months: ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"],
    days: ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"],
    am: "AM",
    pm: "PM",
};

const FR_CULTURE: Culture = {
    lang: "fr",
    group: " ",
    decimal: ",",
    months: ["janvier", "février", "mars", "avril", "mai", "juin", "juillet", "août", "septembre", "octobre", "novembre", "décembre"],
    days: ["dimanche", "lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi"],
    am: "",
    pm: "",
};

function cultureOf(locale: string | undefined | null): Culture {
    return langOf(locale) === "fr" ? FR_CULTURE : EN_CULTURE;
}

/** Most characters of a format string read. */
const MAX_FORMAT = 200;
/** Most decimals written. */
const MAX_DECIMALS = 15;

/** Writes `v` with `format` (a column's format string) in `locale`'s language. */
export function formatValue(v: any, format?: string | null, locale?: string | null): string {
    if (v === null || v === undefined || v === "") {
        return t(locale, "blank");
    }
    const f = typeof format === "string" ? format.slice(0, MAX_FORMAT) : "";
    if (typeof v === "boolean") {
        return t(locale, v ? "true" : "false");
    }
    if (v instanceof Date) {
        return isFinite(v.getTime()) ? formatDate(v, f, locale) : "";
    }
    if (typeof v === "number") {
        return formatNumber(v, f, locale);
    }
    return String(v);
}

// ---------------------------------------------------------------------------------------------
// Numbers
// ---------------------------------------------------------------------------------------------

const NAMED_NUMBERS: { [name: string]: string } = {
    "general number": "",
    "g": "",
    "fixed": "0.00",
    "f": "0.00",
    "standard": "#,0.00",
    "n": "#,0.00",
    "percent": "0.00%",
    "p": "0.00%",
};

/** Splits a format at its `;` section separators (not inside quotes or after `\`). */
export function sections(f: string): string[] {
    const out: string[] = [];
    let cur = "";
    let quoted = false;
    for (let i = 0; i < f.length; i++) {
        const c = f[i];
        if (c === "\\" && !quoted && i + 1 < f.length) {
            cur += c + f[i + 1];
            i++;
        } else if (c === "\"") {
            quoted = !quoted;
            cur += c;
        } else if (c === ";" && !quoted) {
            out.push(cur);
            cur = "";
        } else {
            cur += c;
        }
    }
    out.push(cur);
    return out;
}

/** A number section read: its literals around the digits and its digit rules. */
interface NumPattern {
    prefix: string;
    suffix: string;
    minInt: number;
    minDec: number;
    maxDec: number;
    group: boolean;
    /** Powers of ten the value is multiplied by (`%` +2, `‰` +3, a scaling `,` −3). */
    shift: number;
    /** Exponent: its minimum digits and whether `+` is always written; null without one. */
    exp: { digits: number; plus: boolean; upper: boolean } | null;
}

function readPattern(s: string): NumPattern {
    const p: NumPattern = { prefix: "", suffix: "", minInt: 0, minDec: 0, maxDec: 0, group: false, shift: 0, exp: null };
    let seenDigit = false;
    let inDec = false;
    let pendingCommas = 0;
    for (let i = 0; i < s.length; i++) {
        const c = s[i];
        const lit = (text: string) => {
            if (seenDigit) {
                p.suffix += text;
            } else {
                p.prefix += text;
            }
        };
        if (c === "\\" && i + 1 < s.length) {
            lit(s[i + 1]);
            i++;
        } else if (c === "\"") {
            const end = s.indexOf("\"", i + 1);
            const stop = end < 0 ? s.length : end;
            lit(s.slice(i + 1, stop));
            i = stop;
        } else if ((c === "0" || c === "#") && !p.exp) {
            if (pendingCommas > 0 && !inDec) {
                p.group = true;
            }
            pendingCommas = 0;
            seenDigit = true;
            if (inDec) {
                p.maxDec++;
                if (c === "0") {
                    p.minDec = p.maxDec;
                }
            } else if (c === "0") {
                p.minInt++;
            }
        } else if (c === "." && !inDec && !p.exp) {
            p.shift -= 3 * pendingCommas;
            pendingCommas = 0;
            inDec = true;
            seenDigit = true;
        } else if (c === "," && !inDec && !p.exp) {
            if (seenDigit) {
                pendingCommas++;
            } else {
                lit(c);
            }
        } else if ((c === "E" || c === "e") && seenDigit && !p.exp && (s[i + 1] === "+" || s[i + 1] === "-") && s[i + 2] === "0") {
            let j = i + 2;
            let digits = 0;
            while (j < s.length && s[j] === "0") {
                digits++;
                j++;
            }
            p.exp = { digits, plus: s[i + 1] === "+", upper: c === "E" };
            i = j - 1;
        } else {
            if (c === "%") {
                p.shift += 2;
            } else if (c === "‰") {
                p.shift += 3;
            }
            lit(c);
        }
    }
    // commas after the last integer digit (and no decimal point) scale by 1000 each
    if (!inDec) {
        p.shift -= 3 * pendingCommas;
    }
    return p;
}

/** The decimal digits of `x ≥ 0` rounded half away from zero to `d` decimals: [integer, fraction]. */
export function roundDigits(x: number, d: number): [string, string] {
    const dec = Math.max(0, Math.min(MAX_DECIMALS, Math.floor(d)));
    if (!isFinite(x) || x < 0) {
        return ["0", "0".repeat(dec)];
    }
    let text: string;
    const scaled = x * Math.pow(10, dec);
    if (scaled < 9e15) {
        // toPrecision(15) drops the binary noise of x·10^d (1.005 × 100 = 100.49999…)
        const n = Math.round(Number(scaled.toPrecision(15)));
        text = String(n);
        while (text.length <= dec) {
            text = "0" + text;
        }
    } else if (x < 1e21) {
        text = x.toFixed(dec).replace(".", "");
    } else {
        return [x.toExponential(), ""];
    }
    return [text.slice(0, text.length - dec), text.slice(text.length - dec)];
}

function groupDigits(int: string, sep: string): string {
    let out = "";
    for (let i = 0; i < int.length; i++) {
        if (i > 0 && (int.length - i) % 3 === 0) {
            out += sep;
        }
        out += int[i];
    }
    return out;
}

function applyPattern(x: number, p: NumPattern, c: Culture): [string, boolean] {
    let v = x * Math.pow(10, p.shift);
    let expText = "";
    if (p.exp) {
        let e = 0;
        if (v > 0) {
            e = Math.floor(Math.log10(v)) - Math.max(p.minInt, 1) + 1;
            v = v / Math.pow(10, e);
            const [ri] = roundDigits(v, p.maxDec);
            if (ri.length > Math.max(p.minInt, 1)) {
                e += 1;
                v = v / 10;
            }
        }
        const sign = e < 0 ? "-" : (p.exp.plus ? "+" : "");
        let ed = String(Math.abs(e));
        while (ed.length < p.exp.digits) {
            ed = "0" + ed;
        }
        expText = (p.exp.upper ? "E" : "e") + sign + ed;
    }
    let [int, frac] = roundDigits(v, p.maxDec);
    const zero = /^0*$/.test(int) && /^0*$/.test(frac);
    while (frac.length > p.minDec && frac.endsWith("0")) {
        frac = frac.slice(0, -1);
    }
    int = int.replace(/^0+/, "");
    while (int.length < p.minInt) {
        int = "0" + int;
    }
    if (p.group) {
        int = groupDigits(int, c.group);
    }
    const body = int + (frac.length > 0 ? c.decimal + frac : "") + expText;
    return [p.prefix + body + p.suffix, zero];
}

/** The general form: at most 15 significant digits, no grouping. */
function general(v: number, c: Culture): string {
    if (v === 0) {
        return "0";
    }
    const a = Math.abs(v);
    let s: string;
    if (a >= 1e15 || a < 1e-5) {
        s = Number(a.toPrecision(15)).toExponential().replace("e", "E");
    } else {
        const [int, fracRaw] = roundDigits(a, Math.max(0, Math.min(MAX_DECIMALS, 14 - Math.floor(Math.log10(a)))));
        const frac = fracRaw.replace(/0+$/, "");
        s = int + (frac.length > 0 ? c.decimal + frac : "");
    }
    return (v < 0 ? "-" : "") + s;
}

/** Writes a number with a custom or named number format. */
export function formatNumber(v: number, format: string, locale?: string | null): string {
    const c = cultureOf(locale);
    if (!isFinite(v)) {
        return isNaN(v) ? "NaN" : (v > 0 ? "∞" : "-∞");
    }
    let f = format.trim();
    const named = NAMED_NUMBERS[f.toLowerCase()];
    if (named !== undefined) {
        f = named;
    }
    if (f === "" || f.toLowerCase() === "general") {
        return general(v, c);
    }
    const secs = sections(f);
    if (v < 0 && secs.length > 1 && secs[1] !== "") {
        return applyPattern(-v, readPattern(secs[1]), c)[0];
    }
    if (v === 0 && secs.length > 2 && secs[2] !== "") {
        return applyPattern(0, readPattern(secs[2]), c)[0];
    }
    const [text, zero] = applyPattern(Math.abs(v), readPattern(secs[0]), c);
    return v < 0 && !zero ? "-" + text : text;
}


// ---------------------------------------------------------------------------------------------
// Dates: the twin of BIcraft's Rust date formatting (the DAX `FORMAT` date rules of
// crates/locale/src/vb.rs and the named formats of crates/render/src/fmt.rs `date_label`) for
// English (en-US) and French (fr-FR), so a visual's labels read the same in Power BI and in
// BIcraft's renderer; the engine's tests hold both equal on a corpus of formats.
// ---------------------------------------------------------------------------------------------

/** A date and time's fields on its own clock: month 1–12, weekday 0 = Sunday. */
export interface DateParts {
    y: number;
    M: number;
    d: number;
    w: number;
    H: number;
    m: number;
    s: number;
    /** The date is day 0 of the OLE calendar (1899-12-30): a time alone. */
    timeOnly: boolean;
}

/** The names and patterns of a language's dates (bicraft_locale's en-US and fr-FR data). */
interface DateCulture {
    months: string[];
    monthsAbbr: string[];
    days: string[];
    daysAbbr: string[];
    am: string;
    pm: string;
    shortDate: string;
    longDate: string;
    longTime: string;
    mediumDate: string;
    mediumTime: string;
    /** First day of the week (0 = Sunday) and week-1 rule of `w` and `ww`. */
    firstDay: number;
    firstWeek: number;
}

const EN_DATES: DateCulture = {
    months: EN_CULTURE.months,
    monthsAbbr: ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"],
    days: EN_CULTURE.days,
    daysAbbr: ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"],
    am: "AM",
    pm: "PM",
    shortDate: "M/d/yyyy",
    longDate: "dddd', 'MMMM' 'd', 'yyyy",
    longTime: "h:mm:ss' 'tt",
    mediumDate: "dd'-'MMM'-'yy",
    mediumTime: "hh:mm' 'tt",
    firstDay: 0,
    firstWeek: 0,
};

const FR_DATES: DateCulture = {
    months: FR_CULTURE.months,
    monthsAbbr: ["janv", "févr", "mars", "avr", "mai", "juin", "juil", "août", "sept", "oct", "nov", "déc"],
    days: FR_CULTURE.days,
    daysAbbr: ["dim.", "lun.", "mar.", "mer.", "jeu.", "ven.", "sam."],
    am: "",
    pm: "",
    shortDate: "dd/MM/yyyy",
    longDate: "dddd' 'd' 'MMMM' 'yyyy",
    longTime: "HH:mm:ss",
    mediumDate: "dd'-'MMM'-'yy",
    mediumTime: "hh:mm' '",
    firstDay: 1,
    firstWeek: 2,
};

/** Longest date format read, and most characters written. */
const MAX_DATE_FORMAT = 1024;
const MAX_DATE_OUT = 8192;
const MS_PER_DAY = 86400000;
/** 1970-01-01 as an OLE date serial. */
const OLE_1970 = 25569;

function dateCultureOf(locale: string | undefined | null): DateCulture {
    return langOf(locale) === "fr" ? FR_DATES : EN_DATES;
}

function pad2(n: number, w: number): string {
    let s = String(Math.abs(n));
    while (s.length < w) {
        s = "0" + s;
    }
    return n < 0 ? "-" + s : s;
}

function lowerAscii(s: string): string {
    return s.replace(/[A-Z]/g, (x) => x.toLowerCase());
}

/** Days since 1970-01-01 of a proleptic Gregorian date. */
function daysFromCivil(y: number, m: number, d: number): number {
    const yy = m <= 2 ? y - 1 : y;
    const era = Math.floor(yy / 400);
    const yoe = yy - era * 400;
    const mp = (m + 9) % 12;
    const doy = Math.floor((153 * mp + 2) / 5) + d - 1;
    const doe = yoe * 365 + Math.floor(yoe / 4) - Math.floor(yoe / 100) + doy;
    return era * 146097 + doe - 719468;
}

/** The date of a day count from 1970-01-01: [year, month 1–12, day]. */
function civilFromDays(z: number): number[] {
    const zz = z + 719468;
    const era = Math.floor(zz / 146097);
    const doe = zz - era * 146097;
    const yoe = Math.floor((doe - Math.floor(doe / 1460) + Math.floor(doe / 36524) - Math.floor(doe / 146096)) / 365);
    const doy = doe - (365 * yoe + Math.floor(yoe / 4) - Math.floor(yoe / 100));
    const mp = Math.floor((5 * doy + 2) / 153);
    const d = doy - Math.floor((153 * mp + 2) / 5) + 1;
    const m = mp < 10 ? mp + 3 : mp - 9;
    return [yoe + era * 400 + (m <= 2 ? 1 : 0), m, d];
}

/** Sunday 0 … Saturday 6 of a day count from 1970-01-01 (a Thursday). */
function weekdayOf(z: number): number {
    return (((z + 4) % 7) + 7) % 7;
}

/** The fields of milliseconds since 1970-01-01 on the value's own clock, as an OLE serial reads (years 100–9999, rounded to the millisecond); null outside. */
export function datePartsOfMs(ms: number): DateParts | null {
    const x = ms / MS_PER_DAY + OLE_1970;
    if (!isFinite(x) || !(x >= -657435 && x < 2958466)) {
        return null;
    }
    const r = x * MS_PER_DAY;
    const total = r < 0 ? -Math.round(-r) : Math.round(r);
    const day = Math.floor(total / MS_PER_DAY);
    const rem = total - day * MS_PER_DAY;
    const z = day - OLE_1970;
    const ymd = civilFromDays(z);
    return {
        y: ymd[0],
        M: ymd[1],
        d: ymd[2],
        w: weekdayOf(z),
        H: Math.floor(rem / 3600000),
        m: Math.floor(rem / 60000) % 60,
        s: Math.floor(rem / 1000) % 60,
        timeOnly: Math.floor(x) === 0,
    };
}

/** A .NET custom pattern of the culture's own (`dddd d MMMM yyyy`, `h:mm:ss tt`). */
function datePattern(p: DateParts, pat: string, c: DateCulture): string {
    const ch = Array.from(pat);
    let out = "";
    let i = 0;
    while (i < ch.length) {
        const x = ch[i];
        let n = 1;
        while (i + n < ch.length && ch[i + n] === x) {
            n++;
        }
        const h12 = p.H % 12 === 0 ? 12 : p.H % 12;
        switch (x) {
            case "d":
                out += n <= 2 ? pad2(p.d, n) : n === 3 ? c.daysAbbr[p.w] : c.days[p.w];
                break;
            case "M":
                out += n <= 2 ? pad2(p.M, n) : n === 3 ? c.monthsAbbr[p.M - 1] : c.months[p.M - 1];
                break;
            case "y":
                out += n <= 2 ? pad2(((p.y % 100) + 100) % 100, n) : pad2(p.y, Math.min(n, 9));
                break;
            case "h":
                out += pad2(h12, Math.min(n, 2));
                break;
            case "H":
                out += pad2(p.H, Math.min(n, 2));
                break;
            case "m":
                out += pad2(p.m, Math.min(n, 2));
                break;
            case "s":
                out += pad2(p.s, Math.min(n, 2));
                break;
            case "t": {
                const t = p.H < 12 ? c.am : c.pm;
                out += n === 1 ? Array.from(t).slice(0, 1).join("") : t;
                break;
            }
            case "'":
            case "\"": {
                let j = i + 1;
                while (j < ch.length && ch[j] !== x) {
                    if (ch[j] === "\\") {
                        j++;
                        if (j < ch.length) {
                            out += ch[j];
                        }
                    } else {
                        out += ch[j];
                    }
                    j++;
                }
                i = j + 1;
                continue;
            }
            case "\\":
                if (i + 1 < ch.length) {
                    out += ch[i + 1];
                }
                i += 2;
                continue;
            case "%":
                i += 1;
                continue;
            default:
                for (let k = 0; k < n; k++) {
                    out += x;
                }
        }
        i += n;
    }
    return out;
}

/** Week of the year with weeks from `first` (0 = Sunday): week 1 holds 1 January (rule 0), is the first full week (1) or the first with four days (2). */
function weekOfYear(p: DateParts, first: number, rule: number): number {
    const day = daysFromCivil(p.y, p.M, p.d);
    const week1 = (y: number): number => {
        const jan1 = daysFromCivil(y, 1, 1);
        const back = (((weekdayOf(jan1) - first) % 7) + 7) % 7;
        if (rule === 1) {
            return back === 0 ? jan1 : jan1 + 7 - back;
        }
        if (rule === 2) {
            return back <= 3 ? jan1 - back : jan1 + 7 - back;
        }
        return jan1 - back;
    };
    let start = week1(p.y);
    if (day < start) {
        start = week1(p.y - 1);
    } else if (rule === 2 && day >= week1(p.y + 1)) {
        return 1;
    }
    return Math.floor((day - start) / 7) + 1;
}

/** `General Date` (`c`): the short date, the long time, or both. */
function generalDate(p: DateParts, c: DateCulture): string {
    if (p.timeOnly) {
        return datePattern(p, c.longTime, c);
    }
    const date = datePattern(p, c.shortDate, c);
    return p.H !== 0 || p.m !== 0 || p.s !== 0 ? date + " " + datePattern(p, c.longTime, c) : date;
}

/** A DAX `FORMAT` date/time format string (tokens in any case: `dd/mm/yyyy`, `hh:nn`, `mmm yy`, `AM/PM`); null for a broken one. */
function vbDate(p: DateParts, f: string, c: DateCulture): string | null {
    if (f.length > MAX_DATE_FORMAT) {
        return null;
    }
    const ch = Array.from(f);
    const lower = lowerAscii(f);
    const hasAmPm = lower.indexOf("am/pm") >= 0 || lower.indexOf("a/p") >= 0 || lower.indexOf("ampm") >= 0;
    const run = (i: number, x: string): number => {
        let n = 0;
        while (i + n < ch.length && lowerAscii(ch[i + n]) === x) {
            n++;
        }
        return n;
    };
    let out = "";
    let i = 0;
    // the last time token was an hour: an `m` right after it is minutes
    let afterHour = false;
    while (i < ch.length) {
        if (out.length > MAX_DATE_OUT) {
            return null;
        }
        const x = ch[i];
        switch (lowerAscii(x)) {
            case "\"": {
                const end = ch.indexOf("\"", i + 1);
                if (end < 0) {
                    return null;
                }
                out += ch.slice(i + 1, end).join("");
                i = end + 1;
                continue;
            }
            case "\\":
                if (i + 1 >= ch.length) {
                    return null;
                }
                out += ch[i + 1];
                i += 2;
                continue;
            case "c":
                out += generalDate(p, c);
                i += 1;
                continue;
            case "y": {
                const n = run(i, "y");
                out += n === 1 ? String(daysFromCivil(p.y, p.M, p.d) - daysFromCivil(p.y, 1, 1) + 1) : n <= 3 ? pad2(((p.y % 100) + 100) % 100, 2) : pad2(p.y, 4);
                i += n >= 4 ? 4 : n;
                afterHour = false;
                continue;
            }
            case "d": {
                const n = Math.min(run(i, "d"), 6);
                out += n === 1 ? String(p.d) : n === 2 ? pad2(p.d, 2) : n === 3 ? c.daysAbbr[p.w] : n === 4 ? c.days[p.w] : datePattern(p, n === 5 ? c.shortDate : c.longDate, c);
                i += n;
                afterHour = false;
                continue;
            }
            case "m": {
                const n = Math.min(run(i, "m"), 4);
                // minutes after an hour token (or before seconds), months otherwise
                let nextIsS = false;
                for (let k = i + n; k < ch.length; k++) {
                    if (/[A-Za-z]/.test(ch[k])) {
                        nextIsS = ch[k] === "s" || ch[k] === "S";
                        break;
                    }
                }
                if ((afterHour || nextIsS) && n <= 2) {
                    out += n === 1 ? String(p.m) : pad2(p.m, 2);
                } else {
                    out += n === 1 ? String(p.M) : n === 2 ? pad2(p.M, 2) : n === 3 ? c.monthsAbbr[p.M - 1] : c.months[p.M - 1];
                }
                i += n;
                afterHour = false;
                continue;
            }
            case "n": {
                const n = Math.min(run(i, "n"), 2);
                out += n === 1 ? String(p.m) : pad2(p.m, 2);
                i += n;
                afterHour = false;
                continue;
            }
            case "h": {
                const n = Math.min(run(i, "h"), 2);
                const h = hasAmPm ? (p.H % 12 === 0 ? 12 : p.H % 12) : p.H;
                out += n === 1 ? String(h) : pad2(h, 2);
                i += n;
                afterHour = true;
                continue;
            }
            case "s": {
                const n = Math.min(run(i, "s"), 2);
                out += n === 1 ? String(p.s) : pad2(p.s, 2);
                i += n;
                afterHour = false;
                continue;
            }
            case "t":
                if (run(i, "t") >= 5) {
                    out += datePattern(p, c.longTime, c);
                    i += 5;
                    continue;
                }
                out += x;
                break;
            case "q":
                out += String(Math.floor((p.M - 1) / 3) + 1);
                i += 1;
                continue;
            case "w":
                if (run(i, "w") >= 2) {
                    out += String(weekOfYear(p, c.firstDay, c.firstWeek));
                    i += 2;
                } else {
                    out += String((((p.w - c.firstDay) % 7) + 7) % 7 + 1);
                    i += 1;
                }
                continue;
            case "a": {
                const pm = p.H >= 12;
                const rest5 = ch.slice(i, i + 5).join("");
                if (lowerAscii(rest5) === "am/pm") {
                    // lower case only for a PM time of a value with a date part, as the engine writes it
                    const lc = rest5[0] === "a" && pm && !p.timeOnly;
                    out += pm ? (lc ? "pm" : "PM") : "AM";
                    i += 5;
                    continue;
                }
                if (lowerAscii(ch.slice(i, i + 4).join("")) === "ampm") {
                    out += pm ? c.pm : c.am;
                    i += 4;
                    continue;
                }
                const rest3 = ch.slice(i, i + 3).join("");
                if (lowerAscii(rest3) === "a/p") {
                    const upper = rest3[0] === "A";
                    out += pm ? (upper ? "P" : "p") : (upper ? "A" : "a");
                    i += 3;
                    continue;
                }
                out += x;
                break;
            }
            case "/":
                out += "/";
                break;
            case ":":
                out += ":";
                break;
            default:
                out += x;
        }
        i += 1;
    }
    return out;
}

/** Whether a format string is a date/time one: date or time tokens outside quotes and escapes, and no digit placeholders. */
export function isDateFormat(f: string): boolean {
    const lower = lowerAscii(f);
    if (lower.indexOf("am/pm") >= 0 || lower.indexOf("a/p") >= 0 || lower.indexOf("ampm") >= 0) {
        return true;
    }
    let quoted = false;
    let date = false;
    let digits = false;
    const ch = Array.from(f);
    for (let i = 0; i < ch.length; i++) {
        const x = ch[i];
        if (x === "\"") {
            quoted = !quoted;
        } else if (quoted) {
            continue;
        } else if (x === "\\") {
            i++;
        } else if ("dDmMyYhHnNsSqQwWcCtT".indexOf(x) >= 0) {
            date = true;
        } else if (x === "0" || x === "#") {
            digits = true;
        }
    }
    return date && !digits;
}

/** A named date format as Power BI's visuals draw it (`General Date` with its time, midnight included; Desktop's `d`, `D`, `G`, `T`, `t`); undefined for another format. */
function namedDate(f: string, c: DateCulture, p: DateParts): string | null | undefined {
    const letters: { [k: string]: string } = { d: "Short Date", D: "Long Date", G: "General Date", T: "Long Time", t: "Short Time" };
    const name = Object.prototype.hasOwnProperty.call(letters, f) ? letters[f] : f;
    switch (lowerAscii(name)) {
        case "general date":
            return vbDate(p, "ddddd ttttt", c);
        case "long date":
            return vbDate(p, "dddddd", c);
        case "medium date":
            return datePattern(p, c.mediumDate, c);
        case "short date":
            return vbDate(p, "ddddd", c);
        case "long time":
            return vbDate(p, "ttttt", c);
        case "medium time":
            return datePattern(p, c.mediumTime, c);
        case "short time":
            return datePattern(p, "HH:mm", c);
        default:
            return undefined;
    }
}

/** A date written with a column's format string in `locale`'s language; null when the format is empty or not a date one (the caller keeps its own text). */
export function formatDateParts(p: DateParts, format: string, locale?: string | null): string | null {
    const f = typeof format === "string" ? format.trim() : "";
    if (f === "") {
        return null;
    }
    const c = dateCultureOf(locale);
    const named = namedDate(f, c, p);
    if (typeof named === "string") {
        return named;
    }
    return isDateFormat(f) ? vbDate(p, f, c) : null;
}

/** A label of milliseconds since 1970-01-01 (the value's own clock) with a column's format string, as BIcraft's renderer writes it (`bicraft_render::fmt::date_label`); null: keep the layout's own text. */
export function dateLabel(ms: number, format: string, locale?: string | null): string | null {
    const p = datePartsOfMs(ms);
    return p ? formatDateParts(p, format, locale) : null;
}

/** Writes a date (read on its own clock) with a date format string; without one, or with another kind of format, `yyyy-MM-dd` and the time when it is set. */
export function formatDate(d: Date, format: string, locale?: string | null): string {
    const p: DateParts = {
        y: d.getFullYear(),
        M: d.getMonth() + 1,
        d: d.getDate(),
        w: d.getDay(),
        H: d.getHours(),
        m: d.getMinutes(),
        s: d.getSeconds(),
        timeOnly: d.getFullYear() === 1899 && d.getMonth() === 11 && d.getDate() === 30,
    };
    const text = p.y >= 100 && p.y <= 9999 ? formatDateParts(p, format, locale) : null;
    if (text !== null) {
        return text;
    }
    const date = `${pad2(p.y, 4)}-${pad2(p.M, 2)}-${pad2(p.d, 2)}`;
    return p.H !== 0 || p.m !== 0 || p.s !== 0 ? `${date} ${pad2(p.H, 2)}:${pad2(p.m, 2)}:${pad2(p.s, 2)}` : date;
}
