// BIcraft's visual runtime (docs/community-visuals.md "Runtime"): the text settings of a visual's
// text elements (data labels, the category and value axes and their titles, the legend): the
// format objects that hold them (`labels`, `categoryAxis`, `valueAxis`, `legend`, with the
// property names Power BI's own charts use), read into the `{part: {key: value}}` form the Spec
// layout takes (docs/formats/spec-visual.md "Text"), and their cards in the format pane: a show
// switch in the card's header, the font row (family, size, bold, italic, underline), the colour,
// and for numbers the display units and decimal places. BIcraft's engine reads the same objects
// the same way (`crate::visual_runtime::TEXT_PROPS`).
"use strict";

import { Card, Group, Slice, card, color, group } from "./formatting";
import { pick } from "./strings";

/** What a property holds: a switch, a colour, a text size, a font family, display units or decimal places. */
type Kind = "bool" | "color" | "size" | "family" | "units" | "decimals";

/** Each format property: its object, its name, the text element and setting it feeds, and its kind. */
export const TEXT_PROPS: [string, string, string, string, Kind][] = [
    ["labels", "show", "labels", "show", "bool"],
    ["labels", "color", "labels", "color", "color"],
    ["labels", "fontFamily", "labels", "family", "family"],
    ["labels", "fontSize", "labels", "size", "size"],
    ["labels", "bold", "labels", "bold", "bool"],
    ["labels", "italic", "labels", "italic", "bool"],
    ["labels", "underline", "labels", "underline", "bool"],
    ["labels", "labelDisplayUnits", "labels", "units", "units"],
    ["labels", "labelPrecision", "labels", "decimals", "decimals"],
    ["categoryAxis", "show", "x", "show", "bool"],
    ["categoryAxis", "labelColor", "x", "color", "color"],
    ["categoryAxis", "fontFamily", "x", "family", "family"],
    ["categoryAxis", "fontSize", "x", "size", "size"],
    ["categoryAxis", "bold", "x", "bold", "bool"],
    ["categoryAxis", "italic", "x", "italic", "bool"],
    ["categoryAxis", "underline", "x", "underline", "bool"],
    ["categoryAxis", "showAxisTitle", "xTitle", "show", "bool"],
    ["categoryAxis", "titleColor", "xTitle", "color", "color"],
    ["categoryAxis", "titleFontFamily", "xTitle", "family", "family"],
    ["categoryAxis", "titleFontSize", "xTitle", "size", "size"],
    ["categoryAxis", "titleBold", "xTitle", "bold", "bool"],
    ["categoryAxis", "titleItalic", "xTitle", "italic", "bool"],
    ["categoryAxis", "titleUnderline", "xTitle", "underline", "bool"],
    ["valueAxis", "show", "y", "show", "bool"],
    ["valueAxis", "labelColor", "y", "color", "color"],
    ["valueAxis", "fontFamily", "y", "family", "family"],
    ["valueAxis", "fontSize", "y", "size", "size"],
    ["valueAxis", "bold", "y", "bold", "bool"],
    ["valueAxis", "italic", "y", "italic", "bool"],
    ["valueAxis", "underline", "y", "underline", "bool"],
    ["valueAxis", "labelDisplayUnits", "y", "units", "units"],
    ["valueAxis", "labelPrecision", "y", "decimals", "decimals"],
    ["valueAxis", "showAxisTitle", "yTitle", "show", "bool"],
    ["valueAxis", "titleColor", "yTitle", "color", "color"],
    ["valueAxis", "titleFontFamily", "yTitle", "family", "family"],
    ["valueAxis", "titleFontSize", "yTitle", "size", "size"],
    ["valueAxis", "titleBold", "yTitle", "bold", "bool"],
    ["valueAxis", "titleItalic", "yTitle", "italic", "bool"],
    ["valueAxis", "titleUnderline", "yTitle", "underline", "bool"],
    ["legend", "show", "legend", "show", "bool"],
    ["legend", "labelColor", "legend", "color", "color"],
    ["legend", "fontFamily", "legend", "family", "family"],
    ["legend", "fontSize", "legend", "size", "size"],
    ["legend", "bold", "legend", "bold", "bool"],
    ["legend", "italic", "legend", "italic", "bool"],
    ["legend", "underline", "legend", "underline", "bool"],
];

/** Display units a setting may hold: 0 automatic, 1 none, thousands to trillions. */
const UNIT_VALUES = [0, 1, 1000, 1000000, 1000000000, 1000000000000];

/** A property's value as the layout takes it, or undefined when it is not one it can draw. */
function valueOf(v: any, kind: Kind): any {
    switch (kind) {
        case "bool":
            return typeof v === "boolean" ? v : undefined;
        case "color": {
            const c = v && v.solid ? v.solid.color : v;
            return typeof c === "string" && /^#[0-9A-Fa-f]{6}$/.test(c) ? c.toUpperCase() : undefined;
        }
        case "size":
            return typeof v === "number" && isFinite(v) && v >= 6 && v <= 60 ? v : undefined;
        case "family":
            return typeof v === "string" && Array.from(v).length <= 200 && !/[\u0000-\u001f\u007f-\u009f]/.test(v) ? v : undefined;
        case "units":
            return typeof v === "number" && UNIT_VALUES.indexOf(v) >= 0 ? v : undefined;
        case "decimals":
            return typeof v === "number" && isFinite(v) && v >= 0 && v <= 6 ? Math.floor(v) : undefined;
    }
    return undefined;
}

/** The text settings the format pane holds (`dataView.metadata.objects`), as `{part: {key: value}}`. */
export function textOf(objects: any): any {
    const out: any = {};
    if (!objects || typeof objects !== "object") {
        return out;
    }
    for (const [object, property, part, key, kind] of TEXT_PROPS) {
        const o = objects[object];
        if (!o || typeof o !== "object" || o[property] === undefined) {
            continue;
        }
        const v = valueOf(o[property], kind);
        if (v !== undefined) {
            out[part] = out[part] || {};
            out[part][key] = v;
        }
    }
    return out;
}

/** `own` (the format pane's settings) over `base` (the spec's), element by element. */
export function mergeText(base: any, own: any): any {
    const out: any = {};
    for (const src of [base || {}, own || {}]) {
        for (const part of Object.keys(src)) {
            out[part] = Object.assign({}, out[part] || {}, src[part]);
        }
    }
    return out;
}

/** What one text element shows in the pane: its settings as drawn. */
export interface TextShown { show: boolean; size: number; family: string; color: string; bold: boolean; italic: boolean; underline: boolean; units?: number; decimals?: number | null; }

/** The family shown when the settings give none (the visual's own stack starts with it). */
const DEFAULT_FAMILY = "Segoe UI";

function desc(objectName: string, propertyName: string): { objectName: string; propertyName: string } {
    return { objectName, propertyName };
}

/** The font row: family, size, bold, italic, underline (one FontControl). */
function fontRow(object: string, prefix: string, label: string, t: TextShown): Slice {
    const name = (p: string) => (prefix === "" ? p : prefix + p.charAt(0).toUpperCase() + p.slice(1));
    return {
        uid: `${object}_${name("font")}`,
        displayName: label,
        control: {
            type: "FontControl",
            properties: {
                fontFamily: { descriptor: desc(object, name("fontFamily")), value: t.family !== "" ? t.family : DEFAULT_FAMILY },
                fontSize: { descriptor: desc(object, name("fontSize")), value: t.size },
                bold: { descriptor: desc(object, name("bold")), value: t.bold },
                italic: { descriptor: desc(object, name("italic")), value: t.italic },
                underline: { descriptor: desc(object, name("underline")), value: t.underline },
            },
        },
    };
}

/** The display units drop-down (its items come from the property's `labelDisplayUnits` type). */
function unitsRow(object: string, label: string, value: number): Slice {
    return { uid: `${object}_labelDisplayUnits`, displayName: label, control: { type: "Dropdown", properties: { descriptor: desc(object, "labelDisplayUnits"), value } } };
}

/** The decimal places box (empty: automatic). */
function decimalsRow(object: string, label: string, value: number | null | undefined, auto: string): Slice {
    const properties: any = { descriptor: desc(object, "labelPrecision"), options: { minValue: { type: 0, value: 0 }, maxValue: { type: 1, value: 6 } }, placeholderText: auto };
    if (typeof value === "number") {
        properties.value = value;
    }
    return { uid: `${object}_labelPrecision`, displayName: label, control: { type: "NumUpDown", properties } };
}

/** A card with a show switch in its header. */
function switched(uid: string, name: string, show: boolean, groups: Group[]): Card {
    const c: any = card(uid, name, groups);
    c.topLevelToggle = { uid: `${uid}_show`, suppressDisplayName: true, control: { type: "ToggleSwitch", properties: { descriptor: desc(uid, "show"), value: show } } };
    c.revertToDefaultDescriptors = c.revertToDefaultDescriptors.concat([desc(uid, "show")]);
    return c as Card;
}

/**
 * The text settings' cards (Data labels, X axis, Y axis, Legend) for the elements in `shown`
 * (`labels`, `x`, `y`, `xTitle`, `yTitle`, `legend`); an element left out has no card or group.
 */
export function textCards(locale: string, shown: { [part: string]: TextShown }): Card[] {
    const L = (en: string, fr: string) => pick(locale, { en, fr });
    const font = L("Font", "Police");
    const ink = L("Color", "Couleur");
    const units = L("Display units", "Unités d'affichage");
    const decimals = L("Value decimal places", "Décimales des valeurs");
    const auto = L("Auto", "Auto");
    const cards: Card[] = [];
    const lb = shown.labels;
    if (lb) {
        cards.push(switched("labels", L("Data labels", "Étiquettes de données"), lb.show, [
            group("labels_values", L("Values", "Valeurs"), [
                fontRow("labels", "", font, lb),
                color("labels", "color", ink, lb.color),
                unitsRow("labels", units, lb.units || 0),
                decimalsRow("labels", decimals, lb.decimals, auto),
            ]),
        ]));
    }
    const axis = (uid: string, name: string, part: string, title: string, numbers: boolean) => {
        const t = shown[part];
        if (!t) {
            return;
        }
        const values: Slice[] = [fontRow(uid, "", font, t), color(uid, "labelColor", ink, t.color)];
        if (numbers) {
            values.push(unitsRow(uid, units, t.units || 0), decimalsRow(uid, decimals, t.decimals, auto));
        }
        const groups: Group[] = [group(`${uid}_values`, L("Values", "Valeurs"), values)];
        const tt = shown[title];
        if (tt) {
            const s: any = { uid: `${uid}_showAxisTitle`, displayName: L("Title", "Titre"), control: { type: "ToggleSwitch", properties: { descriptor: desc(uid, "showAxisTitle"), value: tt.show } } };
            groups.push(group(`${uid}_title`, L("Title", "Titre"), [s, fontRow(uid, "title", font, tt), color(uid, "titleColor", ink, tt.color)]));
        }
        cards.push(switched(uid, name, t.show, groups));
    };
    axis("categoryAxis", L("X axis", "Axe X"), "x", "xTitle", false);
    axis("valueAxis", L("Y axis", "Axe Y"), "y", "yTitle", true);
    const lg = shown.legend;
    if (lg) {
        cards.push(switched("legend", L("Legend", "Légende"), lg.show, [
            group("legend_text", L("Text", "Texte"), [fontRow("legend", "", font, lg), color("legend", "labelColor", ink, lg.color)]),
        ]));
    }
    return cards;
}
