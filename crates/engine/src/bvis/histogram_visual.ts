// The BIcraft Histogram (docs/formats/bicraft-visuals.md "Histogram"): the values of a numeric
// column, weighted by a frequency measure, counted in bins. The layout is in ./histogram.ts.
"use strict";

import powerbi from "powerbi-visuals-api";
import VisualConstructorOptions = powerbi.extensibility.visual.VisualConstructorOptions;
import { Data, Unit, col, optColor, optFlag, optInt, optNum } from "./common";
import { color as colorOf, lookOf } from "./runtime/style";
import { MAX_BINS, layout } from "./histogram";
import { BVisual, Def } from "./host";
import { formatValue } from "./runtime/format";
import { card, color, group, number, toggle } from "./runtime/formatting";
import { pick } from "./runtime/strings";
import { TooltipItem } from "./runtime/tooltips";

const L = (locale: string, en: string, fr: string) => pick(locale, { en, fr });

const DEF: Def = {
    title: "BIcraft Histogram",
    roles: [
        { name: "values", en: "Values", fr: "Valeurs" },
        { name: "frequency", en: "Frequency", fr: "Fréquence" },
    ],
    required: ["values"],
    idRole: "values",
    layout,
    tooltip(d: Data, u: Unit, locale: string): TooltipItem[] {
        const vc = col(d, "values", 0);
        const fc = col(d, "frequency", 0);
        const f = vc ? vc.format : "";
        const count = fc ? fc.name : L(locale, "Count", "Nombre");
        const cumulative = u.stats[3] === 1;
        return [
            { displayName: vc ? vc.name : L(locale, "Bin", "Intervalle"), value: `${formatValue(u.stats[0], f, locale)} – ${formatValue(u.stats[1], f, locale)}` },
            { displayName: cumulative ? count + L(locale, " (cumulative)", " (cumulé)") : count, value: formatValue(u.stats[2], fc ? fc.format : "", locale) },
        ];
    },
    idRows: (u: Unit) => u.rows,
    cards(o: any, locale: string) {
        return [
            card("bins", L(locale, "Bins", "Intervalles"), [
                group("bins_group", L(locale, "Bins", "Intervalles"), [
                    number("bins", "count", L(locale, "Number of bins", "Nombre d'intervalles"), optInt(o, "bins", "count", 10, 1, MAX_BINS), 1, MAX_BINS),
                    number("bins", "size", L(locale, "Bin size (0: from the number of bins)", "Largeur d'un intervalle (0 : d'après leur nombre)"), optNum(o, "bins", "size", 0, 0, 1e15), 0),
                    toggle("bins", "cumulative", L(locale, "Cumulative", "Cumulé"), optFlag(o, "bins", "cumulative", false)),
                ]),
            ]),
            card("colors", L(locale, "Colors", "Couleurs"), [
                group("colors_group", L(locale, "Colors", "Couleurs"), [color("colors", "fill", L(locale, "Bars", "Barres"), optColor(o, "colors", "fill", colorOf(lookOf(o.style, o.$theme), 0)))]),
            ]),
            card("labels", L(locale, "Data labels", "Étiquettes de données"), [
                group("labels_group", L(locale, "Data labels", "Étiquettes de données"), [toggle("labels", "show", L(locale, "Show", "Afficher"), optFlag(o, "labels", "show", false))]),
            ]),
        ];
    },
};

export class Visual extends BVisual {
    constructor(options: VisualConstructorOptions) {
        super(options, DEF);
    }
}
