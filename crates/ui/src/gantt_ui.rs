//! A Gantt's reader controls as widgets over the drawn page (G99): each category's
//! expand/collapse icon (a `view.interact` `gantt_toggle`, engine state of the reader's view),
//! the rows' scroll bar and the time line's zoom and position (the app's view state, kept in
//! [`App::gantt`] and drawn by the renderer, never in the file). Each control is labelled for the
//! accessibility tree and reached with Tab; the scroll bar takes the arrows, Page Up / Page Down,
//! Home and End, the time line + and −, the left and right arrows and Home. With the pointer over
//! a Gantt the mouse wheel scrolls its rows (the page when the rows are at their end), a
//! horizontal scroll moves along a zoomed time line and Ctrl+wheel zooms it. The renderer's hit
//! map says where the controls go (`gantt_view` regions, `bicraft_render::gantt`).

use egui::{Id, Key, Pos2, Rect, Sense, Ui, Vec2};
use serde_json::Value;

use crate::App;
use crate::app::clip;
use crate::i18n::Lang;
use bicraft_render::gantt::{MAX_ZOOM, SCROLL_W, View};
use bicraft_render::hits::{Hit, HitAction};

/// Most expand/collapse buttons laid over one page.
const MAX_TOGGLES: usize = 500;
/// Zoom in or out by this factor per key press.
pub const ZOOM_STEP: f32 = 2.0;
/// The left and right arrows move a zoomed window by this share of its width.
pub const PAN_STEP: f32 = 0.1;

/// Where one Gantt's controls go, from its `gantt_view` region (body pixels within `body`).
#[derive(Debug, Clone)]
struct Area {
    visual: String,
    /// The visual's body, in report pixels.
    body: Rect,
    first: usize,
    shown: usize,
    total: usize,
    header_h: f32,
    rows_bottom: f32,
    row_h: f32,
    chart_x: f32,
    chart_w: f32,
    footer_h: f32,
    vbar: bool,
    hbar: bool,
    zoom: f32,
    at: f32,
}

impl Area {
    fn view(&self) -> View {
        View {
            first: self.first,
            zoom: self.zoom,
            at: self.at,
        }
    }

    fn rect(&self, x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(self.body.min + Vec2::new(x, y), Vec2::new(w.max(1.0), h.max(1.0)))
    }

    /// The rows' scroll bar.
    fn vbar_rect(&self) -> Rect {
        self.rect(
            self.chart_x + self.chart_w,
            self.header_h,
            SCROLL_W,
            self.rows_bottom - self.header_h,
        )
    }

    /// The zoomed time line's bar, under the rows.
    fn hbar_rect(&self) -> Rect {
        self.rect(self.chart_x, self.rows_bottom, self.chart_w, SCROLL_W)
    }

    /// The time line's axis band: above the rows, else under them.
    fn axis_rect(&self) -> Rect {
        if self.header_h > 0.0 {
            self.rect(self.chart_x, 0.0, self.chart_w, self.header_h)
        } else {
            let y = self.body.height() - self.footer_h;
            self.rect(self.chart_x, y, self.chart_w, self.footer_h.max(SCROLL_W))
        }
    }
}

fn rect_of(h: &Hit) -> Rect {
    let (x, y, w, hh) = h.shape.bounds();
    Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, hh))
}

fn num(m: &Value, k: &str) -> f32 {
    m.get(k)
        .and_then(Value::as_f64)
        .filter(|x| x.is_finite())
        .unwrap_or(0.0) as f32
}

fn count(m: &Value, k: &str) -> usize {
    m.get(k)
        .and_then(Value::as_u64)
        .and_then(|x| usize::try_from(x).ok())
        .unwrap_or(0)
}

/// The Gantts of the page shown.
fn areas(app: &App) -> Vec<Area> {
    let Some((_, Ok(img))) = app.shown.as_ref() else {
        return Vec::new();
    };
    img.hits
        .iter()
        .take(20_000)
        .filter_map(|h| match &h.action {
            HitAction::Menu(m) if m.get("kind").and_then(Value::as_str) == Some("gantt_view") => Some(Area {
                visual: h.visual.clone(),
                body: rect_of(h),
                first: count(m, "first"),
                shown: count(m, "shown"),
                total: count(m, "total"),
                header_h: num(m, "header_h"),
                rows_bottom: num(m, "rows_bottom"),
                row_h: num(m, "row_h").max(1.0),
                chart_x: num(m, "chart_x"),
                chart_w: num(m, "chart_w").max(1.0),
                footer_h: num(m, "footer_h"),
                vbar: m.get("vbar").and_then(Value::as_bool) == Some(true),
                hbar: m.get("hbar").and_then(Value::as_bool) == Some(true),
                zoom: num(m, "zoom").max(1.0),
                at: num(m, "at").clamp(0.0, 1.0),
            }),
            _ => None,
        })
        .collect()
}

/// `v` scrolled by `by` lines, within a list of `total` lines of which `shown` fit.
pub fn scrolled(v: View, by: i64, total: usize, shown: usize) -> View {
    let last = total.saturating_sub(shown);
    let first = v.first.min(last);
    let first = if by < 0 {
        first.saturating_sub(usize::try_from(by.unsigned_abs()).unwrap_or(usize::MAX))
    } else {
        first
            .saturating_add(usize::try_from(by).unwrap_or(usize::MAX))
            .min(last)
    };
    View { first, ..v }
}

/// `v` with its time line zoomed by `factor` (above 1: in), the window's centre kept; at 1 or
/// less the whole time line.
pub fn zoomed(v: View, factor: f32) -> View {
    let z0 = if v.zoom.is_finite() { v.zoom.max(1.0) } else { 1.0 };
    let f = if factor.is_finite() && factor > 0.0 {
        factor
    } else {
        1.0
    };
    let z1 = (z0 * f).clamp(1.0, MAX_ZOOM);
    if z1 <= 1.0 + 1e-4 {
        return View {
            zoom: 1.0,
            at: 0.0,
            ..v
        };
    }
    // the window's centre, as a share of the whole time line
    let at = if v.at.is_finite() {
        v.at.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let centre = if z0 > 1.0 {
        at * (1.0 - 1.0 / z0) + 0.5 / z0
    } else {
        0.5
    };
    let at = ((centre - 0.5 / z1) / (1.0 - 1.0 / z1)).clamp(0.0, 1.0);
    View { zoom: z1, at, ..v }
}

/// `v` showing its whole time line.
pub fn whole(v: View) -> View {
    View {
        zoom: 1.0,
        at: 0.0,
        ..v
    }
}

/// `v`'s zoomed window moved by `by` of its width (negative: towards the start).
pub fn panned(v: View, by: f32) -> View {
    if !(v.zoom.is_finite() && v.zoom > 1.0) || !by.is_finite() {
        return v;
    }
    let at = (v.at + by / (v.zoom - 1.0)).clamp(0.0, 1.0);
    View { at, ..v }
}

fn member_text(m: &Value, l: Lang) -> String {
    match m {
        Value::String(s) if s.is_empty() => l.t("(Blank)", "(Vide)").to_string(),
        Value::String(s) => clip(s, 60),
        Value::Null => l.t("(Blank)", "(Vide)").to_string(),
        other => clip(&other.to_string(), 60),
    }
}

/// The expand/collapse buttons of the page's Gantts: (visual, report rectangle, label, action).
fn toggles(app: &App) -> Vec<(String, Rect, String, Value)> {
    let l = app.lang;
    let Some((_, Ok(img))) = app.shown.as_ref() else {
        return Vec::new();
    };
    img.hits
        .iter()
        .take(20_000)
        .filter_map(|h| match &h.action {
            HitAction::Send { action, .. }
                if action.get("action").and_then(Value::as_str) == Some("gantt_toggle") =>
            {
                let member = action
                    .get("path")
                    .and_then(Value::as_array)
                    .and_then(|p| p.last())
                    .map(|m| member_text(m, l))
                    .unwrap_or_default();
                let verb = if action.get("collapsed").and_then(Value::as_bool) == Some(true) {
                    l.t("Collapse", "Réduire")
                } else {
                    l.t("Expand", "Développer")
                };
                Some((
                    h.visual.clone(),
                    rect_of(h),
                    format!("{verb} {member}"),
                    action.clone(),
                ))
            }
            _ => None,
        })
        .take(MAX_TOGGLES)
        .collect()
}

fn focus_ring(ui: &Ui, r: Rect) {
    let tokens = crate::theme::Tokens::get(ui.ctx());
    ui.painter().rect_stroke(
        r.expand(1.0),
        2.0,
        egui::Stroke::new(2.0, tokens.focus),
        egui::StrokeKind::Outside,
    );
}

/// The keys pressed this frame among `keys`, consumed (their first match).
fn pressed(ui: &Ui, keys: &[Key]) -> Option<Key> {
    ui.ctx().input_mut(|i| {
        keys.iter()
            .copied()
            .find(|k| i.consume_key(egui::Modifiers::NONE, *k) || i.consume_key(egui::Modifiers::SHIFT, *k))
    })
}

/// Lays the page's Gantt controls over the page (`page_rect` on screen at `zoom`) and runs the
/// one used. Returns whether the pointer is on one.
pub fn controls(app: &mut App, ui: &Ui, page_rect: Rect, zoom: f32) -> bool {
    let l = app.lang;
    let to_screen = |r: Rect| Rect::from_min_size(page_rect.min + r.min.to_vec2() * zoom, r.size() * zoom);
    let mut on = false;
    // expand/collapse: engine state of the reader's view
    let mut send: Option<Value> = None;
    for (k, (visual, r, label, action)) in toggles(app).into_iter().enumerate() {
        let screen = to_screen(r);
        if !page_rect.intersects(screen) {
            continue;
        }
        let resp = ui.interact(
            screen,
            ui.scope_id().with(("bicraft-gantt-toggle", &visual, k)),
            Sense::click(),
        );
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
        if resp.has_focus() {
            focus_ring(ui, screen);
        }
        if resp.hovered() {
            on = true;
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.on_hover_text(&label).clicked() {
            send = Some(action);
        }
    }
    // scrolling and zoom: the app's view state
    for a in areas(app) {
        let start = a.view();
        let mut v = start;
        let body = to_screen(a.body);
        let row_px = (a.row_h * zoom).max(1.0);
        if a.vbar {
            let screen = to_screen(a.vbar_rect());
            let id = ui.scope_id().with(("bicraft-gantt-rows", &a.visual));
            let resp = ui.interact(screen, id, Sense::click_and_drag());
            let label = format!(
                "{} {}–{} / {}",
                l.t("Scroll the tasks: rows", "Faire défiler les tâches : lignes"),
                a.first + 1,
                a.first + a.shown,
                a.total
            );
            resp.widget_info(|| {
                let mut info = egui::WidgetInfo::labeled(egui::Role::ScrollBar, true, &label);
                info.value = Some(a.first as f64);
                info
            });
            if resp.has_focus() {
                focus_ring(ui, screen);
                ui.memory_mut(|m| {
                    m.set_focus_lock_filter(
                        id,
                        egui::EventFilter {
                            vertical_arrows: true,
                            ..Default::default()
                        },
                    )
                });
                let page = i64::try_from(a.shown.max(1)).unwrap_or(1);
                let by = match pressed(
                    ui,
                    &[
                        Key::ArrowUp,
                        Key::ArrowDown,
                        Key::PageUp,
                        Key::PageDown,
                        Key::Home,
                        Key::End,
                    ],
                ) {
                    Some(Key::ArrowUp) => -1,
                    Some(Key::ArrowDown) => 1,
                    Some(Key::PageUp) => -page,
                    Some(Key::PageDown) => page,
                    Some(Key::Home) => i64::MIN / 2,
                    Some(Key::End) => i64::MAX / 2,
                    _ => 0,
                };
                if by != 0 {
                    v = scrolled(v, by, a.total, a.shown);
                }
            }
            if resp.dragged() {
                // the thumb follows the pointer: a line per (track / total) pixels
                let per_line = (screen.height() / a.total.max(1) as f32).max(0.5);
                let acc = drag_lines(ui, id, resp.drag_delta().y, per_line);
                if acc != 0 {
                    v = scrolled(v, acc, a.total, a.shown);
                }
            } else if resp.clicked()
                && let Some(p) = resp.interact_pointer_pos()
            {
                // a click on the track pages towards it
                let thumb = screen.top() + screen.height() * a.first as f32 / a.total.max(1) as f32;
                let page = i64::try_from(a.shown.max(1)).unwrap_or(1);
                v = scrolled(v, if p.y < thumb { -page } else { page }, a.total, a.shown);
            }
            on |= resp.on_hover_text(&label).hovered();
        }
        // the time line: zoom and position
        {
            let screen = to_screen(a.axis_rect());
            let id = ui.scope_id().with(("bicraft-gantt-zoom", &a.visual));
            let resp = ui.interact(screen, id, Sense::click());
            let label = format!(
                "{} {}×",
                l.t("Zoom the time line:", "Zoom de la chronologie :"),
                (a.zoom * 10.0).round() / 10.0
            );
            resp.widget_info(|| {
                let mut info = egui::WidgetInfo::labeled(egui::Role::Slider, true, &label);
                info.value = Some(f64::from(a.zoom));
                info
            });
            if resp.has_focus() {
                focus_ring(ui, screen);
                ui.memory_mut(|m| {
                    m.set_focus_lock_filter(
                        id,
                        egui::EventFilter {
                            horizontal_arrows: true,
                            ..Default::default()
                        },
                    )
                });
                match pressed(
                    ui,
                    &[
                        Key::Plus,
                        Key::Equals,
                        Key::Minus,
                        Key::ArrowLeft,
                        Key::ArrowRight,
                        Key::Home,
                    ],
                ) {
                    Some(Key::Plus | Key::Equals) => v = zoomed(v, ZOOM_STEP),
                    Some(Key::Minus) => v = zoomed(v, 1.0 / ZOOM_STEP),
                    Some(Key::ArrowLeft) => v = panned(v, -PAN_STEP),
                    Some(Key::ArrowRight) => v = panned(v, PAN_STEP),
                    Some(Key::Home) => v = whole(v),
                    _ => {}
                }
            }
            if resp.double_clicked() {
                v = whole(v);
            }
            let hint = l.t(
                "+ and − zoom, the left and right arrows move along it, Home shows all of it; Ctrl+wheel zooms",
                "+ et − zooment, les flèches gauche et droite la parcourent, Origine la montre en entier ; Ctrl+molette zoome",
            );
            on |= resp.on_hover_text(format!("{label}\n{hint}")).hovered();
        }
        if a.hbar {
            let screen = to_screen(a.hbar_rect());
            let id = ui.scope_id().with(("bicraft-gantt-window", &a.visual));
            let resp = ui.interact(screen, id, Sense::click_and_drag());
            let label = l.t("Time line position", "Position dans la chronologie");
            resp.widget_info(|| {
                let mut info = egui::WidgetInfo::labeled(egui::Role::ScrollBar, true, label);
                info.value = Some(f64::from(a.at));
                info
            });
            if resp.has_focus() {
                focus_ring(ui, screen);
            }
            if resp.dragged() {
                // the window follows the pointer over the whole time line
                let by = resp.drag_delta().x / screen.width().max(1.0) * a.zoom;
                v = panned(v, by);
            }
            on |= resp.on_hover_text(label).hovered();
        }
        // the mouse wheel over the Gantt
        if ui.rect_contains_pointer(body) {
            let (dz, d) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta()));
            if (dz - 1.0).abs() > 1e-4 {
                v = zoomed(v, dz);
            }
            if a.vbar && d.y != 0.0 {
                let id = ui.scope_id().with(("bicraft-gantt-wheel", &a.visual));
                let lines = drag_lines(ui, id, -d.y, row_px);
                let next = scrolled(v, lines, a.total, a.shown);
                // at its end the list lets the page scroll
                let room = if d.y > 0.0 {
                    v.first > 0
                } else {
                    v.first + a.shown < a.total
                };
                if room {
                    v = next;
                    ui.ctx().input_mut(|i| i.smooth_scroll_delta.y = 0.0);
                }
            }
            if a.hbar && d.x != 0.0 {
                v = panned(v, -d.x / (a.chart_w * zoom).max(1.0));
                ui.ctx().input_mut(|i| i.smooth_scroll_delta.x = 0.0);
            }
        }
        if v != start {
            app.gantt.insert(a.visual.clone(), v);
            ui.ctx().request_repaint();
        }
    }
    if let Some(action) = send {
        app.popup = None;
        app.interact(action);
    }
    on
}

/// Whole lines from `delta` pixels at `per_line` pixels a line, the remainder carried in the
/// widget's memory (`id`) to the next frame.
fn drag_lines(ui: &Ui, id: Id, delta: f32, per_line: f32) -> i64 {
    let key = id.with("acc");
    let acc: f32 = ui.ctx().data(|d| d.get_temp(key)).unwrap_or(0.0) + delta / per_line.max(1e-3);
    let whole = acc.trunc();
    ui.ctx().data_mut(|d| d.insert_temp(key, acc - whole));
    if whole.is_finite() {
        whole.clamp(-1e9, 1e9) as i64
    } else {
        0
    }
}
