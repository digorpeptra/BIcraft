//! Report images: PNG, JPEG, GIF (first frame), WebP and SVG, decoded in pure Rust with size caps.
//! A decoding failure (or a panic inside a decoder) just leaves the image out.

use std::io::Cursor;
use std::panic::{AssertUnwindSafe, catch_unwind};

use tiny_skia::{ColorU8, Pixmap};

/// Largest decoded image (pixels).
pub const MAX_IMAGE_PIXELS: u64 = 16_000_000;
/// Largest side of a decoded image.
const MAX_SIDE: u32 = 16_384;

/// A decoded image.
pub enum Image {
    Raster(Pixmap),
    Svg(Box<resvg::usvg::Tree>),
}

impl Image {
    /// Natural size in CSS pixels.
    pub fn size(&self) -> (f32, f32) {
        match self {
            Image::Raster(p) => (p.width() as f32, p.height() as f32),
            Image::Svg(t) => (t.size().width(), t.size().height()),
        }
    }

    /// A pixmap of `w`×`h` device pixels (SVG rendered at that size; rasters as they are).
    pub fn pixmap(&self, w: u32, h: u32) -> Option<Pixmap> {
        match self {
            Image::Raster(p) => Some(p.clone()),
            Image::Svg(t) => {
                let (w, h) = (w.clamp(1, MAX_SIDE), h.clamp(1, MAX_SIDE));
                if u64::from(w) * u64::from(h) > MAX_IMAGE_PIXELS {
                    return None;
                }
                let mut pm = Pixmap::new(w, h)?;
                let s = t.size();
                let tf = tiny_skia::Transform::from_scale(
                    w as f32 / s.width().max(1e-3),
                    h as f32 / s.height().max(1e-3),
                );
                resvg::render(t, tf, &mut pm.as_mut());
                Some(pm)
            }
        }
    }
}

/// Decodes `bytes` by sniffing their type.
pub fn decode(bytes: &[u8]) -> Option<Image> {
    catch_unwind(AssertUnwindSafe(|| decode_inner(bytes)))
        .ok()
        .flatten()
}

fn decode_inner(b: &[u8]) -> Option<Image> {
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        let w = u32::from_be_bytes(b.get(16..20)?.try_into().ok()?);
        let h = u32::from_be_bytes(b.get(20..24)?.try_into().ok()?);
        check(w, h)?;
        return Pixmap::decode_png(b).ok().map(Image::Raster);
    }
    if b.starts_with(&[0xff, 0xd8, 0xff]) {
        return jpeg(b).map(Image::Raster);
    }
    if b.starts_with(b"GIF8") {
        return gif_first(b).map(Image::Raster);
    }
    if b.len() > 12 && b.get(0..4) == Some(b"RIFF") && b.get(8..12) == Some(b"WEBP") {
        return webp(b).map(Image::Raster);
    }
    let head = String::from_utf8_lossy(b.get(..b.len().min(4096))?).to_lowercase();
    if head.contains("<svg") {
        let tree = resvg::usvg::Tree::from_data(b, &svg_options()).ok()?;
        return Some(Image::Svg(Box::new(tree)));
    }
    None
}

/// SVG parsing options that read nothing outside the bytes: an `<image>` element's reference
/// (a file path, a `data:` URL, a web link) resolves to nothing, so a hostile SVG cannot make the
/// renderer read a local file (`file:///…`, `C:…`, `/etc/…`) and draw it into a page that a
/// server then sends back. No fonts are loaded either (the `text` feature is off: SVG text is
/// not drawn).
fn svg_options() -> resvg::usvg::Options<'static> {
    resvg::usvg::Options {
        resources_dir: None,
        image_href_resolver: resvg::usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..Default::default()
    }
}

fn check(w: u32, h: u32) -> Option<()> {
    (w > 0 && h > 0 && w <= MAX_SIDE && h <= MAX_SIDE && u64::from(w) * u64::from(h) <= MAX_IMAGE_PIXELS)
        .then_some(())
}

/// Straight RGBA → premultiplied pixmap.
fn from_rgba(w: u32, h: u32, rgba: &[u8]) -> Option<Pixmap> {
    check(w, h)?;
    let mut pm = Pixmap::new(w, h)?;
    let need = (w as usize).checked_mul(h as usize)?.checked_mul(4)?;
    let src = rgba.get(..need)?;
    for (dst, px) in pm.pixels_mut().iter_mut().zip(src.as_chunks::<4>().0.iter()) {
        let [r, g, b, a] = *px;
        *dst = ColorU8::from_rgba(r, g, b, a).premultiply();
    }
    Some(pm)
}

fn jpeg(b: &[u8]) -> Option<Pixmap> {
    use zune_jpeg::zune_core::colorspace::ColorSpace;
    use zune_jpeg::zune_core::options::DecoderOptions;
    let opts = DecoderOptions::default()
        .jpeg_set_out_colorspace(ColorSpace::RGBA)
        .set_max_width(MAX_SIDE as usize)
        .set_max_height(MAX_SIDE as usize);
    let mut d = zune_jpeg::JpegDecoder::new_with_options(Cursor::new(b), opts);
    d.decode_headers().ok()?;
    let info = d.info()?;
    let (w, h) = (u32::from(info.width), u32::from(info.height));
    check(w, h)?;
    let size = d.output_buffer_size()?;
    let mut out = vec![0u8; size];
    d.decode_into(&mut out).ok()?;
    match d.output_colorspace()? {
        ColorSpace::RGBA => from_rgba(w, h, &out),
        ColorSpace::RGB => {
            let rgba: Vec<u8> = out
                .as_chunks::<3>()
                .0
                .iter()
                .flat_map(|&[r, g, b]| [r, g, b, 255])
                .collect();
            from_rgba(w, h, &rgba)
        }
        ColorSpace::Luma => {
            let rgba: Vec<u8> = out.iter().flat_map(|l| [*l, *l, *l, 255]).collect();
            from_rgba(w, h, &rgba)
        }
        _ => None,
    }
}

fn gif_first(b: &[u8]) -> Option<Pixmap> {
    let mut opts = gif::DecodeOptions::new();
    opts.set_color_output(gif::ColorOutput::RGBA);
    let mut d = opts.read_info(Cursor::new(b)).ok()?;
    let (sw, sh) = (u32::from(d.width()), u32::from(d.height()));
    check(sw, sh)?;
    let frame = d.read_next_frame().ok()??;
    let (fw, fh) = (u32::from(frame.width), u32::from(frame.height));
    let (fl, ft) = (u32::from(frame.left), u32::from(frame.top));
    let mut canvas = vec![0u8; (sw as usize) * (sh as usize) * 4];
    for y in 0..fh {
        for x in 0..fw {
            let (cx, cy) = (x + fl, y + ft);
            if cx >= sw || cy >= sh {
                continue;
            }
            let si = ((y * fw + x) * 4) as usize;
            let di = ((cy * sw + cx) * 4) as usize;
            if let (Some(src), Some(dst)) = (frame.buffer.get(si..si + 4), canvas.get_mut(di..di + 4)) {
                dst.copy_from_slice(src);
            }
        }
    }
    from_rgba(sw, sh, &canvas)
}

fn webp(b: &[u8]) -> Option<Pixmap> {
    let mut d = image_webp::WebPDecoder::new(Cursor::new(b)).ok()?;
    let (w, h) = d.dimensions();
    check(w, h)?;
    let size = d.output_buffer_size()?;
    let mut out = vec![0u8; size];
    d.read_image(&mut out).ok()?;
    if d.has_alpha() {
        from_rgba(w, h, &out)
    } else {
        let rgba: Vec<u8> = out
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|&[r, g, b]| [r, g, b, 255])
            .collect();
        from_rgba(w, h, &rgba)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_and_svg_decode_garbage_does_not() {
        let mut pm = Pixmap::new(3, 2).unwrap();
        pm.fill(tiny_skia::Color::from_rgba8(255, 0, 0, 255));
        let png = pm.encode_png().unwrap();
        let img = decode(&png).unwrap();
        assert_eq!(img.size(), (3.0, 2.0));
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="20"><rect width="10" height="20" fill="blue"/></svg>"#;
        let img = decode(svg).unwrap();
        assert_eq!(img.size(), (10.0, 20.0));
        assert!(img.pixmap(5, 10).is_some());
        assert!(decode(b"\x89PNG\r\n\x1a\n garbage").is_none());
        assert!(decode(&[0xff, 0xd8, 0xff, 0, 1, 2]).is_none());
        assert!(decode(b"GIF89a").is_none());
        assert!(decode(b"RIFF\0\0\0\0WEBPxxxx").is_none());
        assert!(decode(b"").is_none());
    }

    /// An SVG's `<image>` naming a local file (here another SVG, which usvg would load and draw
    /// without the raster feature) or a `data:` URL is not followed: nothing of it is drawn.
    #[test]
    fn svg_references_outside_the_bytes_are_not_read() {
        let dir = std::env::temp_dir().join(format!("bicraft-svg-href-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let inner = dir.join("secret.svg");
        std::fs::write(
            &inner,
            br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10" fill="red"/></svg>"#,
        )
        .unwrap();
        let href = inner.to_string_lossy().replace('\\', "/");
        let data = "data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSIxMCIgaGVpZ2h0PSIxMCI+PHJlY3Qgd2lkdGg9IjEwIiBoZWlnaHQ9IjEwIiBmaWxsPSJyZWQiLz48L3N2Zz4=";
        for target in [href.clone(), format!("file:///{href}"), data.to_string()] {
            let svg = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="10" height="10"><image width="10" height="10" xlink:href="{target}"/></svg>"#
            );
            let img = decode(svg.as_bytes()).unwrap();
            let pm = img.pixmap(10, 10).unwrap();
            let red = pm.pixels().iter().filter(|p| p.red() > 0).count();
            assert_eq!(red, 0, "{target}: the referenced image was drawn");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn huge_png_header_is_refused() {
        let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        b.extend_from_slice(&100_000u32.to_be_bytes());
        b.extend_from_slice(&100_000u32.to_be_bytes());
        b.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        assert!(decode(&b).is_none());
    }
}
