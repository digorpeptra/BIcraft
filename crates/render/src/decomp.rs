//! Decomposition trees (`decompositionTreeVisual`): the root bar, one column per shown level with
//! its header (field, selected member, the AI lightbulb, the remove cross), bars proportional
//! within the level, member and value labels, the connector lines from each level's parent (the
//! selected path in the accent colour), the scroll arrows of a level longer than the bars that
//! fit, and the "+" of the last level. Geometry and colours: Power BI Desktop 2.159 observed as
//! a black box (the decomposition tree oracle, docs/render.md); data: `data.decomp` of the
//! snapshot (docs/view-snapshot.md).

use serde_json::{Value, json};
use tiny_skia::PathBuilder;

use crate::canvas::{Anchor, Base, Canvas, Rgb, Text};
use crate::charts::Bx;
use crate::data::{Ctx, Dataset, arr, clampf, fin, get, sk};
use crate::fonts::Font;
use crate::hits::{self, HitAction, HitShape, Point};

/// Bar width (report px; Desktop's 134 CSS px at its 103 % zoom).
const BAR_W: f32 = 130.0;
/// Bar thickness.
const BAR_H: f32 = 9.7;
/// Horizontal distance between levels, default density.
const PITCH_X: f32 = 180.0;
/// Vertical distance between bars, default density.
const PITCH_Y: f32 = 59.333;
/// Left edge of the root bar.
const ORIGIN_X: f32 = 24.3;
/// Height of the level header band.
const HEADER_H: f32 = 56.4;
/// The bars are centred on `(h + CENTRE_SHIFT) / 2`.
const CENTRE_SHIFT: f32 = 22.33;
/// The bar area: the body height less this.
const RESERVED_H: f32 = 104.0;
/// A shorter body is laid out as if it were this high (clipped below).
const MIN_LAYOUT_H: f32 = 319.0;
/// A long level with no selected member ends its window this many members before its last.
const UNSELECTED_TAIL: usize = 4;
/// Most levels and bars drawn.
const MAX_LEVELS: usize = 16;
const MAX_BARS: usize = 100;
/// Most members of one level read.
const MAX_NODES: usize = 5000;

const BAR_BG: Rgb = Rgb(0xEF, 0xEF, 0xEF);
const PATH_BORDER: Rgb = Rgb(0xC8, 0xC8, 0xC8);
const LINK_DEFAULT: Rgb = Rgb(0xEF, 0xEF, 0xEF);

/// One drawn bar.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeBox {
    /// Index of the member in its level's `nodes`.
    pub index: usize,
    /// Vertical centre of the bar.
    pub y: f32,
    /// Filled length of the bar.
    pub bar: f32,
    pub on_path: bool,
}

/// One drawn column (the root is level 0).
#[derive(Debug, Clone, PartialEq)]
pub struct LevelBox {
    pub x: f32,
    /// First member shown (the level's scroll position).
    pub start: usize,
    pub nodes: Vec<NodeBox>,
    pub up: bool,
    pub down: bool,
}

/// Where everything goes, in body pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub levels: Vec<LevelBox>,
    /// Bars that fit in one level.
    pub bars: usize,
    pub bar_w: f32,
    pub pitch_y: f32,
    /// Top and bottom bar centres.
    pub top: f32,
    pub bottom: f32,
    /// Horizontal distance between levels.
    pub pitch_x: f32,
    /// Levels scrolled out on the left (the reader's `shift`).
    pub shift: usize,
    /// Levels are hidden on the left (`left`) or clipped on the right (`right`): the tree's
    /// horizontal scroll arrows.
    pub left: bool,
    pub right: bool,
}

fn opt<'a>(d: &'a Value, obj: &str, prop: &str) -> &'a Value {
    get(get(get(d, "options"), obj), prop)
}

/// The members' measure as numbers (`None`: blank).
fn level_values(l: &Value) -> Vec<Option<f64>> {
    arr(get(l, "nodes"))
        .iter()
        .take(MAX_NODES)
        .map(|n| arr(n).get(1).and_then(fin))
        .collect()
}

/// The layout of a tree whose `data.decomp` is `d`, in a body of `w` × `h` report pixels.
pub fn layout(d: &Value, w: f32, h: f32) -> Layout {
    let (kx, ky) = match sk(get(get(d, "options"), "tree"), "density") {
        "dense" => (0.9, 0.8),
        "sparse" => (1.2, 1.2),
        _ => (1.0, 1.0),
    };
    let bar_w = fin(opt(d, "dataBars", "dataBarWidthPercent"))
        .map_or(BAR_W, |p| BAR_W * clampf(p as f32, 10.0, 300.0) / 100.0);
    let (px, py) = (PITCH_X * kx * bar_w.max(BAR_W) / BAR_W, PITCH_Y * ky);
    // `tree.barsPerLevel` does not limit them: Desktop showed the bars that fit (D08: 6 asked, 4
    // shown; D22: 3 asked, 6 shown)
    let h = if h.is_finite() {
        h.max(MIN_LAYOUT_H)
    } else {
        MIN_LAYOUT_H
    };
    let fit = ((h - RESERVED_H) / py).floor().max(1.0) as usize;
    let bars = fit.clamp(1, MAX_BARS);
    let centre = (h + CENTRE_SHIFT) / 2.0;
    let span = (bars.saturating_sub(1)) as f32 * py;
    let (top, bottom) = (centre - span / 2.0, centre + span / 2.0);
    let scaling = sk(get(get(d, "options"), "dataBars"), "dataBarScalingType").to_string();
    let root = fin(get(d, "root"));
    // the reader's horizontal scroll: at least the last level stays in view
    let shown = arr(get(d, "levels")).len().min(MAX_LEVELS);
    let shift = get(d, "shift")
        .as_u64()
        .and_then(|s| usize::try_from(s).ok())
        .unwrap_or(0)
        .min(shown);
    let x0 = ORIGIN_X - shift as f32 * px;
    let mut levels = vec![LevelBox {
        x: x0,
        start: 0,
        nodes: vec![NodeBox {
            index: 0,
            y: centre,
            bar: if root.is_some_and(|r| r != 0.0) {
                bar_w
            } else {
                0.0
            },
            on_path: true,
        }],
        up: false,
        down: false,
    }];
    let mut parent_y = centre;
    let mut parent_v = root;
    for (li, l) in arr(get(d, "levels")).iter().take(MAX_LEVELS).enumerate() {
        let vals = level_values(l);
        let total = vals.len();
        if total == 0 {
            break;
        }
        let n = total.min(bars);
        let sel = get(l, "selected")
            .as_u64()
            .and_then(|s| usize::try_from(s).ok())
            .filter(|s| *s < total);
        // the reader's scroll position; else the selected member in the middle of the bars
        // shown, and without one, the window ends four members before the last (Desktop 2.159,
        // D08, D15-D17, D19, D20, D22)
        let scrolled = get(l, "start").as_u64().and_then(|s| usize::try_from(s).ok());
        let start = match (scrolled, sel) {
            (Some(s), _) => s,
            (None, Some(s)) => s.saturating_sub(n / 2),
            (None, None) => total.saturating_sub(n).saturating_sub(UNSELECTED_TAIL),
        }
        .min(total - n);
        // centred on the parent, kept inside the bar area
        let first = (parent_y - (n - 1) as f32 * py / 2.0)
            .min(bottom - (n - 1) as f32 * py)
            .max(top);
        let max = vals.iter().flatten().fold(0.0f64, |m, x| m.max(x.abs()));
        let top_v = root.map(f64::abs);
        let x = x0 + (li + 1) as f32 * px;
        let nodes = (start..start + n)
            .enumerate()
            .map(|(k, i)| {
                let v = vals.get(i).copied().flatten().unwrap_or(0.0).abs();
                let base = match scaling.as_str() {
                    "parentNode" => parent_v.map(f64::abs),
                    "topNode" => top_v,
                    _ => Some(max),
                };
                let f = base.filter(|b| *b > 0.0).map_or(0.0, |b| (v / b).min(1.0));
                NodeBox {
                    index: i,
                    y: first + k as f32 * py,
                    bar: bar_w * f as f32,
                    on_path: sel == Some(i),
                }
            })
            .collect::<Vec<_>>();
        // a selected member scrolled out of the window: the level below hangs from the end of
        // the bar area it went past
        let next = nodes.iter().find(|nb| nb.on_path).map(|nb| nb.y).or(match sel {
            Some(s) if s < start => Some(top),
            Some(_) => Some(bottom),
            None => None,
        });
        levels.push(LevelBox {
            x,
            start,
            nodes,
            up: start > 0,
            down: start + n < total,
        });
        match next {
            Some(y) => {
                parent_y = y;
                parent_v = sel.and_then(|s| vals.get(s).copied().flatten());
            }
            None => break,
        }
    }
    // a tree wider than the visual is clipped on the right (D21); the reader scrolls it by
    // levels with the arrows at its sides
    let right = levels.last().is_some_and(|l| l.x + bar_w > w.max(0.0) + 0.5);
    Layout {
        levels,
        bars,
        bar_w,
        pitch_y: py,
        top,
        bottom,
        pitch_x: px,
        shift,
        left: shift > 0,
        right: right && shift < shown,
    }
}

/// The path of member `i` of level `li` (1-based level; the root has none): the selected
/// members above it and itself, as `{c, v}` grouping values.
fn point_of(d: &Value, li: usize, i: usize) -> Point {
    let mut p = Vec::new();
    for (k, l) in arr(get(d, "levels")).iter().take(li).enumerate() {
        let Some(c) = get(l, "column").as_u64().and_then(|c| usize::try_from(c).ok()) else {
            return Vec::new();
        };
        let idx = if k + 1 == li {
            Some(i)
        } else {
            get(l, "selected").as_u64().and_then(|s| usize::try_from(s).ok())
        };
        let Some(m) = idx
            .and_then(|s| arr(get(l, "nodes")).get(s))
            .and_then(|n| arr(n).first())
        else {
            return Vec::new();
        };
        p.push((c, m.clone()));
    }
    p
}

/// The members above the last one of a point.
fn above(p: &Point) -> Value {
    Value::Array(
        p.iter()
            .take(p.len().saturating_sub(1))
            .map(|x| x.1.clone())
            .collect(),
    )
}

/// The data columns of the first `n` levels.
fn columns_above(d: &Value, n: usize) -> Value {
    Value::Array(
        arr(get(d, "levels"))
            .iter()
            .take(n)
            .map(|l| get(l, "column").clone())
            .collect(),
    )
}

fn color_opt(d: &Value, obj: &str, prop: &str) -> Option<Rgb> {
    opt(d, obj, prop).as_str().and_then(Rgb::parse)
}

fn font_of(ctx: &Ctx<'_>, d: &Value, obj: &str, prefix: &str, px: f32, weight: u16) -> (Font, Rgb) {
    let fam = sk(opt(d, obj, &format!("{prefix}FontFamily")), "");
    let family = if fam.is_empty() {
        ctx.font.clone()
    } else {
        crate::data::safe_font(fam)
    };
    let size = fin(opt(d, obj, &format!("{prefix}FontSize")))
        .map_or(px, |pt| clampf(pt as f32, 5.0, 72.0) * crate::data::PT);
    let bold = opt(d, obj, &format!("{prefix}Bold")).as_bool() == Some(true);
    let mut f = Font::new(&family, size).weight(if bold { 700 } else { weight });
    if opt(d, obj, &format!("{prefix}Italic")).as_bool() == Some(true) {
        f = f.italic(true);
    }
    let c = color_opt(d, obj, &format!("{prefix}FontColor")).unwrap_or(ctx.fg);
    (f, c)
}

/// A horizontal link from `(x0, y0)` to `(x1, y1)` (Desktop's curve: both control points at the
/// middle; `round`: the first at the start, the second at the middle at the end's height).
fn link(cv: &mut Canvas, a: (f32, f32), b: (f32, f32), round: bool, c: Rgb, width: f32, dotted: bool) {
    let mid = (a.0 + b.0) / 2.0;
    let mut pb = PathBuilder::new();
    pb.move_to(a.0, a.1);
    if round {
        pb.cubic_to(a.0, a.1, mid, b.1, b.0, b.1);
    } else {
        pb.cubic_to(mid, a.1, mid, b.1, b.0, b.1);
    }
    if let Some(p) = pb.finish() {
        cv.stroke(&p, c, 1.0, width, dotted.then_some((1.5, 2.5)));
    }
}

/// A small chevron (scroll arrow) centred at `(x, y)`.
fn chevron(cv: &mut Canvas, x: f32, y: f32, up: bool, c: Rgb) {
    let dy = if up { -3.0 } else { 3.0 };
    cv.polyline(
        &[(x - 6.0, y - dy), (x, y + dy), (x + 6.0, y - dy)],
        c,
        1.0,
        1.2,
        None,
    );
}

/// A sideways chevron (the tree's horizontal scroll arrows) centred at `(x, y)`.
fn side_chevron(cv: &mut Canvas, x: f32, y: f32, left: bool, c: Rgb) {
    let dx = if left { 3.0 } else { -3.0 };
    cv.polyline(
        &[(x + dx, y - 6.0), (x - dx, y), (x + dx, y + 6.0)],
        c,
        1.0,
        1.2,
        None,
    );
}

/// A `decomp_scroll` action of the visual being drawn (`extra`: `{level, start}` or `{shift}`).
fn scroll_action(ctx: &Ctx<'_>, extra: Value) -> HitAction {
    let (page, visual) = ctx.ix.page_visual();
    let mut a = json!({"action": "decomp_scroll", "page": page, "visual": visual});
    if let (Some(o), Value::Object(e)) = (a.as_object_mut(), extra) {
        o.extend(e);
    }
    HitAction::Send {
        action: a,
        ctrl: false,
    }
}

/// The AI lightbulb of a level header, centred at `(x, y)`.
fn bulb(cv: &mut Canvas, x: f32, y: f32, c: Rgb) {
    cv.circle(x, y - 1.5, 4.2, None, Some((c, 1.1)));
    cv.line(x - 2.0, y + 4.0, x + 2.0, y + 4.0, c, 1.1, None);
    cv.line(x - 1.5, y + 5.8, x + 1.5, y + 5.8, c, 1.1, None);
}

/// Draws the tree (without `data.decomp`, a made-up preview: the root alone).
pub fn decomp_body(cv: &mut Canvas, v: &Value, ds: &Dataset, b: Bx, ctx: &Ctx<'_>) {
    let fallback;
    let mut d = get(get(v, "data"), "decomp");
    if !d.is_object() {
        // no evaluated tree (made-up preview): the root alone
        let analyze = arr(get(v, "fields"))
            .iter()
            .position(|f| sk(f, "role").eq_ignore_ascii_case("Analyze"));
        fallback = json!({"analyze": analyze, "root": ds.rows.first().and_then(|r| analyze.and_then(|a| r.get(a))).cloned(), "levels": []});
        d = &fallback;
    }
    let lay = layout(d, b.w, b.h);
    // a region scrolled out of the body is not clickable
    let shown = |x: f32, w: f32| x + w > 0.0 && x < b.w;
    // `tree.defaultClickAction: toggle` ("Collapse" in the public theme schema): a click on an
    // intermediate member of the selected path removes the levels below it
    let toggle = sk(get(get(d, "options"), "tree"), "defaultClickAction") == "toggle";
    let accent = color_opt(d, "tree", "accentColor").unwrap_or_else(|| ctx.color(0));
    let bar_c = color_opt(d, "dataBars", "dataBarColor")
        .or_else(|| color_opt(d, "dataBars", "positiveBarColor"))
        .unwrap_or_else(|| ctx.color(0));
    let neg_c = color_opt(d, "dataBars", "negativeBarColor").unwrap_or(bar_c);
    let bg_c = color_opt(d, "dataBars", "dataBarBackgroundColor").unwrap_or(BAR_BG);
    let link_c = color_opt(d, "tree", "connectorDefaultColor").unwrap_or(LINK_DEFAULT);
    let round = sk(get(get(d, "options"), "tree"), "connectorType") == "round";
    let analyze = get(d, "analyze").as_u64().and_then(|a| usize::try_from(a).ok());
    let mcol = analyze.and_then(|a| ds.cols.get(a));
    let units = fin(opt(d, "dataLabels", "dataLabelDisplayUnits")).or(Some(1.0));
    let units = units.map(|u| if u == 0.0 { 0.0 } else { u });
    let precision = fin(opt(d, "dataLabels", "dataLabelPrecision")).map(|p| p.clamp(0.0, 10.0) as usize);
    let value_text = |x: Option<f64>| -> String {
        match x {
            Some(x) => crate::fmt::fmt_label(
                x,
                mcol.map(|c| c.cf()).as_ref(),
                &ctx.loc,
                units.filter(|u| *u > 0.0),
                precision,
            ),
            None => ctx.blank(),
        }
    };
    let levels_json = arr(get(d, "levels"));
    let highlights = ds.hl.as_ref();
    // data.rows: the root, then each level's members in order (their highlight rows alike)
    let mut starts = vec![1usize];
    for l in levels_json.iter().take(MAX_LEVELS) {
        let last = starts.last().copied().unwrap_or(1);
        starts.push(last.saturating_add(arr(get(l, "nodes")).len()));
    }
    let hl_of = |row: usize| -> Option<f64> {
        let a = analyze?;
        highlights?.get(row)?.get(a).and_then(fin)
    };
    let explain: Vec<usize> = arr(get(v, "fields"))
        .iter()
        .enumerate()
        .filter(|(_, f)| sk(f, "role").eq_ignore_ascii_case("ExplainBy"))
        .map(|(i, _)| i)
        .collect();
    let used: Vec<usize> = levels_json
        .iter()
        .filter_map(|l| get(l, "column").as_u64().and_then(|c| usize::try_from(c).ok()))
        .collect();
    let free: Vec<usize> = explain.iter().copied().filter(|c| !used.contains(c)).collect();
    let (lab_f, lab_c) = font_of(ctx, d, "categoryLabels", "categoryLabel", 12.0, 400);
    let (val_f, val_c) = font_of(ctx, d, "dataLabels", "dataLabel", 12.0, 400);
    let selected: Vec<Point> = ctx.ix.scope.borrow().selected.clone();
    let dim_of = |p: &Point| -> f32 {
        if selected.is_empty() {
            return 1.0;
        }
        // a node stays opaque when it is on the selected node's path (root included)
        let on = selected.iter().any(|sp| {
            p.iter()
                .all(|(c, x)| sp.iter().any(|(sc, sx)| sc == c && hits::same(sx, x)))
        });
        if on { 1.0 } else { hits::DIM }
    };

    // links first
    for (li, lb) in lay.levels.iter().enumerate().skip(1) {
        let Some(parent) = lay
            .levels
            .get(li - 1)
            .and_then(|p| p.nodes.iter().find(|n| n.on_path))
        else {
            continue;
        };
        let px = lay.levels.get(li - 1).map_or(0.0, |p| p.x) + lay.bar_w;
        // the link to a member an AI split selected is dotted (Desktop 2.159, D13)
        let auto = levels_json
            .get(li - 1)
            .is_some_and(|l| get(l, "auto").as_bool() == Some(true));
        for nb in &lb.nodes {
            let (c, wdt) = if nb.on_path { (accent, 1.5) } else { (link_c, 1.0) };
            link(
                cv,
                (px - 1.0, parent.y),
                (lb.x, nb.y),
                round,
                c,
                wdt,
                nb.on_path && auto,
            );
        }
    }
    // the bars
    let last_level = lay.levels.len().saturating_sub(1);
    for (li, lb) in lay.levels.iter().enumerate() {
        let level = if li == 0 { None } else { levels_json.get(li - 1) };
        let col = level
            .and_then(|l| get(l, "column").as_u64())
            .and_then(|c| usize::try_from(c).ok());
        let ccol = col.and_then(|c| ds.cols.get(c));
        for nb in &lb.nodes {
            let (label, value) = match level {
                None => (
                    mcol.map_or_else(String::new, |c| c.name.clone()),
                    fin(get(d, "root")),
                ),
                Some(l) => {
                    let n = arr(get(l, "nodes")).get(nb.index);
                    let m = n.and_then(|n| arr(n).first()).unwrap_or(&Value::Null);
                    let t = match (m, ccol) {
                        (Value::Null, _) => ctx.blank(),
                        (Value::String(s), _) if s.is_empty() => ctx.blank(),
                        (_, Some(c)) => hits::cell(m, c, ctx),
                        (m, None) => crate::data::st(m),
                    };
                    (t, n.and_then(|n| arr(n).get(1)).and_then(fin))
                }
            };
            let point = if li == 0 {
                Vec::new()
            } else {
                point_of(d, li, nb.index)
            };
            let alpha = dim_of(&point);
            let (x, y0) = (lb.x, nb.y - BAR_H / 2.0);
            let row = if li == 0 {
                0
            } else {
                starts.get(li - 1).copied().unwrap_or(0).saturating_add(nb.index)
            };
            let fill = if value.is_some_and(|x| x < 0.0) {
                neg_c
            } else {
                bar_c
            };
            cv.rect(x, y0, lay.bar_w, BAR_H, bg_c, alpha);
            if nb.on_path {
                cv.round_rect(x, y0, lay.bar_w, BAR_H, 0.0, None, Some((PATH_BORDER, 1.0, None)));
            }
            match (highlights, li > 0) {
                (Some(_), true) => {
                    cv.rect(x, y0, nb.bar, BAR_H, fill, hits::DIM);
                    if let (Some(h), Some(full)) = (hl_of(row), value)
                        && full != 0.0
                    {
                        let f = clampf((h / full) as f32, 0.0, 1.0);
                        cv.rect(x, y0, nb.bar * f, BAR_H, fill, 1.0);
                    }
                }
                _ => cv.rect(x, y0, nb.bar, BAR_H, fill, alpha),
            }
            let weight = if nb.on_path { 700 } else { lab_f.weight };
            let lf = lab_f.clone().weight(weight);
            let t = ctx.fonts.fit(&lf, &label, lay.bar_w - 6.0);
            let mut tl = Text::new(&t, lf, lab_c).anchor(Anchor::Start).base(Base::Central);
            tl.alpha = alpha;
            cv.text(ctx.fonts, &tl, x + 4.9, nb.y + 14.0);
            let vt = value_text(value);
            let mut tv = Text::new(&vt, val_f.clone(), val_c)
                .anchor(Anchor::Start)
                .base(Base::Central);
            tv.alpha = alpha;
            cv.text(ctx.fonts, &tv, x + 4.9, nb.y + 27.6);
            // hits: a node on the path selects (cross-filter), another one moves the path there
            if ctx.ix.on() && li > 0 && shown(x, lay.bar_w) {
                let (page, visual) = ctx.ix.page_visual();
                let action = if nb.on_path && toggle && li < last_level {
                    HitAction::Send {
                        action: json!({"action": "decomp_collapse", "page": page, "visual": visual, "level": li + 1}),
                        ctrl: false,
                    }
                } else if nb.on_path {
                    ctx.ix.select(&point)
                } else {
                    HitAction::Send {
                        action: json!({"action": "decomp_path", "page": page, "visual": visual, "level": li,
                                       "value": point.last().map(|p| p.1.clone()).unwrap_or(Value::Null),
                                       "path": above(&point), "columns": columns_above(d, li)}),
                        ctrl: false,
                    }
                };
                let tip = vec![
                    (ccol.map_or_else(String::new, |c| c.name.clone()), label.clone()),
                    (mcol.map_or_else(String::new, |c| c.name.clone()), vt.clone()),
                ];
                cv.hit(ctx.ix.hit(
                    HitShape::Rect {
                        x,
                        y: y0 - 2.0,
                        w: lay.bar_w,
                        h: 42.0,
                    },
                    action,
                    tip,
                ));
            }
            // "+" on the last level (and the root alone) while fields are left
            if li == last_level && !free.is_empty() {
                let (px_, py_) = (x + lay.bar_w + 11.0, nb.y - 2.4);
                cv.text(
                    ctx.fonts,
                    &Text::new("+", Font::new(&ctx.font, 20.0), ctx.fg2)
                        .anchor(Anchor::Middle)
                        .base(Base::Central),
                    px_,
                    py_,
                );
                if ctx.ix.on() && shown(px_ - 8.0, 16.0) {
                    let items = expand_items(ctx, v, d, li, nb.index, &free);
                    let action = ctx
                        .ix
                        .popup("decomp_expand", json!({"level": li, "items": items}));
                    cv.hit(ctx.ix.hit(
                        HitShape::Rect {
                            x: px_ - 8.0,
                            y: py_ - 12.0,
                            w: 16.0,
                            h: 24.0,
                        },
                        action,
                        Vec::new(),
                    ));
                }
            }
        }
        // the scroll arrows of a level longer than its window: a click shows the window before
        // or after this one
        let cx = lb.x + lay.bar_w / 2.0 - 2.0;
        let n = lb.nodes.len().max(1);
        for (on, up, y) in [(lb.up, true, lay.top - 27.0), (lb.down, false, lay.bottom + 57.0)] {
            if !on {
                continue;
            }
            chevron(cv, cx, y, up, ctx.fg2);
            if ctx.ix.on() && li > 0 && shown(cx - 12.0, 24.0) {
                let start = if up {
                    lb.start.saturating_sub(n)
                } else {
                    lb.start.saturating_add(n)
                };
                cv.hit(ctx.ix.hit(
                    HitShape::Rect {
                        x: cx - 12.0,
                        y: y - 9.0,
                        w: 24.0,
                        h: 18.0,
                    },
                    scroll_action(ctx, json!({"level": li, "start": start})),
                    Vec::new(),
                ));
            }
        }
    }
    // the level headers, over a band of the visual's background
    if lay.levels.len() > 1 {
        let band = color_opt(d, "levelHeader", "levelHeaderBackgroundColor").unwrap_or(ctx.bg);
        cv.rect(0.0, 0.0, b.w, HEADER_H.min(b.h), band, 1.0);
        let (tf, tc) = font_of(ctx, d, "levelHeader", "levelTitle", 16.0, 400);
        let (sf, sc) = font_of(ctx, d, "levelHeader", "levelSubtitle", 12.0, 400);
        let subtitles = opt(d, "levelHeader", "showSubtitles").as_bool() != Some(false);
        for (li, lb) in lay.levels.iter().enumerate().skip(1) {
            let Some(l) = levels_json.get(li - 1) else {
                continue;
            };
            let col = get(l, "column").as_u64().and_then(|c| usize::try_from(c).ok());
            let name = col
                .and_then(|c| ds.cols.get(c))
                .map_or_else(String::new, |c| c.name.clone());
            let t = ctx.fonts.fit(&tf, &name, lay.bar_w - 18.0);
            cv.text(
                ctx.fonts,
                &Text::new(&t, tf.clone(), tc)
                    .anchor(Anchor::Start)
                    .base(Base::Central),
                lb.x,
                19.9,
            );
            cv.rect(lb.x, 29.1, lay.bar_w, 1.95, accent, 1.0);
            cv.text(
                ctx.fonts,
                &Text::new("\u{d7}", Font::new(&ctx.font, 20.0), ctx.fg2)
                    .anchor(Anchor::Middle)
                    .base(Base::Central),
                lb.x + lay.bar_w - 6.5,
                18.5,
            );
            if !sk(l, "ai").is_empty() {
                bulb(cv, lb.x - 11.0, 20.5, accent);
            }
            // the selected member, under the header of a level with another one below
            if subtitles
                && li < last_level
                && let Some(nb) = lb.nodes.iter().find(|n| n.on_path)
            {
                let m = arr(get(l, "nodes"))
                    .get(nb.index)
                    .and_then(|n| arr(n).first())
                    .cloned()
                    .unwrap_or(Value::Null);
                let s = match (&m, col.and_then(|c| ds.cols.get(c))) {
                    (Value::Null, _) => ctx.blank(),
                    (Value::String(s), _) if s.is_empty() => ctx.blank(),
                    (_, Some(c)) => hits::cell(&m, c, ctx),
                    _ => crate::data::st(&m),
                };
                let s = ctx.fonts.fit(&sf, &s, lay.bar_w);
                cv.text(
                    ctx.fonts,
                    &Text::new(&s, sf.clone(), sc)
                        .anchor(Anchor::Start)
                        .base(Base::Central),
                    lb.x,
                    41.8,
                );
            }
            if ctx.ix.on() && shown(lb.x + lay.bar_w - 14.0, 15.0) {
                let (page, visual) = ctx.ix.page_visual();
                let action = HitAction::Send {
                    action: json!({"action": "decomp_collapse", "page": page, "visual": visual, "level": li}),
                    ctrl: false,
                };
                cv.hit(ctx.ix.hit(
                    HitShape::Rect {
                        x: lb.x + lay.bar_w - 14.0,
                        y: 6.0,
                        w: 15.0,
                        h: 26.0,
                    },
                    action,
                    Vec::new(),
                ));
            }
        }
    }
    // the horizontal scroll arrows of a tree wider than the visual, at the sides of the bar
    // area, over a patch of the visual's background
    let mid = (lay.top + lay.bottom) / 2.0;
    for (on, left) in [(lay.left, true), (lay.right, false)] {
        if !on {
            continue;
        }
        let x = if left { 0.0 } else { (b.w - 18.0).max(0.0) };
        cv.rect(x, mid - 16.0, 18.0, 32.0, ctx.bg, 0.9);
        side_chevron(cv, x + 9.0, mid, left, ctx.fg2);
        if ctx.ix.on() {
            let shift = if left {
                lay.shift.saturating_sub(1)
            } else {
                lay.shift.saturating_add(1)
            };
            cv.hit(ctx.ix.hit(
                HitShape::Rect {
                    x,
                    y: mid - 16.0,
                    w: 18.0,
                    h: 32.0,
                },
                scroll_action(ctx, json!({"shift": shift})),
                Vec::new(),
            ));
        }
    }
}

/// The "+" menu of member `index` of level `li`: "High value", "Low value" (when AI splits are
/// on), then the fields left; each item carries the `view.interact` action it sends.
fn expand_items(ctx: &Ctx<'_>, v: &Value, d: &Value, li: usize, index: usize, free: &[usize]) -> Value {
    let (page, visual) = ctx.ix.page_visual();
    let value = if li == 0 {
        Value::Null
    } else {
        point_of(d, li, index)
            .last()
            .map(|p| p.1.clone())
            .unwrap_or(Value::Null)
    };
    let base = |extra: Value| -> Value {
        let point = if li == 0 {
            Vec::new()
        } else {
            point_of(d, li, index)
        };
        let mut a = json!({"action": "decomp_expand", "page": page, "visual": visual, "level": li, "value": value,
                           "path": above(&point), "columns": columns_above(d, li)});
        if let (Some(o), Value::Object(e)) = (a.as_object_mut(), extra) {
            o.extend(e);
        }
        a
    };
    let mut items = Vec::new();
    if opt(d, "analysis", "aiEnabled").as_bool() != Some(false) {
        items.push(json!({"label": ctx.t("decomp_high", "High value"), "ai": "high", "action": base(json!({"ai": "high"}))}));
        items.push(json!({"label": ctx.t("decomp_low", "Low value"), "ai": "low", "action": base(json!({"ai": "low"}))}));
    }
    for c in free.iter().take(64) {
        let name = sk(
            arr(get(get(v, "data"), "columns"))
                .get(*c)
                .unwrap_or(&Value::Null),
            "name",
        )
        .to_string();
        items.push(json!({"label": name, "column": c, "action": base(json!({"column": c}))}));
    }
    Value::Array(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(levels: Value, options: Value) -> Value {
        json!({"analyze": 0, "root": 593333.0, "levels": levels, "options": options})
    }

    #[test]
    fn layout_matches_desktop_geometry() {
        // D03 of the oracle: South (4th of 5) selected; 4 bars fit in a 390 px body
        let regions = json!([
            ["East", 1.0],
            ["North", 2.0],
            ["South", 3.0],
            ["West", 0.5],
            ["Central", 0.2]
        ]);
        let d = tree(
            json!([{"column": 1, "nodes": regions, "selected": 3},
                           {"column": 2, "nodes": [["A", 1.0], ["B", 2.0], ["C", 3.0]], "selected": null}]),
            json!({}),
        );
        let l = layout(&d, 990.0, 390.0);
        assert_eq!(l.bars, 4);
        let root = &l.levels[0];
        assert!((root.nodes[0].y - 206.17).abs() < 0.1);
        let l1 = &l.levels[1];
        assert_eq!((l1.start, l1.up, l1.down), (1, true, false));
        assert!((l1.nodes[0].y - 117.17).abs() < 0.1 && (l1.x - 204.3).abs() < 0.1);
        // children centred on their parent, kept inside the bar area
        let l2 = &l.levels[2];
        assert!((l2.nodes[2].y - 295.17).abs() < 0.1, "{:?}", l2.nodes);
        // dense and sparse pitches
        let dense = layout(
            &tree(json!([]), json!({"tree": {"density": "dense"}})),
            990.0,
            390.0,
        );
        assert_eq!(dense.bars, 6);
        assert!((dense.pitch_y - 47.47).abs() < 0.05);
    }

    #[test]
    fn hostile_trees_are_bounded() {
        let nodes: Vec<Value> = (0..5000).map(|i| json!([i, i])).collect();
        let levels: Vec<Value> = (0..100)
            .map(|_| json!({"column": 1, "nodes": nodes, "selected": 4999}))
            .collect();
        let l = layout(
            &tree(Value::Array(levels), json!({"tree": {"barsPerLevel": 1e12}})),
            10.0,
            1e6,
        );
        assert!(l.levels.len() <= MAX_LEVELS + 1);
        assert!(l.levels.iter().all(|x| x.nodes.len() <= MAX_BARS));
        let l = layout(&json!({"levels": "x", "root": "y"}), -5.0, f32::NAN);
        assert_eq!(l.levels.len(), 1);
    }
}
