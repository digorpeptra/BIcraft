//! The report's page tabs at the bottom of the Report view, as in Power BI Desktop: the active
//! page, hidden pages dimmed with a crossed eye, an issue badge (`⚠ 2`, `✕ 1`), `+` to add a page,
//! a menu on right-click (Rename inline, Duplicate, Delete with a confirmation, Hide / Show, page
//! type, Move left / right), drag to reorder, and the keyboard: Enter or Space opens the focused
//! tab, F2 renames it, Delete deletes it, Alt+Shift+← / → move it. Every action runs the engine's
//! `page.*` command ([`crate::actions`]) on the document's source, whose next snapshot redraws
//! the tabs.

use egui::{Align2, CornerRadius, Key, Modifiers, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use serde_json::{Value, json};

use crate::App;
use crate::actions::{self, Call, PAGE_TYPES};
use crate::app::{Confirm, Then, clip};
use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};

/// Height of the tab strip.
pub const TABS_H: f32 = 32.0;
/// Longest page name drawn on a tab (characters).
const MAX_LABEL: usize = 40;

/// The engine commands the page tab's menu runs (the registry's page-tab menu, ROADMAP M11;
/// Duplicate is `report.extract` then `report.apply`, on the Report tab too).
pub const MENU: &[&str] = &[
    "page.rename",
    "page.remove",
    "page.set_type",
    "page.move",
    "page.set_active",
];

/// A page as its tab shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct TabPage {
    pub name: String,
    pub label: String,
    pub hidden: bool,
    /// `report`, `tooltip`, `drillthrough`.
    pub kind: String,
    pub errors: usize,
    pub warnings: usize,
}

/// The pages of a snapshot (at most 1,000, as the page list).
pub fn tab_pages(snap: &Value) -> Vec<TabPage> {
    snap.pointer("/report/pages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(1000)
        .map(|p| {
            let s = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            let name = s("name");
            let label = Some(s("display_name"))
                .filter(|d| !d.is_empty())
                .unwrap_or_else(|| name.clone());
            let (mut errors, mut warnings) = (0usize, 0usize);
            let mut count = |issues: Option<&Value>| {
                for i in issues
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .take(10_000)
                {
                    match i.get("severity").and_then(Value::as_str) {
                        Some("warning" | "warn") => warnings = warnings.saturating_add(1),
                        _ => errors = errors.saturating_add(1),
                    }
                }
            };
            count(p.get("issues"));
            for v in p
                .get("visuals")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .take(10_000)
            {
                count(v.get("issues"));
            }
            TabPage {
                name,
                label,
                hidden: p.get("hidden").and_then(Value::as_bool).unwrap_or(false),
                kind: Some(s("page_type"))
                    .filter(|k| !k.is_empty())
                    .unwrap_or_else(|| "report".into()),
                errors,
                warnings,
            }
        })
        .collect()
}

/// The strip's state between frames.
#[derive(Debug, Clone, Default)]
pub struct PageTabs {
    /// The page being renamed and the text typed so far.
    pub renaming: Option<(String, String)>,
    /// Renaming started this frame (the field takes the focus).
    rename_fresh: bool,
    /// The tab being dragged.
    pub dragging: Option<usize>,
}

/// The slot (0..=n) a drag over `x` drops into, from the tabs' rectangles.
pub fn drop_slot(rects: &[Rect], x: f32) -> usize {
    rects.iter().take_while(|r| x > r.center().x).count()
}

/// The `page.move` index for a tab moved from `from` to drop slot `slot` (`None`: no move).
pub fn move_index(from: usize, slot: usize) -> Option<usize> {
    let to = if slot > from { slot - 1 } else { slot };
    (to != from).then_some(to)
}

/// What a tab asked for, applied after the strip is drawn.
enum Act {
    Show(String),
    Rename(String, String),
    StartRename(String, String),
    Duplicate(String, String),
    Delete(String, String),
    Hide(String, String, bool),
    Kind(String, String),
    Move(String, usize),
    SetActive(String),
    Add,
}

/// Draws the strip (inside the Report view's bottom panel).
pub fn show(app: &mut App, ui: &mut Ui) {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    let Some(loaded) = app.loaded.clone() else { return };
    let pages = tab_pages(&loaded.snap);
    let active = app.active_page();
    let n = pages.len();
    let mut act: Option<Act> = None;
    let mut rects: Vec<Rect> = Vec::with_capacity(n);
    let mut st = std::mem::take(&mut app.page_tabs);
    let rename_fresh = std::mem::replace(&mut st.rename_fresh, false);
    let taken: Vec<String> = pages.iter().map(|p| p.label.clone()).collect();
    let dragging_pos = ui.input(|i| i.pointer.interact_pos());
    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let plus_w = 30.0;
        let avail = (ui.available_width() - plus_w - 8.0).max(60.0);
        egui::ScrollArea::horizontal()
            .id_salt("bicraft-page-tabs")
            .max_width(avail)
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for (i, p) in pages.iter().enumerate() {
                        let on = i == active;
                        if let Some((rn, buf)) = &mut st.renaming
                            && *rn == p.name
                        {
                            let r = ui.add(
                                egui::TextEdit::singleline(buf)
                                    .char_limit(200)
                                    .desired_width(140.0)
                                    .font(theme::font(13.0)),
                            );
                            if rename_fresh {
                                r.request_focus();
                            }
                            rects.push(r.rect);
                            let (enter, esc) =
                                ui.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
                            if esc {
                                act = Some(Act::Rename(String::new(), String::new()));
                            } else if (enter || r.lost_focus()) && !rename_fresh {
                                act = Some(Act::Rename(p.name.clone(), buf.trim().to_string()));
                            }
                            continue;
                        }
                        let label = clip(&p.label, MAX_LABEL);
                        let font = if on { theme::bold(13.0) } else { theme::font(13.0) };
                        let text_w = ui.fonts_mut(|f| {
                            f.layout_no_wrap(label.clone(), theme::bold(13.0), t.text)
                                .size()
                                .x
                        });
                        let badge = if p.errors > 0 {
                            Some((format!("✕ {}", p.errors.min(999)), t.err))
                        } else if p.warnings > 0 {
                            Some((format!("⚠ {}", p.warnings.min(999)), t.warn))
                        } else {
                            None
                        };
                        let badge_w = badge.as_ref().map_or(0.0, |(s, _)| {
                            ui.fonts_mut(|f| f.layout_no_wrap(s.clone(), theme::font(11.0), t.text).size().x)
                                + 10.0
                        });
                        let eye_w = if p.hidden { 18.0 } else { 0.0 };
                        let w = text_w + 24.0 + badge_w + eye_w;
                        let (rect, resp) =
                            ui.allocate_exact_size(vec2(w, TABS_H - 6.0), Sense::click_and_drag());
                        rects.push(rect);
                        // a page whose snapshot gives no name still has an accessible one
                        let name = if p.label.is_empty() {
                            l.t("Page without a name", "Page sans nom")
                        } else {
                            p.label.as_str()
                        };
                        let a11y = if p.hidden {
                            format!("{name} ({})", l.t("hidden", "masquée"))
                        } else {
                            name.to_string()
                        };
                        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, &a11y));
                        let painter = ui.painter();
                        if on {
                            painter.rect_filled(rect, CornerRadius::same(4), t.card);
                            painter.rect_filled(
                                Rect::from_min_size(
                                    rect.left_bottom() - vec2(-6.0, 3.0),
                                    vec2(rect.width() - 12.0, 3.0),
                                ),
                                CornerRadius::same(1),
                                t.accent,
                            );
                        } else if resp.hovered() || resp.has_focus() {
                            painter.rect_filled(rect, CornerRadius::same(4), t.hover);
                        }
                        if resp.has_focus() {
                            painter.rect_stroke(
                                rect,
                                CornerRadius::same(4),
                                Stroke::new(1.5, t.ring),
                                StrokeKind::Inside,
                            );
                        }
                        let mut x = rect.left() + 12.0;
                        if p.hidden {
                            icons::paint(
                                painter,
                                Rect::from_center_size(pos2(x + 7.0, rect.center().y), vec2(14.0, 14.0)),
                                Icon::EyeOff,
                                t.text_dim,
                                false,
                            );
                            x += eye_w;
                        }
                        painter.text(
                            pos2(x, rect.center().y),
                            Align2::LEFT_CENTER,
                            &label,
                            font,
                            if p.hidden { t.text_faint } else { t.text },
                        );
                        if let Some((s, c)) = &badge {
                            painter.text(
                                pos2(rect.right() - 8.0, rect.center().y),
                                Align2::RIGHT_CENTER,
                                s,
                                theme::font(11.0),
                                *c,
                            );
                        }
                        let tip = match p.kind.as_str() {
                            "tooltip" => {
                                format!("{} · {}", p.label, l.t("tooltip page", "page d'info-bulle"))
                            }
                            "drillthrough" => {
                                format!("{} · {}", p.label, l.t("drillthrough page", "page d'extraction"))
                            }
                            _ => p.label.clone(),
                        };
                        let resp = resp.on_hover_text(tip);
                        // the mouse
                        if resp.clicked() && !on {
                            act = Some(Act::Show(p.name.clone()));
                        }
                        if resp.double_clicked() {
                            act = Some(Act::StartRename(p.name.clone(), p.label.clone()));
                        }
                        if resp.drag_started() {
                            st.dragging = Some(i);
                        }
                        // the keyboard
                        if resp.has_focus() {
                            let (enter, f2, del, left, right) = ui.input(|inp| {
                                let ms = Modifiers::ALT | Modifiers::SHIFT;
                                (
                                    inp.key_pressed(Key::Enter) || inp.key_pressed(Key::Space),
                                    inp.key_pressed(Key::F2),
                                    inp.key_pressed(Key::Delete),
                                    inp.modifiers.matches_exact(ms) && inp.key_pressed(Key::ArrowLeft),
                                    inp.modifiers.matches_exact(ms) && inp.key_pressed(Key::ArrowRight),
                                )
                            });
                            if enter && !on {
                                act = Some(Act::Show(p.name.clone()));
                            } else if f2 {
                                act = Some(Act::StartRename(p.name.clone(), p.label.clone()));
                            } else if del && n > 1 {
                                act = Some(Act::Delete(p.name.clone(), p.label.clone()));
                            } else if left && i > 0 {
                                act = Some(Act::Move(p.name.clone(), i - 1));
                            } else if right && i + 1 < n {
                                act = Some(Act::Move(p.name.clone(), i + 1));
                            }
                        }
                        // the menu
                        crate::keyboard::context_menu(&resp, |ui| {
                            ui.set_min_width(180.0);
                            if ui.button(l.t("Rename", "Renommer")).clicked() {
                                act = Some(Act::StartRename(p.name.clone(), p.label.clone()));
                                ui.close();
                            }
                            if ui.button(l.t("Duplicate", "Dupliquer")).clicked() {
                                act = Some(Act::Duplicate(
                                    p.name.clone(),
                                    actions::copy_name(&p.label, &taken),
                                ));
                                ui.close();
                            }
                            if ui
                                .add_enabled(n > 1, egui::Button::new(l.t("Delete", "Supprimer")))
                                .clicked()
                            {
                                act = Some(Act::Delete(p.name.clone(), p.label.clone()));
                                ui.close();
                            }
                            ui.separator();
                            let hide = if p.hidden {
                                l.t("Show page", "Afficher la page")
                            } else {
                                l.t("Hide page", "Masquer la page")
                            };
                            if ui.button(hide).clicked() {
                                act = Some(Act::Hide(p.name.clone(), p.kind.clone(), !p.hidden));
                                ui.close();
                            }
                            ui.menu_button(l.t("Page type", "Type de page"), |ui| {
                                for (kind, en, fr) in PAGE_TYPES {
                                    if ui.radio(p.kind == kind, l.t(en, fr)).clicked() {
                                        if p.kind != kind {
                                            act = Some(Act::Kind(p.name.clone(), kind.to_string()));
                                        }
                                        ui.close();
                                    }
                                }
                            });
                            ui.separator();
                            if ui
                                .add_enabled(i > 0, egui::Button::new(l.t("Move left", "Déplacer à gauche")))
                                .clicked()
                            {
                                act = Some(Act::Move(p.name.clone(), i.saturating_sub(1)));
                                ui.close();
                            }
                            if ui
                                .add_enabled(
                                    i + 1 < n,
                                    egui::Button::new(l.t("Move right", "Déplacer à droite")),
                                )
                                .clicked()
                            {
                                act = Some(Act::Move(p.name.clone(), i + 1));
                                ui.close();
                            }
                            ui.separator();
                            let open_here = crate::registry::find("page.set_active");
                            if let Some(c) = open_here {
                                let st = crate::ribbon::state(app, c.id);
                                if crate::menus::item(ui, c.text(l), &st, l) {
                                    act = Some(Act::SetActive(p.name.clone()));
                                    ui.close();
                                }
                            }
                        });
                    }
                });
            });
        // `+`: a new page at the end
        let (r, resp) = ui.allocate_exact_size(vec2(plus_w, TABS_H - 6.0), Sense::click());
        let label = l.t("Add a page", "Ajouter une page");
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
        if resp.hovered() || resp.has_focus() {
            ui.painter().rect_filled(r, CornerRadius::same(4), t.hover);
        }
        icons::paint(
            ui.painter(),
            Rect::from_center_size(r.center(), vec2(16.0, 16.0)),
            Icon::Plus,
            t.text,
            false,
        );
        if resp.on_hover_text(label).clicked() {
            act = Some(Act::Add);
        }
    });
    // a drag: the insertion mark while it moves, the move when it ends
    if let Some(from) = st.dragging {
        let released = ui.input(|i| i.pointer.any_released() || !i.pointer.any_down());
        if let Some(pos) = dragging_pos {
            let slot = drop_slot(&rects, pos.x);
            let x = rects
                .get(slot)
                .map(|r| r.left() - 1.0)
                .or_else(|| rects.last().map(|r| r.right() + 1.0));
            if let (Some(x), Some(first)) = (x, rects.first()) {
                ui.painter().line_segment(
                    [pos2(x, first.top()), pos2(x, first.bottom())],
                    Stroke::new(2.0, t.ring),
                );
            }
            if released {
                if let (Some(to), Some(p)) = (move_index(from, slot), pages.get(from)) {
                    act = Some(Act::Move(p.name.clone(), to));
                }
                st.dragging = None;
            }
        } else if released {
            st.dragging = None;
        }
    }
    app.page_tabs = st;
    match act {
        None => {}
        Some(Act::Show(name)) => app.interact(json!({"action": "page", "page": name})),
        Some(Act::StartRename(name, label)) => {
            app.page_tabs.renaming = Some((name, label));
            app.page_tabs.rename_fresh = true;
        }
        Some(Act::Rename(name, new)) => {
            let old = pages.iter().find(|p| p.name == name).map(|p| p.label.clone());
            app.page_tabs.renaming = None;
            if !name.is_empty() && !new.is_empty() && old.as_deref() != Some(new.as_str()) {
                app.run(actions::page_rename(&name, &new), Then::Nothing);
            }
        }
        Some(Act::Duplicate(name, copy)) => {
            let key = format!("copy-of-{name}-{}", app.next_tag());
            app.run(actions::page_extract(&name), Then::Duplicate { key, name: copy });
        }
        Some(Act::Delete(name, label)) => app.confirm = Some(Confirm::DeletePage { name, label }),
        Some(Act::Hide(name, kind, hidden)) => {
            app.run(actions::page_set_type(&name, &kind, Some(hidden)), Then::Nothing);
        }
        Some(Act::Kind(name, kind)) => {
            app.run(actions::page_set_type(&name, &kind, None), Then::Nothing);
        }
        Some(Act::Move(name, to)) => {
            app.run(actions::page_move(&name, to), Then::Nothing);
        }
        Some(Act::SetActive(name)) => {
            app.run(
                Call {
                    id: "page.set_active",
                    params: json!({"page": name}),
                },
                Then::Nothing,
            );
        }
        Some(Act::Add) => {
            let name = actions::new_page_name(l.t("Page", "Page"), &taken);
            app.run(actions::page_add(&name, n), Then::ShowPage);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_and_moves() {
        let rects: Vec<Rect> = (0..3)
            .map(|i| Rect::from_min_size(pos2(i as f32 * 100.0, 0.0), vec2(90.0, 20.0)))
            .collect();
        assert_eq!(drop_slot(&rects, -5.0), 0);
        assert_eq!(drop_slot(&rects, 60.0), 1);
        assert_eq!(drop_slot(&rects, 1000.0), 3);
        assert_eq!(drop_slot(&[], 10.0), 0);
        assert_eq!(move_index(0, 3), Some(2));
        assert_eq!(move_index(2, 0), Some(0));
        assert_eq!(move_index(1, 1), None);
        assert_eq!(move_index(1, 2), None);
        assert_eq!(move_index(0, 2), Some(1));
    }

    #[test]
    fn pages_of_a_snapshot() {
        let snap = json!({"report": {"pages": [
            {"name": "a", "display_name": "", "hidden": true, "page_type": "tooltip",
             "issues": [{"severity": "warning"}], "visuals": [{"issues": [{"severity": "error"}, {"severity": "warning"}]}]},
            {"name": "b", "display_name": "B"},
            7
        ]}});
        let p = tab_pages(&snap);
        assert_eq!(p.len(), 3);
        assert_eq!(
            (p[0].label.as_str(), p[0].hidden, p[0].kind.as_str()),
            ("a", true, "tooltip")
        );
        assert_eq!((p[0].errors, p[0].warnings), (1, 2));
        assert_eq!((p[1].label.as_str(), p[1].kind.as_str()), ("B", "report"));
        assert!(tab_pages(&json!(null)).is_empty());
    }
}
