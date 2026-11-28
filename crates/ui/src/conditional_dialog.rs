//! The conditional-formatting dialog of the Format visual tab (ROADMAP M11.22, docs/gaps.md
//! G54), opened by the **fx** button Desktop draws beside a colour property: a chart's data
//! colour and data labels' colour, a table's or matrix's values background and text colour, a
//! card's callout and category label colours, and in the General cards the title's and
//! subtitle's text colours, the visual's background and border colours. Beside the title's and
//! subtitle's text, it offers Field value only (a title from a text field, Microsoft Learn
//! "Expression-based titles"). A table's or matrix's Cell elements card opens it for the values'
//! **data bars**, **icons** and **web URL**.
//!
//! As Desktop's dialogs (Microsoft Learn, "Apply conditional table formatting in Power BI"):
//! - colours: the format style (Gradient, Rules, Field value), for a table the series formatted
//!   and what it applies to (values, values and totals, totals), the field the colours are based
//!   on (a measure, or a column with its summarization), then
//!   - Gradient: how empty values are formatted, the minimum (lowest value or a number), an
//!     optional middle colour (a diverging scale: the middle value or a number) and the maximum
//!     (highest value or a number), a colour each;
//!   - Rules: "If value" an operator and a number, optionally "and" a second operator and
//!     number, "then" a colour; New rule, delete, Reverse colour order;
//!   - Field value: the field's values are the colours;
//! - icons: rules as above with an icon each, the icon layout (left or right of the data, icon
//!   only), its alignment (top, middle, bottom) and a style (an icon set filling three rules);
//! - data bars: Show bar only, the minimum and maximum (lowest / highest value or a number), the
//!   positive and negative bars' colours, the bar direction and the axis colour;
//! - web URL: the field whose text is the link.
//!
//! A **preview** under the controls draws sample values with the format, computed as the viewer
//! evaluates it (`crates/engine/src/preview_format.rs`). OK sends **one** `visual.conditional`
//! command (one undo step); Remove sends the same command with `rule: null`. The dialog reads an
//! existing format back from `visual.props` (`props.conditional`), so it opens on what the visual
//! has. Rules' bounds and the gradient's stops take a Number or a Percent (of the range from the
//! lowest to the highest value); the gradient's Percentile stops are not offered (docs/gaps.md
//! G54).

use std::collections::BTreeMap;

use egui::{Color32, CornerRadius, RichText, Sense, Stroke, Ui, vec2};
use serde_json::{Map, Value, json};

use crate::App;
use crate::app::clip;
use crate::i18n::Lang;
use crate::ribbon::VisualSel;
use crate::theme::{self, Tokens};
use crate::widgets;

/// The command the dialog runs.
pub const COMMAND: &str = "visual.conditional";

/// Most fields the field lists offer.
const MAX_FIELDS: usize = 2000;
/// Most rules one format keeps (the engine allows 50 cases).
const MAX_CASES: usize = 50;
/// The sample values the preview draws.
const SAMPLES: usize = 5;

/// What the dialog formats, from its target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A colour (background, font, data colour…): gradient, rules, field value.
    Color,
    /// A text (a title from a field): field value only.
    Text,
    Icons,
    Bars,
    Url,
}

impl Kind {
    pub fn of(target: &str) -> Kind {
        match target {
            "icon" => Kind::Icons,
            "data_bars" => Kind::Bars,
            "web_url" => Kind::Url,
            t if is_text(t) => Kind::Text,
            _ => Kind::Color,
        }
    }
}

/// The format style.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Style {
    #[default]
    Gradient,
    Rules,
    FieldValue,
}

impl Style {
    const ALL: [Style; 3] = [Style::Gradient, Style::Rules, Style::FieldValue];

    fn label(self, l: Lang) -> &'static str {
        match self {
            Style::Gradient => l.t("Gradient", "Dégradé"),
            Style::Rules => l.t("Rules", "Règles"),
            Style::FieldValue => l.t("Field value", "Valeur du champ"),
        }
    }
}

/// The first condition's operators: (op, EN, FR). `blank` is "is blank" (`= null`).
const OPS: &[(&str, &str, &str)] = &[
    ("=", "is", "est égal à"),
    (">", "is greater than", "est supérieur à"),
    (">=", "is greater than or equal to", "est supérieur ou égal à"),
    ("<", "is less than", "est inférieur à"),
    ("<=", "is less than or equal to", "est inférieur ou égal à"),
    ("blank", "is blank", "est vide"),
];

/// The second condition's operators ("and …").
const OPS2: &[(&str, &str, &str)] = &[
    ("<", "is less than", "est inférieur à"),
    ("<=", "is less than or equal to", "est inférieur ou égal à"),
    (">", "is greater than", "est supérieur à"),
    (">=", "is greater than or equal to", "est supérieur ou égal à"),
];

/// `visual.conditional`'s key form of an operator.
fn op_key(op: &str) -> Option<&'static str> {
    Some(match op {
        "=" => "eq",
        ">" => "gt",
        ">=" => "gte",
        "<" => "lt",
        "<=" => "lte",
        _ => return None,
    })
}

/// A column's summarization in a gradient, rules or icons: (PBIR function, EN, FR).
const AGGS: &[(&str, &str, &str)] = &[
    ("Sum", "Sum", "Somme"),
    ("Avg", "Average", "Moyenne"),
    ("Min", "Minimum", "Minimum"),
    ("Max", "Maximum", "Maximum"),
    ("Count", "Count", "Nombre"),
];

/// "What it applies to" (tables and matrices): (`apply_to`, EN, FR).
const APPLY: &[(&str, &str, &str)] = &[
    ("values", "Values only", "Valeurs uniquement"),
    ("values_and_totals", "Values and totals", "Valeurs et totaux"),
    ("totals", "Totals only", "Totaux uniquement"),
];

/// The icon sets of the Style list: (key, EN, FR, high, medium, low icons), Desktop's names.
pub const ICON_SETS: &[(&str, &str, &str, [&str; 3])] = &[
    (
        "circles",
        "Circles",
        "Cercles",
        ["CircleHigh", "SignMedium", "SignLow"],
    ),
    (
        "traffic",
        "Traffic lights",
        "Feux tricolores",
        ["TrafficHigh", "TrafficMedium", "TrafficLow"],
    ),
    (
        "triangles",
        "Triangles",
        "Triangles",
        ["TriangleHigh", "TriangleMedium", "TriangleLow"],
    ),
    (
        "symbols",
        "Symbols",
        "Symboles",
        ["SymbolHigh", "SymbolMedium", "SymbolLow"],
    ),
    (
        "flags",
        "Flags",
        "Drapeaux",
        ["FlagHigh", "FlagMedium", "FlagLow"],
    ),
    ("stars", "Stars", "Étoiles", ["StarHigh", "StarMedium", "StarLow"]),
    (
        "arrows",
        "Arrows",
        "Flèches",
        ["ColoredArrowUp", "ColoredArrowRight", "ColoredArrowDown"],
    ),
];

/// Icon layout: (`position`, EN, FR).
const LAYOUTS: &[(&str, &str, &str)] = &[
    ("before", "Left of data", "À gauche des données"),
    ("after", "Right of data", "À droite des données"),
    ("only", "Icon only", "Icône uniquement"),
];

/// Icon alignment: (`align`, EN, FR).
const ALIGNS: &[(&str, &str, &str)] = &[
    ("top", "Top", "Haut"),
    ("middle", "Middle", "Milieu"),
    ("bottom", "Bottom", "Bas"),
];

/// One rule of the Rules style (or of icons): "If value `op` `value` [and `op2` `value2`] then
/// `color`".
#[derive(Debug, Clone, PartialEq)]
pub struct Case {
    pub op: &'static str,
    /// The number typed.
    pub value: String,
    /// The second condition's operator ("and …"), when on.
    pub op2: Option<&'static str>,
    pub value2: String,
    /// The colour (rules) or the icon's name (icons).
    pub color: String,
    /// The numbers are percents of the range from the lowest to the highest value (Percent).
    pub percent: bool,
}

impl Case {
    /// A rule with one condition.
    pub fn new(op: &'static str, value: &str, color: &str) -> Case {
        Case {
            op,
            value: value.into(),
            op2: None,
            value2: String::new(),
            color: color.into(),
            percent: false,
        }
    }

    /// A rule with two conditions.
    pub fn two(op: &'static str, value: &str, op2: &'static str, value2: &str, color: &str) -> Case {
        Case {
            op,
            value: value.into(),
            op2: Some(op2),
            value2: value2.into(),
            color: color.into(),
            percent: false,
        }
    }

    /// The same rule with its numbers read as percents.
    pub fn percent(self) -> Case {
        Case {
            percent: true,
            ..self
        }
    }
}

/// A rule's number types: Number, Percent (of the range from the lowest to the highest value).
const BOUND_TYPES: &[(&str, &str, &str)] = &[
    ("number", "Number", "Nombre"),
    ("percent", "Percent", "Pourcentage"),
];

/// A gradient's stop: its colour and, when a number is chosen, its value (else the lowest,
/// middle or highest value).
#[derive(Debug, Clone, PartialEq)]
pub struct Stop {
    pub color: String,
    /// `Some(text)`: a number typed; `None`: the lowest / middle / highest value.
    pub number: Option<String>,
    /// The number is a percent (0 to 100) of the range from the lowest to the highest value.
    pub percent: bool,
}

impl Stop {
    fn color(c: &str) -> Stop {
        Stop {
            color: c.into(),
            number: None,
            percent: false,
        }
    }
}

/// How a gradient formats blank values.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Blank {
    /// No colour (the default).
    #[default]
    NoFormat,
    /// As zero.
    Zero,
    /// A colour of its own.
    Color(String),
}

/// The data bars' settings.
#[derive(Debug, Clone, PartialEq)]
pub struct Bars {
    pub bar_only: bool,
    /// `Some(text)`: a number; `None`: the lowest value.
    pub min: Option<String>,
    /// `Some(text)`: a number; `None`: the highest value.
    pub max: Option<String>,
    pub positive: String,
    pub negative: String,
    pub axis: String,
    /// Right to left.
    pub reverse: bool,
}

impl Default for Bars {
    fn default() -> Self {
        Bars {
            bar_only: false,
            min: None,
            max: None,
            positive: "#118DFF".into(),
            negative: "#D64550".into(),
            axis: "#000000".into(),
            reverse: false,
        }
    }
}

/// The dialog's state while it is open (pure view state until OK).
#[derive(Debug, Clone, PartialEq)]
pub struct CfDialog {
    pub page: String,
    pub visual: String,
    /// `visual.conditional`'s `target` (`data_color`, `background`, `icon`, `data_bars`…).
    pub target: &'static str,
    /// The property's name (`Values Background`), the dialog's heading.
    pub name: String,
    /// A table or matrix: the series formatted is chosen.
    pub table: bool,
    /// The field formatted (tables).
    pub series: String,
    /// What the format applies to (tables: `values`, `values_and_totals`, `totals`).
    pub apply: &'static str,
    pub style: Style,
    /// The field the colours are based on: a measure (`[Total]`) or a column (`Sales[Amount]`).
    pub field: String,
    /// A column's summarization (gradient, rules, icons).
    pub agg: &'static str,
    pub min: Stop,
    /// The middle stop, when on (a diverging scale).
    pub mid: Option<Stop>,
    pub max: Stop,
    pub blank: Blank,
    pub cases: Vec<Case>,
    /// Icons: `before`, `after`, `only`.
    pub layout: &'static str,
    /// Icons: `top`, `middle`, `bottom`.
    pub align: &'static str,
    pub bars: Bars,
    /// The visual has a conditional format on this property (and series): Remove is offered.
    pub existing: bool,
    /// Hex boxes being typed in the colour popups.
    pub drafts: BTreeMap<String, (String, String)>,
}

/// A field of the model the lists offer.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldChoice {
    /// `[Measure]` or `Table[Column]`.
    pub text: String,
    /// `Table › Name`.
    pub label: String,
    pub measure: bool,
    pub numeric: bool,
}

/// The `visual.conditional` target of property `object.prop` of a visual of type `vtype`, when
/// Desktop's format pane gives it an fx button and the catalog allows a rule there.
pub fn target_of(vtype: &str, object: &str, prop: &str) -> Option<&'static str> {
    let t = match (vtype, object, prop) {
        ("tableEx" | "pivotTable", "values", "backColor") => "background",
        ("tableEx" | "pivotTable", "values", "fontColor") => "font_color",
        ("card", "labels", "color") => "font_color",
        ("card", "categoryLabels", "color") => "category_color",
        (t, "dataPoint", "fill") if !matches!(t, "tableEx" | "pivotTable" | "card") => "data_color",
        (t, "labels", "color") if !matches!(t, "tableEx" | "pivotTable" | "card") => "label_color",
        // one value for the whole visual (axes, legend, gauge, KPI)
        (_, "categoryAxis", "labelColor") => "category_axis_color",
        (_, "categoryAxis", "titleColor") => "category_title_color",
        (_, "valueAxis", "labelColor") => "value_axis_color",
        (_, "valueAxis", "titleColor") => "value_title_color",
        (_, "legend", "labelColor") => "legend_color",
        ("gauge", "dataPoint", "target") => "target_color",
        ("gauge", "target", "color") => "target_label_color",
        ("gauge", "calloutValue", "color") => "callout_color",
        ("kpi", "indicator", "fontColor") => "indicator_color",
        ("kpi", "goals", "goalFontColor") => "goal_color",
        ("kpi", "goals", "distanceFontColor") => "distance_color",
        ("kpi", "status", "goodColor") => "good_color",
        ("kpi", "status", "neutralColor") => "neutral_color",
        ("kpi", "status", "badColor") => "bad_color",
        _ => return None,
    };
    bicraft_visuals::property(vtype, object, prop)
        .filter(|p| p.conditional)
        .map(|_| t)
}

/// The `visual.conditional` target of a General card's row (`group.key` as the Format pane
/// writes it: `title.color`, `subtitle.text`, `background.color`, `border.color`): one value for
/// the whole visual, in `visualContainerObjects`.
pub fn container_target(group: &str, key: &str) -> Option<&'static str> {
    Some(match (group, key) {
        ("title", "text") => "title_text",
        ("title", "color") => "title_color",
        ("subtitle", "text") => "subtitle_text",
        ("subtitle", "color") => "subtitle_color",
        ("background", "color") => "visual_background",
        ("border", "color") => "border_color",
        _ => return None,
    })
}

/// Whether `target` is a text (a title from a field: Field value only), not a colour.
pub fn is_text(target: &str) -> bool {
    target.ends_with("_text")
}

/// Whether `target` formats a table's or matrix's values (the series is chosen).
fn per_series(target: &str) -> bool {
    matches!(
        target,
        "background" | "font_color" | "icon" | "data_bars" | "web_url"
    )
}

/// The conditional formats `visual.props` lists for `target` (every series).
pub fn formats<'a>(props: Option<&'a Value>, target: &str) -> Vec<&'a Value> {
    props
        .and_then(|p| p.get("conditional"))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|i| i.get("target").and_then(Value::as_str) == Some(target))
        .collect()
}

/// The conditional format of `target` for `series` (`None`: the visual's whole property).
fn format_of<'a>(props: Option<&'a Value>, target: &str, series: Option<&str>) -> Option<&'a Value> {
    formats(props, target).into_iter().find(|i| {
        let s = i.get("series").and_then(Value::as_str);
        match series {
            Some(x) => s.is_some_and(|s| s.eq_ignore_ascii_case(x)),
            None => true,
        }
    })
}

/// The model's measures and columns, from the snapshot.
pub fn field_choices(snap: &Value) -> Vec<FieldChoice> {
    let mut out = Vec::new();
    for t in snap
        .pointer("/model/tables")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let Some(table) = t.get("name").and_then(Value::as_str) else {
            continue;
        };
        let list = |k: &str| {
            t.get(k)
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default()
        };
        for m in list("measures") {
            let Some(name) = m.get("name").and_then(Value::as_str) else {
                continue;
            };
            out.push(FieldChoice {
                text: crate::panes::measure_text(name),
                label: format!("{table} › {name}"),
                measure: true,
                numeric: true,
            });
        }
        for c in list("columns") {
            let Some(name) = c.get("name").and_then(Value::as_str) else {
                continue;
            };
            if c.get("hidden").and_then(Value::as_bool) == Some(true) {
                continue;
            }
            out.push(FieldChoice {
                text: crate::panes::column_text(table, name),
                label: format!("{table} › {name}"),
                measure: false,
                numeric: matches!(
                    c.get("data_type").and_then(Value::as_str),
                    Some("int64" | "double" | "decimal")
                ),
            });
        }
        if out.len() >= MAX_FIELDS {
            break;
        }
    }
    out.truncate(MAX_FIELDS);
    out
}

/// `Sum(Sales[Amount])` → (`Sales[Amount]`, `Sum`); anything else is its own field.
fn split_agg(text: &str) -> (String, Option<&'static str>) {
    for (f, ..) in AGGS {
        if let Some(rest) = text
            .strip_prefix(f)
            .and_then(|r| r.strip_prefix('('))
            .and_then(|r| r.strip_suffix(')'))
        {
            return (rest.to_string(), Some(*f));
        }
    }
    (text.to_string(), None)
}

/// A number of a rule as the box shows it.
fn num_text(v: &Value) -> String {
    match v {
        // 50.0 shows as 50, as it was typed
        Value::Number(n) => match n.as_f64() {
            Some(x) if x.fract() == 0.0 && x.abs() < 1e15 => format!("{x:.0}"),
            _ => n.to_string(),
        },
        Value::String(s) => s.trim_end_matches(['D', 'L']).to_string(),
        Value::Object(o) => o
            .get("literal")
            .and_then(Value::as_str)
            .map(|s| s.trim_end_matches(['D', 'L']).to_string())
            .unwrap_or_default(),
        _ => String::new(),
    }
}

/// A number typed in a box (`,` or `.` as the decimal separator).
fn parse_num(s: &str) -> Option<f64> {
    s.trim()
        .replace(',', ".")
        .parse::<f64>()
        .ok()
        .filter(|x| x.is_finite())
}

/// A colour stop as `visual.props` gives it: `"#hex"` or `{color, value, percent?}`.
fn read_stop(v: Option<&Value>) -> Option<Stop> {
    match v? {
        Value::String(s) => Some(Stop::color(s)),
        Value::Object(o) => Some(Stop {
            color: o.get("color").and_then(Value::as_str)?.to_string(),
            number: o.get("value").map(num_text).filter(|s| !s.is_empty()),
            percent: o.get("percent").and_then(Value::as_bool) == Some(true),
        }),
        _ => None,
    }
}

/// A case of `visual.props` (`{op, value | values}` or `{gt?, gte?, lt?, lte?, eq?}`) and its
/// colour or icon (`value_key`).
fn read_case(c: &Value, value_key: &str) -> Option<Case> {
    let case = read_bounds(c, value_key)?;
    Some(if c.get("percent").and_then(Value::as_bool) == Some(true) {
        case.percent()
    } else {
        case
    })
}

/// [`read_case`]'s conditions and colour.
fn read_bounds(c: &Value, value_key: &str) -> Option<Case> {
    let color = c.get(value_key).and_then(Value::as_str)?.to_string();
    if let Some(op) = c.get("op").and_then(Value::as_str) {
        if op == "between" {
            let v = c.get("values").and_then(Value::as_array)?;
            return Some(Case::two(
                ">=",
                &v.first().map(num_text).unwrap_or_default(),
                "<=",
                &v.get(1).map(num_text).unwrap_or_default(),
                &color,
            ));
        }
        if op == "=" && c.get("value").is_some_and(Value::is_null) {
            return Some(Case::new("blank", "", &color));
        }
        let (op, ..) = OPS.iter().find(|(o, ..)| *o == op)?;
        return Some(Case::new(
            op,
            &c.get("value").map(num_text).unwrap_or_default(),
            &color,
        ));
    }
    // two conditions written by key: the lower bound first, as the dialog's rows read
    let mut conds: Vec<(&'static str, String)> = Vec::new();
    for (key, op) in [
        ("gte", ">="),
        ("gt", ">"),
        ("eq", "="),
        ("lte", "<="),
        ("lt", "<"),
    ] {
        if let Some(v) = c.get(key) {
            conds.push((op, num_text(v)));
        }
    }
    match conds.as_slice() {
        [(op, v)] => Some(Case::new(op, v, &color)),
        [(op, v), (op2, v2)] => {
            let op2 = OPS2.iter().find(|(o, ..)| o == op2).map(|(o, ..)| *o)?;
            Some(Case::two(op, v, op2, v2, &color))
        }
        _ => None,
    }
}

/// The visual's fields (snapshot `fields`): (text, measure or aggregation).
fn bound_fields(v: &Value) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = Vec::new();
    for f in v
        .get("fields")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .take(500)
    {
        let Some(text) = f.get("text").and_then(Value::as_str) else {
            continue;
        };
        let numeric = matches!(
            f.get("kind").and_then(Value::as_str),
            Some("measure" | "aggregation")
        );
        if !out.iter().any(|(t, _)| t == text) {
            out.push((text.to_string(), numeric));
        }
    }
    out
}

/// The three rules an icon set fills: high from `hi`, medium between, low under `lo`.
fn icon_set_cases(icons: &[&str; 3], lo: &str, hi: &str) -> Vec<Case> {
    let [high, medium, low] = *icons;
    vec![
        Case::new(">=", hi, high),
        Case::two(">=", lo, "<", hi, medium),
        Case::new("<", lo, low),
    ]
}

impl CfDialog {
    /// The dialog for `target` of visual `sel` (its snapshot entry `v`, its `visual.props`
    /// `props`), filled from the format the visual has, else from its first number field and the
    /// page's colours `palette`.
    pub fn new(
        sel: &VisualSel,
        v: &Value,
        props: Option<&Value>,
        target: &'static str,
        name: String,
        palette: &[String],
    ) -> Self {
        let vtype = v.get("type").and_then(Value::as_str).unwrap_or("");
        let table = matches!(vtype, "tableEx" | "pivotTable") && per_series(target);
        let kind = Kind::of(target);
        let bound = bound_fields(v);
        let first_number = bound.iter().find(|(_, n)| *n).map(|(t, _)| t.clone());
        let first_text = bound.iter().find(|(_, n)| !*n).map(|(t, _)| t.clone());
        let series = if table {
            // a web URL goes on a text column, the others on a number
            let pick = if kind == Kind::Url {
                first_text.clone().or_else(|| first_number.clone())
            } else {
                first_number.clone()
            };
            pick.or_else(|| bound.first().map(|(t, _)| t.clone()))
                .unwrap_or_default()
        } else {
            String::new()
        };
        let accent = palette.first().cloned().unwrap_or_else(|| "#118DFF".into());
        let mut d = CfDialog {
            page: sel.page.clone(),
            visual: sel.visual.clone(),
            target,
            name,
            table,
            series: series.clone(),
            apply: "values",
            style: match kind {
                Kind::Text | Kind::Url => Style::FieldValue,
                Kind::Icons => Style::Rules,
                Kind::Color | Kind::Bars => Style::Gradient,
            },
            field: String::new(),
            agg: "Sum",
            min: Stop::color("#FFFFFF"),
            mid: None,
            max: Stop::color(&accent),
            blank: Blank::NoFormat,
            cases: Vec::new(),
            layout: "before",
            align: "top",
            bars: Bars {
                positive: accent,
                ..Bars::default()
            },
            existing: false,
            drafts: BTreeMap::new(),
        };
        match kind {
            // a title's text and a link come from a text field: none is guessed
            Kind::Text => {}
            Kind::Url => d.set_field(first_text.as_deref().unwrap_or("")),
            _ => d.set_field(first_number.as_deref().unwrap_or("")),
        }
        if kind == Kind::Icons
            && let Some((.., icons)) = ICON_SETS.get(1)
        {
            d.cases = icon_set_cases(icons, "33", "67");
        }
        d.read(props);
        d
    }

    pub fn kind(&self) -> Kind {
        Kind::of(self.target)
    }

    fn set_field(&mut self, text: &str) {
        let (f, agg) = split_agg(text);
        self.field = f;
        if let Some(a) = agg {
            self.agg = a;
        }
    }

    /// Fills the dialog from the visual's format of this target and series (nothing when none).
    fn read(&mut self, props: Option<&Value>) {
        let series = self.table.then_some(self.series.as_str());
        let Some(item) = format_of(props, self.target, series) else {
            self.existing = false;
            return;
        };
        self.existing = true;
        if let Some(a) = item.get("apply_to").and_then(Value::as_str)
            && let Some((a, ..)) = APPLY.iter().find(|(x, ..)| *x == a)
        {
            self.apply = a;
        }
        let rule = item.get("rule").unwrap_or(&Value::Null);
        let cases = |r: &Value, key: &str| -> Vec<Case> {
            r.get("cases")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default()
                .iter()
                .take(MAX_CASES)
                .filter_map(|c| read_case(c, key))
                .collect()
        };
        if let Some(g) = rule.get("gradient") {
            self.style = Style::Gradient;
            if let Some(f) = g.get("field").and_then(Value::as_str) {
                self.set_field(f);
            }
            if let Some(s) = read_stop(g.get("min")) {
                self.min = s;
            }
            if let Some(s) = read_stop(g.get("max")) {
                self.max = s;
            }
            self.mid = read_stop(g.get("mid"));
            self.blank = match g.get("empty").and_then(Value::as_str) {
                Some("zero") => Blank::Zero,
                Some(c) if c.starts_with('#') => Blank::Color(c.to_string()),
                _ => Blank::NoFormat,
            };
        } else if let Some(r) = rule.get("rules") {
            self.style = Style::Rules;
            if let Some(f) = r.get("field").and_then(Value::as_str) {
                self.set_field(f);
            }
            self.cases = cases(r, "color");
        } else if let Some(r) = rule.get("icons") {
            self.style = Style::Rules;
            if let Some(f) = r.get("field").and_then(Value::as_str) {
                self.set_field(f);
            }
            self.cases = cases(r, "icon");
            let pick = |k: &str, t: &[(&'static str, &str, &str)], d: &'static str| {
                r.get(k)
                    .and_then(Value::as_str)
                    .and_then(|x| t.iter().find(|(v, ..)| v.eq_ignore_ascii_case(x)))
                    .map_or(d, |(v, ..)| *v)
            };
            self.layout = pick("position", LAYOUTS, "before");
            self.align = pick("align", ALIGNS, "top");
        } else if let Some(b) = rule.get("data_bars") {
            let color = |k: &str, d: &str| {
                b.get(k)
                    .and_then(Value::as_str)
                    .map_or_else(|| d.to_string(), str::to_string)
            };
            let num = |k: &str| b.get(k).map(num_text).filter(|s| !s.is_empty());
            self.bars = Bars {
                bar_only: b.get("show_bar_only").and_then(Value::as_bool) == Some(true),
                min: num("min"),
                max: num("max"),
                positive: color("positive", &self.bars.positive),
                negative: color("negative", &self.bars.negative),
                axis: color("axis", &self.bars.axis),
                reverse: b.get("reverse").and_then(Value::as_bool) == Some(true),
            };
        } else if let Some(r) = rule.get("field_value").or_else(|| rule.get("web_url")) {
            self.style = Style::FieldValue;
            if let Some(f) = r.get("field").and_then(Value::as_str) {
                // a column read through Min is Desktop's "First" value of that column
                self.set_field(f);
            }
        }
    }

    /// Whether the field list is for numbers (gradient, rules, icons).
    fn numbers_only(&self) -> bool {
        match self.kind() {
            Kind::Color => self.style != Style::FieldValue,
            Kind::Icons => true,
            Kind::Text | Kind::Url | Kind::Bars => false,
        }
    }

    /// The rule's `field`: a measure as it is, a column summarized (gradient, rules, icons) or as
    /// it is (field value, web URL: the engine reads its first value).
    fn rule_field(&self, fields: &[FieldChoice]) -> String {
        let column = fields
            .iter()
            .find(|f| f.text == self.field)
            .map_or(!self.field.starts_with('['), |f| !f.measure);
        if column && self.numbers_only() {
            format!("{}({})", self.agg, self.field)
        } else {
            self.field.clone()
        }
    }

    /// The rules' cases as `visual.conditional` takes them (`value_key` `color` or `icon`).
    fn cases_json(&self, value_key: &str, l: Lang) -> Result<Vec<Value>, String> {
        if self.cases.is_empty() {
            return Err(l.t("Add a rule.", "Ajoutez une règle.").into());
        }
        let mut out = Vec::new();
        for (i, c) in self.cases.iter().enumerate() {
            let bad = |what: (&'static str, &'static str)| {
                format!("{} {}: {}", l.t("Rule", "Règle"), i + 1, l.t(what.0, what.1))
            };
            let number = ("type a number.", "tapez un nombre.");
            let mut o = Map::new();
            if c.op == "blank" {
                o.insert("op".into(), json!("="));
                o.insert("value".into(), Value::Null);
            } else {
                let a = parse_num(&c.value).ok_or_else(|| bad(number))?;
                match c.op2 {
                    None => {
                        o.insert("op".into(), json!(c.op));
                        o.insert("value".into(), json!(a));
                    }
                    Some(op2) => {
                        let b = parse_num(&c.value2).ok_or_else(|| bad(number))?;
                        let (k1, k2) = (op_key(c.op), op_key(op2));
                        let (Some(k1), Some(k2)) = (k1, k2) else {
                            return Err(bad(("pick an operator.", "choisissez un opérateur.")));
                        };
                        if k1 == k2 || k1 == "eq" {
                            return Err(bad((
                                "the two conditions need different operators.",
                                "les deux conditions demandent des opérateurs différents.",
                            )));
                        }
                        o.insert(k1.into(), json!(a));
                        o.insert(k2.into(), json!(b));
                    }
                }
                if c.percent {
                    let pct = |s: &str| parse_num(s).is_some_and(|x| (0.0..=100.0).contains(&x));
                    if !pct(&c.value) || (c.op2.is_some() && !pct(&c.value2)) {
                        return Err(bad((
                            "a percent is a number from 0 to 100.",
                            "un pourcentage est un nombre de 0 à 100.",
                        )));
                    }
                    o.insert("percent".into(), json!(true));
                }
            }
            o.insert(value_key.into(), json!(c.color));
            out.push(Value::Object(o));
        }
        Ok(out)
    }

    /// `visual.conditional`'s `rule`, or why OK cannot run yet.
    pub fn rule(&self, fields: &[FieldChoice], l: Lang) -> Result<Value, String> {
        let kind = self.kind();
        if kind != Kind::Bars && self.field.trim().is_empty() {
            return Err(l
                .t(
                    "Choose the field to base it on.",
                    "Choisissez le champ de référence.",
                )
                .into());
        }
        let field = self.rule_field(fields);
        let number =
            |s: &Option<String>, what: (&'static str, &'static str)| -> Result<Option<f64>, String> {
                match s {
                    None => Ok(None),
                    Some(t) => parse_num(t).map(Some).ok_or_else(|| {
                        format!(
                            "{}: {}",
                            l.t(what.0, what.1),
                            l.t("type a number.", "tapez un nombre.")
                        )
                    }),
                }
            };
        match kind {
            Kind::Bars => {
                let b = &self.bars;
                let min = number(&b.min, ("Minimum", "Minimum"))?;
                let max = number(&b.max, ("Maximum", "Maximum"))?;
                if let (Some(a), Some(z)) = (min, max)
                    && a >= z
                {
                    return Err(l
                        .t(
                            "The minimum must be below the maximum.",
                            "Le minimum doit être inférieur au maximum.",
                        )
                        .into());
                }
                let mut o = Map::new();
                o.insert("positive".into(), json!(b.positive));
                o.insert("negative".into(), json!(b.negative));
                o.insert("axis".into(), json!(b.axis));
                o.insert("reverse".into(), json!(b.reverse));
                o.insert("show_bar_only".into(), json!(b.bar_only));
                if let Some(n) = min {
                    o.insert("min".into(), json!(n));
                }
                if let Some(n) = max {
                    o.insert("max".into(), json!(n));
                }
                return Ok(json!({ "data_bars": o }));
            }
            Kind::Url => return Ok(json!({"web_url": {"field": field}})),
            Kind::Icons => {
                let cases = self.cases_json("icon", l)?;
                return Ok(json!({"icons": {"field": field, "cases": cases,
                                           "position": self.layout, "align": self.align}}));
            }
            Kind::Color | Kind::Text => {}
        }
        Ok(match self.style {
            Style::Gradient => {
                let mut g = Map::new();
                g.insert("field".into(), json!(field));
                let stops: [(&str, Option<&Stop>, (&str, &str)); 3] = [
                    ("min", Some(&self.min), ("Minimum", "Minimum")),
                    ("mid", self.mid.as_ref(), ("Center", "Centre")),
                    ("max", Some(&self.max), ("Maximum", "Maximum")),
                ];
                // (value, is a percent): numbers and percents each go up
                let mut at: Vec<(f64, bool)> = Vec::new();
                for (key, stop, what) in stops {
                    let Some(s) = stop else {
                        continue;
                    };
                    let v = match number(&s.number, what)? {
                        Some(n) if s.percent => {
                            if !(0.0..=100.0).contains(&n) {
                                return Err(format!(
                                    "{}: {}",
                                    l.t(what.0, what.1),
                                    l.t(
                                        "a percent is a number from 0 to 100.",
                                        "un pourcentage est un nombre de 0 à 100."
                                    )
                                ));
                            }
                            at.push((n, true));
                            json!({"color": s.color, "value": n, "percent": true})
                        }
                        Some(n) => {
                            at.push((n, false));
                            json!({"color": s.color, "value": n})
                        }
                        None => json!(s.color),
                    };
                    g.insert(key.into(), v);
                }
                let down = |p: bool| {
                    let xs: Vec<f64> = at.iter().filter(|a| a.1 == p).map(|a| a.0).collect();
                    xs.windows(2).any(|w| w.first() >= w.get(1))
                };
                if down(false) || down(true) {
                    return Err(l
                        .t(
                            "The numbers must go up from the minimum to the maximum.",
                            "Les nombres doivent croître du minimum au maximum.",
                        )
                        .into());
                }
                match &self.blank {
                    Blank::NoFormat => {}
                    Blank::Zero => {
                        g.insert("empty".into(), json!("zero"));
                    }
                    Blank::Color(c) => {
                        g.insert("empty".into(), json!(c));
                    }
                }
                json!({ "gradient": g })
            }
            Style::Rules => {
                let cases = self.cases_json("color", l)?;
                json!({"rules": {"field": field, "cases": cases}})
            }
            Style::FieldValue => json!({"field_value": {"field": field}}),
        })
    }

    /// `visual.conditional`'s parameters (without page and visual) for `rule`.
    pub fn params(&self, rule: Value) -> Value {
        let mut o = Map::new();
        o.insert("target".into(), json!(self.target));
        if self.table && !self.series.is_empty() {
            o.insert("series".into(), json!(self.series));
        }
        if self.table
            && self.apply != "values"
            && !rule.is_null()
            && matches!(self.kind(), Kind::Color | Kind::Icons)
        {
            o.insert("apply_to".into(), json!(self.apply));
        }
        o.insert("rule".into(), rule);
        Value::Object(o)
    }

    /// Reverse colour order: the rules keep their conditions, their colours (or icons) go in the
    /// opposite order; a gradient swaps its lowest and highest colours.
    pub fn reverse(&mut self) {
        if self.style == Style::Gradient && self.kind() == Kind::Color {
            std::mem::swap(&mut self.min.color, &mut self.max.color);
            return;
        }
        let colors: Vec<String> = self.cases.iter().rev().map(|c| c.color.clone()).collect();
        for (c, col) in self.cases.iter_mut().zip(colors) {
            c.color = col;
        }
    }
}

// ----- the preview -----

/// One sample cell of the preview.
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub value: f64,
    /// The colour the format gives (background, font or data colour).
    pub color: Option<[u8; 3]>,
    /// Data bars: the bar's start and end (fractions of the cell) and colour.
    pub bar: Option<(f32, f32, [u8; 3])>,
    /// Data bars: the axis line's place in the cell.
    pub axis: Option<f32>,
    pub icon: Option<String>,
    /// The value's text is drawn (not with Show bar only or Icon only).
    pub text: bool,
}

fn hex(s: &str) -> Option<[u8; 3]> {
    crate::rich::parse_hex(s)
}

/// RGB interpolation, as the viewer's.
fn mix(a: [u8; 3], b: [u8; 3], t: f64) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    let ch = |x: u8, y: u8| {
        (f64::from(x) + (f64::from(y) - f64::from(x)) * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    [ch(a[0], b[0]), ch(a[1], b[1]), ch(a[2], b[2])]
}

/// Whether `x` meets `op b`.
fn holds(op: &str, x: f64, b: f64) -> bool {
    match op {
        "=" => x == b,
        ">" => x > b,
        ">=" => x >= b,
        "<" => x < b,
        "<=" => x <= b,
        _ => false,
    }
}

/// The first rule `x` meets: its colour or icon; Percent numbers are read over `lo..hi`, the
/// samples' range.
fn first_case(cases: &[Case], x: f64, lo: f64, hi: f64) -> Option<&str> {
    cases
        .iter()
        .find(|c| {
            let bound =
                |s: &str| parse_num(s).map(|n| if c.percent { lo + (hi - lo) * n / 100.0 } else { n });
            let Some(a) = bound(&c.value) else {
                return false;
            };
            holds(c.op, x, a)
                && c.op2
                    .is_none_or(|op2| bound(&c.value2).is_some_and(|b| holds(op2, x, b)))
        })
        .map(|c| c.color.as_str())
}

impl CfDialog {
    /// The sample values the preview shows: evenly spread over the numbers the format names
    /// (the gradient's or data bars' bounds, the rules' thresholds), else over 0–100 (data bars
    /// over −25–100, to show a negative bar).
    fn sample_range(&self) -> (f64, f64) {
        let n = |s: &Option<String>| s.as_deref().and_then(parse_num);
        let (lo, hi) = match self.kind() {
            Kind::Bars => (
                n(&self.bars.min).unwrap_or(-25.0),
                n(&self.bars.max).unwrap_or(100.0),
            ),
            // a Percent stop's number is not a value: the samples keep 0–100
            _ if self.style == Style::Gradient => (
                n(&self.min.number).filter(|_| !self.min.percent).unwrap_or(0.0),
                n(&self.max.number).filter(|_| !self.max.percent).unwrap_or(100.0),
            ),
            _ => {
                // a Percent rule's numbers are not values: they spread over the samples' range
                let xs: Vec<f64> = self
                    .cases
                    .iter()
                    .filter(|c| !c.percent)
                    .flat_map(|c| [parse_num(&c.value), c.op2.and_then(|_| parse_num(&c.value2))])
                    .flatten()
                    .collect();
                match (
                    xs.iter().copied().reduce(f64::min),
                    xs.iter().copied().reduce(f64::max),
                ) {
                    (Some(a), Some(b)) if b > a => {
                        let pad = (b - a) * 0.5;
                        (a - pad, b + pad)
                    }
                    (Some(a), _) => (a - 10.0, a + 10.0),
                    _ => (0.0, 100.0),
                }
            }
        };
        if hi > lo { (lo, hi) } else { (lo, lo + 100.0) }
    }

    /// The preview's sample cells, formatted as the viewer formats them; empty for a field value,
    /// a title's text and a web URL (their values come from the field).
    pub fn preview(&self) -> Vec<Sample> {
        let kind = self.kind();
        if matches!(kind, Kind::Text | Kind::Url) || (kind == Kind::Color && self.style == Style::FieldValue)
        {
            return Vec::new();
        }
        let (lo, hi) = self.sample_range();
        let step = (hi - lo) / (SAMPLES - 1) as f64;
        let round = |x: f64| (x * 100.0).round() / 100.0;
        let mut out = Vec::new();
        for i in 0..SAMPLES {
            let x = round(lo + step * i as f64);
            let mut s = Sample {
                value: x,
                color: None,
                bar: None,
                axis: None,
                icon: None,
                text: true,
            };
            match kind {
                Kind::Bars => {
                    let b = &self.bars;
                    let lo = lo.min(0.0);
                    let hi = hi.max(0.0);
                    let f = |v: f64| {
                        let t = ((v - lo) / (hi - lo)).clamp(0.0, 1.0) as f32;
                        if b.reverse { 1.0 - t } else { t }
                    };
                    let axis = f(0.0);
                    let color = if x >= 0.0 { &b.positive } else { &b.negative };
                    if let Some(c) = hex(color) {
                        s.bar = Some((axis.min(f(x)), axis.max(f(x)), c));
                    }
                    s.axis = (axis > 0.0 && axis < 1.0).then_some(axis);
                    s.text = !b.bar_only;
                }
                Kind::Icons => {
                    s.icon = first_case(&self.cases, x, lo, hi).map(str::to_string);
                    s.text = self.layout != "only";
                }
                _ if self.style == Style::Rules => {
                    s.color = first_case(&self.cases, x, lo, hi).and_then(hex);
                }
                _ => {
                    let mut stops: Vec<([u8; 3], f64)> = Vec::new();
                    let mid_at = (lo + hi) / 2.0;
                    for (stop, def) in [
                        (Some(&self.min), lo),
                        (self.mid.as_ref(), mid_at),
                        (Some(&self.max), hi),
                    ] {
                        if let Some(st) = stop
                            && let Some(c) = hex(&st.color)
                        {
                            let at = match st.number.as_deref().and_then(parse_num) {
                                Some(p) if st.percent => lo + (hi - lo) * p / 100.0,
                                Some(n) => n,
                                None => def,
                            };
                            stops.push((c, at));
                        }
                    }
                    s.color = gradient(&stops, x);
                }
            }
            out.push(s);
        }
        out
    }
}

/// The colour at `x` of stops (colour, value) in increasing order.
fn gradient(stops: &[([u8; 3], f64)], x: f64) -> Option<[u8; 3]> {
    let (first, last) = (stops.first()?, stops.last()?);
    if x <= first.1 {
        return Some(first.0);
    }
    if x >= last.1 {
        return Some(last.0);
    }
    stops
        .windows(2)
        .find_map(|w| {
            let (a, b) = (w.first()?, w.get(1)?);
            (x >= a.1 && x <= b.1).then(|| {
                let t = if b.1 > a.1 { (x - a.1) / (b.1 - a.1) } else { 0.0 };
                mix(a.0, b.0, t)
            })
        })
        .or(Some(last.0))
}

/// A sample value as the preview writes it: a whole number without decimals, else two.
fn sample_text(x: f64) -> String {
    if x.fract() == 0.0 && x.abs() < 1e15 {
        format!("{x:.0}")
    } else {
        format!("{x:.2}")
    }
}

fn color32(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

/// An icon of the icon sets as the tables draw it, `size` points square.
fn icon_image(ui: &mut Ui, name: &str, size: f32) -> egui::Response {
    let px = (size * ui.ctx().pixels_per_point()).round().clamp(8.0, 64.0) as u32;
    let id = egui::Id::unique(("bicraft-cf-icon", name, px));
    let tex = ui.ctx().data(|d| d.get_temp::<egui::TextureHandle>(id));
    let tex = tex.or_else(|| {
        let rgba = bicraft_render::icon_rgba(name, px)?;
        let img = egui::ColorImage::from_rgba_unmultiplied([px as usize, px as usize], &rgba);
        let t = ui
            .ctx()
            .load_texture(format!("cf-icon-{name}-{px}"), img, egui::TextureOptions::LINEAR);
        ui.ctx().data_mut(|d| d.insert_temp(id, t.clone()));
        Some(t)
    });
    let (rect, r) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    if let Some(t) = tex {
        ui.painter().image(
            t.id(),
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    r
}

/// The preview: the sample cells, as a column of the table (or the chart's bars) would show them.
fn preview_ui(ui: &mut Ui, d: &CfDialog, l: Lang) {
    let samples = d.preview();
    let t = Tokens::get(ui.ctx());
    let name = l.t("Preview", "Aperçu");
    ui.label(RichText::new(name).small().color(t.text_dim));
    if samples.is_empty() {
        ui.label(
            RichText::new(l.t(
                "The values come from the field when the visual is drawn.",
                "Les valeurs viennent du champ quand le visuel est dessiné.",
            ))
            .small()
            .color(t.text_faint),
        );
        return;
    }
    let font = theme::font(12.0);
    let r = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for s in &samples {
            let (rect, _) = ui.allocate_exact_size(vec2(78.0, 24.0), Sense::hover());
            let p = ui.painter();
            let bg = match (d.kind(), d.target) {
                (Kind::Color, "background" | "visual_background" | "title_background") => {
                    s.color.map(color32)
                }
                _ => None,
            };
            p.rect_filled(rect, CornerRadius::same(2), bg.unwrap_or(t.field_bg));
            p.rect_stroke(
                rect,
                CornerRadius::same(2),
                Stroke::new(1.0, t.border),
                egui::StrokeKind::Inside,
            );
            if let Some((a, b, c)) = s.bar {
                let x0 = rect.left() + rect.width() * a;
                let x1 = rect.left() + rect.width() * b;
                p.rect_filled(
                    egui::Rect::from_min_max(
                        egui::pos2(x0, rect.top() + 2.0),
                        egui::pos2(x1, rect.bottom() - 2.0),
                    ),
                    CornerRadius::ZERO,
                    color32(c),
                );
            }
            if let (Some(at), Some(c)) = (s.axis, hex(&d.bars.axis)) {
                let x = rect.left() + rect.width() * at;
                p.line_segment(
                    [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                    Stroke::new(1.0, color32(c)),
                );
            }
            // the value's colour: the font colour, else readable on the cell
            let fg = match (d.kind(), d.target) {
                (Kind::Color, "background" | "visual_background" | "title_background") => {
                    s.color.map_or(t.text, |c| {
                        let lum = 0.299 * f32::from(c[0]) + 0.587 * f32::from(c[1]) + 0.114 * f32::from(c[2]);
                        if lum > 140.0 {
                            Color32::BLACK
                        } else {
                            Color32::WHITE
                        }
                    })
                }
                (Kind::Color, _) => s.color.map_or(t.text, color32),
                _ => t.text,
            };
            let text = sample_text(s.value).replace('.', l.t(".", ","));
            let icon_w = if s.icon.is_some() { 18.0 } else { 0.0 };
            if let Some(name) = &s.icon {
                let left = match d.layout {
                    "after" => rect.right() - 20.0,
                    "only" => rect.center().x - 8.0,
                    _ => rect.left() + 4.0,
                };
                let y = match d.align {
                    "middle" => rect.center().y - 8.0,
                    "bottom" => rect.bottom() - 18.0,
                    _ => rect.top() + 2.0,
                };
                let mut child = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(egui::Rect::from_min_size(egui::pos2(left, y), vec2(16.0, 16.0))),
                );
                icon_image(&mut child, name, 16.0);
            }
            if s.text {
                let x = match (d.layout, s.icon.is_some()) {
                    ("before", true) => rect.left() + 4.0 + icon_w,
                    _ => rect.left() + 6.0,
                };
                ui.painter().text(
                    egui::pos2(x, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    text,
                    font.clone(),
                    fg,
                );
            }
        }
    });
    let summary = samples
        .iter()
        .map(|s| {
            let mut x = sample_text(s.value);
            if let Some(c) = s.color {
                x.push_str(&format!(" #{:02X}{:02X}{:02X}", c[0], c[1], c[2]));
            }
            if let Some((a, b, _)) = s.bar {
                x.push_str(&format!(" {:.0}–{:.0} %", a * 100.0, b * 100.0));
            }
            if let Some(i) = &s.icon {
                x.push(' ');
                x.push_str(i);
            }
            x
        })
        .collect::<Vec<_>>()
        .join(", ");
    let a = format!("{name}: {summary}");
    r.response
        .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &a));
}

// ----- the window -----

/// Opens the dialog for `target` of the selected visual (its snapshot entry `v`).
pub fn open(app: &mut App, sel: &VisualSel, v: &Value, target: &'static str, name: String) {
    let palette: Vec<String> = app
        .loaded
        .as_ref()
        .map(|x| bicraft_render::data_colors(&x.snap))
        .unwrap_or_default();
    let props = app
        .panes
        .format
        .own
        .as_ref()
        .filter(|o| o.page == sel.page && o.visual == sel.visual)
        .map(|o| o.props.clone());
    app.panes.format.cf = Some(CfDialog::new(sel, v, props.as_ref(), target, name, &palette));
}

/// What the dialog's buttons asked for this frame.
enum Done {
    Keep,
    Close,
    Send(Value),
}

/// The window's title: the kind of format and the property's name, as Desktop titles its dialogs.
fn window_title(kind: Kind, l: Lang) -> &'static str {
    match kind {
        Kind::Icons => l.t("Icons", "Icônes"),
        Kind::Bars => l.t("Data bars", "Barres de données"),
        Kind::Url => l.t("Web URL", "URL web"),
        Kind::Color | Kind::Text => l.t("Conditional formatting", "Mise en forme conditionnelle"),
    }
}

/// Draws the dialog when it is open for the selected visual `sel` (its snapshot entry `v`);
/// `swatches` are the colour popups' colours.
pub fn show(app: &mut App, ctx: &egui::Context, sel: &VisualSel, v: &Value, swatches: &[String]) {
    let Some(mut d) = app.panes.format.cf.take() else {
        return;
    };
    // another visual selected: the dialog was for the one before
    if d.page != sel.page || d.visual != sel.visual {
        return;
    }
    let l = app.lang;
    let fields = app
        .loaded
        .as_ref()
        .map(|x| field_choices(&x.snap))
        .unwrap_or_default();
    let props = app
        .panes
        .format
        .own
        .as_ref()
        .filter(|o| o.page == sel.page && o.visual == sel.visual)
        .map(|o| o.props.clone());
    let state = crate::ribbon::state(app, COMMAND);
    let mut open = true;
    let mut done = Done::Keep;
    egui::Window::new(window_title(d.kind(), l))
        .id(egui::Id::unique("bicraft-conditional-format"))
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_width(660.0);
            done = body(ui, &mut d, v, props.as_ref(), &fields, swatches, &state, l);
        });
    match done {
        Done::Keep if open => app.panes.format.cf = Some(d),
        Done::Keep | Done::Close => {}
        Done::Send(params) => crate::format_pane::send(app, sel, None, Value::Null, COMMAND, params),
    }
}

/// A combo box labelled `name` for the accessibility tree.
fn combo<R>(
    ui: &mut Ui,
    id: &str,
    shown: &str,
    width: f32,
    name: &str,
    add: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    let r = egui::ComboBox::from_id_salt(("bicraft-cf", id))
        .selected_text(clip(shown, 40))
        .width(width)
        .height(300.0)
        .show_ui(ui, add);
    r.response
        .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, name));
    r.inner
}

/// A combo box over (value, EN, FR) choices: the value picked this frame.
fn choice(
    ui: &mut Ui,
    id: &str,
    now: &str,
    list: &[(&'static str, &'static str, &'static str)],
    width: f32,
    name: &str,
    l: Lang,
) -> Option<&'static str> {
    let shown = list
        .iter()
        .find(|(v, ..)| *v == now)
        .map_or(now, |(_, en, fr)| l.t(en, fr));
    combo(ui, id, shown, width, name, |ui| {
        let mut picked = None;
        for (v, en, fr) in list {
            if ui.selectable_label(*v == now, l.t(en, fr)).clicked() {
                picked = Some(*v);
            }
        }
        picked
    })
    .flatten()
}

/// A label above a control, as Desktop's dialog stacks them.
fn caption(ui: &mut Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(RichText::new(text).small().color(t.text_dim));
}

/// A number box labelled `name`.
fn number_box(ui: &mut Ui, text: &mut String, width: f32, name: &str) {
    let r = ui.add(
        egui::TextEdit::singleline(text)
            .char_limit(40)
            .desired_width(width),
    );
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, name));
}

/// A checkbox labelled `label`: whether it was clicked.
fn check(ui: &mut Ui, on: &mut bool, label: &str) -> bool {
    ui.checkbox(on, label).changed()
}

#[allow(clippy::too_many_arguments)]
fn body(
    ui: &mut Ui,
    d: &mut CfDialog,
    v: &Value,
    props: Option<&Value>,
    fields: &[FieldChoice],
    swatches: &[String],
    state: &Result<(), String>,
    l: Lang,
) -> Done {
    let t = Tokens::get(ui.ctx());
    let kind = d.kind();
    ui.label(RichText::new(clip(&d.name, 80)).font(theme::bold(14.0)));
    ui.add_space(6.0);
    // Format style, the series of a table and what it applies to
    ui.horizontal(|ui| {
        if matches!(kind, Kind::Color | Kind::Text | Kind::Icons) {
            ui.vertical(|ui| {
                let name = l.t("Format style", "Style de mise en forme");
                caption(ui, name);
                // a title's text: Field value only, as Desktop's "Title text" dialog; icons by rules
                let styles: &[Style] = match kind {
                    Kind::Text => &[Style::FieldValue],
                    Kind::Icons => &[Style::Rules],
                    _ => &Style::ALL,
                };
                if let Some(Some(s)) = combo(ui, "style", d.style.label(l), 120.0, name, |ui| {
                    let mut picked = None;
                    for &s in styles {
                        if ui.selectable_label(d.style == s, s.label(l)).clicked() {
                            picked = Some(s);
                        }
                    }
                    picked
                }) {
                    d.style = s;
                }
            });
            ui.add_space(8.0);
        }
        if d.table {
            ui.vertical(|ui| {
                let name = l.t("Apply to series", "Appliquer à la série");
                caption(ui, name);
                let bound = bound_fields(v);
                if let Some(Some(s)) = combo(ui, "series", &d.series, 180.0, name, |ui| {
                    let mut picked = None;
                    for (text, _) in &bound {
                        if ui.selectable_label(d.series == *text, clip(text, 40)).clicked() {
                            picked = Some(text.clone());
                        }
                    }
                    picked
                }) && s != d.series
                {
                    d.series = s;
                    // that series' own format, if it has one
                    d.read(props);
                }
            });
            if matches!(kind, Kind::Color | Kind::Icons) {
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    let name = l.t("Apply to", "Appliquer à");
                    caption(ui, name);
                    if let Some(a) = choice(ui, "apply", d.apply, APPLY, 150.0, name, l) {
                        d.apply = a;
                    }
                });
            }
        }
    });
    ui.add_space(6.0);
    // the field the format is based on (data bars: the series itself)
    if kind != Kind::Bars {
        field_ui(ui, d, fields, l);
        ui.add_space(8.0);
    }
    let mut drafts = std::mem::take(&mut d.drafts);
    let visual = d.visual.clone();
    let mut swatch = |ui: &mut Ui, key: &str, cur: &str, name: &str| {
        crate::format_pane::pick_color(
            ui,
            l,
            swatches,
            &mut drafts,
            &visual,
            &format!("cf.{key}"),
            name,
            &json!(cur),
            name,
        )
    };
    match kind {
        Kind::Bars => bars_ui(ui, d, &mut swatch, l),
        Kind::Icons => icons_ui(ui, d, l),
        Kind::Url => {
            ui.label(
                RichText::new(l.t(
                    "The field's text is the link (http or https addresses).",
                    "Le texte du champ est le lien (adresses http ou https).",
                ))
                .small()
                .color(t.text_dim),
            );
        }
        Kind::Color | Kind::Text => match d.style {
            Style::Gradient => gradient_ui(ui, d, &mut swatch, l),
            Style::Rules => rules_ui(ui, d, swatches, &mut swatch, l),
            Style::FieldValue => {
                let hint = if kind == Kind::Text {
                    l.t(
                        "The field's text is shown (a text measure, evaluated for the visual).",
                        "Le texte du champ est affiché (une mesure texte, évaluée pour le visuel).",
                    )
                } else {
                    l.t(
                        "The field's values are the colours (#hex codes or colour names).",
                        "Les valeurs du champ sont les couleurs (codes #hex ou noms de couleurs).",
                    )
                };
                ui.label(RichText::new(hint).small().color(t.text_dim));
            }
        },
    }
    d.drafts = drafts;
    if !matches!(kind, Kind::Text | Kind::Url) {
        ui.add_space(8.0);
        preview_ui(ui, d, l);
    }
    ui.add_space(10.0);
    let rule = d.rule(fields, l);
    let can = state.is_ok();
    let mut done = Done::Keep;
    if let Err(why) = &rule {
        ui.label(RichText::new(why).small().color(t.text_dim));
    }
    if let Err(why) = state {
        ui.label(
            RichText::new(format!(
                "{} {}",
                l.t("Unavailable:", "Indisponible :"),
                clip(why, 120)
            ))
            .small()
            .color(t.text_dim),
        );
    }
    ui.horizontal(|ui| {
        if d.existing {
            let r = ui.add_enabled(
                can,
                egui::Button::new(l.t(
                    "Remove conditional formatting",
                    "Supprimer la mise en forme conditionnelle",
                )),
            );
            if r.clicked() {
                done = Done::Send(d.params(Value::Null));
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(l.t("Cancel", "Annuler")).clicked() {
                done = Done::Close;
            }
            let ok = rule.as_ref().ok().filter(|_| can);
            let why = rule.as_ref().err().map_or("", String::as_str);
            if widgets::primary(ui, l.t("OK", "OK"), ok.is_some(), why).clicked()
                && let Some(r) = ok
            {
                done = Done::Send(d.params(r.clone()));
            }
        });
    });
    done
}

/// "What field should we base this on?" and a column's summarization.
fn field_ui(ui: &mut Ui, d: &mut CfDialog, fields: &[FieldChoice], l: Lang) {
    let numbers_only = d.numbers_only();
    let text_only = matches!(d.kind(), Kind::Text | Kind::Url);
    let shown = fields
        .iter()
        .find(|f| f.text == d.field)
        .map_or_else(|| d.field.clone(), |f| f.label.clone());
    let is_column = fields
        .iter()
        .find(|f| f.text == d.field)
        .is_some_and(|f| !f.measure);
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            let name = l.t(
                "What field should we base this on?",
                "Sur quel champ voulez-vous vous baser ?",
            );
            caption(ui, name);
            if let Some(Some(f)) = combo(ui, "field", &shown, 260.0, name, |ui| {
                let mut picked = None;
                for f in fields {
                    // a text: measures (their type is not known here) and text columns
                    let fits = if text_only {
                        f.measure || !f.numeric
                    } else {
                        !numbers_only || f.numeric
                    };
                    if !fits {
                        continue;
                    }
                    if ui
                        .selectable_label(d.field == f.text, clip(&f.label, 50))
                        .clicked()
                    {
                        picked = Some(f.text.clone());
                    }
                }
                picked
            }) {
                d.field = f;
            }
        });
        if is_column && numbers_only {
            ui.add_space(12.0);
            ui.vertical(|ui| {
                let name = l.t("Summarization", "Synthèse");
                caption(ui, name);
                if let Some(a) = choice(ui, "agg", d.agg, AGGS, 110.0, name, l) {
                    d.agg = a;
                }
            });
        }
    });
}

/// An (EN, FR) pair.
type Words = (&'static str, &'static str);

type Swatch<'a> = dyn FnMut(&mut Ui, &str, &str, &str) -> Option<String> + 'a;

/// Gradient: empty values, then the minimum, optional centre and maximum.
fn gradient_ui(ui: &mut Ui, d: &mut CfDialog, swatch: &mut Swatch<'_>, l: Lang) {
    ui.horizontal(|ui| {
        let name = l.t(
            "How should we format empty values?",
            "Comment mettre en forme les valeurs vides ?",
        );
        ui.label(RichText::new(name).small());
        let now = match &d.blank {
            Blank::NoFormat => "none",
            Blank::Zero => "zero",
            Blank::Color(_) => "color",
        };
        const BLANKS: &[(&str, &str, &str)] = &[
            ("none", "Don't format", "Ne pas mettre en forme"),
            ("zero", "As zero", "Comme zéro"),
            ("color", "Specific colour", "Couleur spécifique"),
        ];
        if let Some(b) = choice(ui, "blank", now, BLANKS, 150.0, name, l) {
            d.blank = match b {
                "zero" => Blank::Zero,
                "color" => Blank::Color("#E6E6E6".into()),
                _ => Blank::NoFormat,
            };
        }
        if let Blank::Color(c) = &d.blank {
            let n = l.t("Empty values colour", "Couleur des valeurs vides");
            if let Some(c2) = swatch(ui, "blank", &c.clone(), n) {
                d.blank = Blank::Color(c2);
            }
        }
    });
    ui.add_space(4.0);
    let mut on = d.mid.is_some();
    if check(
        ui,
        &mut on,
        l.t("Add a middle colour", "Ajouter une couleur médiane"),
    ) {
        d.mid = on.then(|| Stop::color("#FFFF00"));
    }
    ui.add_space(4.0);
    // (key, caption, the value it stands for without a number: also its colour's name)
    let rows: [(&str, Words, Words); 3] = [
        (
            "min",
            ("Minimum", "Minimum"),
            ("Lowest value", "Valeur la plus basse"),
        ),
        ("mid", ("Center", "Centre"), ("Middle value", "Valeur médiane")),
        (
            "max",
            ("Maximum", "Maximum"),
            ("Highest value", "Valeur la plus haute"),
        ),
    ];
    egui::Grid::new("bicraft-cf-gradient")
        .num_columns(4)
        .spacing(vec2(10.0, 6.0))
        .show(ui, |ui| {
            for (key, cap, auto) in rows {
                let stop = match key {
                    "min" => Some(&mut d.min),
                    "mid" => d.mid.as_mut(),
                    _ => Some(&mut d.max),
                };
                let Some(stop) = stop else {
                    continue;
                };
                ui.label(l.t(cap.0, cap.1));
                let list: [(&'static str, &'static str, &'static str); 3] = [
                    ("auto", auto.0, auto.1),
                    ("number", "Number", "Nombre"),
                    ("percent", "Percent", "Pourcentage"),
                ];
                let now = match (&stop.number, stop.percent) {
                    (None, _) => "auto",
                    (Some(_), false) => "number",
                    (Some(_), true) => "percent",
                };
                let name = format!("{} {}", l.t(cap.0, cap.1), l.t("type", "type"));
                if let Some(k) = choice(ui, &format!("{key}-kind"), now, &list, 150.0, &name, l) {
                    stop.number = (k != "auto").then(|| stop.number.clone().unwrap_or_default());
                    stop.percent = k == "percent";
                }
                match &mut stop.number {
                    Some(n) => {
                        let name = format!("{} {}", l.t(cap.0, cap.1), l.t("value", "valeur"));
                        number_box(ui, n, 70.0, &name);
                    }
                    None => {
                        ui.label("");
                    }
                }
                let name = format!("{} {}", l.t(auto.0, auto.1), l.t("colour", "couleur"));
                if let Some(c) = swatch(ui, key, &stop.color.clone(), &name) {
                    stop.color = c;
                }
                ui.end_row();
            }
        });
}

/// The rules' rows ("If value … and … then …"), New rule, Reverse colour order: `icons` gives
/// each row an icon list instead of a colour.
fn rows_ui(ui: &mut Ui, d: &mut CfDialog, icons: bool, swatch: &mut Swatch<'_>, l: Lang) {
    let mut remove = None;
    let rule = l.t("Rule", "Règle");
    // one grid, so that the rows' operators, numbers and buttons line up
    egui::Grid::new(("bicraft-cf-rules", icons))
        .num_columns(if icons { 9 } else { 10 })
        .spacing(vec2(4.0, 4.0))
        .show(ui, |ui| {
            for (i, c) in d.cases.iter_mut().enumerate() {
                let n = i + 1;
                if icons {
                    let name = format!("{rule} {n} {}", l.t("icon", "icône"));
                    icon_pick(ui, &format!("icon{i}"), &mut c.color, &name);
                }
                ui.label(l.t("If value", "Si la valeur"));
                let name = format!("{rule} {n} {}", l.t("operator", "opérateur"));
                if let Some(op) = choice(ui, &format!("op{i}"), c.op, OPS, 120.0, &name, l) {
                    c.op = op;
                    if op == "blank" {
                        c.op2 = None;
                    }
                }
                if c.op == "blank" {
                    for _ in 0..5 {
                        ui.label("");
                    }
                } else {
                    let name = format!("{rule} {n} {}", l.t("value", "valeur"));
                    number_box(ui, &mut c.value, 50.0, &name);
                    ui.label(l.t("and", "et"));
                    let mut list: Vec<(&'static str, &'static str, &'static str)> = vec![("", "—", "—")];
                    list.extend_from_slice(OPS2);
                    let name = format!("{rule} {n} {}", l.t("second operator", "second opérateur"));
                    if let Some(op2) = choice(
                        ui,
                        &format!("opb{i}"),
                        c.op2.unwrap_or(""),
                        &list,
                        110.0,
                        &name,
                        l,
                    ) {
                        c.op2 = (!op2.is_empty()).then_some(op2);
                    }
                    if c.op2.is_some() {
                        let name = format!("{rule} {n} {}", l.t("second value", "seconde valeur"));
                        number_box(ui, &mut c.value2, 50.0, &name);
                    } else {
                        ui.label("");
                    }
                    // Number or Percent (of the range from the lowest to the highest value)
                    let name = format!("{rule} {n} {}", l.t("type", "type"));
                    let cur = if c.percent { "percent" } else { "number" };
                    if let Some(k) = choice(ui, &format!("pct{i}"), cur, BOUND_TYPES, 90.0, &name, l) {
                        c.percent = k == "percent";
                    }
                }
                if !icons {
                    ui.horizontal(|ui| {
                        ui.label(l.t("then", "alors"));
                        let name = format!("{rule} {n} {}", l.t("colour", "couleur"));
                        if let Some(col) = swatch(ui, &format!("case{i}"), &c.color.clone(), &name) {
                            c.color = col;
                        }
                    });
                }
                let name = format!("{} {n}", l.t("Delete rule", "Supprimer la règle"));
                let r = ui.small_button("✕");
                r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
                if r.on_hover_text(&name).clicked() {
                    remove = Some(i);
                }
                ui.end_row();
            }
        });
    if let Some(i) = remove
        && i < d.cases.len()
    {
        d.cases.remove(i);
    }
}

/// The Rules style of a colour.
fn rules_ui(ui: &mut Ui, d: &mut CfDialog, swatches: &[String], swatch: &mut Swatch<'_>, l: Lang) {
    rows_ui(ui, d, false, swatch, l);
    ui.horizontal(|ui| {
        let can_add = d.cases.len() < MAX_CASES;
        if ui
            .add_enabled(can_add, egui::Button::new(l.t("+ New rule", "+ Nouvelle règle")))
            .clicked()
        {
            let color = swatches
                .get(d.cases.len() % swatches.len().max(1))
                .cloned()
                .unwrap_or_else(|| "#118DFF".into());
            d.cases.push(Case::new(">", "", &color));
        }
        reverse_button(ui, d, l);
    });
}

fn reverse_button(ui: &mut Ui, d: &mut CfDialog, l: Lang) {
    let label = if d.kind() == Kind::Icons {
        l.t("⇅ Reverse icon order", "⇅ Inverser l'ordre des icônes")
    } else {
        l.t("⇅ Reverse colour order", "⇅ Inverser l'ordre des couleurs")
    };
    if ui
        .add_enabled(d.cases.len() > 1, egui::Button::new(label))
        .clicked()
    {
        d.reverse();
    }
}

/// The icon list of a rule: every icon of the icon sets, drawn.
fn icon_pick(ui: &mut Ui, id: &str, cur: &mut String, name: &str) {
    let (rect, r) = ui.allocate_exact_size(vec2(36.0, 22.0), Sense::click());
    let t = Tokens::get(ui.ctx());
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(3),
        Stroke::new(1.0, t.border),
        egui::StrokeKind::Inside,
    );
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
        rect.min + vec2(3.0, 3.0),
        vec2(16.0, 16.0),
    )));
    icon_image(&mut child, cur, 16.0);
    widgets::chevron(
        ui.painter(),
        egui::pos2(rect.right() - 8.0, rect.center().y),
        3.0,
        t.text_dim,
    );
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, name));
    let r = r.on_hover_text(cur.as_str());
    egui::Popup::menu(&r)
        .id(egui::Id::unique(("bicraft-cf-iconpick", id)))
        .show(|ui| {
            egui::Grid::new(("bicraft-cf-icons", id))
                .spacing(vec2(4.0, 4.0))
                .show(ui, |ui| {
                    for (.., icons) in ICON_SETS {
                        for icon in icons {
                            let resp = ui
                                .scope(|ui| icon_image(ui, icon, 18.0))
                                .inner
                                .interact(Sense::click());
                            let a = format!("{name} {icon}");
                            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &a));
                            if resp.on_hover_text(*icon).clicked() {
                                *cur = (*icon).to_string();
                                ui.close();
                            }
                        }
                        ui.end_row();
                    }
                });
        });
}

/// Icons: the layout, alignment and style, then the rules with an icon each.
fn icons_ui(ui: &mut Ui, d: &mut CfDialog, l: Lang) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        ui.vertical(|ui| {
            let name = l.t("Icon layout", "Disposition de l'icône");
            caption(ui, name);
            if let Some(x) = choice(ui, "layout", d.layout, LAYOUTS, 150.0, name, l) {
                d.layout = x;
            }
        });
        ui.vertical(|ui| {
            let name = l.t("Icon alignment", "Alignement de l'icône");
            caption(ui, name);
            if let Some(x) = choice(ui, "align", d.align, ALIGNS, 100.0, name, l) {
                d.align = x;
            }
        });
        ui.vertical(|ui| {
            let name = l.t("Style", "Style");
            caption(ui, name);
            // the set whose icons the rules show, if any
            let now = ICON_SETS
                .iter()
                .find(|(.., icons)| {
                    d.cases.len() == 3 && d.cases.iter().zip(icons).all(|(c, i)| c.color == *i)
                })
                .map_or("", |(k, ..)| *k);
            let list: Vec<(&'static str, &'static str, &'static str)> =
                ICON_SETS.iter().map(|(k, en, fr, _)| (*k, *en, *fr)).collect();
            if let Some(k) = choice(ui, "iconset", now, &list, 140.0, name, l)
                && let Some((.., icons)) = ICON_SETS.iter().find(|(x, ..)| *x == k)
            {
                // the rules keep their bounds when there are three, else Desktop's thirds
                if d.cases.len() == 3 {
                    for (c, i) in d.cases.iter_mut().zip(icons) {
                        c.color = (*i).to_string();
                    }
                } else {
                    d.cases = icon_set_cases(icons, "33", "67");
                }
            }
        });
    });
    ui.add_space(6.0);
    let mut none = |_: &mut Ui, _: &str, _: &str, _: &str| None;
    rows_ui(ui, d, true, &mut none, l);
    ui.horizontal(|ui| {
        let can_add = d.cases.len() < MAX_CASES;
        if ui
            .add_enabled(can_add, egui::Button::new(l.t("+ New rule", "+ Nouvelle règle")))
            .clicked()
        {
            d.cases.push(Case::new(">", "", "CircleHigh"));
        }
        reverse_button(ui, d, l);
    });
}

/// Data bars: Show bar only, the bounds, the colours, the direction.
fn bars_ui(ui: &mut Ui, d: &mut CfDialog, swatch: &mut Swatch<'_>, l: Lang) {
    check(
        ui,
        &mut d.bars.bar_only,
        l.t("Show bar only", "Afficher uniquement la barre"),
    );
    ui.add_space(4.0);
    egui::Grid::new("bicraft-cf-bars")
        .num_columns(3)
        .spacing(vec2(10.0, 6.0))
        .show(ui, |ui| {
            for (key, cap, auto) in [
                (
                    "min",
                    ("Minimum", "Minimum"),
                    ("Lowest value", "Valeur la plus basse"),
                ),
                (
                    "max",
                    ("Maximum", "Maximum"),
                    ("Highest value", "Valeur la plus haute"),
                ),
            ] {
                let slot = if key == "min" {
                    &mut d.bars.min
                } else {
                    &mut d.bars.max
                };
                ui.label(l.t(cap.0, cap.1));
                let list: [(&'static str, &'static str, &'static str); 2] =
                    [("auto", auto.0, auto.1), ("number", "Number", "Nombre")];
                let now = if slot.is_some() { "number" } else { "auto" };
                let name = format!("{} {}", l.t(cap.0, cap.1), l.t("type", "type"));
                if let Some(k) = choice(ui, &format!("bar-{key}"), now, &list, 150.0, &name, l) {
                    *slot = (k == "number").then(|| slot.clone().unwrap_or_default());
                }
                match slot {
                    Some(n) => {
                        let name = format!("{} {}", l.t(cap.0, cap.1), l.t("value", "valeur"));
                        number_box(ui, n, 70.0, &name);
                    }
                    None => {
                        ui.label("");
                    }
                }
                ui.end_row();
            }
        });
    ui.add_space(4.0);
    egui::Grid::new("bicraft-cf-bar-colours")
        .num_columns(2)
        .spacing(vec2(10.0, 6.0))
        .show(ui, |ui| {
            let colours: [(&str, (&str, &str)); 3] = [
                ("positive", ("Positive bar", "Barre positive")),
                ("negative", ("Negative bar", "Barre négative")),
                ("axis", ("Axis", "Axe")),
            ];
            for (key, (en, fr)) in colours {
                ui.label(l.t(en, fr));
                let cur = match key {
                    "positive" => d.bars.positive.clone(),
                    "negative" => d.bars.negative.clone(),
                    _ => d.bars.axis.clone(),
                };
                let name = format!("{} {}", l.t(en, fr), l.t("colour", "couleur"));
                if let Some(c) = swatch(ui, &format!("bar-{key}"), &cur, &name) {
                    match key {
                        "positive" => d.bars.positive = c,
                        "negative" => d.bars.negative = c,
                        _ => d.bars.axis = c,
                    }
                }
                ui.end_row();
            }
            let name = l.t("Bar direction", "Direction de la barre");
            ui.label(name);
            const DIRS: &[(&str, &str, &str)] = &[
                ("ltr", "Left to right", "De gauche à droite"),
                ("rtl", "Right to left", "De droite à gauche"),
            ];
            let now = if d.bars.reverse { "rtl" } else { "ltr" };
            if let Some(x) = choice(ui, "direction", now, DIRS, 150.0, name, l) {
                d.bars.reverse = x == "rtl";
            }
            ui.end_row();
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sel() -> VisualSel {
        VisualSel {
            page: "p".into(),
            visual: "v".into(),
        }
    }

    fn fields() -> Vec<FieldChoice> {
        vec![
            FieldChoice {
                text: "[Total]".into(),
                label: "Sales › Total".into(),
                measure: true,
                numeric: true,
            },
            FieldChoice {
                text: "Sales[Amount]".into(),
                label: "Sales › Amount".into(),
                measure: false,
                numeric: true,
            },
            FieldChoice {
                text: "Sales[Link]".into(),
                label: "Sales › Link".into(),
                measure: false,
                numeric: false,
            },
        ]
    }

    fn table() -> Value {
        json!({"type": "tableEx", "fields": [
            {"text": "Sales[Category]", "kind": "column"}, {"text": "[Total]", "kind": "measure"}]})
    }

    #[test]
    fn the_fx_targets_follow_the_type() {
        assert_eq!(
            target_of("clusteredColumnChart", "dataPoint", "fill"),
            Some("data_color")
        );
        assert_eq!(target_of("pieChart", "dataPoint", "fill"), Some("data_color"));
        assert_eq!(target_of("tableEx", "values", "backColor"), Some("background"));
        assert_eq!(target_of("pivotTable", "values", "fontColor"), Some("font_color"));
        assert_eq!(target_of("card", "labels", "color"), Some("font_color"));
        assert_eq!(target_of("tableEx", "values", "backColorSecondary"), None);
        assert_eq!(
            target_of("clusteredColumnChart", "labels", "color"),
            Some("label_color")
        );
        assert_eq!(
            target_of("card", "categoryLabels", "color"),
            Some("category_color")
        );
        assert_eq!(target_of("slicer", "dataPoint", "fill"), None);
        assert_eq!(container_target("title", "color"), Some("title_color"));
        assert_eq!(container_target("title", "text"), Some("title_text"));
        assert_eq!(container_target("subtitle", "color"), Some("subtitle_color"));
        assert_eq!(container_target("background", "color"), Some("visual_background"));
        assert_eq!(container_target("border", "color"), Some("border_color"));
        assert_eq!(container_target("title", "font_size"), None);
        assert_eq!(container_target("shadow", "color"), None);
        assert_eq!(Kind::of("icon"), Kind::Icons);
        assert_eq!(Kind::of("data_bars"), Kind::Bars);
        assert_eq!(Kind::of("web_url"), Kind::Url);
        assert_eq!(Kind::of("title_text"), Kind::Text);
        assert_eq!(Kind::of("background"), Kind::Color);
    }

    #[test]
    fn the_rule_follows_the_style() {
        let fields = fields();
        let mut d = CfDialog::new(
            &sel(),
            &table(),
            None,
            "background",
            "Values Background".into(),
            &[],
        );
        assert_eq!((d.series.as_str(), d.field.as_str()), ("[Total]", "[Total]"));
        assert!(!d.existing);
        let r = d.rule(&fields, Lang::En).unwrap();
        assert_eq!(
            d.params(r),
            json!({"target": "background", "series": "[Total]",
                   "rule": {"gradient": {"field": "[Total]", "min": "#FFFFFF", "max": "#118DFF"}}})
        );
        d.field = "Sales[Amount]".into();
        d.style = Style::Rules;
        assert!(d.rule(&fields, Lang::En).is_err(), "no rule yet");
        d.cases.push(Case::two(">=", "1", "<=", "2,5", "#FF0000"));
        assert_eq!(
            d.rule(&fields, Lang::En).unwrap(),
            json!({"rules": {"field": "Sum(Sales[Amount])",
                             "cases": [{"gte": 1.0, "lte": 2.5, "color": "#FF0000"}]}})
        );
        d.cases[0].value = "x".into();
        assert!(d.rule(&fields, Lang::En).is_err(), "not a number");
        d.cases[0] = Case::two(">", "1", ">", "2", "#FF0000");
        assert!(d.rule(&fields, Lang::En).is_err(), "the same operator twice");
        d.cases[0] = Case::new("blank", "", "#00FF00");
        assert_eq!(
            d.rule(&fields, Lang::En).unwrap()["rules"]["cases"],
            json!([{"op": "=", "value": null, "color": "#00FF00"}])
        );
        d.style = Style::FieldValue;
        assert_eq!(
            d.rule(&fields, Lang::En).unwrap(),
            json!({"field_value": {"field": "Sales[Amount]"}})
        );
    }

    /// The gradient's numbers, middle colour, empty values and Apply to.
    #[test]
    fn the_gradient_takes_numbers_blanks_and_apply_to() {
        let fields = fields();
        let mut d = CfDialog::new(&sel(), &table(), None, "font_color", "Values Font".into(), &[]);
        d.min.number = Some("0".into());
        d.mid = Some(Stop {
            color: "#FFFF00".into(),
            number: Some("50".into()),
            percent: false,
        });
        d.max.number = Some("200".into());
        d.blank = Blank::Color("#EEEEEE".into());
        d.apply = "values_and_totals";
        let r = d.rule(&fields, Lang::En).unwrap();
        assert_eq!(
            d.params(r),
            json!({"target": "font_color", "series": "[Total]", "apply_to": "values_and_totals",
                   "rule": {"gradient": {"field": "[Total]", "min": {"color": "#FFFFFF", "value": 0.0},
                            "mid": {"color": "#FFFF00", "value": 50.0},
                            "max": {"color": "#118DFF", "value": 200.0}, "empty": "#EEEEEE"}}})
        );
        // numbers going down are refused
        if let Some(m) = d.mid.as_mut() {
            m.number = Some("300".into());
        }
        assert!(d.rule(&fields, Lang::En).is_err());
        // the preview: 0 white, 50 yellow, 200 blue; 25 half way between white and yellow
        if let Some(m) = d.mid.as_mut() {
            m.number = Some("50".into());
        }
        let p = d.preview();
        let at = |x: f64| p.iter().find(|s| s.value == x).and_then(|s| s.color);
        assert_eq!(at(0.0), Some([255, 255, 255]));
        assert_eq!(at(50.0), Some([255, 255, 0]));
        assert_eq!(at(200.0), Some([0x11, 0x8D, 0xFF]));
        // read back as the engine answers
        let props = json!({"conditional": [{"target": "font_color", "series": "[Total]",
            "apply_to": "values_and_totals", "rule": d.rule(&fields, Lang::En).unwrap()}]});
        let back = CfDialog::new(&sel(), &table(), Some(&props), "font_color", String::new(), &[]);
        assert!(back.existing);
        assert_eq!(
            (back.min.clone(), back.mid.clone(), back.blank.clone(), back.apply),
            (d.min.clone(), d.mid.clone(), d.blank.clone(), "values_and_totals")
        );
    }

    /// The gradient's Percent stops (G54): sent with `percent`, checked from 0 to 100, drawn in
    /// the preview at that percent of the samples' range, read back.
    #[test]
    fn the_gradient_takes_percent_stops() {
        let fields = fields();
        let mut d = CfDialog::new(&sel(), &table(), None, "font_color", "Values Font".into(), &[]);
        let pct = |c: &str, n: &str| Stop {
            color: c.into(),
            number: Some(n.into()),
            percent: true,
        };
        d.min = pct("#000000", "20");
        d.mid = Some(pct("#FFFF00", "50"));
        d.max = pct("#FFFFFF", "80");
        let r = d.rule(&fields, Lang::En).unwrap();
        assert_eq!(
            r,
            json!({"gradient": {"field": "[Total]",
                "min": {"color": "#000000", "value": 20.0, "percent": true},
                "mid": {"color": "#FFFF00", "value": 50.0, "percent": true},
                "max": {"color": "#FFFFFF", "value": 80.0, "percent": true}}})
        );
        // the samples 0..100: the middle one at 50 % gets the centre colour, the ends clamp
        let p = d.preview();
        assert_eq!(p.get(2).and_then(|s| s.color), Some([255, 255, 0]), "{p:?}");
        assert_eq!(p.first().and_then(|s| s.color), Some([0, 0, 0]), "{p:?}");
        assert_eq!(p.last().and_then(|s| s.color), Some([255, 255, 255]), "{p:?}");
        let props = json!({"conditional": [{"target": "font_color", "series": "[Total]", "rule": r}]});
        let back = CfDialog::new(&sel(), &table(), Some(&props), "font_color", String::new(), &[]);
        assert_eq!(
            (back.min.clone(), back.mid.clone()),
            (d.min.clone(), d.mid.clone())
        );
        // over 100 is refused; percents going down too
        d.max.number = Some("120".into());
        assert!(d.rule(&fields, Lang::En).is_err());
        d.max.number = Some("40".into());
        assert!(d.rule(&fields, Lang::En).is_err());
    }

    #[test]
    fn icons_data_bars_and_web_urls() {
        let fields = fields();
        let mut d = CfDialog::new(&sel(), &table(), None, "icon", "Icons".into(), &[]);
        assert_eq!((d.style, d.cases.len()), (Style::Rules, 3), "a set's three rules");
        d.layout = "after";
        d.align = "middle";
        let r = d.rule(&fields, Lang::En).unwrap();
        assert_eq!(
            r,
            json!({"icons": {"field": "[Total]", "position": "after", "align": "middle", "cases": [
                {"op": ">=", "value": 67.0, "icon": "TrafficHigh"},
                {"gte": 33.0, "lt": 67.0, "icon": "TrafficMedium"},
                {"op": "<", "value": 33.0, "icon": "TrafficLow"}]}})
        );
        // the preview's icons follow the rules: 0 low, 50 medium, 100 high
        let p = d.preview();
        let icon = |x: f64| {
            p.iter()
                .find(|s| (s.value - x).abs() < 1e-9)
                .and_then(|s| s.icon.clone())
        };
        assert!(p.iter().all(|s| s.text));
        assert_eq!(p.first().and_then(|s| s.icon.as_deref()), Some("TrafficLow"));
        assert_eq!(p.last().and_then(|s| s.icon.as_deref()), Some("TrafficHigh"));
        let mid = p.get(2).map(|s| s.value).unwrap_or_default();
        assert_eq!(icon(mid).as_deref(), Some("TrafficMedium"), "{p:?}");
        d.reverse();
        assert_eq!(d.cases[0].color, "TrafficLow");
        // read back
        let props = json!({"conditional": [{"target": "icon", "series": "[Total]", "rule": r}]});
        let back = CfDialog::new(&sel(), &table(), Some(&props), "icon", String::new(), &[]);
        assert_eq!(
            (back.layout, back.align, back.cases[1].op2),
            ("after", "middle", Some("<"))
        );

        let mut b = CfDialog::new(&sel(), &table(), None, "data_bars", "Data bars".into(), &[]);
        b.bars.min = Some("-50".into());
        b.bars.max = Some("150".into());
        b.bars.reverse = true;
        b.bars.bar_only = true;
        b.bars.negative = "#FF0000".into();
        let r = b.rule(&fields, Lang::En).unwrap();
        assert_eq!(
            b.params(r.clone()),
            json!({"target": "data_bars", "series": "[Total]", "rule": {"data_bars": {
                "positive": "#118DFF", "negative": "#FF0000", "axis": "#000000", "reverse": true,
                "show_bar_only": true, "min": -50.0, "max": 150.0}}})
        );
        // the preview: -50 a red bar right of the axis (right to left), no text
        let p = b.preview();
        let first = p.first().cloned().unwrap();
        assert_eq!(first.value, -50.0);
        assert_eq!(first.bar, Some((0.75, 1.0, [255, 0, 0])));
        assert_eq!(first.axis, Some(0.75));
        assert!(!first.text);
        b.bars.max = Some("-60".into());
        assert!(b.rule(&fields, Lang::En).is_err(), "min above max");
        let props = json!({"conditional": [{"target": "data_bars", "series": "[Total]", "rule": r}]});
        let back = CfDialog::new(&sel(), &table(), Some(&props), "data_bars", String::new(), &[]);
        assert_eq!(back.bars.min.as_deref(), Some("-50"));
        assert!(back.bars.reverse && back.bars.bar_only);

        let u = CfDialog::new(&sel(), &table(), None, "web_url", "Web URL".into(), &[]);
        assert_eq!(u.series, "Sales[Category]", "a link goes on a text column");
        let mut u = u;
        u.field = "Sales[Link]".into();
        assert_eq!(
            u.params(u.rule(&fields, Lang::En).unwrap()),
            json!({"target": "web_url", "series": "Sales[Category]", "rule": {"web_url": {"field": "Sales[Link]"}}})
        );
        assert!(u.preview().is_empty());
    }

    /// The rules' preview colours: the first rule met, nothing when none is.
    #[test]
    fn the_rules_preview_takes_the_first_rule_met() {
        let mut d = CfDialog::new(&sel(), &table(), None, "background", String::new(), &[]);
        d.style = Style::Rules;
        d.cases = vec![
            Case::two(">=", "0", "<", "100", "#FF0000"),
            Case::new(">=", "100", "#00FF00"),
        ];
        let p = d.preview();
        // samples from -50 to 150 (the thresholds padded by half their range)
        assert_eq!(
            p.iter().map(|s| s.value).collect::<Vec<_>>(),
            [-50.0, 0.0, 50.0, 100.0, 150.0]
        );
        assert_eq!(
            p.iter().map(|s| s.color).collect::<Vec<_>>(),
            [
                None,
                Some([255, 0, 0]),
                Some([255, 0, 0]),
                Some([0, 255, 0]),
                Some([0, 255, 0])
            ]
        );
        d.reverse();
        assert_eq!(
            (d.cases[0].color.as_str(), d.cases[1].color.as_str()),
            ("#00FF00", "#FF0000")
        );
    }

    #[test]
    fn the_dialog_reads_the_visuals_format_back() {
        let v = json!({"type": "clusteredColumnChart"});
        let props = json!({"conditional": [
            {"target": "data_color", "rule": {"rules": {"field": "Sum(Sales[Amount])", "cases": [
                {"op": ">", "value": 10, "color": "#00FF00"},
                {"gte": {"literal": "1D"}, "lt": 5, "color": "#0000FF"},
                {"op": "between", "values": [1, 2], "color": "#FF0000"}]}}}]});
        let d = CfDialog::new(&sel(), &v, Some(&props), "data_color", "Data colours".into(), &[]);
        assert!(d.existing);
        assert_eq!(d.style, Style::Rules);
        assert_eq!((d.field.as_str(), d.agg), ("Sales[Amount]", "Sum"));
        assert_eq!(d.cases.len(), 3);
        assert_eq!(d.cases[0], Case::new(">", "10", "#00FF00"));
        assert_eq!(d.cases[1], Case::two(">=", "1", "<", "5", "#0000FF"));
        assert_eq!(d.cases[2], Case::two(">=", "1", "<=", "2", "#FF0000"));
        // damaged answers: nothing read, no panic
        for odd in [
            json!(null),
            json!({"conditional": 3}),
            json!({"conditional": [1, {"target": "data_color", "rule": {"gradient": 7}}]}),
            json!({"conditional": [{"target": "data_bars", "rule": {"data_bars": {"min": [], "positive": 3}}}]}),
            json!({"conditional": [{"target": "icon", "rule": {"icons": {"cases": [{"gt": 1, "lt": 2, "eq": 3, "icon": "X"}]}}}]}),
        ] {
            for t in ["data_color", "data_bars", "icon"] {
                let _ = CfDialog::new(&sel(), &v, Some(&odd), t, String::new(), &[]);
            }
        }
    }
}
