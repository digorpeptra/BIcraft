//! Visual properties (M2.4): title, sort, visual-level filters and formatting, plus the
//! `visual.set_title` / `visual.set_sort` / `visual.filter` / `visual.clear_filters` /
//! `visual.format` / `visual.props` commands.
//!
//! FROZEN API (other modules call it; keep the signatures):
//! - [`apply_props`]: applies a property bundle to one visual.
//! - [`extract_props`]: the inverse, for `report.extract`.
//!
//! # The `props` object
//!
//! Every key is optional; an absent key leaves that part of the visual untouched. Unknown keys
//! are errors. In nested objects, `null` removes the property it names.
//!
//! - `title`: `"text"` (sets the text and shows the title), `{text?, show?, font_size?, color?,
//!   alignment?: left|center|right, bold?, italic?, underline?, font_family?}`, or `null`
//!   (removes the title formatting, so
//!   the visual's default applies). Written to `visualContainerObjects.title`.
//! - `sort`: `{field, direction?: asc|desc}`, a list of them (multi-field sort), or `null`
//!   (removes the sort). `field` is `Table[Column]`, `[Measure]` or `Function(Table[Column])`; it
//!   must be bound to the visual or exist in the model. `direction` defaults to `desc` for
//!   measures and aggregations, `asc` for columns. Written to `query.sortDefinition`.
//! - `filters`: the visual-level filters, **as a complete list** (`[]` removes them all). Each
//!   item is one of
//!   - basic: `{field, in: [values]}` or `{field, not_in: [values]}` (columns only);
//!   - advanced: `{field, op, value}` or `{field, conditions: [{op, value}, {op, value}],
//!     logic?: and|or}` with `op` one of `=`, `<>`, `>`, `>=`, `<`, `<=`, `contains`,
//!     `not_contains`, `starts_with`, `not_starts_with` (`= null` is "is blank", `<> null`
//!     "is not blank"); on columns, measures or aggregations;
//!   - top N: `{field, top: N, by: "[Measure]" | "Sum(T[C])", direction?: top|bottom}`;
//!   - verbatim: `{raw: {...}}`, a PBIR filter container kept as given (matched by `name`);
//!
//!   plus `hidden?` / `locked?` (hidden or locked in view mode). Values are JSON strings, numbers,
//!   booleans or `null`; dates are `YYYY-MM-DD` or `YYYY-MM-DDThh:mm:ss`. With a model, values
//!   are typed by the column's data type (`'text'`, `12L`, `1.5D`, `2.5M`,
//!   `datetime'2024-01-01T00:00:00'`). A filter on a field that already has one is updated in
//!   place (keeping its name); filters absent from the list are removed (an automatic filter card
//!   of a bound field keeps its card and loses its condition). Filter cards without a condition
//!   are never removed.
//! - `format`: `{background?, border?, padding?, shadow?, header?, alt_text?, subtitle?,
//!   data_labels?, legend?, x_axis?, y_axis?, objects?}`:
//!   - container (`visualContainerObjects`): `background` (`bool` | `{show?, color?,
//!     transparency?}`), `border` (`bool` | `{show?, color?, radius?, width?}`), `padding`
//!     (number for all sides | `{top?, bottom?, left?, right?}`), `shadow` (`bool` | `{show?,
//!     color?, transparency?}`), `header` (`bool` | `{show?}`: the visual header), `alt_text`
//!     (string), `subtitle` (like `title`);
//!   - visual (`objects`): `data_labels` (`bool` | `{show?, font_size?, color?}`), `legend`
//!     (`bool` | `{show?, position?: Top|Bottom|Left|Right|TopCenter|BottomCenter|LeftCenter|
//!     RightCenter|TopRight|BottomRight, font_size?}`), `x_axis` / `y_axis` (`bool` | `{show?, title?: bool,
//!     font_size?, color?}`);
//!   - `objects`: the escape hatch, `[{object, property, value, selector?, state?, target?:
//!     visual|container}]`. `value` is a boolean, a number (written `12D`), a string (written
//!     `'text'`), `{color: "#RRGGBB"}`, `{literal: "12L"}` (a literal as written), a PBIR value
//!     (`{expr: …}` or `{solid: …}`) kept verbatim, or `null` (removes the property). The target
//!     defaults to the container when the object and property exist in
//!     `visualContainerObjects` (title, background, border…), else the visual's `objects`.
//!
//!   A whole group set to `null` (`"border": null`) removes that object's formatting. Colors are
//!   `#RGB`, `#RRGGBB` or `#RRGGBBAA`; `transparency` is 0–100.
//!
//! **Formatting states.** The stateful objects of the new card, the new slicers, buttons,
//! navigators and shapes (`visual_states.rs`) are read by Desktop from a state entry (selector
//! `{"id": "default"}`, `hover`, `press`, `selected`…), not from the entry without selector.
//! An `objects` item for such an object goes to the `default` state entry unless it names a
//! `state` (`"none"`: the entry without selector); `show` in the `default` state is also written
//! to the entry without selector, as Desktop does. `extract_props` lists the stateful objects'
//! formatting as `format.objects` items (with `state` when not `default`).
//!
//! Other formatting goes to the object's entry without `selector` (created when missing). Properties
//! are written in the literal form PBIR uses (`{"expr": {"Literal": {"Value": "true"}}}`, colors
//! as `{"solid": {"color": …}}`). Values are compared by meaning, not spelling (`12L` = `12D`),
//! so re-applying the same props is a no-op. Everything is validated before the visual changes.
//! Every value written (groups and `objects` alike) is checked against the formatting catalog of
//! the visual type (`visual_catalog`, `visual.options`): unknown objects and properties and bad
//! values are refused; custom visual types are not checked.

use bicraft_model::{DataType, SemanticModel};
use bicraft_report::{FieldRef, Visual};
use serde_json::{Map, Value, json};

use crate::Session;
use crate::edit::new_guid;
use crate::params::{allow, str_param};
use crate::refs::{ParsedField, field_text, parse_field, resolve_field, same_field};
use crate::report::{page_at_mut, page_index, parts, visual_index};

/// Most filters one visual may be given.
const MAX_FILTERS: usize = 100;
/// Most values in one basic filter.
const MAX_FILTER_VALUES: usize = 1000;
/// Most conditions of one advanced filter (Desktop's filter card offers two).
const MAX_CONDITIONS: usize = 2;
/// Largest N of a top N filter.
const MAX_TOP_N: u64 = 100_000;
/// Most sort fields.
const MAX_SORT_FIELDS: usize = 10;
/// Most generic `format.objects` edits.
const MAX_GENERIC: usize = 200;
/// Longest text value (titles, filter values, literals), in characters.
const MAX_TEXT: usize = 4000;
/// Longest object or property name.
const MAX_NAME: usize = 100;

/// Key order of the PBIR `visual` object (Desktop's), for keys this module creates.
pub(crate) const VISUAL_ORDER: &[&str] = &[
    "visualType",
    "autoSelectVisualType",
    "query",
    "expansionStates",
    "objects",
    "visualContainerObjects",
    "syncGroup",
    "drillFilterOtherVisuals",
];
/// Key order of a filter container (Desktop's).
const FILTER_ORDER: &[&str] = &[
    "name",
    "displayName",
    "ordinal",
    "field",
    "type",
    "filter",
    "restatement",
    "howCreated",
    "isHiddenInViewMode",
    "isLockedInViewMode",
    "objects",
];

/// `visualContainerObjects` objects and their properties (public `visualConfiguration` 2.7.0
/// schema, `VisualContainerFormattingObjects`): the schema rejects any other property.
const CONTAINER_OBJECTS: &[(&str, &[&str])] = &[
    (
        "title",
        &[
            "show",
            "text",
            "heading",
            "titleWrap",
            "fontColor",
            "background",
            "alignment",
            "fontSize",
            "bold",
            "italic",
            "underline",
            "fontFamily",
        ],
    ),
    (
        "subTitle",
        &[
            "show",
            "text",
            "heading",
            "titleWrap",
            "fontColor",
            "alignment",
            "fontSize",
            "bold",
            "italic",
            "underline",
            "fontFamily",
        ],
    ),
    ("divider", &["ignorePadding", "show", "color", "width", "style"]),
    (
        "spacing",
        &[
            "customizeSpacing",
            "verticalSpacing",
            "spaceBelowTitle",
            "spaceBelowSubTitle",
            "spaceBelowTitleArea",
            "spaceAbovePlotArea",
            "spaceAboveDivider",
            "spaceAboveSubtitle",
        ],
    ),
    ("background", &["show", "color", "transparency"]),
    ("padding", &["top", "bottom", "left", "right"]),
    ("lockAspect", &["show"]),
    (
        "general",
        &[
            "x",
            "y",
            "width",
            "height",
            "altText",
            "allowBinnedLineSample",
            "allowOverlappingPointsSample",
            "keepLayerOrder",
        ],
    ),
    ("border", &["show", "color", "radius", "width"]),
    (
        "dropShadow",
        &[
            "show",
            "preset",
            "position",
            "color",
            "transparency",
            "shadowSpread",
            "shadowBlur",
            "angle",
            "shadowDistance",
        ],
    ),
    (
        "visualLink",
        &[
            "show",
            "type",
            "bookmark",
            "disabledTooltip",
            "drillthroughSection",
            "enabledTooltip",
            "qna",
            "suppressDefaultTooltip",
            "showDefaultTooltip",
            "navigationSection",
            "tooltip",
            "tooltipPlaceholderText",
            "webUrl",
            "dataFunction",
        ],
    ),
    (
        "visualTooltip",
        &[
            "show",
            "type",
            "section",
            "titleFontColor",
            "valueFontColor",
            "fontSize",
            "bold",
            "italic",
            "underline",
            "fontFamily",
            "background",
            "transparency",
            "actionFontColor",
            "themedTitleFontColor",
            "themedBackground",
            "themedValueFontColor",
            "showTooltipFieldsOnly",
            "showSentenceFormat",
            "sentenceTemplate",
            "showChartSpecificTooltips",
            "showValuesInBold",
        ],
    ),
    ("stylePreset", &["name"]),
    (
        "visualHeader",
        &[
            "show",
            "background",
            "border",
            "transparency",
            "foreground",
            "showVisualInformationButton",
            "showVisualWarningButton",
            "showVisualErrorButton",
            "showDrillRoleSelector",
            "showDrillUpButton",
            "showDrillToggleButton",
            "showDrillDownLevelButton",
            "showDrillDownExpandButton",
            "showPinButton",
            "showFilterRestatementButton",
            "showFocusModeButton",
            "showCopyVisualImageButton",
            "showSeeDataLayoutToggleButton",
            "showOptionsMenu",
            "showCommentButton",
            "showTooltipButton",
            "showPersonalizeVisualButton",
            "showSmartNarrativeButton",
            "showSetAlertButton",
            "showFollowVisualButton",
            "showCopilotSummaryButton",
        ],
    ),
    (
        "visualHeaderTooltip",
        &[
            "type",
            "section",
            "text",
            "titleFontColor",
            "fontSize",
            "fontFamily",
            "bold",
            "italic",
            "underline",
            "background",
            "transparency",
            "themedTitleFontColor",
            "themedBackground",
        ],
    ),
];

/// PBIR aggregation codes (`QueryAggregateFunction`) and the names `refs.rs` resolves to.
const AGGREGATION_CODES: &[(u64, &str)] = &[
    (0, "Sum"),
    (1, "Avg"),
    (2, "Count"),
    (3, "Min"),
    (4, "Max"),
    (5, "CountNonNull"),
    (6, "Median"),
    (7, "StandardDeviation"),
    (8, "Variance"),
];

const LEGEND_POSITIONS: &[&str] = &[
    "Top",
    "Bottom",
    "Left",
    "Right",
    "TopCenter",
    "BottomCenter",
    "LeftCenter",
    "RightCenter",
    "TopRight",
    "BottomRight",
];
const ALIGNMENTS: &[&str] = &["left", "center", "right"];

// ---------------------------------------------------------------------------------------------
// Property values
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    /// `visual.visualContainerObjects`
    Container,
    /// `visual.objects`
    Visual,
}

impl Target {
    fn key(self) -> &'static str {
        match self {
            Self::Container => "visualContainerObjects",
            Self::Visual => "objects",
        }
    }
}

/// A formatting property value, before rendering to PBIR.
#[derive(Debug, Clone, PartialEq)]
enum PropValue {
    Bool(bool),
    Num(f64),
    Text(String),
    /// Text from a closed list, compared case-insensitively.
    Enum(String),
    Color(String),
    /// A literal as written (`12L`, `'x'`).
    Literal(String),
    /// A PBIR property value kept verbatim.
    Raw(Value),
}

pub(crate) fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn literal(text: String) -> Value {
    json!({"expr": {"Literal": {"Value": text}}})
}

/// `f64` as Desktop prints it before a type suffix: no fraction when integral.
pub(crate) fn num_text(v: f64) -> String {
    format!("{v}")
}

/// A JSON number, integral when it can be.
pub(crate) fn num_json(v: f64) -> Value {
    if v.fract() == 0.0 && v.abs() < 9.0e15 {
        json!(v as i64)
    } else {
        json!(v)
    }
}

impl PropValue {
    fn render(&self) -> Value {
        match self {
            Self::Bool(b) => literal(b.to_string()),
            Self::Num(n) => literal(format!("{}D", num_text(*n))),
            Self::Text(s) | Self::Enum(s) => literal(quote(s)),
            Self::Color(c) => json!({"solid": {"color": literal(quote(c))}}),
            Self::Literal(s) => literal(s.clone()),
            Self::Raw(v) => v.clone(),
        }
    }

    /// Whether the PBIR value `existing` means the same.
    fn matches(&self, existing: &Value) -> bool {
        match self {
            Self::Raw(v) => v == existing,
            Self::Literal(s) => literal_text(existing).is_some_and(|t| t == s),
            Self::Color(c) => color_text(existing).is_some_and(|t| t.eq_ignore_ascii_case(c)),
            Self::Bool(b) => matches!(decode_literal(existing), Some(Lit::Bool(x)) if x == *b),
            Self::Num(n) => matches!(decode_literal(existing), Some(Lit::Num(x, _)) if x == *n),
            Self::Text(s) => matches!(decode_literal(existing), Some(Lit::Text(x)) if &x == s),
            Self::Enum(s) => {
                matches!(decode_literal(existing), Some(Lit::Text(x)) if x.eq_ignore_ascii_case(s))
            }
        }
    }

    fn describe(&self) -> String {
        match self {
            Self::Bool(b) => b.to_string(),
            Self::Num(n) => num_text(*n),
            Self::Text(s) | Self::Enum(s) => quote(&shorten(s)),
            Self::Color(c) => c.clone(),
            Self::Literal(s) => shorten(s),
            Self::Raw(_) => "(PBIR value)".into(),
        }
    }
}

fn shorten(s: &str) -> String {
    if s.chars().count() > 40 {
        let head: String = s.chars().take(40).collect();
        format!("{head}…")
    } else {
        s.to_string()
    }
}

/// The literal text of `{"expr": {"Literal": {"Value": s}}}` exactly.
pub(crate) fn literal_text(v: &Value) -> Option<&str> {
    let o = v.as_object()?;
    if o.len() != 1 {
        return None;
    }
    let e = o.get("expr")?.as_object()?;
    if e.len() != 1 {
        return None;
    }
    literal_value(e.get("Literal")?)
}

/// `{"Value": s}` exactly.
fn literal_value(v: &Value) -> Option<&str> {
    let l = v.as_object()?;
    if l.len() != 1 {
        return None;
    }
    l.get("Value")?.as_str()
}

/// The color of `{"solid": {"color": <string literal>}}` exactly.
pub(crate) fn color_text(v: &Value) -> Option<String> {
    let o = v.as_object()?;
    if o.len() != 1 {
        return None;
    }
    let solid = o.get("solid")?.as_object()?;
    if solid.len() != 1 {
        return None;
    }
    match decode_literal(solid.get("color")?)? {
        Lit::Text(s) => Some(s),
        _ => None,
    }
}

pub(crate) fn decode_literal(v: &Value) -> Option<Lit> {
    literal_text(v).map(parse_lit)
}

// ---------------------------------------------------------------------------------------------
// Query literals (filter values)
// ---------------------------------------------------------------------------------------------

/// A query literal (`QueryLiteralExpression.Value`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Lit {
    Null,
    Bool(bool),
    /// Value and type suffix (`L`, `D`, `M`, or `' '` for none).
    Num(f64, char),
    Text(String),
    /// `YYYY-MM-DDThh:mm:ss[.f]`
    Date(String),
    Other(String),
}

pub(crate) fn parse_lit(s: &str) -> Lit {
    match s {
        "null" => return Lit::Null,
        "true" => return Lit::Bool(true),
        "false" => return Lit::Bool(false),
        _ => {}
    }
    if let Some(inner) = s.strip_prefix("datetime'").and_then(|r| r.strip_suffix('\'')) {
        return Lit::Date(inner.to_string());
    }
    if s.len() >= 2
        && let Some(inner) = s.strip_prefix('\'').and_then(|r| r.strip_suffix('\''))
    {
        return Lit::Text(inner.replace("''", "'"));
    }
    let (body, suffix) = match s.chars().last() {
        Some(c @ ('L' | 'D' | 'M')) => (s.get(..s.len().saturating_sub(1)).unwrap_or(""), c),
        _ => (s, ' '),
    };
    if !body.is_empty()
        && body
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '-' | '+' | '.' | 'e' | 'E'))
        && let Ok(n) = body.parse::<f64>()
        && n.is_finite()
    {
        return Lit::Num(n, suffix);
    }
    Lit::Other(s.to_string())
}

impl Lit {
    pub(crate) fn render(&self) -> String {
        match self {
            Self::Null => "null".into(),
            Self::Bool(b) => b.to_string(),
            Self::Num(n, ' ') => num_text(*n),
            Self::Num(n, s) => format!("{}{s}", num_text(*n)),
            Self::Text(t) => quote(t),
            Self::Date(d) => format!("datetime'{d}'"),
            Self::Other(s) => s.clone(),
        }
    }

    pub(crate) fn expr(&self) -> Value {
        json!({"Literal": {"Value": self.render()}})
    }

    pub(crate) fn to_json(&self) -> Value {
        match self {
            Self::Null => Value::Null,
            Self::Bool(b) => json!(b),
            Self::Num(n, _) => num_json(*n),
            Self::Text(t) | Self::Date(t) | Self::Other(t) => json!(t),
        }
    }

    /// Same value, whatever its spelling (`12L` = `12D`; a date-only text = its midnight).
    pub(crate) fn same(&self, other: &Lit) -> bool {
        match (self, other) {
            (Self::Num(a, _), Self::Num(b, _)) => a == b,
            (Self::Text(a), Self::Text(b)) | (Self::Other(a), Self::Other(b)) => a == b,
            (Self::Date(a) | Self::Text(a), Self::Date(b) | Self::Text(b)) => {
                normalize_date(a).is_some_and(|x| Some(x) == normalize_date(b))
            }
            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::Null, Self::Null) => true,
            _ => false,
        }
    }
}

/// `YYYY-MM-DD[Thh:mm[:ss[.fff]]]` → `YYYY-MM-DDThh:mm:ss[.fff]`, or `None` when `s` is not one.
pub(crate) fn normalize_date(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let digits = |r: std::ops::Range<usize>| b.get(r).is_some_and(|x| x.iter().all(u8::is_ascii_digit));
    if b.len() < 10
        || !digits(0..4)
        || b.get(4) != Some(&b'-')
        || !digits(5..7)
        || b.get(7) != Some(&b'-')
        || !digits(8..10)
    {
        return None;
    }
    if b.len() == 10 {
        return Some(format!("{s}T00:00:00"));
    }
    if b.get(10) != Some(&b'T') || !digits(11..13) || b.get(13) != Some(&b':') || !digits(14..16) {
        return None;
    }
    if b.len() == 16 {
        return Some(format!("{s}:00"));
    }
    if b.get(16) != Some(&b':') || !digits(17..19) {
        return None;
    }
    if b.len() == 19 {
        return Some(s.to_string());
    }
    if b.get(19) == Some(&b'.') && b.len() > 20 && b.len() <= 27 && digits(20..b.len()) {
        return Some(s.to_string());
    }
    None
}

/// How a field's filter values are typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValueKind {
    Text,
    Int,
    Double,
    Decimal,
    Date,
    Bool,
    /// Measures and aggregations: numbers are doubles.
    Numeric,
    Unknown,
}

pub(crate) fn value_kind(model: Option<&SemanticModel>, field: &FieldRef) -> ValueKind {
    match field {
        FieldRef::Column { table, column } => model
            .and_then(|m| m.table(table))
            .and_then(|t| t.column(column))
            .map_or(ValueKind::Unknown, |c| match c.data_type {
                DataType::String => ValueKind::Text,
                DataType::Int64 => ValueKind::Int,
                DataType::Double => ValueKind::Double,
                DataType::Decimal => ValueKind::Decimal,
                DataType::DateTime => ValueKind::Date,
                DataType::Boolean => ValueKind::Bool,
                _ => ValueKind::Unknown,
            }),
        FieldRef::Measure { .. } | FieldRef::Aggregation { .. } => ValueKind::Numeric,
        FieldRef::Other { .. } => ValueKind::Unknown,
    }
}

/// A filter value from JSON, typed for the field.
pub(crate) fn to_lit(v: &Value, kind: ValueKind, what: &str) -> Result<Lit, String> {
    let bad = |expected: &str| format!("{what}: expected {expected}, got {v}");
    match v {
        Value::Null => Ok(Lit::Null),
        Value::Bool(b) => match kind {
            ValueKind::Bool | ValueKind::Unknown | ValueKind::Numeric => Ok(Lit::Bool(*b)),
            _ => Err(bad("a value of the field's type, not a boolean")),
        },
        Value::Number(n) => {
            let x = n
                .as_f64()
                .filter(|x| x.is_finite())
                .ok_or_else(|| bad("a finite number"))?;
            let integral = x.fract() == 0.0 && x.abs() < 9.0e15;
            match kind {
                ValueKind::Text => Ok(Lit::Text(n.to_string())),
                ValueKind::Int if integral => Ok(Lit::Num(x, 'L')),
                ValueKind::Int => Err(bad("a whole number (the column is int64)")),
                ValueKind::Double | ValueKind::Numeric => Ok(Lit::Num(x, 'D')),
                ValueKind::Decimal => Ok(Lit::Num(x, 'M')),
                ValueKind::Unknown if integral => Ok(Lit::Num(x, 'L')),
                ValueKind::Unknown => Ok(Lit::Num(x, 'D')),
                ValueKind::Date => Err(bad("a date like 2024-01-31")),
                ValueKind::Bool => Err(bad("true or false")),
            }
        }
        Value::String(s) => {
            if s.chars().count() > MAX_TEXT {
                return Err(format!("{what}: text longer than {MAX_TEXT} characters"));
            }
            match kind {
                ValueKind::Date => normalize_date(s)
                    .map(Lit::Date)
                    .ok_or_else(|| bad("a date like 2024-01-31 or 2024-01-31T08:00:00")),
                ValueKind::Unknown => Ok(normalize_date(s).map_or_else(|| Lit::Text(s.clone()), Lit::Date)),
                ValueKind::Text | ValueKind::Numeric => Ok(Lit::Text(s.clone())),
                ValueKind::Int | ValueKind::Double | ValueKind::Decimal => Err(bad("a number")),
                ValueKind::Bool => Err(bad("true or false")),
            }
        }
        _ => Err(bad("a string, number, boolean or null")),
    }
}

// ---------------------------------------------------------------------------------------------
// Fields
// ---------------------------------------------------------------------------------------------

/// `(alias, entity)` pairs of a `From` list.
pub(crate) type Aliases = Vec<(String, String)>;

fn aggregation_code(name: &str) -> Option<u64> {
    AGGREGATION_CODES
        .iter()
        .find(|(_, n)| n.eq_ignore_ascii_case(name))
        .map(|(c, _)| *c)
}

fn aggregation_name(code: u64) -> Option<&'static str> {
    AGGREGATION_CODES
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, n)| *n)
}

/// `{"Expression": {"SourceRef": {"Entity": t} | {"Source": alias}}, "Property": p}` exactly.
fn property_parts(v: &Value, aliases: &Aliases) -> Option<(String, String)> {
    let o = v.as_object()?;
    if o.len() != 2 {
        return None;
    }
    let expr = o.get("Expression")?.as_object()?;
    if expr.len() != 1 {
        return None;
    }
    let src = expr.get("SourceRef")?.as_object()?;
    if src.len() != 1 {
        return None;
    }
    let table = if let Some(e) = src.get("Entity") {
        e.as_str()?.to_string()
    } else {
        let alias = src.get("Source")?.as_str()?;
        aliases.iter().find(|(a, _)| a == alias)?.1.clone()
    };
    Some((table, o.get("Property")?.as_str()?.to_string()))
}

/// A PBIR field expression (`Column`, `Measure` or `Aggregation`), with `Source` aliases
/// resolved through `aliases`. `extra_ok` names one extra key the container may carry (`Name`).
pub(crate) fn field_from_json(v: &Value, aliases: &Aliases, extra_ok: Option<&str>) -> Option<FieldRef> {
    let o = v.as_object()?;
    let mut found = None;
    for (k, x) in o {
        if Some(k.as_str()) == extra_ok {
            continue;
        }
        if found.is_some() {
            return None;
        }
        found = Some((k.as_str(), x));
    }
    let (kind, inner) = found?;
    match kind {
        "Column" => property_parts(inner, aliases).map(|(table, column)| FieldRef::Column { table, column }),
        "Measure" => {
            property_parts(inner, aliases).map(|(table, measure)| FieldRef::Measure { table, measure })
        }
        "Aggregation" => {
            let a = inner.as_object()?;
            if a.len() != 2 {
                return None;
            }
            let col = a.get("Expression")?.as_object()?;
            if col.len() != 1 {
                return None;
            }
            let (table, column) = property_parts(col.get("Column")?, aliases)?;
            let function = aggregation_name(a.get("Function")?.as_u64()?)?.to_string();
            Some(FieldRef::Aggregation {
                table,
                column,
                function,
            })
        }
        _ => None,
    }
}

/// The PBIR field expression of `f`: `SourceRef` is `{"Entity": table}`, or `{"Source": alias}`.
pub(crate) fn field_to_json(f: &FieldRef, alias: Option<&str>) -> Option<Value> {
    let prop = |table: &str, name: &str| {
        let src = match alias {
            Some(a) => json!({"Source": a}),
            None => json!({"Entity": table}),
        };
        json!({"Expression": {"SourceRef": src}, "Property": name})
    };
    match f {
        FieldRef::Column { table, column } => Some(json!({"Column": prop(table, column)})),
        FieldRef::Measure { table, measure } => Some(json!({"Measure": prop(table, measure)})),
        FieldRef::Aggregation {
            table,
            column,
            function,
        } => Some(json!({"Aggregation": {
            "Expression": {"Column": prop(table, column)},
            "Function": aggregation_code(function)?,
        }})),
        FieldRef::Other { .. } => None,
    }
}

fn field_table(f: &FieldRef) -> Option<&str> {
    match f {
        FieldRef::Column { table, .. }
        | FieldRef::Measure { table, .. }
        | FieldRef::Aggregation { table, .. } => Some(table),
        FieldRef::Other { .. } => None,
    }
}

/// Whether `f` is the field `parsed` names (names compared case-insensitively).
pub(crate) fn parsed_matches(parsed: &ParsedField, f: &FieldRef) -> bool {
    let eq = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
    match (parsed, f) {
        (ParsedField::Measure(name), FieldRef::Measure { measure, .. }) => eq(name, measure),
        (ParsedField::Qualified { table, name }, FieldRef::Column { table: t, column: n })
        | (ParsedField::Qualified { table, name }, FieldRef::Measure { table: t, measure: n }) => {
            eq(table, t) && eq(name, n)
        }
        (
            ParsedField::Aggregation {
                function,
                table,
                column,
            },
            FieldRef::Aggregation {
                table: t,
                column: c,
                function: g,
            },
        ) => eq(function, g) && eq(table, t) && eq(column, c),
        _ => false,
    }
}

/// Fields the visual already refers to (bindings, filters, sort): how fields are resolved when no
/// model is open.
pub(crate) fn known_fields(v: &Visual) -> Vec<FieldRef> {
    let mut out: Vec<FieldRef> = v.fields.iter().map(|b| b.field.clone()).collect();
    out.extend(filter_fields(v.filter_config.as_ref()));
    if let Some(Ok(sorts)) = current_sort(v) {
        out.extend(sorts.into_iter().map(|(f, _)| f));
    }
    out
}

/// The fields a `filterConfig` names (its cards' fields and their top N measures).
pub(crate) fn filter_fields(cfg: Option<&Value>) -> Vec<FieldRef> {
    let mut out = Vec::new();
    for c in filter_list(cfg).unwrap_or_default() {
        if let Some(f) = c.get("field").and_then(|f| field_from_json(f, &Vec::new(), None)) {
            out.push(f);
        }
        if let Some(Typed {
            kind: TypedKind::TopN { by, .. },
            ..
        }) = parse_container(c)
        {
            out.push(by);
        }
    }
    out
}

/// Resolves a field string: against the model when there is one, else against the fields the
/// visual already uses, else as written (`Table[Name]` as a column).
pub(crate) fn resolve(
    text: &str,
    model: Option<&SemanticModel>,
    known: &[FieldRef],
) -> Result<FieldRef, String> {
    let parsed = parse_field(text)?;
    if model.is_none()
        && let Some(f) = known.iter().find(|f| parsed_matches(&parsed, f))
    {
        return Ok(f.clone());
    }
    resolve_field(model, text).map(|r| r.field)
}

// ---------------------------------------------------------------------------------------------
// Small JSON helpers
// ---------------------------------------------------------------------------------------------

/// Inserts `key` into `map` (replacing in place when present), else before the first key that
/// comes after it in `order`, else at the end.
pub(crate) fn insert_ordered(map: &mut Map<String, Value>, key: &str, value: Value, order: &[&str]) {
    if let Some(slot) = map.get_mut(key) {
        *slot = value;
        return;
    }
    let rank = order.iter().position(|k| *k == key);
    let at = rank.and_then(|r| {
        map.keys()
            .position(|k| order.iter().position(|o| o == k).is_some_and(|kr| kr > r))
    });
    match at {
        Some(i) => {
            map.shift_insert(i, key.to_string(), value);
        }
        None => {
            map.insert(key.to_string(), value);
        }
    }
}

fn as_object<'a>(v: &'a Value, what: &str) -> Result<&'a Map<String, Value>, String> {
    v.as_object().ok_or_else(|| format!("`{what}` must be an object"))
}

pub(crate) fn check_keys(o: &Map<String, Value>, allowed: &[&str], what: &str) -> Result<(), String> {
    for k in o.keys() {
        if !allowed.contains(&k.as_str()) {
            return Err(format!(
                "unknown key `{k}` in {what} (expected: {})",
                allowed.join(", ")
            ));
        }
    }
    Ok(())
}

pub(crate) fn check_text(s: &str, what: &str) -> Result<(), String> {
    if s.chars().count() > MAX_TEXT {
        return Err(format!("{what} is longer than {MAX_TEXT} characters"));
    }
    Ok(())
}

pub(crate) fn check_ident(s: &str, what: &str) -> Result<(), String> {
    if s.is_empty() || s.len() > MAX_NAME || !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(format!(
            "{what} `{s}` is not a formatting name like title, labels or fontSize"
        ));
    }
    Ok(())
}

pub(crate) fn is_color(s: &str) -> bool {
    s.strip_prefix('#')
        .is_some_and(|h| matches!(h.len(), 3 | 6 | 8) && h.chars().all(|c| c.is_ascii_hexdigit()))
}

// ---------------------------------------------------------------------------------------------
// Formatting edits
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum EditOp {
    /// Sets (`Some`) or removes (`None`) one property.
    Set(String, Option<PropValue>),
    /// Removes the object's entries without selector.
    RemoveObject,
}

#[derive(Debug, Clone, PartialEq)]
struct Edit {
    target: Target,
    object: String,
    selector: Option<Value>,
    /// The formatting state asked for (`format.objects[].state`), resolved to a selector by
    /// [`resolve_states`].
    state: Option<String>,
    op: EditOp,
}

/// How a friendly property's value is read.
#[derive(Debug, Clone, Copy)]
enum Kind {
    Bool,
    Num(f64, f64),
    Color,
    Text,
    Enum(&'static [&'static str]),
}

/// The shorthand a friendly group accepts besides its object form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shorthand {
    /// `true` / `false` → `{show}`
    Show,
    /// a number → every property
    AllNumbers,
    /// `"text"` → `{text, show: true}`
    TextShow,
    /// `"text"` → the single property; `null` removes it (not the object)
    Text,
}

/// A friendly formatting group: one PBIR object, some of its properties.
struct Group {
    key: &'static str,
    target: Target,
    object: &'static str,
    /// (friendly name, PBIR property, kind)
    props: &'static [(&'static str, &'static str, Kind)],
    shorthand: Shorthand,
}

const FONT: Kind = Kind::Num(1.0, 400.0);
const PERCENT: Kind = Kind::Num(0.0, 100.0);
const PIXELS: Kind = Kind::Num(0.0, 1000.0);

const TITLE: Group = Group {
    key: "title",
    target: Target::Container,
    object: "title",
    props: &[
        ("text", "text", Kind::Text),
        ("show", "show", Kind::Bool),
        ("font_size", "fontSize", FONT),
        ("color", "fontColor", Kind::Color),
        ("alignment", "alignment", Kind::Enum(ALIGNMENTS)),
        ("bold", "bold", Kind::Bool),
        ("italic", "italic", Kind::Bool),
        ("underline", "underline", Kind::Bool),
        ("font_family", "fontFamily", Kind::Text),
    ],
    shorthand: Shorthand::TextShow,
};

const GROUPS: &[Group] = &[
    Group {
        key: "background",
        target: Target::Container,
        object: "background",
        props: &[
            ("show", "show", Kind::Bool),
            ("color", "color", Kind::Color),
            ("transparency", "transparency", PERCENT),
        ],
        shorthand: Shorthand::Show,
    },
    Group {
        key: "border",
        target: Target::Container,
        object: "border",
        props: &[
            ("show", "show", Kind::Bool),
            ("color", "color", Kind::Color),
            ("radius", "radius", PIXELS),
            ("width", "width", Kind::Num(0.0, 100.0)),
        ],
        shorthand: Shorthand::Show,
    },
    Group {
        key: "padding",
        target: Target::Container,
        object: "padding",
        props: &[
            ("top", "top", PIXELS),
            ("bottom", "bottom", PIXELS),
            ("left", "left", PIXELS),
            ("right", "right", PIXELS),
        ],
        shorthand: Shorthand::AllNumbers,
    },
    Group {
        key: "shadow",
        target: Target::Container,
        object: "dropShadow",
        props: &[
            ("show", "show", Kind::Bool),
            ("color", "color", Kind::Color),
            ("transparency", "transparency", PERCENT),
        ],
        shorthand: Shorthand::Show,
    },
    Group {
        key: "header",
        target: Target::Container,
        object: "visualHeader",
        props: &[("show", "show", Kind::Bool)],
        shorthand: Shorthand::Show,
    },
    Group {
        key: "alt_text",
        target: Target::Container,
        object: "general",
        props: &[("text", "altText", Kind::Text)],
        shorthand: Shorthand::Text,
    },
    Group {
        key: "subtitle",
        target: Target::Container,
        object: "subTitle",
        props: &[
            ("text", "text", Kind::Text),
            ("show", "show", Kind::Bool),
            ("font_size", "fontSize", FONT),
            ("color", "fontColor", Kind::Color),
            ("alignment", "alignment", Kind::Enum(ALIGNMENTS)),
            ("bold", "bold", Kind::Bool),
            ("italic", "italic", Kind::Bool),
            ("underline", "underline", Kind::Bool),
            ("font_family", "fontFamily", Kind::Text),
        ],
        shorthand: Shorthand::TextShow,
    },
    Group {
        key: "data_labels",
        target: Target::Visual,
        object: "labels",
        props: &[
            ("show", "show", Kind::Bool),
            ("font_size", "fontSize", FONT),
            ("color", "color", Kind::Color),
        ],
        shorthand: Shorthand::Show,
    },
    Group {
        key: "legend",
        target: Target::Visual,
        object: "legend",
        props: &[
            ("show", "show", Kind::Bool),
            ("position", "position", Kind::Enum(LEGEND_POSITIONS)),
            ("font_size", "fontSize", FONT),
        ],
        shorthand: Shorthand::Show,
    },
    Group {
        key: "x_axis",
        target: Target::Visual,
        object: "categoryAxis",
        props: &[
            ("show", "show", Kind::Bool),
            ("title", "showAxisTitle", Kind::Bool),
            ("font_size", "fontSize", FONT),
            ("color", "labelColor", Kind::Color),
        ],
        shorthand: Shorthand::Show,
    },
    Group {
        key: "y_axis",
        target: Target::Visual,
        object: "valueAxis",
        props: &[
            ("show", "show", Kind::Bool),
            ("title", "showAxisTitle", Kind::Bool),
            ("font_size", "fontSize", FONT),
            ("color", "labelColor", Kind::Color),
        ],
        shorthand: Shorthand::Show,
    },
];

fn kind_value(v: &Value, kind: Kind, what: &str) -> Result<PropValue, String> {
    match kind {
        Kind::Bool => v
            .as_bool()
            .map(PropValue::Bool)
            .ok_or_else(|| format!("{what} must be true, false or null")),
        Kind::Num(lo, hi) => match v.as_f64() {
            Some(n) if n.is_finite() && (lo..=hi).contains(&n) => Ok(PropValue::Num(n)),
            _ => Err(format!("{what} must be a number from {lo} to {hi}, or null")),
        },
        Kind::Color => match v.as_str() {
            Some(s) if is_color(s) => Ok(PropValue::Color(s.to_string())),
            _ => Err(format!(
                "{what} must be a color like #1F77B4 (#RGB, #RRGGBB or #RRGGBBAA), or null"
            )),
        },
        Kind::Text => match v.as_str() {
            Some(s) => {
                check_text(s, what)?;
                Ok(PropValue::Text(s.to_string()))
            }
            None => Err(format!("{what} must be a string or null")),
        },
        Kind::Enum(allowed) => {
            let s = v.as_str().unwrap_or_default();
            allowed
                .iter()
                .find(|a| a.eq_ignore_ascii_case(s))
                .map(|a| PropValue::Enum((*a).to_string()))
                .ok_or_else(|| format!("{what} must be one of {}, or null", allowed.join(", ")))
        }
    }
}

/// The edits a friendly group's value asks for.
fn group_edits(g: &Group, v: &Value, what: &str) -> Result<Vec<Edit>, String> {
    let edit = |prop: &str, value: Option<PropValue>| Edit {
        target: g.target,
        object: g.object.to_string(),
        selector: None,
        state: None,
        op: EditOp::Set(prop.to_string(), value),
    };
    let prop_of = |friendly: &str| g.props.iter().find(|(f, _, _)| *f == friendly);
    match (v, g.shorthand) {
        (Value::Null, Shorthand::Text) => Ok(g.props.iter().map(|(_, p, _)| edit(p, None)).collect()),
        (Value::Null, _) => Ok(vec![Edit {
            target: g.target,
            object: g.object.to_string(),
            selector: None,
            state: None,
            op: EditOp::RemoveObject,
        }]),
        (Value::Bool(b), Shorthand::Show) => Ok(vec![edit("show", Some(PropValue::Bool(*b)))]),
        (Value::Number(_), Shorthand::AllNumbers) => g
            .props
            .iter()
            .map(|(_, p, k)| Ok(edit(p, Some(kind_value(v, *k, what)?))))
            .collect(),
        (Value::String(_), Shorthand::TextShow) => Ok(vec![
            edit("text", Some(kind_value(v, Kind::Text, what)?)),
            edit("show", Some(PropValue::Bool(true))),
        ]),
        (Value::String(_), Shorthand::Text) => {
            let (_, p, k) = g.props.first().ok_or("internal: empty formatting group")?;
            Ok(vec![edit(p, Some(kind_value(v, *k, what)?))])
        }
        (Value::Object(o), _) if g.shorthand != Shorthand::Text => {
            let names: Vec<&str> = g.props.iter().map(|(f, _, _)| *f).collect();
            check_keys(o, &names, what)?;
            let mut out = Vec::new();
            for (k, x) in o {
                let (_, p, kind) = prop_of(k).ok_or("internal: unknown formatting key")?;
                let value = if x.is_null() {
                    None
                } else {
                    Some(kind_value(x, *kind, &format!("{what}.{k}"))?)
                };
                out.push(edit(p, value));
            }
            Ok(out)
        }
        _ => Err(match g.shorthand {
            Shorthand::Show => format!("{what} must be true, false, an object or null"),
            Shorthand::AllNumbers => format!("{what} must be a number, an object or null"),
            Shorthand::TextShow => format!("{what} must be a string, an object or null"),
            Shorthand::Text => format!("{what} must be a string or null"),
        }),
    }
}

fn container_props(object: &str) -> Option<&'static [&'static str]> {
    CONTAINER_OBJECTS
        .iter()
        .find(|(o, _)| *o == object)
        .map(|(_, p)| *p)
}

/// One `format.objects` item.
fn generic_edit(item: &Value, i: usize) -> Result<Edit, String> {
    let what = format!("format.objects[{i}]");
    let o = as_object(item, &what)?;
    check_keys(
        o,
        &["object", "property", "value", "selector", "state", "target"],
        &what,
    )?;
    let object = o
        .get("object")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{what}.object must be a string"))?;
    let property = o
        .get("property")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{what}.property must be a string"))?;
    check_ident(object, "object")?;
    check_ident(property, "property")?;
    let selector = match o.get("selector") {
        None | Some(Value::Null) => None,
        Some(s @ Value::Object(_)) => Some(s.clone()),
        Some(_) => return Err(format!("{what}.selector must be an object")),
    };
    let state = match o.get("state") {
        None | Some(Value::Null) => None,
        Some(Value::String(s))
            if !s.is_empty()
                && s.len() <= MAX_NAME
                && s.chars().all(|c| c.is_ascii_alphanumeric() || c == ':') =>
        {
            Some(s.clone())
        }
        Some(_) => {
            return Err(format!(
                "{what}.state must be a state name like default, hover, press, selected or disabled (or none)"
            ));
        }
    };
    if state.is_some() && selector.is_some() {
        return Err(format!(
            "{what}: pass `state` or `selector`, not both (a state is the selector {{\"id\": state}})"
        ));
    }
    let allowed = container_props(object);
    let target = match o.get("target").and_then(Value::as_str) {
        Some("container") => Target::Container,
        Some("visual") => Target::Visual,
        None if o.get("target").is_none_or(Value::is_null) => {
            if allowed.is_some_and(|p| p.contains(&property)) {
                Target::Container
            } else {
                Target::Visual
            }
        }
        _ => return Err(format!("{what}.target must be \"visual\" or \"container\"")),
    };
    if target == Target::Container {
        match allowed {
            None => {
                return Err(format!(
                    "`{object}` is not a container object (expected one of {})",
                    CONTAINER_OBJECTS
                        .iter()
                        .map(|(o, _)| *o)
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            Some(p) if !p.contains(&property) => {
                return Err(format!(
                    "container object `{object}` has no property `{property}` (expected one of {})",
                    p.join(", ")
                ));
            }
            Some(_) => {}
        }
    }
    let value = match o.get("value") {
        None => return Err(format!("{what}.value is missing (null removes the property)")),
        Some(Value::Null) => None,
        Some(Value::Bool(b)) => Some(PropValue::Bool(*b)),
        Some(Value::Number(n)) => match n.as_f64() {
            Some(x) if x.is_finite() => Some(PropValue::Num(x)),
            _ => return Err(format!("{what}.value must be a finite number")),
        },
        Some(Value::String(s)) => {
            check_text(s, &format!("{what}.value"))?;
            Some(PropValue::Text(s.clone()))
        }
        Some(Value::Object(v)) if v.len() == 1 => {
            if let Some(c) = v.get("color") {
                Some(kind_value(c, Kind::Color, &format!("{what}.value.color"))?)
            } else if let Some(l) = v.get("literal") {
                let s = l
                    .as_str()
                    .ok_or_else(|| format!("{what}.value.literal must be a string like 12L or 'text'"))?;
                check_text(s, &format!("{what}.value.literal"))?;
                if s.is_empty() {
                    return Err(format!("{what}.value.literal is empty"));
                }
                Some(PropValue::Literal(s.to_string()))
            } else if v.contains_key("expr") || v.contains_key("solid") {
                Some(PropValue::Raw(Value::Object(v.clone())))
            } else {
                return Err(format!(
                    "{what}.value: an object value is {{color}}, {{literal}}, or a PBIR {{expr}} / {{solid}} value"
                ));
            }
        }
        Some(_) => {
            return Err(format!(
                "{what}.value must be a boolean, number, string, {{color}}, {{literal}}, {{expr}}, {{solid}} or null"
            ));
        }
    };
    Ok(Edit {
        target,
        object: object.to_string(),
        selector,
        state,
        op: EditOp::Set(property.to_string(), value),
    })
}

fn parse_format(v: &Value) -> Result<Vec<Edit>, String> {
    let o = as_object(v, "format")?;
    let mut keys: Vec<&str> = GROUPS.iter().map(|g| g.key).collect();
    keys.push("objects");
    check_keys(o, &keys, "format")?;
    let mut out = Vec::new();
    for (k, x) in o {
        if k == "objects" {
            let items = match x {
                Value::Null => continue,
                Value::Array(a) => a,
                _ => return Err("format.objects must be a list".into()),
            };
            if items.len() > MAX_GENERIC {
                return Err(format!("format.objects has more than {MAX_GENERIC} items"));
            }
            for (i, item) in items.iter().enumerate() {
                out.push(generic_edit(item, i)?);
            }
        } else if let Some(g) = GROUPS.iter().find(|g| g.key == k) {
            out.extend(group_edits(g, x, &format!("format.{k}"))?);
        }
    }
    Ok(out)
}

/// The index of the entry of `arr` whose selector is `selector` (`None`: no selector).
fn entry_index(arr: &[Value], selector: Option<&Value>) -> Option<usize> {
    arr.iter().position(|e| e.get("selector") == selector)
}

fn describe_target(e: &Edit, prop: &str) -> String {
    let prefix = match e.target {
        Target::Container => "",
        Target::Visual => "objects.",
    };
    let sel = match (&e.state, &e.selector) {
        (Some(st), Some(_)) => format!(" (state {st})"),
        (_, Some(_)) => " (selector)".to_string(),
        _ => String::new(),
    };
    format!("{prefix}{}.{prop}{sel}", e.object)
}

/// Resolves formatting states: an edit of a stateful object (`visual_states`) without explicit
/// selector goes to its state entry (`{"id": state}`, `default` when none is named; `none` names
/// the entry without selector). Setting `show` in the `default` state also sets it in the entry
/// without selector, where Desktop reads the toggle of buttons and shapes (Desktop writes both).
fn resolve_states(visual_type: Option<&str>, edits: &[Edit]) -> Result<Vec<Edit>, String> {
    use crate::visual_states::{DEFAULT_STATE, NO_STATE, object_states, state_selector};
    let mut out = Vec::with_capacity(edits.len());
    for e in edits {
        let states = match e.target {
            Target::Visual if e.selector.is_none() => object_states(visual_type, &e.object),
            _ => None,
        };
        let Some(states) = states else {
            match e.state.as_deref() {
                None | Some(NO_STATE) => {}
                Some(_) if e.target == Target::Container => {
                    return Err(format!(
                        "`{}` is a container object; formatting states apply to visual objects",
                        e.object
                    ));
                }
                Some(_) => {
                    return Err(format!(
                        "`{}` of {} has no formatting states (state selectors apply to the objects of {})",
                        e.object,
                        visual_type.unwrap_or("this visual"),
                        "cardVisual, listSlicer, advancedSlicerVisual, actionButton, bookmarkNavigator, pageNavigator and shape"
                    ));
                }
            }
            let mut x = e.clone();
            x.state = None;
            out.push(x);
            continue;
        };
        if matches!(e.op, EditOp::RemoveObject) {
            out.push(e.clone());
            continue;
        }
        let state = e.state.as_deref().unwrap_or(DEFAULT_STATE);
        if state == NO_STATE {
            let mut x = e.clone();
            x.state = None;
            out.push(x);
            continue;
        }
        if !states.contains(&state) {
            return Err(format!(
                "`{}` of {} has no state `{state}` (expected one of {}, or none)",
                e.object,
                visual_type.unwrap_or("this visual"),
                states.join(", ")
            ));
        }
        let mut x = e.clone();
        x.selector = Some(state_selector(state));
        x.state = Some(state.to_string());
        let mirror = state == DEFAULT_STATE && matches!(&e.op, EditOp::Set(p, _) if p == "show");
        out.push(x);
        if mirror {
            let mut m = e.clone();
            m.state = None;
            out.push(m);
        }
    }
    Ok(out)
}

/// Applies formatting edits to `v`, recording what changed.
/// Each value written is checked against the visual formatting catalog (`visual_catalog`) when the
/// visual type is a built-in one.
fn apply_edits(v: &mut Visual, edits: &[Edit], changes: &mut Vec<String>) -> Result<(), String> {
    let visual_type = v.visual_type.clone();
    let edits = resolve_states(visual_type.as_deref(), edits)?;
    for e in &edits {
        let key = e.target.key();
        let setting = matches!(&e.op, EditOp::Set(_, Some(_)));
        if !v.visual_extra.contains_key(key) {
            if !setting {
                continue;
            }
            insert_ordered(&mut v.visual_extra, key, json!({}), VISUAL_ORDER);
        }
        let objects = v
            .visual_extra
            .get_mut(key)
            .and_then(Value::as_object_mut)
            .ok_or_else(|| format!("the visual's `{key}` is not an object; it cannot be edited"))?;
        if !objects.contains_key(&e.object) {
            if !setting {
                continue;
            }
            objects.insert(e.object.clone(), json!([]));
        }
        let arr = objects
            .get_mut(&e.object)
            .and_then(Value::as_array_mut)
            .ok_or_else(|| {
                format!(
                    "the visual's `{key}.{}` is not a list; it cannot be edited",
                    e.object
                )
            })?;
        match &e.op {
            EditOp::RemoveObject => {
                let before = arr.len();
                arr.retain(|x| x.get("selector").is_some());
                if arr.len() != before {
                    changes.push(format!(
                        "{} removed",
                        describe_target(e, "*").trim_end_matches(".*")
                    ));
                }
            }
            EditOp::Set(prop, value) => {
                let idx = match entry_index(arr, e.selector.as_ref()) {
                    Some(i) => i,
                    None if value.is_some() => {
                        let mut entry = Map::new();
                        entry.insert("properties".into(), json!({}));
                        if let Some(sel) = &e.selector {
                            entry.insert("selector".into(), sel.clone());
                        }
                        arr.push(Value::Object(entry));
                        arr.len().saturating_sub(1)
                    }
                    None => continue,
                };
                let entry = arr
                    .get_mut(idx)
                    .and_then(Value::as_object_mut)
                    .ok_or_else(|| format!("an entry of `{key}.{}` is not an object", e.object))?;
                if !entry.contains_key("properties") {
                    if value.is_none() {
                        continue;
                    }
                    entry.insert("properties".into(), json!({}));
                }
                let props = entry
                    .get_mut("properties")
                    .and_then(Value::as_object_mut)
                    .ok_or_else(|| format!("the properties of `{key}.{}` are not an object", e.object))?;
                match value {
                    Some(val) => {
                        if props.get(prop).is_some_and(|old| val.matches(old)) {
                            continue;
                        }
                        let rendered = val.render();
                        // Only what is written is checked: values the visual already has stay.
                        crate::visual_catalog::check_entry_at(
                            visual_type.as_deref(),
                            Some(e.target == Target::Container),
                            &e.object,
                            prop,
                            &rendered,
                        )
                        .map_err(|m| format!("{}: {m}", describe_target(e, prop)))?;
                        props.insert(prop.clone(), rendered);
                        changes.push(format!("{} = {}", describe_target(e, prop), val.describe()));
                    }
                    None => {
                        if props.shift_remove(prop).is_some() {
                            changes.push(format!("{} removed", describe_target(e, prop)));
                        }
                    }
                }
                // An entry left with no property (and nothing but its selector) means nothing.
                let empty = props.is_empty()
                    && entry.keys().all(|k| k == "properties" || k == "selector")
                    && value.is_none();
                if empty {
                    arr.remove(idx);
                }
            }
        }
        // Drop what the edit left empty.
        if arr.is_empty() {
            objects.shift_remove(&e.object);
        }
        if objects.is_empty() {
            v.visual_extra.shift_remove(key);
        }
    }
    Ok(())
}

/// The selector-less entry's properties of `object` in `target`.
fn object_props<'a>(v: &'a Visual, target: Target, object: &str) -> Option<&'a Map<String, Value>> {
    v.visual_extra
        .get(target.key())?
        .get(object)?
        .as_array()?
        .iter()
        .find(|e| e.get("selector").is_none())?
        .get("properties")?
        .as_object()
}

/// The friendly JSON of a PBIR property value, when it is of `kind`.
fn decode_kind(v: &Value, kind: Kind) -> Option<Value> {
    match kind {
        Kind::Bool => match decode_literal(v)? {
            Lit::Bool(b) => Some(json!(b)),
            _ => None,
        },
        Kind::Num(..) => match decode_literal(v)? {
            Lit::Num(n, _) => Some(num_json(n)),
            _ => None,
        },
        Kind::Color => color_text(v).filter(|c| is_color(c)).map(Value::String),
        Kind::Text => match decode_literal(v)? {
            Lit::Text(s) => Some(Value::String(s)),
            _ => None,
        },
        Kind::Enum(allowed) => match decode_literal(v)? {
            Lit::Text(s) if allowed.iter().any(|a| a.eq_ignore_ascii_case(&s)) => Some(Value::String(s)),
            _ => None,
        },
    }
}

/// The friendly form of a group, or `None` when the visual sets none of its properties.
fn extract_group(v: &Visual, g: &Group) -> Option<Value> {
    let props = object_props(v, g.target, g.object)?;
    let mut out = Map::new();
    for (friendly, prop, kind) in g.props {
        if let Some(x) = props.get(*prop).and_then(|p| decode_kind(p, *kind)) {
            out.insert((*friendly).to_string(), x);
        }
    }
    if out.is_empty() {
        return None;
    }
    if g.shorthand == Shorthand::Text {
        return out.into_iter().next().map(|(_, x)| x);
    }
    if g.shorthand == Shorthand::TextShow
        && out.len() == 2
        && out.get("show") == Some(&json!(true))
        && let Some(Value::String(t)) = out.get("text")
    {
        return Some(json!(t));
    }
    Some(Value::Object(out))
}

/// The `format.objects` value of a PBIR property value: `true`/`false`, a string, `{color}`, a
/// `{literal}` (numbers keep their suffix), or the PBIR value itself.
fn friendly_value(v: &Value) -> Value {
    if let Some(c) = color_text(v).filter(|c| is_color(c)) {
        return json!({ "color": c });
    }
    match (decode_literal(v), literal_text(v)) {
        (Some(Lit::Bool(b)), _) => json!(b),
        (Some(Lit::Text(t)), _) => json!(t),
        (Some(_), Some(text)) => json!({ "literal": text }),
        _ => v.clone(),
    }
}

/// Whether `format.objects` can write `v` back (the escape hatch's value shapes).
fn escape_ok(v: &Value) -> bool {
    match v {
        Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Null => true,
        Value::Object(o) => ["color", "literal", "expr", "solid"]
            .iter()
            .any(|k| o.contains_key(*k)),
        Value::Array(_) => false,
    }
}

/// The visual's plain objects (each entry without selector or state) as `format.objects` items:
/// what the Format visual tab's cards write, so `visual.props` reads a card's value back.
fn extract_plain_objects(v: &Visual) -> Vec<Value> {
    use crate::visual_states::object_states;
    let vt = v.visual_type.as_deref();
    let mut out = Vec::new();
    let Some(objects) = v.visual_extra.get("objects").and_then(Value::as_object) else {
        return out;
    };
    for (object, entries) in objects {
        // a stateful object is read by `extract_state_objects` (a `show` without selector
        // mirrors its default state, it is not a plain object)
        if object_states(vt, object).is_some() {
            continue;
        }
        let Some(entries) = entries.as_array() else {
            continue;
        };
        for e in entries {
            if e.get("selector").is_some() || e.get("state").is_some() {
                continue;
            }
            let Some(props) = e.get("properties").and_then(Value::as_object) else {
                continue;
            };
            for (prop, value) in props {
                if value.is_null() {
                    continue;
                }
                let value = friendly_value(value);
                // only what `format.objects` can write back (the escape hatch's value shapes)
                if !escape_ok(&value) {
                    continue;
                }
                out.push(json!({
                    "object": object,
                    "property": prop,
                    "value": value,
                }));
            }
        }
    }
    out
}

/// The formatting of the stateful objects of a stateful visual (`visual_states`), as
/// `format.objects` items: `{object, property, value, state?}`, `state` left out for the
/// `default` state and `"none"` for an entry without selector (a `show` that only mirrors the
/// `default` state's is left out). Re-applying the list changes nothing.
fn extract_state_objects(v: &Visual) -> Option<Value> {
    use crate::visual_states::{DEFAULT_STATE, NO_STATE, entry_state, object_states};
    let vt = v.visual_type.as_deref();
    if !crate::visual_states::is_stateful_type(vt) {
        return None;
    }
    let objects = v.visual_extra.get("objects")?.as_object()?;
    let mut out = Vec::new();
    for (object, entries) in objects {
        if object_states(vt, object).is_none() {
            continue;
        }
        let Some(entries) = entries.as_array() else {
            continue;
        };
        let default_show = entries
            .iter()
            .filter(|e| entry_state(e) == Some(DEFAULT_STATE))
            .find_map(|e| e.get("properties")?.get("show"));
        for e in entries {
            let Some(state) = entry_state(e) else {
                continue;
            };
            let Some(props) = e.get("properties").and_then(Value::as_object) else {
                continue;
            };
            for (prop, value) in props {
                if state == NO_STATE
                    && prop == "show"
                    && default_show.is_some_and(|d| decode_literal(d) == decode_literal(value))
                {
                    continue;
                }
                let mut item = Map::new();
                item.insert("object".into(), json!(object));
                item.insert("property".into(), json!(prop));
                item.insert("value".into(), friendly_value(value));
                if state != DEFAULT_STATE {
                    item.insert("state".into(), json!(state));
                }
                out.push(Value::Object(item));
            }
        }
    }
    (!out.is_empty()).then_some(Value::Array(out))
}

// ---------------------------------------------------------------------------------------------
// Sort
// ---------------------------------------------------------------------------------------------

/// `Some(Ok(fields))` when the visual has a sort bicraft can read, `Some(Err)` when it has one
/// it cannot, `None` when it has none.
pub(crate) fn current_sort(v: &Visual) -> Option<Result<Vec<(FieldRef, bool)>, ()>> {
    let def = v.visual_extra.get("query")?.get("sortDefinition")?;
    let parse = || -> Option<Vec<(FieldRef, bool)>> {
        let sorts = def.get("sort")?.as_array()?;
        let mut out = Vec::new();
        for s in sorts {
            let o = s.as_object()?;
            if o.len() != 2 {
                return None;
            }
            let field = field_from_json(o.get("field")?, &Vec::new(), None)?;
            let desc = match o.get("direction")?.as_str()? {
                "Ascending" => false,
                "Descending" => true,
                _ => return None,
            };
            out.push((field, desc));
        }
        Some(out)
    };
    Some(parse().ok_or(()))
}

/// The requested sort: `None` clears it.
fn parse_sort(
    v: &Value,
    model: Option<&SemanticModel>,
    visual: &Visual,
    known: &[FieldRef],
) -> Result<Option<Vec<(FieldRef, bool)>>, String> {
    let items: Vec<&Value> = match v {
        Value::Null => return Ok(None),
        Value::Array(a) => a.iter().collect(),
        Value::Object(_) => vec![v],
        _ => return Err("`sort` must be {field, direction}, a list of them, or null".into()),
    };
    if items.is_empty() {
        return Ok(None);
    }
    if items.len() > MAX_SORT_FIELDS {
        return Err(format!("`sort` has more than {MAX_SORT_FIELDS} fields"));
    }
    let mut out: Vec<(FieldRef, bool)> = Vec::new();
    for item in items {
        let o = as_object(item, "sort")?;
        check_keys(o, &["field", "direction"], "sort")?;
        let text = o
            .get("field")
            .and_then(Value::as_str)
            .ok_or("sort.field must be a field string like Sales[Amount] or [Total]")?;
        let parsed = parse_field(text)?;
        // A bound field is always acceptable, even one the model does not know (yet).
        let field = match visual.fields.iter().find(|b| parsed_matches(&parsed, &b.field)) {
            Some(b) if model.is_none() => b.field.clone(),
            _ => resolve(text, model, known)?,
        };
        let desc = match o.get("direction") {
            None | Some(Value::Null) => !matches!(field, FieldRef::Column { .. }),
            Some(Value::String(d)) => match d.to_ascii_lowercase().as_str() {
                "asc" | "ascending" => false,
                "desc" | "descending" => true,
                _ => return Err(format!("sort.direction must be \"asc\" or \"desc\", not `{d}`")),
            },
            Some(_) => return Err("sort.direction must be \"asc\" or \"desc\"".into()),
        };
        if out.iter().any(|(f, _)| same_field(f, &field)) {
            return Err(format!("{} is listed twice in `sort`", field_text(&field)));
        }
        out.push((field, desc));
    }
    Ok(Some(out))
}

/// A reader's sort of `visual` (`view.interact` `sort`): `field` (a field string, or null for
/// the stored sort) and `direction` as `visual.set_sort` takes them; `None` for null.
pub(crate) fn reader_sort(
    visual: &Visual,
    model: Option<&SemanticModel>,
    field: &Value,
    direction: Option<&Value>,
) -> Result<Option<Vec<(FieldRef, bool)>>, String> {
    let item = match field {
        Value::Null => Value::Null,
        f => {
            let mut o = Map::new();
            o.insert("field".into(), f.clone());
            if let Some(d) = direction.filter(|d| !d.is_null()) {
                o.insert("direction".into(), d.clone());
            }
            Value::Object(o)
        }
    };
    parse_sort(&item, model, visual, &known_fields(visual))
}

/// Writes `sorts` as `v`'s `query.sortDefinition` (a derived report's visual: the reader's
/// sort); a visual whose `query` is not an object keeps its own.
pub(crate) fn put_sort(v: &mut Visual, sorts: &[(FieldRef, bool)]) {
    let mut changes = Vec::new();
    let _ = apply_sort(v, Some(sorts.to_vec()), &mut changes);
}

/// A bookmark's `singleVisual.orderBy` (semanticQuery `QuerySortClause`s: `Direction` 1
/// ascending, 2 descending) as sort fields; `None` when absent or not readable.
pub(crate) fn sorts_from_order_by(order_by: &Value) -> Option<Vec<(FieldRef, bool)>> {
    let list = order_by.as_array()?;
    let mut out = Vec::new();
    for c in list.iter().take(MAX_SORT_FIELDS) {
        let field = field_from_json(c.get("Expression")?, &Vec::new(), None)?;
        let desc = match c.get("Direction")?.as_u64()? {
            1 => false,
            2 => true,
            _ => return None,
        };
        out.push((field, desc));
    }
    (!out.is_empty()).then_some(out)
}

/// Sort fields as a bookmark's `singleVisual.orderBy` (see [`sorts_from_order_by`]).
pub(crate) fn order_by_json(sorts: &[(FieldRef, bool)]) -> Option<Value> {
    let mut list = Vec::new();
    for (f, desc) in sorts {
        let e = field_to_json(f, None)?;
        list.push(json!({"Direction": if *desc { 2 } else { 1 }, "Expression": e}));
    }
    Some(Value::Array(list))
}

fn sort_text(sorts: &[(FieldRef, bool)]) -> String {
    sorts
        .iter()
        .map(|(f, d)| format!("{} {}", field_text(f), if *d { "desc" } else { "asc" }))
        .collect::<Vec<_>>()
        .join(", ")
}

fn apply_sort(
    v: &mut Visual,
    req: Option<Vec<(FieldRef, bool)>>,
    changes: &mut Vec<String>,
) -> Result<(), String> {
    let current = current_sort(v);
    match req {
        None => {
            if current.is_some()
                && let Some(q) = v.visual_extra.get_mut("query").and_then(Value::as_object_mut)
            {
                q.shift_remove("sortDefinition");
                changes.push("sort removed".into());
            }
        }
        Some(sorts) => {
            if let Some(Ok(cur)) = &current
                && cur.len() == sorts.len()
                && cur
                    .iter()
                    .zip(&sorts)
                    .all(|((a, da), (b, db))| da == db && same_field(a, b))
            {
                return Ok(());
            }
            let mut list = Vec::new();
            for (f, desc) in &sorts {
                let field = field_to_json(f, None).ok_or("internal: sort field cannot be written")?;
                list.push(
                    json!({"field": field, "direction": if *desc { "Descending" } else { "Ascending" }}),
                );
            }
            if !v.visual_extra.contains_key("query") {
                insert_ordered(
                    &mut v.visual_extra,
                    "query",
                    json!({"queryState": {}}),
                    VISUAL_ORDER,
                );
            }
            let q = v
                .visual_extra
                .get_mut("query")
                .and_then(Value::as_object_mut)
                .ok_or("the visual's `query` is not an object; it cannot be sorted")?;
            if !q.contains_key("queryState") {
                q.shift_insert(0, "queryState".into(), json!({}));
            }
            insert_ordered(
                q,
                "sortDefinition",
                json!({"sort": list}),
                &["queryState", "sortDefinition", "options", "isDrillDisabled"],
            );
            changes.push(format!("sort = {}", sort_text(&sorts)));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Filters
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Op {
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
    Contains,
    NotContains,
    StartsWith,
    NotStartsWith,
}

const OPS: &[(&str, Op)] = &[
    ("=", Op::Eq),
    ("<>", Op::Ne),
    (">", Op::Gt),
    (">=", Op::Ge),
    ("<", Op::Lt),
    ("<=", Op::Le),
    ("contains", Op::Contains),
    ("not_contains", Op::NotContains),
    ("starts_with", Op::StartsWith),
    ("not_starts_with", Op::NotStartsWith),
];

impl Op {
    fn name(self) -> &'static str {
        OPS.iter().find(|(_, o)| *o == self).map_or("=", |(n, _)| n)
    }

    fn comparison_kind(self) -> Option<u64> {
        match self {
            Self::Eq | Self::Ne => Some(0),
            Self::Gt => Some(1),
            Self::Ge => Some(2),
            Self::Lt => Some(3),
            Self::Le => Some(4),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum TypedKind {
    Basic { values: Vec<Lit>, exclude: bool },
    Advanced { conditions: Vec<(Op, Lit)>, or: bool },
    TopN { count: u64, by: FieldRef, bottom: bool },
    Relative(crate::filter_relative::Relative),
}

/// A filter bicraft understands.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Typed {
    field: FieldRef,
    kind: TypedKind,
}

impl TypedKind {
    fn same(&self, other: &TypedKind) -> bool {
        let lits = |a: &[Lit], b: &[Lit]| a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.same(y));
        match (self, other) {
            (
                Self::Basic { values, exclude },
                Self::Basic {
                    values: v2,
                    exclude: e2,
                },
            ) => exclude == e2 && lits(values, v2),
            (
                Self::Advanced { conditions, or },
                Self::Advanced {
                    conditions: c2,
                    or: o2,
                },
            ) => {
                (conditions.len() < 2 || or == o2)
                    && conditions.len() == c2.len()
                    && conditions
                        .iter()
                        .zip(c2)
                        .all(|((a, x), (b, y))| a == b && x.same(y))
            }
            (
                Self::TopN { count, by, bottom },
                Self::TopN {
                    count: c2,
                    by: b2,
                    bottom: m2,
                },
            ) => count == c2 && bottom == m2 && same_field(by, b2),
            (Self::Relative(a), Self::Relative(b)) => a == b,
            _ => false,
        }
    }

    fn type_name(&self) -> &'static str {
        match self {
            Self::Basic { .. } => "Categorical",
            Self::Advanced { .. } => "Advanced",
            Self::TopN { .. } => "TopN",
            Self::Relative(r) => r.type_name(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum FilterReq {
    Typed {
        typed: Typed,
        hidden: Option<bool>,
        locked: Option<bool>,
    },
    Raw(Value),
}

/// The filter containers (cards) of a `filterConfig`.
pub(crate) fn filter_list(cfg: Option<&Value>) -> Option<Vec<&Value>> {
    Some(cfg?.get("filters")?.as_array()?.iter().collect())
}

/// `From` entries that name a model table: `(alias, entity)`.
fn entity_aliases(from: &[Value]) -> Option<Aliases> {
    let mut out = Vec::new();
    for e in from {
        let o = e.as_object()?;
        if o.contains_key("Expression") {
            continue;
        }
        if o.get("Type").is_some_and(|t| t.as_u64() != Some(0)) {
            return None;
        }
        out.push((
            o.get("Name")?.as_str()?.to_string(),
            o.get("Entity")?.as_str()?.to_string(),
        ));
    }
    Some(out)
}

/// The one key of `v` when it has exactly one.
pub(crate) fn only_key(v: &Value) -> Option<(&str, &Value)> {
    let o = v.as_object()?;
    if o.len() != 1 {
        return None;
    }
    o.iter().next().map(|(k, x)| (k.as_str(), x))
}

fn lit_from_expr(v: &Value) -> Option<Lit> {
    let (k, x) = only_key(v)?;
    if k != "Literal" {
        return None;
    }
    match parse_lit(literal_value(x)?) {
        Lit::Other(_) => None,
        l => Some(l),
    }
}

/// One advanced condition on `field`.
fn parse_condition(c: &Value, field: &FieldRef, aliases: &Aliases) -> Option<(Op, Lit)> {
    let (k, x) = only_key(c)?;
    let binary = |x: &Value| -> Option<Lit> {
        let o = x.as_object()?;
        if o.len() != 2 || !same_field(&field_from_json(o.get("Left")?, aliases, None)?, field) {
            return None;
        }
        lit_from_expr(o.get("Right")?)
    };
    match k {
        "Comparison" => {
            let o = x.as_object()?;
            if o.len() != 3 || !same_field(&field_from_json(o.get("Left")?, aliases, None)?, field) {
                return None;
            }
            let lit = lit_from_expr(o.get("Right")?)?;
            let op = match o.get("ComparisonKind")?.as_u64()? {
                0 => Op::Eq,
                1 => Op::Gt,
                2 => Op::Ge,
                3 => Op::Lt,
                4 => Op::Le,
                _ => return None,
            };
            Some((op, lit))
        }
        "Contains" => Some((Op::Contains, binary(x)?)),
        "StartsWith" => Some((Op::StartsWith, binary(x)?)),
        "Not" => {
            let inner = x.as_object().filter(|o| o.len() == 1)?.get("Expression")?;
            match parse_condition(inner, field, aliases)? {
                (Op::Eq, l) => Some((Op::Ne, l)),
                (Op::Contains, l) => Some((Op::NotContains, l)),
                (Op::StartsWith, l) => Some((Op::NotStartsWith, l)),
                _ => None,
            }
        }
        _ => None,
    }
}

/// `{"In": {"Expressions": [field], "Values": [[lit]…]}}` on `field`.
fn parse_in(x: &Value, field: &FieldRef, aliases: &Aliases) -> Option<Vec<Lit>> {
    let o = x.as_object()?;
    if o.len() != 2 {
        return None;
    }
    let exprs = o.get("Expressions")?.as_array()?;
    if exprs.len() != 1 || !same_field(&field_from_json(exprs.first()?, aliases, None)?, field) {
        return None;
    }
    let mut values = Vec::new();
    for tuple in o.get("Values")?.as_array()? {
        let t = tuple.as_array()?;
        if t.len() != 1 {
            return None;
        }
        values.push(lit_from_expr(t.first()?)?);
    }
    if values.is_empty() {
        return None;
    }
    Some(values)
}

fn parse_top_n(filter: &Map<String, Value>, field: &FieldRef) -> Option<TypedKind> {
    let from = filter.get("From")?.as_array()?;
    let aliases = entity_aliases(from)?;
    let mut sub = None;
    for e in from {
        if let Some(expr) = e.get("Expression") {
            if sub.is_some() || e.get("Type").and_then(Value::as_u64) != Some(2) {
                return None;
            }
            let query = expr.get("Subquery")?.get("Query")?.as_object()?;
            sub = Some((e.get("Name")?.as_str()?, query));
        }
    }
    let (sub_name, query) = sub?;
    let wh = single_condition(filter)?;
    let (k, x) = only_key(wh)?;
    let o = x.as_object().filter(|_| k == "In")?;
    if o.len() != 2 {
        return None;
    }
    let exprs = o.get("Expressions")?.as_array()?;
    if exprs.len() != 1 || !same_field(&field_from_json(exprs.first()?, &aliases, None)?, field) {
        return None;
    }
    if o.get("Table")? != &json!({"SourceRef": {"Source": sub_name}}) {
        return None;
    }
    let inner = entity_aliases(query.get("From")?.as_array()?)?;
    let select = query.get("Select")?.as_array()?;
    if select.len() != 1 || !same_field(&field_from_json(select.first()?, &inner, Some("Name"))?, field) {
        return None;
    }
    let order = query.get("OrderBy")?.as_array()?;
    if order.len() != 1 {
        return None;
    }
    let ob = order.first()?.as_object()?;
    let bottom = match ob.get("Direction")?.as_u64()? {
        1 => true,
        2 => false,
        _ => return None,
    };
    let by = field_from_json(ob.get("Expression")?, &inner, None)?;
    let count = query
        .get("Top")?
        .as_f64()
        .filter(|n| n.fract() == 0.0 && *n >= 1.0)?;
    Some(TypedKind::TopN {
        count: count as u64,
        by,
        bottom,
    })
}

/// The condition of a filter's single `Where` clause.
fn single_condition(filter: &Map<String, Value>) -> Option<&Value> {
    let wh = filter.get("Where")?.as_array()?;
    if wh.len() != 1 {
        return None;
    }
    let w = wh.first()?.as_object()?;
    if w.len() != 1 {
        return None;
    }
    w.get("Condition")
}

/// A filter container in a shape bicraft writes, read back; `None` for anything else.
fn parse_container(c: &Value) -> Option<Typed> {
    let field = field_from_json(c.get("field")?, &Vec::new(), None)?;
    let filter = c.get("filter")?.as_object()?;
    if filter.get("Version").is_some_and(|v| v.as_u64() != Some(2)) {
        return None;
    }
    let kind = match c.get("type")?.as_str()? {
        "Categorical" => {
            let aliases = entity_aliases(filter.get("From")?.as_array()?)?;
            let (k, x) = only_key(single_condition(filter)?)?;
            match k {
                "In" => TypedKind::Basic {
                    values: parse_in(x, &field, &aliases)?,
                    exclude: false,
                },
                "Not" => {
                    let inner = x.as_object().filter(|o| o.len() == 1)?.get("Expression")?;
                    let (k2, x2) = only_key(inner)?;
                    if k2 != "In" {
                        return None;
                    }
                    TypedKind::Basic {
                        values: parse_in(x2, &field, &aliases)?,
                        exclude: true,
                    }
                }
                _ => return None,
            }
        }
        "Advanced" => {
            let aliases = entity_aliases(filter.get("From")?.as_array()?)?;
            let cond = single_condition(filter)?;
            let (k, x) = only_key(cond)?;
            if k == "And" || k == "Or" {
                let o = x.as_object().filter(|o| o.len() == 2)?;
                let a = parse_condition(o.get("Left")?, &field, &aliases)?;
                let b = parse_condition(o.get("Right")?, &field, &aliases)?;
                TypedKind::Advanced {
                    conditions: vec![a, b],
                    or: k == "Or",
                }
            } else {
                TypedKind::Advanced {
                    conditions: vec![parse_condition(cond, &field, &aliases)?],
                    or: false,
                }
            }
        }
        "TopN" => parse_top_n(filter, &field)?,
        ty @ ("RelativeDate" | "RelativeTime") => {
            let aliases = entity_aliases(filter.get("From")?.as_array()?)?;
            let is_field =
                |v: &Value| field_from_json(v, &aliases, None).is_some_and(|f| same_field(&f, &field));
            TypedKind::Relative(crate::filter_relative::parse_condition(
                single_condition(filter)?,
                ty,
                is_field,
            )?)
        }
        _ => return None,
    };
    Some(Typed { field, kind })
}

/// A short alias for `entity` (its first letter, like Desktop), not in `taken`.
fn alias_for(entity: &str, taken: &[String]) -> String {
    let base = entity
        .chars()
        .next()
        .filter(char::is_ascii_alphabetic)
        .map_or_else(|| "t".to_string(), |c| c.to_ascii_lowercase().to_string());
    if !taken.contains(&base) {
        return base;
    }
    let mut n: u32 = 1;
    loop {
        let a = format!("{base}{n}");
        if !taken.contains(&a) || n == u32::MAX {
            return a;
        }
        n = n.saturating_add(1);
    }
}

fn condition_json(op: Op, lit: &Lit, field: &Value) -> Value {
    let right = lit.expr();
    let base = match op {
        Op::Contains | Op::NotContains => json!({"Contains": {"Left": field, "Right": right}}),
        Op::StartsWith | Op::NotStartsWith => json!({"StartsWith": {"Left": field, "Right": right}}),
        _ => json!({"Comparison": {
            "ComparisonKind": op.comparison_kind().unwrap_or(0),
            "Left": field,
            "Right": right,
        }}),
    };
    match op {
        Op::Ne | Op::NotContains | Op::NotStartsWith => json!({"Not": {"Expression": base}}),
        _ => base,
    }
}

/// The PBIR `filter` definition of a typed filter.
fn render_filter(t: &Typed) -> Result<Value, String> {
    let table = field_table(&t.field).ok_or("internal: filter field has no table")?;
    let a = alias_for(table, &["subquery".to_string()]);
    let src = field_to_json(&t.field, Some(&a)).ok_or("internal: filter field cannot be written")?;
    let entity = json!({"Name": a, "Entity": table, "Type": 0});
    Ok(match &t.kind {
        TypedKind::Basic { values, exclude } => {
            let values: Vec<Value> = values.iter().map(|l| json!([l.expr()])).collect();
            let cond = json!({"In": {"Expressions": [src], "Values": values}});
            let cond = if *exclude {
                json!({"Not": {"Expression": cond}})
            } else {
                cond
            };
            json!({"Version": 2, "From": [entity], "Where": [{"Condition": cond}]})
        }
        TypedKind::Advanced { conditions, or } => {
            let mut parts = conditions.iter().map(|(op, l)| condition_json(*op, l, &src));
            let first = parts
                .next()
                .ok_or("internal: advanced filter without condition")?;
            let cond = match parts.next() {
                Some(second) => {
                    let key = if *or { "Or" } else { "And" };
                    json!({key: {"Left": first, "Right": second}})
                }
                None => first,
            };
            json!({"Version": 2, "From": [entity], "Where": [{"Condition": cond}]})
        }
        TypedKind::Relative(r) => {
            json!({"Version": 2, "From": [entity], "Where": [{"Condition": r.condition(&src)}]})
        }
        TypedKind::TopN { count, by, bottom } => {
            let by_table = field_table(by).ok_or("internal: top N measure has no table")?;
            let mut inner_from = vec![json!({"Name": a, "Entity": table, "Type": 0})];
            let b = if by_table.eq_ignore_ascii_case(table) {
                a.clone()
            } else {
                let b = alias_for(by_table, &[a.clone(), "subquery".to_string()]);
                inner_from.push(json!({"Name": b, "Entity": by_table, "Type": 0}));
                b
            };
            let mut select = src.clone();
            if let Some(o) = select.as_object_mut() {
                o.insert("Name".into(), json!("field"));
            }
            let by_json = field_to_json(by, Some(&b)).ok_or("internal: top N measure cannot be written")?;
            json!({
                "Version": 2,
                "From": [
                    {"Name": "subquery", "Expression": {"Subquery": {"Query": {
                        "Version": 2,
                        "From": inner_from,
                        "Select": [select],
                        "OrderBy": [{"Direction": if *bottom { 1 } else { 2 }, "Expression": by_json}],
                        "Top": count,
                    }}}, "Type": 2},
                    entity,
                ],
                "Where": [{"Condition": {"In": {
                    "Expressions": [src],
                    "Table": {"SourceRef": {"Source": "subquery"}},
                }}}],
            })
        }
    })
}

fn typed_json(t: &Typed) -> Value {
    let mut o = Map::new();
    o.insert("field".into(), json!(field_text(&t.field)));
    match &t.kind {
        TypedKind::Basic { values, exclude } => {
            let key = if *exclude { "not_in" } else { "in" };
            o.insert(
                key.into(),
                Value::Array(values.iter().map(Lit::to_json).collect()),
            );
        }
        TypedKind::Advanced { conditions, or } => {
            if let [(op, lit)] = conditions.as_slice() {
                o.insert("op".into(), json!(op.name()));
                o.insert("value".into(), lit.to_json());
            } else {
                let list: Vec<Value> = conditions
                    .iter()
                    .map(|(op, lit)| json!({"op": op.name(), "value": lit.to_json()}))
                    .collect();
                o.insert("conditions".into(), Value::Array(list));
                o.insert("logic".into(), json!(if *or { "or" } else { "and" }));
            }
        }
        TypedKind::TopN { count, by, bottom } => {
            o.insert("top".into(), json!(count));
            o.insert("by".into(), json!(field_text(by)));
            o.insert("direction".into(), json!(if *bottom { "bottom" } else { "top" }));
        }
        TypedKind::Relative(r) => {
            o.insert("relative".into(), r.to_json());
        }
    }
    Value::Object(o)
}

fn filter_desc(t: &Typed) -> String {
    match &t.kind {
        TypedKind::Basic { values, exclude } => format!(
            "{} {} {} value(s)",
            field_text(&t.field),
            if *exclude { "not in" } else { "in" },
            values.len()
        ),
        TypedKind::Advanced { conditions, or } => {
            let parts: Vec<String> = conditions
                .iter()
                .map(|(op, l)| format!("{} {}", op.name(), l.render()))
                .collect();
            format!(
                "{} {}",
                field_text(&t.field),
                parts.join(if *or { " or " } else { " and " })
            )
        }
        TypedKind::TopN { count, by, bottom } => format!(
            "{} {} {count} by {}",
            field_text(&t.field),
            if *bottom { "bottom" } else { "top" },
            field_text(by)
        ),
        TypedKind::Relative(r) => format!("{} {}", field_text(&t.field), r.describe()),
    }
}

fn parse_filter_item(
    item: &Value,
    i: usize,
    model: Option<&SemanticModel>,
    known: &[FieldRef],
) -> Result<FilterReq, String> {
    let what = format!("filters[{i}]");
    let o = as_object(item, &what)?;
    if let Some(raw) = o.get("raw") {
        check_keys(o, &["raw"], &what)?;
        let name = raw
            .as_object()
            .and_then(|r| r.get("name"))
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{what}.raw must be a PBIR filter object with a `name`"))?;
        if name.is_empty() || name.len() > 200 {
            return Err(format!("{what}.raw.name must be 1 to 200 characters"));
        }
        return Ok(FilterReq::Raw(raw.clone()));
    }
    check_keys(
        o,
        &[
            "field",
            "in",
            "not_in",
            "op",
            "value",
            "conditions",
            "logic",
            "top",
            "by",
            "direction",
            "relative",
            "hidden",
            "locked",
        ],
        &what,
    )?;
    let text = o
        .get("field")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{what}.field must be a field string like Product[Color] or [Total]"))?;
    let field = resolve(text, model, known).map_err(|e| format!("{what}: {e}"))?;
    let flag = |k: &str| -> Result<Option<bool>, String> {
        match o.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Bool(b)) => Ok(Some(*b)),
            Some(_) => Err(format!("{what}.{k} must be true or false")),
        }
    };
    let hidden = flag("hidden")?;
    let locked = flag("locked")?;
    let has = |k: &str| o.get(k).is_some_and(|v| !v.is_null());
    let shapes = [
        has("in") || has("not_in"),
        has("op") || has("conditions"),
        has("top"),
        has("relative"),
    ];
    if shapes.iter().filter(|s| **s).count() != 1 {
        return Err(format!(
            "{what} needs exactly one of `in` / `not_in` (basic), `op` + `value` or `conditions` (advanced), `top` + `by` (top N), `relative` (relative date or time)"
        ));
    }
    let kind = value_kind(model, &field);
    let lists = |k: &str| has(k);
    let typed_kind = if shapes[3] {
        if !matches!(field, FieldRef::Column { .. }) {
            return Err(format!(
                "{what}: relative date and time filters apply to date columns, not {}",
                field_text(&field)
            ));
        }
        if !matches!(kind, ValueKind::Date | ValueKind::Unknown) {
            return Err(format!(
                "{what}: {} is not a date column; relative date and time filters need one",
                field_text(&field)
            ));
        }
        let rel = o
            .get("relative")
            .ok_or_else(|| format!("{what}.relative is missing"))?;
        TypedKind::Relative(crate::filter_relative::parse_param(
            rel,
            &format!("{what}.relative"),
        )?)
    } else if shapes[0] {
        if lists("in") && lists("not_in") {
            return Err(format!("{what} has both `in` and `not_in`"));
        }
        if !matches!(field, FieldRef::Column { .. }) {
            return Err(format!(
                "{what}: basic filters (`in` / `not_in`) apply to columns; filter {} with `op` and `value`",
                field_text(&field)
            ));
        }
        let exclude = lists("not_in");
        let key = if exclude { "not_in" } else { "in" };
        let arr = o
            .get(key)
            .and_then(Value::as_array)
            .ok_or_else(|| format!("{what}.{key} must be a list of values"))?;
        if arr.is_empty() || arr.len() > MAX_FILTER_VALUES {
            return Err(format!("{what}.{key} must hold 1 to {MAX_FILTER_VALUES} values"));
        }
        let mut values: Vec<Lit> = Vec::with_capacity(arr.len());
        for (j, x) in arr.iter().enumerate() {
            let lit = to_lit(x, kind, &format!("{what}.{key}[{j}]"))?;
            if !values.iter().any(|l| l.same(&lit)) {
                values.push(lit);
            }
        }
        TypedKind::Basic { values, exclude }
    } else if shapes[1] {
        let conds: Vec<(Option<&Value>, Option<&Value>, String)> = if has("conditions") {
            if has("op") || has("value") {
                return Err(format!("{what} has both `conditions` and `op` / `value`"));
            }
            let arr = o
                .get("conditions")
                .and_then(Value::as_array)
                .ok_or_else(|| format!("{what}.conditions must be a list of {{op, value}}"))?;
            if arr.is_empty() || arr.len() > MAX_CONDITIONS {
                return Err(format!(
                    "{what}.conditions must hold 1 or {MAX_CONDITIONS} conditions"
                ));
            }
            let mut out = Vec::new();
            for (j, c) in arr.iter().enumerate() {
                let w = format!("{what}.conditions[{j}]");
                let co = as_object(c, &w)?;
                check_keys(co, &["op", "value"], &w)?;
                out.push((co.get("op"), co.get("value"), w));
            }
            out
        } else {
            vec![(o.get("op"), o.get("value"), what.clone())]
        };
        let mut conditions = Vec::new();
        for (op, value, w) in conds {
            let op_text = op
                .and_then(Value::as_str)
                .ok_or_else(|| format!("{w}.op must be a string"))?;
            let op = OPS
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(op_text))
                .map(|(_, o)| *o)
                .ok_or_else(|| {
                    format!(
                        "{w}.op `{op_text}` is not one of {}",
                        OPS.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")
                    )
                })?;
            let value = value.ok_or_else(|| format!("{w}.value is missing (null means blank)"))?;
            let lit = if op.comparison_kind().is_none() {
                match value {
                    Value::String(s) => {
                        check_text(s, &format!("{w}.value"))?;
                        Lit::Text(s.clone())
                    }
                    _ => return Err(format!("{w}: `{op_text}` needs a text value")),
                }
            } else {
                to_lit(value, kind, &format!("{w}.value"))?
            };
            if lit == Lit::Null && !matches!(op, Op::Eq | Op::Ne) {
                return Err(format!("{w}: null (blank) only works with `=` and `<>`"));
            }
            conditions.push((op, lit));
        }
        let or = match o.get("logic") {
            None | Some(Value::Null) => false,
            Some(Value::String(l)) if l.eq_ignore_ascii_case("and") => false,
            Some(Value::String(l)) if l.eq_ignore_ascii_case("or") => true,
            Some(_) => return Err(format!("{what}.logic must be \"and\" or \"or\"")),
        };
        TypedKind::Advanced { conditions, or }
    } else {
        if !matches!(field, FieldRef::Column { .. }) {
            return Err(format!(
                "{what}: a top N filter keeps values of a column, not {}",
                field_text(&field)
            ));
        }
        let count = o
            .get("top")
            .and_then(Value::as_u64)
            .filter(|n| (1..=MAX_TOP_N).contains(n))
            .ok_or_else(|| format!("{what}.top must be a whole number from 1 to {MAX_TOP_N}"))?;
        let by_text = o.get("by").and_then(Value::as_str).ok_or_else(|| {
            format!("{what}.by must be a measure like [Total] or an aggregation like Sum(Sales[Amount])")
        })?;
        let by = resolve(by_text, model, known).map_err(|e| format!("{what}.by: {e}"))?;
        if matches!(by, FieldRef::Column { .. }) {
            return Err(format!(
                "{what}.by: rank by a measure or an aggregation like Sum({}), not a column",
                field_text(&by)
            ));
        }
        let bottom = match o.get("direction") {
            None | Some(Value::Null) => false,
            Some(Value::String(d)) if d.eq_ignore_ascii_case("top") => false,
            Some(Value::String(d)) if d.eq_ignore_ascii_case("bottom") => true,
            Some(_) => return Err(format!("{what}.direction must be \"top\" or \"bottom\"")),
        };
        TypedKind::TopN { count, by, bottom }
    };
    for k in ["value", "logic"] {
        if has(k) && !shapes[1] {
            return Err(format!("{what}.{k} only applies to advanced filters"));
        }
    }
    for k in ["by", "direction"] {
        if has(k) && !shapes[2] {
            return Err(format!("{what}.{k} only applies to top N filters"));
        }
    }
    Ok(FilterReq::Typed {
        typed: Typed {
            field,
            kind: typed_kind,
        },
        hidden,
        locked,
    })
}

pub(crate) fn parse_filters(
    v: &Value,
    model: Option<&SemanticModel>,
    known: &[FieldRef],
) -> Result<Vec<FilterReq>, String> {
    let items = match v {
        Value::Null => return Ok(Vec::new()),
        Value::Array(a) => a,
        _ => return Err("`filters` must be a list (an empty list removes every filter)".into()),
    };
    if items.len() > MAX_FILTERS {
        return Err(format!("`filters` has more than {MAX_FILTERS} filters"));
    }
    let mut out: Vec<FilterReq> = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let req = parse_filter_item(item, i, model, known)?;
        let dup = out.iter().any(|r| match (r, &req) {
            (FilterReq::Typed { typed: a, .. }, FilterReq::Typed { typed: b, .. }) => {
                same_field(&a.field, &b.field)
            }
            (FilterReq::Raw(a), FilterReq::Raw(b)) => a.get("name") == b.get("name"),
            _ => false,
        });
        if dup {
            return Err(format!(
                "filters[{i}] targets the same field or name as an earlier filter"
            ));
        }
        out.push(req);
    }
    Ok(out)
}

/// A fresh 20-hex-digit filter name not in `taken`.
pub(crate) fn fresh_name(taken: &[String]) -> String {
    for _ in 0..64 {
        let hex: String = new_guid()
            .chars()
            .filter(char::is_ascii_hexdigit)
            .take(20)
            .collect();
        if hex.len() == 20 && !taken.iter().any(|t| t.eq_ignore_ascii_case(&hex)) {
            return hex;
        }
    }
    let mut n: u64 = 0;
    loop {
        let name = format!("{n:020x}");
        if !taken.iter().any(|t| t.eq_ignore_ascii_case(&name)) || n == u64::MAX {
            return name;
        }
        n = n.saturating_add(1);
    }
}

fn container_label(c: &Value) -> String {
    match parse_container(c) {
        Some(t) => field_text(&t.field),
        None => match c.get("field").and_then(|f| field_from_json(f, &Vec::new(), None)) {
            Some(f) => field_text(&f),
            None => format!("`{}`", c.get("name").and_then(Value::as_str).unwrap_or("?")),
        },
    }
}

/// Sets a view-mode flag; an absent flag counts as `false`.
fn set_flag(c: &mut Map<String, Value>, key: &str, value: Option<bool>) -> bool {
    let Some(b) = value else { return false };
    let unchanged = match c.get(key) {
        None => !b,
        Some(x) => x.as_bool() == Some(b),
    };
    if unchanged {
        return false;
    }
    insert_ordered(c, key, json!(b), FILTER_ORDER);
    true
}

pub(crate) fn apply_filters(
    filter_config: &mut Option<Value>,
    reqs: &[FilterReq],
    changes: &mut Vec<String>,
    owner: &str,
) -> Result<(), String> {
    let old: Vec<Value> = match &*filter_config {
        None => Vec::new(),
        Some(cfg) => match cfg.get("filters") {
            None => Vec::new(),
            Some(Value::Array(a)) => a.clone(),
            Some(_) => {
                return Err(format!(
                    "{owner} filterConfig.filters is not a list; it cannot be edited"
                ));
            }
        },
    };
    if filter_config.as_ref().is_some_and(|c| !c.is_object()) {
        return Err(format!(
            "{owner} filterConfig is not an object; it cannot be edited"
        ));
    }
    // Match each request to an existing container.
    let parsed: Vec<Option<Typed>> = old.iter().map(parse_container).collect();
    let mut claimed: Vec<Option<usize>> = Vec::with_capacity(reqs.len());
    for req in reqs {
        let idx = match req {
            FilterReq::Raw(r) => old.iter().position(|c| c.get("name") == r.get("name")),
            FilterReq::Typed { typed, .. } => {
                let same = |c: &Value| {
                    c.get("field")
                        .and_then(|f| field_from_json(f, &Vec::new(), None))
                        .is_some_and(|f| same_field(&f, &typed.field))
                };
                old.iter()
                    .position(|c| same(c) && c.get("filter").is_some())
                    .or_else(|| old.iter().position(same))
            }
        };
        if let Some(i) = idx
            && claimed.contains(&Some(i))
        {
            return Err(format!(
                "two filters in the list target {owner} filter on {}",
                old.get(i).map(container_label).unwrap_or_default()
            ));
        }
        claimed.push(idx);
    }
    let mut slots: Vec<Option<Value>> = old.iter().cloned().map(Some).collect();
    // Updates in place.
    for (req, idx) in reqs.iter().zip(&claimed) {
        let Some(i) = *idx else { continue };
        let Some(Some(slot)) = slots.get_mut(i) else {
            continue;
        };
        match req {
            FilterReq::Raw(r) => {
                if slot != r {
                    *slot = r.clone();
                    changes.push(format!("filter {} replaced", container_label(r)));
                }
            }
            FilterReq::Typed {
                typed,
                hidden,
                locked,
            } => {
                let same = parsed
                    .get(i)
                    .and_then(Option::as_ref)
                    .is_some_and(|p| same_field(&p.field, &typed.field) && p.kind.same(&typed.kind));
                let c = slot
                    .as_object_mut()
                    .ok_or_else(|| format!("a filter of {owner} is not an object; it cannot be edited"))?;
                let mut changed = false;
                if !same {
                    insert_ordered(c, "type", json!(typed.kind.type_name()), FILTER_ORDER);
                    insert_ordered(c, "filter", render_filter(typed)?, FILTER_ORDER);
                    changed = true;
                }
                changed |= set_flag(c, "isHiddenInViewMode", *hidden);
                changed |= set_flag(c, "isLockedInViewMode", *locked);
                if changed {
                    changes.push(format!("filter {}", filter_desc(typed)));
                }
            }
        }
    }
    // Containers no request names: drop their condition.
    for (i, slot) in slots.iter_mut().enumerate() {
        if claimed.contains(&Some(i)) {
            continue;
        }
        let Some(c) = slot.as_mut() else { continue };
        if c.get("filter").is_none() {
            continue;
        }
        let label = container_label(c);
        let auto = c.get("howCreated").and_then(Value::as_str) == Some("Auto");
        if auto && let Some(o) = c.as_object_mut() {
            o.shift_remove("filter");
            changes.push(format!("filter on {label} cleared"));
        } else {
            *slot = None;
            changes.push(format!("filter on {label} removed"));
        }
    }
    let mut list: Vec<Value> = slots.into_iter().flatten().collect();
    // New filters.
    let mut taken: Vec<String> = list
        .iter()
        .filter_map(|c| c.get("name").and_then(Value::as_str).map(str::to_string))
        .collect();
    for (req, idx) in reqs.iter().zip(&claimed) {
        if idx.is_some() {
            continue;
        }
        match req {
            FilterReq::Raw(r) => {
                changes.push(format!("filter {} added", container_label(r)));
                list.push(r.clone());
            }
            FilterReq::Typed {
                typed,
                hidden,
                locked,
            } => {
                let name = fresh_name(&taken);
                taken.push(name.clone());
                let mut c = Map::new();
                c.insert("name".into(), json!(name));
                c.insert(
                    "field".into(),
                    field_to_json(&typed.field, None).ok_or("internal: filter field cannot be written")?,
                );
                c.insert("type".into(), json!(typed.kind.type_name()));
                c.insert("filter".into(), render_filter(typed)?);
                c.insert("howCreated".into(), json!("User"));
                if let Some(h) = hidden {
                    c.insert("isHiddenInViewMode".into(), json!(h));
                }
                if let Some(l) = locked {
                    c.insert("isLockedInViewMode".into(), json!(l));
                }
                changes.push(format!("filter {} added", filter_desc(typed)));
                list.push(Value::Object(c));
            }
        }
    }
    if list == old {
        return Ok(());
    }
    let mut cfg = filter_config
        .as_ref()
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if list.is_empty() {
        cfg.shift_remove("filters");
    } else {
        insert_ordered(
            &mut cfg,
            "filters",
            Value::Array(list),
            &["$schema", "filters", "filterSortOrder"],
        );
    }
    *filter_config = if cfg.is_empty() {
        None
    } else {
        Some(Value::Object(cfg))
    };
    Ok(())
}

pub(crate) fn extract_filters(cfg: Option<&Value>) -> Option<Value> {
    let list = filter_list(cfg)?;
    let mut out = Vec::new();
    for c in list {
        if c.get("filter").is_none() {
            continue;
        }
        match parse_container(c) {
            Some(t) => {
                let mut item = typed_json(&t);
                if let Some(o) = item.as_object_mut() {
                    for (key, friendly) in
                        [("isHiddenInViewMode", "hidden"), ("isLockedInViewMode", "locked")]
                    {
                        if let Some(b) = c.get(key).and_then(Value::as_bool) {
                            o.insert(friendly.into(), json!(b));
                        }
                    }
                }
                out.push(item);
            }
            None => out.push(json!({"raw": c})),
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(Value::Array(out))
    }
}

/// One filter card of a `filterConfig` as the snapshot lists it (docs/view-snapshot.md, "Filters
/// pane"): `{name, field, kind, table, column | measure, function, type, condition, how_created,
/// hidden, locked}`. `type` is `basic`, `advanced`, `topn` or `relative` for a condition BIcraft reads,
/// `none` for a card without one, `other` for one it does not (kept as written); `condition` is
/// the condition in `visual.filter`'s parameters (`in` / `not_in`, `op` + `value` or
/// `conditions` + `logic`, `top` + `by` + `direction`, `relative`).
pub(crate) fn card_json(c: &Value) -> Value {
    let field = c.get("field").and_then(|f| field_from_json(f, &Vec::new(), None));
    let mut o = Map::new();
    o.insert("name".into(), json!(c.get("name").and_then(Value::as_str)));
    o.insert("field".into(), json!(field.as_ref().map(field_text)));
    let (kind, table, name, function) = match &field {
        Some(FieldRef::Column { table, column }) => ("column", Some(table), Some(column), None),
        Some(FieldRef::Measure { table, measure }) => ("measure", Some(table), Some(measure), None),
        Some(FieldRef::Aggregation {
            table,
            column,
            function,
        }) => ("aggregation", Some(table), Some(column), Some(function)),
        _ => ("other", None, None, None),
    };
    o.insert("kind".into(), json!(kind));
    o.insert("table".into(), json!(table));
    let key = if kind == "measure" { "measure" } else { "column" };
    o.insert(key.into(), json!(name));
    o.insert("function".into(), json!(function));
    let (ty, condition) = match (c.get("filter"), parse_container(c)) {
        (None, _) => ("none", Value::Null),
        (Some(_), Some(t)) => {
            let mut j = typed_json(&t);
            if let Some(m) = j.as_object_mut() {
                m.shift_remove("field");
            }
            let ty = match t.kind {
                TypedKind::Basic { .. } => "basic",
                TypedKind::Advanced { .. } => "advanced",
                TypedKind::TopN { .. } => "topn",
                TypedKind::Relative(_) => "relative",
            };
            (ty, j)
        }
        (Some(_), None) => ("other", Value::Null),
    };
    o.insert("type".into(), json!(ty));
    o.insert("condition".into(), condition);
    o.insert(
        "how_created".into(),
        json!(c.get("howCreated").and_then(Value::as_str)),
    );
    for (key, friendly) in [("isHiddenInViewMode", "hidden"), ("isLockedInViewMode", "locked")] {
        o.insert(
            friendly.into(),
            json!(c.get(key).and_then(Value::as_bool).unwrap_or(false)),
        );
    }
    Value::Object(o)
}

// ---------------------------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------------------------

/// Applies `props` to `visual`. `props` is an object whose keys are defined and documented by
/// this module (at least `title`, `sort`, `filters`, `format`); unknown keys are an error.
/// Fields are resolved against `model` when it is present. Validates everything before touching
/// `visual`, and returns a short description of each change made (empty when nothing changed).
pub(crate) fn apply_props(
    visual: &mut Visual,
    props: &Value,
    model: Option<&SemanticModel>,
) -> Result<Vec<String>, String> {
    let o = match props {
        Value::Null => return Ok(Vec::new()),
        Value::Object(o) => o,
        _ => return Err("visual props must be an object {title?, sort?, filters?, format?}".into()),
    };
    check_keys(
        o,
        &[
            "title",
            "sort",
            "filters",
            "format",
            "series",
            "colors",
            "conditional",
            // visual_extras.rs
            "axes",
            "analytics",
            "small_multiples",
            "tooltip_page",
        ],
        "visual props",
    )?;
    if o.is_empty() {
        return Ok(Vec::new());
    }
    if visual.visual_type.is_none() {
        return Err(format!(
            "`{}` is a visual group; groups have no title, sort, filters or formatting here",
            visual.name
        ));
    }
    let known = known_fields(visual);
    let title = o
        .get("title")
        .map(|t| group_edits(&TITLE, t, "title"))
        .transpose()?;
    let sort = o
        .get("sort")
        .map(|s| parse_sort(s, model, visual, &known))
        .transpose()?;
    let filters = o
        .get("filters")
        .map(|f| parse_filters(f, model, &known))
        .transpose()?;
    let format = o.get("format").map(parse_format).transpose()?;
    // Per-series formatting and conditional formatting (visual_series.rs, visual_conditional.rs).
    let extras = crate::visual_extras::parse(o, visual, model)?;
    let mut entry_ops = crate::visual_series::parse(o, visual, model)?;
    let cf = crate::visual_conditional::parse(o, visual, model)?;
    entry_ops.extend(cf.objects);

    let mut v = visual.clone();
    let mut changes = Vec::new();
    if let Some(edits) = &title {
        apply_edits(&mut v, edits, &mut changes)?;
    }
    if let Some(sort) = sort {
        apply_sort(&mut v, sort, &mut changes)?;
    }
    if let Some(reqs) = &filters {
        apply_filters(&mut v.filter_config, reqs, &mut changes, "the visual's")?;
    }
    if let Some(edits) = &format {
        apply_edits(&mut v, edits, &mut changes)?;
    }
    crate::visual_series::apply_ops(&mut v, &entry_ops, true, &mut changes)?;
    crate::visual_series::apply_ops_in(
        &mut v,
        "visualContainerObjects",
        &cf.container,
        false,
        &mut changes,
    )?;
    crate::visual_extras::apply(&mut v, &extras, &mut changes)?;
    if !changes.is_empty() {
        *visual = v;
    }
    Ok(changes)
}

/// The properties of `visual` in the shape [`apply_props`] accepts, so that
/// `apply_props(v, &extract_props(v), m)` changes nothing. Keys with no value are omitted.
pub(crate) fn extract_props(visual: &Visual) -> Value {
    let mut out = Map::new();
    if visual.visual_type.is_none() {
        return Value::Object(out);
    }
    if let Some(t) = extract_group(visual, &TITLE) {
        out.insert("title".into(), t);
    }
    if let Some(Ok(sorts)) = current_sort(visual)
        && !sorts.is_empty()
    {
        let list: Vec<Value> = sorts
            .iter()
            .map(|(f, d)| json!({"field": field_text(f), "direction": if *d { "desc" } else { "asc" }}))
            .collect();
        let sort = match <[Value; 1]>::try_from(list) {
            Ok([one]) => one,
            Err(list) => Value::Array(list),
        };
        out.insert("sort".into(), sort);
    }
    if let Some(f) = extract_filters(visual.filter_config.as_ref()) {
        out.insert("filters".into(), f);
    }
    let mut format = Map::new();
    for g in GROUPS {
        if let Some(x) = extract_group(visual, g) {
            format.insert(g.key.into(), x);
        }
    }
    let mut objects = extract_plain_objects(visual);
    if let Some(items) = extract_state_objects(visual) {
        objects.extend(items.as_array().cloned().unwrap_or_default());
    }
    if !objects.is_empty() {
        format.insert("objects".into(), Value::Array(objects));
    }
    if !format.is_empty() {
        out.insert("format".into(), Value::Object(format));
    }
    crate::visual_series::extract(visual, &mut out);
    crate::visual_conditional::extract(visual, &mut out);
    crate::visual_extras::extract(visual, &mut out);
    Value::Object(out)
}

// ---------------------------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------------------------

/// Looks up `{page, visual}`, builds the props with `build`, applies them and reports.
pub(crate) fn edit_visual(
    s: &mut Session,
    p: &Value,
    build: impl FnOnce(&Visual, Option<&SemanticModel>) -> Result<Value, String>,
) -> Result<Value, String> {
    let page_key = str_param(p, "page")?;
    let visual_key = str_param(p, "visual")?;
    let (model, r) = parts(s)?;
    let pi = page_index(r, page_key)?;
    let page = page_at_mut(r, pi)?;
    let vi = visual_index(page, visual_key)?;
    let page_name = page.name.clone();
    let v = page
        .visuals
        .get_mut(vi)
        .ok_or("internal: visual index out of range")?;
    let props = build(v, model)?;
    let changes = apply_props(v, &props, model)?;
    let changed = !changes.is_empty();
    let formats = props.get("format").is_some() || props.get("title").is_some();
    let warnings: Vec<String> = if changed && formats {
        crate::visual_catalog::unknown_type_warning(v.visual_type.as_deref())
            .into_iter()
            .collect()
    } else {
        Vec::new()
    };
    let out = json!({
        "page": page_name,
        "visual": v.name,
        "changed": changed,
        "changes": changes,
        "props": extract_props(v),
        "warnings": warnings,
    });
    if changed {
        s.report_dirty = true;
    }
    Ok(out)
}

/// The title properties `visual.set_title` also takes flat, beside `title` (its parameters form:
/// a free text field for the text and one field per property).
const TITLE_FLAT: &[&str] = &[
    "show",
    "font_size",
    "color",
    "alignment",
    "bold",
    "italic",
    "underline",
    "font_family",
];

pub(crate) fn visual_set_title(s: &mut Session, p: &Value) -> Result<Value, String> {
    let mut keys = vec!["page", "visual", "title"];
    keys.extend_from_slice(TITLE_FLAT);
    allow(p, &keys)?;
    let mut flat = Map::new();
    for k in TITLE_FLAT {
        if let Some(v) = p.get(*k) {
            flat.insert((*k).to_string(), v.clone());
        }
    }
    let title = match (p.get("title"), flat.is_empty()) {
        (None, true) => {
            return Err(
                "missing parameter `title` (a string, an object, or null to remove it) or one of \
                 show, font_size, color, alignment, bold, italic, underline, font_family"
                    .into(),
            );
        }
        (None, false) => Value::Object(flat),
        (Some(t), true) => t.clone(),
        // the text with flat properties: the text is set, the title shown unless `show` says
        (Some(Value::String(text)), false) => {
            flat.insert("text".into(), json!(text));
            flat.entry("show").or_insert(json!(true));
            Value::Object(flat)
        }
        (Some(Value::Object(o)), false) => {
            let mut o = o.clone();
            for (k, v) in flat {
                if o.contains_key(&k) {
                    return Err(format!("`{k}` is given twice: in `title` and beside it"));
                }
                o.insert(k, v);
            }
            Value::Object(o)
        }
        (Some(_), false) => {
            return Err("`title` must be a string or an object when other title properties are given".into());
        }
    };
    edit_visual(s, p, |_, _| Ok(json!({ "title": title })))
}

pub(crate) fn visual_set_sort(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "visual", "field", "direction"])?;
    let sort = match p.get("field") {
        None => return Err("missing parameter `field` (a field string, or null to remove the sort)".into()),
        Some(Value::Null) => {
            if p.get("direction").is_some_and(|d| !d.is_null()) {
                return Err("`direction` needs a `field`".into());
            }
            Value::Null
        }
        Some(f) => {
            let mut o = Map::new();
            o.insert("field".into(), f.clone());
            if let Some(d) = p.get("direction") {
                o.insert("direction".into(), d.clone());
            }
            Value::Object(o)
        }
    };
    edit_visual(s, p, |_, _| Ok(json!({ "sort": sort })))
}

/// The field a `filters` item (typed or raw) targets.
fn item_field(item: &Value, model: Option<&SemanticModel>, known: &[FieldRef]) -> Option<FieldRef> {
    if let Some(raw) = item.get("raw") {
        return field_from_json(raw.get("field")?, &Vec::new(), None);
    }
    resolve(item.get("field")?.as_str()?, model, known).ok()
}

/// The filter list of `cfg` (in `filters` items) without the filters on `field` (all of them
/// when `None`), and how many were dropped.
pub(crate) fn filters_without(
    cfg: Option<&Value>,
    model: Option<&SemanticModel>,
    known: &[FieldRef],
    field: Option<&FieldRef>,
) -> (Vec<Value>, usize) {
    let current = extract_filters(cfg)
        .and_then(|f| f.as_array().cloned())
        .unwrap_or_default();
    let Some(field) = field else {
        let n = current.len();
        return (Vec::new(), n);
    };
    let before = current.len();
    let kept: Vec<Value> = current
        .into_iter()
        .filter(|item| !item_field(item, model, known).is_some_and(|f| same_field(&f, field)))
        .collect();
    let dropped = before.saturating_sub(kept.len());
    (kept, dropped)
}

/// The parameters of `visual.filter` / `page.filter` / `report.filter` besides the target.
pub(crate) const FILTER_PARAMS: &[&str] = &[
    "field",
    "in",
    "not_in",
    "op",
    "value",
    "conditions",
    "logic",
    "top",
    "by",
    "direction",
    "relative",
    "hidden",
    "locked",
];

/// Whether a `*.filter` call states a condition (else it adds an empty filter card).
pub(crate) fn has_condition(p: &Value) -> bool {
    ["in", "not_in", "op", "conditions", "top", "relative"]
        .iter()
        .any(|k| p.get(*k).is_some_and(|v| !v.is_null()))
}

pub(crate) fn visual_filter(s: &mut Session, p: &Value) -> Result<Value, String> {
    let mut keys = vec!["page", "visual"];
    keys.extend_from_slice(FILTER_PARAMS);
    allow(p, &keys)?;
    if !has_condition(p) {
        return crate::filter_edit::filter_card(s, p, crate::filter_edit::Scope::Visual);
    }
    let mut item = p.as_object().cloned().unwrap_or_default();
    item.shift_remove("page");
    item.shift_remove("visual");
    let item = Value::Object(item);
    edit_visual(s, p, |v, model| {
        let known = known_fields(v);
        let text = item
            .get("field")
            .and_then(Value::as_str)
            .ok_or("missing string parameter `field`")?;
        let field = resolve(text, model, &known)?;
        let (mut list, _) = filters_without(v.filter_config.as_ref(), model, &known, Some(&field));
        list.push(item.clone());
        Ok(json!({ "filters": list }))
    })
}

pub(crate) fn visual_clear_filters(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "visual", "field", "keep_cards"])?;
    let text = p.get("field").and_then(Value::as_str).map(str::to_string);
    if p.get("field").is_some_and(|f| !f.is_null() && !f.is_string()) {
        return Err("parameter `field` must be a string".into());
    }
    let keep = crate::params::opt_bool(p, "keep_cards")?.unwrap_or(false);
    if keep {
        return crate::filter_edit::clear_cards_of(s, p, crate::filter_edit::Scope::Visual, true);
    }
    if let Some(t) = &text {
        // a card without condition (an empty one added from the Filters pane) is removed as
        // the page's and the report's are
        let r = crate::report::report_ref(s)?;
        let pi = page_index(r, str_param(p, "page")?)?;
        let page = r.pages.get(pi).ok_or("internal: page index out of range")?;
        let v = page
            .visuals
            .get(visual_index(page, str_param(p, "visual")?)?)
            .ok_or("internal: visual index out of range")?;
        let known = known_fields(v);
        let Ok(field) = resolve(t, s.model.as_ref(), &known) else {
            // a field the model no longer has: its card is found by name (model.analyze's fix)
            return crate::filter_edit::clear_cards_of(s, p, crate::filter_edit::Scope::Visual, false);
        };
        let (_, dropped) = filters_without(v.filter_config.as_ref(), s.model.as_ref(), &known, Some(&field));
        if dropped == 0 {
            return crate::filter_edit::clear_cards_of(s, p, crate::filter_edit::Scope::Visual, false);
        }
    }
    edit_visual(s, p, |v, model| {
        let known = known_fields(v);
        let field = match &text {
            Some(t) => Some(resolve(t, model, &known)?),
            None => None,
        };
        let (list, dropped) = filters_without(v.filter_config.as_ref(), model, &known, field.as_ref());
        if dropped == 0
            && let Some(f) = &field
        {
            return Err(format!("the visual has no filter on {}", field_text(f)));
        }
        Ok(json!({ "filters": list }))
    })
}

pub(crate) fn visual_format(s: &mut Session, p: &Value) -> Result<Value, String> {
    let mut format = p.as_object().cloned().ok_or("parameters must be a JSON object")?;
    format.shift_remove("page");
    format.shift_remove("visual");
    if format.is_empty() {
        return Err("nothing to format: pass background, border, padding, shadow, header, alt_text, subtitle, data_labels, legend, x_axis, y_axis, objects, series, colors or conditional".into());
    }
    // `series`, `colors` and `conditional` are props keys of their own.
    let mut props = Map::new();
    for k in ["series", "colors", "conditional"] {
        if let Some(x) = format.shift_remove(k) {
            props.insert(k.into(), x);
        }
    }
    if !format.is_empty() {
        props.insert("format".into(), Value::Object(format));
    }
    edit_visual(s, p, |_, _| Ok(Value::Object(props)))
}

pub(crate) fn visual_props(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "visual", "layout"])?;
    let page_key = str_param(p, "page")?;
    let visual_key = str_param(p, "visual")?;
    let mobile = match p.get("layout") {
        None | Some(Value::Null) => false,
        Some(Value::String(l)) if l == "desktop" => false,
        Some(Value::String(l)) if l == "mobile" => true,
        Some(_) => return Err("`layout` must be \"desktop\" or \"mobile\"".into()),
    };
    let r = crate::report::report_ref(s)?;
    let pi = page_index(r, page_key)?;
    let page = r.pages.get(pi).ok_or("internal: page index out of range")?;
    let vi = visual_index(page, visual_key)?;
    let v = page
        .visuals
        .get(vi)
        .ok_or("internal: visual index out of range")?;
    if !mobile {
        return Ok(json!({
            "page": page.name,
            "visual": v.name,
            "visual_type": v.visual_type,
            "props": extract_props(v),
        }));
    }
    // the props the phone layout draws: the visual's with its mobile formatting over them
    let doc = crate::visual_extras::mobile_doc(r, &page.name, &v.name)
        .ok_or_else(|| format!("`{}` is not on the mobile layout", v.name))?;
    let merged = crate::visual_extras::merged_visual(v, &doc);
    let objects: Vec<String> = ["objects", "visualContainerObjects"]
        .iter()
        .filter_map(|k| doc.get(*k).and_then(Value::as_object))
        .flat_map(|o| o.keys().cloned())
        .collect();
    Ok(json!({
        "page": page.name,
        "visual": v.name,
        "visual_type": v.visual_type,
        "props": extract_props(&merged),
        "layout": "mobile",
        "mobile_objects": objects,
    }))
}
