// BIcraft's visual runtime (docs/community-visuals.md "Runtime"): high contrast. When the host's
// palette says `isHighContrast` (`host.colorPalette`, the API's `ISandboxExtendedColorPalette`),
// a visual draws with its foreground and background colours only, clear strokes instead of
// transparency, and `foregroundSelected` for what is selected.
"use strict";

/** The colours to draw with in high contrast. */
export interface Contrast {
    foreground: string;
    background: string;
    selected: string;
}

function colorOf(info: any, fallback: string): string {
    return info && typeof info.value === "string" && info.value !== "" ? info.value : fallback;
}

/** The high-contrast colours, or null when high contrast is off (or the host has no palette). */
export function contrastOf(host: any): Contrast | null {
    const p = host ? host.colorPalette : undefined;
    if (!p || p.isHighContrast !== true) {
        return null;
    }
    const foreground = colorOf(p.foreground, "#FFFFFF");
    return {
        foreground,
        background: colorOf(p.background, "#000000"),
        selected: colorOf(p.foregroundSelected, foreground),
    };
}

/**
 * A drawn primitive (`{t: "rect" | "line" | "path" | "poly" | "circle" | "text", fill?, stroke?,
 * opacity?}`) with the high-contrast colours: lines, strokes and text take the foreground;
 * bars and points are filled with it; areas become an outline over the background.
 */
export function contrastPrim(p: { [k: string]: any }, c: Contrast): { [k: string]: any } {
    const q: { [k: string]: any } = {};
    for (const k of Object.keys(p)) {
        q[k] = p[k];
    }
    if (p.role === "card" || p.role === "pill") {
        // BIcraft's look: the card behind the plot and a value label's chip become an outline
        // over the background, the label on it in the foreground
        q.fill = c.background;
        q.opacity = 1;
        q.stroke = c.foreground;
        q.sw = 1;
        delete q.grad;
        delete q.shadow;
        return q;
    }
    switch (p.t) {
        case "rect":
        case "circle":
            q.fill = c.foreground;
            q.opacity = 1;
            // BIcraft's look: no gradient or shadow; an edge in the background colour parts marks
            delete q.grad;
            delete q.shadow;
            if (typeof q.stroke === "string" && q.stroke !== "") {
                q.stroke = c.background;
            }
            break;
        case "poly":
            q.fill = c.background;
            q.opacity = 1;
            q.stroke = c.foreground;
            break;
        case "line":
        case "path":
            q.stroke = c.foreground;
            break;
        case "text":
            q.fill = c.foreground;
            break;
        default:
            break;
    }
    return q;
}
