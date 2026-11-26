//! The BIcraft Spec visual's chart spec (ROADMAP M11.32, docs/formats/spec-visual.md): a
//! declarative JSON document an agent writes (marks, encodings from the visual's data roles,
//! scales, axes, legend), parsed and checked here, and laid out into drawing primitives.
//!
//! The same layout is written in TypeScript for the custom visual that runs inside Power BI
//! (`crates/engine/src/spec_visual/spec.ts`): both compute the same primitives from the same spec
//! and data, which the engine's golden tests compare (`tests/report/spec_visual_tests.rs`). Any
//! change here goes to the TypeScript too. The arithmetic is kept to what both languages compute
//! alike (IEEE doubles, `floor`, our own number and date formatting); text widths are estimated
//! from the character count, never measured.
//!
//! The spec is hostile input: its text is capped ([`MAX_SPEC_BYTES`]), every list and string is
//! capped, unknown keys are refused with their path, and the layout bounds its loops.

use serde_json::{Map, Value, json};

use crate::canvas::{Anchor, Base, Canvas, Rgb, Text, corner_rect_path};
use crate::charts::{Bx, font};
use crate::data::{Ctx, Dataset};

/// The spec version this code reads.
pub const SPEC_VERSION: u64 = 1;
/// Longest spec text.
pub const MAX_SPEC_BYTES: usize = 32 * 1024;
/// Most marks of a spec.
pub const MAX_MARKS: usize = 16;
/// Most measures a spec refers to (`value.0` … `value.15`).
pub const MAX_VALUES: usize = 16;
/// Most rows laid out (the visual's data reduction keeps 1,000 categories).
pub const MAX_ROWS: usize = 1000;
/// Most colours of a palette.
pub const MAX_PALETTE: usize = 32;
/// Longest name, title or text constant.
pub const MAX_TEXT: usize = 200;
/// The mark types.
pub const MARK_TYPES: &[&str] = &["bar", "line", "point", "area", "text", "rule", "rect"];
/// The default palette (our own colours).
pub const DEFAULT_PALETTE: [&str; 8] = [
    "#1F6F8B", "#E07A1F", "#5A9E3A", "#C23B5A", "#7A5BA8", "#2E9C9C", "#B08A1E", "#6B6B6B",
];
/// The default colour range of a colour encoding by value.
const VALUE_RANGE: [&str; 2] = ["#D6E6EE", "#1F6F8B"];
/// The colour of a palette slot that is missing (never drawn: palettes are not empty).
const TEXT: &str = "#555555";
const MS_PER_DAY: f64 = 86_400_000.0;
/// Most axis ticks.
const MAX_TICKS: usize = 50;
/// Most points of a line drawn with a marker on each (Modern and Bold).
const MAX_LINE_DOTS: usize = 40;

// ---------------------------------------------------------------------------------------------
// The spec
// ---------------------------------------------------------------------------------------------

/// A data field: the category, or the measure at an index of the `values` role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ref {
    Category,
    Value(usize),
}

/// A position encoding: a field, a number or a text (a category's label, an ISO date).
#[derive(Debug, Clone, PartialEq)]
pub enum Enc {
    Field(Ref),
    Num(f64),
    Text(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Color {
    Fixed(String),
    ByCategory,
    ByValue(Ref, String, String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Size {
    Fixed(f64),
    ByValue(Ref, f64, f64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkType {
    Bar,
    Line,
    Point,
    Area,
    Text,
    Rule,
    Rect,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    pub kind: MarkType,
    pub name: Option<String>,
    pub x: Option<Enc>,
    pub y: Option<Enc>,
    pub x2: Option<Enc>,
    pub y2: Option<Enc>,
    pub color: Option<Color>,
    pub size: Option<Size>,
    pub label: Option<Ref>,
    pub opacity: f64,
    pub decimals: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScaleKind {
    Band,
    Linear,
    Time,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Axis {
    pub scale: ScaleKind,
    pub axis: bool,
    pub title: Option<String>,
    pub grid: bool,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub zero: bool,
    pub padding: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Spec {
    pub padding: f64,
    pub palette: Vec<String>,
    /// The spec gives its palette (else the look's: the report theme's or BIcraft's).
    pub own_palette: bool,
    /// The Style options (`{preset, palette, rounded, gradient, shadow, animation}`, checked;
    /// docs/formats/visual-look.md), an empty object when not given.
    pub style: Value,
    /// The text settings by element (`labels`, `x`, `y`, `xTitle`, `yTitle`, `legend`:
    /// docs/formats/spec-visual.md "Text"), checked; an empty object when not given.
    pub text: Value,
    pub x: Axis,
    pub y: Axis,
    pub legend: Option<bool>,
    pub marks: Vec<Mark>,
}

impl Spec {
    /// The measures the spec refers to (highest index plus one).
    pub fn values_used(&self) -> usize {
        self.marks.iter().map(mark_reads).max().unwrap_or(0)
    }
}

/// The measures mark `m` reads (highest index plus one).
fn mark_reads(m: &Mark) -> usize {
    let mut n = 0usize;
    let mut see = |r: &Ref| {
        if let Ref::Value(k) = r {
            n = n.max(k.saturating_add(1));
        }
    };
    for e in [&m.x, &m.y, &m.x2, &m.y2].into_iter().flatten() {
        if let Enc::Field(r) = e {
            see(r);
        }
    }
    if let Some(Color::ByValue(r, _, _)) = &m.color {
        see(r);
    }
    if let Some(Size::ByValue(r, _, _)) = &m.size {
        see(r);
    }
    if let Some(r) = &m.label {
        see(r);
    }
    n
}

fn keys(o: &Map<String, Value>, allowed: &[&str], at: &str) -> Result<(), String> {
    for k in o.keys() {
        if !allowed.contains(&k.as_str()) {
            return Err(format!(
                "{at}: unknown key `{}` (allowed: {})",
                clip(k, 40),
                allowed.join(", ")
            ));
        }
    }
    Ok(())
}

fn text_of(v: &Value, at: &str) -> Result<String, String> {
    match v {
        Value::String(s) if s.chars().count() <= MAX_TEXT && !s.chars().any(char::is_control) => {
            Ok(s.clone())
        }
        Value::String(_) => Err(format!(
            "{at}: at most {MAX_TEXT} characters, without control characters"
        )),
        _ => Err(format!("{at}: a string is needed")),
    }
}

fn num_of(v: &Value, at: &str, lo: f64, hi: f64) -> Result<f64, String> {
    match v.as_f64() {
        Some(x) if x.is_finite() && x >= lo && x <= hi => Ok(x),
        _ => Err(format!("{at}: a number from {lo} to {hi} is needed")),
    }
}

fn bool_of(v: &Value, at: &str) -> Result<bool, String> {
    v.as_bool()
        .ok_or_else(|| format!("{at}: true or false is needed"))
}

/// The spec's `style` (the Style options of BIcraft's look), checked.
fn style_of(v: Option<&Value>) -> Result<Value, String> {
    let Some(v) = v else {
        return Ok(json!({}));
    };
    let o = v.as_object().ok_or("spec.style: an object is needed")?;
    keys(
        o,
        &[
            "preset",
            "palette",
            "rounded",
            "gradient",
            "shadow",
            "animation",
            "card",
            "pills",
        ],
        "spec.style",
    )?;
    for (k, list) in [
        ("preset", &crate::look::PRESETS[..]),
        ("palette", &crate::look::PALETTES[..]),
    ] {
        if let Some(x) = o.get(k)
            && !x.as_str().is_some_and(|x| list.contains(&x))
        {
            return Err(format!("spec.style.{k}: one of {}", list.join(", ")));
        }
    }
    for k in ["rounded", "gradient", "shadow", "animation", "card", "pills"] {
        if let Some(x) = o.get(k) {
            bool_of(x, &format!("spec.style.{k}"))?;
        }
    }
    Ok(v.clone())
}

/// `#RRGGBB` (upper case kept as given, checked).
fn color_of(v: &Value, at: &str) -> Result<String, String> {
    let s = v.as_str().unwrap_or_default();
    let ok = s.len() == 7 && s.starts_with('#') && s.bytes().skip(1).all(|b| b.is_ascii_hexdigit());
    if ok {
        Ok(s.to_ascii_uppercase())
    } else {
        Err(format!("{at}: a colour as #RRGGBB is needed"))
    }
}

/// `category`, `value` (the first measure) or `value.N`.
pub fn parse_ref(s: &str) -> Option<Ref> {
    match s {
        "category" => Some(Ref::Category),
        "value" => Some(Ref::Value(0)),
        _ => {
            let n = s.strip_prefix("value.")?;
            if n.is_empty() || n.len() > 2 || !n.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let k: usize = n.parse().ok()?;
            (k < MAX_VALUES).then_some(Ref::Value(k))
        }
    }
}

fn ref_of(v: &Value, at: &str) -> Result<Ref, String> {
    v.as_str().and_then(parse_ref).ok_or_else(|| {
        format!(
            "{at}: a field is `category`, `value` (the first measure) or `value.N` (N from 0 to {})",
            MAX_VALUES - 1
        )
    })
}

fn enc_of(v: &Value, at: &str) -> Result<Enc, String> {
    match v {
        Value::String(_) => Ok(Enc::Field(ref_of(v, at)?)),
        Value::Object(o) => {
            keys(o, &["field", "value"], at)?;
            match (o.get("field"), o.get("value")) {
                (Some(f), None) => Ok(Enc::Field(ref_of(f, &format!("{at}.field"))?)),
                (None, Some(Value::String(s))) => Ok(Enc::Text(text_of(&Value::String(s.clone()), at)?)),
                (None, Some(n)) => Ok(Enc::Num(num_of(n, &format!("{at}.value"), -1e15, 1e15)?)),
                _ => Err(format!("{at}: give `field` or `value`")),
            }
        }
        _ => Err(format!("{at}: a field name or {{field}} / {{value}} is needed")),
    }
}

fn mark_of(v: &Value, i: usize) -> Result<Mark, String> {
    let at = format!("marks[{i}]");
    let o = v
        .as_object()
        .ok_or_else(|| format!("{at}: an object is needed"))?;
    keys(
        o,
        &[
            "type", "name", "x", "y", "x2", "y2", "color", "size", "label", "opacity", "decimals",
        ],
        &at,
    )?;
    let kind = match o.get("type").and_then(Value::as_str) {
        Some("bar") => MarkType::Bar,
        Some("line") => MarkType::Line,
        Some("point") => MarkType::Point,
        Some("area") => MarkType::Area,
        Some("text") => MarkType::Text,
        Some("rule") => MarkType::Rule,
        Some("rect") => MarkType::Rect,
        _ => {
            return Err(format!("{at}.type: one of {}", MARK_TYPES.join(", ")));
        }
    };
    let enc = |k: &str| -> Result<Option<Enc>, String> {
        o.get(k).map(|v| enc_of(v, &format!("{at}.{k}"))).transpose()
    };
    let rule = kind == MarkType::Rule;
    let x = match enc("x")? {
        None if !rule => Some(Enc::Field(Ref::Category)),
        e => e,
    };
    let y = match enc("y")? {
        None if !rule => Some(Enc::Field(Ref::Value(0))),
        e => e,
    };
    if rule && x.is_some() == y.is_some() {
        return Err(format!(
            "{at}: a rule takes `x` (a vertical line) or `y` (a horizontal one)"
        ));
    }
    let color = match o.get("color") {
        None => None,
        Some(c @ Value::String(_)) => Some(Color::Fixed(color_of(c, &format!("{at}.color"))?)),
        Some(Value::Object(c)) => {
            let cat = format!("{at}.color");
            keys(c, &["field", "range"], &cat)?;
            let f = ref_of(c.get("field").unwrap_or(&Value::Null), &format!("{cat}.field"))?;
            match f {
                Ref::Category => {
                    if c.contains_key("range") {
                        return Err(format!(
                            "{cat}: colours by category come from the palette (no range)"
                        ));
                    }
                    Some(Color::ByCategory)
                }
                r => {
                    let (a, b) = match c.get("range") {
                        None => (VALUE_RANGE[0].to_string(), VALUE_RANGE[1].to_string()),
                        Some(Value::Array(l)) if l.len() == 2 => (
                            color_of(l.first().unwrap_or(&Value::Null), &format!("{cat}.range[0]"))?,
                            color_of(l.get(1).unwrap_or(&Value::Null), &format!("{cat}.range[1]"))?,
                        ),
                        _ => return Err(format!("{cat}.range: two colours are needed")),
                    };
                    Some(Color::ByValue(r, a, b))
                }
            }
        }
        Some(_) => return Err(format!("{at}.color: #RRGGBB or {{field, range?}} is needed")),
    };
    let size = match o.get("size") {
        None => None,
        Some(Value::Object(c)) => {
            let sat = format!("{at}.size");
            keys(c, &["field", "range"], &sat)?;
            let r = ref_of(c.get("field").unwrap_or(&Value::Null), &format!("{sat}.field"))?;
            if r == Ref::Category {
                return Err(format!("{sat}.field: a size comes from a measure"));
            }
            let (a, b) = match c.get("range") {
                None => (3.0, 12.0),
                Some(Value::Array(l)) if l.len() == 2 => (
                    num_of(
                        l.first().unwrap_or(&Value::Null),
                        &format!("{sat}.range[0]"),
                        0.0,
                        200.0,
                    )?,
                    num_of(
                        l.get(1).unwrap_or(&Value::Null),
                        &format!("{sat}.range[1]"),
                        0.0,
                        200.0,
                    )?,
                ),
                _ => return Err(format!("{sat}.range: two numbers are needed")),
            };
            Some(Size::ByValue(r, a, b))
        }
        Some(n) => Some(Size::Fixed(num_of(n, &format!("{at}.size"), 0.0, 200.0)?)),
    };
    let label = o
        .get("label")
        .map(|v| ref_of(v, &format!("{at}.label")))
        .transpose()?;
    if kind == MarkType::Text && label.is_none() {
        return Err(format!("{at}: a text mark needs `label` (the field it writes)"));
    }
    let opacity = match o.get("opacity") {
        None if kind == MarkType::Area => 0.35,
        None => 1.0,
        Some(v) => num_of(v, &format!("{at}.opacity"), 0.0, 1.0)?,
    };
    let decimals = o
        .get("decimals")
        .map(|v| num_of(v, &format!("{at}.decimals"), 0.0, 6.0).map(|d| d.floor() as u32))
        .transpose()?;
    let name = o
        .get("name")
        .map(|v| text_of(v, &format!("{at}.name")))
        .transpose()?;
    Ok(Mark {
        kind,
        name: name.filter(|n| !n.trim().is_empty()),
        x,
        y,
        x2: enc("x2")?,
        y2: enc("y2")?,
        color,
        size,
        label,
        opacity,
        decimals,
    })
}

fn axis_of(v: Option<&Value>, at: &str, is_x: bool) -> Result<Axis, String> {
    let mut a = Axis {
        scale: if is_x { ScaleKind::Band } else { ScaleKind::Linear },
        axis: true,
        title: None,
        grid: !is_x,
        min: None,
        max: None,
        zero: !is_x,
        padding: 0.2,
    };
    let Some(v) = v else {
        return Ok(a);
    };
    let o = v
        .as_object()
        .ok_or_else(|| format!("{at}: an object is needed"))?;
    keys(
        o,
        &["scale", "axis", "title", "grid", "min", "max", "zero", "padding"],
        at,
    )?;
    if let Some(s) = o.get("scale") {
        a.scale = match (s.as_str(), is_x) {
            (Some("band"), true) => ScaleKind::Band,
            (Some("linear"), _) => ScaleKind::Linear,
            (Some("time"), true) => ScaleKind::Time,
            (_, true) => return Err(format!("{at}.scale: band, linear or time")),
            (_, false) => return Err(format!("{at}.scale: the y scale is linear")),
        };
    }
    if let Some(v) = o.get("axis") {
        a.axis = bool_of(v, &format!("{at}.axis"))?;
    }
    if let Some(v) = o.get("title") {
        a.title = Some(text_of(v, &format!("{at}.title"))?).filter(|t| !t.trim().is_empty());
    }
    if let Some(v) = o.get("grid") {
        a.grid = bool_of(v, &format!("{at}.grid"))?;
    }
    if let Some(v) = o.get("zero") {
        a.zero = bool_of(v, &format!("{at}.zero"))?;
    }
    if let Some(v) = o.get("min") {
        a.min = Some(num_of(v, &format!("{at}.min"), -1e15, 1e15)?);
    }
    if let Some(v) = o.get("max") {
        a.max = Some(num_of(v, &format!("{at}.max"), -1e15, 1e15)?);
    }
    if let Some(v) = o.get("padding") {
        a.padding = num_of(v, &format!("{at}.padding"), 0.0, 0.9)?;
    }
    Ok(a)
}

/// Parses and checks a spec (docs/formats/spec-visual.md).
pub fn parse(v: &Value) -> Result<Spec, String> {
    let o = v.as_object().ok_or("the spec must be a JSON object")?;
    keys(
        o,
        &[
            "$schema", "version", "padding", "palette", "style", "text", "x", "y", "legend", "marks",
        ],
        "spec",
    )?;
    if let Some(ver) = o.get("version")
        && ver.as_u64() != Some(SPEC_VERSION)
    {
        return Err(format!(
            "spec.version: {SPEC_VERSION} is the version BIcraft reads"
        ));
    }
    let padding = match o.get("padding") {
        None => 8.0,
        Some(p) => num_of(p, "spec.padding", 0.0, 64.0)?,
    };
    let palette = match o.get("palette") {
        None => DEFAULT_PALETTE.iter().map(|c| c.to_string()).collect(),
        Some(Value::Array(l)) if !l.is_empty() && l.len() <= MAX_PALETTE => l
            .iter()
            .enumerate()
            .map(|(i, c)| color_of(c, &format!("spec.palette[{i}]")))
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => return Err(format!("spec.palette: 1 to {MAX_PALETTE} colours as #RRGGBB")),
    };
    let own_palette = o.contains_key("palette");
    let style = style_of(o.get("style"))?;
    let text = text_set_of(o.get("text"))?;
    let x = axis_of(o.get("x"), "spec.x", true)?;
    let y = axis_of(o.get("y"), "spec.y", false)?;
    let legend = match o.get("legend") {
        None => None,
        Some(v) => Some(bool_of(v, "spec.legend")?),
    };
    let marks = match o.get("marks") {
        Some(Value::Array(l)) if !l.is_empty() && l.len() <= MAX_MARKS => l
            .iter()
            .enumerate()
            .map(|(i, m)| mark_of(m, i))
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Err(format!("spec.marks: a list of 1 to {MAX_MARKS} marks is needed")),
    };
    for (i, m) in marks.iter().enumerate() {
        let at = format!("marks[{i}]");
        for (k, e) in [("x", &m.x), ("x2", &m.x2)] {
            match (x.scale, e) {
                (_, None) => {}
                (ScaleKind::Band, Some(Enc::Num(_))) => {
                    return Err(format!(
                        "{at}.{k}: the x scale is a band (categories): give `category` or a category's text, or make spec.x.scale linear or time"
                    ));
                }
                (ScaleKind::Band, Some(Enc::Field(Ref::Value(_)))) => {
                    return Err(format!(
                        "{at}.{k}: a measure on x needs spec.x.scale linear (a band scale places categories)"
                    ));
                }
                (ScaleKind::Linear, Some(Enc::Text(_))) => {
                    return Err(format!("{at}.{k}: the linear x scale takes numbers"));
                }
                (ScaleKind::Time, Some(Enc::Text(t))) if parse_iso(t).is_none() => {
                    return Err(format!("{at}.{k}: the time scale takes ISO dates (2024-01-31)"));
                }
                _ => {}
            }
        }
        for (k, e) in [("y", &m.y), ("y2", &m.y2)] {
            match e {
                Some(Enc::Field(Ref::Category)) => {
                    return Err(format!(
                        "{at}.{k}: y is a measure (`value`, `value.N`) or a number"
                    ));
                }
                Some(Enc::Text(_)) => return Err(format!("{at}.{k}: y takes numbers")),
                _ => {}
            }
        }
        if m.kind == MarkType::Bar && x.scale != ScaleKind::Band {
            return Err(format!(
                "{at}: bars need the band x scale (use rect on a linear scale)"
            ));
        }
        if m.kind == MarkType::Rect && x.scale != ScaleKind::Band && m.x2.is_none() {
            return Err(format!("{at}: a rect on a linear or time x scale needs `x2`"));
        }
    }
    Ok(Spec {
        padding,
        palette,
        own_palette,
        style,
        text,
        x,
        y,
        legend,
        marks,
    })
}

/// The text elements of `spec.text` and their settings.
const TEXT_PARTS: [&str; 6] = ["labels", "x", "y", "xTitle", "yTitle", "legend"];
const TEXT_KEYS: [&str; 7] = ["show", "size", "family", "color", "bold", "italic", "underline"];
/// Display units: 0 automatic, 1 none, then thousands to trillions.
pub const UNITS: [f64; 6] = [0.0, 1.0, 1e3, 1e6, 1e9, 1e12];

/// The spec's `text` (font and number settings of its text elements), checked; colours upper
/// case, decimals whole.
fn text_set_of(v: Option<&Value>) -> Result<Value, String> {
    let Some(v) = v else {
        return Ok(json!({}));
    };
    let o = v.as_object().ok_or("spec.text: an object is needed")?;
    keys(o, &TEXT_PARTS, "spec.text")?;
    let mut out = Map::new();
    for part in TEXT_PARTS {
        let Some(p) = o.get(part) else {
            continue;
        };
        let at = format!("spec.text.{part}");
        let p = p
            .as_object()
            .ok_or_else(|| format!("{at}: an object is needed"))?;
        let numeric = part == "labels" || part == "y";
        let mut allowed = TEXT_KEYS.to_vec();
        if numeric {
            allowed.extend(["units", "decimals"]);
        }
        keys(p, &allowed, &at)?;
        let mut s = Map::new();
        for k in ["show", "bold", "italic", "underline"] {
            if let Some(x) = p.get(k) {
                s.insert(k.into(), json!(bool_of(x, &format!("{at}.{k}"))?));
            }
        }
        if let Some(x) = p.get("size") {
            s.insert("size".into(), json!(num_of(x, &format!("{at}.size"), 6.0, 60.0)?));
        }
        if let Some(x) = p.get("family") {
            s.insert("family".into(), json!(text_of(x, &format!("{at}.family"))?));
        }
        if let Some(x) = p.get("color") {
            s.insert("color".into(), json!(color_of(x, &format!("{at}.color"))?));
        }
        if let Some(x) = p.get("units") {
            let u = x.as_f64().filter(|u| UNITS.contains(u)).ok_or_else(|| {
                format!(
                    "{at}.units: one of 0 (automatic), 1 (none), 1000, 1000000, 1000000000, 1000000000000"
                )
            })?;
            s.insert("units".into(), json!(u));
        }
        if let Some(x) = p.get("decimals") {
            let d = num_of(x, &format!("{at}.decimals"), 0.0, 6.0)?.floor();
            s.insert("decimals".into(), json!(d as u32));
        }
        out.insert(part.into(), Value::Object(s));
    }
    Ok(Value::Object(out))
}

/// Parses spec text (at most [`MAX_SPEC_BYTES`]).
pub fn parse_text(text: &str) -> Result<(Value, Spec), String> {
    if text.len() > MAX_SPEC_BYTES {
        return Err(format!(
            "the spec is {} bytes; at most {MAX_SPEC_BYTES}",
            text.len()
        ));
    }
    let v: Value = serde_json::from_str(text).map_err(|e| format!("the spec is not valid JSON: {e}"))?;
    let s = parse(&v)?;
    Ok((v, s))
}

/// A starting spec for a chart kind (`bar`, `line`, `area`, `point`, `scatter`, `combo`,
/// `text`, `rule`), `None` for another name.
pub fn template(kind: &str) -> Option<Value> {
    Some(match kind {
        "bar" => json!({"version": 1, "marks": [{"type": "bar", "y": "value", "label": "value"}]}),
        "line" => json!({"version": 1, "marks": [
            {"type": "line", "y": "value", "name": "Value"},
            {"type": "point", "y": "value", "size": 3}
        ]}),
        "area" => json!({"version": 1, "marks": [
            {"type": "area", "y": "value"},
            {"type": "line", "y": "value"}
        ]}),
        "point" => {
            json!({"version": 1, "marks": [{"type": "point", "y": "value", "color": {"field": "category"}}]})
        }
        "scatter" => json!({"version": 1, "x": {"scale": "linear", "grid": true}, "marks": [
            {"type": "point", "x": "value", "y": "value.1", "size": {"field": "value.1", "range": [3, 10]}, "opacity": 0.8}
        ]}),
        "combo" => json!({"version": 1, "marks": [
            {"type": "bar", "y": "value", "name": "First measure"},
            {"type": "line", "y": "value.1", "name": "Second measure", "color": "#E07A1F"}
        ]}),
        "text" => json!({"version": 1, "y": {"axis": false, "grid": false}, "marks": [
            {"type": "text", "y": "value", "label": "value", "size": 14}
        ]}),
        "rule" => json!({"version": 1, "marks": [
            {"type": "bar", "y": "value"},
            {"type": "rule", "y": {"value": 0}, "color": "#C23B5A"}
        ]}),
        _ => return None,
    })
}

/// The chart kinds [`template`] knows.
pub const TEMPLATES: &[&str] = &["bar", "line", "area", "point", "scatter", "combo", "text", "rule"];

// ---------------------------------------------------------------------------------------------
// The data
// ---------------------------------------------------------------------------------------------

/// A category value.
#[derive(Debug, Clone, PartialEq)]
pub enum Cat {
    Text(String),
    Num(f64),
    /// Milliseconds since 1970-01-01 (the value's own clock, no time zone).
    Date(f64),
}

/// What the visual draws: a category per row and the measures of the `values` role.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SpecData {
    pub cats: Vec<Cat>,
    pub values: Vec<Vec<Option<f64>>>,
    /// The highlighted part of each value (the data view's `highlights`, another visual's
    /// selection highlighting this one: docs/formats/bicraft-visuals.md "Highlights"), by value
    /// column then row; empty when nothing is highlighted.
    pub highlights: Vec<Vec<Option<f64>>>,
    /// The category column's format string: its dates are labelled with it.
    pub cat_format: String,
    /// The labels' language (`en`, `fr`: the UI's, as Power BI's `host.locale`).
    pub lang: String,
    /// The category field's name and each value field's name (series names and default axis
    /// titles).
    pub cat_name: String,
    pub names: Vec<String>,
}

impl SpecData {
    /// The golden tests' form: `{categories: [text | number | {date: "2024-01-31"}], values:
    /// [[number | null]], highlights?: [[number | null]], categoryFormat?, locale?}`.
    pub fn from_json(v: &Value) -> SpecData {
        let cats = v
            .get("categories")
            .and_then(Value::as_array)
            .map(|l| l.iter().take(MAX_ROWS).map(cat_from_json).collect())
            .unwrap_or_default();
        let columns = |key: &str| -> Vec<Vec<Option<f64>>> {
            v.get(key)
                .and_then(Value::as_array)
                .map(|l| {
                    l.iter()
                        .take(MAX_VALUES)
                        .map(|c| {
                            c.as_array()
                                .map(|r| r.iter().take(MAX_ROWS).map(num_cell).collect())
                                .unwrap_or_default()
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        SpecData {
            cats,
            values: columns("values"),
            highlights: columns("highlights"),
            cat_format: format_of(v.get("categoryFormat").and_then(Value::as_str).unwrap_or("")),
            lang: lang_of(v.get("locale").and_then(Value::as_str).unwrap_or("")),
            cat_name: v
                .get("categoryName")
                .and_then(Value::as_str)
                .map(|s| clip(s, MAX_TEXT))
                .unwrap_or_default(),
            names: v
                .get("names")
                .and_then(Value::as_array)
                .map(|l| {
                    l.iter()
                        .take(MAX_VALUES)
                        .map(|x| x.as_str().map(|s| clip(s, MAX_TEXT)).unwrap_or_default())
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    fn rows(&self) -> usize {
        self.values
            .iter()
            .map(Vec::len)
            .fold(self.cats.len(), usize::max)
            .min(MAX_ROWS)
    }

    fn value(&self, k: usize, i: usize) -> Option<f64> {
        self.values
            .get(k)?
            .get(i)
            .copied()
            .flatten()
            .filter(|x| x.is_finite())
    }

    /// Whether another visual's selection highlights this one.
    fn highlighted(&self) -> bool {
        !self.highlights.is_empty()
    }

    /// The highlighted part of value `k` at row `i` (none: not highlighted).
    fn highlight(&self, k: usize, i: usize) -> Option<f64> {
        self.highlights
            .get(k)?
            .get(i)
            .copied()
            .flatten()
            .filter(|x| x.is_finite())
    }

    /// Whether row `i`'s marks are drawn dimmed: the visual is highlighted and no value of the
    /// row is.
    fn faded(&self, i: usize) -> bool {
        self.highlighted() && !(0..self.highlights.len()).any(|k| self.highlight(k, i).is_some())
    }
}

fn cat_from_json(v: &Value) -> Cat {
    match v {
        Value::Number(n) => n.as_f64().map_or(Cat::Text(String::new()), Cat::Num),
        Value::Object(o) => o
            .get("date")
            .and_then(Value::as_str)
            .and_then(parse_iso)
            .map_or(Cat::Text(String::new()), Cat::Date),
        Value::String(s) => Cat::Text(s.clone()),
        Value::Bool(b) => Cat::Text(b.to_string()),
        _ => Cat::Text(String::new()),
    }
}

fn num_cell(v: &Value) -> Option<f64> {
    v.as_f64().filter(|x| x.is_finite())
}

/// The data of a visual's snapshot dataset: the first column of the `category` role (else the
/// first column that is not a measure) and the columns of the `values` role (else the measures).
pub(crate) fn data_of(ds: &Dataset, lang: &str) -> SpecData {
    let is_measure = |k: &str| k == "measure" || k == "aggregation";
    let cat = ds
        .cols
        .iter()
        .position(|c| c.role.eq_ignore_ascii_case("category"))
        .or_else(|| ds.cols.iter().position(|c| !is_measure(&c.kind)));
    let mut vals: Vec<usize> = ds
        .cols
        .iter()
        .enumerate()
        .filter(|(_, c)| c.role.eq_ignore_ascii_case("values"))
        .map(|(i, _)| i)
        .collect();
    if vals.is_empty() {
        vals = (0..ds.cols.len()).filter(|i| Some(*i) != cat).collect();
    }
    let date = cat.and_then(|c| ds.cols.get(c)).is_some_and(|c| c.is_date());
    let rows = ds.rows.iter().take(MAX_ROWS);
    let cats = match cat {
        Some(c) => rows
            .clone()
            .map(|r| match r.get(c) {
                Some(Value::String(s)) if date => {
                    parse_iso(s).map_or_else(|| Cat::Text(s.clone()), Cat::Date)
                }
                Some(v) => cat_from_json(v),
                None => Cat::Text(String::new()),
            })
            .collect(),
        None => Vec::new(),
    };
    let values = vals
        .iter()
        .take(MAX_VALUES)
        .map(|k| rows.clone().map(|r| r.get(*k).and_then(num_cell)).collect())
        .collect();
    let highlights = match &ds.hl {
        Some(h) => vals
            .iter()
            .take(MAX_VALUES)
            .map(|k| {
                (0..ds.rows.len().min(MAX_ROWS))
                    .map(|r| h.get(r).and_then(|x| x.get(*k)).and_then(num_cell))
                    .collect()
            })
            .collect(),
        None => Vec::new(),
    };
    SpecData {
        cats,
        values,
        highlights,
        cat_format: format_of(cat.and_then(|c| ds.cols.get(c)).map_or("", |c| c.fmt.as_str())),
        lang: lang_of(lang),
        cat_name: cat
            .and_then(|c| ds.cols.get(c))
            .map(|c| clip(&c.name, MAX_TEXT))
            .unwrap_or_default(),
        names: vals
            .iter()
            .take(MAX_VALUES)
            .map(|k| {
                ds.cols
                    .get(*k)
                    .map(|c| clip(&c.name, MAX_TEXT))
                    .unwrap_or_default()
            })
            .collect(),
    }
}

/// A format string as the layout reads it (its first 200 characters).
fn format_of(f: &str) -> String {
    f.chars().take(200).collect()
}

/// The labels' language: `fr` for a French tag, else `en`.
fn lang_of(lang: &str) -> String {
    if crate::fmt::is_fr(lang) { "fr" } else { "en" }.to_string()
}

// ---------------------------------------------------------------------------------------------
// Numbers, dates and text (the same in spec.ts)
// ---------------------------------------------------------------------------------------------

/// `v` rounded to 0.01 (half up), as the primitives carry it.
pub fn r2(v: f64) -> f64 {
    (v * 100.0 + 0.5).floor() / 100.0
}

/// `v` with `d` decimals, half away from zero, thousands separated by `,`.
pub fn fmt_fixed(v: f64, d: u32) -> String {
    if !v.is_finite() {
        return String::new();
    }
    let d = d.min(6);
    let scale = 10f64.powf(f64::from(d));
    let a = (v.abs() * scale + 0.5).floor();
    if a > 9.0e15 {
        return "#".into();
    }
    let r = a as u64;
    let s = 10u64.pow(d);
    let int = (r / s).to_string();
    let mut out = String::new();
    if v < 0.0 && r != 0 {
        out.push('-');
    }
    let len = int.len();
    for (i, ch) in int.chars().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    if d > 0 {
        let frac = (r % s).to_string();
        out.push('.');
        for _ in frac.len()..(d as usize) {
            out.push('0');
        }
        out.push_str(&frac);
    }
    out
}

/// `v` with no decimals when it is whole, else 2.
pub fn fmt_auto(v: f64) -> String {
    let d = if (v - v.floor()).abs() < 1e-9 { 0 } else { 2 };
    fmt_fixed(v, d)
}

/// `s` (written by [`fmt_fixed`]) with the culture's marks: `1 000,5` in French (a no-break
/// space), unchanged in English.
pub fn local_num(s: &str, lang: &str) -> String {
    if lang != "fr" {
        return s.to_string();
    }
    s.chars()
        .map(|c| match c {
            ',' => '\u{a0}',
            '.' => ',',
            c => c,
        })
        .collect()
}

/// The display unit an automatic setting takes for values up to `max_abs`.
pub fn auto_unit(max_abs: f64) -> f64 {
    if max_abs >= 1e12 {
        1e12
    } else if max_abs >= 1e9 {
        1e9
    } else if max_abs >= 1e6 {
        1e6
    } else if max_abs >= 1e3 {
        1e3
    } else {
        1.0
    }
}

/// The unit drawn for setting `units` (none: none, 0: automatic from `max_abs`).
pub fn unit_of(units: Option<f64>, max_abs: f64) -> f64 {
    match units {
        None => 1.0,
        Some(0.0) => auto_unit(max_abs),
        Some(u) => u,
    }
}

/// `v` in unit `u` with `d` decimals (none: none for whole numbers, else 2), in the culture of
/// `lang`, with the unit's suffix.
pub fn num_text(v: f64, d: Option<u32>, u: f64, lang: &str) -> String {
    let x = if u > 1.0 { v / u } else { v };
    let s = local_num(
        &match d {
            Some(d) => fmt_fixed(x, d),
            None => fmt_auto(x),
        },
        lang,
    );
    let fr = lang == "fr";
    let suffix = if u == 1e3 {
        if fr { "\u{a0}k" } else { "K" }
    } else if u == 1e6 {
        if fr { "\u{a0}M" } else { "M" }
    } else if u == 1e9 {
        if fr { "\u{a0}Md" } else { "bn" }
    } else if u == 1e12 {
        if fr { "\u{a0}Bn" } else { "T" }
    } else {
        ""
    };
    format!("{s}{suffix}")
}

/// The decimals tick labels need for a step.
pub(crate) fn tick_decimals(step: f64) -> u32 {
    if step >= 1.0 || step <= 0.0 {
        0
    } else {
        (-step.log10() - 1e-9).ceil().clamp(0.0, 6.0) as u32
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date.
pub(crate) fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub(crate) fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn digits(s: &str, n: usize) -> Option<i64> {
    (s.len() == n && s.bytes().all(|b| b.is_ascii_digit()))
        .then(|| s.parse().ok())
        .flatten()
}

/// `YYYY-MM-DD`, optionally with `THH:MM[:SS[.fff]]` and a zone suffix (ignored), as
/// milliseconds since 1970-01-01.
pub fn parse_iso(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.len() > 40 || !s.is_ascii() {
        return None;
    }
    let (date, time) = match s.find(['T', ' ']) {
        Some(i) => (s.get(..i)?, s.get(i + 1..)?),
        None => (s, ""),
    };
    let mut dp = date.split('-');
    let y = digits(dp.next()?, 4)?;
    let m = digits(dp.next()?, 2)?;
    let d = digits(dp.next()?, 2)?;
    if dp.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let time = time.trim_end_matches('Z');
    let time = match time.find(['+', '-']) {
        Some(i) => time.get(..i)?,
        None => time,
    };
    let (mut hh, mut mi, mut ss, mut ms) = (0i64, 0i64, 0i64, 0i64);
    if !time.is_empty() {
        let (hms, frac) = match time.split_once('.') {
            Some((a, b)) => (a, Some(b)),
            None => (time, None),
        };
        let mut tp = hms.split(':');
        hh = digits(tp.next()?, 2)?;
        mi = digits(tp.next()?, 2)?;
        if let Some(x) = tp.next() {
            ss = digits(x, 2)?;
        }
        if tp.next().is_some() || hh > 23 || mi > 59 || ss > 59 {
            return None;
        }
        if let Some(f) = frac {
            if f.is_empty() || f.len() > 9 || !f.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let three: String = f.chars().chain("00".chars()).take(3).collect();
            ms = three.parse().ok()?;
        }
    }
    let days = days_from_civil(y, m, d);
    Some(days as f64 * MS_PER_DAY + (hh * 3_600_000 + mi * 60_000 + ss * 1000 + ms) as f64)
}

/// `YYYY-MM-DD`, with ` HH:MM` when `time`.
pub fn date_text(ms: f64, time: bool) -> String {
    if !ms.is_finite() {
        return String::new();
    }
    let ms = ms.clamp(-8.64e15, 8.64e15);
    let days = (ms / MS_PER_DAY).floor();
    let (y, m, d) = civil_from_days(days as i64);
    let mut out = format!("{y:04}-{m:02}-{d:02}");
    if time {
        let rem = ms - days * MS_PER_DAY;
        let h = (rem / 3_600_000.0).floor();
        let mi = ((rem - h * 3_600_000.0) / 60_000.0).floor();
        out.push_str(&format!(" {:02}:{:02}", h as i64, mi as i64));
    }
    out
}

pub(crate) fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// A text's estimated width (no font is measured, so both drawers agree).
pub(crate) fn est(s: &str, size: f64) -> f64 {
    s.chars().count() as f64 * size * 0.55
}

/// The centre of a centred text moved so its estimated width stays within `[0, w]` (the first
/// and last x tick labels; a text wider than the visual starts at 0).
pub(crate) fn inside(x: f64, s: &str, size: f64, w: f64) -> f64 {
    let half = est(s, size) / 2.0;
    x.min(w - half).max(half)
}

fn cat_text(c: Option<&Cat>) -> String {
    match c {
        Some(Cat::Text(s)) => s.clone(),
        Some(Cat::Num(n)) => fmt_auto(*n),
        Some(Cat::Date(ms)) => date_text(*ms, (*ms / MS_PER_DAY).floor() * MS_PER_DAY != *ms),
        None => String::new(),
    }
}

/// A category as a label: a date written with the category's format string (`date_label`),
/// else as [`cat_text`] writes it.
fn cat_label(d: &SpecData, i: usize) -> String {
    let c = d.cats.get(i);
    if let Some(Cat::Date(ms)) = c
        && let Some(l) = crate::fmt::date_label(*ms, &d.cat_format, &d.lang)
    {
        return l;
    }
    if let Some(Cat::Num(n)) = c {
        return local_num(&fmt_auto(*n), &d.lang);
    }
    cat_text(c)
}

fn hex(c: &str) -> (f64, f64, f64) {
    let p = |i: usize| {
        c.get(i..i + 2)
            .and_then(|h| u8::from_str_radix(h, 16).ok())
            .map_or(0.0, f64::from)
    };
    (p(1), p(3), p(5))
}

fn lerp_color(a: &str, b: &str, t: f64) -> String {
    let (a, b) = (hex(a), hex(b));
    let ch = |x: f64, y: f64| (x + (y - x) * t + 0.5).floor().clamp(0.0, 255.0) as u8;
    format!("#{:02X}{:02X}{:02X}", ch(a.0, b.0), ch(a.1, b.1), ch(a.2, b.2))
}

// ---------------------------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------------------------

/// A drawing primitive, in the visual's pixels (origin top left).
#[derive(Debug, Clone, PartialEq)]
pub enum Prim {
    Rect {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        fill: String,
        opacity: f64,
        /// Rounded corners, gradient, edge and shadow (BIcraft's look, `crate::look`).
        deco: Deco,
    },
    Line {
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        stroke: String,
        width: f64,
    },
    Path {
        points: Vec<(f64, f64)>,
        stroke: String,
        width: f64,
    },
    Poly {
        points: Vec<(f64, f64)>,
        fill: String,
        opacity: f64,
    },
    Circle {
        cx: f64,
        cy: f64,
        r: f64,
        fill: String,
        opacity: f64,
        deco: Deco,
    },
    Text {
        x: f64,
        y: f64,
        text: String,
        size: f64,
        fill: String,
        /// `start`, `middle` or `end`.
        anchor: &'static str,
        /// Degrees (`-90`: read bottom to top).
        rotate: f64,
        /// The font weight (400 regular).
        weight: u16,
        /// The family, italic and underline the text settings give (default: none).
        font: TextFont,
    },
}

/// What a text element's settings add to a text primitive (docs/formats/spec-visual.md "Text");
/// the default adds nothing. The golden form writes only what is set.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextFont {
    /// A CSS family list (empty: the visual's own).
    pub family: String,
    pub italic: bool,
    pub underline: bool,
}

/// What BIcraft's look adds to a rectangle or a circle (docs/formats/visual-look.md); the
/// default adds nothing. The golden form writes only what is set.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Deco {
    /// Corner radius (0: square).
    pub rx: f64,
    /// The side left square (`b`, `t`, `l`, `r`; empty: every corner rounded).
    pub sq: &'static str,
    /// The top colour of a vertical gradient down to the fill (empty: a plain fill).
    pub grad: String,
    /// An edge drawn over the fill, and its width (empty: none).
    pub stroke: String,
    pub sw: f64,
    /// A soft shadow's size in pixels (0: none).
    pub shadow: f64,
    /// What the shape is besides a mark (empty: a mark): `card`, the surface behind a plot, or
    /// `pill`, a value label's chip. High contrast leaves both out (the TypeScript draws them
    /// as outlines over the background).
    pub role: &'static str,
}

impl Deco {
    fn add_to(&self, o: &mut Value) {
        let Some(m) = o.as_object_mut() else {
            return;
        };
        if self.rx > 0.0 {
            m.insert("rx".into(), json!(self.rx));
            if !self.sq.is_empty() {
                m.insert("sq".into(), json!(self.sq));
            }
        }
        if !self.grad.is_empty() {
            m.insert("grad".into(), json!(self.grad));
        }
        if !self.stroke.is_empty() {
            m.insert("stroke".into(), json!(self.stroke));
            m.insert("sw".into(), json!(self.sw));
        }
        if self.shadow > 0.0 {
            m.insert("shadow".into(), json!(self.shadow));
        }
        if !self.role.is_empty() {
            m.insert("role".into(), json!(self.role));
        }
    }
}

impl Prim {
    /// The golden tests' form (spec.ts writes the same objects).
    pub fn json(&self) -> Value {
        let pts = |p: &[(f64, f64)]| Value::Array(p.iter().map(|(x, y)| json!([x, y])).collect());
        match self {
            Prim::Rect {
                x,
                y,
                w,
                h,
                fill,
                opacity,
                deco,
            } => {
                let mut o =
                    json!({"t": "rect", "x": x, "y": y, "w": w, "h": h, "fill": fill, "opacity": opacity});
                deco.add_to(&mut o);
                o
            }
            Prim::Line {
                x1,
                y1,
                x2,
                y2,
                stroke,
                width,
            } => {
                json!({"t": "line", "x1": x1, "y1": y1, "x2": x2, "y2": y2, "stroke": stroke, "width": width})
            }
            Prim::Path {
                points,
                stroke,
                width,
            } => {
                json!({"t": "path", "points": pts(points), "stroke": stroke, "width": width})
            }
            Prim::Poly {
                points,
                fill,
                opacity,
            } => {
                json!({"t": "poly", "points": pts(points), "fill": fill, "opacity": opacity})
            }
            Prim::Circle {
                cx,
                cy,
                r,
                fill,
                opacity,
                deco,
            } => {
                let mut o =
                    json!({"t": "circle", "cx": cx, "cy": cy, "r": r, "fill": fill, "opacity": opacity});
                deco.add_to(&mut o);
                o
            }
            Prim::Text {
                x,
                y,
                text,
                size,
                fill,
                anchor,
                rotate,
                weight,
                font,
            } => {
                let mut o = json!({"t": "text", "x": x, "y": y, "text": text, "size": size, "fill": fill, "anchor": anchor, "rotate": rotate});
                if let Some(m) = o.as_object_mut() {
                    if *weight != 400 {
                        m.insert("weight".into(), json!(weight));
                    }
                    if !font.family.is_empty() {
                        m.insert("family".into(), json!(font.family));
                    }
                    if font.italic {
                        m.insert("italic".into(), json!(true));
                    }
                    if font.underline {
                        m.insert("underline".into(), json!(true));
                    }
                }
                o
            }
        }
    }
}

/// A linear domain with its ticks.
#[derive(Debug, Clone)]
pub(crate) struct Domain {
    pub(crate) lo: f64,
    pub(crate) hi: f64,
    pub(crate) step: f64,
    pub(crate) ticks: Vec<f64>,
}

fn nice_step(raw: f64) -> f64 {
    if raw.is_nan() || raw <= 0.0 || !raw.is_finite() {
        return 1.0;
    }
    let p = 10f64.powf(raw.log10().floor());
    let f = raw / p;
    let m = if f <= 1.0 {
        1.0
    } else if f <= 2.0 {
        2.0
    } else if f <= 5.0 {
        5.0
    } else {
        10.0
    };
    m * p
}

pub(crate) fn domain(vals: &[f64], zero: bool, min: Option<f64>, max: Option<f64>, count: f64) -> Domain {
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for v in vals {
        lo = lo.min(*v);
        hi = hi.max(*v);
    }
    if zero {
        lo = lo.min(0.0);
        hi = hi.max(0.0);
    }
    if !lo.is_finite() || !hi.is_finite() {
        lo = 0.0;
        hi = 1.0;
    }
    if let Some(m) = min {
        lo = m;
    }
    if let Some(m) = max {
        hi = m;
    }
    if hi < lo {
        std::mem::swap(&mut lo, &mut hi);
    }
    if hi == lo {
        if lo == 0.0 {
            hi = 1.0;
        } else {
            let d = lo.abs() * 0.1;
            lo -= d;
            hi += d;
        }
    }
    let step = nice_step((hi - lo) / count);
    if min.is_none() {
        lo = (lo / step).floor() * step;
    }
    if max.is_none() {
        hi = (hi / step).ceil() * step;
    }
    let k0 = (lo / step - 1e-9).ceil();
    let k1 = (hi / step + 1e-9).floor();
    let mut ticks = Vec::new();
    let mut k = k0;
    while k <= k1 && ticks.len() < MAX_TICKS {
        ticks.push(k * step);
        k += 1.0;
    }
    Domain { lo, hi, step, ticks }
}

/// The plot area and the scales of one layout.
struct Frame<'a> {
    spec: &'a Spec,
    data: &'a SpecData,
    n: usize,
    left: f64,
    right: f64,
    top: f64,
    bottom: f64,
    y: Domain,
    x: Option<Domain>,
    look: crate::look::Look,
    /// The palette drawn with: the spec's, else the look's.
    pal: Vec<String>,
}

impl Frame<'_> {
    fn ys(&self, v: f64) -> f64 {
        self.bottom - (v - self.y.lo) / (self.y.hi - self.y.lo) * (self.bottom - self.top)
    }

    fn xs(&self, v: f64) -> f64 {
        match &self.x {
            Some(d) => self.left + (v - d.lo) / (d.hi - d.lo) * (self.right - self.left),
            None => self.left,
        }
    }

    /// Row `i`'s band: its start and width.
    fn band(&self, i: usize) -> (f64, f64) {
        let step = (self.right - self.left) / self.n.max(1) as f64;
        let p = self.spec.x.padding;
        (self.left + step * i as f64 + step * p / 2.0, step * (1.0 - p))
    }

    fn band_of_text(&self, t: &str) -> Option<usize> {
        (0..self.n).find(|i| cat_text(self.data.cats.get(*i)) == t)
    }

    fn cat_num(&self, i: usize) -> Option<f64> {
        match self.data.cats.get(i)? {
            Cat::Num(n) => Some(*n),
            Cat::Date(ms) => Some(*ms),
            Cat::Text(s) if self.spec.x.scale == ScaleKind::Time => parse_iso(s),
            Cat::Text(_) => None,
        }
    }

    /// An x encoding's value at row `i` on a linear or time scale.
    fn x_value(&self, e: &Enc, i: usize) -> Option<f64> {
        match e {
            Enc::Field(Ref::Category) => self.cat_num(i),
            Enc::Field(Ref::Value(k)) => self.data.value(*k, i),
            Enc::Num(v) => Some(*v),
            Enc::Text(t) => parse_iso(t),
        }
    }

    /// An x encoding's position at row `i` (a band's middle on the band scale).
    fn x_pos(&self, e: &Enc, i: usize) -> Option<f64> {
        if self.spec.x.scale == ScaleKind::Band {
            let row = match e {
                Enc::Text(t) => self.band_of_text(t)?,
                _ => i,
            };
            let (x0, bw) = self.band(row);
            return Some(x0 + bw / 2.0);
        }
        self.x_value(e, i).map(|v| self.xs(v))
    }

    fn y_value(&self, e: &Enc, i: usize) -> Option<f64> {
        match e {
            Enc::Field(Ref::Value(k)) => self.data.value(*k, i),
            Enc::Num(v) => Some(*v),
            _ => None,
        }
    }

    /// The baseline of a bar, area or rect: `y2`, else 0, inside the domain.
    fn base(&self, m: &Mark, i: usize) -> f64 {
        let v = m.y2.as_ref().and_then(|e| self.y_value(e, i)).unwrap_or(0.0);
        self.ys(v.clamp(self.y.lo, self.y.hi))
    }

    fn ref_value(&self, r: Ref, i: usize) -> Option<f64> {
        match r {
            Ref::Value(k) => self.data.value(k, i),
            Ref::Category => self.cat_num(i),
        }
    }

    /// Where a value lies between the smallest and largest of a field (1 when they are equal).
    fn t_of(&self, r: Ref, i: usize) -> Option<f64> {
        let v = self.ref_value(r, i)?;
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        for j in 0..self.n {
            if let Some(x) = self.ref_value(r, j) {
                lo = lo.min(x);
                hi = hi.max(x);
            }
        }
        Some(if hi > lo { (v - lo) / (hi - lo) } else { 1.0 })
    }

    fn palette(&self, i: usize) -> String {
        let p = &self.pal;
        p.get(i % p.len().max(1))
            .cloned()
            .unwrap_or_else(|| TEXT.to_string())
    }

    fn mark_color(&self, m: &Mark, mi: usize) -> String {
        match &m.color {
            Some(Color::Fixed(c)) => c.clone(),
            _ => self.palette(mi),
        }
    }

    fn color_at(&self, m: &Mark, mi: usize, i: usize) -> String {
        match &m.color {
            Some(Color::Fixed(c)) => c.clone(),
            Some(Color::ByCategory) => self.palette(i),
            Some(Color::ByValue(r, a, b)) => match self.t_of(*r, i) {
                Some(t) => lerp_color(a, b, t),
                None => a.clone(),
            },
            None => self.palette(mi),
        }
    }

    fn size_at(&self, m: &Mark, i: usize, default: f64) -> f64 {
        match &m.size {
            Some(Size::Fixed(v)) => *v,
            Some(Size::ByValue(r, a, b)) => match self.t_of(*r, i) {
                Some(t) => a + t * (b - a),
                None => *a,
            },
            None => default,
        }
    }

    fn label_text(&self, m: &Mark, i: usize) -> Option<String> {
        match m.label? {
            Ref::Category => Some(cat_label(self.data, i)).filter(|s| !s.is_empty()),
            Ref::Value(k) => {
                let v = self.data.value(k, i)?;
                // the text settings' display units and decimals over the mark's decimals
                let t = self.spec.text.get("labels");
                let units = t.and_then(|t| t.get("units")).and_then(Value::as_f64);
                let max_abs = if units == Some(0.0) {
                    (0..self.n)
                        .filter_map(|j| self.data.value(k, j))
                        .fold(0.0, |a: f64, x| a.max(x.abs()))
                } else {
                    0.0
                };
                let d = t
                    .and_then(|t| t.get("decimals"))
                    .and_then(Value::as_u64)
                    .and_then(|d| u32::try_from(d).ok())
                    .or(m.decimals);
                Some(num_text(v, d, unit_of(units, max_abs), &self.data.lang))
            }
        }
    }
}

pub(crate) fn text(
    x: f64,
    y: f64,
    s: String,
    size: f64,
    fill: &str,
    anchor: &'static str,
    rotate: f64,
) -> Prim {
    Prim::Text {
        x: r2(x),
        y: r2(y),
        text: s,
        size,
        fill: fill.to_string(),
        anchor,
        rotate,
        weight: 400,
        font: TextFont::default(),
    }
}

pub(crate) fn line(x1: f64, y1: f64, x2: f64, y2: f64, stroke: &str, width: f64) -> Prim {
    Prim::Line {
        x1: r2(x1),
        y1: r2(y1),
        x2: r2(x2),
        y2: r2(y2),
        stroke: stroke.to_string(),
        width: r2(width),
    }
}

/// Lays out `spec` over `data` in a `w`×`h` visual (docs/formats/spec-visual.md "Layout"),
/// without a report theme.
pub fn layout(spec: &Spec, data: &SpecData, w: f64, h: f64) -> Vec<Prim> {
    layout_themed(spec, data, w, h, &crate::look::theme_json(&[]))
}

/// A data point a layout drew (the TypeScript's `Hit`): data row `row` is primitive `prim`, or
/// (`prim` `None`) the point `at` of a line or an area; `label`: its value label.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub row: usize,
    pub prim: Option<usize>,
    pub at: Option<(f64, f64)>,
    pub label: bool,
    /// Drawn dimmed: another visual's selection highlights none of the row's values (a bar
    /// draws its highlighted part instead).
    pub fade: bool,
}

/// [`layout`] in a report whose theme (`{colors, background?}`) gives the data colours and
/// the surface.
pub fn layout_themed(spec: &Spec, data: &SpecData, w: f64, h: f64, theme: &Value) -> Vec<Prim> {
    layout_hits(spec, data, w, h, theme).0
}

/// Most marks once every bound measure is drawn as its own series ("Series" in spec-visual.md).
const MAX_SERIES_MARKS: usize = 64;

/// A text element's drawing settings: the spec's `text` over the look's defaults.
#[derive(Debug, Clone)]
pub(crate) struct TextSet {
    pub(crate) size: f64,
    pub(crate) family: String,
    pub(crate) color: String,
    pub(crate) bold: Option<bool>,
    pub(crate) italic: bool,
    pub(crate) underline: bool,
}

/// The settings of text element `part` (`labels`, `x`, `y`, `xTitle`, `yTitle`, `legend`),
/// `size` and `color` by default.
pub(crate) fn text_set(spec: &Spec, part: &str, size: f64, color: &str) -> TextSet {
    let o = spec.text.get(part);
    let get = |k: &str| o.and_then(|o| o.get(k));
    TextSet {
        size: get("size").and_then(Value::as_f64).unwrap_or(size),
        family: get("family").and_then(Value::as_str).unwrap_or("").to_string(),
        color: get("color").and_then(Value::as_str).unwrap_or(color).to_string(),
        bold: get("bold").and_then(Value::as_bool),
        italic: get("italic").and_then(Value::as_bool) == Some(true),
        underline: get("underline").and_then(Value::as_bool) == Some(true),
    }
}

/// `p` (a text primitive) with the font of `t`: its weight when bold is set, its family, italic
/// and underline when set.
pub(crate) fn styled(mut p: Prim, t: &TextSet) -> Prim {
    if let Prim::Text { weight, font, .. } = &mut p {
        match t.bold {
            Some(true) => *weight = 700,
            Some(false) => *weight = 400,
            None => {}
        }
        if !t.family.is_empty() {
            font.family.clone_from(&t.family);
        }
        if t.italic {
            font.italic = true;
        }
        if t.underline {
            font.underline = true;
        }
    }
    p
}

/// Whether mark `m` draws the first measure as one plain series (so it can be repeated for each
/// other measure).
fn plain_series(m: &Mark) -> bool {
    matches!(
        m.kind,
        MarkType::Bar | MarkType::Line | MarkType::Area | MarkType::Point
    ) && m.y == Some(Enc::Field(Ref::Value(0)))
        && matches!(m.x, None | Some(Enc::Field(Ref::Category)))
        && m.x2.is_none()
        && matches!(m.y2, None | Some(Enc::Num(_)))
        && matches!(m.color, None | Some(Color::Fixed(_)))
        && matches!(m.size, None | Some(Size::Fixed(_)))
        && matches!(m.label, None | Some(Ref::Value(0)))
}

/// Whether the spec's marks are plain series of the first measure, or marks that read no
/// measure (rules at a value), with at least one series.
fn one_series(spec: &Spec) -> bool {
    spec.x.scale != ScaleKind::Linear
        && spec.marks.iter().any(plain_series)
        && spec.marks.iter().all(|m| plain_series(m) || mark_reads(m) == 0)
}

/// A value field's name (its series name), else `value.N`.
fn series_name(d: &SpecData, k: usize) -> String {
    match d.names.get(k) {
        Some(n) if !n.trim().is_empty() => n.clone(),
        _ if k == 0 => "value".into(),
        _ => format!("value.{k}"),
    }
}

/// The spec as drawn over `data` (docs/formats/spec-visual.md "Series" and "Text"): every bound
/// measure as its own series when the spec draws only the first one as one plain series; the
/// text settings' `show` over the axes, the axis titles, the legend and the value labels. Also
/// how many bound measures stay undrawn (the visual says so).
pub fn resolve(spec: &Spec, data: &SpecData, pal: &[String]) -> (Spec, usize) {
    let cols = data.values.len().min(MAX_VALUES);
    let used = spec.values_used();
    let mut out = spec.clone();
    let mut hidden = cols.saturating_sub(used);
    if cols > 1 && used == 1 && one_series(spec) {
        let plain = spec.marks.iter().filter(|m| plain_series(m)).count();
        let first = spec.marks.iter().position(plain_series);
        let series = cols.min((MAX_SERIES_MARKS / plain.max(1)).max(1));
        let mut marks = Vec::new();
        for k in 0..series {
            let c = pal
                .get(k % pal.len().max(1))
                .cloned()
                .unwrap_or_else(|| TEXT.to_string());
            for (j, m) in spec.marks.iter().enumerate() {
                if !plain_series(m) {
                    // a mark that reads no measure (a rule at a value) is drawn once
                    if k == 0 {
                        marks.push(m.clone());
                    }
                    continue;
                }
                marks.push(Mark {
                    kind: m.kind,
                    name: (Some(j) == first).then(|| clip(&series_name(data, k), MAX_TEXT)),
                    x: m.x.clone(),
                    y: Some(Enc::Field(Ref::Value(k))),
                    x2: m.x2.clone(),
                    y2: m.y2.clone(),
                    color: match &m.color {
                        Some(c0) if k == 0 => Some(c0.clone()),
                        _ => Some(Color::Fixed(c.clone())),
                    },
                    size: m.size.clone(),
                    label: m.label.map(|_| Ref::Value(k)),
                    opacity: m.opacity,
                    decimals: m.decimals,
                });
            }
        }
        out.marks = marks;
        hidden = cols - series;
    }
    let show = |part: &str| {
        spec.text
            .get(part)
            .and_then(|t| t.get("show"))
            .and_then(Value::as_bool)
    };
    if let Some(labels) = show("labels") {
        for m in &mut out.marks {
            if matches!(m.kind, MarkType::Text | MarkType::Rule | MarkType::Rect) {
                continue;
            }
            if !labels {
                m.label = None;
            } else if m.label.is_none()
                && let Some(Enc::Field(r @ Ref::Value(_))) = &m.y
            {
                m.label = Some(*r);
            }
        }
    }
    let value_names: Vec<String> = (0..cols.min(used.max(cols - hidden)))
        .map(|k| series_name(data, k))
        .collect();
    let (xt, yt) = (show("xTitle"), show("yTitle"));
    out.x.title = match xt {
        Some(false) => None,
        _ => spec.x.title.clone().or_else(|| {
            (xt == Some(true) && !data.cat_name.trim().is_empty()).then(|| data.cat_name.clone())
        }),
    };
    out.y.title = match yt {
        Some(false) => None,
        _ => spec
            .y
            .title
            .clone()
            .or_else(|| (yt == Some(true) && !value_names.is_empty()).then(|| value_names.join(", "))),
    };
    out.x.axis = show("x").unwrap_or(spec.x.axis);
    out.y.axis = show("y").unwrap_or(spec.y.axis);
    if let Some(l) = show("legend") {
        out.legend = Some(l);
    }
    (out, hidden)
}

/// The line saying how many bound measures the spec leaves undrawn.
fn notice_text(hidden: usize, cols: usize, lang: &str) -> String {
    if lang == "fr" {
        format!("{hidden} mesure(s) sur {cols} non dessinée(s) : la spécification ne les lit pas")
    } else {
        format!("{hidden} of {cols} measures not drawn: the spec does not read them")
    }
}

/// [`layout_themed`] with the data points drawn (the primitives are the same).
pub fn layout_hits(given: &Spec, data: &SpecData, w: f64, h: f64, theme: &Value) -> (Vec<Prim>, Vec<Hit>) {
    let mut out = Vec::new();
    let mut hits = Vec::new();
    if !(w.is_finite() && h.is_finite()) {
        return (out, hits);
    }
    let look = crate::look::Look::of(Some(&given.style), Some(theme));
    let (tick, size) = (look.tick, look.name);
    let pal = if given.own_palette {
        given.palette.clone()
    } else {
        look.palette.clone()
    };
    let (spec, hidden) = resolve(given, data, &pal);
    let spec = &spec;
    let tx = text_set(spec, "x", tick, look.text);
    let ty = text_set(spec, "y", tick, look.text);
    let txt = text_set(spec, "xTitle", size, look.text);
    let tyt = text_set(spec, "yTitle", size, look.text);
    let tl = text_set(spec, "legend", size, look.text);
    let n = data.rows();
    let pad = spec.padding;
    // the legend: the named marks, else the categories of a mark coloured by category
    let mut items: Vec<(String, String)> = spec
        .marks
        .iter()
        .enumerate()
        .filter_map(|(mi, m)| {
            let name = m.name.as_ref()?;
            let c = match &m.color {
                Some(Color::Fixed(c)) => c.clone(),
                _ => pal.get(mi % pal.len().max(1)).cloned().unwrap_or_default(),
            };
            Some((name.clone(), c))
        })
        .collect();
    if items.is_empty() && spec.marks.iter().any(|m| m.color == Some(Color::ByCategory)) {
        items = (0..n.min(20))
            .map(|i| {
                (
                    cat_label(data, i),
                    pal.get(i % pal.len().max(1)).cloned().unwrap_or_default(),
                )
            })
            .collect();
    }
    let mut legend = spec.legend.unwrap_or(true) && !items.is_empty();
    // a line saying which measures are not drawn, then the legend row, then half the top y tick
    // label (10 px text centred on its tick) and 1 px; larger text takes more room
    let mut notice = (hidden > 0).then(|| notice_text(hidden, data.values.len().min(MAX_VALUES), &data.lang));
    let head_of = |n: bool| pad + if n { 14.0 } else { 0.0 };
    let top_of = |n: bool, legend: bool| {
        head_of(n)
            + if legend { 18.0 + (tl.size - size) } else { 0.0 }
            + if (legend || n) && spec.y.axis {
                6.0 + (ty.size - tick) / 2.0
            } else {
                0.0
            }
    };
    let mut head = head_of(notice.is_some());
    let mut top = top_of(notice.is_some(), legend);
    let left = pad
        + if spec.y.axis { 36.0 * ty.size / tick } else { 0.0 }
        + if spec.y.title.is_some() {
            14.0 + (tyt.size - size)
        } else {
            0.0
        };
    let bottom = h
        - pad
        - if spec.x.axis { 18.0 + (tx.size - tick) } else { 0.0 }
        - if spec.x.title.is_some() {
            14.0 + (txt.size - size)
        } else {
            0.0
        };
    let right = w - pad;
    if notice.is_some() && (right - left < 8.0 || bottom - top < 8.0) {
        // a visual too small for the notice still draws its plot
        notice = None;
        head = head_of(false);
        top = top_of(false, legend);
    }
    if legend && (right - left < 8.0 || bottom - top < 8.0) {
        // nor for the legend
        legend = false;
        top = top_of(false, false);
    }
    if right - left < 8.0 || bottom - top < 8.0 {
        return (out, hits);
    }
    look.push_card(&mut out, w, h);
    // the y domain: every y and y2, and the baseline of bars, areas and rects
    let mut yv: Vec<f64> = Vec::new();
    for m in &spec.marks {
        for e in [&m.y, &m.y2].into_iter().flatten() {
            match e {
                Enc::Field(Ref::Value(k)) => yv.extend((0..n).filter_map(|i| data.value(*k, i))),
                Enc::Num(v) => yv.push(*v),
                _ => {}
            }
        }
    }
    let ydom = domain(&yv, spec.y.zero, spec.y.min, spec.y.max, 5.0);
    let mut f = Frame {
        spec,
        data,
        n,
        left,
        right,
        top,
        bottom,
        y: ydom,
        x: None,
        look,
        pal,
    };
    if spec.x.scale != ScaleKind::Band {
        let mut xv: Vec<f64> = Vec::new();
        for m in &spec.marks {
            for e in [&m.x, &m.x2].into_iter().flatten() {
                xv.extend((0..n).filter_map(|i| f.x_value(e, i)));
            }
        }
        let count = if spec.x.scale == ScaleKind::Time { 4.0 } else { 5.0 };
        f.x = Some(domain(&xv, spec.x.zero, spec.x.min, spec.x.max, count));
    }
    // gridlines
    if spec.y.grid {
        for t in &f.y.ticks {
            let yy = f.ys(*t);
            out.push(line(left, yy, right, yy, f.look.grid, 1.0));
        }
    }
    if spec.x.grid
        && let Some(d) = &f.x
    {
        for t in &d.ticks {
            let xx = f.xs(*t);
            out.push(line(xx, top, xx, bottom, f.look.grid, 1.0));
        }
    }
    // marks, each followed by its labels
    let bars: Vec<usize> = spec
        .marks
        .iter()
        .enumerate()
        .filter(|(_, m)| m.kind == MarkType::Bar)
        .map(|(i, _)| i)
        .collect();
    for (mi, m) in spec.marks.iter().enumerate() {
        mark(&f, m, mi, &bars, &mut out, &mut hits);
    }
    // axes
    if spec.y.axis {
        let yt = spec.text.get("y");
        let u = unit_of(
            yt.and_then(|t| t.get("units")).and_then(Value::as_f64),
            f.y.lo.abs().max(f.y.hi.abs()),
        );
        let d = yt
            .and_then(|t| t.get("decimals"))
            .and_then(Value::as_u64)
            .and_then(|d| u32::try_from(d).ok())
            .unwrap_or_else(|| tick_decimals(f.y.step / u));
        for t in &f.y.ticks {
            out.push(styled(
                text(
                    left - 4.0,
                    f.ys(*t),
                    num_text(*t, Some(d), u, &data.lang),
                    ty.size,
                    &ty.color,
                    "end",
                    0.0,
                ),
                &ty,
            ));
        }
    }
    if spec.x.axis {
        out.push(line(left, bottom, right, bottom, f.look.axis, 1.0));
        let ly = bottom + 9.0 + (tx.size - tick) / 2.0;
        match &f.x {
            None => {
                let labels: Vec<String> = (0..n).map(|i| clip(&cat_label(data, i), 20)).collect();
                let maxw = labels.iter().map(|l| est(l, tx.size)).fold(0.0, f64::max) + 6.0;
                let k = ((n as f64 * maxw / (right - left)).ceil() as usize).max(1);
                for (i, l) in labels.into_iter().enumerate() {
                    if i % k == 0 {
                        let (x0, bw) = f.band(i);
                        let x = inside(x0 + bw / 2.0, &l, tx.size, w);
                        out.push(styled(text(x, ly, l, tx.size, &tx.color, "middle", 0.0), &tx));
                    }
                }
            }
            Some(dx) => {
                let d = tick_decimals(dx.step);
                for t in &dx.ticks {
                    let s = if spec.x.scale == ScaleKind::Time {
                        // a tick of days or more: in the category's date format
                        (dx.step >= MS_PER_DAY)
                            .then(|| crate::fmt::date_label(*t, &data.cat_format, &data.lang))
                            .flatten()
                            .unwrap_or_else(|| date_text(*t, dx.step < MS_PER_DAY))
                    } else {
                        local_num(&fmt_fixed(*t, d), &data.lang)
                    };
                    let x = inside(f.xs(*t), &s, tx.size, w);
                    out.push(styled(text(x, ly, s, tx.size, &tx.color, "middle", 0.0), &tx));
                }
            }
        }
    }
    if let Some(t) = &spec.y.title {
        out.push(styled(
            text(
                pad + 7.0 + (tyt.size - size) / 2.0,
                (top + bottom) / 2.0,
                clip(t, 60),
                tyt.size,
                &tyt.color,
                "middle",
                -90.0,
            ),
            &tyt,
        ));
    }
    if let Some(t) = &spec.x.title {
        out.push(styled(
            text(
                (left + right) / 2.0,
                h - pad - 7.0 - (txt.size - size) / 2.0,
                clip(t, 60),
                txt.size,
                &txt.color,
                "middle",
                0.0,
            ),
            &txt,
        ));
    }
    if let Some(s) = notice {
        let fit = ((w - 2.0 * pad) / (tick * 0.55)).floor().max(1.0) as usize;
        out.push(text(
            pad,
            pad + 6.0,
            clip(&s, fit),
            tick,
            f.look.text,
            "start",
            0.0,
        ));
    }
    if legend {
        let mut x = pad;
        let ly = head + (tl.size - size) / 2.0;
        for (j, (name, c)) in items.iter().enumerate() {
            let name = clip(name, 30);
            let wd = 14.0 + est(&name, tl.size) + 12.0;
            if j > 0 && x + wd - 12.0 > w - pad {
                break;
            }
            out.push(f.look.swatch(x, ly + 4.0, 10.0, c));
            out.push(styled(
                text(x + 14.0, ly + 9.0, name, tl.size, &tl.color, "start", 0.0),
                &tl,
            ));
            x += wd;
        }
    }
    (out, hits)
}

/// The primitives of one mark.
fn mark(f: &Frame<'_>, m: &Mark, mi: usize, bars: &[usize], out: &mut Vec<Prim>, hits: &mut Vec<Hit>) {
    let n = f.n;
    let mut labels: Vec<(Prim, usize)> = Vec::new();
    // the data labels' text settings (their colour by default: the look's ink, on a chip when it
    // has one)
    let tl = text_set(f.spec, "labels", f.look.value, "");
    let mut label = |f: &Frame<'_>, i: usize, x: f64, y: f64, side: char, gap: f64| {
        if let Some(s) = f.label_text(m, i) {
            let mut drawn = Vec::new();
            let t = f.look.value_label(&mut drawn, x, y, s, side, gap, tl.size);
            if let Some(p) = drawn.get_mut(t) {
                if let Prim::Text { fill, .. } = p
                    && !tl.color.is_empty()
                {
                    fill.clone_from(&tl.color);
                }
                *p = styled(p.clone(), &tl);
            }
            labels.extend(drawn.into_iter().map(|p| (p, i)));
        }
    };
    // the primitive just pushed is row i's (`fade`: dimmed when the row is not highlighted)
    let hit_as = |hits: &mut Vec<Hit>, out: &[Prim], i: usize, fade: bool| {
        hits.push(Hit {
            row: i,
            prim: out.len().checked_sub(1),
            at: None,
            label: false,
            fade,
        });
    };
    let hit = |hits: &mut Vec<Hit>, out: &[Prim], i: usize| hit_as(hits, out, i, f.data.faded(i));
    match m.kind {
        MarkType::Bar | MarkType::Rect => {
            let (nb, j) = if m.kind == MarkType::Bar {
                (bars.len().max(1), bars.iter().position(|b| *b == mi).unwrap_or(0))
            } else {
                (1, 0)
            };
            for i in 0..n {
                let Some(v) = m.y.as_ref().and_then(|e| f.y_value(e, i)) else {
                    continue;
                };
                let (x0, x1) = match (&f.x, &m.x, &m.x2) {
                    (None, _, _) => {
                        let (x0, bw) = f.band(i);
                        let sub = bw / nb as f64;
                        (x0 + sub * j as f64, x0 + sub * (j as f64 + 1.0))
                    }
                    (Some(_), Some(a), Some(b)) => {
                        let (Some(a), Some(b)) = (f.x_value(a, i), f.x_value(b, i)) else {
                            continue;
                        };
                        let (a, b) = (f.xs(a), f.xs(b));
                        (a.min(b), a.max(b))
                    }
                    _ => continue,
                };
                let base = f.base(m, i);
                let c = f.color_at(m, mi, i);
                let draw = |v: f64| {
                    let yv = f.ys(v);
                    let (y0, y1) = (yv.min(base), yv.max(base));
                    let mut p = if m.kind == MarkType::Bar {
                        f.look
                            .bar(x0, y0, x1 - x0, y1 - y0, &c, if yv <= base { "b" } else { "t" })
                    } else {
                        f.look
                            .tile(x0, y0, x1 - x0, y1 - y0, &c, f.look.radius() / 2.0, false)
                    };
                    if let Prim::Rect { opacity, .. } = &mut p {
                        *opacity = m.opacity;
                    }
                    p
                };
                // highlighted: a bar of a value from zero draws its highlighted part over the
                // dimmed full value; other bars are dimmed with their row
                let partial = match (&m.y, &m.y2) {
                    (Some(Enc::Field(Ref::Value(k))), None) if f.data.highlighted() => Some(*k),
                    _ => None,
                };
                match partial {
                    Some(k) => {
                        out.push(crate::look::dim(draw(v)));
                        hit_as(hits, out, i, false);
                        if let Some(pv) = crate::look::part(v, f.data.highlight(k, i)) {
                            out.push(draw(pv));
                            hit_as(hits, out, i, false);
                        }
                    }
                    None => {
                        out.push(draw(v));
                        hit(hits, out, i);
                    }
                }
                let yv = f.ys(v);
                let (y0, y1) = (yv.min(base), yv.max(base));
                if yv <= base {
                    label(f, i, (x0 + x1) / 2.0, y0, 'n', 7.0);
                } else {
                    label(f, i, (x0 + x1) / 2.0, y1, 's', 7.0);
                }
            }
        }
        MarkType::Line | MarkType::Area => {
            let mut runs: Vec<Vec<(f64, f64, usize)>> = vec![Vec::new()];
            for i in 0..n {
                let p = match (&m.x, &m.y) {
                    (Some(xe), Some(ye)) => f.x_pos(xe, i).zip(f.y_value(ye, i).map(|v| f.ys(v))),
                    _ => None,
                };
                match p {
                    Some((x, y)) => {
                        if let Some(r) = runs.last_mut() {
                            r.push((r2(x), r2(y), i));
                        }
                    }
                    None => {
                        if runs.last().is_some_and(|r| !r.is_empty()) {
                            runs.push(Vec::new());
                        }
                    }
                }
            }
            let c = f.mark_color(m, mi);
            for r in runs.iter().filter(|r| !r.is_empty()) {
                if m.kind == MarkType::Line {
                    let modern = !f.look.minimal();
                    if r.len() >= 2 {
                        out.push(Prim::Path {
                            points: r.iter().map(|(x, y, _)| (*x, *y)).collect(),
                            stroke: c.clone(),
                            width: r2(f.size_at(m, 0, if modern { 2.5 } else { 2.0 })),
                        });
                    }
                    // Modern and Bold: a marker on each point of a short line, ringed in the surface
                    if modern && r.len() <= MAX_LINE_DOTS {
                        for (x, y, _) in r {
                            out.push(f.look.dot(*x, *y, 3.5, &c, 1.0));
                        }
                    }
                } else {
                    let mut pts: Vec<(f64, f64)> = r.iter().map(|(x, y, _)| (*x, *y)).collect();
                    for (x, _, i) in r.iter().rev() {
                        pts.push((*x, r2(f.base(m, *i))));
                    }
                    out.push(Prim::Poly {
                        points: pts,
                        fill: c.clone(),
                        opacity: m.opacity,
                    });
                }
                for (x, y, i) in r {
                    hits.push(Hit {
                        row: *i,
                        prim: None,
                        at: Some((*x, *y)),
                        label: false,
                        fade: f.data.faded(*i),
                    });
                    label(f, *i, *x, *y, 'n', 8.0);
                }
            }
        }
        MarkType::Point | MarkType::Text => {
            for i in 0..n {
                let (Some(xe), Some(ye)) = (&m.x, &m.y) else {
                    break;
                };
                let (Some(x), Some(v)) = (f.x_pos(xe, i), f.y_value(ye, i)) else {
                    continue;
                };
                let y = f.ys(v);
                if m.kind == MarkType::Point {
                    let r = f.size_at(m, i, 4.0);
                    out.push(f.look.dot(x, y, r, &f.color_at(m, mi, i), m.opacity));
                    hit(hits, out, i);
                    label(f, i, x, y, 'n', r + 6.0);
                } else if let Some(s) = f.label_text(m, i) {
                    let size = f.size_at(m, i, 11.0);
                    out.push(text(x, y, s, r2(size), &f.color_at(m, mi, i), "middle", 0.0));
                    hit(hits, out, i);
                }
            }
        }
        MarkType::Rule => {
            let c = f.mark_color(m, mi);
            let wd = f.size_at(m, 0, 1.0);
            match (&m.x, &m.y) {
                (None, Some(Enc::Num(v))) => {
                    let y = f.ys(*v);
                    out.push(line(f.left, y, f.right, y, &c, wd));
                }
                (None, Some(ye)) => {
                    for i in 0..n {
                        let Some(v) = f.y_value(ye, i) else { continue };
                        let y = f.ys(v);
                        let (x0, x1) = match &f.x {
                            None => {
                                let (x0, bw) = f.band(i);
                                (x0, x0 + bw)
                            }
                            Some(_) => match f.x_pos(&Enc::Field(Ref::Category), i) {
                                Some(x) => (x - 5.0, x + 5.0),
                                None => continue,
                            },
                        };
                        out.push(line(x0, y, x1, y, &c, wd));
                        hit(hits, out, i);
                    }
                }
                (Some(xe @ (Enc::Num(_) | Enc::Text(_))), None) => {
                    if let Some(x) = f.x_pos(xe, 0) {
                        out.push(line(x, f.top, x, f.bottom, &c, wd));
                    }
                }
                (Some(xe), None) => {
                    for i in 0..n {
                        if let Some(x) = f.x_pos(xe, i) {
                            out.push(line(x, f.top, x, f.bottom, &c, wd));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    for (p, i) in labels {
        out.push(p);
        hits.push(Hit {
            row: i,
            prim: out.len().checked_sub(1),
            at: None,
            label: true,
            fade: f.data.faded(i),
        });
    }
}

/// The golden tests' form of a layout.
pub fn layout_json(spec: &Spec, data: &SpecData, w: f64, h: f64) -> Value {
    Value::Array(layout(spec, data, w, h).iter().map(Prim::json).collect())
}

// ---------------------------------------------------------------------------------------------
// Drawing (report.render, PDF export, the app's canvas)
// ---------------------------------------------------------------------------------------------

fn rgb(c: &str) -> Rgb {
    Rgb::parse(c).unwrap_or(Rgb(0x55, 0x55, 0x55))
}

/// The report theme's data colours as `#RRGGBB` (what BIcraft's visuals follow by default).
pub(crate) fn theme_hex(ctx: &Ctx<'_>) -> Vec<String> {
    ctx.colors
        .iter()
        .take(16)
        .map(|c| format!("#{:02X}{:02X}{:02X}", c.0, c.1, c.2))
        .collect()
}

/// The report theme as BIcraft's visuals read it: `{colors, background}` (`#RRGGBB`).
pub(crate) fn theme_value(ctx: &Ctx<'_>) -> Value {
    let bg = format!("#{:02X}{:02X}{:02X}", ctx.bg.0, ctx.bg.1, ctx.bg.2);
    crate::look::theme_json_bg(&theme_hex(ctx), &bg)
}

/// `f` at `weight` (400 keeps the font's own).
fn weighted(mut f: crate::fonts::Font, weight: u16) -> crate::fonts::Font {
    if weight != 400 {
        f.weight = weight;
    }
    f
}

/// A decorated shape (`shape(dx, dy, grow)`: the shape moved and grown): its soft shadow (three
/// widening translucent copies below it; none in high contrast), its fill (a vertical gradient
/// from `deco.grad` at `top` down to `fill` at `bottom`), then its edge at `alpha`, the mark's
/// own opacity (a dimmed mark's edge fades with it, as the SVG element's opacity fades its
/// stroke in Power BI).
#[allow(clippy::too_many_arguments)]
fn decorated(
    cv: &mut Canvas,
    shape: &dyn Fn(f32, f32, f32) -> Option<tiny_skia::Path>,
    top: f32,
    bottom: f32,
    fill: &str,
    opacity: f32,
    alpha: f32,
    deco: &Deco,
) {
    let s = (deco.shadow as f32).clamp(0.0, 24.0);
    if s > 0.0 && !crate::canvas::high_contrast_on() {
        for k in 1..=3 {
            let grow = s * k as f32 / 6.0;
            if let Some(p) = shape(0.0, s / 3.0, grow) {
                cv.fill_path(&p, Rgb(0x10, 0x18, 0x28), 0.05 * opacity.clamp(0.0, 1.0));
            }
        }
    }
    let Some(p) = shape(0.0, 0.0, 0.0) else {
        return;
    };
    let base = rgb(fill);
    match Rgb::parse(&deco.grad) {
        Some(t) => cv.fill_gradient(&p, t, base, top, bottom, opacity),
        None => cv.fill_path(&p, base, opacity),
    }
    if let Some(c) = Rgb::parse(&deco.stroke)
        && deco.sw > 0.0
    {
        cv.stroke(&p, c, alpha, deco.sw as f32, None);
    }
}

/// `c` faded towards white to opacity `a`.
fn faded(c: Rgb, a: f64) -> Rgb {
    if a >= 1.0 {
        return c;
    }
    let ch = |x: u8| (f64::from(x) * a + 255.0 * (1.0 - a)).round().clamp(0.0, 255.0) as u8;
    Rgb(ch(c.0), ch(c.1), ch(c.2))
}

fn pt(p: &[(f64, f64)]) -> Vec<(f32, f32)> {
    p.iter().map(|(x, y)| (*x as f32, *y as f32)).collect()
}

/// Draws the primitives onto a visual's body, primitive `i` at opacity `alpha(i)` (a point the
/// selection leaves out is drawn at [`crate::hits::DIM`]; text is faded towards white).
pub(crate) fn draw_with(cv: &mut Canvas, prims: &[Prim], ctx: &Ctx<'_>, alpha: &dyn Fn(usize) -> f64) {
    for (i, p) in prims.iter().enumerate() {
        let a = alpha(i).clamp(0.0, 1.0);
        match p {
            Prim::Rect {
                x,
                y,
                w,
                h,
                fill,
                opacity,
                deco,
            } => {
                let (x, y, w, h) = (*x as f32, *y as f32, *w as f32, *h as f32);
                if !deco.role.is_empty() && crate::canvas::high_contrast_on() {
                    // a card or a label's chip: high contrast draws the plain marks only
                    continue;
                }
                if *deco == Deco::default() {
                    cv.rect(x, y, w, h, rgb(fill), (*opacity * a) as f32);
                } else {
                    let shape = |dx: f32, dy: f32, grow: f32| {
                        corner_rect_path(
                            x - grow + dx,
                            y - grow + dy,
                            w + 2.0 * grow,
                            h + 2.0 * grow,
                            deco.rx as f32 + grow,
                            deco.sq,
                        )
                    };
                    decorated(cv, &shape, y, y + h, fill, (*opacity * a) as f32, a as f32, deco);
                }
            }
            Prim::Line {
                x1,
                y1,
                x2,
                y2,
                stroke,
                width,
            } => cv.line_a(
                *x1 as f32,
                *y1 as f32,
                *x2 as f32,
                *y2 as f32,
                rgb(stroke),
                a as f32,
                *width as f32,
                None,
                false,
            ),
            Prim::Path {
                points,
                stroke,
                width,
            } => {
                cv.polyline(&pt(points), rgb(stroke), a as f32, *width as f32, None);
            }
            Prim::Poly {
                points,
                fill,
                opacity,
            } => cv.polygon(&pt(points), rgb(fill), (*opacity * a) as f32),
            Prim::Circle {
                cx,
                cy,
                r,
                fill,
                opacity,
                deco,
            } => {
                if *deco == Deco::default() {
                    cv.circle(
                        *cx as f32,
                        *cy as f32,
                        *r as f32,
                        Some((rgb(fill), (*opacity * a) as f32)),
                        None,
                    );
                } else {
                    let (cx, cy, r) = (*cx as f32, *cy as f32, (*r as f32).max(0.05));
                    let shape = |dx: f32, dy: f32, grow: f32| {
                        tiny_skia::PathBuilder::from_circle(cx + dx, cy + dy, r + grow)
                    };
                    decorated(
                        cv,
                        &shape,
                        cy - r,
                        cy + r,
                        fill,
                        (*opacity * a) as f32,
                        a as f32,
                        deco,
                    );
                }
            }
            Prim::Text {
                x,
                y,
                text,
                size,
                fill,
                anchor,
                rotate,
                weight,
                font: tf,
            } => {
                let a = match *anchor {
                    "middle" => Anchor::Middle,
                    "end" => Anchor::End,
                    _ => Anchor::Start,
                };
                cv.text(
                    ctx.fonts,
                    &Text::new(
                        text,
                        weighted(
                            if tf.family.is_empty() {
                                font(ctx, *size as f32)
                            } else {
                                crate::fonts::Font::new(&tf.family, *size as f32)
                            },
                            *weight,
                        )
                        .italic(tf.italic),
                        faded(rgb(fill), alpha(i).clamp(0.0, 1.0)),
                    )
                    .underline(tf.underline)
                    .anchor(a)
                    .base(Base::Central)
                    .rotate(*rotate as f32),
                    *x as f32,
                    *y as f32,
                );
            }
        }
    }
}

/// The body of a BIcraft Spec visual: its spec (the snapshot's `spec`) over its data; an error for a
/// spec that does not parse (the engine checks it before storing it).
pub(crate) fn spec_body(
    cv: &mut Canvas,
    v: &Value,
    ds: &Dataset,
    b: Bx,
    ctx: &Ctx<'_>,
) -> Result<(), String> {
    let spec = parse(crate::data::get(v, "spec"))?;
    let data = if ds.empty {
        SpecData::default()
    } else {
        data_of(ds, &ctx.ui_lang)
    };
    let (prims, hits) = layout_hits(&spec, &data, f64::from(b.w), f64::from(b.h), &theme_value(ctx));
    // one target per data row: its marks, its line points and its labels (those the layout fades
    // in a target of their own); the selection ids are the category's (the TypeScript's
    // `categoryIds`)
    let mut targets: Vec<crate::own_hits::Target> = Vec::new();
    let mut of_row: std::collections::HashMap<(usize, bool), usize> = std::collections::HashMap::new();
    for h in &hits {
        let k = *of_row.entry((h.row, h.fade)).or_insert_with(|| {
            targets.push(crate::own_hits::Target {
                rows: vec![h.row],
                tip: crate::own_hits::row_tip(ds, h.row, ctx),
                faded: h.fade,
                ..crate::own_hits::Target::default()
            });
            targets.len() - 1
        });
        if let Some(t) = targets.get_mut(k) {
            t.prims.extend(h.prim);
            t.at.extend(h.at);
        }
    }
    let ids = crate::own_hits::id_cols(ds, &["category"]);
    crate::own_hits::draw(cv, &prims, &targets, v, ds, &ids, ctx);
    Ok(())
}

#[cfg(test)]
#[path = "spec_tests.rs"]
mod tests;
