// The BIcraft Dumbbell and slope chart (docs/formats/bicraft-visuals.md "Dumbbell and slope
// chart"): two measures per category, as dumbbells or as a slope chart. The layout is in
// ./dumbbell.ts.
"use strict";

import powerbi from "powerbi-visuals-api";
import VisualConstructorOptions = powerbi.extensibility.visual.VisualConstructorOptions;
import { Unit, optColor, optFlag } from "./common";
import { color as colorOf, lookOf } from "./runtime/style";
import { colorsOf, layout } from "./dumbbell";
import { BVisual, Def, rowTooltip } from "./host";
import { card, color, group, toggle } from "./runtime/formatting";
import { pick } from "./runtime/strings";

const L = (locale: string, en: string, fr: string) => pick(locale, { en, fr });

const DEF: Def = {
    title: "BIcraft Dumbbell",
    roles: [
        { name: "category", en: "Category", fr: "Catégorie" },
        { name: "start", en: "Start", fr: "Début" },
        { name: "end", en: "End", fr: "Fin" },
    ],
    required: ["category", "start", "end"],
    idRole: "category",
    layout,
    tooltip: rowTooltip,
    idRows: (u: Unit) => u.rows,
    cards(o: any, locale: string) {
        return [
            card("style", L(locale, "Style", "Style"), [
                group("style_group", L(locale, "Style", "Style"), [toggle("style", "slope", L(locale, "Slope chart", "Graphique de pente"), optFlag(o, "style", "slope", false))]),
            ]),
            card("colors", L(locale, "Colors", "Couleurs"), [
                group("colors_group", L(locale, "Colors", "Couleurs"), [
                    color("colors", "start", L(locale, "Start", "Début"), optColor(o, "colors", "start", colorOf(lookOf(o.style, o.$theme), 0))),
                    color("colors", "end", L(locale, "End", "Fin"), optColor(o, "colors", "end", colorOf(lookOf(o.style, o.$theme), 1))),
                    color("colors", "line", L(locale, "Line", "Ligne"), colorsOf(o, lookOf(o.style, o.$theme)).line),
                ]),
            ]),
            card("labels", L(locale, "Data labels", "Étiquettes de données"), [
                group("labels_group", L(locale, "Data labels", "Étiquettes de données"), [toggle("labels", "show", L(locale, "Show", "Afficher"), optFlag(o, "labels", "show", true))]),
            ]),
        ];
    },
};

export class Visual extends BVisual {
    constructor(options: VisualConstructorOptions) {
        super(options, DEF);
    }
}
