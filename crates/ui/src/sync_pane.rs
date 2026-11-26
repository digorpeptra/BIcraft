//! The Sync slicers pane, as Power BI Desktop's ("Synchroniser les segments", ROADMAP M11.25): the
//! report's slicers listed by page with their sync group, a group name and Desktop's Fields /
//! Filters options, and the Synchronize / Unsync buttons for the slicers selected. Every edit is
//! an engine command (`slicer.set_sync`, one undo step, [`crate::panes::exec`]); the pane is pure
//! view state otherwise (the selection, the group being typed, the two options).

use egui::{Align, Layout, RichText, Sense, Ui, vec2};
use serde_json::{Value, json};

use crate::App;
use crate::icons::Icon;
use crate::panes::exec;
use crate::ribbon;
use crate::theme::Tokens;
use crate::widgets;

/// Most slicers the pane lists (as the engine caps them).
const MAX_ROWS: usize = 1000;

/// The pane's view state, kept by the window for every document.
#[derive(Debug)]
pub struct SyncUi {
    /// The slicers selected: `(page, visual)`; the last one is the pane's primary.
    pub selected: Vec<(String, String)>,
    /// The group name being typed.
    pub group: String,
    /// Desktop's two options: synchronize the fields and the filters.
    pub field_changes: bool,
    pub filter_changes: bool,
}

impl Default for SyncUi {
    fn default() -> Self {
        SyncUi {
            selected: Vec::new(),
            group: String::new(),
            field_changes: true,
            filter_changes: true,
        }
    }
}

/// One listed slicer.
struct Seg {
    page: String,
    page_display: String,
    visual: String,
    label: String,
    group: Option<String>,
    field_changes: bool,
    filter_changes: bool,
}

impl Seg {
    fn selected(&self, selected: &[(String, String)]) -> bool {
        selected.iter().any(|(p, v)| p == &self.page && v == &self.visual)
    }
}

/// The report's slicers, page by page.
fn rows(app: &App) -> Option<Vec<Seg>> {
    let snap = app.loaded.as_ref().map(|l| l.snap.as_ref())?;
    let pages = snap.pointer("/report/pages").and_then(Value::as_array)?;
    let mut out = Vec::new();
    for page in pages.iter().take(MAX_ROWS) {
        let Some(page_name) = page.get("name").and_then(Value::as_str) else {
            continue;
        };
        let page_display = page
            .get("display_name")
            .and_then(Value::as_str)
            .unwrap_or(page_name);
        for v in page
            .get("visuals")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if v.get("type").and_then(Value::as_str) != Some("slicer") {
                continue;
            }
            let Some(visual) = v.get("name").and_then(Value::as_str) else {
                continue;
            };
            let group = v
                .get("sync_group")
                .and_then(|g| g.get("name"))
                .and_then(Value::as_str)
                .map(str::to_string);
            let flag = |k: &str| {
                v.get("sync_group")
                    .and_then(|g| g.get(k))
                    .and_then(Value::as_bool)
                    .unwrap_or(true)
            };
            let label = v
                .get("title")
                .and_then(Value::as_str)
                .filter(|t| !t.trim().is_empty())
                .map(str::to_string)
                .or_else(|| {
                    v.get("type")
                        .and_then(Value::as_str)
                        .and_then(|t| crate::panes::type_label(t, app.lang))
                        .map(str::to_string)
                })
                .unwrap_or_else(|| "Slicer".to_string());
            out.push(Seg {
                page: page_name.to_string(),
                page_display: page_display.to_string(),
                visual: visual.to_string(),
                label,
                group,
                field_changes: flag("field_changes"),
                filter_changes: flag("filter_changes"),
            });
        }
    }
    Some(out)
}

/// The pane's body (the panel's frame, title and width belong to `crate::panes`).
pub fn show(app: &mut App, ui: &mut Ui) {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    if crate::panes::title_row(
        ui,
        l.t("Sync slicers", "Synchroniser les segments"),
        l.t(
            "Collapse the Sync slicers pane",
            "Réduire le volet Synchroniser les segments",
        ),
    ) {
        app.prefs.panes.sync = false;
        app.save_prefs();
        return;
    }
    let Some(rows) = rows(app) else {
        ui.label(
            RichText::new(l.t(
                "Open a report to synchronize its slicers.",
                "Ouvrez un rapport pour synchroniser ses segments.",
            ))
            .color(t.text_dim),
        );
        return;
    };
    if rows.is_empty() {
        ui.label(
            RichText::new(l.t("This report has no slicer.", "Ce rapport n'a aucun segment."))
                .color(t.text_dim),
        );
    }
    let can_edit = ribbon::state(app, "slicer.set_sync").is_ok();
    let selected = app.sync.selected.clone();
    let mut act: Option<(&'static str, Value)> = None;
    let mut select: Option<((String, String), bool)> = None;

    // The group name, the options and the buttons at the pane's foot.
    egui::Panel::bottom("bicraft-sync-actions")
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(l.t("Group", "Groupe"));
                let group = ui.add(
                    egui::TextEdit::singleline(&mut app.sync.group)
                        .desired_width((ui.available_width() - 40.0).max(60.0))
                        .hint_text(l.t("Group name", "Nom du groupe")),
                );
                group.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::Role::TextInput,
                        true,
                        l.t("Sync group name", "Nom du groupe de synchronisation"),
                    )
                });
            });
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut app.sync.field_changes, l.t("Fields", "Champs"))
                    .on_hover_text(l.t(
                        "Synchronize the slicers' fields",
                        "Synchroniser les champs des segments",
                    ));
                ui.checkbox(&mut app.sync.filter_changes, l.t("Filters", "Filtres"))
                    .on_hover_text(l.t(
                        "Synchronize the slicers' selections",
                        "Synchroniser les sélections des segments",
                    ));
            });
            ui.horizontal_wrapped(|ui| {
                let group = app.sync.group.trim().to_string();
                let segments: Vec<Value> = selected
                    .iter()
                    .map(|(page, visual)| json!({"page": page, "visual": visual}))
                    .collect();
                if widgets::small_button(
                    ui,
                    Icon::Link,
                    l.t("Synchronize", "Synchroniser"),
                    !segments.is_empty() && !group.is_empty() && can_edit,
                    false,
                )
                .clicked()
                {
                    act = Some((
                        "slicer.set_sync",
                        json!({"segments": segments, "group": group,
                               "field_changes": app.sync.field_changes,
                               "filter_changes": app.sync.filter_changes}),
                    ));
                }
                if widgets::small_button(
                    ui,
                    Icon::Ungroup,
                    l.t("Unsync", "Dissocier"),
                    !selected.is_empty() && can_edit,
                    false,
                )
                .clicked()
                {
                    act = Some(("slicer.set_sync", json!({"segments": segments, "group": null})));
                }
            });
        });

    egui::ScrollArea::vertical()
        .id_salt("bicraft-sync-rows")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let mut last_page: Option<&str> = None;
            for row in &rows {
                if last_page != Some(row.page.as_str()) {
                    last_page = Some(row.page.as_str());
                    ui.label(
                        RichText::new(&row.page_display)
                            .color(t.text_dim)
                            .small()
                            .strong(),
                    );
                }
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    let mut text = RichText::new(&row.label).color(t.text);
                    if row.selected(&selected) {
                        text = text.strong();
                    }
                    let resp = ui.add(
                        egui::Button::new(text)
                            .frame(false)
                            .min_size(vec2(0.0, 20.0))
                            .sense(Sense::click()),
                    );
                    if resp.clicked() {
                        let add = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);
                        select = Some(((row.page.clone(), row.visual.clone()), add));
                    }
                    let mut tip = row.label.clone();
                    if let Some(g) = &row.group {
                        tip = format!(
                            "{} — {} {g}",
                            row.label,
                            l.t("synchronized with", "synchronisé avec")
                        );
                    }
                    resp.on_hover_text(tip);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if let Some(g) = &row.group {
                            ui.label(RichText::new(g).color(t.text_dim).small());
                        }
                        if row.group.is_some() {
                            let mut notes = Vec::new();
                            if row.field_changes {
                                notes.push(l.t("fields", "champs"));
                            }
                            if row.filter_changes {
                                notes.push(l.t("filters", "filtres"));
                            }
                            if !notes.is_empty() {
                                ui.label(RichText::new(notes.join(" · ")).color(t.text_faint).small());
                            }
                        }
                    });
                });
            }
        });
    if let Some((key, add)) = select {
        let mut names = if add {
            app.sync.selected.clone()
        } else {
            Vec::new()
        };
        if add && names.contains(&key) {
            names.retain(|n| n != &key);
        } else {
            names.push(key.clone());
        }
        app.sync.selected = names;
        // the primary slicer's group becomes the name typed, as Desktop's pane keeps it
        if let Some(row) = rows.iter().rev().find(|r| r.page == key.0 && r.visual == key.1)
            && let Some(g) = &row.group
        {
            app.sync.group = g.clone();
        }
        if let Some(row) = rows.iter().rev().find(|r| r.page == key.0 && r.visual == key.1) {
            app.sync.field_changes = row.field_changes;
            app.sync.filter_changes = row.filter_changes;
        }
    }
    if let Some((id, params)) = act {
        exec(app, id, params);
    }
}
