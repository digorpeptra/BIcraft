// BIcraft's visual runtime (docs/community-visuals.md "Runtime"): one object per visual that
// wires its drawn data points to the host. Hover shows the point's tooltip; a click selects its
// category value (Ctrl or ⌘ adds or removes it) and the host cross-filters the page's other
// visuals; a click on the background clears; the context menu is the host's; Tab reaches each
// point and Enter or Space selects it, arrows move between points, Escape clears; points that are
// not selected are dimmed, and so are points the layout faded because another visual's selection
// highlights none of their rows (`fade`, docs/formats/bicraft-visuals.md "Highlights"). The
// listeners sit on the visual's root once, so redraws keep them.
"use strict";

import { Contrast } from "./contrast";
import { KeyAction, keyAction, moveFocus } from "./keys";
import { Selection, dimmed } from "./selection";
import { t } from "./strings";
import { TooltipItem, Tooltips } from "./tooltips";

/** A drawn element of a data point (`row`: its category value's index). */
export interface Target {
    row: number;
    el: Element;
    /** An invisible hit area (a line's vertex): drawn as a ring when selected. */
    hit?: boolean;
    /** Drawn dimmed: another visual's selection highlights none of its rows. */
    fade?: boolean;
}

/** What a visual hands over after drawing. */
export interface Wiring {
    /** The element that holds the points (an `svg`). */
    container: Element;
    targets: Target[];
    /** One selection id per row (empty: no selection). */
    ids: any[];
    tooltip: (row: number) => TooltipItem[];
    /** The accessible name of a point. */
    label: (row: number) => string;
    /** High-contrast colours, or null. */
    contrast: Contrast | null;
}

/** Opacity of points that are not selected while others are. */
export const DIM_OPACITY = 0.4;
/** The ring of a selected point and the focus ring, outside high contrast. */
const RING = "#252423";

function rowOf(target: any): number {
    const el = target && typeof target.closest === "function" ? target.closest("[data-row]") : null;
    if (!el) {
        return -1;
    }
    const n = Number(el.getAttribute("data-row"));
    return Number.isInteger(n) && n >= 0 ? n : -1;
}

export class Interactions {
    public readonly selection: Selection;
    private readonly tooltips: Tooltips;
    private readonly locale: string;
    private wiring: Wiring | null = null;
    /** The focusable element of each row, in row order. */
    private focusable: Element[] = [];
    private focusRows: number[] = [];
    /** The point that last had the keyboard focus, and its row. */
    private focused: Element | null = null;
    /** The row under the pointer (-1: none): its marks stay, the others are dimmed. */
    private hot = -1;
    private focusedRow = -1;
    private readonly root: HTMLElement;

    constructor(host: any, root: HTMLElement) {
        this.root = root;
        this.selection = new Selection(host);
        this.tooltips = new Tooltips(host);
        this.locale = host && typeof host.locale === "string" ? host.locale : "";
        this.selection.onChange = (rows) => this.restyle(rows);
        root.addEventListener("mousemove", (e: MouseEvent) => this.hover(e));
        root.addEventListener("mouseleave", () => {
            this.tooltips.hide(false);
            this.emphasize(-1);
        });
        root.addEventListener("click", (e: MouseEvent) => this.click(e));
        root.addEventListener("contextmenu", (e: MouseEvent) => this.contextMenu(e));
        root.addEventListener("keydown", (e: KeyboardEvent) => this.key(e));
        root.addEventListener("focusin", (e: Event) => {
            const row = rowOf(e.target);
            this.focused = row >= 0 ? (e.target as Element) : null;
            this.focusedRow = row;
        });
    }

    /**
     * After a redraw: when the focused point was replaced (the host redraws the visual after a
     * selection, seen in Power BI Desktop 2.159.1231.0) and the focus fell to the document, the
     * new element of the same row takes it back, so arrows and Escape keep working.
     */
    private refocus(): void {
        const prev: any = this.focused;
        if (!prev || prev.isConnected !== false) {
            return;
        }
        try {
            const doc: any = (this.root as any).ownerDocument;
            const active = doc ? doc.activeElement : null;
            const lost = !active || active === doc.body;
            const has = doc && typeof doc.hasFocus === "function" ? doc.hasFocus() : false;
            const at = this.focusRows.indexOf(this.focusedRow);
            const el: any = at >= 0 ? this.focusable[at] : null;
            if (lost && has && el && typeof el.focus === "function") {
                el.focus();
                this.focused = el;
            }
        } catch (e) {
            // no document to focus in
        }
    }

    /** Takes the points just drawn; the host's current selection is shown on them. */
    public wire(w: Wiring): void {
        this.wiring = w;
        this.hot = -1;
        this.focusable = [];
        this.focusRows = [];
        const seen: { [row: number]: boolean } = {};
        const selectable = w.ids.length > 0;
        w.container.setAttribute("role", selectable ? "listbox" : "list");
        if (selectable) {
            w.container.setAttribute("aria-multiselectable", "true");
        }
        for (const tg of w.targets) {
            tg.el.setAttribute("data-row", String(tg.row));
            const fo = tg.el.getAttribute("fill-opacity");
            if (fo !== null) {
                tg.el.setAttribute("data-fo", fo);
            }
            if (!seen[tg.row]) {
                seen[tg.row] = true;
                tg.el.setAttribute("tabindex", "0");
                tg.el.setAttribute("focusable", "true");
                tg.el.setAttribute("role", selectable ? "option" : "listitem");
                tg.el.setAttribute("aria-label", w.label(tg.row));
                tg.el.setAttribute("class", ((tg.el.getAttribute("class") || "") + " bicraft-point").trim());
                this.focusable.push(tg.el);
                this.focusRows.push(tg.row);
            }
        }
        this.selection.setIds(w.ids);
        this.refocus();
    }

    private restyle(rows: number[]): void {
        const w = this.wiring;
        if (!w) {
            return;
        }
        const c = w.contrast;
        const any = rows.length > 0;
        for (const tg of w.targets) {
            const el = tg.el;
            const on = any && !dimmed(rows, tg.row);
            const dim = !on && (any || !!tg.fade);
            if (tg.hit) {
                if (on) {
                    el.setAttribute("stroke", c ? c.selected : RING);
                    el.setAttribute("stroke-width", "2");
                } else {
                    el.removeAttribute("stroke");
                    el.removeAttribute("stroke-width");
                }
            } else if (c) {
                // high contrast: no transparency; dimmed points become outlines
                const fo = el.getAttribute("data-fo");
                if (dim) {
                    el.setAttribute("fill-opacity", "0");
                    el.setAttribute("stroke", c.foreground);
                    el.setAttribute("stroke-width", "1");
                } else {
                    if (fo !== null) {
                        el.setAttribute("fill-opacity", fo);
                    }
                    if (on) {
                        el.setAttribute("stroke", c.selected);
                        el.setAttribute("stroke-width", "2");
                    } else {
                        el.removeAttribute("stroke");
                        el.removeAttribute("stroke-width");
                    }
                }
            } else if (dim) {
                el.setAttribute("opacity", String(DIM_OPACITY));
            } else {
                el.removeAttribute("opacity");
            }
            if (el.getAttribute("tabindex") !== null && w.ids.length > 0) {
                el.setAttribute("aria-selected", on ? "true" : "false");
            }
        }
    }

    private hover(e: MouseEvent): void {
        const w = this.wiring;
        const row = rowOf(e.target);
        this.emphasize(row);
        if (!w || row < 0) {
            this.tooltips.hide(false);
            return;
        }
        this.tooltips.show(w.tooltip(row), this.selection.id(row), e.clientX, e.clientY, false);
    }

    /**
     * The hover emphasis of BIcraft's look (docs/formats/visual-look.md): the marks of `row`
     * marked hot and the container marked hovering, so the stylesheet dims the other points; none
     * in high contrast, where the selection alone outlines points.
     */
    private emphasize(row: number): void {
        const w = this.wiring;
        const r = w && !w.contrast ? row : -1;
        if (!w || r === this.hot) {
            return;
        }
        this.hot = r;
        const cls = (el: Element, name: string, on: boolean) => {
            const list = (el.getAttribute("class") || "").split(" ").filter((c) => c !== "" && c !== name);
            if (on) {
                list.push(name);
            }
            el.setAttribute("class", list.join(" "));
        };
        cls(w.container, "bicraft-hovering", r >= 0);
        for (const tg of w.targets) {
            cls(tg.el, "bicraft-hot", r >= 0 && tg.row === r);
        }
    }

    private click(e: MouseEvent): void {
        const row = rowOf(e.target);
        if (row < 0) {
            this.selection.clear();
            return;
        }
        this.selection.click(row, e.ctrlKey || e.metaKey);
        e.stopPropagation();
    }

    private contextMenu(e: MouseEvent): void {
        if (this.selection.menu(rowOf(e.target), e.clientX, e.clientY)) {
            e.preventDefault();
        }
    }

    private key(e: KeyboardEvent): void {
        const row = rowOf(e.target);
        const action: KeyAction = keyAction(e.key, e.ctrlKey || e.metaKey, e.shiftKey);
        if (action === null || row < 0) {
            return;
        }
        switch (action) {
            case "select":
                this.selection.click(row, false);
                break;
            case "multiSelect":
                this.selection.click(row, true);
                break;
            case "clear":
                this.selection.clear();
                break;
            case "menu": {
                const r = (e.target as Element).getBoundingClientRect();
                this.selection.menu(row, r.left + r.width / 2, r.top + r.height / 2);
                break;
            }
            default: {
                const at = this.focusRows.indexOf(row);
                const next = moveFocus(at, action, this.focusable.length);
                const el: any = next >= 0 ? this.focusable[next] : null;
                if (el && typeof el.focus === "function") {
                    el.focus();
                }
            }
        }
        e.preventDefault();
    }

    /** The accessible name of the points' container ("Sales: 12 data points"). */
    public summary(name: string, rows: number): string {
        return t(this.locale, "points", name, rows);
    }
}
