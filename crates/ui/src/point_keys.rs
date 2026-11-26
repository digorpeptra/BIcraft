//! Data points from the keyboard (docs/desktop.md "Keyboard"): with a visual selected on the
//! canvas, Ctrl+Right enters its data points; the arrows move between them (in the order the hit
//! map records them), Enter or Space selects the focused one, Ctrl+Space adds it to the
//! selection, Escape clears the visual's selection and leaves. The focused point has a ring in the
//! theme's `focus` token and shows its tooltip. A selection is the hit's own `view.interact`
//! action, the one a click sends; the focus is view state (egui memory).

use egui::{Context, Id, Key, Modifiers, Rect, Stroke, StrokeKind, Ui, pos2, vec2};
use serde_json::Value;

use crate::App;
use crate::theme::Tokens;

/// Most data points the keyboard walks through in one visual.
const MAX_TARGETS: usize = 5_000;

fn id() -> Id {
    Id::unique("bicraft-point-focus")
}

/// The focused data point: its visual and its index among the visual's points.
pub fn focus(ctx: &Context) -> Option<(String, usize)> {
    ctx.data(|d| d.get_temp::<(String, usize)>(id()))
}

fn set_focus(ctx: &Context, f: Option<(String, usize)>) {
    ctx.data_mut(|d| match f {
        Some(f) => {
            d.insert_temp(id(), f);
        }
        None => {
            d.remove::<(String, usize)>(id());
        }
    });
}

/// One data point of a visual: the click's action, the bounds of its first region (report
/// pixels) and its tooltip.
#[derive(Debug, Clone)]
pub struct Target {
    pub action: Value,
    pub ctrl: bool,
    pub bounds: (f32, f32, f32, f32),
    pub tip: Vec<(String, String)>,
}

/// The distinct `select` actions of `visual` on the page shown (a point, not the background).
pub fn targets(app: &App, visual: &str) -> Vec<Target> {
    let Some((_, Ok(img))) = app.shown.as_ref() else {
        return Vec::new();
    };
    let mut out: Vec<Target> = Vec::new();
    for h in img.hits.iter().filter(|h| h.visual == visual) {
        let bicraft_render::hits::HitAction::Send { action, ctrl } = &h.action else {
            continue;
        };
        if action.get("action").and_then(Value::as_str) != Some("select")
            || action.get("point").is_none_or(Value::is_null)
            || out.iter().any(|t| &t.action == action)
        {
            continue;
        }
        out.push(Target {
            action: action.clone(),
            ctrl: *ctrl,
            bounds: h.shape.bounds(),
            tip: h.tip.clone(),
        });
        if out.len() >= MAX_TARGETS {
            break;
        }
    }
    out
}

/// The action that selects `t`, with Ctrl or not.
fn action_of(t: &Target, ctrl: bool) -> Value {
    let mut a = t.action.clone();
    if t.ctrl
        && let Some(o) = a.as_object_mut()
    {
        o.insert("ctrl".into(), Value::Bool(ctrl));
    }
    a
}

/// The canvas keys of the data points; true when a key was taken (the arrows then do not move
/// the visual).
pub fn keys(app: &mut App, ui: &Ui) -> bool {
    let ctx = ui.ctx().clone();
    let selected = app.cmd.visual.as_ref().map(|v| v.visual.clone());
    let Some(visual) = selected else {
        set_focus(&ctx, None);
        return false;
    };
    let now = focus(&ctx).filter(|(v, _)| *v == visual);
    let Some((_, at)) = now else {
        set_focus(&ctx, None);
        if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::ArrowRight)) {
            if targets(app, &visual).is_empty() {
                app.status = app
                    .lang
                    .t(
                        "This visual has no data point to select",
                        "Ce visuel n'a aucun point de données à sélectionner",
                    )
                    .to_string();
            } else {
                set_focus(&ctx, Some((visual, 0)));
                ctx.request_repaint();
            }
            return true;
        }
        return false;
    };
    let list = targets(app, &visual);
    let n = list.len();
    if n == 0 {
        set_focus(&ctx, None);
        return false;
    }
    let at = at.min(n - 1);
    let mut taken = false;
    let mut to = at;
    for (key, step) in [
        (Key::ArrowLeft, -1i64),
        (Key::ArrowUp, -1),
        (Key::ArrowRight, 1),
        (Key::ArrowDown, 1),
    ] {
        if ui.input_mut(|i| i.consume_key(Modifiers::NONE, key)) {
            let next = (to as i64 + step).rem_euclid(n as i64);
            to = usize::try_from(next).unwrap_or(0);
            taken = true;
        }
    }
    let add = ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Space));
    let pick = !add
        && ui.input_mut(|i| {
            i.consume_key(Modifiers::NONE, Key::Enter) || i.consume_key(Modifiers::NONE, Key::Space)
        });
    let leave = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
    if let Some(t) = list.get(to)
        && (add || pick)
    {
        app.interact(action_of(t, add));
        taken = true;
    }
    if leave {
        if let Some(t) = list.first() {
            let mut a = t.action.clone();
            if let Some(o) = a.as_object_mut() {
                o.insert("point".into(), Value::Null);
                o.remove("points");
                o.remove("ctrl");
            }
            app.interact(a);
        }
        set_focus(&ctx, None);
        ctx.request_repaint();
        return true;
    }
    if to != at || taken {
        set_focus(&ctx, Some((visual, to)));
        ctx.request_repaint();
    }
    taken
}

/// The ring around the focused data point and its tooltip, over the page at `rect` (`zoom`
/// screen points per report pixel).
pub fn paint(app: &App, ui: &Ui, rect: Rect, zoom: f32) {
    let Some((visual, at)) = focus(ui.ctx()) else {
        return;
    };
    let list = targets(app, &visual);
    let Some(t) = list.get(at.min(list.len().saturating_sub(1))) else {
        return;
    };
    let (x, y, w, h) = t.bounds;
    let r = Rect::from_min_size(
        pos2(rect.min.x + x * zoom, rect.min.y + y * zoom),
        vec2((w * zoom).max(2.0), (h * zoom).max(2.0)),
    )
    .expand(2.0);
    let tok = Tokens::get(ui.ctx());
    ui.painter()
        .rect_stroke(r, 2.0, Stroke::new(2.0, tok.focus), StrokeKind::Outside);
    if t.tip.is_empty() {
        return;
    }
    egui::Area::new(Id::unique("bicraft-point-tip"))
        .fixed_pos(r.right_bottom() + vec2(4.0, 4.0))
        .order(egui::Order::Tooltip)
        .interactable(false)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                egui::Grid::new("bicraft-point-tip-grid")
                    .num_columns(2)
                    .show(ui, |ui| {
                        for (k, v) in t.tip.iter().take(12) {
                            ui.label(egui::RichText::new(k).weak());
                            ui.label(v);
                            ui.end_row();
                        }
                    });
            });
        });
}
