//! The Build pane's Format visual tab (ROADMAP M11.22, docs/gaps.md G54), as Power BI Desktop's
//! Visualizations pane draws it: two sub-tabs at the pane's top (Build visual, Format visual),
//! then for the selected visual the **Visual** and **General** groups of collapsible cards, a
//! search box over them, an on/off switch in the header of the cards Desktop gives one, and a
//! "Reset to default" link at the bottom of each card.
//!
//! - **General**: Title (text as free text, font, size, bold / italic / underline, colour,
//!   alignment), Subtitle, Effects (background colour and transparency, visual border, shadow),
//!   Header icons, Tooltips (the tooltip page), Properties (padding, alt text).
//! - **Visual**: the cards of the visual's family: axes, legend, data labels and data colours for
//!   charts; callout value and category label for cards; values and column headers for tables
//!   and matrices; header and values for slicers; scaling and Change image for images. A type
//!   with none says so and offers the palette's Format visual (every property by name).
//!
//! Values shown are the visual's own (`visual.props`, read again at each snapshot revision), then
//! the snapshot's resolved `format` (theme included), then Desktop's defaults. Every committed
//! edit is one engine command (`visual.set_title`, `visual.format`, `visual.axis`,
//! `visual.tooltip_page`, `image.set`), so undo takes it back: a switch, a pick in a list, a
//! colour swatch at once; text when Enter is pressed or the box loses the focus (Escape keeps the
//! old text); a number when it is let go. Readers see no Format tab.
//!
//! A colour row Desktop can format conditionally (a chart's data colour and data labels' colour,
//! a table's values background and text colour, a card's callout and category label colours, the
//! title's, subtitle's, background's and border's colours) has an **fx** button at its right,
//! as the title's and subtitle's text box (a field value), marked
//! when the visual has such a format; it opens [`crate::conditional_dialog`], whose OK is one
//! `visual.conditional` command.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

use bicraft_visuals as visuals;
use egui::{Align2, Color32, CornerRadius, Key, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde_json::{Map, Value, json};

use crate::App;
use crate::actions::Call;
use crate::app::{Then, clip};
use crate::i18n::Lang;
use crate::icons::{self, Icon};
use crate::ribbon::{self, VisualSel};
use crate::theme::{self, Tokens};
use crate::widgets;

/// The commands the Format tab runs (the coverage test reads it).
pub const COMMANDS: &[&str] = &[
    "visual.props",
    "visual.set_title",
    "visual.format",
    "visual.tooltip_page",
    "image.set",
    "shape_map.set_map",
    "visual.series",
    "column.values",
    "visual.conditional",
    "visual.set_data_format",
];

/// Most per-series or per-value colour rows the Data colours card lists.
const MAX_SERIES_ROWS: usize = 100;

/// Longest text a format text box keeps (characters; the engine caps titles at 4000).
const MAX_TEXT: usize = 4000;

/// The Build pane's sub-tab.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Build,
    Format,
    /// The Analytics tab ([`crate::analytics_pane`]).
    Analytics,
}

/// The Format tab's group of cards.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Group {
    #[default]
    Visual,
    General,
}

/// The selected visual's own formatting, as `visual.props` answered.
#[derive(Debug, Clone, PartialEq)]
pub struct Own {
    pub page: String,
    pub visual: String,
    /// The snapshot revision it was read at.
    pub revision: u64,
    pub props: Value,
}

/// The Format tab's view state (pure UI: never sent anywhere).
#[derive(Debug, Default)]
pub struct FormatUi {
    pub tab: Tab,
    pub group: Group,
    pub search: String,
    /// The cards expanded (their keys).
    pub open: BTreeSet<String>,
    /// The selected visual's own formatting.
    pub own: Option<Own>,
    /// The `visual.props` asked for: (page, visual, revision).
    pub asked: Option<(String, String, u64)>,
    /// Text and numbers being typed or dragged: `card.prop` → (visual, draft).
    pub drafts: BTreeMap<String, (String, String)>,
    /// Values sent and not answered yet, shown meanwhile: `card.prop` → value.
    pub pending: BTreeMap<String, Value>,
    /// The distinct values of the column the Data colours card lists one colour per value of
    /// (`column.values`): `Table[Column]` → (snapshot revision, values, truncated).
    pub values: BTreeMap<String, (u64, Vec<Value>, bool)>,
    /// The `column.values` asked for: (`Table[Column]`, revision).
    pub values_asked: BTreeSet<(String, u64)>,
    /// The conditional-formatting dialog an fx button opened.
    pub cf: Option<crate::conditional_dialog::CfDialog>,
    /// The Analytics tab's view state.
    pub analytics: crate::analytics_pane::AnalyticsUi,
    /// The page's Filter pane and Filter cards cards (nothing selected).
    pub page: crate::filter_pane_ui::PageFormatUi,
    /// The state the stateful cards of a button, navigator or shape edit (Desktop's "Apply
    /// settings to › State"): empty or `default`, `hover`, `press`, `selected`, `disabled`.
    pub state: String,
}

/// The states a visual type's stateful cards offer (value, EN, FR): buttons (Microsoft Learn,
/// "Create buttons": default, on hover, on press, disabled), navigators (default, hover, press,
/// selected: "Create page and bookmark navigators"), shapes (default, hover, press); empty for the
/// other types.
pub fn states_of(vtype: &str) -> &'static [(&'static str, &'static str, &'static str)] {
    const BUTTON: &[(&str, &str, &str)] = &[
        ("default", "Default", "Par défaut"),
        ("hover", "On hover", "Au survol"),
        ("press", "On press", "Au clic"),
        ("disabled", "Disabled", "Désactivé"),
    ];
    const NAVIGATOR: &[(&str, &str, &str)] = &[
        ("default", "Default", "Par défaut"),
        ("hover", "On hover", "Au survol"),
        ("press", "On press", "Au clic"),
        ("selected", "Selected", "Sélectionné"),
    ];
    const SHAPE: &[(&str, &str, &str)] = &[
        ("default", "Default", "Par défaut"),
        ("hover", "On hover", "Au survol"),
        ("press", "On press", "Au clic"),
    ];
    match vtype {
        "actionButton" => BUTTON,
        "pageNavigator" | "bookmarkNavigator" => NAVIGATOR,
        "shape" => SHAPE,
        _ => &[],
    }
}

/// Whether `object` of `vtype` takes a state (the engine's `visual_states`, docs/formats/
/// visual-options.md "State selectors").
pub fn stateful_object(vtype: &str, object: &str) -> bool {
    match vtype {
        "actionButton" => matches!(
            object,
            "fill" | "glow" | "icon" | "outline" | "shadow" | "shape" | "text"
        ),
        "pageNavigator" | "bookmarkNavigator" => {
            matches!(
                object,
                "accentBar" | "fill" | "glow" | "outline" | "shadow" | "text"
            )
        }
        "shape" => matches!(object, "fill" | "outline" | "text"),
        _ => false,
    }
}

// ----- the cards -----

/// Where a property is written (and read back).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum To {
    /// `visual.set_title {title: {key: value}}`.
    Title(&'static str),
    /// `visual.format {group: {key: value}}`.
    Format(&'static str, &'static str),
    /// `visual.format {group: value}` (padding for every side, alt text).
    Whole(&'static str),
    /// `visual.axis {axis, key: value}`.
    Axis(&'static str, &'static str),
    /// `visual.format {objects: [{object, property, value}]}`.
    Object(&'static str, &'static str),
    /// A matrix's per-level subtotal property (`levelSubtotalEnabled`, `levelSubtotalLabel`) of
    /// level `.1` of its rows (`.0` 0) or columns (1): `visual.format {objects: [{object:
    /// "subTotals", property, value, selector: {metadata: <the level's queryRef>}}]}`, the
    /// selector added from the snapshot when the edit is committed ([`Cx::commit`]); read back
    /// from the snapshot's `data.row_level_totals` / `column_level_totals`.
    Level(u8, u8, &'static str),
    /// `visual.tooltip_page {tooltip_page}`.
    TooltipPage,
    /// `image.set {scaling}`.
    Scaling,
}

/// A property's control.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Ctl {
    /// Free text (committed on Enter or when the box loses the focus).
    Text,
    /// A font size from Desktop's list.
    Size,
    /// A font family from the text box editor's list.
    Font,
    Color,
    /// Left, center, right.
    Align,
    /// A bold / italic / underline button (`B`, `I`, `U`), drawn in one row with its neighbours.
    Style(&'static str),
    /// An on/off switch.
    Toggle,
    /// 0–100 (%).
    Percent,
    /// A number in a range.
    Number(f64, f64),
    /// One of (value, EN, FR).
    Choice(&'static [(&'static str, &'static str, &'static str)]),
    /// One of (value, EN, FR) written as a number (the catalog's numeric enumerations, such as
    /// the display units: `0`, `1000`).
    Numbered(&'static [(&'static str, &'static str, &'static str)]),
    /// A page of the report, Default or None.
    Pages,
    /// A button that runs this command as its ribbon button does.
    Run(&'static str, &'static str, &'static str),
    /// A button that runs this command at once with one parameter set (command, parameter,
    /// value, EN, FR), for the selected visual.
    RunWith(
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
    ),
    /// Only an fx button: the conditional-formatting dialog of this `visual.conditional` target
    /// (a table's or matrix's data bars, icons, web URL: Desktop's Cell elements).
    Fx(&'static str),
}

/// A default value shown when neither the visual nor the theme sets one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Def {
    None,
    Bool(bool),
    Num(f64),
    Str(&'static str),
    /// The theme's first data colour.
    Accent,
}

/// One property of a card.
#[derive(Debug, Clone, Copy)]
pub struct Prop {
    pub key: &'static str,
    pub en: &'static str,
    pub fr: &'static str,
    pub ctl: Ctl,
    pub to: To,
    pub def: Def,
}

/// A part of a card: an optional sub-header with its own switch (Effects › Background), then
/// properties.
#[derive(Debug, Clone)]
pub struct Section {
    pub title: Option<(&'static str, &'static str)>,
    pub switch: Option<(To, bool)>,
    /// The General group's rows are static; a Visual card's rows are built from the visual
    /// catalog ([`visual_cards`]), so the list is a `Cow`.
    pub props: Cow<'static, [Prop]>,
}

/// A card of the Format tab.
#[derive(Debug, Clone)]
pub struct Card {
    pub key: &'static str,
    pub en: &'static str,
    pub fr: &'static str,
    pub group: Group,
    /// The header's switch: where `show` is written, and Desktop's default.
    pub switch: Option<(To, bool)>,
    pub sections: Cow<'static, [Section]>,
}

impl Card {
    pub fn title(&self, l: Lang) -> &'static str {
        l.t(self.en, self.fr)
    }
}

const fn p(key: &'static str, en: &'static str, fr: &'static str, ctl: Ctl, to: To, def: Def) -> Prop {
    Prop {
        key,
        en,
        fr,
        ctl,
        to,
        def,
    }
}

const LEGEND_POSITIONS: &[(&str, &str, &str)] = &[
    ("Top", "Top", "Haut"),
    ("Bottom", "Bottom", "Bas"),
    ("Left", "Left", "Gauche"),
    ("Right", "Right", "Droite"),
    ("TopCenter", "Top center", "Haut au centre"),
    ("TopRight", "Top right", "Haut à droite"),
    ("BottomCenter", "Bottom center", "Bas au centre"),
    ("BottomRight", "Bottom right", "Bas à droite"),
    ("LeftCenter", "Left center", "Gauche au centre"),
    ("RightCenter", "Right center", "Droite au centre"),
];
const SCALINGS: &[(&str, &str, &str)] = &[
    ("normal", "Normal", "Normal"),
    ("fit", "Fit", "Ajuster"),
    ("fill", "Fill", "Remplir"),
];

/// The title and the subtitle's text properties: `To::$v($($pre)* key)`.
macro_rules! text_props {
    ($v:ident [$($pre:tt)*], $size:expr, $color:literal) => {
        &[
            p("text", "Text", "Texte", Ctl::Text, To::$v($($pre)* "text"), Def::None),
            p("font_family", "Font", "Police", Ctl::Font, To::$v($($pre)* "font_family"), Def::Str("Segoe UI")),
            p("font_size", "Font size", "Taille de police", Ctl::Size, To::$v($($pre)* "font_size"), Def::Num($size)),
            p("bold", "Bold", "Gras", Ctl::Style("B"), To::$v($($pre)* "bold"), Def::Bool(false)),
            p("italic", "Italic", "Italique", Ctl::Style("I"), To::$v($($pre)* "italic"), Def::Bool(false)),
            p("underline", "Underline", "Souligné", Ctl::Style("U"), To::$v($($pre)* "underline"), Def::Bool(false)),
            p("color", "Text colour", "Couleur du texte", Ctl::Color, To::$v($($pre)* "color"), Def::Str($color)),
            p("alignment", "Horizontal alignment", "Alignement horizontal", Ctl::Align, To::$v($($pre)* "alignment"), Def::Str("left")),
        ]
    };
}

const TITLE: Card = Card {
    key: "title",
    en: "Title",
    fr: "Titre",
    group: Group::General,
    switch: Some((To::Title("show"), true)),
    sections: Cow::Borrowed(&[Section {
        title: None,
        switch: None,
        props: Cow::Borrowed(text_props!(Title [], 14.0, "#252423")),
    }]),
};

const SUBTITLE: Card = Card {
    key: "subtitle",
    en: "Subtitle",
    fr: "Sous-titre",
    group: Group::General,
    switch: Some((To::Format("subtitle", "show"), false)),
    sections: Cow::Borrowed(&[Section {
        title: None,
        switch: None,
        props: Cow::Borrowed(text_props!(Format ["subtitle",], 10.0, "#605E5C")),
    }]),
};

const EFFECTS: Card = Card {
    key: "effects",
    en: "Effects",
    fr: "Effets",
    group: Group::General,
    switch: None,
    sections: Cow::Borrowed(&[
        Section {
            title: Some(("Background", "Arrière-plan")),
            switch: Some((To::Format("background", "show"), true)),
            props: Cow::Borrowed(&[
                p(
                    "background_color",
                    "Colour",
                    "Couleur",
                    Ctl::Color,
                    To::Format("background", "color"),
                    Def::Str("#FFFFFF"),
                ),
                p(
                    "background_transparency",
                    "Transparency",
                    "Transparence",
                    Ctl::Percent,
                    To::Format("background", "transparency"),
                    Def::Num(0.0),
                ),
            ]),
        },
        Section {
            title: Some(("Visual border", "Bordure du visuel")),
            switch: Some((To::Format("border", "show"), false)),
            props: Cow::Borrowed(&[
                p(
                    "border_color",
                    "Colour",
                    "Couleur",
                    Ctl::Color,
                    To::Format("border", "color"),
                    Def::Str("#C8C6C4"),
                ),
                p(
                    "border_radius",
                    "Rounded corners",
                    "Coins arrondis",
                    Ctl::Number(0.0, 50.0),
                    To::Format("border", "radius"),
                    Def::Num(0.0),
                ),
                p(
                    "border_width",
                    "Width",
                    "Largeur",
                    Ctl::Number(1.0, 10.0),
                    To::Format("border", "width"),
                    Def::Num(1.0),
                ),
            ]),
        },
        Section {
            title: Some(("Shadow", "Ombre")),
            switch: Some((To::Format("shadow", "show"), false)),
            props: Cow::Borrowed(&[
                p(
                    "shadow_color",
                    "Colour",
                    "Couleur",
                    Ctl::Color,
                    To::Format("shadow", "color"),
                    Def::Str("#000000"),
                ),
                p(
                    "shadow_transparency",
                    "Transparency",
                    "Transparence",
                    Ctl::Percent,
                    To::Format("shadow", "transparency"),
                    Def::Num(70.0),
                ),
            ]),
        },
    ]),
};

const HEADER: Card = Card {
    key: "header",
    en: "Header icons",
    fr: "Icônes d'en-tête",
    group: Group::General,
    switch: Some((To::Format("header", "show"), true)),
    sections: Cow::Borrowed(&[Section {
        title: None,
        switch: None,
        props: Cow::Borrowed(&[
            p(
                "header_background",
                "Background",
                "Arrière-plan",
                Ctl::Color,
                To::Object("visualHeader", "background"),
                Def::Str("#FFFFFF"),
            ),
            p(
                "header_icons",
                "Icon colour",
                "Couleur des icônes",
                Ctl::Color,
                To::Object("visualHeader", "foreground"),
                Def::Str("#252423"),
            ),
        ]),
    }]),
};

const TOOLTIPS: Card = Card {
    key: "tooltips",
    en: "Tooltips",
    fr: "Info-bulles",
    group: Group::General,
    switch: None,
    sections: Cow::Borrowed(&[Section {
        title: None,
        switch: None,
        props: Cow::Borrowed(&[p(
            "tooltip_page",
            "Page",
            "Page",
            Ctl::Pages,
            To::TooltipPage,
            Def::Str("default"),
        )]),
    }]),
};

const PROPERTIES: Card = Card {
    key: "properties",
    en: "Properties",
    fr: "Propriétés",
    group: Group::General,
    switch: None,
    sections: Cow::Borrowed(&[Section {
        title: None,
        switch: None,
        props: Cow::Borrowed(&[
            p(
                "padding",
                "Padding",
                "Marge intérieure",
                Ctl::Number(0.0, 100.0),
                To::Whole("padding"),
                Def::Num(0.0),
            ),
            p(
                "alt_text",
                "Alt text",
                "Texte de remplacement",
                Ctl::Text,
                To::Whole("alt_text"),
                Def::None,
            ),
        ]),
    }]),
};

/// The General group, in Desktop's order.
pub const GENERAL: &[Card] = &[
    TITLE,
    SUBTITLE,
    EFFECTS,
    DATA_FORMAT,
    HEADER,
    TOOLTIPS,
    PROPERTIES,
];

/// General › Data format (Desktop 2.159): a field's format for this visual only, drawn by
/// [`crate::data_format_card`] and written by `visual.set_data_format`.
const DATA_FORMAT: Card = Card {
    key: "dataFormat",
    en: "Data format",
    fr: "Format des données",
    group: Group::General,
    switch: None,
    sections: Cow::Borrowed(&[]),
};

// ----- the Visual group: cards generated from the visual catalog -----
//
// Desktop's Format visual pane shows a different set of cards for each visual type: a pie has a
// Legend and Data colours, a gauge a Gauge axis and a Target, a table Column headers and a Grid.
// The cards below are built from `bicraft_visuals`: an object and each of its rows are offered
// only when the catalog gives that object and property to the type, so the presence of a card
// follows the type exactly. A card's title is the catalog's own object name, normalised and then
// translated from its meaning (docs/design.md §5).

/// One row of a generated Visual card: one property of its object, in Desktop's order.
struct Row {
    prop: &'static str,
    en: &'static str,
    fr: &'static str,
    ctl: Ctl,
    def: Def,
}

const fn row(prop: &'static str, en: &'static str, fr: &'static str, ctl: Ctl, def: Def) -> Row {
    Row {
        prop,
        en,
        fr,
        ctl,
        def,
    }
}

/// A card of the Visual group: one formatting object of the catalog and its rows.
struct ObjSpec {
    object: &'static str,
    /// The card's title when the catalog names no object (our own words).
    en: &'static str,
    fr: &'static str,
    /// The header switch's default when the object takes `show`.
    show_def: bool,
    rows: &'static [Row],
}

// the enumerated values the cards offer, with our own labels (values as PBIR writes them)
const LABEL_POSITIONS: &[(&str, &str, &str)] = &[
    ("Auto", "Auto", "Auto"),
    ("InsideEnd", "Inside end", "Fin intérieure"),
    ("OutsideEnd", "Outside end", "Fin extérieure"),
    ("InsideCenter", "Inside center", "Centre intérieur"),
    ("InsideBase", "Inside base", "Base intérieure"),
    ("Above", "Above", "Au-dessus"),
    ("Under", "Under", "En dessous"),
];
const PIE_POSITIONS: &[(&str, &str, &str)] = &[
    ("outside", "Outside", "Extérieur"),
    ("inside", "Inside", "Intérieur"),
    ("preferOutside", "Prefer outside", "Extérieur de préférence"),
    ("preferInside", "Prefer inside", "Intérieur de préférence"),
];
const LINE_STYLES: &[(&str, &str, &str)] = &[
    ("solid", "Solid", "Continu"),
    ("dashed", "Dashed", "Tirets"),
    ("dotted", "Dotted", "Pointillés"),
    ("custom", "Custom", "Personnalisé"),
];
/// The gridlines' style: no custom dashes there (the catalog allows three values only).
const GRID_STYLES: &[(&str, &str, &str)] = &[
    ("solid", "Solid", "Continu"),
    ("dashed", "Dashed", "Tirets"),
    ("dotted", "Dotted", "Pointillés"),
];
const LINE_TYPES: &[(&str, &str, &str)] = &[
    ("linear", "Linear", "Linéaire"),
    ("smooth", "Smooth", "Lissée"),
    ("step", "Step", "En escalier"),
];
const MARKER_SHAPES: &[(&str, &str, &str)] = &[
    ("circle", "Circle", "Cercle"),
    ("square", "Square", "Carré"),
    ("diamond", "Diamond", "Losange"),
    ("triangle", "Triangle", "Triangle"),
    ("x", "X", "Croix"),
    ("shortDash", "Short dash", "Tiret court"),
    ("longDash", "Long dash", "Tiret long"),
    ("plus", "Plus", "Plus"),
];
const DISPLAY_UNIT_VALUES: &[(&str, &str, &str)] = &[
    ("0", "Auto", "Auto"),
    ("1", "None", "Aucune"),
    ("1000", "Thousands", "Milliers"),
    ("1000000", "Millions", "Millions"),
    ("1000000000", "Billions", "Milliards"),
    ("1000000000000", "Trillions", "Billions"),
];
const MAP_THEMES: &[(&str, &str, &str)] = &[
    ("aerial", "Aerial", "Aérien"),
    ("canvasDark", "Dark canvas", "Canevas sombre"),
    ("canvasLight", "Light canvas", "Canevas clair"),
    ("grayscale", "Grayscale", "Niveaux de gris"),
    ("road", "Road", "Routes"),
];
const GOAL_DIRECTIONS: &[(&str, &str, &str)] = &[
    ("High is good", "High is good", "Une valeur haute est favorable"),
    ("Low is good", "Low is good", "Une valeur basse est favorable"),
];
const GOOD_BAD: &[(&str, &str, &str)] = &[
    ("Positive", "Positive", "Positive"),
    ("Negative", "Negative", "Négative"),
];
const TOP_BOTTOM: &[(&str, &str, &str)] = &[("Top", "Top", "Haut"), ("Bottom", "Bottom", "Bas")];
const SERIES_SIDES: &[(&str, &str, &str)] = &[("Left", "Left", "Gauche"), ("Right", "Right", "Droite")];
const FILL_STYLES: &[(&str, &str, &str)] = &[
    ("Fill only", "Fill only", "Remplissage seul"),
    ("Border only", "Border only", "Bordure seule"),
];
const IMAGE_FITS: &[(&str, &str, &str)] = &[
    ("Fit", "Fit", "Ajuster"),
    ("Stretch", "Stretch", "Étirer"),
    ("Fill", "Fill", "Remplir"),
    ("Normal", "Normal", "Normal"),
];
const SPARK_TYPES: &[(&str, &str, &str)] = &[("line", "Line", "Ligne"), ("column", "Column", "Colonne")];
const AI_MODES: &[(&str, &str, &str)] = &[
    ("absolute", "Absolute", "Absolue"),
    ("relative", "Relative", "Relative"),
];
const TREE_DENSITIES: &[(&str, &str, &str)] = &[
    ("dense", "Dense", "Dense"),
    ("default", "Default", "Par défaut"),
    ("sparse", "Sparse", "Aérée"),
];
const CONNECTORS: &[(&str, &str, &str)] = &[
    ("curve", "Curved", "Courbes"),
    ("round", "Right angle", "Angle droit"),
];
const PROJECTIONS: &[(&str, &str, &str)] = &[
    ("mercator", "Mercator", "Mercator"),
    ("equirectangular", "Equirectangular", "Équirectangulaire"),
    ("orthographic", "Orthographic", "Orthographique"),
    ("albersUsa", "Albers USA", "Albers États-Unis"),
];
const RADIUS_UNITS: &[(&str, &str, &str)] = &[("pixels", "Pixels", "Pixels"), ("meters", "Meters", "Mètres")];
const LABEL_VERTICAL: &[(&str, &str, &str)] = &[
    ("aboveValue", "Above value", "Au-dessus de la valeur"),
    ("belowValue", "Below value", "En dessous de la valeur"),
];

// the rows shared by several cards
const FONT: Row = row("fontFamily", "Font", "Police", Ctl::Font, Def::None);
const SIZE: Row = row(
    "fontSize",
    "Font size",
    "Taille de police",
    Ctl::Size,
    Def::Num(9.0),
);
const TEXT_SIZE: Row = row(
    "textSize",
    "Text size",
    "Taille du texte",
    Ctl::Size,
    Def::Num(10.0),
);
const BOLD: Row = row("bold", "Bold", "Gras", Ctl::Style("B"), Def::Bool(false));
const ITALIC: Row = row("italic", "Italic", "Italique", Ctl::Style("I"), Def::Bool(false));
const UNDERLINE: Row = row(
    "underline",
    "Underline",
    "Souligné",
    Ctl::Style("U"),
    Def::Bool(false),
);
const COLOR: Row = row("fontColor", "Colour", "Couleur", Ctl::Color, Def::None);
const TEXT_COLOR: Row = row("color", "Colour", "Couleur", Ctl::Color, Def::None);
const LABEL_COLOR: Row = row("labelColor", "Colour", "Couleur", Ctl::Color, Def::None);
const FILL: Row = row("fill", "Colour", "Couleur", Ctl::Color, Def::Accent);
const BACKGROUND: Row = row("backColor", "Background", "Arrière-plan", Ctl::Color, Def::None);
const TRANSPARENCY: Row = row(
    "transparency",
    "Transparency",
    "Transparence",
    Ctl::Percent,
    Def::Num(0.0),
);
const ALIGN: Row = row(
    "horizontalAlignment",
    "Alignment",
    "Alignement",
    Ctl::Align,
    Def::Str("left"),
);
const UNITS: Row = row(
    "labelDisplayUnits",
    "Display units",
    "Unités d'affichage",
    Ctl::Numbered(DISPLAY_UNIT_VALUES),
    Def::None,
);
const PRECISION: Row = row(
    "labelPrecision",
    "Decimal places",
    "Décimales",
    Ctl::Number(0.0, 10.0),
    Def::Num(0.0),
);
const AXIS_TITLE: Row = row("showAxisTitle", "Title", "Titre", Ctl::Toggle, Def::Bool(false));
const TITLE_TEXT: Row = row("titleText", "Title text", "Texte du titre", Ctl::Text, Def::None);
const TITLE_COLOR: Row = row(
    "titleColor",
    "Title colour",
    "Couleur du titre",
    Ctl::Color,
    Def::None,
);
const TITLE_SIZE: Row = row(
    "titleFontSize",
    "Title size",
    "Taille du titre",
    Ctl::Size,
    Def::Num(11.0),
);
const GRIDLINES: Row = row(
    "gridlineShow",
    "Gridlines",
    "Quadrillage",
    Ctl::Toggle,
    Def::Bool(false),
);
const GRID_COLOR: Row = row(
    "gridlineColor",
    "Gridline colour",
    "Couleur du quadrillage",
    Ctl::Color,
    Def::None,
);
const GRID_STYLE: Row = row(
    "gridlineStyle",
    "Gridline style",
    "Style du quadrillage",
    Ctl::Choice(GRID_STYLES),
    Def::None,
);
const GRID_WIDTH: Row = row(
    "gridlineThickness",
    "Gridline width",
    "Épaisseur du quadrillage",
    Ctl::Number(1.0, 10.0),
    Def::Num(1.0),
);
const BORDER_SHOW: Row = row("borderShow", "Border", "Bordure", Ctl::Toggle, Def::Bool(false));
const BORDER_COLOR: Row = row(
    "borderColor",
    "Border colour",
    "Couleur de la bordure",
    Ctl::Color,
    Def::None,
);
const BORDER_WIDTH: Row = row(
    "borderSize",
    "Border width",
    "Épaisseur de la bordure",
    Ctl::Number(0.0, 10.0),
    Def::Num(1.0),
);

/// The Visual group's objects, in the order Desktop lists their cards.
const VISUAL_OBJECTS: &[ObjSpec] = &[
    ObjSpec {
        object: "legend",
        en: "Legend",
        fr: "Légende",
        show_def: true,
        rows: &[
            row(
                "position",
                "Position",
                "Position",
                Ctl::Choice(LEGEND_POSITIONS),
                Def::None,
            ),
            FONT,
            SIZE,
            BOLD,
            ITALIC,
            UNDERLINE,
            LABEL_COLOR,
            row("showTitle", "Title", "Titre", Ctl::Toggle, Def::Bool(false)),
            row("titleText", "Title text", "Texte du titre", Ctl::Text, Def::None),
        ],
    },
    ObjSpec {
        object: "categoryAxis",
        en: "X-axis",
        fr: "Axe X",
        show_def: true,
        rows: &[
            FONT,
            SIZE,
            BOLD,
            ITALIC,
            UNDERLINE,
            LABEL_COLOR,
            AXIS_TITLE,
            TITLE_TEXT,
            TITLE_COLOR,
            TITLE_SIZE,
        ],
    },
    ObjSpec {
        object: "valueAxis",
        en: "Y-axis",
        fr: "Axe Y",
        show_def: true,
        rows: &[
            FONT,
            SIZE,
            BOLD,
            ITALIC,
            UNDERLINE,
            LABEL_COLOR,
            UNITS,
            PRECISION,
            GRIDLINES,
            GRID_COLOR,
            GRID_STYLE,
            GRID_WIDTH,
            AXIS_TITLE,
            TITLE_TEXT,
            TITLE_COLOR,
            TITLE_SIZE,
        ],
    },
    ObjSpec {
        object: "axis",
        en: "Gauge axis",
        fr: "Axe de la jauge",
        show_def: false,
        rows: &[
            row(
                "min",
                "Minimum",
                "Minimum",
                Ctl::Number(-1.0e9, 1.0e9),
                Def::Num(0.0),
            ),
            row(
                "max",
                "Maximum",
                "Maximum",
                Ctl::Number(-1.0e9, 1.0e9),
                Def::Num(100.0),
            ),
            row(
                "target",
                "Target value",
                "Valeur cible",
                Ctl::Number(-1.0e9, 1.0e9),
                Def::None,
            ),
        ],
    },
    ObjSpec {
        object: "calloutValue",
        en: "Callout value",
        fr: "Valeur de légende",
        show_def: true,
        rows: &[FONT, BOLD, ITALIC, UNDERLINE, TEXT_COLOR, UNITS, PRECISION],
    },
    ObjSpec {
        object: "target",
        en: "Target",
        fr: "Cible",
        show_def: true,
        rows: &[FONT, SIZE, BOLD, ITALIC, UNDERLINE, TEXT_COLOR, UNITS, PRECISION],
    },
    ObjSpec {
        object: "labels",
        en: "Data labels",
        fr: "Étiquettes de données",
        show_def: false,
        rows: &[
            row(
                "labelPosition",
                "Position",
                "Position",
                Ctl::Choice(LABEL_POSITIONS),
                Def::None,
            ),
            row(
                "position",
                "Position",
                "Position",
                Ctl::Choice(PIE_POSITIONS),
                Def::None,
            ),
            FONT,
            SIZE,
            BOLD,
            ITALIC,
            UNDERLINE,
            TEXT_COLOR,
            row(
                "backgroundColor",
                "Background",
                "Arrière-plan",
                Ctl::Color,
                Def::None,
            ),
            row(
                "transparency",
                "Transparency",
                "Transparence",
                Ctl::Percent,
                Def::Num(0.0),
            ),
            UNITS,
            PRECISION,
        ],
    },
    ObjSpec {
        object: "seriesLabels",
        en: "Series labels",
        fr: "Étiquettes de séries",
        show_def: true,
        rows: &[
            row("seriesFontFamily", "Font", "Police", Ctl::Font, Def::None),
            row(
                "textSize",
                "Text size",
                "Taille du texte",
                Ctl::Size,
                Def::Num(9.0),
            ),
            BOLD,
            ITALIC,
            UNDERLINE,
            row("seriesColor", "Colour", "Couleur", Ctl::Color, Def::None),
            row(
                "seriesPosition",
                "Position",
                "Position",
                Ctl::Choice(SERIES_SIDES),
                Def::None,
            ),
        ],
    },
    ObjSpec {
        object: "lineStyles",
        en: "Lines",
        fr: "Lignes",
        show_def: false,
        rows: &[
            row(
                "lineChartType",
                "Line type",
                "Type de ligne",
                Ctl::Choice(LINE_TYPES),
                Def::None,
            ),
            row(
                "lineStyle",
                "Line style",
                "Style de ligne",
                Ctl::Choice(LINE_STYLES),
                Def::None,
            ),
            row(
                "strokeWidth",
                "Width",
                "Épaisseur",
                Ctl::Number(0.0, 20.0),
                Def::None,
            ),
            row("strokeColor", "Colour", "Couleur", Ctl::Color, Def::None),
            row(
                "showMarker",
                "Markers",
                "Marqueurs",
                Ctl::Toggle,
                Def::Bool(false),
            ),
            row(
                "markerShape",
                "Marker shape",
                "Forme du marqueur",
                Ctl::Choice(MARKER_SHAPES),
                Def::None,
            ),
            row(
                "markerSize",
                "Marker size",
                "Taille du marqueur",
                Ctl::Number(1.0, 50.0),
                Def::None,
            ),
            row(
                "markerColor",
                "Marker colour",
                "Couleur du marqueur",
                Ctl::Color,
                Def::None,
            ),
        ],
    },
    ObjSpec {
        object: "markers",
        en: "Markers",
        fr: "Marqueurs",
        show_def: false,
        rows: &[BORDER_SHOW, BORDER_COLOR, BORDER_WIDTH, TRANSPARENCY],
    },
    ObjSpec {
        object: "dataPoint",
        en: "Data colours",
        fr: "Couleurs des données",
        show_def: false,
        rows: &[
            FILL,
            BORDER_SHOW,
            BORDER_COLOR,
            BORDER_WIDTH,
            row(
                "borderTransparency",
                "Border transparency",
                "Transparence de la bordure",
                Ctl::Percent,
                Def::Num(0.0),
            ),
            row(
                "fillTransparency",
                "Transparency",
                "Transparence",
                Ctl::Percent,
                Def::Num(0.0),
            ),
            row(
                "showAllDataPoints",
                "Show all",
                "Afficher tout",
                Ctl::Toggle,
                Def::Bool(false),
            ),
        ],
    },
    ObjSpec {
        object: "percentBarLabel",
        en: "Conversion rate labels",
        fr: "Étiquettes de taux de conversion",
        show_def: true,
        rows: &[FONT, SIZE, BOLD, ITALIC, UNDERLINE, TEXT_COLOR],
    },
    ObjSpec {
        object: "breakdown",
        en: "Breakdown",
        fr: "Décomposition",
        show_def: false,
        rows: &[row(
            "maxBreakdowns",
            "Maximum breakdowns",
            "Décompositions maximales",
            Ctl::Number(1.0, 10.0),
            Def::Num(10.0),
        )],
    },
    ObjSpec {
        object: "sentimentColors",
        en: "Sentiment colours",
        fr: "Couleurs de sentiment",
        show_def: false,
        rows: &[
            row("increaseFill", "Increase", "Augmentation", Ctl::Color, Def::None),
            row("decreaseFill", "Decrease", "Diminution", Ctl::Color, Def::None),
            row("totalFill", "Total", "Total", Ctl::Color, Def::None),
            row("otherFill", "Other", "Autre", Ctl::Color, Def::None),
        ],
    },
    ObjSpec {
        object: "categoryLabels",
        en: "Category label",
        fr: "Étiquette de catégorie",
        show_def: true,
        rows: &[FONT, SIZE, BOLD, ITALIC, UNDERLINE, TEXT_COLOR],
    },
    ObjSpec {
        object: "value",
        en: "Value",
        fr: "Valeur",
        show_def: true,
        rows: &[
            FONT, SIZE, BOLD, ITALIC, UNDERLINE, COLOR, UNITS, PRECISION, ALIGN,
        ],
    },
    ObjSpec {
        object: "label",
        en: "Label",
        fr: "Étiquette",
        show_def: true,
        rows: &[
            FONT,
            SIZE,
            BOLD,
            ITALIC,
            UNDERLINE,
            COLOR,
            ALIGN,
            row(
                "position",
                "Position",
                "Position",
                Ctl::Choice(LABEL_VERTICAL),
                Def::None,
            ),
            row("text", "Text", "Texte", Ctl::Text, Def::None),
        ],
    },
    ObjSpec {
        object: "card",
        en: "Card",
        fr: "Carte",
        show_def: false,
        rows: &[
            row(
                "barShow",
                "Accent bar",
                "Barre d'accent",
                Ctl::Toggle,
                Def::Bool(false),
            ),
            row(
                "barColor",
                "Accent bar colour",
                "Couleur de la barre d'accent",
                Ctl::Color,
                Def::None,
            ),
            row(
                "barWeight",
                "Accent bar width",
                "Épaisseur de la barre d'accent",
                Ctl::Number(0.0, 20.0),
                Def::None,
            ),
            row(
                "cardBackground",
                "Background",
                "Arrière-plan",
                Ctl::Color,
                Def::None,
            ),
            row(
                "cardPadding",
                "Padding",
                "Marge intérieure",
                Ctl::Number(0.0, 50.0),
                Def::None,
            ),
            row(
                "outlineColor",
                "Outline colour",
                "Couleur du contour",
                Ctl::Color,
                Def::None,
            ),
        ],
    },
    ObjSpec {
        object: "cardTitle",
        en: "Card title",
        fr: "Titre de la carte",
        show_def: false,
        rows: &[FONT, SIZE, BOLD, ITALIC, UNDERLINE, TEXT_COLOR],
    },
    ObjSpec {
        object: "dataLabels",
        en: "Data labels",
        fr: "Étiquettes de données",
        show_def: false,
        rows: &[FONT, SIZE, BOLD, ITALIC, UNDERLINE, TEXT_COLOR],
    },
    ObjSpec {
        object: "values",
        en: "Values",
        fr: "Valeurs",
        show_def: false,
        rows: &[
            FONT,
            SIZE,
            BOLD,
            ITALIC,
            UNDERLINE,
            row(
                "fontColor",
                "Text colour",
                "Couleur du texte",
                Ctl::Color,
                Def::None,
            ),
            row("backColor", "Background", "Arrière-plan", Ctl::Color, Def::None),
            row(
                "backColorSecondary",
                "Alternate background",
                "Arrière-plan alterné",
                Ctl::Color,
                Def::None,
            ),
        ],
    },
    ObjSpec {
        object: "columnHeaders",
        en: "Column headers",
        fr: "En-têtes de colonnes",
        show_def: false,
        rows: &[FONT, SIZE, BOLD, ITALIC, UNDERLINE, COLOR, BACKGROUND],
    },
    ObjSpec {
        object: "rowHeaders",
        en: "Row headers",
        fr: "En-têtes de lignes",
        show_def: false,
        rows: &[FONT, SIZE, BOLD, ITALIC, UNDERLINE, COLOR, BACKGROUND],
    },
    ObjSpec {
        object: "grid",
        en: "Grid",
        fr: "Grille",
        show_def: false,
        rows: &[
            row(
                "gridVertical",
                "Vertical gridlines",
                "Quadrillage vertical",
                Ctl::Toggle,
                Def::Bool(false),
            ),
            row(
                "gridVerticalColor",
                "Vertical colour",
                "Couleur verticale",
                Ctl::Color,
                Def::None,
            ),
            row(
                "gridVerticalWeight",
                "Vertical width",
                "Épaisseur verticale",
                Ctl::Number(0.0, 10.0),
                Def::None,
            ),
            row(
                "gridHorizontal",
                "Horizontal gridlines",
                "Quadrillage horizontal",
                Ctl::Toggle,
                Def::Bool(false),
            ),
            row(
                "gridHorizontalColor",
                "Horizontal colour",
                "Couleur horizontale",
                Ctl::Color,
                Def::None,
            ),
            row(
                "gridHorizontalWeight",
                "Horizontal width",
                "Épaisseur horizontale",
                Ctl::Number(0.0, 10.0),
                Def::None,
            ),
            row(
                "rowPadding",
                "Row padding",
                "Marge des lignes",
                Ctl::Number(0.0, 20.0),
                Def::None,
            ),
            row(
                "outlineColor",
                "Outline colour",
                "Couleur du contour",
                Ctl::Color,
                Def::None,
            ),
            row(
                "outlineWeight",
                "Outline width",
                "Épaisseur du contour",
                Ctl::Number(0.0, 10.0),
                Def::None,
            ),
        ],
    },
    ObjSpec {
        object: "total",
        en: "Total",
        fr: "Total",
        show_def: false,
        rows: &[
            FONT,
            SIZE,
            BOLD,
            ITALIC,
            UNDERLINE,
            COLOR,
            BACKGROUND,
            row("label", "Label", "Libellé", Ctl::Text, Def::None),
            row("totals", "Totals", "Totaux", Ctl::Toggle, Def::Bool(true)),
        ],
    },
    ObjSpec {
        object: "subTotals",
        en: "Subtotals",
        fr: "Sous-totaux",
        show_def: false,
        rows: &[
            FONT, SIZE, BOLD, ITALIC, UNDERLINE, COLOR,
            // the switches, per-level settings, position and labels are the Row subtotals and
            // Column subtotals cards' ([`SUBTOTAL_CARDS`])
            BACKGROUND,
        ],
    },
    ObjSpec {
        object: "columnTotal",
        en: "Column grand total",
        fr: "Total général des colonnes",
        show_def: false,
        rows: &[FONT, SIZE, BOLD, ITALIC, UNDERLINE, COLOR, BACKGROUND],
    },
    ObjSpec {
        object: "rowTotal",
        en: "Row grand total",
        fr: "Total général des lignes",
        show_def: false,
        rows: &[FONT, SIZE, BOLD, ITALIC, UNDERLINE, COLOR, BACKGROUND],
    },
    ObjSpec {
        object: "sparklines",
        en: "Sparklines",
        fr: "Graphiques sparkline",
        show_def: false,
        rows: &[
            row(
                "chartType",
                "Chart type",
                "Type de graphique",
                Ctl::Choice(SPARK_TYPES),
                Def::None,
            ),
            row("dataColor", "Colour", "Couleur", Ctl::Color, Def::None),
            row(
                "markerColor",
                "Marker colour",
                "Couleur des marqueurs",
                Ctl::Color,
                Def::None,
            ),
            row(
                "markerShape",
                "Marker shape",
                "Forme des marqueurs",
                Ctl::Choice(MARKER_SHAPES),
                Def::None,
            ),
            row(
                "markerSize",
                "Marker size",
                "Taille des marqueurs",
                Ctl::Number(1.0, 50.0),
                Def::None,
            ),
            row(
                "strokeWidth",
                "Width",
                "Épaisseur",
                Ctl::Number(0.0, 10.0),
                Def::None,
            ),
            row(
                "markers",
                // bits as Desktop 2.159.1231.0 writes them: 1 every point … 128 isolated
                "Markers",
                "Marqueurs",
                Ctl::Number(0.0, 255.0),
                Def::None,
            ),
        ],
    },
    ObjSpec {
        object: "header",
        en: "Slicer header",
        fr: "En-tête du segment",
        show_def: true,
        rows: &[
            FONT,
            TEXT_SIZE,
            BOLD,
            ITALIC,
            UNDERLINE,
            COLOR,
            row("background", "Background", "Arrière-plan", Ctl::Color, Def::None),
            row("text", "Text", "Texte", Ctl::Text, Def::None),
            row(
                "showRestatement",
                "Restatement",
                "Rappel de la sélection",
                Ctl::Toggle,
                Def::Bool(false),
            ),
        ],
    },
    ObjSpec {
        object: "items",
        en: "Items",
        fr: "Éléments",
        show_def: false,
        rows: &[
            FONT,
            TEXT_SIZE,
            BOLD,
            ITALIC,
            UNDERLINE,
            COLOR,
            row("background", "Background", "Arrière-plan", Ctl::Color, Def::None),
            row(
                "padding",
                "Padding",
                "Marge intérieure",
                Ctl::Number(0.0, 20.0),
                Def::None,
            ),
        ],
    },
    ObjSpec {
        object: "selection",
        en: "Selection controls",
        fr: "Contrôles de sélection",
        show_def: false,
        rows: &[
            row(
                "singleSelect",
                "Single select",
                "Sélection unique",
                Ctl::Toggle,
                Def::Bool(false),
            ),
            row(
                "selectAllCheckboxEnabled",
                "Select all",
                "Tout sélectionner",
                Ctl::Toggle,
                Def::Bool(false),
            ),
            row(
                "strictSingleSelect",
                "Require a selection",
                "Sélection obligatoire",
                Ctl::Toggle,
                Def::Bool(false),
            ),
        ],
    },
    ObjSpec {
        object: "slider",
        en: "Slider",
        fr: "Curseur",
        show_def: true,
        rows: &[
            row("color", "Colour", "Couleur", Ctl::Color, Def::None),
            row(
                "handleFillColor",
                "Handle colour",
                "Couleur du bouton",
                Ctl::Color,
                Def::None,
            ),
            row(
                "handleBorderColor",
                "Handle border colour",
                "Couleur de la bordure du bouton",
                Ctl::Color,
                Def::None,
            ),
            row(
                "secondaryLineColor",
                "Secondary colour",
                "Couleur secondaire",
                Ctl::Color,
                Def::None,
            ),
        ],
    },
    ObjSpec {
        object: "dropdown",
        en: "Dropdown",
        fr: "Liste déroulante",
        show_def: false,
        rows: &[
            row(
                "iconColor",
                "Icon colour",
                "Couleur de l'icône",
                Ctl::Color,
                Def::None,
            ),
            row("iconSize", "Icon size", "Taille de l'icône", Ctl::Size, Def::None),
            row(
                "borderColor",
                "Border colour",
                "Couleur de la bordure",
                Ctl::Color,
                Def::None,
            ),
            row(
                "borderRadius",
                "Corner radius",
                "Rayon des coins",
                Ctl::Number(0.0, 50.0),
                Def::None,
            ),
            row(
                "accentBarColor",
                "Accent bar colour",
                "Couleur de la barre d'accent",
                Ctl::Color,
                Def::None,
            ),
            row(
                "accentBarWidth",
                "Accent bar width",
                "Épaisseur de la barre d'accent",
                Ctl::Number(0.0, 20.0),
                Def::None,
            ),
        ],
    },
    ObjSpec {
        object: "date",
        en: "Date inputs",
        fr: "Saisies de date",
        show_def: false,
        rows: &[
            FONT,
            TEXT_SIZE,
            BOLD,
            ITALIC,
            UNDERLINE,
            COLOR,
            row("background", "Background", "Arrière-plan", Ctl::Color, Def::None),
            row(
                "hideDatePickerButton",
                "Date picker",
                "Sélecteur de date",
                Ctl::Toggle,
                Def::Bool(false),
            ),
        ],
    },
    ObjSpec {
        object: "searchBox",
        en: "Search box",
        fr: "Zone de recherche",
        show_def: false,
        rows: &[
            row("background", "Background", "Arrière-plan", Ctl::Color, Def::None),
            row(
                "borderColor",
                "Border colour",
                "Couleur de la bordure",
                Ctl::Color,
                Def::None,
            ),
        ],
    },
    ObjSpec {
        object: "inputText",
        en: "Input text",
        fr: "Texte de saisie",
        show_def: false,
        rows: &[
            FONT,
            SIZE,
            BOLD,
            ITALIC,
            UNDERLINE,
            COLOR,
            row(
                "placeholder",
                "Placeholder",
                "Texte indicatif",
                Ctl::Text,
                Def::None,
            ),
            row("backColor", "Background", "Arrière-plan", Ctl::Color, Def::None),
            row(
                "borderColor",
                "Border colour",
                "Couleur de la bordure",
                Ctl::Color,
                Def::None,
            ),
            row(
                "borderWidth",
                "Border width",
                "Épaisseur de la bordure",
                Ctl::Number(0.0, 10.0),
                Def::None,
            ),
        ],
    },
    ObjSpec {
        object: "applyButton",
        en: "Apply button",
        fr: "Bouton Appliquer",
        show_def: false,
        rows: &[
            row("backColor", "Background", "Arrière-plan", Ctl::Color, Def::None),
            row(
                "borderColor",
                "Border colour",
                "Couleur de la bordure",
                Ctl::Color,
                Def::None,
            ),
            row(
                "borderWidth",
                "Border width",
                "Épaisseur de la bordure",
                Ctl::Number(0.0, 10.0),
                Def::None,
            ),
            row(
                "iconColor",
                "Icon colour",
                "Couleur de l'icône",
                Ctl::Color,
                Def::None,
            ),
            row("iconSize", "Icon size", "Taille de l'icône", Ctl::Size, Def::None),
        ],
    },
    ObjSpec {
        object: "slicerSettings",
        en: "Slicer settings",
        fr: "Paramètres du segment",
        show_def: false,
        rows: &[
            row(
                "multiselect",
                "Multi-select",
                "Sélection multiple",
                Ctl::Toggle,
                Def::Bool(true),
            ),
            row(
                "numericInputOnly",
                "Numbers only",
                "Nombres uniquement",
                Ctl::Toggle,
                Def::Bool(false),
            ),
        ],
    },
    ObjSpec {
        object: "indicator",
        en: "Indicator",
        fr: "Indicateur",
        show_def: false,
        rows: &[
            FONT,
            SIZE,
            BOLD,
            ITALIC,
            UNDERLINE,
            COLOR,
            row("iconSize", "Icon size", "Taille de l'icône", Ctl::Size, Def::None),
            row("showIcon", "Icon", "Icône", Ctl::Toggle, Def::Bool(true)),
            ALIGN,
            UNITS,
            PRECISION,
        ],
    },
    ObjSpec {
        object: "trendline",
        en: "Trend axis",
        fr: "Axe de tendance",
        show_def: true,
        rows: &[TRANSPARENCY],
    },
    ObjSpec {
        object: "goals",
        en: "Goals",
        fr: "Objectifs",
        show_def: false,
        rows: &[
            row(
                "goalText",
                "Goal text",
                "Texte de l'objectif",
                Ctl::Text,
                Def::None,
            ),
            row(
                "goalFontColor",
                "Goal colour",
                "Couleur de l'objectif",
                Ctl::Color,
                Def::None,
            ),
            row("showGoal", "Goal", "Objectif", Ctl::Toggle, Def::Bool(true)),
            row("showDistance", "Distance", "Écart", Ctl::Toggle, Def::Bool(true)),
            row(
                "distanceFontColor",
                "Distance colour",
                "Couleur de l'écart",
                Ctl::Color,
                Def::None,
            ),
            row(
                "direction",
                "Direction",
                "Sens",
                Ctl::Choice(GOAL_DIRECTIONS),
                Def::None,
            ),
            PRECISION,
        ],
    },
    ObjSpec {
        object: "status",
        en: "Color coding",
        fr: "Code couleur",
        show_def: false,
        rows: &[
            row(
                "goodColor",
                "Good colour",
                "Couleur favorable",
                Ctl::Color,
                Def::None,
            ),
            row(
                "neutralColor",
                "Neutral colour",
                "Couleur neutre",
                Ctl::Color,
                Def::None,
            ),
            row(
                "badColor",
                "Bad colour",
                "Couleur défavorable",
                Ctl::Color,
                Def::None,
            ),
            row("direction", "Direction", "Sens", Ctl::Choice(GOOD_BAD), Def::None),
        ],
    },
    ObjSpec {
        object: "lastDate",
        en: "Date",
        fr: "Date",
        show_def: true,
        rows: &[
            FONT,
            SIZE,
            BOLD,
            ITALIC,
            UNDERLINE,
            row("lastDateFontColor", "Colour", "Couleur", Ctl::Color, Def::None),
        ],
    },
    ObjSpec {
        object: "mapControls",
        en: "Map controls",
        fr: "Commandes de la carte",
        show_def: false,
        rows: &[row(
            "defaultStyle",
            "Theme",
            "Thème",
            Ctl::Choice(MAP_THEMES),
            Def::None,
        )],
    },
    ObjSpec {
        object: "mapStyles",
        en: "Map styles",
        fr: "Styles de la carte",
        show_def: false,
        rows: &[
            row(
                "mapTheme",
                "Map theme",
                "Thème de la carte",
                Ctl::Choice(MAP_THEMES),
                Def::None,
            ),
            row("showLabels", "Labels", "Étiquettes", Ctl::Toggle, Def::Bool(true)),
        ],
    },
    ObjSpec {
        object: "bubbles",
        en: "Bubbles",
        fr: "Bulles",
        show_def: false,
        rows: &[
            row(
                "markerShape",
                "Shape",
                "Forme",
                Ctl::Choice(MARKER_SHAPES),
                Def::None,
            ),
            row("bubbleSize", "Size", "Taille", Ctl::Number(0.0, 100.0), Def::None),
            row(
                "preventOverflow",
                "Prevent overflow",
                "Éviter le débordement",
                Ctl::Toggle,
                Def::Bool(false),
            ),
            row("showSeries", "Series", "Séries", Ctl::Toggle, Def::Bool(false)),
        ],
    },
    ObjSpec {
        object: "plotArea",
        en: "Plot area",
        fr: "Zone de tracé",
        show_def: false,
        rows: &[TRANSPARENCY],
    },
    ObjSpec {
        object: "smallMultiplesLayout",
        en: "Grid layout",
        fr: "Disposition de la grille",
        show_def: false,
        rows: &[
            row(
                "columnCount",
                "Columns",
                "Colonnes",
                Ctl::Number(1.0, 20.0),
                Def::None,
            ),
            row("rowCount", "Rows", "Lignes", Ctl::Number(1.0, 20.0), Def::None),
            row(
                "backgroundColor",
                "Background",
                "Arrière-plan",
                Ctl::Color,
                Def::None,
            ),
            row(
                "gridLineShow",
                "Gridlines",
                "Quadrillage",
                Ctl::Toggle,
                Def::Bool(false),
            ),
            row(
                "gridLineColor",
                "Gridline colour",
                "Couleur du quadrillage",
                Ctl::Color,
                Def::None,
            ),
            row(
                "gridLineStyle",
                "Gridline style",
                "Style du quadrillage",
                Ctl::Choice(GRID_STYLES),
                Def::None,
            ),
        ],
    },
    ObjSpec {
        object: "ribbonBands",
        en: "Ribbons",
        fr: "Rubans",
        show_def: true,
        rows: &[
            // Learn, "Use ribbon charts": Match series color (off: grey ribbons), Transparency
            // (30 by default), Border (off by default)
            row(
                "fillMatchColor",
                "Match series colour",
                "Couleur de la série",
                Ctl::Toggle,
                Def::Bool(true),
            ),
            row("fillColor", "Colour", "Couleur", Ctl::Color, Def::None),
            row(
                "fillTransparency",
                "Transparency",
                "Transparence",
                Ctl::Percent,
                Def::Num(30.0),
            ),
            BORDER_SHOW,
            BORDER_COLOR,
            BORDER_WIDTH,
        ],
    },
    // the ribbon chart's Spacing: a percentage of the tallest column (Learn)
    ObjSpec {
        object: "layout",
        en: "Spacing",
        fr: "Espacement",
        show_def: false,
        rows: &[row(
            "ribbonGapSize",
            "Spacing",
            "Espacement",
            Ctl::Number(0.0, 100.0),
            Def::Num(0.0),
        )],
    },
    ObjSpec {
        object: "totals",
        en: "Total labels",
        fr: "Étiquettes de total",
        show_def: false,
        rows: &[FONT, SIZE, BOLD, TEXT_COLOR, UNITS, PRECISION],
    },
    // the Gantt visual (docs/formats/gantt.md): the theme schema's objects, our own labels
    ObjSpec {
        object: "bar",
        en: "Bars",
        fr: "Barres",
        show_def: true,
        rows: &[
            row("color", "Colour", "Couleur", Ctl::Color, Def::None),
            row(
                "transparency",
                "Transparency",
                "Transparence",
                Ctl::Percent,
                Def::Num(0.0),
            ),
            row("height", "Height", "Hauteur", Ctl::Number(1.0, 100.0), Def::None),
            row(
                "cornerRadius",
                "Corner radius",
                "Rayon des angles",
                Ctl::Number(0.0, 50.0),
                Def::None,
            ),
            BORDER_SHOW,
        ],
    },
    ObjSpec {
        object: "milestone",
        en: "Milestones",
        fr: "Jalons",
        show_def: true,
        rows: &[
            row("color", "Colour", "Couleur", Ctl::Color, Def::None),
            row(
                "transparency",
                "Transparency",
                "Transparence",
                Ctl::Percent,
                Def::Num(0.0),
            ),
            row("size", "Size", "Taille", Ctl::Number(1.0, 100.0), Def::None),
            BORDER_SHOW,
        ],
    },
    ObjSpec {
        object: "progress",
        en: "Progress",
        fr: "Avancement",
        show_def: true,
        rows: &[
            row("color", "Colour", "Couleur", Ctl::Color, Def::None),
            row(
                "transparency",
                "Transparency",
                "Transparence",
                Ctl::Percent,
                Def::None,
            ),
            row(
                "relativeHeight",
                "Height (% of the bar)",
                "Hauteur (% de la barre)",
                Ctl::Percent,
                Def::None,
            ),
            BORDER_SHOW,
        ],
    },
    ObjSpec {
        object: "row",
        en: "Rows",
        fr: "Lignes",
        show_def: false,
        rows: &[
            row("color", "Colour", "Couleur", Ctl::Color, Def::None),
            row(
                "colorByCategory",
                "More granular",
                "Plus détaillé",
                Ctl::Toggle,
                Def::Bool(false),
            ),
            row(
                "backgroundShow",
                "Background",
                "Arrière-plan",
                Ctl::Toggle,
                Def::None,
            ),
            row(
                "backgroundTransparency",
                "Background transparency",
                "Transparence de l'arrière-plan",
                Ctl::Percent,
                Def::None,
            ),
            row("dividerShow", "Dividers", "Séparateurs", Ctl::Toggle, Def::None),
        ],
    },
    ObjSpec {
        object: "axis",
        en: "Axis",
        fr: "Axe",
        show_def: false,
        rows: &[
            row(
                "autoScale",
                "Auto-scale",
                "Échelle automatique",
                Ctl::Toggle,
                Def::Bool(true),
            ),
            row(
                "invertAxis",
                "Invert range",
                "Inverser la plage",
                Ctl::Toggle,
                Def::Bool(false),
            ),
            row("gridlineShow", "Gridlines", "Quadrillage", Ctl::Toggle, Def::None),
            row(
                "shadingShow",
                "Shade days",
                "Griser des jours",
                Ctl::Toggle,
                Def::None,
            ),
            row(
                "zoomShow",
                "Zoom slider",
                "Curseur de zoom",
                Ctl::Toggle,
                Def::None,
            ),
        ],
    },
    ObjSpec {
        object: "dataBars",
        en: "Bars",
        fr: "Barres",
        show_def: false,
        rows: &[
            row(
                "positiveBarColor",
                "Positive colour",
                "Couleur positive",
                Ctl::Color,
                Def::None,
            ),
            row(
                "negativeBarColor",
                "Negative colour",
                "Couleur négative",
                Ctl::Color,
                Def::None,
            ),
            row("dataBarColor", "Colour", "Couleur", Ctl::Color, Def::None),
            row(
                "dataBarBackgroundColor",
                "Background",
                "Arrière-plan",
                Ctl::Color,
                Def::None,
            ),
            row(
                "dataBarWidthPercent",
                "Width",
                "Largeur",
                Ctl::Number(1.0, 100.0),
                Def::None,
            ),
        ],
    },
    ObjSpec {
        object: "tree",
        en: "Tree",
        fr: "Arborescence",
        show_def: false,
        rows: &[
            row(
                "accentColor",
                "Accent colour",
                "Couleur d'accent",
                Ctl::Color,
                Def::None,
            ),
            row(
                "connectorDefaultColor",
                "Connector colour",
                "Couleur des connecteurs",
                Ctl::Color,
                Def::None,
            ),
            row(
                "connectorType",
                "Connector",
                "Connecteurs",
                Ctl::Choice(CONNECTORS),
                Def::None,
            ),
            row(
                "density",
                "Density",
                "Densité",
                Ctl::Choice(TREE_DENSITIES),
                Def::None,
            ),
            row(
                "responsiveLayout",
                "Responsive",
                "Disposition adaptative",
                Ctl::Toggle,
                Def::Bool(true),
            ),
        ],
    },
    ObjSpec {
        object: "analysis",
        en: "Analysis",
        fr: "Analyse",
        show_def: false,
        rows: &[
            row(
                "aiEnabled",
                "AI insights",
                "Analyses par IA",
                Ctl::Toggle,
                Def::Bool(false),
            ),
            row(
                "aiMode",
                "AI mode",
                "Mode d'analyse",
                Ctl::Choice(AI_MODES),
                Def::None,
            ),
        ],
    },
    ObjSpec {
        object: "keyInfluencersVisual",
        en: "Analysis visual colours",
        fr: "Couleurs du visuel d'analyse",
        show_def: false,
        rows: &[
            row(
                "canvasColor",
                "Canvas colour",
                "Couleur du canevas",
                Ctl::Color,
                Def::None,
            ),
            row(
                "fontColor",
                "Text colour",
                "Couleur du texte",
                Ctl::Color,
                Def::None,
            ),
            row(
                "primaryColor",
                "Primary colour",
                "Couleur principale",
                Ctl::Color,
                Def::None,
            ),
            row(
                "primaryFontColor",
                "Primary text colour",
                "Couleur du texte principal",
                Ctl::Color,
                Def::None,
            ),
            row(
                "secondaryColor",
                "Secondary colour",
                "Couleur secondaire",
                Ctl::Color,
                Def::None,
            ),
            row(
                "secondaryFontColor",
                "Secondary text colour",
                "Couleur du texte secondaire",
                Ctl::Color,
                Def::None,
            ),
        ],
    },
    ObjSpec {
        object: "keyDriversDrillVisual",
        en: "Drill visual colours",
        fr: "Couleurs du visuel d'exploration",
        show_def: false,
        rows: &[
            row("defaultColor", "Colour", "Couleur", Ctl::Color, Def::None),
            row(
                "referenceLineColor",
                "Reference line colour",
                "Couleur de la ligne de référence",
                Ctl::Color,
                Def::None,
            ),
        ],
    },
    ObjSpec {
        object: "cardCalloutArea",
        en: "Callout area",
        fr: "Zone de légende",
        show_def: true,
        rows: &[
            row(
                "backgroundFillColor",
                "Background",
                "Arrière-plan",
                Ctl::Color,
                Def::None,
            ),
            row(
                "backgroundTransparency",
                "Transparency",
                "Transparence",
                Ctl::Percent,
                Def::Num(0.0),
            ),
            row(
                "paddingUniform",
                "Padding",
                "Marge intérieure",
                Ctl::Number(0.0, 50.0),
                Def::None,
            ),
        ],
    },
    ObjSpec {
        object: "cardImage",
        en: "Images",
        fr: "Images",
        show_def: false,
        rows: &[
            row(
                "imageDataFit",
                "Image fit",
                "Ajustement de l'image",
                Ctl::Choice(IMAGE_FITS),
                Def::None,
            ),
            row(
                "addBackground",
                "Background",
                "Arrière-plan",
                Ctl::Toggle,
                Def::Bool(false),
            ),
            row(
                "backgroundFillColor",
                "Background colour",
                "Couleur d'arrière-plan",
                Ctl::Color,
                Def::None,
            ),
            row(
                "altText",
                "Alt text",
                "Texte de remplacement",
                Ctl::Text,
                Def::None,
            ),
        ],
    },
    ObjSpec {
        object: "slices",
        en: "Shapes",
        fr: "Formes",
        show_def: false,
        rows: &[
            row(
                "startAngle",
                "Start angle",
                "Angle de départ",
                Ctl::Number(0.0, 360.0),
                Def::Num(0.0),
            ),
            row(
                "innerRadiusRatio",
                "Inner radius",
                "Rayon intérieur",
                Ctl::Number(0.0, 100.0),
                Def::Num(50.0),
            ),
        ],
    },
    ObjSpec {
        object: "centerValue",
        en: "Center value",
        fr: "Valeur centrale",
        show_def: true,
        rows: &[],
    },
    ObjSpec {
        object: "zoom",
        en: "Zoom",
        fr: "Zoom",
        show_def: false,
        rows: &[
            row(
                "showLabels",
                "Labels",
                "Étiquettes",
                Ctl::Toggle,
                Def::Bool(false),
            ),
            row(
                "showTooltip",
                "Tooltip",
                "Info-bulle",
                Ctl::Toggle,
                Def::Bool(false),
            ),
            row(
                "showOnCategoryAxis",
                "Category axis",
                "Axe des catégories",
                Ctl::Toggle,
                Def::Bool(false),
            ),
            row(
                "showOnValueAxis",
                "Value axis",
                "Axe des valeurs",
                Ctl::Toggle,
                Def::Bool(false),
            ),
        ],
    },
    ObjSpec {
        object: "heatMap",
        en: "Heat map",
        fr: "Carte thermique",
        show_def: true,
        rows: &[
            row("color0", "Low colour", "Couleur basse", Ctl::Color, Def::None),
            row(
                "color50",
                "Middle colour",
                "Couleur médiane",
                Ctl::Color,
                Def::None,
            ),
            row("color100", "High colour", "Couleur haute", Ctl::Color, Def::None),
            TRANSPARENCY,
            row(
                "unit",
                "Radius unit",
                "Unité du rayon",
                Ctl::Choice(RADIUS_UNITS),
                Def::None,
            ),
        ],
    },
    ObjSpec {
        object: "defaultColors",
        en: "Default colour",
        fr: "Couleur par défaut",
        show_def: false,
        rows: &[
            row("defaultShow", "Fill", "Remplissage", Ctl::Toggle, Def::Bool(true)),
            row("defaultColor", "Colour", "Couleur", Ctl::Color, Def::None),
            row(
                "borderColor",
                "Border colour",
                "Couleur de la bordure",
                Ctl::Color,
                Def::None,
            ),
            row(
                "borderThickness",
                "Border width",
                "Épaisseur de la bordure",
                Ctl::Number(0.0, 10.0),
                Def::None,
            ),
        ],
    },
    ObjSpec {
        object: "shape",
        en: "Shape",
        fr: "Forme",
        show_def: false,
        rows: &[
            row("mapUrl", "Map URL", "URL de la carte", Ctl::Text, Def::None),
            row(
                "projectionEnum",
                "Projection",
                "Projection",
                Ctl::Choice(PROJECTIONS),
                Def::None,
            ),
        ],
    },
    ObjSpec {
        object: "colorByCategory",
        en: "Color by category",
        fr: "Couleur par catégorie",
        show_def: true,
        rows: &[],
    },
    ObjSpec {
        object: "ratioLine",
        en: "Ratio line",
        fr: "Ligne de ratio",
        show_def: true,
        rows: &[
            row("lineColor", "Colour", "Couleur", Ctl::Color, Def::None),
            row("width", "Width", "Épaisseur", Ctl::Number(0.0, 20.0), Def::None),
            row("style", "Style", "Style", Ctl::Choice(LINE_STYLES), Def::None),
            TRANSPARENCY,
            row(
                "autoScale",
                "Automatic scale",
                "Échelle automatique",
                Ctl::Toggle,
                Def::Bool(true),
            ),
            row("displayName", "Name", "Nom", Ctl::Text, Def::None),
        ],
    },
    ObjSpec {
        object: "fillPoint",
        en: "Fill point",
        fr: "Point de remplissage",
        show_def: false,
        rows: &[row(
            "style",
            "Style",
            "Style",
            Ctl::Choice(FILL_STYLES),
            Def::None,
        )],
    },
    ObjSpec {
        object: "wordWrap",
        en: "Word wrap",
        fr: "Retour à la ligne",
        show_def: true,
        rows: &[],
    },
];

/// The card's title: the catalog's object name, normalised, then translated from its meaning.
fn card_title(
    vtype: &str,
    o: &'static visuals::Object,
    spec: &'static ObjSpec,
) -> (&'static str, &'static str) {
    // Desktop names a card's value "Callout value", whatever the catalog calls its object
    if matches!(vtype, "card" | "multiRowCard") && o.name == "labels" {
        return ("Callout value", "Valeur de légende");
    }
    // Microsoft Learn names a shape map's `shape` card Map settings
    if vtype == "shapeMap" && o.name == "shape" {
        return ("Map settings", "Paramètres de la carte");
    }
    match o.display_name.as_deref() {
        Some(name) => {
            let en = title_en(name);
            (en, title_fr(en))
        }
        None => (spec.en, spec.fr),
    }
}

/// Desktop's spelling of the object names our French and our labels expect.
fn title_en(s: &'static str) -> &'static str {
    match s {
        "X axis" => "X-axis",
        "Y axis" => "Y-axis",
        "Data colors" => "Data colours",
        "Sentiment colors" => "Sentiment colours",
        "Default Color" => "Default colour",
        "Card Title" => "Card title",
        "Color by category" => "Colour by category",
        "Color coding" => "Colour coding",
        other => other,
    }
}

/// Our French for a card's English title (translated from its meaning, never copied from a
/// product's own translations); an unknown title stays in English.
fn title_fr(en: &'static str) -> &'static str {
    match en {
        "Legend" => "Légende",
        "X-axis" => "Axe X",
        "Y-axis" => "Axe Y",
        "Gauge axis" => "Axe de la jauge",
        "Callout value" => "Valeur de légende",
        "Target" => "Cible",
        "Data label" => "Étiquette de données",
        "Data labels" => "Étiquettes de données",
        "Detail labels" => "Étiquettes de détail",
        "Series labels" => "Étiquettes de séries",
        "Lines" | "Shapes" => "Formes",
        "Markers" => "Marqueurs",
        "Data colours" => "Couleurs des données",
        "Conversion rate labels" => "Étiquettes de taux de conversion",
        "Breakdown" => "Décomposition",
        "Sentiment colours" => "Couleurs de sentiment",
        "Category label" => "Étiquette de catégorie",
        "Category labels" => "Étiquettes de catégorie",
        "Value" => "Valeur",
        "Values" => "Valeurs",
        "Label" => "Étiquette",
        "Card" => "Carte",
        "Card title" => "Titre de la carte",
        "Column headers" => "En-têtes de colonnes",
        "Row headers" => "En-têtes de lignes",
        "Grid" | "Grid layout" => "Grille",
        "Total" => "Total",
        "Grand total" => "Total général",
        "Subtotals" => "Sous-totaux",
        "Column grand total" => "Total général des colonnes",
        "Row grand total" => "Total général des lignes",
        "Sparklines" => "Graphiques sparkline",
        "Slicer header" => "En-tête du segment",
        "Items" => "Éléments",
        "Selection controls" => "Contrôles de sélection",
        "Slider" => "Curseur",
        "Dropdown" => "Liste déroulante",
        "Date inputs" => "Saisies de date",
        "Search box" => "Zone de recherche",
        "Input text" => "Texte de saisie",
        "Apply button" => "Bouton Appliquer",
        "Slicer settings" => "Paramètres du segment",
        "Indicator" => "Indicateur",
        "Trend axis" => "Axe de tendance",
        "Goals" => "Objectifs",
        "Colour coding" => "Code couleur",
        "Shape glow" => "Halo de la forme",
        "Shape shadow" => "Ombre de la forme",
        "Date" => "Date",
        "Map controls" | "Map settings" => "Commandes de la carte",
        "Map styles" => "Styles de la carte",
        "Bubbles" => "Bulles",
        "Plot area" => "Zone de tracé",
        "Multi-category layout" => "Disposition multicatégorie",
        "Ribbons" => "Rubans",
        "Total labels" => "Étiquettes de total",
        "Bars" => "Barres",
        "Tree" => "Arborescence",
        "Headers" => "En-têtes",
        "Analysis" => "Analyse",
        "Analysis visual colors" => "Couleurs du visuel d'analyse",
        "Drill visual colors" => "Couleurs du visuel d'exploration",
        "Callout area" => "Zone de légende",
        "Image" | "Images" => "Images",
        "Center value" => "Valeur centrale",
        "Zoom" | "Zoom slider" => "Zoom",
        "Heat map" => "Carte thermique",
        "Default colour" => "Couleur par défaut",
        "Shape" => "Forme",
        "Colour by category" => "Couleur par catégorie",
        "Ratio line" => "Ligne de ratio",
        "Fill point" => "Point de remplissage",
        "Word wrap" => "Retour à la ligne",
        "Text" => "Texte",
        "Fill" => "Remplissage",
        "Icon" => "Icône",
        "Outline" => "Contour",
        "Accent bar" => "Barre d'accentuation",
        "Pages" => "Pages",
        "Bookmarks" => "Signets",
        other => other,
    }
}

/// The props of `spec` for visual type `vtype`: the rows whose property the catalog gives the
/// type, one per label (the first one wins), with the object and property the command writes.
fn props_for(vtype: &str, spec: &'static ObjSpec) -> Vec<Prop> {
    let mut out: Vec<Prop> = Vec::new();
    let mut labels: Vec<&'static str> = Vec::new();
    for r in spec.rows {
        let Some(property) = visuals::property(vtype, spec.object, r.prop) else {
            continue;
        };
        if labels.contains(&r.en) {
            continue;
        }
        labels.push(r.en);
        out.push(Prop {
            key: property.name.as_str(),
            en: r.en,
            fr: r.fr,
            ctl: r.ctl,
            to: To::Object(spec.object, property.name.as_str()),
            def: r.def,
        });
    }
    out
}

/// The card of `spec` for visual type `vtype`: `None` when the type has neither the object nor
/// one of its rows.
fn card_for(vtype: &str, spec: &'static ObjSpec) -> Option<Card> {
    let object = visuals::object(vtype, spec.object)?;
    let props = props_for(vtype, spec);
    let show = visuals::property(vtype, spec.object, "show").is_some();
    if props.is_empty() && !show {
        return None;
    }
    let (en, fr) = card_title(vtype, object, spec);
    Some(Card {
        key: object.name.as_str(),
        en,
        fr,
        group: Group::Visual,
        switch: show.then_some((To::Object(spec.object, "show"), spec.show_def)),
        sections: Cow::Owned(vec![Section {
            title: None,
            switch: None,
            props: Cow::Owned(props),
        }]),
    })
}

const IMAGE: Card = Card {
    key: "image",
    en: "Image",
    fr: "Image",
    group: Group::Visual,
    switch: None,
    sections: Cow::Borrowed(&[Section {
        title: None,
        switch: None,
        props: Cow::Borrowed(&[
            p(
                "scaling",
                "Image fit",
                "Ajustement de l'image",
                Ctl::Choice(SCALINGS),
                To::Scaling,
                Def::Str("normal"),
            ),
            p(
                "change",
                "Change image",
                "Changer l'image",
                Ctl::Run("image.set", "Change image…", "Changer l'image…"),
                To::Scaling,
                Def::None,
            ),
        ]),
    }]),
};

/// The shape visual's cards (Insert › Shapes): its shape, rotation, fill and outline. The shape
/// and the rotation are written without selector, the fill and the outline in the `default`
/// state, where Desktop reads them (`visual.format` routes them, `visual_states.rs`).
const SHAPE_CARDS: &[Card] = &[
    Card {
        key: "shape",
        en: "Shape",
        fr: "Forme",
        group: Group::Visual,
        switch: None,
        sections: Cow::Borrowed(&[Section {
            title: None,
            switch: None,
            props: Cow::Borrowed(&[
                p(
                    "tileShape",
                    "Shape",
                    "Forme",
                    Ctl::Choice(crate::rich::SHAPES),
                    To::Object("shape", "tileShape"),
                    Def::Str("rectangle"),
                ),
                p(
                    "rectangleRoundedCurve",
                    "Rounded corners (%)",
                    "Coins arrondis (%)",
                    Ctl::Number(0.0, 100.0),
                    To::Object("shape", "rectangleRoundedCurve"),
                    Def::Num(10.0),
                ),
            ]),
        }]),
    },
    Card {
        key: "rotation",
        en: "Rotation",
        fr: "Rotation",
        group: Group::Visual,
        switch: None,
        sections: Cow::Borrowed(&[Section {
            title: None,
            switch: None,
            props: Cow::Borrowed(&[p(
                "shapeAngle",
                "Shape rotation (°)",
                "Rotation de la forme (°)",
                Ctl::Number(0.0, 360.0),
                To::Object("rotation", "shapeAngle"),
                Def::Num(0.0),
            )]),
        }]),
    },
    Card {
        key: "fill",
        en: "Fill",
        fr: "Remplissage",
        group: Group::Visual,
        switch: Some((To::Object("fill", "show"), true)),
        sections: Cow::Borrowed(&[Section {
            title: None,
            switch: None,
            props: Cow::Borrowed(&[
                p(
                    "fillColor",
                    "Colour",
                    "Couleur",
                    Ctl::Color,
                    To::Object("fill", "fillColor"),
                    Def::Accent,
                ),
                p(
                    "transparency",
                    "Transparency",
                    "Transparence",
                    Ctl::Percent,
                    To::Object("fill", "transparency"),
                    Def::Num(0.0),
                ),
            ]),
        }]),
    },
    Card {
        key: "outline",
        en: "Outline",
        fr: "Contour",
        group: Group::Visual,
        switch: Some((To::Object("outline", "show"), true)),
        sections: Cow::Borrowed(&[Section {
            title: None,
            switch: None,
            props: Cow::Borrowed(&[
                p(
                    "lineColor",
                    "Colour",
                    "Couleur",
                    Ctl::Color,
                    To::Object("outline", "lineColor"),
                    Def::None,
                ),
                p(
                    "weight",
                    "Weight (px)",
                    "Épaisseur (px)",
                    Ctl::Number(0.0, 50.0),
                    To::Object("outline", "weight"),
                    Def::Num(1.0),
                ),
                p(
                    "transparency",
                    "Transparency",
                    "Transparence",
                    Ctl::Percent,
                    To::Object("outline", "transparency"),
                    Def::Num(0.0),
                ),
            ]),
        }]),
    },
];

/// A table's and matrix's Cell elements (Desktop's card of that name): the conditional formats
/// of the values that are not a colour, each an fx button opening its dialog (data bars, icons,
/// web URL). The values' background and font colours keep their fx in the Values card.
const CELL_ELEMENTS: Card = Card {
    key: "cellElements",
    en: "Cell elements",
    fr: "Éléments de cellule",
    group: Group::Visual,
    switch: None,
    sections: Cow::Borrowed(&[Section {
        title: None,
        switch: None,
        props: Cow::Borrowed(&[
            p(
                "dataBars",
                "Data bars",
                "Barres de données",
                Ctl::Fx("data_bars"),
                To::Object("columnFormatting", "dataBars"),
                Def::None,
            ),
            p(
                "icon",
                "Icons",
                "Icônes",
                Ctl::Fx("icon"),
                To::Object("values", "icon"),
                Def::None,
            ),
            p(
                "webURL",
                "Web URL",
                "URL web",
                Ctl::Fx("web_url"),
                To::Object("values", "webURL"),
                Def::None,
            ),
        ]),
    }]),
};

/// A matrix's Column subtotals and Row subtotals cards (Microsoft Learn, "Create a matrix visual
/// in Power BI": the Format pane's Row subtotals and Column subtotals, each with "Per row level" /
/// "Per column level"): the `subTotals` switch, the per-level switch, the row subtotals'
/// position, the label; with per level on, one Show subtotal switch and one label per level
/// (`levels_ui`).
const SUBTOTAL_CARDS: &[Card] = &[
    Card {
        key: "columnSubtotals",
        en: "Column subtotals",
        fr: "Sous-totaux de colonnes",
        group: Group::Visual,
        switch: Some((To::Object("subTotals", "columnSubtotals"), true)),
        sections: Cow::Borrowed(&[Section {
            title: None,
            switch: None,
            props: Cow::Borrowed(&[
                p(
                    "perColumnLevel",
                    "Per column level",
                    "Par niveau de colonne",
                    Ctl::Toggle,
                    To::Object("subTotals", "perColumnLevel"),
                    Def::Bool(false),
                ),
                p(
                    "columnSubtotalsLabel",
                    "Subtotal label",
                    "Libellé du sous-total",
                    Ctl::Text,
                    To::Object("subTotals", "columnSubtotalsLabel"),
                    Def::None,
                ),
            ]),
        }]),
    },
    Card {
        key: "rowSubtotals",
        en: "Row subtotals",
        fr: "Sous-totaux de lignes",
        group: Group::Visual,
        switch: Some((To::Object("subTotals", "rowSubtotals"), true)),
        sections: Cow::Borrowed(&[Section {
            title: None,
            switch: None,
            props: Cow::Borrowed(&[
                p(
                    "perRowLevel",
                    "Per row level",
                    "Par niveau de ligne",
                    Ctl::Toggle,
                    To::Object("subTotals", "perRowLevel"),
                    Def::Bool(false),
                ),
                p(
                    "rowSubtotalsPosition",
                    "Position",
                    "Position",
                    Ctl::Choice(TOP_BOTTOM),
                    To::Object("subTotals", "rowSubtotalsPosition"),
                    Def::Str("Top"),
                ),
                p(
                    "rowSubtotalsLabel",
                    "Subtotal label",
                    "Libellé du sous-total",
                    Ctl::Text,
                    To::Object("subTotals", "rowSubtotalsLabel"),
                    Def::None,
                ),
            ]),
        }]),
    },
];

/// A shape's text (Format › Style › Text): written to the state edited, as Desktop reads it.
const SHAPE_TEXT: Card = Card {
    key: "text",
    en: "Text",
    fr: "Texte",
    group: Group::Visual,
    switch: Some((To::Object("text", "show"), false)),
    sections: Cow::Borrowed(&[Section {
        title: None,
        switch: None,
        props: Cow::Borrowed(&[
            p(
                "text",
                "Text",
                "Texte",
                Ctl::Text,
                To::Object("text", "text"),
                Def::None,
            ),
            p(
                "fontSize",
                "Font size",
                "Taille de police",
                Ctl::Size,
                To::Object("text", "fontSize"),
                Def::Num(10.0),
            ),
            p(
                "bold",
                "Bold",
                "Gras",
                Ctl::Style("B"),
                To::Object("text", "bold"),
                Def::Bool(false),
            ),
            p(
                "italic",
                "Italic",
                "Italique",
                Ctl::Style("I"),
                To::Object("text", "italic"),
                Def::Bool(false),
            ),
            p(
                "fontColor",
                "Font colour",
                "Couleur de police",
                Ctl::Color,
                To::Object("text", "fontColor"),
                Def::None,
            ),
            p(
                "horizontalAlignment",
                "Horizontal alignment",
                "Alignement horizontal",
                Ctl::Align,
                To::Object("text", "horizontalAlignment"),
                Def::Str("center"),
            ),
            p(
                "verticalAlignment",
                "Vertical alignment",
                "Alignement vertical",
                Ctl::Choice(VERTICAL_ALIGNMENTS),
                To::Object("text", "verticalAlignment"),
                Def::Str("middle"),
            ),
        ]),
    }]),
};

const VERTICAL_ALIGNMENTS: &[(&str, &str, &str)] = &[
    ("top", "Top", "Haut"),
    ("middle", "Middle", "Milieu"),
    ("bottom", "Bottom", "Bas"),
];
const ORIENTATIONS: &[(&str, &str, &str)] = &[
    ("2", "Horizontal", "Horizontale"),
    ("1", "Vertical", "Verticale"),
    ("0", "Grid", "Grille"),
];

/// A navigator's buttons' Shape (one for every state, written without selector), Glow and Shadow
/// (by state) cards: the report theme schema's `shape`, `glow` and `shadow` objects of
/// `pageNavigator` / `bookmarkNavigator`.
const NAV_SHAPE: &[ObjSpec] = &[
    ObjSpec {
        object: "shape",
        en: "Shape",
        fr: "Forme",
        show_def: true,
        rows: &[
            row(
                "tileShape",
                "Shape",
                "Forme",
                Ctl::Choice(crate::rich::SHAPES),
                Def::Str("rectangle"),
            ),
            row(
                "rectangleRoundedCurve",
                "Rounded corners (%)",
                "Coins arrondis (%)",
                Ctl::Number(0.0, 100.0),
                Def::Num(10.0),
            ),
        ],
    },
    ObjSpec {
        object: "glow",
        en: "Glow",
        fr: "Halo",
        show_def: false,
        rows: &[
            row("color", "Colour", "Couleur", Ctl::Color, Def::None),
            row(
                "shadowBlur",
                "Blur",
                "Flou",
                Ctl::Number(0.0, 100.0),
                Def::Num(5.0),
            ),
            TRANSPARENCY,
        ],
    },
    ObjSpec {
        object: "shadow",
        en: "Shadow",
        fr: "Ombre",
        show_def: false,
        rows: &[
            row("color", "Colour", "Couleur", Ctl::Color, Def::None),
            row(
                "shadowBlur",
                "Blur",
                "Flou",
                Ctl::Number(0.0, 100.0),
                Def::Num(5.0),
            ),
            row(
                "shadowDistance",
                "Distance",
                "Distance",
                Ctl::Number(0.0, 100.0),
                Def::Num(3.0),
            ),
            row("angle", "Angle", "Angle", Ctl::Number(0.0, 360.0), Def::Num(45.0)),
            TRANSPARENCY,
        ],
    },
];

const ACCENT_POSITIONS: &[(&str, &str, &str)] = &[
    ("Bottom", "Bottom", "Bas"),
    ("Top", "Top", "Haut"),
    ("Left", "Left", "Gauche"),
    ("Right", "Right", "Droite"),
];

/// The cards of buttons and navigators, in Desktop's order (each shown when the catalog gives
/// the type its object: Pages only for page navigators, Bookmarks only for bookmark navigators,
/// Icon only for buttons).
const NAV_OBJECTS: &[ObjSpec] = &[
    ObjSpec {
        object: "layout",
        en: "Grid layout",
        fr: "Grille",
        show_def: true,
        rows: &[
            row(
                "orientation",
                "Orientation",
                "Orientation",
                Ctl::Numbered(ORIENTATIONS),
                Def::Str("2"),
            ),
            row(
                "cellPadding",
                "Padding",
                "Marge intérieure",
                Ctl::Number(0.0, 100.0),
                Def::Num(6.0),
            ),
            row("rowCount", "Rows", "Lignes", Ctl::Number(1.0, 100.0), Def::None),
            row(
                "columnCount",
                "Columns",
                "Colonnes",
                Ctl::Number(1.0, 100.0),
                Def::None,
            ),
        ],
    },
    ObjSpec {
        object: "pages",
        en: "Pages",
        fr: "Pages",
        show_def: true,
        rows: &[
            row(
                "showHiddenPages",
                "Show hidden pages",
                "Afficher les pages masquées",
                Ctl::Toggle,
                Def::Bool(false),
            ),
            row(
                "showTooltipPages",
                "Show tooltip pages",
                "Afficher les pages d'info-bulle",
                Ctl::Toggle,
                Def::Bool(false),
            ),
        ],
    },
    ObjSpec {
        object: "bookmarks",
        en: "Bookmarks",
        fr: "Signets",
        show_def: true,
        rows: &[
            row(
                "bookmarkGroup",
                "Bookmark group",
                "Groupe de signets",
                Ctl::Text,
                Def::None,
            ),
            row(
                "allowDeselectionBookmark",
                "Allow deselection",
                "Autoriser la désélection",
                Ctl::Toggle,
                Def::Bool(false),
            ),
            row(
                "deselectionBookmark",
                "Launch on deselection",
                "Lancer à la désélection",
                Ctl::Text,
                Def::None,
            ),
            row(
                "hideDeselectedBookmark",
                "Hide deselection bookmark",
                "Masquer le signet de désélection",
                Ctl::Toggle,
                Def::Bool(false),
            ),
        ],
    },
    ObjSpec {
        object: "text",
        en: "Text",
        fr: "Texte",
        show_def: true,
        rows: &[
            row("text", "Text", "Texte", Ctl::Text, Def::None),
            SIZE,
            BOLD,
            ITALIC,
            COLOR,
            ALIGN,
        ],
    },
    ObjSpec {
        object: "icon",
        en: "Icon",
        fr: "Icône",
        show_def: true,
        rows: &[
            row("lineColor", "Colour", "Couleur", Ctl::Color, Def::None),
            row(
                "placement",
                "Icon placement",
                "Position de l'icône",
                Ctl::Choice(ICON_PLACEMENTS),
                Def::Str("left"),
            ),
        ],
    },
    ObjSpec {
        object: "fill",
        en: "Fill",
        fr: "Remplissage",
        show_def: true,
        rows: &[
            row("fillColor", "Colour", "Couleur", Ctl::Color, Def::None),
            TRANSPARENCY,
        ],
    },
    ObjSpec {
        object: "outline",
        en: "Border",
        fr: "Bordure",
        show_def: false,
        rows: &[
            row("lineColor", "Colour", "Couleur", Ctl::Color, Def::None),
            row(
                "weight",
                "Width",
                "Épaisseur",
                Ctl::Number(0.0, 20.0),
                Def::Num(1.0),
            ),
            TRANSPARENCY,
        ],
    },
    ObjSpec {
        object: "accentBar",
        en: "Accent bar",
        fr: "Barre d'accentuation",
        show_def: true,
        rows: &[
            row("color", "Colour", "Couleur", Ctl::Color, Def::None),
            row(
                "width",
                "Width",
                "Épaisseur",
                Ctl::Number(0.0, 100.0),
                Def::Num(3.0),
            ),
            row(
                "position",
                "Position",
                "Position",
                Ctl::Choice(ACCENT_POSITIONS),
                Def::Str("Bottom"),
            ),
        ],
    },
];

const ICON_PLACEMENTS: &[(&str, &str, &str)] = &[
    ("left", "Left of text", "À gauche du texte"),
    ("right", "Right of text", "À droite du texte"),
];

/// Most levels whose per-level subtotal rows a card lists.
const MAX_LEVEL_ROWS: usize = 8;

/// The Visual group's cards of visual type `vtype`: the image visual's and the shape's own cards,
/// then the catalog's objects the type has, in Desktop's order (empty: none for that type yet),
/// a matrix's Column subtotals and Row subtotals after its Subtotals style card, and a table's or
/// matrix's Cell elements.
pub fn visual_cards(vtype: &str) -> Vec<Card> {
    if vtype == "image" {
        return vec![IMAGE.clone()];
    }
    if vtype == "shape" {
        let mut out = SHAPE_CARDS.to_vec();
        out.push(SHAPE_TEXT.clone());
        return out;
    }
    // buttons and navigators: Desktop's Format button / Format navigator cards, as the catalog
    // gives them to the type (Grid layout, Pages, Bookmarks, then Style's Text, Icon, Fill,
    // Border, Accent bar; a navigator's buttons also their Shape, Glow and Shadow, G55)
    if matches!(vtype, "actionButton" | "pageNavigator" | "bookmarkNavigator") {
        let navigator = vtype != "actionButton";
        return NAV_OBJECTS
            .iter()
            .chain(NAV_SHAPE.iter().filter(|_| navigator))
            .filter_map(|spec| card_for(vtype, spec))
            .collect();
    }
    let mut out: Vec<Card> = VISUAL_OBJECTS
        .iter()
        .filter_map(|spec| card_for(vtype, spec))
        .collect();
    if vtype == "pivotTable"
        && let Some(at) = out.iter().position(|c| c.key == "subTotals")
    {
        for (k, c) in SUBTOTAL_CARDS.iter().enumerate() {
            out.insert(at + 1 + k, c.clone());
        }
    }
    if matches!(vtype, "tableEx" | "pivotTable") {
        out.push(CELL_ELEMENTS.clone());
    }
    // a shape map's Map settings: Add a map type (a custom TopoJSON or GeoJSON map), first
    if vtype == "shapeMap"
        && let Some(card) = out.iter_mut().find(|c| c.key == "shape")
        && let Some(sec) = card.sections.to_mut().first_mut()
    {
        sec.props.to_mut().insert(0, ADD_MAP);
        sec.props.to_mut().insert(1, WORLD_MAP);
    }
    out
}

/// Format › Map settings › World countries (Natural Earth): BIcraft's built-in map, written into
/// the report as an ordinary custom TopoJSON map (`shape_map.set_map {builtin}`); Desktop's own
/// built-in maps are Microsoft's map data.
const WORLD_MAP: Prop = p(
    "worldMap",
    "Built-in map",
    "Carte intégrée",
    Ctl::RunWith(
        "shape_map.set_map",
        "builtin",
        "world_countries",
        "World countries (Natural Earth)",
        "Pays du monde (Natural Earth)",
    ),
    To::Object("shape", "map"),
    Def::None,
);

/// Format › Map settings › Add a map type (Microsoft Learn, "Create Shape Map visualizations in
/// Power BI"): `shape_map.set_map`'s form, for the map file.
const ADD_MAP: Prop = p(
    "addMap",
    "Custom map",
    "Carte personnalisée",
    Ctl::Run(
        "shape_map.set_map",
        "Add a map type…",
        "Ajouter un type de carte…",
    ),
    To::Object("shape", "map"),
    Def::None,
);

/// The cards of `group` for visual type `vtype`.
pub fn cards(group: Group, vtype: &str) -> Vec<Card> {
    match group {
        Group::General => GENERAL.to_vec(),
        Group::Visual => visual_cards(vtype),
    }
}
// ----- edits -----

/// The command and parameters (without `page` and `visual`) that write `v` to `to`.
pub fn edit(to: To, ctl: Ctl, v: Value) -> (&'static str, Value) {
    let one = |k: &str, v: Value| Value::Object(Map::from_iter([(k.to_string(), v)]));
    let mut o = Map::new();
    let id = match to {
        To::Title(k) => {
            o.insert("title".into(), one(k, v));
            "visual.set_title"
        }
        To::Format(g, k) => {
            o.insert(g.into(), one(k, v));
            "visual.format"
        }
        To::Whole(g) => {
            o.insert(g.into(), v);
            "visual.format"
        }
        To::Axis(a, k) => {
            o.insert("axis".into(), json!(a));
            o.insert(k.into(), v);
            "visual.axis"
        }
        To::Object(obj, prop) => {
            let value = match (&v, ctl) {
                (Value::String(c), Ctl::Color) => json!({ "color": c }),
                // a numeric enumeration is written as a number (`1000D`), not as text
                (Value::String(c), Ctl::Numbered(..)) => json!(c.parse::<f64>().unwrap_or(0.0)),
                _ => v,
            };
            o.insert(
                "objects".into(),
                json!([{"object": obj, "property": prop, "value": value}]),
            );
            "visual.format"
        }
        To::Level(_, _, prop) => {
            // the selector naming the level is added by `Cx::commit`, from the snapshot
            o.insert(
                "objects".into(),
                json!([{"object": "subTotals", "property": prop, "value": v}]),
            );
            "visual.format"
        }
        To::TooltipPage => {
            o.insert("tooltip_page".into(), v);
            "visual.tooltip_page"
        }
        To::Scaling => {
            o.insert("scaling".into(), v);
            "image.set"
        }
    };
    (id, Value::Object(o))
}

/// The `queryRef` of level `level` of the matrix's rows (`role` 0) or columns (1), from its
/// snapshot entry `snap` (`data.row_level_refs` / `column_level_refs`).
fn level_ref(snap: &Value, role: u8, level: u8) -> Option<String> {
    let key = if role == 0 {
        "row_level_refs"
    } else {
        "column_level_refs"
    };
    snap.get("data")?
        .get(key)?
        .get(usize::from(level))?
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Every target of `card` (its switches and properties).
fn targets(card: &Card) -> Vec<To> {
    let mut out: Vec<To> = card.switch.iter().map(|s| s.0).collect();
    for s in card.sections.iter() {
        out.extend(s.switch.iter().map(|x| x.0));
        out.extend(
            s.props
                .iter()
                .filter(|p| !matches!(p.ctl, Ctl::Run(..) | Ctl::RunWith(..) | Ctl::Fx(_)))
                .map(|p| p.to),
        );
    }
    out
}

/// The command that puts `card` back to its defaults (one command: undo takes it back at once).
pub fn reset(card: &Card) -> Option<(&'static str, Value)> {
    let ts = targets(card);
    let first = *ts.first()?;
    let mut o = Map::new();
    let id = match first {
        To::Title(_) => {
            o.insert("title".into(), Value::Null);
            "visual.set_title"
        }
        To::Axis(a, _) => {
            o.insert("axis".into(), json!(a));
            o.insert("clear".into(), json!(true));
            "visual.axis"
        }
        To::TooltipPage => {
            o.insert("tooltip_page".into(), json!("default"));
            "visual.tooltip_page"
        }
        To::Scaling => {
            o.insert("scaling".into(), json!("normal"));
            "image.set"
        }
        // a per-level entry stays (its rows are not the card's static ones)
        To::Format(..) | To::Whole(_) | To::Object(..) | To::Level(..) => {
            let mut objects: Vec<Value> = Vec::new();
            for t in &ts {
                match t {
                    To::Format(g, _) | To::Whole(g) => {
                        o.insert((*g).into(), Value::Null);
                    }
                    To::Object(obj, prop) => {
                        let item = json!({"object": obj, "property": prop, "value": null});
                        if !objects.contains(&item) {
                            objects.push(item);
                        }
                    }
                    _ => {}
                }
            }
            if !objects.is_empty() {
                o.insert("objects".into(), Value::Array(objects));
            }
            "visual.format"
        }
    };
    Some((id, Value::Object(o)))
}

// ----- reading the values -----

/// A group of the visual's own props as an object: `true` → `{show}`, `"text"` → `{text, show}`
/// (the title's shorthand), a number → `{top, bottom, left, right}`.
fn own_group(v: Option<&Value>) -> Option<Map<String, Value>> {
    match v? {
        Value::Object(o) => Some(o.clone()),
        Value::Bool(b) => Some(Map::from_iter([("show".to_string(), json!(b))])),
        Value::String(s) => Some(Map::from_iter([
            ("text".to_string(), json!(s)),
            ("show".to_string(), json!(true)),
        ])),
        Value::Number(n) => Some(Map::from_iter(
            ["top", "bottom", "left", "right"].map(|k| (k.to_string(), Value::Number(n.clone()))),
        )),
        _ => None,
    }
}

/// What a snapshot `format` group calls property `k` of `to`.
fn snap_key(to: To, k: &'static str) -> &'static str {
    match (to, k) {
        (To::Axis(..), "color") => "labels_color",
        _ => k,
    }
}

/// The current value of `to`: the visual's own (`own`, the `visual.props` answer's `props`), else
/// the snapshot's resolved one (`snap`, the visual's snapshot entry).
pub fn current(own: Option<&Value>, snap: &Value, to: To) -> Option<Value> {
    let none = Value::Null;
    let own = own.unwrap_or(&none);
    let fmt = snap.get("format").unwrap_or(&none);
    let pick =
        |g: Option<Map<String, Value>>, k: &str| g.and_then(|g| g.get(k).cloned()).filter(|v| !v.is_null());
    match to {
        To::Title(k) => pick(own_group(own.get("title")), k).or_else(|| {
            if k == "text" {
                snap.get("title").filter(|t| t.is_string()).cloned()
            } else {
                pick(own_group(fmt.get("title")), k)
            }
        }),
        To::Format(g, k) => pick(own_group(own.pointer(&format!("/format/{g}"))), k)
            .or_else(|| pick(own_group(fmt.get(g)), snap_key(to, k))),
        To::Whole(g) => {
            let o = own.pointer(&format!("/format/{g}")).cloned();
            match (g, o) {
                ("padding", Some(Value::Object(m))) => m.get("top").cloned(),
                (_, Some(v)) if !v.is_null() => Some(v),
                ("padding", _) => fmt.pointer("/padding/top").cloned(),
                _ => None,
            }
        }
        To::Axis(a, k) => pick(own_group(own.pointer(&format!("/axes/{a}"))), k)
            .or_else(|| pick(own_group(fmt.get(format!("{a}_axis"))), snap_key(to, k))),
        To::Object(obj, prop) => own
            .pointer("/format/objects")
            .and_then(Value::as_array)
            .and_then(|items| {
                items.iter().find(|i| {
                    i.get("object").and_then(Value::as_str) == Some(obj)
                        && i.get("property").and_then(Value::as_str) == Some(prop)
                        && i.get("state").is_none()
                })
            })
            .and_then(|i| i.get("value").cloned())
            .or_else(|| match (obj, prop) {
                ("dataPoint", "fill") => fmt
                    .get("series_colors")
                    .and_then(Value::as_array)
                    .and_then(|s| {
                        s.iter()
                            .find(|c| c.get("column").is_none() && c.get("field").is_none())
                    })
                    .and_then(|c| c.get("color").cloned()),
                _ => None,
            }),
        To::Level(role, level, prop) => {
            let key = if role == 0 {
                "row_level_totals"
            } else {
                "column_level_totals"
            };
            let field = if prop == "levelSubtotalLabel" {
                "label"
            } else {
                "show"
            };
            snap.get("data")
                .and_then(|d| d.get(key))
                .and_then(|l| l.get(usize::from(level)))
                .and_then(|l| l.get(field))
                .cloned()
        }
        To::TooltipPage => own.get("tooltip_page").cloned(),
        To::Scaling => snap.pointer("/image/scaling").cloned(),
    }
    .filter(|v| !v.is_null())
}

fn as_bool(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn as_num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim_end_matches(['D', 'L']).parse().ok(),
        // a number literal keeps its suffix (`{literal: "1000D"}`)
        Value::Object(o) => o
            .get("literal")
            .and_then(Value::as_str)
            .and_then(|s| s.trim_end_matches(['D', 'L']).parse().ok()),
        _ => None,
    }
    .filter(|x| x.is_finite())
}

fn as_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Object(o) => o
            .get("color")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                o.get("literal")
                    .and_then(Value::as_str)
                    .map(|s| s.trim_end_matches(['D', 'L']).to_string())
            }),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// A font family as the lists name it (`'Segoe UI'`, `"Segoe UI", sans-serif` → `Segoe UI`).
fn family(s: &str) -> String {
    s.split(',')
        .next()
        .unwrap_or("")
        .trim()
        .trim_matches(['\'', '"'])
        .to_string()
}

// ----- the Data colours card's series -----

/// What the Data colours card gives one colour each, as Desktop lists them under the default
/// colour: the measures of a chart with several, the values of its legend, or the values of its
/// category (pies, donuts and treemaps always; other charts when Show all is on).
#[derive(Debug, Clone, PartialEq)]
pub enum Colouring {
    /// Nothing more than the default colour.
    None,
    /// The value fields: (index in the snapshot's `fields`, label, field text).
    Measures(Vec<(usize, String, String)>),
    /// The values of a bound column: its field text, table and column.
    Values {
        field: String,
        table: String,
        column: String,
    },
}

/// One colour row of the Data colours card.
#[derive(Debug, Clone, PartialEq)]
pub struct SeriesRow {
    pub label: String,
    /// `visual.series`'s `target`.
    pub target: Value,
    /// The colour the visual sets, else `None` (the theme's colour in order is shown).
    pub color: Option<String>,
    /// The theme's colour for this row's place.
    pub def: String,
}

/// The column bound to role `role` (a plain column, not a hierarchy level).
fn role_column(fields: &[Value], role: &str) -> Option<(String, String, String)> {
    fields
        .iter()
        .find(|f| {
            f.get("role").and_then(Value::as_str) == Some(role)
                && f.get("kind").and_then(Value::as_str) == Some("column")
        })
        .and_then(|f| {
            Some((
                f.get("text")?.as_str()?.to_string(),
                f.get("table")?.as_str()?.to_string(),
                f.get("name")?.as_str()?.to_string(),
            ))
        })
}

/// What the Data colours card of a visual of type `vtype` with snapshot fields `fields` lists
/// colours for, with Show all on or off.
pub fn colouring(vtype: &str, fields: &[Value], show_all: bool) -> Colouring {
    let values =
        |(field, table, column): (String, String, String)| Colouring::Values { field, table, column };
    // pies, donuts and treemaps colour each value of their category
    let by_category = match vtype {
        "pieChart" | "donutChart" => Some("Category"),
        "treemap" => Some("Group"),
        _ => None,
    };
    if let Some(role) = by_category {
        return role_column(fields, role).map_or(Colouring::None, values);
    }
    if let Some(c) = role_column(fields, "Series") {
        return values(c);
    }
    let measures: Vec<(usize, String, String)> = fields
        .iter()
        .enumerate()
        .filter(|(_, f)| {
            matches!(f.get("role").and_then(Value::as_str), Some("Y" | "Y2"))
                && matches!(
                    f.get("kind").and_then(Value::as_str),
                    Some("measure" | "aggregation")
                )
        })
        .filter_map(|(i, f)| {
            let text = f.get("text")?.as_str()?.to_string();
            let label = f
                .get("display_name")
                .or_else(|| f.get("label"))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map_or_else(|| text.clone(), str::to_string);
            Some((i, label, text))
        })
        .collect();
    if measures.len() > 1 {
        return Colouring::Measures(measures);
    }
    if show_all && let Some(c) = role_column(fields, "Category") {
        return values(c);
    }
    Colouring::None
}

/// A column value as the card shows it.
fn value_label(v: &Value, l: Lang) -> String {
    match v {
        Value::Null => l.t("(Blank)", "(Vide)").to_string(),
        Value::String(s) if s.is_empty() => l.t("(Blank)", "(Vide)").to_string(),
        Value::String(s) => clip(s, 60),
        other => clip(&other.to_string(), 60),
    }
}

/// Whether two column values are the same (numbers by value: `3` and `3.0`).
fn same_value(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => (x - y).abs() < 1e-9,
        _ => a == b,
    }
}

/// The rows of `c` for the visual's snapshot entry `snap`, the column's `values` when they are
/// needed and known, and the theme's data colours `palette`.
pub fn series_rows(
    c: &Colouring,
    snap: &Value,
    values: Option<&[Value]>,
    palette: &[String],
    l: Lang,
) -> Vec<SeriesRow> {
    let set: &[Value] = snap
        .pointer("/format/series_colors")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice);
    let def = |i: usize| {
        palette
            .get(i % palette.len().max(1))
            .cloned()
            .unwrap_or_else(|| "#118DFF".into())
    };
    let hex = |c: &Value| {
        c.get("color")
            .and_then(Value::as_str)
            .filter(|h| crate::form::color(h).is_some())
            .map(str::to_string)
    };
    match c {
        Colouring::None => Vec::new(),
        Colouring::Measures(m) => m
            .iter()
            .take(MAX_SERIES_ROWS)
            .enumerate()
            .map(|(n, (i, label, text))| SeriesRow {
                label: label.clone(),
                target: json!(text),
                color: set
                    .iter()
                    .find(|s| {
                        s.get("column").and_then(Value::as_u64) == Some(*i as u64) && s.get("value").is_none()
                    })
                    .and_then(hex),
                def: def(n),
            })
            .collect(),
        Colouring::Values { field, .. } => values
            .unwrap_or_default()
            .iter()
            .take(MAX_SERIES_ROWS)
            .enumerate()
            .map(|(n, v)| SeriesRow {
                label: value_label(v, l),
                target: json!({"field": field, "value": v}),
                color: set
                    .iter()
                    .find(|s| {
                        s.get("field").and_then(Value::as_str) == Some(field.as_str())
                            && s.get("value").is_some_and(|x| same_value(x, v))
                    })
                    .and_then(hex),
                def: def(n),
            })
            .collect(),
    }
}

/// Keeps the answer of a `column.values` call for the Data colours card
/// ([`Then::FormatValues`]).
pub fn values_answer(app: &mut App, field: String, revision: u64, r: Result<Value, String>) {
    let (values, cut) = match r {
        Ok(v) => (
            v.get("values")
                .and_then(Value::as_array)
                .map(|a| a.iter().take(MAX_SERIES_ROWS).cloned().collect())
                .unwrap_or_default(),
            v.get("truncated").and_then(Value::as_bool).unwrap_or(false),
        ),
        Err(e) => {
            app.status = clip(&e, 300);
            (Vec::new(), false)
        }
    };
    app.panes.format.values.insert(field, (revision, values, cut));
}

// ----- the pane -----

/// What one frame of the tab needs to draw its cards.
struct Cx<'a> {
    sel: &'a VisualSel,
    /// The visual's type (which colour rows get an fx button).
    vtype: &'a str,
    own: Option<&'a Value>,
    snap: &'a Value,
    pending: &'a BTreeMap<String, Value>,
    drafts: &'a mut BTreeMap<String, (String, String)>,
    /// The theme's data colours and the neutrals, for the swatches.
    swatches: &'a [String],
    accent: &'a str,
    /// Tooltip page choices: (value, label).
    pages: &'a [(String, String)],
    /// Whether each command can run now (else why not).
    states: &'a BTreeMap<&'static str, Result<(), String>>,
    /// The Data colours card's colour rows, whether its list is cut, whether its values are
    /// still being read.
    series: Vec<SeriesRow>,
    series_cut: bool,
    series_wait: bool,
    l: Lang,
    /// The state the stateful cards edit (`default` for the other types).
    state: &'a str,
    /// The edits committed this frame: (pending key, value shown, command, params).
    acts: Vec<Act>,
}

enum Act {
    Edit {
        key: Option<String>,
        shown: Value,
        id: &'static str,
        params: Value,
    },
    /// Runs a command as its ribbon button does.
    Invoke(&'static str),
    /// Opens the conditional-formatting dialog of `target` (an fx button), headed `name`.
    Fx { target: &'static str, name: String },
}

impl Cx<'_> {
    /// The state `to` is edited in: `Some(state)` for a stateful object outside the default state.
    fn state_of(&self, to: To) -> Option<&str> {
        match to {
            To::Object(obj, _) if self.state != "default" && stateful_object(self.vtype, obj) => {
                Some(self.state)
            }
            _ => None,
        }
    }

    /// The pending key of `key` in the state edited.
    fn pending_key(&self, key: &str, to: To) -> String {
        match self.state_of(to) {
            Some(s) => format!("{s}:{key}"),
            None => key.to_string(),
        }
    }

    fn value(&self, key: &str, to: To, def: Def) -> Value {
        if let Some(v) = self.pending.get(&self.pending_key(key, to)) {
            return v.clone();
        }
        // another state than the default one: only what that state sets
        if let (Some(state), To::Object(obj, prop)) = (self.state_of(to), to) {
            let set = self
                .own
                .and_then(|o| o.pointer("/format/objects"))
                .and_then(Value::as_array)
                .and_then(|items| {
                    items.iter().find(|i| {
                        i.get("object").and_then(Value::as_str) == Some(obj)
                            && i.get("property").and_then(Value::as_str) == Some(prop)
                            && i.get("state").and_then(Value::as_str) == Some(state)
                    })
                })
                .and_then(|i| i.get("value").cloned())
                .filter(|v| !v.is_null());
            if let Some(v) = set {
                return v;
            }
        }
        current(self.own, self.snap, to).unwrap_or(match def {
            Def::None => Value::Null,
            Def::Bool(b) => json!(b),
            Def::Num(n) => json!(n),
            Def::Str(s) => json!(s),
            Def::Accent => json!(self.accent),
        })
    }

    fn state(&self, id: &str) -> Result<(), String> {
        self.states.get(id).cloned().unwrap_or(Ok(()))
    }

    fn commit(&mut self, key: String, to: To, ctl: Ctl, v: Value) {
        let (id, mut params) = edit(to, ctl, v.clone());
        // a stateful object in another state than the default one: that state's entry
        let key = self.pending_key(&key, to);
        if let Some(state) = self.state_of(to).map(str::to_string)
            && let Some(item) = params.pointer_mut("/objects/0").and_then(Value::as_object_mut)
        {
            item.insert("state".into(), json!(state));
        }
        // a per-level subtotal names its level by the field's queryRef
        if let To::Level(role, level, _) = to {
            let Some(r) = level_ref(self.snap, role, level) else {
                return;
            };
            if let Some(item) = params.pointer_mut("/objects/0").and_then(Value::as_object_mut) {
                item.insert("selector".into(), json!({ "metadata": r }));
            }
        }
        self.acts.push(Act::Edit {
            key: Some(key),
            shown: v,
            id,
            params,
        });
    }
}

/// Runs `visual.props` for the selected visual when the snapshot moved on since it was read.
pub(crate) fn ask_props(app: &mut App, sel: &VisualSel, revision: u64) {
    let key = (sel.page.clone(), sel.visual.clone(), revision);
    if app.panes.format.asked.as_ref() == Some(&key) || ribbon::state(app, "visual.props").is_err() {
        return;
    }
    app.panes.format.asked = Some(key);
    let mut params = json!({"page": sel.page, "visual": sel.visual});
    if crate::mobile::editing(app) {
        params["layout"] = json!("mobile");
    }
    app.run(
        Call {
            id: "visual.props",
            params,
        },
        Then::FormatProps {
            page: sel.page.clone(),
            visual: sel.visual.clone(),
            revision,
        },
    );
}

/// Keeps the answer of a `visual.props` call ([`Then::FormatProps`]).
pub fn props_answer(app: &mut App, page: String, visual: String, revision: u64, r: Result<Value, String>) {
    match r {
        Ok(v) => {
            app.panes.format.own = Some(Own {
                page,
                visual,
                revision,
                props: v.get("props").cloned().unwrap_or(Value::Null),
            });
        }
        // a visual removed meanwhile: nothing to show, nothing to say
        Err(_) => app.panes.format.own = None,
    }
}

/// Takes in the answer of one of the tab's edits ([`Then::FormatEdit`]): its new props (the
/// answer's `props`) shown at once; an error to the error bar.
pub fn edit_answer(app: &mut App, page: &str, visual: &str, r: Result<Value, String>) {
    app.panes.format.pending.clear();
    app.panes.format.analytics.pending = None;
    match r {
        Ok(v) => {
            if let (Some(own), Some(props)) = (&mut app.panes.format.own, v.get("props"))
                && own.page == page
                && own.visual == visual
            {
                own.props = props.clone();
            }
        }
        Err(e) => app.error = Some(format!("✕ {}", clip(&e, 600))),
    }
}

/// The ribbon's Format visual button (and the palette's Enter) without values: opens the Build
/// pane on its Format tab, as Desktop's Format button does. True when it did.
pub fn intercept(app: &mut App, id: &str, params: &Value) -> bool {
    if id != "visual.format" || app.access.as_deref() == Some("reader") {
        return false;
    }
    let values = params
        .as_object()
        .is_some_and(|o| o.keys().any(|k| k != "page" && k != "visual"));
    if values {
        return false;
    }
    open_format(app);
    true
}

/// Opens the Build pane on its Format visual tab.
pub fn open_format(app: &mut App) {
    if !app.prefs.panes.build {
        app.prefs.panes.build = true;
        app.save_prefs();
    }
    app.panes.format.tab = Tab::Format;
}

/// The two sub-tabs at the Build pane's top (Desktop's Build visual and Format visual icons):
/// the tab shown. A reader gets the Build visual tab only.
pub fn tabs(app: &mut App, ui: &mut Ui) -> Tab {
    let l = app.lang;
    let reader = app.access.as_deref() == Some("reader");
    if reader {
        app.panes.format.tab = Tab::Build;
    }
    let t = Tokens::of(app.prefs.theme);
    // Desktop's Visualizations pane: the view shown at the left, its two icon tabs at the
    // right of the same row
    let name = match app.panes.format.tab {
        Tab::Build => l.t("Build a visual", "Générer un visuel"),
        Tab::Format => l.t("Format a visual", "Mettre en forme le visuel"),
        Tab::Analytics => l.t("Add further analyses", "Ajouter des analyses"),
    };
    let mut pick = None;
    // the icons are laid out first, from the right, so the name takes only the width left and
    // is cut with an ellipsis instead of running under them (a long French name in a narrow pane)
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), 26.0),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            // right to left: Build visual, Format visual, Analytics as read from the left
            for (tab, icon, en, fr) in [
                (Tab::Analytics, Icon::Analytics, "Analytics", "Analyse"),
                (
                    Tab::Format,
                    Icon::Format,
                    "Format visual",
                    "Mettre en forme le visuel",
                ),
                (Tab::Build, Icon::Chart, "Build visual", "Générer un visuel"),
            ] {
                if tab != Tab::Build && reader {
                    continue;
                }
                let on = app.panes.format.tab == tab;
                if icon_tab(ui, icon, l.t(en, fr), on).clicked() {
                    pick = Some(tab);
                }
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add(
                    egui::Label::new(egui::RichText::new(name).font(theme::bold(13.0)).color(t.text))
                        .truncate(),
                );
            });
        },
    );
    if let Some(tab) = pick {
        app.panes.format.tab = tab;
    }
    ui.add_space(4.0);
    app.panes.format.tab
}

/// A Build pane sub-tab: its icon (30 × 26) with an accent underline when shown, labelled by its
/// name (the tooltip).
fn icon_tab(ui: &mut Ui, icon: Icon, label: &str, on: bool) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(30.0, 26.0), Sense::click());
    let p = ui.painter();
    if resp.hovered() && !on {
        p.rect_filled(rect, CornerRadius::same(4), t.hover);
    }
    icons::paint(
        p,
        rect.shrink2(vec2(6.0, 5.0)).translate(vec2(0.0, -1.0)),
        icon,
        t.text,
        false,
    );
    if on {
        p.rect_filled(
            Rect::from_min_max(
                pos2(rect.min.x + 4.0, rect.max.y - 3.0),
                pos2(rect.max.x - 4.0, rect.max.y),
            ),
            CornerRadius::same(1),
            t.accent,
        );
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, label));
    resp.on_hover_text(label)
}

/// A group tab (Visual, General): its text with an accent underline when shown.
fn group_tab(ui: &mut Ui, label: &str, a11y: &str, on: bool) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let font = if on { theme::bold(12.0) } else { theme::font(12.0) };
    let w = ui.fonts_mut(|f| {
        f.layout_no_wrap(label.to_string(), theme::bold(12.0), Color32::WHITE)
            .size()
            .x
    }) + 16.0;
    let (r, resp) = ui.allocate_exact_size(vec2(w, 22.0), Sense::click());
    if resp.hovered() && !on {
        ui.painter()
            .rect_filled(r.shrink(1.0), CornerRadius::same(4), t.hover);
    }
    ui.painter()
        .text(r.center(), Align2::CENTER_CENTER, label, font, t.text);
    if on {
        ui.painter().rect_filled(
            Rect::from_center_size(pos2(r.center().x, r.max.y - 1.0), vec2((w - 12.0).max(12.0), 2.0)),
            CornerRadius::same(1),
            t.accent,
        );
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, a11y));
    resp
}

/// The Format visual tab for the selected visual `sel` (its snapshot entry `v`).
pub fn pane(app: &mut App, ui: &mut Ui, sel: &VisualSel, v: &Value) {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    let Some(snap) = app.loaded.as_ref().map(|x| x.snap.clone()) else {
        return;
    };
    // the mobile layout formats the phone's visual (crate::mobile): its values are the phone's
    let phone;
    let v =
        if crate::mobile::editing(app) {
            let Some(p) = crate::mobile::phone_entry(v) else {
                ui.label(egui::RichText::new(l.t(
                "Place the visual on the phone canvas to format it for the phone.",
                "Placez le visuel sur le canevas du téléphone pour le mettre en forme pour le téléphone.",
            ))
            .color(t.text_dim));
                return;
            };
            let formatted = p.pointer("/mobile/formatted").and_then(Value::as_bool) == Some(true);
            crate::mobile::format_note(app, ui, formatted);
            phone = p;
            &phone
        } else {
            v
        };
    let revision = snap.get("revision").and_then(Value::as_u64).unwrap_or(0);
    ask_props(app, sel, revision);
    let vtype = v.get("type").and_then(Value::as_str).unwrap_or("").to_string();
    // Visual | General
    ui.horizontal(|ui| {
        let f = &mut app.panes.format;
        for (g, en, fr, a11y_en, a11y_fr) in [
            (
                Group::Visual,
                "Visual",
                "Visuel",
                "Visual cards",
                "Cartes du visuel",
            ),
            (
                Group::General,
                "General",
                "Général",
                "General cards",
                "Cartes générales",
            ),
        ] {
            if group_tab(ui, l.t(en, fr), l.t(a11y_en, a11y_fr), f.group == g).clicked() {
                f.group = g;
            }
        }
    });
    ui.add_space(4.0);
    // the search box
    let label = l.t("Search the format cards", "Rechercher dans la mise en forme");
    let r = ui.add(
        egui::TextEdit::singleline(&mut app.panes.format.search)
            .hint_text(l.t("Search", "Rechercher"))
            .char_limit(200)
            .desired_width(ui.available_width()),
    );
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, label));
    ui.add_space(6.0);
    let query = crate::palette::fold(app.panes.format.search.trim());
    let group = app.panes.format.group;
    let list = cards(group, &vtype);
    // the values shown
    let own = app
        .panes
        .format
        .own
        .as_ref()
        .filter(|o| o.page == sel.page && o.visual == sel.visual)
        .map(|o| o.props.clone());
    // the colours the page draws with (the theme's, else Desktop's palette for that report)
    let palette: Vec<String> = bicraft_render::data_colors(&snap);
    let mut swatches: Vec<String> = palette.iter().take(10).cloned().collect();
    let accent = swatches.first().cloned().unwrap_or_else(|| "#118DFF".into());
    for n in ["#FFFFFF", "#F3F2F1", "#C8C6C4", "#605E5C", "#252423", "#000000"] {
        if !swatches.iter().any(|s| s.eq_ignore_ascii_case(n)) {
            swatches.push(n.to_string());
        }
    }
    let mut pages: Vec<(String, String)> = vec![
        ("default".into(), l.t("Default", "Par défaut").into()),
        ("none".into(), l.t("None", "Aucune").into()),
    ];
    for pg in snap
        .pointer("/report/pages")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .take(500)
    {
        let name = pg.get("name").and_then(Value::as_str).unwrap_or("");
        if name.is_empty() || name == sel.page {
            continue;
        }
        let shown = pg
            .get("display_name")
            .and_then(Value::as_str)
            .filter(|d| !d.is_empty())
            .unwrap_or(name);
        pages.push((name.to_string(), clip(shown, 60)));
    }
    let states: BTreeMap<&'static str, Result<(), String>> =
        COMMANDS.iter().map(|id| (*id, ribbon::state(app, id))).collect();
    // the Data colours card's rows: one per series, legend value or category value
    let fields: Vec<Value> = v
        .get("fields")
        .and_then(Value::as_array)
        .map(|a| a.iter().take(500).cloned().collect())
        .unwrap_or_default();
    let show_all_to = To::Object("dataPoint", "showAllDataPoints");
    let show_all = app
        .panes
        .format
        .pending
        .get("dataPoint.showAllDataPoints")
        .cloned()
        .or_else(|| current(own.as_ref(), v, show_all_to))
        .and_then(|x| as_bool(&x))
        .unwrap_or(false);
    let colouring = if list.iter().any(|c| c.key == "dataPoint") {
        colouring(&vtype, &fields, show_all)
    } else {
        Colouring::None
    };
    let (mut series_cut, mut series_wait) = (false, false);
    let values: Option<Vec<Value>> = match &colouring {
        Colouring::Values { field, table, column } => {
            // the values read at an earlier revision stay shown while they are read again
            let known = app.panes.format.values.get(field).cloned();
            let fresh = known.as_ref().is_some_and(|x| x.0 == revision);
            if !fresh {
                let key = (field.clone(), revision);
                if !app.panes.format.values_asked.contains(&key)
                    && ribbon::state(app, "column.values").is_ok()
                {
                    app.panes.format.values_asked.insert(key);
                    app.run(
                        Call {
                            id: "column.values",
                            params: json!({"table": table, "column": column, "limit": MAX_SERIES_ROWS}),
                        },
                        Then::FormatValues {
                            field: field.clone(),
                            revision,
                        },
                    );
                }
            }
            match known {
                Some((_, vals, cut)) => {
                    series_cut = cut;
                    Some(vals)
                }
                None => {
                    series_wait = true;
                    None
                }
            }
        }
        _ => None,
    };
    let series = series_rows(&colouring, v, values.as_deref(), &palette, l);
    let pending = std::mem::take(&mut app.panes.format.pending);
    let mut drafts = std::mem::take(&mut app.panes.format.drafts);
    // Apply settings to › State (buttons, navigators, shapes)
    let offered = states_of(&vtype);
    if group == Group::Visual && !offered.is_empty() {
        let cur = app.panes.format.state.clone();
        let cur = if offered.iter().any(|(k, ..)| *k == cur) {
            cur
        } else {
            "default".to_string()
        };
        let mut pick = cur.clone();
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(l.t("Apply settings to: State", "Appliquer les paramètres à : État"))
                    .small()
                    .color(t.text_dim),
            );
            let shown = offered
                .iter()
                .find(|(k, ..)| *k == pick)
                .map_or("", |(_, en, fr)| l.t(en, fr));
            egui::ComboBox::from_id_salt("bicraft-format-state")
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    for (k, en, fr) in offered {
                        ui.selectable_value(&mut pick, (*k).to_string(), l.t(en, fr));
                    }
                });
        });
        ui.add_space(4.0);
        app.panes.format.state = pick;
    }
    let state = if offered.iter().any(|(k, ..)| *k == app.panes.format.state) {
        app.panes.format.state.clone()
    } else {
        "default".to_string()
    };
    let mut open = std::mem::take(&mut app.panes.format.open);
    let mut cx = Cx {
        sel,
        vtype: &vtype,
        own: own.as_ref(),
        snap: v,
        pending: &pending,
        drafts: &mut drafts,
        swatches: &swatches,
        accent: &accent,
        pages: &pages,
        states: &states,
        series,
        series_cut,
        series_wait,
        l,
        state: &state,
        acts: Vec::new(),
    };
    egui::ScrollArea::vertical()
        .id_salt("bicraft-format-cards")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if list.is_empty() {
                ui.label(
                    egui::RichText::new(l.t(
                        "This visual type has no Visual cards here yet: its title, effects and \
                         header are under General, any property by name in Format visual (all \
                         properties).",
                        "Ce type de visuel n'a pas encore de cartes Visuel ici : son titre, ses \
                         effets et son en-tête sont sous Général, toute propriété par son nom dans \
                         Mettre en forme le visuel (toutes les propriétés).",
                    ))
                    .color(t.text_dim),
                );
                ui.add_space(4.0);
                if ui
                    .button(l.t(
                        "Format visual (all properties)…",
                        "Mettre en forme le visuel (toutes les propriétés)…",
                    ))
                    .clicked()
                {
                    cx.acts.push(Act::Invoke("visual.format"));
                }
            }
            let mut shown = 0;
            for card in &list {
                if !query.is_empty() && !matches(card, &query, l) {
                    continue;
                }
                shown += 1;
                let is_open = !query.is_empty() || open.contains(card.key);
                if card_ui(ui, &mut cx, card, is_open) {
                    if open.contains(card.key) {
                        open.remove(card.key);
                    } else {
                        open.insert(card.key.to_string());
                    }
                }
            }
            if shown == 0 && !list.is_empty() {
                ui.label(
                    egui::RichText::new(l.t("No card matches.", "Aucune carte ne correspond."))
                        .color(t.text_dim),
                );
            }
        });
    let acts = std::mem::take(&mut cx.acts);
    let f = &mut app.panes.format;
    f.pending = pending;
    f.drafts = drafts;
    f.open = open;
    for a in acts {
        match a {
            Act::Edit {
                key,
                shown,
                id,
                params,
            } => send(app, sel, key, shown, id, params),
            Act::Invoke(id) => {
                if let Some(c) = crate::registry::find(id) {
                    let mut p = ribbon::prefill(app, c, None);
                    if let Some(o) = p.as_object_mut() {
                        o.insert("page".into(), json!(sel.page));
                        o.insert("visual".into(), json!(sel.visual));
                    }
                    // the map file is chosen in the form (a path, or the map text)
                    if matches!(id, "visual.format" | "shape_map.set_map") {
                        crate::form::open(app, id, p);
                    } else {
                        ribbon::invoke(app, id, p);
                    }
                }
            }
            Act::Fx { target, name } => crate::conditional_dialog::open(app, sel, v, target, name),
        }
    }
    crate::conditional_dialog::show(app, ui.ctx(), sel, v, &swatches);
}

/// The edit `(id, params)` as it is sent: while the mobile layout is edited, the phone's
/// (`visual.mobile_format`, [`crate::mobile::format_edit`]); `None` (a note in the status bar)
/// for an edit the mobile layout has no place for.
fn routed(app: &mut App, id: &'static str, params: Value) -> Option<(&'static str, Value)> {
    if !crate::mobile::editing(app) || id == "visual.mobile_format" {
        return Some((id, params));
    }
    match crate::mobile::format_edit(id, &params) {
        Some(p) => Some(("visual.mobile_format", p)),
        None => {
            app.status = app
                .lang
                .t(
                    "This setting is the visual's own: switch the mobile layout off to change it.",
                    "Ce réglage est celui du visuel : désactivez la disposition mobile pour le changer.",
                )
                .into();
            None
        }
    }
}

/// Sends one edit of the selected visual (refused with the reason in the status bar when the
/// session cannot run it).
pub(crate) fn send(
    app: &mut App,
    sel: &VisualSel,
    key: Option<String>,
    shown: Value,
    id: &'static str,
    params: Value,
) {
    let Some((id, params)) = routed(app, id, params) else {
        return;
    };
    if let Err(why) = ribbon::state(app, id) {
        app.status = format!("{} {why}", app.lang.t("Unavailable:", "Indisponible :"));
        return;
    }
    let mut p = params.as_object().cloned().unwrap_or_default();
    p.insert("page".into(), json!(sel.page));
    p.insert("visual".into(), json!(sel.visual));
    let sent = app.run(
        Call {
            id,
            params: Value::Object(p),
        },
        Then::FormatEdit {
            page: sel.page.clone(),
            visual: sel.visual.clone(),
        },
    );
    if sent && let Some(k) = key {
        app.panes.format.pending.insert(k, shown);
    }
}

/// [`send`] without a pending value: true when the command was sent.
pub(crate) fn send_tracked(app: &mut App, sel: &VisualSel, id: &'static str, params: Value) -> bool {
    let Some((id, params)) = routed(app, id, params) else {
        return false;
    };
    if let Err(why) = ribbon::state(app, id) {
        app.status = format!("{} {why}", app.lang.t("Unavailable:", "Indisponible :"));
        return false;
    }
    let mut p = params.as_object().cloned().unwrap_or_default();
    p.insert("page".into(), json!(sel.page));
    p.insert("visual".into(), json!(sel.visual));
    app.run(
        Call {
            id,
            params: Value::Object(p),
        },
        Then::FormatEdit {
            page: sel.page.clone(),
            visual: sel.visual.clone(),
        },
    )
}

/// Whether `card` matches the search `q` (folded): its title, a section's or a property's name.
fn matches(card: &Card, q: &str, l: Lang) -> bool {
    let has = |s: &str| crate::palette::fold(s).contains(q);
    has(card.title(l))
        || card.sections.iter().any(|s| {
            s.title.is_some_and(|(en, fr)| has(l.t(en, fr))) || s.props.iter().any(|p| has(l.t(p.en, p.fr)))
        })
}

/// One card: its header (chevron, title, switch), then when open its sections and the Reset to
/// default link. True when the header was clicked (open or close it).
fn card_ui(ui: &mut Ui, cx: &mut Cx<'_>, card: &Card, open: bool) -> bool {
    let t = Tokens::get(ui.ctx());
    let l = cx.l;
    let title = card.title(l);
    let mut toggled = false;
    egui::Frame::NONE
        .stroke(Stroke::new(1.0, t.card_border))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(6, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let w = (ui.available_width() - if card.switch.is_some() { 44.0 } else { 0.0 }).max(40.0);
                ui.allocate_ui(vec2(w, 26.0), |ui| {
                    toggled = widgets::section(ui, title, open) != open;
                });
                if let Some((to, def)) = card.switch {
                    let key = format!("{}.show", card.key);
                    switch_ui(ui, cx, &key, to, def, title);
                }
            });
            if open {
                for s in card.sections.iter() {
                    section_ui(ui, cx, card, s);
                }
                if card.key == "dataPoint" {
                    series_ui(ui, cx, card);
                }
                if matches!(card.key, "rowSubtotals" | "columnSubtotals") {
                    levels_ui(ui, cx, card);
                }
                if card.key == "dataFormat" {
                    let st = cx.state("visual.set_data_format");
                    if let Some(params) = crate::data_format_card::ui(ui, l, cx.snap, &cx.sel.visual, &st) {
                        cx.acts.push(Act::Edit {
                            key: None,
                            shown: Value::Null,
                            id: "visual.set_data_format",
                            params,
                        });
                    }
                }
                ui.add_space(2.0);
                if let Some((id, params)) = reset(card) {
                    let st = cx.state(id);
                    let r = ui.add_enabled(
                        st.is_ok(),
                        egui::Button::new(
                            egui::RichText::new(l.t("Reset to default", "Rétablir les valeurs par défaut"))
                                .small()
                                .color(t.accent),
                        )
                        .frame(false),
                    );
                    let a11y = format!(
                        "{} {title}",
                        l.t("Reset to default", "Rétablir les valeurs par défaut")
                    );
                    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, st.is_ok(), &a11y));
                    if r.clicked() {
                        cx.acts.push(Act::Edit {
                            key: None,
                            shown: Value::Null,
                            id,
                            params,
                        });
                    }
                }
            }
        });
    ui.add_space(4.0);
    toggled
}

/// The Data colours card's colour rows under its own properties: one per series, legend value or
/// category value ([`colouring`]), each a swatch writing `visual.series {target, color}` (one
/// command, one undo step). A row the visual does not colour shows the theme's colour for its
/// place.
fn series_ui(ui: &mut Ui, cx: &mut Cx<'_>, card: &Card) {
    let t = Tokens::get(ui.ctx());
    let l = cx.l;
    if cx.series_wait {
        ui.label(
            egui::RichText::new(l.t("Reading the values…", "Lecture des valeurs…"))
                .small()
                .color(t.text_dim),
        );
        return;
    }
    if cx.series.is_empty() {
        return;
    }
    ui.add_space(4.0);
    let st = cx.state("visual.series");
    let rows = std::mem::take(&mut cx.series);
    for r in &rows {
        let key = format!("dataPoint.series.{}", r.target);
        let cur = cx
            .pending
            .get(&key)
            .cloned()
            .or_else(|| r.color.clone().map(Value::from))
            .unwrap_or_else(|| json!(r.def));
        let name = format!("{} {}", card.title(l), r.label);
        ui.add_enabled_ui(st.is_ok(), |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(clip(&r.label, 30)).small());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(c) = pick_color(
                        ui,
                        l,
                        cx.swatches,
                        cx.drafts,
                        &cx.sel.visual,
                        &key,
                        &r.label,
                        &cur,
                        &name,
                    ) {
                        cx.acts.push(Act::Edit {
                            key: Some(key.clone()),
                            shown: json!(c),
                            id: "visual.series",
                            params: json!({"target": r.target, "color": c}),
                        });
                    }
                });
            });
        });
    }
    cx.series = rows;
    if cx.series_cut {
        ui.label(
            egui::RichText::new(l.t(
                "Only the first 100 values are listed.",
                "Seules les 100 premières valeurs sont listées.",
            ))
            .small()
            .color(t.text_dim),
        );
    }
    if let Err(why) = &st {
        ui.label(
            egui::RichText::new(format!(
                "{} {}",
                l.t("Unavailable:", "Indisponible :"),
                clip(why, 120)
            ))
            .small()
            .color(t.text_faint),
        );
    }
}

/// The per-level rows of a matrix's Row subtotals or Column subtotals card, shown while "Per row
/// level" / "Per column level" is on: for each level of the hierarchy shown (Desktop names a
/// subtotal by the field it totals over: the top field's entry is the grand total), a Show
/// subtotal switch and a Subtotal label, each one `visual.format` command on the `subTotals`
/// entry whose `selector.metadata` is that field.
fn levels_ui(ui: &mut Ui, cx: &mut Cx<'_>, card: &Card) {
    let t = Tokens::get(ui.ctx());
    let l = cx.l;
    let (role, flag, prefix, role_name) = if card.key == "rowSubtotals" {
        (0u8, "perRowLevel", "row", "Rows")
    } else {
        (1u8, "perColumnLevel", "column", "Columns")
    };
    let per = as_bool(&cx.value(
        &format!("{}.{flag}", card.key),
        To::Object("subTotals", flag),
        Def::Bool(false),
    ))
    .unwrap_or(false);
    if !per {
        return;
    }
    let refs: Vec<String> = cx
        .snap
        .pointer(&format!("/data/{prefix}_level_refs"))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .take(MAX_LEVEL_ROWS)
                .map(|x| x.as_str().unwrap_or("").to_string())
                .collect()
        })
        .unwrap_or_default();
    // the levels' names: the data's grouping columns of the role, in order
    let names: Vec<String> = cx
        .snap
        .pointer("/data/columns")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|c| c.get("role").and_then(Value::as_str) == Some(role_name))
        .filter_map(|c| c.get("name").and_then(Value::as_str).map(str::to_string))
        .collect();
    if refs.is_empty() {
        ui.label(
            egui::RichText::new(l.t(
                "Each level's subtotal is set here once the matrix has fields on it.",
                "Le sous-total de chaque niveau se règle ici dès que la matrice a des champs.",
            ))
            .small()
            .color(t.text_dim),
        );
        return;
    }
    let st = cx.state("visual.format");
    let enabled = st.is_ok();
    let title = card.title(l);
    for (i, r) in refs.iter().enumerate() {
        let Ok(level) = u8::try_from(i) else { break };
        let name = names.get(i).cloned().unwrap_or_else(|| r.clone());
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(clip(&name, 40))
                .font(theme::bold(12.0))
                .color(t.text),
        );
        // Show subtotal
        let to = To::Level(role, level, "levelSubtotalEnabled");
        let key = format!("{}.level.{i}.show", card.key);
        let on = as_bool(&cx.value(&key, to, Def::Bool(true))).unwrap_or(true);
        let show = l.t("Show subtotal", "Afficher le sous-total");
        let a11y_show = format!("{title} {name} {show}");
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(show).small().color(t.text_dim));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::switch(ui, on, enabled, &a11y_show).clicked() {
                    cx.commit(key.clone(), to, Ctl::Toggle, json!(!on));
                }
            });
        });
        // Subtotal label (committed on Enter or when the box loses the focus)
        let to = To::Level(role, level, "levelSubtotalLabel");
        let key = format!("{}.level.{i}.label", card.key);
        let label = l.t("Subtotal label", "Libellé du sous-total");
        let a11y_label = format!("{title} {name} {label}");
        let now = as_text(&cx.value(&key, to, Def::None)).unwrap_or_default();
        ui.label(egui::RichText::new(label).small().color(t.text_dim));
        let visual = cx.sel.visual.clone();
        let entry = cx
            .drafts
            .entry(key.clone())
            .or_insert_with(|| (visual.clone(), now.clone()));
        if entry.0 != visual {
            *entry = (visual, now.clone());
        }
        let resp = ui.add_enabled(
            enabled,
            egui::TextEdit::singleline(&mut entry.1)
                .char_limit(MAX_TEXT)
                .desired_width(ui.available_width()),
        );
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, enabled, &a11y_label));
        let mut typed = None;
        if resp.lost_focus() {
            if ui.input(|x| x.key_pressed(Key::Escape)) {
                entry.1 = now;
            } else if entry.1 != now {
                typed = Some(entry.1.clone());
            }
        } else if !resp.has_focus() {
            entry.1 = now;
        }
        if let Some(text) = typed {
            cx.commit(key, to, Ctl::Text, json!(text));
        }
    }
}

/// A switch bound to `to` (`show`), labelled `label`.
fn switch_ui(ui: &mut Ui, cx: &mut Cx<'_>, key: &str, to: To, def: bool, label: &str) {
    let on = as_bool(&cx.value(key, to, Def::Bool(def))).unwrap_or(def);
    let (id, _) = edit(to, Ctl::Toggle, json!(true));
    let st = cx.state(id);
    let r = widgets::switch(ui, on, st.is_ok(), label);
    let r = match &st {
        Ok(()) => r.on_hover_text(if on {
            cx.l.t("On: click to turn it off", "Activé : cliquez pour désactiver")
        } else {
            cx.l.t("Off: click to turn it on", "Désactivé : cliquez pour activer")
        }),
        Err(why) => r.on_hover_text(format!("{} {why}", cx.l.t("Unavailable:", "Indisponible :"))),
    };
    if r.clicked() {
        cx.commit(key.to_string(), to, Ctl::Toggle, json!(!on));
    }
}

/// A section of a card: its sub-header with a switch, then its properties (consecutive style
/// buttons in one row).
fn section_ui(ui: &mut Ui, cx: &mut Cx<'_>, card: &Card, s: &Section) {
    let t = Tokens::get(ui.ctx());
    let l = cx.l;
    if let Some((en, fr)) = s.title {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let name = l.t(en, fr);
            ui.label(egui::RichText::new(name).font(theme::bold(12.0)).color(t.text));
            if let Some((to, def)) = s.switch {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let key = format!("{}.{en}.show", card.key);
                    switch_ui(ui, cx, &key, to, def, name);
                });
            }
        });
    }
    let mut i = 0;
    while let Some(prop) = s.props.get(i) {
        if let Ctl::Style(_) = prop.ctl {
            let run: Vec<&Prop> = s
                .props
                .get(i..)
                .unwrap_or_default()
                .iter()
                .take_while(|p| matches!(p.ctl, Ctl::Style(_)))
                .collect();
            i += run.len();
            styles_ui(ui, cx, card, s, &run);
            continue;
        }
        prop_ui(ui, cx, card, s, prop);
        i += 1;
    }
}

/// A property's accessible name: `<card> [<section>] <property>` (`Title Font size`, `Effects
/// Background Colour`).
fn a11y(card: &Card, s: &Section, prop: &Prop, l: Lang) -> String {
    match s.title {
        Some((en, fr)) => format!("{} {} {}", card.title(l), l.t(en, fr), l.t(prop.en, prop.fr)),
        None => format!("{} {}", card.title(l), l.t(prop.en, prop.fr)),
    }
}

/// Bold, italic and underline buttons in one row.
fn styles_ui(ui: &mut Ui, cx: &mut Cx<'_>, card: &Card, s: &Section, run: &[&Prop]) {
    let l = cx.l;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(l.t("Style", "Style")).small());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            for prop in run.iter().rev() {
                let Ctl::Style(glyph) = prop.ctl else { continue };
                let key = format!("{}.{}", card.key, prop.key);
                let on = as_bool(&cx.value(&key, prop.to, prop.def)).unwrap_or(false);
                let text = egui::RichText::new(glyph);
                let text = match glyph {
                    "B" => text.strong(),
                    "I" => text.italics(),
                    _ => text.underline(),
                };
                let (id, _) = edit(prop.to, prop.ctl, json!(true));
                let st = cx.state(id);
                let r = ui.add_enabled(st.is_ok(), egui::Button::selectable(on, text));
                let name = a11y(card, s, prop, l);
                r.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, st.is_ok(), on, &name));
                if r.on_hover_text(l.t(prop.en, prop.fr)).clicked() {
                    cx.commit(key, prop.to, prop.ctl, json!(!on));
                }
            }
        });
    });
}

/// One property's row: its name, then its control.
fn prop_ui(ui: &mut Ui, cx: &mut Cx<'_>, card: &Card, s: &Section, prop: &Prop) {
    let l = cx.l;
    let t = Tokens::get(ui.ctx());
    let key = format!("{}.{}", card.key, prop.key);
    let name = a11y(card, s, prop, l);
    let label = l.t(prop.en, prop.fr);
    let (id, _) = edit(prop.to, prop.ctl, Value::Null);
    let id = match prop.ctl {
        Ctl::Run(cmd, ..) | Ctl::RunWith(cmd, ..) => cmd,
        _ => id,
    };
    let st = cx.state(id);
    let enabled = st.is_ok();
    let cur = cx.value(&key, prop.to, prop.def);
    ui.add_enabled_ui(enabled, |ui| match prop.ctl {
        Ctl::Text => {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(label).small().color(t.text_dim));
                // a title's or subtitle's text from a field (Field value)
                if let Some(target) = fx_target(cx.vtype, prop.to, prop.ctl) {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        fx_ui(ui, cx, target, &name);
                    });
                }
            });
            let now = as_text(&cur).unwrap_or_default();
            let visual = cx.sel.visual.clone();
            let entry = cx
                .drafts
                .entry(key.clone())
                .or_insert_with(|| (visual.clone(), now.clone()));
            if entry.0 != visual {
                *entry = (visual, now.clone());
            }
            let r = ui.add(
                egui::TextEdit::singleline(&mut entry.1)
                    .char_limit(MAX_TEXT)
                    .desired_width(ui.available_width()),
            );
            r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, enabled, &name));
            let mut typed = None;
            if r.lost_focus() {
                if ui.input(|i| i.key_pressed(Key::Escape)) {
                    entry.1 = now;
                } else if entry.1 != now {
                    typed = Some(entry.1.clone());
                }
            } else if !r.has_focus() {
                entry.1 = now;
            }
            if let Some(text) = typed {
                cx.commit(key, prop.to, prop.ctl, json!(text));
            }
        }
        Ctl::Run(cmd, en, fr) => {
            let r = ui.button(l.t(en, fr));
            r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, &name));
            if r.clicked() {
                cx.acts.push(Act::Invoke(cmd));
            }
        }
        Ctl::RunWith(cmd, param, value, en, fr) => {
            let r = ui.button(l.t(en, fr));
            r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, &name));
            if r.clicked() {
                cx.acts.push(Act::Edit {
                    key: None,
                    shown: Value::Null,
                    id: cmd,
                    params: json!({ param: value }),
                });
            }
        }
        _ => {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(label).small());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Desktop's fx button, at the right of the colour it formats
                    if let Some(target) = fx_target(cx.vtype, prop.to, prop.ctl) {
                        fx_ui(ui, cx, target, &name);
                    }
                    control(ui, cx, &key, prop, &cur, &name);
                });
            });
        }
    });
    if let Err(why) = &st {
        ui.label(
            egui::RichText::new(format!(
                "{} {}",
                l.t("Unavailable:", "Indisponible :"),
                clip(why, 120)
            ))
            .small()
            .color(t.text_faint),
        );
    }
}

/// The `visual.conditional` target of a row Desktop gives an fx button: a colour of the visual's
/// cards (data colours, data labels, a table's values, a card's labels), or in the General
/// cards the title's and subtitle's text and colour, the background's and border's colour.
fn fx_target(vtype: &str, to: To, ctl: Ctl) -> Option<&'static str> {
    use crate::conditional_dialog::{container_target, target_of};
    match (ctl, to) {
        (Ctl::Fx(t), _) => Some(t),
        (Ctl::Color, To::Object(obj, p)) => target_of(vtype, obj, p),
        (Ctl::Color | Ctl::Text, To::Title(k)) => container_target("title", k),
        (Ctl::Color | Ctl::Text, To::Format(g, k)) => container_target(g, k),
        _ => None,
    }
}

/// The control of a property drawn right-aligned in its row.
fn control(ui: &mut Ui, cx: &mut Cx<'_>, key: &str, prop: &Prop, cur: &Value, name: &str) {
    let l = cx.l;
    match prop.ctl {
        Ctl::Size => {
            let size = as_num(cur);
            let shown = size.map_or_else(|| "—".to_string(), |s| format!("{s}"));
            let mut picked = None;
            let r = egui::ComboBox::from_id_salt(("bicraft-format-size", key))
                .selected_text(shown)
                .width(64.0)
                .height(300.0)
                .show_ui(ui, |ui| {
                    for s in crate::rich::SIZES {
                        let s = f64::from(*s);
                        if ui
                            .selectable_label(size.is_some_and(|x| (x - s).abs() < 0.01), format!("{s}"))
                            .clicked()
                        {
                            picked = Some(s);
                        }
                    }
                });
            r.response
                .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, name));
            if let Some(s) = picked {
                cx.commit(key.to_string(), prop.to, prop.ctl, json!(s));
            }
        }
        Ctl::Font => {
            let fam = as_text(cur).map(|s| family(&s)).unwrap_or_default();
            let mut picked = None;
            let r = egui::ComboBox::from_id_salt(("bicraft-format-font", key))
                .selected_text(clip(&fam, 22))
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
            if let Some(f) = picked {
                cx.commit(key.to_string(), prop.to, prop.ctl, json!(f));
            }
        }
        Ctl::Choice(values) | Ctl::Numbered(values) => {
            let now = as_text(cur).unwrap_or_default();
            let shown = values
                .iter()
                .find(|(v, _, _)| v.eq_ignore_ascii_case(&now))
                .map_or_else(|| now.clone(), |(_, en, fr)| l.t(en, fr).to_string());
            let mut picked = None;
            let r = egui::ComboBox::from_id_salt(("bicraft-format-choice", key))
                .selected_text(clip(&shown, 22))
                .width(118.0)
                .show_ui(ui, |ui| {
                    for (v, en, fr) in values {
                        if ui
                            .selectable_label(v.eq_ignore_ascii_case(&now), l.t(en, fr))
                            .clicked()
                        {
                            picked = Some(*v);
                        }
                    }
                });
            r.response
                .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, name));
            if let Some(v) = picked {
                cx.commit(key.to_string(), prop.to, prop.ctl, json!(v));
            }
        }
        Ctl::Pages => {
            let now = as_text(cur).unwrap_or_else(|| "default".into());
            let shown = cx
                .pages
                .iter()
                .find(|(v, _)| v.eq_ignore_ascii_case(&now))
                .map_or_else(|| now.clone(), |(_, d)| d.clone());
            let mut picked = None;
            let r = egui::ComboBox::from_id_salt(("bicraft-format-pages", key))
                .selected_text(clip(&shown, 22))
                .width(128.0)
                .height(300.0)
                .show_ui(ui, |ui| {
                    for (v, d) in cx.pages {
                        if ui.selectable_label(v.eq_ignore_ascii_case(&now), d).clicked() {
                            picked = Some(v.clone());
                        }
                    }
                });
            r.response
                .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, name));
            if let Some(v) = picked {
                cx.commit(key.to_string(), prop.to, prop.ctl, json!(v));
            }
        }
        Ctl::Align => {
            let now = as_text(cur).unwrap_or_else(|| "left".into());
            // right to left: drawn in reverse
            for (v, glyph, en, fr) in [
                ("right", "⇥", "Align right", "Aligner à droite"),
                ("center", "≡", "Center", "Centrer"),
                ("left", "⇤", "Align left", "Aligner à gauche"),
            ] {
                let on = now.eq_ignore_ascii_case(v);
                let r = ui.add(egui::Button::selectable(on, glyph));
                let a = format!("{name} {}", l.t(en, fr));
                r.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, &a));
                if r.on_hover_text(l.t(en, fr)).clicked() && !on {
                    cx.commit(key.to_string(), prop.to, prop.ctl, json!(v));
                }
            }
        }
        Ctl::Toggle => {
            let on = as_bool(cur).unwrap_or(false);
            if widgets::switch(ui, on, true, name).clicked() {
                cx.commit(key.to_string(), prop.to, prop.ctl, json!(!on));
            }
        }
        Ctl::Color => color_ui(ui, cx, key, prop, cur, name),
        Ctl::Percent | Ctl::Number(..) => number_ui(ui, cx, key, prop, cur, name),
        Ctl::Text | Ctl::Style(_) | Ctl::Run(..) | Ctl::RunWith(..) | Ctl::Fx(_) => {}
    }
}

/// The fx button of a colour property (`target` of `visual.conditional`, the property's name
/// `name`): it opens the conditional-formatting dialog, and is marked (accent, selected) when the
/// visual has a conditional format there.
fn fx_ui(ui: &mut Ui, cx: &mut Cx<'_>, target: &'static str, name: &str) {
    let t = Tokens::get(ui.ctx());
    let l = cx.l;
    let on = !crate::conditional_dialog::formats(cx.own, target).is_empty();
    let st = cx.state(crate::conditional_dialog::COMMAND);
    let (rect, r) = ui.allocate_exact_size(
        vec2(24.0, 20.0),
        if st.is_ok() {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let p = ui.painter();
    if on {
        p.rect_filled(rect, CornerRadius::same(3), t.accent_soft);
        p.rect_stroke(
            rect.shrink(0.5),
            CornerRadius::same(3),
            Stroke::new(1.0, t.accent),
            egui::StrokeKind::Inside,
        );
    } else if r.hovered() && st.is_ok() {
        p.rect_filled(rect, CornerRadius::same(3), t.hover);
    }
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        "fx",
        theme::font(12.0),
        if st.is_ok() { t.text } else { t.text_faint },
    );
    let a = format!(
        "{name} {}",
        l.t("Conditional formatting", "Mise en forme conditionnelle")
    );
    r.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, st.is_ok(), on, &a));
    let hover = match (&st, on) {
        (Err(why), _) => format!("{} {why}", l.t("Unavailable:", "Indisponible :")),
        (Ok(()), true) => format!(
            "{} ({})",
            l.t("Conditional formatting", "Mise en forme conditionnelle"),
            l.t("on", "activée")
        ),
        (Ok(()), false) => l
            .t("Conditional formatting", "Mise en forme conditionnelle")
            .to_string(),
    };
    if r.on_hover_text(hover).clicked() {
        cx.acts.push(Act::Fx {
            target,
            name: name.to_string(),
        });
    }
}

/// A colour: its swatch, opening the theme's colours and the neutrals, and a hex box.
fn color_ui(ui: &mut Ui, cx: &mut Cx<'_>, key: &str, prop: &Prop, cur: &Value, name: &str) {
    let hover = cx.l.t(prop.en, prop.fr);
    if let Some(c) = pick_color(
        ui,
        cx.l,
        cx.swatches,
        cx.drafts,
        &cx.sel.visual,
        key,
        hover,
        cur,
        name,
    ) {
        cx.commit(key.to_string(), prop.to, prop.ctl, json!(c));
    }
}

/// A colour swatch (40 × 20) showing `cur`, whose popup offers `swatches` (the theme's colours,
/// the neutrals) and a hex box (its text kept in `drafts` for `visual`): the colour picked this
/// frame.
#[allow(clippy::too_many_arguments)]
pub(crate) fn pick_color(
    ui: &mut Ui,
    l: Lang,
    swatches: &[String],
    drafts: &mut BTreeMap<String, (String, String)>,
    visual: &str,
    key: &str,
    hover: &str,
    cur: &Value,
    name: &str,
) -> Option<String> {
    let hex = as_text(cur).filter(|c| crate::form::color(c).is_some());
    let fill = hex
        .as_deref()
        .and_then(crate::form::color)
        .unwrap_or(Color32::TRANSPARENT);
    let t = Tokens::get(ui.ctx());
    let (rect, r) = ui.allocate_exact_size(vec2(40.0, 20.0), Sense::click());
    ui.painter()
        .rect_filled(rect.shrink(1.0), CornerRadius::same(3), fill);
    ui.painter().rect_stroke(
        rect.shrink(1.0),
        CornerRadius::same(3),
        Stroke::new(1.0, t.border),
        egui::StrokeKind::Inside,
    );
    let shown = hex.clone().unwrap_or_else(|| "—".into());
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name));
    let r = r.on_hover_text(format!("{hover} {shown}"));
    let mut picked: Option<String> = None;
    let draft_key = format!("{key}#hex");
    egui::Popup::menu(&r).show(|ui| {
        ui.label(egui::RichText::new(l.t("Theme colours", "Couleurs du thème")).small());
        egui::Grid::new(("bicraft-format-swatches", key))
            .spacing(vec2(3.0, 3.0))
            .show(ui, |ui| {
                for (n, c) in swatches.iter().enumerate() {
                    let col = crate::form::color(c).unwrap_or(Color32::BLACK);
                    let (sr, sresp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
                    ui.painter().rect_filled(sr, CornerRadius::same(2), col);
                    if hex.as_deref().is_some_and(|h| h.eq_ignore_ascii_case(c)) {
                        ui.painter().rect_stroke(
                            sr.expand(1.0),
                            CornerRadius::same(2),
                            Stroke::new(2.0, t.accent),
                            egui::StrokeKind::Outside,
                        );
                    }
                    let a = format!("{name} {c}");
                    sresp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &a));
                    if sresp.on_hover_text(c).clicked() {
                        picked = Some(c.clone());
                        ui.close();
                    }
                    if n % 6 == 5 {
                        ui.end_row();
                    }
                }
            });
        ui.add_space(4.0);
        let entry = drafts
            .entry(draft_key.clone())
            .or_insert_with(|| (visual.to_string(), hex.clone().unwrap_or_default()));
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(l.t("Hex", "Hex")).small());
            let er = ui.add(
                egui::TextEdit::singleline(&mut entry.1)
                    .char_limit(9)
                    .desired_width(80.0)
                    .hint_text("#118DFF"),
            );
            let a = format!("{name} {}", l.t("hex", "hex"));
            er.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, &a));
            let typed = entry.1.trim().to_string();
            if er.lost_focus()
                && ui.input(|i| i.key_pressed(Key::Enter))
                && crate::form::color(&typed).is_some()
            {
                picked = Some(typed.to_ascii_uppercase());
                ui.close();
            }
        });
    });
    if picked.is_some() {
        drafts.remove(&draft_key);
    }
    picked
}

/// A percentage or a number (a drag value): committed when let go or typed.
fn number_ui(ui: &mut Ui, cx: &mut Cx<'_>, key: &str, prop: &Prop, cur: &Value, name: &str) {
    let (lo, hi) = match prop.ctl {
        Ctl::Number(lo, hi) => (lo, hi),
        _ => (0.0, 100.0),
    };
    let visual = cx.sel.visual.clone();
    let now = as_num(cur).unwrap_or(lo).clamp(lo, hi);
    let mut x = cx
        .drafts
        .get(key)
        .filter(|d| d.0 == visual)
        .and_then(|d| d.1.parse::<f64>().ok())
        .unwrap_or(now);
    let r = match prop.ctl {
        Ctl::Percent => ui.add(
            egui::DragValue::new(&mut x)
                .range(lo..=hi)
                .speed(0.5)
                .max_decimals(0)
                .suffix(" %"),
        ),
        _ => ui.add(
            egui::DragValue::new(&mut x)
                .range(lo..=hi)
                .speed(0.5)
                .max_decimals(0),
        ),
    };
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Slider, true, name));
    let busy = r.dragged() || r.has_focus();
    let done = r.drag_stopped() || r.lost_focus() || (r.changed() && !busy);
    if done {
        cx.drafts.remove(key);
        if (x - now).abs() > f64::EPSILON {
            cx.commit(key.to_string(), prop.to, prop.ctl, json!(x.round()));
        }
    } else if busy {
        cx.drafts.insert(key.to_string(), (visual, x.to_string()));
    } else {
        cx.drafts.remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_card_resets_with_one_command() {
        let mut all: Vec<Card> = GENERAL.to_vec();
        for ty in [
            "clusteredColumnChart",
            "pieChart",
            "card",
            "tableEx",
            "slicer",
            "image",
            "funnel",
        ] {
            all.extend(visual_cards(ty));
        }
        for c in &all {
            // a card of fx buttons only (Cell elements) has nothing of its own to reset
            if targets(c).is_empty() {
                assert!(
                    c.sections
                        .iter()
                        .flat_map(|s| s.props.iter())
                        .all(|p| matches!(p.ctl, Ctl::Fx(_))),
                    "{}",
                    c.key
                );
                assert!(reset(c).is_none());
                continue;
            }
            let (id, p) = reset(c).unwrap_or_else(|| panic!("{} has no reset", c.key));
            assert!(COMMANDS.contains(&id), "{id}");
            assert!(p.is_object());
            // a card writes through one command only
            let ids: BTreeSet<&str> = targets(c)
                .into_iter()
                .map(|t| edit(t, Ctl::Text, json!(1)).0)
                .collect();
            assert_eq!(ids.len(), 1, "{} writes through {ids:?}", c.key);
        }
    }

    #[test]
    fn edits_name_their_command() {
        assert_eq!(
            edit(To::Title("text"), Ctl::Text, json!("Sales")),
            ("visual.set_title", json!({"title": {"text": "Sales"}}))
        );
        assert_eq!(
            edit(To::Object("dataPoint", "fill"), Ctl::Color, json!("#FF0000")),
            (
                "visual.format",
                json!({"objects": [{"object": "dataPoint", "property": "fill", "value": {"color": "#FF0000"}}]})
            )
        );
        assert_eq!(
            edit(To::Axis("y", "show"), Ctl::Toggle, json!(false)),
            ("visual.axis", json!({"axis": "y", "show": false}))
        );
        assert_eq!(
            reset(&EFFECTS),
            Some((
                "visual.format",
                json!({"background": null, "border": null, "shadow": null})
            ))
        );
    }

    #[test]
    fn current_reads_own_then_snapshot_values() {
        let snap = json!({"title": "Sales", "format": {"title": {"font_size": 16, "font_family": "'Segoe UI Semibold'"},
                          "x_axis": {"labels_color": "#123456"}, "background": {"show": true}}});
        assert_eq!(current(None, &snap, To::Title("text")), Some(json!("Sales")));
        assert_eq!(current(None, &snap, To::Title("font_size")), Some(json!(16)));
        assert_eq!(
            current(None, &snap, To::Axis("x", "color")),
            Some(json!("#123456"))
        );
        let own = json!({"title": "Own", "format": {"background": false, "padding": 8,
                         "objects": [{"object": "labels", "property": "color", "value": {"color": "#AA0000"}}]},
                         "axes": {"x": {"show": false}}});
        assert_eq!(current(Some(&own), &snap, To::Title("text")), Some(json!("Own")));
        assert_eq!(current(Some(&own), &snap, To::Title("show")), Some(json!(true)));
        assert_eq!(
            current(Some(&own), &snap, To::Format("background", "show")),
            Some(json!(false))
        );
        assert_eq!(current(Some(&own), &snap, To::Whole("padding")), Some(json!(8)));
        assert_eq!(
            current(Some(&own), &snap, To::Axis("x", "show")),
            Some(json!(false))
        );
        assert_eq!(
            as_text(&current(Some(&own), &snap, To::Object("labels", "color")).unwrap()),
            Some("#AA0000".into())
        );
        assert_eq!(
            family("'Segoe UI Semibold', wf_segoe-ui_semibold, sans-serif"),
            "Segoe UI Semibold"
        );
        // hostile shapes give nothing, never a panic
        for bad in [
            json!(null),
            json!([1, 2]),
            json!("x"),
            json!({"format": [1], "title": {"show": "?"}}),
        ] {
            for to in targets(&TITLE)
                .into_iter()
                .chain(targets(&EFFECTS))
                .chain(targets(
                    visual_cards("clusteredColumnChart")
                        .first()
                        .expect("the column chart has Visual cards"),
                ))
            {
                let _ = current(Some(&bad), &bad, to);
            }
        }
    }
}
