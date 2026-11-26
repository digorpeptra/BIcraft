// BIcraft Waterfall+ (docs/formats/chart-pack.md "Waterfall+"): the `IVisual` class. The roles
// category, group, subtotal, target and values (up to eight measures); ./waterfall lays out the
// steps, subtotals, total and target. A step selects its row (with one measure); subtotals, the
// total and the target show their tooltip.
"use strict";

import powerbi from "powerbi-visuals-api";
import VisualConstructorOptions = powerbi.extensibility.visual.VisualConstructorOptions;
import IVisual = powerbi.extensibility.visual.IVisual;
import { Point, PackVisual, RoleColumn, cellOf, formatsOf, idOf, roleColumn, rowsOf, tipLine } from "./base";
import { Hit, Out } from "./kit";
import { MAX_MEASURES, MAX_ROWS, layoutWaterfall, pctText, signed } from "./waterfall";
import { objectValue } from "./runtime/data";
import { formatValue } from "./runtime/format";
import { Slice, card, color, group, model, number, toggle, withLook } from "./runtime/formatting";
import { pick } from "./runtime/strings";
import { TooltipItem } from "./runtime/tooltips";

const VALUE = { en: "Value", fr: "Valeur" };
const STEP = { en: "Step", fr: "Étape" };
const RUNNING = { en: "Running total", fr: "Cumul" };
const CHANGE = { en: "Change", fr: "Variation" };
const SUBTOTAL = { en: "Subtotal", fr: "Sous-total" };
const TARGET = { en: "Target", fr: "Cible" };
const VARIANCE = { en: "Variance", fr: "Écart" };
const ORIENTATIONS = [
    { value: "vertical", en: "Vertical", fr: "Vertical" },
    { value: "horizontal", en: "Horizontal", fr: "Horizontal" },
];
const COLOR_BY = [
    { value: "sign", en: "By sign", fr: "Par signe" },
    { value: "category", en: "By category", fr: "Par catégorie" },
    { value: "rule", en: "By rule", fr: "Par règle" },
];
const LABELS = [
    { value: "value", en: "Value", fr: "Valeur" },
    { value: "percent", en: "Percent", fr: "Pourcentage" },
    { value: "both", en: "Value and percent", fr: "Valeur et pourcentage" },
    { value: "none", en: "None", fr: "Aucune" },
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

function hasRole(c: any, role: string): boolean {
    return !!(c && c.source && c.source.roles && c.source.roles[role]);
}

/** Every value column of `role`, in the data view's order (at most MAX_MEASURES). */
function roleColumns(dv: any, role: string): RoleColumn[] {
    const vals: any[] = dv && dv.categorical && dv.categorical.values && typeof dv.categorical.values.length === "number" ? dv.categorical.values : [];
    const out: RoleColumn[] = [];
    for (let k = 0; k < vals.length && k < 64 && out.length < MAX_MEASURES; k++) {
        const c = vals[k];
        if (hasRole(c, role)) {
            const src = c.source || {};
            out.push({
                name: typeof src.displayName === "string" ? src.displayName : "",
                format: typeof src.format === "string" ? src.format : "",
                values: Array.isArray(c.values) ? c.values.slice(0, MAX_ROWS) : [],
                source: c,
            });
        }
    }
    return out;
}

export class Visual extends PackVisual implements IVisual {
    private cols: (RoleColumn | null)[] = [];
    private measures: RoleColumn[] = [];

    constructor(options: VisualConstructorOptions) {
        super(options, "bicraft-waterfall");
    }

    protected title(): string {
        return "BIcraft Waterfall+";
    }

    protected required() {
        return [{ role: "values", en: "Values", fr: "Valeurs" }];
    }

    protected input(dv: any): any {
        this.measures = roleColumns(dv, "values");
        this.cols = [roleColumn(dv, "category"), roleColumn(dv, "group"), roleColumn(dv, "subtotal"), roleColumn(dv, "target")];
        const all = this.cols.concat(this.measures);
        const input: any = {
            rows: rowsOf(all),
            formats: formatsOf(all),
            names: this.measures.map((m) => m.name),
            options: {
                orientation: choice(dv, "waterfall", "orientation", ORIENTATIONS),
                showTotal: objectValue(dv, "waterfall", "showTotal", true),
                subtotalEvery: objectValue(dv, "waterfall", "subtotalEvery", 0),
                showConnectors: objectValue(dv, "waterfall", "connectors", true),
                colorBy: choice(dv, "colors", "colorBy", COLOR_BY),
                increaseFill: objectValue(dv, "colors", "increase", ""),
                decreaseFill: objectValue(dv, "colors", "decrease", ""),
                totalFill: objectValue(dv, "colors", "total", ""),
                ruleValue: objectValue(dv, "colors", "ruleValue", 0),
                ruleAbove: objectValue(dv, "colors", "ruleAbove", ""),
                ruleBelow: objectValue(dv, "colors", "ruleBelow", ""),
                labelMode: choice(dv, "labels", "mode", LABELS),
                fontSize: objectValue(dv, "labels", "fontSize", NaN),
                targetFill: objectValue(dv, "target", "fill", ""),
            },
        };
        // another visual's selection highlighting this one: each row's highlighted parts of the measures
        const lit = this.measures.some((m) => m.source && Array.isArray(m.source.highlights));
        if (lit) {
            const n = input.rows.length;
            const hl: any[] = [];
            for (let i = 0; i < n; i++) {
                hl.push(this.measures.map((m) => (m.source && Array.isArray(m.source.highlights) ? cellOf(m.source.highlights[i]) : null)));
            }
            input.highlights = hl;
        }
        return input;
    }

    protected layout(input: any, w: number, h: number): Out {
        return layoutWaterfall(input, w, h);
    }

    protected point(hit: Hit): Point | null {
        const [category, groupCol, subtotal, target] = this.cols;
        const m = typeof hit.measure === "number" && hit.measure < this.measures.length ? this.measures[hit.measure] : this.measures[0];
        const f = m ? m.format : "";
        const value = (v: number) => formatValue(v, f, this.locale);
        const name = m && m.name !== "" ? m.name : pick(this.locale, VALUE);
        let tip: TooltipItem[] = [];
        if (hit.kind === "step") {
            if (hit.row >= 0) {
                tip = tip.concat(tipLine(category, hit.row, this.locale), tipLine(groupCol, hit.row, this.locale));
            } else {
                tip.push({ displayName: pick(this.locale, STEP), value: String(hit.label) });
            }
            tip.push({ displayName: name, value: signed(hit.value, value(hit.value)) });
            tip.push({ displayName: pick(this.locale, RUNNING), value: value(hit.end) });
            if (hit.start !== 0) {
                tip.push({ displayName: pick(this.locale, CHANGE), value: pctText(hit.value / Math.abs(hit.start), this.locale) });
            }
        } else if (hit.kind === "sub" || hit.kind === "total") {
            tip.push({ displayName: hit.kind === "total" ? "Total" : pick(this.locale, SUBTOTAL), value: String(hit.label) });
            const first = this.measures[0];
            tip.push({ displayName: first && first.name !== "" ? first.name : pick(this.locale, VALUE), value: value(hit.value) });
        } else {
            const t = typeof hit.target === "number" ? hit.target : hit.value;
            const diff = hit.total - t;
            let s = signed(diff, value(diff));
            if (t !== 0) {
                s += " (" + pctText(diff / Math.abs(t), this.locale) + ")";
            }
            tip.push({ displayName: pick(this.locale, TARGET), value: value(t) });
            tip.push({ displayName: "Total", value: value(hit.total) });
            tip.push({ displayName: pick(this.locale, VARIANCE), value: s });
        }
        return { id: hit.row >= 0 ? idOf(this.host, [category, groupCol, subtotal], hit.row) : undefined, tip };
    }

    public getFormattingModel(): powerbi.visuals.FormattingModel {
        const dv = this.dataView;
        const fr = this.locale.toLowerCase().startsWith("fr");
        const items = (l: { value: string; en: string; fr: string }[]) => l.map((c) => ({ value: c.value, displayName: fr ? c.fr : c.en }));
        const waterfall = card("waterfall", fr ? "Cascade" : "Waterfall", [
            group("waterfall_group", fr ? "Cascade" : "Waterfall", [
                dropdown("waterfall", "orientation", "Orientation", choice(dv, "waterfall", "orientation", ORIENTATIONS), items(ORIENTATIONS)),
                toggle("waterfall", "showTotal", fr ? "Afficher le total" : "Show the total", objectValue(dv, "waterfall", "showTotal", true)),
                number("waterfall", "subtotalEvery", fr ? "Sous-total toutes les N étapes (0 : aucun)" : "Subtotal every N steps (0: none)", objectValue(dv, "waterfall", "subtotalEvery", 0), 0, 100),
                toggle("waterfall", "connectors", fr ? "Connecteurs" : "Connectors", objectValue(dv, "waterfall", "connectors", true)),
            ]),
        ]);
        const colors = card("colors", fr ? "Couleurs" : "Colors", [
            group("colors_group", fr ? "Couleurs" : "Colors", [
                dropdown("colors", "colorBy", fr ? "Couleur selon" : "Color by", choice(dv, "colors", "colorBy", COLOR_BY), items(COLOR_BY)),
                color("colors", "increase", fr ? "Hausse" : "Increase", objectValue(dv, "colors", "increase", "#1F9D55")),
                color("colors", "decrease", fr ? "Baisse" : "Decrease", objectValue(dv, "colors", "decrease", "#D64545")),
                color("colors", "total", "Total", objectValue(dv, "colors", "total", this.look().palette[0] || "#4A5BE8")),
                number("colors", "ruleValue", fr ? "Seuil de la règle" : "Rule threshold", objectValue(dv, "colors", "ruleValue", 0)),
                color("colors", "ruleAbove", fr ? "Au seuil ou au-dessus" : "At or above the threshold", objectValue(dv, "colors", "ruleAbove", "#1C8FCC")),
                color("colors", "ruleBelow", fr ? "Sous le seuil" : "Below the threshold", objectValue(dv, "colors", "ruleBelow", "#BF8700")),
            ]),
        ]);
        const labels = card("labels", fr ? "Étiquettes" : "Labels", [
            group("labels_group", fr ? "Étiquettes" : "Labels", [
                dropdown("labels", "mode", fr ? "Afficher" : "Show", choice(dv, "labels", "mode", LABELS), items(LABELS)),
                number("labels", "fontSize", fr ? "Taille du texte" : "Text size", objectValue(dv, "labels", "fontSize", this.look().value), 8, 20),
            ]),
        ]);
        const target = card("target", fr ? "Cible" : "Target", [
            group("target_group", fr ? "Cible" : "Target", [color("target", "fill", fr ? "Couleur de la cible" : "Target color", objectValue(dv, "target", "fill", this.look().label))]),
        ]);
        return model(withLook([waterfall, colors, labels, target], this.locale, this.look(), this.paletteOption())) as any;
    }
}

