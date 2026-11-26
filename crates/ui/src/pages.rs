//! Report pages rendered by `bicraft-render` off the window's thread, at the size and DPI they
//! are shown at, with their hit maps. Results are cached by snapshot, page and scale (a new
//! selection or slicer state is a new snapshot), and only the latest request is drawn when
//! several queue up (a window being resized).

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

use bicraft_render::states::Pointer;
use bicraft_render::{FontBook, PageImage, RenderOptions};

use crate::source::{Assets, Loaded, Notify};

/// Rendered pages kept.
const CACHE: usize = 8;
/// Largest page drawn, in device pixels (the scale is lowered to fit).
const MAX_PIXELS: u64 = 40_000_000;

/// What a rendered page is: the snapshot, the page, the scale (per mille), the view of it
/// (0: the page itself; else a focus mode's layout, [`crate::focus::Focus::view_key`]) and
/// whether it is drawn in high contrast (View › High-contrast colours, never in the file), and
/// where the reader's pointer is (a hash of the [`Pointer`]: 0 for none; a button under it is
/// drawn in its hover or press state).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key {
    pub seq: u64,
    pub page: usize,
    pub scale_milli: u32,
    pub view: u64,
    pub high_contrast: bool,
    pub pointer: u64,
}

impl Key {
    pub fn new(seq: u64, page: usize, scale: f32) -> Key {
        let s = if scale.is_finite() {
            scale.clamp(0.05, 8.0)
        } else {
            1.0
        };
        Key {
            seq,
            page,
            scale_milli: (s * 1000.0).round() as u32,
            view: 0,
            high_contrast: false,
            pointer: 0,
        }
    }
    /// The same page with the pointer at `p` (`None`: away from every button).
    pub fn with_pointer(self, p: Option<&Pointer>) -> Key {
        let pointer = p.map_or(0, |p| {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            p.hash(&mut h);
            h.finish() | 1
        });
        Key { pointer, ..self }
    }
    /// The same page drawn as view `view` (a focus mode's layout).
    pub fn with_view(self, view: u64) -> Key {
        Key { view, ..self }
    }
    /// The same page drawn with the high-contrast palette, or not.
    pub fn with_contrast(self, high_contrast: bool) -> Key {
        Key {
            high_contrast,
            ..self
        }
    }
    pub fn scale(&self) -> f32 {
        self.scale_milli as f32 / 1000.0
    }
}

/// A rendered page, or why it could not be.
pub type Rendered = (Key, Result<Arc<PageImage>, String>);

struct Job {
    key: Key,
    snap: Arc<serde_json::Value>,
    assets: Arc<Assets>,
    pointer: Option<Pointer>,
}

/// The pages' fonts: the bundled faces only (Selawik), in every shell, so the desktop draws
/// the pixels the browser draws (decision of 2026-11-04, docs/design.md §3). `bicraft render`
/// keeps the system's fonts, for comparisons with Power BI Desktop.
pub fn font_book() -> FontBook {
    FontBook::bundled_only()
}

/// Device pixels per report pixel under which the window's pages draw subpixel (LCD) text:
/// below it a report's small text is a few device pixels tall and the subpixels triple its
/// horizontal resolution, as Power BI Desktop's ClearType text does; above it (high-DPI
/// screens) grayscale is sharp enough and colour fringes would only show.
pub const LCD_TEXT_BELOW: f32 = 1.75;

/// Whether a page rendered at `scale` device pixels per report pixel draws subpixel text.
pub fn lcd_text(scale: f32) -> bool {
    scale.is_finite() && scale < LCD_TEXT_BELOW
}

/// Renders one page (on the calling thread), with subpixel text below [`LCD_TEXT_BELOW`]: the
/// page is shown at its own size on whole device pixels (`app.rs`, `texel_rect`).
pub fn render(
    snap: &serde_json::Value,
    assets: &Assets,
    fonts: &FontBook,
    key: Key,
) -> Result<PageImage, String> {
    render_at(snap, assets, fonts, key, None)
}

/// [`render`] with the reader's pointer at `pointer` (`key` made with [`Key::with_pointer`]).
pub fn render_at(
    snap: &serde_json::Value,
    assets: &Assets,
    fonts: &FontBook,
    key: Key,
    pointer: Option<&Pointer>,
) -> Result<PageImage, String> {
    let opts = RenderOptions {
        pointer: pointer.cloned(),
        scale: key.scale(),
        hits: true,
        raw: true,
        max_pixels: MAX_PIXELS,
        lcd_text: lcd_text(key.scale()),
        high_contrast: key.high_contrast,
        ..RenderOptions::default()
    };
    let img = bicraft_render::render_page(snap, key.page, assets, fonts, &opts).map_err(|e| e.to_string())?;
    let expected = (img.width as usize)
        .checked_mul(img.height as usize)
        .and_then(|n| n.checked_mul(4));
    if expected != Some(img.rgba.len()) {
        return Err("the renderer returned a malformed image".into());
    }
    Ok(img)
}

/// The render thread and its cache.
pub struct Pages {
    tx: Option<Sender<Job>>,
    rx: Option<Receiver<Rendered>>,
    cache: Vec<Rendered>,
    in_flight: Option<Key>,
    /// Renders on the window's thread (tests: deterministic frames).
    sync: Option<FontBook>,
}

impl Pages {
    /// A render thread that wakes the window with `notify` when a page is ready.
    pub fn threaded(notify: Notify) -> Pages {
        let (tx, jobs) = channel::<Job>();
        let (done, rx) = channel::<Rendered>();
        let spawned = std::thread::Builder::new()
            .name("bicraft-render".into())
            .spawn(move || {
                let fonts = font_book();
                while let Ok(mut job) = jobs.recv() {
                    // only the latest request matters
                    loop {
                        match jobs.try_recv() {
                            Ok(j) => job = j,
                            Err(TryRecvError::Empty) => break,
                            Err(TryRecvError::Disconnected) => return,
                        }
                    }
                    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        render_at(&job.snap, &job.assets, &fonts, job.key, job.pointer.as_ref())
                    }))
                    .unwrap_or_else(|_| Err("internal error while rendering".into()));
                    if done.send((job.key, r.map(Arc::new))).is_err() {
                        return;
                    }
                    notify();
                }
            })
            .is_ok();
        Pages {
            tx: spawned.then_some(tx),
            rx: spawned.then_some(rx),
            cache: Vec::new(),
            in_flight: None,
            sync: (!spawned).then(font_book),
        }
    }

    /// Renders on the caller's thread with `fonts`.
    pub fn synchronous(fonts: FontBook) -> Pages {
        Pages {
            tx: None,
            rx: None,
            cache: Vec::new(),
            in_flight: None,
            sync: Some(fonts),
        }
    }

    fn receive(&mut self) {
        let got: Vec<Rendered> = match &self.rx {
            Some(rx) => rx.try_iter().collect(),
            None => return,
        };
        for r in got {
            if self.in_flight == Some(r.0) {
                self.in_flight = None;
            }
            self.store(r);
        }
    }

    fn store(&mut self, r: Rendered) {
        self.cache.retain(|c| c.0 != r.0);
        self.cache.push(r);
        if self.cache.len() > CACHE {
            let _ = self.cache.remove(0);
        }
    }

    /// The page for `key`, rendering it when missing; while it renders, the closest page already
    /// drawn of the same snapshot (or an older one of the same page) stands in, scaled.
    pub fn get(&mut self, loaded: &Loaded, key: Key) -> Option<Rendered> {
        self.get_at(loaded, key, None)
    }

    /// [`Pages::get`] with the reader's pointer at `pointer` (`key` made with
    /// [`Key::with_pointer`]).
    pub fn get_at(&mut self, loaded: &Loaded, key: Key, pointer: Option<&Pointer>) -> Option<Rendered> {
        self.receive();
        if let Some(r) = self.cache.iter().find(|c| c.0 == key) {
            return Some(r.clone());
        }
        if let Some(fonts) = &self.sync {
            let r = render_at(&loaded.snap, &loaded.assets, fonts, key, pointer).map(Arc::new);
            self.store((key, r));
            return self.cache.last().cloned();
        }
        if self.in_flight != Some(key)
            && let Some(tx) = &self.tx
        {
            let job = Job {
                key,
                snap: Arc::clone(&loaded.snap),
                assets: Arc::clone(&loaded.assets),
                pointer: pointer.cloned(),
            };
            if tx.send(job).is_ok() {
                self.in_flight = Some(key);
            }
        }
        self.cache
            .iter()
            .rev()
            .find(|c| c.0.page == key.page && c.0.view == key.view && c.0.seq == key.seq && c.1.is_ok())
            .or_else(|| {
                self.cache
                    .iter()
                    .rev()
                    .find(|c| c.0.page == key.page && c.0.view == key.view && c.1.is_ok())
            })
            .cloned()
    }

    /// Drops the drawings of page `page` (every snapshot and scale), so it is drawn again: the
    /// Performance analyzer's Refresh visuals measures that drawing.
    pub fn forget(&mut self, page: usize) {
        self.receive();
        self.cache.retain(|c| c.0.page != page);
        if self.in_flight.is_some_and(|k| k.page == page) {
            self.in_flight = None;
        }
    }

    /// Whether a page is being rendered.
    pub fn busy(&self) -> bool {
        self.in_flight.is_some()
    }

    /// Pages rendered and kept (tests).
    pub fn cached(&self) -> usize {
        self.cache.len()
    }
}
