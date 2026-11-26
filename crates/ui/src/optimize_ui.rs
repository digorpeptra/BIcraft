//! Desktop's Optimize ribbon in the app (`docs/gaps.md` G60): Analyze › Optimize holds Pause
//! visuals (a toggle: `report.pause_visuals`), Refresh visuals (`report.refresh_visuals`) and
//! Optimization presets (a menu: Query reduction, Interactivity, Customize…, which runs
//! `report.query_reduction` or opens File › Options and settings at its Query reduction
//! section). While the report is paused a banner over the views says so, with Refresh and Resume
//! visual queries, as Desktop's report banner.

use egui::{RichText, Ui, vec2};
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

/// Every command the Optimize group, the banner and the Query reduction section run.
pub const IDS: &[&str] = &[PAUSE, REFRESH, PRESETS];

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
