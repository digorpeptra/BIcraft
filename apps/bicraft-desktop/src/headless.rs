//! `bicraft FILE --render OUT.png`: the window drawn with no window (docs/desktop.md,
//! "Headless render"). The app runs offscreen ([`bicraft_ui::offscreen::Offscreen`]) and is
//! driven by the control channel's own requests, as an agent would drive the window: `ui.wait`
//! (the file open, its data computed, the page drawn), `ui.set` when asked (a page, the theme,
//! a view…), `ui.wait` again, then `ui.screenshot`, whose frame is drawn in software. No GPU,
//! no display, no settings file, no recovery journal.

use std::time::{Duration, Instant};

use bicraft_ui::App;
use bicraft_ui::control::{self, Job, Request};
use bicraft_ui::offscreen::{FRAME_SECS, Offscreen};
use serde_json::{Value, json};

/// What to draw: the window's size in points, its scale (pixels per point), the `ui.set`
/// params applied first, and how long the whole render may take.
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    pub size: [f32; 2],
    pub scale: f32,
    pub set: Option<Value>,
    pub timeout: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            size: [1366.0, 860.0],
            scale: 1.0,
            set: None,
            timeout: Duration::from_secs(120),
        }
    }
}

/// Reads `--size WxH` (points).
pub fn size(s: &str) -> Result<[f32; 2], String> {
    let bad = || format!("--size needs WIDTHxHEIGHT in points (1366x860), not `{s}`");
    let (w, h) = s.trim().split_once(['x', 'X']).ok_or_else(bad)?;
    let w: f32 = w.trim().parse().map_err(|_| bad())?;
    let h: f32 = h.trim().parse().map_err(|_| bad())?;
    Ok([w, h])
}

/// Runs `app` offscreen until its window is idle and returns the frame as PNG with its size in
/// pixels.
pub fn render(app: &mut App, opts: &Options) -> Result<(Vec<u8>, [usize; 2]), String> {
    let mut win = Offscreen::new(egui::vec2(opts.size[0], opts.size[1]), opts.scale)?;
    let (tx, rx) = std::sync::mpsc::sync_channel(control::MAX_QUEUED);
    app.control.attach(rx);
    let deadline = Instant::now() + opts.timeout;
    let mut steps = vec![("ui.wait", json!({}))];
    if let Some(set) = &opts.set {
        steps.push(("ui.set", set.clone()));
        steps.push(("ui.wait", json!({})));
    }
    steps.push(("ui.screenshot", json!({})));
    let mut shot = Value::Null;
    for (n, (method, params)) in steps.into_iter().enumerate() {
        let (job, ticket) = Job::new(Request {
            id: json!(n),
            method: method.into(),
            params,
        });
        tx.try_send(job).map_err(|_| "the window's queue is full")?;
        let reply = loop {
            win.frame(app);
            if let Some(r) = ticket.try_take() {
                break r;
            }
            if Instant::now() > deadline {
                return Err(format!("{method}: no answer within {} s", opts.timeout.as_secs()));
            }
            // the window's time advances by FRAME_SECS per frame: keep it near the clock's, so
            // the channel's waits last what they say
            std::thread::sleep(Duration::from_secs_f64(FRAME_SECS));
        };
        if reply.get("ok") != Some(&Value::Bool(true)) {
            let e = reply.get("error").and_then(Value::as_str).unwrap_or("refused");
            return Err(format!("{method}: {e}"));
        }
        shot = reply.get("result").cloned().unwrap_or(Value::Null);
    }
    if let Some(e) = &app.error {
        return Err(e.clone());
    }
    let png = shot
        .get("png")
        .and_then(Value::as_str)
        .and_then(control::unbase64)
        .ok_or("the screenshot has no PNG")?;
    let dim = |k: &str| {
        shot.get(k)
            .and_then(Value::as_u64)
            .and_then(|v| usize::try_from(v).ok())
            .unwrap_or(0)
    };
    Ok((png, [dim("width"), dim("height")]))
}
