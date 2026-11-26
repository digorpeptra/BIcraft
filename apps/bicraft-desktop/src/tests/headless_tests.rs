//! `--render`: the window drawn with no window, no GPU and no display (docs/desktop.md,
//! "Headless render"; docs/gaps.md G39). Runs everywhere (`cargo test`, no `gpu-tests`).
//! `BICRAFT_DESKTOP_SHOTS=DIR` keeps the PNGs (`headless-*.png`).

use std::path::PathBuf;
use std::time::Duration;

use serde_json::json;

use super::{Temp, shop};
use crate::headless::{self, Options};

fn render(path: &std::path::Path, set: Option<serde_json::Value>) -> (Vec<u8>, tiny_skia::Pixmap) {
    let mut app = crate::app_with("en-US", None, None).with_sync_render();
    app.open_path(path);
    let opts = Options {
        size: [1280.0, 800.0],
        set,
        timeout: Duration::from_secs(120),
        ..Options::default()
    };
    let (png, size) = headless::render(&mut app, &opts).unwrap();
    let pm = tiny_skia::Pixmap::decode_png(&png).unwrap();
    assert_eq!([pm.width() as usize, pm.height() as usize], size);
    (png, pm)
}

/// Pixels of `pm` inside the rectangle (pixels) that are not `bg`.
fn painted(pm: &tiny_skia::Pixmap, x0: u32, y0: u32, x1: u32, y1: u32) -> usize {
    let bg = pm.pixel(x0, y0).unwrap();
    (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .filter(|(x, y)| pm.pixel(*x, *y).unwrap() != bg)
        .count()
}

#[test]
fn the_window_is_drawn_with_no_window() {
    let dir = Temp::new("headless");
    let (path, _) = shop(dir.path());
    let (png, pm) = render(&path, None);
    // the window's size, opaque, and far from blank: the chrome's text and the page's visuals
    assert_eq!((pm.width(), pm.height()), (1280, 800));
    assert!(pm.pixels().iter().all(|p| p.alpha() == 255));
    let colours: std::collections::HashSet<[u8; 4]> = pm
        .pixels()
        .iter()
        .map(|p| [p.red(), p.green(), p.blue(), p.alpha()])
        .collect();
    assert!(colours.len() > 100, "{} colours", colours.len());
    // the ribbon's labels (the top rows) and the page (the middle) are drawn
    assert!(painted(&pm, 0, 0, 1280, 120) > 2000);
    assert!(painted(&pm, 200, 200, 1000, 600) > 2000);
    // the canvas left of the panes shows the page's image (a texture): its card, chart and
    // slicer text in many colours, not the bare canvas
    let canvas: std::collections::HashSet<_> = (180..740)
        .flat_map(|y| (0..500).map(move |x| (x, y)))
        .filter_map(|(x, y)| pm.pixel(x, y))
        .map(|p| [p.red(), p.green(), p.blue()])
        .collect();
    assert!(canvas.len() > 20, "{} colours on the canvas", canvas.len());
    // the same file gives the same bytes
    let (again, _) = render(&path, None);
    assert!(png == again, "two renders of the same window differ");
    // another page and the dark theme, through ui.set: another picture, darker
    let (dark, dpm) = render(&path, Some(json!({"page": "Second", "theme": "dark"})));
    assert!(dark != png);
    let lum = |pm: &tiny_skia::Pixmap| -> u64 {
        pm.pixels()
            .iter()
            .map(|p| u64::from(p.red()) + u64::from(p.green()) + u64::from(p.blue()))
            .sum()
    };
    assert!(lum(&dpm) < lum(&pm));
    if let Some(d) = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from) {
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("headless-light.png"), &png).unwrap();
        std::fs::write(d.join("headless-dark.png"), &dark).unwrap();
    }
}

#[test]
fn a_file_that_does_not_open_is_an_error() {
    let dir = Temp::new("headless-bad");
    let bad = dir.path().join("broken.pbix");
    std::fs::write(&bad, b"not a zip").unwrap();
    let mut app = crate::app_with("en-US", None, None).with_sync_render();
    app.open_path(&bad);
    let r = headless::render(&mut app, &Options::default());
    assert!(r.is_err(), "{:?}", r.map(|(p, s)| (p.len(), s)));
    // a hostile size is refused before anything runs
    let mut app = crate::app_with("en-US", None, None);
    let opts = Options {
        size: [1e9, 10.0],
        ..Options::default()
    };
    assert!(headless::render(&mut app, &opts).is_err());
}
