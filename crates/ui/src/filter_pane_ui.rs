//! The Filters pane's formatting (ROADMAP M11.13, docs/gaps.md G41): the Format tab's **Filter
//! pane** and **Filter cards** cards, shown as Desktop shows them when nothing is selected on the
//! page (the page's own formatting), and the style the Filters pane draws with.
//!
//! - **Filter pane**: background colour and transparency, the border between the report and the
//!   pane, the font, the font and icon colour, the title and header text sizes, the search box's
//!   text size and input box colour, the checkbox and Apply colour.
//! - **Filter cards**: "Apply settings to" Available (cards without a condition) or Applied (cards
//!   with one), then the background colour and transparency, the border, the font, the font and
//!   icon colour, the text size and the input box colour.
//!
//! Each edit is one `page.format_filter_pane` command (a swatch, a switch or a pick at once, a
//! number when let go); "Reset to default" removes the card's properties. Values shown are the
//! snapshot's `filter_pane_format` (the page's own, then the theme's `outspacePane` and
//! `filterCard`); a property neither sets is drawn with the app's own tokens. [`PaneStyle`] is
//! what [`crate::filters_pane`] draws with, for an author and a reader alike.

use std::collections::{BTreeMap, BTreeSet};

use egui::{Color32, FontFamily, FontId, Ui};
use serde_json::{Map, Value, json};

use crate::App;
use crate::actions::Call;
use crate::app::{Then, clip};
use crate::i18n::Lang;
use crate::ribbon;
use crate::theme::{self, Tokens};
use crate::widgets;

/// The command the cards run (the coverage test reads it).
pub const COMMANDS: &[&str] = &["page.format_filter_pane"];
/// The command id.
const CMD: &str = "page.format_filter_pane";

/// The cards' view state (pure UI: never sent anywhere).
#[derive(Debug, Default)]
pub struct PageFormatUi {
    /// The cards expanded (`pane`, `cards`).
    pub open: BTreeSet<String>,
    /// Filter cards: the Applied state is formatted (else Available).
    pub applied: bool,
    /// Numbers being dragged and colour hex drafts: key → (page, draft).
    pub drafts: BTreeMap<String, (String, String)>,
}

/// A property's control.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Ctl {
    Color,
    Percent,
    Size,
    Switch,
    Font,
}

/// One row: the parameter, its control, its English and French names.
type Row = (&'static str, Ctl, &'static str, &'static str);

const PANE_ROWS: &[Row] = &[
    (
        "background_color",
        Ctl::Color,
        "Background colour",
        "Couleur d'arrière-plan",
    ),
    ("transparency", Ctl::Percent, "Transparency", "Transparence"),
    ("border", Ctl::Switch, "Border", "Bordure"),
    ("border_color", Ctl::Color, "Border colour", "Couleur de bordure"),
    ("font_family", Ctl::Font, "Font", "Police"),
    (
        "foreground_color",
        Ctl::Color,
        "Font and icon colour",
        "Couleur de police et d'icône",
    ),
    (
        "title_size",
        Ctl::Size,
        "Title text size",
        "Taille du texte du titre",
    ),
    (
        "header_size",
        Ctl::Size,
        "Header text size",
        "Taille du texte de l'en-tête",
    ),
    (
        "search_text_size",
        Ctl::Size,
        "Search text size",
        "Taille du texte de recherche",
    ),
    (
        "input_box_color",
        Ctl::Color,
        "Input box colour",
        "Couleur de la zone de saisie",
    ),
    (
        "checkbox_color",
        Ctl::Color,
        "Checkbox and Apply colour",
        "Couleur des cases et d'Appliquer",
    ),
];

const CARD_ROWS: &[Row] = &[
    (
        "background_color",
        Ctl::Color,
        "Background colour",
        "Couleur d'arrière-plan",
    ),
    ("transparency", Ctl::Percent, "Transparency", "Transparence"),
    ("border", Ctl::Switch, "Border", "Bordure"),
    ("border_color", Ctl::Color, "Border colour", "Couleur de bordure"),
    ("font_family", Ctl::Font, "Font", "Police"),
    (
        "foreground_color",
        Ctl::Color,
        "Font and icon colour",
        "Couleur de police et d'icône",
    ),
    ("text_size", Ctl::Size, "Text size", "Taille du texte"),
    (
        "input_box_color",
        Ctl::Color,
        "Input box colour",
        "Couleur de la zone de saisie",
    ),
];

// ----- the style the Filters pane draws with -----

/// One target's resolved style (each `None` where neither the page nor the theme sets it).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Style {
    pub background: Option<Color32>,
    pub foreground: Option<Color32>,
    /// `Some(false)`: no border; `None`: the app's.
    pub border: Option<bool>,
    pub border_color: Option<Color32>,
    pub input_box: Option<Color32>,
    pub checkbox: Option<Color32>,
    pub family: Option<String>,
    pub title_size: Option<f32>,
    pub header_size: Option<f32>,
    pub search_size: Option<f32>,
    pub text_size: Option<f32>,
}

/// The Filters pane's style: the pane's, the available cards' and the applied cards'.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PaneStyle {
    pub pane: Style,
    pub available: Style,
    pub applied: Style,
}

fn hex(v: Option<&Value>) -> Option<Color32> {
    v.and_then(Value::as_str).and_then(crate::form::color)
}

fn size(v: Option<&Value>) -> Option<f32> {
    v.and_then(Value::as_f64)
        .filter(|x| x.is_finite())
        .map(|x| x.clamp(6.0, 72.0) as f32)
}

/// A target of the snapshot's `filter_pane_format` as a [`Style`] (transparency applied to the
/// background).
pub fn style_of(v: Option<&Value>) -> Style {
    let Some(v) = v.filter(|v| v.is_object()) else {
        return Style::default();
    };
    let alpha = v
        .get("transparency")
        .and_then(Value::as_f64)
        .filter(|x| x.is_finite())
        .map_or(1.0, |t| 1.0 - (t.clamp(0.0, 100.0) / 100.0));
    Style {
        background: hex(v.get("background_color")).map(|c| c.gamma_multiply(alpha as f32)),
        foreground: hex(v.get("foreground_color")),
        border: v.get("border").and_then(Value::as_bool),
        border_color: hex(v.get("border_color")),
        input_box: hex(v.get("input_box_color")),
        checkbox: hex(v.get("checkbox_color")),
        family: v.get("font_family").and_then(Value::as_str).map(|s| clip(s, 60)),
        title_size: size(v.get("title_size")),
        header_size: size(v.get("header_size")),
        search_size: size(v.get("search_text_size")),
        text_size: size(v.get("text_size")),
    }
}

/// The style of a snapshot page.
pub fn pane_style(page: &Value) -> PaneStyle {
    let f = page.get("filter_pane_format");
    PaneStyle {
        pane: style_of(f.and_then(|f| f.get("pane"))),
        available: style_of(f.and_then(|f| f.get("available"))),
        applied: style_of(f.and_then(|f| f.get("applied"))),
    }
}

/// The active page's style, from the snapshot.
pub fn active_style(app: &App) -> PaneStyle {
    app.loaded
        .as_ref()
        .and_then(|l| l.snap.pointer(&format!("/report/pages/{}", app.active_page())))
        .map(pane_style)
        .unwrap_or_default()
}

/// The app's face closest to font family `family` at `px` (its weight: the UI bundles Selawik,
/// metric-compatible with Segoe UI, in four weights); `bold` for titles without a weight named.
pub fn font(family: Option<&str>, px: f32, bold: bool) -> FontId {
    let f = family.map(str::to_ascii_lowercase).unwrap_or_default();
    let fam = if f.contains("light") {
        crate::fonts::light()
    } else if f.contains("semibold") || f.contains("semi bold") {
        crate::fonts::semibold()
    } else if f.contains("bold") || f.contains("black") || f.contains("heavy") {
        crate::fonts::bold()
    } else if bold {
        crate::fonts::semibold()
    } else {
        FontFamily::Proportional
    };
    FontId::new(px, fam)
}

/// The pane's frame: the app's, filled with the pane's background when the report sets one.
pub fn frame(app: &App, frame: egui::Frame) -> egui::Frame {
    match active_style(app).pane.background {
        Some(c) => frame.fill(c),
        None => frame,
    }
}

/// The border between the report and the pane (`border` on), drawn at the pane's left edge.
pub fn paint_border(app: &App, ui: &Ui, rect: egui::Rect) {
    let s = active_style(app).pane;
    if s.border == Some(true) {
        let t = Tokens::get(ui.ctx());
        ui.painter().vline(
            rect.left() + 0.5,
            rect.y_range(),
            egui::Stroke::new(1.0, s.border_color.unwrap_or(t.border)),
        );
    }
}

// ----- the Format tab's page cards -----

fn value_of<'a>(f: &'a Value, card: &str, key: &str) -> Option<&'a Value> {
    f.get(card).and_then(|c| c.get(key)).filter(|v| !v.is_null())
}

/// Sends one edit (refused with the reason in the status bar).
fn send(app: &mut App, page: &str, card: &str, params: Map<String, Value>) {
    if let Err(why) = ribbon::state(app, CMD) {
        app.status = format!("{} {why}", app.lang.t("Unavailable:", "Indisponible :"));
        return;
    }
    let mut p = params;
    p.insert("page".into(), json!(page));
    p.insert("card".into(), json!(card));
    app.run(
        Call {
            id: CMD,
            params: Value::Object(p),
        },
        Then::Nothing,
    );
}

/// The Format tab with nothing selected: the page's Filter pane and Filter cards cards.
pub fn page_cards(app: &mut App, ui: &mut Ui) {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    let Some(snap) = app.loaded.as_ref().map(|x| x.snap.clone()) else {
        return;
    };
    let Some(page) = snap
        .pointer(&format!("/report/pages/{}", app.active_page()))
        .cloned()
    else {
        ui.label(
            egui::RichText::new(l.t("The report has no page.", "Le rapport n'a pas de page."))
                .color(t.text_dim),
        );
        return;
    };
    let Some(page_name) = page.get("name").and_then(Value::as_str).map(str::to_string) else {
        return;
    };
    let fmt = page.get("filter_pane_format").cloned().unwrap_or(Value::Null);
    ui.label(
        egui::RichText::new(l.t("Format page", "Mettre en forme la page"))
            .font(theme::bold(12.0))
            .color(t.text_dim),
    );
    ui.add_space(4.0);
    let state = ribbon::state(app, CMD);
    // the swatches: the theme's colours, then the neutrals
    let mut swatches: Vec<String> = bicraft_render::data_colors(&snap).into_iter().take(10).collect();
    for n in ["#FFFFFF", "#F3F2F1", "#C8C6C4", "#605E5C", "#252423", "#000000"] {
        if !swatches.iter().any(|s| s.eq_ignore_ascii_case(n)) {
            swatches.push(n.to_string());
        }
    }
    let mut edits: Vec<(&'static str, Map<String, Value>)> = Vec::new();
    let st = &mut app.panes.format.page;
    egui::ScrollArea::vertical()
        .id_salt("bicraft-format-page")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (key, en, fr) in [
                ("pane", "Filter pane", "Volet Filtre"),
                ("cards", "Filter cards", "Cartes de filtre"),
            ] {
                let title = l.t(en, fr);
                let open = st.open.contains(key);
                let mut now_open = open;
                egui::Frame::NONE
                    .stroke(egui::Stroke::new(1.0, t.card_border))
                    .corner_radius(egui::CornerRadius::same(6))
                    .inner_margin(egui::Margin::symmetric(6, 4))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        now_open = widgets::section(ui, title, open);
                        if !open {
                            return;
                        }
                        let (card, rows) = if key == "pane" {
                            ("pane", PANE_ROWS)
                        } else {
                            // Desktop's "Apply settings to": the state the rows format
                            ui.horizontal(|ui| {
                                let name = l.t("Apply settings to", "Appliquer les paramètres à");
                                ui.label(egui::RichText::new(name).small().color(t.text_dim));
                                let shown = if st.applied {
                                    l.t("Applied", "Appliqué")
                                } else {
                                    l.t("Available", "Disponible")
                                };
                                let r = egui::ComboBox::from_id_salt("bicraft-filter-card-state")
                                    .selected_text(shown)
                                    .width(110.0)
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut st.applied,
                                            false,
                                            l.t("Available", "Disponible"),
                                        );
                                        ui.selectable_value(
                                            &mut st.applied,
                                            true,
                                            l.t("Applied", "Appliqué"),
                                        );
                                    });
                                let a11y = format!("{title} {name}");
                                r.response.widget_info(|| {
                                    egui::WidgetInfo::labeled(egui::Role::ComboBox, true, &a11y)
                                });
                            });
                            (if st.applied { "applied" } else { "available" }, CARD_ROWS)
                        };
                        for (param, ctl, en, fr) in rows {
                            let name = l.t(en, fr);
                            let cur = value_of(&fmt, card, param).cloned().unwrap_or(Value::Null);
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(name).small().color(t.text));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    let a11y = format!("{title} {name}");
                                    if let Some(v) = control(
                                        ui,
                                        l,
                                        st,
                                        &swatches,
                                        &page_name,
                                        card,
                                        param,
                                        *ctl,
                                        &cur,
                                        &a11y,
                                        state.is_ok(),
                                    ) {
                                        let mut m = Map::new();
                                        m.insert((*param).to_string(), v);
                                        edits.push((card, m));
                                    }
                                });
                            });
                        }
                        ui.add_space(2.0);
                        let reset = l.t("Reset to default", "Rétablir les valeurs par défaut");
                        let r = ui.add_enabled(
                            state.is_ok(),
                            egui::Button::new(egui::RichText::new(reset).small().color(t.accent))
                                .frame(false),
                        );
                        let a11y = format!("{reset} {title}");
                        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, state.is_ok(), &a11y));
                        if r.clicked() {
                            let mut m = Map::new();
                            m.insert("reset".into(), json!(true));
                            edits.push((card, m));
                        }
                    });
                if now_open != open {
                    if now_open {
                        st.open.insert(key.to_string());
                    } else {
                        st.open.remove(key);
                    }
                }
                ui.add_space(4.0);
            }
            if let Err(why) = &state {
                ui.label(
                    egui::RichText::new(format!("{} {why}", l.t("Read-only:", "Lecture seule :")))
                        .small()
                        .color(t.text_dim),
                );
            }
        });
    for (card, m) in edits {
        send(app, &page_name, card, m);
    }
}

/// One row's control: the value picked this frame.
#[allow(clippy::too_many_arguments)]
fn control(
    ui: &mut Ui,
    l: Lang,
    st: &mut PageFormatUi,
    swatches: &[String],
    page: &str,
    card: &str,
    param: &str,
    ctl: Ctl,
    cur: &Value,
    name: &str,
    enabled: bool,
) -> Option<Value> {
    let key = format!("{card}.{param}");
    if !enabled {
        ui.disable();
    }
    match ctl {
        Ctl::Color => {
            crate::format_pane::pick_color(ui, l, swatches, &mut st.drafts, page, &key, name, cur, name)
                .map(Value::String)
        }
        Ctl::Switch => {
            let on = cur.as_bool().unwrap_or(false);
            widgets::switch(ui, on, enabled, name)
                .clicked()
                .then(|| json!(!on))
        }
        Ctl::Font => {
            let fam = cur.as_str().unwrap_or("").to_string();
            let shown = if fam.is_empty() {
                "—".to_string()
            } else {
                clip(&fam, 22)
            };
            let mut picked = None;
            let r = egui::ComboBox::from_id_salt(("bicraft-filter-pane-font", &key))
                .selected_text(shown)
                .width(128.0)
                .height(300.0)
                .show_ui(ui, |ui| {
                    for f in crate::rich::FAMILIES {
                        if ui.selectable_label(fam == *f, *f).clicked() {
                            picked = Some(*f);
                        }
                    }
                });
            r.response
                .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, name));
            picked.map(|f| json!(f))
        }
        Ctl::Percent | Ctl::Size => {
            let (lo, hi, def) = if ctl == Ctl::Percent {
                (0.0, 100.0, 0.0)
            } else {
                (6.0, 72.0, 10.0)
            };
            let now = cur
                .as_f64()
                .filter(|x| x.is_finite())
                .unwrap_or(def)
                .clamp(lo, hi);
            let mut x = st
                .drafts
                .get(&key)
                .filter(|d| d.0 == page)
                .and_then(|d| d.1.parse::<f64>().ok())
                .unwrap_or(now);
            let mut dv = egui::DragValue::new(&mut x)
                .range(lo..=hi)
                .speed(0.5)
                .max_decimals(0);
            if ctl == Ctl::Percent {
                dv = dv.suffix(" %");
            }
            let r = ui.add(dv);
            r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Slider, true, name));
            let busy = r.dragged() || r.has_focus();
            let done = r.drag_stopped() || r.lost_focus() || (r.changed() && !busy);
            if done {
                st.drafts.remove(&key);
                ((x - now).abs() > f64::EPSILON).then(|| json!(x.round()))
            } else {
                if busy {
                    st.drafts.insert(key, (page.to_string(), x.to_string()));
                } else {
                    st.drafts.remove(&key);
                }
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styles_read_the_snapshot_and_survive_hostile_shapes() {
        let page = json!({"filter_pane_format": {
            "pane": {"background_color": "#102030", "transparency": 50, "border": true, "title_size": 99},
            "applied": {"foreground_color": "#FFFFFF", "text_size": 14, "font_family": "Segoe UI Bold"},
            "available": {}
        }});
        let s = pane_style(&page);
        let bg = s.pane.background.unwrap();
        assert!(bg.a() < 255 && bg.a() > 100, "{bg:?}");
        assert_eq!(s.pane.border, Some(true));
        assert_eq!(s.pane.title_size, Some(72.0));
        assert_eq!(s.applied.foreground, Some(Color32::WHITE));
        assert_eq!(s.applied.text_size, Some(14.0));
        assert_eq!(s.available, Style::default());
        assert_eq!(
            font(Some("Segoe UI Bold"), 12.0, false).family,
            crate::fonts::bold()
        );
        assert_eq!(font(None, 12.0, false).family, FontFamily::Proportional);
        for bad in [
            json!(null),
            json!([1]),
            json!({"filter_pane_format": 3}),
            json!({"filter_pane_format": {"pane": {"background_color": 3, "title_size": "x"}}}),
        ] {
            let _ = pane_style(&bad);
        }
    }
}
