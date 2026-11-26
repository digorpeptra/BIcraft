//! The chrome's widgets: ribbon buttons and groups, icon toggles, the primary pill, pills, the
//! tab row, pane sections, the error bar (docs/design.md §7).
//!
//! Ported from DeckCraft `crates/ui-egui/src/widgets.rs` and `status.rs` @ 0ec0cc5 (MIT OR
//! Apache-2.0, © 2026 ArtCraft Team and the DeckCraft contributors): `bg`, `big_button`,
//! `wrap_label`, `chevron`, `small_button`, `icon_toggle`, `group`, `separator`, `section`,
//! `small_icon`. New for BIcraft: `primary`, `pill`, `tab`, `error_bar`, `card`, `switch`.

use egui::{Align2, Color32, CornerRadius, Rect, Response, Sense, Stroke, Ui, pos2, vec2};

use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};

/// Height of the ribbon's content area.
pub const RIBBON_H: f32 = 66.0;

fn bg(ui: &Ui, r: &Response, selected: bool, t: &Tokens) -> Option<Color32> {
    if !r.sense.senses_click() {
        return None;
    }
    if selected {
        Some(t.selected)
    } else if r.is_pointer_button_down_on() {
        Some(t.pressed)
    } else if r.hovered() && ui.is_enabled() {
        Some(t.hover)
    } else {
        None
    }
}

fn text_w(ui: &Ui, s: &str, font: egui::FontId) -> f32 {
    ui.fonts_mut(|f| f.layout_no_wrap(s.to_string(), font, Color32::WHITE).size().x)
}

/// A large ribbon button: a 32 px icon over a one- or two-line label.
pub fn big_button(ui: &mut Ui, icon: Icon, label: &str, enabled: bool) -> Response {
    big(ui, icon, label, enabled, None)
}

/// A ribbon button that stays pressed while its mode is on (`selected`), big or small.
pub fn toggle_button(
    ui: &mut Ui,
    icon: Icon,
    label: &str,
    enabled: bool,
    selected: bool,
    large: bool,
) -> Response {
    if large {
        big(ui, icon, label, enabled, Some(selected))
    } else {
        small_button(ui, icon, label, enabled, selected)
    }
}

fn big(ui: &mut Ui, icon: Icon, label: &str, enabled: bool, toggle: Option<bool>) -> Response {
    let selected = toggle.unwrap_or(false);
    let t = Tokens::get(ui.ctx());
    let font = theme::font(11.0);
    let lines = wrap_label(label);
    let w = lines
        .iter()
        .map(|l| text_w(ui, l, font.clone()))
        .fold(0.0f32, f32::max);
    let w = (w + 10.0).max(40.0);
    let (rect, resp) = ui.allocate_exact_size(
        vec2(w, RIBBON_H - 2.0),
        if enabled { Sense::click() } else { Sense::hover() },
    );
    let painter = ui.painter_at(rect.expand(1.0));
    if let Some(c) = bg(ui, &resp, selected, &t) {
        painter.rect_filled(rect, CornerRadius::same(4), c);
    }
    let icon_rect = Rect::from_center_size(pos2(rect.center().x, rect.min.y + 19.0), vec2(30.0, 30.0));
    icons::paint(&painter, icon_rect, icon, t.text, !enabled);
    let col = if enabled { t.text } else { t.text_faint };
    let mut y = rect.min.y + 39.0;
    for l in &lines {
        painter.text(
            pos2(rect.center().x, y),
            Align2::CENTER_TOP,
            *l,
            font.clone(),
            col,
        );
        y += 12.5;
    }
    let text = label.replace('\n', " ");
    resp.widget_info(|| match toggle {
        Some(on) => egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, on, &text),
        None => egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &text),
    });
    resp
}

/// Splits a label on its explicit `\n`, else near the middle when it has several words.
pub fn wrap_label(label: &str) -> Vec<&str> {
    if let Some((a, b)) = label.split_once('\n') {
        return vec![a, b];
    }
    let words: Vec<&str> = label.split(' ').collect();
    if words.len() >= 2 && label.chars().count() > 8 {
        let mut best = 1;
        let mut best_diff = usize::MAX;
        for k in 1..words.len() {
            let a: usize = words.iter().take(k).map(|w| w.chars().count() + 1).sum();
            let b: usize = words.iter().skip(k).map(|w| w.chars().count() + 1).sum();
            let d = a.abs_diff(b);
            if d < best_diff {
                best_diff = d;
                best = k;
            }
        }
        let split_at: usize = words
            .iter()
            .take(best)
            .map(|w| w.len() + 1)
            .sum::<usize>()
            .saturating_sub(1);
        if let (Some(a), Some(b)) = (label.get(..split_at), label.get(split_at + 1..)) {
            return vec![a, b];
        }
    }
    vec![label]
}

/// A painted chevron (the UI font has no ▾).
pub fn chevron(p: &egui::Painter, c: egui::Pos2, s: f32, col: Color32) {
    p.line_segment(
        [pos2(c.x - s, c.y - s * 0.5), pos2(c.x, c.y + s * 0.5)],
        Stroke::new(1.2, col),
    );
    p.line_segment(
        [pos2(c.x, c.y + s * 0.5), pos2(c.x + s, c.y - s * 0.5)],
        Stroke::new(1.2, col),
    );
}

/// A small ribbon button: a 17 px icon and a 12 pt label in one row.
pub fn small_button(ui: &mut Ui, icon: Icon, label: &str, enabled: bool, selected: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let font = theme::font(12.0);
    let tw = if label.is_empty() {
        0.0
    } else {
        text_w(ui, label, font.clone()) + 5.0
    };
    let (rect, resp) = ui.allocate_exact_size(
        vec2(24.0 + tw, 22.0),
        if enabled { Sense::click() } else { Sense::hover() },
    );
    let painter = ui.painter_at(rect);
    if let Some(c) = bg(ui, &resp, selected, &t) {
        painter.rect_filled(rect, CornerRadius::same(4), c);
    }
    icons::paint(
        &painter,
        Rect::from_center_size(pos2(rect.min.x + 12.0, rect.center().y), vec2(17.0, 17.0)),
        icon,
        t.text,
        !enabled,
    );
    if !label.is_empty() {
        painter.text(
            pos2(rect.min.x + 24.0, rect.center().y),
            Align2::LEFT_CENTER,
            label,
            font,
            if enabled { t.text } else { t.text_faint },
        );
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, selected, label));
    resp
}

/// A square icon toggle (24 × 22), labelled by its tooltip.
pub fn icon_toggle(ui: &mut Ui, icon: Icon, tip: &str, enabled: bool, on: bool) -> Response {
    icon_toggle_tinted(ui, icon, tip, enabled, on, None)
}

/// [`icon_toggle`] with its icon in `tint` (a report's colour, e.g. the Filters pane's font and
/// icon colour) instead of the text token.
pub fn icon_toggle_tinted(
    ui: &mut Ui,
    icon: Icon,
    tip: &str,
    enabled: bool,
    on: bool,
    tint: Option<egui::Color32>,
) -> Response {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(
        vec2(24.0, 22.0),
        if enabled { Sense::click() } else { Sense::hover() },
    );
    let painter = ui.painter_at(rect);
    if let Some(c) = bg(ui, &resp, on, &t) {
        painter.rect_filled(rect, CornerRadius::same(4), c);
    }
    icons::paint(&painter, rect.shrink(3.0), icon, tint.unwrap_or(t.text), !enabled);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, on, tip));
    resp.on_hover_text(tip)
}

/// An on/off switch (36 × 18, Desktop's Format pane toggles): an accent track with the knob at
/// the right when on. Labelled `label` (a checkbox to assistive technologies).
pub fn switch(ui: &mut Ui, on: bool, enabled: bool, label: &str) -> Response {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(
        vec2(36.0, 18.0),
        if enabled { Sense::click() } else { Sense::hover() },
    );
    let p = ui.painter();
    let track = match (on, enabled) {
        (true, true) => t.accent,
        (true, false) => t.text_faint,
        (false, _) => t.field_bg,
    };
    p.rect_filled(rect, CornerRadius::same(9), track);
    p.rect_stroke(
        rect,
        CornerRadius::same(9),
        Stroke::new(1.0, if on { track } else { t.border }),
        egui::StrokeKind::Inside,
    );
    let x = if on { rect.max.x - 9.0 } else { rect.min.x + 9.0 };
    let knob = if on { t.accent_text } else { t.text_dim };
    p.circle_filled(pos2(x, rect.center().y), 5.5, knob);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, on, label));
    resp
}

/// A status-bar icon button (26 × 20), DeckCraft's `small_icon`.
pub fn status_icon(ui: &mut Ui, icon: Icon, tip: &str, on: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(26.0, 20.0), Sense::click());
    if on || resp.hovered() {
        ui.painter()
            .rect_filled(r, CornerRadius::same(4), if on { t.selected } else { t.hover });
    }
    icons::paint(
        ui.painter(),
        Rect::from_center_size(r.center(), vec2(16.0, 16.0)),
        icon,
        t.text_dim,
        false,
    );
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, on, tip));
    resp.on_hover_text(tip)
}

/// A ribbon group: its contents, then a thin vertical separator.
pub fn group<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let r = ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing = vec2(1.0, 1.0);
        add(ui)
    });
    separator(ui);
    r.inner
}

pub fn separator(ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(13.0, RIBBON_H - 6.0), Sense::hover());
    ui.painter().line_segment(
        [
            pos2(rect.center().x, rect.min.y + 6.0),
            pos2(rect.center().x, rect.max.y - 4.0),
        ],
        Stroke::new(1.0, t.separator),
    );
}

/// Two or three rows of small controls stacked in one ribbon column.
pub fn rows<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing = vec2(1.0, 0.0);
        add(ui)
    })
    .inner
}

/// A collapsible pane section header: chevron, semibold title. Returns the new open state.
pub fn section(ui: &mut Ui, title: &str, open: bool) -> bool {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
    let p = ui.painter();
    let c = pos2(rect.min.x + 8.0, rect.center().y);
    if open {
        chevron(p, c, 3.5, t.text);
    } else {
        p.line_segment(
            [pos2(c.x - 2.0, c.y - 4.0), pos2(c.x + 2.0, c.y)],
            Stroke::new(1.2, t.text),
        );
        p.line_segment(
            [pos2(c.x + 2.0, c.y), pos2(c.x - 2.0, c.y + 4.0)],
            Stroke::new(1.2, t.text),
        );
    }
    p.text(
        pos2(rect.min.x + 18.0, rect.center().y),
        Align2::LEFT_CENTER,
        title,
        theme::bold(13.0),
        t.text,
    );
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::CollapsingHeader, true, open, title));
    if resp.clicked() { !open } else { open }
}

/// The primary pill button: amber fill, slate semibold text, radius 13 (DeckCraft's "Present").
/// Disabled: a grey pill with the reason as its tooltip.
pub fn primary(ui: &mut Ui, label: &str, enabled: bool, reason: &str) -> Response {
    let t = Tokens::get(ui.ctx());
    let font = theme::bold(13.0);
    let w = (text_w(ui, label, font.clone()) + 28.0).max(84.0);
    let (rect, resp) = ui.allocate_exact_size(
        vec2(w, 26.0),
        if enabled { Sense::click() } else { Sense::hover() },
    );
    let fill = if !enabled {
        t.hover
    } else if resp.is_pointer_button_down_on() {
        t.accent_pressed
    } else if resp.hovered() {
        t.accent_hover
    } else {
        t.accent
    };
    ui.painter().rect_filled(rect, CornerRadius::same(13), fill);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        font,
        if enabled { t.accent_text } else { t.text_faint },
    );
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    if enabled || reason.is_empty() {
        resp
    } else {
        resp.on_hover_text(reason)
    }
}

/// A pill: 20 high, 11 pt, an optional leading dot. With `strong`, the text is `text`, else
/// `text_dim`.
pub fn pill(ui: &mut Ui, label: &str, dot: Option<Color32>, strong: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let font = theme::font(11.0);
    let lead = if dot.is_some() { 12.0 } else { 0.0 };
    let w = text_w(ui, label, font.clone()) + 16.0 + lead;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 20.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(10), t.selected);
    if let Some(c) = dot {
        ui.painter()
            .circle_filled(pos2(rect.min.x + 11.0, rect.center().y), 3.0, c);
    }
    ui.painter().text(
        pos2(rect.min.x + 8.0 + lead, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        font,
        if strong { t.text } else { t.text_dim },
    );
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, label));
    resp
}

/// A tab of the tab row: semibold with a 3 px accent underline when active, `contextual` text
/// for contextual tabs.
pub fn tab(ui: &mut Ui, label: &str, active: bool, contextual: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let font = if active {
        theme::bold(13.0)
    } else {
        theme::font(13.0)
    };
    let w = text_w(ui, label, theme::bold(13.0)) + 18.0;
    let (r, resp) = ui.allocate_exact_size(vec2(w, 24.0), Sense::click());
    if resp.hovered() && !active {
        ui.painter()
            .rect_filled(r.shrink2(vec2(2.0, 2.0)), CornerRadius::same(4), t.hover);
    }
    let col = if contextual { t.contextual } else { t.text };
    ui.painter()
        .text(r.center(), Align2::CENTER_CENTER, label, font, col);
    if active {
        let uw = (w - 20.0).max(16.0);
        ui.painter().rect_filled(
            Rect::from_center_size(pos2(r.center().x, r.max.y - 1.0), vec2(uw, 3.0)),
            CornerRadius::same(1),
            t.accent,
        );
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, active, label));
    resp
}

/// A card frame (`card` fill, `card_border`, radius 10).
pub fn card(t: &Tokens) -> egui::Frame {
    egui::Frame::new()
        .fill(t.card)
        .stroke(Stroke::new(1.0, t.card_border))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::same(12))
}

/// The error bar: `err_soft` card with the warning icon, the message and a close button.
/// Returns true when it is closed.
pub fn error_bar(ui: &mut Ui, msg: &str, close_tip: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    let mut close = false;
    egui::Frame::new()
        .fill(t.err_soft)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                icons::paint(ui.painter(), r, Icon::Warning, t.err, false);
                // the message wraps; past ERROR_MAX_H it scrolls inside the bar
                let w = (ui.available_width() - 28.0).max(40.0);
                ui.allocate_ui(vec2(w, ERROR_MAX_H), |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("bicraft-error-bar")
                        .max_height(ERROR_MAX_H)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.add(egui::Label::new(egui::RichText::new(msg).color(t.err_text)).wrap());
                        });
                });
                let (r, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(r, CornerRadius::same(4), t.hover);
                }
                icons::paint(ui.painter(), r.shrink(4.0), Icon::Close, t.err_text, false);
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, close_tip));
                if resp.on_hover_text(close_tip).clicked() {
                    close = true;
                }
            });
        });
    close
}

/// The error bar's message wraps up to this height, then scrolls.
pub const ERROR_MAX_H: f32 = 120.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_wrap_near_the_middle() {
        assert_eq!(wrap_label("Open"), vec!["Open"]);
        assert_eq!(wrap_label("Connect to server"), vec!["Connect", "to server"]);
        assert_eq!(wrap_label("Fit\npage"), vec!["Fit", "page"]);
        assert_eq!(wrap_label(""), vec![""]);
        assert_eq!(wrap_label("Ouvrir dans Power BI Desktop").len(), 2);
    }
}
