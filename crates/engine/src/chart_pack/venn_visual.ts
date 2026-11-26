// BIcraft Venn (docs/formats/chart-pack.md "Venn diagram"): the `IVisual` class. The roles item
// and set give the membership table; ./venn lays out the three largest sets. A circle selects
// its set (and cross-filters the page); a region's count shows which sets it holds.
"use strict";

import powerbi from "powerbi-visuals-api";
import VisualConstructorOptions = powerbi.extensibility.visual.VisualConstructorOptions;
import IVisual = powerbi.extensibility.visual.IVisual;
import { Point, PackVisual, RoleColumn, formatsOf, idOf, roleColumn, rowsOf, tipLine } from "./base";
import { Hit, Out, cellText } from "./kit";
import { color as colorOf } from "./runtime/style";
import { MAX_ROWS, layoutVenn } from "./venn";
import { objectValue } from "./runtime/data";
import { formatValue } from "./runtime/format";
import { card, color, group, model, number, toggle, withLook } from "./runtime/formatting";
import { pick } from "./runtime/strings";

const ITEMS = { en: "Items", fr: "Éléments" };
const REGION = { en: "Region", fr: "Zone" };
const NOT = { en: "not ", fr: "sans " };

export class Visual extends PackVisual implements IVisual {
    private cols: (RoleColumn | null)[] = [];
    /** The drawn sets' names, by drawn index. */
    private sets: string[] = [];
    /** The rows of each set name (a circle selects all of them). */
    private members: { [set: string]: number[] } = {};

    constructor(options: VisualConstructorOptions) {
        super(options, "bicraft-venn");
    }

    protected title(): string {
        return "BIcraft Venn";
    }

    protected required() {
        return [
            { role: "item", en: "Item", fr: "Élément" },
            { role: "set", en: "Set", fr: "Ensemble" },
        ];
    }

    protected input(dv: any): any {
        this.cols = [roleColumn(dv, "item"), roleColumn(dv, "set")];
        return {
            rows: rowsOf(this.cols),
            formats: formatsOf(this.cols),
            options: {
                // no colour set: the layout takes the look's
                color1: objectValue(dv, "sets", "color1", ""),
                color2: objectValue(dv, "sets", "color2", ""),
                color3: objectValue(dv, "sets", "color3", ""),
                proportional: objectValue(dv, "sets", "proportional", false),
                showCounts: objectValue(dv, "labels", "show", true),
                // unset: the look's text size, as report.render draws it
                fontSize: objectValue(dv, "labels", "fontSize", NaN),
            },
        };
    }

    protected layout(input: any, w: number, h: number): Out {
        const out = layoutVenn(input, w, h);
        this.sets = [];
        this.members = {};
        const rows: any[] = Array.isArray(input.rows) ? input.rows : [];
        for (let r = 0; r < rows.length && r < MAX_ROWS; r++) {
            const name = rows[r] ? cellText(rows[r][1]) : "";
            if (name !== "") {
                (this.members[name] = this.members[name] || []).push(r);
            }
        }
        for (const hit of out.hits) {
            if (typeof hit.set === "number") {
                const r = input.rows[hit.row];
                this.sets[hit.set] = r ? cellText(r[1]) : "";
            }
        }
        return out;
    }

    protected point(hit: Hit): Point | null {
        const items = { displayName: pick(this.locale, ITEMS), value: formatValue(hit.n, "#,0", this.locale) };
        if (typeof hit.set === "number") {
            // the host gives one identity per row (item and set), so a set is all its rows
            // (seen in Power BI Desktop 2.159.1231.0: the first row's id selected one item)
            const name = this.sets[hit.set] || "";
            const rows = this.members[name] || [hit.row];
            const ids = rows.map((r) => idOf(this.host, [this.cols[1]], r)).filter((id) => id !== undefined);
            const id = ids.length === 0 ? undefined : ids.length === 1 ? ids[0] : ids;
            return { id, tip: tipLine(this.cols[1], hit.row, this.locale).concat([items]) };
        }
        const inside: string[] = [];
        const outside: string[] = [];
        for (let j = 0; j < this.sets.length; j++) {
            (hit.region & (1 << j) ? inside : outside).push(this.sets[j]);
        }
        const name = inside.join(" ∩ ") + (outside.length > 0 ? ` (${pick(this.locale, NOT)}${outside.join(", ")})` : "");
        return { id: undefined, tip: [{ displayName: pick(this.locale, REGION), value: name }, items] };
    }

    public getFormattingModel(): powerbi.visuals.FormattingModel {
        const dv = this.dataView;
        const fr = this.locale.toLowerCase().startsWith("fr");
        const k = this.look();
        const sets = card("sets", fr ? "Ensembles" : "Sets", [
            group("sets_group", fr ? "Couleurs" : "Colors", [
                color("sets", "color1", fr ? "Premier ensemble" : "First set", objectValue(dv, "sets", "color1", colorOf(k, 0))),
                color("sets", "color2", fr ? "Deuxième ensemble" : "Second set", objectValue(dv, "sets", "color2", colorOf(k, 1))),
                color("sets", "color3", fr ? "Troisième ensemble" : "Third set", objectValue(dv, "sets", "color3", colorOf(k, 2))),
                toggle("sets", "proportional", fr ? "Surfaces selon les nombres (deux ensembles)" : "Areas as the counts (two sets)", objectValue(dv, "sets", "proportional", false)),
            ]),
        ]);
        const labels = card("labels", fr ? "Nombres" : "Counts", [
            group("labels_group", fr ? "Nombres" : "Counts", [
                toggle("labels", "show", fr ? "Afficher" : "Show", objectValue(dv, "labels", "show", true)),
                number("labels", "fontSize", fr ? "Taille du texte" : "Text size", objectValue(dv, "labels", "fontSize", 12 + k.value - 10), 8, 24),
            ]),
        ]);
        return model(withLook([sets, labels], this.locale, k, this.paletteOption())) as any;
    }
}
