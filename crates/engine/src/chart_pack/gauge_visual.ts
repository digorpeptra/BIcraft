// BIcraft Gauge / KPI+ (docs/formats/chart-pack.md "Gauge / KPI+"): the `IVisual` class. The
// roles trend, value, target, min, max, three thresholds and previous; ./gauge lays out the arc,
// the bullet or the KPI card and the trend's sparkline. The value and the target select the
// latest trend point, a sparkline point its own.
"use strict";

import powerbi from "powerbi-visuals-api";
import VisualConstructorOptions = powerbi.extensibility.visual.VisualConstructorOptions;
import IVisual = powerbi.extensibility.visual.IVisual;
import { Point, PackVisual, RoleColumn, formatsOf, idOf, roleColumn, rowsOf, tipLine } from "./base";
import { Hit, Out } from "./kit";
import { layoutGauge, pctText, scaleCommas, signed } from "./gauge";
import { objectValue } from "./runtime/data";
import { formatValue } from "./runtime/format";
import { Slice, card, color, group, model, number, toggle, withLook } from "./runtime/formatting";
import { pick } from "./runtime/strings";
import { TooltipItem } from "./runtime/tooltips";

const ROLES = ["trend", "value", "target", "min", "max", "threshold1", "threshold2", "threshold3", "previous"];
const VALUE = { en: "Value", fr: "Valeur" };
const TARGET = { en: "Target", fr: "Cible" };
const VS_TARGET = { en: "Variance vs target", fr: "Écart à la cible" };
const VS_PREVIOUS = { en: "Variance vs previous", fr: "Écart au précédent" };
const STYLES = [
    { value: "arc", en: "Radial arc", fr: "Arc radial" },
    { value: "bullet", en: "Linear bullet", fr: "Barre linéaire" },
    { value: "kpi", en: "KPI card", fr: "Carte d'indicateur" },
];
const COMPARE = [
    { value: "target", en: "Target", fr: "Cible" },
    { value: "previous", en: "Previous value", fr: "Valeur précédente" },
];
const BANDS = [
    { en: "First band", fr: "Première plage" },
    { en: "Second band", fr: "Deuxième plage" },
    { en: "Third band", fr: "Troisième plage" },
    { en: "Fourth band", fr: "Quatrième plage" },
];
const THRESHOLDS = [
    { en: "First threshold", fr: "Premier seuil" },
    { en: "Second threshold", fr: "Deuxième seuil" },
    { en: "Third threshold", fr: "Troisième seuil" },
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

/** An enumeration property's value when it is one of `items`, else the first. */
function choice(dv: any, object: string, property: string, items: { value: string }[]): string {
    const v = objectValue(dv, object, property, items[0].value);
    return items.some((i) => i.value === v) ? v : items[0].value;
}

export class Visual extends PackVisual implements IVisual {
    private cols: (RoleColumn | null)[] = [];

    constructor(options: VisualConstructorOptions) {
        super(options, "bicraft-gauge");
    }

    protected measureRole(): string | null {
        return "value";
    }

    protected title(): string {
        return "BIcraft Gauge / KPI+";
    }

    protected required() {
        return [{ role: "value", en: "Value", fr: "Valeur" }];
    }

    protected input(dv: any): any {
        this.cols = ROLES.map((r) => roleColumn(dv, r));
        return {
            rows: rowsOf(this.cols),
            formats: formatsOf(this.cols),
            options: {
                gaugeStyle: choice(dv, "gauge", "style", STYLES),
                compareTo: choice(dv, "gauge", "compareTo", COMPARE),
                lowerIsBetter: objectValue(dv, "gauge", "lowerIsBetter", false),
                showTrend: objectValue(dv, "gauge", "showTrend", true),
                min: objectValue(dv, "range", "min", NaN),
                max: objectValue(dv, "range", "max", NaN),
                target: objectValue(dv, "range", "target", NaN),
                threshold1: objectValue(dv, "range", "threshold1", NaN),
                threshold2: objectValue(dv, "range", "threshold2", NaN),
                threshold3: objectValue(dv, "range", "threshold3", NaN),
                valueFill: objectValue(dv, "colors", "value", ""),
                valueByBand: objectValue(dv, "colors", "valueByBand", false),
                band1: objectValue(dv, "colors", "band1", ""),
                band2: objectValue(dv, "colors", "band2", ""),
                band3: objectValue(dv, "colors", "band3", ""),
                band4: objectValue(dv, "colors", "band4", ""),
                targetFill: objectValue(dv, "colors", "target", ""),
            },
        };
    }

    protected layout(input: any, w: number, h: number): Out {
        return layoutGauge(input, w, h);
    }

    protected point(hit: Hit): Point | null {
        const trend = this.cols[0];
        const valueCol = this.cols[1];
        const f = valueCol ? scaleCommas(valueCol.format) : "";
        const value = (v: number) => formatValue(v, f, this.locale);
        const name = valueCol && valueCol.name !== "" ? valueCol.name : pick(this.locale, VALUE);
        let tip: TooltipItem[] = [];
        if (hit.part === "value") {
            tip = tip.concat(tipLine(trend, hit.row, this.locale));
            tip.push({ displayName: name, value: value(hit.value) });
            if (typeof hit.target === "number") {
                tip.push({ displayName: pick(this.locale, TARGET), value: value(hit.target) });
            }
            if (typeof hit.ref === "number") {
                const diff = hit.value - hit.ref;
                let s = signed(diff, value(diff));
                if (hit.ref !== 0) {
                    s += " (" + pctText(diff / Math.abs(hit.ref), this.locale) + ")";
                }
                tip.push({ displayName: pick(this.locale, hit.refKind === "previous" ? VS_PREVIOUS : VS_TARGET), value: s });
            }
        } else if (hit.part === "target") {
            tip.push({ displayName: pick(this.locale, TARGET), value: value(hit.value) });
        } else {
            tip = tip.concat(tipLine(trend, hit.row, this.locale));
            tip.push({ displayName: name, value: value(hit.value) });
        }
        return { id: idOf(this.host, [trend], hit.row), tip };
    }

    public getFormattingModel(): powerbi.visuals.FormattingModel {
        const dv = this.dataView;
        const fr = this.locale.toLowerCase().startsWith("fr");
        const L = (x: { en: string; fr: string }) => (fr ? x.fr : x.en);
        const items = (l: { value: string; en: string; fr: string }[]) => l.map((c) => ({ value: c.value, displayName: L(c) }));
        const look = this.look();
        const gauge = card("gauge", fr ? "Jauge" : "Gauge", [
            group("gauge_group", fr ? "Jauge" : "Gauge", [
                dropdown("gauge", "style", "Style", choice(dv, "gauge", "style", STYLES), items(STYLES)),
                dropdown("gauge", "compareTo", fr ? "Comparer à" : "Compare with", choice(dv, "gauge", "compareTo", COMPARE), items(COMPARE)),
                toggle("gauge", "lowerIsBetter", fr ? "Plus bas est mieux" : "Lower is better", objectValue(dv, "gauge", "lowerIsBetter", false)),
                toggle("gauge", "showTrend", fr ? "Afficher la tendance" : "Show the trend", objectValue(dv, "gauge", "showTrend", true)),
            ]),
        ]);
        const range = card("range", fr ? "Échelle" : "Range", [
            group("range_group", fr ? "Échelle" : "Range", [
                number("range", "min", "Minimum", objectValue(dv, "range", "min", 0)),
                number("range", "max", "Maximum", objectValue(dv, "range", "max", 0)),
                number("range", "target", L(TARGET), objectValue(dv, "range", "target", 0)),
            ].concat(THRESHOLDS.map((t, j) => number("range", "threshold" + (j + 1), L(t), objectValue(dv, "range", "threshold" + (j + 1), 0))))),
        ]);
        const colors = card("colors", fr ? "Couleurs" : "Colors", [
            group("colors_group", fr ? "Couleurs" : "Colors", [
                color("colors", "value", L(VALUE), objectValue(dv, "colors", "value", look.palette[0] || "#4A5BE8")),
                toggle("colors", "valueByBand", fr ? "Valeur dans la couleur de sa plage" : "Value in its band's color", objectValue(dv, "colors", "valueByBand", false)),
            ].concat(BANDS.map((b, j) => color("colors", "band" + (j + 1), L(b), objectValue(dv, "colors", "band" + (j + 1), look.empty))), [
                color("colors", "target", fr ? "Couleur de la cible" : "Target color", objectValue(dv, "colors", "target", look.label)),
            ])),
        ]);
        return model(withLook([gauge, range, colors], this.locale, look, this.paletteOption())) as any;
    }
}
