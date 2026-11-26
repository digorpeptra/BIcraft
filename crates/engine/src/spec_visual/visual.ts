// The BIcraft Spec visual (docs/formats/spec-visual.md): draws the declarative chart spec stored
// in its `spec.json` format property over the data of its `category` and `values` roles, as SVG.
// BIcraft writes this file and compiles it with its own TypeScript build (visual.build, no Node);
// the layout is in ./spec.ts, the interactions (tooltips, selection, context menu, keyboard,
// high contrast, landing page) in BIcraft's visual runtime (./runtime, docs/community-visuals.md
// "Runtime"). Only the types of powerbi-visuals-api are used: the host page provides the API at
// run time.
"use strict";

import powerbi from "powerbi-visuals-api";
import VisualConstructorOptions = powerbi.extensibility.visual.VisualConstructorOptions;
import VisualUpdateOptions = powerbi.extensibility.visual.VisualUpdateOptions;
import IVisual = powerbi.extensibility.visual.IVisual;
import DataView = powerbi.DataView;
import { Cat, Hit, Prim, Spec, SpecData, catOf, formatOf, layout, parseText, resolve, textSet } from "./spec";
import { Contrast, contrastOf, contrastPrim } from "./runtime/contrast";
import { Table, boundRoles, tableOf } from "./runtime/data";
import { card, group, model, textArea, withLook } from "./runtime/formatting";
import { Interactions, Target } from "./runtime/interactions";
import { clear, landingText, showLanding, showMessage } from "./runtime/landing";
import { decorate, freshData, hostTheme, lookSvg, motionAllowed } from "./runtime/paint";
import { categoryIds } from "./runtime/selection";
import { lookOf, valueWeight } from "./runtime/style";
import { TextShown, mergeText, textCards, textOf } from "./runtime/textsettings";
import { langOf, pick } from "./runtime/strings";
import { tooltipItems, tooltipText } from "./runtime/tooltips";

const SVG = "http://www.w3.org/2000/svg";
/** Most primitives drawn (the layout's own bounds keep far below). */
const MAX_PRIMS = 20000;
/** Radius of the invisible hit area around a line's or area's vertex. */
const HIT_RADIUS = 6;

const ROLE_NAMES = { en: ["Category", "Values"], fr: ["Catégorie", "Valeurs"] };
const NO_SPEC = {
    en: "This visual has no spec yet: set it with BIcraft (visual.set_spec).",
    fr: "Ce visuel n'a pas encore de spécification : définissez-la avec BIcraft (visual.set_spec).",
};
const SPEC_ERROR = { en: "The spec cannot be drawn: ", fr: "La spécification ne peut pas être dessinée : " };

export class Visual implements IVisual {
    private readonly root: HTMLElement;
    private readonly host: any;
    private readonly locale: string;
    private readonly interactions: Interactions;
    private specText = "";
    /** The look's settings drawn with (the spec's, under the Style card's) and the host theme. */
    private style: any = {};
    private theme: { colors: string[]; background?: string } = { colors: [] };
    /** The text elements as drawn (the format pane's Data labels, X axis, Y axis and Legend cards). */
    private shown: { [part: string]: TextShown } = {};

    constructor(options: VisualConstructorOptions) {
        this.host = (options as any).host;
        this.locale = this.host && typeof this.host.locale === "string" ? this.host.locale : "";
        this.root = document.createElement("div");
        this.root.className = "bicraft-spec";
        options.element.appendChild(this.root);
        this.interactions = new Interactions(this.host, this.root);
    }

    public update(options: VisualUpdateOptions): void {
        const dataView: DataView | undefined = options.dataViews ? options.dataViews[0] : undefined;
        const objects: any = dataView && dataView.metadata ? dataView.metadata.objects : undefined;
        const text = objects && objects.spec && typeof objects.spec.json === "string" ? objects.spec.json : "";
        this.specText = text;
        const contrast = contrastOf(this.host);
        this.root.classList.toggle("bicraft-hc", contrast !== null);
        this.root.style.background = contrast ? contrast.background : "";
        this.root.style.setProperty("--bicraft-fg", contrast ? contrast.foreground : "");
        this.root.style.setProperty("--bicraft-focus", contrast ? contrast.selected : "");
        clear(this.root);
        const w = options.viewport ? options.viewport.width : 0;
        const h = options.viewport ? options.viewport.height : 0;
        if (boundRoles(dataView).length === 0) {
            const roles = langOf(this.locale) === "fr" ? ROLE_NAMES.fr : ROLE_NAMES.en;
            showLanding(this.root, "BIcraft Spec", landingText(this.locale, roles));
            return;
        }
        if (text.trim() === "") {
            showMessage(this.root, pick(this.locale, NO_SPEC));
            return;
        }
        const table = tableOf(dataView);
        const hits: Hit[] = [];
        let prims: Prim[];
        let motion = false;
        try {
            const spec = parseText(text);
            // the Style card's settings over the spec's own
            const own = objects && objects.style && typeof objects.style === "object" ? objects.style : {};
            spec.style = Object.assign({}, spec.style, own);
            // the text cards' settings over the spec's own `text`
            spec.text = mergeText(spec.text, textOf(objects));
            this.style = spec.style;
            const theme = hostTheme(this.host);
            this.theme = theme;
            const look = lookOf(spec.style, theme);
            motion = look.motion;
            const data = this.data(dataView);
            this.shown = this.textShown(spec, data, look);
            prims = layout(spec, data, w, h, hits, theme);
        } catch (e) {
            showMessage(this.root, pick(this.locale, SPEC_ERROR) + (e && (e as any).message ? (e as any).message : String(e)));
            return;
        }
        const svg = document.createElementNS(SVG, "svg");
        svg.setAttribute("width", String(w));
        svg.setAttribute("height", String(h));
        lookSvg(svg, freshData(options) && motionAllowed(motion, contrast !== null));
        const drawn: (Element | null)[] = [];
        for (const p of prims.slice(0, MAX_PRIMS)) {
            const q = contrast ? contrastPrim(p, contrast) : p;
            const el = decorate(svg, q, this.element(q));
            drawn.push(el);
            if (el) {
                svg.appendChild(el);
            }
        }
        const targets = this.targets(hits, drawn, svg, contrast);
        svg.setAttribute("aria-label", this.interactions.summary("BIcraft Spec", table.rows));
        this.root.appendChild(svg);
        this.interactions.wire({
            container: svg,
            targets,
            ids: categoryIds(this.host, table.category ? table.category.source : null, table.rows),
            tooltip: (row) => tooltipItems(table, row, this.locale),
            label: (row) => tooltipText(tooltipItems(table, row, this.locale)),
            contrast,
        });
    }

    public getFormattingModel(): powerbi.visuals.FormattingModel {
        // the reset button would erase the spec: the card has none
        const name = pick(this.locale, { en: "Spec", fr: "Spécification" });
        const json = pick(this.locale, { en: "Spec (JSON)", fr: "Spécification (JSON)" });
        const spec = card("spec", name, [group("spec_group", name, [textArea("spec", "json", json, this.specText)])], false);
        const cards = [spec].concat(textCards(this.locale, this.shown));
        return model(withLook(cards, this.locale, lookOf(this.style, this.theme), this.style.palette)) as any;
    }

    /** Each text element's settings as drawn, for the format pane. */
    private textShown(spec: Spec, data: SpecData, look: any): { [part: string]: TextShown } {
        const r = resolve(spec, data, spec.ownPalette ? spec.palette : look.palette).spec;
        const t = r.text || {};
        const shown = (part: string, size: number, color: string, show: boolean, bold: boolean): TextShown => {
            const s = textSet(r, part, size, color);
            const o = t[part] || {};
            return { show, size: s.size, family: s.family, color: s.color, bold: s.bold !== null ? s.bold : bold, italic: s.italic, underline: s.underline, units: typeof o.units === "number" ? o.units : 1, decimals: typeof o.decimals === "number" ? o.decimals : null };
        };
        return {
            labels: shown("labels", look.value, look.label, r.marks.some((m) => m.kind !== "text" && m.label !== null), valueWeight(look) >= 600),
            x: shown("x", look.tick, look.text, r.x.axis, false),
            xTitle: shown("xTitle", look.name, look.text, r.x.title !== null, false),
            y: shown("y", look.tick, look.text, r.y.axis, false),
            yTitle: shown("yTitle", look.name, look.text, r.y.title !== null, false),
            legend: shown("legend", look.name, look.text, r.legend !== false, false),
        };
    }

    /** The data points' elements: bars, points, rules and labels as drawn, invisible circles at the vertices of lines and areas (drawn on top). */
    private targets(hits: Hit[], drawn: (Element | null)[], svg: Element, contrast: Contrast | null): Target[] {
        const marks: Target[] = [];
        const labels: Target[] = [];
        for (const hit of hits) {
            if (hit.prim >= 0) {
                const el = hit.prim < drawn.length ? drawn[hit.prim] : null;
                if (el) {
                    (hit.label ? labels : marks).push({ row: hit.row, el, fade: !!hit.fade });
                }
            } else if (typeof hit.x === "number" && typeof hit.y === "number") {
                const c = document.createElementNS(SVG, "circle");
                c.setAttribute("cx", String(hit.x));
                c.setAttribute("cy", String(hit.y));
                c.setAttribute("r", String(HIT_RADIUS));
                c.setAttribute("fill", contrast ? contrast.background : "#FFFFFF");
                c.setAttribute("fill-opacity", "0");
                svg.appendChild(c);
                marks.push({ row: hit.row, el: c, hit: true, fade: !!hit.fade });
            }
        }
        // a row's first mark takes the keyboard focus; labels follow
        return marks.concat(labels);
    }

    /** The categorical data view as the layout reads it. */
    private data(dataView: DataView | undefined): SpecData {
        const categorical: any = dataView ? dataView.categorical : undefined;
        const category = categorical && categorical.categories ? categorical.categories[0] : undefined;
        const cats: Cat[] = category && category.values ? category.values.map((v: any) => catOf(v)) : [];
        const values: (number | null)[][] = [];
        // another visual's selection highlighting this one: each value column's `highlights`
        const highlights: (number | null)[][] = [];
        const num = (v: any) => (typeof v === "number" && isFinite(v) ? v : null);
        const columns: any[] = categorical && categorical.values ? categorical.values : [];
        for (let k = 0; k < columns.length && k < 16; k++) {
            const col = columns[k] && columns[k].values ? columns[k].values : [];
            values.push(col.map(num));
            const hl = columns[k] && Array.isArray(columns[k].highlights) ? columns[k].highlights : null;
            if (hl) {
                highlights.push(hl.map(num));
            }
        }
        const source = category && category.source ? category.source : {};
        const names = columns.slice(0, 16).map((c: any) => (c && c.source && typeof c.source.displayName === "string" ? c.source.displayName : ""));
        const catName = typeof source.displayName === "string" ? source.displayName : "";
        return { cats, values, highlights: highlights.length === values.length ? highlights : [], catFormat: formatOf(source.format), lang: langOf(this.locale), catName, names };
    }

    private element(p: Prim): Element | null {
        const el = (name: string, attrs: { [k: string]: any }) => {
            const e = document.createElementNS(SVG, name);
            for (const k of Object.keys(attrs)) {
                e.setAttribute(k, String(attrs[k]));
            }
            return e;
        };
        const pts = (l: number[][]) => l.map((q) => `${q[0]},${q[1]}`).join(" ");
        switch (p.t) {
            case "rect":
                return el("rect", { x: p.x, y: p.y, width: Math.max(p.w, 0), height: Math.max(p.h, 0), fill: p.fill, "fill-opacity": p.opacity });
            case "line":
                return el("line", { x1: p.x1, y1: p.y1, x2: p.x2, y2: p.y2, stroke: p.stroke, "stroke-width": p.width });
            case "path":
                return el("polyline", { points: pts(p.points), fill: "none", stroke: p.stroke, "stroke-width": p.width, "stroke-linejoin": "round", "stroke-linecap": "round" });
            case "poly": {
                const g = el("polygon", { points: pts(p.points), fill: p.fill, "fill-opacity": p.opacity });
                if (p.stroke) {
                    g.setAttribute("stroke", p.stroke);
                }
                return g;
            }
            case "circle":
                return el("circle", { cx: p.cx, cy: p.cy, r: Math.max(p.r, 0), fill: p.fill, "fill-opacity": p.opacity });
            case "text": {
                const t = el("text", { x: p.x, y: p.y, fill: p.fill, "font-size": p.size, "text-anchor": p.anchor, "dominant-baseline": "central" });
                if (p.rotate) {
                    t.setAttribute("transform", `rotate(${p.rotate} ${p.x} ${p.y})`);
                }
                // the text settings' font (docs/formats/spec-visual.md "Text")
                if (typeof p.family === "string" && p.family !== "") {
                    t.setAttribute("font-family", p.family);
                }
                if (p.italic) {
                    t.setAttribute("font-style", "italic");
                }
                if (p.underline) {
                    t.setAttribute("text-decoration", "underline");
                }
                t.textContent = p.text;
                return t;
            }
            default:
                return null;
        }
    }
}
