//! Keyboard focus on the report canvas's visuals (docs/desktop.md "Keyboard navigation and
//! screen readers"): each visual of the page shown is a focusable widget, visited in the
//! report's tab order (`tabOrder`, the Selection pane's Tab order view), then in reading order
//! (top to bottom, left to right). Tab and Shift+Tab move between them (egui's own focus); the
//! focused visual is selected, as a click selects it, so the canvas keys act on it: the arrows
//! move it (Shift: 10 px, `crate::canvas::keys`), Ctrl+Right enters its data points
//! (`crate::point_keys`), Shift+F10 or the Menu key opens its right-click menu, Delete removes
//! it, Ctrl+C copies it. Escape gives the focus back to the page. Each visual's node in the
//! accessibility tree is a figure named after its title (else its automatic title, else its
//! type), selected or not, its place in the order, and read-only when the session cannot move
//! it. View state only: the selection is the click's, a move is `visual.move`.

use std::collections::HashMap;

use egui::accesskit::Role;
use egui::{Context, EventFilter, Id, Rect, Sense, Ui, WidgetInfo};
use serde_json::Value;

use crate::App;

/// The widget id of visual `name` on the canvas.
pub fn id(name: &str) -> Id {
    Id::unique("bicraft-canvas-visual").with(name)
}

/// A visual the keyboard visits.
#[derive(Debug, Clone)]
pub struct Stop {
    pub name: String,
    /// Its accessible name: the title first.
    pub label: String,
    /// Its alt text, when set.
    pub alt: Option<String>,
    /// Report pixels.
    pub rect: Rect,
}

/// The page shown and its visuals in keyboard order: those with a `tabOrder` (0 or more) by it,
/// then the others top to bottom, left to right. Hidden visuals and groups are left out.
pub fn order(app: &App) -> Option<(String, Vec<Stop>)> {
    let (page, _, items) = crate::canvas::page_items(app)?;
    let l = app.lang;
    let snap = &app.loaded.as_ref()?.snap;
    let visuals: HashMap<&str, &Value> = snap
        .pointer(&format!("/report/pages/{}/visuals", app.active_page()))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| Some((v.get("name")?.as_str()?, v)))
        .collect();
    let text = |v: Option<&&Value>, k: &str| {
        v.and_then(|v| v.get(k))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
    };
    let mut stops: Vec<(Option<i64>, Stop)> = items
        .iter()
        .filter(|i| i.pickable)
        .map(|i| {
            let v = visuals.get(i.name.as_str());
            let ty = v.and_then(|v| v.get("type")).and_then(Value::as_str);
            let kind = ty
                .and_then(|t| crate::panes::type_label(t, l))
                .map(str::to_string)
                .or_else(|| ty.map(str::to_string));
            let title = text(v, "title").or_else(|| text(v, "auto_title"));
            let label = match (title, kind) {
                (Some(t), Some(k)) => format!("{t}, {k}"),
                (Some(t), None) => t,
                (None, Some(k)) => k,
                (None, None) => i.name.clone(),
            };
            let tab = v
                .and_then(|v| v.get("tab_order"))
                .and_then(Value::as_i64)
                .filter(|t| *t >= 0);
            (
                tab,
                Stop {
                    name: i.name.clone(),
                    label,
                    alt: text(v, "alt_text"),
                    rect: i.rect,
                },
            )
        })
        .collect();
    stops.sort_by(|(ta, a), (tb, b)| {
        tb.is_some()
            .cmp(&ta.is_some())
            .then(ta.cmp(tb))
            .then(a.rect.min.y.total_cmp(&b.rect.min.y))
            .then(a.rect.min.x.total_cmp(&b.rect.min.x))
    });
    Some((page, stops.into_iter().map(|(_, s)| s).collect()))
}

/// The canvas visual that has the keyboard focus (drawn in the last frame).
pub fn focused(app: &App, ctx: &Context) -> Option<String> {
    let f = ctx.memory(|m| m.focused())?;
    app.keyboard
        .prev_visuals
        .iter()
        .chain(&app.keyboard.visuals)
        .find(|(id, _)| *id == f)
        .map(|(_, n)| n.clone())
}

/// Whether widget `id` is one of the canvas's visuals (drawn in the last frame or this one).
pub fn is_visual(app: &App, id: Id) -> bool {
    app.keyboard
        .prev_visuals
        .iter()
        .chain(&app.keyboard.visuals)
        .any(|(v, _)| *v == id)
}

/// The visual the keyboard enters the canvas on: the selected one, else the first in order.
pub fn entry(app: &App) -> Option<Id> {
    let sel = app.cmd.visual.as_ref().map(|v| v.visual.as_str());
    let drawn = &app.keyboard.visuals;
    drawn
        .iter()
        .find(|(_, n)| Some(n.as_str()) == sel)
        .or(drawn.first())
        .map(|(id, _)| *id)
}

/// Draws the visuals' focusable widgets over page `page` shown at `zoom` (in keyboard order,
/// after the page's own widget, so Tab goes from the page to the first visual).
pub fn draw(app: &mut App, ui: &Ui, page: Rect, zoom: f32) {
    let Some((page_name, stops)) = order(app) else {
        return;
    };
    let ctx = ui.ctx().clone();
    let sel = crate::canvas::selection(app);
    let read_only = !crate::canvas::editable(app);
    let n = stops.len();
    for (k, s) in stops.iter().enumerate() {
        let r = crate::canvas::to_screen(page, zoom, s.rect);
        if !r.is_positive() {
            continue;
        }
        let wid = id(&s.name);
        // focusable only: the page under it keeps the pointer's clicks and drags
        let resp = ui.interact(r, wid, Sense::focusable_noninteractive());
        let mut selected = sel.contains(&s.name);
        if resp.has_focus() {
            // the arrows move the visual and Escape leaves it: egui keeps the focus
            ctx.memory_mut(|m| {
                m.set_focus_lock_filter(
                    wid,
                    EventFilter {
                        tab: false,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape: true,
                    },
                );
            });
            // the focused visual is the selection (a menu's Escape or a pane may have cleared
            // it); Ctrl+A keeps the others with it
            if (resp.gained_focus() && sel.len() != 1) || !selected {
                crate::canvas::select(app, &page_name, std::slice::from_ref(&s.name));
                selected = true;
            }
        }
        resp.widget_info(|| WidgetInfo::selected(Role::Figure, true, selected, &s.label));
        ctx.accesskit_node_builder(wid, |node| {
            node.set_selected(selected);
            node.set_position_in_set(k + 1);
            node.set_size_of_set(n);
            if read_only {
                node.set_read_only();
            }
            if let Some(alt) = &s.alt {
                node.set_description(alt.clone());
            }
        });
        app.keyboard.visuals.push((wid, s.name.clone()));
    }
    // the focused visual went away (removed, cut, hidden): the focus goes to the page
    if let Some(f) = ctx.memory(|m| m.focused())
        && app.keyboard.prev_visuals.iter().any(|(id, _)| *id == f)
        && !app.keyboard.visuals.iter().any(|(id, _)| *id == f)
        && let Some(p) = app.keyboard.page
    {
        ctx.memory_mut(|m| m.request_focus(p));
    }
}

/// Escape on a focused visual: the focus goes back to the page and the selection is cleared,
/// as Escape clears it on the page.
pub fn leave(app: &mut App, ctx: &Context) {
    if let Some(p) = app.keyboard.page_prev {
        ctx.memory_mut(|m| m.request_focus(p));
    }
    app.cmd.visual = None;
    app.cmd.more.clear();
}

/// The report page's accessible name: "Report page: <its name>".
pub fn page_label(app: &App) -> String {
    let l = app.lang;
    let active = app.active_page();
    let shown = app
        .page_names()
        .into_iter()
        .nth(active)
        .map(|(_, label, _)| label)
        .unwrap_or_default();
    l.t("Report page: {}", "Page du rapport : {}")
        .replace("{}", &shown)
}
