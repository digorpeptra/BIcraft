//! The Selection pane, as Power BI Desktop's (ROADMAP M11.23): the visuals of the page shown
//! from front to back, groups nested and foldable, with an eye that shows or hides a visual, a
//! click that selects it on the canvas (Ctrl or Shift adds it), a double click that renames a
//! group (`visual.rename_group`), and the layer and tab-order moves at the bottom. Every edit is
//! an engine command (one undo step, [`crate::panes::exec`]); the pane is pure view state
//! otherwise (folded groups, the tab-order mode, the rename being typed).

use std::collections::{BTreeSet, HashMap};

use egui::{Align, Key, Layout, RichText, Sense, Ui, vec2};
use serde_json::{Value, json};

use crate::App;
use crate::canvas;
use crate::i18n::Lang;
use crate::icons::Icon;
use crate::panes::exec;
use crate::ribbon;
use crate::theme::Tokens;
use crate::widgets;

/// Most visuals the pane lists (as the canvas: a report page holds no more).
const MAX_ROWS: usize = 2000;
/// How deep the pane indents nested groups.
const MAX_DEPTH: usize = 16;

/// The pane's view state, kept by the window for every document.
#[derive(Debug, Default)]
pub struct SelectionUi {
    /// Groups whose members are folded away.
    pub folded: BTreeSet<String>,
    /// The group being renamed and the text typed so far (the pane asks for the focus once).
    pub renaming: Option<(String, String)>,
    /// The Tab order mode: the visuals listed in keyboard order instead of layer order.
    pub tab_order: bool,
    /// The rename editor still has to take the focus.
    focus: bool,
}

/// One listed visual.
struct Row {
    name: String,
    label: String,
    group: bool,
    hidden: bool,
    depth: usize,
    /// The visuals listed under this group, folded or not.
    foldable: bool,
    selected: bool,
    tab: Option<i64>,
}

/// The page shown and its rows, in the pane's order.
fn rows(app: &App, l: Lang) -> Option<(String, Vec<Row>)> {
    let snap = &app.loaded.as_ref()?.snap;
    let page = snap.pointer(&format!("/report/pages/{}", app.active_page()))?;
    let page_name = page.get("name")?.as_str()?.to_string();
    let visuals: Vec<&Value> = page
        .get("visuals")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(MAX_ROWS)
        .filter(|v| v.is_object())
        .collect();
    let selected = canvas::selection(app);
    let lower = |s: &str| s.to_lowercase();
    // Children of each parent (a visual group's name), front to back.
    let mut children: HashMap<String, Vec<usize>> = HashMap::new();
    let mut top: Vec<usize> = Vec::new();
    for (i, v) in visuals.iter().enumerate() {
        match v.get("parent_group").and_then(Value::as_str) {
            Some(p) if !p.is_empty() => children.entry(lower(p)).or_default().push(i),
            _ => top.push(i),
        }
    }
    let z = |i: usize| {
        visuals
            .get(i)
            .and_then(|v| v.get("z"))
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
    };
    // Front to back: the highest z first; ties keep the file's later visual in front.
    let front_to_back = |v: &mut Vec<usize>| {
        v.sort_by(|&a, &b| {
            z(b).partial_cmp(&z(a))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.cmp(&a))
        });
    };
    front_to_back(&mut top);
    for list in children.values_mut() {
        front_to_back(list);
    }
    let label = |v: &Value| -> String {
        let title = v
            .get("title")
            .and_then(Value::as_str)
            .filter(|t| !t.trim().is_empty());
        let ty = v.get("type").and_then(Value::as_str);
        title
            .map(str::to_string)
            .or_else(|| {
                ty.and_then(|t| crate::panes::type_label(t, l))
                    .map(str::to_string)
            })
            .or_else(|| ty.map(str::to_string))
            .unwrap_or_else(|| l.t("Group", "Groupe").to_string())
    };
    let make = |i: usize, depth: usize| -> Option<Row> {
        let v = visuals.get(i)?;
        let name = v.get("name")?.as_str()?.to_string();
        let group = v.get("type").is_none_or(Value::is_null);
        Some(Row {
            hidden: v.get("hidden").and_then(Value::as_bool).unwrap_or(false),
            label: label(v),
            group,
            foldable: children.contains_key(&lower(&name)),
            depth,
            selected: selected.iter().any(|s| s.eq_ignore_ascii_case(&name)),
            tab: v.get("tab_order").and_then(Value::as_i64),
            name,
        })
    };
    let mut out = Vec::new();
    if app.selection.tab_order {
        // Keyboard order: a flat list by tabOrder (visuals without one last).
        let mut flat: Vec<usize> = (0..visuals.len()).collect();
        flat.sort_by(|&a, &b| {
            let t = |i: usize| {
                visuals
                    .get(i)
                    .and_then(|v| v.get("tab_order"))
                    .and_then(Value::as_i64)
            };
            t(a).is_some_and(|x| x >= 0)
                .cmp(&t(b).is_some_and(|x| x >= 0))
                .reverse()
                .then(t(a).unwrap_or(i64::MAX).cmp(&t(b).unwrap_or(i64::MAX)))
                .then(b.cmp(&a))
        });
        for i in flat {
            if let Some(row) = make(i, 0) {
                out.push(row);
            }
        }
        return Some((page_name, out));
    }
    // Layer order: the page's top level, each group's members under it.
    fn walk(
        list: &[usize],
        depth: usize,
        children: &HashMap<String, Vec<usize>>,
        folded: &BTreeSet<String>,
        make: &dyn Fn(usize, usize) -> Option<Row>,
        out: &mut Vec<Row>,
    ) {
        if depth > MAX_DEPTH {
            return;
        }
        for &i in list {
            let Some(row) = make(i, depth) else { continue };
            let name = row.name.to_lowercase();
            let folded_here = folded.contains(&name);
            out.push(row);
            if folded_here {
                continue;
            }
            if let Some(kids) = children.get(&name) {
                walk(kids, depth + 1, children, folded, make, out);
            }
        }
    }
    walk(&top, 0, &children, &app.selection.folded, &make, &mut out);
    Some((page_name, out))
}

/// The pane's body (the panel's frame, title and width belong to `crate::panes`).
pub fn show(app: &mut App, ui: &mut Ui) {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    if crate::panes::title_row(
        ui,
        l.t("Selection", "Sélection"),
        l.t("Collapse the Selection pane", "Réduire le volet Sélection"),
    ) {
        app.prefs.panes.selection = false;
        app.save_prefs();
        return;
    }
    let Some((page, rows)) = rows(app, l) else {
        ui.label(
            RichText::new(l.t(
                "Open a report to select its visuals.",
                "Ouvrez un rapport pour sélectionner ses visuels.",
            ))
            .color(t.text_dim),
        );
        return;
    };
    if rows.is_empty() {
        ui.label(
            RichText::new(l.t("This page has no visuals.", "Cette page n'a aucun visuel.")).color(t.text_dim),
        );
    }
    let edit = ribbon::state(app, "visual.set_hidden").is_ok();
    let rename_ok = ribbon::state(app, "visual.rename_group").is_ok();
    let mut act: Option<(&'static str, Value)> = None;
    let mut select: Option<(String, bool)> = None;
    let mut fold: Option<String> = None;
    let mut start_rename: Option<(String, String)> = None;
    let mut end_rename = false;

    // The layer and tab-order buttons sit at the pane's foot, in a bottom panel so a short
    // window never clips them; the list scrolls between the title row and them.
    let sel = canvas::selection(app);
    let primary = sel.first().cloned();
    let is_group = primary
        .as_deref()
        .and_then(|n| rows.iter().find(|r| r.name == n))
        .is_some_and(|r| r.group);
    egui::Panel::bottom("bicraft-selection-actions")
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                let order_ok = !sel.is_empty() && ribbon::state(app, "visual.order").is_ok();
                if widgets::small_button(
                    ui,
                    Icon::ChevronUp,
                    l.t("Move forward", "Avancer"),
                    order_ok,
                    false,
                )
                .clicked()
                {
                    act = Some((
                        "visual.order",
                        json!({"page": page, "visuals": sel, "to": "forward"}),
                    ));
                }
                if widgets::small_button(
                    ui,
                    Icon::ChevronDown,
                    l.t("Move backward", "Reculer"),
                    order_ok,
                    false,
                )
                .clicked()
                {
                    act = Some((
                        "visual.order",
                        json!({"page": page, "visuals": sel, "to": "backward"}),
                    ));
                }
                let group_ok = sel.len() >= 2 && ribbon::state(app, "visual.group").is_ok();
                if widgets::small_button(ui, Icon::Group, l.t("Group", "Grouper"), group_ok, false).clicked()
                {
                    act = Some(("visual.group", json!({"page": page, "visuals": sel})));
                }
                let ungroup_ok = primary.is_some() && ribbon::state(app, "visual.ungroup").is_ok();
                if widgets::small_button(ui, Icon::Ungroup, l.t("Ungroup", "Dissocier"), ungroup_ok, false)
                    .clicked()
                    && let Some(name) = &primary
                {
                    act = Some(("visual.ungroup", json!({"page": page, "visual": name})));
                }
                if widgets::small_button(
                    ui,
                    Icon::Rename,
                    l.t("Rename group", "Renommer le groupe"),
                    is_group && rename_ok,
                    false,
                )
                .clicked()
                    && let Some(row) = primary.as_deref().and_then(|n| rows.iter().find(|r| r.name == n))
                {
                    app.selection.renaming = Some((row.name.clone(), row.label.clone()));
                    app.selection.focus = true;
                }
            });
            // Tab order: the keyboard order of every visual of the page, moved one step at a time.
            let total = rows.iter().filter(|_| app.selection.tab_order).count();
            ui.horizontal_wrapped(|ui| {
                if widgets::small_button(
                    ui,
                    Icon::Sort,
                    l.t("Tab order", "Ordre de tabulation"),
                    ribbon::state(app, "visual.tab_order").is_ok(),
                    app.selection.tab_order,
                )
                .clicked()
                {
                    app.selection.tab_order = !app.selection.tab_order;
                }
                if app.selection.tab_order && total > 1 {
                    let mut step = |dir: i64, ui: &mut Ui| {
                        let Some(row) = primary
                            .as_deref()
                            .and_then(|n| rows.iter().position(|r| r.name == n))
                        else {
                            return;
                        };
                        let target = row as i64 + dir;
                        if target < 0 || target as usize >= rows.len() {
                            return;
                        }
                        let mut names: Vec<String> = rows.iter().map(|r| r.name.clone()).collect();
                        names.swap(row, target as usize);
                        if widgets::icon_toggle(
                            ui,
                            if dir < 0 {
                                Icon::ChevronUp
                            } else {
                                Icon::ChevronDown
                            },
                            if dir < 0 {
                                l.t("Earlier in the tab order", "Plus tôt dans l'ordre")
                            } else {
                                l.t("Later in the tab order", "Plus tard dans l'ordre")
                            },
                            ribbon::state(app, "visual.tab_order").is_ok(),
                            false,
                        )
                        .clicked()
                        {
                            act = Some(("visual.tab_order", json!({"page": page, "visuals": names})));
                        }
                    };
                    step(-1, ui);
                    step(1, ui);
                }
            });
        });

    let mut renaming = app.selection.renaming.take();
    let focus = app.selection.focus;
    app.selection.focus = false;

    egui::ScrollArea::vertical()
        .id_salt("bicraft-selection-rows")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for row in &rows {
                let collapsed = app.selection.folded.contains(&row.name.to_lowercase());
                ui.horizontal(|ui| {
                    ui.add_space((row.depth as f32 * 12.0).min(96.0));
                    if row.foldable {
                        let tip = if collapsed {
                            l.t("Expand the group", "Développer le groupe")
                        } else {
                            l.t("Collapse the group", "Réduire le groupe")
                        };
                        let icon = if collapsed {
                            Icon::ChevronRight
                        } else {
                            Icon::ChevronDown
                        };
                        if widgets::icon_toggle(ui, icon, tip, true, false).clicked() {
                            fold = Some(row.name.to_lowercase());
                        }
                    } else {
                        ui.add_space(24.0);
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
                            egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &edit_label)
                        });
                        if focus {
                            edit.request_focus();
                        }
                        let enter = ui.input(|i| i.key_pressed(Key::Enter));
                        let esc = ui.input(|i| i.key_pressed(Key::Escape));
                        if enter && !text.trim().is_empty() {
                            act = Some((
                                "visual.rename_group",
                                json!({"page": page, "group": row.name, "display_name": text.trim()}),
                            ));
                            end_rename = true;
                        } else if esc || (edit.lost_focus() && !enter) {
                            end_rename = true;
                        } else {
                            renaming = Some((row.name.clone(), text));
                        }
                    } else {
                        let color = if row.hidden { t.text_faint } else { t.text };
                        let mut text = RichText::new(&row.label).color(color);
                        if row.selected {
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
                        if resp.double_clicked() && row.group && rename_ok {
                            start_rename = Some((row.name.clone(), row.label.clone()));
                        }
                        resp.on_hover_text(&row.name);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let tip = if row.hidden {
                            format!("{} {}", l.t("Show", "Afficher"), row.label)
                        } else {
                            format!("{} {}", l.t("Hide", "Masquer"), row.label)
                        };
                        let icon = if row.hidden { Icon::EyeOff } else { Icon::Eye };
                        if widgets::icon_toggle(ui, icon, &tip, edit, false).clicked() {
                            act = Some((
                                "visual.set_hidden",
                                json!({"page": page, "visual": row.name, "hidden": !row.hidden}),
                            ));
                        }
                        if app.selection.tab_order {
                            let tab = row.tab.map_or_else(|| "—".to_string(), |t| t.to_string());
                            ui.label(RichText::new(tab).color(t.text_dim).small());
                        }
                    });
                });
            }
        });
    if let Some(group) = fold
        && !app.selection.folded.remove(&group)
    {
        app.selection.folded.insert(group);
    }
    if let Some(pair) = start_rename {
        app.selection.renaming = Some(pair);
        app.selection.focus = true;
    } else if end_rename {
        app.selection.renaming = None;
    } else if let Some(current) = renaming {
        app.selection.renaming = Some(current);
    }
    if let Some((name, add)) = select {
        let mut names = if add { canvas::selection(app) } else { Vec::new() };
        if add && names.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
            names.retain(|n| !n.eq_ignore_ascii_case(&name));
        } else {
            names.push(name);
        }
        canvas::select(app, &page, &names);
    }
    if let Some((id, params)) = act {
        exec(app, id, params);
    }
}
