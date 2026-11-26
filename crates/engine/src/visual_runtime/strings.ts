// BIcraft's visual runtime (docs/community-visuals.md "Runtime"): the strings its visuals show,
// in English and French, chosen from the host's locale (`host.locale`, a BCP 47 tag such as
// "fr-FR"). Each French string is translated from the meaning of the English one.
"use strict";

export type Lang = "en" | "fr";

const EN: { [key: string]: string } = {
    blank: "(Blank)",
    true: "True",
    false: "False",
    landingAdd: "Add fields to {0} to draw this visual.",
    and: " and ",
    points: "{0}: {1} data points",
};

const FR: { [key: string]: string } = {
    blank: "(Vide)",
    true: "Vrai",
    false: "Faux",
    landingAdd: "Ajoutez des champs dans {0} pour dessiner ce visuel.",
    and: " et ",
    points: "{0} : {1} points de données",
};

/** The catalogue's language for a locale tag: French for `fr` and `fr-*`, else English. */
export function langOf(locale: string | undefined | null): Lang {
    const tag = typeof locale === "string" ? locale.trim().toLowerCase() : "";
    return tag === "fr" || tag.startsWith("fr-") || tag.startsWith("fr_") ? "fr" : "en";
}

/** The string `key` in the locale's language, `{0}`, `{1}`… replaced by `args`. */
export function t(locale: string | undefined | null, key: string, ...args: (string | number)[]): string {
    const table = langOf(locale) === "fr" ? FR : EN;
    const text = Object.prototype.hasOwnProperty.call(table, key) ? table[key] : (Object.prototype.hasOwnProperty.call(EN, key) ? EN[key] : key);
    return text.replace(/\{(\d)\}/g, (m: string, d: string) => {
        const i = Number(d);
        return i < args.length ? String(args[i]) : m;
    });
}

/** Names joined as a sentence: "A", "A and B", "A, B and C". */
export function joinNames(locale: string | undefined | null, names: string[]): string {
    if (names.length <= 1) {
        return names.join("");
    }
    return names.slice(0, -1).join(", ") + t(locale, "and") + names[names.length - 1];
}

/** A visual's own text in the locale's language. */
export function pick(locale: string | undefined | null, text: { en: string; fr: string }): string {
    return langOf(locale) === "fr" ? text.fr : text.en;
}
