// BIcraft's visual runtime (docs/community-visuals.md "Runtime"): selection and cross-filtering
// through the host's selection manager (`host.createSelectionManager()`: `select`, `clear`,
// `getSelectionIds`, `registerOnSelectCallback`, `showContextMenu`) with one selection id per
// category value (`host.createSelectionIdBuilder().withCategory(column, i)`). The pure rules here
// are tested under Node; `Selection` talks to the host.
"use strict";

/** What a click (or Enter) on row `row` does, given the rows selected now. */
export function clickPlan(selected: number[], row: number, multi: boolean): "select" | "clear" {
    if (!multi && selected.length === 1 && selected[0] === row) {
        return "clear";
    }
    return "select";
}

function sameId(a: any, b: any): boolean {
    if (a === b) {
        return true;
    }
    try {
        return !!a && typeof a.equals === "function" && !!a.equals(b);
    } catch (e) {
        return false;
    }
}

function among(ids: any[], id: any): boolean {
    return ids.some((x) => sameId(id, x));
}

/**
 * The rows (sorted) whose ids are among `ids`. A row may stand for several category values (a
 * histogram's bin: an array of ids); it is selected when all of them are.
 */
export function rowsOf(ids: any[], rowIds: any[]): number[] {
    const out: number[] = [];
    if (!Array.isArray(ids) || ids.length === 0) {
        return out;
    }
    for (let r = 0; r < rowIds.length; r++) {
        const id = rowIds[r];
        const on = Array.isArray(id) ? id.length > 0 && id.every((x) => among(ids, x)) : !!id && among(ids, id);
        if (on) {
            out.push(r);
        }
    }
    return out;
}

/**
 * What a Ctrl+click passes to the host's `select(ids, true)`, which toggles each id: for a row of
 * several ids (a Sankey node, a Venn set) not all selected yet, only those not yet selected, so
 * ids it shares with a selected row stay selected; otherwise the row's id(s) as they are.
 */
export function multiIds(current: any[], id: any): any {
    if (!Array.isArray(id) || !Array.isArray(current) || id.every((x) => among(current, x))) {
        return id;
    }
    return id.filter((x) => !among(current, x));
}

/** Whether row `row` is drawn dimmed: something is selected and it is not. */
export function dimmed(selected: number[], row: number): boolean {
    return selected.length > 0 && selected.indexOf(row) < 0;
}

/** One selection id per row of the category column (none without a category or a host). */
export function categoryIds(host: any, category: any, rows: number): any[] {
    const out: any[] = [];
    if (!host || typeof host.createSelectionIdBuilder !== "function" || !category) {
        return out;
    }
    try {
        for (let i = 0; i < rows; i++) {
            out.push(host.createSelectionIdBuilder().withCategory(category, i).createSelectionId());
        }
    } catch (e) {
        return [];
    }
    return out;
}

/** The host's selection manager for one visual, with the rows it has selected. */
export class Selection {
    private readonly manager: any;
    private readonly allowed: boolean;
    private ids: any[] = [];
    public selected: number[] = [];
    /** Called after the selected rows change (by a click, a key, the host or a bookmark). */
    public onChange: (selected: number[]) => void = () => undefined;

    constructor(host: any) {
        let m: any = null;
        try {
            m = host && typeof host.createSelectionManager === "function" ? host.createSelectionManager() : null;
        } catch (e) {
            m = null;
        }
        this.manager = m;
        const caps = host ? host.hostCapabilities : undefined;
        this.allowed = !!m && !(caps && caps.allowInteractions === false);
        if (m && typeof m.registerOnSelectCallback === "function") {
            // the host restores a selection (a bookmark): show it
            m.registerOnSelectCallback((ids: any[]) => this.sync(ids));
        }
    }

    /** Whether points can be selected (a manager, interactions allowed, ids for the rows). */
    public enabled(): boolean {
        return this.allowed && this.ids.length > 0;
    }

    /** Sets the rows' ids after an update and keeps the host's current selection. */
    public setIds(ids: any[]): void {
        this.ids = ids;
        this.sync(this.current());
    }

    /** The host's selected ids now (none when it cannot tell). */
    private current(): any[] {
        try {
            const c = this.manager && typeof this.manager.getSelectionIds === "function" ? this.manager.getSelectionIds() : [];
            return Array.isArray(c) ? c : [];
        } catch (e) {
            return [];
        }
    }

    public id(row: number): any {
        const id = row >= 0 && row < this.ids.length ? this.ids[row] : undefined;
        return id === null ? undefined : id;
    }

    private sync(ids: any[]): void {
        this.selected = rowsOf(ids, this.ids);
        this.onChange(this.selected);
    }

    private settle(p: any): void {
        if (p && typeof p.then === "function") {
            p.then((ids: any) => this.sync(Array.isArray(ids) ? ids : []), () => undefined);
        }
    }

    /** Selects row `row` (with `multi`, adds or removes it); selecting the only selected row clears. */
    public click(row: number, multi: boolean): void {
        const id = this.id(row);
        if (!this.enabled() || id === undefined) {
            return;
        }
        if (clickPlan(this.selected, row, multi) === "clear") {
            this.clear();
            return;
        }
        try {
            this.settle(this.manager.select(multi ? multiIds(this.current(), id) : id, multi));
        } catch (e) {
            // the host refused the selection
        }
    }

    /** Clears the selection (a click on the background, Escape). */
    public clear(): void {
        if (!this.manager) {
            return;
        }
        try {
            const p = this.manager.clear();
            this.selected = [];
            this.onChange(this.selected);
            if (p && typeof p.then === "function") {
                p.then(() => undefined, () => undefined);
            }
        } catch (e) {
            // nothing selected to clear
        }
    }

    /** Opens the host's context menu at `x`, `y` for row `row` (or the visual, row −1). */
    public menu(row: number, x: number, y: number): boolean {
        if (!this.allowed || typeof this.manager.showContextMenu !== "function") {
            return false;
        }
        try {
            const id = this.id(row);
            // a row of several category values: the menu of the first
            const one = Array.isArray(id) ? id[0] : id;
            this.manager.showContextMenu(one === undefined ? {} : one, { x, y });
            return true;
        } catch (e) {
            return false;
        }
    }
}
