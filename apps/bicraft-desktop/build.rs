//! Draws the app icon from `assets/logo/bicraft-icon.svg` (the viewer's logo) into `OUT_DIR`:
//! `icon-256.png` for the window and the taskbar, and on Windows `bicraft-desktop.ico` (16 to
//! 256 px, PNG entries) embedded in the executable with its version information, so the icon
//! shows in Explorer, the Start menu and Alt-Tab. One source, no committed binaries.
//!
//! A missing resource compiler on Windows is a warning (the app still builds, without an
//! embedded icon), unless `BICRAFT_REQUIRE_WINRES=1` turns it into an error (release builds).

use std::path::{Path, PathBuf};

const SVG: &str = "../../assets/logo/bicraft-icon.svg";
const SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={SVG}");
    println!("cargo:rerun-if-env-changed=BICRAFT_REQUIRE_WINRES");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_default());
    let svg = std::fs::read(SVG).unwrap_or_default();
    let mut pngs = Vec::new();
    for size in SIZES {
        match draw(&svg, size) {
            Some(png) => pngs.push((size, png)),
            None => println!("cargo:warning=cannot draw the app icon at {size} px"),
        }
    }
    let big = pngs.last().map(|(_, p)| p.clone()).unwrap_or_default();
    if let Err(e) = std::fs::write(out.join("icon-256.png"), &big) {
        println!("cargo:warning=cannot write the icon: {e}");
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let ico = out.join("bicraft-desktop.ico");
        if let Err(e) = std::fs::write(&ico, ico_bytes(&pngs)) {
            println!("cargo:warning=cannot write the icon: {e}");
            return;
        }
        resources(&ico);
    }
}

/// The SVG drawn at `size` × `size`, as PNG.
fn draw(svg: &[u8], size: u32) -> Option<Vec<u8>> {
    let tree = resvg::usvg::Tree::from_data(svg, &resvg::usvg::Options::default()).ok()?;
    let mut pm = tiny_skia::Pixmap::new(size, size)?;
    let s = tree.size();
    let t = tiny_skia::Transform::from_scale(size as f32 / s.width(), size as f32 / s.height());
    resvg::render(&tree, t, &mut pm.as_mut());
    pm.encode_png().ok()
}

/// An ICO file whose entries are PNG images (Windows Vista and later).
fn ico_bytes(pngs: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut b = Vec::new();
    let n = pngs.len() as u16;
    b.extend_from_slice(&0u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&n.to_le_bytes());
    let mut offset = 6 + 16 * pngs.len() as u32;
    for (size, png) in pngs {
        let dim = if *size >= 256 { 0 } else { *size as u8 };
        b.extend_from_slice(&[dim, dim, 0, 0]);
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&32u16.to_le_bytes());
        b.extend_from_slice(&(png.len() as u32).to_le_bytes());
        b.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for (_, png) in pngs {
        b.extend_from_slice(png);
    }
    b
}

fn resources(ico: &Path) {
    let mut res = winresource::WindowsResource::new();
    res.set_icon(&ico.to_string_lossy())
        .set("ProductName", "BIcraft")
        .set("FileDescription", "BIcraft desktop")
        .set(
            "LegalCopyright",
            "Copyright (c) the BIcraft authors. MIT OR Apache-2.0.",
        )
        .set("OriginalFilename", "bicraft-desktop.exe")
        .set("InternalName", "bicraft-desktop");
    if let Err(e) = res.compile() {
        if bicraft_env::var_os("BICRAFT_REQUIRE_WINRES").is_some() {
            eprintln!("embedding Windows resources failed: {e}");
            std::process::exit(1);
        }
        println!("cargo:warning=bicraft-desktop.exe built without icon/version resources: {e}");
    }
}
