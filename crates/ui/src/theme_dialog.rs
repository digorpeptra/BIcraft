//! View › Themes › Customize current theme: the Customize theme dialog, shaped as Desktop's
//! (Microsoft Learn, "Use report themes in Power BI Desktop", Customize current theme). Its
//! sections on the left: Name and colors (the theme's name, its theme colours with More colours,
//! the sentiment and divergent colours; Advanced: the structural colours), Text (General, Title,
//! Cards and KPIs, Tab headers: font family, size and colour), Visuals (Background, Border,
//! Header, Tooltips), Page (wallpaper, page background) and Filter pane (the pane, the filter
//! cards). It reads the current theme from the snapshot's `report.theme`; Apply sends what
//! changed as one `theme.customize` (one undo step), Cancel closes it.

use std::collections::BTreeMap;

use egui::{Color32, RichText, Ui, vec2};
use serde_json::{Map, Value, json};

use crate::App;
use crate::actions::Call;
use crate::app::Then;
use crate::i18n::Lang;
use crate::theme::{self, Tokens};
use crate::widgets;

/// The command Apply runs (and whose button opens the dialog).
pub const COMMAND: &str = "theme.customize";
/// Theme colours shown before More colours.
const FIRST_COLORS: usize = 8;
/// Most theme colours the dialog edits (the engine takes up to 200).
const MAX_COLORS: usize = 64;
/// The body's height (every section the same size, as Desktop's dialog).
const BODY_HEIGHT: f32 = 400.0;
/// The labels' column width.
const LABEL_W: f32 = 170.0;
/// The theme colours shown when the report has no theme: the default Power BI palette's first
/// colours (publicly documented), as the engine resolves a report without a theme.
const DEFAULT_COLORS: [&str; 8] = [
    "#118DFF", "#12239E", "#E66C37", "#6B007B", "#E044A7", "#744EC2", "#D9B300", "#D64550",
];

/// The dialog's sections, in Desktop's order.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Section {
    #[default]
    NameColors,
    Text,
    Visuals,
    Page,
    FilterPane,
}

pub const SECTIONS: [Section; 5] = [
    Section::NameColors,
    Section::Text,
    Section::Visuals,
    Section::Page,
    Section::FilterPane,
];

impl Section {
    pub fn label(self, l: Lang) -> &'static str {
        match self {
            Section::NameColors => l.t("Name and colors", "Nom et couleurs"),
            Section::Text => l.t("Text", "Texte"),
            Section::Visuals => l.t("Visuals", "Visuels"),
            Section::Page => l.t("Page", "Page"),
            Section::FilterPane => l.t("Filter pane", "Volet Filtres"),
        }
    }

    /// The section's tabs.
    fn tabs(self, l: Lang) -> Vec<&'static str> {
        match self {
            Section::NameColors => vec![
                l.t("Name and colors", "Nom et couleurs"),
                l.t("Advanced", "Avancé"),
            ],
            Section::Text => vec![
                l.t("General", "Général"),
                l.t("Title", "Titre"),
                l.t("Cards and KPIs", "Cartes et indicateurs de performance clés"),
                l.t("Tab headers", "En-têtes d'onglet"),
            ],
            Section::Visuals => vec![
                l.t("Background", "Arrière-plan"),
                l.t("Border", "Bordure"),
                l.t("Header", "En-tête"),
                l.t("Tooltips", "Info-bulles"),
            ],
            Section::Page => vec![l.t("Page", "Page")],
            Section::FilterPane => vec![
                l.t("Filter pane", "Volet Filtres"),
                l.t("Filter cards", "Cartes de filtre"),
            ],
        }
    }
}

/// How a setting is edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Color,
    /// A font family (the text box toolbar's list).
    Font,
    /// A font size in points.
    Size,
    /// A transparency, 0–100 %.
    Percent,
    /// A length in pixels (rounded corners).
    Pixels,
    Toggle,
}

/// Where a setting lives in the theme (what `theme.customize` sets).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// A top-level colour (`good`, `background`…): `theme.customize`'s parameter of that name.
    Named(&'static str),
    /// A text class property: (class, `font_face` | `font_size` | `color`).
    Text(&'static str, &'static str),
    /// A visual style: (visual type, object, `$id` state, property).
    Style(&'static str, &'static str, Option<&'static str>, &'static str),
}

/// One setting of the dialog.
#[derive(Debug, Clone, Copy)]
pub struct Field {
    pub key: &'static str,
    en: &'static str,
    fr: &'static str,
    pub kind: Kind,
    pub target: Target,
}

impl Field {
    pub fn label(&self, l: Lang) -> &'static str {
        l.t(self.en, self.fr)
    }
}

/// A row of a tab: a heading or a setting.
enum Row {
    Heading(&'static str, &'static str),
    Field(Field),
}

const fn f(key: &'static str, en: &'static str, fr: &'static str, kind: Kind, target: Target) -> Row {
    Row::Field(Field {
        key,
        en,
        fr,
        kind,
        target,
    })
}

const fn text_tab(class: &'static str) -> [Row; 3] {
    [
        f(
            class,
            "Font family",
            "Famille de polices",
            Kind::Font,
            Target::Text(class, "font_face"),
        ),
        f(
            class,
            "Font size",
            "Taille de police",
            Kind::Size,
            Target::Text(class, "font_size"),
        ),
        f(
            class,
            "Font color",
            "Couleur de police",
            Kind::Color,
            Target::Text(class, "color"),
        ),
    ]
}

const STAR: &str = "*";
const PAGE: &str = "page";

static NAME_COLORS: [Row; 8] = [
    Row::Heading("Sentiment colors", "Couleurs de sentiment"),
    f("good", "Positive", "Positif", Kind::Color, Target::Named("good")),
    f(
        "neutral",
        "Neutral",
        "Neutre",
        Kind::Color,
        Target::Named("neutral"),
    ),
    f("bad", "Negative", "Négatif", Kind::Color, Target::Named("bad")),
    Row::Heading("Divergent colors", "Couleurs divergentes"),
    f("maximum", "Max", "Max", Kind::Color, Target::Named("maximum")),
    f("center", "Middle", "Milieu", Kind::Color, Target::Named("center")),
    f("minimum", "Min", "Min", Kind::Color, Target::Named("minimum")),
];
static ADVANCED: [Row; 4] = [
    Row::Heading("Structural colors", "Couleurs structurelles"),
    f(
        "foreground",
        "First-level elements",
        "Éléments de premier niveau",
        Kind::Color,
        Target::Named("foreground"),
    ),
    f(
        "background",
        "Background",
        "Arrière-plan",
        Kind::Color,
        Target::Named("background"),
    ),
    f(
        "table_accent",
        "Table accent",
        "Accent de tableau",
        Kind::Color,
        Target::Named("table_accent"),
    ),
];
static TEXT_GENERAL: [Row; 3] = text_tab("label");
static TEXT_TITLE: [Row; 3] = text_tab("title");
static TEXT_CARDS: [Row; 3] = text_tab("callout");
static TEXT_TABS: [Row; 3] = text_tab("header");
static VIS_BACKGROUND: [Row; 2] = [
    f(
        "vis.bg.color",
        "Color",
        "Couleur",
        Kind::Color,
        Target::Style(STAR, "background", None, "color"),
    ),
    f(
        "vis.bg.transparency",
        "Transparency",
        "Transparence",
        Kind::Percent,
        Target::Style(STAR, "background", None, "transparency"),
    ),
];
static VIS_BORDER: [Row; 3] = [
    f(
        "vis.border.show",
        "Border",
        "Bordure",
        Kind::Toggle,
        Target::Style(STAR, "border", None, "show"),
    ),
    f(
        "vis.border.color",
        "Color",
        "Couleur",
        Kind::Color,
        Target::Style(STAR, "border", None, "color"),
    ),
    f(
        "vis.border.radius",
        "Rounded corners",
        "Coins arrondis",
        Kind::Pixels,
        Target::Style(STAR, "border", None, "radius"),
    ),
];
static VIS_HEADER: [Row; 3] = [
    f(
        "vis.header.bg",
        "Background color",
        "Couleur d'arrière-plan",
        Kind::Color,
        Target::Style(STAR, "visualHeader", None, "background"),
    ),
    f(
        "vis.header.border",
        "Border color",
        "Couleur de bordure",
        Kind::Color,
        Target::Style(STAR, "visualHeader", None, "border"),
    ),
    f(
        "vis.header.icon",
        "Icon color",
        "Couleur des icônes",
        Kind::Color,
        Target::Style(STAR, "visualHeader", None, "foreground"),
    ),
];
static VIS_TOOLTIP: [Row; 3] = [
    f(
        "vis.tip.label",
        "Label text color",
        "Couleur du texte des étiquettes",
        Kind::Color,
        Target::Style(STAR, "visualTooltip", None, "titleFontColor"),
    ),
    f(
        "vis.tip.value",
        "Value text color",
        "Couleur du texte des valeurs",
        Kind::Color,
        Target::Style(STAR, "visualTooltip", None, "valueFontColor"),
    ),
    f(
        "vis.tip.bg",
        "Background color",
        "Couleur d'arrière-plan",
        Kind::Color,
        Target::Style(STAR, "visualTooltip", None, "background"),
    ),
];
static PAGE_ROWS: [Row; 6] = [
    Row::Heading("Wallpaper", "Papier peint"),
    f(
        "wallpaper.color",
        "Color",
        "Couleur",
        Kind::Color,
        Target::Style(PAGE, "outspace", None, "color"),
    ),
    f(
        "wallpaper.transparency",
        "Transparency",
        "Transparence",
        Kind::Percent,
        Target::Style(PAGE, "outspace", None, "transparency"),
    ),
    Row::Heading("Page background", "Arrière-plan de la page"),
    f(
        "page_bg.color",
        "Color",
        "Couleur",
        Kind::Color,
        Target::Style(PAGE, "background", None, "color"),
    ),
    f(
        "page_bg.transparency",
        "Transparency",
        "Transparence",
        Kind::Percent,
        Target::Style(PAGE, "background", None, "transparency"),
    ),
];
static FILTER_PANE: [Row; 4] = [
    f(
        "fp.bg",
        "Background color",
        "Couleur d'arrière-plan",
        Kind::Color,
        Target::Style(PAGE, "outspacePane", None, "backgroundColor"),
    ),
    f(
        "fp.transparency",
        "Background transparency",
        "Transparence de l'arrière-plan",
        Kind::Percent,
        Target::Style(PAGE, "outspacePane", None, "transparency"),
    ),
    f(
        "fp.font",
        "Font and icon color",
        "Couleur de police et des icônes",
        Kind::Color,
        Target::Style(PAGE, "outspacePane", None, "foregroundColor"),
    ),
    f(
        "fp.border",
        "Border color",
        "Couleur de bordure",
        Kind::Color,
        Target::Style(PAGE, "outspacePane", None, "borderColor"),
    ),
];
static FILTER_CARDS: [Row; 8] = [
    Row::Heading("Available filter cards", "Cartes de filtre disponibles"),
    f(
        "fc.av.bg",
        "Background color",
        "Couleur d'arrière-plan",
        Kind::Color,
        Target::Style(PAGE, "filterCard", Some("Available"), "backgroundColor"),
    ),
    f(
        "fc.av.font",
        "Font and icon color",
        "Couleur de police et des icônes",
        Kind::Color,
        Target::Style(PAGE, "filterCard", Some("Available"), "foregroundColor"),
    ),
    f(
        "fc.av.border",
        "Border color",
        "Couleur de bordure",
        Kind::Color,
        Target::Style(PAGE, "filterCard", Some("Available"), "borderColor"),
    ),
    Row::Heading("Applied filter cards", "Cartes de filtre appliquées"),
    f(
        "fc.ap.bg",
        "Background color",
        "Couleur d'arrière-plan",
        Kind::Color,
        Target::Style(PAGE, "filterCard", Some("Applied"), "backgroundColor"),
    ),
    f(
        "fc.ap.font",
        "Font and icon color",
        "Couleur de police et des icônes",
        Kind::Color,
        Target::Style(PAGE, "filterCard", Some("Applied"), "foregroundColor"),
    ),
    f(
        "fc.ap.border",
        "Border color",
        "Couleur de bordure",
        Kind::Color,
        Target::Style(PAGE, "filterCard", Some("Applied"), "borderColor"),
    ),
];

/// The rows of a section's tab.
fn rows(section: Section, tab: usize) -> &'static [Row] {
    match (section, tab) {
        (Section::NameColors, 0) => &NAME_COLORS,
        (Section::NameColors, _) => &ADVANCED,
        (Section::Text, 0) => &TEXT_GENERAL,
        (Section::Text, 1) => &TEXT_TITLE,
        (Section::Text, 2) => &TEXT_CARDS,
        (Section::Text, _) => &TEXT_TABS,
        (Section::Visuals, 0) => &VIS_BACKGROUND,
        (Section::Visuals, 1) => &VIS_BORDER,
        (Section::Visuals, 2) => &VIS_HEADER,
        (Section::Visuals, _) => &VIS_TOOLTIP,
        (Section::Page, _) => &PAGE_ROWS,
        (Section::FilterPane, 0) => &FILTER_PANE,
        (Section::FilterPane, _) => &FILTER_CARDS,
    }
}

/// Every setting of the dialog.
pub fn fields() -> impl Iterator<Item = &'static Field> {
    SECTIONS
        .iter()
        .flat_map(|s| (0..s.tabs(Lang::En).len()).map(move |t| rows(*s, t)))
        .flatten()
        .filter_map(|r| match r {
            Row::Field(f) => Some(f),
            Row::Heading(..) => None,
        })
}

/// A setting's unique id: its key, and for text settings the property too.
fn id_of(f: &Field) -> String {
    match f.target {
        Target::Text(class, prop) => format!("text.{class}.{prop}"),
        _ => f.key.to_string(),
    }
}

/// The dialog while it is open.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Dialog {
    pub section: Section,
    pub tab: usize,
    pub name: String,
    name0: String,
    /// The theme colours, `#RRGGBB` as typed.
    pub colors: Vec<String>,
    colors0: Vec<String>,
    /// More colours shown (theme colours past the eighth).
    pub more: bool,
    /// Every setting by id ([`id_of`]): `null` when the theme does not set it.
    pub values: BTreeMap<String, Value>,
    init: BTreeMap<String, Value>,
}

/// `#RGB`, `#RRGGBB` or `#RRGGBBAA` (the theme schema's colour pattern).
fn is_color(s: &str) -> bool {
    s.strip_prefix('#')
        .is_some_and(|h| matches!(h.len(), 3 | 6 | 8) && h.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// A colour as the picker shows it (`#RGB` expanded); `None` for anything else.
fn to_color32(s: &str) -> Option<Color32> {
    let h = s.trim().strip_prefix('#')?;
    let full: String = match h.len() {
        3 => h.chars().flat_map(|c| [c, c]).collect(),
        6 | 8 => h.chars().take(6).collect(),
        _ => return None,
    };
    let n = u32::from_str_radix(&full, 16).ok()?;
    Some(Color32::from_rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

fn hex(c: Color32) -> String {
    format!("#{:02X}{:02X}{:02X}", c.r(), c.g(), c.b())
}

/// The value of setting `f` in the snapshot's resolved theme (`null` when unset).
pub fn read(theme: &Value, f: &Field) -> Value {
    let v = match f.target {
        Target::Named(k) => theme.get(k),
        Target::Text(class, prop) => theme.get("text_classes").and_then(|c| c.get(class)?.get(prop)),
        Target::Style(vt, object, state, prop) => {
            let key = match state {
                Some(st) => format!("{object}.{st}"),
                None => object.to_string(),
            };
            theme.get("styles").and_then(|s| s.get(vt)?.get(&key)?.get(prop))
        }
    };
    let v = v.cloned().unwrap_or(Value::Null);
    match (f.kind, &v) {
        (Kind::Color | Kind::Font, Value::String(_))
        | (Kind::Size | Kind::Percent | Kind::Pixels, Value::Number(_))
        | (Kind::Toggle, Value::Bool(_)) => v,
        _ => Value::Null,
    }
}

impl Dialog {
    /// The dialog on the snapshot's resolved theme (`null`: the default theme).
    pub fn from_theme(theme: &Value) -> Self {
        let name = theme
            .get("name")
            .and_then(Value::as_str)
            .filter(|n| !n.trim().is_empty())
            // the name the engine gives a custom theme it starts
            .unwrap_or("Custom theme")
            .to_string();
        let mut colors: Vec<String> = theme
            .get("data_colors")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .take(MAX_COLORS)
            .map(str::to_string)
            .collect();
        if colors.is_empty() {
            colors = DEFAULT_COLORS.iter().map(|c| (*c).to_string()).collect();
        }
        let values: BTreeMap<String, Value> = fields().map(|f| (id_of(f), read(theme, f))).collect();
        Dialog {
            name0: name.clone(),
            name,
            colors0: colors.clone(),
            more: colors.len() > FIRST_COLORS,
            colors,
            init: values.clone(),
            values,
            ..Dialog::default()
        }
    }

    /// What is wrong with the settings (Apply is disabled while any is).
    pub fn problems(&self, l: Lang) -> Vec<String> {
        let mut out = Vec::new();
        if self.name.trim().is_empty() {
            out.push(
                l.t("The theme needs a name.", "Le thème doit avoir un nom.")
                    .to_string(),
            );
        }
        for (i, c) in self.colors.iter().enumerate() {
            if !is_color(c.trim()) {
                out.push(format!(
                    "{} {}: {}",
                    l.t("Color", "Couleur"),
                    i + 1,
                    l.t("use #RRGGBB", "utilisez #RRVVBB")
                ));
            }
        }
        for f in fields() {
            if f.kind == Kind::Color
                && let Some(Value::String(s)) = self.values.get(&id_of(f))
                && !is_color(s.trim())
            {
                out.push(format!(
                    "{}: {}",
                    f.label(l),
                    l.t("use #RRGGBB", "utilisez #RRVVBB")
                ));
            }
        }
        out
    }

    /// `theme.customize` with what changed; `None` when nothing did.
    pub fn apply_call(&self) -> Option<Call> {
        let mut p = Map::new();
        let name = self.name.trim();
        if name != self.name0.trim() && !name.is_empty() {
            p.insert("name".into(), json!(name));
        }
        if self.colors != self.colors0 {
            let list: Vec<String> = self.colors.iter().map(|c| c.trim().to_uppercase()).collect();
            p.insert("data_colors".into(), json!(list));
        }
        let mut classes = Map::new();
        let mut styles = Vec::new();
        for f in fields() {
            let id = id_of(f);
            let Some(v) = self.values.get(&id) else { continue };
            if v.is_null() || self.init.get(&id) == Some(v) {
                continue;
            }
            let v = match (f.kind, v) {
                (Kind::Color, Value::String(s)) => json!(s.trim().to_uppercase()),
                _ => v.clone(),
            };
            match f.target {
                Target::Named(k) => {
                    p.insert(k.into(), v);
                }
                Target::Text(class, prop) => {
                    if let Value::Object(c) = classes.entry(class.to_string()).or_insert_with(|| json!({})) {
                        c.insert(prop.into(), v);
                    }
                }
                Target::Style(vt, object, state, prop) => {
                    let value = if f.kind == Kind::Color {
                        json!({ "color": v })
                    } else {
                        v
                    };
                    let mut s =
                        json!({"visual_type": vt, "object": object, "property": prop, "value": value});
                    if let (Some(st), Value::Object(o)) = (state, &mut s) {
                        o.insert("state".into(), json!(st));
                    }
                    styles.push(s);
                }
            }
        }
        if !classes.is_empty() {
            p.insert("text_classes".into(), Value::Object(classes));
        }
        if !styles.is_empty() {
            p.insert("styles".into(), Value::Array(styles));
        }
        (!p.is_empty()).then(|| Call {
            id: COMMAND,
            params: Value::Object(p),
        })
    }
}

/// The snapshot's resolved theme.
fn current(app: &App) -> Value {
    app.loaded
        .as_ref()
        .and_then(|l| l.snap.get("report")?.get("theme").cloned())
        .unwrap_or(Value::Null)
}

/// Opens the dialog on the report's current theme.
pub fn open(app: &mut App) {
    app.cmd.theme = Some(Dialog::from_theme(&current(app)));
}

/// `theme.customize` without settings (its button, the palette) opens the dialog.
pub fn intercept(app: &mut App, id: &str, params: &Value) -> bool {
    if id != COMMAND {
        return false;
    }
    let has_settings = params.as_object().is_some_and(|o| o.keys().any(|k| k != "force"));
    if has_settings {
        return false;
    }
    open(app);
    true
}

/// Draws the dialog when it is open.
pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut d) = app.cmd.theme.clone() else {
        return;
    };
    let l = app.lang;
    let mut open = true;
    let mut close = false;
    let mut call = None;
    egui::Window::new(l.t("Customize theme", "Personnaliser le thème"))
        .id(egui::Id::unique("bicraft-customize-theme"))
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_width(720.0);
            body(ui, &mut d, l, &mut call, &mut close);
        });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        close = true;
    }
    app.cmd.theme = (open && !close).then_some(d);
    if let Some(c) = call {
        app.run(c, Then::Nothing);
    }
}

fn body(ui: &mut Ui, d: &mut Dialog, l: Lang, call: &mut Option<Call>, close: &mut bool) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal_top(|ui| {
        ui.set_height(BODY_HEIGHT);
        // the sections
        ui.vertical(|ui| {
            ui.set_width(150.0);
            for s in SECTIONS {
                let on = d.section == s;
                let r = ui.selectable_label(
                    on,
                    RichText::new(s.label(l)).font(if on { theme::bold(13.0) } else { theme::font(13.0) }),
                );
                if r.clicked() && !on {
                    d.section = s;
                    d.tab = 0;
                }
            }
        });
        ui.separator();
        ui.vertical(|ui| {
            let tabs = d.section.tabs(l);
            if tabs.len() > 1 {
                ui.horizontal(|ui| {
                    for (i, name) in tabs.iter().enumerate() {
                        if widgets::tab(ui, name, d.tab == i, false).clicked() {
                            d.tab = i;
                        }
                    }
                });
                ui.add_space(6.0);
            }
            egui::ScrollArea::vertical()
                .id_salt(("bicraft-theme-body", d.section as u8, d.tab))
                .max_height(BODY_HEIGHT - 40.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    if d.section == Section::NameColors && d.tab == 0 {
                        name_and_colors(ui, d, l, &t);
                    }
                    let rows = rows(d.section, d.tab.min(tabs.len().saturating_sub(1)));
                    grid(ui, d, rows, l, &t);
                });
        });
    });
    ui.add_space(6.0);
    let problems = d.problems(l);
    for p in problems.iter().take(3) {
        ui.label(RichText::new(format!("⚠ {p}")).color(t.err_text));
    }
    ui.separator();
    ui.horizontal(|ui| {
        let c = d.apply_call();
        let ok = problems.is_empty();
        let reason = problems.first().map(String::as_str).unwrap_or_default();
        let r = widgets::primary(ui, l.t("Apply", "Appliquer"), ok, reason);
        if r.clicked() && ok {
            *call = c;
            *close = true;
        }
        if ui.button(l.t("Cancel", "Annuler")).clicked() {
            *close = true;
        }
        if d.apply_call().is_none() {
            ui.label(RichText::new(l.t("No change yet.", "Aucune modification.")).color(t.text_dim));
        }
    });
}

/// Name and colors: the theme's name, then its theme colours (eight, then More colours).
fn name_and_colors(ui: &mut Ui, d: &mut Dialog, l: Lang, t: &Tokens) {
    egui::Grid::new("bicraft-theme-name")
        .num_columns(2)
        .spacing(vec2(12.0, 8.0))
        .min_col_width(LABEL_W)
        .show(ui, |ui| {
            let label = ui.label(l.t("Name", "Nom"));
            ui.add(
                egui::TextEdit::singleline(&mut d.name)
                    .char_limit(200)
                    .desired_width(300.0),
            )
            .labelled_by(label.id);
            ui.end_row();
        });
    ui.add_space(6.0);
    ui.label(RichText::new(l.t("Theme colors", "Couleurs du thème")).font(theme::bold(13.0)));
    let shown = if d.more {
        d.colors.len()
    } else {
        d.colors.len().min(FIRST_COLORS)
    };
    egui::Grid::new("bicraft-theme-colors")
        .spacing(vec2(10.0, 6.0))
        .show(ui, |ui| {
            for i in 0..shown {
                let what = format!("{} {}", l.t("Color", "Couleur"), i + 1);
                if let Some(c) = d.colors.get_mut(i) {
                    ui.vertical(|ui| {
                        ui.set_min_width(118.0);
                        ui.label(RichText::new(&what).color(t.text_dim).font(theme::font(11.0)));
                        color_edit(ui, c, &what, 72.0);
                    });
                }
                if i % 4 == 3 {
                    ui.end_row();
                }
            }
        });
    ui.horizontal(|ui| {
        if d.colors.len() > FIRST_COLORS || d.more {
            let text = if d.more {
                l.t("Fewer colors", "Moins de couleurs")
            } else {
                l.t("More colors", "Plus de couleurs")
            };
            if ui.button(text).clicked() {
                d.more = !d.more;
            }
        }
        if d.colors.len() < MAX_COLORS && ui.button(l.t("Add a color", "Ajouter une couleur")).clicked() {
            let next = d
                .colors
                .last()
                .cloned()
                .unwrap_or_else(|| DEFAULT_COLORS[0].to_string());
            d.colors.push(next);
            d.more = d.colors.len() > FIRST_COLORS;
        }
        if d.colors.len() > 1
            && ui
                .button(l.t("Remove the last color", "Retirer la dernière couleur"))
                .clicked()
        {
            d.colors.pop();
        }
    });
    ui.add_space(8.0);
}

/// A colour picker (the swatch opens egui's picker) and its `#RRGGBB` text, both labelled
/// `what` for the accessibility tree.
fn color_edit(ui: &mut Ui, text: &mut String, what: &str, w: f32) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        let mut c = to_color32(text).unwrap_or(Color32::GRAY);
        let r = ui.color_edit_button_srgba(&mut c);
        let a11y = format!("{what} ({})", text.trim());
        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ColorWell, true, &a11y));
        if r.changed() {
            *text = hex(c);
            changed = true;
        }
        let e = ui.add(
            egui::TextEdit::singleline(text)
                .char_limit(9)
                .desired_width(w)
                .hint_text("#RRGGBB"),
        );
        e.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, what));
        changed |= e.changed();
    });
    changed
}

/// The settings of a tab, a label column and a control column. Each control's accessibility
/// label says where it is: `tab › heading › setting` (the tab and heading when there are some).
fn grid(ui: &mut Ui, d: &mut Dialog, rows: &[Row], l: Lang, t: &Tokens) {
    let tab = (d.section.tabs(l).len() > 1).then(|| tab_name(d, l));
    let mut heading: Option<&str> = None;
    egui::Grid::new(("bicraft-theme-grid", d.section as u8, d.tab))
        .num_columns(2)
        .spacing(vec2(12.0, 8.0))
        .min_col_width(LABEL_W)
        .show(ui, |ui| {
            for row in rows {
                match row {
                    Row::Heading(en, fr) => {
                        let h = l.t(en, fr);
                        heading = Some(h);
                        ui.label(RichText::new(h).font(theme::bold(13.0)));
                        ui.end_row();
                    }
                    Row::Field(f) => {
                        ui.label(f.label(l));
                        let what: Vec<&str> = tab.into_iter().chain(heading).chain([f.label(l)]).collect();
                        field(ui, d, f, &what.join(" › "), l, t);
                        ui.end_row();
                    }
                }
            }
        });
}

/// The control of one setting (`what` its accessibility label); a setting the theme leaves
/// unset says "Default".
fn field(ui: &mut Ui, d: &mut Dialog, f: &Field, what: &str, l: Lang, t: &Tokens) {
    let id = id_of(f);
    let value = d.values.get(&id).cloned().unwrap_or(Value::Null);
    let default_note = |ui: &mut Ui| {
        ui.label(RichText::new(l.t("Default", "Par défaut")).color(t.text_dim));
    };
    let mut next: Option<Value> = None;
    ui.horizontal(|ui| match f.kind {
        Kind::Color => {
            let mut text = value.as_str().unwrap_or_default().to_string();
            if color_edit(ui, &mut text, what, 90.0) {
                next = Some(json!(text));
            }
            if value.is_null() && next.is_none() {
                default_note(ui);
            }
        }
        Kind::Font => {
            let now = value.as_str().map(str::to_string);
            let shown = now
                .clone()
                .unwrap_or_else(|| l.t("Default", "Par défaut").to_string());
            let r = egui::ComboBox::from_id_salt(("bicraft-theme-font", id.as_str()))
                .selected_text(&shown)
                .width(220.0)
                .show_ui(ui, |ui| {
                    let mut pick = None;
                    let mut list: Vec<&str> = crate::rich::FAMILIES.to_vec();
                    if let Some(n) = now.as_deref()
                        && !list.contains(&n)
                    {
                        list.insert(0, n);
                    }
                    for fam in list {
                        let on = now.as_deref() == Some(fam);
                        if ui.selectable_label(on, fam).clicked() {
                            pick = Some(fam.to_string());
                        }
                    }
                    pick
                });
            r.response
                .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, what));
            if let Some(Some(fam)) = r.inner {
                next = Some(json!(fam));
            }
        }
        Kind::Size | Kind::Percent | Kind::Pixels => {
            let (fallback, range, suffix) = match f.kind {
                Kind::Size => (10.0, 1.0..=1000.0, " pt"),
                Kind::Percent => (0.0, 0.0..=100.0, " %"),
                _ => (0.0, 0.0..=100.0, " px"),
            };
            let mut x = value.as_f64().unwrap_or(fallback);
            let r = ui.add(
                egui::DragValue::new(&mut x)
                    .range(range)
                    .speed(1.0)
                    .max_decimals(1)
                    .suffix(suffix),
            );
            r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::SpinButton, true, what));
            if r.changed() {
                next = Some(number(x));
            }
            if value.is_null() && next.is_none() {
                default_note(ui);
            }
        }
        Kind::Toggle => {
            let mut on = value.as_bool().unwrap_or(false);
            let r = ui.checkbox(&mut on, l.t("On", "Activé"));
            r.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, on, what));
            if r.changed() {
                next = Some(json!(on));
            }
        }
    });
    if let Some(v) = next {
        d.values.insert(id, v);
    }
}

/// The name of the tab shown (the accessibility labels say which tab a setting is on).
fn tab_name(d: &Dialog, l: Lang) -> &'static str {
    let tabs = d.section.tabs(l);
    tabs.get(d.tab)
        .or_else(|| tabs.first())
        .copied()
        .unwrap_or_default()
}

/// A number as JSON, whole values without a fraction.
fn number(x: f64) -> Value {
    if x.is_finite() && x.fract() == 0.0 && x.abs() < 9.0e15 {
        json!(x as i64)
    } else {
        json!(x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dialog_reads_the_theme_and_applies_only_what_changed() {
        let theme = json!({
            "name": "Mine", "data_colors": ["#111111", "#222222"], "background": "#FFFFFF",
            "foreground": "#252423", "table_accent": "#118DFF", "good": "#00AA00",
            "text_classes": {"title": {"font_face": "Georgia", "font_size": 14}},
            "styles": {"*": {"border": {"show": true}}, "page": {"filterCard.Applied": {"backgroundColor": "#EEEEEE"}}}
        });
        let mut d = Dialog::from_theme(&theme);
        assert_eq!(d.colors, ["#111111", "#222222"]);
        assert_eq!(d.values["good"], "#00AA00");
        assert_eq!(d.values["text.title.font_face"], "Georgia");
        assert_eq!(d.values["text.title.font_size"], 14);
        assert_eq!(d.values["vis.border.show"], true);
        assert_eq!(d.values["fc.ap.bg"], "#EEEEEE");
        assert_eq!(d.values["neutral"], Value::Null);
        assert!(d.apply_call().is_none());
        d.name = "Renamed".into();
        d.colors[1] = "#abcdef".into();
        d.values.insert("bad".into(), json!("#cc0000"));
        d.values.insert("text.callout.font_size".into(), json!(40));
        d.values.insert("fc.ap.bg".into(), json!("#DDDDDD"));
        d.values.insert("vis.bg.transparency".into(), json!(20));
        let c = d.apply_call().unwrap();
        assert_eq!(c.id, COMMAND);
        assert_eq!(
            c.params,
            json!({
                "name": "Renamed", "data_colors": ["#111111", "#ABCDEF"], "bad": "#CC0000",
                "text_classes": {"callout": {"font_size": 40}},
                "styles": [
                    {"visual_type": "*", "object": "background", "property": "transparency", "value": 20},
                    {"visual_type": "page", "object": "filterCard", "property": "backgroundColor",
                     "value": {"color": "#DDDDDD"}, "state": "Applied"}
                ]
            })
        );
        d.colors[0] = "red".into();
        assert_eq!(d.problems(Lang::En).len(), 1);
        // no theme: the default palette, nothing to apply
        let d = Dialog::from_theme(&Value::Null);
        assert_eq!(d.colors.len(), 8);
        assert!(d.apply_call().is_none());
        // every setting has a unique id and both languages
        let ids: std::collections::BTreeSet<String> = fields().map(id_of).collect();
        assert_eq!(ids.len(), fields().count());
        for f in fields() {
            assert!(!f.label(Lang::Fr).is_empty() && !f.label(Lang::En).is_empty());
        }
    }
}
