// BIcraft's visual runtime (docs/community-visuals.md "Runtime", docs/formats/visual-look.md): the
// look every BIcraft visual shares. A look comes from the visual's Style options (a preset,
// Modern, Minimal or Bold, and switches for rounded corners, gradients, shadows and animation)
// and the report theme's colours; it gives the colour tokens and the marks (bars, tiles, cards,
// dots, value labels) as drawing primitives. This is the TypeScript twin of
// crates/render/src/look.rs: both build the same primitives, which the engine's golden tests
// compare. Nothing here touches the browser, so the tests run it under Node.
"use strict";

export type Prim = { [key: string]: any };

/**
 * BIcraft's own categorical palette (no theme of the report's own, or `palette: bicraft`), in its
 * fixed order: indigo, vermilion, teal, amber, magenta, sky, violet, crimson.
 */
export const BICRAFT_PALETTE = ["#4A5BE8", "#EA5A28", "#0E9F84", "#BF8700", "#D6408E", "#1C8FCC", "#8A4FE0", "#E23D4F"];
/** The palette of the 1.x visuals: the Minimal preset's when the report gives no theme. */
export const LEGACY_PALETTE = ["#1F6F8B", "#E07A1F", "#5A9E3A", "#C23B5A", "#7A5BA8", "#2E9C9C", "#B08A1E", "#6B6B6B"];
/**
 * The first data colours of Power BI's stock themes (the default theme since 2020 and the classic
 * one): a report on one of them has no palette of its own, so Modern and Bold draw with BIcraft's.
 */
const STOCK_THEMES = [
    ["#118DFF", "#12239E", "#E66C37", "#6B007B", "#E044A7", "#744EC2", "#D9B300", "#D64550"],
    ["#01B8AA", "#374649", "#FD625E", "#F2C80F", "#5F6B6D", "#8AD4EB", "#FE9666", "#A66999"],
];
export const PRESETS = ["modern", "minimal", "bold"];
export const PALETTES = ["theme", "bicraft"];
/** Most theme colours read. */
const MAX_THEME = 16;
const WHITE = "#FFFFFF";
const BLACK = "#000000";

/** The colours, sizes and switches a visual draws with. */
export interface Look {
    preset: string;
    rounded: boolean;
    gradient: boolean;
    shadow: boolean;
    motion: boolean;
    /** The categorical colours, in order (theme or BIcraft's). */
    palette: string[];
    /** Axis tick and category text (at least 4.5:1 on white). */
    text: string;
    /** Values and names. */
    label: string;
    grid: string;
    axis: string;
    /** The surface the marks sit on (gaps and rings). */
    surface: string;
    /** The dark ink of a label inside a fill. */
    dark: string;
    /** A cell without data. */
    empty: string;
    /** Qualitative bands, darkest first. */
    shades: string[];
    /** The report's surface is dark (a dark theme): light inks on dark greys. */
    night: boolean;
    /** A rounded card behind the plot. */
    card: boolean;
    /** Value labels on a rounded chip. */
    pills: boolean;
    /** Sizes: axis ticks, names (categories, legends), value labels. */
    tick: number;
    name: number;
    value: number;
}

export function r2(v: number): number {
    return Math.floor(v * 100 + 0.5) / 100;
}

export function isColor(c: any): boolean {
    return typeof c === "string" && /^#[0-9A-Fa-f]{6}$/.test(c);
}

/** The theme's data colours (`{colors: ["#RRGGBB", …]}`), upper case, at most 16. */
export function themeColors(theme: any): string[] {
    const l: any[] = theme && typeof theme === "object" && Array.isArray(theme.colors) ? theme.colors : [];
    const out: string[] = [];
    for (let i = 0; i < l.length && out.length < MAX_THEME; i++) {
        if (isColor(l[i])) {
            out.push(String(l[i]).toUpperCase());
        }
    }
    return out;
}

function flag(v: any, def: boolean): boolean {
    return typeof v === "boolean" ? v : def;
}

/** Whether `colors` start with a stock Power BI theme's data colours. */
function isStock(colors: string[]): boolean {
    return STOCK_THEMES.some((t) => colors.length >= t.length && t.every((c, i) => colors[i] === c));
}

/** The report's background (`theme.background`, `#RRGGBB`) when it is dark, else "". */
function darkBackground(theme: any): string {
    const bg = theme && typeof theme === "object" ? theme.background : undefined;
    return isColor(bg) && lum(String(bg)) < 0.07 ? String(bg).toUpperCase() : "";
}

/**
 * A look from the Style options (each may be missing or invalid: the preset's default) and the
 * theme (`{colors, background?}`).
 */
export function makeLook(preset: any, palette: any, rounded: any, gradient: any, shadow: any, motion: any, theme: any, card?: any, pills?: any): Look {
    const p = typeof preset === "string" && PRESETS.indexOf(preset) >= 0 ? preset : "modern";
    const minimal = p === "minimal";
    const colors = themeColors(theme);
    // Modern and Bold take BIcraft's colours over a stock theme's
    const own = palette === "bicraft" || colors.length === 0 || (!minimal && isStock(colors));
    const pal = own ? (minimal && palette !== "bicraft" ? LEGACY_PALETTE : BICRAFT_PALETTE) : colors;
    const withCard = flag(card, !minimal);
    const bg = darkBackground(theme);
    const night = bg !== "";
    const base = night ? bg : WHITE;
    return {
        preset: p,
        rounded: flag(rounded, !minimal),
        gradient: flag(gradient, !minimal),
        shadow: flag(shadow, !minimal),
        motion: flag(motion, !minimal),
        palette: pal.slice(),
        text: night ? "#CFCEC4" : minimal ? "#555555" : "#5F5E5A",
        label: night ? "#F0EFEC" : minimal ? "#333333" : "#1F1E1C",
        grid: night ? "#3B3A37" : minimal ? "#E5E5E5" : "#E8E7E2",
        axis: night ? "#5E5D58" : minimal ? "#B3B3B3" : "#C8C7C0",
        surface: withCard ? (night ? mix(bg, WHITE, 0.06) : CARD) : base,
        dark: minimal ? "#252423" : BLACK,
        empty: night ? "#34332F" : minimal ? "#EFEFEF" : "#F0EFEB",
        shades: night
            ? ["#6E6D68", "#5E5D59", "#4F4E4A", "#43423F", "#393835"]
            : minimal
              ? ["#9E9E9E", "#BDBDBD", "#D3D3D3", "#E4E4E4", "#F0F0F0"]
              : ["#C4C3BC", "#D5D4CE", "#E2E1DC", "#ECEBE7", "#F5F4F1"],
        night,
        card: withCard,
        pills: flag(pills, p === "bold"),
        tick: minimal ? 10 : 11,
        name: minimal ? 11 : 12,
        value: minimal ? 10 : p === "bold" ? 12 : 11,
    };
}

/** The light card's fill behind the plot (5.9:1 under the axis text). */
const CARD = "#F7F6F3";

/** The look of a visual whose Style options are an object `{preset, palette, rounded, gradient, shadow, animation, card, pills}`. */
export function lookOf(style: any, theme: any): Look {
    const s = style && typeof style === "object" ? style : {};
    return makeLook(s.preset, s.palette, s.rounded, s.gradient, s.shadow, s.animation, theme, s.card, s.pills);
}

/** The look of a chart pack visual, whose options are flat (`stylePreset`, `styleRounded`…). */
export function lookFlat(o: any, theme: any): Look {
    const s = o && typeof o === "object" ? o : {};
    return makeLook(s.stylePreset, s.stylePalette, s.styleRounded, s.styleGradient, s.styleShadow, s.styleAnimation, theme, s.styleCard, s.stylePills);
}

export function minimal(k: Look): boolean {
    return k.preset === "minimal";
}

export function bold(k: Look): boolean {
    return k.preset === "bold";
}

/** The `i`-th categorical colour. */
export function color(k: Look, i: number): string {
    const n = Math.max(k.palette.length, 1);
    return k.palette[((i % n) + n) % n] || BICRAFT_PALETTE[0];
}

/** The corner radius of bars and boxes (0: square). */
export function radius(k: Look): number {
    return k.rounded ? (bold(k) ? 6 : 4) : 0;
}

/** The weight of value labels. */
export function valueWeight(k: Look): number {
    return minimal(k) ? 400 : bold(k) ? 700 : 600;
}

/** The width of a connecting stem (a dumbbell's) and of a data line (a slope's). */
export function stemWidth(k: Look): number {
    return minimal(k) ? 2 : bold(k) ? 5 : 4;
}

export function lineWidth(k: Look): number {
    return minimal(k) ? 2 : bold(k) ? 3 : 2.5;
}

function hexOf(c: string): number[] {
    const p = (i: number) => {
        const v = parseInt(c.slice(i, i + 2), 16);
        return isNaN(v) ? 0 : v;
    };
    return [p(1), p(3), p(5)];
}

/** `a` moved towards `b` by `t` (0..1), each channel rounded half up. */
export function mix(a: string, b: string, t: number): string {
    const x = hexOf(a);
    const y = hexOf(b);
    const ch = (p: number, q: number) => {
        const v = Math.min(Math.max(Math.floor(p + (q - p) * t + 0.5), 0), 255);
        const h = v.toString(16).toUpperCase();
        return h.length < 2 ? "0" + h : h;
    };
    return "#" + ch(x[0], y[0]) + ch(x[1], y[1]) + ch(x[2], y[2]);
}

function lum(c: string): number {
    const v = hexOf(c).map((x) => {
        const s = x / 255;
        return s <= 0.03928 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4);
    });
    return 0.2126 * v[0] + 0.7152 * v[1] + 0.0722 * v[2];
}

/** The WCAG contrast ratio of two colours. */
export function contrast(a: string, b: string): number {
    const x = lum(a);
    const y = lum(b);
    return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
}

/** The ink of a label inside a fill (from `a` to `b` when it is a gradient): white or the look's dark, whichever reads better on both. */
export function onColor(k: Look, a: string, b: string): string {
    const w = Math.min(contrast(WHITE, a), contrast(WHITE, b));
    const d = Math.min(contrast(k.dark, a), contrast(k.dark, b));
    return w >= d ? WHITE : k.dark;
}

/** The lighter top colour of a fill drawn as a gradient ("" when gradients are off). */
export function gradTop(k: Look, fill: string): string {
    return k.gradient ? mix(fill, WHITE, bold(k) ? 0.32 : 0.22) : "";
}

/** Adds the decorations BIcraft's renderer and the SVG drawer read; only set ones are written. */
function deco(p: Prim, rx: number, sq: string, grad: string, stroke: string, sw: number, shadow: number): Prim {
    if (rx > 0) {
        p.rx = r2(rx);
        if (sq !== "") {
            p.sq = sq;
        }
    }
    if (grad !== "") {
        p.grad = grad;
    }
    if (stroke !== "") {
        p.stroke = stroke;
        p.sw = r2(sw);
    }
    if (shadow > 0) {
        p.shadow = shadow;
    }
    return p;
}

/**
 * How much of its opacity a mark keeps behind its highlighted part, or when another visual's
 * selection highlights none of its rows (docs/formats/bicraft-visuals.md "Highlights"; the same
 * as the selection's dimming).
 */
export const HIGHLIGHT_DIM = 0.4;

/** `p` drawn as the full value behind its highlighted part: a rect, circle or poly at HIGHLIGHT_DIM of its opacity (others as they are). */
export function dim(p: Prim): Prim {
    if ((p.t === "rect" || p.t === "circle" || p.t === "poly") && typeof p.opacity === "number") {
        return Object.assign({}, p, { opacity: r2(p.opacity * HIGHLIGHT_DIM) });
    }
    return p;
}

/** The highlighted part of value `v` to draw over it (the data view's `highlights` entry `h`), kept between 0 and `v`; null without a highlight or when nothing of `v` is highlighted. */
export function part(v: number, h: number | null): number | null {
    if (typeof h !== "number" || !isFinite(h) || !isFinite(v)) {
        return null;
    }
    const p = v >= 0 ? Math.min(Math.max(h, 0), v) : Math.min(Math.max(h, v), 0);
    return p === 0 ? null : p;
}

function rectP(x: number, y: number, w: number, h: number, fill: string, opacity: number): Prim {
    return { t: "rect", x: r2(x), y: r2(y), w: r2(w), h: r2(h), fill, opacity };
}

/**
 * A bar from a baseline: its corners rounded except on the side `sq` (`b`, `t`, `l`, `r`: the
 * baseline's; "" rounds all four), a gradient, Bold's crisp edge and shadow.
 */
export function bar(k: Look, x: number, y: number, w: number, h: number, fill: string, sq: string): Prim {
    const r = Math.min(radius(k), w / 2, h / 2);
    const b = bold(k);
    return deco(rectP(x, y, w, h, fill, 1), r >= 0.5 ? r : 0, sq, gradTop(k, fill), b ? mix(fill, BLACK, 0.22) : "", 1, b && k.shadow ? 3 : 0);
}

/** A band behind a bar (a bullet's): no gradient, its corners rounded except on side `sq` (`x`: none rounded). */
export function band(k: Look, x: number, y: number, w: number, h: number, fill: string, sq: string): Prim {
    const r = sq === "x" ? 0 : Math.min(radius(k), w / 2, h / 2);
    return deco(rectP(x, y, w, h, fill, 1), r >= 0.5 ? r : 0, sq, "", "", 0, 0);
}

/** A box (a box plot's quartiles): translucent; Modern and Bold round it, shade it and draw its edge in its colour (Minimal draws the edge as a separate path). */
export function boxed(k: Look, x: number, y: number, w: number, h: number, fill: string): Prim {
    if (boxPlain(k)) {
        return rectP(x, y, w, h, fill, 0.3);
    }
    const r = Math.min(radius(k), w / 2, h / 2);
    return deco(rectP(x, y, w, h, fill, 0.22), r >= 0.5 ? r : 0, "", gradTop(k, fill), fill, bold(k) ? 2 : 1.5, 0);
}

/** Whether a box is drawn plain, its edge a separate path. */
export function boxPlain(k: Look): boolean {
    return minimal(k) && !k.rounded && !k.gradient;
}

/** A tile of a grid or a mosaic: all corners rounded up to `max`, a softer gradient when `soft`. */
export function tile(k: Look, x: number, y: number, w: number, h: number, fill: string, max: number, soft: boolean): Prim {
    const r = k.rounded ? Math.min(max, w / 2, h / 2) : 0;
    return deco(rectP(x, y, w, h, fill, 1), r >= 0.5 ? r : 0, "", soft && k.gradient ? mix(fill, WHITE, 0.14) : "", "", 0, 0);
}

/** A card (an organization chart's node): rounded, its hairline edge, a gentle shadow. */
export function card(k: Look, x: number, y: number, w: number, h: number, fill: string, edge: string): Prim {
    const r = k.rounded ? Math.min(bold(k) ? 10 : 8, w / 2, h / 2) : 0;
    return deco(rectP(x, y, w, h, fill, 1), r >= 0.5 ? r : 0, "", "", edge, 1, k.shadow ? (bold(k) ? 8 : 6) : 0);
}

/** An accent strip down a card's left edge, its left corners rounded as the card's. */
export function accent(k: Look, x: number, y: number, w: number, h: number, fill: string): Prim {
    const r = k.rounded ? Math.min(bold(k) ? 10 : 8, w, h / 2) : 0;
    return deco(rectP(x, y, w, h, fill, 1), r >= 0.5 ? r : 0, "r", "", "", 0, 0);
}

function circleP(cx: number, cy: number, r: number, fill: string, opacity: number): Prim {
    return { t: "circle", cx: r2(cx), cy: r2(cy), r: r2(r), fill, opacity };
}

/** A dot: Modern and Bold ring it with the surface so it reads where it crosses other marks. */
export function dot(k: Look, cx: number, cy: number, r: number, fill: string, opacity: number): Prim {
    return deco(circleP(cx, cy, r, fill, opacity), 0, "", "", minimal(k) ? "" : k.surface, 1.5, 0);
}

/** A large translucent disc (a Venn set): a gradient, its edge in its colour, a gentle shadow. */
export function blob(k: Look, cx: number, cy: number, r: number, fill: string, opacity: number): Prim {
    const m = minimal(k);
    return deco(circleP(cx, cy, r, fill, m ? opacity : opacity * 0.8), 0, "", gradTop(k, fill), m ? "" : fill, bold(k) ? 2 : 1.5, k.shadow ? (bold(k) ? 5 : 4) : 0);
}

/** A legend swatch. */
export function swatch(k: Look, x: number, y: number, s: number, fill: string): Prim {
    return deco(rectP(x, y, s, s, fill, 1), k.rounded ? s / 2 - 2 : 0, "", "", "", 0, 0);
}

/** A text primitive, its weight written when not regular. */
export function textW(x: number, y: number, s: string, size: number, fill: string, anchor: string, rotate: number, weight: number): Prim {
    const p: Prim = { t: "text", x: r2(x), y: r2(y), text: s, size, fill, anchor, rotate };
    if (weight !== 400) {
        p.weight = weight;
    }
    return p;
}

/** A text's estimated width, as the layouts estimate it. */
function estW(s: string, size: number): number {
    return Array.from(s).length * size * 0.55;
}

/**
 * The card behind a `w`×`h` visual's plot (null without one): a rounded surface with a hairline
 * edge; high contrast draws it in the background colour (`role: card`).
 */
export function cardOf(k: Look, w: number, h: number): Prim | null {
    if (!k.card || w < 24 || h < 24) {
        return null;
    }
    const p = deco(rectP(1, 1, w - 2, h - 2, k.surface, 1), k.rounded ? (bold(k) ? 14 : 12) : 0, "", "", k.night ? mix(k.surface, WHITE, 0.08) : "#E9E8E3", 1, 0);
    p.role = "card";
    return p;
}

/** Pushes the card of a `w`×`h` visual (when the look has one) onto `out`. */
export function pushCard(out: Prim[], k: Look, w: number, h: number): void {
    const c = cardOf(k, w, h);
    if (c) {
        out.push(c);
    }
}

/**
 * Pushes a value label for the point (`x`, `y`) of a mark onto `out`, `gap` pixels away on side
 * `side` (`n` above, `s` below, `e` right, `w` left, `c` on it), at the look's value size and
 * weight; with pills, on a rounded chip of the label ink (`role: pill`) in the ink that reads on
 * it. Returns the index of the text.
 */
export function valueLabel(out: Prim[], k: Look, x: number, y: number, s: string, side: string, gap: number, size: number): number {
    const padX = k.pills ? 5 : 0;
    const v = side === "n" || side === "s" ? gap + (size - 10) / 2 + (k.pills ? 3 : 0) : 0;
    const tx = side === "e" ? x + gap + padX : side === "w" ? x - gap - padX : x;
    // Modern and Bold keep a label above its mark inside the visual
    const above = Math.max(y - v, minimal(k) ? -Infinity : size / 2 + (k.pills ? 3 : 0) + 1);
    const ty = side === "n" ? above : side === "s" ? y + v : y;
    const anchor = side === "e" ? "start" : side === "w" ? "end" : "middle";
    let ink = k.label;
    if (k.pills) {
        const wd = estW(s, size) + 2 * 5;
        const ht = size + 6;
        const px = anchor === "start" ? tx - 5 : anchor === "end" ? tx - wd + 5 : tx - wd / 2;
        const pill = deco(rectP(px, ty - ht / 2, wd, ht, k.label, 1), ht / 2, "", "", "", 0, 0);
        pill.role = "pill";
        out.push(pill);
        ink = onColor(k, k.label, k.label);
    }
    out.push(textW(tx, ty, s, size, ink, anchor, 0, valueWeight(k)));
    return out.length - 1;
}
