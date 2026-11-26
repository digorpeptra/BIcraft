//! BIcraft's own visuals in the live viewer's HTML page (`bicraft view`, docs/view-snapshot.md
//! "BIcraft visuals"): the page draws no BIcraft visual itself, so the publisher adds to each one
//! of the snapshot a `drawing`: the renderer's image of the visual (a PNG served at
//! `/api/asset/<id>`, drawn as `report.render` and the app draw it, with its selection dimmed)
//! and its hit map in the visual's own pixels (shapes, the `view.interact` action of a click, the
//! tooltip rows), so a click there sends what a click in the app sends.

use std::collections::HashMap;
use std::sync::Arc;

use bicraft_render::hits::{Hit, HitAction, HitShape};
use bicraft_render::{FontBook, RenderOptions};
use serde_json::{Value, json};

use crate::images::SnapshotAsset;

/// Most BIcraft visuals drawn for one snapshot; the rest keep the viewer's placeholder.
pub const MAX_DRAWINGS: usize = 64;
/// Most hit regions sent for one visual.
const MAX_VISUAL_HITS: usize = 5_000;
/// Device pixels per report pixel of the images (sharp when the page is zoomed in).
const SCALE: f32 = 2.0;
/// The largest visual drawn, in report pixels.
const MAX_SIDE: f64 = 4_000.0;

thread_local! {
    /// The fonts of the drawings: the system's and the bundled ones, indexed once per thread (the
    /// publisher's).
    static FONTS: FontBook = FontBook::with(Vec::new(), true);
}

/// Whether a snapshot visual is one of BIcraft's.
fn is_own(v: &Value) -> bool {
    v.get("type")
        .and_then(Value::as_str)
        .and_then(crate::builtin_visuals::find)
        .is_some()
}

fn shape_json(s: &HitShape) -> Value {
    match s {
        HitShape::Rect { x, y, w, h } => json!({"rect": [x, y, w, h]}),
        HitShape::Circle { cx, cy, r } => json!({"circle": [cx, cy, r]}),
        HitShape::Polygon { rings } => {
            json!({"poly": rings.first().map(|r| r.iter().map(|(x, y)| json!([x, y])).collect::<Vec<_>>()).unwrap_or_default()})
        }
        HitShape::Sector { .. } => {
            let (x, y, w, h) = s.bounds();
            json!({"rect": [x, y, w, h]})
        }
    }
}

fn hit_json(h: &Hit) -> Option<Value> {
    let (action, ctrl) = match &h.action {
        HitAction::Send { action, ctrl } => (action.clone(), *ctrl),
        HitAction::None if !h.tip.is_empty() => (Value::Null, false),
        _ => return None,
    };
    Some(json!({
        "shape": shape_json(&h.shape),
        "action": action,
        "ctrl": ctrl,
        "tip": h.tip.iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>(),
    }))
}

/// The snapshot `base` (the session's, without pages) with page `page` holding visual `v` alone,
/// at the top left of a page its size.
fn alone(base: &Value, page: &Value, v: &Value, w: f64, h: f64) -> Value {
    let mut one = v.clone();
    if let Some(o) = one.as_object_mut() {
        o.insert("x".into(), json!(0));
        o.insert("y".into(), json!(0));
        o.remove("drawing");
    }
    let mut pg = page.clone();
    if let Some(o) = pg.as_object_mut() {
        o.insert("visuals".into(), json!([one]));
        o.insert("width".into(), json!(w));
        o.insert("height".into(), json!(h));
    }
    let mut out = base.clone();
    if let Some(pages) = out.pointer_mut("/report/pages") {
        *pages = json!([pg]);
    }
    out
}

/// Adds a `drawing` to every BIcraft visual of `snap` whose data is in (at most
/// [`MAX_DRAWINGS`]) and returns the images to serve. A visual the renderer cannot draw keeps
/// the viewer's placeholder.
pub fn add_drawings(snap: &mut Value, assets: &HashMap<String, Arc<[u8]>>) -> Vec<SnapshotAsset> {
    let mut jobs: Vec<(usize, usize, Value)> = Vec::new();
    let pages = snap
        .pointer("/report/pages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if !pages.iter().any(|p| {
        p.get("visuals")
            .and_then(Value::as_array)
            .is_some_and(|l| l.iter().any(is_own))
    }) {
        return Vec::new();
    }
    let mut base = snap.clone();
    if let Some(p) = base.pointer_mut("/report/pages") {
        *p = json!([]);
    }
    'pages: for (pi, page) in pages.iter().enumerate() {
        let visuals = page.get("visuals").and_then(Value::as_array);
        for (vi, v) in visuals.into_iter().flatten().enumerate() {
            if jobs.len() >= MAX_DRAWINGS {
                break 'pages;
            }
            if !is_own(v) || v.pointer("/data/status").and_then(Value::as_str) != Some("ok") {
                continue;
            }
            let side = |k: &str| {
                v.get(k)
                    .and_then(Value::as_f64)
                    .filter(|x| x.is_finite() && *x >= 1.0)
            };
            let (Some(w), Some(h)) = (side("width"), side("height")) else {
                continue;
            };
            let (w, h) = (w.min(MAX_SIDE), h.min(MAX_SIDE));
            jobs.push((pi, vi, alone(&base, page, v, w, h)));
        }
    }
    let opts = RenderOptions {
        scale: SCALE,
        system_fonts: true,
        hits: true,
        badges: false,
        ..RenderOptions::default()
    };
    let mut out: Vec<SnapshotAsset> = Vec::new();
    for (pi, vi, one) in jobs {
        let Ok(img) = FONTS.with(|fonts| bicraft_render::render_page(&one, 0, assets, fonts, &opts)) else {
            continue;
        };
        if img.png.is_empty() {
            continue;
        }
        let id = crate::images::asset_id(&img.png);
        let hits: Vec<Value> = img
            .hits
            .iter()
            .filter_map(hit_json)
            .take(MAX_VISUAL_HITS)
            .collect();
        let drawing = json!({
            "asset": id,
            "width": f64::from(img.width) / f64::from(img.scale.max(0.01)),
            "height": f64::from(img.height) / f64::from(img.scale.max(0.01)),
            "hits": hits,
        });
        let Some(v) = snap
            .pointer_mut(&format!("/report/pages/{pi}/visuals/{vi}"))
            .and_then(Value::as_object_mut)
        else {
            continue;
        };
        v.insert("drawing".into(), drawing);
        if !out.iter().any(|a| a.id == id) {
            out.push(SnapshotAsset {
                id,
                mime: "image/png",
                bytes: Arc::from(img.png),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hits_become_json_shapes() {
        let h = Hit {
            visual: "v".into(),
            shape: HitShape::Polygon {
                rings: vec![vec![(0.0, 0.0), (4.0, 0.0), (4.0, 3.0)]],
            },
            action: HitAction::Send {
                action: json!({"action": "select", "point": [{"c": 0, "v": "a"}]}),
                ctrl: true,
            },
            tip: vec![("K".into(), "a".into())],
            item: None,
        };
        let j = hit_json(&h).unwrap();
        assert_eq!(j["shape"]["poly"], json!([[0.0, 0.0], [4.0, 0.0], [4.0, 3.0]]));
        assert_eq!(j["ctrl"], true);
        assert_eq!(j["tip"], json!([["K", "a"]]));
        let none = Hit {
            action: HitAction::None,
            tip: Vec::new(),
            ..h
        };
        assert!(hit_json(&none).is_none());
    }
}
