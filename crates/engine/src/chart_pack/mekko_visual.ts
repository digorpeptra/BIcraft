// BIcraft Marimekko (docs/formats/chart-pack.md "Marimekko"): the `IVisual` class. The roles
// category, series and value; ./mekko lays out the variable-width 100 % stacked columns. A
// segment selects its category and series.
"use strict";

import powerbi from "powerbi-visuals-api";
import VisualConstructorOptions = powerbi.extensibility.visual.VisualConstructorOptions;
import IVisual = powerbi.extensibility.visual.IVisual;
import { Point, PackVisual, RoleColumn, formatsOf, idOf, roleColumn, rowsOf, tipLine } from "./base";
import { Hit, Out } from "./kit";
import { SERIES_COLORS, layoutMekko } from "./mekko";
import { objectValue } from "./runtime/data";
import { formatNumber } from "./runtime/format";
import { card, color, group, model, number, toggle, withLook } from "./runtime/formatting";
import { color as colorOf } from "./runtime/style";
import { pick } from "./runtime/strings";

const SHARE = { en: "Share of the category", fr: "Part de la catégorie" };

export class Visual extends PackVisual implements IVisual {
    private cols: (RoleColumn | null)[] = [];

    constructor(options: VisualConstructorOptions) {
        super(options, "bicraft-mekko");
    }

    protected measureRole(): string | null {
        return "value";
    }

    protected title(): string {
        return "BIcraft Marimekko";
    }

    protected required() {
        return [
            { role: "category", en: "Category", fr: "Catégorie" },
            { role: "value", en: "Value", fr: "Valeur" },
        ];
    }

    protected input(dv: any): any {
        this.cols = [roleColumn(dv, "category"), roleColumn(dv, "series"), roleColumn(dv, "value")];
        const options: any = {
            showLabels: objectValue(dv, "labels", "show", true),
            // unset: the look's text size, as report.render draws it
            fontSize: objectValue(dv, "labels", "fontSize", NaN),
            showLegend: objectValue(dv, "legend", "show", true),
            sortColumns: objectValue(dv, "columns", "sort", false),
            widthAxis: objectValue(dv, "columns", "widthAxis", false),
        };
        // no colour set: the series take the look's palette
        for (let i = 1; i <= SERIES_COLORS; i++) {
            options["color" + i] = objectValue(dv, "colors", "series" + i, "");
        }
        return { rows: rowsOf(this.cols), formats: formatsOf(this.cols), options };
    }

    protected layout(input: any, w: number, h: number): Out {
        return layoutMekko(input, w, h);
    }

    protected point(hit: Hit): Point | null {
        const [category, series, value] = this.cols;
        const share = { displayName: pick(this.locale, SHARE), value: formatNumber(hit.share, "0.0%", this.locale) };
        return {
            id: idOf(this.host, [category, series], hit.row),
            tip: tipLine(category, hit.row, this.locale).concat(tipLine(series, hit.row, this.locale), tipLine(value, hit.row, this.locale), [share]),
        };
    }

    public getFormattingModel(): powerbi.visuals.FormattingModel {
        const dv = this.dataView;
        const fr = this.locale.toLowerCase().startsWith("fr");
        const labels = card("labels", fr ? "Étiquettes de données" : "Data labels", [
            group("labels_group", fr ? "Étiquettes" : "Labels", [
                toggle("labels", "show", fr ? "Afficher" : "Show", objectValue(dv, "labels", "show", true)),
                number("labels", "fontSize", fr ? "Taille du texte" : "Text size", objectValue(dv, "labels", "fontSize", this.look().value), 8, 20),
            ]),
        ]);
        const legend = card("legend", fr ? "Légende" : "Legend", [
            group("legend_group", fr ? "Légende" : "Legend", [toggle("legend", "show", fr ? "Afficher" : "Show", objectValue(dv, "legend", "show", true))]),
        ]);
        const columns = card("columns", fr ? "Colonnes" : "Columns", [
            group("columns_group", fr ? "Colonnes" : "Columns", [
                toggle("columns", "sort", fr ? "Les plus larges d'abord" : "Widest first", objectValue(dv, "columns", "sort", false)),
                toggle("columns", "widthAxis", fr ? "Axe des largeurs" : "Width axis", objectValue(dv, "columns", "widthAxis", false)),
            ]),
        ]);
        const k = this.look();
        const swatches: any[] = [];
        for (let i = 1; i <= SERIES_COLORS; i++) {
            swatches.push(color("colors", "series" + i, (fr ? "Série " : "Series ") + i, objectValue(dv, "colors", "series" + i, colorOf(k, i - 1))));
        }
        const colors = card("colors", fr ? "Couleurs" : "Colors", [group("colors_group", fr ? "Couleurs" : "Colors", swatches)]);
        return model(withLook([colors, labels, legend, columns], this.locale, k, this.paletteOption())) as any;
    }
}
