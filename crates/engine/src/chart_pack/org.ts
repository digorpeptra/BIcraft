// BIcraft Org Chart (docs/formats/chart-pack.md "Organization chart"): a hierarchy from id and
// parent id columns drawn as a compact tidy tree, top-down or left to right (or with the leaves in equal slots),
// each node a box with its label (and value), collapsible. The TypeScript twin of crates/render/src/chart_pack/org.rs.
"use strict";

import { Out, PAD, Prim, cellLabel, cellNum, cellText, circle, fit, fmtAuto, highlighted, hlNum, line, lookOfInput, optBool, optColor, optNum, optTexts, optionsOf, rect, rowsOf, text } from "./kit";
import { accent, bold, card, color as colorOf, dot, gradTop, minimal, mix, onColor, pushCard, textW, valueWeight } from "./runtime/style";

/** Most nodes read. */
export const MAX_NODES = 1000;
/** Most ids kept in the toggled list. */
export const MAX_TOGGLED = 500;

interface OrgNode {
    id: string;
    label: string;
    value: number | null;
    row: number;
    parent: number;
    kids: number[];
    tree: number[];
    depth: number;
    shown: number[];
    x: number;
}

/**
 * Places the subtrees of `list` left to right, each as far left as its contour allows: one unit
 * between any two nodes of a level. `lc` and `rc` are each subtree's left and right contours
 * (by depth below its root, relative to the root); `off` gets each root's place, relative to the
 * first. Returns the merged contours.
 */
function place(list: number[], lc: number[][], rc: number[][], off: number[]): number[][] {
    const left: number[] = [];
    const right: number[] = [];
    for (let i = 0; i < list.length; i++) {
        const k = list[i];
        const l = lc[k];
        const r = rc[k];
        let s = 0;
        if (i > 0) {
            const m = Math.min(right.length, l.length);
            for (let d = 0; d < m; d++) {
                s = d === 0 ? right[0] + 1 - l[0] : Math.max(s, right[d] + 1 - l[d]);
            }
        }
        off[k] = s;
        for (let d = 0; d < r.length; d++) {
            if (d < right.length) {
                right[d] = s + r[d];
            } else {
                right.push(s + r[d]);
            }
        }
        for (let d = left.length; d < l.length; d++) {
            left.push(s + l[d]);
        }
    }
    return [left, right];
}

/**
 * The compact tidy tree: children before parents, each node's children placed by `place` and
 * the node at the middle of its first and last child, then the roots placed the same way. Sets
 * every shown node's `x` (the leftmost 0) and returns the width in units.
 */
function tidy(nodes: OrgNode[], vis: number[], roots: number[]): number {
    const lc: number[][] = nodes.map(() => []);
    const rc: number[][] = nodes.map(() => []);
    const off: number[] = nodes.map(() => 0);
    for (let j = vis.length - 1; j >= 0; j--) {
        const n = vis[j];
        const kids = nodes[n].shown;
        if (kids.length === 0) {
            lc[n] = [0];
            rc[n] = [0];
            continue;
        }
        const [l, r] = place(kids, lc, rc, off);
        const mid = (off[kids[0]] + off[kids[kids.length - 1]]) / 2;
        for (const k of kids) {
            off[k] -= mid;
        }
        lc[n] = [0].concat(l.map((v) => v - mid));
        rc[n] = [0].concat(r.map((v) => v - mid));
    }
    place(roots, lc, rc, off);
    for (const n of roots) {
        nodes[n].x = off[n];
    }
    for (const n of vis) {
        for (const k of nodes[n].shown) {
            nodes[k].x = nodes[n].x + off[k];
        }
    }
    let lo = Infinity;
    let hi = -Infinity;
    for (const n of vis) {
        lo = Math.min(lo, nodes[n].x);
        hi = Math.max(hi, nodes[n].x);
    }
    for (const n of vis) {
        nodes[n].x -= lo;
    }
    return hi - lo + 1;
}

/** Lays out `{rows: [[id, parent, label, value]], options}` in a `w`×`h` visual. */
export function layoutOrg(input: any, w: number, h: number): Out {
    const o = optionsOf(input);
    const lk = lookOfInput(input);
    const own = optColor(o, "color", "");
    const color0 = own !== "" ? own : colorOf(lk, 0);
    // Modern and Bold give each branch under a root its own colour, unless one colour is set
    const byBranch = own === "" && !minimal(lk);
    // Minimal: a solid box with white text; Modern: a tinted card with an accent strip and the
    // look's inks; Bold: a solid shaded card, its ink white or dark as reads best
    const modern = !minimal(lk) && !bold(lk);
    const tint = lk.night ? lk.surface : "#FFFFFF";
    const paint = (color: string) => {
        const grad = bold(lk) ? gradTop(lk, color) : "";
        const autoInk = minimal(lk) ? "#FFFFFF" : modern ? lk.label : onColor(lk, grad === "" ? color : grad, color);
        const ink = optColor(o, "textColor", autoInk);
        return {
            color,
            fill: modern ? mix(color, tint, 0.88) : color,
            edge: modern ? mix(color, tint, 0.6) : mix(color, "#000000", 0.2),
            grad,
            ink,
            valueInk: modern && optColor(o, "textColor", "") === "" ? lk.text : ink,
        };
    };
    const labelWeight = minimal(lk) ? 400 : valueWeight(lk);
    const link = minimal(lk) ? 1 : 1.5;
    const size = optNum(o, "fontSize", 8, 24, lk.name);
    const showValues = optBool(o, "showValues", true);
    const levels = Math.floor(optNum(o, "levels", 0, 20, 0));
    const toggled = optTexts(o, "toggled", MAX_TOGGLED);
    const compact = optBool(o, "compact", true);
    const ltr = optBool(o, "leftToRight", false);
    const out: Out = { prims: [], hits: [] };
    const nodes: OrgNode[] = [];
    const index = new Map<string, number>();
    const parents: string[] = [];
    const rows = rowsOf(input, MAX_NODES);
    for (let i = 0; i < rows.length; i++) {
        const r = rows[i];
        const id = cellText(r[0]);
        if (id === "" || index.has(id)) {
            continue;
        }
        const label = cellLabel(input, 2, r[2]);
        index.set(id, nodes.length);
        parents.push(cellText(r[1]));
        nodes.push({ id, label: label === "" ? cellLabel(input, 0, r[0]) : label, value: cellNum(r[3]), row: i, parent: -1, kids: [], tree: [], depth: 0, shown: [], x: 0 });
    }
    if (nodes.length === 0) {
        return out;
    }
    for (let i = 0; i < nodes.length; i++) {
        const p = index.get(parents[i]);
        if (p !== undefined && p !== i) {
            nodes[i].parent = p;
            nodes[p].kids.push(i);
        }
    }
    // the tree: from each root (no known parent) in order, then from each node a cycle kept
    // unreached; a node's tree children are its children not reached yet
    const seen: boolean[] = nodes.map(() => false);
    const roots: number[] = [];
    const order: number[] = [];
    for (let pass = 0; pass < 2; pass++) {
        for (let i = 0; i < nodes.length; i++) {
            if (seen[i] || (pass === 0 && nodes[i].parent >= 0)) {
                continue;
            }
            roots.push(i);
            seen[i] = true;
            const stack = [i];
            while (stack.length > 0) {
                const n = stack.pop() as number;
                order.push(n);
                const fresh: number[] = [];
                for (const k of nodes[n].kids) {
                    if (!seen[k]) {
                        seen[k] = true;
                        nodes[k].depth = nodes[n].depth + 1;
                        fresh.push(k);
                    }
                }
                nodes[n].tree = fresh;
                for (let j = fresh.length - 1; j >= 0; j--) {
                    stack.push(fresh[j]);
                }
            }
        }
    }
    // what is shown: a node is open when its depth is within `levels` (0: all), toggled otherwise
    const toggledSet = new Set<string>(toggled);
    const vis: number[] = [];
    const stack: number[] = [];
    for (let j = roots.length - 1; j >= 0; j--) {
        stack.push(roots[j]);
    }
    let depths = 0;
    while (stack.length > 0) {
        const n = stack.pop() as number;
        const node = nodes[n];
        vis.push(n);
        depths = Math.max(depths, node.depth + 1);
        const byDefault = levels === 0 || node.depth < levels - 1;
        const open = toggledSet.has(node.id) ? !byDefault : byDefault;
        node.shown = open ? node.tree : [];
        for (let j = node.shown.length - 1; j >= 0; j--) {
            stack.push(node.shown[j]);
        }
    }
    let units = 0;
    if (compact) {
        units = tidy(nodes, vis, roots);
    } else {
        // the visible leaves in equal slots, left to right
        for (const n of vis) {
            if (nodes[n].shown.length === 0) {
                nodes[n].x = units;
                units += 1;
            }
        }
        for (let j = vis.length - 1; j >= 0; j--) {
            const node = nodes[vis[j]];
            if (node.shown.length > 0) {
                node.x = (nodes[node.shown[0]].x + nodes[node.shown[node.shown.length - 1]].x) / 2;
            }
        }
    }
    const aw = w - 2 * PAD;
    const ah = h - 2 * PAD;
    if (aw < 8 || ah < 8) {
        return out;
    }
    // top-down: the leaves' slots across, the levels down; left to right: the slots down, the
    // levels across
    const slot = (ltr ? ah : aw) / units;
    const lh = (ltr ? aw : ah) / depths;
    const bw = ltr ? Math.min(160, lh * 0.75) : Math.min(160, slot * 0.85);
    const bh = ltr ? Math.min(44, slot * 0.85) : Math.min(44, lh * 0.6);
    if (bw < 2 || bh < 2) {
        return out;
    }
    // a node's box: its centre's x and its top
    const cx = (n: OrgNode) => (ltr ? PAD + lh * n.depth + bw / 2 : PAD + slot * (n.x + 0.5));
    const top = (n: OrgNode) => (ltr ? PAD + slot * (n.x + 0.5) - bh / 2 : PAD + lh * n.depth);
    const p: Prim[] = out.prims;
    pushCard(p, lk, w, h);
    // each node's branch: the rank of its ancestor one level under a root (vis is in preorder; -1 for a root)
    const branch = new Map<number, number>();
    let cur = -1;
    let branches = 0;
    for (const n of vis) {
        if (nodes[n].depth === 0) {
            cur = -1;
        } else if (nodes[n].depth === 1) {
            cur = branches;
            branches += 1;
        }
        branch.set(n, cur);
    }
    // connectors: the parent's stem, the bar over its children, each child's stem (left to
    // right: from the parent's right edge, the bar left of the children)
    for (const n of vis) {
        const node = nodes[n];
        if (node.shown.length === 0) {
            continue;
        }
        if (ltr) {
            const right = cx(node) + bw / 2;
            const mx = right + (lh - bw) / 2;
            const my = (m: OrgNode) => top(m) + bh / 2;
            p.push(line(right, my(node), mx, my(node), lk.axis, link));
            if (node.shown.length > 1) {
                p.push(line(mx, my(nodes[node.shown[0]]), mx, my(nodes[node.shown[node.shown.length - 1]]), lk.axis, link));
            }
            for (const k of node.shown) {
                p.push(line(mx, my(nodes[k]), cx(nodes[k]) - bw / 2, my(nodes[k]), lk.axis, link));
            }
            continue;
        }
        const mid = top(node) + bh + (lh - bh) / 2;
        p.push(line(cx(node), top(node) + bh, cx(node), mid, lk.axis, link));
        if (node.shown.length > 1) {
            p.push(line(cx(nodes[node.shown[0]]), mid, cx(nodes[node.shown[node.shown.length - 1]]), mid, lk.axis, link));
        }
        for (const k of node.shown) {
            p.push(line(cx(nodes[k]), mid, cx(nodes[k]), top(nodes[k]), lk.axis, link));
        }
    }
    const toggles = bw >= 16 && bh >= 12;
    for (const n of vis) {
        const node = nodes[n];
        const x = cx(node);
        const y = top(node);
        const b = branch.get(n);
        const pc = paint(byBranch && b !== undefined ? colorOf(lk, b + 1) : color0);
        // highlighted: a node whose row is not is dimmed (docs/formats/bicraft-visuals.md "Highlights")
        if (highlighted(input) && hlNum(input, node.row) === null) {
            out.hits.push({ row: node.row, prim: p.length, id: node.id, fade: true });
        } else {
            out.hits.push({ row: node.row, prim: p.length, id: node.id });
        }
        if (minimal(lk) && !lk.rounded && !lk.shadow) {
            p.push(rect(x - bw / 2, y, bw, bh, pc.color, 1));
        } else {
            const box = card(lk, x - bw / 2, y, bw, bh, pc.fill, pc.edge);
            if (pc.grad !== "") {
                box.grad = pc.grad;
            }
            p.push(box);
            if (modern && bw >= 16) {
                p.push(accent(lk, x - bw / 2, y, 4, bh, pc.color));
            }
        }
        const tw = bw - 8;
        if (bh >= size + 2) {
            const value = showValues && node.value !== null && bh >= 2 * size + 6 ? fmtAuto(node.value) : "";
            const label = fit(node.label, size, tw);
            if (value !== "") {
                if (label !== "") {
                    p.push(textW(x, y + bh / 2 - size * 0.55, label, size, pc.ink, "middle", 0, labelWeight));
                }
                const vt = fit(value, size - 1, tw);
                if (vt !== "") {
                    p.push(text(x, y + bh / 2 + size * 0.6, vt, size - 1, pc.valueInk, "middle"));
                }
            } else if (label !== "") {
                p.push(textW(x, y + bh / 2, label, size, pc.ink, "middle", 0, labelWeight));
            }
        }
        if (toggles && node.tree.length > 0) {
            out.hits.push({ row: node.row, prim: p.length, id: node.id, toggle: true });
            const sign = node.shown.length > 0 ? "−" : "+";
            // on the box's bottom edge (left to right: its right edge)
            const tx = ltr ? x + bw / 2 : x;
            const ty = ltr ? y + bh / 2 : y + bh;
            if (minimal(lk)) {
                p.push(circle(tx, ty, 6, "#605E5C", 1));
                p.push(text(tx, ty, sign, 10, "#FFFFFF", "middle"));
            } else {
                p.push(dot(lk, tx, ty, 6.5, pc.color, 1));
                p.push(textW(tx, ty, sign, 11, onColor(lk, pc.color, pc.color), "middle", 0, 700));
            }
        }
    }
    return out;
}
