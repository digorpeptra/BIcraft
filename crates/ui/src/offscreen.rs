//! The window without a window (docs/gaps.md G39, docs/desktop.md "Headless render"): runs the
//! app's frames offscreen and draws them in software, so a shell with no display (a script, a
//! server, a test machine without a GPU) gets the same pixels a window shows.
//!
//! [`Offscreen`] plays the part of the native window: it gives egui its size and DPI and a time
//! that advances by a fixed step per frame, keeps the textures egui asks for (the font atlas, the
//! report page, icons), and answers egui's [`egui::ViewportCommand::Screenshot`] with the frame
//! drawn by [`Raster`]. So the control channel's `ui.screenshot` works here as in a window, and
//! everything else the channel does (`ui.set`, `command`, `ui.wait`) is the same code.
//!
//! [`Raster`] fills egui's tessellated triangles (the meshes a GPU would draw) on the CPU:
//! colours interpolated across each triangle, the texture sampled (bilinear or nearest, as its
//! options say), clipped to the primitive's rectangle, blended with premultiplied alpha. It uses
//! no GPU, no thread and no clock, so the same frame gives the same bytes on every machine.

use std::collections::HashMap;
use std::sync::Arc;

use egui::epaint::{ClippedPrimitive, Primitive, Vertex};
use egui::{
    Color32, ColorImage, Event, FullOutput, ImageData, Pos2, RawInput, Rect, TextureFilter, TextureId,
    TexturesDelta, Vec2, ViewportCommand, ViewportId,
};

use crate::app::App;

/// The largest side of a frame in points, and the largest pixel density.
pub const MAX_SIDE: f32 = 8192.0;
pub const MAX_PPP: f32 = 4.0;
/// The most pixels a frame has (64 M: 256 MB of RGBA).
pub const MAX_PIXELS: usize = 1 << 26;
/// The window's time advances by this much per frame, in seconds (animations end on their own).
pub const FRAME_SECS: f64 = 1.0 / 30.0;

/// A texture as egui sent it (premultiplied sRGBA) with its filter.
struct Texture {
    size: [usize; 2],
    pixels: Vec<Color32>,
    linear: bool,
}

/// The textures egui uses and the software painter of its meshes.
#[derive(Default)]
pub struct Raster {
    textures: HashMap<TextureId, Texture>,
    /// Textures egui freed last frame: removed at the next frame's [`Raster::apply`] (egui frees
    /// a texture after the frame that last painted it).
    freed: Vec<TextureId>,
}

impl Raster {
    /// Applies what egui changed in its textures this frame (new and patched now, freed at the
    /// next frame); the delta is emptied (egui checks it was handled).
    pub fn apply(&mut self, delta: &mut TexturesDelta) {
        for id in std::mem::take(&mut self.freed) {
            self.textures.remove(&id);
        }
        for (id, deltas) in delta.set.drain() {
            for d in &deltas {
                self.set(id, d);
            }
        }
        self.freed.extend(delta.free.drain());
    }

    fn set(&mut self, id: TextureId, d: &egui::epaint::ImageDelta) {
        let ImageData::Color(img) = &d.image;
        let linear = d.options.magnification == TextureFilter::Linear;
        let Some([x0, y0]) = d.pos else {
            self.textures.insert(
                id,
                Texture {
                    size: img.size,
                    pixels: img.pixels.clone(),
                    linear,
                },
            );
            return;
        };
        let Some(t) = self.textures.get_mut(&id) else {
            return;
        };
        t.linear = linear;
        let [w, h] = img.size;
        let [tw, th] = t.size;
        for y in 0..h {
            for x in 0..w {
                let (Some(tx), Some(ty)) = (x0.checked_add(x), y0.checked_add(y)) else {
                    continue;
                };
                if tx >= tw || ty >= th {
                    continue;
                }
                let src = y.checked_mul(w).and_then(|i| i.checked_add(x));
                let dst = ty.checked_mul(tw).and_then(|i| i.checked_add(tx));
                if let (Some(c), Some(p)) = (
                    src.and_then(|i| img.pixels.get(i)),
                    dst.and_then(|i| t.pixels.get_mut(i)),
                ) {
                    *p = *c;
                }
            }
        }
    }

    /// Draws `prims` (positions in points, `ppp` pixels per point) on a `size` pixel frame filled
    /// with `background`. Paint callbacks (none in this UI) are skipped. The frame is opaque.
    pub fn paint(
        &self,
        prims: &[ClippedPrimitive],
        ppp: f32,
        size: [usize; 2],
        background: Color32,
    ) -> Result<ColorImage, String> {
        let [w, h] = size;
        let n = w
            .checked_mul(h)
            .filter(|n| *n > 0 && *n <= MAX_PIXELS)
            .ok_or_else(|| format!("a {w}×{h} frame: at most {MAX_PIXELS} pixels"))?;
        let bg = background.to_opaque();
        let mut px: Vec<[f32; 4]> = vec![rgba(bg); n];
        for p in prims {
            let Primitive::Mesh(mesh) = &p.primitive else {
                continue;
            };
            let clip = Rect::from_min_max(
                Pos2::new(
                    (p.clip_rect.min.x * ppp).round(),
                    (p.clip_rect.min.y * ppp).round(),
                ),
                Pos2::new(
                    (p.clip_rect.max.x * ppp).round(),
                    (p.clip_rect.max.y * ppp).round(),
                ),
            )
            .intersect(Rect::from_min_size(Pos2::ZERO, Vec2::new(w as f32, h as f32)));
            if !clip.is_positive() {
                continue;
            }
            let tex = self.textures.get(&mesh.texture_id);
            for [i, j, k] in mesh.indices.as_chunks::<3>().0 {
                let v = |i: &u32| mesh.vertices.get(*i as usize);
                let (Some(a), Some(b), Some(c)) = (v(i), v(j), v(k)) else {
                    continue;
                };
                triangle(&mut px, w, clip, ppp, [a, b, c], tex);
            }
        }
        let pixels = px
            .iter()
            .map(|c| {
                let q = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
                Color32::from_rgba_premultiplied(q(c[0]), q(c[1]), q(c[2]), 255)
            })
            .collect();
        Ok(ColorImage::new(size, pixels))
    }
}

fn rgba(c: Color32) -> [f32; 4] {
    let [r, g, b, a] = c.to_array();
    [r, g, b, a].map(|x| f32::from(x) / 255.0)
}

/// Fills one triangle into `px` (`w` pixels a row), inside `clip` (pixels). A pixel is covered
/// when its centre is inside; a centre on an edge shared by two triangles belongs to one of
/// them only (the top-left rule), so meshes with shared edges blend each pixel once.
fn triangle(px: &mut [[f32; 4]], w: usize, clip: Rect, ppp: f32, v: [&Vertex; 3], tex: Option<&Texture>) {
    let p = v.map(|x| [x.pos.x * ppp, x.pos.y * ppp]);
    let [p0, p1, p2] = p;
    let area = (p1[0] - p0[0]) * (p2[1] - p0[1]) - (p1[1] - p0[1]) * (p2[0] - p0[0]);
    if !area.is_finite() || area == 0.0 {
        return;
    }
    // the same orientation for every triangle: then the two triangles of a shared edge walk it in
    // opposite directions, and the rule below gives the edge to one of them
    let (v, p) = if area < 0.0 {
        ([v[0], v[2], v[1]], [p0, p2, p1])
    } else {
        (v, p)
    };
    let area = area.abs();
    let edges = [(p[1], p[2]), (p[2], p[0]), (p[0], p[1])];
    let owns = edges.map(|(a, b)| {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        dy < 0.0 || (dy == 0.0 && dx > 0.0)
    });
    let min_x = p
        .iter()
        .map(|q| q[0])
        .fold(f32::INFINITY, f32::min)
        .max(clip.min.x);
    let max_x = p
        .iter()
        .map(|q| q[0])
        .fold(f32::NEG_INFINITY, f32::max)
        .min(clip.max.x);
    let min_y = p
        .iter()
        .map(|q| q[1])
        .fold(f32::INFINITY, f32::min)
        .max(clip.min.y);
    let max_y = p
        .iter()
        .map(|q| q[1])
        .fold(f32::NEG_INFINITY, f32::max)
        .min(clip.max.y);
    if !(min_x < max_x && min_y < max_y) {
        return;
    }
    // pixel columns and rows whose centres can be inside (all within the clip, so within the frame)
    let (x0, x1) = ((min_x - 0.5).ceil().max(0.0) as usize, (max_x - 0.5).floor());
    let (y0, y1) = ((min_y - 0.5).ceil().max(0.0) as usize, (max_y - 0.5).floor());
    if x1 < 0.0 || y1 < 0.0 {
        return;
    }
    let (x1, y1) = (x1 as usize, y1 as usize);
    let cols = [v[0].color, v[1].color, v[2].color].map(rgba);
    for y in y0..=y1 {
        let cy = y as f32 + 0.5;
        if cy >= clip.max.y {
            break;
        }
        for x in x0..=x1 {
            let cx = x as f32 + 0.5;
            if cx >= clip.max.x {
                break;
            }
            let mut wts = [0.0f32; 3];
            let mut inside = true;
            for (k, (a, b)) in edges.iter().enumerate() {
                let e = (b[0] - a[0]) * (cy - a[1]) - (b[1] - a[1]) * (cx - a[0]);
                if e < 0.0 || (e == 0.0 && !owns.get(k).copied().unwrap_or(false)) {
                    inside = false;
                    break;
                }
                if let Some(slot) = wts.get_mut(k) {
                    *slot = e / area;
                }
            }
            if !inside {
                continue;
            }
            let mut src = [0.0f32; 4];
            for (k, c) in cols.iter().enumerate() {
                let wk = wts.get(k).copied().unwrap_or(0.0);
                for (s, ci) in src.iter_mut().zip(c) {
                    *s += wk * ci;
                }
            }
            if let Some(t) = tex {
                let u = wts[0] * v[0].uv.x + wts[1] * v[1].uv.x + wts[2] * v[2].uv.x;
                let vv = wts[0] * v[0].uv.y + wts[1] * v[1].uv.y + wts[2] * v[2].uv.y;
                let s = sample(t, u, vv);
                for (d, si) in src.iter_mut().zip(s) {
                    *d *= si;
                }
            }
            let Some(dst) = y
                .checked_mul(w)
                .and_then(|i| i.checked_add(x))
                .and_then(|i| px.get_mut(i))
            else {
                continue;
            };
            let keep = 1.0 - src[3].clamp(0.0, 1.0);
            for (d, s) in dst.iter_mut().zip(src) {
                *d = s + *d * keep;
            }
        }
    }
}

/// Texture coordinates are sampled on a grid of this many steps per texel.
const TEXEL_STEPS: f32 = 64.0;

/// The texture's colour at normalized coordinates (`u`, `v`), clamped to its edges.
fn sample(t: &Texture, u: f32, v: f32) -> [f32; 4] {
    let [w, h] = t.size;
    if w == 0 || h == 0 {
        return [0.0; 4];
    }
    let at = |x: usize, y: usize| -> [f32; 4] {
        let i = y.min(h - 1) * w + x.min(w - 1);
        t.pixels.get(i).copied().map_or([0.0; 4], rgba)
    };
    // snapped to 1/64 texel: the rounding of `u * w` grows with where the texture's content sits
    // (a glyph's place in egui's atlas depends on the text drawn before it), and the same glyph
    // must give the same pixels wherever it sits
    let snap = |f: f32| (f * TEXEL_STEPS).round() / TEXEL_STEPS;
    let (fx, fy) = (snap(u * w as f32), snap(v * h as f32));
    if !fx.is_finite() || !fy.is_finite() {
        return [0.0; 4];
    }
    if !t.linear {
        let x = fx.floor().clamp(0.0, (w - 1) as f32) as usize;
        let y = fy.floor().clamp(0.0, (h - 1) as f32) as usize;
        return at(x, y);
    }
    let (sx, sy) = (
        (fx - 0.5).clamp(0.0, (w - 1) as f32),
        (fy - 0.5).clamp(0.0, (h - 1) as f32),
    );
    let (x, y) = (sx.floor() as usize, sy.floor() as usize);
    let (tx, ty) = (sx - x as f32, sy - y as f32);
    let (c00, c10, c01, c11) = (at(x, y), at(x + 1, y), at(x, y + 1), at(x + 1, y + 1));
    let mut out = [0.0; 4];
    for (k, o) in out.iter_mut().enumerate() {
        let g = |c: [f32; 4]| c.get(k).copied().unwrap_or(0.0);
        let top = g(c00) + (g(c10) - g(c00)) * tx;
        let bottom = g(c01) + (g(c11) - g(c01)) * tx;
        *o = top + (bottom - top) * ty;
    }
    out
}

/// The app's frames run with no window: a fixed size and DPI, a time that advances by
/// [`FRAME_SECS`] per frame, screenshots drawn by [`Raster`].
pub struct Offscreen {
    pub ctx: egui::Context,
    size: Vec2,
    ppp: f32,
    frames: u64,
    raster: Raster,
    /// Events for the next frame (the screenshots egui asked for).
    events: Vec<Event>,
}

impl Offscreen {
    /// A `size` (points) window at `ppp` pixels per point; refused past [`MAX_SIDE`],
    /// [`MAX_PPP`] or [`MAX_PIXELS`].
    pub fn new(size: Vec2, ppp: f32) -> Result<Offscreen, String> {
        let side_ok = |s: f32| s.is_finite() && (64.0..=MAX_SIDE).contains(&s);
        if !side_ok(size.x) || !side_ok(size.y) {
            return Err(format!(
                "a size of {}×{}: each side from 64 to {MAX_SIDE} points",
                size.x, size.y
            ));
        }
        if !(ppp.is_finite() && (0.5..=MAX_PPP).contains(&ppp)) {
            return Err(format!("a scale of {ppp}: from 0.5 to {MAX_PPP}"));
        }
        let o = Offscreen {
            ctx: egui::Context::default(),
            size,
            ppp,
            frames: 0,
            raster: Raster::default(),
            events: Vec::new(),
        };
        o.pixels()?;
        Ok(o)
    }

    /// The frame's size in pixels.
    pub fn pixels(&self) -> Result<[usize; 2], String> {
        let w = (self.size.x * self.ppp).round() as usize;
        let h = (self.size.y * self.ppp).round() as usize;
        match w.checked_mul(h) {
            Some(n) if n > 0 && n <= MAX_PIXELS => Ok([w, h]),
            _ => Err(format!("a {w}×{h} pixel frame: at most {MAX_PIXELS} pixels")),
        }
    }

    /// Runs one frame of `app` (its logic, then its UI, as the shells do), keeps the textures,
    /// and draws the screenshots egui asked for; returns the frame's output.
    pub fn frame(&mut self, app: &mut App) -> FullOutput {
        let mut input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            time: Some(self.frames as f64 * FRAME_SECS),
            predicted_dt: FRAME_SECS as f32,
            focused: true,
            events: std::mem::take(&mut self.events),
            ..Default::default()
        };
        if let Some(v) = input.viewports.get_mut(&ViewportId::ROOT) {
            v.native_pixels_per_point = Some(self.ppp);
            v.inner_rect = Some(Rect::from_min_size(Pos2::ZERO, self.size));
            v.focused = Some(true);
        }
        self.frames = self.frames.saturating_add(1);
        let mut out = self.ctx.run_ui(input, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        });
        self.raster.apply(&mut out.textures_delta);
        let shots: Vec<_> = out
            .viewport_output
            .iter()
            .flat_map(|(id, v)| {
                v.commands.iter().filter_map(move |c| match c {
                    ViewportCommand::Screenshot(data) => Some((*id, data.clone())),
                    _ => None,
                })
            })
            .collect();
        if !shots.is_empty() {
            // a failed drawing leaves the request unanswered: the channel's own timeout says so
            if let Ok(img) = self.draw(&out) {
                let img = Arc::new(img);
                for (viewport_id, user_data) in shots {
                    self.events.push(Event::Screenshot {
                        viewport_id,
                        user_data,
                        image: Arc::clone(&img),
                    });
                }
                self.ctx.request_repaint();
            }
        }
        out
    }

    /// Draws frame `out` in software.
    pub fn draw(&self, out: &FullOutput) -> Result<ColorImage, String> {
        let prims = self.ctx.tessellate(out.shapes.clone(), out.pixels_per_point);
        let bg = self.ctx.global_style().visuals.panel_fill;
        self.raster
            .paint(&prims, out.pixels_per_point, self.pixels()?, bg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::epaint::Mesh;

    fn mesh_prim(mesh: Mesh) -> ClippedPrimitive {
        ClippedPrimitive {
            clip_rect: Rect::EVERYTHING,
            primitive: Primitive::Mesh(mesh),
        }
    }

    #[test]
    fn a_rectangle_is_filled_once_and_clipped() {
        let mut m = Mesh::default();
        // two triangles sharing a diagonal through pixel centres, half-transparent: a pixel blended
        // twice would be darker
        m.add_colored_rect(
            Rect::from_min_max(Pos2::new(2.0, 2.0), Pos2::new(8.0, 8.0)),
            Color32::from_rgba_premultiplied(0, 0, 128, 128),
        );
        let r = Raster::default();
        let img = r
            .paint(&[mesh_prim(m.clone())], 1.0, [10, 10], Color32::WHITE)
            .unwrap();
        let px = |x: usize, y: usize| img.pixels[y * 10 + x];
        assert_eq!(px(0, 0), Color32::WHITE);
        let inside: Vec<Color32> = (2..8)
            .flat_map(|x| (2..8).map(move |y| (x, y)))
            .map(|(x, y)| px(x, y))
            .collect();
        assert!(inside.iter().all(|c| *c == inside[0]), "{inside:?}");
        assert!(inside[0].b() > inside[0].r(), "{:?}", inside[0]);
        assert_eq!(px(8, 3), Color32::WHITE);
        // a clip rectangle cuts it
        let clipped = ClippedPrimitive {
            clip_rect: Rect::from_min_max(Pos2::ZERO, Pos2::new(5.0, 10.0)),
            primitive: Primitive::Mesh(m),
        };
        let img = r.paint(&[clipped], 1.0, [10, 10], Color32::WHITE).unwrap();
        assert_eq!(img.pixels[3 * 10 + 4], inside[0]);
        assert_eq!(img.pixels[3 * 10 + 6], Color32::WHITE);
    }

    #[test]
    fn a_texture_is_sampled_and_patched() {
        let mut r = Raster::default();
        let id = TextureId::Managed(7);
        let red = ColorImage::new([2, 2], vec![Color32::RED; 4]);
        let mut delta = TexturesDelta::default();
        delta.push(
            id,
            egui::epaint::ImageDelta::full(red, egui::TextureOptions::NEAREST),
        );
        r.apply(&mut delta);
        let mut m = Mesh::with_texture(id);
        m.add_rect_with_uv(
            Rect::from_min_max(Pos2::ZERO, Pos2::new(4.0, 4.0)),
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
        let img = r
            .paint(&[mesh_prim(m.clone())], 1.0, [4, 4], Color32::BLACK)
            .unwrap();
        assert!(img.pixels.iter().all(|c| *c == Color32::RED));
        // the top-left texel turned green: the top-left quarter of the rectangle follows
        let mut delta = TexturesDelta::default();
        let green = ColorImage::new([1, 1], vec![Color32::GREEN]);
        delta.push(
            id,
            egui::epaint::ImageDelta::partial([0, 0], green, egui::TextureOptions::NEAREST),
        );
        r.apply(&mut delta);
        let img = r.paint(&[mesh_prim(m)], 1.0, [4, 4], Color32::BLACK).unwrap();
        assert_eq!(img.pixels[0], Color32::GREEN);
        assert_eq!(img.pixels[3], Color32::RED);
    }

    #[test]
    fn a_glyph_draws_the_same_wherever_it_sits_in_the_atlas() {
        // where egui puts a glyph in its atlas depends on the text drawn before (a status line
        // shown for a frame or not): the glyph's pixels must not
        let (tw, th) = (2048usize, 512usize);
        let patch: Vec<Color32> = (0..16u8)
            .map(|i| Color32::from_rgba_premultiplied(i * 13, i * 7 + 20, 200 - i * 9, 90 + i * 11))
            .collect();
        let draw = |x0: usize, y0: usize, at: Pos2| -> Vec<[f32; 4]> {
            let mut pixels = vec![Color32::TRANSPARENT; tw * th];
            for (i, c) in patch.iter().enumerate() {
                pixels[(y0 + i / 4) * tw + x0 + i % 4] = *c;
            }
            let t = Texture {
                size: [tw, th],
                pixels,
                linear: true,
            };
            let mut m = Mesh::with_texture(TextureId::Managed(0));
            let uv = Rect::from_min_size(
                Pos2::new(x0 as f32 / tw as f32, y0 as f32 / th as f32),
                Vec2::new(4.0 / tw as f32, 4.0 / th as f32),
            );
            m.add_rect_with_uv(
                Rect::from_min_size(at, Vec2::new(4.0, 4.0)),
                uv,
                Color32::from_rgb(250, 240, 230),
            );
            let clip = Rect::from_min_size(Pos2::ZERO, Vec2::new(10.0, 8.0));
            let mut px = vec![[0.0f32; 4]; 80];
            for [i, j, k] in m.indices.as_chunks::<3>().0 {
                let v = |i: &u32| &m.vertices[*i as usize];
                triangle(&mut px, 10, clip, 1.0, [v(i), v(j), v(k)], Some(&t));
            }
            px
        };
        for at in [Pos2::new(3.0, 2.0), Pos2::new(2.25, 1.5), Pos2::new(3.6, 2.3)] {
            let first = draw(1, 1, at);
            assert!(first.iter().any(|p| p[3] > 0.0));
            for x0 in (5..2040).step_by(37) {
                for y0 in [2, 3, 97, 250, 506] {
                    assert!(
                        draw(x0, y0, at) == first,
                        "the patch at ({x0}, {y0}) draws other pixels at {at:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn hostile_sizes_are_refused() {
        assert!(Offscreen::new(Vec2::new(0.0, 100.0), 1.0).is_err());
        assert!(Offscreen::new(Vec2::new(f32::NAN, 100.0), 1.0).is_err());
        assert!(Offscreen::new(Vec2::new(8192.0, 8192.0), 4.0).is_err());
        assert!(Offscreen::new(Vec2::new(800.0, 600.0), 0.0).is_err());
        let r = Raster::default();
        assert!(r.paint(&[], 1.0, [0, 10], Color32::WHITE).is_err());
        assert!(r.paint(&[], 1.0, [usize::MAX, 2], Color32::WHITE).is_err());
    }
}
