// The BIcraft Bullet chart (docs/formats/bicraft-visuals.md "Bullet chart"): a value against
// its target over qualitative bands, one bullet per category. The layout is in ./bullet.ts.
"use strict";

import powerbi from "powerbi-visuals-api";
import VisualConstructorOptions = powerbi.extensibility.visual.VisualConstructorOptions;
import { Unit, optColor, optFlag, optNum } from "./common";
import { color as colorOf, lookOf } from "./runtime/style";
import { layout } from "./bullet";
import { BVisual, Def, rowTooltip } from "./host";
import { card, color, group, number, toggle } from "./runtime/formatting";
import { pick } from "./runtime/strings";

const L = (locale: string, en: string, fr: string) => pick(locale, { en, fr });

const DEF: Def = {
    title: "BIcraft Bullet",
    roles: [
        { name: "category", en: "Category", fr: "Catégorie" },
        { name: "value", en: "Value", fr: "Valeur" },
        { name: "target", en: "Target", fr: "Cible" },
        { name: "bands", en: "Bands", fr: "Plages" },
        { name: "maximum", en: "Maximum", fr: "Maximum" },
    ],
    required: ["value"],
    idRole: "category",
    layout,
    tooltip: rowTooltip,
    idRows: (u: Unit) => u.rows,
    cards(o: any, locale: string) {
        const pct = (prop: string, en: string, fr: string, def: number) => number("bands", prop, L(locale, en, fr), optNum(o, "bands", prop, def, 0, 1000), 0, 1000);
        return [
            card("bands", L(locale, "Bands", "Plages"), [
                group("bands_group", L(locale, "Bands as % of the target (without band fields)", "Plages en % de la cible (sans champ de plage)"), [
                    pct("first", "First band", "Première plage", 60),
                    pct("second", "Second band", "Deuxième plage", 80),
                    pct("third", "Third band", "Troisième plage", 100),
                ]),
            ]),
            card("colors", L(locale, "Colors", "Couleurs"), [
                group("colors_group", L(locale, "Colors", "Couleurs"), [
                    color("colors", "value", L(locale, "Value", "Valeur"), optColor(o, "colors", "value", colorOf(lookOf(o.style, o.$theme), 0))),
                    color("colors", "target", L(locale, "Target", "Cible"), optColor(o, "colors", "target", lookOf(o.style, o.$theme).label)),
                ]),
            ]),
            card("labels", L(locale, "Data labels", "Étiquettes de données"), [
                group("labels_group", L(locale, "Data labels", "Étiquettes de données"), [toggle("labels", "show", L(locale, "Show", "Afficher"), optFlag(o, "labels", "show", false))]),
            ]),
            card("style", L(locale, "Style", "Style"), [
                group("style_group", L(locale, "Style", "Style"), [toggle("style", "vertical", L(locale, "Vertical", "Vertical"), optFlag(o, "style", "vertical", false))]),
            ]),
        ];
    },
};

export class Visual extends BVisual {
    constructor(options: VisualConstructorOptions) {
        super(options, DEF);
    }
}
