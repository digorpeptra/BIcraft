// BIcraft Calendar Heatmap (docs/formats/chart-pack.md "Calendar heatmap"): the `IVisual`
// class. The roles date and value; ./calendar lays out one cell per day. A day selects its date.
"use strict";

import powerbi from "powerbi-visuals-api";
import VisualConstructorOptions = powerbi.extensibility.visual.VisualConstructorOptions;
import IVisual = powerbi.extensibility.visual.IVisual;
import { Point, PackVisual, RoleColumn, formatsOf, idOf, roleColumn, rowsOf, tipLine } from "./base";
import { layoutCalendar } from "./calendar";
import { Hit, Out } from "./kit";
import { objectValue } from "./runtime/data";
import { formatValue } from "./runtime/format";
import { card, color, group, model, toggle, withLook } from "./runtime/formatting";
import { color as colorOf, minimal, mix } from "./runtime/style";

export class Visual extends PackVisual implements IVisual {
    private cols: (RoleColumn | null)[] = [];

    constructor(options: VisualConstructorOptions) {
        super(options, "bicraft-calendar");
    }

    protected measureRole(): string | null {
        return "value";
    }

    protected title(): string {
        return "BIcraft Calendar Heatmap";
    }

    protected required() {
        return [
            { role: "date", en: "Date", fr: "Date" },
            { role: "value", en: "Value", fr: "Valeur" },
        ];
    }

    protected input(dv: any): any {
        this.cols = [roleColumn(dv, "date"), roleColumn(dv, "value")];
        return {
            rows: rowsOf(this.cols),
            formats: formatsOf(this.cols),
            options: {
                // no colour set: the layout takes the look's
                low: objectValue(dv, "colors", "low", ""),
                high: objectValue(dv, "colors", "high", ""),
                empty: objectValue(dv, "colors", "empty", ""),
                showLegend: objectValue(dv, "legend", "show", true),
                monthsOnly: objectValue(dv, "range", "months", false),
                sundayFirst: objectValue(dv, "range", "sundayFirst", false),
            },
        };
    }

    protected layout(input: any, w: number, h: number): Out {
        return layoutCalendar(input, w, h);
    }

    protected point(hit: Hit): Point | null {
        const [date, value] = this.cols;
        const v = value ? [{ displayName: value.name, value: formatValue(hit.value, value.format, this.locale) }] : [];
        return { id: idOf(this.host, [date], hit.row), tip: tipLine(date, hit.row, this.locale).concat(v) };
    }

    public getFormattingModel(): powerbi.visuals.FormattingModel {
        const dv = this.dataView;
        const fr = this.locale.toLowerCase().startsWith("fr");
        const k = this.look();
        const m = minimal(k);
        const high = objectValue(dv, "colors", "high", m ? "#1F6F8B" : colorOf(k, 0));
        const colors = card("colors", fr ? "Couleurs" : "Colors", [
            group("colors_group", fr ? "Couleurs" : "Colors", [
                color("colors", "low", fr ? "Valeur la plus basse" : "Lowest value", objectValue(dv, "colors", "low", m ? "#D6E6EE" : mix(high, "#FFFFFF", 0.86))),
                color("colors", "high", fr ? "Valeur la plus haute" : "Highest value", high),
                color("colors", "empty", fr ? "Jour sans donnée" : "Day without data", objectValue(dv, "colors", "empty", m ? "#EFEFEF" : k.empty)),
            ]),
        ]);
        const legend = card("legend", fr ? "Légende" : "Legend", [
            group("legend_group", fr ? "Légende" : "Legend", [toggle("legend", "show", fr ? "Afficher" : "Show", objectValue(dv, "legend", "show", true))]),
        ]);
        const range = card("range", fr ? "Dates" : "Dates", [
            group("range_group", fr ? "Dates" : "Dates", [
                toggle("range", "months", fr ? "Seulement les mois avec des données" : "Only the months with data", objectValue(dv, "range", "months", false)),
                toggle("range", "sundayFirst", fr ? "Semaines commençant le dimanche" : "Weeks start on Sunday", objectValue(dv, "range", "sundayFirst", false)),
            ]),
        ]);
        return model(withLook([colors, legend, range], this.locale, k, this.paletteOption())) as any;
    }
}
