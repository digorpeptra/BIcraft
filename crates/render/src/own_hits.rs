//! The interactions of BIcraft's own visuals in the app (the ten of `crate::bvis`,
//! `crate::chart_pack` and the Spec visual, `crate::spec`): what their TypeScript runtime does
//! inside Power BI (docs/community-visuals.md "Runtime"), drawn and recorded by the renderer so
//! the desktop and browser app, and the live viewer, behave the same.
//!
//! A layout says which primitives draw which data rows (a *target*: a bin, a box, a bullet, a
//! node, a link, a set…). Here each target becomes hit regions (`crate::hits`) whose click is a
//! `view.interact` `select` of the target's rows, as their selection ids over the visual's
//! identity columns (the TypeScript's `idOf` / `idRole`): one point per row, `point` the first
//! and `points` all of them when there are several. A target is drawn at [`crate::hits::DIM`]
//! when the visual is the source of a selection that leaves any of its rows out (the runtime's
//! `rowsOf` / `dimmed`), or when the layout faded it because another visual's selection
//! highlights none of its rows (`faded`, docs/formats/bicraft-visuals.md "Highlights"). Tooltips
//! are the TypeScript's rows, values written with their column's format.

use serde_json::{Value, json};

use crate::canvas::Canvas;
use crate::data::{Col, Ctx, Dataset, arr, get};
use crate::hits::{DIM, HitAction, HitShape, cell};
use crate::spec::{Prim, est};

/// Most selection points one target sends (the engine's cap on a selection).
pub const MAX_POINTS: usize = 1_000;
/// Most identity columns of a point (the engine's cap).
const MAX_ID_COLUMNS: usize = 16;
/// Half the thickness of the hit band around a line, in pixels.
const LINE_SLOP: f64 = 3.0;

/// One thing the user selects and hovers: its data rows, the primitives drawn for it, points of
/// a line it is at (`at`), and its tooltip.
#[derive(Debug, Clone, Default)]
pub struct Target {
    pub rows: Vec<usize>,
    pub prims: Vec<usize>,
    pub at: Vec<(f64, f64)>,
    pub tip: Vec<(String, String)>,
    /// Drawn dimmed: another visual's selection highlights none of its rows.
    pub faded: bool,
    /// A `view.interact` action its click sends instead of selecting its rows (the org chart's
    /// collapse button: `{"action": "tree_toggle", "id"}`); the page and visual are added.
    pub send: Option<Value>,
}

/// The dataset columns of the visual's identity roles that can be selected (grouping columns:
/// a measure gives no selection id).
pub fn id_cols(ds: &Dataset, roles: &[&str]) -> Vec<usize> {
    ds.cols
        .iter()
        .enumerate()
        .filter(|(_, c)| roles.iter().any(|r| c.role.eq_ignore_ascii_case(r)))
        .filter(|(_, c)| c.kind != "measure" && c.kind != "aggregation")
        .map(|(i, _)| i)
        .take(MAX_ID_COLUMNS)
        .collect()
}

/// The `view.interact` points of `rows`: each row's values in the identity columns, without
/// duplicates, at most [`MAX_POINTS`].
pub fn points(ds: &Dataset, ids: &[usize], rows: &[usize]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    if ids.is_empty() {
        return out;
    }
    for r in rows {
        let Some(row) = ds.rows.get(*r) else { continue };
        let p = Value::Array(
            ids.iter()
                .map(|c| json!({"c": c, "v": row.get(*c).cloned().unwrap_or(Value::Null)}))
                .collect(),
        );
        if !out.contains(&p) {
            out.push(p);
            if out.len() >= MAX_POINTS {
                break;
            }
        }
    }
    out
}

/// The rows the visual's own selection matches (`interaction.selected.rows`); `None` when the
/// visual is not the source of a selection.
fn selected_rows(v: &Value) -> Option<Vec<usize>> {
    let sel = get(get(v, "interaction"), "selected");
    if !sel.is_object() {
        return None;
    }
    Some(
        arr(get(sel, "rows"))
            .iter()
            .take(100_000)
            .filter_map(|r| r.as_u64().and_then(|r| usize::try_from(r).ok()))
            .collect(),
    )
}

/// Whether a target is drawn dimmed: the visual has a selection and the target has rows not all
/// of which it selects.
fn dimmed(t: &Target, sel: Option<&[usize]>) -> bool {
    match sel {
        Some(s) => !t.rows.is_empty() && !t.rows.iter().all(|r| s.contains(r)),
        None => false,
    }
}

/// Draws `prims` with the targets' selection state and records each target's hits.
pub(crate) fn draw(
    cv: &mut Canvas,
    prims: &[Prim],
    targets: &[Target],
    v: &Value,
    ds: &Dataset,
    ids: &[usize],
    ctx: &Ctx<'_>,
) {
    let sel = selected_rows(v);
    let mut alpha = vec![1.0_f64; prims.len()];
    for t in targets.iter().filter(|t| t.faded || dimmed(t, sel.as_deref())) {
        for p in &t.prims {
            if let Some(a) = alpha.get_mut(*p) {
                *a = f64::from(DIM);
            }
        }
    }
    crate::spec::draw_with(cv, prims, ctx, &|i| alpha.get(i).copied().unwrap_or(1.0));
    if !ctx.ix.on() {
        return;
    }
    let off = ctx.ix.scope.borrow().off;
    let (page, visual) = ctx.ix.page_visual();
    for t in targets {
        let pts = points(ds, ids, &t.rows);
        let action = match (pts.first(), &t.send) {
            (_, Some(send)) if !off => {
                let mut a = send.clone();
                if let Some(o) = a.as_object_mut() {
                    o.insert("page".into(), json!(page));
                    o.insert("visual".into(), json!(visual));
                }
                HitAction::Send {
                    action: a,
                    ctrl: false,
                }
            }
            (_, Some(_)) => HitAction::None,
            (Some(first), None) if !off => {
                let mut a = json!({"action": "select", "page": page, "visual": visual, "point": first});
                if pts.len() > 1 {
                    a["points"] = Value::Array(pts.clone());
                }
                HitAction::Send {
                    action: a,
                    ctrl: true,
                }
            }
            _ => HitAction::None,
        };
        let mut shapes: Vec<HitShape> = t
            .prims
            .iter()
            .filter_map(|p| prims.get(*p))
            .flat_map(shapes_of)
            .collect();
        shapes.extend(t.at.iter().map(|(x, y)| HitShape::Circle {
            cx: *x as f32,
            cy: *y as f32,
            r: 5.0,
        }));
        for s in shapes {
            cv.hit(ctx.ix.hit(s, action.clone(), t.tip.clone()));
        }
    }
}

/// The hit regions of a primitive: its own shape, a band around a line, a text's box.
pub(crate) fn shapes_of(p: &Prim) -> Vec<HitShape> {
    let f = |x: f64| x as f32;
    match p {
        Prim::Rect { x, y, w, h, .. } => vec![HitShape::Rect {
            x: f(*x),
            y: f(*y),
            w: f(w.max(1.0)),
            h: f(h.max(1.0)),
        }],
        Prim::Circle { cx, cy, r, .. } => vec![HitShape::Circle {
            cx: f(*cx),
            cy: f(*cy),
            r: f(r.max(4.0)),
        }],
        Prim::Poly { points, .. } if points.len() >= 3 => vec![HitShape::Polygon {
            rings: vec![points.iter().map(|(x, y)| (f(*x), f(*y))).collect()],
        }],
        Prim::Poly { .. } => Vec::new(),
        Prim::Line {
            x1,
            y1,
            x2,
            y2,
            width,
            ..
        } => band((*x1, *y1), (*x2, *y2), *width).into_iter().collect(),
        Prim::Path { points, width, .. } => points
            .windows(2)
            .take(2_000)
            .filter_map(|w| match w {
                [a, b] => band(*a, *b, *width),
                _ => None,
            })
            .collect(),
        Prim::Text {
            x,
            y,
            text,
            size,
            anchor,
            rotate,
            ..
        } if *rotate == 0.0 && !text.is_empty() => {
            let w = est(text, *size);
            let x0 = match *anchor {
                "middle" => x - w / 2.0,
                "end" => x - w,
                _ => *x,
            };
            vec![HitShape::Rect {
                x: f(x0),
                y: f(y - size / 2.0),
                w: f(w),
                h: f(*size),
            }]
        }
        Prim::Text { .. } => Vec::new(),
    }
}

/// A band of half-width `width / 2` (at least [`LINE_SLOP`]) around the segment `a`–`b`.
fn band(a: (f64, f64), b: (f64, f64), width: f64) -> Option<HitShape> {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt();
    let half = (width / 2.0).max(LINE_SLOP);
    if !(len.is_finite() && half.is_finite()) {
        return None;
    }
    let (nx, ny) = if len > 1e-9 {
        (-dy / len * half, dx / len * half)
    } else {
        (0.0, half)
    };
    // a degenerate segment: a square around its point
    let (ex, ey) = if len > 1e-9 { (0.0, 0.0) } else { (half, 0.0) };
    let ring = vec![
        ((a.0 + nx - ex) as f32, (a.1 + ny - ey) as f32),
        ((b.0 + nx + ex) as f32, (b.1 + ny + ey) as f32),
        ((b.0 - nx + ex) as f32, (b.1 - ny + ey) as f32),
        ((a.0 - nx - ex) as f32, (a.1 - ny - ey) as f32),
    ];
    Some(HitShape::Polygon { rings: vec![ring] })
}

// ---------------------------------------------------------------------------------------------
// Tooltips (the TypeScript's, in the UI's language)

/// `en` or `fr` by the UI's language.
pub(crate) fn word(ctx: &Ctx<'_>, en: &'static str, fr: &'static str) -> String {
    if ctx.ui_lang.to_ascii_lowercase().starts_with("fr") {
        fr
    } else {
        en
    }
    .to_string()
}

/// Column `c`'s cell at row `r`, written with its format (`(Blank)` for a blank).
pub(crate) fn cell_text(ds: &Dataset, c: usize, r: usize, ctx: &Ctx<'_>) -> Option<String> {
    let col = ds.cols.get(c)?;
    let x = ds.rows.get(r)?.get(c)?;
    Some(if x.is_null() {
        ctx.blank()
    } else {
        cell(x, col, ctx)
    })
}

/// A number written with column `c`'s format (none: as BIcraft writes numbers).
pub(crate) fn num_text(ds: &Dataset, c: Option<usize>, x: f64, ctx: &Ctx<'_>) -> String {
    if !x.is_finite() {
        return ctx.blank();
    }
    match c.and_then(|c| ds.cols.get(c)) {
        Some(col) => cell(&json!(x), col, ctx),
        None => crate::spec::fmt_auto(x),
    }
}

/// The first column of `role`.
pub(crate) fn role_col(ds: &Dataset, role: &str) -> Option<usize> {
    ds.cols.iter().position(|c| c.role.eq_ignore_ascii_case(role))
}

/// The tooltip line of column `c` at row `r` (none without the column): its name and value.
pub(crate) fn tip_line(ds: &Dataset, c: Option<usize>, r: usize, ctx: &Ctx<'_>) -> Vec<(String, String)> {
    let Some(c) = c else { return Vec::new() };
    match (ds.cols.get(c), cell_text(ds, c, r, ctx)) {
        (Some(col), Some(t)) => vec![(col_name(col), t)],
        _ => Vec::new(),
    }
}

fn col_name(c: &Col) -> String {
    crate::data::clip(&c.name, 120)
}

/// Every field of row `r` (a one-row target's tooltip, the TypeScript's `rowTooltip`).
pub(crate) fn row_tip(ds: &Dataset, r: usize, ctx: &Ctx<'_>) -> Vec<(String, String)> {
    (0..ds.cols.len().min(16))
        .flat_map(|c| tip_line(ds, Some(c), r, ctx))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_texts_and_shapes_have_hit_regions() {
        let l = crate::spec::line(0.0, 10.0, 100.0, 10.0, "#000000", 1.0);
        let s = shapes_of(&l);
        assert_eq!(s.len(), 1);
        assert!(s[0].contains(50.0, 12.0) && !s[0].contains(50.0, 14.0));
        let t = crate::spec::text(50.0, 20.0, "Hello".into(), 10.0, "#000000", "middle", 0.0);
        let s = shapes_of(&t);
        assert!(s[0].contains(50.0, 20.0) && !s[0].contains(80.0, 20.0));
        let p = Prim::Path {
            points: vec![(0.0, 0.0), (10.0, 10.0), (20.0, 0.0)],
            stroke: "#000000".into(),
            width: 2.0,
        };
        assert_eq!(shapes_of(&p).len(), 2);
        // a degenerate segment still has an area
        assert!(band((5.0, 5.0), (5.0, 5.0), 1.0).is_some_and(|b| b.contains(5.0, 5.0)));
    }

    #[test]
    fn a_target_is_dimmed_when_the_selection_leaves_a_row_out() {
        let t = Target {
            rows: vec![1, 2],
            ..Target::default()
        };
        assert!(!dimmed(&t, None));
        assert!(!dimmed(&t, Some(&[1, 2, 3])));
        assert!(dimmed(&t, Some(&[1])));
        assert!(dimmed(&t, Some(&[])));
        assert!(!dimmed(&Target::default(), Some(&[1])));
    }
}
