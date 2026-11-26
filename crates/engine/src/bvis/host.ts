// BIcraft's own chart visuals (docs/formats/bicraft-visuals.md): the `IVisual` they share. Each
// visual hands over its definition (roles, layout, tooltips, format pane); this class reads the
// data view, draws the layout's primitives as SVG and wires them to BIcraft's visual runtime
// (./runtime: tooltips, selection and cross-filtering, context menu, keyboard focus, high
// contrast, landing page). Only the types of powerbi-visuals-api are used: the host page provides
// the API at run time.
"use strict";

import powerbi from "powerbi-visuals-api";
import VisualConstructorOptions = powerbi.extensibility.visual.VisualConstructorOptions;
import VisualUpdateOptions = powerbi.extensibility.visual.VisualUpdateOptions;
import IVisual = powerbi.extensibility.visual.IVisual;
import { Prim } from "./spec";
import { Data, Out, Unit, dataOf, labelOf, plainObjects } from "./common";
import { Contrast, contrastOf, contrastPrim } from "./runtime/contrast";
import { boundRoles } from "./runtime/data";
import { formatValue } from "./runtime/format";
import { Card, model, withLook } from "./runtime/formatting";
import { Interactions, Target } from "./runtime/interactions";
import { clear, landingText, showLanding, showMessage } from "./runtime/landing";
import { decorate, freshData, hostTheme, lookSvg, motionAllowed } from "./runtime/paint";
import { lookOf } from "./runtime/style";
import { joinNames, langOf, pick } from "./runtime/strings";
import { TooltipItem, tooltipText } from "./runtime/tooltips";

const SVG = "http://www.w3.org/2000/svg";
/** Most primitives drawn. */
const MAX_PRIMS = 20000;
/** Most selection ids of one unit (a histogram bin). */
const MAX_IDS = 5000;

export interface Role {
    name: string;
    en: string;
    fr: string;
}

export interface Def {
    /** The name shown on the landing page. */
    title: string;
    /** The roles, in the order the landing page lists them. */
    roles: Role[];
    /** The roles without which nothing is drawn. */
    required: string[];
    /** The role whose column gives the selection ids. */
    idRole: string;
    layout(d: Data, o: any, w: number, h: number, out: Out): Prim[];
    tooltip(d: Data, u: Unit, locale: string): TooltipItem[];
    /** The rows whose selection ids select a unit (all of them, or one row of a category value). */
    idRows(u: Unit): number[];
    cards(o: any, locale: string): Card[];
}

/** A one-row unit's tooltip: each field of the row, written with its format string. */
export function rowTooltip(d: Data, u: Unit, locale: string): TooltipItem[] {
    const out: TooltipItem[] = [];
    const row = u.rows.length > 0 ? u.rows[0] : -1;
    for (const c of d.cols) {
        if (row >= 0 && row < c.cells.length) {
            const v = c.cells[row];
            out.push({ displayName: c.name, value: typeof v === "string" && v !== "" ? labelOf(c, row) : formatValue(v, c.format, locale) });
        }
    }
    return out;
}

const NEEDS = { en: "Add a field to {0} to draw this visual.", fr: "Ajoutez un champ dans {0} pour dessiner ce visuel." };
const NOTHING = { en: "No values to draw.", fr: "Aucune valeur à dessiner." };

function element(p: Prim, outline: boolean, c: Contrast | null): Element | null {
    const el = (name: string, attrs: { [k: string]: any }) => {
        const e = document.createElementNS(SVG, name);
        for (const k of Object.keys(attrs)) {
            e.setAttribute(k, String(attrs[k]));
        }
        return e;
    };
    const pts = (l: number[][]) => l.map((q) => `${q[0]},${q[1]}`).join(" ");
    switch (p.t) {
        case "rect": {
            const r = el("rect", { x: p.x, y: p.y, width: Math.max(p.w, 0), height: Math.max(p.h, 0), fill: p.fill, "fill-opacity": p.opacity });
            if (outline && c) {
                // high contrast: a band or a box's fill as an outline over the background
                r.setAttribute("fill", c.background);
                r.setAttribute("fill-opacity", "1");
                r.setAttribute("stroke", c.foreground);
                r.setAttribute("stroke-width", "1");
            }
            return r;
        }
        case "line":
            return el("line", { x1: p.x1, y1: p.y1, x2: p.x2, y2: p.y2, stroke: p.stroke, "stroke-width": p.width });
        case "path":
            return el("polyline", { points: pts(p.points), fill: "none", stroke: p.stroke, "stroke-width": p.width, "stroke-linejoin": "round", "stroke-linecap": "round" });
        case "circle":
            return el("circle", { cx: p.cx, cy: p.cy, r: Math.max(p.r, 0), fill: p.fill, "fill-opacity": p.opacity });
        case "text": {
            const t = el("text", { x: p.x, y: p.y, fill: p.fill, "font-size": p.size, "text-anchor": p.anchor, "dominant-baseline": "central", "font-weight": p.weight || 400 });
            if (p.rotate) {
                t.setAttribute("transform", `rotate(${p.rotate} ${p.x} ${p.y})`);
            }
            t.textContent = p.text;
            return t;
        }
        default:
            return null;
    }
}

export class BVisual implements IVisual {
    private readonly root: HTMLElement;
    private readonly host: any;
    private readonly locale: string;
    private readonly interactions: Interactions;
    private readonly def: Def;
    private options: any = {};
    private theme: { colors: string[] } = { colors: [] };

    constructor(options: VisualConstructorOptions, def: Def) {
        this.def = def;
        this.host = (options as any).host;
        this.locale = this.host && typeof this.host.locale === "string" ? this.host.locale : "";
        this.root = document.createElement("div");
        this.root.className = "bicraft-bvis";
        options.element.appendChild(this.root);
        this.interactions = new Interactions(this.host, this.root);
    }

    public update(options: VisualUpdateOptions): void {
        const dataView: any = options.dataViews ? options.dataViews[0] : undefined;
        this.options = plainObjects(dataView && dataView.metadata ? dataView.metadata.objects : undefined);
        // the report theme's colours, which BIcraft's look follows by default
        const theme = hostTheme(this.host);
        this.theme = theme;
        const contrast = contrastOf(this.host);
        this.root.classList.toggle("bicraft-hc", contrast !== null);
        this.root.style.background = contrast ? contrast.background : "";
        this.root.style.setProperty("--bicraft-fg", contrast ? contrast.foreground : "");
        this.root.style.setProperty("--bicraft-focus", contrast ? contrast.selected : "");
        clear(this.root);
        const fr = langOf(this.locale) === "fr";
        const roleName = (n: string) => {
            const r = this.def.roles.filter((x) => x.name === n)[0];
            return r ? (fr ? r.fr : r.en) : n;
        };
        const bound = boundRoles(dataView);
        if (bound.length === 0) {
            showLanding(this.root, this.def.title, landingText(this.locale, this.def.required.map(roleName)));
            return;
        }
        const missing = this.def.required.filter((r) => bound.indexOf(r) < 0);
        if (missing.length > 0) {
            showMessage(this.root, pick(this.locale, NEEDS).replace("{0}", joinNames(this.locale, missing.map(roleName))));
            return;
        }
        const w = options.viewport ? options.viewport.width : 0;
        const h = options.viewport ? options.viewport.height : 0;
        const data = dataOf(dataView, this.locale);
        const out: Out = { units: [], hits: [] };
        const outline: { [p: number]: boolean } = {};
        const prims = this.def.layout(data, Object.assign({}, this.options, { $theme: theme }), w, h, out);
        if (prims.length === 0) {
            showMessage(this.root, pick(this.locale, NOTHING));
            return;
        }
        for (const p of out.outline || []) {
            outline[p] = true;
        }
        const svg = document.createElementNS(SVG, "svg");
        svg.setAttribute("width", String(w));
        svg.setAttribute("height", String(h));
        lookSvg(svg, freshData(options) && motionAllowed(lookOf(this.options.style, theme).motion, contrast !== null));
        const drawn: (Element | null)[] = [];
        for (let i = 0; i < prims.length && i < MAX_PRIMS; i++) {
            const p = prims[i];
            const q = contrast && !outline[i] ? contrastPrim(p, contrast) : p;
            const el = outline[i] ? element(q, true, contrast) : decorate(svg, q, element(q, false, contrast));
            drawn.push(el);
            if (el) {
                svg.appendChild(el);
            }
        }
        const targets: Target[] = [];
        // the units another visual's selection highlights none of (docs/formats/bicraft-visuals.md "Highlights")
        const faded = out.faded || [];
        for (const hit of out.hits) {
            const el = hit.prim < drawn.length ? drawn[hit.prim] : null;
            if (el) {
                targets.push({ row: hit.unit, el, fade: faded.indexOf(hit.unit) >= 0 });
            }
        }
        svg.setAttribute("aria-label", this.interactions.summary(this.def.title, out.units.length));
        this.root.appendChild(svg);
        this.interactions.wire({
            container: svg,
            targets,
            ids: this.ids(data, out.units),
            tooltip: (u) => (u >= 0 && u < out.units.length ? this.def.tooltip(data, out.units[u], this.locale) : []),
            label: (u) => (u >= 0 && u < out.units.length ? tooltipText(this.def.tooltip(data, out.units[u], this.locale)) : ""),
            contrast,
        });
    }

    public getFormattingModel(): powerbi.visuals.FormattingModel {
        const o = Object.assign({}, this.options, { $theme: this.theme });
        const style = o.style && typeof o.style === "object" ? o.style : {};
        return model(withLook(this.def.cards(o, this.locale), this.locale, lookOf(style, this.theme), style.palette)) as any;
    }

    /** One selection id per unit (several for a unit of many category values), none without a host or the id column. */
    private ids(data: Data, units: Unit[]): any[] {
        const host = this.host;
        const c = data.cols.filter((x) => x.role === this.def.idRole)[0];
        if (!host || typeof host.createSelectionIdBuilder !== "function" || !c || !c.source) {
            return [];
        }
        const out: any[] = [];
        try {
            for (const u of units) {
                const list = this.def.idRows(u).slice(0, MAX_IDS).map((r) => host.createSelectionIdBuilder().withCategory(c.source, r).createSelectionId());
                out.push(list.length === 1 ? list[0] : list.length === 0 ? null : list);
            }
        } catch (e) {
            return [];
        }
        return out;
    }
}
