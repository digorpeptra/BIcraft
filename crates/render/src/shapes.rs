//! The shape visual's body (Insert â€º Shapes): the tile shapes of Microsoft's public report theme
//! schema (`visual-shape`, `shape.tileShape`) with their fill, outline and rotation, from the
//! snapshot's `shape` object (`docs/view-snapshot.md`, "Visual details"). The geometry and the
//! defaults of the adjustments (slants, snips, roundings, arrowheads) are BIcraft's own reading
//! of the property names, not measured against Desktop (`docs/gaps.md` G54).

use serde_json::Value;
use tiny_skia::{PathBuilder, Transform};

use crate::canvas::{Canvas, Rgb};
use crate::charts::Bx;
use crate::data::{Ctx, get, sk};

/// Bezier handle length of a quarter circle (radius 1).
const KAPPA: f32 = 0.552_284_8;

/// One corner of a rectangle-like shape.
#[derive(Clone, Copy)]
enum Corner {
    Square,
    Round(f32),
    Snip(f32),
}

fn num(v: &Value, k: &str) -> Option<f32> {
    get(v, k).as_f64().filter(|x| x.is_finite()).map(|x| x as f32)
}

fn adjust(sh: &Value, k: &str) -> Option<f32> {
    num(get(sh, "adjust"), k)
}

/// A percentage adjustment (`k`, else `def`) as a fraction in 0..=`max`.
fn pct(sh: &Value, k: &str, def: f32, max: f32) -> f32 {
    (adjust(sh, k).unwrap_or(def) / 100.0).clamp(0.0, max)
}

/// The outline of a box `w` Ã— `h` at (0, 0) with its four corners (top left, top right, bottom
/// right, bottom left), each radius or snip capped at half the shorter side.
fn corners(w: f32, h: f32, c: [Corner; 4]) -> Option<tiny_skia::Path> {
    let cap = (w.min(h) / 2.0).max(0.0);
    let size = |k: Corner| match k {
        Corner::Square => 0.0,
        Corner::Round(r) | Corner::Snip(r) => r.clamp(0.0, cap),
    };
    let [tl, tr, br, bl] = c;
    let mut pb = PathBuilder::new();
    pb.move_to(size(tl), 0.0);
    // top edge, then top right
    pb.line_to(w - size(tr), 0.0);
    match tr {
        Corner::Round(_) => {
            let r = size(tr);
            pb.cubic_to(w - r + r * KAPPA, 0.0, w, r - r * KAPPA, w, r);
        }
        _ => pb.line_to(w, size(tr)),
    }
    pb.line_to(w, h - size(br));
    match br {
        Corner::Round(_) => {
            let r = size(br);
            pb.cubic_to(w, h - r + r * KAPPA, w - r + r * KAPPA, h, w - r, h);
        }
        _ => pb.line_to(w - size(br), h),
    }
    pb.line_to(size(bl), h);
    match bl {
        Corner::Round(_) => {
            let r = size(bl);
            pb.cubic_to(r - r * KAPPA, h, 0.0, h - r + r * KAPPA, 0.0, h - r);
        }
        _ => pb.line_to(0.0, h - size(bl)),
    }
    pb.line_to(0.0, size(tl));
    match tl {
        Corner::Round(_) => {
            let r = size(tl);
            pb.cubic_to(0.0, r - r * KAPPA, r - r * KAPPA, 0.0, r, 0.0);
        }
        _ => pb.line_to(size(tl), 0.0),
    }
    pb.close();
    pb.finish()
}

fn polygon(pts: &[(f32, f32)]) -> Option<tiny_skia::Path> {
    crate::canvas::poly(pts, true)
}

/// The outline of tile shape `tile` in a box `w` Ã— `h` at (0, 0) (`None`: nothing to draw).
pub(crate) fn outline(tile: &str, sh: &Value, w: f32, h: f32) -> Option<tiny_skia::Path> {
    let m = w.min(h);
    let sq = Corner::Square;
    match tile {
        "rectangleRounded" => {
            let r = pct(sh, "rectangleRoundedCurve", 10.0, 1.0) * m / 2.0;
            corners(w, h, [Corner::Round(r); 4])
        }
        "rectangleRoundedByPixel" => {
            let r = adjust(sh, "roundEdge").unwrap_or(10.0);
            corners(w, h, [Corner::Round(r); 4])
        }
        "pill" => corners(w, h, [Corner::Round(m / 2.0); 4]),
        "oval" => {
            tiny_skia::Rect::from_xywh(0.0, 0.0, w.max(0.01), h.max(0.01)).and_then(PathBuilder::from_oval)
        }
        "triangleIsoc" => {
            let t = pct(sh, "isocelesTriangleTipPosition", 50.0, 1.0);
            polygon(&[(w * t, 0.0), (w, h), (0.0, h)])
        }
        "triangleRight" => polygon(&[(0.0, 0.0), (w, h), (0.0, h)]),
        "arrow" => {
            let stem = pct(sh, "arrowStemWidth", 50.0, 1.0) * h;
            let head = (pct(sh, "arrowheadSize", 40.0, 1.0) * w).min(w);
            let (a, b) = ((h - stem) / 2.0, (h + stem) / 2.0);
            polygon(&[
                (0.0, a),
                (w - head, a),
                (w - head, 0.0),
                (w, h / 2.0),
                (w - head, h),
                (w - head, b),
                (0.0, b),
            ])
        }
        "arrowPentagon" => {
            let d = (h / 2.0).min(w);
            polygon(&[(0.0, 0.0), (w - d, 0.0), (w, h / 2.0), (w - d, h), (0.0, h)])
        }
        "arrowChevron" => {
            let d = (h / 2.0).min(w / 2.0);
            polygon(&[
                (0.0, 0.0),
                (w - d, 0.0),
                (w, h / 2.0),
                (w - d, h),
                (0.0, h),
                (d, h / 2.0),
            ])
        }
        "pentagon" => polygon(&[
            (w / 2.0, 0.0),
            (w, h * 0.38),
            (w * 0.81, h),
            (w * 0.19, h),
            (0.0, h * 0.38),
        ]),
        "hexagon" => {
            let s = pct(sh, "hexagonSlant", 25.0, 0.5) * w;
            polygon(&[
                (s, 0.0),
                (w - s, 0.0),
                (w, h / 2.0),
                (w - s, h),
                (s, h),
                (0.0, h / 2.0),
            ])
        }
        "octagon" => {
            let c = pct(sh, "octagonSnipSize", 29.0, 0.5) * m;
            corners(w, h, [Corner::Snip(c); 4])
        }
        "parallelogram" => {
            let s = pct(sh, "parallelogramSlant", 25.0, 1.0) * w;
            polygon(&[(s, 0.0), (w, 0.0), (w - s, h), (0.0, h)])
        }
        "trapezoid" => {
            let s = pct(sh, "trapezoidSlant", 25.0, 0.5) * w;
            polygon(&[(s, 0.0), (w - s, 0.0), (w, h), (0.0, h)])
        }
        "heart" => {
            let mut pb = PathBuilder::new();
            pb.move_to(w / 2.0, h * 0.3);
            pb.cubic_to(w / 2.0, 0.0, 0.0, 0.0, 0.0, h * 0.3);
            pb.cubic_to(0.0, h * 0.6, w / 2.0, h * 0.8, w / 2.0, h);
            pb.cubic_to(w / 2.0, h * 0.8, w, h * 0.6, w, h * 0.3);
            pb.cubic_to(w, 0.0, w / 2.0, 0.0, w / 2.0, h * 0.3);
            pb.close();
            pb.finish()
        }
        "speechbubbleRectangle" => {
            let bh = h * (1.0 - pct(sh, "speechBubbleHeight", 20.0, 0.9));
            let right = sk(sh, "tail").to_ascii_lowercase().contains("right");
            let (a, b, tip) = if right {
                (w * 0.65, w * 0.8, w * 0.85)
            } else {
                (w * 0.2, w * 0.35, w * 0.15)
            };
            polygon(&[
                (0.0, 0.0),
                (w, 0.0),
                (w, bh),
                (b, bh),
                (tip, h),
                (a, bh),
                (0.0, bh),
            ])
        }
        "tabCutCorner" => {
            let c = pct(sh, "tabCutCornerSnipSizeTopRight", 20.0, 0.5) * m;
            corners(w, h, [sq, Corner::Snip(c), sq, sq])
        }
        "tabCutTopCorners" | "tabCutTopCornersByPixel" => {
            let (t, b) = if tile == "tabCutTopCornersByPixel" {
                (
                    adjust(sh, "tabCutCornerSnipSizeTop").unwrap_or(10.0),
                    adjust(sh, "tabCutCornerSnipSizeBottom").unwrap_or(0.0),
                )
            } else {
                (
                    pct(sh, "tabCutCornerSnipSizeTop", 20.0, 0.5) * m,
                    pct(sh, "tabCutCornerSnipSizeBottom", 0.0, 0.5) * m,
                )
            };
            corners(
                w,
                h,
                [Corner::Snip(t), Corner::Snip(t), Corner::Snip(b), Corner::Snip(b)],
            )
        }
        "tabRoundCorner" => {
            let r = pct(sh, "tabRoundCornerTopRight", 20.0, 0.5) * m;
            corners(w, h, [sq, Corner::Round(r), sq, sq])
        }
        "tabRoundTopCorners" => {
            let t = pct(sh, "tabRoundCornerTop", 20.0, 0.5) * m;
            let b = pct(sh, "tabRoundCornerBottom", 0.0, 0.5) * m;
            corners(
                w,
                h,
                [
                    Corner::Round(t),
                    Corner::Round(t),
                    Corner::Round(b),
                    Corner::Round(b),
                ],
            )
        }
        // rectangle, and a tile shape BIcraft does not know
        _ => corners(w, h, [sq; 4]),
    }
}

/// The shape visual: its tile shape (a rectangle by default) filled with the theme's first data
/// colour and a slightly darker 1 px outline unless the visual sets them, turned by its rotation
/// about the box's centre. A line is its outline across the middle of the box.
pub fn shape_body(cv: &mut Canvas, v: &Value, b: Bx, ctx: &Ctx<'_>) {
    let sh = get(v, "shape");
    let tile = sk(sh, "tile");
    let base = ctx.color(0);
    let fill = Rgb::from_value(get(sh, "fill")).unwrap_or(base);
    let fill_a = 1.0 - (num(sh, "fill_transparency").unwrap_or(0.0) / 100.0).clamp(0.0, 1.0);
    let show_fill = get(sh, "show_fill").as_bool().unwrap_or(true);
    let is_line = tile == "line";
    let line_default = if is_line {
        base
    } else {
        base.mix(Rgb(0, 0, 0), 0.08)
    };
    let line = Rgb::from_value(get(sh, "line")).unwrap_or(line_default);
    let line_a = 1.0 - (num(sh, "line_transparency").unwrap_or(0.0) / 100.0).clamp(0.0, 1.0);
    let weight = num(sh, "line_weight")
        .unwrap_or(if is_line { 3.0 } else { 1.0 })
        .clamp(0.0, 100.0);
    let show_line = get(sh, "show_line").as_bool().unwrap_or(true) && weight > 0.0;
    let angle = num(sh, "angle").unwrap_or(0.0).rem_euclid(360.0);
    let (cx, cy) = (b.w / 2.0, b.h / 2.0);
    let turn = Transform::from_rotate_at(angle, cx, cy);
    if is_line {
        if !show_line {
            return;
        }
        let round = sk(sh, "cap").eq_ignore_ascii_case("round");
        let inset = if round { weight / 2.0 } else { 0.0 };
        let mut p = [
            tiny_skia::Point::from_xy(inset, cy),
            tiny_skia::Point::from_xy(b.w - inset, cy),
        ];
        turn.map_points(&mut p);
        let [a, z] = p;
        cv.line_a(a.x, a.y, z.x, z.y, line, line_a, weight, None, round);
        return;
    }
    // the outline stays inside the box
    let inset = if show_line { weight / 2.0 } else { 0.0 };
    let (w, h) = ((b.w - 2.0 * inset).max(0.0), (b.h - 2.0 * inset).max(0.0));
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let Some(path) = outline(tile, sh, w, h)
        .and_then(|p| p.transform(Transform::from_translate(inset, inset).post_concat(turn)))
    else {
        return;
    };
    // a turned shape that leaves its box is scaled down to fit it, centred (BIcraft's choice:
    // clipped corners read as a drawing fault)
    let r = path.bounds();
    let inside = r.left() >= inset - 0.5
        && r.top() >= inset - 0.5
        && r.right() <= b.w - inset + 0.5
        && r.bottom() <= b.h - inset + 0.5;
    let path = if inside || r.width() <= 0.0 || r.height() <= 0.0 {
        Some(path)
    } else {
        let s = (w / r.width()).min(h / r.height()).min(1.0);
        let (mx, my) = (r.left() + r.width() / 2.0, r.top() + r.height() / 2.0);
        path.transform(
            Transform::from_translate(-mx, -my)
                .post_scale(s, s)
                .post_translate(cx, cy),
        )
    };
    let Some(path) = path else {
        return;
    };
    if show_fill && fill_a > 0.0 {
        cv.fill_path(&path, fill, fill_a);
    }
    if show_line && line_a > 0.0 {
        cv.stroke(&path, line, line_a, weight, None);
    }
    shape_text(cv, get(sh, "text"), b, ctx);
}

/// The text of a shape (Format > Style > Text): its lines in the box, upright, aligned as it says
/// (centred by default, as BIcraft chose: no Desktop default was observed), in its font colour
/// (the theme's text colour by default), size (10 pt by default), bold and italic.
fn shape_text(cv: &mut Canvas, t: &Value, b: Bx, ctx: &Ctx<'_>) {
    use crate::canvas::{Anchor, Base, Text};
    let text = sk(t, "text");
    if text.is_empty() || get(t, "show_text") == &Value::Bool(false) {
        return;
    }
    let px = (num(t, "font_size").unwrap_or(10.0).clamp(1.0, 200.0)) * crate::data::PT;
    let f = crate::charts::font(ctx, px)
        .weight(if get(t, "bold") == &Value::Bool(true) {
            700
        } else {
            400
        })
        .italic(get(t, "italic") == &Value::Bool(true));
    let color = Rgb::from_value(get(t, "font_color")).unwrap_or(ctx.fg);
    let lines: Vec<&str> = text.lines().take(50).collect();
    let lh = px * 1.3;
    let total = lh * lines.len() as f32;
    let pad = 6.0_f32.min(b.w / 4.0);
    let top = match sk(t, "valign") {
        "top" => pad,
        "bottom" => b.h - pad - total,
        _ => (b.h - total) / 2.0,
    };
    let (x, anchor) = match sk(t, "align") {
        "left" => (pad, Anchor::Start),
        "right" => (b.w - pad, Anchor::End),
        _ => (b.w / 2.0, Anchor::Middle),
    };
    for (i, l) in lines.iter().enumerate() {
        let s = ctx.fonts.fit(&f, l, (b.w - 2.0 * pad).max(0.0));
        let y = top + lh * (i as f32 + 0.5);
        cv.text(
            ctx.fonts,
            &Text::new(&s, f.clone(), color).anchor(anchor).base(Base::Central),
            x,
            y,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    /// The colour at (x, y) of a 400 Ã— 200 page holding `visuals`.
    fn page(visuals: Value) -> tiny_skia::Pixmap {
        let s = json!({"version": 1, "model": {"tables": []},
            "report": {"theme": null, "pages": [{"name": "p", "display_name": "P", "width": 400, "height": 200,
                "visuals": visuals}]}});
        let fonts = crate::FontBook::bundled_only();
        let o = crate::RenderOptions {
            system_fonts: false,
            ..crate::RenderOptions::default()
        };
        let img = crate::render_page(&s, 0, &HashMap::<String, Vec<u8>>::new(), &fonts, &o)
            .unwrap_or_else(|e| panic!("{e:?}"));
        tiny_skia::Pixmap::decode_png(&img.png).unwrap()
    }

    fn rgb(pm: &tiny_skia::Pixmap, x: u32, y: u32) -> (u8, u8, u8) {
        let p = pm.pixel(x, y).unwrap();
        (p.red(), p.green(), p.blue())
    }

    /// A turned shape that would leave its box is scaled to fit it: a 200 Ã— 100 rectangle at
    /// 45Â° becomes a 100 Ã— 100 diamond about the centre, not a clipped band.
    #[test]
    fn turned_shapes_fit_their_box() {
        let pm = page(
            json!([{"name": "r", "type": "shape", "x": 0, "y": 0, "width": 200, "height": 100,
            "shape": {"tile": "rectangle", "fill": "#FF0000", "show_line": false, "angle": 45}}]),
        );
        assert_eq!(rgb(&pm, 100, 50), (255, 0, 0));
        assert_eq!(rgb(&pm, 100, 20), (255, 0, 0), "the diamond near the top");
        assert_eq!(rgb(&pm, 40, 50), (255, 255, 255), "not the clipped turned band");
    }

    #[test]
    fn shapes_draw_their_tile_fill_outline_and_rotation() {
        let red = (255, 0, 0);
        let white = (255, 255, 255);
        // an oval: red inside, the page in the box's corner
        let pm = page(
            json!([{"name": "o", "type": "shape", "x": 0, "y": 0, "width": 200, "height": 100,
            "shape": {"tile": "oval", "fill": "#FF0000", "show_line": false}}]),
        );
        assert_eq!(rgb(&pm, 100, 50), red);
        assert_eq!(rgb(&pm, 3, 3), white);
        // a rectangle fills its corner; a transparent fill shows the page through
        let pm = page(
            json!([{"name": "r", "type": "shape", "x": 0, "y": 0, "width": 200, "height": 100,
            "shape": {"fill": "#FF0000", "show_line": false}},
            {"name": "t", "type": "shape", "x": 210, "y": 0, "width": 100, "height": 100,
            "shape": {"fill": "#FF0000", "show_fill": false, "line": "#0000FF", "line_weight": 4}}]),
        );
        assert_eq!(rgb(&pm, 3, 3), red);
        assert_eq!(rgb(&pm, 260, 50), white, "no fill");
        assert_eq!(rgb(&pm, 211, 50), (0, 0, 255), "the outline");
        // a triangle's top corners are empty; turned by 180Â° its bottom corners are
        let tri = |angle: f64| {
            page(
                json!([{"name": "t", "type": "shape", "x": 0, "y": 0, "width": 200, "height": 200,
                "shape": {"tile": "triangleIsoc", "fill": "#FF0000", "show_line": false, "angle": angle}}]),
            )
        };
        let up = tri(0.0);
        assert_eq!(rgb(&up, 5, 5), white);
        assert_eq!(rgb(&up, 100, 190), red);
        let down = tri(180.0);
        assert_eq!(rgb(&down, 5, 195), white);
        assert_eq!(rgb(&down, 100, 10), red);
        // a line crosses the middle of its box in its outline colour
        let pm = page(
            json!([{"name": "l", "type": "shape", "x": 0, "y": 0, "width": 200, "height": 40,
            "shape": {"tile": "line", "line": "#00FF00", "line_weight": 4}}]),
        );
        assert_eq!(rgb(&pm, 100, 20), (0, 255, 0));
        assert_eq!(rgb(&pm, 100, 5), white);
    }

    #[test]
    fn every_tile_shape_has_an_outline_at_any_size() {
        let tiles = [
            "rectangle",
            "rectangleRounded",
            "rectangleRoundedByPixel",
            "oval",
            "triangleIsoc",
            "triangleRight",
            "arrow",
            "arrowChevron",
            "arrowPentagon",
            "pentagon",
            "hexagon",
            "octagon",
            "parallelogram",
            "trapezoid",
            "pill",
            "heart",
            "speechbubbleRectangle",
            "tabCutCorner",
            "tabCutTopCorners",
            "tabCutTopCornersByPixel",
            "tabRoundCorner",
            "tabRoundTopCorners",
            "unknown",
        ];
        let hostile = json!({"adjust": {"rectangleRoundedCurve": 1e9, "hexagonSlant": -50,
            "roundEdge": f64::MAX, "octagonSnipSize": 400}});
        for t in tiles {
            for (w, h) in [(200.0, 100.0), (1.0, 1.0), (0.5, 300.0)] {
                assert!(outline(t, &json!({}), w, h).is_some(), "{t} {w}x{h}");
                // hostile adjustments are clamped, never a crash
                let _ = outline(t, &hostile, w, h);
            }
        }
    }
}
