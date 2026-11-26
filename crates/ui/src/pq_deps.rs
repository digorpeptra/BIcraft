//! The Power Query editor's View › Query dependencies (Microsoft Learn, "Query dependencies"): a
//! diagram of the editor's queries, the data sources they read and the model tables they load
//! into, from `query.dependencies`, with zoom and fit. Sources are drawn at the left, each query
//! after the queries it reads, the model tables at the right; an arrow goes from what is read to
//! what reads it. Each node has an icon and the word of its kind (never the colour alone).
//!
//! Only the view (zoom, pan) is the UI's; the graph is the engine's.

use egui::{Color32, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use serde_json::Value;

use crate::i18n::Lang;
use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::widgets;

pub const DEPENDENCIES: &str = "query.dependencies";

/// A node's box, in diagram units.
const NODE_W: f32 = 210.0;
const NODE_H: f32 = 52.0;
const GAP_X: f32 = 70.0;
const GAP_Y: f32 = 22.0;
/// Zoom limits and step.
pub const MIN_ZOOM: f32 = 0.2;
pub const MAX_ZOOM: f32 = 2.5;
const ZOOM_STEP: f32 = 1.25;
/// Most nodes drawn.
const MAX_NODES: usize = 4_000;

/// One node of the graph.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub id: String,
    /// `source`, `query`, `parameter`, `table` (a query loaded into the model) or `model_table`.
    pub kind: String,
    pub name: String,
    pub layer: usize,
    /// Its place in its layer.
    pub row: usize,
}

/// The graph `query.dependencies` answered.
#[derive(Debug, Clone, PartialEq)]
pub struct Graph {
    pub nodes: Vec<Node>,
    /// (from, to) node indexes.
    pub edges: Vec<(usize, usize)>,
}

impl Graph {
    pub fn of(v: &Value) -> Self {
        let s = |x: &Value, k: &str| x.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        let mut rows: Vec<usize> = Vec::new();
        let nodes: Vec<Node> = v
            .get("nodes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .take(MAX_NODES)
            .map(|n| {
                let layer = n
                    .get("layer")
                    .and_then(Value::as_u64)
                    .and_then(|l| usize::try_from(l).ok())
                    .unwrap_or(0)
                    .min(MAX_NODES);
                if rows.len() <= layer {
                    rows.resize(layer.saturating_add(1), 0);
                }
                let row = rows.get(layer).copied().unwrap_or(0);
                if let Some(r) = rows.get_mut(layer) {
                    *r = r.saturating_add(1);
                }
                Node {
                    id: s(n, "id"),
                    kind: s(n, "kind"),
                    name: s(n, "name"),
                    layer,
                    row,
                }
            })
            .collect();
        let at = |id: &str| nodes.iter().position(|n| n.id == id);
        let edges = v
            .get("edges")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|e| Some((at(e.get("from")?.as_str()?)?, at(e.get("to")?.as_str()?)?)))
            .collect();
        Self { nodes, edges }
    }

    /// A node's box in diagram units.
    pub fn rect(n: &Node) -> Rect {
        Rect::from_min_size(
            pos2(n.layer as f32 * (NODE_W + GAP_X), n.row as f32 * (NODE_H + GAP_Y)),
            vec2(NODE_W, NODE_H),
        )
    }

    /// The box around every node (diagram units).
    pub fn bounds(&self) -> Rect {
        self.nodes
            .iter()
            .map(Self::rect)
            .reduce(|a, b| a.union(b))
            .unwrap_or(Rect::from_min_size(pos2(0.0, 0.0), vec2(NODE_W, NODE_H)))
    }
}

/// The open view's state.
#[derive(Debug, Clone, PartialEq)]
pub struct Deps {
    /// None while read.
    pub graph: Option<Result<Graph, String>>,
    pub zoom: f32,
    /// The diagram's origin from the canvas's top-left corner, in points.
    pub pan: egui::Vec2,
    /// Fit the diagram to the canvas at the next frame (on opening and Fit).
    pub fit: bool,
}

impl Deps {
    pub fn new() -> Self {
        Self {
            graph: None,
            zoom: 1.0,
            pan: vec2(16.0, 16.0),
            fit: true,
        }
    }

    pub fn zoom_by(&mut self, k: f32) {
        self.zoom = (self.zoom * k).clamp(MIN_ZOOM, MAX_ZOOM);
        self.fit = false;
    }
}

impl Default for Deps {
    fn default() -> Self {
        Self::new()
    }
}

/// What a click in the view asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepsAct {
    Close,
}

fn kind_word(kind: &str, l: Lang) -> &'static str {
    match kind {
        "source" => l.t("Source", "Source"),
        "parameter" => l.t("Parameter", "Paramètre"),
        "model_table" => l.t("Model table", "Table du modèle"),
        "table" => l.t("Query (loaded)", "Requête (chargée)"),
        _ => l.t("Query", "Requête"),
    }
}

fn kind_icon(kind: &str) -> Icon {
    match kind {
        "source" => Icon::Server,
        "parameter" => Icon::Settings,
        "model_table" => Icon::Model,
        "table" => Icon::Table,
        _ => Icon::List,
    }
}

/// Draws the view; returns what was asked.
pub fn show(ui: &mut Ui, d: &mut Deps, l: Lang, t: &Tokens) -> Option<DepsAct> {
    let mut act = None;
    ui.horizontal(|ui| {
        ui.label(RichText::new(l.t("Query dependencies", "Dépendances de requête")).font(theme::bold(14.0)));
        ui.add_space(12.0);
        let ready = matches!(d.graph, Some(Ok(_)));
        if widgets::small_button(ui, Icon::ZoomOut, l.t("Zoom out", "Zoom arrière"), ready, false).clicked()
            && ready
        {
            d.zoom_by(1.0 / ZOOM_STEP);
        }
        if widgets::small_button(ui, Icon::ZoomIn, l.t("Zoom in", "Zoom avant"), ready, false).clicked()
            && ready
        {
            d.zoom_by(ZOOM_STEP);
        }
        if widgets::small_button(ui, Icon::FitSmall, l.t("Fit", "Ajuster"), ready, false).clicked() && ready {
            d.fit = true;
        }
        ui.label(RichText::new(format!("{:.0} %", d.zoom * 100.0)).color(t.text_dim));
        if widgets::small_button(
            ui,
            Icon::Close,
            l.t("Close dependencies", "Fermer les dépendances"),
            true,
            false,
        )
        .clicked()
        {
            act = Some(DepsAct::Close);
        }
    });
    ui.label(
        RichText::new(l.t(
            "Arrows go from what is read to what reads it: sources, then queries, then the model's tables. Drag to move.",
            "Les flèches vont de ce qui est lu vers ce qui le lit : sources, puis requêtes, puis tables du modèle. Faites glisser pour déplacer.",
        ))
        .color(t.text_dim),
    );
    let g = match &d.graph {
        None => {
            ui.spinner();
            return act;
        }
        Some(Err(e)) => {
            ui.label(RichText::new(format!("⚠ {e}")).color(t.warn));
            return act;
        }
        Some(Ok(g)) => g.clone(),
    };
    let size = vec2(ui.available_width(), (ui.available_height() - 4.0).max(120.0));
    let (canvas, resp) = ui.allocate_exact_size(size, Sense::drag());
    if resp.dragged() {
        d.pan += resp.drag_delta();
        d.fit = false;
    }
    if d.fit {
        let b = g.bounds();
        let k = ((canvas.width() - 32.0) / b.width().max(1.0))
            .min((canvas.height() - 32.0) / b.height().max(1.0))
            .clamp(MIN_ZOOM, 1.5);
        d.zoom = k;
        let drawn = b.size() * k;
        d.pan = (canvas.size() - drawn) * 0.5 - b.min.to_vec2() * k;
        d.fit = false;
    }
    let p = ui.painter_at(canvas);
    p.rect_filled(canvas, 0.0, t.canvas);
    let z = d.zoom;
    let at =
        |r: Rect| -> Rect { Rect::from_min_size(canvas.min + d.pan + r.min.to_vec2() * z, r.size() * z) };
    if g.nodes.is_empty() {
        p.text(
            canvas.center(),
            egui::Align2::CENTER_CENTER,
            l.t("The editor has no query.", "L'éditeur n'a aucune requête."),
            theme::font(13.0),
            t.text_dim,
        );
    }
    // the arrows first, under the boxes
    let ink = t.text_dim;
    for (a, b) in &g.edges {
        let (Some(na), Some(nb)) = (g.nodes.get(*a), g.nodes.get(*b)) else {
            continue;
        };
        let (ra, rb) = (at(Graph::rect(na)), at(Graph::rect(nb)));
        let from = pos2(ra.max.x, ra.center().y);
        let to = pos2(rb.min.x, rb.center().y);
        let mid = (from.x + to.x) * 0.5;
        let curve = egui::epaint::CubicBezierShape::from_points_stroke(
            [from, pos2(mid, from.y), pos2(mid, to.y), to],
            false,
            Color32::TRANSPARENT,
            Stroke::new(1.2, ink),
        );
        p.add(curve);
        let s = (7.0 * z).clamp(3.0, 10.0);
        p.add(egui::Shape::convex_polygon(
            vec![to, pos2(to.x - s, to.y - s * 0.6), pos2(to.x - s, to.y + s * 0.6)],
            ink,
            Stroke::NONE,
        ));
    }
    for (i, n) in g.nodes.iter().enumerate() {
        let r = at(Graph::rect(n));
        if !r.intersects(canvas) {
            continue;
        }
        let loaded = n.kind == "table" || n.kind == "model_table";
        p.rect_filled(r, 4.0 * z, t.card);
        p.rect_stroke(
            r,
            4.0 * z,
            Stroke::new(
                if loaded { 2.0 } else { 1.0 },
                if loaded { t.accent } else { t.border },
            ),
            egui::StrokeKind::Inside,
        );
        let pad = 8.0 * z;
        let icon = (18.0 * z).clamp(6.0, 36.0);
        icons::paint(
            &p,
            Rect::from_min_size(pos2(r.min.x + pad, r.center().y - icon * 0.5), vec2(icon, icon)),
            kind_icon(&n.kind),
            t.text,
            false,
        );
        let x = r.min.x + pad * 2.0 + icon;
        let width = (r.max.x - x - pad).max(4.0);
        let word = kind_word(&n.kind, l);
        let small = (10.0 * z).clamp(5.0, 22.0);
        let big = (12.5 * z).clamp(6.0, 28.0);
        p.text(
            pos2(x, r.min.y + pad * 0.8),
            egui::Align2::LEFT_TOP,
            word,
            theme::font(small),
            t.text_dim,
        );
        let name = fit_text(ui, &n.name, theme::bold(big), width);
        p.text(
            pos2(x, r.max.y - pad * 0.8),
            egui::Align2::LEFT_BOTTOM,
            name,
            theme::bold(big),
            t.text,
        );
        // a node of the accessibility tree: its kind and name
        let label = format!("{word} {}", n.name);
        ui.interact(
            r.intersect(canvas),
            ui.id().with(("bicraft-pq-dep", i)),
            Sense::hover(),
        )
        .on_hover_text(&n.name)
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &label));
    }
    act
}

/// `text` cut with an ellipsis to fit `width`.
fn fit_text(ui: &Ui, text: &str, font: egui::FontId, width: f32) -> String {
    let w = |s: &str| {
        ui.fonts_mut(|f| {
            f.layout_no_wrap(s.to_string(), font.clone(), Color32::WHITE)
                .size()
                .x
        })
    };
    if w(text) <= width {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut n = chars.len();
    while n > 1 {
        n -= 1;
        let s: String = chars.iter().take(n).collect::<String>() + "…";
        if w(&s) <= width {
            return s;
        }
    }
    "…".into()
}
