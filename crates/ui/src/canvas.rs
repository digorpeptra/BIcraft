//! Canvas editing (ROADMAP M11.16, as Power BI Desktop's report canvas): the selection of one or
//! several visuals (click, Ctrl+click, a rubber band on the empty page, Ctrl+A), dragging them,
//! the eight resize handles, snapping to the other visuals' edges and centres with smart guides
//! (and to an 8 px grid with View › Snap to grid; View › Gridlines shows it, View › Lock objects
//! stops moves and resizes), arrow-key nudges, and the clipboard: Ctrl+C,
//! Ctrl+X, Ctrl+V, Ctrl+D and the menus' Copy, Cut, Paste and Duplicate, across pages and open
//! documents.
//!
//! Every gesture is one engine command, so one undo step: a drag or a nudge is `visual.move`,
//! the clipboard is `visual.copy` / `visual.paste` / `visual.duplicate`. While a drag runs the
//! page image stays as it is and ghost outlines show where the visuals go; on release the command
//! is sent and the outlines stay until the next snapshot draws the moved visuals. A resize also
//! draws each visual's content from the page image, scaled into its outline, every frame of the
//! drag and after the release until the page rendered at the new size is on screen ([`previews`]):
//! the content follows the handle instead of waiting for the engine and the renderer.
//!
//! The clipboard holds `visual.copy`'s JSON document as text (`{"bicraft": "visuals", …}`): the
//! OS clipboard on the desktop, the browser's async clipboard API in the browser, and always an
//! in-app copy, used when the system clipboard cannot be read (an empty desktop clipboard, a
//! browser that refuses to read it). What is read back is untrusted: its size is capped
//! ([`MAX_CLIP_TEXT`]), it must parse as a clip, and the engine checks it again.

use std::collections::{HashMap, HashSet};

use egui::{
    CornerRadius, CursorIcon, Key, Modifiers, PointerButton, Pos2, Rect, Response, Stroke, StrokeKind, Ui,
    Vec2, pos2, vec2,
};
use serde_json::{Value, json};

use crate::App;
use crate::actions::Call;
use crate::app::{Tab as View, Then};
use crate::i18n::Lang;
use crate::ribbon::{self, VisualSel};
use crate::theme::Tokens;

/// Side of a resize handle (points on the screen).
pub const HANDLE: f32 = 8.0;
/// How close an edge must come to a guide to snap (points on the screen).
pub const SNAP: f32 = 6.0;
/// The grid step of Gridlines and Snap to grid (report pixels).
pub const GRID: f32 = 8.0;
/// Arrow-key steps (report pixels): plain and with Shift.
pub const NUDGE: f32 = 1.0;
pub const NUDGE_BIG: f32 = 10.0;
/// Smallest size a resize leaves (report pixels).
pub const MIN_SIZE: f32 = 10.0;
/// Largest clipboard text read (bytes); more is refused before it is parsed.
pub const MAX_CLIP_TEXT: usize = 8 * 1024 * 1024;
/// Frames a Ctrl+V waits for the shell's paste event before it uses the in-app clipboard.
const PASTE_WAIT: u8 = 3;
/// Most visuals of a page the canvas considers.
const MAX_VISUALS: usize = 2000;
/// Most grid dots drawn.
const MAX_DOTS: usize = 20_000;

/// The shell's clipboard reader puts the text it read here (or why it could not).
pub type ClipSlot = std::rc::Rc<std::cell::RefCell<Option<Result<String, String>>>>;

/// A resize handle of the selection's rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    N,
    S,
    E,
    W,
    NE,
    NW,
    SE,
    SW,
}

impl Handle {
    pub const ALL: [Handle; 8] = [
        Handle::NW,
        Handle::N,
        Handle::NE,
        Handle::E,
        Handle::SE,
        Handle::S,
        Handle::SW,
        Handle::W,
    ];

    /// Where it sits on `r` (fractions of the width and height).
    fn at(self) -> (f32, f32) {
        match self {
            Handle::NW => (0.0, 0.0),
            Handle::N => (0.5, 0.0),
            Handle::NE => (1.0, 0.0),
            Handle::E => (1.0, 0.5),
            Handle::SE => (1.0, 1.0),
            Handle::S => (0.5, 1.0),
            Handle::SW => (0.0, 1.0),
            Handle::W => (0.0, 0.5),
        }
    }

    /// Its centre on `r`.
    pub fn pos(self, r: Rect) -> Pos2 {
        let (fx, fy) = self.at();
        pos2(r.min.x + r.width() * fx, r.min.y + r.height() * fy)
    }

    fn cursor(self) -> CursorIcon {
        match self {
            Handle::N | Handle::S => CursorIcon::ResizeVertical,
            Handle::E | Handle::W => CursorIcon::ResizeHorizontal,
            Handle::NE | Handle::SW => CursorIcon::ResizeNeSw,
            Handle::NW | Handle::SE => CursorIcon::ResizeNwSe,
        }
    }

    fn moves_left(self) -> bool {
        matches!(self, Handle::W | Handle::NW | Handle::SW)
    }
    fn moves_right(self) -> bool {
        matches!(self, Handle::E | Handle::NE | Handle::SE)
    }
    fn moves_top(self) -> bool {
        matches!(self, Handle::N | Handle::NE | Handle::NW)
    }
    fn moves_bottom(self) -> bool {
        matches!(self, Handle::S | Handle::SE | Handle::SW)
    }
}

/// A smart guide: a line the dragged edges snapped to (report pixels).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Guide {
    /// A vertical line at x = `at` (else horizontal at y = `at`).
    pub vertical: bool,
    pub at: f32,
    pub from: f32,
    pub to: f32,
}

/// What a drag on the page does.
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Move,
    Resize(Handle),
    /// A rubber band from the press point; `add` keeps the selection (Ctrl).
    Band {
        add: bool,
    },
}

/// A drag in progress (report pixels).
#[derive(Debug, Clone, PartialEq)]
pub struct Gesture {
    pub page: String,
    pub kind: Kind,
    /// The press point.
    pub start: Pos2,
    /// The visuals and their rectangles when the drag started.
    pub items: Vec<(String, Rect)>,
    /// Where they are now (snapped, kept on the page).
    pub now: Vec<(String, Rect)>,
    /// The band's rectangle.
    pub band: Option<Rect>,
    pub guides: Vec<Guide>,
}

/// The canvas's state: kept by the window for every document (the in-app clipboard serves
/// them all).
#[derive(Default)]
pub struct CanvasUi {
    /// The last clip copied or pasted, in any document (the in-app clipboard).
    pub clip: Option<Value>,
    /// Text to hand to the system clipboard on the next frame.
    os_copy: Option<String>,
    /// The drag in progress.
    pub gesture: Option<Gesture>,
    /// Where the last drag left the visuals, drawn until the next snapshot (its `seq`).
    settle: Option<(u64, Vec<Rect>)>,
    /// After a resize: the visuals' rectangles before and after it, and the snapshot it was
    /// made over; their content is drawn scaled ([`previews`]) until a page image rendered from
    /// a later snapshot is on screen, or the command ended without a new snapshot.
    resized: Option<(u64, Vec<Preview>)>,
    /// A Ctrl+V waiting for the shell's paste event (frames left).
    paste_wait: u8,
    /// A clipboard read the shell is doing for a Paste button.
    reading: Option<ClipSlot>,
    /// Where the last right click on the page was (report pixels): a Paste of its menu goes there.
    pub menu_at: Option<Pos2>,
    /// Where the next paste puts the clip's top-left corner (the menu's Paste), else the copied
    /// place.
    pub paste_at: Option<Pos2>,
    /// Home › Format painter on: the visual whose formatting the next click on a visual copies.
    pub painter: Option<VisualSel>,
}

/// The format painter's command (`visual.copy_format`).
pub const PAINTER: &str = "visual.copy_format";

/// Home › Format painter: on with the selected visual as the source, off when it is on.
pub fn painter_toggle(app: &mut App) {
    let l = app.lang;
    if app.canvas.painter.take().is_some() {
        return;
    }
    let Some(from) = app.cmd.visual.clone() else {
        app.status = l
            .t(
                "Select the visual whose formatting you want to copy.",
                "Sélectionnez le visuel dont vous voulez copier la mise en forme.",
            )
            .into();
        return;
    };
    if let Err(why) = ribbon::state(app, PAINTER) {
        app.status = why;
        return;
    }
    app.status = l
        .t(
            "Click a visual to apply the formatting (Esc cancels).",
            "Cliquez sur un visuel pour appliquer la mise en forme (Échap annule).",
        )
        .into();
    app.canvas.painter = Some(from);
}

/// The format painter's click on visual `to` of page `page`: the source's formatting copied onto
/// it (one undo step), then the painter goes off and `to` is selected.
fn paint_onto(app: &mut App, page: &str, to: &str) {
    let Some(from) = app.canvas.painter.take() else {
        return;
    };
    select(app, page, &[to.to_string()]);
    if from.page == page && from.visual == to {
        return;
    }
    app.run(
        Call {
            id: PAINTER,
            params: json!({"page": from.page, "from": from.visual, "to": [to], "to_page": page}),
        },
        Then::Nothing,
    );
}

/// The brush drawn next to the pointer while the format painter is on.
fn painter_cursor(app: &App, ui: &Ui, resp: &Response) {
    if app.canvas.painter.is_none() {
        return;
    }
    let Some(p) = resp.hover_pos() else {
        return;
    };
    ui.ctx().set_cursor_icon(CursorIcon::Copy);
    let t = Tokens::of(app.prefs.theme);
    let painter = ui.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        egui::Id::unique("bicraft-format-painter-cursor"),
    ));
    // the small buttons' icon size (widgets::small_button)
    let r = Rect::from_min_size(p + vec2(14.0, 10.0), vec2(17.0, 17.0));
    painter.rect_filled(r.expand(2.0), CornerRadius::same(3), t.card);
    painter.rect_stroke(
        r.expand(2.0),
        CornerRadius::same(3),
        Stroke::new(1.0, t.card_border),
        StrokeKind::Inside,
    );
    crate::icons::paint(&painter, r, crate::icons::Icon::Format, t.text, false);
}

/// One visual of the page shown, as the canvas reads the snapshot.
#[derive(Debug, Clone)]
pub struct Item {
    pub name: String,
    pub rect: Rect,
    pub z: f64,
    /// A visual that can be picked: not a group, not hidden.
    pub pickable: bool,
    pub parent: Option<String>,
}

/// The page shown: its name, size and visuals.
pub fn page_items(app: &App) -> Option<(String, Vec2, Vec<Item>)> {
    let snap = &app.loaded.as_ref()?.snap;
    let page = snap.pointer(&format!("/report/pages/{}", app.active_page()))?;
    let name = page.get("name")?.as_str()?.to_string();
    let n = |v: &Value, k: &str| v.get(k).and_then(Value::as_f64).filter(|x| x.is_finite());
    let size = vec2(
        n(page, "width")
            .filter(|w| *w > 0.0)
            .unwrap_or(1280.0)
            .min(20_000.0) as f32,
        n(page, "height")
            .filter(|h| *h > 0.0)
            .unwrap_or(720.0)
            .min(20_000.0) as f32,
    );
    let items = page
        .get("visuals")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(MAX_VISUALS)
        .collect::<Vec<_>>();
    // A hidden group hides its members: their visuals cannot be picked either.
    let lower = |s: &str| s.to_lowercase();
    let mut parents: HashMap<String, String> = HashMap::new();
    let mut hidden: HashSet<String> = HashSet::new();
    for v in &items {
        let Some(n) = v.get("name").and_then(Value::as_str) else {
            continue;
        };
        if let Some(p) = v.get("parent_group").and_then(Value::as_str)
            && !p.is_empty()
        {
            parents.insert(lower(n), lower(p));
        }
        if v.get("hidden").and_then(Value::as_bool) == Some(true) {
            hidden.insert(lower(n));
        }
    }
    let hidden_above = |name: &str| {
        let mut cur = parents.get(name).cloned();
        for _ in 0..32 {
            let Some(p) = cur else { return false };
            if hidden.contains(&p) {
                return true;
            }
            cur = parents.get(&p).cloned();
        }
        false
    };
    let items = items
        .into_iter()
        .filter_map(|v| {
            let (x, y, w, h) = (n(v, "x")?, n(v, "y")?, n(v, "width")?, n(v, "height")?);
            let name = v.get("name")?.as_str()?.to_string();
            Some(Item {
                pickable: v.get("type").is_some_and(Value::is_string)
                    && v.get("hidden").and_then(Value::as_bool) != Some(true)
                    && !hidden_above(&lower(&name)),
                rect: Rect::from_min_size(pos2(x as f32, y as f32), vec2(w as f32, h as f32)),
                z: n(v, "z").unwrap_or(0.0),
                parent: v.get("parent_group").and_then(Value::as_str).map(str::to_string),
                name,
            })
        })
        .collect();
    Some((name, size, items))
}

/// The selected visuals of the page shown, the primary one (the Visual tab's) first.
pub fn selection(app: &App) -> Vec<String> {
    let Some(p) = &app.cmd.visual else {
        return Vec::new();
    };
    let mut out = vec![p.visual.clone()];
    for m in &app.cmd.more {
        if !out.contains(m) {
            out.push(m.clone());
        }
    }
    out
}

/// Selects `names` of page `page` (the first one primary); none clears the selection.
pub fn select(app: &mut App, page: &str, names: &[String]) {
    match names.split_first() {
        Some((first, rest)) => {
            app.cmd.visual = Some(VisualSel {
                page: page.to_string(),
                visual: first.clone(),
            });
            app.cmd.more = rest.to_vec();
        }
        None => {
            app.cmd.visual = None;
            app.cmd.more.clear();
        }
    }
}

/// Whether the canvas may change the report (the session lets `visual.move` run).
fn editable(app: &App) -> bool {
    // the phone canvas edits the mobile layout (crate::mobile)
    let id = if crate::mobile::drawn(app) {
        "visual.mobile"
    } else {
        "visual.move"
    };
    ribbon::state(app, id).is_ok()
}

/// The rectangle around the selected visuals.
fn sel_bounds(items: &[Item], sel: &[String]) -> Option<Rect> {
    items
        .iter()
        .filter(|i| sel.contains(&i.name))
        .map(|i| i.rect)
        .reduce(|a, b| a.union(b))
}

/// The topmost pickable visual at report point `p`.
fn hit(items: &[Item], p: Pos2) -> Option<&Item> {
    items
        .iter()
        .filter(|i| i.pickable && i.rect.contains(p))
        .max_by(|a, b| a.z.total_cmp(&b.z))
}

/// The groups above visual `name`, outermost first, then `name` itself (bounded, cycles cut).
fn chain(items: &[Item], name: &str) -> Vec<String> {
    let mut out = vec![name.to_string()];
    let mut cur = items
        .iter()
        .find(|i| i.name == name)
        .and_then(|i| i.parent.clone());
    for _ in 0..32 {
        let Some(p) = cur.filter(|p| !p.is_empty()) else {
            break;
        };
        if out.iter().any(|x| x.eq_ignore_ascii_case(&p)) {
            break;
        }
        let Some(g) = items.iter().find(|i| i.name.eq_ignore_ascii_case(&p)) else {
            break;
        };
        out.insert(0, g.name.clone());
        cur = g.parent.clone();
    }
    out
}

/// What a click on visual `name` selects, as in Desktop: its outermost group first; a click on a
/// visual of the selected group selects the next level inside it, down to the visual itself.
pub fn click_target(items: &[Item], sel: &[String], name: &str) -> String {
    let c = chain(items, name);
    match c.iter().rposition(|x| sel.contains(x)) {
        Some(k) => c.get(k + 1).or_else(|| c.get(k)).cloned(),
        None => c.first().cloned(),
    }
    .unwrap_or_else(|| name.to_string())
}

/// Whether visual `name` is selected, itself or through one of its groups.
fn covered(items: &[Item], sel: &[String], name: &str) -> bool {
    chain(items, name).iter().any(|x| sel.contains(x))
}

fn to_screen(page: Rect, zoom: f32, r: Rect) -> Rect {
    Rect::from_min_size(page.min + r.min.to_vec2() * zoom, r.size() * zoom)
}

/// The handle under screen point `p` of the selection's rectangle `b` (report pixels).
fn handle_at(page: Rect, zoom: f32, b: Rect, p: Pos2) -> Option<Handle> {
    let s = to_screen(page, zoom, b);
    // the drill icons in a visual's top-right corner keep their clicks
    if crate::drill::icon_rects(s).iter().any(|(_, r)| r.contains(p)) {
        return None;
    }
    Handle::ALL
        .into_iter()
        .find(|h| Rect::from_center_size(h.pos(s), Vec2::splat(HANDLE + 4.0)).contains(p))
}

/// The page's pointer: clicks select (Ctrl+click adds or removes), drags move, resize or draw a
/// rubber band. Called by the page's input (`crate::menus::on_page`) every frame.
pub fn pointer(app: &mut App, ui: &Ui, resp: &Response, rect: Rect, zoom: f32) {
    let Some((page, size, items)) = page_items(app) else {
        app.canvas.gesture = None;
        return;
    };
    // a visual asked for on another page (Best practices' Go to visual) is selected once its
    // page is shown
    if let Some(r) = app.cmd.reveal.take() {
        if r.page != page {
            app.cmd.reveal = Some(r);
        } else if items.iter().any(|i| i.name == r.visual) {
            select(app, &page, std::slice::from_ref(&r.visual));
        }
    }
    // the selection lives on one page and names visuals that exist (a visual just placed on the
    // phone canvas is selected before its snapshot comes: crate::mobile)
    if app.cmd.visual.as_ref().is_some_and(|v| v.page != page)
        || app.cmd.visual.as_ref().is_some_and(|v| {
            !items.iter().any(|i| i.name == v.visual) && !crate::mobile::placing(app, &v.visual)
        })
    {
        select(app, &page, &[]);
    }
    let names: HashSet<String> = items.iter().map(|i| i.name.clone()).collect();
    app.cmd.more.retain(|m| names.contains(m));
    if app.canvas.gesture.as_ref().is_some_and(|g| g.page != page) {
        app.canvas.gesture = None;
    }
    let to_report = |p: Pos2| pos2((p.x - rect.min.x) / zoom, (p.y - rect.min.y) / zoom);
    let ctrl = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);
    let sel = selection(app);
    // Format painter on: a click on a visual applies the formatting, elsewhere cancels; no drag
    if app.canvas.painter.is_some() {
        painter_cursor(app, ui, resp);
        if resp.clicked()
            && let Some(pos) = resp.interact_pointer_pos()
        {
            match hit(&items, to_report(pos)).map(|i| i.name.clone()) {
                Some(n) => paint_onto(app, &page, &n),
                None => app.canvas.painter = None,
            }
        } else if resp.secondary_clicked() {
            app.canvas.painter = None;
        }
        app.canvas.gesture = None;
        return;
    }
    // clicks
    if (resp.clicked() || resp.secondary_clicked())
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let at = hit(&items, to_report(pos)).map(|i| i.name.clone());
        match at {
            Some(n) if ctrl && resp.clicked() => {
                // Ctrl+click adds or removes the visual's outermost group (or what of its
                // chain is selected)
                let c = chain(&items, &n);
                let mut s = sel.clone();
                if let Some(i) = s.iter().position(|x| c.contains(x)) {
                    s.remove(i);
                } else if let Some(head) = c.first() {
                    s.insert(0, head.clone());
                }
                select(app, &page, &s);
            }
            // a right click on the selection keeps it (its menu acts on all of it)
            Some(n) if resp.secondary_clicked() && covered(&items, &sel, &n) => {}
            Some(n) => {
                let t = click_target(&items, &sel, &n);
                select(app, &page, &[t]);
            }
            None if ctrl => {}
            None => select(app, &page, &[]),
        }
    }
    let edit = editable(app);
    // View › Lock objects: no move or resize, a rubber band still selects
    let locked = app.prefs.lock_objects;
    // the cursor over a handle or a selected visual
    if edit
        && !locked
        && app.canvas.gesture.is_none()
        && let Some(p) = resp.hover_pos()
    {
        let sel = selection(app);
        if let Some(h) = sel_bounds(&items, &sel).and_then(|b| handle_at(rect, zoom, b, p)) {
            ui.ctx().set_cursor_icon(h.cursor());
        } else if hit(&items, to_report(p)).is_some_and(|i| covered(&items, &sel, &i.name)) {
            ui.ctx().set_cursor_icon(CursorIcon::Move);
        }
    }
    // a drag starts
    if edit && resp.drag_started_by(PointerButton::Primary) {
        let origin = ui
            .input(|i| i.pointer.press_origin())
            .or(resp.interact_pointer_pos());
        if let Some(o) = origin {
            let start = to_report(o);
            let mut sel = selection(app);
            let handle = sel_bounds(&items, &sel).and_then(|b| handle_at(rect, zoom, b, o));
            let kind = match (handle, hit(&items, start)) {
                (Some(_), _) | (None, Some(_)) if locked => {
                    app.status = locked_text(app.lang).to_string();
                    None
                }
                (Some(h), _) => Some(Kind::Resize(h)),
                (None, Some(i)) => {
                    // a drag on a visual of a selected group moves the group
                    if !covered(&items, &sel, &i.name) {
                        let head = chain(&items, &i.name)
                            .first()
                            .cloned()
                            .unwrap_or_else(|| i.name.clone());
                        if ctrl {
                            sel.insert(0, head);
                        } else {
                            sel = vec![head];
                        }
                        select(app, &page, &sel);
                    }
                    Some(Kind::Move)
                }
                (None, None) => Some(Kind::Band { add: ctrl }),
            };
            if let Some(kind) = kind {
                let picked: Vec<(String, Rect)> = items
                    .iter()
                    .filter(|i| sel.contains(&i.name) && !matches!(kind, Kind::Band { .. }))
                    .map(|i| (i.name.clone(), i.rect))
                    .collect();
                app.canvas.gesture = Some(Gesture {
                    page: page.clone(),
                    kind,
                    start,
                    now: picked.clone(),
                    items: picked,
                    band: None,
                    guides: Vec::new(),
                });
            }
        }
    }
    // it goes on
    // the phone canvas snaps to Desktop's 18 × 20 grid, always; the page to View › Snap to grid
    let grid = if crate::mobile::drawn(app) {
        Some(crate::mobile::GRID)
    } else {
        app.prefs.snap_grid.then_some(Vec2::splat(GRID))
    };
    if let Some(g) = &mut app.canvas.gesture
        && let Some(p) = ui.input(|i| i.pointer.interact_pos())
        && resp.dragged_by(PointerButton::Primary)
    {
        let mods = ui.input(|i| i.modifiers);
        let free = mods.alt;
        let others: Vec<Rect> = items
            .iter()
            .filter(|i| i.pickable && !g.items.iter().any(|(n, _)| *n == i.name))
            .map(|i| i.rect)
            .collect();
        update(
            g,
            to_report(p),
            &others,
            size,
            SNAP / zoom,
            grid.filter(|_| !free),
            mods,
        );
        ui.ctx().request_repaint();
    }
    if resp.drag_stopped_by(PointerButton::Primary)
        && let Some(g) = app.canvas.gesture.take()
    {
        finish(app, g, &items);
    }
}

/// Moves the gesture to report point `p`: the rectangles follow, snapped to `others`' edges and
/// centres and the page's (or the grid), kept on the page. Alt (`mods.alt`) moves freely, without
/// snapping; Shift (`mods.shift`) keeps the aspect ratio of a resize.
pub fn update(
    g: &mut Gesture,
    p: Pos2,
    others: &[Rect],
    page: Vec2,
    snap: f32,
    grid: Option<Vec2>,
    mods: Modifiers,
) {
    let free = mods.alt;
    let d = p - g.start;
    g.guides.clear();
    match g.kind.clone() {
        Kind::Band { .. } => g.band = Some(Rect::from_two_pos(g.start, p)),
        Kind::Move => {
            let Some(b0) = g.items.iter().map(|(_, r)| *r).reduce(|a, b| a.union(b)) else {
                return;
            };
            let b = b0.translate(d);
            let (mut dx, mut dy) = (d.x, d.y);
            if !free {
                let (sx, gx) = snap_axis(&[b.min.x, b.center().x, b.max.x], others, page.x, snap, true);
                let (sy, gy) = snap_axis(&[b.min.y, b.center().y, b.max.y], others, page.y, snap, false);
                match sx {
                    Some(s) => dx += s,
                    None => {
                        if let Some(s) = grid {
                            dx += (b.min.x / s.x).round() * s.x - b.min.x;
                        }
                    }
                }
                match sy {
                    Some(s) => dy += s,
                    None => {
                        if let Some(s) = grid {
                            dy += (b.min.y / s.y).round() * s.y - b.min.y;
                        }
                    }
                }
                g.guides.extend(gx);
                g.guides.extend(gy);
            }
            // kept on the page when it fits
            if b0.width() <= page.x {
                dx = clampf(dx, -b0.min.x, page.x - b0.max.x);
            }
            if b0.height() <= page.y {
                dy = clampf(dy, -b0.min.y, page.y - b0.max.y);
            }
            let t = vec2(dx, dy);
            g.now = g.items.iter().map(|(n, r)| (n.clone(), r.translate(t))).collect();
            let moved = b0.translate(t);
            g.guides.retain(|gd| on_edge(gd, moved));
            for gd in &mut g.guides {
                if gd.vertical {
                    gd.from = gd.from.min(moved.min.y);
                    gd.to = gd.to.max(moved.max.y);
                } else {
                    gd.from = gd.from.min(moved.min.x);
                    gd.to = gd.to.max(moved.max.x);
                }
            }
        }
        Kind::Resize(h) => {
            let Some(b0) = g.items.iter().map(|(_, r)| *r).reduce(|a, b| a.union(b)) else {
                return;
            };
            let (mut x0, mut y0, mut x1, mut y1) = (b0.min.x, b0.min.y, b0.max.x, b0.max.y);
            let edge = |v: f32, vertical: bool, guides: &mut Vec<Guide>| -> f32 {
                if free {
                    return v;
                }
                let lim = if vertical { page.x } else { page.y };
                match snap_axis(&[v], others, lim, snap, vertical) {
                    (Some(s), gs) => {
                        guides.extend(gs);
                        v + s
                    }
                    (None, _) => match grid {
                        Some(s) => {
                            let step = if vertical { s.x } else { s.y };
                            (v / step).round() * step
                        }
                        None => v,
                    },
                }
            };
            let mut guides = Vec::new();
            if h.moves_left() {
                x0 = clampf(
                    edge(x0 + d.x, true, &mut guides),
                    0.0_f32.min(b0.min.x),
                    x1 - MIN_SIZE,
                );
            }
            if h.moves_right() {
                x1 = clampf(
                    edge(x1 + d.x, true, &mut guides),
                    x0 + MIN_SIZE,
                    page.x.max(b0.max.x).max(x0 + MIN_SIZE),
                );
            }
            if h.moves_top() {
                y0 = clampf(
                    edge(y0 + d.y, false, &mut guides),
                    0.0_f32.min(b0.min.y),
                    y1 - MIN_SIZE,
                );
            }
            if h.moves_bottom() {
                y1 = clampf(
                    edge(y1 + d.y, false, &mut guides),
                    y0 + MIN_SIZE,
                    page.y.max(b0.max.y).max(y0 + MIN_SIZE),
                );
            }
            if mods.shift && b0.width() > 0.0 && b0.height() > 0.0 {
                (x0, y0, x1, y1) = keep_ratio(h, b0, (x0, y0, x1, y1));
            }
            let nb = Rect::from_min_max(pos2(x0, y0), pos2(x1, y1));
            let (sx, sy) = (
                if b0.width() > 0.0 {
                    nb.width() / b0.width()
                } else {
                    1.0
                },
                if b0.height() > 0.0 {
                    nb.height() / b0.height()
                } else {
                    1.0
                },
            );
            g.now = g
                .items
                .iter()
                .map(|(n, r)| {
                    let min = pos2(
                        nb.min.x + (r.min.x - b0.min.x) * sx,
                        nb.min.y + (r.min.y - b0.min.y) * sy,
                    );
                    let size = vec2((r.width() * sx).max(1.0), (r.height() * sy).max(1.0));
                    (n.clone(), Rect::from_min_size(min, size))
                })
                .collect();
            guides.retain(|gd| on_edge(gd, nb));
            for gd in &mut guides {
                if gd.vertical {
                    gd.from = gd.from.min(nb.min.y);
                    gd.to = gd.to.max(nb.max.y);
                } else {
                    gd.from = gd.from.min(nb.min.x);
                    gd.to = gd.to.max(nb.max.x);
                }
            }
            g.guides = guides;
        }
    }
}

/// The resized rectangle `(x0, y0, x1, y1)` brought back to the aspect ratio of `b0`: a corner
/// handle takes the larger of the two scales and keeps the opposite corner; a side handle scales
/// the other side about its centre.
fn keep_ratio(h: Handle, b0: Rect, (x0, y0, x1, y1): (f32, f32, f32, f32)) -> (f32, f32, f32, f32) {
    let (sx, sy) = ((x1 - x0) / b0.width(), (y1 - y0) / b0.height());
    let horizontal = h.moves_left() || h.moves_right();
    let vertical = h.moves_top() || h.moves_bottom();
    let s = match (horizontal, vertical) {
        (true, true) => sx.max(sy),
        (true, false) => sx,
        _ => sy,
    };
    let min = MIN_SIZE / b0.width().min(b0.height());
    let s = if s.is_finite() { s.max(min) } else { 1.0 };
    let (w, hh) = (b0.width() * s, b0.height() * s);
    let (nx0, nx1) = if h.moves_left() {
        (x1 - w, x1)
    } else if h.moves_right() {
        (x0, x0 + w)
    } else {
        let c = b0.center().x;
        (c - w / 2.0, c + w / 2.0)
    };
    let (ny0, ny1) = if h.moves_top() {
        (y1 - hh, y1)
    } else if h.moves_bottom() {
        (y0, y0 + hh)
    } else {
        let c = b0.center().y;
        (c - hh / 2.0, c + hh / 2.0)
    };
    (nx0, ny0, nx1, ny1)
}

/// Whether guide `gd` touches an edge or the centre of `r` (after the move).
fn on_edge(gd: &Guide, r: Rect) -> bool {
    let vals = if gd.vertical {
        [r.min.x, r.center().x, r.max.x]
    } else {
        [r.min.y, r.center().y, r.max.y]
    };
    vals.iter().any(|v| (v - gd.at).abs() < 0.01)
}

/// The smallest shift (within `snap`) that puts one of `vals` on a line of `others` (their
/// edges and centres) or of the page (edges, centre); with the guides of every line it lands on.
fn snap_axis(
    vals: &[f32],
    others: &[Rect],
    page: f32,
    snap: f32,
    vertical: bool,
) -> (Option<f32>, Vec<Guide>) {
    let mut lines: Vec<(f32, f32, f32)> = vec![(0.0, 0.0, 0.0), (page / 2.0, 0.0, 0.0), (page, 0.0, 0.0)];
    for r in others {
        let (a, b, c, from, to) = if vertical {
            (r.min.x, r.center().x, r.max.x, r.min.y, r.max.y)
        } else {
            (r.min.y, r.center().y, r.max.y, r.min.x, r.max.x)
        };
        lines.extend([(a, from, to), (b, from, to), (c, from, to)]);
    }
    let mut best: Option<f32> = None;
    for v in vals {
        for (l, _, _) in &lines {
            let s = l - v;
            if s.abs() <= snap && best.is_none_or(|b| s.abs() < b.abs()) {
                best = Some(s);
            }
        }
    }
    let Some(s) = best else { return (None, Vec::new()) };
    let mut guides: Vec<Guide> = Vec::new();
    for v in vals {
        for (l, from, to) in &lines {
            if (l - (v + s)).abs() < 0.01 {
                match guides.iter_mut().find(|g| (g.at - l).abs() < 0.01) {
                    Some(g) => {
                        g.from = g.from.min(*from);
                        g.to = g.to.max(*to);
                    }
                    None => guides.push(Guide {
                        vertical,
                        at: *l,
                        from: *from,
                        to: *to,
                    }),
                }
            }
        }
    }
    (Some(s), guides)
}

/// The end of a drag: a move or a resize is one `visual.move`; a band selects what it touches.
fn finish(app: &mut App, g: Gesture, items: &[Item]) {
    match g.kind {
        Kind::Band { add } => {
            let Some(band) = g.band else { return };
            let mut sel = if add { selection(app) } else { Vec::new() };
            let mut hits: Vec<&Item> = items
                .iter()
                .filter(|i| i.pickable && i.rect.intersects(band))
                .collect();
            hits.sort_by(|a, b| b.z.total_cmp(&a.z));
            for i in hits {
                if !sel.contains(&i.name) {
                    sel.push(i.name.clone());
                }
            }
            select(app, &g.page, &sel);
        }
        Kind::Move | Kind::Resize(_) => {
            let rows: Vec<Value> = g
                .now
                .iter()
                .zip(&g.items)
                .filter_map(|((n, r), (_, r0))| {
                    let r = round(*r);
                    (r != round(*r0)).then(|| {
                        json!({"visual": n, "x": r.min.x, "y": r.min.y, "width": r.width(), "height": r.height()})
                    })
                })
                .collect();
            if rows.is_empty() {
                return;
            }
            let seq = app.loaded.as_ref().map_or(0, |l| l.seq);
            app.canvas.settle = Some((seq, g.now.iter().map(|(_, r)| round(*r)).collect()));
            if matches!(g.kind, Kind::Resize(_)) {
                let moved = g
                    .items
                    .iter()
                    .zip(&g.now)
                    .map(|((n, r0), (_, r))| (n.clone(), *r0, round(*r)))
                    .collect();
                app.canvas.resized = Some((seq, moved));
            }
            let params = json!({"page": g.page, "visuals": rows});
            // on the phone canvas the mobile layout moves (crate::mobile)
            if crate::mobile::drawn(app) {
                crate::mobile::send(app, params);
            } else {
                ribbon::invoke(app, "visual.move", params);
            }
        }
    }
}

/// A rectangle on whole report pixels.
fn round(r: Rect) -> Rect {
    Rect::from_min_size(
        pos2(r.min.x.round(), r.min.y.round()),
        vec2(r.width().round().max(1.0), r.height().round().max(1.0)),
    )
}

/// Draws the selection (rings, the handles), the drag's ghost outlines, smart guides, the band
/// and the grid (View › Gridlines).
pub fn paint(app: &mut App, ui: &Ui, rect: Rect, zoom: f32) {
    let Some((_, size, items)) = page_items(app) else {
        return;
    };
    let t = Tokens::of(app.prefs.theme);
    let p = ui.painter().with_clip_rect(rect.expand(HANDLE));
    let sel = selection(app);
    let screen = |r: Rect| to_screen(rect, zoom, r);
    // View › Gridlines, for the author (the phone canvas shows its grid while a drag runs)
    let g = app.canvas.gesture.clone();
    let dragging = g.as_ref().is_some_and(|g| !matches!(g.kind, Kind::Band { .. }));
    if (app.prefs.show_gridlines && editable(app)) || (crate::mobile::drawn(app) && dragging) {
        let step = if crate::mobile::drawn(app) {
            crate::mobile::GRID
        } else {
            Vec2::splat(GRID)
        };
        grid(&p, rect, zoom, size, step, t.border.gamma_multiply(0.7));
    }
    for i in items.iter().filter(|i| sel.contains(&i.name)) {
        p.rect_stroke(
            screen(i.rect),
            CornerRadius::same(2),
            Stroke::new(2.0, t.ring),
            StrokeKind::Outside,
        );
    }
    // a resize: the visuals' content scaled into their new rectangles, over their old place
    let shown = previews(app).unwrap_or_else(|| {
        app.canvas.resized = None;
        Vec::new()
    });
    if let Some((tex, _)) = app.page_texture() {
        for (_, from, _) in &shown {
            let old = screen(*from);
            let under = app.backdrop_at(old.expand(6.0)).unwrap_or(egui::Color32::WHITE);
            p.rect_filled(old, CornerRadius::ZERO, under);
        }
        for (_, from, to) in &shown {
            let uv = Rect::from_min_max(
                pos2(from.min.x / size.x, from.min.y / size.y),
                pos2(from.max.x / size.x, from.max.y / size.y),
            );
            p.image(tex, screen(*to), uv, egui::Color32::WHITE);
        }
    }
    // ghosts and the band: the ring colour, see-through over the page
    let veil = egui::Color32::from_rgba_unmultiplied(t.ring.r(), t.ring.g(), t.ring.b(), 36);
    if let Some(g) = &g {
        for (n, r) in &g.now {
            // a visual drawn at its new size needs no veil over its content
            if !shown.iter().any(|(s, _, _)| s == n) {
                p.rect_filled(screen(*r), CornerRadius::same(2), veil);
            }
            p.rect_stroke(
                screen(*r),
                CornerRadius::same(2),
                Stroke::new(1.5, t.ring),
                StrokeKind::Middle,
            );
        }
        for gd in &g.guides {
            let (a, b) = if gd.vertical {
                (pos2(gd.at, gd.from), pos2(gd.at, gd.to))
            } else {
                (pos2(gd.from, gd.at), pos2(gd.to, gd.at))
            };
            let (a, b) = (rect.min + a.to_vec2() * zoom, rect.min + b.to_vec2() * zoom);
            p.extend(egui::Shape::dashed_line(
                &[a, b],
                Stroke::new(1.0, t.guide),
                4.0,
                3.0,
            ));
        }
        if let Some(b) = g.band {
            p.rect_filled(screen(b), CornerRadius::ZERO, veil);
            p.rect_stroke(
                screen(b),
                CornerRadius::ZERO,
                Stroke::new(1.0, t.ring),
                StrokeKind::Middle,
            );
        }
    } else if let Some((seq, rs)) = app.canvas.settle.clone() {
        if app.loaded.as_ref().is_some_and(|l| l.seq == seq) {
            for r in rs {
                p.rect_stroke(
                    screen(r),
                    CornerRadius::same(2),
                    Stroke::new(1.5, t.ring),
                    StrokeKind::Middle,
                );
            }
        } else {
            app.canvas.settle = None;
        }
    }
    // the handles of the selection's rectangle
    let bounds = match &g {
        Some(Gesture {
            kind: Kind::Band { .. },
            ..
        }) => None,
        Some(g) => g.now.iter().map(|(_, r)| *r).reduce(|a, b| a.union(b)),
        None => sel_bounds(&items, &sel),
    };
    if editable(app)
        && !app.prefs.lock_objects
        && let Some(b) = bounds
    {
        let s = screen(b);
        for h in Handle::ALL {
            let hr = Rect::from_center_size(h.pos(s), Vec2::splat(HANDLE));
            p.rect_filled(hr, CornerRadius::same(1), t.handle_fill);
            p.rect_stroke(
                hr,
                CornerRadius::same(1),
                Stroke::new(1.0, t.handle_stroke),
                StrokeKind::Inside,
            );
        }
    }
}

/// A visual drawn from the page image elsewhere: its name, where the image has it, where it is
/// drawn (report pixels).
pub(crate) type Preview = (String, Rect, Rect);

/// The visuals whose content is drawn from the page image at another rectangle than the image
/// holds: `(name, where the image has it, where it is drawn)`, report pixels. During a resize,
/// every resized visual at its outline; after the release, at the size sent, until the page on
/// screen was rendered from a later snapshot than the resize's, or the command ended without one
/// (refused: the visual keeps its size). `None`: the after-release state is over.
pub(crate) fn previews(app: &App) -> Option<Vec<Preview>> {
    let (_, drawn) = app.page_texture()?;
    if let Some(g) = &app.canvas.gesture
        && matches!(g.kind, Kind::Resize(_))
    {
        return Some(
            g.items
                .iter()
                .zip(&g.now)
                .map(|((n, r0), (_, r))| (n.clone(), *r0, *r))
                .collect(),
        );
    }
    let Some((seq, moved)) = &app.canvas.resized else {
        return Some(Vec::new());
    };
    let latest = app.loaded.as_ref().map_or(0, |l| l.seq);
    let idle = app.calls_pending() == 0;
    // the latest snapshot has every visual at its new rectangle (whole report pixels)
    let at_target = || {
        page_items(app).is_some_and(|(_, _, items)| {
            moved.iter().all(|(n, _, to)| {
                items.iter().any(|i| {
                    i.name == *n
                        && (i.rect.min - to.min).length() < 0.5
                        && (i.rect.max - to.max).length() < 0.5
                })
            })
        })
    };
    let done = drawn > *seq && (idle || at_target());
    (!done && !(idle && latest == *seq)).then(|| moved.clone())
}

/// Grid dots every `step` (report pixels, per axis; doubled until they are 10 points apart).
fn grid(p: &egui::Painter, rect: Rect, zoom: f32, size: Vec2, step: Vec2, col: egui::Color32) {
    let mut step = step.max(Vec2::splat(1.0));
    while step.min_elem() * zoom < 10.0 && step.max_elem() < 1024.0 {
        step *= 2.0;
    }
    let (nx, ny) = ((size.x / step.x) as usize, (size.y / step.y) as usize);
    if nx.saturating_mul(ny) > MAX_DOTS {
        return;
    }
    for i in 0..=nx {
        for j in 0..=ny {
            let c = rect.min + vec2(i as f32 * step.x, j as f32 * step.y) * zoom;
            p.rect_filled(
                Rect::from_center_size(c, Vec2::splat(1.5)),
                CornerRadius::ZERO,
                col,
            );
        }
    }
}

/// The status bar's text while a drag runs: the rectangle of what moves.
pub fn status(app: &App) -> Option<String> {
    let g = app.canvas.gesture.as_ref()?;
    let r = match g.kind {
        Kind::Band { .. } => g.band?,
        _ => g.now.iter().map(|(_, r)| *r).reduce(|a, b| a.union(b))?,
    };
    let r = round(r);
    Some(format!(
        "x {} · y {} · {} × {}",
        r.min.x,
        r.min.y,
        r.width(),
        r.height()
    ))
}

// ---------------------------------------------------------------------------------------------
// Keyboard and clipboard
// ---------------------------------------------------------------------------------------------

/// The canvas's keys on the Report view, when nothing else takes the keyboard: Ctrl+C and Ctrl+V
/// (the shells' copy and paste events), Ctrl+A, the arrows (Shift: 10 px), Escape (drops a drag).
pub fn keys(app: &mut App, ui: &Ui) {
    if let Some(text) = app.canvas.os_copy.take() {
        ui.ctx().copy_text(text);
    }
    poll_read(app);
    if app.canvas.reading.is_some() {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(50));
    }
    // focus mode is a reading view: no editing of the canvas (crate::focus); the open Enter data
    // grid takes Ctrl+V itself (crate::enter_data)
    if app.tab != View::Report
        || ui.ctx().egui_wants_keyboard_input()
        || crate::focus::active(app)
        || app.cmd.enter_data.is_some()
    {
        app.canvas.paste_wait = 0;
        return;
    }
    // the phone canvas has keys of its own (crate::mobile)
    if crate::mobile::editing(app) {
        app.canvas.paste_wait = 0;
        crate::mobile::keys(app, ui);
        return;
    }
    let (copy, cut, pasted, ctrl_v) = ui.input(|i| {
        let mut copy = false;
        let mut cut = false;
        let mut pasted = Vec::new();
        let mut ctrl_v = false;
        for e in &i.events {
            match e {
                egui::Event::Copy => copy = true,
                egui::Event::Cut => cut = true,
                egui::Event::Key {
                    key: Key::X,
                    pressed: true,
                    modifiers,
                    ..
                } if modifiers.command && !modifiers.shift && !modifiers.alt => cut = true,
                egui::Event::Paste(s) => pasted.push(s.clone()),
                egui::Event::Key {
                    key: Key::C,
                    pressed: true,
                    modifiers,
                    ..
                } if modifiers.command && !modifiers.shift && !modifiers.alt => copy = true,
                egui::Event::Key {
                    key: Key::V,
                    pressed: true,
                    modifiers,
                    ..
                } if modifiers.command && !modifiers.shift && !modifiers.alt => ctrl_v = true,
                _ => {}
            }
        }
        (copy, cut, pasted, ctrl_v)
    });
    if cut {
        self::cut(app);
    } else if copy {
        self::copy(app);
    }
    // Ctrl+V pastes at the copied place, not where a menu was opened
    if ctrl_v || !pasted.is_empty() {
        app.canvas.paste_at = None;
    }
    if let Some(text) = pasted.into_iter().next() {
        app.canvas.paste_wait = 0;
        paste_text(app, &text, true);
    } else if ctrl_v {
        app.canvas.paste_wait = PASTE_WAIT;
    } else if app.canvas.paste_wait > 0 {
        app.canvas.paste_wait -= 1;
        if app.canvas.paste_wait == 0 {
            paste_in_app(app);
        }
    }
    if app.canvas.gesture.is_some() && ui.input(|i| i.key_pressed(Key::Escape)) {
        app.canvas.gesture = None;
    }
    // Esc turns the format painter off
    if app.canvas.painter.is_some() && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        app.canvas.painter = None;
    }
    let focused = ui.ctx().memory(|m| m.focused().is_some());
    if focused || app.form.is_some() || app.palette.is_some() || app.confirm.is_some() {
        return;
    }
    if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::A))
        && let Some((page, _, items)) = page_items(app)
    {
        let mut all: Vec<&Item> = items.iter().filter(|i| i.pickable).collect();
        all.sort_by(|a, b| b.z.total_cmp(&a.z));
        let names: Vec<String> = all.into_iter().map(|i| i.name.clone()).collect();
        select(app, &page, &names);
    }
    // a visual's data points from the keyboard (Ctrl+Right, the arrows, Enter, Escape)
    if app.canvas.gesture.is_none() && crate::point_keys::keys(app, ui) {
        return;
    }
    if app.cmd.visual.is_none() || app.canvas.gesture.is_some() {
        return;
    }
    let mut d = Vec2::ZERO;
    for (key, v) in [
        (Key::ArrowLeft, vec2(-1.0, 0.0)),
        (Key::ArrowRight, vec2(1.0, 0.0)),
        (Key::ArrowUp, vec2(0.0, -1.0)),
        (Key::ArrowDown, vec2(0.0, 1.0)),
    ] {
        // Shift first: a plain chord matches with Shift held too
        if ui.input_mut(|i| i.consume_key(Modifiers::SHIFT, key)) {
            d += v * NUDGE_BIG;
        }
        if ui.input_mut(|i| i.consume_key(Modifiers::NONE, key)) {
            d += v * NUDGE;
        }
    }
    if d != Vec2::ZERO {
        nudge(app, d);
    }
}

/// The status text when a gesture meets View › Lock objects.
fn locked_text(l: Lang) -> &'static str {
    l.t(
        "Objects are locked: turn off View › Lock objects to move or resize them",
        "Les objets sont verrouillés : désactivez Affichage › Verrouiller les objets pour les déplacer ou les redimensionner",
    )
}

/// Moves the selection by `d` report pixels (kept on the page), one `visual.move`.
pub fn nudge(app: &mut App, d: Vec2) {
    if !editable(app) {
        if let Err(why) = ribbon::state(app, "visual.move") {
            app.status = why;
        }
        return;
    }
    if app.prefs.lock_objects {
        app.status = locked_text(app.lang).to_string();
        return;
    }
    let Some((page, size, items)) = page_items(app) else {
        return;
    };
    let sel = selection(app);
    let Some(b) = sel_bounds(&items, &sel) else { return };
    let mut d = d;
    if b.width() <= size.x {
        d.x = clampf(d.x, -b.min.x, size.x - b.max.x);
    }
    if b.height() <= size.y {
        d.y = clampf(d.y, -b.min.y, size.y - b.max.y);
    }
    if d == Vec2::ZERO {
        return;
    }
    let rows: Vec<Value> = items
        .iter()
        .filter(|i| sel.contains(&i.name))
        .map(|i| json!({"visual": i.name, "x": i.rect.min.x + d.x, "y": i.rect.min.y + d.y}))
        .collect();
    ribbon::invoke(app, "visual.move", json!({"page": page, "visuals": rows}));
}

/// Copies the selection (`visual.copy`); the answer goes to the clipboards.
pub fn copy(app: &mut App) {
    let l = app.lang;
    let sel = selection(app);
    let Some(page) = app.cmd.visual.as_ref().map(|v| v.page.clone()) else {
        app.status = l
            .t("Select a visual to copy.", "Sélectionnez un visuel à copier.")
            .into();
        return;
    };
    if let Err(why) = ribbon::state(app, "visual.copy") {
        app.status = why;
        return;
    }
    app.run(
        Call {
            id: "visual.copy",
            params: json!({"page": page, "visuals": sel}),
        },
        Then::Clipboard,
    );
}

/// Cuts the selection (`visual.cut`: copied and removed in one step, so one undo); the answer
/// goes to the clipboards as a copy's does.
pub fn cut(app: &mut App) {
    let l = app.lang;
    let sel = selection(app);
    let Some(page) = app.cmd.visual.as_ref().map(|v| v.page.clone()) else {
        app.status = l
            .t("Select a visual to cut.", "Sélectionnez un visuel à couper.")
            .into();
        return;
    };
    if let Err(why) = ribbon::state(app, "visual.cut") {
        app.status = why;
        return;
    }
    select(app, &page, &[]);
    app.run(
        Call {
            id: "visual.cut",
            params: json!({"page": page, "visuals": sel}),
        },
        Then::Clipboard,
    );
}

/// `visual.copy`'s answer: its clip on the system and in-app clipboards.
pub fn copied(app: &mut App, v: &Value) {
    let l = app.lang;
    let Some(clip) = v.get("clip").filter(|c| is_clip(c)) else {
        return;
    };
    match serde_json::to_string(clip) {
        Ok(text) if text.len() <= MAX_CLIP_TEXT => {
            let n = v.get("count").and_then(Value::as_u64).unwrap_or(0);
            app.canvas.clip = Some(clip.clone());
            app.canvas.os_copy = Some(text);
            app.status = match l {
                Lang::En => format!("Copied {n} visual(s)"),
                Lang::Fr => format!("{n} visuel(s) copié(s)"),
            };
        }
        _ => {
            app.status = l
                .t(
                    "Too much to copy at once.",
                    "Trop d'éléments à copier en une fois.",
                )
                .into();
        }
    }
}

/// Whether `v` looks like a clip (`visual.copy`'s document); the engine checks the rest.
pub fn is_clip(v: &Value) -> bool {
    v.get("bicraft").and_then(Value::as_str) == Some("visuals")
        && v.get("version").and_then(Value::as_u64) == Some(1)
        && v.get("visuals").is_some_and(Value::is_array)
}

/// Pastes clipboard text: a clip goes to the page shown, anything else is refused with a note
/// (`fallback`: the in-app clipboard is used instead).
pub fn paste_text(app: &mut App, text: &str, fallback: bool) {
    let l = app.lang;
    let clip = (text.len() <= MAX_CLIP_TEXT)
        .then(|| serde_json::from_str::<Value>(text).ok())
        .flatten()
        .filter(is_clip);
    match clip {
        Some(c) => {
            app.canvas.clip = Some(c.clone());
            paste(app, c);
        }
        None if fallback && app.canvas.clip.is_some() && text.trim().is_empty() => paste_in_app(app),
        None => {
            app.status = if text.len() > MAX_CLIP_TEXT {
                l.t(
                    "The clipboard holds too much to paste.",
                    "Le presse-papiers contient trop de données à coller.",
                )
            } else {
                l.t(
                    "The clipboard holds no BIcraft visuals.",
                    "Le presse-papiers ne contient pas de visuels BIcraft.",
                )
            }
            .into();
        }
    }
}

/// Pastes the in-app clipboard.
pub fn paste_in_app(app: &mut App) {
    match app.canvas.clip.clone() {
        Some(c) => paste(app, c),
        None => {
            app.status = app
                .lang
                .t(
                    "Nothing to paste: copy visuals first (Ctrl+C).",
                    "Rien à coller : copiez d'abord des visuels (Ctrl+C).",
                )
                .into();
        }
    }
}

/// Pastes `clip` on the page shown (`visual.paste`); the pasted visuals are selected.
pub fn paste(app: &mut App, clip: Value) {
    let Some(page) = app.page_names().get(app.active_page()).map(|p| p.0.clone()) else {
        return;
    };
    if let Err(why) = ribbon::state(app, "visual.paste") {
        app.status = why;
        return;
    }
    let mut params = json!({"page": page, "clip": clip});
    if let Some(at) = app.canvas.paste_at.take() {
        params["x"] = json!(at.x.round().max(0.0));
        params["y"] = json!(at.y.round().max(0.0));
    }
    app.run(
        Call {
            id: "visual.paste",
            params,
        },
        Then::SelectPasted,
    );
}

/// The Paste button and menu item: the system clipboard where the shell reads it, else the
/// in-app one.
pub fn paste_command(app: &mut App) {
    if let Err(why) = ribbon::state(app, "visual.paste") {
        app.status = why;
        return;
    }
    match &app.shell.read_clipboard {
        Some(read) => {
            let slot: ClipSlot = std::rc::Rc::new(std::cell::RefCell::new(None));
            read(slot.clone());
            app.canvas.reading = Some(slot);
            poll_read(app);
        }
        None => paste_in_app(app),
    }
}

/// Takes a finished clipboard read: its clip, else the in-app clipboard.
fn poll_read(app: &mut App) {
    let Some(slot) = &app.canvas.reading else { return };
    let got = slot.borrow_mut().take();
    let Some(r) = got else { return };
    app.canvas.reading = None;
    match r {
        Ok(text)
            if text.len() <= MAX_CLIP_TEXT
                && serde_json::from_str::<Value>(&text).is_ok_and(|v| is_clip(&v)) =>
        {
            paste_text(app, &text, false)
        }
        _ => paste_in_app(app),
    }
}

/// Duplicates the selection (`visual.duplicate`); the copies are selected.
pub fn duplicate(app: &mut App) {
    let l = app.lang;
    let sel = selection(app);
    let Some(page) = app.cmd.visual.as_ref().map(|v| v.page.clone()) else {
        app.status = l
            .t(
                "Select a visual to duplicate.",
                "Sélectionnez un visuel à dupliquer.",
            )
            .into();
        return;
    };
    if let Err(why) = ribbon::state(app, "visual.duplicate") {
        app.status = why;
        return;
    }
    app.run(
        Call {
            id: "visual.duplicate",
            params: json!({"page": page, "visuals": sel}),
        },
        Then::SelectPasted,
    );
}

/// `visual.paste` and `visual.duplicate`'s answer: the new visuals are the selection.
pub fn pasted(app: &mut App, v: &Value) {
    let Some(page) = v.get("page").and_then(Value::as_str) else {
        return;
    };
    let names: Vec<String> = v
        .get("selectable")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .take(MAX_VISUALS)
        .map(str::to_string)
        .collect();
    let n = names.len();
    select(app, page, &names);
    let warnings = v.get("warnings").and_then(Value::as_array).map_or(0, Vec::len);
    app.status = match (app.lang, warnings) {
        (Lang::En, 0) => format!("Pasted {n} visual(s)"),
        (Lang::Fr, 0) => format!("{n} visuel(s) collé(s)"),
        (Lang::En, w) => format!(
            "Pasted {n} visual(s), {w} warning(s): fields this model lacks are kept as broken references"
        ),
        (Lang::Fr, w) => format!(
            "{n} visuel(s) collé(s), {w} avertissement(s) : les champs absents du modèle sont gardés comme références rompues"
        ),
    };
}

// ---------------------------------------------------------------------------------------------
// The ribbon's Arrange and Page view menus and the View tab's Page options
// ---------------------------------------------------------------------------------------------

/// Commands whose ribbon button is a menu of their `to` values.
pub const MENU_BUTTONS: &[&str] = &["visual.align", "visual.order", "page.set_display_option"];

/// The items of a menu button: (`to`, English, French).
pub fn menu_items(id: &str) -> &'static [(&'static str, &'static str, &'static str)] {
    match id {
        "visual.align" => &[
            ("left", "Align left", "Aligner à gauche"),
            ("center", "Align center", "Centrer horizontalement"),
            ("right", "Align right", "Aligner à droite"),
            ("top", "Align top", "Aligner en haut"),
            ("middle", "Align middle", "Centrer verticalement"),
            ("bottom", "Align bottom", "Aligner en bas"),
            (
                "distribute_horizontal",
                "Distribute horizontally",
                "Distribuer horizontalement",
            ),
            (
                "distribute_vertical",
                "Distribute vertically",
                "Distribuer verticalement",
            ),
        ],
        "visual.order" => &[
            ("front", "Bring to front", "Mettre au premier plan"),
            ("forward", "Bring forward", "Avancer"),
            ("backward", "Send backward", "Reculer"),
            ("back", "Send to back", "Mettre en arrière-plan"),
        ],
        // View › Page view (Microsoft Learn, "Apply page size and settings in a Power BI report")
        "page.set_display_option" => &[
            ("FitToPage", "Fit to page", "Ajuster à la page"),
            ("FitToWidth", "Fit to width", "Ajuster à la largeur"),
            ("ActualSize", "Actual size", "Taille réelle"),
        ],
        _ => &[],
    }
}

/// Runs a menu button's item: the command with the selection and `to` (a page's display
/// option for Page view, which then shows the page by it again).
pub fn run_menu_item(app: &mut App, c: &'static crate::registry::Cmd, to: &str) {
    let mut p = ribbon::prefill(app, c, None);
    if let Some(o) = p.as_object_mut() {
        if c.id == "page.set_display_option" {
            o.insert("display_option".into(), json!(to));
        } else {
            o.insert("to".into(), json!(to));
            o.insert("visuals".into(), json!(selection(app)));
        }
    }
    if c.id == "page.set_display_option" {
        app.view_zoom = None;
    }
    ribbon::invoke(app, c.id, p);
}

/// The View tab's Page options (Microsoft Learn, "Apply page size and settings in a Power BI
/// report"): Gridlines, Snap to grid and Lock objects, independent of each other. Pure view state
/// of the author's window, kept in the preferences: the report's published schemas have no
/// property for them, and readers are not affected.
pub fn view_group(app: &mut App, ui: &mut Ui) {
    use crate::icons::Icon;
    let l = app.lang;
    crate::widgets::group(ui, |ui| {
        crate::widgets::rows(ui, |ui| {
            let p = &app.prefs;
            let items: [(Icon, &str, &str, bool); 3] = [
                (
                    Icon::Grid,
                    l.t("Gridlines", "Quadrillage"),
                    l.t(
                        "Show the canvas grid (8 px) to line visuals up",
                        "Afficher la grille du canevas (8 px) pour aligner les visuels",
                    ),
                    p.show_gridlines,
                ),
                (
                    Icon::Align,
                    l.t("Snap to grid", "Aimanter à la grille"),
                    l.t(
                        "Moved and resized visuals snap to the 8 px grid where no smart guide is near (Alt: no snapping)",
                        "Les visuels déplacés ou redimensionnés s'aimantent à la grille de 8 px quand aucun repère n'est proche (Alt : sans aimantation)",
                    ),
                    p.snap_grid,
                ),
                (
                    Icon::Lock,
                    l.t("Lock objects", "Verrouiller les objets"),
                    l.t(
                        "Visuals cannot be moved or resized on the canvas; selecting, formatting and the menus still work",
                        "Les visuels ne peuvent plus être déplacés ni redimensionnés sur le canevas ; la sélection, la mise en forme et les menus restent possibles",
                    ),
                    p.lock_objects,
                ),
            ];
            for (i, (icon, label, tip, on)) in items.into_iter().enumerate() {
                let r = crate::widgets::small_button(ui, icon, label, true, on).on_hover_text(tip);
                if r.clicked() {
                    let p = &mut app.prefs;
                    match i {
                        0 => p.show_gridlines = !on,
                        1 => p.snap_grid = !on,
                        _ => {
                            p.lock_objects = !on;
                            app.canvas.gesture = None;
                        }
                    }
                    app.save_prefs();
                }
            }
        });
    });
    // the report pages in high contrast: a view option of this window, never written to the file
    crate::widgets::group(ui, |ui| {
        let on = app.prefs.report_contrast;
        let r = crate::widgets::toggle_button(
            ui,
            crate::icons::Icon::Contrast,
            l.t("High-contrast colours", "Couleurs à contraste élevé"),
            true,
            on,
            true,
        )
        .on_hover_text(l.t(
            "Draw the report pages in high contrast (white text on black, bright series colours); the file does not change",
            "Afficher les pages du rapport en contraste élevé (texte blanc sur noir, séries en couleurs vives) ; le fichier ne change pas",
        ));
        if r.clicked() {
            app.prefs.report_contrast = !on;
            app.save_prefs();
        }
    });
}

/// `v` within [`lo`, `hi`] (`hi` wins when they cross; never panics, unlike `f32::clamp`).
fn clampf(v: f32, lo: f32, hi: f32) -> f32 {
    v.max(lo).min(hi)
}
