//! The Model tab: the live viewer's model diagram painted with egui. The table cards, their
//! column and measure rows, the relationship lines with their cardinality marks and
//! filter-direction arrows (inactive ones dashed), pan, zoom and fit, and the details panel on
//! the right when a table, column, measure or relationship is clicked; right-most, the Data /
//! Model pane ([`crate::model_explorer`]); under the diagram, the layout tabs (the file's
//! diagrams: `diagram.add`, `diagram.rename`, `diagram.remove`).
//!
//! Positions come from [`crate::diagram`] (the viewer's layout); colours and sizes from the
//! viewer's CSS (`crates/view/src/page.html`), light or dark with the window. Platform-neutral:
//! egui and the snapshot only (no threads, no files), so it works the same for a local file, a
//! server and a WebAssembly build. Drawing is bounded: only what is on screen is painted, text is
//! skipped when it would be too small to read.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{
    Align2, Color32, CornerRadius, CursorIcon, FontId, Painter, Pos2, Rect, RichText, Sense, Shape, Stroke,
    StrokeKind, Vec2, pos2, vec2,
};
use serde_json::Value;

use crate::app::{clip, issue_text};
use crate::diagram::{self, Diagram, Hit, Order};
use crate::i18n::Lang;
use crate::source::Loaded;
use crate::widgets;

/// Zoom range (1 = the viewer's size).
pub const MIN_ZOOM: f32 = 0.05;
pub const MAX_ZOOM: f32 = 4.0;
/// Text smaller than this many points is not drawn.
const MIN_TEXT: f32 = 3.0;
/// Rows of a list in the details panel.
const MAX_LIST: usize = 500;
/// Measures "Mentioned in" scans, and results it keeps.
const MAX_SCAN: usize = 50_000;
const MAX_MENTIONS: usize = 200;

/// What the details panel shows (by name, as the viewer: it survives a re-read).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sel {
    Table(String),
    Column {
        table: String,
        name: String,
    },
    Measure {
        table: String,
        name: String,
    },
    /// A relationship, by its index in the model's `relationships`.
    Rel(usize),
    /// An object of the Model explorer that the diagram does not draw (selected alone).
    Item(Item),
}

/// The Model explorer's objects that have a Properties pane but no card in the diagram.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    CalcItem { group: String, name: String },
    Hierarchy { table: String, name: String },
    Role(String),
    Culture(String),
    Expression(String),
    DataSource(String),
}

/// The diagram's colours, from the design tokens (docs/design.md §2: `bg` is `canvas`,
/// `surface` `card`, `surface2` `hover`, `muted` `text_dim`, `faint` `text_faint`, `accent` the
/// selection `ring`, `accent_ink` the accent-coloured text).
#[derive(Debug, Clone, Copy)]
struct Pal {
    bg: Color32,
    surface: Color32,
    surface2: Color32,
    text: Color32,
    muted: Color32,
    faint: Color32,
    border: Color32,
    accent: Color32,
    accent_soft: Color32,
    err: Color32,
    err_soft: Color32,
    shadow: Color32,
    accent_ink: Color32,
    ok: Color32,
    warn: Color32,
}

const fn rgb(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

impl Pal {
    fn of(dark: bool) -> Pal {
        let t = if dark {
            crate::theme::Tokens::dark()
        } else {
            crate::theme::Tokens::light()
        };
        Pal {
            bg: t.canvas,
            surface: t.card,
            surface2: t.hover,
            text: t.text,
            muted: t.text_dim,
            faint: t.text_faint,
            border: t.border,
            accent: t.ring,
            accent_soft: t.accent_soft,
            err: t.err,
            err_soft: if dark {
                t.err_soft
            } else {
                Color32::from_rgba_unmultiplied(207, 52, 52, 26)
            },
            shadow: t.shadow,
            accent_ink: t.contextual,
            ok: t.ok,
            warn: t.warn,
        }
    }
}

/// The viewer's type badge (`typeBadge`): the text and its colour.
pub fn type_badge(dt: &str) -> (String, Option<Color32>) {
    let s = dt.to_lowercase();
    let has = |w: &[&str]| w.iter().any(|x| s.contains(x));
    let (num, text, date, boolean) = (rgb(0x2f7dd1), rgb(0xa0569f), rgb(0xc06a00), rgb(0x1d8a4c));
    if has(&["int", "whole"]) {
        ("123".into(), Some(num))
    } else if has(&["double", "decimal", "currency", "number", "float"]) {
        ("1.2".into(), Some(num))
    } else if has(&["string", "text"]) {
        ("abc".into(), Some(text))
    } else if has(&["date", "time"]) {
        ("date".into(), Some(date))
    } else if has(&["bool"]) {
        ("t/f".into(), Some(boolean))
    } else if has(&["binary"]) {
        ("bin".into(), None)
    } else if s.is_empty() {
        ("?".into(), None)
    } else {
        (clip(&s, 5), None)
    }
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

fn yes(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool) == Some(true)
}

/// Diagram → screen.
#[derive(Debug, Clone, Copy)]
struct Xf {
    origin: Pos2,
    zoom: f32,
}

impl Xf {
    fn p(&self, p: Pos2) -> Pos2 {
        self.origin + p.to_vec2() * self.zoom
    }
    fn r(&self, r: Rect) -> Rect {
        Rect::from_min_max(self.p(r.min), self.p(r.max))
    }
    fn inv(&self, p: Pos2) -> Pos2 {
        ((p - self.origin) / self.zoom).to_pos2()
    }
    /// A length of `x` diagram pixels, at least `min` points.
    fn len(&self, x: f32, min: f32) -> f32 {
        (x * self.zoom).max(min)
    }
}

/// The Model tab's state.
#[derive(Debug)]
pub struct ModelView {
    pub zoom: f32,
    /// Where the diagram's origin is, from the canvas' top left corner.
    pub pan: Vec2,
    pub sel: Option<Sel>,
    /// Every object selected when Ctrl or Shift picked more than one (tables, columns, measures;
    /// `sel` is the last one picked). Empty, or stale (it does not hold `sel`): `sel` alone.
    pub multi: Vec<Sel>,
    /// Cards showing all their rows.
    pub expanded: HashSet<String>,
    order: Order,
    /// The layout and the snapshot and expansion it is for.
    diagram: Option<(u64, u64, Arc<Diagram>)>,
    generation: u64,
    placed: bool,
    fit_requested: bool,
    /// The canvas on screen (last frame).
    pub canvas: Rect,
    pub hover: Option<Hit>,
    mentions: Option<(u64, Sel, Mentions)>,
    /// The states of the right-click menu's commands (the app sets them each frame).
    pub menu_states: HashMap<&'static str, Result<(), String>>,
    /// The menu item clicked: the app runs its command on the selection.
    pub picked: Option<(&'static str, serde_json::Value)>,
    /// A command the diagram or the Properties pane asks for, with all its values: the app sends
    /// it as it is (a card dropped: `diagram.move`; a column dropped on another table's:
    /// `relationship.add`; Delete on a relationship; a property changed: its `*.set`).
    pub run: Option<(&'static str, Value)>,
    /// A command whose form the app opens with these values (a double click on a relationship:
    /// `relationship.set`).
    pub form: Option<(&'static str, Value)>,
    /// Whether the session can edit the model (the app sets it each frame): drags and the
    /// Properties pane's fields are off otherwise.
    pub editable: bool,
    /// The gesture in progress.
    drag: Option<Drag>,
    /// Cards dropped, shown at their new places until the next snapshot: (snapshot, [(table,
    /// place in the file's coordinates)]).
    moved: Option<(u64, Vec<(String, Pos2)>)>,
    /// The Properties pane's text field being typed in: (field, text).
    edit: Option<(String, String)>,
    /// The snapshot shown.
    seq: u64,
    /// The Data / Model pane (`model_explorer.rs`).
    pub explorer: crate::model_explorer::Explorer,
    /// The side panes drawn in this frame (the Data pane, the Properties pane): sections of
    /// their own for Ctrl+F6 (`crate::keyboard`).
    pub panes: Vec<egui::Rect>,
    /// The layout tab shown (`None`: the file's selected one, `model.diagram.name`).
    pub layout: Option<String>,
    /// The model as the layout tab shows it: (snapshot, layout, model).
    view: Option<(u64, String, Option<Arc<Value>>)>,
    /// The layout tab drawn (its name; `None`: the file has none), for the drops.
    pub shown_layout: Option<String>,
    /// The perspective the diagram is filtered to (pure view state; `None`: the whole model).
    pub perspective: Option<String>,
}

/// A layout tab of the snapshot's `model.diagram.layouts`.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutTab {
    pub name: String,
    /// The layout of all tables.
    pub all: bool,
}

/// The layout tabs of `model` (none when the file has no layout).
pub fn layout_tabs(model: &Value) -> Vec<LayoutTab> {
    model
        .pointer("/diagram/layouts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(100)
        .filter_map(|d| {
            Some(LayoutTab {
                name: d.get("name")?.as_str()?.to_string(),
                all: d.get("all").and_then(Value::as_bool) == Some(true),
            })
        })
        .collect()
}

/// A layout name not taken yet: `Layout 1`, `Layout 2`… (in the UI's language).
pub fn new_layout_name(tabs: &[LayoutTab], l: Lang) -> String {
    let base = l.t("Layout", "Disposition");
    (1..=1000)
        .map(|k| format!("{base} {k}"))
        .find(|n| !tabs.iter().any(|t| t.name.eq_ignore_ascii_case(n)))
        .unwrap_or_else(|| base.to_string())
}

/// `model` as layout tab `name` shows it: its nodes are the diagram's (`diagram.nodes`); a layout
/// other than the one of all tables draws only its own tables. `None`: the model as it is.
fn layout_model(model: &Value, name: &str) -> Option<Value> {
    let layouts = model.pointer("/diagram/layouts")?.as_array()?;
    let d = layouts
        .iter()
        .find(|d| d.get("name").and_then(Value::as_str) == Some(name))?;
    let selected = model.pointer("/diagram/name").and_then(Value::as_str) == Some(name);
    let all = d.get("all").and_then(Value::as_bool) == Some(true);
    if selected && all {
        return None;
    }
    let nodes = d.get("nodes").cloned().unwrap_or(Value::Array(Vec::new()));
    let mut m = model.clone();
    if let Some(diagram) = m.get_mut("diagram").and_then(Value::as_object_mut) {
        diagram.insert("nodes".into(), nodes.clone());
        diagram.insert("name".into(), Value::String(name.to_string()));
    }
    if !all {
        let keep: HashSet<&str> = nodes
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|n| n.get("table").and_then(Value::as_str))
            .collect();
        let keep: HashSet<String> = keep.into_iter().map(str::to_string).collect();
        if let Some(tables) = m.get_mut("tables").and_then(Value::as_array_mut) {
            tables.retain(|t| {
                t.get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|n| keep.contains(n))
            });
        }
    }
    Some(m)
}

/// A drag on the diagram.
#[derive(Debug, Clone)]
enum Drag {
    /// The background: pans.
    Pan,
    /// A table card by its header: `at` is its top-left corner in the file's coordinates, `grab`
    /// where the pointer holds it, `shift` the diagram's when it started (kept during the drag).
    Card {
        name: String,
        grab: Vec2,
        /// Where it was when the drag started (a drop there moves nothing).
        start: Pos2,
        at: Pos2,
        shift: Vec2,
        /// The other cards the file has no place for, where they are (saved with the drop, so
        /// the diagram does not rearrange).
        rest: Vec<(String, Pos2)>,
    },
    /// A column row, towards another table's column: from its row's middle to the pointer.
    Column {
        table: String,
        column: String,
        from: Pos2,
        at: Pos2,
    },
}

/// Measures that mention the selection: (table, measure).
type Mentions = Arc<Vec<(String, String)>>;

impl Default for ModelView {
    fn default() -> ModelView {
        ModelView {
            zoom: 1.0,
            pan: Vec2::ZERO,
            sel: None,
            multi: Vec::new(),
            expanded: HashSet::new(),
            order: Order::default(),
            diagram: None,
            generation: 0,
            placed: false,
            fit_requested: false,
            canvas: Rect::NOTHING,
            hover: None,
            mentions: None,
            menu_states: HashMap::new(),
            picked: None,
            run: None,
            form: None,
            editable: false,
            drag: None,
            moved: None,
            edit: None,
            seq: 0,
            explorer: crate::model_explorer::Explorer::default(),
            panes: Vec::new(),
            layout: None,
            view: None,
            shown_layout: None,
            perspective: None,
        }
    }
}

impl ModelView {
    /// Every selected object, in the order picked (`sel` alone without a multi-selection).
    pub fn selection(&self) -> Vec<Sel> {
        match &self.sel {
            Some(s) if self.multi.contains(s) => self.multi.clone(),
            Some(s) => vec![s.clone()],
            None => Vec::new(),
        }
    }

    /// Whether `x` is selected.
    pub fn is_selected(&self, x: &Sel) -> bool {
        match &self.sel {
            Some(s) if s == x => true,
            Some(s) => self.multi.contains(s) && self.multi.contains(x),
            None => false,
        }
    }

    /// A plain click: `x` alone.
    pub fn select(&mut self, x: Option<Sel>) {
        self.sel = x;
        self.multi.clear();
    }

    /// Ctrl+click (or Shift+click on the diagram): `x` joins the selection, or leaves it when it
    /// is in it. A relationship is selected alone (it has no properties in common with others).
    pub fn toggle_selected(&mut self, x: Sel) {
        let alone = |x: &Sel| matches!(x, Sel::Rel(_) | Sel::Item(_));
        if alone(&x) || self.sel.as_ref().is_some_and(alone) {
            self.select(Some(x));
            return;
        }
        let mut all = self.selection();
        if let Some(i) = all.iter().position(|s| *s == x) {
            all.remove(i);
        } else {
            all.push(x);
        }
        self.sel = all.last().cloned();
        self.multi = if all.len() > 1 { all } else { Vec::new() };
    }

    /// Shift+click in the Data pane: the rows from the last one picked to `x`, in the pane's order
    /// (`order`); `x` alone when the last one is not in it.
    pub fn select_range(&mut self, order: &[Sel], x: Sel) {
        let from = self.sel.as_ref().and_then(|s| order.iter().position(|o| o == s));
        let to = order.iter().position(|o| *o == x);
        match (from, to) {
            (Some(a), Some(b)) => {
                let (lo, hi) = (a.min(b), a.max(b));
                let mut all: Vec<Sel> = order
                    .get(lo..=hi)
                    .unwrap_or_default()
                    .iter()
                    .filter(|s| !matches!(s, Sel::Rel(_) | Sel::Item(_)))
                    .cloned()
                    .collect();
                // the last one picked is `sel`
                if let Some(i) = all.iter().position(|s| *s == x) {
                    let last = all.remove(i);
                    all.push(last);
                }
                self.sel = all.last().cloned();
                self.multi = if all.len() > 1 { all } else { Vec::new() };
            }
            _ => self.select(Some(x)),
        }
    }

    /// The layout shown (tests).
    pub fn diagram(&self) -> Option<&Diagram> {
        self.diagram.as_ref().map(|d| d.2.as_ref())
    }

    /// Where diagram point `p` is on the screen.
    pub fn screen_pos(&self, p: Pos2) -> Pos2 {
        self.xf().p(p)
    }

    fn xf(&self) -> Xf {
        Xf {
            origin: self.canvas.min + self.pan,
            zoom: self.zoom,
        }
    }

    /// Fits the diagram in the canvas on the next frame.
    pub fn fit(&mut self) {
        self.fit_requested = true;
    }

    /// Zooms by `factor` about screen point `at`.
    pub fn zoom_about(&mut self, at: Pos2, factor: f32) {
        if !factor.is_finite() || factor <= 0.0 {
            return;
        }
        let nz = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let k = nz / self.zoom;
        let rel = at - self.canvas.min;
        self.pan = rel - (rel - self.pan) * k;
        self.zoom = nz;
    }

    /// The cards placed by the gesture in progress or a drop not yet in the snapshot (in the
    /// file's coordinates), and the diagram's shift to keep while dragging.
    fn overrides(&self) -> (HashMap<String, Pos2>, Option<Vec2>) {
        let mut out = HashMap::new();
        let mut shift = None;
        if let Some((_, list)) = &self.moved {
            out.extend(list.iter().cloned());
            shift = self.diagram().map(|d| d.shift);
        }
        if let Some(Drag::Card {
            name,
            at,
            shift: s,
            rest,
            ..
        }) = &self.drag
        {
            out.extend(rest.iter().cloned());
            out.insert(name.clone(), *at);
            shift = Some(*s);
        }
        (out, shift)
    }

    /// The drag a press at diagram point `p` starts: a card by its header, a column row; `None`
    /// (a pan) elsewhere or when the model cannot be edited.
    fn drag_start(&self, model: &Value, d: &Diagram, p: Pos2, tol: f32) -> Option<Drag> {
        if !self.editable {
            return None;
        }
        match d.hit(p, tol)? {
            Hit::Table(k) => {
                let c = d.cards.get(k)?;
                let saved = diagram::saved_positions(model);
                let rest = d
                    .cards
                    .iter()
                    .filter(|o| o.name != c.name && !saved.contains_key(&o.name))
                    .map(|o| (o.name.clone(), o.rect.min - d.shift))
                    .collect();
                Some(Drag::Card {
                    name: c.name.clone(),
                    grab: p - c.rect.min,
                    start: c.rect.min - d.shift,
                    at: c.rect.min - d.shift,
                    shift: d.shift,
                    rest,
                })
            }
            Hit::Column { card, index } => {
                let Some(Sel::Column { table, name }) = row_sel(model, d, card, index, "columns") else {
                    return None;
                };
                let from = d
                    .cards
                    .get(card)?
                    .columns
                    .iter()
                    .find(|r| r.index == index)?
                    .rect
                    .center();
                Some(Drag::Column {
                    table,
                    column: name,
                    from,
                    at: p,
                })
            }
            _ => None,
        }
    }

    /// Forgets the cards dropped but not saved (their command failed): they go back.
    pub fn drop_moved(&mut self) {
        self.moved = None;
    }

    /// Whether a card or a column is being dragged (tests).
    pub fn dragging(&self) -> bool {
        matches!(self.drag, Some(Drag::Card { .. } | Drag::Column { .. }))
    }

    fn toggle_expanded(&mut self, name: &str) {
        if !self.expanded.remove(name) {
            self.expanded.insert(name.to_string());
        }
        self.generation = self.generation.wrapping_add(1);
    }

    /// The tab: the bar, the details panel and the diagram.
    pub fn ui(&mut self, ui: &mut egui::Ui, loaded: Option<&Loaded>, l: Lang) {
        self.panes.clear();
        let Some(loaded) = loaded else {
            ui.label(l.t("Nothing open.", "Rien d'ouvert."));
            return;
        };
        let snap = loaded.snap.clone();
        let Some(model) = snap.get("model").filter(|m| m.is_object()) else {
            ui.label(l.t("This file has no model.", "Ce fichier n'a pas de modèle."));
            if let Some(note) = thin_note(&snap, l) {
                ui.label(note);
            }
            return;
        };
        if !self.zoom.is_finite() || self.zoom <= 0.0 {
            self.zoom = 1.0;
        }
        if !self.pan.x.is_finite() || !self.pan.y.is_finite() {
            self.pan = Vec2::ZERO;
        }
        // a dropped card waits for the snapshot that has its new place
        if self.moved.as_ref().is_some_and(|m| m.0 != loaded.seq) {
            self.moved = None;
        }
        self.seq = loaded.seq;
        // the layout tab shown, and the model as it shows it
        let tabs = layout_tabs(model);
        let shown = self
            .layout
            .clone()
            .filter(|n| tabs.iter().any(|t| t.name == *n))
            .or_else(|| {
                model
                    .pointer("/diagram/name")
                    .and_then(Value::as_str)
                    .filter(|n| tabs.iter().any(|t| t.name == *n))
                    .map(str::to_string)
            });
        // the layout of all tables is one view whether or not the file has a layout yet
        // a perspective that is gone shows the whole model again
        if self
            .perspective
            .as_ref()
            .is_some_and(|p| crate::perspective_dialog::members_of(model, p).is_none())
        {
            self.perspective = None;
        }
        let layout_key = shown
            .clone()
            .filter(|n| !tabs.iter().any(|t| t.name == *n && t.all))
            .unwrap_or_default();
        let key = match &self.perspective {
            Some(p) => format!("{layout_key}\u{1}{p}"),
            None => layout_key.clone(),
        };
        let cached = matches!(&self.view, Some((seq, k, _)) if *seq == loaded.seq && *k == key);
        if !cached {
            if self.view.as_ref().is_some_and(|v| v.1 != key) {
                // another tab: laid out again and fitted
                self.generation = self.generation.wrapping_add(1);
                self.moved = None;
                self.placed = false;
            }
            let laid = shown.as_deref().and_then(|n| layout_model(model, n));
            let vm = match &self.perspective {
                Some(p) => crate::perspective_dialog::filter_model(laid.as_ref().unwrap_or(model), p),
                None => laid,
            }
            .map(Arc::new);
            self.view = Some((loaded.seq, key, vm));
        }
        let vm = self.view.as_ref().and_then(|v| v.2.clone());
        let full = model;
        let model: &Value = vm.as_deref().unwrap_or(full);
        let (overrides, shift) = self.overrides();
        let fresh = overrides.is_empty()
            && matches!(&self.diagram, Some((seq, g, _)) if *seq == loaded.seq && *g == self.generation);
        if !fresh {
            let d = diagram::layout_at(model, &mut self.order, &self.expanded, &overrides, shift);
            // the cards stay where they are on the screen when the diagram's shift changes
            if let Some(old) = self.diagram().map(|d| d.shift)
                && self.placed
            {
                self.pan -= (d.shift - old) * self.zoom;
            }
            // a card moved: the next frame lays the diagram out again from the snapshot
            let generation = if overrides.is_empty() {
                self.generation
            } else {
                self.generation.wrapping_add(1)
            };
            self.diagram = Some((loaded.seq, generation, Arc::new(d)));
        }
        let Some(d) = self.diagram.as_ref().map(|d| d.2.clone()) else {
            return;
        };
        let pal = Pal::of(ui.visuals().dark_mode);
        let all_shown = shown
            .as_ref()
            .is_none_or(|n| tabs.iter().any(|t| t.name == *n && t.all));
        egui::Panel::top("bicraft-model-bar")
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(10, 6)))
            .show(ui, |ui| self.bar(ui, full, &d, l, pal));
        // the Data / Model pane, right-most as in Desktop
        let in_layout: HashSet<String> = d.cards.iter().map(|c| c.name.clone()).collect();
        let add_at = {
            let right = d
                .cards
                .iter()
                .map(|c| c.rect.max.x - d.shift.x)
                .fold(f32::NEG_INFINITY, f32::max);
            let top = d
                .cards
                .iter()
                .map(|c| c.rect.min.y - d.shift.y)
                .fold(f32::INFINITY, f32::min);
            let r = |x: f32| (f64::from(x) / 10.0).round() * 10.0;
            if right.is_finite() && top.is_finite() {
                (r(right + diagram::COLUMN_GAP), r(top))
            } else {
                (60.0, 60.0)
            }
        };
        let mut out = crate::model_explorer::Out::default();
        let explorer = egui::Panel::right("bicraft-model-explorer")
            .resizable(true)
            .frame(
                egui::Frame::NONE
                    .fill(pal.surface)
                    .stroke(egui::Stroke::new(1.0, pal.border))
                    .inner_margin(egui::Margin::same(8)),
            )
            .default_size(260.0)
            .min_size(180.0)
            .show(ui, |ui| {
                let picked = self.selection();
                let cx = crate::model_explorer::Cx {
                    model: full,
                    sel: self.sel.as_ref(),
                    picked: &picked,
                    states: &self.menu_states,
                    editable: self.editable,
                    l,
                    layout: shown
                        .as_deref()
                        .filter(|_| !all_shown)
                        .map(|n| (n, add_at, &in_layout)),
                };
                self.explorer.ui(ui, &cx, &mut out);
            });
        self.panes.push(explorer.response.rect);
        if let Some(sel) = out.sel {
            self.select(sel);
        }
        if let Some(x) = out.toggle {
            self.toggle_selected(x);
        }
        if let Some(x) = out.range {
            let order = self.explorer.order.clone();
            self.select_range(&order, x);
        }
        if out.picked.is_some() {
            self.picked = out.picked;
        }
        if out.form.is_some() {
            self.form = out.form;
        }
        if out.run.is_some() {
            self.run = out.run;
        }
        let props = egui::Panel::right("bicraft-model-panel")
            .resizable(true)
            .frame(
                egui::Frame::NONE
                    .fill(pal.surface)
                    .stroke(egui::Stroke::new(1.0, pal.border))
                    .inner_margin(egui::Margin::same(10)),
            )
            .default_size(360.0)
            .min_size(220.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.panel(ui, full, &snap, loaded.seq, l, pal));
            });
        self.panes.push(props.response.rect);
        // the layout tabs under the diagram
        egui::Panel::bottom("bicraft-model-layouts")
            .frame(
                egui::Frame::NONE
                    .fill(pal.surface)
                    .stroke(egui::Stroke::new(1.0, pal.border))
                    .inner_margin(egui::Margin::symmetric(8, 2)),
            )
            .show(ui, |ui| self.layout_tabs_ui(ui, &tabs, shown.as_deref(), l));
        self.shown_layout = shown.clone();
        if d.cards.is_empty() {
            let text = if all_shown {
                l.t("The model has no tables yet", "Le modèle n'a pas encore de table")
            } else {
                l.t(
                    "This layout has no tables yet: right-click a table in the Data pane, Add to layout",
                    "Cette disposition n'a pas encore de table : clic droit sur une table du volet Données, Ajouter à la disposition",
                )
            };
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new(text).color(pal.muted));
            });
            return;
        }
        self.canvas_ui(ui, model, &d, l, pal);
    }

    /// The layout tabs: a click shows one, a double click renames it, its menu renames or
    /// deletes it; + adds one. A file without a layout shows the layout of all tables alone.
    fn layout_tabs_ui(&mut self, ui: &mut egui::Ui, tabs: &[LayoutTab], shown: Option<&str>, l: Lang) {
        ui.horizontal(|ui| {
            if tabs.is_empty() {
                widgets::tab(ui, l.t("All tables", "Toutes les tables"), true, false);
            }
            for t in tabs {
                let active = shown == Some(t.name.as_str());
                let r = widgets::tab(ui, &clip(&t.name, 40), active, false);
                if r.clicked() {
                    self.layout = Some(t.name.clone());
                }
                if r.double_clicked() && self.editable {
                    self.form = Some(("diagram.rename", serde_json::json!({ "name": t.name })));
                }
                crate::keyboard::context_menu(&r, |ui| {
                    ui.set_min_width(180.0);
                    for (id, via_form) in [("diagram.rename", true), ("diagram.remove", false)] {
                        let Some(c) = crate::registry::find(id) else {
                            continue;
                        };
                        let st = if id == "diagram.remove" && t.all {
                            Err(l
                                .t(
                                    "the layout of all tables cannot be deleted",
                                    "la disposition de toutes les tables ne peut pas être supprimée",
                                )
                                .to_string())
                        } else {
                            self.menu_states
                                .get(id)
                                .cloned()
                                .unwrap_or_else(|| Err("not available".into()))
                        };
                        if crate::menus::item(ui, c.text(l), &st, l) {
                            let p = serde_json::json!({ "name": t.name });
                            if via_form {
                                self.form = Some((c.id, p));
                            } else {
                                self.picked = Some((c.id, p));
                            }
                            ui.close();
                        }
                    }
                });
            }
            let add = crate::registry::find("diagram.add");
            let tip = add.map_or("", |c| c.text(l));
            let r = ui
                .add_enabled(self.editable, egui::Button::new("+").small())
                .on_hover_text(tip);
            r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, self.editable, tip));
            if r.clicked() {
                self.run = Some((
                    "diagram.add",
                    serde_json::json!({ "name": new_layout_name(tabs, l) }),
                ));
                // the new layout becomes the selected one
                self.layout = None;
            }
        });
    }

    // --- the bar ----------------------------------------------------------------------------

    fn bar(&mut self, ui: &mut egui::Ui, model: &Value, d: &Diagram, l: Lang, pal: Pal) {
        let tables: Vec<&Value> = diagram::objects(model, "tables").map(|x| x.1).collect();
        let count = |k: &str| -> usize { tables.iter().map(|t| diagram::objects(t, k).count()).sum() };
        let (n_cols, n_meas) = (count("columns"), count("measures"));
        let n_bad: usize = tables
            .iter()
            .map(|t| {
                diagram::objects(t, "measures")
                    .filter(|(_, m)| m.get("valid").and_then(Value::as_bool) == Some(false))
                    .count()
            })
            .sum();
        let n_rels = diagram::objects(model, "relationships").count();
        let truncated = yes(model, "truncated") || tables.iter().any(|t| yes(t, "truncated"));
        ui.horizontal_wrapped(|ui| {
            let name = s(model, "name");
            ui.label(
                RichText::new(clip(
                    if name.is_empty() {
                        l.t("Model", "Modèle")
                    } else {
                        name
                    },
                    80,
                ))
                .font(crate::fonts::semibold_px(14.0)),
            );
            if !s(model, "culture").is_empty() {
                pill(ui, s(model, "culture"), pal.muted, pal.surface2);
            }
            ui.label(
                RichText::new(
                    [
                        plural(l, tables.len(), ("table", "tables"), ("table", "tables")),
                        plural(l, n_cols, ("column", "columns"), ("colonne", "colonnes")),
                        plural(l, n_meas, ("measure", "measures"), ("mesure", "mesures")),
                        plural(
                            l,
                            n_rels,
                            ("relationship", "relationships"),
                            ("relation", "relations"),
                        ),
                    ]
                    .join(" · "),
                )
                .color(pal.muted),
            );
            if n_bad > 0 {
                pill(
                    ui,
                    &plural(
                        l,
                        n_bad,
                        ("invalid measure", "invalid measures"),
                        ("mesure non valide", "mesures non valides"),
                    ),
                    pal.err,
                    pal.err_soft,
                );
            }
            if d.total_tables > diagram::MAX_TABLES {
                ui.label(
                    RichText::new(format!(
                        "{} {} {}",
                        l.t("drawing the first", "seules les"),
                        diagram::MAX_TABLES,
                        l.t("tables", "premières tables sont dessinées")
                    ))
                    .italics()
                    .color(pal.muted),
                );
            }
            if truncated {
                pill(ui, l.t("truncated", "tronqué"), pal.warn, pal.surface2);
            }
            // the perspective the diagram shows (view state, as Desktop's diagram filter)
            let persps: Vec<&str> = model
                .get("perspectives")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .take(500)
                .collect();
            if !persps.is_empty() {
                ui.separator();
                let label = ui.label(RichText::new(l.t("Perspective", "Perspective")).color(pal.muted));
                let all = l.t("(All)", "(Toutes)");
                let current = self.perspective.clone().unwrap_or_else(|| all.to_string());
                egui::ComboBox::from_id_salt("bicraft-model-perspective")
                    .selected_text(clip(&current, 40))
                    .width(160.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.perspective, None, all);
                        for p in &persps {
                            ui.selectable_value(&mut self.perspective, Some((*p).to_string()), *p);
                        }
                    })
                    .response
                    .labelled_by(label.id);
            }
            ui.separator();
            // the legend, as the viewer's
            let small = |t: &str| RichText::new(t).small().color(pal.muted);
            ui.label(RichText::new("* — 1").monospace().small().color(pal.muted));
            ui.label(small(l.t("cardinality", "cardinalité")));
            ui.label(small(l.t("▶ filter direction", "▶ sens du filtre")));
            ui.label(small(l.t("- - - inactive", "- - - inactive")));
            ui.label(RichText::new("ƒx").italics().small().color(pal.accent_ink));
            ui.label(small(l.t("calculated", "calculé")));
            ui.separator();
            let center = self.canvas.center();
            if ui
                .small_button("−")
                .on_hover_text(l.t("Zoom out (Ctrl+wheel)", "Zoom arrière (Ctrl+molette)"))
                .clicked()
            {
                self.zoom_about(center, 1.0 / 1.25);
            }
            ui.label(RichText::new(format!("{:.0} %", self.zoom * 100.0)).small());
            if ui
                .small_button("+")
                .on_hover_text(l.t("Zoom in (Ctrl+wheel)", "Zoom avant (Ctrl+molette)"))
                .clicked()
            {
                self.zoom_about(center, 1.25);
            }
            if ui.small_button("100 %").clicked() {
                self.zoom = 1.0;
                self.pan = Vec2::ZERO;
            }
            if ui
                .small_button(l.t("Fit", "Ajuster"))
                .on_hover_text(l.t(
                    "The whole diagram in the window",
                    "Tout le diagramme dans la fenêtre",
                ))
                .clicked()
            {
                self.fit();
            }
        });
    }

    // --- the diagram ------------------------------------------------------------------------

    fn place(&mut self, d: &Diagram, max: f32) {
        let r = self.canvas;
        if d.size.x <= 0.0 || d.size.y <= 0.0 || r.width() <= 0.0 || r.height() <= 0.0 {
            self.zoom = 1.0;
            self.pan = Vec2::ZERO;
            return;
        }
        let z = ((r.width() - 24.0).max(10.0) / d.size.x)
            .min((r.height() - 24.0).max(10.0) / d.size.y)
            .clamp(MIN_ZOOM, max);
        self.zoom = z;
        // centred across, at the top as the viewer's
        let free = (r.size() - d.size * z).max(Vec2::ZERO);
        self.pan = vec2(free.x / 2.0, free.y.min(12.0));
    }

    fn canvas_ui(&mut self, ui: &mut egui::Ui, model: &Value, d: &Diagram, l: Lang, pal: Pal) {
        let size = ui.available_size().max(vec2(50.0, 50.0));
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Other,
                true,
                l.t("Model diagram", "Diagramme du modèle"),
            )
        });
        self.canvas = rect;
        if !self.placed {
            // the viewer's size when it fits, else the whole diagram
            self.placed = true;
            if d.size.x + 24.0 <= rect.width() && d.size.y + 24.0 <= rect.height() {
                self.zoom = 1.0;
                self.pan = Vec2::ZERO;
            } else {
                self.place(d, 1.0);
            }
        }
        if std::mem::take(&mut self.fit_requested) {
            self.place(d, 2.0);
        }
        // a drag by a card's header moves the card, from a column row relates it, else pans
        if resp.drag_started() {
            let xf = self.xf();
            let tol = 6.0 / self.zoom;
            let origin = ui
                .input(|i| i.pointer.press_origin())
                .or_else(|| resp.interact_pointer_pos());
            self.drag = Some(
                origin
                    .and_then(|p| self.drag_start(model, d, xf.inv(p), tol))
                    .unwrap_or(Drag::Pan),
            );
        }
        // pan (drag, wheel) and zoom (Ctrl+wheel, pinch)
        if resp.dragged() {
            let pointer = resp.interact_pointer_pos().map(|p| self.xf().inv(p));
            match (&mut self.drag, pointer) {
                (Some(Drag::Card { grab, at, shift, .. }), Some(p)) => *at = p - *grab - *shift,
                (Some(Drag::Column { at, .. }), Some(p)) => *at = p,
                (Some(Drag::Card { .. } | Drag::Column { .. }), None) => {}
                _ => self.pan += resp.drag_delta(),
            }
        }
        if resp.drag_stopped() {
            let tol = 6.0 / self.zoom;
            let drop = resp.interact_pointer_pos().map(|p| self.xf().inv(p));
            match self.drag.take() {
                Some(Drag::Card {
                    name,
                    at,
                    grab,
                    start,
                    shift,
                    rest,
                }) => {
                    let at = drop.map_or(at, |p| p - grab - shift);
                    if (start - at).length() > 0.5 {
                        // one step: the card, and the cards the file had no place for yet
                        let r = |x: f32| (f64::from(x) * 10.0).round() / 10.0;
                        let mut list = rest;
                        list.push((name, at));
                        let tables: Vec<Value> = list
                            .iter()
                            .map(|(t, p)| serde_json::json!({"table": t, "x": r(p.x), "y": r(p.y)}))
                            .collect();
                        let mut p = serde_json::json!({ "tables": tables });
                        if let (Some(o), Some(n)) = (p.as_object_mut(), &self.shown_layout) {
                            o.insert("diagram".into(), Value::String(n.clone()));
                        }
                        self.run = Some(("diagram.move", p));
                        self.moved = Some((self.seq, list));
                    }
                }
                Some(Drag::Column { table, column, .. }) => {
                    if let Some(Hit::Column { card, index }) = drop.and_then(|p| d.hit(p, tol))
                        && let Some(Sel::Column {
                            table: to_table,
                            name: to_column,
                        }) = row_sel(model, d, card, index, "columns")
                        && to_table != table
                    {
                        self.run = Some((
                            "relationship.add",
                            serde_json::json!({
                                "from": format!("{table}[{column}]"),
                                "to": format!("{to_table}[{to_column}]"),
                            }),
                        ));
                    }
                }
                _ => {}
            }
        }
        // Delete removes the selected relationship
        if self.editable
            && !ui.ctx().egui_wants_keyboard_input()
            && let Some(Sel::Rel(i)) = self.sel
            && let Some(r) = rel_at(model, i)
            && ui.input_mut(|inp| inp.consume_key(egui::Modifiers::NONE, egui::Key::Delete))
        {
            self.run = Some(("relationship.remove", rel_ref(r)));
            self.select(None);
        }
        if resp.contains_pointer() {
            let (zd, scroll, at) =
                ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta(), i.pointer.hover_pos()));
            if (zd - 1.0).abs() > f32::EPSILON {
                self.zoom_about(at.unwrap_or(rect.center()), zd);
            } else if scroll != Vec2::ZERO {
                self.pan += scroll;
            }
        }
        // keep part of the diagram in the window
        let (w, h) = (d.size.x * self.zoom, d.size.y * self.zoom);
        self.pan.x = self
            .pan
            .x
            .clamp(-(w - 60.0).max(0.0), (rect.width() - 60.0).max(0.0));
        self.pan.y = self
            .pan
            .y
            .clamp(-(h - 60.0).max(0.0), (rect.height() - 60.0).max(0.0));
        let xf = self.xf();
        let tol = 6.0 / self.zoom;
        self.hover = resp.hover_pos().and_then(|p| d.hit(xf.inv(p), tol));
        // a right click selects what it is on (nothing: the model itself) for the menu
        if resp.secondary_clicked()
            && let Some(p) = resp.interact_pointer_pos()
        {
            let hit = match d.hit(xf.inv(p), tol) {
                Some(Hit::Table(k) | Hit::Card(k) | Hit::More(k)) => {
                    d.cards.get(k).map(|c| Sel::Table(c.name.clone()))
                }
                Some(Hit::Column { card, index }) => row_sel(model, d, card, index, "columns"),
                Some(Hit::Measure { card, index }) => row_sel(model, d, card, index, "measures"),
                Some(Hit::Rel(k)) => d.links.get(k).map(|link| Sel::Rel(link.rel)),
                None => None,
            };
            // on an object already selected, the selection stays
            if !hit.as_ref().is_some_and(|x| self.is_selected(x)) {
                self.select(hit);
            }
        }
        crate::keyboard::context_menu(&resp, |ui| {
            if let Some(p) = crate::menus::model_menu(ui, l, self.sel.as_ref(), &self.menu_states, model) {
                self.picked = Some(p);
            }
        });
        // a double click's first click selected the object (the formula bar may have moved the
        // diagram under the second one)
        let sel_before = self.sel.clone();
        if resp.clicked()
            && let Some(p) = resp.interact_pointer_pos()
        {
            // Ctrl or Shift adds the object to the selection (or takes it out), as in Desktop
            let add = ui.input(|i| i.modifiers.command || i.modifiers.shift);
            let picked = match d.hit(xf.inv(p), tol) {
                Some(Hit::Table(k) | Hit::Card(k)) => d.cards.get(k).map(|c| Sel::Table(c.name.clone())),
                Some(Hit::Column { card, index }) => row_sel(model, d, card, index, "columns"),
                Some(Hit::Measure { card, index }) => row_sel(model, d, card, index, "measures"),
                Some(Hit::More(k)) => {
                    if let Some(name) = d.cards.get(k).map(|c| c.name.clone()) {
                        self.toggle_expanded(&name);
                    }
                    None
                }
                Some(Hit::Rel(k)) => d.links.get(k).map(|link| Sel::Rel(link.rel)),
                None => None,
            };
            if let Some(x) = picked {
                if add {
                    self.toggle_selected(x);
                } else {
                    self.select(Some(x));
                }
            }
        }
        // a double click edits a measure, a calculated column or a calculated table in the
        // formula bar
        if resp.double_clicked() {
            use crate::formula::{Kind, pick};
            self.sel = sel_before;
            match &self.sel {
                Some(Sel::Measure { table, name }) => pick(ui.ctx(), Kind::Measure, Some(table), name, true),
                Some(Sel::Column { table, name }) => pick(ui.ctx(), Kind::Column, Some(table), name, true),
                Some(Sel::Table(t)) => pick(ui.ctx(), Kind::Table, Some(t), t, true),
                // a relationship: its editor (relationship.set's form, with its values)
                Some(Sel::Rel(i)) => {
                    if let Some(r) = rel_at(model, *i).filter(|_| self.editable) {
                        self.form = Some(("relationship.set", rel_values(r)));
                    }
                }
                Some(Sel::Item(_)) | None => {}
            }
        }
        if matches!(self.drag, Some(Drag::Card { .. })) {
            ui.ctx().set_cursor_icon(CursorIcon::Move);
        } else if matches!(self.drag, Some(Drag::Column { .. })) {
            ui.ctx().set_cursor_icon(CursorIcon::Alias);
        } else if resp.dragged() {
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        } else if matches!(self.hover, Some(h) if !matches!(h, Hit::Card(_))) {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, pal.bg);
        let visible = Rect::from_min_max(xf.inv(rect.min), xf.inv(rect.max));
        let rels: Vec<&Value> = model
            .get("relationships")
            .and_then(Value::as_array)
            .map(|a| a.iter().collect())
            .unwrap_or_default();
        for (k, link) in d.links.iter().enumerate() {
            if !link.bounds.expand(20.0).intersects(visible) {
                continue;
            }
            let on = self.hover == Some(Hit::Rel(k)) || self.sel == Some(Sel::Rel(link.rel));
            self.draw_link(&painter, xf, link, on, pal);
        }
        let tables = model.get("tables").and_then(Value::as_array);
        // a composite (or DirectQuery) model: every card shows its storage mode
        let composite = crate::storage_ui::composite(model);
        for (k, card) in d.cards.iter().enumerate() {
            if !card.rect.intersects(visible) {
                continue;
            }
            let Some(t) = tables.and_then(|a| a.get(card.table)) else {
                continue;
            };
            self.draw_card(&painter, xf, k, card, t, visible, composite, l, pal);
        }
        // a column dragged towards another: a line to the pointer, the row under it lit
        if let Some(Drag::Column { from, at, .. }) = &self.drag {
            if let Some(Hit::Column { card, index }) = d.hit(*at, tol)
                && let Some(row) = d
                    .cards
                    .get(card)
                    .and_then(|c| c.columns.iter().find(|r| r.index == index))
            {
                painter.rect_stroke(
                    xf.r(row.rect),
                    0.0,
                    Stroke::new(xf.len(2.0, 1.0), pal.accent),
                    StrokeKind::Inside,
                );
            }
            painter.extend(Shape::dashed_line(
                &[xf.p(*from), xf.p(*at)],
                Stroke::new(xf.len(2.0, 1.0), pal.accent),
                xf.len(6.0, 2.0),
                xf.len(4.0, 1.0),
            ));
        }
        // the viewer's tooltips (`title`)
        let tip = match self.hover {
            Some(Hit::Rel(k)) => d
                .links
                .get(k)
                .and_then(|lk| rels.get(lk.rel))
                .map(|r| rel_tip(r, l)),
            Some(Hit::Column { card, index }) => row_value(model, d, card, index, "columns").map(|c| {
                let mut t = format!(
                    "{} · {}",
                    clip(s(c, "name"), 120),
                    if s(c, "data_type").is_empty() {
                        l.t("unknown type", "type inconnu")
                    } else {
                        s(c, "data_type")
                    }
                );
                if yes(c, "calculated") {
                    t.push_str(l.t(" · calculated", " · calculée"));
                }
                if yes(c, "hidden") {
                    t.push_str(l.t(" · hidden", " · masquée"));
                }
                t
            }),
            Some(Hit::Measure { card, index }) => row_value(model, d, card, index, "measures").map(|m| {
                let mut t = clip(s(m, "name"), 120);
                if m.get("valid").and_then(Value::as_bool) == Some(false) {
                    t.push('\n');
                    t.push_str(l.t("invalid DAX", "DAX non valide"));
                }
                t.push('\n');
                t.push_str(&clip(s(m, "expression"), 300));
                t
            }),
            Some(Hit::Table(k)) => d.cards.get(k).map(|c| clip(&c.name, 120)),
            _ => None,
        };
        if let Some(tip) = tip {
            let _ = resp.on_hover_ui_at_pointer(|ui| {
                ui.label(tip);
            });
        }
    }

    fn draw_link(&self, painter: &Painter, xf: Xf, link: &diagram::Link, on: bool, pal: Pal) {
        let color = if on { pal.accent } else { pal.faint };
        let stroke = Stroke::new(xf.len(if on { 2.2 } else { 1.5 }, 0.6), color);
        let pts: Vec<Pos2> = link.path.iter().map(|p| xf.p(*p)).collect();
        if link.inactive {
            painter.extend(Shape::dashed_line(
                &pts,
                stroke,
                xf.len(5.0, 1.0),
                xf.len(4.0, 1.0),
            ));
        } else {
            painter.add(Shape::line(pts, stroke));
        }
        for a in &link.arrows {
            painter.add(Shape::convex_polygon(
                a.iter().map(|p| xf.p(*p)).collect(),
                color,
                Stroke::NONE,
            ));
        }
        for (at, t) in [link.from_mark, link.to_mark].into_iter().flatten() {
            text(
                painter,
                xf,
                xf.p(at),
                Align2::LEFT_BOTTOM,
                t,
                12.0,
                Face::Mono,
                pal.muted,
                40.0,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_card(
        &self,
        painter: &Painter,
        xf: Xf,
        k: usize,
        card: &diagram::Card,
        t: &Value,
        visible: Rect,
        composite: bool,
        l: Lang,
        pal: Pal,
    ) {
        let fade = |c: Color32, hidden: bool| if hidden { c.gamma_multiply(0.5) } else { c };
        let th = yes(t, "hidden");
        let z = xf.zoom;
        let r = xf.r(card.rect);
        let radius = CornerRadius::same((8.0 * z).round().clamp(0.0, 255.0) as u8);
        let selected = self.is_selected(&Sel::Table(card.name.clone()));
        painter.rect_filled(
            r.translate(vec2(0.0, xf.len(2.0, 1.0))),
            radius,
            fade(pal.shadow, th),
        );
        painter.rect(
            r,
            radius,
            fade(pal.surface, th),
            Stroke::new(
                xf.len(1.0, 0.5),
                fade(if selected { pal.accent } else { pal.border }, th),
            ),
            StrokeKind::Inside,
        );
        if selected {
            painter.rect_stroke(
                r,
                radius,
                Stroke::new(xf.len(2.0, 1.0), pal.accent_soft),
                StrokeKind::Outside,
            );
        }
        // the header
        let hr = xf.r(card.head);
        let top = (7.0 * z).round().clamp(0.0, 255.0) as u8;
        let head_radius = CornerRadius {
            nw: top,
            ne: top,
            sw: 0,
            se: 0,
        };
        painter.rect_filled(hr, head_radius, fade(pal.surface2, th));
        if self.hover == Some(Hit::Table(k)) {
            painter.rect_filled(hr, head_radius, pal.accent_soft);
        }
        painter.line_segment(
            [hr.left_bottom(), hr.right_bottom()],
            Stroke::new(xf.len(1.0, 0.5), fade(pal.border, th)),
        );
        let cy = hr.center().y;
        let ix = hr.min.x + 9.0 * z;
        table_icon(
            painter,
            Rect::from_center_size(pos2(ix + 8.0 * z, cy), vec2(14.0, 12.0) * z),
            fade(pal.accent, th),
            z,
        );
        let count = format!("{} · {}", card.n_columns, card.n_measures);
        let cw = text(
            painter,
            xf,
            pos2(hr.max.x - 9.0 * z, cy),
            Align2::RIGHT_CENTER,
            &count,
            11.0,
            Face::Regular,
            fade(pal.faint, th),
            80.0,
        )
        .width();
        // the storage mode badge: an icon and a word, left of the counts
        let mut right = hr.max.x - 9.0 * z - cw - 6.0 * z;
        if composite {
            let mode = crate::storage_ui::table_mode(t);
            // never colour alone: the word says it
            let ink = fade(
                match mode {
                    "directQuery" | "directLake" => pal.accent,
                    "dual" | "hybrid" => pal.warn,
                    _ => pal.muted,
                },
                th,
            );
            let word = crate::storage_ui::mode_text(mode, l);
            let ww = text(
                painter,
                xf,
                pos2(right - 4.0 * z, cy),
                Align2::RIGHT_CENTER,
                word,
                10.5,
                Face::Semibold,
                ink,
                90.0,
            )
            .width();
            let icon_size = 11.0 * z;
            let icon_rect = Rect::from_center_size(
                pos2(right - 4.0 * z - ww - 2.0 * z - icon_size / 2.0, cy),
                vec2(icon_size, icon_size),
            );
            if icon_size >= MIN_TEXT {
                crate::icons::paint(painter, icon_rect, crate::storage_ui::mode_icon(mode), ink, false);
            }
            let pill = Rect::from_min_max(
                pos2(icon_rect.min.x - 4.0 * z, cy - 8.0 * z),
                pos2(right, cy + 8.0 * z),
            );
            painter.rect_stroke(
                pill,
                CornerRadius::same((8.0 * z).round().clamp(0.0, 255.0) as u8),
                Stroke::new(xf.len(1.0, 0.5), ink.gamma_multiply(0.6)),
                StrokeKind::Inside,
            );
            right = pill.min.x - 6.0 * z;
        }
        let name = if card.name.is_empty() {
            l.t("(unnamed)", "(sans nom)")
        } else {
            card.name.as_str()
        };
        let name_x = ix + 22.0 * z;
        let name_w = (right - name_x) / z;
        text(
            painter,
            xf,
            pos2(name_x, cy),
            Align2::LEFT_CENTER,
            name,
            13.0,
            Face::Semibold,
            fade(pal.text, th),
            name_w,
        );
        // the rows
        let cols = t.get("columns").and_then(Value::as_array);
        let meas = t.get("measures").and_then(Value::as_array);
        for row in &card.columns {
            if !row.rect.intersects(visible) {
                continue;
            }
            let Some(c) = cols.and_then(|a| a.get(row.index)) else {
                continue;
            };
            let on = self.hover
                == Some(Hit::Column {
                    card: k,
                    index: row.index,
                })
                || self.is_selected(&Sel::Column {
                    table: card.name.clone(),
                    name: s(c, "name").to_string(),
                });
            let (badge, color) = type_badge(s(c, "data_type"));
            let right = yes(c, "calculated").then(|| ("ƒx".to_string(), pal.accent_ink, false, 10.5));
            self.draw_row(
                painter,
                xf,
                row.rect,
                on,
                th || yes(c, "hidden"),
                th,
                &badge,
                color.unwrap_or(pal.muted),
                pal.surface2,
                s(c, "name"),
                pal.text,
                right,
                pal,
            );
        }
        for row in &card.measures {
            if !row.rect.intersects(visible) {
                continue;
            }
            let Some(m) = meas.and_then(|a| a.get(row.index)) else {
                continue;
            };
            let on = self.hover
                == Some(Hit::Measure {
                    card: k,
                    index: row.index,
                })
                || self.is_selected(&Sel::Measure {
                    table: card.name.clone(),
                    name: s(m, "name").to_string(),
                });
            let bad = m.get("valid").and_then(Value::as_bool) == Some(false);
            let fmt = s(m, "format_string");
            let right = (!fmt.is_empty()).then(|| (fmt.to_string(), pal.faint, true, 10.5));
            let (bc, bg, nc) = if bad {
                (pal.err, pal.err_soft, pal.err)
            } else {
                (pal.accent_ink, pal.accent_soft, pal.text)
            };
            let name_rect = self.draw_row(
                painter,
                xf,
                row.rect,
                on,
                th || yes(m, "hidden"),
                th,
                if bad { "!" } else { "Σ" },
                bc,
                bg,
                s(m, "name"),
                nc,
                right,
                pal,
            );
            if bad && name_rect.width() > 0.0 {
                // the viewer's `text-decoration: underline wavy`, under the name
                let y = xf.r(row.rect).center().y + 7.0 * z;
                wavy(
                    painter,
                    name_rect.min.x,
                    name_rect.max.x,
                    y,
                    z,
                    Stroke::new(xf.len(1.0, 0.5), pal.err.gamma_multiply(0.8)),
                );
            }
        }
        if let Some(y) = card.measures_top {
            let a = xf.p(pos2(card.rect.min.x + 1.0, y + 0.5));
            let b = xf.p(pos2(card.rect.max.x - 1.0, y + 0.5));
            painter.extend(Shape::dashed_line(
                &[a, b],
                Stroke::new(xf.len(1.0, 0.5), fade(pal.border, th)),
                xf.len(3.0, 1.0),
                xf.len(3.0, 1.0),
            ));
        }
        if let Some(m) = card.more {
            let mr = xf.r(m);
            if self.hover == Some(Hit::More(k)) {
                painter.rect_filled(mr, 0.0, pal.accent_soft);
            }
            painter.line_segment(
                [mr.left_top(), mr.right_top()],
                Stroke::new(xf.len(1.0, 0.5), fade(pal.border, th)),
            );
            let label = if card.expanded {
                l.t("Show less", "Afficher moins").to_string()
            } else {
                format!(
                    "{} ({}, {})",
                    l.t("Show all", "Tout afficher"),
                    plural(l, card.n_columns, ("column", "columns"), ("colonne", "colonnes")),
                    plural(l, card.n_measures, ("measure", "measures"), ("mesure", "mesures"))
                )
            };
            text(
                painter,
                xf,
                mr.center(),
                Align2::CENTER_CENTER,
                &label,
                12.0,
                Face::Regular,
                fade(pal.accent, th),
                diagram::CARD_W - 12.0,
            );
        }
        if let Some(n) = card.note {
            let nr = xf.r(n);
            text(
                painter,
                xf,
                pos2(nr.min.x + 9.0 * z, nr.center().y),
                Align2::LEFT_CENTER,
                l.t("truncated", "tronqué"),
                12.0,
                Face::Regular,
                pal.muted,
                200.0,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_row(
        &self,
        painter: &Painter,
        xf: Xf,
        rect: Rect,
        on: bool,
        hidden: bool,
        table_hidden: bool,
        badge: &str,
        badge_color: Color32,
        badge_bg: Color32,
        name: &str,
        name_color: Color32,
        right: Option<(String, Color32, bool, f32)>,
        pal: Pal,
    ) -> Rect {
        let z = xf.zoom;
        let alpha = if hidden {
            if table_hidden { 0.5 } else { 0.45 }
        } else {
            1.0
        };
        let fade = |c: Color32| if alpha < 1.0 { c.gamma_multiply(alpha) } else { c };
        let rr = xf.r(rect);
        if on {
            painter.rect_filled(rr, 0.0, pal.accent_soft);
        }
        let cy = rr.center().y;
        // the badge: monospace 9.5 px, at least 30 px wide
        let bw = (badge.chars().count() as f32 * 5.8 + 6.0).max(30.0);
        let br = Rect::from_min_size(pos2(rr.min.x + 9.0 * z, cy - 7.0 * z), vec2(bw, 14.0) * z);
        painter.rect_filled(
            br,
            CornerRadius::same((3.0 * z).round().clamp(0.0, 255.0) as u8),
            fade(badge_bg),
        );
        text(
            painter,
            xf,
            br.center(),
            Align2::CENTER_CENTER,
            badge,
            9.5,
            Face::Mono,
            fade(badge_color),
            bw,
        );
        let mut right_edge = rr.max.x - 9.0 * z;
        if let Some((t, c, mono, size)) = right {
            let w = text(
                painter,
                xf,
                pos2(right_edge, cy),
                Align2::RIGHT_CENTER,
                &t,
                size,
                if mono { Face::Mono } else { Face::Regular },
                fade(c),
                80.0,
            )
            .width();
            right_edge -= w + 6.0 * z;
        }
        let nx = br.max.x + 6.0 * z;
        text(
            painter,
            xf,
            pos2(nx, cy),
            Align2::LEFT_CENTER,
            name,
            13.0,
            Face::Regular,
            fade(name_color),
            (right_edge - nx) / z,
        )
    }

    // --- the details panel ------------------------------------------------------------------

    fn panel(&mut self, ui: &mut egui::Ui, model: &Value, snap: &Value, seq: u64, l: Lang, pal: Pal) {
        let mut go: Option<Option<Sel>> = None;
        let tables: Vec<&Value> = diagram::objects(model, "tables").map(|x| x.1).collect();
        let find_table = |n: &str| tables.iter().copied().find(|t| s(t, "name") == n);
        let sel = self.sel.clone();
        let all = self.selection();
        if all.len() > 1 {
            // several objects: the properties they share
            let picked: Vec<crate::props_pane::Picked<'_>> = all
                .iter()
                .filter_map(|x| {
                    use crate::props_pane::{ObjKind, Picked};
                    let (kind, table, name) = match x {
                        Sel::Table(t) => (ObjKind::Table, t, None),
                        Sel::Column { table, name } => (ObjKind::Column, table, Some((name, "columns"))),
                        Sel::Measure { table, name } => (ObjKind::Measure, table, Some((name, "measures"))),
                        Sel::Rel(_) | Sel::Item(_) => return None,
                    };
                    let t = find_table(table)?;
                    let obj = match name {
                        None => t,
                        Some((n, list)) => diagram::objects(t, list)
                            .map(|x| x.1)
                            .find(|o| s(o, "name") == n)?,
                    };
                    Some(Picked { kind, table: t, obj })
                })
                .collect();
            let mut props = crate::props_pane::Props {
                l,
                editable: self.editable,
                edit: &mut self.edit,
                run: &mut self.run,
                muted: pal.muted,
            };
            multi_panel(ui, &picked, l, pal, &mut go, &mut props);
            if let Some(next) = go {
                self.select(next);
            }
            return;
        }
        let mentions = self.mentions(model, seq, sel.as_ref());
        let mut props = crate::props_pane::Props {
            l,
            editable: self.editable,
            edit: &mut self.edit,
            run: &mut self.run,
            muted: pal.muted,
        };
        let shown = match &sel {
            Some(Sel::Measure { table, name }) => find_table(table)
                .and_then(|t| {
                    diagram::objects(t, "measures")
                        .map(|x| x.1)
                        .find(|m| s(m, "name") == name)
                })
                .map(|m| measure_panel(ui, m, table, snap, &mentions, l, pal, &mut go, &mut props))
                .is_some(),
            Some(Sel::Column { table, name }) => find_table(table)
                .and_then(|t| {
                    diagram::objects(t, "columns")
                        .map(|x| x.1)
                        .find(|c| s(c, "name") == name)
                        .map(|c| (t, c))
                })
                .map(|(t, c)| column_panel(ui, c, t, model, snap, &mentions, l, pal, &mut go, &mut props))
                .is_some(),
            Some(Sel::Table(name)) => find_table(name)
                .map(|t| table_panel(ui, t, model, l, pal, &mut go, &mut props))
                .is_some(),
            Some(Sel::Rel(i)) => rel_at(model, *i)
                .map(|r| rel_panel(ui, r, l, pal, &mut go, &mut props))
                .is_some(),
            Some(Sel::Item(item)) => item_panel(ui, item, model, l, pal, &mut go, &mut props),
            None => false,
        };
        if !shown {
            summary_panel(ui, model, &tables, l, pal, &mut go);
        }
        if let Some(next) = go {
            self.select(next);
        }
    }

    /// The measures whose expression mentions the selected measure or column (cached).
    fn mentions(&mut self, model: &Value, seq: u64, sel: Option<&Sel>) -> Mentions {
        let Some(sel) = sel else {
            return Arc::default();
        };
        if let Some((sq, ss, m)) = &self.mentions
            && *sq == seq
            && ss == sel
        {
            return m.clone();
        }
        let m = Arc::new(mentions(model, sel));
        self.mentions = Some((seq, sel.clone(), m.clone()));
        m
    }
}

/// The measures whose DAX mentions a measure (`[Name]`) or a column (`Table[Column]`,
/// `'Table'[Column]`), compared without case: a text search, bounded.
pub fn mentions(model: &Value, sel: &Sel) -> Vec<(String, String)> {
    let (needles, me): (Vec<String>, Option<(&str, &str)>) = match sel {
        Sel::Measure { table, name } => (
            vec![format!("[{}]", name.to_lowercase())],
            Some((table.as_str(), name.as_str())),
        ),
        Sel::Column { table, name } => {
            let (t, n) = (table.to_lowercase(), name.to_lowercase());
            (vec![format!("{t}[{n}]"), format!("'{t}'[{n}]")], None)
        }
        _ => return Vec::new(),
    };
    let mut out = Vec::new();
    let mut scanned = 0usize;
    for (_, t) in diagram::objects(model, "tables") {
        for (_, m) in diagram::objects(t, "measures") {
            scanned += 1;
            if scanned > MAX_SCAN || out.len() >= MAX_MENTIONS {
                return out;
            }
            let (tn, mn) = (s(t, "name"), s(m, "name"));
            if me == Some((tn, mn)) {
                continue;
            }
            let e = s(m, "expression").to_lowercase();
            if needles.iter().any(|n| e.contains(n.as_str())) {
                out.push((tn.to_string(), mn.to_string()));
            }
        }
    }
    out
}

fn row_value<'a>(model: &'a Value, d: &Diagram, card: usize, index: usize, k: &str) -> Option<&'a Value> {
    let c = d.cards.get(card)?;
    model
        .get("tables")?
        .as_array()?
        .get(c.table)?
        .get(k)?
        .as_array()?
        .get(index)
}

fn row_sel(model: &Value, d: &Diagram, card: usize, index: usize, k: &str) -> Option<Sel> {
    let v = row_value(model, d, card, index, k)?;
    let table = d.cards.get(card)?.name.clone();
    let name = s(v, "name").to_string();
    Some(if k == "columns" {
        Sel::Column { table, name }
    } else {
        Sel::Measure { table, name }
    })
}

fn card_mark(c: &str) -> &'static str {
    match c.to_ascii_lowercase().as_str() {
        "one" => "1",
        "many" => "*",
        _ => "?",
    }
}

/// Relationship `i` of the snapshot's model.
fn rel_at(model: &Value, i: usize) -> Option<&Value> {
    model
        .get("relationships")
        .and_then(Value::as_array)
        .and_then(|a| a.get(i))
        .filter(|r| r.is_object())
}

/// How the `relationship.*` commands name relationship `r`: its name, else its two columns.
pub(crate) fn rel_ref(r: &Value) -> Value {
    if s(r, "name").is_empty() {
        serde_json::json!({
            "from": format!("{}[{}]", s(r, "from_table"), s(r, "from_column")),
            "to": format!("{}[{}]", s(r, "to_table"), s(r, "to_column")),
        })
    } else {
        serde_json::json!({ "name": s(r, "name") })
    }
}

/// `relationship.set`'s form for relationship `r`, with its current values.
pub(crate) fn rel_values(r: &Value) -> Value {
    let mut p = rel_ref(r);
    if let Some(o) = p.as_object_mut() {
        o.insert(
            "active".into(),
            Value::Bool(r.get("active").and_then(Value::as_bool) != Some(false)),
        );
        for (k, allowed) in [
            ("cross_filter", &["single", "both"][..]),
            ("from_cardinality", &["one", "many"][..]),
            ("to_cardinality", &["one", "many"][..]),
            ("security_filtering", &["single", "both", "none"][..]),
        ] {
            let v = s(r, k).to_ascii_lowercase();
            if allowed.contains(&v.as_str()) {
                o.insert(k.into(), Value::String(v));
            }
        }
    }
    p
}

fn rel_tip(r: &Value, l: Lang) -> String {
    let mut t = format!(
        "{}[{}] {} → {} {}[{}]\n{}: {}",
        s(r, "from_table"),
        s(r, "from_column"),
        card_mark(s(r, "from_cardinality")),
        card_mark(s(r, "to_cardinality")),
        s(r, "to_table"),
        s(r, "to_column"),
        l.t("cross filter", "filtrage croisé"),
        if s(r, "cross_filter").is_empty() {
            "?"
        } else {
            s(r, "cross_filter")
        },
    );
    if r.get("active").and_then(Value::as_bool) == Some(false) {
        t.push('\n');
        t.push_str(l.t("inactive", "inactive"));
    }
    clip(&t, 600)
}

/// `n word(s)` in the chrome's language.
fn plural(l: Lang, n: usize, en: (&str, &str), fr: (&str, &str)) -> String {
    let (one, many) = if l == Lang::Fr { fr } else { en };
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The face of diagram text: the viewer's `font-family` and `font-weight`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Face {
    Regular,
    /// `font-weight: 600`–`650` (card headers).
    Semibold,
    /// `var(--mono)`: badges, cardinality marks, format strings.
    Mono,
}

/// Text in the diagram: `size` diagram pixels, at most `max_w` diagram pixels wide (elided).
/// Returns where it went (nothing when too small to read).
#[allow(clippy::too_many_arguments)]
fn text(
    painter: &Painter,
    xf: Xf,
    at: Pos2,
    align: Align2,
    t: &str,
    size: f32,
    face: Face,
    color: Color32,
    max_w: f32,
) -> Rect {
    // sizes in half points, so zooming does not fill the font atlas with sizes
    let px = (size * xf.zoom * 2.0).round() / 2.0;
    if px < MIN_TEXT || max_w <= 0.0 || t.is_empty() {
        return Rect::from_min_size(at, Vec2::ZERO);
    }
    let font = match face {
        Face::Regular => FontId::proportional(px),
        Face::Semibold => crate::fonts::semibold_px(px),
        Face::Mono => FontId::monospace(px),
    };
    let mut job = LayoutJob::single_section(
        clip(t, 300),
        TextFormat {
            font_id: font,
            color,
            ..Default::default()
        },
    );
    job.wrap = TextWrapping::truncate_at_width((max_w * xf.zoom).max(1.0));
    let g = painter.layout_job(job);
    let r = align.anchor_size(at, g.size());
    painter.galley(r.min, g, color);
    r
}

/// A small table glyph (the viewer's `icon('model')`).
/// A wavy line from `x0` to `x1` at `y` (CSS `underline wavy`: 2 px amplitude peak to peak,
/// 4 px wavelength at 100 %), at most 400 segments.
fn wavy(painter: &Painter, x0: f32, x1: f32, y: f32, z: f32, stroke: Stroke) {
    let half = (2.0 * z).max(1.0);
    let amp = (1.0 * z).max(0.5);
    if !(x0.is_finite() && x1.is_finite() && x1 > x0) {
        return;
    }
    let n = (((x1 - x0) / half).ceil() as usize).clamp(1, 400);
    let step = (x1 - x0) / n as f32;
    let points: Vec<Pos2> = (0..=n)
        .map(|i| {
            let dy = if i % 2 == 0 { amp } else { -amp };
            pos2(x0 + step * i as f32, y + dy)
        })
        .collect();
    painter.add(Shape::line(points, stroke));
}

fn table_icon(painter: &Painter, r: Rect, color: Color32, z: f32) {
    let st = Stroke::new((1.3 * z).max(0.5), color);
    let cr = CornerRadius::same((2.0 * z).round().clamp(0.0, 255.0) as u8);
    painter.rect_stroke(r, cr, st, StrokeKind::Inside);
    let y = r.min.y + r.height() * 0.38;
    painter.line_segment([pos2(r.min.x, y), pos2(r.max.x, y)], st);
    let x = r.min.x + r.width() * 0.4;
    painter.line_segment([pos2(x, y), pos2(x, r.max.y)], st);
}

fn pill(ui: &mut egui::Ui, t: &str, fg: Color32, bg: Color32) {
    egui::Frame::new()
        .fill(bg)
        .corner_radius(CornerRadius::same(9))
        .inner_margin(egui::Margin::symmetric(7, 1))
        .show(ui, |ui| {
            ui.label(RichText::new(t).small().color(fg));
        });
}

fn header(ui: &mut egui::Ui, title: &str, sub: &str, pal: Pal, go: &mut Option<Option<Sel>>, l: Lang) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new(clip(title, 200)).heading().strong());
            if !sub.is_empty() {
                ui.label(RichText::new(sub).small().color(pal.muted));
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            if ui
                .small_button("×")
                .on_hover_text(l.t("Close (Esc)", "Fermer (Échap)"))
                .clicked()
            {
                *go = Some(None);
            }
        });
    });
}

fn section(ui: &mut egui::Ui, title: &str, pal: Pal) {
    ui.add_space(10.0);
    ui.label(
        RichText::new(title)
            .font(crate::fonts::semibold_px(11.0))
            .color(pal.muted),
    );
}

fn kv(ui: &mut egui::Ui, id: &str, rows: &[(&str, String)], pal: Pal) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([12.0, 3.0])
        .show(ui, |ui| {
            for (k, v) in rows {
                ui.label(RichText::new(*k).color(pal.muted));
                ui.label(clip(v, 200));
                ui.end_row();
            }
        });
}

fn or_dash(t: &str) -> String {
    if t.is_empty() { "–".into() } else { t.to_string() }
}

fn table_link(ui: &mut egui::Ui, name: &str, go: &mut Option<Option<Sel>>) {
    if ui.link(clip(name, 120)).clicked() {
        *go = Some(Some(Sel::Table(name.to_string())));
    }
}

/// The diagnostics of `table`/`name` in the snapshot's validation (the viewer's `diagnosticsFor`).
fn diagnostics(snap: &Value, table: &str, name: &str) -> Vec<String> {
    snap.pointer("/validation/model/objects")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|o| s(o, "table") == table && s(o, "name") == name)
        .take(50)
        .flat_map(|o| {
            let d: Vec<String> = o
                .get("diagnostics")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .take(50)
                .map(issue_text)
                .collect();
            if d.is_empty() {
                vec!["invalid".to_string()]
            } else {
                d
            }
        })
        .collect()
}

fn diagnostics_section(ui: &mut egui::Ui, diags: &[String], l: Lang, pal: Pal) {
    if diags.is_empty() {
        return;
    }
    section(ui, l.t("Diagnostics", "Diagnostics"), pal);
    for d in diags {
        ui.label(RichText::new(clip(d, 400)).color(pal.err));
    }
}

fn expression(ui: &mut egui::Ui, e: &str, l: Lang, pal: Pal) {
    section(ui, l.t("Expression", "Expression"), pal);
    let e = clip(e, 200_000);
    let shown = if e.is_empty() {
        l.t("(empty)", "(vide)").to_string()
    } else {
        e
    };
    let mut shown: &str = &shown;
    egui::Frame::new()
        .fill(pal.surface2)
        .stroke(Stroke::new(1.0, pal.border))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut shown)
                    .code_editor()
                    .frame(egui::Frame::NONE)
                    .desired_rows(1)
                    .desired_width(f32::INFINITY),
            );
        });
}

fn mentions_section(
    ui: &mut egui::Ui,
    m: &[(String, String)],
    l: Lang,
    pal: Pal,
    go: &mut Option<Option<Sel>>,
) {
    if m.is_empty() {
        return;
    }
    section(ui, l.t("Mentioned in measures", "Cité dans les mesures"), pal);
    for (t, n) in m {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Σ").color(pal.accent_ink));
            if ui.selectable_label(false, clip(n, 120)).clicked() {
                *go = Some(Some(Sel::Measure {
                    table: t.clone(),
                    name: n.clone(),
                }));
            }
            ui.label(RichText::new(clip(t, 60)).small().color(pal.muted));
        });
    }
}

fn raw_json(ui: &mut egui::Ui, id: &str, v: &Value, l: Lang) {
    egui::CollapsingHeader::new(l.t("Raw JSON", "JSON brut"))
        .id_salt(id)
        .default_open(false)
        .show(ui, |ui| {
            let t = clip(&serde_json::to_string_pretty(v).unwrap_or_default(), 60_000);
            let mut t: &str = &t;
            ui.add(
                egui::TextEdit::multiline(&mut t)
                    .code_editor()
                    .desired_width(f32::INFINITY),
            );
        });
}

/// Most selected objects the multi-selection panel names.
const MAX_NAMED: usize = 12;

/// Several selected objects: how many, their names, then the properties they share.
fn multi_panel(
    ui: &mut egui::Ui,
    objs: &[crate::props_pane::Picked<'_>],
    l: Lang,
    pal: Pal,
    go: &mut Option<Option<Sel>>,
    props: &mut crate::props_pane::Props<'_>,
) {
    use crate::props_pane::ObjKind;
    let n = objs.len();
    let title = match l {
        Lang::En => format!("{n} objects selected"),
        Lang::Fr => format!("{n} objets sélectionnés"),
    };
    header(
        ui,
        &title,
        l.t(
            "Ctrl+click adds or removes one",
            "Ctrl+clic en ajoute ou en retire un",
        ),
        pal,
        go,
        l,
    );
    ui.add_space(4.0);
    for o in objs.iter().take(MAX_NAMED) {
        let text = match o.kind {
            ObjKind::Table => s(o.table, "name").to_string(),
            _ => format!("{}[{}]", s(o.table, "name"), s(o.obj, "name")),
        };
        ui.label(RichText::new(clip(&text, 120)).small().color(pal.muted));
    }
    if n > MAX_NAMED {
        let more = n.saturating_sub(MAX_NAMED);
        let text = match l {
            Lang::En => format!("and {more} more"),
            Lang::Fr => format!("et {more} de plus"),
        };
        ui.label(RichText::new(text).small().color(pal.muted));
    }
    section(ui, l.t("Properties", "Propriétés"), pal);
    props.multi(ui, objs);
}

#[allow(clippy::too_many_arguments)]
fn measure_panel(
    ui: &mut egui::Ui,
    m: &Value,
    table: &str,
    snap: &Value,
    mentions: &[(String, String)],
    l: Lang,
    pal: Pal,
    go: &mut Option<Option<Sel>>,
    props: &mut crate::props_pane::Props<'_>,
) {
    header(ui, s(m, "name"), "", pal, go, l);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(l.t("Measure in", "Mesure dans"))
                .small()
                .color(pal.muted),
        );
        table_link(ui, table, go);
    });
    match m.get("valid").and_then(Value::as_bool) {
        Some(false) => pill(ui, l.t("invalid DAX", "DAX non valide"), pal.err, pal.err_soft),
        Some(true) => pill(ui, l.t("valid", "valide"), pal.ok, pal.ok.gamma_multiply(0.15)),
        None => pill(ui, l.t("not validated", "non validée"), pal.muted, pal.surface2),
    }
    diagnostics_section(ui, &diagnostics(snap, table, s(m, "name")), l, pal);
    expression(ui, s(m, "expression"), l, pal);
    section(ui, l.t("Properties", "Propriétés"), pal);
    props.measure(ui, m, table);
    mentions_section(ui, mentions, l, pal, go);
    raw_json(ui, "measure-raw", m, l);
}

#[allow(clippy::too_many_arguments)]
fn column_panel(
    ui: &mut egui::Ui,
    c: &Value,
    t: &Value,
    model: &Value,
    snap: &Value,
    mentions: &[(String, String)],
    l: Lang,
    pal: Pal,
    go: &mut Option<Option<Sel>>,
    props: &mut crate::props_pane::Props<'_>,
) {
    let table = s(t, "name");
    let name = s(c, "name");
    header(ui, name, "", pal, go, l);
    ui.horizontal(|ui| {
        let what = if yes(c, "calculated") {
            l.t("Calculated column in", "Colonne calculée dans")
        } else {
            l.t("Column in", "Colonne dans")
        };
        ui.label(RichText::new(what).small().color(pal.muted));
        table_link(ui, table, go);
    });
    diagnostics_section(ui, &diagnostics(snap, table, name), l, pal);
    section(ui, l.t("Properties", "Propriétés"), pal);
    props.column(ui, c, t);
    if !s(c, "expression").is_empty() {
        expression(ui, s(c, "expression"), l, pal);
    }
    rels_section(ui, model, table, Some(name), l, pal, go);
    mentions_section(ui, mentions, l, pal, go);
    raw_json(ui, "column-raw", c, l);
}

fn table_panel(
    ui: &mut egui::Ui,
    t: &Value,
    model: &Value,
    l: Lang,
    pal: Pal,
    go: &mut Option<Option<Sel>>,
    props: &mut crate::props_pane::Props<'_>,
) {
    let name = s(t, "name");
    let sub = if yes(t, "hidden") {
        l.t("Table · hidden", "Table · masquée")
    } else {
        l.t("Table", "Table")
    };
    header(ui, name, sub, pal, go, l);
    section(ui, l.t("Properties", "Propriétés"), pal);
    props.table(ui, t);
    let cols: Vec<&Value> = diagram::objects(t, "columns").map(|x| x.1).collect();
    let meas: Vec<&Value> = diagram::objects(t, "measures").map(|x| x.1).collect();
    section(ui, l.t("Contents", "Contenu"), pal);
    kv(
        ui,
        "table-contents",
        &[
            (l.t("columns", "colonnes"), cols.len().to_string()),
            (l.t("measures", "mesures"), meas.len().to_string()),
        ],
        pal,
    );
    rels_section(ui, model, name, None, l, pal, go);
    section(ui, l.t("Columns and measures", "Colonnes et mesures"), pal);
    if cols.is_empty() && meas.is_empty() {
        ui.label(RichText::new(l.t("Empty table.", "Table vide.")).color(pal.muted));
    }
    let mut shown = 0usize;
    for c in &cols {
        if shown >= MAX_LIST {
            break;
        }
        shown += 1;
        let (badge, color) = type_badge(s(c, "data_type"));
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("{badge:>4}"))
                    .monospace()
                    .small()
                    .color(color.unwrap_or(pal.muted)),
            );
            let mut label = RichText::new(clip(s(c, "name"), 120));
            if yes(c, "hidden") {
                label = label.color(pal.faint);
            }
            if ui.selectable_label(false, label).clicked() {
                *go = Some(Some(Sel::Column {
                    table: name.to_string(),
                    name: s(c, "name").to_string(),
                }));
            }
            if yes(c, "calculated") {
                ui.label(RichText::new("ƒx").italics().small().color(pal.accent_ink));
            }
        });
    }
    for m in &meas {
        if shown >= MAX_LIST {
            break;
        }
        shown += 1;
        let bad = m.get("valid").and_then(Value::as_bool) == Some(false);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(if bad { "   !" } else { "   Σ" })
                    .monospace()
                    .small()
                    .color(if bad { pal.err } else { pal.accent_ink }),
            );
            let mut label = RichText::new(clip(s(m, "name"), 120));
            if yes(m, "hidden") {
                label = label.color(pal.faint);
            }
            if ui.selectable_label(false, label).clicked() {
                *go = Some(Some(Sel::Measure {
                    table: name.to_string(),
                    name: s(m, "name").to_string(),
                }));
            }
            if bad {
                pill(ui, l.t("invalid", "non valide"), pal.err, pal.err_soft);
            }
        });
    }
    let total = cols.len().saturating_add(meas.len());
    if total > shown {
        ui.label(RichText::new(format!("… {} {}", total - shown, l.t("more", "de plus"))).color(pal.muted));
    }
    raw_json(ui, "table-raw", t, l);
}

fn rels_section(
    ui: &mut egui::Ui,
    model: &Value,
    table: &str,
    column: Option<&str>,
    l: Lang,
    pal: Pal,
    go: &mut Option<Option<Sel>>,
) {
    let list: Vec<(usize, &Value)> = diagram::objects(model, "relationships")
        .filter(|(_, r)| {
            let from = s(r, "from_table") == table && column.is_none_or(|c| s(r, "from_column") == c);
            let to = s(r, "to_table") == table && column.is_none_or(|c| s(r, "to_column") == c);
            from || to
        })
        .take(200)
        .collect();
    if list.is_empty() {
        return;
    }
    section(ui, l.t("Relationships", "Relations"), pal);
    for (i, r) in list {
        ui.horizontal(|ui| {
            let t = format!(
                "{}[{}] → {}[{}]",
                s(r, "from_table"),
                s(r, "from_column"),
                s(r, "to_table"),
                s(r, "to_column")
            );
            if ui
                .selectable_label(false, RichText::new(clip(&t, 160)).small())
                .clicked()
            {
                *go = Some(Some(Sel::Rel(i)));
            }
            if r.get("active").and_then(Value::as_bool) == Some(false) {
                pill(ui, l.t("inactive", "inactive"), pal.muted, pal.surface2);
            }
        });
    }
}

fn rel_panel(
    ui: &mut egui::Ui,
    r: &Value,
    l: Lang,
    pal: Pal,
    go: &mut Option<Option<Sel>>,
    props: &mut crate::props_pane::Props<'_>,
) {
    let active = r.get("active").and_then(Value::as_bool) != Some(false);
    header(
        ui,
        l.t("Relationship", "Relation"),
        if active {
            l.t("active", "active")
        } else {
            l.t("inactive", "inactive")
        },
        pal,
        go,
        l,
    );
    for (title, side) in [(l.t("From", "De"), "from"), (l.t("To", "Vers"), "to")] {
        section(ui, title, pal);
        egui::Grid::new(("rel", side))
            .num_columns(2)
            .spacing([12.0, 3.0])
            .show(ui, |ui| {
                ui.label(RichText::new(l.t("table", "table")).color(pal.muted));
                table_link(ui, s(r, &format!("{side}_table")), go);
                ui.end_row();
                ui.label(RichText::new(l.t("column", "colonne")).color(pal.muted));
                ui.label(clip(s(r, &format!("{side}_column")), 120));
                ui.end_row();
            });
    }
    section(ui, l.t("Behaviour", "Comportement"), pal);
    props.relationship(ui, r, &rel_ref(r));
    raw_json(ui, "rel-raw", r, l);
}

/// The object named `name` in list `k` of `v` (`None` when it is not there).
fn named<'a>(v: &'a Value, k: &str, name: &str) -> Option<&'a Value> {
    diagram::objects(v, k).map(|x| x.1).find(|o| s(o, "name") == name)
}

/// A Model explorer object the diagram does not draw: its Properties pane. Whether it was
/// found in the snapshot.
fn item_panel(
    ui: &mut egui::Ui,
    item: &Item,
    model: &Value,
    l: Lang,
    pal: Pal,
    go: &mut Option<Option<Sel>>,
    props: &mut crate::props_pane::Props<'_>,
) -> bool {
    let table = |n: &str| named(model, "tables", n);
    let (obj, name, sub) = match item {
        Item::CalcItem { group, name } => (
            table(group).and_then(|t| named(t, "calculation_item_details", name)),
            name,
            l.t("Calculation item", "Élément de calcul"),
        ),
        Item::Hierarchy { table: t, name } => (
            table(t).and_then(|t| named(t, "hierarchy_levels", name)),
            name,
            l.t("Hierarchy", "Hiérarchie"),
        ),
        Item::Role(name) => (named(model, "security", name), name, l.t("Role", "Rôle")),
        Item::Culture(name) => (
            named(model, "culture_details", name),
            name,
            l.t("Culture", "Culture"),
        ),
        Item::Expression(name) => (
            named(model, "expression_details", name),
            name,
            l.t("Shared expression", "Expression partagée"),
        ),
        Item::DataSource(name) => (
            named(model, "data_sources", name),
            name,
            l.t("Data source", "Source de données"),
        ),
    };
    let Some(obj) = obj else { return false };
    header(ui, name, sub, pal, go, l);
    match item {
        Item::CalcItem { group, .. } | Item::Hierarchy { table: group, .. } => {
            ui.horizontal(|ui| {
                ui.label(RichText::new(l.t("In", "Dans")).small().color(pal.muted));
                table_link(ui, group, go);
            });
        }
        _ => {}
    }
    section(ui, l.t("Properties", "Propriétés"), pal);
    match item {
        Item::CalcItem { group, .. } => props.calc_item(ui, obj, group),
        Item::Hierarchy { table, .. } => props.hierarchy(ui, obj, table),
        Item::Role(_) => props.role(ui, obj),
        Item::Culture(_) => props.culture(ui, obj),
        Item::Expression(_) => props.expression(ui, obj),
        Item::DataSource(_) => props.data_source(ui, obj),
    }
    raw_json(ui, "item-raw", obj, l);
    true
}

fn summary_panel(
    ui: &mut egui::Ui,
    model: &Value,
    tables: &[&Value],
    l: Lang,
    pal: Pal,
    go: &mut Option<Option<Sel>>,
) {
    let name = s(model, "name");
    ui.label(
        RichText::new(clip(
            if name.is_empty() {
                l.t("Model", "Modèle")
            } else {
                name
            },
            200,
        ))
        .heading()
        .strong(),
    );
    ui.label(
        RichText::new(l.t(
            "Semantic model · click a table, column or measure",
            "Modèle sémantique · cliquez une table, une colonne ou une mesure",
        ))
        .small()
        .color(pal.muted),
    );
    section(ui, l.t("Model", "Modèle"), pal);
    kv(
        ui,
        "model-summary",
        &[
            (l.t("culture", "culture"), or_dash(s(model, "culture"))),
            (l.t("tables", "tables"), tables.len().to_string()),
            (
                l.t("relationships", "relations"),
                diagram::objects(model, "relationships").count().to_string(),
            ),
        ],
        pal,
    );
    let bad: Vec<(&str, &str)> = tables
        .iter()
        .flat_map(|t| {
            diagram::objects(t, "measures")
                .filter(|(_, m)| m.get("valid").and_then(Value::as_bool) == Some(false))
                .map(|(_, m)| (s(t, "name"), s(m, "name")))
        })
        .take(MAX_LIST)
        .collect();
    if !bad.is_empty() {
        section(ui, l.t("Invalid measures", "Mesures non valides"), pal);
        for (t, m) in bad {
            ui.horizontal(|ui| {
                ui.label(RichText::new("!").monospace().color(pal.err));
                if ui.selectable_label(false, clip(m, 120)).clicked() {
                    *go = Some(Some(Sel::Measure {
                        table: t.to_string(),
                        name: m.to_string(),
                    }));
                }
                ui.label(RichText::new(clip(t, 60)).small().color(pal.muted));
            });
        }
    }
    section(ui, l.t("Tables", "Tables"), pal);
    if tables.is_empty() {
        ui.label(RichText::new(l.t("No tables yet.", "Aucune table.")).color(pal.muted));
    }
    for t in tables.iter().take(MAX_LIST) {
        let n = s(t, "name");
        ui.horizontal(|ui| {
            let mut label = RichText::new(clip(n, 120));
            if yes(t, "hidden") {
                label = label.color(pal.faint);
            }
            if ui.selectable_label(false, label).clicked() {
                *go = Some(Some(Sel::Table(n.to_string())));
            }
            ui.label(
                RichText::new(format!(
                    "{} · {}",
                    diagram::objects(t, "columns").count(),
                    diagram::objects(t, "measures").count()
                ))
                .small()
                .color(pal.faint),
            );
        });
    }
    if tables.len() > MAX_LIST {
        ui.label(
            RichText::new(format!(
                "… {} {}",
                tables.len() - MAX_LIST,
                l.t("more", "de plus")
            ))
            .color(pal.muted),
        );
    }
}

/// What a thin report says in the Model view (the snapshot's `source.live_connection`): its model
/// is published in the Power BI service, which BIcraft does not sign in to, a known limitation not
/// pursued by the maintainers' decision (docs/gaps.md G12).
pub fn thin_note(snap: &Value, l: Lang) -> Option<String> {
    let live = snap
        .pointer("/source/live_connection")
        .filter(|v| v.is_object())?;
    let s = |k: &str| {
        live.get(k)
            .and_then(Value::as_str)
            .map(|v| clip(v, 200))
            .filter(|v| !v.is_empty())
    };
    let mut via: Vec<String> = Vec::new();
    if let Some(t) = s("type") {
        via.push(t);
    }
    if let Some(d) = s("data_source") {
        via.push(d);
    }
    if let Some(c) = s("catalog") {
        via.push(format!("{} {c}", l.t("model", "modèle")));
    }
    let via = if via.is_empty() {
        String::new()
    } else {
        format!(" ({})", via.join(", "))
    };
    Some(format!(
        "{}{via}. {}",
        l.t(
            "A thin report: its semantic model is published in the Power BI service, reached through a live connection",
            "Un rapport léger : son modèle sémantique est publié dans le service Power BI, atteint par une connexion active",
        ),
        l.t(
            "BIcraft reads and edits the report but does not sign in to Power BI, so the model is not opened. This is a known limitation, not pursued by the maintainers' decision.",
            "BIcraft lit et modifie le rapport mais ne se connecte pas à Power BI : le modèle n'est donc pas ouvert. C'est une limite connue, non traitée par décision des mainteneurs.",
        )
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn badges_follow_the_viewer() {
        assert_eq!(type_badge("int64").0, "123");
        assert_eq!(type_badge("double").0, "1.2");
        assert_eq!(type_badge("string").0, "abc");
        assert_eq!(type_badge("dateTime").0, "date");
        assert_eq!(type_badge("boolean").0, "t/f");
        assert_eq!(type_badge("binary").0, "bin");
        assert_eq!(type_badge("").0, "?");
        assert_eq!(type_badge("variant").0, "vari…");
    }

    #[test]
    fn mentions_find_measures_and_columns() {
        let m = json!({"tables": [{"name": "Sales", "columns": [{"name": "Amount"}], "measures": [
            {"name": "Total", "expression": "SUM ( Sales[Amount] )"},
            {"name": "Avg", "expression": "DIVIDE([total], COUNTROWS(Sales))"},
            {"name": "Q", "expression": "SUM('Sales'[Amount]) + [Avg]"},
        ]}]});
        let total = Sel::Measure {
            table: "Sales".into(),
            name: "Total".into(),
        };
        assert_eq!(
            mentions(&m, &total),
            vec![("Sales".to_string(), "Avg".to_string())]
        );
        let amount = Sel::Column {
            table: "Sales".into(),
            name: "Amount".into(),
        };
        let names: Vec<String> = mentions(&m, &amount).into_iter().map(|x| x.1).collect();
        assert_eq!(names, ["Total", "Q"]);
        assert!(mentions(&json!(null), &total).is_empty());
    }
}
