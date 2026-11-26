// BIcraft's chart pack (docs/formats/chart-pack.md): what the visuals' `IVisual` classes
// share. A visual reads its roles from the categorical data view into the layout's input
// (`{rows, options, lang}`), lays it out with the same code the golden tests run (./kit and its
// layout), draws the primitives as SVG and wires the data points to BIcraft's visual runtime
// (./runtime: tooltips, selection, context menu, keyboard focus, high contrast, landing page).
// Only the types of powerbi-visuals-api are used: the host page provides the API at run time.
"use strict";

import powerbi from "powerbi-visuals-api";
import VisualConstructorOptions = powerbi.extensibility.visual.VisualConstructorOptions;
import VisualUpdateOptions = powerbi.extensibility.visual.VisualUpdateOptions;
import { Hit, Out, Prim } from "./kit";
import { contrastOf, contrastPrim } from "./runtime/contrast";
import { boundRoles, objectValue } from "./runtime/data";
import { formatValue } from "./runtime/format";
import { Interactions, Target } from "./runtime/interactions";
import { clear, landingText, showLanding, showMessage } from "./runtime/landing";
import { decorate, freshData, hostTheme, lookSvg, motionAllowed } from "./runtime/paint";
import { langOf } from "./runtime/strings";
import { Look, lookFlat } from "./runtime/style";
import { TooltipItem, tooltipText } from "./runtime/tooltips";

const SVG = "http://www.w3.org/2000/svg";
/** Most rows read from the data view (each layout keeps fewer). */
const MAX_ROWS = 30000;
/** Most primitives drawn. */
const MAX_PRIMS = 20000;

/** A role's column of the categorical data view. */
export interface RoleColumn {
    name: string;
    format: string;
    values: any[];
    /** The data view's column (for selection ids). */
    source: any;
}

function columnOf(c: any): RoleColumn {
    const src = c && c.source ? c.source : {};
    return {
        name: typeof src.displayName === "string" ? src.displayName : "",
        format: typeof src.format === "string" ? src.format : "",
        values: c && Array.isArray(c.values) ? c.values.slice(0, MAX_ROWS) : [],
        source: c,
    };
}

function hasRole(c: any, role: string): boolean {
    return !!(c && c.source && c.source.roles && c.source.roles[role]);
}

/** The column of `role` among the categories, then the values, of the data view. */
export function roleColumn(dataView: any, role: string): RoleColumn | null {
    const cat = dataView && dataView.categorical ? dataView.categorical : null;
    const cats: any[] = cat && Array.isArray(cat.categories) ? cat.categories : [];
    for (const c of cats) {
        if (hasRole(c, role)) {
            return columnOf(c);
        }
    }
    const vals: any[] = cat && cat.values && typeof cat.values.length === "number" ? cat.values : [];
    for (let k = 0; k < vals.length && k < 64; k++) {
        if (hasRole(vals[k], role)) {
            return columnOf(vals[k]);
        }
    }
    return null;
}

function pad(n: number, w: number): string {
    let s = String(Math.abs(n));
    while (s.length < w) {
        s = "0" + s;
    }
    return n < 0 ? "-" + s : s;
}

/** A value as the layout's input takes it: a Date as ISO text on its own clock, numbers, texts and booleans as they are, else null. */
export function cellOf(v: any): any {
    if (v instanceof Date) {
        if (!isFinite(v.getTime())) {
            return null;
        }
        return `${pad(v.getFullYear(), 4)}-${pad(v.getMonth() + 1, 2)}-${pad(v.getDate(), 2)}T${pad(v.getHours(), 2)}:${pad(v.getMinutes(), 2)}:${pad(v.getSeconds(), 2)}`;
    }
    if (typeof v === "number" || typeof v === "string" || typeof v === "boolean") {
        return v;
    }
    return null;
}

/** The rows of the columns (one cell per column, null for a missing one). */
export function rowsOf(cols: (RoleColumn | null)[]): any[][] {
    let n = 0;
    for (const c of cols) {
        n = Math.max(n, c ? c.values.length : 0);
    }
    const out: any[][] = [];
    for (let i = 0; i < n && i < MAX_ROWS; i++) {
        out.push(cols.map((c) => (c ? cellOf(c.values[i]) : null)));
    }
    return out;
}

/** The columns' format strings, in the order of the rows' cells ("" for a missing column). */
export function formatsOf(cols: (RoleColumn | null)[]): string[] {
    return cols.map((c) => (c ? c.format : ""));
}

/** The selection id of a row over the given category columns (undefined without them or a host). */
export function idOf(host: any, cols: (RoleColumn | null)[], row: number): any {
    if (!host || typeof host.createSelectionIdBuilder !== "function" || row < 0) {
        return undefined;
    }
    try {
        let b = host.createSelectionIdBuilder();
        let any = false;
        for (const c of cols) {
            if (c) {
                b = b.withCategory(c.source, row);
                any = true;
            }
        }
        return any ? b.createSelectionId() : undefined;
    } catch (e) {
        return undefined;
    }
}

/** A tooltip line: the column's name and its value at `row` written with its format. */
export function tipLine(c: RoleColumn | null, row: number, locale: string): TooltipItem[] {
    return c && row >= 0 && row < c.values.length ? [{ displayName: c.name, value: formatValue(c.values[row], c.format, locale) }] : [];
}

/** The Style card's options (format object `style`) as the layouts' flat keys. */
export function styleOptions(dv: any): { [k: string]: any } {
    const out: { [k: string]: any } = {};
    const keys: [string, string][] = [["preset", "stylePreset"], ["palette", "stylePalette"], ["rounded", "styleRounded"], ["gradient", "styleGradient"], ["shadow", "styleShadow"], ["animation", "styleAnimation"], ["card", "styleCard"], ["pills", "stylePills"]];
    const objects = dv && dv.metadata ? dv.metadata.objects : undefined;
    const style = objects && typeof objects === "object" ? objects.style : undefined;
    for (const [p, k] of keys) {
        const v = style && typeof style === "object" ? style[p] : undefined;
        if (typeof v === "string" || typeof v === "boolean") {
            out[k] = v;
        }
    }
    return out;
}

/** What a hit becomes: its selection id and tooltip lines (null: not a data point). */
export interface Point {
    id: any;
    tip: TooltipItem[];
}

export abstract class PackVisual {
    protected readonly root: HTMLElement;
    protected readonly host: any;
    protected readonly locale: string;
    protected readonly interactions: Interactions;
    protected dataView: any = null;
    private w = 0;
    private h = 0;
    /** The next drawing plays the entry animation (new data). */
    private fresh = true;

    constructor(options: VisualConstructorOptions, cls: string) {
        this.host = (options as any).host;
        this.locale = this.host && typeof this.host.locale === "string" ? this.host.locale : "";
        this.root = document.createElement("div");
        this.root.className = "bicraft-pack " + cls;
        options.element.appendChild(this.root);
        this.interactions = new Interactions(this.host, this.root);
    }

    /** The visual's name (the landing page's title and the points' container label). */
    protected abstract title(): string;
    /** The roles that must have a field, with their display names in English and French. */
    protected abstract required(): { role: string; en: string; fr: string }[];
    /** The layout's input (`rows`, `options`) from the data view. */
    protected abstract input(dataView: any): any;
    protected abstract layout(input: any, w: number, h: number): Out;
    /** A hit's selection id and tooltip, or null when it is no data point (a toggle). */
    protected abstract point(hit: Hit): Point | null;
    /** The role of the measure whose `highlights` the layout reads (null: none). */
    protected measureRole(): string | null {
        return null;
    }
    /** After drawing: the hits that are no data points get their own handling. */
    protected wired(svg: Element, out: Out, drawn: (Element | null)[]): void {
        // nothing by default
    }

    public update(options: VisualUpdateOptions): void {
        this.dataView = options.dataViews ? options.dataViews[0] : null;
        this.w = options.viewport ? options.viewport.width : 0;
        this.h = options.viewport ? options.viewport.height : 0;
        this.fresh = freshData(options);
        this.changed();
        this.draw();
    }

    /** The look the visual draws with (its Style options and the report theme). */
    protected look(): Look {
        const input = this.input(this.dataView);
        return lookFlat(Object.assign({}, input && input.options, styleOptions(this.dataView)), hostTheme(this.host));
    }

    /** The palette option as set (`theme` or `bicraft`). */
    protected paletteOption(): string {
        return String(objectValue(this.dataView, "style", "palette", "theme"));
    }

    /** Called with each update before drawing. */
    protected changed(): void {
        // nothing by default
    }

    protected draw(): void {
        const contrast = contrastOf(this.host);
        this.root.classList.toggle("bicraft-hc", contrast !== null);
        this.root.style.background = contrast ? contrast.background : "";
        this.root.style.setProperty("--bicraft-fg", contrast ? contrast.foreground : "");
        this.root.style.setProperty("--bicraft-focus", contrast ? contrast.selected : "");
        clear(this.root);
        const fr = langOf(this.locale) === "fr";
        const bound = boundRoles(this.dataView);
        const missing = this.required().filter((r) => bound.indexOf(r.role) < 0);
        const names = missing.map((r) => (fr ? r.fr : r.en));
        if (bound.length === 0) {
            showLanding(this.root, this.title(), landingText(this.locale, this.required().map((r) => (fr ? r.fr : r.en))));
            return;
        }
        if (missing.length > 0) {
            showMessage(this.root, landingText(this.locale, names));
            return;
        }
        const input = this.input(this.dataView);
        input.lang = fr ? "fr" : "en";
        input.options = Object.assign({}, input.options, styleOptions(this.dataView));
        input.theme = hostTheme(this.host);
        // another visual's selection highlighting this one: the measure's highlighted parts
        const role = this.measureRole();
        const measure = role ? roleColumn(this.dataView, role) : null;
        const hl = measure && measure.source && Array.isArray(measure.source.highlights) ? measure.source.highlights : null;
        if (hl) {
            input.highlights = hl.slice(0, MAX_ROWS).map(cellOf);
        }
        const out = this.layout(input, this.w, this.h);
        const svg = document.createElementNS(SVG, "svg");
        svg.setAttribute("width", String(this.w));
        svg.setAttribute("height", String(this.h));
        lookSvg(svg, this.fresh && motionAllowed(lookFlat(input.options, input.theme).motion, contrast !== null));
        this.fresh = false;
        const drawn: (Element | null)[] = [];
        const hitPrims: { [k: number]: boolean } = {};
        for (const hit of out.hits) {
            hitPrims[hit.prim] = true;
        }
        const prims = out.prims.slice(0, MAX_PRIMS);
        for (let i = 0; i < prims.length; i++) {
            const q = contrast ? contrastPrim(prims[i], contrast) : prims[i];
            const el = decorate(svg, q, element(q));
            if (el && prims[i].t === "text" && !hitPrims[i]) {
                // labels let the pointer through to the mark below
                el.setAttribute("pointer-events", "none");
            }
            drawn.push(el);
            if (el) {
                svg.appendChild(el);
            }
        }
        const targets: Target[] = [];
        const ids: any[] = [];
        const tips: TooltipItem[][] = [];
        for (const hit of out.hits) {
            const el = hit.prim < drawn.length ? drawn[hit.prim] : null;
            const pt = el ? this.point(hit) : null;
            if (el && pt) {
                targets.push({ row: targets.length, el, fade: hit.fade === true });
                ids.push(pt.id);
                tips.push(pt.tip);
            }
        }
        svg.setAttribute("aria-label", this.interactions.summary(this.title(), targets.length));
        this.root.appendChild(svg);
        this.interactions.wire({
            container: svg,
            targets,
            ids: ids.some((id) => id !== undefined) ? ids : [],
            tooltip: (t) => (t >= 0 && t < tips.length ? tips[t] : []),
            label: (t) => tooltipText(t >= 0 && t < tips.length ? tips[t] : []),
            contrast,
        });
        this.wired(svg, out, drawn);
    }
}

/** The SVG element of a primitive. */
export function element(p: Prim): Element | null {
    const el = (name: string, attrs: { [k: string]: any }) => {
        const e = document.createElementNS(SVG, name);
        for (const k of Object.keys(attrs)) {
            e.setAttribute(k, String(attrs[k]));
        }
        return e;
    };
    switch (p.t) {
        case "rect":
            return el("rect", { x: p.x, y: p.y, width: Math.max(p.w, 0), height: Math.max(p.h, 0), fill: p.fill, "fill-opacity": p.opacity });
        case "line":
            return el("line", { x1: p.x1, y1: p.y1, x2: p.x2, y2: p.y2, stroke: p.stroke, "stroke-width": p.width });
        case "path": {
            // an open polyline (a series' line, a calendar's month edge)
            const pts = Array.isArray(p.points) ? p.points.map((q: number[]) => q[0] + "," + q[1]).join(" ") : "";
            return el("polyline", { points: pts, fill: "none", stroke: p.stroke, "stroke-width": p.width, "stroke-linejoin": "round", "stroke-linecap": "round" });
        }
        case "poly": {
            const pts = Array.isArray(p.points) ? p.points.map((q: number[]) => q[0] + "," + q[1]).join(" ") : "";
            const attrs: { [k: string]: any } = { points: pts, fill: p.fill, "fill-opacity": p.opacity };
            if (typeof p.stroke === "string") {
                // high contrast draws an area as an outline
                attrs.stroke = p.stroke;
                attrs["stroke-width"] = 1;
            }
            return el("polygon", attrs);
        }
        case "circle":
            return el("circle", { cx: p.cx, cy: p.cy, r: Math.max(p.r, 0), fill: p.fill, "fill-opacity": p.opacity });
        case "text": {
            const t = el("text", { x: p.x, y: p.y, fill: p.fill, "font-size": p.size, "text-anchor": p.anchor, "dominant-baseline": "central" });
            t.textContent = p.text;
            return t;
        }
        default:
            return null;
    }
}

