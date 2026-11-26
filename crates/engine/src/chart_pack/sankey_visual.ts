// BIcraft Sankey (docs/formats/chart-pack.md "Sankey"): the `IVisual` class. The roles source,
// destination and weight; ./sankey lays out the nodes in columns and the links as ribbons. A
// link selects its source and destination rows; a node selects every row it is in.
"use strict";

import powerbi from "powerbi-visuals-api";
import VisualConstructorOptions = powerbi.extensibility.visual.VisualConstructorOptions;
import IVisual = powerbi.extensibility.visual.IVisual;
import { Point, PackVisual, RoleColumn, formatsOf, idOf, roleColumn, rowsOf } from "./base";
import { Hit, Out, cellText } from "./kit";
import { FIXED_LINK, MAX_ROWS, NODE_COLORS, layoutSankey } from "./sankey";
import { objectValue } from "./runtime/data";
import { formatValue } from "./runtime/format";
import { Slice, card, color, group, model, number, toggle, withLook } from "./runtime/formatting";
import { color as colorOf } from "./runtime/style";
import { pick } from "./runtime/strings";
import { TooltipItem } from "./runtime/tooltips";

const SOURCE = { en: "Source", fr: "Source" };
const DESTINATION = { en: "Destination", fr: "Destination" };
const WEIGHT = { en: "Weight", fr: "Poids" };
const NODE = { en: "Node", fr: "Nœud" };
const INFLOW = { en: "Incoming", fr: "Entrant" };
const OUTFLOW = { en: "Outgoing", fr: "Sortant" };
/** The link colour choices (`links.colorBy`). */
const COLOR_BY = [
    { value: "source", en: "Source", fr: "Source" },
    { value: "destination", en: "Destination", fr: "Destination" },
    { value: "fixed", en: "One color", fr: "Une couleur" },
];

/** A drop-down of fixed items for an `enumeration` property (the API's `ItemDropdown`). */
function dropdown(objectName: string, propertyName: string, displayName: string, value: string, items: { value: string; displayName: string }[]): Slice {
    let current = items[0];
    for (const it of items) {
        if (it.value === value) {
            current = it;
        }
    }
    return {
        uid: `${objectName}_${propertyName}`,
        displayName,
        control: { type: "Dropdown", properties: { descriptor: { objectName, propertyName }, value: current, items } },
    };
}

function colorBy(dv: any): string {
    const v = objectValue(dv, "links", "colorBy", "source");
    return v === "destination" || v === "fixed" ? v : "source";
}

export class Visual extends PackVisual implements IVisual {
    private cols: (RoleColumn | null)[] = [];
    private rows: any[][] = [];
    /** The drawn nodes' labels by their order seen, for the format pane's node colours. */
    private names: string[] = [];

    constructor(options: VisualConstructorOptions) {
        super(options, "bicraft-sankey");
    }

    protected measureRole(): string | null {
        return "weight";
    }

    protected title(): string {
        return "BIcraft Sankey";
    }

    protected required() {
        return [
            { role: "source", en: SOURCE.en, fr: SOURCE.fr },
            { role: "destination", en: DESTINATION.en, fr: DESTINATION.fr },
            { role: "weight", en: WEIGHT.en, fr: WEIGHT.fr },
        ];
    }

    protected input(dv: any): any {
        this.cols = [roleColumn(dv, "source"), roleColumn(dv, "destination"), roleColumn(dv, "weight")];
        this.rows = rowsOf(this.cols);
        const options: any = {
            nodeWidth: objectValue(dv, "nodes", "width", 12),
            nodePadding: objectValue(dv, "nodes", "padding", 10),
            linkColor: colorBy(dv),
            linkFill: objectValue(dv, "links", "fill", FIXED_LINK),
            linkOpacity: objectValue(dv, "links", "opacity", 40),
            showLabels: objectValue(dv, "labels", "show", true),
            // unset: the look's text size, as report.render draws it
            fontSize: objectValue(dv, "labels", "fontSize", NaN),
        };
        // no colour set: the nodes take the look's palette
        for (let i = 1; i <= NODE_COLORS; i++) {
            options["nodeColor" + i] = objectValue(dv, "colors", "node" + i, "");
        }
        return { rows: this.rows, formats: formatsOf(this.cols), options };
    }

    protected layout(input: any, w: number, h: number): Out {
        const out = layoutSankey(input, w, h);
        this.names = [];
        for (const hit of out.hits) {
            if (typeof hit.node === "number" && hit.node < NODE_COLORS) {
                this.names[hit.node] = String(hit.label);
            }
        }
        return out;
    }

    /** The rows whose source and destination texts pass `keep`. */
    private rowsWhere(keep: (s: string, d: string) => boolean): number[] {
        const out: number[] = [];
        for (let r = 0; r < this.rows.length && r < MAX_ROWS; r++) {
            const row = this.rows[r];
            if (row && keep(cellText(row[0]), cellText(row[1]))) {
                out.push(r);
            }
        }
        return out;
    }

    protected point(hit: Hit): Point | null {
        const [source, destination, weight] = this.cols;
        const name = (c: RoleColumn | null, text: { en: string; fr: string }) => (c && c.name !== "" ? c.name : pick(this.locale, text));
        const value = (v: any) => formatValue(v, weight ? weight.format : "", this.locale);
        let rows: number[];
        let tip: TooltipItem[];
        if (typeof hit.link === "number") {
            rows = this.rowsWhere((s, d) => s === hit.source && d === hit.target);
            tip = [
                { displayName: name(source, SOURCE), value: String(hit.sourceLabel) },
                { displayName: name(destination, DESTINATION), value: String(hit.targetLabel) },
                { displayName: name(weight, WEIGHT), value: value(hit.value) },
            ];
        } else {
            rows = this.rowsWhere((s, d) => s === hit.name || d === hit.name);
            tip = [{ displayName: pick(this.locale, NODE), value: String(hit.label) }];
            if (hit.inflow > 0) {
                tip.push({ displayName: pick(this.locale, INFLOW), value: value(hit.inflow) });
            }
            if (hit.outflow > 0) {
                tip.push({ displayName: pick(this.locale, OUTFLOW), value: value(hit.outflow) });
            }
        }
        // the host gives one identity per row (source and destination), so a link or a node is
        // all its rows
        const ids = rows.map((r) => idOf(this.host, [source, destination], r)).filter((id) => id !== undefined);
        const id = ids.length === 0 ? undefined : ids.length === 1 ? ids[0] : ids;
        return { id, tip };
    }

    public getFormattingModel(): powerbi.visuals.FormattingModel {
        const dv = this.dataView;
        const fr = this.locale.toLowerCase().startsWith("fr");
        const nodes = card("nodes", fr ? "Nœuds" : "Nodes", [
            group("nodes_group", fr ? "Nœuds" : "Nodes", [
                number("nodes", "width", fr ? "Largeur" : "Width", objectValue(dv, "nodes", "width", 12), 2, 60),
                number("nodes", "padding", fr ? "Espacement" : "Spacing", objectValue(dv, "nodes", "padding", 10), 0, 60),
            ]),
        ]);
        const items = COLOR_BY.map((c) => ({ value: c.value, displayName: fr ? c.fr : c.en }));
        const links = card("links", fr ? "Liens" : "Links", [
            group("links_group", fr ? "Liens" : "Links", [
                dropdown("links", "colorBy", fr ? "Couleur selon" : "Color by", colorBy(dv), items),
                color("links", "fill", fr ? "Couleur" : "Color", objectValue(dv, "links", "fill", FIXED_LINK)),
                number("links", "opacity", fr ? "Opacité (%)" : "Opacity (%)", objectValue(dv, "links", "opacity", 40), 5, 100),
            ]),
        ]);
        const labels = card("labels", fr ? "Étiquettes" : "Labels", [
            group("labels_group", fr ? "Étiquettes" : "Labels", [
                toggle("labels", "show", fr ? "Afficher" : "Show", objectValue(dv, "labels", "show", true)),
                number("labels", "fontSize", fr ? "Taille du texte" : "Text size", objectValue(dv, "labels", "fontSize", 11 + this.look().value - 10), 8, 20),
            ]),
        ]);
        // a swatch per node, named after the node drawn in that place of the order seen
        const k = this.look();
        const swatches: Slice[] = [];
        for (let i = 1; i <= NODE_COLORS; i++) {
            const name = this.names[i - 1] ? this.names[i - 1] : (fr ? "Nœud " : "Node ") + i;
            swatches.push(color("colors", "node" + i, name, objectValue(dv, "colors", "node" + i, colorOf(k, i - 1))));
        }
        const colors = card("colors", fr ? "Couleurs des nœuds" : "Node colors", [group("colors_group", fr ? "Couleurs des nœuds" : "Node colors", swatches)]);
        return model(withLook([nodes, colors, links, labels], this.locale, k, this.paletteOption())) as any;
    }
}
