// BIcraft's visual runtime (docs/community-visuals.md "Runtime"): the format pane's model
// (`IVisual.getFormattingModel`), built from cards, groups and slices with the shapes of the
// public `powerbi-visuals-api` formatting model types (`FormattingModel`, `FormattingCard`,
// `FormattingGroup`, `SimpleVisualFormattingSlice` and the `FormattingComponent` names).
"use strict";

import { pick } from "./strings";
import { Look, PALETTES, PRESETS } from "./style";

export type Slice = { [k: string]: any };
export type Group = { uid: string; displayName: string; slices: Slice[] };
export type Card = { uid: string; displayName: string; groups: Group[]; revertToDefaultDescriptors: any[] };

function descriptor(objectName: string, propertyName: string): { objectName: string; propertyName: string } {
    return { objectName, propertyName };
}

function slice(type: string, objectName: string, propertyName: string, displayName: string, value: any, extra?: { [k: string]: any }): Slice {
    const properties: { [k: string]: any } = { descriptor: descriptor(objectName, propertyName), value };
    if (extra) {
        for (const k of Object.keys(extra)) {
            properties[k] = extra[k];
        }
    }
    return { uid: `${objectName}_${propertyName}`, displayName, control: { type, properties } };
}

/** An on/off switch for a `bool` property. */
export function toggle(objectName: string, propertyName: string, displayName: string, value: boolean): Slice {
    return slice("ToggleSwitch", objectName, propertyName, displayName, !!value);
}

/** A colour picker for a `fill` property (`value` a `#RRGGBB` text). */
export function color(objectName: string, propertyName: string, displayName: string, value: string): Slice {
    return slice("ColorPicker", objectName, propertyName, displayName, { value });
}

/** A number box for a `numeric` property, optionally bounded. */
export function number(objectName: string, propertyName: string, displayName: string, value: number, min?: number, max?: number): Slice {
    const options: { [k: string]: any } = {};
    // ValidatorType: Min = 0, Max = 1 (the API's const enum, written as numbers)
    if (typeof min === "number") {
        options.minValue = { type: 0, value: min };
    }
    if (typeof max === "number") {
        options.maxValue = { type: 1, value: max };
    }
    return slice("NumUpDown", objectName, propertyName, displayName, value, Object.keys(options).length > 0 ? { options } : undefined);
}

/** A one-line text box for a `text` property. */
export function textInput(objectName: string, propertyName: string, displayName: string, value: string, placeholder?: string): Slice {
    return slice("TextInput", objectName, propertyName, displayName, value, { placeholder: placeholder || "" });
}

/** A text area for a long `text` property. */
export function textArea(objectName: string, propertyName: string, displayName: string, value: string, placeholder?: string): Slice {
    return slice("TextArea", objectName, propertyName, displayName, value, placeholder ? { placeholder } : undefined);
}

/** A drop-down list for an `enumeration` property: `items` its values and names, `value` the current one. */
export function dropdown(objectName: string, propertyName: string, displayName: string, value: string, items: { value: string; displayName: string }[]): Slice {
    const cur = items.filter((i) => i.value === value)[0] || items[0];
    return slice("Dropdown", objectName, propertyName, displayName, cur, { items });
}

const PRESET_NAMES: { [k: string]: { en: string; fr: string } } = {
    modern: { en: "Modern", fr: "Moderne" },
    minimal: { en: "Minimal", fr: "Sobre" },
    bold: { en: "Bold", fr: "Affirmé" },
};
const PALETTE_NAMES: { [k: string]: { en: string; fr: string } } = {
    theme: { en: "Report theme", fr: "Thème du rapport" },
    bicraft: { en: "BIcraft colors", fr: "Couleurs BIcraft" },
};

/**
 * The Style card's look settings (docs/formats/visual-look.md), in the format object `style`:
 * the preset, the palette and the switches, showing the look `k` drawn with (`palette` the
 * palette option as set: `theme` or `bicraft`).
 */
export function lookGroup(locale: string, k: Look, palette: string): Group {
    const L = (en: string, fr: string) => pick(locale, { en, fr });
    return group("look_group", L("Look", "Apparence"), [
        dropdown("style", "preset", L("Preset", "Préréglage"), k.preset, PRESETS.map((v) => ({ value: v, displayName: pick(locale, PRESET_NAMES[v]) }))),
        dropdown("style", "palette", L("Colors", "Couleurs"), palette === "bicraft" ? "bicraft" : "theme", PALETTES.map((v) => ({ value: v, displayName: pick(locale, PALETTE_NAMES[v]) }))),
        toggle("style", "rounded", L("Rounded corners", "Coins arrondis"), k.rounded),
        toggle("style", "gradient", L("Gradient", "Dégradé"), k.gradient),
        toggle("style", "shadow", L("Shadow", "Ombre"), k.shadow),
        toggle("style", "animation", L("Animation", "Animation"), k.motion),
        toggle("style", "card", L("Card background", "Fond en carte"), k.card),
        toggle("style", "pills", L("Value chips", "Pastilles de valeur"), k.pills),
    ]);
}

/**
 * `cards` with the look settings: in the visual's own `style` card when it has one (after its
 * settings), else in a new Style card at the end.
 */
export function withLook(cards: Card[], locale: string, k: Look, palette: string): Card[] {
    const g = lookGroup(locale, k, palette);
    const own = cards.filter((c) => c.uid === "style")[0];
    if (own) {
        return cards.map((c) => (c === own ? card(c.uid, c.displayName, c.groups.concat([g])) : c));
    }
    return cards.concat([card("style", pick(locale, { en: "Style", fr: "Style" }), [g])]);
}

export function group(uid: string, displayName: string, slices: Slice[]): Group {
    return { uid, displayName, slices };
}

/** A card; with `resettable` (the default), its reset button restores every property its slices write. */
export function card(uid: string, displayName: string, groups: Group[], resettable?: boolean): Card {
    const revert: any[] = [];
    for (const g of resettable === false ? [] : groups) {
        for (const s of g.slices) {
            const props = s.control && s.control.properties ? s.control.properties : undefined;
            if (props && props.descriptor) {
                revert.push(props.descriptor);
            } else if (props) {
                // a composite slice (the font row): each of its parts
                for (const k of Object.keys(props)) {
                    if (props[k] && props[k].descriptor) {
                        revert.push(props[k].descriptor);
                    }
                }
            }
        }
    }
    return { uid, displayName, groups, revertToDefaultDescriptors: revert };
}

/** The model the format pane reads. */
export function model(cards: Card[]): { cards: Card[] } {
    return { cards };
}
