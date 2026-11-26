//! The hit map of a rendered page (`RenderOptions::hits`): where a click lands and what it
//! does, so a native front end (`apps/bicraft-desktop`) behaves like the live viewer. The
//! regions and their actions are the ones `crates/view/src/page.html` registers (`ixHit`,
//! `ixMark`, `ixRect`, `slActs`): the same `view.interact` actions
//! (`docs/view-snapshot.md`, "Interactions"), the same tooltip rows. Rules stay in the engine:
//! a hit only says what was clicked.

use std::cell::RefCell;

use serde_json::{Value, json};

use crate::data::{ChartData, Col, Ctx, arr, get, sk};

/// Most hit regions a page records.
pub const MAX_HITS: usize = 50_000;
/// Opacity of a point the selection leaves out, in its own visual (the viewer's `IX_DIM`).
pub const DIM: f32 = 0.4;

/// The area a hit covers, in logical (report) pixels of the page.
#[derive(Debug, Clone, PartialEq)]
pub enum HitShape {
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    },
    Circle {
        cx: f32,
        cy: f32,
        r: f32,
    },
    /// A ring sector (pie and donut slices): radii and angles in radians, clockwise from 12 o'clock.
    Sector {
        cx: f32,
        cy: f32,
        r0: f32,
        r1: f32,
        a0: f32,
        a1: f32,
    },
    /// Polygons with holes (a shape map's areas): a point is inside when it crosses the rings'
    /// edges an odd number of times.
    Polygon {
        rings: Vec<Vec<(f32, f32)>>,
    },
}

/// Whether `(px, py)` is inside `rings` (even-odd rule).
fn in_rings(rings: &[Vec<(f32, f32)>], px: f32, py: f32) -> bool {
    let mut inside = false;
    for r in rings {
        let n = r.len();
        for i in 0..n {
            let (Some(&(xa, ya)), Some(&(xb, yb))) = (r.get(i), r.get((i + 1) % n)) else {
                continue;
            };
            if (ya > py) != (yb > py) && px < xa + (py - ya) / (yb - ya) * (xb - xa) {
                inside = !inside;
            }
        }
    }
    inside
}

impl HitShape {
    fn moved(&self, dx: f32, dy: f32) -> HitShape {
        if let HitShape::Polygon { rings } = self {
            return HitShape::Polygon {
                rings: rings
                    .iter()
                    .map(|r| r.iter().map(|(x, y)| (x + dx, y + dy)).collect())
                    .collect(),
            };
        }
        match *self {
            HitShape::Polygon { .. } => self.clone(),
            HitShape::Rect { x, y, w, h } => HitShape::Rect {
                x: x + dx,
                y: y + dy,
                w,
                h,
            },
            HitShape::Circle { cx, cy, r } => HitShape::Circle {
                cx: cx + dx,
                cy: cy + dy,
                r,
            },
            HitShape::Sector {
                cx,
                cy,
                r0,
                r1,
                a0,
                a1,
            } => HitShape::Sector {
                cx: cx + dx,
                cy: cy + dy,
                r0,
                r1,
                a0,
                a1,
            },
        }
    }

    /// Whether the point is inside.
    pub fn contains(&self, px: f32, py: f32) -> bool {
        match *self {
            HitShape::Polygon { ref rings } => in_rings(rings, px, py),
            HitShape::Rect { x, y, w, h } => px >= x && px <= x + w && py >= y && py <= y + h,
            HitShape::Circle { cx, cy, r } => (px - cx).powi(2) + (py - cy).powi(2) <= r * r,
            HitShape::Sector {
                cx,
                cy,
                r0,
                r1,
                a0,
                a1,
            } => {
                let (dx, dy) = (px - cx, py - cy);
                let d = (dx * dx + dy * dy).sqrt();
                if d < r0 || d > r1 {
                    return false;
                }
                let tau = std::f32::consts::TAU;
                // clockwise from 12 o'clock, like the drawing
                let a = dx.atan2(-dy).rem_euclid(tau);
                let lo = a0.rem_euclid(tau);
                let span = (a1 - a0).clamp(0.0, tau);
                (a - lo).rem_euclid(tau) <= span
            }
        }
    }

    /// The bounding box `(x, y, w, h)`.
    pub fn bounds(&self) -> (f32, f32, f32, f32) {
        match *self {
            HitShape::Polygon { ref rings } => {
                let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
                for &(x, y) in rings.iter().flatten() {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x);
                    y1 = y1.max(y);
                }
                if x1 < x0 {
                    (0.0, 0.0, 0.0, 0.0)
                } else {
                    (x0, y0, x1 - x0, y1 - y0)
                }
            }
            HitShape::Rect { x, y, w, h } => (x, y, w, h),
            HitShape::Circle { cx, cy, r } => (cx - r, cy - r, 2.0 * r, 2.0 * r),
            HitShape::Sector { cx, cy, r1, .. } => (cx - r1, cy - r1, 2.0 * r1, 2.0 * r1),
        }
    }
}

/// What a click on a hit does.
#[derive(Debug, Clone, PartialEq)]
pub enum HitAction {
    /// Nothing (a tooltip only).
    None,
    /// A `view.interact` action; `ctrl`: the front end adds `"ctrl": true|false` from the
    /// click's modifier (slicer items, data points).
    Send { action: Value, ctrl: bool },
    /// The front end asks for more first: `{"kind": "dropdown" | "range" | "relative" |
    /// "text", "page", "visual", …}` (the viewer's popups: `slDropPopup`, `slRangePopup`,
    /// `slRelativePopup`, `slTextPopup`).
    Popup(Value),
    /// No click action of its own; a right click opens a menu about it: `{"kind": "matrix_row",
    /// "page", "visual", "path": [values], "open": bool|null}` (a matrix row header).
    Menu(Value),
}

/// One clickable or hoverable region of a page.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    /// The visual it belongs to.
    pub visual: String,
    pub shape: HitShape,
    pub action: HitAction,
    /// Tooltip rows (label, value).
    pub tip: Vec<(String, String)>,
    /// The navigator button it is (a page or bookmark name): the front end draws that button in
    /// its hover or press state (`crate::states::Pointer::item`).
    pub item: Option<String>,
}

impl Hit {
    pub(crate) fn moved(&self, dx: f32, dy: f32) -> Hit {
        Hit {
            shape: self.shape.moved(dx, dy),
            ..self.clone()
        }
    }

    /// The action to send for a click with or without Ctrl (`None`: a popup or nothing).
    pub fn action_for(&self, ctrl: bool) -> Option<Value> {
        match &self.action {
            HitAction::Send { action, ctrl: true } => {
                let mut a = action.clone();
                if let Some(o) = a.as_object_mut() {
                    o.insert("ctrl".into(), Value::Bool(ctrl));
                }
                Some(a)
            }
            HitAction::Send { action, .. } => Some(action.clone()),
            _ => None,
        }
    }
}

/// The topmost hit under a point (hits are recorded back to front).
pub fn hit_at(hits: &[Hit], x: f32, y: f32) -> Option<&Hit> {
    hits.iter().rev().find(|h| h.shape.contains(x, y))
}

/// A point: the grouping values of one data point (`{c, v}`: a `data.columns` index and its
/// value), as `view.interact` `select` takes it.
pub type Point = Vec<(usize, Value)>;

pub fn point_json(p: &Point) -> Value {
    Value::Array(p.iter().map(|(c, v)| json!({"c": c, "v": v})).collect())
}

/// The visual being drawn: its page and name, and the selection it is the source of.
#[derive(Debug, Default)]
pub struct Scope {
    pub on: bool,
    pub page: String,
    pub visual: String,
    /// Selected points of this visual (`interaction.selected.points`).
    pub selected: Vec<Point>,
    /// No point hits in this visual (small-multiple panels).
    pub off: bool,
}

/// The renderer's interaction context (part of `Ctx`).
#[derive(Debug, Default)]
pub struct Ix {
    pub scope: RefCell<Scope>,
}

impl Ix {
    pub fn new(on: bool) -> Ix {
        Ix {
            scope: RefCell::new(Scope {
                on,
                ..Scope::default()
            }),
        }
    }

    /// Enters visual `v` of page `page`.
    pub fn enter(&self, page: &str, v: &Value) {
        let mut s = self.scope.borrow_mut();
        s.page = page.to_string();
        s.visual = sk(v, "name").to_string();
        s.off = false;
        s.selected = arr(get(get(get(v, "interaction"), "selected"), "points"))
            .iter()
            .take(1000)
            .map(|p| {
                arr(p)
                    .iter()
                    .take(16)
                    .filter_map(|e| Some((get(e, "c").as_u64()? as usize, get(e, "v").clone())))
                    .collect()
            })
            .collect();
    }

    pub fn on(&self) -> bool {
        self.scope.borrow().on
    }

    pub fn set_off(&self, off: bool) {
        self.scope.borrow_mut().off = off;
    }

    pub fn page_visual(&self) -> (String, String) {
        let s = self.scope.borrow();
        (s.page.clone(), s.visual.clone())
    }

    /// Whether `point` shows as selected in its own visual (`None`: the visual has no
    /// selection), as the viewer's `ptSel`: it matches a selected point when every value they
    /// share is equal.
    pub fn selected(&self, point: &Point) -> Option<bool> {
        let s = self.scope.borrow();
        if s.off || s.selected.is_empty() {
            return None;
        }
        Some(s.selected.iter().any(|sp| {
            sp.iter()
                .all(|(c, v)| match point.iter().find(|(pc, _)| pc == c) {
                    None => true,
                    Some((_, pv)) => same(pv, v),
                })
        }))
    }

    /// The opacity of a data point: [`DIM`] when the selection leaves it out.
    pub fn alpha(&self, point: &Point) -> f32 {
        if self.selected(point) == Some(false) {
            DIM
        } else {
            1.0
        }
    }

    /// The `select` action of a point (`None`: hits off, or no grouping value).
    pub fn select(&self, point: &Point) -> HitAction {
        let s = self.scope.borrow();
        if !s.on || s.off || point.is_empty() {
            return HitAction::None;
        }
        HitAction::Send {
            action: json!({"action": "select", "page": s.page, "visual": s.visual, "point": point_json(point)}),
            ctrl: true,
        }
    }

    /// A slicer action (`slicer_toggle` with `value`, `slicer_select_all`, …).
    pub fn slicer(&self, action: &str, value: Option<&Value>) -> HitAction {
        let s = self.scope.borrow();
        if !s.on {
            return HitAction::None;
        }
        let mut a = json!({"action": action, "page": s.page, "visual": s.visual});
        let toggle = action == "slicer_toggle";
        if toggle {
            a["value"] = value.cloned().unwrap_or(Value::Null);
        }
        HitAction::Send {
            action: a,
            ctrl: toggle,
        }
    }

    /// A popup of this visual (`kind` plus `extra` fields).
    pub fn popup(&self, kind: &str, extra: Value) -> HitAction {
        let s = self.scope.borrow();
        if !s.on {
            return HitAction::None;
        }
        let mut p = json!({"kind": kind, "page": s.page, "visual": s.visual});
        if let (Some(o), Value::Object(e)) = (p.as_object_mut(), extra) {
            o.extend(e);
        }
        HitAction::Popup(p)
    }

    /// A context-menu region of this visual (`kind` plus `extra` fields).
    pub fn menu(&self, kind: &str, extra: Value) -> HitAction {
        let s = self.scope.borrow();
        if !s.on {
            return HitAction::None;
        }
        let mut p = json!({"kind": kind, "page": s.page, "visual": s.visual});
        if let (Some(o), Value::Object(e)) = (p.as_object_mut(), extra) {
            o.extend(e);
        }
        HitAction::Menu(p)
    }

    /// An action without Ctrl (`page`, `button`).
    pub fn plain(&self, action: Value) -> HitAction {
        if !self.on() {
            return HitAction::None;
        }
        HitAction::Send { action, ctrl: false }
    }

    pub fn hit(&self, shape: HitShape, action: HitAction, tip: Vec<(String, String)>) -> Option<Hit> {
        let s = self.scope.borrow();
        if !s.on {
            return None;
        }
        if action == HitAction::None && tip.is_empty() {
            return None;
        }
        Some(Hit {
            visual: s.visual.clone(),
            shape,
            action,
            tip,
            item: None,
        })
    }
}

/// Values compare as the viewer's `ixSame`: text without case, others exactly.
pub fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::String(x), Value::String(y)) => x.to_lowercase() == y.to_lowercase(),
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        _ => a == b,
    }
}

/// The grouping values of category `i` and series `j` of a chart (the viewer's `cdPoint`).
pub fn cd_point(cd: &ChartData, i: Option<usize>, j: Option<usize>) -> Point {
    let mut p = Vec::new();
    if let (Some(ci), Some(i)) = (cd.ci, i) {
        p.push((ci, cd.raw.get(i).cloned().unwrap_or(Value::Null)));
        for (k, j) in cd.more.iter().enumerate() {
            let x = cd
                .raw_more
                .get(i)
                .and_then(|m| m.get(k))
                .cloned()
                .unwrap_or(Value::Null);
            p.push((*j, x));
        }
    }
    let se = j.and_then(|j| cd.series.get(j));
    if let (Some(li), Some(se)) = (cd.li, se)
        && !se.y2
    {
        p.push((li, se.raw.clone()));
    }
    p
}

/// Tooltip rows of category `i`, series `j` (the viewer's `cdTip`).
pub fn cd_tip(cd: &ChartData, i: usize, j: usize, ctx: &Ctx<'_>) -> Vec<(String, String)> {
    let Some(se) = cd.series.get(j) else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    if cd.ci.is_some() {
        rows.push((cd.cat_name.clone(), cd.cats.get(i).cloned().unwrap_or_default()));
    }
    let legend = cd.li.is_some() && !se.y2;
    if legend {
        rows.push((cd.legend_name.clone(), se.name.clone()));
    }
    let label = if legend {
        se.col.name.clone()
    } else {
        se.name.clone()
    };
    let fmt = |x: Option<f64>| match x {
        None => ctx.blank(),
        Some(x) => cell(&json!(x), &se.col, ctx),
    };
    rows.push((label, fmt(se.vals.get(i).copied().flatten())));
    if let Some(h) = se.hvals.as_ref().map(|h| h.get(i).copied().flatten()) {
        let word = if ctx.ui_lang == "fr" {
            "Surligné"
        } else {
            "Highlighted"
        };
        rows.push((word.to_string(), fmt(h)));
    }
    rows
}

/// A cell as the viewer's `fmtCell` writes it.
pub fn cell(x: &Value, c: &Col, ctx: &Ctx<'_>) -> String {
    crate::fmt::fmt_cell(x, Some(&c.cf()), &ctx.loc, false)
}

/// Draws one data-point rectangle of a chart as the viewer's `ixMark` + `hlBar`: the whole mark
/// faded when the selection leaves it out, or when the visual is highlighted (then the
/// highlighted part `part`, a rectangle, is drawn opaque over it); records its hit.
#[allow(clippy::too_many_arguments)]
pub(crate) fn bar_mark(
    cv: &mut crate::canvas::Canvas,
    ctx: &Ctx<'_>,
    cd: &ChartData,
    i: usize,
    j: Option<usize>,
    rect: (f32, f32, f32, f32),
    c: crate::canvas::Rgb,
    part: Option<Option<(f32, f32, f32, f32)>>,
) {
    let pt = cd_point(cd, Some(i), j);
    let (x, y, w, h) = rect;
    let alpha = match part {
        Some(_) => DIM,
        None => ctx.ix.alpha(&pt),
    };
    cv.rect(x, y, w, h, c, alpha);
    if let Some(Some((px, py, pw, ph))) = part {
        cv.rect(px, py, pw, ph, c, 1.0);
    }
    if ctx.ix.on() {
        let tip = cd_tip(cd, i, j.unwrap_or(0), ctx);
        cv.hit(ctx.ix.hit(HitShape::Rect { x, y, w, h }, ctx.ix.select(&pt), tip));
    }
}

/// The highlighted value of series `j` at category `i`: `None` when the visual is not
/// highlighted, `Some(None)` when nothing of that point is.
pub(crate) fn hv(cd: &ChartData, i: usize, j: usize) -> Option<Option<f64>> {
    cd.series
        .get(j)
        .and_then(|s| s.hvals.as_ref())
        .map(|h| h.get(i).copied().flatten())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_contain_their_points() {
        let r = HitShape::Rect {
            x: 10.0,
            y: 10.0,
            w: 5.0,
            h: 5.0,
        };
        assert!(r.contains(12.0, 12.0));
        assert!(!r.contains(16.0, 12.0));
        let c = HitShape::Circle {
            cx: 0.0,
            cy: 0.0,
            r: 2.0,
        };
        assert!(c.contains(1.0, 1.0) && !c.contains(2.0, 2.0));
        let q = std::f32::consts::FRAC_PI_2;
        // the first quarter clockwise from 12 o'clock: up-right
        let s = HitShape::Sector {
            cx: 0.0,
            cy: 0.0,
            r0: 1.0,
            r1: 10.0,
            a0: 0.0,
            a1: q,
        };
        assert!(s.contains(3.0, -3.0));
        assert!(!s.contains(-3.0, -3.0) && !s.contains(3.0, 3.0) && !s.contains(0.1, -0.1));
        // a sector crossing 12 o'clock
        let w = HitShape::Sector {
            cx: 0.0,
            cy: 0.0,
            r0: 0.0,
            r1: 10.0,
            a0: -q,
            a1: q,
        };
        assert!(w.contains(-3.0, -3.0) && w.contains(3.0, -3.0) && !w.contains(0.0, 3.0));
        assert_eq!(r.moved(1.0, 2.0).bounds(), (11.0, 12.0, 5.0, 5.0));
    }

    #[test]
    fn selection_matches_shared_values_without_case() {
        let ix = Ix::new(true);
        ix.enter(
            "P",
            &json!({"name": "V", "interaction": {"selected": {"points": [[{"c": 0, "v": "Bikes"}]]}}}),
        );
        assert_eq!(
            ix.selected(&vec![(0, json!("bikes")), (1, json!(2025))]),
            Some(true)
        );
        assert_eq!(ix.selected(&vec![(0, json!("Cars"))]), Some(false));
        assert_eq!(ix.alpha(&vec![(0, json!("Cars"))]), DIM);
        ix.enter("P", &json!({"name": "W"}));
        assert_eq!(ix.selected(&vec![(0, json!("Cars"))]), None);
        let HitAction::Send { action, ctrl } = ix.select(&vec![(0, json!("Cars"))]) else {
            panic!("a select action");
        };
        assert!(ctrl);
        assert_eq!(action["visual"], "W");
        let hit = ix.hit(
            HitShape::Circle {
                cx: 0.0,
                cy: 0.0,
                r: 1.0,
            },
            ix.select(&vec![(0, json!(1))]),
            vec![],
        );
        assert_eq!(
            hit.and_then(|h| h.action_for(true)).map(|a| a["ctrl"].clone()),
            Some(json!(true))
        );
        // hits off: nothing recorded, no action
        let off = Ix::new(false);
        assert_eq!(off.select(&vec![(0, json!(1))]), HitAction::None);
        assert!(
            off.hit(
                HitShape::Circle {
                    cx: 0.0,
                    cy: 0.0,
                    r: 1.0
                },
                HitAction::None,
                vec![("a".into(), "b".into())]
            )
            .is_none()
        );
    }
}
