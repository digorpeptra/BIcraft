//! The right-click menus of the registry's menu-placed commands (ROADMAP M11): a visual on the
//! canvas, and the model diagram's tables, columns, measures, relationships and background. The
//! page tab's menu is in `page_tabs.rs` ([`crate::page_tabs::MENU`]). Each item runs its command
//! through [`crate::ribbon::invoke`] with the object clicked as parameters, greyed with the
//! reason when the session cannot run it.
//!
//! Also the visual selection: a click on the page selects the topmost visual under the pointer
//! (the Visual tab appears), a click on nothing clears it, Escape too.

use std::collections::HashMap;

use egui::{Rect, Response, Ui};
use serde_json::Value;

use crate::App;
use crate::i18n::Lang;
use crate::model_view::Sel;
use crate::registry;
use crate::ribbon::{self, VisualSel};

/// The visual's menu.
pub const VISUAL: &[&str] = &[
    "visual.refresh",
    "visual.export_data",
    "visual.records",
    "visual.analyze",
    "visual.explain_anomaly",
    "visual.group_points",
    "textbox.set",
    "visual.cut",
    "visual.copy",
    "visual.paste",
    "visual.duplicate",
    "visual.remove",
    "visual.set_sort",
    "visual.calculation.add",
    "visual.rename_group",
    "view.keep_drill",
    "visual.expand",
];
/// The empty page's menu.
pub const EMPTY: &[&str] = &["visual.paste"];
/// A table card's menu (a calculation group is a table).
pub const TABLE: &[&str] = &[
    "model.refresh",
    "table.rename",
    "table.entered_data",
    "table.set_refresh_policy",
    "table.remove",
    "hierarchy.remove",
    "calculation_group.remove",
    "calculation_item.remove",
];
/// A column row's menu.
pub const COLUMN: &[&str] = &["column.rename", "column.remove"];
/// A measure row's menu.
pub const MEASURE: &[&str] = &["measure.rename", "measure.remove"];
/// A relationship line's menu.
pub const RELATIONSHIP: &[&str] = &["relationship.remove"];
/// The diagram background's menu: the model's objects the diagram does not draw.
pub const MODEL: &[&str] = &[
    "expression.rename",
    "expression.remove",
    "role.remove",
    "culture.remove",
];
/// A layout tab's menu (under the model diagram).
pub const LAYOUT_TAB: &[&str] = &["diagram.rename", "diagram.remove", "diagram.add"];

/// Every command id the context menus run (the coverage test reads it).
pub fn ids() -> Vec<&'static str> {
    [
        VISUAL,
        TABLE,
        COLUMN,
        MEASURE,
        RELATIONSHIP,
        MODEL,
        LAYOUT_TAB,
        crate::model_explorer::COMMANDS,
        crate::page_tabs::MENU,
    ]
    .concat()
}

/// The model diagram menu's items for the selection (`None`: the background).
pub fn model_items(sel: Option<&Sel>) -> &'static [&'static str] {
    match sel {
        Some(Sel::Table(_)) => TABLE,
        Some(Sel::Column { .. }) => COLUMN,
        Some(Sel::Measure { .. }) => MEASURE,
        Some(Sel::Rel(_)) => RELATIONSHIP,
        Some(Sel::Item(_)) => MODEL,
        None => MODEL,
    }
}

/// The topmost visual at report position (`x`, `y`) of the page shown (groups and hidden visuals
/// are not picked).
pub fn visual_at(app: &App, x: f32, y: f32) -> Option<VisualSel> {
    let snap = &app.loaded.as_ref()?.snap;
    let page = snap.pointer(&format!("/report/pages/{}", app.active_page()))?;
    let page_name = page.get("name")?.as_str()?.to_string();
    let num = |v: &Value, k: &str| v.get(k).and_then(Value::as_f64).filter(|x| x.is_finite());
    page.get("visuals")?
        .as_array()?
        .iter()
        .take(2000)
        .filter(|v| v.get("type").is_some_and(Value::is_string))
        .filter(|v| v.get("hidden").and_then(Value::as_bool) != Some(true))
        .filter(|v| {
            let (Some(vx), Some(vy), Some(w), Some(h)) =
                (num(v, "x"), num(v, "y"), num(v, "width"), num(v, "height"))
            else {
                return false;
            };
            let (x, y) = (f64::from(x), f64::from(y));
            x >= vx && y >= vy && x <= vx + w && y <= vy + h
        })
        .max_by(|a, b| num(a, "z").unwrap_or(0.0).total_cmp(&num(b, "z").unwrap_or(0.0)))
        .and_then(|v| v.get("name")?.as_str())
        .map(|v| VisualSel {
            page: page_name,
            visual: v.to_string(),
        })
}

/// The selected visual's rectangle in report pixels (`None` when it is not on the page shown).
pub(crate) fn visual_rect(app: &App, sel: &VisualSel) -> Option<Rect> {
    let snap = &app.loaded.as_ref()?.snap;
    let page = snap.pointer(&format!("/report/pages/{}", app.active_page()))?;
    if page.get("name")?.as_str()? != sel.page {
        return None;
    }
    let v = page
        .get("visuals")?
        .as_array()?
        .iter()
        .find(|v| v.get("name").and_then(Value::as_str) == Some(sel.visual.as_str()))?;
    let n = |k: &str| v.get(k).and_then(Value::as_f64).filter(|x| x.is_finite());
    let (x, y, w, h) = (n("x")?, n("y")?, n("width")?, n("height")?);
    Some(Rect::from_min_size(
        egui::pos2(x as f32, y as f32),
        egui::vec2(w as f32, h as f32),
    ))
}

/// The page's selection and its menu: a primary or secondary click selects the visual under the
/// pointer (or clears the selection), Ctrl+click adds or removes one, a drag moves, resizes or
/// selects with a rubber band (`crate::canvas`); the selected ones get the `ring` outline, a
/// right click opens their menu (the empty page's: Paste).
pub fn on_page(app: &mut App, ui: &Ui, resp: &Response, rect: Rect, zoom: f32) {
    crate::keyboard::canvas_page(app, resp);
    // the phone canvas has gestures and a menu of its own (crate::mobile); read on a narrow
    // window, it is not edited
    if crate::mobile::drawn(app) {
        if crate::mobile::editing(app) {
            crate::mobile::on_phone(app, ui, resp, rect, zoom);
        }
        return;
    }
    // focus mode reads one visual: no drop, move, resize or rubber band (crate::focus)
    let focus = crate::focus::active(app);
    if !focus {
        // each visual takes the keyboard focus, after the page in Tab order
        crate::visual_focus::draw(app, ui, rect, zoom);
        // a field dragged from the Data pane onto a visual
        crate::panes::canvas(app, ui, resp, rect, zoom);
        crate::canvas::pointer(app, ui, resp, rect, zoom);
    }
    // Shift+F10 on the page opens the menu at the selected visual (crate::keyboard)
    let secondary = resp
        .secondary_clicked()
        .then(|| resp.interact_pointer_pos())
        .flatten()
        .or_else(|| crate::keyboard::menu_key(resp));
    if let Some(pos) = secondary {
        let hit = app.hit_at_screen(pos);
        crate::drill::remember_menu_hit(app, hit);
        // the menu's Paste puts the clip at the pointer
        app.canvas.menu_at = (zoom > 0.0)
            .then(|| egui::pos2((pos.x - rect.min.x) / zoom, (pos.y - rect.min.y) / zoom))
            .filter(|p| p.x.is_finite() && p.y.is_finite());
        // the data point under the right click, for See records
        (app.cmd.point, app.cmd.points) = match crate::data_view::point_at(app, pos) {
            Some((point, points)) => (Some(point), points),
            None => (None, None),
        };
    }
    // a selection on another page (the page changed) is dropped
    if let Some(sel) = app.cmd.visual.clone()
        && visual_rect(app, &sel).is_none()
    {
        app.cmd.visual = None;
        app.cmd.more.clear();
    }
    if !focus {
        crate::canvas::paint(app, ui, rect, zoom);
        // the Edit interactions mode's icon sets, over the targets
        crate::interactions::overlay(app, ui, rect, zoom);
    }
    let ids = if app.cmd.visual.is_some() { VISUAL } else { EMPTY };
    crate::keyboard::context_menu(resp, |ui| {
        ui.set_min_width(180.0);
        // the reader's drill, expand / collapse and drillthrough entries (drill.rs), Show as a
        // table and Sort axis (focus.rs)
        if let Some(v) = app.cmd.visual.as_ref().map(|s| s.visual.clone()) {
            crate::drill::menu(app, ui, &v);
            crate::focus::menu(app, ui, &v);
        }
        // Edit text only on a text box, Expand or collapse only on a matrix
        let vtype = |want: &str| {
            app.cmd.visual.as_ref().is_some_and(|s| {
                crate::rich::snap_visual(app, &s.page, &s.visual).and_then(|v| {
                    v.get("type")
                        .and_then(Value::as_str)
                        .map(|t| t.eq_ignore_ascii_case(want))
                }) == Some(true)
            })
        };
        let textbox = vtype("textbox");
        let matrix = vtype("pivotTable");
        let point = app.cmd.point.is_some();
        // Explain the anomaly only on a point the line's anomaly detection flags
        let anomaly = app.cmd.visual.as_ref().is_some_and(|s| {
            crate::rich::snap_visual(app, &s.page, &s.visual)
                .is_some_and(|v| app.cmd.point.as_ref().is_some_and(|p| is_anomaly(&v, p)))
        });
        let paused = crate::optimize_ui::paused(app);
        let ids: Vec<&'static str> = ids
            .iter()
            .copied()
            // Refresh visual only while the report's visual queries are paused
            .filter(|id| *id != crate::optimize_ui::REFRESH_ONE || paused)
            .filter(|id| *id != "textbox.set" || textbox)
            .filter(|id| *id != "visual.expand" || matrix)
            // Analyze only on a data point
            .filter(|id| *id != "visual.analyze" || point)
            .filter(|id| *id != "visual.explain_anomaly" || anomaly)
            // Group data on a data point (the selection, else the point right-clicked)
            .filter(|id| *id != "visual.group_points" || point)
            .collect();
        run_items_at(app, ui, &ids);
    });
    if app.cmd.visual.is_some() {
        header(app, ui, rect, zoom);
    }
    crate::export_dialog::show(app, ui.ctx());
}

/// Runs the item of `ids` clicked in a menu as its button does (Export data opens its dialog).
fn run_items(app: &mut App, ui: &mut Ui, ids: &[&'static str]) {
    if let Some(id) = items(app, ui, ids)
        && let Some(c) = registry::find(id)
    {
        let p = ribbon::prefill(app, c, None);
        ribbon::invoke(app, id, p);
    }
}

/// [`run_items`] for the page's right-click menu: its Paste goes to the pointer.
fn run_items_at(app: &mut App, ui: &mut Ui, ids: &[&'static str]) {
    if let Some(id) = items(app, ui, ids)
        && let Some(c) = registry::find(id)
    {
        app.canvas.paste_at = if id == "visual.paste" {
            app.canvas.menu_at
        } else {
            None
        };
        let p = ribbon::prefill(app, c, None);
        ribbon::invoke(app, id, p);
    }
}

/// The visual's menu of the header's More options: Show as a table and Sort axis, then its
/// commands.
fn visual_menu(app: &mut App, ui: &mut Ui, visual: &str) {
    ui.set_min_width(180.0);
    crate::focus::menu(app, ui, visual);
    let paused = crate::optimize_ui::paused(app);
    let ids: Vec<&'static str> = VISUAL
        .iter()
        .copied()
        .filter(|id| *id != crate::optimize_ui::REFRESH_ONE || paused)
        .collect();
    run_items(app, ui, &ids);
}

/// The selected visual's header: a focus mode button and a More options button (⋯) at its top
/// right, opening the visual's menu, as Power BI's visual header does.
fn header(app: &mut App, ui: &Ui, rect: Rect, zoom: f32) {
    // the conditional-formatting dialog is open over the canvas: the header (a foreground
    // layer) would be drawn over it
    if app.panes.format.cf.is_some() {
        return;
    }
    let Some(sel) = app.cmd.visual.clone() else { return };
    let Some(r) = visual_rect(app, &sel) else { return };
    let screen = Rect::from_min_size(rect.min + r.min.to_vec2() * zoom, r.size() * zoom);
    // above the visual's top right; a visual at the page's top keeps it inside its top right, as
    // Power BI's header sits, clear of the title at its top left and of the drill icons
    // (drill.rs) right-aligned there on a drillable visual. A focused visual fills the page: its
    // header goes above the page when there is room, clear of its title.
    let top = if crate::focus::active(app) {
        ui.clip_rect().top().min(rect.min.y)
    } else {
        rect.min.y
    };
    // inside the visual, the header keeps to More options alone (it covers the least of the
    // visual); its menu has Focus mode too
    let above = screen.top() - 24.0 >= top;
    let (at, pivot) = if above {
        (
            egui::pos2(screen.right() - 54.0, screen.top() - 24.0),
            egui::Align2::LEFT_TOP,
        )
    } else {
        let drillable = crate::rich::snap_visual(app, &sel.page, &sel.visual)
            .is_some_and(|v| v.get("drill").is_some_and(Value::is_object));
        let right = if drillable {
            crate::drill::icon_rects(screen)
                .iter()
                .map(|(_, r)| r.left())
                .fold(screen.right(), f32::min)
                - 2.0
        } else {
            screen.right() - 2.0
        };
        (
            egui::pos2(right.max(rect.min.x), screen.top() + 2.0),
            egui::Align2::RIGHT_TOP,
        )
    };
    let at = egui::pos2(at.x.max(rect.min.x), at.y.max(top));
    let l = app.lang;
    egui::Area::new(egui::Id::unique("bicraft-visual-header"))
        .pivot(pivot)
        .fixed_pos(at)
        .order(egui::Order::Foreground)
        .show(ui.ctx(), |ui| {
            // a card under the buttons: inside the visual they sit over its title and marks
            let t = crate::theme::Tokens::get(ui.ctx());
            egui::Frame::new()
                .fill(t.card)
                .stroke(egui::Stroke::new(1.0, t.card_border))
                .corner_radius(egui::CornerRadius::same(4))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        if above {
                            crate::focus::header_button(app, ui, &sel);
                        }
                        more_options(app, ui, l, &sel.visual);
                    });
                });
        });
}

/// The header's More options button and its menu. The three dots are drawn (the UI's fonts have
/// no "⋯" glyph: the button showed as an empty frame).
fn more_options(app: &mut App, ui: &mut Ui, l: Lang, visual: &str) {
    let b = crate::widgets::icon_toggle(
        ui,
        crate::icons::Icon::More,
        l.t("More options", "Plus d'options"),
        true,
        false,
    );
    egui::Popup::menu(&b).show(|ui| visual_menu(app, ui, visual));
}

/// The menu items `ids` with their state; returns the one clicked (the menu closes).
pub fn items(app: &App, ui: &mut Ui, ids: &[&'static str]) -> Option<&'static str> {
    let mut picked = None;
    for id in ids {
        let Some(c) = registry::find(id) else { continue };
        let st = ribbon::state(app, id);
        if item(ui, c.text(app.lang), &st, app.lang) {
            picked = Some(c.id);
            ui.close();
        }
    }
    picked
}

/// One menu item: its text, greyed with the reason as tooltip.
pub fn item(ui: &mut Ui, text: &str, st: &Result<(), String>, l: Lang) -> bool {
    let r = ui.add_enabled(st.is_ok(), egui::Button::new(text));
    match st {
        Ok(()) => r.clicked(),
        Err(why) => {
            r.on_disabled_hover_text(format!("{} {why}", l.t("Unavailable:", "Indisponible :")));
            false
        }
    }
}

/// The states of the model menu's commands, for the diagram (which has no access to the app).
pub fn model_states(app: &App) -> HashMap<&'static str, Result<(), String>> {
    [
        TABLE,
        COLUMN,
        MEASURE,
        RELATIONSHIP,
        MODEL,
        LAYOUT_TAB,
        crate::model_explorer::COMMANDS,
    ]
    .concat()
    .into_iter()
    .map(|id| (id, ribbon::state(app, id)))
    .collect()
}

/// The objects an item of the model menu acts on, for its submenu: the selected table's
/// hierarchies or calculation items, the model's queries, roles and cultures (the snapshot's
/// names, docs/view-snapshot.md). `None`: the item takes the selection itself.
pub fn menu_objects(id: &str, sel: Option<&Sel>, model: &Value) -> Option<Vec<String>> {
    let strings = |v: Option<&Value>| -> Vec<String> {
        v.and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .take(MAX_SUBMENU)
            .map(str::to_string)
            .collect()
    };
    let table = || {
        let Some(Sel::Table(t)) = sel else { return None };
        model
            .get("tables")?
            .as_array()?
            .iter()
            .find(|x| x.get("name").and_then(Value::as_str) == Some(t.as_str()))
    };
    Some(match id {
        "hierarchy.remove" => strings(table()?.get("hierarchies")),
        "calculation_item.remove" => strings(table()?.get("calculation_items")),
        "expression.rename" | "expression.remove" => strings(model.get("expressions")),
        "role.remove" => strings(model.get("roles")),
        "culture.remove" => strings(model.get("cultures")),
        _ => return None,
    })
}

/// Most objects a submenu lists.
const MAX_SUBMENU: usize = 200;

/// The model diagram's menu for the selection; returns the command clicked, with the object
/// picked in its submenu as parameters (`{"name": …}`; an empty object without one).
pub fn model_menu(
    ui: &mut Ui,
    l: Lang,
    sel: Option<&Sel>,
    states: &HashMap<&'static str, Result<(), String>>,
    model: &Value,
) -> Option<(&'static str, Value)> {
    ui.set_min_width(200.0);
    let mut picked = None;
    for id in model_items(sel) {
        let Some(c) = registry::find(id) else { continue };
        let st = states
            .get(id)
            .cloned()
            .unwrap_or_else(|| Err("not available".into()));
        match menu_objects(id, sel, model) {
            // a submenu of the objects it can act on
            Some(names) if st.is_ok() && !names.is_empty() => {
                ui.menu_button(c.text(l), |ui| {
                    egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                        for n in &names {
                            if ui.button(crate::app::clip(n, 60)).clicked() {
                                picked = Some((c.id, serde_json::json!({ "name": n })));
                                ui.close();
                            }
                        }
                    });
                });
            }
            _ => {
                if item(ui, &item_text(c, l), &st, l) {
                    picked = Some((c.id, item_params(c.id, sel)));
                    ui.close();
                }
            }
        }
    }
    picked
}

/// A menu item's text: the registry's button text, but "Refresh data" for a table's refresh,
/// as in Desktop's table menu.
pub fn item_text(c: &registry::Cmd, l: Lang) -> String {
    match c.id {
        "model.refresh" => refresh_text(l).to_string(),
        _ => c.text(l).to_string(),
    }
}

/// The text of a table's "Refresh data" item.
pub fn refresh_text(l: Lang) -> &'static str {
    l.t("Refresh data", "Actualiser les données")
}

/// The parameters a menu item adds for the selection: a table's refresh names the table.
pub(crate) fn item_params(id: &str, sel: Option<&Sel>) -> Value {
    match (id, sel) {
        ("model.refresh", Some(Sel::Table(t))) => serde_json::json!({ "tables": [t] }),
        // the Incremental refresh dialog for the table
        ("table.set_refresh_policy", Some(Sel::Table(t))) => serde_json::json!({ "table": t }),
        _ => Value::Object(serde_json::Map::new()),
    }
}

/// Whether the data point `point` (`[{c, v}]`) is a row of the visual `v`'s data that its anomaly
/// detection flags (`data.anomalies`).
fn is_anomaly(v: &Value, point: &Value) -> bool {
    let Some(flags) = v.pointer("/data/anomalies/anomaly").and_then(Value::as_array) else {
        return false;
    };
    let Some(rows) = v.pointer("/data/rows").and_then(Value::as_array) else {
        return false;
    };
    let Some(want) = point.as_array().filter(|p| !p.is_empty()) else {
        return false;
    };
    rows.iter().zip(flags).any(|(r, f)| {
        f.as_bool() == Some(true)
            && want.iter().all(|e| {
                let c = e
                    .get("c")
                    .and_then(Value::as_u64)
                    .and_then(|c| usize::try_from(c).ok());
                c.and_then(|c| r.get(c)).is_some_and(|x| Some(x) == e.get("v"))
            })
    })
}
