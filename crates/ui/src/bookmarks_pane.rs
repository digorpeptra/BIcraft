//! The Bookmarks pane, as Power BI Desktop's (ROADMAP M11.24): the report's bookmarks in their
//! saved order, groups foldable, with the Add, View, Update, Rename, Delete, Group, Ungroup and
//! move buttons at the foot and Desktop's Data / Display / Current page options for the bookmark
//! selected. Every edit is an engine command (one undo step, [`crate::panes::exec`]); the pane is
//! pure view state otherwise (folded groups, the selection, the name being typed).

use std::collections::BTreeSet;

use egui::{Align, Key, Layout, RichText, Sense, Ui, vec2};
use serde_json::{Value, json};

use crate::App;
use crate::icons::Icon;
use crate::panes::exec;
use crate::ribbon;
use crate::theme::Tokens;
use crate::widgets;

/// Most rows the pane lists (a report holds at most 1000 bookmarks).
const MAX_ROWS: usize = 1000;

/// The pane's view state, kept by the window for every document.
#[derive(Debug, Default)]
pub struct BookmarksUi {
    /// Groups whose bookmarks are folded away (lowercased group names).
    pub folded: BTreeSet<String>,
    /// The bookmarks (or groups) selected, the last one the pane's buttons act on.
    pub selected: Vec<String>,
    /// The bookmark or group being renamed and the text typed so far.
    pub renaming: Option<(String, String)>,
    /// The rename editor still has to take the focus.
    focus: bool,
}

/// One listed row: a bookmark, or a group's caption above its bookmarks.
struct Row {
    name: String,
    label: String,
    group: bool,
    /// The group this bookmark belongs to (the group's name), `None` at the top level.
    parent: Option<String>,
    folded: bool,
    hidden: u64,
    filters: u64,
    data: bool,
    display: bool,
    current_page: bool,
    selected: bool,
    page: Option<String>,
}

impl Row {
    /// A bookmark row from its snapshot entry.
    fn entry(e: &Value, parent: Option<&str>, selected: &[String]) -> Option<Row> {
        let name = e.get("name").and_then(Value::as_str)?;
        Some(Row {
            name: name.to_string(),
            label: e
                .get("display_name")
                .and_then(Value::as_str)
                .unwrap_or(name)
                .to_string(),
            group: false,
            parent: parent.map(str::to_string),
            folded: false,
            hidden: e.get("hidden").and_then(Value::as_u64).unwrap_or(0),
            filters: e.get("filters").and_then(Value::as_u64).unwrap_or(0),
            data: e.get("data").and_then(Value::as_bool).unwrap_or(true),
            display: e.get("display").and_then(Value::as_bool).unwrap_or(true),
            current_page: e.get("current_page").and_then(Value::as_bool).unwrap_or(false),
            selected: selected.iter().any(|s| s == name),
            page: e.get("page").and_then(Value::as_str).map(str::to_string),
        })
    }
}

/// The pane's rows, in the report's saved order; `None` without a document.
fn rows(app: &App) -> Option<Vec<Row>> {
    let snap = app.loaded.as_ref().map(|l| l.snap.as_ref())?;
    let doc = snap.pointer("/bookmarks")?;
    let entries = doc.get("bookmarks").and_then(Value::as_array)?;
    let groups = doc.get("groups").and_then(Value::as_array)?;
    let selected = &app.bookmarks.selected;
    let mut out = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for e in entries.iter().take(MAX_ROWS) {
        let Some(group) = e.get("group").and_then(Value::as_str).filter(|g| !g.is_empty()) else {
            if let Some(row) = Row::entry(e, None, selected) {
                out.push(row);
            }
            continue;
        };
        let key = groups
            .iter()
            .find(|g| g.get("display_name").and_then(Value::as_str) == Some(group))
            .and_then(|g| g.get("name").and_then(Value::as_str))
            .unwrap_or(group)
            .to_string();
        if seen.insert(key.clone()) {
            out.push(Row {
                name: key.clone(),
                label: group.to_string(),
                group: true,
                parent: None,
                folded: app.bookmarks.folded.contains(&key.to_lowercase()),
                hidden: 0,
                filters: 0,
                data: true,
                display: true,
                current_page: false,
                selected: selected.iter().any(|s| s == &key),
                page: None,
            });
        }
        if app.bookmarks.folded.contains(&key.to_lowercase()) {
            continue;
        }
        if let Some(row) = Row::entry(e, Some(&key), selected) {
            out.push(row);
        }
    }
    Some(out)
}

/// A default name for a new bookmark or group: `base` and the next number after the highest one
/// already used (`Bookmark 1`, `Bookmark 2`…), as Desktop numbers them.
fn default_name(app: &App, base: &str) -> String {
    let Some(snap) = app.loaded.as_ref().map(|l| l.snap.as_ref()) else {
        return format!("{base} 1");
    };
    let mut highest = 0_usize;
    for key in ["/bookmarks/bookmarks", "/bookmarks/groups"] {
        for e in snap.pointer(key).and_then(Value::as_array).into_iter().flatten() {
            let Some(name) = e.get("display_name").and_then(Value::as_str) else {
                continue;
            };
            if let Some(rest) = name.strip_prefix(base)
                && let Ok(n) = rest.trim().parse::<usize>()
                && n > highest
            {
                highest = n;
            }
        }
    }
    format!("{base} {}", highest + 1)
}

/// The row of the bookmark (or group) the pane's buttons act on: the last selected one.
fn primary<'a>(app: &App, rows: &'a [Row]) -> Option<&'a Row> {
    let name = app.bookmarks.selected.last()?;
    rows.iter().find(|r| &r.name == name)
}

/// The pane's body (the panel's frame, title and width belong to `crate::panes`).
pub fn show(app: &mut App, ui: &mut Ui) {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    if crate::panes::title_row(
        ui,
        l.t("Bookmarks", "Signets"),
        l.t("Collapse the Bookmarks pane", "Réduire le volet Signets"),
    ) {
        app.prefs.panes.bookmarks = false;
        app.save_prefs();
        return;
    }
    let Some(rows) = rows(app) else {
        ui.label(
            RichText::new(l.t(
                "Open a report to see its bookmarks.",
                "Ouvrez un rapport pour voir ses signets.",
            ))
            .color(t.text_dim),
        );
        return;
    };
    if rows.is_empty() {
        ui.label(
            RichText::new(l.t("This report has no bookmark.", "Ce rapport n'a aucun signet."))
                .color(t.text_dim),
        );
    }
    let can_capture = ribbon::state(app, "bookmark.capture").is_ok();
    let can_edit = ribbon::state(app, "bookmark.set").is_ok();
    let primary_row = primary(app, &rows);
    let is_group = primary_row.is_some_and(|r| r.group);
    let is_entry = primary_row.is_some_and(|r| !r.group);
    let selection: Vec<String> = app.bookmarks.selected.clone();
    let mut act: Option<(&'static str, Value)> = None;
    let mut select: Option<(String, bool)> = None;
    let mut fold: Option<String> = None;
    let mut start_rename: Option<(String, String)> = None;
    let mut end_rename = false;

    // The buttons sit at the pane's foot, in a bottom panel so a short window never clips them.
    egui::Panel::bottom("bicraft-bookmarks-actions")
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                if widgets::small_button(
                    ui,
                    Icon::Pin,
                    l.t("Add bookmark", "Ajouter un signet"),
                    can_capture,
                    false,
                )
                .clicked()
                {
                    let name = default_name(app, l.t("Bookmark", "Signet"));
                    act = Some(("bookmark.capture", json!({"display_name": name})));
                }
                let one = primary_row.filter(|r| !r.group);
                if widgets::small_button(
                    ui,
                    Icon::Play,
                    l.t("View bookmark", "Afficher le signet"),
                    one.is_some() && ribbon::state(app, "bookmark.apply").is_ok(),
                    false,
                )
                .clicked()
                    && let Some(row) = one
                {
                    act = Some(("bookmark.apply", json!({"bookmark": row.name})));
                }
                if widgets::small_button(
                    ui,
                    Icon::Reload,
                    l.t("Update bookmark", "Mettre à jour le signet"),
                    one.is_some() && ribbon::state(app, "bookmark.update").is_ok(),
                    false,
                )
                .clicked()
                    && let Some(row) = one
                {
                    act = Some(("bookmark.update", json!({"bookmark": row.name})));
                }
                let rename_ok =
                    is_entry && can_edit || is_group && ribbon::state(app, "bookmark.rename_group").is_ok();
                if widgets::small_button(ui, Icon::Edit, l.t("Rename", "Renommer"), rename_ok, false)
                    .clicked()
                    && let Some(row) = primary_row
                {
                    app.bookmarks.renaming = Some((row.name.clone(), row.label.clone()));
                    app.bookmarks.focus = true;
                }
                if widgets::small_button(
                    ui,
                    Icon::Delete,
                    l.t("Delete", "Supprimer"),
                    primary_row.is_some() && can_edit,
                    false,
                )
                .clicked()
                    && let Some(row) = primary_row
                {
                    act = Some(("bookmark.remove", json!({"bookmark": row.name})));
                }
            });
            ui.horizontal_wrapped(|ui| {
                let listed: Vec<String> = selection
                    .iter()
                    .filter(|n| !rows.iter().any(|r| &r.name == *n && r.group))
                    .cloned()
                    .collect();
                if widgets::small_button(
                    ui,
                    Icon::Group,
                    l.t("Group", "Grouper"),
                    !listed.is_empty() && can_capture,
                    false,
                )
                .clicked()
                {
                    let name = default_name(app, l.t("Bookmark group", "Groupe de signets"));
                    act = Some((
                        "bookmark.group",
                        json!({"bookmarks": listed, "display_name": name}),
                    ));
                }
                if widgets::small_button(
                    ui,
                    Icon::Ungroup,
                    l.t("Ungroup", "Dissocier"),
                    is_group && can_edit,
                    false,
                )
                .clicked()
                    && let Some(row) = primary_row
                {
                    act = Some(("bookmark.ungroup", json!({"group": row.name})));
                }
                // Move the primary row one place among its siblings (its group's bookmarks, or
                // the report's list).
                let siblings: Vec<&Row> = primary_row
                    .map(|row| rows.iter().filter(|r| r.parent == row.parent).collect::<Vec<_>>())
                    .unwrap_or_default();
                let at = primary_row.and_then(|row| siblings.iter().position(|r| r.name == row.name));
                let mut step = |dir: i64, ui: &mut Ui| {
                    let (Some(row), Some(at)) = (primary_row, at) else {
                        return;
                    };
                    let target = at as i64 + dir;
                    if target < 0 || target as usize >= siblings.len() {
                        return;
                    }
                    if widgets::icon_toggle(
                        ui,
                        if dir < 0 {
                            Icon::ChevronUp
                        } else {
                            Icon::ChevronDown
                        },
                        if dir < 0 {
                            l.t("Move up", "Monter")
                        } else {
                            l.t("Move down", "Descendre")
                        },
                        can_edit,
                        false,
                    )
                    .clicked()
                    {
                        act = Some(("bookmark.set", json!({"bookmark": row.name, "index": target})));
                    }
                };
                step(-1, ui);
                step(1, ui);
            });
            // Desktop's three checkboxes, for the bookmark selected.
            if let Some(row) = primary_row.filter(|r| !r.group && can_edit) {
                ui.horizontal_wrapped(|ui| {
                    let mut data = row.data;
                    if ui
                        .checkbox(&mut data, l.t("Data", "Données"))
                        .on_hover_text(l.t("Apply the bookmark's filters", "Appliquer les filtres du signet"))
                        .changed()
                    {
                        act = Some(("bookmark.set", json!({"bookmark": row.name, "data": data})));
                    }
                    let mut display = row.display;
                    if ui
                        .checkbox(&mut display, l.t("Display", "Affichage"))
                        .on_hover_text(l.t(
                            "Apply the bookmark's hidden visuals",
                            "Appliquer les visuels masqués du signet",
                        ))
                        .changed()
                    {
                        act = Some(("bookmark.set", json!({"bookmark": row.name, "display": display})));
                    }
                    let mut current_page = row.current_page;
                    if ui
                        .checkbox(&mut current_page, l.t("Current page", "Page active"))
                        .on_hover_text(l.t("Go to the bookmark's page", "Aller à la page du signet"))
                        .changed()
                    {
                        act = Some((
                            "bookmark.set",
                            json!({"bookmark": row.name, "current_page": current_page}),
                        ));
                    }
                });
            }
        });

    let mut renaming = app.bookmarks.renaming.take();
    let focus = app.bookmarks.focus;
    app.bookmarks.focus = false;

    egui::ScrollArea::vertical()
        .id_salt("bicraft-bookmarks-rows")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for row in &rows {
                ui.horizontal(|ui| {
                    ui.add_space(if row.parent.is_some() { 24.0 } else { 0.0 });
                    if row.group {
                        let tip = if row.folded {
                            l.t("Expand the group", "Développer le groupe")
                        } else {
                            l.t("Collapse the group", "Réduire le groupe")
                        };
                        let icon = if row.folded {
                            Icon::ChevronRight
                        } else {
                            Icon::ChevronDown
                        };
                        if widgets::icon_toggle(ui, icon, tip, true, false).clicked() {
                            fold = Some(row.name.to_lowercase());
                        }
                    }
                    let editing = renaming.as_ref().is_some_and(|(n, _)| n == &row.name);
                    if editing {
                        let mut text = renaming.as_ref().map(|(_, s)| s.clone()).unwrap_or_default();
                        let edit = ui.add(
                            egui::TextEdit::singleline(&mut text)
                                .desired_width((ui.available_width() - 34.0).max(60.0)),
                        );
                        let edit_label = format!("{} {}", l.t("Rename", "Renommer"), row.label);
                        edit.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::Role::TextInput, true, &edit_label)
                        });
                        if focus {
                            edit.request_focus();
                        }
                        let enter = ui.input(|i| i.key_pressed(Key::Enter));
                        let esc = ui.input(|i| i.key_pressed(Key::Escape));
                        if enter && !text.trim().is_empty() {
                            let id = if row.group {
                                "bookmark.rename_group"
                            } else {
                                "bookmark.set"
                            };
                            let key = if row.group { "group" } else { "bookmark" };
                            act = Some((id, json!({key: row.name, "display_name": text.trim()})));
                            end_rename = true;
                        } else if esc || (edit.lost_focus() && !enter) {
                            end_rename = true;
                        } else {
                            renaming = Some((row.name.clone(), text));
                        }
                    } else {
                        let mut text = RichText::new(&row.label).color(t.text);
                        if row.group || row.selected {
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
                            select = Some((row.name.clone(), add));
                        }
                        if resp.double_clicked() {
                            start_rename = Some((row.name.clone(), row.label.clone()));
                        }
                        let detail = match (&row.page, row.group) {
                            (_, true) => String::new(),
                            (Some(page), false) => page.to_string(),
                            (None, false) => String::new(),
                        };
                        resp.on_hover_text(detail);
                    }
                    if !row.group {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let mut notes = Vec::new();
                            if row.hidden > 0 {
                                notes.push(format!("{} {}", row.hidden, l.t("hidden", "masqué(s)")));
                            }
                            if row.filters > 0 {
                                notes.push(format!("{} {}", row.filters, l.t("filters", "filtres")));
                            }
                            if !notes.is_empty() {
                                ui.label(RichText::new(notes.join(" · ")).color(t.text_dim).small());
                            }
                        });
                    }
                });
            }
        });
    if let Some(group) = fold
        && !app.bookmarks.folded.remove(&group)
    {
        app.bookmarks.folded.insert(group);
    }
    if let Some(pair) = start_rename {
        app.bookmarks.renaming = Some(pair);
        app.bookmarks.focus = true;
    } else if end_rename {
        app.bookmarks.renaming = None;
    } else if let Some(current) = renaming {
        app.bookmarks.renaming = Some(current);
    }
    if let Some((name, add)) = select {
        let mut names = if add {
            app.bookmarks.selected.clone()
        } else {
            Vec::new()
        };
        if add && names.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
            names.retain(|n| !n.eq_ignore_ascii_case(&name));
        } else {
            names.push(name);
        }
        app.bookmarks.selected = names;
    }
    if let Some((id, params)) = act {
        exec(app, id, params);
    }
}
