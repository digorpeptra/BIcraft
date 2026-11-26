// BIcraft Scatter+ (docs/formats/chart-pack.md "Scatter+"): the `IVisual` class. The roles
// details, legend, x, y and size; ./scatter lays out the points (or the density cells), the
// marginal strips, the trend line, the quadrants and the labels. A point selects its row, a
// density cell every row in it. With labels on the selected points, a selection redraws the
// visual with their labels.
"use strict";

import powerbi from "powerbi-visuals-api";
import VisualConstructorOptions = powerbi.extensibility.visual.VisualConstructorOptions;
import IVisual = powerbi.extensibility.visual.IVisual;
import { Point, PackVisual, RoleColumn, formatsOf, idOf, roleColumn, rowsOf, tipLine } from "./base";
import { Hit, Out } from "./kit";
import { MAX_LABELS, MAX_POINTS, layoutScatter } from "./scatter";
import { objectValue } from "./runtime/data";
import { formatValue } from "./runtime/format";
import { card, dropdown, group, model, number, textInput, toggle, withLook } from "./runtime/formatting";
import { pick } from "./runtime/strings";
import { TooltipItem } from "./runtime/tooltips";

const DETAILS = { en: "Details", fr: "Détails" };
const X = { en: "X axis", fr: "Axe X" };
const Y = { en: "Y axis", fr: "Axe Y" };
const POINTS = { en: "Points", fr: "Points" };

function choice(dv: any, object: string, prop: string, items: string[], def: string): string {
    const v = objectValue(dv, object, prop, def);
    return typeof v === "string" && items.indexOf(v) >= 0 ? v : def;
}

/** A number property as set, else null (the layout's default, which follows the look). */
function numberOrNull(dv: any, object: string, prop: string): number | null {
    const v = objectValue(dv, object, prop, NaN);
    return isFinite(v) ? v : null;
}

function items(list: [string, string, string][], fr: boolean): { value: string; displayName: string }[] {
    return list.map(([value, en, f]) => ({ value, displayName: fr ? f : en }));
}

export class Visual extends PackVisual implements IVisual {
    private cols: (RoleColumn | null)[] = [];
    private rows: any[][] = [];
    /** The row of each target of the drawing being made, and of the last one. */
    private targetRows: number[] = [];
    private prevRows: number[] = [];
    /** The selection the labels were drawn for. */
    private labelled = "";
    private drawing = false;

    constructor(options: VisualConstructorOptions) {
        super(options, "bicraft-scatter");
        const restyle = this.interactions.selection.onChange;
        this.interactions.selection.onChange = (rows: number[]) => {
            restyle(rows);
            if (!this.drawing && this.labelsFollow() && rows.join(",") !== this.labelled) {
                this.draw();
            }
        };
    }

    private labelsFollow(): boolean {
        return choice(this.dataView, "labels", "mode", ["none", "top", "selected"], "none") === "selected";
    }

    protected draw(): void {
        // a second pass when the host's selection, shown once the points are wired, asks for other labels
        for (let pass = 0; pass < 2; pass++) {
            this.prevRows = this.targetRows;
            this.targetRows = [];
            this.drawing = true;
            try {
                super.draw();
            } finally {
                this.drawing = false;
            }
            if (!(this.labelsFollow() && this.interactions.selection.selected.join(",") !== this.labelled)) {
                break;
            }
        }
    }

    protected measureRole(): string | null {
        return "y";
    }

    protected title(): string {
        return "BIcraft Scatter+";
    }

    protected required() {
        return [
            { role: "details", en: DETAILS.en, fr: DETAILS.fr },
            { role: "x", en: X.en, fr: X.fr },
            { role: "y", en: Y.en, fr: Y.fr },
        ];
    }

    protected input(dv: any): any {
        this.cols = ["details", "legend", "x", "y", "size"].map((r) => roleColumn(dv, r));
        this.rows = rowsOf(this.cols);
        const sel = this.interactions.selection.selected;
        this.labelled = sel.join(",");
        const selected: number[] = [];
        for (const t of sel) {
            if (t >= 0 && t < this.prevRows.length && selected.length < MAX_LABELS) {
                selected.push(this.prevRows[t]);
            }
        }
        return {
            rows: this.rows,
            formats: formatsOf(this.cols),
            names: this.cols.map((c) => (c ? c.name : "")),
            selected,
            options: {
                markerSize: objectValue(dv, "markers", "size", 5),
                markerFill: objectValue(dv, "markers", "fill", true),
                markerBorder: objectValue(dv, "markers", "border", true),
                fillOpacity: objectValue(dv, "markers", "opacity", 70),
                density: choice(dv, "density", "mode", ["auto", "on", "off"], "auto"),
                densityThreshold: objectValue(dv, "density", "threshold", 2000),
                hexSize: objectValue(dv, "density", "cellSize", 10),
                marginals: choice(dv, "marginals", "kind", ["none", "histogram", "box"], "none"),
                trendLine: objectValue(dv, "trend", "show", false),
                quadrants: objectValue(dv, "quadrants", "show", false),
                quadrantAt: choice(dv, "quadrants", "at", ["average", "value"], "average"),
                quadrantX: objectValue(dv, "quadrants", "x", 0),
                quadrantY: objectValue(dv, "quadrants", "y", 0),
                labelTopRight: objectValue(dv, "quadrants", "topRight", ""),
                labelTopLeft: objectValue(dv, "quadrants", "topLeft", ""),
                labelBottomLeft: objectValue(dv, "quadrants", "bottomLeft", ""),
                labelBottomRight: objectValue(dv, "quadrants", "bottomRight", ""),
                labelMode: choice(dv, "labels", "mode", ["none", "top", "selected"], "none"),
                labelCount: objectValue(dv, "labels", "count", 5),
                // unset: the look's size
                fontSize: numberOrNull(dv, "labels", "fontSize"),
                showLegend: objectValue(dv, "legend", "show", true),
            },
        };
    }

    protected layout(input: any, w: number, h: number): Out {
        return layoutScatter(input, w, h);
    }

    protected point(hit: Hit): Point | null {
        const [details, legend] = this.cols;
        let tip: TooltipItem[];
        let rows: number[];
        if (Array.isArray(hit.rows)) {
            rows = hit.rows.filter((r: any) => typeof r === "number" && r >= 0 && r < MAX_POINTS);
            tip = [{ displayName: pick(this.locale, POINTS), value: formatValue(hit.n, "", this.locale) }];
        } else {
            rows = [hit.row];
            tip = [];
            for (const c of this.cols) {
                tip = tip.concat(tipLine(c, hit.row, this.locale));
            }
        }
        this.targetRows.push(typeof hit.row === "number" ? hit.row : -1);
        const ids = rows.map((r) => idOf(this.host, [details, legend], r)).filter((id) => id !== undefined);
        const id = ids.length === 0 ? undefined : ids.length === 1 ? ids[0] : ids;
        return { id, tip };
    }

    public getFormattingModel(): powerbi.visuals.FormattingModel {
        const dv = this.dataView;
        const fr = this.locale.toLowerCase().startsWith("fr");
        const markers = card("markers", fr ? "Marqueurs" : "Markers", [
            group("markers_group", fr ? "Marqueurs" : "Markers", [
                number("markers", "size", fr ? "Taille" : "Size", objectValue(dv, "markers", "size", 5), 2, 20),
                toggle("markers", "fill", fr ? "Remplissage" : "Fill", objectValue(dv, "markers", "fill", true)),
                toggle("markers", "border", fr ? "Bordure" : "Border", objectValue(dv, "markers", "border", true)),
                number("markers", "opacity", fr ? "Opacité du remplissage (%)" : "Fill opacity (%)", objectValue(dv, "markers", "opacity", 70), 10, 100),
            ]),
        ]);
        const density = card("density", fr ? "Densité" : "Density", [
            group("density_group", fr ? "Densité" : "Density", [
                dropdown("density", "mode", "Mode", choice(dv, "density", "mode", ["auto", "on", "off"], "auto"), items([["auto", "Automatic", "Automatique"], ["on", "On", "Activé"], ["off", "Off", "Désactivé"]], fr)),
                number("density", "threshold", fr ? "Points avant la densité" : "Points before density", objectValue(dv, "density", "threshold", 2000), 100, MAX_POINTS),
                number("density", "cellSize", fr ? "Taille des cellules" : "Cell size", objectValue(dv, "density", "cellSize", 10), 4, 40),
            ]),
        ]);
        const marginals = card("marginals", fr ? "Distributions marginales" : "Marginal distributions", [
            group("marginals_group", fr ? "Distributions marginales" : "Marginal distributions", [
                dropdown("marginals", "kind", fr ? "Type" : "Kind", choice(dv, "marginals", "kind", ["none", "histogram", "box"], "none"), items([["none", "None", "Aucun"], ["histogram", "Histograms", "Histogrammes"], ["box", "Box strips", "Bandes à boîtes"]], fr)),
            ]),
        ]);
        const trend = card("trend", fr ? "Courbe de tendance" : "Trend line", [
            group("trend_group", fr ? "Courbe de tendance" : "Trend line", [toggle("trend", "show", fr ? "Afficher" : "Show", objectValue(dv, "trend", "show", false))]),
        ]);
        const quadrants = card("quadrants", "Quadrants", [
            group("quadrants_group", "Quadrants", [
                toggle("quadrants", "show", fr ? "Afficher" : "Show", objectValue(dv, "quadrants", "show", false)),
                dropdown("quadrants", "at", fr ? "Lignes à" : "Lines at", choice(dv, "quadrants", "at", ["average", "value"], "average"), items([["average", "Averages", "Moyennes"], ["value", "Given values", "Valeurs données"]], fr)),
                number("quadrants", "x", fr ? "Valeur X" : "X value", objectValue(dv, "quadrants", "x", 0)),
                number("quadrants", "y", fr ? "Valeur Y" : "Y value", objectValue(dv, "quadrants", "y", 0)),
                textInput("quadrants", "topRight", fr ? "Étiquette en haut à droite" : "Top right label", objectValue(dv, "quadrants", "topRight", "")),
                textInput("quadrants", "topLeft", fr ? "Étiquette en haut à gauche" : "Top left label", objectValue(dv, "quadrants", "topLeft", "")),
                textInput("quadrants", "bottomLeft", fr ? "Étiquette en bas à gauche" : "Bottom left label", objectValue(dv, "quadrants", "bottomLeft", "")),
                textInput("quadrants", "bottomRight", fr ? "Étiquette en bas à droite" : "Bottom right label", objectValue(dv, "quadrants", "bottomRight", "")),
            ]),
        ]);
        const labels = card("labels", fr ? "Étiquettes" : "Labels", [
            group("labels_group", fr ? "Étiquettes" : "Labels", [
                dropdown("labels", "mode", fr ? "Points étiquetés" : "Points labelled", choice(dv, "labels", "mode", ["none", "top", "selected"], "none"), items([["none", "None", "Aucun"], ["top", "Top points", "Points les plus hauts"], ["selected", "Selected points", "Points sélectionnés"]], fr)),
                number("labels", "count", fr ? "Combien" : "How many", objectValue(dv, "labels", "count", 5), 1, MAX_LABELS),
                number("labels", "fontSize", fr ? "Taille du texte" : "Text size", objectValue(dv, "labels", "fontSize", 11), 8, 20),
            ]),
        ]);
        const legend = card("legend", fr ? "Légende" : "Legend", [
            group("legend_group", fr ? "Légende" : "Legend", [toggle("legend", "show", fr ? "Afficher" : "Show", objectValue(dv, "legend", "show", true))]),
        ]);
        return model(withLook([markers, density, marginals, trend, quadrants, labels, legend], this.locale, this.look(), this.paletteOption())) as any;
    }
}
