// BIcraft's visual runtime (docs/community-visuals.md "Runtime"): the tooltip of a data point,
// shown through the host's tooltip service (`host.tooltipService`: `enabled`, `show`, `move`,
// `hide`, with the `VisualTooltipDataItem` fields of the public `powerbi-visuals-api` types).
"use strict";

import { Table } from "./data";
import { formatValue } from "./format";

/** A tooltip line (the API's `VisualTooltipDataItem`). */
export interface TooltipItem {
    displayName: string;
    value: string;
    color?: string;
}

/** The lines of row `row`: the category, then each value column, written with its format. */
export function tooltipItems(table: Table, row: number, locale?: string | null): TooltipItem[] {
    const out: TooltipItem[] = [];
    if (row < 0 || row >= table.rows) {
        return out;
    }
    if (table.category) {
        out.push({ displayName: table.category.name, value: formatValue(table.category.values[row], table.category.format, locale) });
    }
    for (const c of table.values) {
        out.push({ displayName: c.name, value: formatValue(c.values[row], c.format, locale) });
    }
    return out;
}

/** The lines as one text (an accessible name for the point). */
export function tooltipText(items: TooltipItem[]): string {
    return items.map((i) => (i.displayName ? `${i.displayName}: ${i.value}` : i.value)).join(", ");
}

/** Shows, moves and hides the host's tooltip; does nothing when the host has no tooltip service. */
export class Tooltips {
    private readonly service: any;
    private shown = false;

    constructor(host: any) {
        this.service = host && host.tooltipService ? host.tooltipService : null;
    }

    private enabled(): boolean {
        try {
            return !!this.service && (typeof this.service.enabled !== "function" || !!this.service.enabled());
        } catch (e) {
            return false;
        }
    }

    /** Shows `items` at the pointer (`x`, `y`: the event's client coordinates in the visual's frame). */
    public show(items: TooltipItem[], identity: any, x: number, y: number, touch: boolean): void {
        if (!this.enabled() || items.length === 0) {
            return;
        }
        const options = { coordinates: [x, y], isTouchEvent: touch, dataItems: items, identities: Array.isArray(identity) ? identity : identity ? [identity] : [] };
        try {
            if (this.shown) {
                this.service.move(options);
            } else {
                this.service.show(options);
                this.shown = true;
            }
        } catch (e) {
            this.shown = false;
        }
    }

    public hide(touch: boolean): void {
        if (!this.shown || !this.service) {
            return;
        }
        this.shown = false;
        try {
            this.service.hide({ isTouchEvent: touch, immediately: false });
        } catch (e) {
            // the host refused: nothing else to do
        }
    }
}
