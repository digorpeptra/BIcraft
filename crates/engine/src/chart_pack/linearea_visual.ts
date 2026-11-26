// BIcraft Line / Area+ (docs/formats/chart-pack.md "Line / Area+"): the `IVisual` class. The
// roles axis, series, value and value2; ./linearea lays out the lines, areas, bands, threshold
// shading, forecast stretch, labels and line-end names. A point selects its row; when a series
// has too many points for one target each, its line selects every row of the series.
"use strict";

import powerbi from "powerbi-visuals-api";
import VisualConstructorOptions = powerbi.extensibility.visual.VisualConstructorOptions;
import IVisual = powerbi.extensibility.visual.IVisual;
import { Point, PackVisual, RoleColumn, formatsOf, idOf, roleColumn, rowsOf, tipLine } from "./base";
import { Hit, MS_PER_DAY, Out, cellText } from "./kit";
import { MAX_ROWS, layoutLineArea } from "./linearea";
import { objectValue } from "./runtime/data";
import { card, color, dropdown, group, model, number, textInput, toggle, withLook } from "./runtime/formatting";
import { TooltipItem } from "./runtime/tooltips";

const AXIS = { en: "Axis", fr: "Axe" };
const VALUE = { en: "Value", fr: "Valeur" };

function choice(dv: any, object: string, prop: string, items: string[], def: string): string {
    const v = objectValue(dv, object, prop, def);
    return typeof v === "string" && items.indexOf(v) >= 0 ? v : def;
}

/** A number property as set, else null (the layout's default, which follows the look). */
function numberOrNull(dv: any, object: string, prop: string): number | null {
    const v = objectValue(dv, object, prop, NaN);
    return isFinite(v) ? v : null;
}

/** Whether the value axis starts at zero unless set: with an area or stacked series. */
function zeroByDefault(dv: any): boolean {
    return objectValue(dv, "area", "show", false) === true || objectValue(dv, "area", "stacked", false) === true;
}

function items(list: [string, string, string][], fr: boolean): { value: string; displayName: string }[] {
    return list.map(([value, en, f]) => ({ value, displayName: fr ? f : en }));
}

export class Visual extends PackVisual implements IVisual {
    private cols: (RoleColumn | null)[] = [];
    private rows: any[][] = [];

    constructor(options: VisualConstructorOptions) {
        super(options, "bicraft-linearea");
    }

    protected measureRole(): string | null {
        return "value";
    }

    protected title(): string {
        return "BIcraft Line / Area+";
    }

    protected required() {
        return [
            { role: "axis", en: AXIS.en, fr: AXIS.fr },
            { role: "value", en: VALUE.en, fr: VALUE.fr },
        ];
    }

    protected input(dv: any): any {
        this.cols = ["axis", "series", "value", "value2"].map((r) => roleColumn(dv, r));
        this.rows = rowsOf(this.cols);
        return {
            rows: this.rows,
            formats: formatsOf(this.cols),
            // today at midnight UTC, for a forecast from `today`
            today: Math.floor(Date.now() / MS_PER_DAY) * MS_PER_DAY,
            options: {
                lineColor: objectValue(dv, "lines", "color", ""),
                lineWidth: numberOrNull(dv, "lines", "width"),
                markers: objectValue(dv, "lines", "markers", false),
                emphasize: objectValue(dv, "lines", "emphasize", ""),
                area: objectValue(dv, "area", "show", false),
                areaFill: objectValue(dv, "area", "fill", ""),
                areaOpacity: objectValue(dv, "area", "opacity", 25),
                stacked: objectValue(dv, "area", "stacked", false),
                band: objectValue(dv, "band", "show", true),
                bandUp: objectValue(dv, "band", "up", "#0E9F84"),
                bandDown: objectValue(dv, "band", "down", "#E23D4F"),
                bandOpacity: objectValue(dv, "band", "opacity", 30),
                threshold: objectValue(dv, "threshold", "show", false),
                thresholdValue: objectValue(dv, "threshold", "value", 0),
                thresholdSide: choice(dv, "threshold", "side", ["above", "below"], "above"),
                thresholdColor: objectValue(dv, "threshold", "color", "#E23D4F"),
                colorRule: choice(dv, "rule", "mode", ["none", "above", "below"], "none"),
                ruleValue: objectValue(dv, "rule", "value", 0),
                ruleColor: objectValue(dv, "rule", "color", "#E23D4F"),
                forecastFrom: objectValue(dv, "forecast", "from", ""),
                labelMode: choice(dv, "labels", "mode", ["last", "minmax", "all", "none"], "last"),
                fontSize: numberOrNull(dv, "labels", "fontSize"),
                endLabels: objectValue(dv, "labels", "endLabels", true),
                axisType: choice(dv, "axes", "type", ["auto", "category"], "auto"),
                yZero: objectValue(dv, "axes", "zero", zeroByDefault(dv)),
                showLegend: objectValue(dv, "legend", "show", true),
            },
        };
    }

    protected layout(input: any, w: number, h: number): Out {
        return layoutLineArea(input, w, h);
    }

    protected point(hit: Hit): Point | null {
        const [axis, series] = this.cols;
        let rows: number[];
        let tip: TooltipItem[] = [];
        if (hit.line === true) {
            // the whole series: every row of it
            const row = this.rows[hit.row];
            const name = row ? cellText(row[1]) : "";
            rows = [];
            for (let r = 0; r < this.rows.length && r < MAX_ROWS; r++) {
                if (this.rows[r] && cellText(this.rows[r][1]) === name) {
                    rows.push(r);
                }
            }
            tip = tipLine(series, hit.row, this.locale);
        } else {
            rows = [hit.row];
            for (const c of this.cols) {
                tip = tip.concat(tipLine(c, hit.row, this.locale));
            }
        }
        const ids = rows.map((r) => idOf(this.host, [axis, series], r)).filter((id) => id !== undefined);
        const id = ids.length === 0 ? undefined : ids.length === 1 ? ids[0] : ids;
        return { id, tip };
    }

    public getFormattingModel(): powerbi.visuals.FormattingModel {
        const dv = this.dataView;
        const fr = this.locale.toLowerCase().startsWith("fr");
        const show = fr ? "Afficher" : "Show";
        const lines = card("lines", fr ? "Lignes" : "Lines", [
            group("lines_group", fr ? "Lignes" : "Lines", [
                color("lines", "color", fr ? "Couleur" : "Color", objectValue(dv, "lines", "color", "#4A5BE8")),
                number("lines", "width", fr ? "Largeur" : "Width", objectValue(dv, "lines", "width", 2.5), 0.5, 8),
                toggle("lines", "markers", fr ? "Marqueurs" : "Markers", objectValue(dv, "lines", "markers", false)),
                textInput("lines", "emphasize", fr ? "Série mise en avant" : "Emphasized series", objectValue(dv, "lines", "emphasize", "")),
            ]),
        ]);
        const area = card("area", fr ? "Aire" : "Area", [
            group("area_group", fr ? "Aire" : "Area", [
                toggle("area", "show", show, objectValue(dv, "area", "show", false)),
                color("area", "fill", fr ? "Couleur" : "Color", objectValue(dv, "area", "fill", "#4A5BE8")),
                number("area", "opacity", fr ? "Opacité (%)" : "Opacity (%)", objectValue(dv, "area", "opacity", 25), 0, 100),
                toggle("area", "stacked", fr ? "Empiler les séries" : "Stack the series", objectValue(dv, "area", "stacked", false)),
            ]),
        ]);
        const band = card("band", fr ? "Bande entre les valeurs" : "Band between values", [
            group("band_group", fr ? "Bande entre les valeurs" : "Band between values", [
                toggle("band", "show", show, objectValue(dv, "band", "show", true)),
                color("band", "up", fr ? "Première valeur au-dessus" : "First value above", objectValue(dv, "band", "up", "#0E9F84")),
                color("band", "down", fr ? "Première valeur en dessous" : "First value below", objectValue(dv, "band", "down", "#E23D4F")),
                number("band", "opacity", fr ? "Opacité (%)" : "Opacity (%)", objectValue(dv, "band", "opacity", 30), 5, 100),
            ]),
        ]);
        const threshold = card("threshold", fr ? "Seuil" : "Threshold", [
            group("threshold_group", fr ? "Seuil" : "Threshold", [
                toggle("threshold", "show", show, objectValue(dv, "threshold", "show", false)),
                number("threshold", "value", fr ? "Valeur" : "Value", objectValue(dv, "threshold", "value", 0)),
                dropdown("threshold", "side", fr ? "Ombrer" : "Shade", choice(dv, "threshold", "side", ["above", "below"], "above"), items([["above", "Above", "Au-dessus"], ["below", "Below", "En dessous"]], fr)),
                color("threshold", "color", fr ? "Couleur" : "Color", objectValue(dv, "threshold", "color", "#E23D4F")),
            ]),
        ]);
        const rule = card("rule", fr ? "Règle de couleur" : "Color rule", [
            group("rule_group", fr ? "Règle de couleur" : "Color rule", [
                dropdown("rule", "mode", "Mode", choice(dv, "rule", "mode", ["none", "above", "below"], "none"), items([["none", "None", "Aucun"], ["above", "Above a value", "Au-dessus d'une valeur"], ["below", "Below a value", "En dessous d'une valeur"]], fr)),
                number("rule", "value", fr ? "Valeur" : "Value", objectValue(dv, "rule", "value", 0)),
                color("rule", "color", fr ? "Couleur" : "Color", objectValue(dv, "rule", "color", "#E23D4F")),
            ]),
        ]);
        const forecast = card("forecast", fr ? "Prévision" : "Forecast", [
            group("forecast_group", fr ? "Prévision" : "Forecast", [
                textInput("forecast", "from", fr ? "À partir du (aaaa-mm-jj ou today)" : "From date (yyyy-mm-dd or today)", objectValue(dv, "forecast", "from", ""), "2025-01-01"),
            ]),
        ]);
        const labels = card("labels", fr ? "Étiquettes" : "Labels", [
            group("labels_group", fr ? "Étiquettes" : "Labels", [
                dropdown("labels", "mode", "Mode", choice(dv, "labels", "mode", ["last", "minmax", "all", "none"], "last"), items([["last", "Last point", "Dernier point"], ["minmax", "Lowest and highest", "Plus bas et plus haut"], ["all", "All points", "Tous les points"], ["none", "None", "Aucun"]], fr)),
                number("labels", "fontSize", fr ? "Taille du texte" : "Text size", objectValue(dv, "labels", "fontSize", 11), 8, 20),
                toggle("labels", "endLabels", fr ? "Noms des séries en bout de ligne" : "Series names at line ends", objectValue(dv, "labels", "endLabels", true)),
            ]),
        ]);
        const axes = card("axes", "Axes", [
            group("axes_group", "Axes", [
                dropdown("axes", "type", fr ? "Type d'axe" : "Axis type", choice(dv, "axes", "type", ["auto", "category"], "auto"), items([["auto", "Automatic", "Automatique"], ["category", "Categories", "Catégories"]], fr)),
                toggle("axes", "zero", fr ? "Commencer à zéro" : "Start at zero", objectValue(dv, "axes", "zero", zeroByDefault(dv))),
            ]),
        ]);
        const legend = card("legend", fr ? "Légende" : "Legend", [
            group("legend_group", fr ? "Légende" : "Legend", [toggle("legend", "show", show, objectValue(dv, "legend", "show", true))]),
        ]);
        return model(withLook([lines, area, band, threshold, rule, forecast, labels, axes, legend], this.locale, this.look(), this.paletteOption())) as any;
    }
}
