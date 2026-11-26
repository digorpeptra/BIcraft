//! Keyboard navigation of the app's chrome, after Microsoft Learn's "Keyboard shortcuts in
//! Power BI Desktop" (docs/desktop.md, "Keyboard navigation"):
//!
//! - **Sections.** Ctrl+F6 moves the focus to the next section of the window (the ribbon and
//!   title rows, the formula bar, the view in the middle, each side pane, the page tabs, the
//!   status bar), Ctrl+Shift+F6 to the previous one; F6 activates the Selection pane when it is
//!   open, as in Desktop, and otherwise moves as Ctrl+F6 does (Shift+F6 backwards). Each
//!   section is registered by the code that draws it ([`section`]) and the focus lands on its
//!   first focusable control.
//! - **Tab and Shift+Tab stay inside a section**: from the last control of a section Tab wraps
//!   to its first, Shift+Tab from the first to its last.
//! - **Dialogs** (every `egui::Window`, the only areas of the `Middle` order) are modal for the
//!   keyboard and the pointer: the topmost one is egui's modal layer, so Tab cycles inside it.
//!   Opened from the keyboard, the focus moves to its first control; closed (Escape, Cancel,
//!   OK), the focus returns to the control that had it before (the ribbon button that opened
//!   it).
//! - **The focus ring**: while the focus moves by the keyboard (Tab, F6, the arrows) a 2 px ring
//!   in the theme's `focus` token is drawn around the focused control; a pointer press hides it.
//!
//! - **Context menus**: Shift+F10 opens the focused control's menu, the one a right click opens
//!   ([`context_menu`] in place of `Response::context_menu`); on the report page (focused, or
//!   nothing focused) the selected visual's. Its first item takes the focus; closed, the focus
//!   returns to the control.
//!
//! Arrow keys move the focus between neighbouring controls (egui's own focus directions), and
//! Enter or Space activates the focused one (egui's click from the keyboard). The ribbon's key
//! tips are `crate::key_tips`, the shortcuts list (Shift+?) `crate::shortcuts_list`. Pure view
//! state: nothing here runs a command.

use egui::{Context, Event, FocusDirection, Id, Key, LayerId, Order, Rect, Stroke, StrokeKind};

use crate::App;
use crate::theme::Tokens;

/// A part of the window that Ctrl+F6 moves to, in the order it cycles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Section {
    /// The title row, the document tabs and the ribbon (and the banners under it).
    Ribbon,
    /// The DAX formula bar.
    Formula,
    /// The view in the middle: the report canvas, the table view, the model diagram, the DAX
    /// query view.
    Main,
    /// A side pane (the Selection pane is [`Section::Selection`]); panes cycle left to right.
    Pane,
    /// The Selection pane (F6 activates it).
    Selection,
    /// The page tabs under the canvas.
    PageTabs,
    /// The status bar.
    Status,
}

impl Section {
    /// The rank of the section in the Ctrl+F6 cycle (the panes share theirs, by position).
    fn rank(self) -> u8 {
        match self {
            Section::Ribbon => 0,
            Section::Formula => 1,
            Section::Main => 2,
            Section::Pane | Section::Selection => 3,
            Section::PageTabs => 4,
            Section::Status => 5,
        }
    }
}

/// The keyboard navigation's state (view state only).
#[derive(Debug, Default)]
pub struct KeyboardUi {
    /// The sections drawn in this frame: kind, layer and rectangle.
    sections: Vec<(Section, LayerId, Rect)>,
    /// The focus moves by the keyboard: the ring is drawn.
    pub keyboard: bool,
    /// The open dialogs, bottom first, with the control focused before each opened.
    dialogs: Vec<(LayerId, Option<Id>)>,
    /// The control focused at the end of the previous frame.
    last_focus: Option<Id>,
    /// Tab or Shift+Tab was pressed in this section (index into the previous frame's
    /// sections is not stable, so its rectangle): wrap when the focus leaves it.
    tab_from: Option<(Rect, LayerId, bool, u8)>,
    /// Dialogs just opened that have not taken the focus yet, with the frames left to wait.
    settle: Vec<(LayerId, u8)>,
    /// The ribbon's key tips (Alt, F10; `crate::key_tips`).
    pub tips: crate::key_tips::Tips,
    /// The shortcuts list, while it is open (Shift+?; `crate::shortcuts_list`).
    pub shortcuts: Option<crate::shortcuts_list::List>,
    /// The report page's widget drawn in the last frame (Shift+F10 on it opens the selected
    /// visual's menu).
    page: Option<Id>,
    /// The control Shift+F10 opened a menu for, to give the focus back when it closes.
    menu_owner: Option<Id>,
}

impl KeyboardUi {
    /// A dialog is open (the last frame's).
    pub fn in_dialog(&self) -> bool {
        !self.dialogs.is_empty()
    }
}

/// Where Shift+F10 opens a menu in this frame, and where the menu it opened stays: kept in the
/// context's temporary data so every widget's menu code reads it without the app.
#[derive(Debug, Clone, Copy, Default)]
struct KeyMenu {
    /// The widget whose menu opens in this frame, at this global position.
    target: Option<(Id, egui::Pos2)>,
    /// The popup opened from the keyboard, at its position (the popup has no pointer position).
    open: Option<(Id, egui::Pos2)>,
}

fn key_menu_id() -> Id {
    Id::unique("bicraft-key-menu")
}

fn key_menu(ctx: &Context) -> KeyMenu {
    ctx.data(|d| d.get_temp::<KeyMenu>(key_menu_id()))
        .unwrap_or_default()
}

fn set_key_menu(ctx: &Context, m: KeyMenu) {
    ctx.data_mut(|d| d.insert_temp(key_menu_id(), m));
}

/// Where Shift+F10 (the context key) opens `resp`'s menu in this frame, if it targets `resp`:
/// what a right click at that position would do (the selection stays as it is).
pub fn menu_key(resp: &egui::Response) -> Option<egui::Pos2> {
    key_menu(&resp.ctx)
        .target
        .filter(|(id, _)| *id == resp.id)
        .map(|(_, p)| p)
}

/// `resp`'s context menu (`Response::context_menu`): opened by a right click, or by Shift+F10
/// while `resp` has the focus, where its first item takes the focus.
pub fn context_menu<R>(
    resp: &egui::Response,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> Option<egui::InnerResponse<R>> {
    let ctx = &resp.ctx;
    let popup_id = egui::Popup::default_response_id(resp);
    let mut m = key_menu(ctx);
    let mut popup = egui::Popup::context_menu(resp);
    if let Some(pos) = menu_key(resp) {
        popup = popup.open_memory(egui::SetOpenCommand::Bool(true));
        m.open = Some((popup_id, pos));
        set_key_menu(ctx, m);
    } else if resp.secondary_clicked() && m.open.is_some_and(|(id, _)| id == popup_id) {
        m.open = None;
        set_key_menu(ctx, m);
    }
    if let Some((id, pos)) = m.open
        && id == popup_id
    {
        popup = popup.at_position(pos);
    }
    popup.show(add)
}

/// Notes the report page's widget (Shift+F10 on it, or with nothing focused, opens the selected
/// visual's menu).
pub fn canvas_page(app: &mut App, resp: &egui::Response) {
    app.keyboard.page = Some(resp.id);
}

/// Shift+F10: the focused control's menu opens in this frame (the report page's at the selected
/// visual); nothing focused on the report page with a visual selected, the visual's.
fn context_key(app: &mut App, ctx: &Context) {
    let mut m = key_menu(ctx);
    m.target = None;
    // a menu closed: forget it
    if let Some((id, _)) = m.open
        && !egui::Popup::is_id_open(ctx, id)
    {
        m.open = None;
    }
    let hit =
        ctx.input(|i| i.modifiers.shift && !i.modifiers.ctrl && !i.modifiers.alt && i.key_pressed(Key::F10));
    if hit && app.keyboard.dialogs.is_empty() {
        ctx.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, Key::F10));
        app.keyboard.keyboard = true;
        let focus = ctx.memory(|mem| mem.focused());
        let prev = |id: Id| {
            ctx.viewport(|v| v.prev_pass.widgets.get(id).map(|w| (w.layer_id, w.interact_rect)))
                .map(|(layer, r)| ctx.layer_transform_to_global(layer).map_or(r, |t| t.mul_rect(r)))
        };
        let visual = app
            .cmd
            .visual
            .clone()
            .filter(|_| app.tab == crate::app::Tab::Report);
        let page = app.keyboard.page;
        let target = match (focus, page) {
            (Some(f), Some(p)) if f == p => visual_center(app, visual.as_ref()).map(|c| (p, c)),
            (None, Some(p)) => visual_center(app, visual.as_ref()).map(|c| (p, c)),
            _ => None,
        }
        .or_else(|| {
            let f = focus?;
            let r = prev(f)?;
            Some((f, egui::pos2(r.left() + (r.width() / 2.0).min(24.0), r.bottom())))
        });
        if let Some((id, pos)) = target {
            m.target = Some((id, pos));
            app.keyboard.menu_owner = Some(id);
            app.keyboard.tips.menu = Some((id.with("popup"), crate::key_tips::MENU_FRAMES));
        }
    }
    set_key_menu(ctx, m);
}

/// The selected visual's centre on the screen, when it is on the page shown.
fn visual_center(app: &App, sel: Option<&crate::ribbon::VisualSel>) -> Option<egui::Pos2> {
    let placed = app.placed?;
    let r = crate::menus::visual_rect(app, sel?)?;
    Some(placed.rect.min + r.center().to_vec2() * placed.zoom)
}

/// A menu opened from the keyboard (Shift+F10, a menu button's key tip): its first item takes
/// the focus once it is drawn; closed, the focus goes back to the control it was opened from.
fn menu_focus(app: &mut App, ctx: &Context) {
    if let Some((popup, left)) = app.keyboard.tips.menu.take() {
        let layer = LayerId::new(Order::Foreground, popup);
        let focus_in = ctx
            .memory(|m| m.focused())
            .and_then(|id| widget(ctx, id))
            .is_some_and(|(l, _)| l == layer);
        if !focus_in {
            match focusable(ctx, layer).first() {
                Some(&(id, _)) => ctx.memory_mut(|m| m.request_focus(id)),
                None if left > 0 => app.keyboard.tips.menu = Some((popup, left - 1)),
                None => {}
            }
        }
    }
    if let Some(owner) = app.keyboard.menu_owner
        && !egui::Popup::is_id_open(ctx, owner.with("popup"))
        && app.keyboard.tips.menu.is_none()
    {
        app.keyboard.menu_owner = None;
        let lost = ctx
            .memory(|m| m.focused())
            .is_none_or(|id| widget(ctx, id).is_none());
        if lost && widget(ctx, owner).is_some() {
            ctx.memory_mut(|m| m.request_focus(owner));
        }
    }
}

/// Frames a new dialog waits for a focusable control before giving up.
const SETTLE_FRAMES: u8 = 4;

/// Registers section `s` drawn in `rect` of `ui`'s layer in this frame (the rows of one kind,
/// as the title row, the document tabs and the ribbon, make one section; each pane is its own).
pub fn section(app: &mut App, ui: &egui::Ui, s: Section, rect: Rect) {
    if !rect.is_positive() {
        return;
    }
    let layer = ui.layer_id();
    let merge = !matches!(s, Section::Pane);
    if merge
        && let Some(e) = app
            .keyboard
            .sections
            .iter_mut()
            .find(|(k, l, _)| *k == s && *l == layer)
    {
        e.2 = e.2.union(rect);
        return;
    }
    app.keyboard.sections.push((s, layer, rect));
}

/// The start of a frame: forgets the sections drawn, notes whether the focus moves by the
/// keyboard (a navigation key) or the pointer (a button press).
pub fn begin(app: &mut App, ctx: &Context) {
    app.keyboard.sections.clear();
    crate::key_tips::input(app, ctx);
    crate::shortcuts_list::keys(app, ctx);
    // the page of the last frame: this frame's registers itself again
    context_key(app, ctx);
    app.keyboard.page = None;
    ctx.input(|i| {
        for e in &i.events {
            match e {
                Event::Key {
                    key:
                        Key::Tab | Key::F6 | Key::ArrowDown | Key::ArrowUp | Key::ArrowLeft | Key::ArrowRight,
                    pressed: true,
                    ..
                } => app.keyboard.keyboard = true,
                Event::PointerButton { pressed: true, .. } => app.keyboard.keyboard = false,
                _ => {}
            }
        }
    });
}

/// The end of a frame, after every panel and dialog was drawn: the dialogs' focus trap and
/// return, Ctrl+F6 and F6, Tab kept inside a section, the focus ring.
pub fn finish(app: &mut App, ctx: &Context) {
    crate::shortcuts_list::show(app, ctx);
    crate::key_tips::finish(app, ctx);
    dialogs(app, ctx);
    menu_focus(app, ctx);
    if app.keyboard.dialogs.is_empty() {
        sections_keys(app, ctx);
        tab_wrap(app, ctx);
    } else {
        app.keyboard.tab_from = None;
    }
    ring(app, ctx);
    app.keyboard.last_focus = ctx.memory(|m| m.focused());
}

/// The rectangle and layer of widget `id` drawn in this frame.
fn widget(ctx: &Context, id: Id) -> Option<(LayerId, Rect)> {
    ctx.viewport(|v| v.this_pass.widgets.get(id).map(|w| (w.layer_id, w.interact_rect)))
}

/// Thinner than this, a focusable widget is a panel's resize handle, not a control.
const MIN_CONTROL: f32 = 11.0;

/// The focusable, enabled controls of layer `layer` drawn in this frame, in drawing order
/// (panel resize handles left out).
fn focusable(ctx: &Context, layer: LayerId) -> Vec<(Id, Rect)> {
    ctx.viewport(|v| {
        v.this_pass
            .widgets
            .get_layer(layer)
            .filter(|w| {
                let r = w.interact_rect;
                w.enabled && w.sense.is_focusable() && r.width().min(r.height()) >= MIN_CONTROL
            })
            .map(|w| (w.id, w.interact_rect))
            .collect()
    })
}

/// Height of a dialog's title bar: its close button is not the control focused on opening.
const TITLE_BAND: f32 = 34.0;

/// The control a dialog on `layer` focuses when it opens: its first one under the title bar.
fn first_in_dialog(ctx: &Context, layer: LayerId) -> Option<Id> {
    let all = focusable(ctx, layer);
    let top = ctx.viewport(|v| {
        v.this_pass
            .widgets
            .get_layer(layer)
            .map(|w| w.rect.min.y)
            .fold(f32::INFINITY, f32::min)
    });
    all.iter()
        .find(|(_, r)| r.center().y > top + TITLE_BAND)
        .or(all.first())
        .map(|(id, _)| *id)
}

/// The index of the smallest section holding `p` on `layer`.
fn section_at(app: &App, layer: LayerId, p: egui::Pos2) -> Option<usize> {
    app.keyboard
        .sections
        .iter()
        .enumerate()
        .filter(|(_, (_, l, r))| *l == layer && r.contains(p))
        .min_by(|(_, a), (_, b)| a.2.area().total_cmp(&b.2.area()))
        .map(|(i, _)| i)
}

/// The controls of section `i`, in drawing order (those of a smaller section inside it, as a
/// pane in the model view, are that section's).
fn controls(app: &App, ctx: &Context, i: usize) -> Vec<Id> {
    let Some(&(_, layer, _)) = app.keyboard.sections.get(i) else {
        return Vec::new();
    };
    focusable(ctx, layer)
        .into_iter()
        .filter(|(_, r)| section_at(app, layer, r.center()) == Some(i))
        .map(|(id, _)| id)
        .collect()
}

/// The sections in the Ctrl+F6 order: by rank, then left to right, then top to bottom.
fn cycle(app: &App) -> Vec<usize> {
    let mut order: Vec<usize> = (0..app.keyboard.sections.len()).collect();
    order.sort_by(|&a, &b| {
        let (sa, _, ra) = app.keyboard.sections[a];
        let (sb, _, rb) = app.keyboard.sections[b];
        sa.rank()
            .cmp(&sb.rank())
            .then(ra.min.x.total_cmp(&rb.min.x))
            .then(ra.min.y.total_cmp(&rb.min.y))
    });
    order
}

/// The section of the focused control, if any.
fn current(app: &App, ctx: &Context) -> Option<usize> {
    let id = ctx.memory(|m| m.focused())?;
    let (layer, r) = widget(ctx, id)?;
    section_at(app, layer, r.center())
}

/// Moves the focus to the next (or previous) section that has a control.
fn move_section(app: &mut App, ctx: &Context, back: bool) -> bool {
    let order = cycle(app);
    let n = order.len();
    if n == 0 {
        return false;
    }
    let at = current(app, ctx).and_then(|c| order.iter().position(|&i| i == c));
    for step in 1..=n {
        let k = match (at, back) {
            (Some(p), false) => (p + step) % n,
            (Some(p), true) => (p + n - step % n) % n,
            (None, false) => step - 1,
            (None, true) => n - step,
        };
        let Some(&i) = order.get(k) else { continue };
        if let Some(&first) = controls(app, ctx, i).first() {
            ctx.memory_mut(|m| m.request_focus(first));
            return true;
        }
    }
    false
}

/// Ctrl+F6 and Ctrl+Shift+F6 cycle the sections; F6 activates the Selection pane when it is
/// open (Shift+F6 and F6 without it move as Ctrl+F6).
fn sections_keys(app: &mut App, ctx: &Context) {
    let Some((ctrl, shift)) = ctx.input(|i| {
        i.events.iter().find_map(|e| match e {
            Event::Key {
                key: Key::F6,
                pressed: true,
                modifiers,
                ..
            } if !modifiers.alt => Some((modifiers.command || modifiers.ctrl, modifiers.shift)),
            _ => None,
        })
    }) else {
        return;
    };
    app.keyboard.keyboard = true;
    if !ctrl && !shift {
        let selection = app
            .keyboard
            .sections
            .iter()
            .position(|(s, _, _)| *s == Section::Selection);
        if let Some(i) = selection
            && current(app, ctx) != Some(i)
            && let Some(&first) = controls(app, ctx, i).first()
        {
            ctx.memory_mut(|m| m.request_focus(first));
            return;
        }
    }
    move_section(app, ctx, shift);
}

/// Tab and Shift+Tab stay in the section: when the focus leaves the section Tab was pressed
/// in, it wraps to the section's first (Shift+Tab: last) control.
fn tab_wrap(app: &mut App, ctx: &Context) {
    let tab = ctx.input(|i| {
        i.events.iter().find_map(|e| match e {
            Event::Key {
                key: Key::Tab,
                pressed: true,
                modifiers,
                ..
            } if !modifiers.ctrl && !modifiers.command && !modifiers.alt => Some(modifiers.shift),
            _ => None,
        })
    });
    if let Some(back) = tab
        && let Some(last) = app.keyboard.last_focus
        && let Some((layer, r)) = widget(ctx, last)
        && let Some(i) = section_at(app, layer, r.center())
        && let Some(&(_, _, rect)) = app.keyboard.sections.get(i)
    {
        app.keyboard.tab_from = Some((rect, layer, back, 3));
    }
    let Some((rect, layer, back, left)) = app.keyboard.tab_from else {
        return;
    };
    let Some(now) = ctx.memory(|m| m.focused()) else {
        // egui gives the focus to the next control in the next frame
        app.keyboard.tab_from = left.checked_sub(1).map(|l| (rect, layer, back, l));
        return;
    };
    if tab.is_some() && Some(now) == app.keyboard.last_focus {
        // the focus moves in the next frame (Shift+Tab)
        app.keyboard.tab_from = left.checked_sub(1).map(|l| (rect, layer, back, l));
        return;
    }
    app.keyboard.tab_from = None;
    let inside = widget(ctx, now).is_some_and(|(l, r)| l == layer && rect.contains(r.center()));
    if inside {
        return;
    }
    let Some(i) = app
        .keyboard
        .sections
        .iter()
        .position(|&(_, l, r)| l == layer && r == rect)
    else {
        return;
    };
    let ids = controls(app, ctx, i);
    let to = if back { ids.last() } else { ids.first() };
    if let Some(&to) = to {
        ctx.memory_mut(|m| m.request_focus(to));
    }
}

/// The open dialogs: each `Middle` layer drawn in this frame. The topmost is egui's modal layer
/// (Tab stays inside it, the window under it does not take clicks); a new one takes the focus
/// (its first control when opened from the keyboard); a closed one gives the focus back.
fn dialogs(app: &mut App, ctx: &Context) {
    let mut open: Vec<LayerId> = ctx.viewport(|v| {
        // a layer drawn before keeps an empty list: only those with widgets are open
        v.this_pass
            .widgets
            .layers()
            .filter(|(l, w)| l.order == Order::Middle && !w.is_empty())
            .map(|(l, _)| *l)
            .collect()
    });
    open.sort_by_key(|l| l.id.value());
    // closed ones, from the top: the focus goes back to what had it before
    let mut back_to = None;
    while let Some(pos) = app.keyboard.dialogs.iter().rposition(|(l, _)| !open.contains(l)) {
        let (_, before) = app.keyboard.dialogs.remove(pos);
        back_to = before;
    }
    for l in open {
        if !app.keyboard.dialogs.iter().any(|(d, _)| *d == l) {
            let before = app
                .keyboard
                .last_focus
                .filter(|id| widget(ctx, *id).is_none_or(|(layer, _)| layer != l));
            app.keyboard.dialogs.push((l, before));
            app.keyboard.settle.push((l, SETTLE_FRAMES));
            back_to = None;
        }
    }
    // a new dialog takes the focus once its controls are laid out (a window's first frame
    // only measures them)
    let settle = std::mem::take(&mut app.keyboard.settle);
    for (l, left) in settle {
        if !app.keyboard.dialogs.iter().any(|(d, _)| *d == l) {
            continue;
        }
        let focus_in = ctx
            .memory(|m| m.focused())
            .and_then(|id| widget(ctx, id))
            .is_some_and(|(layer, _)| layer == l);
        if focus_in {
            continue;
        }
        match first_in_dialog(ctx, l) {
            Some(id) if app.keyboard.keyboard => ctx.memory_mut(|m| m.request_focus(id)),
            None if left > 0 => app.keyboard.settle.push((l, left - 1)),
            _ => {
                if let Some(f) = ctx.memory(|m| m.focused()) {
                    ctx.memory_mut(|m| m.surrender_focus(f));
                }
            }
        }
    }
    if let Some(&(top, _)) = app.keyboard.dialogs.last() {
        ctx.memory_mut(|m| m.set_modal_layer(top));
    } else if let Some(id) = back_to
        && widget(ctx, id).is_some()
    {
        ctx.memory_mut(|m| {
            m.request_focus(id);
            m.move_focus(FocusDirection::None);
        });
    }
}

/// Right arrow expands the focused tree row when it is collapsed, Left arrow collapses it when
/// it is open (Desktop's pane navigation); true when the row toggles. The arrow does not also
/// move the focus.
pub fn tree_toggle(ui: &egui::Ui, resp: &egui::Response, open: bool) -> bool {
    if !resp.has_focus() {
        return false;
    }
    let key = if open { Key::ArrowLeft } else { Key::ArrowRight };
    let hit = ui.input(|i| i.modifiers.is_none() && i.key_pressed(key));
    if hit {
        ui.ctx().memory_mut(|m| m.move_focus(FocusDirection::None));
    }
    hit
}

/// The focus ring: 2 px in the `focus` token around the control focused from the keyboard.
fn ring(app: &App, ctx: &Context) {
    if !app.keyboard.keyboard {
        return;
    }
    let Some(id) = ctx.memory(|m| m.focused()) else {
        return;
    };
    let Some((layer, r)) = widget(ctx, id) else {
        return;
    };
    let r = ctx.layer_transform_to_global(layer).map_or(r, |t| t.mul_rect(r));
    let t = Tokens::of(app.prefs.theme);
    let painter = ctx.layer_painter(LayerId::new(Order::Foreground, Id::unique("bicraft-focus-ring")));
    painter.rect_stroke(
        r.expand(1.0),
        egui::CornerRadius::same(4),
        Stroke::new(2.0, t.focus),
        StrokeKind::Outside,
    );
}

/// The control focused after the last frame, for tests: its layer and rectangle.
#[cfg(test)]
pub(crate) fn focused(ctx: &Context) -> Option<(LayerId, Rect)> {
    let id = ctx.memory(|m| m.focused())?;
    ctx.viewport(|v| v.prev_pass.widgets.get(id).map(|w| (w.layer_id, w.interact_rect)))
}

/// The sections drawn in the last frame, for tests.
#[cfg(test)]
pub(crate) fn sections(app: &App) -> Vec<(Section, Rect)> {
    app.keyboard.sections.iter().map(|&(s, _, r)| (s, r)).collect()
}
