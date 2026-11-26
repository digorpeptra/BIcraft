//! BIcraft Sankey (docs/formats/chart-pack.md "Sankey"): one level of weighted links from a
//! source to a destination, nodes in columns by their longest path from a node without incoming
//! links, node heights proportional to the larger of their incoming and outgoing flows, links as
//! ribbons between cubic Bézier edges, the order within each column set by a fixed number of
//! barycentre sweeps. A link that skips columns runs through a waypoint in each column between
//! its ends, the waypoints sorted by the sweeps as nodes are, so it passes between the nodes
//! rather than over them. A label that would overlap a larger node's label in its column is left
//! out. A link that closes a cycle is left out and counted in a note. Drawn with
//! BIcraft's look (docs/formats/visual-look.md): nodes in its colours, rounded and shaded as its
//! bars, labels bolder in Bold. The Rust twin of `crates/engine/src/chart_pack/sankey.ts`.

use std::collections::HashMap;

use serde_json::{Value, json};

use super::{
    Out, PAD, cell_label, cell_num, cell_text, fit, highlighted, hl_num, is_fr, look_of, opt_bool, opt_color,
    opt_num, options_of, poly, rows_of, text,
};
use crate::look::text_w;

/// Most link rows read.
pub const MAX_ROWS: usize = 3000;
/// Most distinct nodes (sources and destinations).
pub const MAX_NODES: usize = 100;
/// Most distinct source and destination pairs.
pub const MAX_LINKS: usize = 1000;
/// Barycentre sweeps, each one left to right then right to left.
pub const SWEEPS: usize = 4;
/// Segments of each Bézier edge of a ribbon.
pub const STEPS: usize = 16;
/// The colour of every link with `linkColor` `fixed` (unless `linkFill` says another).
pub const FIXED_LINK: &str = "#8C8C8C";
/// Nodes with a colour option (`nodeColor1` … `nodeColor12`), in the order seen.
pub const NODE_COLORS: usize = 12;

struct Node {
    name: String,
    /// Its text drawn: a date written with its column's format string.
    label: String,
    first: usize,
    inflow: f64,
    outflow: f64,
    col: usize,
    y: f64,
    h: f64,
}

struct Link {
    s: usize,
    t: usize,
    value: f64,
    first: usize,
    kept: bool,
    sy: f64,
    ty: f64,
    /// The nodes it runs through: its source, its waypoints, its destination.
    hops: Vec<usize>,
    /// The highlighted part of its weight (`None`: none of its rows is highlighted).
    hl: Option<f64>,
}

/// One column-to-next-column piece of a link, between nodes or waypoints.
struct Seg {
    s: usize,
    t: usize,
    value: f64,
}

/// Adds the piece from `s` to `t`.
fn piece(
    segs: &mut Vec<Seg>,
    ins: &mut [Vec<usize>],
    outs: &mut [Vec<usize>],
    s: usize,
    t: usize,
    value: f64,
) {
    if let Some(x) = outs.get_mut(s) {
        x.push(segs.len());
    }
    if let Some(x) = ins.get_mut(t) {
        x.push(segs.len());
    }
    segs.push(Seg { s, t, value });
}

impl Node {
    fn value(&self) -> f64 {
        self.inflow.max(self.outflow)
    }

    fn centre(&self) -> f64 {
        self.y + self.h / 2.0
    }
}

/// The node named `name`, added (first seen at `row`) unless there are [`MAX_NODES`] already.
fn node_of(
    nodes: &mut Vec<Node>,
    ix: &mut HashMap<String, usize>,
    name: String,
    label: impl FnOnce() -> String,
    row: usize,
) -> Option<usize> {
    if let Some(&i) = ix.get(&name) {
        return Some(i);
    }
    if nodes.len() >= MAX_NODES {
        return None;
    }
    ix.insert(name.clone(), nodes.len());
    nodes.push(Node {
        name,
        label: label(),
        first: row,
        inflow: 0.0,
        outflow: 0.0,
        col: 0,
        y: 0.0,
        h: 0.0,
    });
    Some(nodes.len() - 1)
}

/// A cubic Bézier coordinate at `t`.
fn bez(a: f64, b: f64, c: f64, d: f64, t: f64) -> f64 {
    let u = 1.0 - t;
    u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c + t * t * t * d
}

/// Stacks the nodes of a column from the top, centred in the plot's height.
fn pack(nodes: &mut [Node], list: &[usize], y0: f64, ih: f64, pad: f64) {
    let mut total = 0.0;
    for &i in list {
        total += nodes.get(i).map_or(0.0, |n| n.h);
    }
    total += list.len().saturating_sub(1) as f64 * pad;
    let mut y = y0 + (ih - total) / 2.0;
    for &i in list {
        if let Some(n) = nodes.get_mut(i) {
            n.y = y;
            y += n.h + pad;
        }
    }
}

/// Sorts a column by the barycentre of its nodes' and waypoints' neighbours (`by`: each one's
/// pieces in piece order; `incoming`: the neighbour is the piece's source), ties in the column's
/// current order.
fn reorder(nodes: &[Node], links: &[Seg], list: &mut Vec<usize>, by: &[Vec<usize>], incoming: bool) {
    let mut keyed: Vec<(f64, usize, usize)> = Vec::with_capacity(list.len());
    for (rank, &i) in list.iter().enumerate() {
        let mut sum = 0.0;
        let mut weight = 0.0;
        for &li in by.get(i).map_or(&[][..], Vec::as_slice) {
            let Some(l) = links.get(li) else { continue };
            let other = if incoming { l.s } else { l.t };
            if let Some(o) = nodes.get(other) {
                sum += o.centre() * l.value;
                weight += l.value;
            }
        }
        let own = nodes.get(i).map_or(0.0, Node::centre);
        keyed.push((if weight > 0.0 { sum / weight } else { own }, rank, i));
    }
    keyed.sort_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.cmp(&b.1))
    });
    *list = keyed.into_iter().map(|k| k.2).collect();
}

/// The note under a drawing that left links out.
fn note(n: usize, fr: bool) -> String {
    match (fr, n == 1) {
        (false, true) => "1 link in a cycle not drawn".into(),
        (false, false) => format!("{n} links in cycles not drawn"),
        (true, true) => "1 lien en boucle non dessiné".into(),
        (true, false) => format!("{n} liens en boucle non dessinés"),
    }
}

/// Lays out `{rows: [[source, destination, weight]], options, lang}` in a `w`×`h` visual.
pub fn layout(input: &Value, w: f64, h: f64) -> Out {
    let o = options_of(input);
    let node_w = opt_num(o, "nodeWidth", 2.0, 60.0, 12.0);
    let padding = opt_num(o, "nodePadding", 0.0, 60.0, 10.0);
    let opacity = opt_num(o, "linkOpacity", 5.0, 100.0, 40.0) / 100.0;
    let mode = match o.get("linkColor").and_then(Value::as_str) {
        Some("destination") => "destination",
        Some("fixed") => "fixed",
        _ => "source",
    };
    let fixed = opt_color(o, "linkFill", FIXED_LINK);
    let show_labels = opt_bool(o, "showLabels", true);
    let fr = is_fr(input);
    let k = look_of(input);
    // 11 in Minimal, a step larger with the modern look's value labels
    let size = opt_num(o, "fontSize", 8.0, 20.0, 11.0 + k.value - 10.0);
    let mut out = Out::default();
    let mut nodes: Vec<Node> = Vec::new();
    let mut node_ix: HashMap<String, usize> = HashMap::new();
    let mut links: Vec<Link> = Vec::new();
    let mut link_ix: HashMap<(usize, usize), usize> = HashMap::new();
    for (i, r) in rows_of(input, MAX_ROWS).into_iter().enumerate() {
        let s = cell_text(r.first());
        let d = cell_text(r.get(1));
        let Some(v) = cell_num(r.get(2)) else { continue };
        if s.is_empty() || d.is_empty() || v <= 0.0 {
            continue;
        }
        let Some(si) = node_of(&mut nodes, &mut node_ix, s, || cell_label(input, 0, r.first()), i) else {
            continue;
        };
        let Some(ti) = node_of(&mut nodes, &mut node_ix, d, || cell_label(input, 1, r.get(1)), i) else {
            continue;
        };
        let li = match link_ix.get(&(si, ti)) {
            Some(&li) => li,
            None => {
                if links.len() >= MAX_LINKS {
                    continue;
                }
                link_ix.insert((si, ti), links.len());
                links.push(Link {
                    s: si,
                    t: ti,
                    value: 0.0,
                    first: i,
                    kept: true,
                    sy: 0.0,
                    ty: 0.0,
                    hops: Vec::new(),
                    hl: None,
                });
                links.len() - 1
            }
        };
        if let Some(l) = links.get_mut(li) {
            l.value += v;
            if let Some(hv) = hl_num(input, i) {
                l.hl = Some(l.hl.unwrap_or(0.0) + hv);
            }
        }
    }
    let lit = highlighted(input);
    let n = nodes.len();
    let mut outs: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (li, l) in links.iter().enumerate() {
        if let Some(x) = outs.get_mut(l.s) {
            x.push(li);
        }
    }
    // cycles: a depth-first walk from each node in the order seen, links in the order seen; a
    // link to a node still on the walk's path closes a cycle and is left out
    let mut state = vec![0u8; n];
    for start in 0..n {
        if state.get(start) != Some(&0) {
            continue;
        }
        if let Some(s) = state.get_mut(start) {
            *s = 1;
        }
        let mut stack: Vec<(usize, usize)> = vec![(start, 0)];
        while let Some(&(v, k)) = stack.last() {
            match outs.get(v).and_then(|l| l.get(k)).copied() {
                Some(li) => {
                    if let Some(top) = stack.last_mut() {
                        top.1 += 1;
                    }
                    let Some(l) = links.get_mut(li) else { continue };
                    let t = l.t;
                    match state.get(t).copied() {
                        Some(1) => l.kept = false,
                        Some(0) => {
                            if let Some(s) = state.get_mut(t) {
                                *s = 1;
                            }
                            stack.push((t, 0));
                        }
                        _ => {}
                    }
                }
                None => {
                    stack.pop();
                    if let Some(s) = state.get_mut(v) {
                        *s = 2;
                    }
                }
            }
        }
    }
    let broken = links.iter().filter(|l| !l.kept).count();
    let mut ins: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut kept_outs: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut indeg = vec![0usize; n];
    for (li, l) in links.iter().enumerate() {
        if !l.kept {
            continue;
        }
        if let Some(x) = kept_outs.get_mut(l.s) {
            x.push(li);
        }
        if let Some(x) = ins.get_mut(l.t) {
            x.push(li);
        }
        if let Some(x) = indeg.get_mut(l.t) {
            *x += 1;
        }
        if let Some(x) = nodes.get_mut(l.s) {
            x.outflow += l.value;
        }
        if let Some(x) = nodes.get_mut(l.t) {
            x.inflow += l.value;
        }
    }
    // columns: the longest path from a node without incoming links (in topological order)
    let mut queue: Vec<usize> = (0..n).filter(|&i| indeg.get(i) == Some(&0)).collect();
    let mut head = 0;
    while let Some(&v) = queue.get(head) {
        head += 1;
        let cv = nodes.get(v).map_or(0, |x| x.col);
        for &li in kept_outs.get(v).map_or(&[][..], Vec::as_slice) {
            let Some(t) = links.get(li).map(|l| l.t) else {
                continue;
            };
            if let Some(x) = nodes.get_mut(t) {
                x.col = x.col.max(cv + 1);
            }
            if let Some(x) = indeg.get_mut(t) {
                *x = x.saturating_sub(1);
                if *x == 0 {
                    queue.push(t);
                }
            }
        }
    }
    let drawn: Vec<usize> = (0..n)
        .filter(|&i| nodes.get(i).is_some_and(|x| x.value() > 0.0))
        .collect();
    let Some(ncols) = drawn
        .iter()
        .filter_map(|&i| nodes.get(i).map(|x| x.col + 1))
        .max()
    else {
        return out;
    };
    // the pieces of the links: a link that skips columns gets a waypoint in each column between
    // its ends (after the nodes, in link order)
    let mut segs: Vec<Seg> = Vec::new();
    let mut seg_ins: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut seg_outs: Vec<Vec<usize>> = vec![Vec::new(); n];
    for l in links.iter_mut() {
        if !l.kept {
            continue;
        }
        let (Some(sc), Some(tc)) = (nodes.get(l.s).map(|x| x.col), nodes.get(l.t).map(|x| x.col)) else {
            continue;
        };
        l.hops = vec![l.s];
        let mut prev = l.s;
        for c in sc + 1..tc {
            let d = nodes.len();
            nodes.push(Node {
                name: String::new(),
                label: String::new(),
                first: l.first,
                inflow: l.value,
                outflow: l.value,
                col: c,
                y: 0.0,
                h: 0.0,
            });
            seg_ins.push(Vec::new());
            seg_outs.push(Vec::new());
            piece(&mut segs, &mut seg_ins, &mut seg_outs, prev, d, l.value);
            l.hops.push(d);
            prev = d;
        }
        piece(&mut segs, &mut seg_ins, &mut seg_outs, prev, l.t, l.value);
        l.hops.push(l.t);
    }
    let with_note = broken > 0;
    let x0 = PAD;
    let x1 = w - PAD;
    let y0 = PAD;
    let y1 = h - PAD - if with_note { 14.0 } else { 0.0 };
    let iw = x1 - x0;
    let ih = y1 - y0;
    if iw.is_nan() || ih.is_nan() || iw < node_w + 2.0 || ih < 8.0 {
        return out;
    }
    let mut cols: Vec<Vec<usize>> = vec![Vec::new(); ncols];
    for i in drawn.iter().copied().chain(n..nodes.len()) {
        if let Some(c) = nodes.get(i).and_then(|x| cols.get_mut(x.col)) {
            c.push(i);
        }
    }
    let step = if ncols > 1 {
        (iw - node_w) / (ncols - 1) as f64
    } else {
        0.0
    };
    let max_n = cols.iter().map(Vec::len).max().unwrap_or(1).max(1);
    let mut pad = padding;
    if max_n > 1 && (max_n - 1) as f64 * pad > ih / 2.0 {
        pad = ih / 2.0 / (max_n - 1) as f64;
    }
    let mut ky = f64::INFINITY;
    for c in &cols {
        let mut total = 0.0;
        for &i in c {
            total += nodes.get(i).map_or(0.0, Node::value);
        }
        if total > 0.0 {
            let k = (ih - c.len().saturating_sub(1) as f64 * pad) / total;
            if k < ky {
                ky = k;
            }
        }
    }
    if !ky.is_finite() {
        return out;
    }
    for c in &cols {
        for &i in c {
            if let Some(x) = nodes.get_mut(i) {
                x.h = x.value() * ky;
            }
        }
    }
    for c in &cols {
        pack(&mut nodes, c, y0, ih, pad);
    }
    // node and waypoint order: barycentre sweeps, left to right over incoming pieces, then right
    // to left over outgoing ones
    for _ in 0..SWEEPS {
        for c in 1..ncols {
            if let Some(list) = cols.get_mut(c) {
                reorder(&nodes, &segs, list, &seg_ins, true);
                pack(&mut nodes, list, y0, ih, pad);
            }
        }
        for c in (0..ncols - 1).rev() {
            if let Some(list) = cols.get_mut(c) {
                reorder(&nodes, &segs, list, &seg_outs, false);
                pack(&mut nodes, list, y0, ih, pad);
            }
        }
    }
    // each node's links stacked from its top, in the order of the nodes (or waypoints) at the
    // other end of their first or last piece
    let y_of = |i: usize| nodes.get(i).map_or(0.0, |x| x.y);
    let next_hop = |li: usize| links.get(li).and_then(|x| x.hops.get(1)).copied().unwrap_or(0);
    let prev_hop = |li: usize| {
        links
            .get(li)
            .and_then(|x| x.hops.len().checked_sub(2).and_then(|j| x.hops.get(j)))
            .copied()
            .unwrap_or(0)
    };
    let mut tops: Vec<(usize, f64)> = Vec::new();
    let mut bottoms: Vec<(usize, f64)> = Vec::new();
    for &i in &drawn {
        let mut l: Vec<usize> = kept_outs.get(i).cloned().unwrap_or_default();
        let other = |li: &usize| y_of(next_hop(*li));
        l.sort_by(|a, b| {
            other(a)
                .partial_cmp(&other(b))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.cmp(b))
        });
        let mut y = y_of(i);
        for li in l {
            tops.push((li, y));
            y += links.get(li).map_or(0.0, |x| x.value) * ky;
        }
        let mut l: Vec<usize> = ins.get(i).cloned().unwrap_or_default();
        let other = |li: &usize| y_of(prev_hop(*li));
        l.sort_by(|a, b| {
            other(a)
                .partial_cmp(&other(b))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.cmp(b))
        });
        let mut y = y_of(i);
        for li in l {
            bottoms.push((li, y));
            y += links.get(li).map_or(0.0, |x| x.value) * ky;
        }
    }
    for (li, y) in tops {
        if let Some(l) = links.get_mut(li) {
            l.sy = y;
        }
    }
    for (li, y) in bottoms {
        if let Some(l) = links.get_mut(li) {
            l.ty = y;
        }
    }
    let x_of = |col: usize| x0 + col as f64 * step;
    // a node takes the look's palette colour by its order seen, or its colour option when one is
    // set (the first twelve); its links coloured by it follow
    let color = |i: usize| {
        if i < NODE_COLORS {
            opt_color(o, &format!("nodeColor{}", i + 1), &k.color(i))
        } else {
            k.color(i)
        }
    };
    k.push_card(&mut out.prims, w, h);
    for (li, l) in links.iter().enumerate() {
        if !l.kept {
            continue;
        }
        let (Some(s), Some(t)) = (nodes.get(l.s), nodes.get(l.t)) else {
            continue;
        };
        let th = l.value * ky;
        // the top edge piece by piece, from the source through the waypoints, then the bottom
        // edge back
        let last = l.hops.len().saturating_sub(1);
        let top_of = |j: usize| {
            if j == 0 {
                l.sy
            } else if j == last {
                l.ty
            } else {
                l.hops.get(j).map_or(0.0, |&d| y_of(d))
            }
        };
        let col_of = |j: usize| l.hops.get(j).and_then(|&d| nodes.get(d)).map_or(0, |x| x.col);
        // the ribbon `th` thick below its top edge
        let ribbon = |th: f64| {
            let mut pts: Vec<(f64, f64)> = Vec::with_capacity(last * (2 * STEPS + 2));
            for j in 0..last {
                let sx = x_of(col_of(j)) + node_w;
                let tx = x_of(col_of(j + 1));
                let xm = (sx + tx) / 2.0;
                let (a, b) = (top_of(j), top_of(j + 1));
                for q in 0..=STEPS {
                    let u = q as f64 / STEPS as f64;
                    pts.push((bez(sx, xm, xm, tx, u), bez(a, a, b, b, u)));
                }
            }
            for j in (0..last).rev() {
                let sx = x_of(col_of(j)) + node_w;
                let tx = x_of(col_of(j + 1));
                let xm = (sx + tx) / 2.0;
                let (a, b) = (top_of(j) + th, top_of(j + 1) + th);
                for q in (0..=STEPS).rev() {
                    let u = q as f64 / STEPS as f64;
                    pts.push((bez(sx, xm, xm, tx, u), bez(a, a, b, b, u)));
                }
            }
            pts
        };
        let fill = match mode {
            "destination" => color(l.t),
            "fixed" => fixed.clone(),
            _ => color(l.s),
        };
        let hit = json!({"link": li, "source": s.name, "target": t.name, "sourceLabel": s.label, "targetLabel": t.label, "value": l.value});
        out.hit(l.first as i64, hit.clone());
        let full = poly(&ribbon(th), &fill, opacity);
        if !lit {
            out.prims.push(full);
            continue;
        }
        // highlighted: the full ribbon dimmed and its highlighted part along its top edge
        out.prims.push(crate::look::dim(full));
        if let Some(hv) = crate::look::part(l.value, l.hl) {
            out.hit(l.first as i64, hit);
            out.prims.push(poly(&ribbon(hv * ky), &fill, opacity));
        }
    }
    for &i in &drawn {
        let Some(x) = nodes.get(i) else { continue };
        let mut hit = json!({"node": i, "name": x.name, "label": x.label, "value": x.value(), "inflow": x.inflow, "outflow": x.outflow});
        // highlighted: a node none of whose links is highlighted is dimmed
        if lit
            && !links
                .iter()
                .any(|l| l.kept && (l.s == i || l.t == i) && l.hl.is_some())
        {
            hit["fade"] = json!(true);
        }
        out.hit(x.first as i64, hit);
        out.prims
            .push(k.bar(x_of(x.col), x.y, node_w, x.h, &color(i), ""));
    }
    let weight = if k.bold() { 600 } else { 400 };
    if show_labels {
        // (node, y, text) of each label that fits
        let mut labs: Vec<(usize, f64, String)> = Vec::new();
        let y_of = |x: &Node| x.centre().max(size / 2.0).min(h - size / 2.0);
        for &i in &drawn {
            let Some(x) = nodes.get(i) else { continue };
            let y = y_of(x);
            let mut avail = step - node_w - 8.0;
            if ncols == 1 {
                avail = x1 - (x_of(x.col) + node_w + 4.0);
            } else if x.col + 2 >= ncols {
                // the last column's labels (on the left of their nodes) and the one before's
                // share the gap between them: each takes half of it only where a label of the
                // other column faces it
                let facing = if x.col + 1 == ncols { ncols - 2 } else { ncols - 1 };
                let near = cols.get(facing).is_some_and(|c| {
                    c.iter()
                        .any(|&j| j < n && nodes.get(j).is_some_and(|o| (y_of(o) - y).abs() < size))
                });
                if near {
                    avail = (step - node_w) / 2.0 - 6.0;
                }
            }
            let label = fit(&x.label, size, avail);
            if !label.is_empty() {
                labs.push((i, y, label));
            }
        }
        // in each column, the larger nodes' labels first: one closer than its size to a label
        // kept is left out
        let value_of = |j: usize| labs.get(j).and_then(|l| nodes.get(l.0)).map_or(0.0, Node::value);
        let mut order: Vec<usize> = (0..labs.len()).collect();
        order.sort_by(|&a, &b| {
            value_of(b)
                .partial_cmp(&value_of(a))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.cmp(&b))
        });
        let mut kept_y: Vec<Vec<f64>> = vec![Vec::new(); ncols];
        let mut keep = vec![false; labs.len()];
        for j in order {
            let Some((i, y, _)) = labs.get(j) else { continue };
            let Some(ys) = nodes.get(*i).and_then(|x| kept_y.get_mut(x.col)) else {
                continue;
            };
            if ys.iter().all(|k| (k - y).abs() >= size) {
                ys.push(*y);
                if let Some(f) = keep.get_mut(j) {
                    *f = true;
                }
            }
        }
        for ((i, y, label), kept) in labs.into_iter().zip(keep) {
            let Some(x) = nodes.get(i).filter(|_| kept) else {
                continue;
            };
            let nx = x_of(x.col);
            let (lx, anchor) = if ncols > 1 && x.col + 1 == ncols {
                (nx - 4.0, "end")
            } else {
                (nx + node_w + 4.0, "start")
            };
            out.prims
                .push(text_w(lx, y, label, size, k.label, anchor, 0.0, weight));
        }
    }
    if with_note {
        let msg = fit(&note(broken, fr), k.tick, iw);
        if !msg.is_empty() {
            out.prims
                .push(text(PAD, h - PAD - 5.0, msg, k.tick, k.text, "start"));
        }
    }
    out
}
