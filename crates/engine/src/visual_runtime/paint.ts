// BIcraft's look in SVG (docs/formats/visual-look.md): what a layout's primitives add to their
// plain drawing (rounded corners with one side square, a vertical gradient, an edge, a soft
// shadow, a text weight) and the entry animation. BIcraft's renderer draws the same decorations
// (crates/render/src/spec.rs `decorated`) as the animation's final frame.
"use strict";

const SVG = "http://www.w3.org/2000/svg";
/** A prefix unique to this copy of the runtime, so two visuals' gradient ids never meet. */
const PREFIX = "bc" + Math.floor(Math.random() * 1e9).toString(36);
let counter = 0;

type Prim = { [key: string]: any };

interface State {
    defs: Element | null;
    ids: { [key: string]: string };
}

const STATES = new WeakMap<Element, State>();

function stateOf(svg: Element): State {
    let s = STATES.get(svg);
    if (!s) {
        s = { defs: null, ids: {} };
        STATES.set(svg, s);
    }
    return s;
}

function defsOf(svg: Element, s: State): Element {
    if (!s.defs) {
        s.defs = document.createElementNS(SVG, "defs");
        const first = (svg as any).firstChild;
        if (first && typeof svg.insertBefore === "function") {
            svg.insertBefore(s.defs, first);
        } else {
            svg.appendChild(s.defs);
        }
    }
    return s.defs;
}

function make(name: string, attrs: { [k: string]: any }): Element {
    const e = document.createElementNS(SVG, name);
    for (const k of Object.keys(attrs)) {
        e.setAttribute(k, String(attrs[k]));
    }
    return e;
}

/** The id of a vertical gradient from `top` to `bottom` in the svg's defs. */
function gradient(svg: Element, top: string, bottom: string): string {
    const s = stateOf(svg);
    const key = "g" + top + bottom;
    if (!s.ids[key]) {
        const id = `${PREFIX}-${counter++}`;
        const g = make("linearGradient", { id, x1: 0, y1: 0, x2: 0, y2: 1 });
        g.appendChild(make("stop", { offset: 0, "stop-color": top }));
        g.appendChild(make("stop", { offset: 1, "stop-color": bottom }));
        defsOf(svg, s).appendChild(g);
        s.ids[key] = id;
    }
    return s.ids[key];
}

/** The id of a soft shadow filter of size `size`. */
function shadow(svg: Element, size: number): string {
    const s = stateOf(svg);
    const key = "s" + size;
    if (!s.ids[key]) {
        const id = `${PREFIX}-${counter++}`;
        const f = make("filter", { id, x: "-30%", y: "-30%", width: "160%", height: "180%" });
        f.appendChild(make("feDropShadow", { dx: 0, dy: size / 3, stdDeviation: size / 2, "flood-color": "#101828", "flood-opacity": 0.14 }));
        defsOf(svg, s).appendChild(f);
        s.ids[key] = id;
    }
    return s.ids[key];
}

/** A rectangle's outline with its corners rounded by `r` except the two on side `sq`. */
export function cornerPath(x: number, y: number, w: number, h: number, r: number, sq: string): string {
    const k = Math.max(Math.min(r, w / 2, h / 2), 0);
    const tl = sq === "t" || sq === "l" ? 0 : k;
    const tr = sq === "t" || sq === "r" ? 0 : k;
    const br = sq === "b" || sq === "r" ? 0 : k;
    const bl = sq === "b" || sq === "l" ? 0 : k;
    return [
        `M${x + tl},${y}`,
        `H${x + w - tr}`,
        tr ? `A${tr},${tr} 0 0 1 ${x + w},${y + tr}` : "",
        `V${y + h - br}`,
        br ? `A${br},${br} 0 0 1 ${x + w - br},${y + h}` : "",
        `H${x + bl}`,
        bl ? `A${bl},${bl} 0 0 1 ${x},${y + h - bl}` : "",
        `V${y + tl}`,
        tl ? `A${tl},${tl} 0 0 1 ${x + tl},${y}` : "",
        "Z",
    ].join("");
}

/**
 * The element of `p` drawn with its decorations: `el` (the plain element the visual made) with
 * them added, or a path in its place for a rectangle with a square side. `svg` takes the
 * gradients and shadows. Without decorations `el` is returned as it is.
 */
export function decorate(svg: Element, p: Prim, el: Element | null): Element | null {
    if (!el) {
        return el;
    }
    let out = el;
    if (p.t === "rect" && p.rx > 0) {
        if (p.sq) {
            out = make("path", { d: cornerPath(p.x, p.y, Math.max(p.w, 0), Math.max(p.h, 0), p.rx, p.sq) });
            for (const a of ["fill", "fill-opacity", "stroke", "stroke-width"]) {
                const v = el.getAttribute(a);
                if (v !== null) {
                    out.setAttribute(a, v);
                }
            }
        } else {
            out.setAttribute("rx", String(p.rx));
            out.setAttribute("ry", String(p.rx));
        }
    }
    if ((p.t === "rect" || p.t === "circle") && typeof p.grad === "string" && p.grad !== "") {
        out.setAttribute("fill", `url(#${gradient(svg, p.grad, p.fill)})`);
    }
    if ((p.t === "rect" || p.t === "circle") && typeof p.stroke === "string" && p.stroke !== "") {
        out.setAttribute("stroke", p.stroke);
        out.setAttribute("stroke-width", String(p.sw > 0 ? p.sw : 1));
    }
    if ((p.t === "rect" || p.t === "circle") && p.shadow > 0) {
        out.setAttribute("filter", `url(#${shadow(svg, p.shadow)})`);
    }
    if (p.t === "text" && p.weight && p.weight !== 400) {
        out.setAttribute("font-weight", String(p.weight));
    }
    if (p.t === "rect" || p.t === "circle" || p.t === "poly") {
        // the entry animation: marks grow from their baseline (a dot from its centre)
        const side = p.sq || "";
        const origin = side === "b" ? "50% 100%" : side === "t" ? "50% 0%" : side === "l" ? "0% 50%" : side === "r" ? "100% 50%" : "50% 50%";
        const from = side === "b" || side === "t" ? "scale(1, 0.001)" : side === "l" || side === "r" ? "scale(0.001, 1)" : "scale(0.6)";
        out.setAttribute("class", ((out.getAttribute("class") || "") + " bicraft-grow").trim());
        out.setAttribute("style", `--bc-o: ${origin}; --bc-s: ${from}`);
    } else if (p.t === "line" || p.t === "path" || p.t === "text") {
        out.setAttribute("class", ((out.getAttribute("class") || "") + " bicraft-fade").trim());
    }
    return out;
}

/** Whether the entry animation may play: the look asks for it, no high contrast, no reduced motion. */
export function motionAllowed(motion: boolean, highContrast: boolean): boolean {
    if (!motion || highContrast) {
        return false;
    }
    try {
        const w: any = typeof window !== "undefined" ? window : null;
        return !(w && typeof w.matchMedia === "function" && w.matchMedia("(prefers-reduced-motion: reduce)").matches);
    } catch (e) {
        return true;
    }
}

/**
 * The report theme's data colours as the host gives them (`host.colorPalette`, keys "0" to "7"
 * in order) and its background (`colorPalette.background`, outside high contrast), as the
 * layouts' `theme` (`{colors, background?}`); empty without a palette.
 */
export function hostTheme(host: any): { colors: string[]; background?: string } {
    const colors: string[] = [];
    const out: { colors: string[]; background?: string } = { colors };
    try {
        const p = host ? host.colorPalette : null;
        if (p && typeof p.getColor === "function") {
            for (let i = 0; i < 8; i++) {
                const c = p.getColor(String(i));
                const v = c && typeof c.value === "string" ? c.value : "";
                if (/^#[0-9A-Fa-f]{6}$/.test(v)) {
                    colors.push(v.toUpperCase());
                }
            }
        }
        const bg = p && p.isHighContrast !== true && p.background ? p.background.value : undefined;
        if (typeof bg === "string" && /^#[0-9A-Fa-f]{6}$/.test(bg)) {
            out.background = bg.toUpperCase();
        }
    } catch (e) {
        // no palette: BIcraft's own colours
    }
    return out;
}

/** Marks `svg` as drawn with BIcraft's look, playing the entry animation when `animate`. */
export function lookSvg(svg: Element, animate: boolean): void {
    svg.setAttribute("class", animate ? "bicraft-look bicraft-anim" : "bicraft-look");
}

/** Whether an update brings new data (the first update, or one of type Data), so the marks enter again. */
export function freshData(options: any): boolean {
    const t = options ? options.type : undefined;
    return typeof t !== "number" || (t & 2) !== 0;
}
