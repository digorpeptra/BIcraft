//! An M editor for the Power Query editor's Advanced editor and formula bar, with the language
//! services of `bicraft-mlang` (M11.35; the DAX formula bar's pattern, [`crate::dax_box`]):
//! completion of the library functions BIcraft's evaluator runs, the constants, keywords, the
//! query's steps, the window's queries and, inside `[ ]`, the preview's columns (typing, or
//! Ctrl+Space; ↑/↓ and Enter or Tab to accept, Esc to close the list); the signature of the
//! library call being typed with its parameter underlined; and what a name is when the pointer
//! rests on it. Enter adds a line while no list is open. The text stays the window's draft: the
//! engine checks it when the user applies it.

use std::ops::Range;

use egui::text::{CCursor, CCursorRange, LayoutJob, TextFormat};
use egui::{Id, Key, Modifiers, RichText, Stroke, Ui, vec2};

use bicraft_mlang::{Item, ItemKind};

use crate::formula;
use crate::i18n::Lang;
use crate::theme::Tokens;

/// The editor's list and pending cursor move.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MAssist {
    /// The autocomplete list, when open: its items, the one selected, the bytes it replaces.
    pub completion: Option<(Vec<Item>, usize, Range<usize>)>,
    /// Where the cursor goes on the next frame (a character index).
    pub cursor_to: Option<usize>,
    /// Take the keyboard on the next frame.
    pub focus: bool,
}

/// The names in scope the text does not tell.
#[derive(Debug, Clone, Default)]
pub struct Names {
    pub queries: Vec<String>,
    pub steps: Vec<String>,
    pub columns: Vec<String>,
}

impl Names {
    fn scope(&self) -> bicraft_mlang::Scope<'_> {
        bicraft_mlang::Scope {
            queries: &self.queries,
            steps: &self.steps,
            columns: &self.columns,
        }
    }
}

/// The word of a suggestion's kind.
pub fn kind_word(k: ItemKind, l: Lang) -> &'static str {
    match k {
        ItemKind::Column => l.t("column", "colonne"),
        ItemKind::Step => l.t("step", "étape"),
        ItemKind::Query => l.t("query", "requête"),
        ItemKind::Keyword => l.t("keyword", "mot clé"),
        ItemKind::Function => l.t("function", "fonction"),
        ItemKind::Constant => l.t("constant", "constante"),
    }
}

/// The editor, `w` wide and at least `rows` lines high, labelled `label` for the
/// accessibility tree. Returns the text edit's response.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut Ui,
    text: &mut String,
    st: &mut MAssist,
    id: Id,
    label: &str,
    names: &Names,
    rows: usize,
    w: f32,
    enabled: bool,
    l: Lang,
) -> egui::Response {
    let ctx = ui.ctx().clone();
    let t = Tokens::get(&ctx);
    let focused = enabled && ctx.memory(|m| m.has_focus(id));
    let mut accept: Option<usize> = None;
    let mut explicit = false;
    if focused {
        let plain = |m: Modifiers| m.is_none();
        if let Some((items, selected, _)) = st.completion.as_mut() {
            let n = items.len().max(1);
            if formula::take_key(ui, Key::ArrowDown, plain) {
                *selected = (*selected + 1) % n;
            }
            if formula::take_key(ui, Key::ArrowUp, plain) {
                *selected = (*selected + n - 1) % n;
            }
            if formula::take_key(ui, Key::Enter, plain) || formula::take_key(ui, Key::Tab, plain) {
                accept = Some(*selected);
            }
            if formula::take_key(ui, Key::Escape, |_| true) {
                st.completion = None;
            }
        }
        if formula::take_key(ui, Key::Space, |m| m.command && !m.shift && !m.alt) {
            explicit = true;
        }
    }
    let clicking = ctx.input(|i| i.pointer.any_pressed() || i.pointer.any_released());
    if st.focus && enabled {
        if clicking {
            ctx.request_repaint();
        } else {
            ctx.memory_mut(|m| m.request_focus(id));
            st.focus = false;
        }
    }
    let out = ui
        .add_enabled_ui(enabled, |ui| {
            egui::TextEdit::multiline(text)
                .id(id)
                .code_editor()
                .desired_rows(rows.max(1))
                .desired_width(w)
                .char_limit(1 << 20)
                .show(ui)
        })
        .inner;
    let resp = out.response.response.clone();
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, enabled, label));
    // Tab, the arrows and Esc stay with the editor while it has the list
    if resp.has_focus() && st.completion.is_some() {
        ctx.memory_mut(|m| {
            m.set_focus_lock_filter(
                id,
                egui::EventFilter {
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                },
            )
        });
    }
    let changed = resp.changed();
    let cursor_char = out.cursor_range.map(|r| r.primary.index.0);
    if let Some(c) = st.cursor_to.take() {
        let mut s = out.state.clone();
        s.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(c))));
        s.store(&ctx, id);
    }
    let cursor_byte = cursor_char.map(|c| bicraft_mlang::byte_of_char(text, c));
    if (changed || explicit)
        && let Some(cb) = cursor_byte
    {
        st.completion =
            bicraft_mlang::complete(text, cb, &names.scope(), explicit).map(|c| (c.items, 0, c.replace));
    } else if !focused && !resp.has_focus() {
        st.completion = None;
    }
    let anchor = cursor_char.map(|c| {
        out.galley_pos
            + out
                .galley
                .pos_from_cursor(CCursor::new(c))
                .left_bottom()
                .to_vec2()
    });
    if let (Some((items, selected, _)), Some(pos)) = (st.completion.as_ref(), anchor) {
        let rows: Vec<(&str, &str)> = items
            .iter()
            .map(|i| (i.label.as_str(), kind_word(i.kind, l)))
            .collect();
        if let Some(i) = formula::rows_ui(&ctx, id.with("list"), &rows, *selected, pos, &t, None) {
            accept = Some(i);
        }
    } else if let (Some(pos), Some(cb), true) = (anchor, cursor_byte, resp.has_focus()) {
        signature_ui(&ctx, id, text, cb, pos, &t);
    }
    if st.completion.is_none() {
        hover_ui(&ctx, id, text, names, &out, &t, l);
    }
    if let Some(i) = accept
        && let Some((items, _, replace)) = st.completion.take()
        && let Some(s) = items.get(i)
        && replace.end <= text.len()
        && text.is_char_boundary(replace.start)
        && text.is_char_boundary(replace.end)
    {
        text.replace_range(replace.clone(), &s.insert);
        let at = bicraft_mlang::char_of_byte(text, replace.start) + s.insert.chars().count();
        st.cursor_to = Some(at);
        st.focus = true;
    }
    resp
}

/// The signature of the library call at the cursor, the parameter being typed underlined, its
/// description under it.
fn signature_ui(ctx: &egui::Context, id: Id, text: &str, cursor: usize, pos: egui::Pos2, t: &Tokens) {
    let Some(h) = bicraft_mlang::signature_at(text, cursor) else {
        return;
    };
    let f = h.function;
    let font = formula::mono();
    let plain = TextFormat {
        font_id: font.clone(),
        color: t.text,
        ..Default::default()
    };
    let on = TextFormat {
        font_id: font,
        color: t.contextual,
        underline: Stroke::new(1.0, t.contextual),
        ..Default::default()
    };
    let mut job = LayoutJob::default();
    job.append(f.name, 0.0, plain.clone());
    job.append("(", 0.0, plain.clone());
    for (i, p) in f.params.iter().enumerate() {
        if i > 0 {
            job.append(", ", 0.0, plain.clone());
        }
        if p.optional {
            job.append("optional ", 0.0, plain.clone());
        }
        let fmt = if Some(i) == h.active {
            on.clone()
        } else {
            plain.clone()
        };
        job.append(p.name, 0.0, fmt);
    }
    job.append(")", 0.0, plain);
    let signature = f.signature();
    egui::Area::new(id.with("signature"))
        .order(egui::Order::Tooltip)
        .fixed_pos(pos + vec2(0.0, 4.0))
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_max_width(520.0);
                let r = ui.label(job);
                r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &signature));
                ui.label(RichText::new(f.description).size(11.0).color(t.text_dim));
            });
        });
}

/// What the name under the pointer is, while the pointer rests on the text.
fn hover_ui(
    ctx: &egui::Context,
    id: Id,
    text: &str,
    names: &Names,
    out: &egui::text_edit::TextEditOutput,
    t: &Tokens,
    l: Lang,
) {
    if !out.response.response.hovered() {
        return;
    }
    let Some(pointer) = ctx.input(|i| i.pointer.hover_pos()) else {
        return;
    };
    let local = pointer - out.galley_pos;
    if !out.galley.rect.contains(local.to_pos2()) {
        return;
    }
    let ch = out.galley.cursor_from_pos(local).index.0;
    let at = bicraft_mlang::byte_of_char(text, ch);
    let Some(h) = bicraft_mlang::hover(text, at, &names.scope()) else {
        return;
    };
    let head = h.signature.clone().unwrap_or_else(|| h.name.clone());
    let kind = kind_word(h.kind, l);
    egui::Area::new(id.with("hover"))
        .order(egui::Order::Tooltip)
        .fixed_pos(pointer + vec2(12.0, 16.0))
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_max_width(520.0);
                ui.horizontal(|ui| {
                    let r = ui.label(RichText::new(&head).font(formula::mono()).color(t.text));
                    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &head));
                    ui.label(RichText::new(kind).size(11.0).color(t.text_dim));
                });
                if !h.description.is_empty() {
                    ui.label(RichText::new(&h.description).size(11.0).color(t.text_dim));
                }
            });
        });
}
