// BIcraft Org Chart (docs/formats/chart-pack.md "Organization chart"): the `IVisual` class. The
// roles id and parent (and label, value) give the hierarchy; ./org lays it out. A node's
// round button collapses or expands it; the list of toggled nodes is persisted in the
// visual's `tree.toggled` property, so BIcraft's renderer draws the same tree.
"use strict";

import powerbi from "powerbi-visuals-api";
import VisualConstructorOptions = powerbi.extensibility.visual.VisualConstructorOptions;
import IVisual = powerbi.extensibility.visual.IVisual;
import { Point, PackVisual, RoleColumn, formatsOf, idOf, roleColumn, rowsOf, tipLine } from "./base";
import { Hit, Out } from "./kit";
import { MAX_TOGGLED, layoutOrg } from "./org";
import { objectValue } from "./runtime/data";
import { card, color, group, model, number, toggle, withLook } from "./runtime/formatting";
import { color as colorOf, minimal } from "./runtime/style";
import { pick } from "./runtime/strings";

const EXPAND = { en: "Expand", fr: "Développer" };
const COLLAPSE = { en: "Collapse", fr: "Réduire" };

/** The toggled ids of a `tree.toggled` text (a JSON list). */
export function toggledOf(text: string): string[] {
    try {
        const v = text.length <= 65536 ? JSON.parse(text) : null;
        return Array.isArray(v) ? v.filter((x: any) => typeof x === "string").slice(0, MAX_TOGGLED) : [];
    } catch (e) {
        return [];
    }
}

export class Visual extends PackVisual implements IVisual {
    private cols: (RoleColumn | null)[] = [];
    private labels: { [id: string]: string } = {};
    /** The toggled list after a click, until the host hands back another one. */
    private local: string[] | null = null;
    private seen = "";

    constructor(options: VisualConstructorOptions) {
        super(options, "bicraft-org");
    }

    protected measureRole(): string | null {
        return "value";
    }

    protected title(): string {
        return "BIcraft Org Chart";
    }

    protected required() {
        return [
            { role: "id", en: "Id", fr: "Id" },
            { role: "parent", en: "Parent id", fr: "Id du parent" },
        ];
    }

    protected changed(): void {
        const text = objectValue(this.dataView, "tree", "toggled", "");
        if (text !== this.seen) {
            this.seen = text;
            this.local = null;
        }
    }

    private toggled(): string[] {
        return this.local !== null ? this.local : toggledOf(this.seen);
    }

    protected input(dv: any): any {
        this.cols = ["id", "parent", "label", "value"].map((r) => roleColumn(dv, r));
        return {
            rows: rowsOf(this.cols),
            formats: formatsOf(this.cols),
            options: {
                // no colour set: the layout takes the look's
                color: objectValue(dv, "nodes", "fill", ""),
                textColor: objectValue(dv, "nodes", "fontColor", ""),
                // unset: the look's text size, as report.render draws it
                fontSize: objectValue(dv, "nodes", "fontSize", NaN),
                showValues: objectValue(dv, "nodes", "showValues", true),
                levels: objectValue(dv, "tree", "levels", 0),
                compact: objectValue(dv, "tree", "compact", true),
                leftToRight: objectValue(dv, "tree", "leftToRight", false),
                toggled: this.toggled(),
            },
        };
    }

    protected layout(input: any, w: number, h: number): Out {
        const out = layoutOrg(input, w, h);
        this.labels = {};
        for (const hit of out.hits) {
            if (!hit.toggle) {
                const label = this.cols[2] ? tipLine(this.cols[2], hit.row, this.locale) : [];
                this.labels[hit.id] = label.length > 0 ? label[0].value : String(hit.id);
            }
        }
        return out;
    }

    protected point(hit: Hit): Point | null {
        if (hit.toggle) {
            return null;
        }
        const [id, parent, label, value] = this.cols;
        return {
            id: idOf(this.host, [id], hit.row),
            tip: tipLine(label, hit.row, this.locale).concat(tipLine(id, hit.row, this.locale), tipLine(parent, hit.row, this.locale), tipLine(value, hit.row, this.locale)),
        };
    }

    protected wired(svg: Element, out: Out, drawn: (Element | null)[]): void {
        const list = this.toggled();
        for (const hit of out.hits) {
            const el = hit.toggle && hit.prim < drawn.length ? drawn[hit.prim] : null;
            if (!el) {
                continue;
            }
            const sign = hit.prim + 1 < drawn.length ? drawn[hit.prim + 1] : null;
            const open = !!sign && sign.textContent === "−";
            el.setAttribute("tabindex", "0");
            el.setAttribute("role", "button");
            el.setAttribute("aria-expanded", open ? "true" : "false");
            el.setAttribute("aria-label", `${pick(this.locale, open ? COLLAPSE : EXPAND)} ${this.labels[hit.id] || hit.id}`);
            el.setAttribute("style", "cursor: pointer");
            const id = String(hit.id);
            const flip = (e: Event) => {
                e.stopPropagation();
                e.preventDefault();
                const next = list.indexOf(id) >= 0 ? list.filter((x) => x !== id) : list.concat([id]).slice(-MAX_TOGGLED);
                this.local = next;
                this.persist(next);
                this.draw();
            };
            el.addEventListener("click", flip);
            el.addEventListener("keydown", (e: Event) => {
                const k = (e as KeyboardEvent).key;
                if (k === "Enter" || k === " ") {
                    flip(e);
                }
            });
        }
    }

    private persist(list: string[]): void {
        try {
            if (this.host && typeof this.host.persistProperties === "function") {
                this.host.persistProperties({ merge: [{ objectName: "tree", selector: null, properties: { toggled: JSON.stringify(list) } }] });
            }
        } catch (e) {
            // the host keeps no state in this mode: the tree stays toggled until the next update
        }
    }

    public getFormattingModel(): powerbi.visuals.FormattingModel {
        const dv = this.dataView;
        const fr = this.locale.toLowerCase().startsWith("fr");
        const k = this.look();
        const nodes = card("nodes", fr ? "Nœuds" : "Nodes", [
            group("nodes_group", fr ? "Nœuds" : "Nodes", [
                color("nodes", "fill", fr ? "Couleur" : "Color", objectValue(dv, "nodes", "fill", colorOf(k, 0))),
                color("nodes", "fontColor", fr ? "Couleur du texte" : "Text color", objectValue(dv, "nodes", "fontColor", minimal(k) ? "#FFFFFF" : k.label)),
                number("nodes", "fontSize", fr ? "Taille du texte" : "Text size", objectValue(dv, "nodes", "fontSize", k.name), 8, 24),
                toggle("nodes", "showValues", fr ? "Afficher les valeurs" : "Show values", objectValue(dv, "nodes", "showValues", true)),
            ]),
        ]);
        // the reset button would also forget the collapsed nodes: the card resets the levels only
        const tree = card("tree", fr ? "Arbre" : "Tree", [
            group("tree_group", fr ? "Arbre" : "Tree", [
                number("tree", "levels", fr ? "Niveaux ouverts (0 : tous)" : "Open levels (0: all)", objectValue(dv, "tree", "levels", 0), 0, 20),
                toggle("tree", "compact", fr ? "Arbre compact" : "Compact tree", objectValue(dv, "tree", "compact", true)),
                toggle("tree", "leftToRight", fr ? "De gauche à droite" : "Left to right", objectValue(dv, "tree", "leftToRight", false)),
            ]),
        ]);
        return model(withLook([nodes, tree], this.locale, k, this.paletteOption())) as any;
    }
}
