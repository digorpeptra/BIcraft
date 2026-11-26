//! The Model tab's diagram: where the table cards, their rows and the relationship lines go.
//!
//! A port of the live viewer's model view (`renderModel`, `tableCard` and `drawRels` in
//! `crates/view/src/page.html`), so the app places the cards where the viewer's page does:
//!
//! - a layered layout: each relationship points from its "many" side to its "one" side (from →
//!   to, swapped when the from side is "one" and the to side "many"); a table's layer is one more
//!   than the layers pointing at it (40 passes, at most layer 8); tables in no relationship go
//!   last; each layer is a column of at most 10 cards, in the order the tables were first seen;
//! - the CSS sizes of the viewer's cards (236 px wide, a 33 px header, 22 px rows, 12 rows per
//!   section until "Show all", 96 px between columns, 22 px between cards);
//! - a relationship is a cubic curve from the row of its column (the header when the row is not
//!   shown) on the facing sides of the two cards, with the cardinality marks (`*`, `1`) at both
//!   ends and the filter-direction arrows at the middle (two for both directions).
//!
//! Pure: JSON in (the snapshot's `model`), positions out, in the viewer's CSS pixels. The window
//! (`model_view`) scales and pans them. Bounded: at most [`MAX_TABLES`] cards and [`MAX_RELS`]
//! lines, [`ROWS_EXPANDED`] rows per section.

use std::collections::{HashMap, HashSet};

use egui::{Pos2, Rect, Vec2, pos2, vec2};
use serde_json::Value;

/// Table cards drawn (the viewer's `MAX_TABLES`).
pub const MAX_TABLES: usize = 300;
/// Relationship lines drawn (`MAX_RELS`).
pub const MAX_RELS: usize = 1500;
/// Columns or measures shown per card before "Show all" (`ROWS_COLLAPSED`).
pub const ROWS_COLLAPSED: usize = 12;
/// … and after (`ROWS_EXPANDED`).
pub const ROWS_EXPANDED: usize = 400;
/// Cards per column of a layer.
pub const PER_COLUMN: usize = 10;
/// `.tcard { width: 236px }`.
pub const CARD_W: f32 = 236.0;
/// `.mcanvas { gap: 96px }`.
pub const COLUMN_GAP: f32 = 96.0;
/// `.mlayer { gap: 22px }`.
pub const CARD_GAP: f32 = 22.0;
/// `.mcanvas { padding: 24px 28px 40px }`.
pub const PAD_TOP: f32 = 24.0;
pub const PAD_X: f32 = 28.0;
pub const PAD_BOTTOM: f32 = 40.0;
/// `.th`: 7 px padding around a 13 px line (1.4) and a 1 px bottom border.
pub const HEAD_H: f32 = 33.0;
/// `.trow { height: 22px }`.
pub const ROW_H: f32 = 22.0;
/// `.tsec { padding: 3px 0 }`.
pub const SEC_PAD: f32 = 3.0;
/// The "Show all" button (`.more`: 4 px padding, a 12 px line, a 1 px top border).
pub const MORE_H: f32 = 26.0;
/// The "truncated" note.
pub const NOTE_H: f32 = 20.0;
/// The card's border.
pub const BORDER: f32 = 1.0;
/// Points sampled along a relationship's curve (drawing and hit testing).
pub const CURVE_STEPS: usize = 32;

const MAX_LAYER: usize = 8;
const UNLINKED: usize = 99;
const PASSES: usize = 40;
/// Names remembered by [`Order`] before it starts over.
const MAX_ORDER: usize = 100_000;

/// The order in which table names were first seen (the viewer's `UI.order`): cards keep their
/// place when tables are added or re-read.
#[derive(Debug, Default, Clone)]
pub struct Order(HashMap<String, usize>);

impl Order {
    pub fn see(&mut self, name: &str) {
        if self.0.len() >= MAX_ORDER && !self.0.contains_key(name) {
            self.0.clear();
        }
        let n = self.0.len();
        self.0.entry(name.to_string()).or_insert(n);
    }

    pub fn rank(&self, name: &str) -> usize {
        self.0.get(name).copied().unwrap_or(0)
    }
}

/// A column or measure row of a card.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// Its index in the table's `columns` or `measures` array.
    pub index: usize,
    pub rect: Rect,
}

/// A table card.
#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    /// Its index in the model's `tables` array.
    pub table: usize,
    pub name: String,
    /// The border box.
    pub rect: Rect,
    pub head: Rect,
    pub columns: Vec<Row>,
    pub measures: Vec<Row>,
    /// The "Show all" / "Show less" button.
    pub more: Option<Rect>,
    /// The "truncated" note.
    pub note: Option<Rect>,
    pub n_columns: usize,
    pub n_measures: usize,
    pub expanded: bool,
    /// Where the columns and the measures sections start (their dashed separator).
    pub measures_top: Option<f32>,
}

/// A relationship line.
#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    /// Its index in the model's `relationships` array.
    pub rel: usize,
    /// The cubic curve's points.
    pub curve: [Pos2; 4],
    /// The cardinality marks: the text's left end at its baseline, and the text.
    pub from_mark: Option<(Pos2, &'static str)>,
    pub to_mark: Option<(Pos2, &'static str)>,
    /// The filter-direction triangles.
    pub arrows: Vec<[Pos2; 3]>,
    pub inactive: bool,
    /// The curve's samples ([`CURVE_STEPS`] + 1 points).
    pub path: Vec<Pos2>,
    /// The bounding box of the samples.
    pub bounds: Rect,
}

/// The whole diagram, in the viewer's CSS pixels.
#[derive(Debug, Clone, Default)]
pub struct Diagram {
    pub cards: Vec<Card>,
    pub links: Vec<Link>,
    pub size: Vec2,
    /// Tables in the model (cards drawn: at most [`MAX_TABLES`]).
    pub total_tables: usize,
    /// Diagram point = file's diagram point (`model.diagram`, `diagram.move`) + `shift`; zero
    /// when no card has a saved place.
    pub shift: Vec2,
}

/// What a point of the diagram is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    /// The header of card `card`.
    Table(usize),
    /// Column `index` of card `card`.
    Column {
        card: usize,
        index: usize,
    },
    Measure {
        card: usize,
        index: usize,
    },
    /// The card's "Show all" button.
    More(usize),
    /// Elsewhere on the card.
    Card(usize),
    /// Link `link` (index in [`Diagram::links`]).
    Rel(usize),
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

/// The objects of array `k` of `v`, with their indices.
pub fn objects<'a>(v: &'a Value, k: &str) -> impl Iterator<Item = (usize, &'a Value)> + 'a {
    v.get(k)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
        .filter(|(_, x)| x.is_object())
}

fn card_of(c: &str) -> &'static str {
    match c.to_ascii_lowercase().as_str() {
        "one" => "1",
        "many" => "*",
        _ => "",
    }
}

/// A point of the cubic curve `c` at `t`.
pub fn cubic(c: &[Pos2; 4], t: f32) -> Pos2 {
    let u = 1.0 - t;
    let (a, b, cc, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    pos2(
        a * c[0].x + b * c[1].x + cc * c[2].x + d * c[3].x,
        a * c[0].y + b * c[1].y + cc * c[2].y + d * c[3].y,
    )
}

/// The distance from `p` to the segment `a`–`b`.
pub fn segment_distance(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let len2 = ab.length_sq();
    let t = if len2 > 0.0 {
        ((p - a).dot(ab) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (a + ab * t - p).length()
}

/// Lays out `model` (the snapshot's `model` object). `order` remembers the order tables were
/// first seen; `expanded` holds the names of the cards showing all their rows.
pub fn layout(model: &Value, order: &mut Order, expanded: &HashSet<String>) -> Diagram {
    layout_at(model, order, expanded, &HashMap::new(), None)
}

/// The saved positions of the snapshot's model diagram (`model.diagram.nodes`: the top-left
/// corner of each table's card, in the file's coordinates), at most one per table.
pub fn saved_positions(model: &Value) -> HashMap<String, Pos2> {
    let num = |v: &Value, k: &str| {
        v.get(k)
            .and_then(Value::as_f64)
            .filter(|x| x.is_finite() && x.abs() <= MAX_COORD)
            .map(|x| x as f32)
    };
    let mut out = HashMap::new();
    for n in model
        .pointer("/diagram/nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(MAX_TABLES.saturating_mul(4))
    {
        if let (Some(t), Some(x), Some(y)) =
            (n.get("table").and_then(Value::as_str), num(n, "x"), num(n, "y"))
        {
            out.insert(t.to_string(), pos2(x, y));
        }
    }
    out
}

/// Largest saved coordinate used.
const MAX_COORD: f64 = 1_000_000.0;

/// [`layout`] with the cards of the file's model diagram at their saved places, and those of
/// `overrides` (a card being dragged, one just moved) at theirs, in the file's coordinates. The
/// others keep the layered layout, to the right of the placed ones. The diagram is the file's
/// coordinates plus [`Diagram::shift`]: `shift` when given (it stays put during a drag), else so
/// that the cards start at the padding.
pub fn layout_at(
    model: &Value,
    order: &mut Order,
    expanded: &HashSet<String>,
    overrides: &HashMap<String, Pos2>,
    shift: Option<Vec2>,
) -> Diagram {
    let mut d = auto_layout(model, order, expanded);
    let mut placed = saved_positions(model);
    for (k, v) in overrides {
        placed.insert(k.clone(), *v);
    }
    if !d.cards.iter().any(|c| placed.contains_key(&c.name)) {
        return d;
    }
    // the placed cards where the file says, the others after them
    let mut at = Rect::NOTHING;
    let mut rest = Rect::NOTHING;
    for c in &mut d.cards {
        match placed.get(&c.name) {
            Some(p) => {
                let delta = *p - c.rect.min;
                move_card(c, delta);
                at = at.union(c.rect);
            }
            None => rest = rest.union(c.rect),
        }
    }
    if rest.is_positive() {
        let delta = vec2(at.max.x + COLUMN_GAP - rest.min.x, at.min.y - rest.min.y);
        for c in d.cards.iter_mut().filter(|c| !placed.contains_key(&c.name)) {
            move_card(c, delta);
        }
    }
    let all = d.cards.iter().fold(Rect::NOTHING, |b, c| b.union(c.rect));
    let shift = shift
        .filter(|v| v.x.is_finite() && v.y.is_finite())
        .unwrap_or(vec2(PAD_X - all.min.x, PAD_TOP - all.min.y));
    for c in &mut d.cards {
        move_card(c, shift);
    }
    d.shift = shift;
    let all = all.translate(shift);
    d.size = vec2((all.max.x + PAD_X).max(0.0), (all.max.y + PAD_BOTTOM).max(0.0));
    d.links = links(model, &d.cards);
    d
}

/// Moves a card and everything in it by `delta`.
fn move_card(c: &mut Card, delta: Vec2) {
    c.rect = c.rect.translate(delta);
    c.head = c.head.translate(delta);
    for r in c.columns.iter_mut().chain(c.measures.iter_mut()) {
        r.rect = r.rect.translate(delta);
    }
    c.more = c.more.map(|r| r.translate(delta));
    c.note = c.note.map(|r| r.translate(delta));
    c.measures_top = c.measures_top.map(|y| y + delta.y);
}

/// The viewer's layered layout.
fn auto_layout(model: &Value, order: &mut Order, expanded: &HashSet<String>) -> Diagram {
    let tables: Vec<(usize, &Value)> = objects(model, "tables").collect();
    for (_, t) in &tables {
        order.see(s(t, "name"));
    }
    let drawn = &tables[..tables.len().min(MAX_TABLES)];
    let known: HashSet<&str> = drawn.iter().map(|(_, t)| s(t, "name")).collect();
    let rels: Vec<(usize, &Value)> = objects(model, "relationships").take(MAX_RELS).collect();

    // layers: the "many" side to the left, the tables it points to further right
    let mut edges: Vec<(&str, &str)> = Vec::new();
    for (_, r) in &rels {
        let (mut a, mut b) = (s(r, "from_table"), s(r, "to_table"));
        if s(r, "from_cardinality").eq_ignore_ascii_case("one")
            && s(r, "to_cardinality").eq_ignore_ascii_case("many")
        {
            std::mem::swap(&mut a, &mut b);
        }
        if known.contains(a) && known.contains(b) && a != b {
            edges.push((a, b));
        }
    }
    let mut layer: HashMap<&str, usize> = known.iter().map(|n| (*n, 0)).collect();
    let linked: HashSet<&str> = edges.iter().flat_map(|(a, b)| [*a, *b]).collect();
    for _ in 0..PASSES {
        let mut changed = false;
        for (a, b) in &edges {
            let want = (layer.get(a).copied().unwrap_or(0).saturating_add(1)).min(MAX_LAYER);
            if layer.get(b).copied().unwrap_or(0) < want {
                layer.insert(b, want);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let mut by_layer: Vec<(usize, Vec<(usize, &Value)>)> = Vec::new();
    for (i, t) in drawn {
        let n = s(t, "name");
        let l = if linked.contains(n) {
            layer.get(n).copied().unwrap_or(0)
        } else {
            UNLINKED
        };
        match by_layer.iter_mut().find(|(k, _)| *k == l) {
            Some((_, list)) => list.push((*i, *t)),
            None => by_layer.push((l, vec![(*i, *t)])),
        }
    }
    by_layer.sort_by_key(|(l, _)| *l);
    let mut columns: Vec<Vec<(usize, &Value)>> = Vec::new();
    for (_, mut list) in by_layer {
        // stable, as Array.prototype.sort
        list.sort_by_key(|(_, t)| order.rank(s(t, "name")));
        for chunk in list.chunks(PER_COLUMN) {
            columns.push(chunk.to_vec());
        }
    }

    // the cards
    let mut cards = Vec::new();
    let mut height: f32 = 0.0;
    for (ci, col) in columns.iter().enumerate() {
        let x = PAD_X + ci as f32 * (CARD_W + COLUMN_GAP);
        let mut y = PAD_TOP;
        for (ti, t) in col {
            let card = card(*ti, t, x, y, expanded);
            y = card.rect.max.y + CARD_GAP;
            height = height.max(card.rect.max.y);
            cards.push(card);
        }
    }
    let n = columns.len() as f32;
    let width = if columns.is_empty() {
        0.0
    } else {
        PAD_X * 2.0 + n * CARD_W + (n - 1.0) * COLUMN_GAP
    };
    let size = vec2(
        width,
        if cards.is_empty() {
            0.0
        } else {
            height + PAD_BOTTOM
        },
    );

    let links = links(model, &cards);
    Diagram {
        cards,
        links,
        size,
        total_tables: tables.len(),
        shift: Vec2::ZERO,
    }
}

/// The relationship lines between `cards`.
fn links(model: &Value, cards: &[Card]) -> Vec<Link> {
    let rels: Vec<(usize, &Value)> = objects(model, "relationships").take(MAX_RELS).collect();
    let mut by_name: HashMap<&str, usize> = HashMap::new();
    for (k, c) in cards.iter().enumerate() {
        by_name.insert(c.name.as_str(), k);
    }
    let mut links = Vec::new();
    for (ri, r) in &rels {
        let (Some(&a), Some(&b)) = (by_name.get(s(r, "from_table")), by_name.get(s(r, "to_table"))) else {
            continue;
        };
        let (Some(ca), Some(cb)) = (cards.get(a), cards.get(b)) else {
            continue;
        };
        let (Some(ta), Some(tb)) = (tables_get(model, ca.table), tables_get(model, cb.table)) else {
            continue;
        };
        let ya = anchor(ca, ta, s(r, "from_column"));
        let yb = anchor(cb, tb, s(r, "to_column"));
        let (ra, rb) = (ca.rect, cb.rect);
        let (xa, xb, da, db) = if rb.min.x >= ra.max.x {
            (ra.max.x, rb.min.x, 1.0, -1.0)
        } else if ra.min.x >= rb.max.x {
            (ra.min.x, rb.max.x, -1.0, 1.0)
        } else {
            (ra.max.x, rb.max.x, 1.0, 1.0)
        };
        let bend = ((xb - xa).abs() / 2.0).max(36.0);
        let curve = [
            pos2(xa, ya),
            pos2(xa + da * bend, ya),
            pos2(xb + db * bend, yb),
            pos2(xb, yb),
        ];
        let (fc, tc) = (s(r, "from_cardinality"), s(r, "to_cardinality"));
        let cf = s(r, "cross_filter").to_ascii_lowercase();
        // the middle of the curve and its tangent there
        let mid = cubic(&curve, 0.5);
        let tan = (curve[3] - curve[0]) + (curve[2] - curve[1]);
        let len = tan.length();
        let u = if len > 0.0 { tan / len } else { vec2(1.0, 0.0) };
        let nrm = vec2(-u.y, u.x);
        let arrow = |dir: f32, off: f32| -> [Pos2; 3] {
            let c = mid + u * off;
            [
                c + u * (dir * 5.0),
                c - u * (dir * 4.0) + nrm * 4.5,
                c - u * (dir * 4.0) - nrm * 4.5,
            ]
        };
        // filters flow from the "one" side to the "many" side (towards `from` by default)
        let towards_from = !(fc.eq_ignore_ascii_case("one") && tc.eq_ignore_ascii_case("many"));
        let arrows = if cf == "both" {
            vec![arrow(-1.0, -7.0), arrow(1.0, 7.0)]
        } else {
            vec![arrow(if towards_from { -1.0 } else { 1.0 }, 0.0)]
        };
        let mark = |x: f32, d: f32, y: f32, c: &str| -> Option<(Pos2, &'static str)> {
            let t = card_of(c);
            (!t.is_empty()).then(|| (pos2(x + d * 8.0 - if d < 0.0 { 7.0 } else { 0.0 }, y - 5.0), t))
        };
        let path: Vec<Pos2> = (0..=CURVE_STEPS)
            .map(|k| cubic(&curve, k as f32 / CURVE_STEPS as f32))
            .collect();
        let bounds = path
            .iter()
            .fold(Rect::NOTHING, |b, p| b.union(Rect::from_min_max(*p, *p)));
        links.push(Link {
            rel: *ri,
            curve,
            from_mark: mark(xa, da, ya, fc),
            to_mark: mark(xb, db, yb, tc),
            arrows,
            inactive: r.get("active").and_then(Value::as_bool) == Some(false),
            path,
            bounds,
        });
    }
    links
}

fn tables_get(model: &Value, i: usize) -> Option<&Value> {
    model
        .get("tables")
        .and_then(Value::as_array)
        .and_then(|a| a.get(i))
}

/// The y of a relationship's end: the middle of the column's row when it is shown (the last
/// shown row of that name, as the viewer's map), else of the header.
fn anchor(card: &Card, table: &Value, column: &str) -> f32 {
    let cols = table.get("columns").and_then(Value::as_array);
    card.columns
        .iter()
        .rev()
        .find(|r| {
            cols.and_then(|c| c.get(r.index))
                .is_some_and(|c| s(c, "name") == column)
        })
        .map_or(card.head.center().y, |r| r.rect.center().y)
}

/// One card at (`x`, `y`).
fn card(table: usize, t: &Value, x: f32, y: f32, expanded: &HashSet<String>) -> Card {
    let name = s(t, "name").to_string();
    let cols: Vec<usize> = objects(t, "columns").map(|(i, _)| i).collect();
    let meas: Vec<usize> = objects(t, "measures").map(|(i, _)| i).collect();
    let is_expanded = expanded.contains(&name);
    let lim = if is_expanded {
        ROWS_EXPANDED
    } else {
        ROWS_COLLAPSED
    };
    let inner = |top: f32, h: f32| Rect::from_min_size(pos2(x + BORDER, top), vec2(CARD_W - 2.0 * BORDER, h));
    let head = inner(y + BORDER, HEAD_H);
    let mut cur = head.max.y;
    let rows = |list: &[usize], cur: &mut f32| -> Vec<Row> {
        *cur += SEC_PAD;
        let out = list
            .iter()
            .take(lim)
            .map(|&index| {
                let r = Row {
                    index,
                    rect: inner(*cur, ROW_H),
                };
                *cur += ROW_H;
                r
            })
            .collect();
        *cur += SEC_PAD;
        out
    };
    let columns = if cols.is_empty() {
        Vec::new()
    } else {
        rows(&cols, &mut cur)
    };
    let mut measures_top = None;
    let measures = if meas.is_empty() {
        Vec::new()
    } else {
        if !cols.is_empty() {
            measures_top = Some(cur);
            cur += BORDER;
        }
        rows(&meas, &mut cur)
    };
    let more = (cols.len() > ROWS_COLLAPSED || meas.len() > ROWS_COLLAPSED).then(|| {
        let r = inner(cur, MORE_H);
        cur += MORE_H;
        r
    });
    let note = (t.get("truncated").and_then(Value::as_bool) == Some(true)).then(|| {
        let r = inner(cur, NOTE_H);
        cur += NOTE_H;
        r
    });
    Card {
        table,
        name,
        rect: Rect::from_min_max(pos2(x, y), pos2(x + CARD_W, cur + BORDER)),
        head,
        columns,
        measures,
        more,
        note,
        n_columns: cols.len(),
        n_measures: meas.len(),
        expanded: is_expanded,
        measures_top,
    }
}

impl Diagram {
    /// What `p` is on; `tol` is the distance (diagram pixels) within which a line counts.
    pub fn hit(&self, p: Pos2, tol: f32) -> Option<Hit> {
        // cards are drawn over the lines: the last card drawn wins
        for (k, c) in self.cards.iter().enumerate().rev() {
            if !c.rect.contains(p) {
                continue;
            }
            if c.head.contains(p) {
                return Some(Hit::Table(k));
            }
            if let Some(r) = c.columns.iter().find(|r| r.rect.contains(p)) {
                return Some(Hit::Column {
                    card: k,
                    index: r.index,
                });
            }
            if let Some(r) = c.measures.iter().find(|r| r.rect.contains(p)) {
                return Some(Hit::Measure {
                    card: k,
                    index: r.index,
                });
            }
            if c.more.is_some_and(|m| m.contains(p)) {
                return Some(Hit::More(k));
            }
            return Some(Hit::Card(k));
        }
        let mut best: Option<(f32, usize)> = None;
        for (k, l) in self.links.iter().enumerate() {
            if !l.bounds.expand(tol).contains(p) {
                continue;
            }
            let d = l
                .path
                .windows(2)
                .map(|w| match w {
                    [a, b] => segment_distance(p, *a, *b),
                    _ => f32::INFINITY,
                })
                .fold(f32::INFINITY, f32::min);
            if d <= tol && best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, k));
            }
        }
        best.map(|(_, k)| Hit::Rel(k))
    }

    /// The card of table `name` (the last one, as the viewer's map).
    pub fn card_named(&self, name: &str) -> Option<usize> {
        self.cards.iter().rposition(|c| c.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn table(name: &str, cols: &[&str], measures: usize) -> Value {
        json!({
            "name": name,
            "hidden": false,
            "columns": cols.iter().map(|c| json!({"name": c, "data_type": "string"})).collect::<Vec<_>>(),
            "measures": (0..measures).map(|i| json!({"name": format!("M{i}"), "expression": "1"})).collect::<Vec<_>>(),
        })
    }

    fn rel(from: &str, to: &str, extra: Value) -> Value {
        let (ft, fc) = from.split_once('.').unwrap();
        let (tt, tc) = to.split_once('.').unwrap();
        let mut r = json!({"from_table": ft, "from_column": fc, "to_table": tt, "to_column": tc,
                           "active": true, "cross_filter": "single",
                           "from_cardinality": "many", "to_cardinality": "one"});
        if let (Some(o), Value::Object(e)) = (r.as_object_mut(), extra) {
            o.extend(e);
        }
        r
    }

    /// A star: Sales → Product → Category, Sales → Date; Notes is in no relationship.
    fn star() -> Value {
        json!({
            "name": "Star",
            "tables": [
                table("Notes", &["Text"], 0),
                table("Sales", &["ProductKey", "DateKey", "Amount"], 2),
                table("Product", &["ProductKey", "Name", "CategoryKey"], 0),
                table("Date", &["DateKey", "Year"], 0),
                table("Category", &["CategoryKey", "Label"], 0),
            ],
            "relationships": [
                rel("Sales.ProductKey", "Product.ProductKey", json!({})),
                rel("Sales.DateKey", "Date.DateKey", json!({"active": false})),
                // written from the one side: swapped by the layout
                rel("Category.CategoryKey", "Product.CategoryKey",
                    json!({"from_cardinality": "one", "to_cardinality": "many", "cross_filter": "both"})),
            ],
        })
    }

    fn at(d: &Diagram, name: &str) -> Rect {
        d.cards[d.card_named(name).unwrap()].rect
    }

    /// The positions the viewer's algorithm gives (worked by hand from `renderModel` and the CSS).
    #[test]
    fn star_positions_match_the_viewer() {
        let d = layout(&star(), &mut Order::default(), &HashSet::new());
        let col = |i: f32| PAD_X + i * (CARD_W + COLUMN_GAP);
        // layer 0: Sales; layer 1: Product, Date (model order); layer 2: Category; then Notes
        assert_eq!(at(&d, "Sales").min, pos2(col(0.0), PAD_TOP));
        assert_eq!(at(&d, "Product").min, pos2(col(1.0), PAD_TOP));
        // Product: border, header, 3 rows with the section padding, border; then the 22 px gap
        let product_h = 1.0 + 33.0 + 3.0 + 3.0 * 22.0 + 3.0 + 1.0;
        assert_eq!(at(&d, "Product").height(), product_h);
        assert_eq!(at(&d, "Date").min, pos2(col(1.0), PAD_TOP + product_h + CARD_GAP));
        assert_eq!(at(&d, "Category").min, pos2(col(2.0), PAD_TOP));
        assert_eq!(at(&d, "Notes").min, pos2(col(3.0), PAD_TOP));
        // Sales: 3 columns, a dashed border, 2 measures
        let sales_h = 1.0 + 33.0 + (6.0 + 3.0 * 22.0) + 1.0 + (6.0 + 2.0 * 22.0) + 1.0;
        assert_eq!(at(&d, "Sales").height(), sales_h);
        assert_eq!(d.size.x, PAD_X * 2.0 + 4.0 * CARD_W + 3.0 * COLUMN_GAP);
        let date_bottom = PAD_TOP + product_h + CARD_GAP + (1.0 + 33.0 + 6.0 + 2.0 * 22.0 + 1.0);
        assert_eq!(d.size.y, date_bottom + PAD_BOTTOM);
    }

    #[test]
    fn lines_go_from_row_to_row_with_the_viewers_marks() {
        let d = layout(&star(), &mut Order::default(), &HashSet::new());
        assert_eq!(d.links.len(), 3);
        let sales = at(&d, "Sales");
        let product = at(&d, "Product");
        let l = &d.links[0];
        // from the right side of Sales, row ProductKey (the first), to the left of Product
        let row0 = sales.min.y + 1.0 + 33.0 + 3.0 + 11.0;
        assert_eq!(l.curve[0], pos2(sales.max.x, row0));
        assert_eq!(
            l.curve[3],
            pos2(product.min.x, product.min.y + 1.0 + 33.0 + 3.0 + 11.0)
        );
        // bend = max(36, |xb - xa| / 2) = 48
        assert_eq!(l.curve[1], pos2(sales.max.x + 48.0, row0));
        assert_eq!(l.curve[2], pos2(product.min.x - 48.0, l.curve[3].y));
        assert_eq!(l.from_mark, Some((pos2(sales.max.x + 8.0, row0 - 5.0), "*")));
        assert_eq!(
            l.to_mark,
            Some((pos2(product.min.x - 8.0 - 7.0, l.curve[3].y - 5.0), "1"))
        );
        // single direction: one arrow pointing back towards Sales (the many side)
        assert_eq!(l.arrows.len(), 1);
        let [tip, b1, _] = l.arrows[0];
        assert!(tip.x < b1.x, "{:?}", l.arrows);
        assert!(!l.inactive);
        // the inactive Date line, the both-ways Category line
        assert!(d.links[1].inactive);
        assert_eq!(d.links[2].arrows.len(), 2);
        // Category (from, "one") is right of Product: the line leaves its left side
        let cat = at(&d, "Category");
        assert_eq!(d.links[2].curve[0].x, cat.min.x);
        assert_eq!(d.links[2].from_mark.unwrap().1, "1");
        assert_eq!(d.links[2].to_mark.unwrap().1, "*");
    }

    #[test]
    fn a_hidden_row_anchors_on_the_header() {
        let cols: Vec<String> = (0..20).map(|i| format!("C{i}")).collect();
        let refs: Vec<&str> = cols.iter().map(String::as_str).collect();
        let m = json!({"tables": [table("F", &refs, 0), table("D", &["K"], 0)],
                       "relationships": [rel("F.C15", "D.K", json!({}))]});
        let d = layout(&m, &mut Order::default(), &HashSet::new());
        let f = &d.cards[0];
        assert_eq!(f.columns.len(), ROWS_COLLAPSED);
        assert!(f.more.is_some());
        assert_eq!(d.links[0].curve[0].y, f.head.center().y);
        // expanded: the row is shown, and the line goes to it
        let open: HashSet<String> = ["F".to_string()].into();
        let d = layout(&m, &mut Order::default(), &open);
        let f = &d.cards[0];
        assert_eq!(f.columns.len(), 20);
        assert_eq!(d.links[0].curve[0].y, f.columns[15].rect.center().y);
    }

    #[test]
    fn hits() {
        let d = layout(&star(), &mut Order::default(), &HashSet::new());
        let k = d.card_named("Sales").unwrap();
        let c = &d.cards[k];
        assert_eq!(d.hit(c.head.center(), 6.0), Some(Hit::Table(k)));
        assert_eq!(
            d.hit(c.columns[1].rect.center(), 6.0),
            Some(Hit::Column { card: k, index: 1 })
        );
        assert_eq!(
            d.hit(c.measures[0].rect.center(), 6.0),
            Some(Hit::Measure { card: k, index: 0 })
        );
        let mid = cubic(&d.links[0].curve, 0.5);
        assert_eq!(d.hit(mid + vec2(0.0, 3.0), 6.0), Some(Hit::Rel(0)));
        assert_eq!(d.hit(pos2(-100.0, -100.0), 6.0), None);
    }

    #[test]
    fn order_is_kept_when_tables_come_and_go() {
        let mut order = Order::default();
        let m = json!({"tables": [table("B", &["x"], 0), table("A", &["x"], 0)]});
        layout(&m, &mut order, &HashSet::new());
        // a re-read lists A first: B keeps its place (both unlinked: one column, by first sight)
        let m2 = json!({"tables": [table("A", &["x"], 0), table("B", &["x"], 0)]});
        let d = layout(&m2, &mut order, &HashSet::new());
        assert_eq!(d.cards[0].name, "B");
        assert_eq!(d.cards[1].name, "A");
    }

    #[test]
    fn empty_and_damaged_models() {
        for m in [
            json!(null),
            json!({}),
            json!({"tables": "x", "relationships": 3}),
            json!({"tables": [1, null, {"name": 5, "columns": {}}], "relationships": [{"from_table": null}]}),
        ] {
            let d = layout(&m, &mut Order::default(), &HashSet::new());
            assert!(d.size.x.is_finite() && d.size.y.is_finite());
            let _ = d.hit(pos2(30.0, 30.0), 6.0);
        }
        // a self relationship and a cycle do not loop
        let m = json!({"tables": [table("A", &["k"], 0), table("B", &["k"], 0)],
                       "relationships": [rel("A.k", "A.k", json!({})), rel("A.k", "B.k", json!({})),
                                         rel("B.k", "A.k", json!({}))]});
        let d = layout(&m, &mut Order::default(), &HashSet::new());
        assert_eq!(d.cards.len(), 2);
        assert_eq!(d.links.len(), 3);
    }

    #[test]
    fn huge_models_are_bounded() {
        let tables: Vec<Value> = (0..400)
            .map(|i| table(&format!("T{i}"), &["k", "a", "b"], if i < 250 { 20 } else { 0 }))
            .collect();
        let rels: Vec<Value> = (1..400)
            .map(|i| rel(&format!("T{i}.k"), &format!("T{}.k", i / 2), json!({})))
            .chain((0..2000).map(|i| rel(&format!("T{}.a", i % 300), "T0.k", json!({}))))
            .collect();
        let m = json!({"tables": tables, "relationships": rels});
        let d = layout(&m, &mut Order::default(), &HashSet::new());
        assert_eq!(d.cards.len(), MAX_TABLES);
        assert_eq!(d.total_tables, 400);
        assert!(d.links.len() <= MAX_RELS);
        assert!(d.cards.iter().all(|c| c.measures.len() <= ROWS_COLLAPSED));
        // a column is at most 10 cards high (a collapsed card is at most 12 + 12 rows)
        let card_max = 1.0 + HEAD_H + 2.0 * (6.0 + 12.0 * ROW_H) + 1.0 + MORE_H + NOTE_H + 1.0;
        assert!(d.size.y <= PAD_TOP + 10.0 * (card_max + CARD_GAP) + PAD_BOTTOM);
    }
}
