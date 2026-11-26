//! Per-visual scores between Desktop's crop (`d`) and bicraft's crop (`o`) of the same size.
//!
//! - `sim`: 1 − mean |grey(d) − grey(o)| / 255 after a 2 px box blur of each grey crop.
//! - `ink`: intersection over union of the ink masks (pixels whose grey differs from the crop's
//!   median by more than 24, dilated 2 px); 1 when neither crop has ink.
//! - `ssim`: mean structural similarity of the grey crops (Wang et al. 2004: 11×11 Gaussian
//!   window, σ = 1.5, K1 = 0.01, K2 = 0.03, L = 255), over the windows that fit inside the crop
//!   (no padding); `None` for a crop smaller than the window.
//! - `de`: mean CIEDE2000 colour difference (sRGB, D65) over the pixels that are ink in both crops
//!   (masks not dilated), on the 2 px box-blurred colours; `None` without common ink.

use super::img::{Grey, Rgb};

/// Grey difference above which a pixel counts as ink.
const INK_THRESHOLD: f64 = 24.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scores {
    pub sim: f64,
    pub ink: f64,
    pub ssim: Option<f64>,
    pub de: Option<f64>,
}

pub fn score(d: &Rgb, o: &Rgb) -> Scores {
    let (gd, go) = (d.grey(), o.grey());
    let (bd, bo) = (gd.box_blur(2), go.box_blur(2));
    let n = bd.px.len().min(bo.px.len());
    let sim = if n == 0 {
        1.0
    } else {
        let s: u64 = bd
            .px
            .iter()
            .zip(&bo.px)
            .map(|(a, b)| u64::from(a.abs_diff(*b)))
            .sum();
        1.0 - s as f64 / 255.0 / n as f64
    };
    let (md, mo) = (ink_mask(&gd), ink_mask(&go));
    let (dd, dout) = (md.max_filter(2), mo.max_filter(2));
    let (mut inter, mut union) = (0u64, 0u64);
    for (a, b) in dd.px.iter().zip(&dout.px) {
        let (a, b) = (*a > 0, *b > 0);
        inter += u64::from(a && b);
        union += u64::from(a || b);
    }
    let ink = if union == 0 {
        1.0
    } else {
        inter as f64 / union as f64
    };
    let de = {
        let (cd, co) = (d.box_blur(2), o.box_blur(2));
        let (mut sum, mut k) = (0.0, 0u64);
        for (i, (a, b)) in md.px.iter().zip(&mo.px).enumerate() {
            if *a > 0
                && *b > 0
                && let (Some(p), Some(q)) = (cd.px.get(i), co.px.get(i))
            {
                sum += ciede2000(lab(*p), lab(*q));
                k += 1;
            }
        }
        (k > 0).then(|| sum / k as f64)
    };
    Scores {
        sim,
        ink,
        ssim: ssim(&gd, &go),
        de,
    }
}

/// 255 where the pixel differs from the median grey by more than [`INK_THRESHOLD`].
pub fn ink_mask(g: &Grey) -> Grey {
    let bg = g.median();
    Grey {
        w: g.w,
        h: g.h,
        px: g
            .px
            .iter()
            .map(|p| {
                if (f64::from(*p) - bg).abs() > INK_THRESHOLD {
                    255
                } else {
                    0
                }
            })
            .collect(),
    }
}

const WIN: usize = 11;
/// Index in `maps` of: mean a, mean b, a², b², ab.
const MAP: [usize; 5] = [3, 4, 0, 1, 2];

fn gauss() -> [f64; WIN] {
    let mut k = [0.0; WIN];
    let c = (WIN / 2) as f64;
    for (i, v) in k.iter_mut().enumerate() {
        let x = i as f64 - c;
        *v = (-(x * x) / (2.0 * 1.5 * 1.5)).exp();
    }
    let s: f64 = k.iter().sum();
    k.map(|v| v / s)
}

/// Mean SSIM (see the module notes); `None` when the images differ in size or are smaller than
/// the 11×11 window.
pub fn ssim(a: &Grey, b: &Grey) -> Option<f64> {
    if (a.w, a.h) != (b.w, b.h) || a.w < WIN || a.h < WIN {
        return None;
    }
    let (w, h) = (a.w, a.h);
    let k = gauss();
    let (ow, oh) = (w - WIN + 1, h - WIN + 1);
    // the five maps, filtered horizontally then vertically (valid region only)
    let fa: Vec<f64> = a.px.iter().map(|v| f64::from(*v)).collect();
    let fb: Vec<f64> = b.px.iter().map(|v| f64::from(*v)).collect();
    let src: [Vec<f64>; 5] = [
        fa.iter().map(|v| v * v).collect(),
        fb.iter().map(|v| v * v).collect(),
        fa.iter().zip(&fb).map(|(x, y)| x * y).collect(),
        fa,
        fb,
    ];
    let mut maps: Vec<Vec<f64>> = Vec::with_capacity(5);
    for f in &src {
        let mut hz = vec![0.0; ow * h];
        for y in 0..h {
            let Some(row) = f.get(y * w..(y + 1) * w) else {
                continue;
            };
            for (x, win) in row.windows(WIN).enumerate() {
                let s: f64 = k.iter().zip(win).map(|(kj, v)| kj * v).sum();
                if let Some(v) = hz.get_mut(y * ow + x) {
                    *v = s;
                }
            }
        }
        let mut vt = vec![0.0; ow * oh];
        for y in 0..oh {
            for (j, kj) in k.iter().enumerate() {
                let (Some(src_row), Some(dst)) = (
                    hz.get((y + j) * ow..(y + j + 1) * ow),
                    vt.get_mut(y * ow..(y + 1) * ow),
                ) else {
                    continue;
                };
                for (d, s) in dst.iter_mut().zip(src_row) {
                    *d += kj * s;
                }
            }
        }
        maps.push(vt);
    }
    let c1 = (0.01f64 * 255.0).powi(2);
    let c2 = (0.03f64 * 255.0).powi(2);
    let (mut sum, n) = (0.0, ow * oh);
    for i in 0..n {
        let m = |k: usize| {
            MAP.get(k)
                .and_then(|j| maps.get(*j))
                .and_then(|v| v.get(i))
                .copied()
                .unwrap_or(0.0)
        };
        let (ma, mb) = (m(0), m(1));
        let va = m(2) - ma * ma;
        let vb = m(3) - mb * mb;
        let cov = m(4) - ma * mb;
        sum += ((2.0 * ma * mb + c1) * (2.0 * cov + c2)) / ((ma * ma + mb * mb + c1) * (va + vb + c2));
    }
    Some(sum / n as f64)
}

/// CIE L*a*b* (D65) of an 8-bit sRGB colour.
pub fn lab(c: [u8; 3]) -> [f64; 3] {
    let lin = |v: u8| {
        let v = f64::from(v) / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    let (r, g, b) = (lin(c[0]), lin(c[1]), lin(c[2]));
    let x = (0.4124564 * r + 0.3575761 * g + 0.1804375 * b) / 0.95047;
    let y = 0.2126729 * r + 0.7151522 * g + 0.0721750 * b;
    let z = (0.0193339 * r + 0.1191920 * g + 0.9503041 * b) / 1.08883;
    let f = |t: f64| {
        if t > 216.0 / 24389.0 {
            t.cbrt()
        } else {
            (24389.0 / 27.0 * t + 16.0) / 116.0
        }
    };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// CIEDE2000 colour difference (kL = kC = kH = 1), after Sharma, Wu and Dalal (2005).
pub fn ciede2000(p: [f64; 3], q: [f64; 3]) -> f64 {
    use std::f64::consts::PI;
    let [l1, a1, b1] = p;
    let [l2, a2, b2] = q;
    let c1 = a1.hypot(b1);
    let c2 = a2.hypot(b2);
    let cm = (c1 + c2) / 2.0;
    let g = 0.5 * (1.0 - (cm.powi(7) / (cm.powi(7) + 25f64.powi(7))).sqrt());
    let (a1p, a2p) = ((1.0 + g) * a1, (1.0 + g) * a2);
    let (c1p, c2p) = (a1p.hypot(b1), a2p.hypot(b2));
    let hue = |b: f64, a: f64| {
        if b == 0.0 && a == 0.0 {
            0.0
        } else {
            let h = b.atan2(a).to_degrees();
            if h < 0.0 { h + 360.0 } else { h }
        }
    };
    let (h1p, h2p) = (hue(b1, a1p), hue(b2, a2p));
    let dl = l2 - l1;
    let dc = c2p - c1p;
    let dh = if c1p * c2p == 0.0 {
        0.0
    } else if (h2p - h1p).abs() <= 180.0 {
        h2p - h1p
    } else if h2p - h1p > 180.0 {
        h2p - h1p - 360.0
    } else {
        h2p - h1p + 360.0
    };
    let dhh = 2.0 * (c1p * c2p).sqrt() * (dh.to_radians() / 2.0).sin();
    let lm = (l1 + l2) / 2.0;
    let cmp = (c1p + c2p) / 2.0;
    let hm = if c1p * c2p == 0.0 {
        h1p + h2p
    } else if (h1p - h2p).abs() <= 180.0 {
        (h1p + h2p) / 2.0
    } else if h1p + h2p < 360.0 {
        (h1p + h2p + 360.0) / 2.0
    } else {
        (h1p + h2p - 360.0) / 2.0
    };
    let t = 1.0 - 0.17 * (hm - 30.0).to_radians().cos()
        + 0.24 * (2.0 * hm).to_radians().cos()
        + 0.32 * (3.0 * hm + 6.0).to_radians().cos()
        - 0.20 * (4.0 * hm - 63.0).to_radians().cos();
    let dtheta = 30.0 * (-((hm - 275.0) / 25.0).powi(2)).exp();
    let rc = 2.0 * (cmp.powi(7) / (cmp.powi(7) + 25f64.powi(7))).sqrt();
    let sl = 1.0 + 0.015 * (lm - 50.0).powi(2) / (20.0 + (lm - 50.0).powi(2)).sqrt();
    let sc = 1.0 + 0.045 * cmp;
    let sh = 1.0 + 0.015 * cmp * t;
    let rt = -(2.0 * dtheta * PI / 180.0).sin() * rc;
    let (x, y, z) = (dl / sl, dc / sc, dhh / sh);
    (x * x + y * y + z * z + rt * y * z).max(0.0).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checker(w: usize, h: usize, a: [u8; 3], b: [u8; 3], cell: usize) -> Rgb {
        let mut im = Rgb::new(w, h, a).unwrap();
        for y in 0..h {
            for x in 0..w {
                if (x / cell + y / cell) % 2 == 1 {
                    im.px[y * w + x] = b;
                }
            }
        }
        im
    }

    #[test]
    fn identical_images_score_perfectly() {
        let im = checker(40, 30, [255; 3], [30, 60, 200], 5);
        let s = score(&im, &im);
        assert_eq!(s.sim, 1.0);
        assert_eq!(s.ink, 1.0);
        assert!((s.ssim.unwrap() - 1.0).abs() < 1e-9);
        assert!(s.de.unwrap().abs() < 1e-9);
    }

    #[test]
    fn blank_crops_have_full_ink_score_and_no_colour() {
        let im = Rgb::new(20, 20, [255; 3]).unwrap();
        let s = score(&im, &im);
        assert_eq!(s.ink, 1.0);
        assert_eq!(s.de, None);
        let tiny = Rgb::new(5, 5, [255; 3]).unwrap();
        assert_eq!(score(&tiny, &tiny).ssim, None);
        let empty = Rgb::new(0, 0, [255; 3]).unwrap();
        assert_eq!(score(&empty, &empty).sim, 1.0);
    }

    #[test]
    fn black_versus_white_is_the_worst_grey_difference() {
        let w = Rgb::new(30, 30, [255; 3]).unwrap();
        let b = Rgb::new(30, 30, [0; 3]).unwrap();
        let s = score(&w, &b);
        assert!(s.sim.abs() < 1e-12);
        assert!(s.ssim.unwrap() < 0.01);
    }

    #[test]
    fn disjoint_marks_have_no_ink_overlap_and_colour_shift_is_seen() {
        let mut a = Rgb::new(40, 40, [255; 3]).unwrap();
        let mut b = a.clone();
        for y in 5..10 {
            for x in 5..10 {
                a.px[y * 40 + x] = [0, 0, 0];
                b.px[(y + 25) * 40 + x + 25] = [0, 0, 0];
            }
        }
        assert_eq!(score(&a, &b).ink, 0.0);
        // same shape, red versus blue
        let red = checker(40, 40, [255; 3], [220, 30, 30], 8);
        let blue = checker(40, 40, [255; 3], [30, 30, 220], 8);
        let s = score(&red, &blue);
        assert_eq!(s.ink, 1.0);
        assert!(s.de.unwrap() > 20.0);
    }

    #[test]
    fn ciede2000_reference_pairs() {
        // Sharma et al. (2005) test data, pairs 1, 7 and 17
        let cases = [
            ([50.0, 2.6772, -79.7751], [50.0, 0.0, -82.7485], 2.0425),
            ([50.0, 0.0, 0.0], [50.0, -1.0, 2.0], 2.3669),
            ([50.0, 2.5, 0.0], [73.0, 25.0, -18.0], 27.1492),
        ];
        for (p, q, want) in cases {
            let got = ciede2000(p, q);
            assert!((got - want).abs() < 1e-4, "{got} vs {want}");
        }
        let white = lab([255; 3]);
        assert!((white[0] - 100.0).abs() < 1e-3 && white[1].abs() < 1e-2 && white[2].abs() < 1e-2);
    }

    #[test]
    fn ssim_drops_with_noise() {
        let a = checker(32, 32, [255; 3], [0; 3], 4);
        let mut b = a.clone();
        for (i, p) in b.px.iter_mut().enumerate() {
            if i % 7 == 0 {
                *p = [128; 3];
            }
        }
        let s = ssim(&a.grey(), &b.grey()).unwrap();
        assert!(s < 0.95 && s > 0.3, "{s}");
    }
}
