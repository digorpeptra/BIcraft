//! Image primitives of the render comparison: an 8-bit RGB raster, PNG input/output through
//! tiny-skia, and the operations the metrics need. The grey conversion, box blur, rank filter and
//! bicubic resize follow Pillow's documented arithmetic (ITU-R 601-2 luma in 16-bit fixed
//! point, edge-extended box blur with 24-bit fixed-point weights, Keys bicubic with a = -0.5,
//! separable, horizontal pass first), so the scores stay comparable with the figures the earlier
//! Python script produced.

use std::path::Path;

/// Largest image accepted (pixels): 16k x 16k.
const MAX_PIXELS: usize = 1 << 28;

/// An 8-bit RGB image, row-major.
#[derive(Debug, Clone, PartialEq)]
pub struct Rgb {
    pub w: usize,
    pub h: usize,
    pub px: Vec<[u8; 3]>,
}

/// An 8-bit single-channel image, row-major.
#[derive(Debug, Clone, PartialEq)]
pub struct Grey {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

impl Rgb {
    pub fn new(w: usize, h: usize, fill: [u8; 3]) -> Option<Rgb> {
        let n = w.checked_mul(h).filter(|n| *n <= MAX_PIXELS)?;
        Some(Rgb {
            w,
            h,
            px: vec![fill; n],
        })
    }

    pub fn get(&self, x: usize, y: usize) -> [u8; 3] {
        if x >= self.w {
            return [0; 3];
        }
        y.checked_mul(self.w)
            .and_then(|i| i.checked_add(x))
            .and_then(|i| self.px.get(i))
            .copied()
            .unwrap_or([0; 3])
    }

    fn set(&mut self, x: usize, y: usize, c: [u8; 3]) {
        if x >= self.w {
            return;
        }
        if let Some(p) = y
            .checked_mul(self.w)
            .and_then(|i| i.checked_add(x))
            .and_then(|i| self.px.get_mut(i))
        {
            *p = c;
        }
    }

    /// Reads a PNG (any colour type tiny-skia decodes); transparent pixels are composited on
    /// white.
    pub fn load_png(path: &Path) -> Result<Rgb, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Rgb::decode_png(&bytes).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn decode_png(bytes: &[u8]) -> Result<Rgb, String> {
        let pm = tiny_skia::Pixmap::decode_png(bytes).map_err(|e| format!("bad PNG: {e}"))?;
        let (w, h) = (pm.width() as usize, pm.height() as usize);
        let mut out = Rgb::new(w, h, [255; 3]).ok_or("image too large")?;
        for (o, p) in out.px.iter_mut().zip(pm.pixels()) {
            // premultiplied over white: c + (1 - a)
            let inv = 255u16.saturating_sub(u16::from(p.alpha()));
            let f = |c: u8| u8::try_from(u16::from(c).saturating_add(inv).min(255)).unwrap_or(255);
            *o = [f(p.red()), f(p.green()), f(p.blue())];
        }
        Ok(out)
    }

    pub fn encode_png(&self) -> Result<Vec<u8>, String> {
        let w = u32::try_from(self.w).map_err(|_| "image too wide")?;
        let h = u32::try_from(self.h).map_err(|_| "image too tall")?;
        let mut pm = tiny_skia::Pixmap::new(w.max(1), h.max(1)).ok_or("cannot allocate image")?;
        for (o, p) in pm.pixels_mut().iter_mut().zip(&self.px) {
            if let Some(c) = tiny_skia::PremultipliedColorU8::from_rgba(p[0], p[1], p[2], 255) {
                *o = c;
            }
        }
        pm.encode_png().map_err(|e| format!("cannot encode PNG: {e}"))
    }

    /// The rectangle [x0, x1) x [y0, y1); parts outside the image are black (as Pillow's crop).
    pub fn crop(&self, x0: i64, y0: i64, x1: i64, y1: i64) -> Option<Rgb> {
        let w = usize::try_from(x1.saturating_sub(x0)).ok()?;
        let h = usize::try_from(y1.saturating_sub(y0)).ok()?;
        let mut out = Rgb::new(w, h, [0; 3])?;
        for y in 0..h {
            let sy = y0.saturating_add(y as i64);
            let Ok(sy) = usize::try_from(sy) else { continue };
            if sy >= self.h {
                continue;
            }
            for x in 0..w {
                let sx = x0.saturating_add(x as i64);
                if let Ok(sx) = usize::try_from(sx)
                    && sx < self.w
                {
                    out.set(x, y, self.get(sx, sy));
                }
            }
        }
        Some(out)
    }

    /// Pastes `src` with its top-left corner at (`x`, `y`), clipped.
    pub fn paste(&mut self, src: &Rgb, x: usize, y: usize) {
        for sy in 0..src.h {
            for sx in 0..src.w {
                self.set(x.saturating_add(sx), y.saturating_add(sy), src.get(sx, sy));
            }
        }
    }

    /// ITU-R 601-2 luma, as Pillow's `convert("L")`: (R·19595 + G·38470 + B·7471 + 2^15) >> 16.
    pub fn grey(&self) -> Grey {
        let px = self
            .px
            .iter()
            .map(|p| {
                let l = u32::from(p[0]) * 19595 + u32::from(p[1]) * 38470 + u32::from(p[2]) * 7471 + 0x8000;
                u8::try_from(l >> 16).unwrap_or(255)
            })
            .collect();
        Grey {
            w: self.w,
            h: self.h,
            px,
        }
    }

    fn channel(&self, c: usize) -> Grey {
        Grey {
            w: self.w,
            h: self.h,
            px: self.px.iter().map(|p| p.get(c).copied().unwrap_or(0)).collect(),
        }
    }

    /// Each channel box-blurred like [`Grey::box_blur`].
    pub fn box_blur(&self, r: usize) -> Rgb {
        let ch: Vec<Grey> = (0..3).map(|c| self.channel(c).box_blur(r)).collect();
        let px = (0..self.px.len())
            .map(|i| {
                let g = |c: usize| ch.get(c).and_then(|g| g.px.get(i)).copied().unwrap_or(0);
                [g(0), g(1), g(2)]
            })
            .collect();
        Rgb {
            w: self.w,
            h: self.h,
            px,
        }
    }

    /// Resized to `w` x `h` with Pillow's bicubic filter (the default of `Image.resize`); the
    /// same size returns a copy.
    pub fn resize(&self, w: usize, h: usize) -> Option<Rgb> {
        if (w, h) == (self.w, self.h) {
            return Some(self.clone());
        }
        let mut cur = self.clone();
        if w != cur.w {
            let k = Kernel::new(cur.w, w)?;
            let mut out = Rgb::new(w, cur.h, [0; 3])?;
            for y in 0..cur.h {
                for x in 0..w {
                    let c = k.apply(x, |i| cur.get(i, y));
                    out.set(x, y, c);
                }
            }
            cur = out;
        }
        if h != cur.h {
            let k = Kernel::new(cur.h, h)?;
            let mut out = Rgb::new(cur.w, h, [0; 3])?;
            for y in 0..h {
                for x in 0..cur.w {
                    let c = k.apply(y, |i| cur.get(x, i));
                    out.set(x, y, c);
                }
            }
            cur = out;
        }
        Some(cur)
    }
}

/// Keys bicubic (a = -0.5), Pillow's `BICUBIC` filter; support 2.
fn bicubic(x: f64) -> f64 {
    const A: f64 = -0.5;
    let x = x.abs();
    if x < 1.0 {
        ((A + 2.0) * x - (A + 3.0)) * x * x + 1.0
    } else if x < 2.0 {
        (((x - 5.0) * x + 8.0) * x - 4.0) * A
    } else {
        0.0
    }
}

/// Fixed-point coefficients of a 1-D resampling pass (Pillow: 22 fractional bits, rounding the
/// accumulator by half a unit, clamped to 0..255).
struct Kernel {
    /// Per output index: first input index and integer weights.
    taps: Vec<(usize, Vec<i64>)>,
}

const PRECISION_BITS: u32 = 22;

impl Kernel {
    fn new(src: usize, dst: usize) -> Option<Kernel> {
        if src == 0 || dst == 0 {
            return None;
        }
        let scale = src as f64 / dst as f64;
        let fscale = scale.max(1.0);
        let support = 2.0 * fscale;
        let mut taps = Vec::with_capacity(dst);
        for xx in 0..dst {
            let center = (xx as f64 + 0.5) * scale;
            let ss = 1.0 / fscale;
            let xmin = ((center - support + 0.5).trunc().max(0.0)) as usize;
            let xmax = ((center + support + 0.5).trunc() as usize).min(src);
            let n = xmax.saturating_sub(xmin);
            let mut w: Vec<f64> = (0..n)
                .map(|x| bicubic(((x + xmin) as f64 - center + 0.5) * ss))
                .collect();
            let total: f64 = w.iter().sum();
            if total != 0.0 {
                for v in &mut w {
                    *v /= total;
                }
            }
            let one = f64::from(1u32 << PRECISION_BITS);
            let iw = w
                .iter()
                .map(|v| {
                    let s = v * one;
                    (if s < 0.0 { s - 0.5 } else { s + 0.5 }).trunc() as i64
                })
                .collect();
            taps.push((xmin, iw));
        }
        Some(Kernel { taps })
    }

    fn apply(&self, o: usize, src: impl Fn(usize) -> [u8; 3]) -> [u8; 3] {
        let Some((first, w)) = self.taps.get(o) else {
            return [0; 3];
        };
        let mut acc = [1i64 << (PRECISION_BITS - 1); 3];
        for (k, wk) in w.iter().enumerate() {
            let p = src(first.saturating_add(k));
            for (a, c) in acc.iter_mut().zip(p) {
                *a = a.saturating_add(wk.saturating_mul(i64::from(c)));
            }
        }
        acc.map(|a| u8::try_from((a >> PRECISION_BITS).clamp(0, 255)).unwrap_or(0))
    }
}

impl Grey {
    #[cfg(test)]
    pub fn new(w: usize, h: usize) -> Option<Grey> {
        let n = w.checked_mul(h).filter(|n| *n <= MAX_PIXELS)?;
        Some(Grey { w, h, px: vec![0; n] })
    }

    #[cfg(test)]
    pub fn get(&self, x: usize, y: usize) -> u8 {
        if x >= self.w {
            return 0;
        }
        y.checked_mul(self.w)
            .and_then(|i| i.checked_add(x))
            .and_then(|i| self.px.get(i))
            .copied()
            .unwrap_or(0)
    }

    /// Box blur of radius `r` (a (2r+1)-wide box), horizontal then vertical, edges extended,
    /// each pass rounded to 8 bits: Pillow's `ImageFilter.BoxBlur(r)`.
    pub fn box_blur(&self, r: usize) -> Grey {
        let h = blur_lines(&self.px, self.w, self.h, 1, self.w, r);
        let v = blur_lines(&h, self.h, self.w, self.w, 1, r);
        Grey {
            w: self.w,
            h: self.h,
            px: v,
        }
    }

    /// Maximum over a (2r+1) square window (Pillow's `MaxFilter(2r+1)`), separable.
    pub fn max_filter(&self, r: usize) -> Grey {
        let max_lines = |src: &[u8], len: usize, lines: usize, step: usize, stride: usize| {
            let mut out = src.to_vec();
            for l in 0..lines {
                for i in 0..len {
                    let lo = i.saturating_sub(r);
                    let hi = i.saturating_add(r).min(len.saturating_sub(1));
                    let m = (lo..=hi)
                        .filter_map(|j| src.get(l * stride + j * step))
                        .copied()
                        .max()
                        .unwrap_or(0);
                    if let Some(o) = out.get_mut(l * stride + i * step) {
                        *o = m;
                    }
                }
            }
            out
        };
        let h = max_lines(&self.px, self.w, self.h, 1, self.w);
        let v = max_lines(&h, self.h, self.w, self.w, 1);
        Grey {
            w: self.w,
            h: self.h,
            px: v,
        }
    }

    /// Median as numpy computes it (mean of the two middle values for an even count).
    pub fn median(&self) -> f64 {
        let mut hist = [0usize; 256];
        for p in &self.px {
            if let Some(h) = hist.get_mut(usize::from(*p)) {
                *h += 1;
            }
        }
        let n = self.px.len();
        if n == 0 {
            return 0.0;
        }
        let nth = |k: usize| -> f64 {
            let mut seen = 0usize;
            for (v, c) in hist.iter().enumerate() {
                seen += c;
                if seen > k {
                    return v as f64;
                }
            }
            255.0
        };
        if n % 2 == 1 {
            nth(n / 2)
        } else {
            (nth(n / 2 - 1) + nth(n / 2)) / 2.0
        }
    }
}

/// One box-blur pass over `lines` lines of `len` samples (`step` apart, lines `stride` apart).
fn blur_lines(src: &[u8], len: usize, lines: usize, step: usize, stride: usize, r: usize) -> Vec<u8> {
    let mut out = src.to_vec();
    if len == 0 {
        return out;
    }
    // Pillow: ww = 2^24 / (2r + 1) (truncated), fw = (2^24 - (2r+1)·ww) / 2 for the far taps
    // (zero for an integer radius); out = (sum·ww + 2^23) >> 24.
    let width = 2 * r as u64 + 1;
    let ww = (1u64 << 24) / width;
    let last = len - 1;
    for l in 0..lines {
        let at =
            |i: usize| -> u64 { u64::from(src.get(l * stride + i.min(last) * step).copied().unwrap_or(0)) };
        for i in 0..len {
            let lo = i as i64 - r as i64;
            let sum: u64 = (0..width)
                .map(|k| {
                    let j = (lo + k as i64).max(0);
                    at(usize::try_from(j).unwrap_or(0))
                })
                .sum();
            let v = (sum * ww + (1 << 23)) >> 24;
            if let Some(o) = out.get_mut(l * stride + i * step) {
                *o = u8::try_from(v.min(255)).unwrap_or(255);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grey_matches_pillow_luma() {
        let mut im = Rgb::new(3, 1, [0; 3]).unwrap();
        im.px = vec![[255, 0, 0], [0, 255, 0], [1, 184, 170]];
        // Pillow: L of (255,0,0) = 76, (0,255,0) = 150, (1,184,170) = 128
        assert_eq!(im.grey().px, vec![76, 150, 128]);
    }

    #[test]
    fn box_blur_extends_edges_and_rounds() {
        let g = Grey {
            w: 5,
            h: 1,
            px: vec![0, 0, 255, 0, 0],
        };
        let b = g.box_blur(2);
        // horizontal: 255/5 = 51 everywhere the box reaches the peak; vertical is a no-op on one row
        assert_eq!(b.px, vec![51, 51, 51, 51, 51]);
        let edge = Grey {
            w: 3,
            h: 1,
            px: vec![255, 0, 0],
        };
        // left edge: 255·3 / 5 = 153 (edge pixel counted three times)
        assert_eq!(edge.box_blur(2).px.first().copied(), Some(153));
    }

    #[test]
    fn max_filter_dilates() {
        let mut g = Grey::new(7, 7).unwrap();
        g.px[3 * 7 + 3] = 255;
        let d = g.max_filter(2);
        assert_eq!(d.px.iter().filter(|p| **p == 255).count(), 25);
        assert_eq!(d.get(1, 1), 255);
        assert_eq!(d.get(0, 0), 0);
    }

    #[test]
    fn median_even_and_odd() {
        let g = Grey {
            w: 4,
            h: 1,
            px: vec![1, 9, 3, 4],
        };
        assert_eq!(g.median(), 3.5);
        let g = Grey {
            w: 3,
            h: 1,
            px: vec![7, 1, 3],
        };
        assert_eq!(g.median(), 3.0);
    }

    #[test]
    fn crop_pads_with_black_and_resize_keeps_flat_colour() {
        let im = Rgb::new(4, 4, [200, 100, 50]).unwrap();
        let c = im.crop(2, 2, 6, 6).unwrap();
        assert_eq!(c.get(0, 0), [200, 100, 50]);
        assert_eq!(c.get(3, 3), [0, 0, 0]);
        let r = im.resize(7, 3).unwrap();
        assert_eq!((r.w, r.h), (7, 3));
        assert!(r.px.iter().all(|p| *p == [200, 100, 50]));
        assert_eq!(im.resize(4, 4).unwrap(), im);
        assert!(im.crop(5, 5, 2, 2).is_none());
    }

    #[test]
    fn png_round_trip() {
        let mut im = Rgb::new(3, 2, [10, 20, 30]).unwrap();
        im.px[4] = [255, 0, 128];
        let back = Rgb::decode_png(&im.encode_png().unwrap()).unwrap();
        assert_eq!(back, im);
        assert!(Rgb::decode_png(b"not a png").is_err());
    }
}
