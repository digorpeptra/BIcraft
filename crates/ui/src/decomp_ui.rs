//! A decomposition tree's controls as widgets over the drawn page (G34): the "+" of the last
//! level, the header's remove cross and the scroll arrows, each labelled for the accessibility
//! tree, reachable with Tab and clicked with Enter or Space, and sending the same action as a
//! click on the drawn control (the hit map of `bicraft_render::decomp`). The "+" opens the
//! "Expand by" popup ([`popup`]): "High value", "Low value" and the fields left, each a
//! `view.interact` `decomp_expand` action.

use egui::{Pos2, Rect, Sense, Ui, Vec2};
use serde_json::Value;

use crate::App;
use crate::app::clip;
use crate::i18n::Lang;
use bicraft_render::hits::{Hit, HitAction};

/// Most controls laid over one page.
const MAX_CONTROLS: usize = 500;

/// One control: its visual, report rectangle, label and what a click does.
struct Control {
    visual: String,
    rect: Rect,
    label: String,
    action: HitAction,
}

fn rect_of(h: &Hit) -> Rect {
    let (x, y, w, hh) = h.shape.bounds();
    Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, hh))
}

/// The tree visual `name` of the snapshot shown (its `x`, `width`, `data`).
fn visual<'a>(snap: &'a Value, name: &str) -> Option<&'a Value> {
    snap.get("report")?
        .get("pages")?
        .as_array()?
        .iter()
        .flat_map(|p| p.get("visuals").and_then(Value::as_array).into_iter().flatten())
        .find(|v| v.get("name").and_then(Value::as_str) == Some(name))
}

/// The name of data column `c` of a visual.
fn column_name(v: Option<&Value>, c: Option<u64>) -> String {
    let name = c
        .and_then(|c| usize::try_from(c).ok())
        .and_then(|c| {
            v?.get("data")?
                .get("columns")?
                .as_array()?
                .get(c)?
                .get("name")?
                .as_str()
                .map(str::to_string)
        })
        .unwrap_or_default();
    clip(&name, 60)
}

/// The field shown by level `level` (1-based) of a tree visual.
fn level_field(v: Option<&Value>, level: u64) -> String {
    let c = level
        .checked_sub(1)
        .and_then(|k| usize::try_from(k).ok())
        .and_then(|k| v?.get("data")?.get("decomp")?.get("levels")?.as_array()?.get(k))
        .and_then(|l| l.get("column"))
        .and_then(Value::as_u64);
    column_name(v, c)
}

fn member_text(m: &Value, l: Lang) -> String {
    match m {
        Value::String(s) if s.is_empty() => l.t("(Blank)", "(Vide)").to_string(),
        Value::String(s) => clip(s, 60),
        Value::Null => l.t("(Blank)", "(Vide)").to_string(),
        other => clip(&other.to_string(), 60),
    }
}

/// The controls of the decomposition trees on the page shown, with their labels.
fn controls(app: &App) -> Vec<Control> {
    let l = app.lang;
    let Some((_, Ok(img))) = app.shown.as_ref() else {
        return Vec::new();
    };
    let snap = app.loaded.as_ref().map(|x| &*x.snap);
    let mut out = Vec::new();
    for h in img.hits.iter().take(20_000) {
        if out.len() >= MAX_CONTROLS {
            break;
        }
        let v = snap.and_then(|s| visual(s, &h.visual));
        let label = match &h.action {
            HitAction::Popup(p) if p.get("kind").and_then(Value::as_str) == Some("decomp_expand") => {
                // the member it opens a level under (none: the root)
                let member = p
                    .get("items")
                    .and_then(Value::as_array)
                    .and_then(|i| i.first())
                    .and_then(|i| i.get("action"))
                    .filter(|_| p.get("level").and_then(Value::as_u64).is_some_and(|x| x > 0))
                    .and_then(|a| a.get("value"));
                match member {
                    Some(m) => format!(
                        "{} {}",
                        l.t("Add a level under", "Ajouter un niveau sous"),
                        member_text(m, l)
                    ),
                    None => l.t("Add a level", "Ajouter un niveau").to_string(),
                }
            }
            HitAction::Send { action, .. } => match action.get("action").and_then(Value::as_str) {
                Some("decomp_collapse") if is_cross(h) => {
                    let level = action.get("level").and_then(Value::as_u64).unwrap_or(0);
                    format!(
                        "{} {}",
                        l.t("Remove the level", "Retirer le niveau"),
                        level_field(v, level)
                    )
                }
                Some("decomp_scroll") if action.get("shift").is_some() => {
                    // the left arrow is on the visual's left half
                    let centre = v.map_or(0.0, |v| {
                        let n = |k: &str| v.get(k).and_then(Value::as_f64).unwrap_or(0.0) as f32;
                        n("x") + n("width") / 2.0
                    });
                    if rect_of(h).center().x < centre {
                        l.t("Scroll the tree left", "Faire défiler l'arbre vers la gauche")
                            .to_string()
                    } else {
                        l.t("Scroll the tree right", "Faire défiler l'arbre vers la droite")
                            .to_string()
                    }
                }
                Some("decomp_scroll") => {
                    let level = action.get("level").and_then(Value::as_u64).unwrap_or(0);
                    let field = level_field(v, level);
                    // the up arrow is above the tree's bars, the down arrow below them
                    if rect_of(h).center().y < bars_middle(img.hits.as_slice(), &h.visual) {
                        format!(
                            "{} {field} {}",
                            l.t("Scroll", "Faire défiler"),
                            l.t("up", "vers le haut")
                        )
                    } else {
                        format!(
                            "{} {field} {}",
                            l.t("Scroll", "Faire défiler"),
                            l.t("down", "vers le bas")
                        )
                    }
                }
                _ => continue,
            },
            _ => continue,
        };
        out.push(Control {
            visual: h.visual.clone(),
            rect: rect_of(h),
            label,
            action: h.action.clone(),
        });
    }
    out
}

/// Whether a `decomp_collapse` hit is a header's cross, a small region (with
/// `defaultClickAction: toggle`, a path member's bar sends it too: that one is a data point).
fn is_cross(h: &Hit) -> bool {
    let r = rect_of(h);
    r.height() < 30.0 && r.width() < 20.0
}

/// The vertical middle of a tree's bars (its `decomp_path` and `select` regions).
fn bars_middle(hits: &[Hit], visual: &str) -> f32 {
    let ys: Vec<f32> = hits
        .iter()
        .filter(|x| x.visual == visual)
        .filter(|x| match &x.action {
            HitAction::Send { action, .. } => matches!(
                action.get("action").and_then(Value::as_str),
                Some("decomp_path" | "select")
            ),
            _ => false,
        })
        .map(|x| rect_of(x).center().y)
        .filter(|y| y.is_finite())
        .collect();
    if ys.is_empty() {
        return f32::MAX;
    }
    ys.iter().sum::<f32>() / ys.len() as f32
}

/// Lays the controls over the page (`page_rect` on screen at `zoom`) and runs the one clicked.
/// Returns whether the pointer is on one.
pub fn buttons(app: &mut App, ui: &Ui, page_rect: Rect, zoom: f32) -> bool {
    let to_screen = |r: Rect| Rect::from_min_size(page_rect.min + r.min.to_vec2() * zoom, r.size() * zoom);
    let mut on = false;
    let mut clicked: Option<(HitAction, Pos2)> = None;
    for (k, c) in controls(app).into_iter().enumerate() {
        let screen = to_screen(c.rect);
        if !page_rect.intersects(screen) {
            continue;
        }
        let id = ui.scope_id().with(("bicraft-decomp", &c.visual, k));
        let resp = ui.interact(screen, id, Sense::click());
        let label = c.label.clone();
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
        if resp.has_focus() {
            let tokens = crate::theme::Tokens::get(ui.ctx());
            ui.painter().rect_stroke(
                screen.expand(1.0),
                2.0,
                egui::Stroke::new(2.0, tokens.focus),
                egui::StrokeKind::Outside,
            );
        }
        if resp.hovered() {
            on = true;
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.on_hover_text(&c.label).clicked() {
            clicked = Some((c.action, screen.left_bottom()));
        }
    }
    match clicked {
        Some((HitAction::Send { action, .. }, _)) => {
            app.popup = None;
            app.interact(action);
        }
        Some((HitAction::Popup(spec), at)) => app.open_popup(spec, at),
        _ => {}
    }
    on
}

/// The "Expand by" popup of a tree's "+": one button per item, the first focused when it opens.
/// Returns the action of the item chosen.
pub fn popup(ui: &mut Ui, spec: &Value, fresh: bool, l: Lang) -> Option<Value> {
    ui.label(egui::RichText::new(l.t("Expand by", "Développer par")).strong());
    let items = spec
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if items.is_empty() {
        ui.label(l.t("No field left", "Aucun champ restant"));
    }
    let mut send = None;
    for (k, it) in items.iter().take(64).enumerate() {
        let label = match it.get("ai").and_then(Value::as_str) {
            Some("high") => l.t("High value", "Valeur haute").to_string(),
            Some("low") => l.t("Low value", "Valeur basse").to_string(),
            _ => clip(it.get("label").and_then(Value::as_str).unwrap_or(""), 80),
        };
        let r = ui.add(egui::Button::new(label).min_size(Vec2::new(160.0, 0.0)));
        if fresh && k == 0 {
            r.request_focus();
        }
        if r.clicked()
            && let Some(a) = it.get("action").filter(|a| a.is_object())
        {
            send = Some(a.clone());
        }
    }
    send
}
