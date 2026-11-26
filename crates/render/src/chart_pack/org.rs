//! BIcraft Org Chart (docs/formats/chart-pack.md "Organization chart"): a hierarchy from id and
//! parent id columns drawn as a compact tidy tree, top-down or left to right (or with the leaves in equal slots),
//! each node a box with its label (and value), collapsible. The Rust twin of `crates/engine/src/chart_pack/org.ts`.

use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};

use super::{
    Out, PAD, cell_label, cell_num, cell_text, circle, fit, highlighted, hl_num, line, look_of, opt_bool,
    opt_color, opt_num, opt_texts, options_of, rect, rows_of, text,
};
use crate::look::{mix, text_w};
use crate::spec::{Prim, fmt_auto};

/// Most nodes read.
pub const MAX_NODES: usize = 1000;
/// Most ids kept in the toggled list.
pub const MAX_TOGGLED: usize = 500;

/// A node's colours: its own, its card's fill and edge, the gradient's top, its inks.
struct Paint {
    color: String,
    fill: String,
    edge: String,
    grad: String,
    ink: String,
    value_ink: String,
}

struct Node {
    id: String,
    label: String,
    value: Option<f64>,
    row: usize,
    parent: Option<usize>,
    kids: Vec<usize>,
    tree: Vec<usize>,
    depth: usize,
    shown: Vec<usize>,
    x: f64,
}

/// Places the subtrees of `list` left to right, each as far left as its contour allows: one unit
/// between any two nodes of a level. `lc` and `rc` are each subtree's left and right contours
/// (by depth below its root, relative to the root); `off` gets each root's place, relative to
/// the first. Returns the merged contours.
fn place(list: &[usize], lc: &[Vec<f64>], rc: &[Vec<f64>], off: &mut [f64]) -> (Vec<f64>, Vec<f64>) {
    let mut left: Vec<f64> = Vec::new();
    let mut right: Vec<f64> = Vec::new();
    let empty: Vec<f64> = Vec::new();
    for (i, &k) in list.iter().enumerate() {
        let l = lc.get(k).unwrap_or(&empty);
        let r = rc.get(k).unwrap_or(&empty);
        let mut s = 0.0;
        if i > 0 {
            for (d, (a, b)) in right.iter().zip(l).enumerate() {
                let need = a + 1.0 - b;
                s = if d == 0 { need } else { f64::max(s, need) };
            }
        }
        if let Some(o) = off.get_mut(k) {
            *o = s;
        }
        for (d, v) in r.iter().enumerate() {
            match right.get_mut(d) {
                Some(x) => *x = s + v,
                None => right.push(s + v),
            }
        }
        let start = left.len();
        for v in l.iter().skip(start) {
            left.push(s + v);
        }
    }
    (left, right)
}

/// The compact tidy tree: children before parents, each node's children placed by [`place`] and
/// the node at the middle of its first and last child, then the roots placed the same way. Sets
/// every shown node's `x` (the leftmost 0) and returns the width in units.
fn tidy(nodes: &mut [Node], vis: &[usize], roots: &[usize]) -> f64 {
    let mut lc: Vec<Vec<f64>> = vec![Vec::new(); nodes.len()];
    let mut rc: Vec<Vec<f64>> = vec![Vec::new(); nodes.len()];
    let mut off: Vec<f64> = vec![0.0; nodes.len()];
    for &n in vis.iter().rev() {
        let Some(kids) = nodes.get(n).map(|node| node.shown.clone()) else {
            continue;
        };
        let (l, r) = if kids.is_empty() {
            (vec![0.0], vec![0.0])
        } else {
            let (l, r) = place(&kids, &lc, &rc, &mut off);
            let at = |k: Option<&usize>| k.and_then(|k| off.get(*k)).copied().unwrap_or(0.0);
            let mid = (at(kids.first()) + at(kids.last())) / 2.0;
            for k in &kids {
                if let Some(o) = off.get_mut(*k) {
                    *o -= mid;
                }
            }
            (
                std::iter::once(0.0).chain(l.iter().map(|v| v - mid)).collect(),
                std::iter::once(0.0).chain(r.iter().map(|v| v - mid)).collect(),
            )
        };
        if let Some(x) = lc.get_mut(n) {
            *x = l;
        }
        if let Some(x) = rc.get_mut(n) {
            *x = r;
        }
    }
    place(roots, &lc, &rc, &mut off);
    for &n in roots {
        if let (Some(node), Some(o)) = (nodes.get_mut(n), off.get(n)) {
            node.x = *o;
        }
    }
    for &n in vis {
        let Some((x, kids)) = nodes.get(n).map(|node| (node.x, node.shown.clone())) else {
            continue;
        };
        for k in kids {
            if let (Some(kn), Some(o)) = (nodes.get_mut(k), off.get(k)) {
                kn.x = x + o;
            }
        }
    }
    let lo = vis
        .iter()
        .filter_map(|n| nodes.get(*n))
        .map(|n| n.x)
        .fold(f64::INFINITY, f64::min);
    let hi = vis
        .iter()
        .filter_map(|n| nodes.get(*n))
        .map(|n| n.x)
        .fold(f64::NEG_INFINITY, f64::max);
    for &n in vis {
        if let Some(node) = nodes.get_mut(n) {
            node.x -= lo;
        }
    }
    hi - lo + 1.0
}

/// Lays out `{rows: [[id, parent, label, value]], options}` in a `w`×`h` visual.
pub fn layout(input: &Value, w: f64, h: f64) -> Out {
    let o = options_of(input);
    let lk = look_of(input);
    let own = opt_color(o, "color", "");
    let color0 = if own.is_empty() { lk.color(0) } else { own.clone() };
    // Modern and Bold give each branch under a root its own colour, unless one colour is set
    let by_branch = own.is_empty() && !lk.minimal();
    // Minimal: a solid box with white text; Modern: a tinted card with an accent strip and the
    // look's inks; Bold: a solid shaded card, its ink white or dark as reads best
    let modern = !lk.minimal() && !lk.bold();
    let tint = if lk.night {
        lk.surface.clone()
    } else {
        "#FFFFFF".to_string()
    };
    let paint = |color: String| -> Paint {
        let (fill, edge) = if modern {
            (mix(&color, &tint, 0.88), mix(&color, &tint, 0.6))
        } else {
            (color.clone(), mix(&color, "#000000", 0.2))
        };
        let grad = if lk.bold() {
            lk.grad_top(&color)
        } else {
            String::new()
        };
        let auto_ink = if lk.minimal() {
            "#FFFFFF".to_string()
        } else if modern {
            lk.label.to_string()
        } else {
            lk.on(if grad.is_empty() { &color } else { &grad }, &color)
        };
        let ink = opt_color(o, "textColor", &auto_ink);
        let value_ink = if modern && opt_color(o, "textColor", "").is_empty() {
            lk.text.to_string()
        } else {
            ink.clone()
        };
        Paint {
            color,
            fill,
            edge,
            grad,
            ink,
            value_ink,
        }
    };
    let (label_weight, link) = if lk.minimal() {
        (400, 1.0)
    } else {
        (lk.value_weight(), 1.5)
    };
    let size = opt_num(o, "fontSize", 8.0, 24.0, lk.name);
    let show_values = opt_bool(o, "showValues", true);
    let levels = opt_num(o, "levels", 0.0, 20.0, 0.0).floor() as usize;
    let toggled: HashSet<String> = opt_texts(o, "toggled", MAX_TOGGLED).into_iter().collect();
    let compact = opt_bool(o, "compact", true);
    let ltr = opt_bool(o, "leftToRight", false);
    let mut out = Out::default();
    let mut nodes: Vec<Node> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut parents: Vec<String> = Vec::new();
    for (i, r) in rows_of(input, MAX_NODES).into_iter().enumerate() {
        let id = cell_text(r.first());
        if id.is_empty() || index.contains_key(&id) {
            continue;
        }
        let label = cell_label(input, 2, r.get(2));
        index.insert(id.clone(), nodes.len());
        parents.push(cell_text(r.get(1)));
        nodes.push(Node {
            label: if label.is_empty() {
                cell_label(input, 0, r.first())
            } else {
                label
            },
            id,
            value: cell_num(r.get(3)),
            row: i,
            parent: None,
            kids: Vec::new(),
            tree: Vec::new(),
            depth: 0,
            shown: Vec::new(),
            x: 0.0,
        });
    }
    if nodes.is_empty() {
        return out;
    }
    for (i, p) in parents.iter().enumerate() {
        if let Some(&p) = index.get(p)
            && p != i
        {
            if let Some(n) = nodes.get_mut(i) {
                n.parent = Some(p);
            }
            if let Some(n) = nodes.get_mut(p) {
                n.kids.push(i);
            }
        }
    }
    // the tree: from each root (no known parent) in order, then from each node a cycle kept
    // unreached; a node's tree children are its children not reached yet
    let mut seen = vec![false; nodes.len()];
    let mut roots: Vec<usize> = Vec::new();
    for pass in 0..2 {
        for i in 0..nodes.len() {
            let has_parent = nodes.get(i).is_some_and(|n| n.parent.is_some());
            if seen.get(i).copied().unwrap_or(true) || (pass == 0 && has_parent) {
                continue;
            }
            roots.push(i);
            if let Some(s) = seen.get_mut(i) {
                *s = true;
            }
            let mut stack = vec![i];
            while let Some(n) = stack.pop() {
                let (kids, depth) = match nodes.get(n) {
                    Some(node) => (node.kids.clone(), node.depth),
                    None => continue,
                };
                let mut fresh = Vec::new();
                for k in kids {
                    if seen.get(k) == Some(&false) {
                        if let Some(s) = seen.get_mut(k) {
                            *s = true;
                        }
                        if let Some(kn) = nodes.get_mut(k) {
                            kn.depth = depth + 1;
                        }
                        fresh.push(k);
                    }
                }
                stack.extend(fresh.iter().rev());
                if let Some(node) = nodes.get_mut(n) {
                    node.tree = fresh;
                }
            }
        }
    }
    // what is shown: a node is open when its depth is within `levels` (0: all), toggled otherwise
    let mut vis: Vec<usize> = Vec::new();
    let mut stack: Vec<usize> = roots.iter().rev().copied().collect();
    let mut depths = 0usize;
    while let Some(n) = stack.pop() {
        let Some(node) = nodes.get_mut(n) else { continue };
        vis.push(n);
        depths = depths.max(node.depth + 1);
        let by_default = levels == 0 || node.depth + 1 < levels;
        let open = if toggled.contains(&node.id) {
            !by_default
        } else {
            by_default
        };
        node.shown = if open { node.tree.clone() } else { Vec::new() };
        stack.extend(node.shown.iter().rev());
    }
    let units = if compact {
        tidy(&mut nodes, &vis, &roots)
    } else {
        // the visible leaves in equal slots, left to right
        let mut leaves = 0usize;
        for &n in &vis {
            if let Some(node) = nodes.get_mut(n)
                && node.shown.is_empty()
            {
                node.x = leaves as f64;
                leaves += 1;
            }
        }
        let x_of = |nodes: &[Node], k: Option<&usize>| k.and_then(|k| nodes.get(*k)).map_or(0.0, |n| n.x);
        for &n in vis.iter().rev() {
            let (first, last) = match nodes.get(n) {
                Some(node) if !node.shown.is_empty() => {
                    (x_of(&nodes, node.shown.first()), x_of(&nodes, node.shown.last()))
                }
                _ => continue,
            };
            if let Some(node) = nodes.get_mut(n) {
                node.x = (first + last) / 2.0;
            }
        }
        leaves as f64
    };
    let aw = w - 2.0 * PAD;
    let ah = h - 2.0 * PAD;
    if aw < 8.0 || ah < 8.0 {
        return out;
    }
    // top-down: the leaves' slots across, the levels down; left to right: the slots down, the
    // levels across
    let (across, along) = if ltr { (ah, aw) } else { (aw, ah) };
    let slot = across / units;
    let lh = along / depths as f64;
    let (bw, bh) = if ltr {
        ((lh * 0.75).min(160.0), (slot * 0.85).min(44.0))
    } else {
        ((slot * 0.85).min(160.0), (lh * 0.6).min(44.0))
    };
    if bw < 2.0 || bh < 2.0 {
        return out;
    }
    // a node's box: its centre's x and its top
    let cx = |n: &Node| {
        if ltr {
            PAD + lh * n.depth as f64 + bw / 2.0
        } else {
            PAD + slot * (n.x + 0.5)
        }
    };
    let top = |n: &Node| {
        if ltr {
            PAD + slot * (n.x + 0.5) - bh / 2.0
        } else {
            PAD + lh * n.depth as f64
        }
    };
    lk.push_card(&mut out.prims, w, h);
    // each node's branch: the rank of its ancestor one level under a root (vis is in
    // preorder; -1 for a root)
    let mut branch: HashMap<usize, i64> = HashMap::new();
    let (mut cur, mut branches) = (-1i64, 0i64);
    for &n in &vis {
        match nodes.get(n).map(|x| x.depth) {
            Some(0) => cur = -1,
            Some(1) => {
                cur = branches;
                branches += 1;
            }
            _ => {}
        }
        branch.insert(n, cur);
    }
    // connectors: the parent's stem, the bar over its children, each child's stem (left to
    // right: from the parent's right edge, the bar left of the children)
    for &n in &vis {
        let Some(node) = nodes.get(n) else { continue };
        if node.shown.is_empty() {
            continue;
        }
        if ltr {
            let right = cx(node) + bw / 2.0;
            let mx = right + (lh - bw) / 2.0;
            let my = |m: &Node| top(m) + bh / 2.0;
            out.prims.push(line(right, my(node), mx, my(node), lk.axis, link));
            if node.shown.len() > 1
                && let (Some(a), Some(b)) = (
                    node.shown.first().and_then(|k| nodes.get(*k)),
                    node.shown.last().and_then(|k| nodes.get(*k)),
                )
            {
                out.prims.push(line(mx, my(a), mx, my(b), lk.axis, link));
            }
            for k in &node.shown {
                if let Some(kn) = nodes.get(*k) {
                    out.prims
                        .push(line(mx, my(kn), cx(kn) - bw / 2.0, my(kn), lk.axis, link));
                }
            }
            continue;
        }
        let mid = top(node) + bh + (lh - bh) / 2.0;
        out.prims
            .push(line(cx(node), top(node) + bh, cx(node), mid, lk.axis, link));
        if node.shown.len() > 1
            && let (Some(a), Some(b)) = (
                node.shown.first().and_then(|k| nodes.get(*k)),
                node.shown.last().and_then(|k| nodes.get(*k)),
            )
        {
            out.prims.push(line(cx(a), mid, cx(b), mid, lk.axis, link));
        }
        for k in &node.shown {
            if let Some(kn) = nodes.get(*k) {
                out.prims.push(line(cx(kn), mid, cx(kn), top(kn), lk.axis, link));
            }
        }
    }
    let toggles = bw >= 16.0 && bh >= 12.0;
    for &n in &vis {
        let Some(node) = nodes.get(n) else { continue };
        let (x, y) = (cx(node), top(node));
        let pc = match branch.get(&n) {
            Some(b) if by_branch => paint(lk.color((b + 1).max(0) as usize)),
            _ => paint(color0.clone()),
        };
        // highlighted: a node whose row is not is dimmed (docs/formats/bicraft-visuals.md "Highlights")
        if highlighted(input) && hl_num(input, node.row).is_none() {
            out.hit(node.row as i64, json!({"id": node.id, "fade": true}));
        } else {
            out.hit(node.row as i64, json!({"id": node.id}));
        }
        if lk.minimal() && !lk.rounded && !lk.shadow {
            out.prims.push(rect(x - bw / 2.0, y, bw, bh, &pc.color, 1.0));
        } else {
            let mut card = lk.card(x - bw / 2.0, y, bw, bh, &pc.fill, &pc.edge);
            if let Prim::Rect { deco, .. } = &mut card {
                deco.grad = pc.grad.clone();
            }
            out.prims.push(card);
            if modern && bw >= 16.0 {
                out.prims.push(lk.accent(x - bw / 2.0, y, 4.0, bh, &pc.color));
            }
        }
        let tw = bw - 8.0;
        if bh >= size + 2.0 {
            let value = match node.value {
                Some(v) if show_values && bh >= 2.0 * size + 6.0 => fmt_auto(v),
                _ => String::new(),
            };
            let label = fit(&node.label, size, tw);
            if !value.is_empty() {
                if !label.is_empty() {
                    out.prims.push(text_w(
                        x,
                        y + bh / 2.0 - size * 0.55,
                        label,
                        size,
                        &pc.ink,
                        "middle",
                        0.0,
                        label_weight,
                    ));
                }
                let vt = fit(&value, size - 1.0, tw);
                if !vt.is_empty() {
                    out.prims.push(text(
                        x,
                        y + bh / 2.0 + size * 0.6,
                        vt,
                        size - 1.0,
                        &pc.value_ink,
                        "middle",
                    ));
                }
            } else if !label.is_empty() {
                out.prims.push(text_w(
                    x,
                    y + bh / 2.0,
                    label,
                    size,
                    &pc.ink,
                    "middle",
                    0.0,
                    label_weight,
                ));
            }
        }
        if toggles && !node.tree.is_empty() {
            out.hit(node.row as i64, json!({"id": node.id, "toggle": true}));
            let sign = if node.shown.is_empty() { "+" } else { "−" };
            // on the box's bottom edge (left to right: its right edge)
            let (tx, ty) = if ltr {
                (x + bw / 2.0, y + bh / 2.0)
            } else {
                (x, y + bh)
            };
            if lk.minimal() {
                out.prims.push(circle(tx, ty, 6.0, "#605E5C", 1.0));
                out.prims
                    .push(text(tx, ty, sign.to_string(), 10.0, "#FFFFFF", "middle"));
            } else {
                out.prims.push(lk.dot(tx, ty, 6.5, &pc.color, 1.0));
                let on = lk.on(&pc.color, &pc.color);
                out.prims
                    .push(text_w(tx, ty, sign.to_string(), 11.0, &on, "middle", 0.0, 700));
            }
        }
    }
    out
}
