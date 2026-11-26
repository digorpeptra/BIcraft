//! Conditional formatting (M2.5 d): the `conditional` key of the visual `props` object and the
//! `visual.conditional` command.
//!
//! `conditional` is a list of `{target?, series?, apply_to?, rule}`; each item sets (or, with
//! `rule: null`, removes) the conditional format of one property for one series. It is a patch:
//! properties not named stay as they are.
//!
//! - `target`: a friendly name or `{object, property}`:
//!   - tables and matrices (`tableEx`, `pivotTable`): `background` (`values.backColor`),
//!     `font_color` (`values.fontColor`), `icon` (`values.icon`), `data_bars`
//!     (`columnFormatting.dataBars`), `web_url` (`values.webURL`);
//!   - cards: `font_color` (`labels.color`);
//!   - charts: `data_color` (`dataPoint.fill`), `label_color` (`labels.color`, the data labels);
//!   - cards: also `category_color` (`categoryLabels.color`);
//!   - charts, gauges and KPIs, one value for the whole visual (written, as the container's, in
//!     the object's entry without selector): `category_axis_color` (`categoryAxis.labelColor`),
//!     `category_title_color` (`categoryAxis.titleColor`), `value_axis_color`
//!     (`valueAxis.labelColor`), `value_title_color` (`valueAxis.titleColor`), `legend_color`
//!     (`legend.labelColor`); gauges `target_color` (`dataPoint.target`), `target_label_color`
//!     (`target.color`), `callout_color` (`calloutValue.color`); KPIs `indicator_color`
//!     (`indicator.fontColor`), `goal_color` (`goals.goalFontColor`), `distance_color`
//!     (`goals.distanceFontColor`), `good_color`, `neutral_color`, `bad_color`
//!     (`status.goodColor`…). Each needs the property in the type's catalog;
//!   - every visual, in `visualContainerObjects` (one value for the whole visual, written in the
//!     object's entry without selector): `title_text` (`title.text`, a field value), `title_color`
//!     (`title.fontColor`), `title_background` (`title.background`), `subtitle_text`
//!     (`subTitle.text`), `subtitle_color` (`subTitle.fontColor`), `visual_background`
//!     (`background.color`), `border_color` (`border.color`). They take no `series` and no
//!     `apply_to`.
//!
//!   Default: `data_bars`, `icon` and `web_url` for those rules; else `background` (tables),
//!   `font_color` (cards) or `data_color`.
//! - `series`: the bound field whose values are formatted (a table column, a chart's measure),
//!   written as the selector's `metadata` (its `queryRef`). Tables need it; it defaults to the
//!   rule's field when that is bound.
//! - `occurrence`: which binding of `series` is formatted when the field is bound more than once
//!   (1, the default, is the first; Desktop gives the others their own `queryRef`).
//! - `apply_to`: `values` (default), `values_and_totals` or `totals`
//!   (`dataViewWildcard.matchingOption` 1, 0, 2).
//! - `rule`, one of
//!   - `{gradient: {field, min, mid?, max, empty?}}`: a color scale. Stops are `"#hex"` (lowest /
//!     middle / highest value) or `{color, value}` (a number). `empty`: `zero`, `none` or a color
//!     for blank values. Written as a `FillRule` (`linearGradient2` / `linearGradient3`).
//!   - `{rules: {field, cases: [{op, value | values, color}], else?}}`: `op` is `=`, `>`, `>=`,
//!     `<`, `<=` or `between` (`values: [low, high]`, both included); `{gte?, gt?, lte?, lt?,
//!     eq?, color}` combines up to two bounds. `= null` is "is blank". `percent: true` reads the
//!     case's bounds as percents (0 to 100) of the range from the lowest to the highest value
//!     (Desktop's Percent); each bound is then written as `Min + (Max - Min) * p / 100` of the
//!     rule field (`Arithmetic` over two `Aggregation`s, BIcraft's shape). Written as
//!     `Conditional` cases.
//!   - `{field_value: {field, as?: color|value}}`: the color (or text) comes from a field. A
//!     column is read through `Min` (Desktop's "First").
//!   - `{icons: {field, cases: [{…, icon}] | preset + thresholds: [low, high], else?, position?:
//!     before|after|only, align?: top|middle|bottom}}`. Presets: circles, traffic, triangles,
//!     symbols, flags, stars, arrows.
//!   - `{data_bars: {positive?, negative?, axis?, reverse?, show_bar_only?, min?, max?}}`.
//!   - `{web_url: {field}}`.
//!
//! Fields are `[Measure]` or `Function(Table[Column])`; gradient, rules and icons need a number
//! (a measure or an aggregation).

use bicraft_model::SemanticModel;
use bicraft_report::{FieldRef, Visual};
use bicraft_visuals::PropType;
use serde_json::{Map, Value, json};

use crate::refs::{field_text, same_field};
use crate::visual_props::{
    Lit, check_keys, color_text, decode_literal, field_from_json, field_to_json, is_color, num_text,
    parse_lit, quote, to_lit, value_kind,
};
use crate::visual_series::{Check, Ctx, EntryOp, color_value, literal};

/// Most `conditional` items.
const MAX_ITEMS: usize = 200;
/// Most cases of one rule.
const MAX_CASES: usize = 50;
/// Longest icon name.
const MAX_ICON: usize = 100;

const DEFAULT_POSITIVE: &str = "#118DFF";
const DEFAULT_NEGATIVE: &str = "#D64550";
const DEFAULT_AXIS: &str = "#000000";

/// Icon sets: (preset, high, medium, low).
const ICON_PRESETS: &[(&str, &str, &str, &str)] = &[
    ("circles", "CircleHigh", "SignMedium", "SignLow"),
    ("traffic", "TrafficHigh", "TrafficMedium", "TrafficLow"),
    ("triangles", "TriangleHigh", "TriangleMedium", "TriangleLow"),
    ("symbols", "SymbolHigh", "SymbolMedium", "SymbolLow"),
    ("flags", "FlagHigh", "FlagMedium", "FlagLow"),
    ("stars", "StarHigh", "StarMedium", "StarLow"),
    (
        "arrows",
        "ColoredArrowUp",
        "ColoredArrowRight",
        "ColoredArrowDown",
    ),
];

const POSITIONS: &[(&str, &str)] = &[("before", "Before"), ("after", "After"), ("only", "IconOnly")];
const ALIGNS: &[(&str, &str)] = &[("top", "Top"), ("middle", "Middle"), ("bottom", "Bottom")];
const APPLY_TO: &[(&str, u64)] = &[("values", 1), ("values_and_totals", 0), ("totals", 2)];
/// (op, QueryComparisonKind, key form)
const OPS: &[(&str, u64, &str)] = &[
    ("=", 0, "eq"),
    (">", 1, "gt"),
    (">=", 2, "gte"),
    ("<", 3, "lt"),
    ("<=", 4, "lte"),
];

// ---------------------------------------------------------------------------------------------
// Targets
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    Table,
    Card,
    Chart,
    /// Every visual (the container's targets).
    Any,
}

impl Family {
    /// Whether an alias of this family is a target of a visual of family `fam`.
    fn has(self, fam: Family) -> bool {
        self == fam || self == Family::Any
    }
}

/// The container objects (`visualContainerObjects`) the aliases name.
const CONTAINER_ALIASED: &[&str] = &["title", "subTitle", "background", "border"];

/// Whether `object` of a `vt` lives in `visualContainerObjects`: an aliased container object, or
/// (named as `{object, property}`) a container object when the type has no visual object of
/// that name.
fn is_container(vt: Option<&str>, object: &str, aliased: bool) -> bool {
    if aliased {
        return CONTAINER_ALIASED.contains(&object);
    }
    let visual_object = vt.is_some_and(|t| bicraft_visuals::object_in(t, object, false).is_some());
    !visual_object
        && bicraft_visuals::container_objects()
            .iter()
            .any(|(n, _)| *n == object)
}

fn family(visual_type: Option<&str>) -> Family {
    match visual_type {
        Some("tableEx" | "pivotTable") => Family::Table,
        Some("card") => Family::Card,
        _ => Family::Chart,
    }
}

/// What a property holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PropKind {
    Color,
    Icon,
    Bars,
    /// A plain value (`{"expr": …}`): text, URL.
    Plain,
}

/// (alias, family, object, property, kind)
const ALIASES: &[(&str, Family, &str, &str, PropKind)] = &[
    (
        "background",
        Family::Table,
        "values",
        "backColor",
        PropKind::Color,
    ),
    (
        "font_color",
        Family::Table,
        "values",
        "fontColor",
        PropKind::Color,
    ),
    ("icon", Family::Table, "values", "icon", PropKind::Icon),
    (
        "data_bars",
        Family::Table,
        "columnFormatting",
        "dataBars",
        PropKind::Bars,
    ),
    ("web_url", Family::Table, "values", "webURL", PropKind::Plain),
    ("font_color", Family::Card, "labels", "color", PropKind::Color),
    (
        "category_color",
        Family::Card,
        "categoryLabels",
        "color",
        PropKind::Color,
    ),
    ("data_color", Family::Chart, "dataPoint", "fill", PropKind::Color),
    ("label_color", Family::Chart, "labels", "color", PropKind::Color),
    ("title_text", Family::Any, "title", "text", PropKind::Plain),
    ("title_color", Family::Any, "title", "fontColor", PropKind::Color),
    (
        "title_background",
        Family::Any,
        "title",
        "background",
        PropKind::Color,
    ),
    ("subtitle_text", Family::Any, "subTitle", "text", PropKind::Plain),
    (
        "subtitle_color",
        Family::Any,
        "subTitle",
        "fontColor",
        PropKind::Color,
    ),
    (
        "visual_background",
        Family::Any,
        "background",
        "color",
        PropKind::Color,
    ),
    ("border_color", Family::Any, "border", "color", PropKind::Color),
    (
        "category_axis_color",
        Family::Chart,
        "categoryAxis",
        "labelColor",
        PropKind::Color,
    ),
    (
        "category_title_color",
        Family::Chart,
        "categoryAxis",
        "titleColor",
        PropKind::Color,
    ),
    (
        "value_axis_color",
        Family::Chart,
        "valueAxis",
        "labelColor",
        PropKind::Color,
    ),
    (
        "value_title_color",
        Family::Chart,
        "valueAxis",
        "titleColor",
        PropKind::Color,
    ),
    (
        "legend_color",
        Family::Chart,
        "legend",
        "labelColor",
        PropKind::Color,
    ),
    (
        "target_color",
        Family::Chart,
        "dataPoint",
        "target",
        PropKind::Color,
    ),
    (
        "target_label_color",
        Family::Chart,
        "target",
        "color",
        PropKind::Color,
    ),
    (
        "callout_color",
        Family::Chart,
        "calloutValue",
        "color",
        PropKind::Color,
    ),
    (
        "indicator_color",
        Family::Chart,
        "indicator",
        "fontColor",
        PropKind::Color,
    ),
    (
        "goal_color",
        Family::Chart,
        "goals",
        "goalFontColor",
        PropKind::Color,
    ),
    (
        "distance_color",
        Family::Chart,
        "goals",
        "distanceFontColor",
        PropKind::Color,
    ),
    (
        "good_color",
        Family::Chart,
        "status",
        "goodColor",
        PropKind::Color,
    ),
    (
        "neutral_color",
        Family::Chart,
        "status",
        "neutralColor",
        PropKind::Color,
    ),
    ("bad_color", Family::Chart, "status", "badColor", PropKind::Color),
];

/// The visual objects' properties formatted as one value for the whole visual (an axis's
/// labels and title, the legend's text, a gauge's target and callout, a KPI's indicator, goal
/// and status colours): written in the object's entry without selector, as the container's
/// targets (no public source gives Desktop's selector for them).
const WHOLE: &[(&str, &str)] = &[
    ("categoryAxis", "labelColor"),
    ("categoryAxis", "titleColor"),
    ("valueAxis", "labelColor"),
    ("valueAxis", "titleColor"),
    ("legend", "labelColor"),
    ("dataPoint", "target"),
    ("target", "color"),
    ("calloutValue", "color"),
    ("indicator", "fontColor"),
    ("goals", "goalFontColor"),
    ("goals", "distanceFontColor"),
    ("status", "goodColor"),
    ("status", "neutralColor"),
    ("status", "badColor"),
];

/// Whether `object.prop` is one of [`WHOLE`].
fn whole_prop(object: &str, prop: &str) -> bool {
    WHOLE.iter().any(|(o, p)| *o == object && *p == prop)
}

/// Whether `object.prop` of a `vt` is one of [`WHOLE`] and in the type's catalog.
fn is_whole(vt: Option<&str>, object: &str, prop: &str) -> bool {
    whole_prop(object, prop) && vt.is_some_and(|t| bicraft_visuals::property(t, object, prop).is_some())
}

#[derive(Debug, Clone, PartialEq)]
struct Target {
    object: String,
    prop: String,
    kind: PropKind,
    alias: Option<&'static str>,
    /// In `visualContainerObjects` (one value for the visual, no selector).
    container: bool,
    /// One value for the whole visual in its `objects` ([`WHOLE`]), no selector.
    whole: bool,
}

/// What a property not in [`ALIASES`] holds, by the catalog or by its name.
fn guess_kind(visual_type: Option<&str>, object: &str, prop: &str) -> PropKind {
    if let Some(p) = visual_type.and_then(|vt| bicraft_visuals::property(vt, object, prop)) {
        return match p.ty {
            PropType::Color => PropKind::Color,
            _ if prop == "icon" => PropKind::Icon,
            _ if prop == "dataBars" => PropKind::Bars,
            _ => PropKind::Plain,
        };
    }
    let lower = prop.to_ascii_lowercase();
    if lower.ends_with("color") || lower == "fill" {
        PropKind::Color
    } else if prop == "icon" {
        PropKind::Icon
    } else if prop == "dataBars" {
        PropKind::Bars
    } else {
        PropKind::Plain
    }
}

fn alias_target(name: &str, fam: Family) -> Option<Target> {
    ALIASES
        .iter()
        .find(|(a, f, ..)| *a == name && f.has(fam))
        .map(|(a, _, o, p, k)| Target {
            object: (*o).to_string(),
            prop: (*p).to_string(),
            kind: *k,
            alias: Some(*a),
            container: is_container(None, o, true),
            whole: whole_prop(o, p),
        })
}

/// The friendly name of an object / property pair on this visual type, when it has one.
fn alias_of(object: &str, prop: &str, fam: Family) -> Option<&'static str> {
    ALIASES
        .iter()
        .find(|(_, f, o, p, _)| f.has(fam) && *o == object && *p == prop)
        .map(|(a, ..)| *a)
}

/// Whether `alias` names a container target (`visualContainerObjects`).
fn container_alias(alias: &str) -> bool {
    ALIASES.iter().any(|(a, f, ..)| *a == alias && *f == Family::Any)
}

fn parse_target(
    t: Option<&Value>,
    rule_kind: &str,
    ctx: &Ctx<'_>,
    what: &str,
) -> Result<(Target, Option<String>), String> {
    let vt = ctx.visual_type.as_deref();
    let fam = family(vt);
    let default = match rule_kind {
        "data_bars" => "data_bars",
        "icons" => "icon",
        "web_url" => "web_url",
        _ => match fam {
            Family::Table => "background",
            Family::Card => "font_color",
            Family::Chart | Family::Any => "data_color",
        },
    };
    let names = |fam: Family| -> String {
        ALIASES
            .iter()
            .filter(|(_, f, ..)| f.has(fam))
            .map(|(a, ..)| *a)
            .collect::<Vec<_>>()
            .join(", ")
    };
    let (target, series) = match t {
        None | Some(Value::Null) => (
            alias_target(default, fam).ok_or_else(|| {
                format!(
                    "{what}: a {} has no `{default}` target; name one as {{object, property}}",
                    vt.unwrap_or("visual")
                )
            })?,
            None,
        ),
        Some(Value::String(s)) => (
            alias_target(s, fam).ok_or_else(|| {
                format!(
                    "{what}.target `{s}` is not a target of a {} (expected {} or {{object, property}})",
                    vt.unwrap_or("visual"),
                    names(fam)
                )
            })?,
            None,
        ),
        Some(Value::Object(o)) => {
            check_keys(o, &["object", "property", "series"], &format!("{what}.target"))?;
            let object = o
                .get("object")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("{what}.target.object must be a string"))?;
            let prop = o
                .get("property")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("{what}.target.property must be a string"))?;
            crate::visual_props::check_ident(object, "object")?;
            crate::visual_props::check_ident(prop, "property")?;
            let series = match o.get("series") {
                None | Some(Value::Null) => None,
                Some(Value::String(s)) => Some(s.clone()),
                Some(_) => return Err(format!("{what}.target.series must be a field string")),
            };
            let kind = ALIASES
                .iter()
                .find(|(_, f, ob, p, _)| f.has(fam) && *ob == object && *p == prop)
                .map_or_else(|| guess_kind(vt, object, prop), |(.., k)| *k);
            let alias = alias_of(object, prop, fam)
                .filter(|a| !container_alias(a) || is_container(vt, object, false));
            (
                Target {
                    object: object.to_string(),
                    prop: prop.to_string(),
                    kind,
                    alias,
                    container: is_container(vt, object, alias.is_some()),
                    whole: is_whole(vt, object, prop),
                },
                series,
            )
        }
        Some(_) => return Err(format!("{what}.target must be a name or {{object, property}}")),
    };
    if target.whole && !is_whole(vt, &target.object, &target.prop) {
        return Err(format!(
            "{what}: a {} has no {}.{}",
            vt.unwrap_or("visual"),
            target.object,
            target.prop
        ));
    }
    Ok((target, series))
}

// ---------------------------------------------------------------------------------------------
// Rules
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Empty {
    Zero,
    NoColor,
    Color(String),
}

/// One case: up to two comparisons (kind, literal), and the color or icon it gives.
#[derive(Debug, Clone, PartialEq)]
struct Case {
    conds: Vec<(u64, Lit)>,
    value: String,
    /// The bounds are percents (0 to 100) of the rule field's range (Desktop's Percent).
    percent: bool,
}

#[derive(Debug, Clone, PartialEq)]
enum Rule {
    Gradient {
        field: FieldRef,
        /// (stop name, color, value)
        stops: Vec<(&'static str, String, Option<f64>)>,
        empty: Option<Empty>,
    },
    Rules {
        field: FieldRef,
        cases: Vec<Case>,
        default: Option<String>,
    },
    FieldValue {
        field: FieldRef,
    },
    Icons {
        field: FieldRef,
        cases: Vec<Case>,
        default: Option<String>,
        layout: String,
        align: String,
    },
    DataBars {
        positive: String,
        negative: String,
        axis: String,
        reverse: bool,
        hide_text: bool,
        min: Option<f64>,
        max: Option<f64>,
    },
    WebUrl {
        field: FieldRef,
    },
}

impl Rule {
    fn kind_name(&self) -> &'static str {
        match self {
            Self::Gradient { .. } => "gradient",
            Self::Rules { .. } => "rules",
            Self::FieldValue { .. } => "field_value",
            Self::Icons { .. } => "icons",
            Self::DataBars { .. } => "data_bars",
            Self::WebUrl { .. } => "web_url",
        }
    }

    fn field(&self) -> Option<&FieldRef> {
        match self {
            Self::Gradient { field, .. }
            | Self::Rules { field, .. }
            | Self::FieldValue { field }
            | Self::Icons { field, .. }
            | Self::WebUrl { field } => Some(field),
            Self::DataBars { .. } => None,
        }
    }
}

fn lit_expr(text: String) -> Value {
    json!({"Literal": {"Value": text}})
}

fn field_expr(f: &FieldRef) -> Result<Value, String> {
    field_to_json(f, None).ok_or_else(|| format!("{} cannot be written in a rule", field_text(f)))
}

/// `Min + (Max - Min) * p / 100` of `field` over the visual's points: a Percent bound.
fn percent_bound(field: &Value, p: f64) -> Value {
    let agg = |f: u64| json!({"Aggregation": {"Expression": field, "Function": f}});
    json!({"Arithmetic": {
        "Left": agg(3),
        "Right": {"Arithmetic": {
            "Left": {"Arithmetic": {"Left": agg(4), "Right": agg(3), "Operator": 1}},
            "Right": lit_expr(format!("{}D", num_text(p / 100.0))),
            "Operator": 2
        }},
        "Operator": 0
    }})
}

fn condition(field: &Value, conds: &[(u64, Lit)], percent: bool) -> Result<Value, String> {
    let right = |l: &Lit| match (percent, l) {
        (true, Lit::Num(n, _)) => percent_bound(field, *n),
        _ => l.expr(),
    };
    let cmp =
        |k: u64, l: &Lit| json!({"Comparison": {"ComparisonKind": k, "Left": field, "Right": right(l)}});
    match conds {
        [(k, l)] => Ok(cmp(*k, l)),
        [(k1, l1), (k2, l2)] => Ok(json!({"And": {"Left": cmp(*k1, l1), "Right": cmp(*k2, l2)}})),
        _ => Err("internal: a case has one or two conditions".into()),
    }
}

fn conditional(field: &FieldRef, cases: &[Case], default: Option<&String>) -> Result<Value, String> {
    let f = field_expr(field)?;
    let mut list = Vec::new();
    for c in cases {
        list.push(
            json!({"Condition": condition(&f, &c.conds, c.percent)?, "Value": lit_expr(quote(&c.value))}),
        );
    }
    let mut out = Map::new();
    out.insert("Cases".into(), Value::Array(list));
    if let Some(d) = default {
        out.insert("DefaultValue".into(), lit_expr(quote(d)));
    }
    Ok(json!({ "Conditional": out }))
}

/// The PBIR value of `rule` on a property of `kind`.
fn render(rule: &Rule, kind: PropKind) -> Result<Value, String> {
    let color = |expr: Value| json!({"solid": {"color": {"expr": expr}}});
    Ok(match rule {
        Rule::Gradient { field, stops, empty } => {
            let mut g = Map::new();
            for (name, c, v) in stops {
                let mut stop = Map::new();
                stop.insert("color".into(), lit_expr(quote(c)));
                if let Some(n) = v {
                    stop.insert("value".into(), lit_expr(format!("{}D", num_text(*n))));
                }
                g.insert((*name).to_string(), Value::Object(stop));
            }
            if let Some(e) = empty {
                let s = match e {
                    Empty::Zero => json!({"strategy": lit_expr(quote("asZero"))}),
                    Empty::NoColor => json!({"strategy": lit_expr(quote("noColor"))}),
                    Empty::Color(c) => {
                        json!({"strategy": lit_expr(quote("specificColor")), "color": lit_expr(quote(c))})
                    }
                };
                g.insert("nullColoringStrategy".into(), s);
            }
            let name = if stops.len() == 3 {
                "linearGradient3"
            } else {
                "linearGradient2"
            };
            color(json!({"FillRule": {"Input": field_expr(field)?, "FillRule": {name: g}}}))
        }
        Rule::Rules {
            field,
            cases,
            default,
        } => color(conditional(field, cases, default.as_ref())?),
        Rule::FieldValue { field } => match kind {
            PropKind::Color => color(field_expr(field)?),
            _ => json!({"expr": field_expr(field)?}),
        },
        Rule::WebUrl { field } => json!({"expr": field_expr(field)?}),
        Rule::Icons {
            field,
            cases,
            default,
            layout,
            align,
        } => json!({
            "kind": "Icon",
            "value": {"expr": conditional(field, cases, default.as_ref())?},
            "layout": literal(quote(layout)),
            "verticalAlignment": literal(quote(align)),
        }),
        Rule::DataBars {
            positive,
            negative,
            axis,
            reverse,
            hide_text,
            min,
            max,
        } => {
            // Desktop's key order: colors, flags, then the bounds.
            let mut b = Map::new();
            b.insert("positiveColor".into(), color_value(positive));
            b.insert("negativeColor".into(), color_value(negative));
            b.insert("axisColor".into(), color_value(axis));
            b.insert("reverseDirection".into(), literal(reverse.to_string()));
            b.insert("hideText".into(), literal(hide_text.to_string()));
            if let Some(n) = min {
                b.insert("minValue".into(), literal(format!("{}D", bound_text(*n))));
            }
            if let Some(n) = max {
                b.insert("maxValue".into(), literal(format!("{}D", bound_text(*n))));
            }
            Value::Object(b)
        }
    })
}

/// A data-bar bound as Desktop writes it: C's `%g` (six significant digits, `1.2e+06` from a
/// million up) when that is exact, else every digit.
pub(crate) fn bound_text(x: f64) -> String {
    let sci = format!("{x:.5e}");
    let Some((mantissa, exp)) = sci.split_once('e') else {
        return num_text(x);
    };
    let Ok(e) = exp.parse::<i32>() else {
        return num_text(x);
    };
    let trim = |t: &str| -> String {
        if t.contains('.') {
            t.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            t.to_string()
        }
    };
    let g = if (-4..6).contains(&e) {
        let decimals = usize::try_from(5 - e).unwrap_or(0);
        trim(&format!("{x:.decimals$}"))
    } else {
        let sign = if e < 0 { '-' } else { '+' };
        format!("{}e{sign}{:02}", trim(mantissa), e.unsigned_abs())
    };
    if g.parse::<f64>().ok() == Some(x) {
        g
    } else {
        num_text(x)
    }
}

// ---------------------------------------------------------------------------------------------
// Parsing rules
// ---------------------------------------------------------------------------------------------

fn obj<'a>(v: &'a Value, what: &str) -> Result<&'a Map<String, Value>, String> {
    v.as_object().ok_or_else(|| format!("{what} must be an object"))
}

fn color_str(v: Option<&Value>, what: &str) -> Result<String, String> {
    match v.and_then(Value::as_str) {
        Some(s) if is_color(s) => Ok(s.to_string()),
        _ => Err(format!("{what} must be a color like #118DFF")),
    }
}

/// A rule's field: a number for gradient / rules / icons; a column is read through `Min` for
/// field values.
fn rule_field(o: &Map<String, Value>, ctx: &Ctx<'_>, numeric: bool, what: &str) -> Result<FieldRef, String> {
    let text = o
        .get("field")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{what}.field must be a field like [Total] or Sum(Sales[Amount])"))?;
    let f = ctx.field(text).map_err(|e| format!("{what}.field: {e}"))?;
    match f {
        FieldRef::Measure { .. } | FieldRef::Aggregation { .. } => Ok(f),
        FieldRef::Column { table, column } if !numeric => Ok(FieldRef::Aggregation {
            table,
            column,
            function: "Min".into(),
        }),
        FieldRef::Column { .. } => Err(format!(
            "{what}.field: {} is a column; use a measure or an aggregation like Sum({})",
            field_text(&f),
            field_text(&f)
        )),
        FieldRef::Other { .. } => Err(format!("{what}.field: unsupported field")),
    }
}

fn number(v: &Value, what: &str) -> Result<f64, String> {
    v.as_f64()
        .filter(|n| n.is_finite())
        .ok_or_else(|| format!("{what} must be a number"))
}

fn stop(v: &Value, what: &str) -> Result<(String, Option<f64>), String> {
    match v {
        Value::String(_) => Ok((color_str(Some(v), what)?, None)),
        Value::Object(o) => {
            check_keys(o, &["color", "value"], what)?;
            let c = color_str(o.get("color"), &format!("{what}.color"))?;
            let value = match o.get("value") {
                None | Some(Value::Null) => None,
                Some(Value::String(s)) if ["min", "max", "middle", "auto"].contains(&s.as_str()) => None,
                Some(x) => Some(number(
                    x,
                    &format!("{what}.value (a number, or min / max / middle)"),
                )?),
            };
            Ok((c, value))
        }
        _ => Err(format!("{what} must be a color or {{color, value?}}")),
    }
}

fn parse_gradient(v: &Value, ctx: &Ctx<'_>, what: &str) -> Result<Rule, String> {
    let o = obj(v, what)?;
    check_keys(o, &["field", "min", "mid", "max", "empty"], what)?;
    let field = rule_field(o, ctx, true, what)?;
    let mut stops = Vec::new();
    for name in ["min", "mid", "max"] {
        match o.get(name) {
            None | Some(Value::Null) if name == "mid" => {}
            None | Some(Value::Null) => {
                return Err(format!("{what}.{name} is missing (a color or {{color, value}})"));
            }
            Some(x) => {
                let (c, val) = stop(x, &format!("{what}.{name}"))?;
                stops.push((name, c, val));
            }
        }
    }
    let empty = match o.get("empty") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if s == "zero" => Some(Empty::Zero),
        Some(Value::String(s)) if s == "none" => Some(Empty::NoColor),
        Some(Value::String(s)) if is_color(s) => Some(Empty::Color(s.clone())),
        Some(_) => return Err(format!("{what}.empty must be \"zero\", \"none\" or a color")),
    };
    Ok(Rule::Gradient { field, stops, empty })
}

fn op_kind(op: &str) -> Option<u64> {
    OPS.iter().find(|(o, ..)| *o == op).map(|(_, k, _)| *k)
}

fn parse_case(
    v: &Value,
    field: &FieldRef,
    ctx: &Ctx<'_>,
    value_key: &str,
    what: &str,
) -> Result<Case, String> {
    let o = obj(v, what)?;
    let mut allowed = vec!["op", "value", "values", "percent", value_key];
    allowed.extend(OPS.iter().map(|(.., k)| *k));
    check_keys(o, &allowed, what)?;
    let kind = value_kind(ctx.model, field);
    let lit = |x: &Value, w: &str| -> Result<Lit, String> { to_lit(x, kind, w) };
    let mut conds: Vec<(u64, Lit)> = Vec::new();
    if let Some(op) = o.get("op") {
        let op = op
            .as_str()
            .ok_or_else(|| format!("{what}.op must be =, >, >=, <, <= or between"))?;
        if OPS.iter().any(|(.., k)| o.contains_key(*k)) {
            return Err(format!("{what}: use either op or gt/gte/lt/lte/eq, not both"));
        }
        if op == "between" {
            if o.contains_key("value") {
                return Err(format!("{what}: between takes values: [low, high]"));
            }
            let vals = o
                .get("values")
                .and_then(Value::as_array)
                .filter(|a| a.len() == 2)
                .ok_or_else(|| format!("{what}.values must be [low, high]"))?;
            for (k, x) in [(2u64, vals.first()), (4, vals.get(1))] {
                let x = x.ok_or("internal: between bounds")?;
                if x.is_null() {
                    return Err(format!("{what}: between bounds cannot be null"));
                }
                conds.push((k, lit(x, &format!("{what}.values"))?));
            }
        } else {
            let k =
                op_kind(op).ok_or_else(|| format!("{what}.op `{op}` is not =, >, >=, <, <= or between"))?;
            if o.contains_key("values") {
                return Err(format!("{what}: values goes with op between"));
            }
            let x = o
                .get("value")
                .ok_or_else(|| format!("{what}.value is missing (null means blank, with =)"))?;
            let l = lit(x, &format!("{what}.value"))?;
            if l == Lit::Null && k != 0 {
                return Err(format!("{what}: null (blank) only works with ="));
            }
            conds.push((k, l));
        }
    } else {
        if o.contains_key("value") || o.contains_key("values") {
            return Err(format!("{what}: value / values need an op"));
        }
        for (_, k, key) in OPS {
            if let Some(x) = o.get(*key) {
                let l = lit(x, &format!("{what}.{key}"))?;
                if l == Lit::Null && *k != 0 {
                    return Err(format!("{what}: null (blank) only works with eq"));
                }
                conds.push((*k, l));
            }
        }
        if conds.is_empty() || conds.len() > 2 {
            return Err(format!(
                "{what} needs op + value, or one or two of gt / gte / lt / lte / eq"
            ));
        }
    }
    let value = match value_key {
        "color" => color_str(o.get("color"), &format!("{what}.color"))?,
        _ => icon_name(o.get("icon"), &format!("{what}.icon"))?,
    };
    let percent = match o.get("percent") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(format!("{what}.percent must be true or false")),
    };
    if percent {
        for (_, l) in &conds {
            match l {
                Lit::Num(n, _) if (0.0..=100.0).contains(n) => {}
                _ => {
                    return Err(format!(
                        "{what}: with percent, each bound is a number from 0 to 100"
                    ));
                }
            }
        }
    }
    Ok(Case {
        conds,
        value,
        percent,
    })
}

fn icon_name(v: Option<&Value>, what: &str) -> Result<String, String> {
    match v.and_then(Value::as_str) {
        Some(s)
            if !s.is_empty()
                && s.len() <= MAX_ICON
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') =>
        {
            Ok(s.to_string())
        }
        _ => Err(format!(
            "{what} must be an icon name like CircleHigh, TrafficLow or ColoredArrowUp"
        )),
    }
}

fn parse_cases(
    o: &Map<String, Value>,
    field: &FieldRef,
    ctx: &Ctx<'_>,
    value_key: &str,
    what: &str,
) -> Result<Vec<Case>, String> {
    let list = o
        .get("cases")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{what}.cases must be a list of cases"))?;
    if list.is_empty() || list.len() > MAX_CASES {
        return Err(format!("{what}.cases must have 1 to {MAX_CASES} cases"));
    }
    list.iter()
        .enumerate()
        .map(|(i, c)| parse_case(c, field, ctx, value_key, &format!("{what}.cases[{i}]")))
        .collect()
}

fn parse_rules(v: &Value, ctx: &Ctx<'_>, what: &str) -> Result<Rule, String> {
    let o = obj(v, what)?;
    check_keys(o, &["field", "cases", "else"], what)?;
    let field = rule_field(o, ctx, true, what)?;
    let cases = parse_cases(o, &field, ctx, "color", what)?;
    let default = match o.get("else") {
        None | Some(Value::Null) => None,
        x => Some(color_str(x, &format!("{what}.else"))?),
    };
    Ok(Rule::Rules {
        field,
        cases,
        default,
    })
}

fn enum_param(
    o: &Map<String, Value>,
    key: &str,
    table: &[(&str, &str)],
    default: &str,
    what: &str,
) -> Result<String, String> {
    match o.get(key) {
        None | Some(Value::Null) => Ok(default.to_string()),
        Some(Value::String(s)) => table
            .iter()
            .find(|(f, p)| f.eq_ignore_ascii_case(s) || p.eq_ignore_ascii_case(s))
            .map(|(_, p)| (*p).to_string())
            .ok_or_else(|| {
                format!(
                    "{what}.{key} must be one of {}",
                    table.iter().map(|(f, _)| *f).collect::<Vec<_>>().join(", ")
                )
            }),
        Some(_) => Err(format!("{what}.{key} must be a string")),
    }
}

fn parse_icons(v: &Value, ctx: &Ctx<'_>, what: &str) -> Result<Rule, String> {
    let o = obj(v, what)?;
    check_keys(
        o,
        &[
            "field",
            "cases",
            "preset",
            "thresholds",
            "else",
            "position",
            "align",
        ],
        what,
    )?;
    let field = rule_field(o, ctx, true, what)?;
    let cases = match (o.get("preset"), o.get("cases")) {
        (Some(_), Some(_)) => return Err(format!("{what}: give cases or preset, not both")),
        (Some(p), None) => {
            let name = p.as_str().unwrap_or_default();
            let (_, high, mid, low) = ICON_PRESETS.iter().find(|(n, ..)| *n == name).ok_or_else(|| {
                format!(
                    "{what}.preset must be one of {}",
                    ICON_PRESETS
                        .iter()
                        .map(|(n, ..)| *n)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?;
            let t = o
                .get("thresholds")
                .and_then(Value::as_array)
                .filter(|a| a.len() == 2)
                .ok_or_else(|| {
                    format!("{what}.thresholds must be [low, high]: the bounds between the three icons")
                })?;
            let lo = number(
                t.first().unwrap_or(&Value::Null),
                &format!("{what}.thresholds[0]"),
            )?;
            let hi = number(t.get(1).unwrap_or(&Value::Null), &format!("{what}.thresholds[1]"))?;
            if lo >= hi {
                return Err(format!("{what}.thresholds must be increasing"));
            }
            let kind = value_kind(ctx.model, &field);
            let l = |n: f64| to_lit(&json!(n), kind, what);
            vec![
                Case {
                    conds: vec![(2, l(hi)?)],
                    value: (*high).to_string(),
                    percent: false,
                },
                Case {
                    conds: vec![(2, l(lo)?), (3, l(hi)?)],
                    value: (*mid).to_string(),
                    percent: false,
                },
                Case {
                    conds: vec![(3, l(lo)?)],
                    value: (*low).to_string(),
                    percent: false,
                },
            ]
        }
        (None, _) => {
            if o.contains_key("thresholds") {
                return Err(format!("{what}.thresholds goes with preset"));
            }
            parse_cases(o, &field, ctx, "icon", what)?
        }
    };
    let default = match o.get("else") {
        None | Some(Value::Null) => None,
        x => Some(icon_name(x, &format!("{what}.else"))?),
    };
    Ok(Rule::Icons {
        field,
        cases,
        default,
        layout: enum_param(o, "position", POSITIONS, "Before", what)?,
        align: enum_param(o, "align", ALIGNS, "Top", what)?,
    })
}

fn parse_bars(v: &Value, what: &str) -> Result<Rule, String> {
    let none = Map::new();
    let o = match v {
        Value::Bool(true) => &none,
        _ => obj(v, what)?,
    };
    check_keys(
        o,
        &[
            "positive",
            "negative",
            "axis",
            "reverse",
            "show_bar_only",
            "min",
            "max",
        ],
        what,
    )?;
    let c = |k: &str, d: &str| match o.get(k) {
        None | Some(Value::Null) => Ok(d.to_string()),
        x => color_str(x, &format!("{what}.{k}")),
    };
    let b = |k: &str| match o.get(k) {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(format!("{what}.{k} must be true or false")),
    };
    let n = |k: &str| match o.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(x) => number(x, &format!("{what}.{k}")).map(Some),
    };
    let (min, max) = (n("min")?, n("max")?);
    if let (Some(a), Some(z)) = (min, max)
        && a >= z
    {
        return Err(format!("{what}: min must be below max"));
    }
    Ok(Rule::DataBars {
        positive: c("positive", DEFAULT_POSITIVE)?,
        negative: c("negative", DEFAULT_NEGATIVE)?,
        axis: c("axis", DEFAULT_AXIS)?,
        reverse: b("reverse")?,
        hide_text: b("show_bar_only")?,
        min,
        max,
    })
}

/// The rule and, for field values, the kind it forces (`as`).
fn parse_rule(v: &Value, ctx: &Ctx<'_>, what: &str) -> Result<(Rule, Option<PropKind>), String> {
    let o = obj(v, what)?;
    if o.len() != 1 {
        return Err(format!(
            "{what} must have exactly one of gradient, rules, field_value, icons, data_bars, web_url"
        ));
    }
    let (k, x) = o.iter().next().ok_or("internal: empty rule")?;
    let w = format!("{what}.{k}");
    Ok(match k.as_str() {
        "gradient" => (parse_gradient(x, ctx, &w)?, None),
        "rules" => (parse_rules(x, ctx, &w)?, None),
        "icons" => (parse_icons(x, ctx, &w)?, None),
        "data_bars" => (parse_bars(x, &w)?, None),
        "field_value" | "web_url" => {
            let fo = obj(x, &w)?;
            check_keys(
                fo,
                if k == "field_value" {
                    &["field", "as"]
                } else {
                    &["field"]
                },
                &w,
            )?;
            let field = rule_field(fo, ctx, false, &w)?;
            let forced = match fo.get("as") {
                None | Some(Value::Null) => None,
                Some(Value::String(s)) if s == "color" => Some(PropKind::Color),
                Some(Value::String(s)) if s == "value" => Some(PropKind::Plain),
                Some(_) => return Err(format!("{w}.as must be \"color\" or \"value\"")),
            };
            if k == "web_url" {
                (Rule::WebUrl { field }, None)
            } else {
                (Rule::FieldValue { field }, forced)
            }
        }
        _ => {
            return Err(format!(
                "{what}: `{k}` is not gradient, rules, field_value, icons, data_bars or web_url"
            ));
        }
    })
}

/// Whether `rule` can go on a property of `kind`.
fn fits(rule: &Rule, kind: PropKind) -> bool {
    match rule {
        Rule::Gradient { .. } | Rule::Rules { .. } => kind == PropKind::Color,
        Rule::FieldValue { .. } => matches!(kind, PropKind::Color | PropKind::Plain),
        Rule::WebUrl { .. } => kind == PropKind::Plain,
        Rule::Icons { .. } => kind == PropKind::Icon,
        Rule::DataBars { .. } => kind == PropKind::Bars,
    }
}

fn kind_text(kind: PropKind) -> &'static str {
    match kind {
        PropKind::Color => "a color",
        PropKind::Icon => "an icon",
        PropKind::Bars => "data bars",
        PropKind::Plain => "a value",
    }
}

// ---------------------------------------------------------------------------------------------
// Items
// ---------------------------------------------------------------------------------------------

fn wildcard(m: u64, query_ref: Option<&str>) -> Value {
    let mut s = Map::new();
    s.insert(
        "data".into(),
        json!([{"dataViewWildcard": {"matchingOption": m}}]),
    );
    if let Some(r) = query_ref {
        s.insert("metadata".into(), json!(r));
    }
    Value::Object(s)
}

/// (object, property, metadata, matchingOption) of a parsed item, to catch duplicates.
type ItemKey = (String, String, Option<String>, Option<u64>);

/// The edits `conditional` asks for: in `objects`, and in `visualContainerObjects`.
#[derive(Debug, Default)]
pub(crate) struct Ops {
    pub objects: Vec<EntryOp>,
    pub container: Vec<EntryOp>,
}

/// A container target's item (`title`, `subTitle`, `background`, `border`), or a [`WHOLE`] one
/// (in `objects`): one value for the whole visual, in the object's entry without selector (as Desktop writes a title from a field,
/// Microsoft Learn "Expression-based titles"), so no series and no `apply_to`.
fn container_item(
    o: &Map<String, Value>,
    what: &str,
    target: Target,
    rule: Option<&Rule>,
    series: Option<String>,
    ops: &mut Ops,
) -> Result<ItemKey, String> {
    let name = target
        .alias
        .map_or_else(|| format!("{}.{}", target.object, target.prop), str::to_string);
    if series.is_some() || o.get("occurrence").is_some_and(|v| !v.is_null()) {
        return Err(format!(
            "{what}: {name} is one value for the whole visual; it takes no series"
        ));
    }
    if o.get("apply_to").is_some_and(|v| !v.is_null()) {
        return Err(format!("{what}.apply_to does not apply to {name}"));
    }
    if matches!(rule, Some(Rule::DataBars { .. } | Rule::Icons { .. })) {
        return Err(format!("{what}: {name} takes gradient, rules or field_value"));
    }
    let selectors = vec![Value::Null];
    let list = if target.whole {
        &mut ops.objects
    } else {
        &mut ops.container
    };
    match rule {
        None => list.push(EntryOp::Remove {
            object: target.object.clone(),
            selectors,
            prop: Some(target.prop.clone()),
            label: format!("conditional {name} removed"),
        }),
        Some(r) => list.push(EntryOp::Set {
            object: target.object.clone(),
            selectors,
            prop: target.prop.clone(),
            value: render(r, target.kind)?,
            label: format!("conditional {name} = {}", r.kind_name()),
            check: Check::None,
        }),
    }
    let place = if target.whole { "whole" } else { "container" };
    Ok((format!("{place}.{}", target.object), target.prop, None, None))
}

fn parse_item(item: &Value, i: usize, ctx: &Ctx<'_>, ops: &mut Ops) -> Result<ItemKey, String> {
    let what = format!("conditional[{i}]");
    let o = obj(item, &what)?;
    check_keys(o, &["target", "series", "occurrence", "apply_to", "rule"], &what)?;
    let rule_v = o
        .get("rule")
        .ok_or_else(|| format!("{what}.rule is missing (null removes the conditional format)"))?;
    let rule = match rule_v {
        Value::Null => None,
        r => Some(parse_rule(r, ctx, &format!("{what}.rule"))?),
    };
    let rule_kind = rule.as_ref().map_or("", |(r, _)| r.kind_name());
    let (mut target, target_series) = parse_target(o.get("target"), rule_kind, ctx, &what)?;
    let series_text = match (o.get("series"), target_series) {
        (Some(Value::String(_)), Some(_)) => {
            return Err(format!("{what}: give series once (in the item or in target)"));
        }
        (Some(Value::String(s)), None) => Some(s.clone()),
        (None | Some(Value::Null), t) => t,
        (Some(_), _) => return Err(format!("{what}.series must be a field string like [Total]")),
    };
    if let Some((r, Some(forced))) = &rule {
        if target.alias.is_some() && *forced != target.kind {
            return Err(format!(
                "{what}: target {} holds {}",
                target.alias.unwrap_or_default(),
                kind_text(target.kind)
            ));
        }
        if matches!(r, Rule::FieldValue { .. }) {
            target.kind = *forced;
        }
    }
    if let Some((r, _)) = &rule
        && !fits(r, target.kind)
    {
        return Err(format!(
            "{what}: a {} rule cannot go on {}.{} ({})",
            r.kind_name(),
            target.object,
            target.prop,
            kind_text(target.kind)
        ));
    }
    if target.container || target.whole {
        return container_item(o, &what, target, rule.as_ref().map(|(r, _)| r), series_text, ops);
    }
    let fam = family(ctx.visual_type.as_deref());
    let occurrence = match o.get("occurrence") {
        None | Some(Value::Null) => 1,
        Some(v) => match v.as_u64() {
            Some(n) if (1..=MAX_ITEMS as u64).contains(&n) => n as usize,
            _ => return Err(format!("{what}.occurrence must be a whole number from 1")),
        },
    };
    if occurrence > 1 && series_text.is_none() {
        return Err(format!("{what}.occurrence needs series"));
    }
    // The formatted series.
    let series = match series_text {
        Some(s) => {
            let (f, first) = ctx.bound(&s).map_err(|e| format!("{what}.series: {e}"))?;
            if occurrence == 1 {
                Some((f, first))
            } else {
                let r = ctx
                    .projections
                    .iter()
                    .filter(|(p, _)| same_field(p, &f))
                    .nth(occurrence - 1)
                    .map(|(_, r)| r.clone())
                    .ok_or_else(|| format!("{what}.occurrence: {s} is not bound {occurrence} times"))?;
                Some((f, r))
            }
        }
        None => {
            let from_rule = rule
                .as_ref()
                .and_then(|(r, _)| r.field())
                .and_then(|f| ctx.query_ref(f).map(|q| (f.clone(), q.to_string())));
            if fam == Family::Table || target.kind == PropKind::Bars {
                Some(from_rule.ok_or_else(|| {
                    format!("{what}.series is missing: name the column to format, like \"[Total]\" or \"Product[Name]\"")
                })?)
            } else {
                None
            }
        }
    };
    let query_ref = series.as_ref().map(|(_, r)| r.clone());
    let matching = match o.get("apply_to") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(
            APPLY_TO
                .iter()
                .find(|(n, _)| n == s)
                .map(|(_, m)| *m)
                .ok_or_else(|| format!("{what}.apply_to must be values, values_and_totals or totals"))?,
        ),
        Some(_) => return Err(format!("{what}.apply_to must be a string")),
    };
    let selectors = if target.kind == PropKind::Bars {
        if matching.is_some() {
            return Err(format!("{what}.apply_to does not apply to data bars"));
        }
        let r = query_ref.as_deref().ok_or("internal: data bars without series")?;
        vec![json!({ "metadata": r })]
    } else {
        let mut s = vec![wildcard(matching.unwrap_or(1), query_ref.as_deref())];
        if matching.is_none()
            && let Some(r) = &query_ref
        {
            // Older reports scope conditional formats by metadata alone.
            s.push(json!({ "metadata": r }));
        }
        s
    };
    let name = target
        .alias
        .map_or_else(|| format!("{}.{}", target.object, target.prop), str::to_string);
    let on = series
        .as_ref()
        .map_or_else(String::new, |(f, _)| format!(" of {}", field_text(f)));
    let check = if target.alias.is_some() || target.kind == PropKind::Bars {
        Check::None
    } else {
        Check::Conditional
    };
    match &rule {
        None => ops.objects.push(EntryOp::Remove {
            object: target.object.clone(),
            selectors,
            prop: Some(target.prop.clone()),
            label: format!("conditional {name}{on} removed"),
        }),
        Some((r, _)) => ops.objects.push(EntryOp::Set {
            object: target.object.clone(),
            selectors,
            prop: target.prop.clone(),
            value: render(r, target.kind)?,
            label: format!("conditional {name}{on} = {}", r.kind_name()),
            check,
        }),
    }
    Ok((target.object, target.prop, query_ref, matching))
}

/// The edits the `conditional` key of `props` asks for (empty when it is not there). Validates
/// everything; nothing is written.
pub(crate) fn parse(
    props: &Map<String, Value>,
    visual: &Visual,
    model: Option<&SemanticModel>,
) -> Result<Ops, String> {
    let mut ops = Ops::default();
    let items = match props.get("conditional") {
        None | Some(Value::Null) => return Ok(ops),
        Some(Value::Array(a)) => a,
        Some(_) => return Err("`conditional` must be a list: [{target?, series?, apply_to?, rule}]".into()),
    };
    if items.len() > MAX_ITEMS {
        return Err(format!("`conditional` has more than {MAX_ITEMS} items"));
    }
    let ctx = Ctx::new(visual, model);
    let mut keys: Vec<ItemKey> = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let key = parse_item(item, i, &ctx, &mut ops)?;
        if keys.contains(&key) {
            return Err(format!(
                "conditional[{i}] formats {}.{} of the same series again",
                key.0, key.1
            ));
        }
        keys.push(key);
    }
    Ok(ops)
}

// ---------------------------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------------------------

/// A field of a rule: a measure or an aggregation.
fn decode_field(v: &Value) -> Option<FieldRef> {
    match field_from_json(v, &Vec::new(), None)? {
        f @ (FieldRef::Measure { .. } | FieldRef::Aggregation { .. }) => Some(f),
        _ => None,
    }
}

fn exactly<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    let o = v.as_object()?;
    if o.len() != 1 {
        return None;
    }
    o.get(key)
}

/// `{"Literal": {"Value": s}}` exactly.
fn decode_lit_expr(v: &Value) -> Option<Lit> {
    let l = exactly(exactly(v, "Literal")?, "Value")?.as_str()?;
    match parse_lit(l) {
        Lit::Other(_) => None,
        x => Some(x),
    }
}

fn decode_text_lit(v: &Value) -> Option<String> {
    match decode_lit_expr(v)? {
        Lit::Text(s) => Some(s),
        _ => None,
    }
}

fn decode_color_lit(v: &Value) -> Option<String> {
    decode_text_lit(v).filter(|c| is_color(c))
}

/// The percent `p` of a [`percent_bound`] of `field`, exactly that shape.
fn decode_percent_bound(v: &Value, field: &FieldRef) -> Option<f64> {
    let arith = |v: &Value, op: u64| -> Option<(Value, Value)> {
        let a = exactly(v, "Arithmetic")?.as_object()?;
        if a.len() != 3 || a.get("Operator")?.as_u64()? != op {
            return None;
        }
        Some((a.get("Left")?.clone(), a.get("Right")?.clone()))
    };
    let agg = |v: &Value, f: u64| -> Option<()> {
        let a = exactly(v, "Aggregation")?.as_object()?;
        if a.len() != 2 || a.get("Function")?.as_u64()? != f {
            return None;
        }
        same_field(&decode_field(a.get("Expression")?)?, field).then_some(())
    };
    let (min, scaled) = arith(v, 0)?;
    agg(&min, 3)?;
    let (range, frac) = arith(&scaled, 2)?;
    let (max, min2) = arith(&range, 1)?;
    agg(&max, 4)?;
    agg(&min2, 3)?;
    let Lit::Num(f, _) = decode_lit_expr(&frac)? else {
        return None;
    };
    let p = (f * 100.0 * 1e9).round() / 1e9;
    (0.0..=100.0).contains(&p).then_some(p)
}

/// A comparison of `field` with a literal, or with a Percent bound (`true`).
fn decode_comparison(v: &Value, field: &FieldRef) -> Option<(u64, Lit, bool)> {
    let c = exactly(v, "Comparison")?.as_object()?;
    if c.len() != 3 {
        return None;
    }
    let k = c.get("ComparisonKind")?.as_u64().filter(|k| *k <= 4)?;
    if !same_field(&decode_field(c.get("Left")?)?, field) {
        return None;
    }
    let right = c.get("Right")?;
    if let Some(p) = decode_percent_bound(right, field) {
        return Some((k, Lit::Num(p, 'D'), true));
    }
    Some((k, decode_lit_expr(right)?, false))
}

/// The field and cases of a `Conditional` expression whose values are colors (`icons: false`)
/// or icon names.
fn decode_conditional(v: &Value, icons: bool) -> Option<(FieldRef, Vec<Case>, Option<String>)> {
    let c = exactly(v, "Conditional")?.as_object()?;
    if c.keys().any(|k| k != "Cases" && k != "DefaultValue") {
        return None;
    }
    let value_of = |x: &Value| -> Option<String> {
        if icons {
            decode_text_lit(x).filter(|s| icon_name(Some(&json!(s)), "").is_ok())
        } else {
            decode_color_lit(x)
        }
    };
    let list = c.get("Cases")?.as_array()?;
    if list.is_empty() || list.len() > MAX_CASES {
        return None;
    }
    // The field: the Left of the first comparison.
    let first = list.first()?.get("Condition")?;
    let first_cmp = match exactly(first, "And") {
        Some(a) => a.get("Left")?,
        None => first,
    };
    let field = decode_field(exactly(first_cmp, "Comparison")?.get("Left")?)?;
    let mut cases = Vec::new();
    for case in list {
        let o = case.as_object()?;
        if o.len() != 2 {
            return None;
        }
        let cond = o.get("Condition")?;
        let conds = match exactly(cond, "And") {
            Some(a) => {
                let a = a.as_object()?;
                if a.len() != 2 {
                    return None;
                }
                let x = decode_comparison(a.get("Left")?, &field)?;
                let y = decode_comparison(a.get("Right")?, &field)?;
                if x.0 == y.0 || x.1 == Lit::Null || y.1 == Lit::Null || x.2 != y.2 {
                    return None;
                }
                vec![x, y]
            }
            None => {
                let x = decode_comparison(cond, &field)?;
                if x.1 == Lit::Null && x.0 != 0 {
                    return None;
                }
                vec![x]
            }
        };
        let percent = conds.first().is_some_and(|c| c.2);
        cases.push(Case {
            conds: conds.into_iter().map(|(k, l, _)| (k, l)).collect(),
            value: value_of(o.get("Value")?)?,
            percent,
        });
    }
    let default = match c.get("DefaultValue") {
        None => None,
        Some(d) => Some(value_of(d)?),
    };
    Some((field, cases, default))
}

fn decode_number_lit(v: &Value) -> Option<f64> {
    match decode_literal(v)? {
        Lit::Num(n, _) => Some(n),
        _ => None,
    }
}

fn decode_bool_lit(v: &Value) -> Option<bool> {
    match decode_literal(v)? {
        Lit::Bool(b) => Some(b),
        _ => None,
    }
}

fn decode_gradient(v: &Value) -> Option<Rule> {
    let f = exactly(v, "FillRule")?.as_object()?;
    if f.len() != 2 {
        return None;
    }
    let field = decode_field(f.get("Input")?)?;
    let inner = f.get("FillRule")?.as_object()?;
    if inner.len() != 1 {
        return None;
    }
    let (name, g) = inner.iter().next()?;
    let names: &[&'static str] = match name.as_str() {
        "linearGradient2" => &["min", "max"],
        "linearGradient3" => &["min", "mid", "max"],
        _ => return None,
    };
    let g = g.as_object()?;
    if g.keys()
        .any(|k| !names.contains(&k.as_str()) && k != "nullColoringStrategy")
    {
        return None;
    }
    let mut stops = Vec::new();
    for n in names {
        let s = g.get(*n)?.as_object()?;
        if s.keys().any(|k| k != "color" && k != "value") {
            return None;
        }
        let color = decode_color_lit(s.get("color")?)?;
        let value = match s.get("value") {
            None => None,
            Some(x) => match decode_lit_expr(x)? {
                Lit::Num(n, _) => Some(n),
                _ => return None,
            },
        };
        stops.push((*n, color, value));
    }
    let empty = match g.get("nullColoringStrategy") {
        None => None,
        Some(s) => {
            let s = s.as_object()?;
            let strategy = decode_text_lit(s.get("strategy")?)?;
            match (strategy.as_str(), s.len()) {
                ("asZero", 1) => Some(Empty::Zero),
                ("noColor", 1) => Some(Empty::NoColor),
                ("specificColor", 2) => Some(Empty::Color(decode_color_lit(s.get("color")?)?)),
                _ => return None,
            }
        }
    };
    Some(Rule::Gradient { field, stops, empty })
}

fn decode_bars(v: &Value) -> Option<Rule> {
    let o = v.as_object()?;
    const KEYS: &[&str] = &[
        "minValue",
        "maxValue",
        "positiveColor",
        "negativeColor",
        "axisColor",
        "reverseDirection",
        "hideText",
    ];
    if o.keys().any(|k| !KEYS.contains(&k.as_str())) {
        return None;
    }
    let color = |k: &str| color_text(o.get(k)?).filter(|c| is_color(c));
    let num = |k: &str| -> Option<Option<f64>> {
        match o.get(k) {
            None => Some(None),
            Some(x) => decode_number_lit(x).map(Some),
        }
    };
    Some(Rule::DataBars {
        positive: color("positiveColor")?,
        negative: color("negativeColor")?,
        axis: color("axisColor")?,
        reverse: decode_bool_lit(o.get("reverseDirection")?)?,
        hide_text: decode_bool_lit(o.get("hideText")?)?,
        min: num("minValue")?,
        max: num("maxValue")?,
    })
}

/// The rule a PBIR property value holds, and the kind of property it was written for.
fn decode_value(prop: &str, v: &Value) -> Option<(Rule, PropKind)> {
    if prop == "dataBars" {
        return decode_bars(v).map(|r| (r, PropKind::Bars));
    }
    let o = v.as_object()?;
    if o.get("kind").and_then(Value::as_str) == Some("Icon") {
        if o.len() != 4 {
            return None;
        }
        let (field, cases, default) = decode_conditional(exactly(o.get("value")?, "expr")?, true)?;
        let layout = decode_text_lit(exactly(o.get("layout")?, "expr")?)?;
        let align = decode_text_lit(exactly(o.get("verticalAlignment")?, "expr")?)?;
        if !POSITIONS.iter().any(|(_, p)| *p == layout) || !ALIGNS.iter().any(|(_, p)| *p == align) {
            return None;
        }
        return Some((
            Rule::Icons {
                field,
                cases,
                default,
                layout,
                align,
            },
            PropKind::Icon,
        ));
    }
    if let Some(expr) = exactly(exactly(exactly(v, "solid")?, "color")?, "expr") {
        if let Some(r) = decode_gradient(expr) {
            return Some((r, PropKind::Color));
        }
        if let Some((field, cases, default)) = decode_conditional(expr, false) {
            return Some((
                Rule::Rules {
                    field,
                    cases,
                    default,
                },
                PropKind::Color,
            ));
        }
        let field = decode_field(expr)?;
        return Some((Rule::FieldValue { field }, PropKind::Color));
    }
    None
}

/// Like [`decode_value`], plus plain field values (`{"expr": field}`).
fn decode_any(prop: &str, v: &Value) -> Option<(Rule, PropKind)> {
    if let Some(x) = decode_value(prop, v) {
        return Some(x);
    }
    let field = decode_field(exactly(v, "expr")?)?;
    let rule = if prop == "webURL" {
        Rule::WebUrl { field }
    } else {
        Rule::FieldValue { field }
    };
    Some((rule, PropKind::Plain))
}

/// Whether `value` (of `object.prop`) is a conditional format [`extract`] returns, so that series
/// extraction leaves it out.
pub(crate) fn is_conditional(_object: &str, prop: &str, value: &Value) -> bool {
    decode_any(prop, value).is_some()
}

// ---------------------------------------------------------------------------------------------
// Extraction
// ---------------------------------------------------------------------------------------------

fn lit_json(l: &Lit) -> Value {
    l.to_json()
}

fn case_json(c: &Case, value_key: &str) -> Value {
    let mut o = Map::new();
    let sym = |k: u64| OPS.iter().find(|(_, x, _)| *x == k);
    match c.conds.as_slice() {
        [(k, l)] => {
            if let Some((op, ..)) = sym(*k) {
                o.insert("op".into(), json!(op));
                o.insert("value".into(), lit_json(l));
            }
        }
        [(2, a), (4, b)] => {
            o.insert("op".into(), json!("between"));
            o.insert("values".into(), json!([lit_json(a), lit_json(b)]));
        }
        conds => {
            for (k, l) in conds {
                if let Some((.., key)) = sym(*k) {
                    o.insert((*key).to_string(), lit_json(l));
                }
            }
        }
    }
    if c.percent {
        o.insert("percent".into(), json!(true));
    }
    o.insert(value_key.into(), json!(c.value));
    Value::Object(o)
}

fn rule_json(rule: &Rule, kind: PropKind, guessed: PropKind) -> Value {
    match rule {
        Rule::Gradient { field, stops, empty } => {
            let mut g = Map::new();
            g.insert("field".into(), json!(field_text(field)));
            for (name, c, v) in stops {
                let s = match v {
                    None => json!(c),
                    Some(n) => json!({"color": c, "value": crate::visual_props::num_json(*n)}),
                };
                g.insert((*name).to_string(), s);
            }
            match empty {
                None => {}
                Some(Empty::Zero) => {
                    g.insert("empty".into(), json!("zero"));
                }
                Some(Empty::NoColor) => {
                    g.insert("empty".into(), json!("none"));
                }
                Some(Empty::Color(c)) => {
                    g.insert("empty".into(), json!(c));
                }
            }
            json!({ "gradient": g })
        }
        Rule::Rules {
            field,
            cases,
            default,
        } => {
            let mut r = Map::new();
            r.insert("field".into(), json!(field_text(field)));
            r.insert(
                "cases".into(),
                Value::Array(cases.iter().map(|c| case_json(c, "color")).collect()),
            );
            if let Some(d) = default {
                r.insert("else".into(), json!(d));
            }
            json!({ "rules": r })
        }
        Rule::FieldValue { field } => {
            let mut r = Map::new();
            r.insert("field".into(), json!(field_text(field)));
            if kind != guessed {
                r.insert(
                    "as".into(),
                    json!(if kind == PropKind::Color { "color" } else { "value" }),
                );
            }
            json!({ "field_value": r })
        }
        Rule::WebUrl { field } => json!({"web_url": {"field": field_text(field)}}),
        Rule::Icons {
            field,
            cases,
            default,
            layout,
            align,
        } => {
            let mut r = Map::new();
            r.insert("field".into(), json!(field_text(field)));
            r.insert(
                "cases".into(),
                Value::Array(cases.iter().map(|c| case_json(c, "icon")).collect()),
            );
            if let Some(d) = default {
                r.insert("else".into(), json!(d));
            }
            let friendly = |t: &[(&str, &str)], p: &str| {
                t.iter()
                    .find(|(_, x)| *x == p)
                    .map_or(p.to_string(), |(f, _)| (*f).to_string())
            };
            r.insert("position".into(), json!(friendly(POSITIONS, layout)));
            r.insert("align".into(), json!(friendly(ALIGNS, align)));
            json!({ "icons": r })
        }
        Rule::DataBars {
            positive,
            negative,
            axis,
            reverse,
            hide_text,
            min,
            max,
        } => {
            let mut r = Map::new();
            r.insert("positive".into(), json!(positive));
            r.insert("negative".into(), json!(negative));
            r.insert("axis".into(), json!(axis));
            r.insert("reverse".into(), json!(reverse));
            r.insert("show_bar_only".into(), json!(hide_text));
            if let Some(n) = min {
                r.insert("min".into(), crate::visual_props::num_json(*n));
            }
            if let Some(n) = max {
                r.insert("max".into(), crate::visual_props::num_json(*n));
            }
            json!({ "data_bars": r })
        }
    }
}

/// The series metadata and matching option of a conditional-format selector:
/// `{metadata}` or `{data: [{dataViewWildcard}], metadata?}`.
fn decode_cf_selector(sel: &Value) -> Option<(Option<&str>, Option<u64>)> {
    let o = sel.as_object()?;
    let metadata = match o.get("metadata") {
        None => None,
        Some(m) => Some(m.as_str()?),
    };
    match o.get("data") {
        None => {
            if o.len() != 1 {
                return None;
            }
            Some((metadata, None))
        }
        Some(d) => {
            if o.len() != 1 + usize::from(metadata.is_some()) {
                return None;
            }
            let d = d.as_array()?;
            if d.len() != 1 {
                return None;
            }
            let w = exactly(d.first()?, "dataViewWildcard")?;
            let m = exactly(w, "matchingOption")?.as_u64().filter(|m| *m <= 2)?;
            Some((metadata, Some(m)))
        }
    }
}

/// The bound field whose `queryRef` is `r`, and which of its bindings that is (1: the first).
fn field_of_any_ref<'c>(ctx: &'c Ctx<'_>, r: &str) -> Option<(&'c FieldRef, usize)> {
    let i = ctx.projections.iter().position(|(_, q)| q == r)?;
    let (f, _) = ctx.projections.get(i)?;
    if matches!(f, FieldRef::Other { .. }) {
        return None;
    }
    let before = ctx
        .projections
        .iter()
        .take(i)
        .filter(|(p, _)| same_field(p, f))
        .count();
    Some((f, before.saturating_add(1)))
}

/// `v` with every `{"SelectRef": {"ExpressionName": r}}` (Desktop's way to name a rule input
/// by the queryRef of a bound field) replaced by that field's expression; `None` when there is
/// none (or one names no bound field, or `v` nests deeper than 32 levels).
fn resolve_select_refs(v: &Value, ctx: &Ctx<'_>, depth: usize) -> Option<Value> {
    if depth > 32 {
        return None;
    }
    match v {
        Value::Object(o) => {
            if o.len() == 1
                && let Some(r) = o
                    .get("SelectRef")
                    .and_then(Value::as_object)
                    .filter(|s| s.len() == 1)
                    .and_then(|s| s.get("ExpressionName"))
                    .and_then(Value::as_str)
            {
                let (f, _) = field_of_any_ref(ctx, r)?;
                return field_to_json(f, None);
            }
            let mut out = Map::new();
            for (k, x) in o {
                out.insert(k.clone(), resolve_select_refs(x, ctx, depth.saturating_add(1))?);
            }
            Some(Value::Object(out))
        }
        Value::Array(a) => a
            .iter()
            .map(|x| resolve_select_refs(x, ctx, depth.saturating_add(1)))
            .collect::<Option<Vec<_>>>()
            .map(Value::Array),
        other => Some(other.clone()),
    }
}

/// `v` with the rule inputs of its `objects` that name a bound field by `SelectRef` (as
/// Desktop 2.158 writes a matrix's color scale) replaced by that field's expression, so that
/// [`extract`] reads them; `None` when there is nothing to replace. For evaluating the rules
/// only: the visual itself keeps Desktop's form.
pub(crate) fn with_resolved_inputs(v: &Visual) -> Option<Visual> {
    let objects = v.visual_extra.get("objects")?;
    if !objects.to_string().contains("\"SelectRef\"") {
        return None;
    }
    let ctx = Ctx::new(v, None);
    let mut out = Map::new();
    for (object, list) in objects.as_object()? {
        let mut entries = Vec::new();
        for e in list.as_array().into_iter().flatten() {
            entries.push(resolve_select_refs(e, &ctx, 0).unwrap_or_else(|| e.clone()));
        }
        out.insert(object.clone(), Value::Array(entries));
    }
    let mut copy = v.clone();
    copy.visual_extra.insert("objects".into(), Value::Object(out));
    Some(copy)
}

/// Adds `conditional` to `out`: every conditional format bicraft can express.
pub(crate) fn extract(v: &Visual, out: &mut Map<String, Value>) {
    let mut list = Vec::new();
    extract_objects(v, &mut list);
    extract_whole(v, &mut list);
    extract_container(v, &mut list);
    if !list.is_empty() {
        out.insert("conditional".into(), Value::Array(list));
    }
}

/// The conditional formats of the container's objects (`visualContainerObjects`, their entry
/// without selector), as [`container_item`] writes them.
fn extract_container(v: &Visual, list: &mut Vec<Value>) {
    let Some(objects) = v
        .visual_extra
        .get("visualContainerObjects")
        .and_then(Value::as_object)
    else {
        return;
    };
    let vt = v.visual_type.as_deref();
    let fam = family(vt);
    for (object, arr) in objects {
        let Some(entry) = arr
            .as_array()
            .into_iter()
            .flatten()
            .find(|e| e.get("selector").is_none_or(Value::is_null))
        else {
            continue;
        };
        // what [`parse_target`] writes here: the aliased objects, and any container object the
        // type has no visual object of that name
        let aliased = CONTAINER_ALIASED.contains(&object.as_str());
        if !is_container(vt, object, aliased) {
            continue;
        }
        let alias = |prop: &str| {
            if aliased {
                alias_of(object, prop, fam)
            } else {
                None
            }
        };
        let Some(props) = entry.get("properties").and_then(Value::as_object) else {
            continue;
        };
        for (prop, val) in props {
            let Some((rule, kind)) = decode_any(prop, val) else {
                continue;
            };
            if matches!(
                rule,
                Rule::DataBars { .. } | Rule::Icons { .. } | Rule::WebUrl { .. }
            ) {
                continue;
            }
            let a = alias(prop);
            let guessed = ALIASES
                .iter()
                .find(|(n, ..)| Some(*n) == a)
                .map_or_else(|| guess_kind(vt, object, prop), |(.., k)| *k);
            if !fits(&rule, kind) || (a.is_some() && kind != guessed) {
                continue;
            }
            let mut item = Map::new();
            item.insert(
                "target".into(),
                a.map_or_else(|| json!({"object": object, "property": prop}), |a| json!(a)),
            );
            item.insert("rule".into(), rule_json(&rule, kind, guessed));
            list.push(Value::Object(item));
        }
    }
}

/// The conditional formats of the [`WHOLE`] properties (their object's entry without selector in
/// `objects`), as [`container_item`] writes them.
fn extract_whole(v: &Visual, list: &mut Vec<Value>) {
    let Some(objects) = v.visual_extra.get("objects").and_then(Value::as_object) else {
        return;
    };
    let vt = v.visual_type.as_deref();
    let fam = family(vt);
    for (object, arr) in objects {
        let Some(props) = arr
            .as_array()
            .into_iter()
            .flatten()
            .find(|e| e.get("selector").is_none_or(Value::is_null))
            .and_then(|e| e.get("properties"))
            .and_then(Value::as_object)
        else {
            continue;
        };
        for (prop, val) in props {
            if !is_whole(vt, object, prop) {
                continue;
            }
            let Some((rule, kind)) = decode_any(prop, val) else {
                continue;
            };
            if kind != PropKind::Color || !fits(&rule, kind) {
                continue;
            }
            let mut item = Map::new();
            item.insert(
                "target".into(),
                alias_of(object, prop, fam)
                    .map_or_else(|| json!({"object": object, "property": prop}), |a| json!(a)),
            );
            item.insert("rule".into(), rule_json(&rule, kind, PropKind::Color));
            list.push(Value::Object(item));
        }
    }
}

fn extract_objects(v: &Visual, list: &mut Vec<Value>) {
    let Some(objects) = v.visual_extra.get("objects").and_then(Value::as_object) else {
        return;
    };
    let ctx = Ctx::new(v, None);
    let vt = v.visual_type.as_deref();
    let fam = family(vt);
    let mut seen: Vec<ItemKey> = Vec::new();
    for (object, arr) in objects {
        for entry in arr.as_array().into_iter().flatten() {
            let Some(sel) = entry.get("selector") else {
                continue;
            };
            let Some((metadata, matching)) = decode_cf_selector(sel) else {
                continue;
            };
            let (series, occurrence) = match metadata {
                None => (None, 1),
                Some(r) => match field_of_any_ref(&ctx, r) {
                    Some((f, n)) => (Some(f.clone()), n),
                    None => continue,
                },
            };
            let Some(props) = entry.get("properties").and_then(Value::as_object) else {
                continue;
            };
            for (prop, val) in props {
                let Some((rule, kind)) = decode_any(prop, val) else {
                    continue;
                };
                // Selector shapes apply writes: data bars by metadata alone; the rest with a
                // wildcard, or (older reports) by metadata alone.
                let bars = kind == PropKind::Bars;
                if bars && (matching.is_some() || series.is_none()) {
                    continue;
                }
                if !bars && matching.is_none() && series.is_none() {
                    continue;
                }
                // Tables need a series.
                if fam == Family::Table && series.is_none() {
                    continue;
                }
                let alias = alias_of(object, prop, fam).filter(|a| !container_alias(a));
                let guessed = ALIASES
                    .iter()
                    .find(|(_, f, o, p, _)| f.has(fam) && o == object && p == prop)
                    .map_or_else(|| guess_kind(vt, object, prop), |(.., k)| *k);
                if !fits(&rule, kind) || (alias.is_some() && kind != guessed) {
                    continue;
                }
                if matches!(rule, Rule::WebUrl { .. }) && kind != guessed {
                    continue;
                }
                let key: ItemKey = (
                    object.clone(),
                    prop.clone(),
                    metadata.map(str::to_string),
                    matching.filter(|m| *m != 1),
                );
                if seen.contains(&key) {
                    continue;
                }
                seen.push(key);
                let mut item = Map::new();
                item.insert(
                    "target".into(),
                    alias.map_or_else(|| json!({"object": object, "property": prop}), |a| json!(a)),
                );
                if let Some(f) = &series {
                    item.insert("series".into(), json!(field_text(f)));
                    if occurrence > 1 {
                        item.insert("occurrence".into(), json!(occurrence));
                    }
                }
                if let Some(m) = matching.filter(|m| *m != 1)
                    && let Some((name, _)) = APPLY_TO.iter().find(|(_, x)| *x == m)
                {
                    item.insert("apply_to".into(), json!(name));
                }
                item.insert("rule".into(), rule_json(&rule, kind, guessed));
                list.push(Value::Object(item));
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Command
// ---------------------------------------------------------------------------------------------

/// `visual.conditional {page, visual, target?, series?, apply_to?, rule}` or
/// `{page, visual, conditional: [...]}`.
pub(crate) fn visual_conditional(s: &mut crate::Session, p: &Value) -> Result<Value, String> {
    crate::params::allow(
        p,
        &[
            "page",
            "visual",
            "target",
            "series",
            "occurrence",
            "apply_to",
            "rule",
            "conditional",
        ],
    )?;
    let mut list: Vec<Value> = match p.get("conditional") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) => a.clone(),
        Some(_) => return Err("parameter `conditional` must be a list".into()),
    };
    let mut item = Map::new();
    for k in ["target", "series", "occurrence", "apply_to", "rule"] {
        if let Some(x) = p.get(k) {
            item.insert(k.to_string(), x.clone());
        }
    }
    if !item.is_empty() {
        if !item.contains_key("rule") {
            return Err("missing parameter `rule` (null removes the conditional format)".into());
        }
        list.push(Value::Object(item));
    }
    if list.is_empty() {
        return Err("nothing to do: pass rule (with target?, series?) or conditional: [...]".into());
    }
    crate::visual_props::edit_visual(s, p, |_, _| Ok(json!({ "conditional": list })))
}
