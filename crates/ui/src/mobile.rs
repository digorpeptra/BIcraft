//! The mobile layout view (docs/desktop.md, "Mobile layout"), as Power BI Desktop's View ›
//! Mobile layout: the page's phone canvas, portrait, 324 points wide and 3600 high with a grid of
//! 18 × 20 cells, as Power BI Desktop 2.159.1231.0 draws it (docs/gaps.md G72), and the
//! **Page visuals** pane in the Data pane's place, listing the page's visuals with their
//! appearance (shown or hidden) and whether they are on the phone canvas.
//!
//! - A visual dragged from the pane onto the phone canvas, or double-clicked there, is placed on
//!   it (`visual.mobile`), snapped to the grid, on top of the others.
//! - On the phone canvas the gestures are the report canvas's (`crate::canvas`): click and
//!   Ctrl+click select, a drag moves, the eight handles resize (Shift keeps the ratio), the
//!   arrows nudge, all snapped to the 18 × 20 grid and to the other visuals' edges (Alt: free);
//!   each is one `visual.mobile`, one undo step.
//! - Delete, the X at the selected visual's top right or its menu's Remove from mobile layout
//!   takes it off the phone canvas (`visual.mobile {layout: null}`); the visual stays on the
//!   page.
//! - The Format pane formats the selected visual for the phone only (`visual.mobile_format`):
//!   its values are the phone's (`visual.props {layout: "mobile"}`), and Clear mobile formatting
//!   puts it back to the visual's own.
//!
//! The phone canvas is drawn as focus mode draws its page ([`crate::focus`]): a derived snapshot,
//! the page 324 wide with only the visuals placed on it, at their phone positions and with their
//! phone format (`visuals[].mobile`, docs/view-snapshot.md), which the renderer draws as any page
//! and the canvas's helpers read while it is drawn.
//!
//! **Narrow windows** (View › Phone layout when narrow, a preference): a page with a mobile
//! layout is read on its phone canvas when the report area is narrower than [`NARROW`] points,
//! as the Power BI mobile apps show it in portrait (Microsoft Learn, "Mobile-optimized Power BI
//! reports overview"). Nothing can be edited there.
//!
//! Every edit is an engine command; the view, its mode and the pane are pure view state.

use std::sync::Arc;

use egui::{
    Align2, CornerRadius, DragAndDrop, Id, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};
use serde_json::{Map, Value, json};

use crate::App;
use crate::actions::Call;
use crate::app::{Tab as View, Then, clip};
use crate::canvas;
use crate::i18n::Lang;
use crate::icons::{self, Icon};
use crate::ribbon;
use crate::source::Loaded;
use crate::theme::{self, Tokens};
use crate::widgets;

/// The phone canvas's width (points): Power BI Desktop 2.159.1231.0's, 324 ([`bicraft_render::phone`]).
pub const PHONE_W: f32 = bicraft_render::phone::WIDTH as f32;
/// The phone canvas's height while it is edited: Desktop's, 3600 (more when visuals go further).
pub const EDIT_H: f32 = bicraft_render::phone::HEIGHT as f32;
/// The phone canvas's grid: Desktop's cells, 18 wide by 20 high.
pub const GRID: egui::Vec2 = egui::Vec2::new(
    bicraft_render::phone::GRID_X as f32,
    bicraft_render::phone::GRID_Y as f32,
);
/// A portrait phone screen (points): the least height of the phone layout read in a narrow
/// window, and the height the zoom fits while editing. BIcraft's choice (Desktop's visible
/// height is its window's).
pub const PHONE_H: f32 = 568.0;
/// Room left under the lowest visual while editing, to drop the next one.
const EDIT_ROOM: f32 = 160.0;
/// Under this width (points) of the report area, a page with a mobile layout is read on its
/// phone canvas when the preference is on.
pub const NARROW: f32 = 600.0;
/// Most visuals of a page considered.
const MAX_VISUALS: usize = 2000;
/// Height of a row of the Page visuals pane.
const ROW_H: f32 = 26.0;
/// The render cache's view keys of the phone canvas (never 0, the page, nor odd, focus mode).
const VIEW_EDIT: u64 = 2;
const VIEW_READ: u64 = 4;

/// The commands the mobile layout view runs (the coverage test reads it).
pub const COMMANDS: &[&str] = &["visual.mobile", "visual.mobile_format"];

/// The view's state, kept by the window.
#[derive(Default)]
pub struct MobileUi {
    /// View › Mobile layout is on.
    pub on: bool,
    /// The derived snapshot last built: (snapshot seq, page, editing) and the snapshot.
    built: Option<((u64, usize, bool), Loaded)>,
    /// The phone canvas drawn this frame (its render view key), else `None`.
    pub(crate) drawn: Option<u64>,
    /// The visual just placed on the phone canvas and selected, until a snapshot holds it there.
    placing: Option<String>,
}

/// Whether visual `visual` was just placed on the phone canvas (selected, its snapshot to come).
pub fn placing(app: &App, visual: &str) -> bool {
    drawn(app) && app.mobile.placing.as_deref() == Some(visual)
}

/// A visual dragged from the Page visuals pane: its name and its desktop size.
#[derive(Debug, Clone, PartialEq)]
pub struct Drag {
    pub visual: String,
    pub label: String,
    pub size: (f32, f32),
}

/// Whether the phone canvas is the one being edited (View › Mobile layout on the Report view,
/// with a session that may change the report).
pub fn editing(app: &App) -> bool {
    app.mobile.on
        && app.tab == View::Report
        && !crate::focus::active(app)
        && ribbon::state(app, "visual.mobile").is_ok()
}

/// Whether the page drawn now is the phone canvas.
pub fn drawn(app: &App) -> bool {
    app.mobile.drawn.is_some()
}

/// The view key of the phone canvas drawn now (render cache).
pub fn view_key(app: &App) -> Option<u64> {
    app.mobile.drawn
}

/// Whether page `page` of `snap` has visuals on its mobile layout.
fn has_layout(snap: &Value, page: usize) -> bool {
    snap.pointer(&format!("/report/pages/{page}/visuals"))
        .and_then(Value::as_array)
        .is_some_and(|a| {
            a.iter()
                .take(MAX_VISUALS)
                .any(|v| v.get("mobile").is_some_and(Value::is_object))
        })
}

/// The phone canvas to draw in a report area `width` points wide: the derived snapshot, or
/// `None` (the page itself). Sets what [`drawn`] answers.
pub fn loaded(app: &mut App, width: f32) -> Option<Loaded> {
    app.mobile.drawn = None;
    let l = app.loaded.as_ref()?;
    let page = app.active_page();
    let edit = editing(app);
    let shown = app.mobile.on || (app.prefs.mobile_narrow && width < NARROW && has_layout(&l.snap, page));
    if !shown || crate::focus::active(app) {
        return None;
    }
    let key = (l.seq, page, edit);
    let built = match &app.mobile.built {
        Some((k, b)) if *k == key => Some(b.clone()),
        _ => snapshot(&l.snap, page, edit).map(|s| Loaded {
            snap: Arc::new(s),
            assets: Arc::clone(&l.assets),
            seq: l.seq,
        }),
    };
    match &built {
        Some(b) => app.mobile.built = Some((key, b.clone())),
        None => app.mobile.built = None,
    }
    if built.is_some() {
        app.mobile.drawn = Some(if edit { VIEW_EDIT } else { VIEW_READ });
    }
    built
}

fn num(v: &Value, k: &str) -> Option<f64> {
    v.get(k).and_then(Value::as_f64).filter(|x| x.is_finite())
}

/// `snap` with page `page` as its phone canvas: [`PHONE_W`] wide, at least [`PHONE_H`] high
/// (more when its visuals go further, plus room to drop more while editing), holding only the
/// visuals placed on the mobile layout, at their phone place, with their phone format.
pub fn snapshot(snap: &Value, page: usize, editing: bool) -> Option<Value> {
    let p = snap.pointer(&format!("/report/pages/{page}"))?;
    let mut list = Vec::new();
    let mut bottom: f64 = 0.0;
    for v in p
        .get("visuals")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(MAX_VISUALS)
    {
        let Some(m) = v.get("mobile").filter(|m| m.is_object()) else {
            continue;
        };
        if !v.get("type").is_some_and(Value::is_string) {
            continue;
        }
        let (Some(x), Some(y), Some(w), Some(h)) =
            (num(m, "x"), num(m, "y"), num(m, "width"), num(m, "height"))
        else {
            continue;
        };
        let mut e = v.clone();
        let Some(o) = e.as_object_mut() else { continue };
        for (k, n) in [
            ("x", x),
            ("y", y),
            ("width", w),
            ("height", h),
            ("z", num(m, "z").unwrap_or(0.0)),
        ] {
            o.insert(k.into(), json!(n));
        }
        if let Some(f) = m.get("format") {
            o.insert("format".into(), f.clone());
        }
        if let Some(t) = m.get("title") {
            o.insert("title".into(), t.clone());
        }
        o.remove("mobile");
        // groups are the desktop layout's: on the phone every visual stands alone
        o.insert("parent_group".into(), Value::Null);
        bottom = bottom.max(y + h);
        list.push(e);
    }
    // edited: Desktop's 3600; read in a narrow window: a phone screen at least
    let (least, room) = if editing {
        (f64::from(EDIT_H), f64::from(EDIT_ROOM))
    } else {
        (f64::from(PHONE_H), 8.0)
    };
    let height = least.max((bottom + room).ceil()).min(20_000.0);
    let mut out = snap.clone();
    let slot = out
        .pointer_mut(&format!("/report/pages/{page}"))?
        .as_object_mut()?;
    slot.insert("width".into(), json!(PHONE_W));
    slot.insert("height".into(), json!(height));
    slot.insert("visuals".into(), Value::Array(list));
    // the phone canvas starts at the top
    slot.remove("vertical_alignment");
    Some(out)
}

/// The zoom of the phone canvas in a report area of `avail` points.
pub fn zoom(app: &App, avail: egui::Vec2) -> f32 {
    if editing(app) || app.mobile.on {
        ((avail.y - 48.0).max(100.0) / PHONE_H)
            .min((avail.x - 48.0).max(100.0) / PHONE_W)
            .clamp(0.5, 2.0)
    } else {
        // a narrow window reads it as a phone does: the full width
        ((avail.x - 16.0).max(50.0) / PHONE_W).clamp(0.3, 4.0)
    }
}

/// The phone's outline around the canvas `rect` (screen points).
pub fn bezel(ui: &Ui, rect: Rect) {
    let t = Tokens::get(ui.ctx());
    ui.painter().rect_stroke(
        rect.expand(6.0),
        CornerRadius::same(14),
        Stroke::new(2.0, t.border),
        StrokeKind::Outside,
    );
}

// ----- the View ribbon -----

/// The View tab's Mobile layout button and the Phone layout when narrow preference.
pub fn view_group(app: &mut App, ui: &mut Ui) {
    let l = app.lang;
    widgets::group(ui, |ui| {
        let on = app.mobile.on;
        let r = widgets::toggle_button(ui, Icon::Mobile, l.t("Mobile layout", "Disposition mobile"), true, on, true)
            .on_hover_text(l.t(
                "Lay out the page for phones: drag visuals from the Page visuals pane onto the phone canvas",
                "Disposer la page pour les téléphones : glissez les visuels du volet Visuels de la page sur le canevas du téléphone",
            ));
        if r.clicked() {
            toggle(app);
        }
        let narrow = app.prefs.mobile_narrow;
        let r = widgets::toggle_button(
            ui,
            Icon::Mobile,
            l.t("Phone layout when narrow", "Disposition mobile si étroit"),
            true,
            narrow,
            false,
        )
        .on_hover_text(l.t(
            "A page with a mobile layout is read on its phone canvas when the window is narrow",
            "Une page dotée d'une disposition mobile se lit sur son canevas de téléphone quand la fenêtre est étroite",
        ));
        if r.clicked() {
            app.prefs.mobile_narrow = !narrow;
            app.save_prefs();
        }
    });
}

/// View › Mobile layout: on or off (the Report view shows it).
pub fn toggle(app: &mut App) {
    app.mobile.on = !app.mobile.on;
    app.canvas.gesture = None;
    app.panes.format.own = None;
    app.panes.format.asked = None;
    if app.mobile.on && app.tab != View::Report {
        app.show_tab(View::Report);
    }
}

// ----- the phone canvas's input -----

/// The phone canvas's input while editing, in the report canvas's place
/// (`crate::menus::on_page`): drops from the Page visuals pane, the canvas's gestures, its
/// menu and the selected visual's X.
pub fn on_phone(app: &mut App, ui: &Ui, resp: &Response, rect: Rect, zoom: f32) {
    // a placed visual is on the phone canvas once its snapshot comes (or another is selected)
    if let Some(v) = app.mobile.placing.clone()
        && (app.cmd.visual.as_ref().is_none_or(|s| s.visual != v)
            || canvas::page_items(app).is_some_and(|(_, _, items)| items.iter().any(|i| i.name == v)))
    {
        app.mobile.placing = None;
    }
    drop_target(app, ui, resp, rect, zoom);
    canvas::pointer(app, ui, resp, rect, zoom);
    canvas::paint(app, ui, rect, zoom);
    crate::keyboard::context_menu(resp, |ui| {
        ui.set_min_width(200.0);
        menu(app, ui);
    });
    remove_button(app, ui, rect, zoom);
}

/// The selected visual's menu on the phone canvas.
fn menu(app: &mut App, ui: &mut Ui) {
    let l = app.lang;
    let has = app.cmd.visual.is_some();
    if ui
        .add_enabled(
            has,
            egui::Button::new(l.t("Remove from mobile layout", "Retirer de la disposition mobile")),
        )
        .clicked()
    {
        remove_selection(app);
        ui.close();
    }
    let formatted = has
        && app.cmd.visual.as_ref().is_some_and(|s| {
            snap_entry(app, &s.page, &s.visual)
                .and_then(|v| v.pointer("/mobile/formatted").and_then(Value::as_bool))
                == Some(true)
        });
    if ui
        .add_enabled(
            formatted,
            egui::Button::new(l.t("Clear mobile formatting", "Effacer la mise en forme mobile")),
        )
        .clicked()
    {
        clear_formatting(app);
        ui.close();
    }
}

/// The X at the selected visual's top right: removes it from the phone canvas.
fn remove_button(app: &mut App, ui: &Ui, rect: Rect, zoom: f32) {
    let Some(sel) = app.cmd.visual.clone() else { return };
    if !app.cmd.more.is_empty() || app.canvas.gesture.is_some() {
        return;
    }
    let Some(r) = crate::menus::visual_rect(app, &sel) else {
        return;
    };
    let s = Rect::from_min_size(rect.min + r.min.to_vec2() * zoom, r.size() * zoom);
    let l = app.lang;
    let at = pos2(s.right() - 22.0, s.top() + 2.0);
    let mut clicked = false;
    egui::Area::new(Id::unique("bicraft-mobile-remove"))
        .fixed_pos(at)
        .order(egui::Order::Foreground)
        .show(ui.ctx(), |ui| {
            // on the app's own surface, so the X reads on any page colour in either theme
            let t = Tokens::get(ui.ctx());
            egui::Frame::new()
                .fill(t.card)
                .corner_radius(CornerRadius::same(4))
                .show(ui, |ui| {
                    let tip = l.t("Remove from mobile layout", "Retirer de la disposition mobile");
                    clicked = widgets::icon_toggle(ui, Icon::Close, tip, true, false).clicked();
                });
        });
    if clicked {
        remove_selection(app);
    }
}

/// The page's snapshot entry of visual `visual` (the real snapshot, with its `mobile`).
fn snap_entry(app: &App, page: &str, visual: &str) -> Option<Value> {
    let snap = &app.loaded.as_ref()?.snap;
    snap.pointer("/report/pages")?
        .as_array()?
        .iter()
        .find(|p| p.get("name").and_then(Value::as_str) == Some(page))?
        .get("visuals")?
        .as_array()?
        .iter()
        .take(MAX_VISUALS)
        .find(|v| v.get("name").and_then(Value::as_str) == Some(visual))
        .cloned()
}

/// Runs `visual.mobile` with `params` as they are (the canvas's edits carry every value: no
/// form).
pub fn send(app: &mut App, params: Value) {
    app.run(
        Call {
            id: "visual.mobile",
            params,
        },
        Then::Nothing,
    );
}

/// Takes the selected visuals off the phone canvas (one `visual.mobile`).
pub fn remove_selection(app: &mut App) {
    let sel = canvas::selection(app);
    let Some(page) = app.cmd.visual.as_ref().map(|v| v.page.clone()) else {
        return;
    };
    if let Err(why) = ribbon::state(app, "visual.mobile") {
        app.status = why;
        return;
    }
    let rows: Vec<Value> = sel.iter().map(|v| json!({"visual": v, "layout": null})).collect();
    canvas::select(app, &page, &[]);
    send(app, json!({"page": page, "visuals": rows}));
}

/// Clears the selected visual's mobile formatting (`visual.mobile_format {clear: true}`).
pub fn clear_formatting(app: &mut App) {
    let Some(sel) = app.cmd.visual.clone() else { return };
    crate::format_pane::send_tracked(app, &sel, "visual.mobile_format", json!({"clear": true}));
}

/// The phone canvas's keys while editing (in [`crate::canvas::keys`]' place): Delete takes the
/// selection off it, Ctrl+A selects what is on it, the arrows nudge (Shift: 10 points).
pub fn keys(app: &mut App, ui: &Ui) {
    if app.canvas.gesture.is_some() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.canvas.gesture = None;
    }
    let focused = ui.ctx().memory(|m| m.focused().is_some());
    if focused || app.form.is_some() || app.palette.is_some() || app.confirm.is_some() {
        return;
    }
    // what the report canvas does with these keys does not apply to the phone canvas
    for (m, k) in [
        (egui::Modifiers::COMMAND, egui::Key::D),
        (egui::Modifiers::COMMAND, egui::Key::G),
    ] {
        ui.input_mut(|i| i.consume_key(m, k));
    }
    if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::A))
        && let Some(page) = app.page_names().get(app.active_page()).map(|p| p.0.clone())
    {
        let names: Vec<String> = rows(app, app.lang)
            .0
            .into_iter()
            .filter(|r| r.placed && !r.hidden)
            .map(|r| r.name)
            .collect();
        canvas::select(app, &page, &names);
    }
    if app.cmd.visual.is_none() || app.canvas.gesture.is_some() {
        return;
    }
    if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Delete)) {
        remove_selection(app);
        return;
    }
    let mut d = egui::Vec2::ZERO;
    for (key, v) in [
        (egui::Key::ArrowLeft, vec2(-1.0, 0.0)),
        (egui::Key::ArrowRight, vec2(1.0, 0.0)),
        (egui::Key::ArrowUp, vec2(0.0, -1.0)),
        (egui::Key::ArrowDown, vec2(0.0, 1.0)),
    ] {
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, key)) {
            d += v * canvas::NUDGE_BIG;
        }
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, key)) {
            d += v * canvas::NUDGE;
        }
    }
    if d != egui::Vec2::ZERO {
        nudge(app, d);
    }
}

/// Moves the selection on the phone canvas by `d` points (kept on it): one `visual.mobile`.
fn nudge(app: &mut App, d: egui::Vec2) {
    let Some(snap) = app.loaded.as_ref().map(|l| l.snap.clone()) else {
        return;
    };
    let page = app.active_page();
    let sel = canvas::selection(app);
    let Some(page_name) = app.cmd.visual.as_ref().map(|v| v.page.clone()) else {
        return;
    };
    let rows: Vec<Value> = snap
        .pointer(&format!("/report/pages/{page}/visuals"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(MAX_VISUALS)
        .filter(|v| {
            v.get("name")
                .and_then(Value::as_str)
                .is_some_and(|n| sel.iter().any(|s| s == n))
        })
        .filter_map(|v| {
            let m = v.get("mobile")?;
            let (x, y, w) = (num(m, "x")? as f32, num(m, "y")? as f32, num(m, "width")? as f32);
            let nx = (x + d.x).max(0.0).min((PHONE_W - w).max(0.0));
            let ny = (y + d.y).max(0.0);
            ((nx, ny) != (x, y)).then(|| json!({"visual": v.get("name"), "x": nx, "y": ny}))
        })
        .collect();
    if !rows.is_empty() {
        send(app, json!({"page": page_name, "visuals": rows}));
    }
}

// ----- drops from the Page visuals pane -----

/// Where a visual of desktop size `size` lands on the phone canvas when dropped at report point
/// `at` (its top-left corner): as wide as the phone at most, its ratio kept, on Desktop's
/// 18 × 20 grid ([`bicraft_render::phone::snap_rect`], the engine's `snap: true`).
pub fn drop_rect(at: Pos2, size: (f32, f32)) -> Rect {
    let (w0, h0) = (size.0.max(canvas::MIN_SIZE), size.1.max(canvas::MIN_SIZE));
    let w = w0.min(PHONE_W);
    let h = (h0 * w / w0).max(canvas::MIN_SIZE);
    let (x, y, w, h) =
        bicraft_render::phone::snap_rect(f64::from(at.x), f64::from(at.y), f64::from(w), f64::from(h));
    Rect::from_min_size(pos2(x as f32, y as f32), vec2(w as f32, h as f32))
}

/// The Page visuals pane's drag over the phone canvas: a ghost where it goes, placed on release.
fn drop_target(app: &mut App, ui: &Ui, resp: &Response, rect: Rect, zoom: f32) {
    let Some(d) = DragAndDrop::payload::<Drag>(ui.ctx()).map(|d| (*d).clone()) else {
        return;
    };
    if !resp.contains_pointer() || zoom <= 0.0 {
        return;
    }
    let Some(pos) = ui.input(|i| i.pointer.hover_pos().or(i.pointer.interact_pos())) else {
        return;
    };
    let at = pos2((pos.x - rect.min.x) / zoom, (pos.y - rect.min.y) / zoom);
    let r = drop_rect(at, d.size);
    let t = Tokens::of(app.prefs.theme);
    let s = Rect::from_min_size(rect.min + r.min.to_vec2() * zoom, r.size() * zoom);
    ui.painter().rect_stroke(
        s,
        CornerRadius::same(2),
        Stroke::new(1.5, t.ring),
        StrokeKind::Middle,
    );
    if ui.input(|i| i.pointer.any_released()) && DragAndDrop::take_payload::<Drag>(ui.ctx()).is_some() {
        place(app, &d.visual, r);
    }
}

/// Places visual `visual` of the page shown on the phone canvas at `r` (`visual.mobile`), and
/// selects it.
fn place(app: &mut App, visual: &str, r: Rect) {
    if let Err(why) = ribbon::state(app, "visual.mobile") {
        app.status = format!("{} {why}", app.lang.t("Unavailable:", "Indisponible :"));
        return;
    }
    let Some(page) = app.page_names().get(app.active_page()).map(|p| p.0.clone()) else {
        return;
    };
    canvas::select(app, &page, &[visual.to_string()]);
    app.mobile.placing = Some(visual.to_string());
    send(
        app,
        json!({"page": page, "visual": visual, "x": r.min.x, "y": r.min.y,
               "width": r.width(), "height": r.height()}),
    );
}

// ----- the Page visuals pane -----

/// One row of the Page visuals pane.
struct Row {
    name: String,
    label: String,
    hidden: bool,
    placed: bool,
    size: (f32, f32),
}

/// The page's visuals (no groups), front to back, and the bottom of the phone layout.
fn rows(app: &App, l: Lang) -> (Vec<Row>, f32) {
    let Some(snap) = app.loaded.as_ref().map(|l| l.snap.clone()) else {
        return (Vec::new(), 0.0);
    };
    let mut list: Vec<(f64, Row)> = Vec::new();
    let mut bottom: f32 = 0.0;
    for v in snap
        .pointer(&format!("/report/pages/{}/visuals", app.active_page()))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(MAX_VISUALS)
    {
        let Some(ty) = v.get("type").and_then(Value::as_str) else {
            continue;
        };
        let Some(name) = v.get("name").and_then(Value::as_str) else {
            continue;
        };
        let m = v.get("mobile").filter(|m| m.is_object());
        if let Some(m) = m
            && let (Some(y), Some(h)) = (num(m, "y"), num(m, "height"))
        {
            bottom = bottom.max((y + h) as f32);
        }
        let label = v
            .get("title")
            .and_then(Value::as_str)
            .filter(|t| !t.trim().is_empty())
            .map(str::to_string)
            .or_else(|| crate::panes::type_label(ty, l).map(str::to_string))
            .unwrap_or_else(|| ty.to_string());
        list.push((
            num(v, "z").unwrap_or(0.0),
            Row {
                name: name.to_string(),
                label,
                hidden: v.get("hidden").and_then(Value::as_bool) == Some(true),
                placed: m.is_some(),
                size: (
                    num(v, "width").unwrap_or(300.0) as f32,
                    num(v, "height").unwrap_or(200.0) as f32,
                ),
            },
        ));
    }
    list.sort_by(|a, b| b.0.total_cmp(&a.0));
    (list.into_iter().map(|(_, r)| r).collect(), bottom)
}

/// The Page visuals pane (the Data pane's place while the mobile layout is edited).
pub fn pane(app: &mut App, ui: &mut Ui) {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    if crate::panes::title_row(
        ui,
        l.t("Page visuals", "Visuels de la page"),
        l.t(
            "Collapse the Page visuals pane",
            "Réduire le volet Visuels de la page",
        ),
    ) {
        app.prefs.panes.data = false;
        app.save_prefs();
        return;
    }
    ui.label(
        egui::RichText::new(l.t(
            "Drag a visual onto the phone canvas, or double-click it.",
            "Glissez un visuel sur le canevas du téléphone, ou double-cliquez dessus.",
        ))
        .color(t.text_dim),
    );
    ui.add_space(6.0);
    let can = ribbon::state(app, "visual.mobile");
    let (list, bottom) = rows(app, l);
    if list.is_empty() {
        ui.label(
            egui::RichText::new(l.t("This page has no visuals.", "Cette page n'a pas de visuel."))
                .color(t.text_dim),
        );
        return;
    }
    let sel = canvas::selection(app);
    let mut add: Option<(String, (f32, f32))> = None;
    let mut pick: Option<String> = None;
    egui::ScrollArea::vertical()
        .id_salt("bicraft-page-visuals")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for r in &list {
                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::hover());
                let sense = if can.is_ok() {
                    Sense::click_and_drag()
                } else {
                    Sense::click()
                };
                let resp = ui.interact(rect, Id::unique(("bicraft-page-visual", &r.name)), sense);
                let on = sel.contains(&r.name);
                let p = ui.painter();
                if on {
                    p.rect_filled(rect, CornerRadius::same(4), t.accent_soft);
                } else if resp.hovered() {
                    p.rect_filled(rect, CornerRadius::same(4), t.hover);
                }
                let ink = if r.hidden { t.text_dim } else { t.text };
                // the appearance: an eye (shown) or a struck eye (hidden), with its word
                let eye = Rect::from_center_size(pos2(rect.min.x + 12.0, rect.center().y), vec2(16.0, 16.0));
                icons::paint(
                    p,
                    eye,
                    if r.hidden { Icon::EyeOff } else { Icon::Eye },
                    ink,
                    false,
                );
                // placed on the phone canvas: a check and a word, not colour alone
                let state = if r.placed {
                    l.t("On phone", "Sur mobile")
                } else {
                    ""
                };
                // the name, cut to leave room for the state
                let state_w = if r.placed {
                    p.layout_no_wrap(state.to_string(), theme::font(11.0), t.text_dim)
                        .size()
                        .x
                        + 30.0
                } else {
                    8.0
                };
                let room = (rect.width() - 26.0 - state_w).max(20.0);
                let mut chars = r.label.chars().count().min(60);
                let mut shown = clip(&r.label, chars);
                while chars > 4 && p.layout_no_wrap(shown.clone(), theme::font(13.0), ink).size().x > room {
                    chars -= 2;
                    shown = clip(&r.label, chars);
                }
                p.text(
                    pos2(rect.min.x + 26.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    shown,
                    theme::font(13.0),
                    ink,
                );
                if r.placed {
                    let c =
                        Rect::from_center_size(pos2(rect.max.x - 10.0, rect.center().y), vec2(14.0, 14.0));
                    icons::paint(p, c, Icon::Check, t.text_dim, false);
                    p.text(
                        pos2(rect.max.x - 20.0, rect.center().y),
                        Align2::RIGHT_CENTER,
                        state,
                        theme::font(11.0),
                        t.text_dim,
                    );
                }
                let appearance = if r.hidden {
                    l.t("hidden", "masqué")
                } else {
                    l.t("shown", "affiché")
                };
                let a11y = if r.placed {
                    format!("{} ({appearance}, {state})", r.label)
                } else {
                    format!("{} ({appearance})", r.label)
                };
                resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, &a11y));
                if resp.drag_started() && can.is_ok() {
                    DragAndDrop::set_payload(
                        ui.ctx(),
                        Drag {
                            visual: r.name.clone(),
                            label: r.label.clone(),
                            size: r.size,
                        },
                    );
                }
                if resp.double_clicked() && can.is_ok() && !r.placed {
                    add = Some((r.name.clone(), r.size));
                } else if resp.clicked() && r.placed {
                    pick = Some(r.name.clone());
                }
                if let Err(why) = &can {
                    resp.on_hover_text(format!("{} {why}", l.t("Read-only:", "Lecture seule :")));
                }
            }
        });
    // the dragged visual's name follows the pointer
    if let Some(d) = DragAndDrop::payload::<Drag>(ui.ctx())
        && let Some(pos) = ui.ctx().input(|i| i.pointer.hover_pos())
    {
        let painter = ui.ctx().layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            Id::unique("bicraft-mobile-drag-chip"),
        ));
        let galley = painter.layout_no_wrap(clip(&d.label, 40), theme::font(12.0), t.text);
        let r = Rect::from_min_size(pos + vec2(12.0, 10.0), galley.size() + vec2(16.0, 8.0));
        painter.rect_filled(r, CornerRadius::same(4), t.card);
        painter.rect_stroke(
            r,
            CornerRadius::same(4),
            Stroke::new(1.0, t.ring),
            StrokeKind::Inside,
        );
        painter.galley(r.min + vec2(8.0, 4.0), galley, t.text);
    }
    let page = app.page_names().get(app.active_page()).map(|p| p.0.clone());
    if let (Some((name, size)), Some(_)) = (add, &page) {
        // a double click puts it under what is on the phone canvas already
        let y = if bottom > 0.0 { bottom + GRID.y } else { 0.0 };
        place(app, &name, drop_rect(pos2(0.0, y), size));
    }
    if let (Some(name), Some(page)) = (pick, page) {
        canvas::select(app, &page, &[name]);
    }
}

/// A visual's snapshot entry `v` as the phone layout draws it (its phone format and title), or
/// `None` when it is not on the phone canvas.
pub fn phone_entry(v: &Value) -> Option<Value> {
    let m = v.get("mobile").filter(|m| m.is_object())?;
    let mut e = v.clone();
    let o = e.as_object_mut()?;
    for k in ["format", "title"] {
        if let Some(x) = m.get(k) {
            o.insert(k.into(), x.clone());
        }
    }
    Some(e)
}

/// The mobile Format pane's note: what the Format pane edits now, and Clear mobile formatting
/// when the selected visual has some.
pub fn format_note(app: &mut App, ui: &mut Ui, formatted: bool) {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    ui.label(
        egui::RichText::new(l.t(
            "Mobile layout: these settings apply to the phone only.",
            "Disposition mobile : ces réglages ne s'appliquent qu'au téléphone.",
        ))
        .color(t.text_dim),
    );
    if formatted
        && ui
            .link(l.t("Clear mobile formatting", "Effacer la mise en forme mobile"))
            .clicked()
    {
        clear_formatting(app);
    }
    ui.add_space(4.0);
}

/// The Format pane's edit `(id, params)` as the phone layout takes it: `visual.mobile_format`
/// with the same settings (titles, axes and tooltips folded in), or `None` for an edit the
/// mobile layout has no place for.
pub fn format_edit(id: &str, params: &Value) -> Option<Value> {
    let o = params.as_object()?;
    let mut out = Map::new();
    match id {
        "visual.format" | "visual.mobile_format" => {
            out.extend(o.clone());
        }
        // one series item (`target`…) goes in the series list
        "visual.series" if o.contains_key("target") => {
            let mut item = o.clone();
            for k in ["page", "visual"] {
                item.shift_remove(k);
            }
            out.insert("series".into(), json!([item]));
        }
        "visual.series" => out.extend(o.clone()),
        "visual.set_title" => {
            out.insert("title".into(), o.get("title").cloned().unwrap_or(Value::Null));
        }
        "visual.tooltip_page" => {
            out.insert(
                "tooltip_page".into(),
                o.get("tooltip_page").cloned().unwrap_or(Value::Null),
            );
        }
        "visual.axis" => {
            let axis = o.get("axis")?.as_str()?.to_string();
            let mut body = o.clone();
            body.shift_remove("axis");
            let clear = body.shift_remove("clear").and_then(|c| c.as_bool()) == Some(true);
            let spec = if clear { Value::Null } else { Value::Object(body) };
            out.insert("axes".into(), json!({ axis: spec }));
        }
        _ => return None,
    }
    for k in ["page", "visual"] {
        out.shift_remove(k);
    }
    Some(Value::Object(out))
}
