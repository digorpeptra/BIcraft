// The BIcraft Box plot (docs/formats/bicraft-visuals.md "Box plot"): the distribution of a
// measure over the items of a column, one box per category. The layout is in ./box_plot.ts.
"use strict";

import powerbi from "powerbi-visuals-api";
import VisualConstructorOptions = powerbi.extensibility.visual.VisualConstructorOptions;
import { layout } from "./box_plot";
import { Data, Unit, col, labelOf, optColor, optFlag } from "./common";
import { color as colorOf, lookOf } from "./runtime/style";
import { BVisual, Def } from "./host";
import { formatValue } from "./runtime/format";
import { card, color, group, toggle } from "./runtime/formatting";
import { pick } from "./runtime/strings";
import { TooltipItem } from "./runtime/tooltips";

const L = (locale: string, en: string, fr: string) => pick(locale, { en, fr });

const DEF: Def = {
    title: "BIcraft Box Plot",
    roles: [
        { name: "category", en: "Category", fr: "Catégorie" },
        { name: "sample", en: "Sample", fr: "Échantillon" },
        { name: "value", en: "Value", fr: "Valeur" },
    ],
    required: ["sample", "value"],
    idRole: "category",
    layout,
    tooltip(d: Data, u: Unit, locale: string): TooltipItem[] {
        const cat = col(d, "category", 0);
        const vc = col(d, "value", 0);
        const f = vc ? vc.format : "";
        const v = (en: string, fr: string, x: number) => ({ displayName: L(locale, en, fr), value: formatValue(x, f, locale) });
        const s = u.stats;
        const out: TooltipItem[] = [];
        if (cat && u.rows.length > 0) {
            out.push({ displayName: cat.name, value: labelOf(cat, u.rows[0]) });
        }
        out.push({ displayName: L(locale, "Count", "Nombre"), value: formatValue(s[0], "", locale) });
        out.push(v("Maximum", "Maximum", s[5]), v("Third quartile", "Troisième quartile", s[4]), v("Median", "Médiane", s[3]));
        out.push(v("First quartile", "Premier quartile", s[2]), v("Minimum", "Minimum", s[1]), v("Mean", "Moyenne", s[6]));
        out.push({ displayName: L(locale, "Outliers", "Valeurs aberrantes"), value: formatValue(s[7], "", locale) });
        return out;
    },
    // a box is one category value: the host gives one identity per row (category and sample),
    // so the box selects all its rows (seen in Power BI Desktop 2.159.1231.0: the first row's id
    // selected one sample)
    idRows: (u: Unit) => u.rows,
    cards(o: any, locale: string) {
        return [
            card("box", L(locale, "Boxes", "Boîtes"), [
                group("box_group", L(locale, "Boxes", "Boîtes"), [
                    toggle("box", "mean", L(locale, "Show the mean", "Afficher la moyenne"), optFlag(o, "box", "mean", true)),
                    toggle("box", "outliers", L(locale, "Show outliers", "Afficher les valeurs aberrantes"), optFlag(o, "box", "outliers", true)),
                    toggle("box", "horizontal", L(locale, "Horizontal", "Horizontal"), optFlag(o, "box", "horizontal", false)),
                ]),
            ]),
            card("colors", L(locale, "Colors", "Couleurs"), [
                group("colors_group", L(locale, "Colors", "Couleurs"), [color("colors", "fill", L(locale, "Boxes", "Boîtes"), optColor(o, "colors", "fill", colorOf(lookOf(o.style, o.$theme), 0)))]),
            ]),
        ];
    },
};

export class Visual extends BVisual {
    constructor(options: VisualConstructorOptions) {
        super(options, DEF);
    }
}
