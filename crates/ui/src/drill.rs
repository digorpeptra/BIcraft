//! The reader's drill on the canvas, as Power BI Desktop offers it (docs/desktop.md, "Drill,
//! drillthrough and page tooltips"): the visual header's drill icons on hover (drill up, drill
//! mode, go to the next level, expand all down one level), the right-click entries (Drill down,
//! Drill up, Show next level, Expand to next level, a matrix's Expand and Collapse, Drill
//! through), and report-page tooltips drawn small under the pointer. Every action is a
//! `view.interact` action sent like the other reader clicks; the rules are the engine's
//! (`crates/engine/src/interact_drill.rs`).

use egui::{Color32, Pos2, Rect, Response, Sense, Stroke, TextureHandle, TextureOptions, Ui, Vec2};
use serde_json::{Value, json};

use crate::App;
use crate::i18n::Lang;
use crate::pages::Key;
use crate::theme::Tokens;

/// Side of a header icon, in screen points.
const ICON: f32 = 20.0;
/// How long the pointer rests on a point before its tooltip page is asked for (seconds).
const DWELL: f64 = 0.25;
/// Widest tooltip page drawn, in screen points.
const MAX_TIP_W: f32 = 480.0;

/// The canvas's drill view state.
#[derive(Default)]
pub struct DrillUi {
    /// The hit under the pointer at the last right click (the menu acts on it).
    pub menu_hit: Option<bicraft_render::hits::Hit>,
    /// The point the pointer rests on: (visual, point, since).
    hover: Option<(String, Value, f64)>,
    /// The tooltip asked for: (visual, point).
    pub asked: Option<(String, Value)>,
    /// The tooltip page drawn: (snapshot seq, page index) and its texture.
    tip_tex: Option<((u64, usize), TextureHandle, Vec2)>,
    fonts: Option<bicraft_render::FontBook>,
}

/// One header icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Up,
    Mode,
    Next,
    Expand,
}

impl Icon {
    pub const ALL: [Icon; 4] = [Icon::Up, Icon::Mode, Icon::Next, Icon::Expand];

    /// The accessible label (Desktop's tooltips, in BIcraft's words).
    pub fn label(self, l: Lang, on: bool) -> &'static str {
        match self {
            Icon::Up => l.t("Drill up", "Remonter"),
            Icon::Mode if on => l.t(
                "Drill mode on (click a point to drill into it)",
                "Mode d'exploration activé (cliquez un point pour y descendre)",
            ),
            Icon::Mode => l.t("Turn drill mode on", "Activer le mode d'exploration"),
            Icon::Next => l.t("Go to the next level", "Aller au niveau suivant"),
            Icon::Expand => l.t("Expand all down one level", "Tout développer d'un niveau"),
        }
    }

    fn action(self) -> &'static str {
        match self {
            Icon::Up => "drill_up",
            Icon::Mode => "drill_mode",
            Icon::Next => "drill_next",
            Icon::Expand => "drill_expand",
        }
    }
}

/// A visual of the page shown: name, box in report pixels, its `drill` and `interaction`.
fn visuals(app: &App) -> Vec<(String, Rect, Value, Value)> {
    let Some(l) = &app.loaded else { return Vec::new() };
    let Some(page) = l.snap.pointer(&format!("/report/pages/{}", app.active_page())) else {
        return Vec::new();
    };
    let n = |v: &Value, k: &str| v.get(k).and_then(Value::as_f64).filter(|x| x.is_finite());
    page.get("visuals")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(2000)
        .filter(|v| v.get("hidden").and_then(Value::as_bool) != Some(true))
        .filter_map(|v| {
            let r = Rect::from_min_size(
                egui::pos2(n(v, "x")? as f32, n(v, "y")? as f32),
                egui::vec2(n(v, "width")? as f32, n(v, "height")? as f32),
            );
            Some((
                v.get("name")?.as_str()?.to_string(),
                r,
                v.get("drill").cloned().unwrap_or(Value::Null),
                v.get("interaction").cloned().unwrap_or(Value::Null),
            ))
        })
        .collect()
}

fn page_name(app: &App) -> Option<String> {
    app.page_names().get(app.active_page()).map(|p| p.0.clone())
}

/// Where the header icons of a visual whose box is `screen` sit (right-aligned at its top).
pub fn icon_rects(screen: Rect) -> Vec<(Icon, Rect)> {
    let y = screen.min.y + 2.0;
    Icon::ALL
        .iter()
        .rev()
        .enumerate()
        .map(|(i, ic)| {
            let x = screen.max.x - 4.0 - (i as f32 + 1.0) * (ICON + 2.0);
            (*ic, Rect::from_min_size(egui::pos2(x, y), Vec2::splat(ICON)))
        })
        .collect()
}

/// Draws one icon in `r`.
fn paint(ui: &Ui, ic: Icon, r: Rect, color: Color32) {
    let p = ui.painter();
    let s = Stroke::new(1.6, color);
    let c = r.center();
    let h = r.height() * 0.32;
    let arrow = |top: Pos2, bottom: Pos2, down: bool| {
        p.line_segment([top, bottom], s);
        let tip = if down { bottom } else { top };
        let k = if down { -1.0 } else { 1.0 };
        p.line_segment([tip, tip + Vec2::new(-h * 0.6, k * h * 0.6)], s);
        p.line_segment([tip, tip + Vec2::new(h * 0.6, k * h * 0.6)], s);
    };
    match ic {
        Icon::Up => arrow(c - Vec2::new(0.0, h), c + Vec2::new(0.0, h), false),
        Icon::Mode => {
            p.circle_stroke(c, h * 1.35, Stroke::new(1.2, color));
            arrow(c - Vec2::new(0.0, h * 0.8), c + Vec2::new(0.0, h * 0.8), true);
        }
        Icon::Next => {
            for dx in [-h * 0.55, h * 0.55] {
                arrow(c + Vec2::new(dx, -h), c + Vec2::new(dx, h), true);
            }
        }
        Icon::Expand => {
            // a fork: one stem splitting in two arrows
            let mid = c + Vec2::new(0.0, -h * 0.1);
            p.line_segment([c - Vec2::new(0.0, h), mid], s);
            for dx in [-h * 0.7, h * 0.7] {
                let end = c + Vec2::new(dx, h);
                p.line_segment([mid, end], s);
                p.line_segment([end, end + Vec2::new(0.0, -h * 0.55)], s);
                p.line_segment([end, end + Vec2::new(-dx.signum() * h * 0.55, 0.0)], s);
            }
        }
    }
}

/// The header icons of the visual under the pointer: drawn over the page and clickable, greyed
/// when the action cannot be done (drill up at the top level…). Returns whether the pointer is
/// on one (the page's own click is then not a page click).
pub fn header(app: &mut App, ui: &Ui, page_rect: Rect, zoom: f32) -> bool {
    let Some(pointer) = ui.ctx().pointer_hover_pos() else {
        return false;
    };
    if !page_rect.contains(pointer) {
        return false;
    }
    let to_screen = |r: Rect| Rect::from_min_size(page_rect.min + r.min.to_vec2() * zoom, r.size() * zoom);
    // the topmost drillable visual under the pointer (the icons stay while the pointer is on them)
    let Some((name, screen, drill)) = visuals(app)
        .into_iter()
        .filter(|(_, _, d, _)| d.is_object())
        .map(|(n, r, d, _)| (n, to_screen(r), d))
        .rfind(|(_, s, _)| s.expand(2.0).contains(pointer))
    else {
        return false;
    };
    let Some(page) = page_name(app) else { return false };
    let t = Tokens::of(app.prefs.theme);
    let l = app.lang;
    let flag = |k: &str| drill.get(k).and_then(Value::as_bool) == Some(true);
    let mut on_icon = false;
    let mut send = None;
    for (ic, r) in icon_rects(screen) {
        let enabled = match ic {
            Icon::Up => flag("can_up"),
            Icon::Mode => true,
            Icon::Next | Icon::Expand => flag("can_down"),
        };
        let mode_on = ic == Icon::Mode && flag("mode");
        let id = ui.scope_id().with(("bicraft-drill-icon", &name, ic.action()));
        let resp = ui.interact(r, id, if enabled { Sense::click() } else { Sense::hover() });
        let label = ic.label(l, mode_on);
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
        let bg = if mode_on {
            t.accent_soft
        } else if resp.hovered() && enabled {
            t.hover
        } else {
            t.card
        };
        ui.painter().rect_filled(r, egui::CornerRadius::same(3), bg);
        ui.painter().rect_stroke(
            r,
            egui::CornerRadius::same(3),
            Stroke::new(1.0, t.border),
            egui::StrokeKind::Inside,
        );
        paint(ui, ic, r, if enabled { t.text } else { t.text_faint });
        if resp.hovered() {
            on_icon = true;
        }
        let resp = resp.on_hover_text(label);
        if resp.clicked() {
            send = Some(json!({"action": ic.action(), "page": page, "visual": name}));
        }
    }
    if let Some(a) = send {
        app.interact(a);
    }
    on_icon
}

/// The command that edits the report for a matrix's expand and collapse.
pub const EXPAND: &str = "visual.expand";
/// The command that saves an org chart node's collapse or expand in the report.
pub const TOGGLE_NODE: &str = "visual.toggle_node";

/// In the editing view, a matrix's expand and collapse (the row header's +/-, the right-click
/// Expand / Collapse entries, the header's Expand all down one level, Go to the next level and
/// Drill up) are edits of the report, saved with the file as Desktop saves them, and undone
/// like any edit: `visual.expand` instead of the reader's `view.interact`. So is a BIcraft Org
/// Chart node's collapse button (`tree_toggle`: `visual.toggle_node`, which saves `tree.toggled`
/// as the visual's TypeScript persists it in Desktop). A reader (or a session that may not edit
/// the report) keeps the reader's state. True when the action went to the edit command.
pub fn route(app: &mut App, action: &Value) -> bool {
    let Some(kind) = action.get("action").and_then(Value::as_str) else {
        return false;
    };
    let (Some(page), Some(visual)) = (
        action.get("page").and_then(Value::as_str),
        action.get("visual").and_then(Value::as_str),
    ) else {
        return false;
    };
    let drill = visuals(app)
        .into_iter()
        .find(|(n, ..)| n == visual)
        .map(|(_, _, d, _)| d)
        .unwrap_or(Value::Null);
    let matrix = drill.get("matrix").and_then(Value::as_bool) == Some(true);
    let mut p = json!({"page": page, "visual": visual});
    let mut id = EXPAND;
    match kind {
        "tree_toggle" => {
            p["id"] = action.get("id").cloned().unwrap_or(Value::Null);
            id = TOGGLE_NODE;
        }
        "matrix_expand" | "matrix_collapse" => {
            for k in ["path", "level", "all", "role"] {
                if let Some(x) = action.get(k).filter(|x| !x.is_null()) {
                    p[k] = x.clone();
                }
            }
            if kind == "matrix_collapse" {
                p["collapse"] = json!(true);
            }
        }
        "drill_expand" | "drill_next" | "drill_up" if matrix => {
            p["step"] = json!(match kind {
                "drill_expand" => "expand_all",
                "drill_next" => "next",
                _ => "up",
            });
            if let Some(r) = drill.get("role").filter(|r| r.is_string()) {
                p["role"] = r.clone();
            }
        }
        _ => return false,
    }
    if app.access.as_deref() == Some("reader") || crate::ribbon::state(app, id).is_err() {
        return false;
    }
    app.last_action = Some(action.clone());
    app.run(crate::actions::Call { id, params: p }, crate::app::Then::Nothing)
}

/// A matrix row header's +/- button over the page drawn: (visual, hit rectangle in report pixels,
/// expanded, member path, the action it sends).
fn plus_minus(app: &App) -> Vec<(String, Rect, bool, Vec<Value>, Value)> {
    let Some((_, Ok(img))) = app.shown.as_ref() else {
        return Vec::new();
    };
    img.hits
        .iter()
        .take(20_000)
        .filter_map(|h| {
            let a = h.action_for(false)?;
            let open = match a.get("action").and_then(Value::as_str)? {
                "matrix_collapse" => true,
                "matrix_expand" => false,
                _ => return None,
            };
            let path = a.get("path")?.as_array()?.clone();
            let (x, y, w, hh) = h.shape.bounds();
            let r = Rect::from_min_size(egui::pos2(x, y), Vec2::new(w, hh));
            Some((h.visual.clone(), r, open, path, a))
        })
        .take(500)
        .collect()
}

/// A member path as the words of a +/- button's label (`North › Bikes`).
fn path_words(path: &[Value]) -> String {
    path.iter()
        .take(8)
        .map(|x| match x {
            Value::String(s) => crate::app::clip(s, 40),
            Value::Null => "(Blank)".to_string(),
            other => crate::app::clip(&other.to_string(), 40),
        })
        .collect::<Vec<_>>()
        .join(" › ")
}

/// The matrices' +/- buttons as widgets over the drawn page: each one labelled for the
/// accessibility tree ("Expand North", "Collapse North › Bikes") and clickable like the drawn
/// icon (the same action, [`route`]d). Returns whether the pointer is on one.
pub fn expand_buttons(app: &mut App, ui: &Ui, page_rect: Rect, zoom: f32) -> bool {
    let l = app.lang;
    let to_screen = |r: Rect| Rect::from_min_size(page_rect.min + r.min.to_vec2() * zoom, r.size() * zoom);
    let mut on = false;
    let mut send = None;
    for (visual, r, open, path, action) in plus_minus(app) {
        let screen = to_screen(r);
        if !page_rect.intersects(screen) {
            continue;
        }
        let words = path_words(&path);
        let label = if open {
            format!("{} {words}", l.t("Collapse", "Réduire"))
        } else {
            format!("{} {words}", l.t("Expand", "Développer"))
        };
        let id = ui.scope_id().with(("bicraft-matrix-pm", &visual, &words));
        let resp = ui.interact(screen, id, Sense::click());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
        if resp.hovered() {
            on = true;
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.on_hover_text(&label).clicked() {
            send = Some(action);
        }
    }
    if let Some(a) = send {
        app.popup = None;
        app.interact(a);
    }
    on
}

/// The point of a hit (`select` hits), `None` for other regions.
fn point_of(hit: &bicraft_render::hits::Hit) -> Option<Value> {
    match &hit.action {
        bicraft_render::hits::HitAction::Send { action, .. }
            if action.get("action") == Some(&json!("select")) =>
        {
            action.get("point").cloned()
        }
        _ => None,
    }
}

/// The drill, matrix and drillthrough entries of the visual's right-click menu (above its
/// commands): what they do depends on the region right-clicked ([`DrillUi::menu_hit`]).
pub fn menu(app: &mut App, ui: &mut Ui, visual: &str) {
    let l = app.lang;
    let Some(page) = page_name(app) else { return };
    let Some((_, _, drill, interaction)) = visuals(app).into_iter().find(|(n, ..)| n == visual) else {
        return;
    };
    let hit = app.drill.menu_hit.clone().filter(|h| h.visual == visual);
    let point = hit.as_ref().and_then(point_of);
    let row = hit.as_ref().and_then(|h| match &h.action {
        bicraft_render::hits::HitAction::Menu(m) if m.get("kind") == Some(&json!("matrix_row")) => {
            Some(m.clone())
        }
        _ => None,
    });
    let flag = |k: &str| drill.get(k).and_then(Value::as_bool) == Some(true);
    let base = json!({"page": page, "visual": visual});
    let with = |action: &str, extra: Value| {
        let mut a = base.clone();
        a["action"] = json!(action);
        if let (Some(o), Value::Object(e)) = (a.as_object_mut(), extra) {
            o.extend(e);
        }
        a
    };
    let mut send: Option<Value> = None;
    let mut any = false;
    if drill.is_object() {
        any = true;
        let item = |ui: &mut Ui, text: &str, on: bool| ui.add_enabled(on, egui::Button::new(text)).clicked();
        if item(
            ui,
            l.t("Drill down", "Descendre"),
            point.is_some() && flag("can_down"),
        ) && let Some(p) = &point
        {
            send = Some(with("drill_down", json!({"point": p})));
        }
        if item(ui, l.t("Drill up", "Remonter"), flag("can_up")) {
            send = Some(with("drill_up", json!({})));
        }
        if item(
            ui,
            l.t("Show next level", "Afficher le niveau suivant"),
            flag("can_down"),
        ) {
            send = Some(with("drill_next", json!({})));
        }
        if item(
            ui,
            l.t("Expand to next level", "Développer au niveau suivant"),
            flag("can_down"),
        ) {
            send = Some(with("drill_expand", json!({})));
        }
    }
    if let Some(r) = &row {
        any = true;
        let path = r.get("path").cloned().unwrap_or(Value::Null);
        let depth = path.as_array().map_or(1, Vec::len).max(1);
        let level = json!(depth - 1);
        ui.separator();
        ui.menu_button(l.t("Expand", "Développer"), |ui| {
            if ui.button(l.t("Selection", "Sélection")).clicked() {
                send = Some(with("matrix_expand", json!({"path": path})));
                ui.close();
            }
            if ui.button(l.t("Entire level", "Niveau entier")).clicked() {
                send = Some(with("matrix_expand", json!({"level": level})));
                ui.close();
            }
            if ui.button(l.t("All", "Tout")).clicked() {
                send = Some(with("matrix_expand", json!({"all": true})));
                ui.close();
            }
        });
        ui.menu_button(l.t("Collapse", "Réduire"), |ui| {
            if ui.button(l.t("Selection", "Sélection")).clicked() {
                send = Some(with("matrix_collapse", json!({"path": path})));
                ui.close();
            }
            if ui.button(l.t("Entire level", "Niveau entier")).clicked() {
                send = Some(with("matrix_collapse", json!({"level": level})));
                ui.close();
            }
            if ui.button(l.t("All", "Tout")).clicked() {
                send = Some(with("matrix_collapse", json!({"all": true})));
                ui.close();
            }
        });
    }
    // drillthrough: the pages one of whose fields the point holds
    let targets: Vec<Value> = interaction
        .get("drillthrough")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if let Some(p) = &point
        && !targets.is_empty()
    {
        let cs: Vec<u64> = p
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|e| e.get("c").and_then(Value::as_u64))
            .collect();
        let usable: Vec<&Value> = targets
            .iter()
            .filter(|t| {
                t.get("columns")
                    .and_then(Value::as_array)
                    .is_some_and(|c| c.iter().filter_map(Value::as_u64).any(|c| cs.contains(&c)))
            })
            .collect();
        if !usable.is_empty() {
            any = true;
            ui.separator();
            ui.menu_button(l.t("Drill through", "Extraire"), |ui| {
                for t in usable.iter().take(50) {
                    let label = t.get("label").and_then(Value::as_str).unwrap_or("");
                    let cross = t.get("cross_report").and_then(Value::as_bool) == Some(true);
                    let r = ui.add_enabled(!cross, egui::Button::new(crate::app::clip(label, 60)));
                    if cross {
                        r.on_disabled_hover_text(l.t(
                            "Cross-report drillthrough is a Power BI service feature; BIcraft does not follow it.",
                            "L'extraction entre rapports est une fonction du service Power BI ; BIcraft ne la suit pas.",
                        ));
                    } else if r.clicked() {
                        let target = t.get("page").cloned().unwrap_or(Value::Null);
                        send = Some(with("drillthrough", json!({"point": p, "target": target})));
                        ui.close();
                    }
                }
            });
        }
    }
    if any {
        ui.separator();
    }
    if let Some(a) = send {
        ui.close();
        app.interact(a);
    }
}

/// Report-page tooltips: while the pointer rests on a point of a visual with a tooltip page,
/// asks the engine for that page under the point's context, then draws it small under the
/// pointer instead of the default tooltip. Returns whether the hovered point has a tooltip page
/// (the default tooltip is then not shown).
pub fn tooltip(app: &mut App, ui: &Ui, resp: &Response, hit: Option<&bicraft_render::hits::Hit>) -> bool {
    let now = ui.input(|i| i.time);
    let target = hit.and_then(|h| {
        let point = point_of(h)?;
        let (_, _, _, ix) = visuals(app).into_iter().find(|(n, ..)| *n == h.visual)?;
        let page = ix.get("tooltip_page")?.as_str()?.to_string();
        Some((h.visual.clone(), point, page))
    });
    let Some((visual, point, tip_page)) = target else {
        app.drill.hover = None;
        if app.drill.asked.take().is_some()
            && let Some(page) = page_name(app)
        {
            app.interact(json!({"action": "tooltip", "page": page, "visual": "", "point": null}));
        }
        return false;
    };
    // the dwell: a new point restarts it
    let since = match &app.drill.hover {
        Some((v, p, t)) if *v == visual && *p == point => *t,
        _ => {
            app.drill.hover = Some((visual.clone(), point.clone(), now));
            now
        }
    };
    let asked = app
        .drill
        .asked
        .as_ref()
        .is_some_and(|(v, p)| *v == visual && *p == point);
    if !asked {
        if now - since >= DWELL || app.sync_rendering() {
            if let Some(page) = page_name(app) {
                app.interact(json!({"action": "tooltip", "page": page, "visual": visual, "point": point}));
                app.drill.asked = Some((visual.clone(), point.clone()));
            }
        } else {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(DWELL));
        }
        return true;
    }
    // the snapshot under the point's context has arrived: draw its tooltip page
    let Some(loaded) = app.loaded.clone() else {
        return true;
    };
    let shown = loaded.snap.pointer("/interaction/tooltip");
    let ready = shown.is_some_and(|t| {
        t.get("visual").and_then(Value::as_str) == Some(visual.as_str()) && t.get("point") == Some(&point)
    });
    if !ready {
        return true;
    }
    let Some(index) = app.page_names().iter().position(|p| p.0 == tip_page) else {
        return true;
    };
    let key = (loaded.seq, index);
    if app.drill.tip_tex.as_ref().map(|t| t.0) != Some(key) {
        let pv = loaded.snap.pointer(&format!("/report/pages/{index}"));
        let dim = |k: &str, d: f64| {
            pv.and_then(|p| p.get(k))
                .and_then(Value::as_f64)
                .filter(|x| x.is_finite() && *x > 0.0)
                .unwrap_or(d)
        };
        let w = dim("width", 320.0) as f32;
        let scale = (MAX_TIP_W / w).min(1.0);
        let ppp = ui.ctx().pixels_per_point();
        let fonts = app.drill.fonts.get_or_insert_with(crate::pages::font_book);
        let img = crate::pages::render(
            &loaded.snap,
            &loaded.assets,
            fonts,
            Key::new(loaded.seq, index, scale * ppp).with_contrast(app.prefs.report_contrast),
        );
        if let Ok(img) = img {
            let ci = egui::ColorImage::from_rgba_premultiplied(
                [img.width as usize, img.height as usize],
                &img.rgba,
            );
            let tex = ui
                .ctx()
                .load_texture("bicraft-tooltip-page", ci, TextureOptions::LINEAR);
            // the image's own size, so that it is not resampled (blurred) when shown
            let size = Vec2::new(img.width as f32, img.height as f32) / ppp.max(1e-3);
            app.drill.tip_tex = Some((key, tex, size));
        }
    }
    if let Some((_, tex, size)) = &app.drill.tip_tex {
        let (id, size) = (tex.id(), *size);
        let _ = resp.clone().on_hover_ui_at_pointer(|ui| {
            let r = ui.add(egui::Image::new((id, size)));
            r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Unknown, true, "Report page tooltip"));
        });
    }
    true
}

/// Takes the snapshot of the hit under the pointer at a right click, for [`menu`].
pub fn remember_menu_hit(app: &mut App, hit: Option<bicraft_render::hits::Hit>) {
    app.drill.menu_hit = hit;
}

/// Whether the page shown has a tooltip page drawn (tests).
pub fn tooltip_drawn(app: &App) -> bool {
    app.drill.tip_tex.is_some()
}
