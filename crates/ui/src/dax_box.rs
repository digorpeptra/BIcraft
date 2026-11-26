//! A DAX editor for dialogs (Manage roles' DAX editor): the formula bar's pieces
//! ([`crate::formula`]) in a box of several lines: the DAX colours of the theme, the first
//! error underlined, completion of the model's tables, columns and measures, the engine's
//! functions and the keywords (typing, or Ctrl+Space), ↑/↓ and Enter or Tab to accept, Esc to
//! close the list. Enter adds a line. No engine: the caller checks the text.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use egui::text::{CCursor, CCursorRange, LayoutJob};
use egui::{Galley, Id, Key, Modifiers, Stroke, Ui};

use crate::dax_text::{self, Func, Names, Want};
use crate::formula::{self, Completion, Diag};
use crate::i18n::Lang;
use crate::theme::Tokens;

/// The editor's text and its list.
#[derive(Debug, Clone, Default)]
pub struct DaxBox {
    pub text: String,
    /// The autocomplete list, when open.
    pub completion: Option<Completion>,
    /// Where the cursor goes on the next frame (a character index).
    pub cursor_to: Option<usize>,
    /// Take the keyboard on the next frame.
    pub focus: bool,
    galley: Option<(u64, LayoutJob)>,
}

impl PartialEq for DaxBox {
    fn eq(&self, o: &Self) -> bool {
        self.text == o.text && self.completion == o.completion && self.cursor_to == o.cursor_to
    }
}

impl DaxBox {
    pub fn new(text: &str) -> Self {
        Self {
            text: text.to_string(),
            ..Self::default()
        }
    }
}

/// What the suggestions come from.
pub struct Scope<'a> {
    pub names: &'a Names,
    pub funcs: &'a [Func],
    pub keywords: &'a [String],
    /// The expression's table: its columns come bare (a row context) when `columns_bare`.
    pub home: Option<&'a str>,
    pub columns_bare: bool,
}

/// The editor, `w` wide and `rows` lines high, labelled `label` for the accessibility tree;
/// `diags` (of the current text) underline the first error. Returns the text edit's response.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut Ui,
    b: &mut DaxBox,
    id: Id,
    label: &str,
    scope: &Scope<'_>,
    diags: &[Diag],
    w: f32,
    rows: usize,
    l: Lang,
) -> egui::Response {
    let ctx = ui.ctx().clone();
    let t = Tokens::get(&ctx);
    let focused = ctx.memory(|m| m.has_focus(id));
    let mut accept: Option<usize> = None;
    let mut explicit = false;
    if focused {
        let plain = |m: Modifiers| m.is_none();
        if let Some(c) = b.completion.as_mut() {
            let n = c.items.len().max(1);
            if formula::take_key(ui, Key::ArrowDown, plain) {
                c.selected = (c.selected + 1) % n;
            }
            if formula::take_key(ui, Key::ArrowUp, plain) {
                c.selected = (c.selected + n - 1) % n;
            }
            if formula::take_key(ui, Key::Enter, plain) || formula::take_key(ui, Key::Tab, plain) {
                accept = Some(c.selected);
            }
            if formula::take_key(ui, Key::Escape, |_| true) {
                b.completion = None;
            }
        }
        if formula::take_key(ui, Key::Space, |m| m.command && !m.shift && !m.alt) {
            explicit = true;
        }
    }
    let clicking = ctx.input(|i| i.pointer.any_pressed() || i.pointer.any_released());
    if b.focus {
        if clicking {
            ctx.request_repaint();
        } else {
            ctx.memory_mut(|m| m.request_focus(id));
            b.focus = false;
        }
    }
    let underline = formula::underline_of(&b.text, diags);
    let theme_key = t.dark;
    let cache = &mut b.galley;
    let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| -> Arc<Galley> {
        let s = buf.as_str();
        let mut hs = std::collections::hash_map::DefaultHasher::new();
        s.hash(&mut hs);
        wrap.to_bits().hash(&mut hs);
        theme_key.hash(&mut hs);
        underline.hash(&mut hs);
        let key = hs.finish();
        if cache.as_ref().is_none_or(|(k, _)| *k != key) {
            *cache = Some((
                key,
                formula::job(s, &Tokens::get(ui.ctx()), underline.clone(), wrap),
            ));
        }
        let laid = cache.as_ref().map(|(_, j)| j.clone()).unwrap_or_default();
        ui.fonts_mut(|f| f.layout_job(laid))
    };
    let row = ctx.fonts_mut(|f| f.row_height(&formula::mono()));
    let h = row * rows.max(1) as f32 + 8.0;
    let out = egui::Frame::NONE
        .fill(t.field_bg)
        .stroke(Stroke::new(1.0, if focused { t.ring } else { t.border }))
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::symmetric(4, 2))
        .show(ui, |ui| {
            ui.set_width(w);
            egui::ScrollArea::vertical()
                .id_salt(id.with("scroll"))
                .max_height(h)
                .min_scrolled_height(h)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    egui::TextEdit::multiline(&mut b.text)
                        .id(id)
                        .font(formula::mono())
                        .code_editor()
                        .frame(egui::Frame::NONE)
                        .desired_width(f32::INFINITY)
                        .desired_rows(rows.max(1))
                        .char_limit(64 * 1024)
                        .layouter(&mut layouter)
                        .show(ui)
                })
                .inner
        })
        .inner;
    let resp = out.response.response.clone();
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, label));
    // Tab and the arrows stay with the editor while it has the list
    if resp.has_focus() && b.completion.is_some() {
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
    if let Some(c) = b.cursor_to.take() {
        let mut st = out.state.clone();
        st.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(c))));
        st.store(&ctx, id);
    }
    let cursor_byte = cursor_char.map(|c| dax_text::byte_of_char(&b.text, c));
    if (changed || explicit)
        && let Some(cb) = cursor_byte
    {
        b.completion = dax_text::context(&b.text, cb, explicit).and_then(|cx| {
            let items = dax_text::suggestions(
                &cx.want,
                scope.names,
                scope.funcs,
                scope.keywords,
                scope.home,
                scope.columns_bare,
            );
            let only_itself = matches!(&cx.want, Want::Word(w) if items.len() == 1
                && items.first().is_some_and(|s| s.label.eq_ignore_ascii_case(w)));
            (!items.is_empty() && !only_itself).then_some(Completion {
                items,
                selected: 0,
                replace: cx.replace,
            })
        });
    } else if !focused && !resp.has_focus() {
        b.completion = None;
    }
    let anchor = cursor_char.map(|c| {
        out.galley_pos
            + out
                .galley
                .pos_from_cursor(CCursor::new(c))
                .left_bottom()
                .to_vec2()
    });
    if let (Some(c), Some(pos)) = (b.completion.clone(), anchor)
        && let Some(i) = formula::list_ui(&ctx, id.with("list"), &c, pos, &t, l, scope.funcs)
    {
        accept = Some(i);
    }
    if let Some(i) = accept
        && let Some(c) = b.completion.take()
        && let Some(s) = c.items.get(i)
        && c.replace.end <= b.text.len()
        && b.text.is_char_boundary(c.replace.start)
        && b.text.is_char_boundary(c.replace.end)
    {
        b.text.replace_range(c.replace.clone(), &s.insert);
        let at = dax_text::char_of_byte(&b.text, c.replace.start) + s.insert.chars().count();
        b.cursor_to = Some(at);
        b.focus = true;
    }
    resp
}
