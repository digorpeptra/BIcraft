// BIcraft Sankey (docs/formats/chart-pack.md "Sankey"): one level of weighted links from a
// source to a destination, nodes in columns by their longest path from a node without incoming
// links, node heights proportional to the larger of their incoming and outgoing flows, links as
// ribbons between cubic Bézier edges, the order within each column set by a fixed number of
// barycentre sweeps. A link that skips columns runs through a waypoint in each column between
// its ends, the waypoints sorted by the sweeps as nodes are, so it passes between the nodes
// rather than over them. Labels sit beside their nodes on the outer side (the first column's on
// the left, the last column's on the right, in margins kept for them), a middle column's on a side
// clear of the ribbons or else over a halo; a label that would overlap a larger node's label in its
// column is left out. A link that closes a cycle is left out and counted in a note. Drawn with
// BIcraft's look (docs/formats/visual-look.md): nodes in its colours, rounded and shaded as its
// bars, labels bolder in Bold. The TypeScript twin of crates/render/src/chart_pack/sankey.rs.
"use strict";

import { Out, PAD, cellLabel, cellNum, cellText, est, fit, highlighted, hlNum, lookOfInput, optBool, optColor, optNum, optionsOf, poly, rect, rowsOf, text } from "./kit";
import { bar, bold, color as colorOf, dim, part, pushCard, textW } from "./runtime/style";

/** Most link rows read. */
export const MAX_ROWS = 3000;
/** Most distinct nodes (sources and destinations). */
export const MAX_NODES = 100;
/** Most distinct source and destination pairs. */
export const MAX_LINKS = 1000;
/** Barycentre sweeps, each one left to right then right to left. */
export const SWEEPS = 4;
/** Segments of each Bézier edge of a ribbon. */
export const STEPS = 16;
/** The colour of every link with `linkColor` `fixed` (unless `linkFill` says another). */
export const FIXED_LINK = "#8C8C8C";
/** Nodes with a colour option (`nodeColor1` … `nodeColor12`), in the order seen. */
export const NODE_COLORS = 12;

interface SNode {
    name: string;
    /** Its text drawn: a date written with its column's format string. */
    label: string;
    first: number;
    inflow: number;
    outflow: number;
    col: number;
    y: number;
    h: number;
}

interface SLink {
    s: number;
    t: number;
    value: number;
    first: number;
    kept: boolean;
    sy: number;
    ty: number;
    /** The nodes it runs through: its source, its waypoints, its destination. */
    hops: number[];
    /** The highlighted part of its weight (null: none of its rows is highlighted). */
    hl: number | null;
}

/** One column-to-next-column piece of a link, between nodes or waypoints. */
interface Seg {
    s: number;
    t: number;
    value: number;
}

function valueOf(n: SNode): number {
    return Math.max(n.inflow, n.outflow);
}

function centreOf(n: SNode): number {
    return n.y + n.h / 2;
}

/** A cubic Bézier coordinate at `t`. */
function bez(a: number, b: number, c: number, d: number, t: number): number {
    const u = 1 - t;
    return u * u * u * a + 3 * u * u * t * b + 3 * u * t * t * c + t * t * t * d;
}

/** Whether the segment from `a` to `b` meets the box `[x0, y0, x1, y1]` (Liang–Barsky clipping). */
function segmentMeetsBox(ax: number, ay: number, bx: number, by: number, r: number[]): boolean {
    const dx = bx - ax;
    const dy = by - ay;
    let t0 = 0;
    let t1 = 1;
    const sides = [
        [-dx, ax - r[0]],
        [dx, r[2] - ax],
        [-dy, ay - r[1]],
        [dy, r[3] - ay],
    ];
    for (const [p, q] of sides) {
        if (p === 0) {
            if (q < 0) {
                return false;
            }
        } else {
            const t = q / p;
            if (p < 0) {
                t0 = Math.max(t0, t);
            } else {
                t1 = Math.min(t1, t);
            }
        }
    }
    return t0 <= t1;
}

/** Whether the polygon `pts` and the box `r` overlap: an edge meets the box, or the box's centre is inside the polygon (even-odd). */
export function polygonMeetsBox(pts: number[][], r: number[]): boolean {
    const n = pts.length;
    if (n < 3) {
        return false;
    }
    const cx = (r[0] + r[2]) / 2;
    const cy = (r[1] + r[3]) / 2;
    let inside = false;
    for (let i = 0; i < n; i++) {
        const a = pts[i];
        const b = pts[(i + 1) % n];
        if (segmentMeetsBox(a[0], a[1], b[0], b[1], r)) {
            return true;
        }
        if (a[1] > cy !== b[1] > cy && cx < a[0] + ((cy - a[1]) * (b[0] - a[0])) / (b[1] - a[1])) {
            inside = !inside;
        }
    }
    return inside;
}

/** Stacks the nodes of a column from the top, centred in the plot's height. */
function pack(nodes: SNode[], list: number[], y0: number, ih: number, pad: number): void {
    let total = 0;
    for (const i of list) {
        total += nodes[i].h;
    }
    total += Math.max(list.length - 1, 0) * pad;
    let y = y0 + (ih - total) / 2;
    for (const i of list) {
        nodes[i].y = y;
        y += nodes[i].h + pad;
    }
}

/**
 * A column sorted by the barycentre of its nodes' and waypoints' neighbours (`by`: each one's
 * pieces in piece order; `incoming`: the neighbour is the piece's source), ties in the column's
 * current order.
 */
function reorder(nodes: SNode[], links: Seg[], list: number[], by: number[][], incoming: boolean): number[] {
    const keyed: { b: number; r: number; i: number }[] = [];
    for (let rank = 0; rank < list.length; rank++) {
        const i = list[rank];
        let sum = 0;
        let weight = 0;
        for (const li of by[i]) {
            const l = links[li];
            const o = nodes[incoming ? l.s : l.t];
            sum += centreOf(o) * l.value;
            weight += l.value;
        }
        keyed.push({ b: weight > 0 ? sum / weight : centreOf(nodes[i]), r: rank, i });
    }
    keyed.sort((a, b) => (a.b < b.b ? -1 : a.b > b.b ? 1 : a.r - b.r));
    return keyed.map((k) => k.i);
}

/** The note under a drawing that left links out. */
function note(n: number, fr: boolean): string {
    if (fr) {
        return n === 1 ? "1 lien en boucle non dessiné" : `${n} liens en boucle non dessinés`;
    }
    return n === 1 ? "1 link in a cycle not drawn" : `${n} links in cycles not drawn`;
}

/** Lays out `{rows: [[source, destination, weight]], options, lang}` in a `w`×`h` visual. */
export function layoutSankey(input: any, w: number, h: number): Out {
    const o = optionsOf(input);
    const nodeW = optNum(o, "nodeWidth", 2, 60, 12);
    const padding = optNum(o, "nodePadding", 0, 60, 10);
    const opacity = optNum(o, "linkOpacity", 5, 100, 40) / 100;
    const mode = o.linkColor === "destination" ? "destination" : o.linkColor === "fixed" ? "fixed" : "source";
    const fixed = optColor(o, "linkFill", FIXED_LINK);
    const showLabels = optBool(o, "showLabels", true);
    const fr = !!input && input.lang === "fr";
    const k = lookOfInput(input);
    // 11 in Minimal, a step larger with the modern look's value labels
    const size = optNum(o, "fontSize", 8, 20, 11 + k.value - 10);
    const out: Out = { prims: [], hits: [] };
    const p = out.prims;
    const nodes: SNode[] = [];
    const nodeIx = new Map<string, number>();
    const links: SLink[] = [];
    const linkIx = new Map<string, number>();
    const nodeOf = (name: string, label: () => string, row: number): number | undefined => {
        const known = nodeIx.get(name);
        if (known !== undefined) {
            return known;
        }
        if (nodes.length >= MAX_NODES) {
            return undefined;
        }
        nodeIx.set(name, nodes.length);
        nodes.push({ name, label: label(), first: row, inflow: 0, outflow: 0, col: 0, y: 0, h: 0 });
        return nodes.length - 1;
    };
    const rows = rowsOf(input, MAX_ROWS);
    for (let i = 0; i < rows.length; i++) {
        const s = cellText(rows[i][0]);
        const d = cellText(rows[i][1]);
        const v = cellNum(rows[i][2]);
        if (v === null || s === "" || d === "" || v <= 0) {
            continue;
        }
        const si = nodeOf(s, () => cellLabel(input, 0, rows[i][0]), i);
        if (si === undefined) {
            continue;
        }
        const ti = nodeOf(d, () => cellLabel(input, 1, rows[i][1]), i);
        if (ti === undefined) {
            continue;
        }
        const key = si + "|" + ti;
        let li = linkIx.get(key);
        if (li === undefined) {
            if (links.length >= MAX_LINKS) {
                continue;
            }
            li = links.length;
            linkIx.set(key, li);
            links.push({ s: si, t: ti, value: 0, first: i, kept: true, sy: 0, ty: 0, hops: [], hl: null });
        }
        links[li].value += v;
        const hv = hlNum(input, i);
        if (hv !== null) {
            const q = links[li].hl;
            links[li].hl = (q === null ? 0 : q) + hv;
        }
    }
    const lit = highlighted(input);
    const n = nodes.length;
    const outs: number[][] = nodes.map(() => []);
    for (let li = 0; li < links.length; li++) {
        outs[links[li].s].push(li);
    }
    // cycles: a depth-first walk from each node in the order seen, links in the order seen; a
    // link to a node still on the walk's path closes a cycle and is left out
    const state: number[] = nodes.map(() => 0);
    for (let start = 0; start < n; start++) {
        if (state[start] !== 0) {
            continue;
        }
        state[start] = 1;
        const stack: number[][] = [[start, 0]];
        while (stack.length > 0) {
            const top = stack[stack.length - 1];
            const v = top[0];
            if (top[1] < outs[v].length) {
                const li = outs[v][top[1]];
                top[1] += 1;
                const t = links[li].t;
                if (state[t] === 1) {
                    links[li].kept = false;
                } else if (state[t] === 0) {
                    state[t] = 1;
                    stack.push([t, 0]);
                }
            } else {
                stack.pop();
                state[v] = 2;
            }
        }
    }
    let broken = 0;
    const ins: number[][] = nodes.map(() => []);
    const keptOuts: number[][] = nodes.map(() => []);
    const indeg: number[] = nodes.map(() => 0);
    for (let li = 0; li < links.length; li++) {
        const l = links[li];
        if (!l.kept) {
            broken += 1;
            continue;
        }
        keptOuts[l.s].push(li);
        ins[l.t].push(li);
        indeg[l.t] += 1;
        nodes[l.s].outflow += l.value;
        nodes[l.t].inflow += l.value;
    }
    // columns: the longest path from a node without incoming links (in topological order)
    const queue: number[] = [];
    for (let i = 0; i < n; i++) {
        if (indeg[i] === 0) {
            queue.push(i);
        }
    }
    for (let head = 0; head < queue.length; head++) {
        const v = queue[head];
        const cv = nodes[v].col;
        for (const li of keptOuts[v]) {
            const t = links[li].t;
            nodes[t].col = Math.max(nodes[t].col, cv + 1);
            indeg[t] = Math.max(indeg[t] - 1, 0);
            if (indeg[t] === 0) {
                queue.push(t);
            }
        }
    }
    const drawn: number[] = [];
    let ncols = 0;
    for (let i = 0; i < n; i++) {
        if (valueOf(nodes[i]) > 0) {
            drawn.push(i);
            ncols = Math.max(ncols, nodes[i].col + 1);
        }
    }
    if (drawn.length === 0) {
        return out;
    }
    // the pieces of the links: a link that skips columns gets a waypoint in each column between
    // its ends (after the nodes, in link order)
    const segs: Seg[] = [];
    const segIns: number[][] = nodes.map(() => []);
    const segOuts: number[][] = nodes.map(() => []);
    const piece = (s: number, t: number, value: number) => {
        segOuts[s].push(segs.length);
        segIns[t].push(segs.length);
        segs.push({ s, t, value });
    };
    for (const l of links) {
        if (!l.kept) {
            continue;
        }
        l.hops = [l.s];
        for (let c = nodes[l.s].col + 1; c < nodes[l.t].col; c++) {
            const d = nodes.length;
            nodes.push({ name: "", label: "", first: l.first, inflow: l.value, outflow: l.value, col: c, y: 0, h: 0 });
            segIns.push([]);
            segOuts.push([]);
            piece(l.hops[l.hops.length - 1], d, l.value);
            l.hops.push(d);
        }
        piece(l.hops[l.hops.length - 1], l.t, l.value);
        l.hops.push(l.t);
    }
    const withNote = broken > 0;
    // the first column's labels on the left of their nodes, the last column's on the right: room
    // beside the nodes for the widest of each (at most 30% of the width on each side)
    const room = (w - 2 * PAD) * 0.3;
    let ml = 0;
    let mr = 0;
    if (showLabels) {
        for (const i of drawn) {
            const wd = Math.min(est(nodes[i].label, size) + 6, room);
            if (ncols > 1 && nodes[i].col === 0) {
                ml = Math.max(ml, wd);
            }
            if (nodes[i].col === ncols - 1) {
                mr = Math.max(mr, wd);
            }
        }
    }
    const x0 = PAD + ml;
    const x1 = w - PAD - mr;
    const y0 = PAD;
    const y1 = h - PAD - (withNote ? 14 : 0);
    const iw = x1 - x0;
    const ih = y1 - y0;
    if (isNaN(iw) || isNaN(ih) || iw < nodeW + 2 || ih < 8) {
        return out;
    }
    const cols: number[][] = [];
    for (let c = 0; c < ncols; c++) {
        cols.push([]);
    }
    for (const i of drawn) {
        cols[nodes[i].col].push(i);
    }
    for (let d = n; d < nodes.length; d++) {
        cols[nodes[d].col].push(d);
    }
    const step = ncols > 1 ? (iw - nodeW) / (ncols - 1) : 0;
    let maxN = 1;
    for (const c of cols) {
        maxN = Math.max(maxN, c.length);
    }
    let pad = padding;
    if (maxN > 1 && (maxN - 1) * pad > ih / 2) {
        pad = ih / 2 / (maxN - 1);
    }
    let ky = Infinity;
    for (const c of cols) {
        let total = 0;
        for (const i of c) {
            total += valueOf(nodes[i]);
        }
        if (total > 0) {
            const k = (ih - Math.max(c.length - 1, 0) * pad) / total;
            if (k < ky) {
                ky = k;
            }
        }
    }
    if (!isFinite(ky)) {
        return out;
    }
    for (const c of cols) {
        for (const i of c) {
            nodes[i].h = valueOf(nodes[i]) * ky;
        }
    }
    for (const c of cols) {
        pack(nodes, c, y0, ih, pad);
    }
    // node and waypoint order: barycentre sweeps, left to right over incoming pieces, then right
    // to left over outgoing ones
    for (let sweep = 0; sweep < SWEEPS; sweep++) {
        for (let c = 1; c < ncols; c++) {
            cols[c] = reorder(nodes, segs, cols[c], segIns, true);
            pack(nodes, cols[c], y0, ih, pad);
        }
        for (let c = ncols - 2; c >= 0; c--) {
            cols[c] = reorder(nodes, segs, cols[c], segOuts, false);
            pack(nodes, cols[c], y0, ih, pad);
        }
    }
    // each node's links stacked from its top, in the order of the nodes (or waypoints) at the
    // other end of their first or last piece
    const byY = (other: (li: number) => number) => (a: number, b: number) => {
        const ya = nodes[other(a)].y;
        const yb = nodes[other(b)].y;
        return ya < yb ? -1 : ya > yb ? 1 : a - b;
    };
    for (const i of drawn) {
        let y = nodes[i].y;
        for (const li of keptOuts[i].slice().sort(byY((x) => links[x].hops[1]))) {
            links[li].sy = y;
            y += links[li].value * ky;
        }
        y = nodes[i].y;
        for (const li of ins[i].slice().sort(byY((x) => links[x].hops[links[x].hops.length - 2]))) {
            links[li].ty = y;
            y += links[li].value * ky;
        }
    }
    const xOf = (col: number) => x0 + col * step;
    // a node takes the look's palette colour by its order seen, or its colour option when one is
    // set (the first twelve); its links coloured by it follow
    const color = (i: number) => (i < NODE_COLORS ? optColor(o, "nodeColor" + (i + 1), colorOf(k, i)) : colorOf(k, i));
    pushCard(p, k, w, h);
    // each ribbon's column-to-column pieces, their bounding boxes first: the labels keep clear
    const pieces: { box: number[]; pts: number[][] }[] = [];
    for (let li = 0; li < links.length; li++) {
        const l = links[li];
        if (!l.kept) {
            continue;
        }
        const s = nodes[l.s];
        const t = nodes[l.t];
        const th = l.value * ky;
        // the top edge piece by piece, from the source through the waypoints, then the bottom
        // edge back
        const last = l.hops.length - 1;
        const topOf = (j: number) => (j === 0 ? l.sy : j === last ? l.ty : nodes[l.hops[j]].y);
        // the ribbon `thk` thick below its top edge
        const ribbon = (thk: number): number[][] => {
            const pts: number[][] = [];
            for (let j = 0; j < last; j++) {
                const sx = xOf(nodes[l.hops[j]].col) + nodeW;
                const tx = xOf(nodes[l.hops[j + 1]].col);
                const xm = (sx + tx) / 2;
                const a = topOf(j);
                const b = topOf(j + 1);
                for (let q = 0; q <= STEPS; q++) {
                    const u = q / STEPS;
                    pts.push([bez(sx, xm, xm, tx, u), bez(a, a, b, b, u)]);
                }
            }
            for (let j = last - 1; j >= 0; j--) {
                const sx = xOf(nodes[l.hops[j]].col) + nodeW;
                const tx = xOf(nodes[l.hops[j + 1]].col);
                const xm = (sx + tx) / 2;
                const a = topOf(j) + thk;
                const b = topOf(j + 1) + thk;
                for (let q = STEPS; q >= 0; q--) {
                    const u = q / STEPS;
                    pts.push([bez(sx, xm, xm, tx, u), bez(a, a, b, b, u)]);
                }
            }
            return pts;
        };
        for (let j = 0; j < last; j++) {
            const sx = xOf(nodes[l.hops[j]].col) + nodeW;
            const tx = xOf(nodes[l.hops[j + 1]].col);
            const xm = (sx + tx) / 2;
            const a = topOf(j);
            const b = topOf(j + 1);
            const pts: number[][] = [];
            for (let q = 0; q <= STEPS; q++) {
                const u = q / STEPS;
                pts.push([bez(sx, xm, xm, tx, u), bez(a, a, b, b, u)]);
            }
            for (let q = STEPS; q >= 0; q--) {
                const u = q / STEPS;
                pts.push([bez(sx, xm, xm, tx, u), bez(a + th, a + th, b + th, b + th, u)]);
            }
            pieces.push({ box: [Math.min(sx, tx), Math.min(a, b), Math.max(sx, tx), Math.max(a, b) + th], pts });
        }
        const fill = mode === "destination" ? color(l.t) : mode === "fixed" ? fixed : color(l.s);
        const hit = { row: l.first, prim: p.length, link: li, source: s.name, target: t.name, sourceLabel: s.label, targetLabel: t.label, value: l.value };
        out.hits.push(hit);
        const full = poly(ribbon(th), fill, opacity);
        if (!lit) {
            p.push(full);
            continue;
        }
        // highlighted: the full ribbon dimmed and its highlighted part along its top edge
        p.push(dim(full));
        const hv = part(l.value, l.hl);
        if (hv !== null) {
            out.hits.push(Object.assign({}, hit, { prim: p.length }));
            p.push(poly(ribbon(hv * ky), fill, opacity));
        }
    }
    for (const i of drawn) {
        const x = nodes[i];
        const hit: { [k: string]: any } = { row: x.first, prim: p.length, node: i, name: x.name, label: x.label, value: valueOf(x), inflow: x.inflow, outflow: x.outflow };
        // highlighted: a node none of whose links is highlighted is dimmed
        if (lit && !links.some((l) => l.kept && (l.s === i || l.t === i) && l.hl !== null)) {
            hit.fade = true;
        }
        out.hits.push(hit);
        p.push(bar(k, xOf(x.col), x.y, nodeW, x.h, color(i), ""));
    }
    const weight = bold(k) ? 600 : 400;
    if (showLabels) {
        const labs: { i: number; y: number; label: string; keep: boolean }[] = [];
        const yOf = (x: SNode) => Math.min(Math.max(centreOf(x), size / 2), h - size / 2);
        for (const i of drawn) {
            const x = nodes[i];
            const y = yOf(x);
            // the outer columns' labels in the room beside the plot, a middle column's in the gap
            // to the next column
            const outer = ncols > 1 && x.col === 0 ? ml : x.col === ncols - 1 ? mr : 0;
            const avail = outer > 0 ? outer - 6 : step - nodeW - 8;
            const label = fit(x.label, size, avail);
            if (label !== "") {
                labs.push({ i, y, label, keep: false });
            }
        }
        // in each column, the larger nodes' labels first: one closer than its size to a label
        // kept is left out
        const order = labs.map((_, j) => j);
        order.sort((a, b) => {
            const va = valueOf(nodes[labs[a].i]);
            const vb = valueOf(nodes[labs[b].i]);
            return va > vb ? -1 : va < vb ? 1 : a - b;
        });
        const keptY: number[][] = cols.map(() => []);
        for (const j of order) {
            const lb = labs[j];
            const ys = keptY[nodes[lb.i].col];
            if (ys.every((y) => Math.abs(y - lb.y) >= size)) {
                lb.keep = true;
                ys.push(lb.y);
            }
        }
        // each label beside its node: the first column's on the left, the last one's on the
        // right, a middle column's on the first side (right, then left) clear of the ribbons and
        // the labels placed, else on the first side clear of the labels on a halo of the surface;
        // one with no side clear of the labels is left out
        const boxes: number[][] = [];
        const meets = (r: number[]) => boxes.some((q) => r[0] < q[2] && q[0] < r[2] && r[1] < q[3] && q[1] < r[3]);
        const onRibbon = (r: number[]) =>
            pieces.some((c) => c.box[0] < r[2] && r[0] < c.box[2] && c.box[1] < r[3] && r[1] < c.box[3] && polygonMeetsBox(c.pts, r));
        for (const lb of labs) {
            if (!lb.keep) {
                continue;
            }
            const x = nodes[lb.i];
            const nx = xOf(x.col);
            const wd = est(lb.label, size);
            const right = [nx + nodeW + 4, lb.y - size / 2, nx + nodeW + 4 + wd, lb.y + size / 2];
            const left = [nx - 4 - wd, lb.y - size / 2, nx - 4, lb.y + size / 2];
            const sides = ncols > 1 && x.col === 0 ? [left] : x.col === ncols - 1 ? [right] : [right, left];
            let pick = sides.find((r) => !meets(r) && !onRibbon(r));
            let halo = false;
            if (pick === undefined) {
                pick = sides.find((r) => !meets(r));
                halo = true;
            }
            if (pick === undefined) {
                continue;
            }
            boxes.push(pick);
            if (halo) {
                p.push(rect(pick[0] - 2, pick[1] - 1, wd + 4, size + 2, k.surface, 0.85));
            }
            if (pick === left) {
                p.push(textW(nx - 4, lb.y, lb.label, size, k.label, "end", 0, weight));
            } else {
                p.push(textW(nx + nodeW + 4, lb.y, lb.label, size, k.label, "start", 0, weight));
            }
        }
    }
    if (withNote) {
        const msg = fit(note(broken, fr), k.tick, iw);
        if (msg !== "") {
            p.push(text(PAD, h - PAD - 5, msg, k.tick, k.text, "start"));
        }
    }
    return out;
}
