//! Desktop's Optimize ribbon in the app (`docs/gaps.md` G60): Analyze › Optimize holds Pause
//! visuals (a toggle: `report.pause_visuals`), Refresh visuals (`report.refresh_visuals`) and
//! Optimization presets (a menu: Query reduction, Interactivity, Customize…, which runs
//! `report.query_reduction` or opens File › Options and settings at its Query reduction
//! section). While the report is paused a banner over the views says so, with Refresh and Resume
//! visual queries, as Desktop's report banner. The Optimize tab holds these groups, plus the
//! Performance analyzer button that Analyze › Performance also has.
//!
//! While paused, each visual with pending changes (its data kept from before, or none yet) shows a
//! strip at its top: a pause icon, the words "Pending changes" and a Refresh visual button
//! (`visual.refresh`, also in the visual's right-click menu) that evaluates that visual alone.
//! Such a visual shows no tooltip, and no report-page tooltip is asked for while paused.

use egui::{Rect, RichText, Sense, Stroke, Ui, vec2};
use serde_json::{Value, json};

use crate::App;
use crate::actions::Call;
use crate::app::Then;
use crate::backstage::{Backstage, Page};
use crate::icons::{self, Icon};
use crate::registry::Cmd;
use crate::theme::{self, Tokens};
use crate::widgets;

pub const PAUSE: &str = "report.pause_visuals";
pub const REFRESH: &str = "report.refresh_visuals";
pub const PRESETS: &str = "report.query_reduction";
/// A visual's own Refresh (its pending-changes strip, its right-click menu).
pub const REFRESH_ONE: &str = "visual.refresh";
/// The Performance analyzer (the Optimize tab's button opens its pane).
pub const PERF: &str = "perf.results";

/// Every command the Optimize tab, the banners and the Query reduction section run.
pub const IDS: &[&str] = &[PAUSE, REFRESH, PRESETS, REFRESH_ONE, PERF];

fn snap_settings(app: &App) -> Value {
    app.loaded
        .as_ref()
        .and_then(|l| l.snap.pointer("/report/settings"))
        .cloned()
        .unwrap_or(Value::Null)
}

/// Whether the report's visual queries are paused (the snapshot's `report.settings.paused`).
pub fn paused(app: &App) -> bool {
    snap_settings(app).get("paused").and_then(Value::as_bool) == Some(true)
}

fn call(id: &'static str, params: Value) -> Call {
    Call { id, params }
}

/// Pause or resume the report's visual queries.
pub fn pause(app: &mut App, on: bool) {
    app.run(call(PAUSE, json!({"paused": on})), Then::Nothing);
}

/// Refresh visuals: the next view evaluates every visual once.
pub fn refresh(app: &mut App) {
    app.run(call(REFRESH, json!({})), Then::Nothing);
}

/// The presets of the Optimization presets menu: (preset, English, French); `None` is
/// Customize, which opens the Options page.
const PRESET_ITEMS: [(Option<&str>, &str, &str); 3] = [
    (
        Some("query_reduction"),
        "Query reduction",
        "Réduction des requêtes",
    ),
    (Some("interactivity"), "Interactivity", "Interactivité"),
    (None, "Customize…", "Personnaliser…"),
];

/// Draws the Optimize group's button `c` when it is one of its own (a toggle or a menu); false
/// for any other command.
pub fn button(app: &mut App, ui: &mut Ui, c: &'static Cmd, big: bool) -> bool {
    let st = crate::ribbon::state(app, c.id);
    let label = c.text(app.lang);
    let tip = c.tooltip(app.lang, &st, app.caps().reserved_keys);
    match c.id {
        PAUSE => {
            let on = paused(app);
            let r = widgets::toggle_button(ui, c.icon(), label, st.is_ok(), on, big);
            let tipped = crate::key_tips::button(app, ui, c.id, &r) && st.is_ok();
            if r.on_hover_text(tip).clicked() || tipped {
                pause(app, !on);
            }
            true
        }
        PRESETS => {
            let r = if big {
                widgets::big_button(ui, c.icon(), label, st.is_ok())
            } else {
                widgets::small_button(ui, c.icon(), label, st.is_ok(), false)
            };
            let tipped = crate::key_tips::button(app, ui, c.id, &r);
            let r = r.on_hover_text(tip);
            if st.is_ok() {
                let popup = crate::key_tips::menu(app, &r, tipped);
                let l = app.lang;
                let current = snap_settings(app)
                    .pointer("/query_reduction/preset")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let mut picked: Option<Option<&str>> = None;
                popup.show(|ui| {
                    for (preset, en, fr) in PRESET_ITEMS {
                        let on = preset.is_some() && preset == current.as_deref();
                        if ui.selectable_label(on, l.t(en, fr)).clicked() {
                            picked = Some(preset);
                            ui.close();
                        }
                    }
                });
                match picked {
                    Some(Some(p)) => {
                        app.run(call(PRESETS, json!({"preset": p})), Then::Nothing);
                    }
                    Some(None) => {
                        app.backstage = Some(Backstage::new(Page::Options));
                    }
                    None => {}
                }
            }
            true
        }
        _ => false,
    }
}

/// The banner over the views while the report is paused: Refresh and Resume visual queries.
pub fn banner(app: &mut App, ui: &mut Ui) {
    if !paused(app) {
        return;
    }
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    let can = crate::ribbon::state(app, PAUSE).is_ok();
    let (mut go_refresh, mut resume) = (false, false);
    egui::Panel::top("bicraft-paused-banner")
        .frame(
            egui::Frame::NONE
                .fill(t.accent_soft)
                .inner_margin(egui::Margin::symmetric(12, 6)),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), egui::Sense::hover());
                icons::paint(ui.painter(), r, Icon::EyeOff, t.text, false);
                ui.label(
                    RichText::new(l.t(
                        "Visual queries are paused: visuals keep their last state until refreshed.",
                        "Les requêtes des visuels sont en pause : les visuels gardent leur dernier état jusqu'à l'actualisation.",
                    ))
                    .font(theme::bold(13.0))
                    .color(t.text),
                );
                ui.add_space(12.0);
                if ui.button(l.t("Refresh", "Actualiser")).clicked() {
                    go_refresh = true;
                }
                if ui
                    .add_enabled(
                        can,
                        egui::Button::new(l.t("Resume visual queries", "Reprendre les requêtes des visuels")),
                    )
                    .clicked()
                {
                    resume = true;
                }
            });
        });
    if go_refresh {
        refresh(app);
    }
    if resume {
        pause(app, false);
    }
}

/// File › Options and settings › Query reduction: the options each preset sets, one by one
/// (Desktop's Customize).
pub fn query_reduction_section(app: &mut App, ui: &mut Ui) {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    let qr = snap_settings(app)
        .get("query_reduction")
        .cloned()
        .unwrap_or(Value::Null);
    let on = crate::ribbon::state(app, PRESETS).is_ok();
    ui.add_space(18.0);
    ui.label(
        RichText::new(l.t("Query reduction", "Réduction des requêtes"))
            .font(theme::bold(16.0))
            .color(t.text),
    );
    ui.label(
        RichText::new(l.t(
            "Fewer queries while readers select: what the Optimize ribbon's presets set.",
            "Moins de requêtes pendant les sélections des lecteurs : ce que règlent les préréglages du ruban Optimiser.",
        ))
        .font(theme::font(12.5))
        .color(t.text_dim),
    );
    let mut run: Option<Value> = None;
    let mut cross = qr.get("disable_cross_highlighting").and_then(Value::as_bool) == Some(true);
    if ui
        .add_enabled(
            on,
            egui::Checkbox::new(
                &mut cross,
                l.t(
                    "Disable cross highlighting/filtering by default",
                    "Désactiver par défaut la sélection et le filtrage croisés",
                ),
            ),
        )
        .changed()
    {
        run = Some(json!({"disable_cross_highlighting": cross}));
    }
    let slicers = qr.get("slicers").and_then(Value::as_str).unwrap_or("instantly");
    ui.add_space(6.0);
    ui.label(
        RichText::new(l.t("Slicers", "Segments"))
            .font(theme::bold(13.0))
            .color(t.text),
    );
    for (id, en, fr) in [
        (
            "instantly",
            "Instantly apply slicer changes",
            "Appliquer immédiatement les changements des segments",
        ),
        (
            "apply_button",
            "Add an Apply button to each slicer",
            "Ajouter un bouton Appliquer à chaque segment",
        ),
    ] {
        if ui
            .add_enabled(on, egui::RadioButton::new(slicers == id, l.t(en, fr)))
            .clicked()
            && slicers != id
        {
            run = Some(json!({"slicers": id}));
        }
    }
    let filters = qr.get("filters").and_then(Value::as_str).unwrap_or("instantly");
    ui.add_space(6.0);
    ui.label(
        RichText::new(l.t("Filters", "Filtres"))
            .font(theme::bold(13.0))
            .color(t.text),
    );
    for (id, en, fr) in [
        (
            "instantly",
            "Instantly apply basic filter changes",
            "Appliquer immédiatement les changements des filtres de base",
        ),
        (
            "apply_buttons",
            "Add an Apply button to each basic filter",
            "Ajouter un bouton Appliquer à chaque filtre de base",
        ),
        (
            "single_button",
            "Add a single Apply button to the filter pane to apply changes at once",
            "Ajouter un seul bouton Appliquer au volet Filtres pour appliquer les changements en une fois",
        ),
    ] {
        if ui
            .add_enabled(on, egui::RadioButton::new(filters == id, l.t(en, fr)))
            .clicked()
            && filters != id
        {
            run = Some(json!({"filters": id}));
        }
    }
    if let Some(p) = run {
        app.run(call(PRESETS, p), Then::Nothing);
    }
}

/// Optimize › Performance: the Performance analyzer button (it toggles the pane, as the one in
/// Analyze › Performance does).
pub fn performance_group(app: &mut App, ui: &mut Ui) {
    let Some(c) = crate::registry::find(PERF) else {
        return;
    };
    let label = c.text(app.lang);
    let st = crate::ribbon::state(app, PERF);
    let tip = c.tooltip(app.lang, &st, app.caps().reserved_keys);
    let on = app.cmd.perf.open;
    let mut go = false;
    widgets::group(ui, |ui| {
        let r = widgets::toggle_button(ui, c.icon(), label, st.is_ok(), on, true);
        if r.on_hover_text(tip).clicked() && st.is_ok() {
            go = true;
        }
    });
    if go {
        crate::ribbon::invoke(app, PERF, json!({}));
    }
}

/// Whether a visual's snapshot entry waits for a refresh: its data kept from before
/// (`data.pending`) or none yet (`data.status: "pending"`).
pub fn pending(v: &Value) -> bool {
    let d = v.get("data");
    d.and_then(|d| d.get("pending")).and_then(Value::as_bool) == Some(true)
        || d.and_then(|d| d.get("status")).and_then(Value::as_str) == Some("pending")
}

/// The visuals of the page shown that wait for a refresh: name and box in report pixels.
fn pending_visuals(app: &App) -> Vec<(String, Rect)> {
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
        .filter(|v| v.get("hidden").and_then(Value::as_bool) != Some(true) && pending(v))
        .filter_map(|v| {
            let r = Rect::from_min_size(
                egui::pos2(n(v, "x")? as f32, n(v, "y")? as f32),
                vec2(n(v, "width")? as f32, n(v, "height")? as f32),
            );
            Some((v.get("name")?.as_str()?.to_string(), r))
        })
        .collect()
}

/// Whether visual `visual` of the page shown waits for a refresh (its tooltips are off).
pub fn visual_pending(app: &App, visual: &str) -> bool {
    paused(app) && pending_visuals(app).iter().any(|(n, _)| n == visual)
}

/// Height of a pending-changes strip, in screen pixels.
const STRIP: f32 = 22.0;

/// The pending-changes strip of every waiting visual of the page shown (while paused): a pause
/// icon, "Pending changes" (an icon and a word, not colour alone) and a Refresh visual button,
/// focusable and run by Enter or Space as any button. Returns whether the pointer is on one (the
/// page's own click is then not a page click).
pub fn pending_strips(app: &mut App, ui: &Ui, page_rect: Rect, zoom: f32) -> bool {
    if !paused(app) {
        return false;
    }
    let Some(page) = app.page_names().get(app.active_page()).map(|p| p.0.clone()) else {
        return false;
    };
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    let can = crate::ribbon::state(app, REFRESH_ONE).is_ok();
    let pointer = ui.ctx().pointer_hover_pos();
    let mut on = false;
    let mut send: Option<String> = None;
    let word = l.t("Pending changes", "Modifications en attente");
    let button = l.t("Refresh visual", "Actualiser le visuel");
    let font = theme::font(11.5);
    for (name, r) in pending_visuals(app) {
        let screen = Rect::from_min_size(page_rect.min + r.min.to_vec2() * zoom, r.size() * zoom);
        if screen.width() < 40.0 || screen.height() < STRIP {
            continue;
        }
        let strip = Rect::from_min_size(screen.min, vec2(screen.width(), STRIP));
        let painter = ui.painter().with_clip_rect(strip.intersect(ui.clip_rect()));
        painter.rect_filled(strip, egui::CornerRadius::same(2), t.accent_soft);
        painter.rect_stroke(
            strip,
            egui::CornerRadius::same(2),
            Stroke::new(1.0, t.border),
            egui::StrokeKind::Inside,
        );
        let icon = Rect::from_min_size(strip.min + vec2(4.0, 4.0), vec2(14.0, 14.0));
        icons::paint(&painter, icon, Icon::EyeOff, t.text, false);
        let galley = painter.layout_no_wrap(button.to_string(), font.clone(), t.text);
        let bw = galley.size().x + 12.0;
        let b = Rect::from_min_size(
            egui::pos2(strip.max.x - bw - 3.0, strip.min.y + 2.0),
            vec2(bw, STRIP - 4.0),
        );
        let text_room = b.min.x - icon.max.x - 8.0;
        if text_room > 20.0 {
            let words = painter.layout(word.to_string(), font.clone(), t.text, text_room);
            painter.galley(
                egui::pos2(icon.max.x + 4.0, strip.center().y - words.size().y / 2.0),
                words,
                t.text,
            );
        }
        // the strip's words for the accessibility tree
        let strip_id = ui.scope_id().with(("bicraft-pending-strip", &name));
        ui.interact(
            Rect::from_min_max(strip.min, egui::pos2(b.min.x, strip.max.y)),
            strip_id,
            Sense::hover(),
        )
        .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, word));
        let id = ui.scope_id().with(("bicraft-pending-refresh", &name));
        let resp = ui.interact(b, id, if can { Sense::click() } else { Sense::hover() });
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, can, button));
        let focused = resp.has_focus();
        let bg = if resp.hovered() && can { t.hover } else { t.card };
        painter.rect_filled(b, egui::CornerRadius::same(3), bg);
        painter.rect_stroke(
            b,
            egui::CornerRadius::same(3),
            Stroke::new(
                if focused { 2.0 } else { 1.0 },
                if focused { t.ring } else { t.border },
            ),
            egui::StrokeKind::Inside,
        );
        painter.galley(
            egui::pos2(b.min.x + 6.0, b.center().y - galley.size().y / 2.0),
            galley,
            if can { t.text } else { t.text_faint },
        );
        if pointer.is_some_and(|p| strip.contains(p)) {
            on = true;
        }
        if resp
            .on_hover_text(l.t(
                "Evaluate this visual now; the others stay paused",
                "Évaluer ce visuel maintenant ; les autres restent en pause",
            ))
            .clicked()
        {
            send = Some(name);
        }
    }
    if let Some(visual) = send {
        app.run(
            call(REFRESH_ONE, json!({"page": page, "visual": visual})),
            Then::Nothing,
        );
    }
    on
}

/// The slicers of the page shown with query reduction's own Apply button
/// (`slicer.apply_button`): name, box in report pixels, whether a change waits
/// (`slicer.pending`).
fn apply_slicers(app: &App) -> Vec<(String, Rect, bool)> {
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
        .filter(|v| v.pointer("/slicer/apply_button").and_then(Value::as_bool) == Some(true))
        .filter_map(|v| {
            let r = Rect::from_min_size(
                egui::pos2(n(v, "x")? as f32, n(v, "y")? as f32),
                vec2(n(v, "width")? as f32, n(v, "height")? as f32),
            );
            let waits = v.pointer("/slicer/pending").and_then(Value::as_bool) == Some(true);
            Some((v.get("name")?.as_str()?.to_string(), r, waits))
        })
        .collect()
}

/// Query reduction's Apply button on each slicer (File › Options and settings › Query
/// reduction, "Add an Apply button to each slicer"): at the slicer's bottom right, enabled while
/// a change waits; it sends `view.interact` `slicer_apply`. Returns whether the pointer is on one.
pub fn slicer_apply(app: &mut App, ui: &Ui, page_rect: Rect, zoom: f32) -> bool {
    let Some(page) = app.page_names().get(app.active_page()).map(|p| p.0.clone()) else {
        return false;
    };
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    let pointer = ui.ctx().pointer_hover_pos();
    let label = l.t("Apply", "Appliquer");
    let font = theme::font(11.5);
    let mut on = false;
    let mut send: Option<String> = None;
    for (name, r, waits) in apply_slicers(app) {
        let screen = Rect::from_min_size(page_rect.min + r.min.to_vec2() * zoom, r.size() * zoom);
        let painter = ui.painter().with_clip_rect(screen.intersect(ui.clip_rect()));
        let galley = painter.layout_no_wrap(label.to_string(), font.clone(), t.text);
        let size = vec2(galley.size().x + 16.0, 20.0);
        if screen.width() < size.x + 4.0 || screen.height() < size.y + 4.0 {
            continue;
        }
        let b = Rect::from_min_size(screen.max - size - vec2(3.0, 3.0), size);
        let id = ui.scope_id().with(("bicraft-slicer-apply", &name));
        let resp = ui.interact(b, id, if waits { Sense::click() } else { Sense::hover() });
        // the name starts with the visible label; the state says whether a change waits
        let state = if waits {
            l.t("changes waiting", "changements en attente")
        } else {
            l.t("nothing to apply", "rien à appliquer")
        };
        let a11y = format!("{label} ({name}): {state}");
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, waits, &a11y));
        let focused = resp.has_focus();
        let bg = if waits && resp.hovered() {
            t.hover
        } else if waits {
            t.accent_soft
        } else {
            t.card
        };
        painter.rect_filled(b, egui::CornerRadius::same(3), bg);
        painter.rect_stroke(
            b,
            egui::CornerRadius::same(3),
            Stroke::new(
                if focused { 2.0 } else { 1.0 },
                if focused { t.ring } else { t.border },
            ),
            egui::StrokeKind::Inside,
        );
        painter.galley(
            egui::pos2(b.min.x + 8.0, b.center().y - galley.size().y / 2.0),
            galley,
            if waits { t.text } else { t.text_faint },
        );
        if pointer.is_some_and(|p| b.contains(p)) {
            on = true;
        }
        if resp.clicked() {
            send = Some(name);
        }
    }
    if let Some(visual) = send {
        app.interact(json!({"action": "slicer_apply", "page": page, "visual": visual}));
    }
    on
}
