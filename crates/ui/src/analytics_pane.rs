//! The Visualizations pane's Analytics tab (docs/gaps.md G56, G41), beside Build visual and
//! Format visual as in Power BI Desktop: one collapsible card per kind of analytics the selected
//! visual's type allows (constant lines on each axis, min, max, average, median and percentile
//! lines, trend line, forecast, error bars, a line chart's anomalies, a scatter chart's ratio
//! line and symmetry shading),
//! each listing the visual's items of that kind with their options and a Remove button, and an
//! Add button.
//!
//! Which kinds a type allows is the visual catalog's: a kind is offered when the type has its
//! formatting object (`y1AxisReferenceLine`, `referenceLine`, `error`…), as `visual.analytics`
//! checks. The items are the visual's `analytics` list as `visual.props` reads it (the Format
//! tab's read, kept in [`crate::format_pane::FormatUi::own`]); every edit (Add, an option, Remove)
//! sends the whole list again in one `visual.analytics` command, so undo takes each edit back.
//! Entries BIcraft keeps as written (`{object, raw}`) stay in the list, in a last card where they
//! can only be removed. Readers see no Analytics tab.

use std::collections::BTreeMap;

use bicraft_visuals as visuals;
use egui::{Stroke, Ui, vec2};
use serde_json::{Map, Value, json};

use crate::App;
use crate::app::clip;
use crate::i18n::Lang;
use crate::ribbon::{self, VisualSel};
use crate::theme::Tokens;
use crate::widgets;

/// The commands the Analytics tab runs (the coverage test reads it).
pub const COMMANDS: &[&str] = &["visual.analytics", "visual.props"];

/// Most items listed (the engine takes at most 50 lines).
const MAX_ITEMS: usize = 50;

/// One kind of analytics item: its `kind` (and constant lines' `axis`), the formatting object the
/// visual type must have, its card title, whether a visual has one at most.
#[derive(Debug)]
pub struct KindSpec {
    pub key: &'static str,
    pub kind: &'static str,
    pub axis: Option<&'static str>,
    pub object: &'static str,
    pub en: &'static str,
    pub fr: &'static str,
    pub single: bool,
}

impl KindSpec {
    pub fn title(&self, l: Lang) -> &'static str {
        l.t(self.en, self.fr)
    }

    /// Whether the list entry `e` is an item of this kind.
    fn holds(&self, e: &Value) -> bool {
        e.get("kind").and_then(Value::as_str) == Some(self.kind)
            && (self.kind != "constant"
                || e.get("axis").and_then(Value::as_str).unwrap_or("y") == self.axis.unwrap_or("y"))
    }
}

/// Every kind, in the order Desktop's Analytics pane lists them.
pub const KINDS: &[KindSpec] = &[
    KindSpec {
        key: "constant_x",
        kind: "constant",
        axis: Some("x"),
        object: "xAxisReferenceLine",
        en: "X-axis constant line",
        fr: "Ligne constante de l'axe X",
        single: false,
    },
    KindSpec {
        key: "constant_y",
        kind: "constant",
        axis: Some("y"),
        object: "y1AxisReferenceLine",
        en: "Y-axis constant line",
        fr: "Ligne constante de l'axe Y",
        single: false,
    },
    KindSpec {
        key: "min",
        kind: "min",
        axis: None,
        object: "referenceLine",
        en: "Min line",
        fr: "Ligne minimum",
        single: false,
    },
    KindSpec {
        key: "max",
        kind: "max",
        axis: None,
        object: "referenceLine",
        en: "Max line",
        fr: "Ligne maximum",
        single: false,
    },
    KindSpec {
        key: "average",
        kind: "average",
        axis: None,
        object: "referenceLine",
        en: "Average line",
        fr: "Ligne de moyenne",
        single: false,
    },
    KindSpec {
        key: "median",
        kind: "median",
        axis: None,
        object: "referenceLine",
        en: "Median line",
        fr: "Ligne médiane",
        single: false,
    },
    KindSpec {
        key: "percentile",
        kind: "percentile",
        axis: None,
        object: "referenceLine",
        en: "Percentile line",
        fr: "Ligne de centile",
        single: false,
    },
    KindSpec {
        key: "trend",
        kind: "trend",
        axis: None,
        object: "trend",
        en: "Trend line",
        fr: "Ligne de tendance",
        single: true,
    },
    KindSpec {
        key: "forecast",
        kind: "forecast",
        axis: None,
        object: "forecast",
        en: "Forecast",
        fr: "Prévision",
        single: true,
    },
    KindSpec {
        key: "error_bars",
        kind: "error_bars",
        axis: None,
        object: "error",
        en: "Error bars",
        fr: "Barres d'erreur",
        single: false,
    },
    KindSpec {
        key: "anomaly_detection",
        kind: "anomaly_detection",
        axis: None,
        object: "anomalyDetection",
        en: "Find anomalies",
        fr: "Rechercher les anomalies",
        single: true,
    },
    KindSpec {
        key: "ratio_line",
        kind: "ratio_line",
        axis: None,
        object: "ratioLine",
        en: "Ratio line",
        fr: "Ligne de rapport",
        single: true,
    },
    KindSpec {
        key: "symmetry_shading",
        kind: "symmetry_shading",
        axis: None,
        object: "plotAreaShading",
        en: "Symmetry shading",
        fr: "Ombrage de symétrie",
        single: true,
    },
];

/// The kinds the visual type `vtype` allows (none for a type the catalog does not know).
pub fn kinds_for(vtype: &str) -> Vec<&'static KindSpec> {
    if visuals::visual(vtype).is_none() {
        return Vec::new();
    }
    KINDS
        .iter()
        .filter(|k| visuals::object(vtype, k.object).is_some())
        .collect()
}

/// The tab's view state (pure UI: never sent anywhere).
#[derive(Debug, Default)]
pub struct AnalyticsUi {
    /// The cards expanded (their keys).
    pub open: std::collections::BTreeSet<String>,
    /// Numbers and texts being typed or dragged: control key → (visual, draft).
    pub drafts: BTreeMap<String, (String, String)>,
    /// The new error bars' bounds picked before Add: (visual, upper, lower).
    pub new_bounds: Option<(String, Option<String>, Option<String>)>,
    /// The list last sent, shown until the engine answers (so two quick edits build on each
    /// other): (visual, list).
    pub pending: Option<(String, Vec<Value>)>,
}

/// The visual's analytics list as `visual.props` read it (at most [`MAX_ITEMS`]).
pub fn lines_of(props: Option<&Value>) -> Vec<Value> {
    props
        .and_then(|p| p.get("analytics"))
        .and_then(Value::as_array)
        .map(|a| a.iter().take(MAX_ITEMS).cloned().collect())
        .unwrap_or_default()
}

/// The new item Add appends for `k` (error bars take the bounds picked).
pub fn new_item(k: &KindSpec, upper: Option<&str>, lower: Option<&str>) -> Value {
    let mut o = Map::new();
    o.insert("kind".into(), json!(k.kind));
    match k.kind {
        "constant" => {
            if k.axis == Some("x") {
                o.insert("axis".into(), json!("x"));
            }
            o.insert("value".into(), json!(0));
        }
        "percentile" => {
            o.insert("percentile".into(), json!(50));
        }
        "error_bars" => {
            if let Some(u) = upper {
                o.insert("upper".into(), json!(u));
            }
            if let Some(l) = lower {
                o.insert("lower".into(), json!(l));
            }
        }
        _ => {}
    }
    Value::Object(o)
}

/// `list` with item `i`'s `key` set to `v` (`null` removes the key: the default).
pub fn with_option(list: &[Value], i: usize, key: &str, v: Value) -> Vec<Value> {
    let mut out = list.to_vec();
    if let Some(Value::Object(o)) = out.get_mut(i) {
        if v.is_null() {
            o.shift_remove(key);
        } else {
            o.insert(key.to_string(), v);
        }
    }
    out
}

/// `list` without item `i`.
pub fn without(list: &[Value], i: usize) -> Vec<Value> {
    list.iter()
        .enumerate()
        .filter(|(j, _)| *j != i)
        .map(|(_, e)| e.clone())
        .collect()
}

// ----- the options -----

#[derive(Debug, Clone, Copy)]
enum Ctl {
    /// A number from lo to hi, its default shown, decimals.
    Num(f64, f64, f64, usize),
    Text,
    /// A switch and its default.
    Bool(bool),
    Color,
    /// One of (value, en, fr).
    Choice(&'static [(&'static str, &'static str, &'static str)]),
    /// A measure of the visual's series.
    Series,
    /// A measure of the model (or none).
    Measure,
}

/// One option of an item: its key, name, control.
type Opt = (&'static str, &'static str, &'static str, Ctl);

const STYLES: &[(&str, &str, &str)] = &[
    ("solid", "Solid", "Continu"),
    ("dashed", "Dashed", "Tirets"),
    ("dotted", "Dotted", "Pointillés"),
];
const POSITIONS: &[(&str, &str, &str)] = &[("front", "In front", "Devant"), ("back", "Behind", "Derrière")];
const SHAPES: &[(&str, &str, &str)] = &[
    ("circle", "Circle", "Cercle"),
    ("square", "Square", "Carré"),
    ("diamond", "Diamond", "Losange"),
    ("triangle", "Triangle", "Triangle"),
    ("x", "Cross", "Croix"),
    ("shortDash", "Short dash", "Tiret court"),
    ("longDash", "Long dash", "Tiret long"),
    ("plus", "Plus", "Plus"),
    ("none", "None", "Aucune"),
];
const BAND_STYLES: &[(&str, &str, &str)] = &[
    ("fill", "Fill", "Remplissage"),
    ("line", "Line", "Ligne"),
    ("fillLine", "Fill and line", "Remplissage et ligne"),
];

const NAME: Opt = ("label", "Name", "Nom", Ctl::Text);
const COLOR: Opt = ("color", "Color", "Couleur", Ctl::Color);
const TRANSPARENCY: Opt = (
    "transparency",
    "Transparency (%)",
    "Transparence (%)",
    Ctl::Num(0.0, 100.0, 0.0, 0),
);
const STYLE: Opt = ("style", "Line style", "Style de trait", Ctl::Choice(STYLES));
const WIDTH: Opt = ("width", "Width", "Largeur", Ctl::Num(0.0, 100.0, 1.0, 1));
const POSITION: Opt = ("position", "Position", "Position", Ctl::Choice(POSITIONS));
const DATA_LABEL: Opt = (
    "data_label",
    "Data label",
    "Étiquette de données",
    Ctl::Bool(false),
);
const SERIES: Opt = ("measure", "Series", "Série", Ctl::Series);

const CONSTANT_OPTS: &[Opt] = &[
    ("value", "Value", "Valeur", Ctl::Num(-1.0e15, 1.0e15, 0.0, 2)),
    NAME,
    COLOR,
    TRANSPARENCY,
    STYLE,
    WIDTH,
    POSITION,
    DATA_LABEL,
];
const STAT_OPTS: &[Opt] = &[
    SERIES,
    NAME,
    COLOR,
    TRANSPARENCY,
    STYLE,
    WIDTH,
    POSITION,
    DATA_LABEL,
];
const PERCENTILE_OPTS: &[Opt] = &[
    SERIES,
    (
        "percentile",
        "Percentile",
        "Centile",
        Ctl::Num(0.0, 100.0, 50.0, 0),
    ),
    NAME,
    COLOR,
    TRANSPARENCY,
    STYLE,
    WIDTH,
    POSITION,
    DATA_LABEL,
];
const TREND_OPTS: &[Opt] = &[NAME, COLOR, TRANSPARENCY, STYLE, WIDTH];
const FORECAST_OPTS: &[Opt] = &[
    SERIES,
    (
        "length",
        "Forecast length (points)",
        "Durée de la prévision (points)",
        Ctl::Num(0.0, 100_000.0, 10.0, 0),
    ),
    (
        "ignore_last",
        "Ignore the last (points)",
        "Ignorer les derniers (points)",
        Ctl::Num(0.0, 100_000.0, 0.0, 0),
    ),
    (
        "confidence",
        "Confidence interval",
        "Intervalle de confiance",
        Ctl::Num(0.01, 0.999, 0.95, 3),
    ),
    NAME,
    COLOR,
    STYLE,
    WIDTH,
];
const ERROR_OPTS: &[Opt] = &[
    SERIES,
    ("upper", "Upper bound", "Borne supérieure", Ctl::Measure),
    ("lower", "Lower bound", "Borne inférieure", Ctl::Measure),
    (
        "relative",
        "Bounds relative to the series",
        "Bornes relatives à la série",
        Ctl::Bool(false),
    ),
    ("bar", "Bar", "Barre", Ctl::Bool(true)),
    ("color", "Bar color", "Couleur de la barre", Ctl::Color),
    (
        "width",
        "Bar width",
        "Largeur de la barre",
        Ctl::Num(0.0, 100.0, 2.0, 1),
    ),
    ("marker", "Markers", "Marqueurs", Ctl::Bool(true)),
    (
        "marker_shape",
        "Marker shape",
        "Forme des marqueurs",
        Ctl::Choice(SHAPES),
    ),
    ("band", "Band", "Bande", Ctl::Bool(false)),
    (
        "band_style",
        "Band style",
        "Style de bande",
        Ctl::Choice(BAND_STYLES),
    ),
    ("band_color", "Band color", "Couleur de la bande", Ctl::Color),
    (
        "transparency",
        "Band transparency (%)",
        "Transparence de la bande (%)",
        Ctl::Num(0.0, 100.0, 70.0, 0),
    ),
    DATA_LABEL,
];
const ANOMALY_SHAPES: &[(&str, &str, &str)] = &[
    ("circle", "Circle", "Cercle"),
    ("square", "Square", "Carré"),
    ("diamond", "Diamond", "Losange"),
    ("triangle", "Triangle", "Triangle"),
    ("droplet", "Droplet", "Goutte"),
];
const RANGE_STYLES: &[(&str, &str, &str)] = &[
    ("fill", "Fill", "Remplissage"),
    ("line", "Line", "Ligne"),
    ("none", "None", "Aucun"),
];
const ANOMALY_OPTS: &[Opt] = &[
    SERIES,
    NAME,
    (
        "sensitivity",
        "Sensitivity (%)",
        "Sensibilité (%)",
        Ctl::Num(0.0, 100.0, 70.0, 0),
    ),
    (
        "marker",
        "Anomaly markers",
        "Marqueurs d'anomalie",
        Ctl::Bool(true),
    ),
    (
        "marker_shape",
        "Marker shape",
        "Forme des marqueurs",
        Ctl::Choice(ANOMALY_SHAPES),
    ),
    (
        "marker_size",
        "Marker size",
        "Taille des marqueurs",
        Ctl::Num(1.0, 50.0, 5.0, 0),
    ),
    (
        "marker_color",
        "Marker color",
        "Couleur des marqueurs",
        Ctl::Color,
    ),
    (
        "marker_transparency",
        "Marker transparency (%)",
        "Transparence des marqueurs (%)",
        Ctl::Num(0.0, 100.0, 0.0, 0),
    ),
    ("band", "Expected range", "Plage attendue", Ctl::Bool(true)),
    (
        "band_style",
        "Range style",
        "Style de la plage",
        Ctl::Choice(RANGE_STYLES),
    ),
    ("band_color", "Range color", "Couleur de la plage", Ctl::Color),
    (
        "transparency",
        "Range transparency (%)",
        "Transparence de la plage (%)",
        Ctl::Num(0.0, 100.0, 80.0, 0),
    ),
];
const SHADING_OPTS: &[Opt] = &[
    NAME,
    ("upper_color", "Upper shading", "Ombrage supérieur", Ctl::Color),
    ("lower_color", "Lower shading", "Ombrage inférieur", Ctl::Color),
    (
        "transparency",
        "Transparency (%)",
        "Transparence (%)",
        Ctl::Num(0.0, 100.0, 70.0, 0),
    ),
];

/// The options of an item of `kind` on a visual of type `vtype`.
fn options(kind: &str, vtype: &str) -> Vec<Opt> {
    let list: &[Opt] = match kind {
        "constant" => CONSTANT_OPTS,
        "average" | "min" | "max" | "median" => STAT_OPTS,
        "percentile" => PERCENTILE_OPTS,
        "trend" | "ratio_line" => TREND_OPTS,
        "forecast" => FORECAST_OPTS,
        "error_bars" => ERROR_OPTS,
        "anomaly_detection" => ANOMALY_OPTS,
        "symmetry_shading" => SHADING_OPTS,
        _ => &[],
    };
    // the band of error bars exists on line charts only (the catalog's `error.shadeShow`)
    let band = visuals::property(vtype, "error", "shadeShow").is_some();
    list.iter()
        .filter(|(k, ..)| {
            kind != "error_bars"
                || band
                || !matches!(*k, "band" | "band_style" | "band_color" | "transparency")
        })
        .copied()
        .collect()
}

// ----- the pane -----

/// What one frame of the tab works with.
struct Cx<'a> {
    l: Lang,
    visual: &'a str,
    vtype: &'a str,
    list: Vec<Value>,
    /// The visual's series measures (`Y` / `Values` measures and aggregations).
    series: Vec<String>,
    /// The model's measures, `[Name]`.
    measures: Vec<String>,
    swatches: &'a [String],
    drafts: &'a mut BTreeMap<String, (String, String)>,
    enabled: Result<(), String>,
    /// The new list to send this frame.
    send: Option<Vec<Value>>,
}

/// The Analytics tab for the selected visual `sel` (its snapshot entry `v`).
pub fn pane(app: &mut App, ui: &mut Ui, sel: &VisualSel, v: &Value) {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    let Some(snap) = app.loaded.as_ref().map(|x| x.snap.clone()) else {
        return;
    };
    let revision = snap.get("revision").and_then(Value::as_u64).unwrap_or(0);
    crate::format_pane::ask_props(app, sel, revision);
    let vtype = v.get("type").and_then(Value::as_str).unwrap_or("").to_string();
    let kinds = kinds_for(&vtype);
    // the list: what was sent and not answered yet, else what `visual.props` read
    let own = app
        .panes
        .format
        .own
        .as_ref()
        .filter(|o| o.page == sel.page && o.visual == sel.visual)
        .map(|o| o.props.clone());
    let pending = app
        .panes
        .format
        .analytics
        .pending
        .as_ref()
        .filter(|(vis, _)| *vis == sel.visual)
        .map(|(_, list)| list.clone());
    let read = own.is_some();
    let list = pending.unwrap_or_else(|| lines_of(own.as_ref()));
    let series: Vec<String> = v
        .get("fields")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(200)
        .filter(|f| {
            let role = f.get("role").and_then(Value::as_str).unwrap_or("");
            let kind = f.get("kind").and_then(Value::as_str).unwrap_or("");
            matches!(role, "Y" | "Y2" | "Values" | "X") && matches!(kind, "measure" | "aggregation")
        })
        .filter_map(|f| f.get("text").and_then(Value::as_str).map(str::to_string))
        .collect();
    let mut measures: Vec<String> = Vec::new();
    for tb in snap
        .pointer("/model/tables")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(1000)
    {
        for m in tb
            .get("measures")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .take(1000)
        {
            if let Some(n) = m.get("name").and_then(Value::as_str) {
                let text = format!("[{}]", n.replace(']', "]]"));
                if !measures.contains(&text) && measures.len() < 500 {
                    measures.push(text);
                }
            }
        }
    }
    let palette: Vec<String> = bicraft_render::data_colors(&snap);
    let mut swatches: Vec<String> = palette.iter().take(10).cloned().collect();
    for n in ["#FFFFFF", "#F3F2F1", "#C8C6C4", "#605E5C", "#252423", "#000000"] {
        if !swatches.iter().any(|s| s.eq_ignore_ascii_case(n)) {
            swatches.push(n.to_string());
        }
    }
    let enabled = ribbon::state(app, "visual.analytics");
    let mut open = std::mem::take(&mut app.panes.format.analytics.open);
    let mut drafts = std::mem::take(&mut app.panes.format.analytics.drafts);
    let mut new_bounds = app
        .panes
        .format
        .analytics
        .new_bounds
        .take()
        .filter(|(vis, ..)| *vis == sel.visual)
        .unwrap_or_else(|| (sel.visual.clone(), None, None));
    let mut cx = Cx {
        l,
        visual: &sel.visual,
        vtype: &vtype,
        list,
        series,
        measures,
        swatches: &swatches,
        drafts: &mut drafts,
        enabled,
        send: None,
    };
    egui::ScrollArea::vertical()
        .id_salt("bicraft-analytics-cards")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if kinds.is_empty() {
                ui.label(
                    egui::RichText::new(l.t(
                        "This visual type has no analytics: Power BI adds reference lines, trend \
                         lines, forecasts and error bars to charts.",
                        "Ce type de visuel n'a pas d'analyse : Power BI ajoute des lignes de \
                         référence, des courbes de tendance, des prévisions et des barres d'erreur \
                         aux graphiques.",
                    ))
                    .color(t.text_dim),
                );
                return;
            }
            if !read {
                ui.label(
                    egui::RichText::new(l.t("Reading the visual…", "Lecture du visuel…"))
                        .small()
                        .color(t.text_dim),
                );
            }
            for k in &kinds {
                let is_open = open.contains(k.key);
                if card_ui(ui, &mut cx, k, is_open, &mut new_bounds) {
                    if is_open {
                        open.remove(k.key);
                    } else {
                        open.insert(k.key.to_string());
                    }
                }
            }
            // entries kept as written
            let raw: Vec<usize> = (0..cx.list.len())
                .filter(|i| cx.list.get(*i).is_some_and(|e| e.get("raw").is_some()))
                .collect();
            if !raw.is_empty() {
                raw_card(ui, &mut cx, &raw);
            }
        });
    let send = cx.send.take();
    let f = &mut app.panes.format.analytics;
    f.open = open;
    f.drafts = drafts;
    f.new_bounds = Some(new_bounds);
    if let Some(list) = send {
        let sent = crate::format_pane::send_tracked(app, sel, "visual.analytics", json!({"lines": list}));
        if sent {
            app.panes.format.analytics.pending = Some((sel.visual.clone(), list));
        }
    }
}

/// A card's frame, header (chevron, title, count) and, open, its items and Add. True when the
/// header was clicked.
fn card_ui(
    ui: &mut Ui,
    cx: &mut Cx<'_>,
    k: &KindSpec,
    open: bool,
    new_bounds: &mut (String, Option<String>, Option<String>),
) -> bool {
    let t = Tokens::get(ui.ctx());
    let l = cx.l;
    let items: Vec<usize> = (0..cx.list.len())
        .filter(|i| cx.list.get(*i).is_some_and(|e| k.holds(e)))
        .collect();
    let title = k.title(l);
    let shown = if items.is_empty() {
        title.to_string()
    } else {
        format!("{title} ({})", items.len())
    };
    let mut toggled = false;
    egui::Frame::NONE
        .stroke(Stroke::new(1.0, t.card_border))
        .corner_radius(egui::CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(6, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            // the header is labelled by the card's title (the count is shown, not named)
            let before = open;
            let r = ui
                .allocate_ui(vec2(ui.available_width(), 26.0), |ui| {
                    widgets::section(ui, &shown, open)
                })
                .inner;
            toggled = r != before;
            if !open {
                return;
            }
            if k.kind == "anomaly_detection" {
                ui.label(
                    egui::RichText::new(l.t(
                        "BIcraft estimate: the expected range and the anomalies are computed by \
                         BIcraft, not by Power BI's algorithm. Line charts with one value and no \
                         legend; no forecast or min, max, average, median and percentile lines.",
                        "Estimation BIcraft : la plage attendue et les anomalies sont calculées par \
                         BIcraft, pas par l'algorithme de Power BI. Graphiques en courbes à une \
                         seule valeur, sans légende ; pas de prévision ni de lignes minimum, \
                         maximum, moyenne, médiane ou centile.",
                    ))
                    .small()
                    .color(t.text_dim),
                );
            }
            for (n, i) in items.iter().enumerate() {
                item_ui(ui, cx, k, *i, n + 1);
            }
            if k.single && !items.is_empty() {
                return;
            }
            ui.add_space(2.0);
            add_ui(ui, cx, k, new_bounds);
        });
    ui.add_space(4.0);
    toggled
}

/// The Add row of card `k` (error bars: their bounds first).
fn add_ui(
    ui: &mut Ui,
    cx: &mut Cx<'_>,
    k: &KindSpec,
    new_bounds: &mut (String, Option<String>, Option<String>),
) {
    let l = cx.l;
    let title = k.title(l);
    let mut ready = true;
    if k.kind == "error_bars" {
        for (en, fr, upper) in [
            (
                "New error bars' upper bound",
                "Borne supérieure des nouvelles barres",
                true,
            ),
            (
                "New error bars' lower bound",
                "Borne inférieure des nouvelles barres",
                false,
            ),
        ] {
            let cur = if upper {
                new_bounds.1.clone()
            } else {
                new_bounds.2.clone()
            };
            let name = l.t(en, fr);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(l.t(
                        if upper { "Upper bound" } else { "Lower bound" },
                        if upper {
                            "Borne supérieure"
                        } else {
                            "Borne inférieure"
                        },
                    ))
                    .small(),
                );
                if let Some(p) = measure_combo(ui, &cx.measures, cur.as_deref(), name) {
                    if upper {
                        new_bounds.1 = p;
                    } else {
                        new_bounds.2 = p;
                    }
                }
            });
        }
        ready = new_bounds.1.is_some() || new_bounds.2.is_some();
    }
    let ok = cx.enabled.is_ok() && ready;
    let label = format!("{} {title}", l.t("Add", "Ajouter"));
    let r = ui.add_enabled(ok, egui::Button::new(format!("+ {}", l.t("Add", "Ajouter"))));
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ok, &label));
    let r = match (&cx.enabled, ready) {
        (Err(why), _) => r.on_disabled_hover_text(format!("{} {why}", l.t("Unavailable:", "Indisponible :"))),
        (_, false) => r.on_disabled_hover_text(l.t(
            "Pick an upper or a lower bound first.",
            "Choisissez d'abord une borne supérieure ou inférieure.",
        )),
        _ => r.on_hover_text(label.clone()),
    };
    if r.clicked() {
        let mut list = cx.list.clone();
        list.push(new_item(k, new_bounds.1.as_deref(), new_bounds.2.as_deref()));
        if k.kind == "error_bars" {
            new_bounds.1 = None;
            new_bounds.2 = None;
        }
        cx.send = Some(list);
    }
}

/// One item of card `k` (the list's entry `i`, the card's `n`-th): its options, Remove.
fn item_ui(ui: &mut Ui, cx: &mut Cx<'_>, k: &KindSpec, i: usize, n: usize) {
    let t = Tokens::get(ui.ctx());
    let l = cx.l;
    let Some(item) = cx.list.get(i).cloned() else {
        return;
    };
    let head = if k.single {
        k.title(l).to_string()
    } else {
        format!("{} {n}", k.title(l))
    };
    ui.add_space(2.0);
    ui.separator();
    ui.horizontal(|ui| {
        let shown = item
            .get("label")
            .and_then(Value::as_str)
            .map_or_else(|| head.clone(), |s| clip(s, 40));
        ui.label(egui::RichText::new(shown).strong().color(t.text));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let name = format!("{} {head}", l.t("Remove", "Supprimer"));
            if remove_button(ui, l, &name, cx.enabled.is_ok()) {
                cx.send = Some(without(&cx.list, i));
            }
        });
    });
    // On/off: `show: false` hides the item (the switch writes it; on is the default)
    let on = item.get("show").and_then(Value::as_bool).unwrap_or(true);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(l.t("Show", "Afficher")).small());
        let name = format!("{head} {}", l.t("Show", "Afficher"));
        let r = widgets::switch(ui, on, cx.enabled.is_ok(), &name);
        if r.clicked() && cx.enabled.is_ok() {
            cx.send = Some(with_option(
                &cx.list,
                i,
                "show",
                if on { json!(false) } else { Value::Null },
            ));
        }
    });
    for opt in options(k.kind, cx.vtype) {
        option_ui(ui, cx, i, &head, &item, opt);
    }
}

/// One option row: its name, then its control.
fn option_ui(ui: &mut Ui, cx: &mut Cx<'_>, i: usize, head: &str, item: &Value, (key, en, fr, ctl): Opt) {
    let l = cx.l;
    let name = format!("{head} {}", l.t(en, fr));
    let cur = item.get(key).cloned().unwrap_or(Value::Null);
    let ok = cx.enabled.is_ok();
    let ckey = format!("{i}.{key}");
    let mut set: Option<Value> = None;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(l.t(en, fr)).small());
        ui.add_enabled_ui(ok, |ui| match ctl {
            Ctl::Bool(def) => {
                let on = cur.as_bool().unwrap_or(def);
                if widgets::switch(ui, on, ok, &name).clicked() {
                    // back to the default removes the key
                    set = Some(if !on == def { Value::Null } else { json!(!on) });
                }
            }
            Ctl::Color => {
                if let Some(c) = crate::format_pane::pick_color(
                    ui,
                    l,
                    cx.swatches,
                    cx.drafts,
                    cx.visual,
                    &format!("analytics.{ckey}"),
                    l.t("Color", "Couleur"),
                    &cur,
                    &name,
                ) {
                    set = Some(json!(c));
                }
            }
            Ctl::Choice(values) => {
                let now = cur.as_str().unwrap_or("");
                let shown = values
                    .iter()
                    .find(|(v, ..)| *v == now)
                    .map_or("—", |(_, en, fr)| l.t(en, fr));
                let r = egui::ComboBox::from_id_salt(("bicraft-analytics-choice", &ckey))
                    .selected_text(shown)
                    .width(120.0)
                    .show_ui(ui, |ui| {
                        for (v, en, fr) in values {
                            if ui.selectable_label(*v == now, l.t(en, fr)).clicked() {
                                set = Some(json!(v));
                            }
                        }
                    });
                r.response
                    .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, ok, &name));
            }
            Ctl::Series => {
                let mut list = cx.series.clone();
                if let Some(s) = cur.as_str()
                    && !list.iter().any(|x| x == s)
                {
                    list.insert(0, s.to_string());
                }
                if let Some(p) = measure_combo(ui, &list, cur.as_str(), &name) {
                    set = Some(p.map_or(Value::Null, Value::String));
                }
            }
            Ctl::Measure => {
                if let Some(p) = measure_combo(ui, &cx.measures, cur.as_str(), &name) {
                    set = Some(p.map_or(Value::Null, Value::String));
                }
            }
            Ctl::Text => {
                let now = cur.as_str().unwrap_or("").to_string();
                let mut text = cx
                    .drafts
                    .get(&ckey)
                    .filter(|d| d.0 == cx.visual)
                    .map_or(now.clone(), |d| d.1.clone());
                let r = ui.add(
                    egui::TextEdit::singleline(&mut text)
                        .char_limit(200)
                        .desired_width(140.0),
                );
                r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, ok, &name));
                if r.lost_focus() {
                    cx.drafts.remove(&ckey);
                    if text != now && !ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        set = Some(if text.is_empty() { Value::Null } else { json!(text) });
                    }
                } else if r.has_focus() {
                    cx.drafts.insert(ckey.clone(), (cx.visual.to_string(), text));
                }
            }
            Ctl::Num(lo, hi, def, decimals) => {
                let now = cur.as_f64().unwrap_or(def).clamp(lo, hi);
                let mut x = cx
                    .drafts
                    .get(&ckey)
                    .filter(|d| d.0 == cx.visual)
                    .and_then(|d| d.1.parse::<f64>().ok())
                    .unwrap_or(now);
                let speed = if hi - lo <= 1.0 { 0.005 } else { 0.5 };
                let r = ui.add(
                    egui::DragValue::new(&mut x)
                        .range(lo..=hi)
                        .speed(speed)
                        .max_decimals(decimals),
                );
                r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Slider, ok, &name));
                let busy = r.dragged() || r.has_focus();
                let done = r.drag_stopped() || r.lost_focus() || (r.changed() && !busy);
                if done {
                    cx.drafts.remove(&ckey);
                    if (x - now).abs() > f64::EPSILON {
                        set = Some(json!(x));
                    }
                } else if busy {
                    cx.drafts
                        .insert(ckey.clone(), (cx.visual.to_string(), x.to_string()));
                } else {
                    cx.drafts.remove(&ckey);
                }
            }
        });
    });
    if let Some(v) = set {
        cx.send = Some(with_option(&cx.list, i, key, v));
    }
}

/// A Remove button (the bin icon and the word), labelled `name` for assistive technologies: true
/// when clicked.
fn remove_button(ui: &mut Ui, l: Lang, name: &str, ok: bool) -> bool {
    let r = widgets::small_button(
        ui,
        crate::icons::Icon::Delete,
        l.t("Remove", "Supprimer"),
        ok,
        false,
    );
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ok, name));
    r.on_hover_text(name).clicked() && ok
}

/// A list of measures to pick from (or none): the pick, when one was made (`Some(None)`: none).
fn measure_combo(ui: &mut Ui, list: &[String], cur: Option<&str>, name: &str) -> Option<Option<String>> {
    let mut picked = None;
    let r = egui::ComboBox::from_id_salt(("bicraft-analytics-measure", name))
        .selected_text(cur.map_or_else(|| "—".to_string(), |s| clip(s, 28)))
        .width(150.0)
        .height(300.0)
        .show_ui(ui, |ui| {
            if ui.selectable_label(cur.is_none(), "—").clicked() {
                picked = Some(None);
            }
            for m in list {
                if ui
                    .selectable_label(cur == Some(m.as_str()), clip(m, 40))
                    .clicked()
                {
                    picked = Some(Some(m.clone()));
                }
            }
        });
    r.response
        .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, name));
    picked
}

/// The last card: entries BIcraft keeps as written (their object named), each with Remove.
fn raw_card(ui: &mut Ui, cx: &mut Cx<'_>, raw: &[usize]) {
    let t = Tokens::get(ui.ctx());
    let l = cx.l;
    egui::Frame::NONE
        .stroke(Stroke::new(1.0, t.card_border))
        .corner_radius(egui::CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(6, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new(l.t("Kept as written", "Conservés tels quels")).strong());
            ui.label(
                egui::RichText::new(l.t(
                    "Settings BIcraft does not edit here; they are saved unchanged.",
                    "Réglages que BIcraft ne modifie pas ici ; ils sont enregistrés sans changement.",
                ))
                .small()
                .color(t.text_dim),
            );
            for (n, i) in raw.iter().enumerate() {
                let object = cx
                    .list
                    .get(*i)
                    .and_then(|e| e.get("object"))
                    .and_then(Value::as_str)
                    .unwrap_or("?")
                    .to_string();
                ui.horizontal(|ui| {
                    ui.label(format!("{} {}", n + 1, clip(&object, 40)));
                    let name = format!("{} {object} {}", l.t("Remove", "Supprimer"), n + 1);
                    if remove_button(ui, l, &name, cx.enabled.is_ok()) {
                        cx.send = Some(without(&cx.list, *i));
                    }
                });
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_follow_the_catalog() {
        let col: Vec<&str> = kinds_for("clusteredColumnChart").iter().map(|k| k.key).collect();
        assert!(
            col.contains(&"constant_y") && col.contains(&"average") && col.contains(&"error_bars"),
            "{col:?}"
        );
        assert!(
            !col.contains(&"ratio_line") && !col.contains(&"forecast"),
            "{col:?}"
        );
        let line: Vec<&str> = kinds_for("lineChart").iter().map(|k| k.key).collect();
        assert!(
            line.contains(&"forecast") && line.contains(&"error_bars"),
            "{line:?}"
        );
        let sc: Vec<&str> = kinds_for("scatterChart").iter().map(|k| k.key).collect();
        assert!(
            sc.contains(&"ratio_line") && sc.contains(&"symmetry_shading"),
            "{sc:?}"
        );
        assert!(!sc.contains(&"error_bars"), "{sc:?}");
        assert!(kinds_for("card").is_empty());
        assert!(kinds_for("no such visual").is_empty());
        // the band is a line chart's only
        assert!(options("error_bars", "lineChart").iter().any(|o| o.0 == "band"));
        assert!(
            !options("error_bars", "clusteredColumnChart")
                .iter()
                .any(|o| o.0 == "band")
        );
    }

    #[test]
    fn list_edits_keep_the_other_items() {
        let list = vec![json!({"kind": "trend"}), json!({"kind": "constant", "value": 3})];
        let k = KINDS.iter().find(|k| k.key == "error_bars").unwrap();
        assert_eq!(
            new_item(k, Some("[Up]"), None),
            json!({"kind": "error_bars", "upper": "[Up]"})
        );
        assert_eq!(
            with_option(&list, 1, "color", json!("#FF0000"))[1],
            json!({"kind": "constant", "value": 3, "color": "#FF0000"})
        );
        assert_eq!(
            with_option(&list, 1, "value", Value::Null)[1],
            json!({"kind": "constant"})
        );
        assert_eq!(with_option(&list, 9, "value", json!(1)), list);
        assert_eq!(without(&list, 0), vec![json!({"kind": "constant", "value": 3})]);
        assert_eq!(without(&list, 9), list);
        assert!(lines_of(Some(&json!({"analytics": "x"}))).is_empty());
    }
}
