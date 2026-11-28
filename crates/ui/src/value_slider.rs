//! A what-if parameter's single-value slider (docs/formats/parameters.md) as a widget over the
//! drawn page: the renderer draws the value box and the slider and marks the track with a hit
//! whose popup spec is `{kind: "slider", page, visual, values, value, label, text}`
//! (`bicraft_render::slicers`). Here the track is a labelled slider in the accessibility tree:
//! a drag, a click on the track, or ←/→ (↓/↑, Home, End) with the focus on it picks one of the
//! parameter's values, and the choice goes through `view.interact` `slicer_set` like a list
//! slicer's, so the parameter's `SELECTEDVALUE` measure follows.

use egui::{Rect, Sense, Ui, Vec2};
use serde_json::{Value, json};

use crate::App;
use bicraft_render::hits::HitAction;

/// At most this many sliders over a page, and values per slider.
const MAX_SLIDERS: usize = 50;
const MAX_VALUES: usize = 1000;

struct Slider {
    page: String,
    visual: String,
    /// In report pixels.
    rect: Rect,
    values: Vec<Value>,
    /// The value shown now (index in `values`).
    at: usize,
    label: String,
}

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or(f64::NAN)
}

fn sliders(app: &App) -> Vec<Slider> {
    let Some((_, Ok(img))) = app.shown.as_ref() else {
        return Vec::new();
    };
    img.hits
        .iter()
        .take(20_000)
        .filter_map(|h| {
            let HitAction::Popup(spec) = &h.action else {
                return None;
            };
            if spec.get("kind").and_then(Value::as_str) != Some("slider") {
                return None;
            }
            let values: Vec<Value> = spec
                .get("values")?
                .as_array()?
                .iter()
                .filter(|v| num(v).is_finite())
                .take(MAX_VALUES)
                .cloned()
                .collect();
            if values.is_empty() {
                return None;
            }
            let shown = spec.get("value").map_or(f64::NAN, num);
            let at = nearest(&values, shown).unwrap_or(0);
            let (x, y, w, hh) = h.shape.bounds();
            Some(Slider {
                page: spec.get("page")?.as_str()?.to_string(),
                visual: h.visual.clone(),
                rect: Rect::from_min_size(egui::pos2(x, y), Vec2::new(w, hh)),
                values,
                at,
                label: spec
                    .get("label")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            })
        })
        .take(MAX_SLIDERS)
        .collect()
}

/// The index of the value nearest `x` (`values` sorted ascending).
fn nearest(values: &[Value], x: f64) -> Option<usize> {
    if !x.is_finite() {
        return None;
    }
    values
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| (num(a) - x).abs().total_cmp(&(num(b) - x).abs()))
        .map(|(i, _)| i)
}

/// The value index under screen x `px` on a track spanning `track` (the renderer maps the values
/// linearly from the first to the last).
fn at_pointer(s: &Slider, track: Rect, px: f32) -> Option<usize> {
    let (lo, hi) = (num(s.values.first()?), num(s.values.last()?));
    let f = if track.width() > 0.0 {
        f64::from(((px - track.min.x) / track.width()).clamp(0.0, 1.0))
    } else {
        0.0
    };
    nearest(&s.values, lo + f * (hi - lo))
}

/// The sliders as widgets over the page; returns whether the pointer is on one (the page then
/// takes no click).
pub fn overlay(app: &mut App, ui: &Ui, page_rect: Rect, zoom: f32) -> bool {
    let l = app.lang;
    let to_screen = |r: Rect| Rect::from_min_size(page_rect.min + r.min.to_vec2() * zoom, r.size() * zoom);
    let mut on = false;
    let mut send: Option<(Slider, usize)> = None;
    for s in sliders(app) {
        let screen = to_screen(s.rect);
        if !page_rect.intersects(screen) {
            continue;
        }
        let id = ui.scope_id().with(("bicraft-value-slider", &s.page, &s.visual));
        let resp = ui.interact(screen, id, Sense::click_and_drag());
        let name = if s.label.is_empty() {
            l.t("Value", "Valeur").to_string()
        } else {
            s.label.clone()
        };
        let shown = s.values.get(s.at).map_or(f64::NAN, num);
        resp.widget_info(|| egui::WidgetInfo::slider(true, shown, &name));
        if resp.hovered() || resp.dragged() {
            on = true;
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        let tokens = crate::theme::Tokens::get(ui.ctx());
        if resp.has_focus() {
            ui.painter().rect_stroke(
                screen.expand(1.0),
                2.0,
                egui::Stroke::new(2.0, tokens.focus),
                egui::StrokeKind::Outside,
            );
        }
        // while dragging: a mark where the value would go, the value in a tip
        let pointer = resp
            .interact_pointer_pos()
            .or_else(|| ui.ctx().pointer_latest_pos());
        if resp.dragged()
            && let Some(p) = pointer
            && let Some(i) = at_pointer(&s, screen, p.x)
        {
            ui.data_mut(|d| d.insert_temp(id, i));
            let f = match (s.values.first(), s.values.last(), s.values.get(i)) {
                (Some(a), Some(b), Some(x)) if num(b) > num(a) => {
                    ((num(x) - num(a)) / (num(b) - num(a))) as f32
                }
                _ => 0.0,
            };
            let f = if f.is_finite() { f.clamp(0.0, 1.0) } else { 0.0 };
            let x = screen.min.x + f * screen.width();
            ui.painter().line_segment(
                [egui::pos2(x, screen.min.y), egui::pos2(x, screen.max.y)],
                egui::Stroke::new(2.0, tokens.focus),
            );
            if let Some(v) = s.values.get(i) {
                egui::Tooltip::always_open(
                    ui.ctx().clone(),
                    ui.layer_id(),
                    id.with("tip"),
                    egui::PopupAnchor::Pointer,
                )
                .show(|ui| ui.label(v.to_string()));
            }
        }
        let mut pick = None;
        if resp.drag_stopped() {
            pick = ui.data_mut(|d| d.remove_temp::<usize>(id));
        } else if resp.clicked()
            && let Some(p) = pointer
        {
            pick = at_pointer(&s, screen, p.x);
        } else if resp.has_focus() {
            let last = s.values.len().saturating_sub(1);
            ui.input(|i| {
                if i.key_pressed(egui::Key::ArrowLeft) || i.key_pressed(egui::Key::ArrowDown) {
                    pick = Some(s.at.saturating_sub(1));
                } else if i.key_pressed(egui::Key::ArrowRight) || i.key_pressed(egui::Key::ArrowUp) {
                    pick = Some(s.at.saturating_add(1).min(last));
                } else if i.key_pressed(egui::Key::Home) {
                    pick = Some(0);
                } else if i.key_pressed(egui::Key::End) {
                    pick = Some(last);
                }
            });
        }
        if !resp.dragged() {
            let now = s.values.get(s.at).map(Value::to_string).unwrap_or_default();
            resp.on_hover_text(format!("{name}: {now}"));
        }
        // the value shown is sent too: with nothing selected it is the default, and picking it
        // makes it the selection
        if let Some(i) = pick {
            send = Some((s, i));
        }
    }
    if let Some((s, i)) = send
        && let Some(v) = s.values.get(i)
    {
        app.popup = None;
        app.interact(json!({"action": "slicer_set", "page": s.page, "visual": s.visual, "values": [v]}));
    }
    on
}
