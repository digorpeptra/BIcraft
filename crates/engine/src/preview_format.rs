//! Conditional formatting evaluated against a visual's rows ([`super`], `data.cell_styles`).
//!
//! The rules are read with the M2.5 decoder (`visual_props::extract_props`, its `conditional`
//! list), so exactly the shapes bicraft writes and Desktop writes for them are understood:
//! - `gradient` (`FillRule` `linearGradient2`/`3`): stops without a value take the lowest /
//!   middle / highest value of the rule field over the visual's rows; colors are interpolated
//!   in RGB; blank values follow `empty` (`zero`, `none` (default), or a color);
//! - `rules` (`Conditional` cases): the first case whose conditions hold gives the color, else
//!   `else`;
//! - `field_value`: the rule field's value is the color;
//! - `data_bars`: a bar from the axis (0, or the bound when all values are on one side) to the
//!   value, as fractions of the cell width;
//! - `icons`: the icon of the first case whose conditions hold, else `else`, with its position
//!   (`before`, `after`, `only`) and vertical alignment;
//! - `web_url`: the rule field's text, when it is an `http(s)` URL, is the cell's link.
//!
//! Targets: table/matrix `background` / `font_color` / `data_bars` / `icon` / `web_url` (the
//! `series` column; `occurrence` picks a field bound more than once), card `font_color`, chart
//! `data_color` (one color per row for the series' bars). The rule field is read from the row: a
//! bound field, or a hidden field the evaluator adds ([`Rules::fields`]).

use bicraft_model::SemanticModel;
use bicraft_report::{FieldRef, Visual};
use serde_json::{Map, Value, json};

use super::{Out, out_num};
use crate::refs::resolve_field;

/// Most rules evaluated per visual.
const MAX_RULES: usize = 50;
/// Most cases per rule.
const MAX_CASES: usize = 50;

/// What a rule styles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Background,
    Font,
    Bars,
    Fill,
    Icon,
    Url,
    /// A chart's data label colour (`labels.color`).
    Label,
    /// A card's category label colour (`categoryLabels.color`).
    Category,
    /// One value for the whole visual (`visualContainerObjects`), named by its
    /// `visual.conditional` target (`title_color`, `title_text`, `visual_background`…).
    Visual(&'static str),
}

/// The container targets and the visual objects' whole-visual colours (axes, legend, gauge,
/// KPI) evaluated once per visual (`data.visual_styles`); the `_text` ones are text, the others
/// colours.
const VISUAL_TARGETS: &[&str] = &[
    "title_text",
    "title_color",
    "title_background",
    "subtitle_text",
    "subtitle_color",
    "visual_background",
    "border_color",
    "category_axis_color",
    "category_title_color",
    "value_axis_color",
    "value_title_color",
    "legend_color",
    "target_color",
    "target_label_color",
    "callout_color",
    "indicator_color",
    "goal_color",
    "distance_color",
    "good_color",
    "neutral_color",
    "bad_color",
];

/// The visual objects' whole-visual colours named as `{object, property}`: (object, property,
/// [`VISUAL_TARGETS`] key).
const WHOLE_TARGETS: &[(&str, &str, &str)] = &[
    ("categoryAxis", "labelColor", "category_axis_color"),
    ("categoryAxis", "titleColor", "category_title_color"),
    ("valueAxis", "labelColor", "value_axis_color"),
    ("valueAxis", "titleColor", "value_title_color"),
    ("legend", "labelColor", "legend_color"),
    ("dataPoint", "target", "target_color"),
    ("target", "color", "target_label_color"),
    ("calloutValue", "color", "callout_color"),
    ("indicator", "fontColor", "indicator_color"),
    ("goals", "goalFontColor", "goal_color"),
    ("goals", "distanceFontColor", "distance_color"),
    ("status", "goodColor", "good_color"),
    ("status", "neutralColor", "neutral_color"),
    ("status", "badColor", "bad_color"),
];

impl Slot {
    fn key(self) -> &'static str {
        match self {
            Slot::Background => "bg",
            Slot::Font => "fg",
            Slot::Bars => "bar",
            Slot::Fill => "fill",
            Slot::Icon => "icon",
            Slot::Url => "url",
            Slot::Label => "label",
            Slot::Category => "cat",
            Slot::Visual(k) => k,
        }
    }

    fn text(self) -> bool {
        matches!(self, Slot::Visual(k) if k.ends_with("_text"))
    }
}

/// Longest web URL drawn as a link.
const MAX_URL: usize = 2048;

#[derive(Debug, Clone)]
struct Stop {
    color: [u8; 3],
    value: Option<f64>,
}

#[derive(Debug, Clone)]
struct Case {
    /// (op, bound): `=`, `>`, `>=`, `<`, `<=`; a `None` bound is "blank".
    conds: Vec<(String, Option<f64>)>,
    /// The color (rules) or icon name (icons).
    color: String,
    /// The bounds are percents of the range from the lowest to the highest value.
    percent: bool,
}

impl Case {
    /// Whether `x` meets every condition, Percent bounds read over `lo..hi`.
    fn holds(&self, x: Option<f64>, lo: f64, hi: f64) -> bool {
        self.conds.iter().all(|(op, b)| {
            let b = match (self.percent, b) {
                (true, Some(p)) if lo.is_finite() && hi.is_finite() => Some(lo + (hi - lo) * p / 100.0),
                (true, Some(_)) => return false,
                _ => *b,
            };
            holds(op, x, b)
        })
    }
}

#[derive(Debug, Clone)]
enum Kind {
    Gradient {
        stops: Vec<Stop>,
        /// `None`: blank values get no color; `Some(None)`: as zero; `Some(Some(c))`: that color.
        empty: Option<Option<String>>,
    },
    Rules {
        cases: Vec<Case>,
        default: Option<String>,
    },
    FieldValue,
    /// The field's value as text (a title from a field).
    Text,
    Bars {
        positive: String,
        negative: String,
        /// The axis line's colour (`axisColor`), drawn where 0 falls inside the bar's range.
        axis: Option<String>,
        /// Right to left (`reverseDirection`).
        reverse: bool,
        /// Show bar only (`hideText`).
        hide_text: bool,
        min: Option<f64>,
        max: Option<f64>,
    },
    Icons {
        cases: Vec<Case>,
        default: Option<String>,
        position: String,
        align: String,
    },
    WebUrl,
}

#[derive(Debug, Clone)]
struct Rule {
    slot: Slot,
    /// The column formatted (`None`: the first value field).
    series: Option<FieldRef>,
    /// Which binding of `series` (0: the first) when it is bound more than once.
    occurrence: usize,
    /// The field whose values drive the rule (`None` for data bars: the series itself).
    field: Option<FieldRef>,
    kind: Kind,
}

/// The conditional formatting rules of one visual.
#[derive(Debug, Clone, Default)]
pub(super) struct Rules {
    rules: Vec<Rule>,
}

fn hex(c: &str) -> Option<[u8; 3]> {
    let h = c.strip_prefix('#')?;
    let h = match h.len() {
        3 => h.chars().flat_map(|x| [x, x]).collect::<String>(),
        6 | 8 => h.get(..6)?.to_string(),
        _ => return None,
    };
    let n = u32::from_str_radix(&h, 16).ok()?;
    Some([(n >> 16) as u8, (n >> 8) as u8, n as u8])
}

fn color_text(c: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2])
}

fn mix(a: [u8; 3], b: [u8; 3], t: f64) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    let ch = |i: usize| {
        let (x, y) = (
            f64::from(a.get(i).copied().unwrap_or(0)),
            f64::from(b.get(i).copied().unwrap_or(0)),
        );
        (x + (y - x) * t).round().clamp(0.0, 255.0) as u8
    };
    [ch(0), ch(1), ch(2)]
}

fn num(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_f64).filter(|x| x.is_finite())
}

fn stop(v: &Value) -> Option<Stop> {
    match v {
        Value::String(s) => Some(Stop {
            color: hex(s)?,
            value: None,
        }),
        Value::Object(o) => Some(Stop {
            color: hex(o.get("color")?.as_str()?)?,
            value: num(o.get("value")),
        }),
        _ => None,
    }
}

/// An icon name as the snapshot carries it (letters, digits, `_`, `-`).
fn icon_ok(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 100
        && n.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// A case of `rules` (`value_key` "color") or `icons` ("icon").
fn case(v: &Value, value_key: &str) -> Option<Case> {
    let o = v.as_object()?;
    let color = o.get(value_key)?.as_str()?.to_string();
    if value_key == "color" {
        hex(&color)?;
    } else if !icon_ok(&color) {
        return None;
    }
    let mut conds = Vec::new();
    match o.get("op").and_then(Value::as_str) {
        Some("between") => {
            let vs = o.get("values")?.as_array()?;
            conds.push((">=".to_string(), Some(num(vs.first())?)));
            conds.push(("<=".to_string(), Some(num(vs.get(1))?)));
        }
        Some(op) => conds.push((op.to_string(), num(o.get("value")))),
        None => {
            for (k, op) in [
                ("gt", ">"),
                ("gte", ">="),
                ("lt", "<"),
                ("lte", "<="),
                ("eq", "="),
            ] {
                if let Some(x) = o.get(k) {
                    conds.push((op.to_string(), num(Some(x))));
                }
            }
        }
    }
    let percent = o.get("percent").and_then(Value::as_bool).unwrap_or(false);
    (!conds.is_empty()).then_some(Case {
        conds,
        color,
        percent,
    })
}

fn slot_of(target: &Value) -> Option<Slot> {
    match target {
        Value::String(s) => match s.as_str() {
            "background" => Some(Slot::Background),
            "font_color" => Some(Slot::Font),
            "data_bars" => Some(Slot::Bars),
            "data_color" => Some(Slot::Fill),
            "icon" => Some(Slot::Icon),
            "web_url" => Some(Slot::Url),
            "label_color" => Some(Slot::Label),
            "category_color" => Some(Slot::Category),
            t => VISUAL_TARGETS.iter().find(|k| **k == t).map(|k| Slot::Visual(k)),
        },
        Value::Object(o) => match (
            o.get("object").and_then(Value::as_str)?,
            o.get("property").and_then(Value::as_str)?,
        ) {
            ("values", "backColor") => Some(Slot::Background),
            ("values", "fontColor") | ("labels", "color") => Some(Slot::Font),
            ("dataPoint", "fill") => Some(Slot::Fill),
            ("values", "icon") => Some(Slot::Icon),
            ("values", "webURL") => Some(Slot::Url),
            (obj, prop) => WHOLE_TARGETS
                .iter()
                .find(|(o, p, _)| *o == obj && *p == prop)
                .map(|(.., k)| Slot::Visual(k)),
        },
        _ => None,
    }
}

impl Rules {
    pub(super) fn of(v: &Visual, model: Option<&SemanticModel>) -> Rules {
        // rule inputs written as `SelectRef` (Desktop's matrices) read as the bound field
        let resolved = crate::visual_conditional::with_resolved_inputs(v);
        let props = crate::visual_props::extract_props(resolved.as_ref().unwrap_or(v));
        let mut rules = Vec::new();
        let field = |t: Option<&Value>| -> Option<FieldRef> {
            resolve_field(model, t?.as_str()?).ok().map(|r| r.field)
        };
        for item in props
            .get("conditional")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .take(MAX_RULES)
        {
            if item.get("apply_to").and_then(Value::as_str) == Some("totals") {
                continue;
            }
            let Some(slot) = item.get("target").and_then(slot_of) else {
                continue;
            };
            let series = field(item.get("series"));
            let occurrence = item
                .get("occurrence")
                .and_then(Value::as_u64)
                .and_then(|n| usize::try_from(n).ok())
                .map_or(0, |n| n.saturating_sub(1));
            let Some((name, r)) = item
                .get("rule")
                .and_then(Value::as_object)
                .and_then(|o| o.iter().next())
            else {
                continue;
            };
            let kind = match name.as_str() {
                "gradient" => {
                    let stops: Option<Vec<Stop>> = ["min", "mid", "max"]
                        .iter()
                        .filter_map(|k| r.get(*k))
                        .map(stop)
                        .collect();
                    let Some(stops) = stops.filter(|s| s.len() >= 2) else {
                        continue;
                    };
                    let empty = match r.get("empty").and_then(Value::as_str) {
                        Some("zero") => Some(None),
                        Some(c) if hex(c).is_some() => Some(Some(c.to_string())),
                        _ => None,
                    };
                    Kind::Gradient { stops, empty }
                }
                "rules" => {
                    let cases: Vec<Case> = r
                        .get("cases")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .take(MAX_CASES)
                        .filter_map(|c| case(c, "color"))
                        .collect();
                    let default = r
                        .get("else")
                        .and_then(Value::as_str)
                        .filter(|c| hex(c).is_some())
                        .map(str::to_string);
                    Kind::Rules { cases, default }
                }
                "icons" if slot == Slot::Icon => {
                    let cases: Vec<Case> = r
                        .get("cases")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .take(MAX_CASES)
                        .filter_map(|c| case(c, "icon"))
                        .collect();
                    let default = r
                        .get("else")
                        .and_then(Value::as_str)
                        .filter(|c| icon_ok(c))
                        .map(str::to_string);
                    let pick = |k: &str, ok: &[&str], d: &str| {
                        r.get(k)
                            .and_then(Value::as_str)
                            .filter(|x| ok.contains(x))
                            .unwrap_or(d)
                            .to_string()
                    };
                    Kind::Icons {
                        cases,
                        default,
                        position: pick("position", &["before", "after", "only"], "before"),
                        align: pick("align", &["top", "middle", "bottom"], "top"),
                    }
                }
                "web_url" if slot == Slot::Url => Kind::WebUrl,
                "field_value" if slot == Slot::Url => Kind::WebUrl,
                "field_value" if slot.text() => Kind::Text,
                _ if slot.text() => continue,
                "field_value" if r.get("as").and_then(Value::as_str) != Some("value") => Kind::FieldValue,
                // data bars are read from the visual's objects below (every shape Desktop writes)
                _ => continue,
            };
            let field = if matches!(kind, Kind::Bars { .. }) {
                None
            } else {
                match field(r.get("field")) {
                    Some(f) => Some(f),
                    None => continue,
                }
            };
            rules.push(Rule {
                slot,
                series,
                occurrence,
                field,
                kind,
            });
        }
        rules.extend(data_bars(v));
        Rules { rules }
    }

    /// The fields the rules read.
    pub(super) fn fields(&self) -> impl Iterator<Item = &FieldRef> {
        self.rules
            .iter()
            .filter(|r| !matches!(r.slot, Slot::Visual(_)))
            .filter_map(|r| r.field.as_ref())
    }

    /// The fields the whole-visual rules (title, background, border) read, each once.
    pub(super) fn visual_fields(&self) -> Vec<FieldRef> {
        let mut out: Vec<FieldRef> = Vec::new();
        for r in &self.rules {
            if let (Slot::Visual(_), Some(f)) = (r.slot, &r.field)
                && !out.contains(f)
            {
                out.push(f.clone());
            }
        }
        out
    }

    /// `data.visual_styles`: `{title_color?, title_text?, …}` from the whole-visual rules, each
    /// rule field's value (`value(f)`, in the visual's filter context without its grouping) being
    /// the only value (a gradient whose stops have no value gives its lowest colour).
    pub(super) fn visual_styles(&self, value: &dyn Fn(&FieldRef) -> Option<Out>) -> Map<String, Value> {
        let mut out = Map::new();
        for rule in &self.rules {
            let Slot::Visual(key) = rule.slot else {
                continue;
            };
            if out.contains_key(key) {
                continue;
            }
            let Some(cell) = rule.field.as_ref().and_then(value) else {
                continue;
            };
            let x = out_num(&cell);
            let text = match &cell {
                Out::Cell(bicraft_vertipaq::CellValue::Text(t)) => Some(t.as_str()),
                _ => None,
            };
            let style = match &rule.kind {
                Kind::Text => text.filter(|t| t.len() <= MAX_URL).map(|t| json!(t)),
                Kind::Gradient { stops, empty } => {
                    let x = match (x, empty) {
                        (Some(x), _) => Some(x),
                        (None, Some(None)) => Some(0.0),
                        (None, Some(Some(c))) => {
                            out.insert(key.into(), json!(c));
                            continue;
                        }
                        (None, None) => None,
                    };
                    x.and_then(|x| gradient(stops, x, x, x))
                        .map(|c| json!(color_text(c)))
                }
                Kind::Rules { cases, default } => {
                    let at = x.unwrap_or(f64::NAN);
                    cases
                        .iter()
                        .find(|c| c.holds(x, at, at))
                        .map(|c| c.color.clone())
                        .or_else(|| default.clone())
                        .map(|c| json!(c))
                }
                Kind::FieldValue => text.filter(|t| hex(t).is_some()).map(|t| json!(t)),
                Kind::Bars { .. } | Kind::Icons { .. } | Kind::WebUrl => None,
            };
            if let Some(s) = style {
                out.insert(key.into(), s);
            }
        }
        out
    }

    /// `data.cell_styles` for the first `cap` rows: `{r, c, bg?, fg?, fill?, bar?, icon?, url?}`
    /// per styled cell (`c`: index among the visible columns). `pos(f, k)` finds the cell of the
    /// `k`-th binding of a field in a row.
    pub(super) fn styles(
        &self,
        rows: &[Vec<Out>],
        cap: usize,
        pos: &dyn Fn(&FieldRef, usize) -> Option<usize>,
        shown: usize,
    ) -> Vec<Value> {
        if self.rules.is_empty() {
            return Vec::new();
        }
        // (row, column) → style object
        let mut cells: Cells = Vec::new();
        for rule in &self.rules {
            if matches!(rule.slot, Slot::Visual(_)) {
                continue;
            }
            let col = match &rule.series {
                Some(f) => pos(f, rule.occurrence),
                // the first value: a non-group cell is a number in some row
                None => (0..shown).find(|i| rows.iter().any(|r| r.get(*i).and_then(out_num).is_some())),
            };
            let Some(col) = col.filter(|c| *c < shown) else {
                continue;
            };
            let src = match &rule.field {
                Some(f) => pos(f, 0),
                None => Some(col),
            };
            let Some(src) = src else {
                continue;
            };
            let values: Vec<Option<f64>> = rows.iter().map(|r| r.get(src).and_then(out_num)).collect();
            let (lo, hi) = values
                .iter()
                .flatten()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), x| {
                    (a.min(*x), b.max(*x))
                });
            for (r, row) in rows.iter().enumerate().take(cap) {
                let x = values.get(r).copied().flatten();
                let style: Option<Value> = match &rule.kind {
                    Kind::Gradient { stops, empty } => {
                        let x = match (x, empty) {
                            (Some(x), _) => Some(x),
                            (None, Some(None)) => Some(0.0),
                            (None, Some(Some(c))) => {
                                push(&mut cells, r, col, rule.slot, json!(c));
                                None
                            }
                            (None, None) => None,
                        };
                        x.and_then(|x| gradient(stops, lo, hi, x))
                            .map(|c| json!(color_text(c)))
                    }
                    Kind::Rules { cases, default } => cases
                        .iter()
                        .find(|c| c.holds(x, lo, hi))
                        .map(|c| c.color.clone())
                        .or_else(|| default.clone())
                        .map(|c| json!(c)),
                    Kind::FieldValue => match row.get(src) {
                        Some(Out::Cell(bicraft_vertipaq::CellValue::Text(t))) if hex(t).is_some() => {
                            Some(json!(t))
                        }
                        _ => None,
                    },
                    Kind::Text => None,
                    Kind::Icons {
                        cases,
                        default,
                        position,
                        align,
                    } => cases
                        .iter()
                        .find(|c| c.holds(x, lo, hi))
                        .map(|c| c.color.clone())
                        .or_else(|| default.clone())
                        .map(|name| json!({"name": name, "position": position, "align": align})),
                    Kind::WebUrl => match row.get(src) {
                        Some(Out::Cell(bicraft_vertipaq::CellValue::Text(t)))
                            if t.len() <= MAX_URL && is_web_url(t) =>
                        {
                            Some(json!(t))
                        }
                        _ => None,
                    },
                    Kind::Bars {
                        positive,
                        negative,
                        axis: axis_color,
                        reverse,
                        hide_text,
                        min,
                        max,
                    } => x.and_then(|x| {
                        let lo = min.unwrap_or(lo.min(0.0));
                        let hi = max.unwrap_or(hi.max(0.0));
                        if hi.partial_cmp(&lo) != Some(std::cmp::Ordering::Greater) {
                            return None;
                        }
                        // right to left: the cell's fractions are mirrored
                        let f = |v: f64| {
                            let t = ((v - lo) / (hi - lo)).clamp(0.0, 1.0);
                            if *reverse { 1.0 - t } else { t }
                        };
                        let axis = f(0.0);
                        let (a, b) = (axis.min(f(x)), axis.max(f(x)));
                        let round = |v: f64| (v * 1000.0).round() / 1000.0;
                        let mut bar = Map::new();
                        bar.insert("from".into(), json!(round(a)));
                        bar.insert("to".into(), json!(round(b)));
                        bar.insert("color".into(), json!(if x >= 0.0 { positive } else { negative }));
                        // the axis is drawn when it falls inside the cell (negative values shown)
                        if let Some(c) = axis_color
                            && axis > 0.0
                            && axis < 1.0
                        {
                            bar.insert("axis".into(), json!(round(axis)));
                            bar.insert("axis_color".into(), json!(c));
                        }
                        if *hide_text {
                            bar.insert("only".into(), json!(true));
                        }
                        Some(Value::Object(bar))
                    }),
                };
                if let Some(s) = style {
                    push(&mut cells, r, col, rule.slot, s);
                }
            }
        }
        cells
            .into_iter()
            .map(|((r, c), mut m)| {
                m.insert("r".into(), json!(r));
                m.insert("c".into(), json!(c));
                Value::Object(m)
            })
            .collect()
    }
}

/// Styled cells: ((row, column), style).
type Cells = Vec<((usize, usize), Map<String, Value>)>;

/// A literal of `{"expr": {"Literal": {"Value": s}}}` or a solid color.
fn literal_of(v: &Value) -> Option<&str> {
    let v = v.get("solid").and_then(|s| s.get("color")).unwrap_or(v);
    v.get("expr")?.get("Literal")?.get("Value")?.as_str()
}

/// Whether a text is an absolute `http`/`https` URL (anything else is never a link).
fn is_web_url(t: &str) -> bool {
    let lower = t.get(..8).unwrap_or(t).to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://"))
        && !t.chars().any(|c| c.is_control() || c.is_whitespace())
}

/// Data bars (`objects.columnFormatting[].properties.dataBars`, scoped by `selector.metadata`).
fn data_bars(v: &Visual) -> Vec<Rule> {
    let mut refs: Vec<(String, FieldRef)> = Vec::new();
    if let Some(state) = v
        .visual_extra
        .get("query")
        .and_then(|q| q.get("queryState"))
        .and_then(Value::as_object)
    {
        for role in state.values() {
            for p in role
                .get("projections")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(r) = p.get("queryRef").and_then(Value::as_str) {
                    refs.push((r.to_string(), bicraft_report::query::field_ref(p.get("field"))));
                }
            }
        }
    }
    for b in &v.fields {
        if let Some(r) = bicraft_report::query::default_query_ref(&b.field) {
            refs.push((r, b.field.clone()));
        }
    }
    let color = |x: Option<&Value>, d: &str| -> String {
        x.and_then(literal_of)
            .and_then(|t| t.strip_prefix('\'')?.strip_suffix('\''))
            .filter(|c| hex(c).is_some())
            .unwrap_or(d)
            .to_string()
    };
    let number = |x: Option<&Value>| -> Option<f64> {
        let t = literal_of(x?)?;
        t.strip_suffix(['D', 'L', 'M'])
            .unwrap_or(t)
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite())
    };
    let flag = |x: Option<&Value>| {
        x.and_then(literal_of)
            .is_some_and(|t| t.eq_ignore_ascii_case("true"))
    };
    let mut out = Vec::new();
    let list = v
        .visual_extra
        .get("objects")
        .and_then(|o| o.get("columnFormatting"))
        .and_then(Value::as_array);
    for e in list.into_iter().flatten().take(MAX_RULES) {
        let (Some(meta), Some(db)) = (
            e.get("selector")
                .and_then(|s| s.get("metadata"))
                .and_then(Value::as_str),
            e.get("properties").and_then(|p| p.get("dataBars")),
        ) else {
            continue;
        };
        let Some(at) = refs.iter().position(|(r, _)| r == meta) else {
            continue;
        };
        let Some((_, field)) = refs.get(at) else {
            continue;
        };
        // a field bound twice: its second binding has its own queryRef
        let occurrence = refs
            .iter()
            .take(at)
            .filter(|(_, f)| crate::refs::same_field(f, field))
            .count();
        out.push(Rule {
            slot: Slot::Bars,
            series: Some(field.clone()),
            occurrence,
            field: None,
            kind: Kind::Bars {
                positive: color(db.get("positiveColor"), "#5F6B6D"),
                negative: color(db.get("negativeColor"), "#D64554"),
                axis: Some(color(db.get("axisColor"), "#000000")),
                reverse: flag(db.get("reverseDirection")),
                hide_text: flag(db.get("hideText")),
                min: number(db.get("minValue")),
                max: number(db.get("maxValue")),
            },
        });
    }
    out
}

fn push(cells: &mut Cells, r: usize, c: usize, slot: Slot, v: Value) {
    match cells.iter_mut().find(|(k, _)| *k == (r, c)) {
        Some((_, m)) => {
            m.entry(slot.key()).or_insert(v);
        }
        None => {
            let mut m = Map::new();
            m.insert(slot.key().into(), v);
            cells.push(((r, c), m));
        }
    }
}

fn holds(op: &str, x: Option<f64>, b: Option<f64>) -> bool {
    match (x, b) {
        (None, None) => op == "=",
        (Some(_), None) | (None, Some(_)) => false,
        (Some(x), Some(b)) => match op {
            "=" => x == b,
            ">" => x > b,
            ">=" => x >= b,
            "<" => x < b,
            "<=" => x <= b,
            _ => false,
        },
    }
}

/// The gradient color of `x`; stops without a value sit at `lo`, the middle, and `hi`.
fn gradient(stops: &[Stop], lo: f64, hi: f64, x: f64) -> Option<[u8; 3]> {
    let n = stops.len();
    if n < 2 || !lo.is_finite() || !hi.is_finite() {
        return stops.first().map(|s| s.color);
    }
    let at: Vec<f64> = stops
        .iter()
        .enumerate()
        .map(|(i, s)| {
            s.value
                .unwrap_or_else(|| lo + (hi - lo) * i as f64 / (n.saturating_sub(1)) as f64)
        })
        .collect();
    let first = stops.first()?;
    let last = stops.last()?;
    if x <= *at.first()? {
        return Some(first.color);
    }
    if x >= *at.last()? {
        return Some(last.color);
    }
    for i in 0..n.saturating_sub(1) {
        let (a, b) = (*at.get(i)?, *at.get(i + 1)?);
        if x >= a && x <= b {
            let t = if b > a { (x - a) / (b - a) } else { 0.0 };
            return Some(mix(stops.get(i)?.color, stops.get(i + 1)?.color, t));
        }
    }
    Some(last.color)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gradients_and_rules() {
        let stops = vec![
            Stop {
                color: [0, 0, 0],
                value: None,
            },
            Stop {
                color: [255, 255, 255],
                value: None,
            },
        ];
        assert_eq!(gradient(&stops, 0.0, 10.0, 5.0), Some([128, 128, 128]));
        assert_eq!(gradient(&stops, 0.0, 10.0, -1.0), Some([0, 0, 0]));
        assert_eq!(gradient(&stops, 0.0, 10.0, 99.0), Some([255, 255, 255]));
        assert!(holds(">=", Some(2.0), Some(2.0)));
        assert!(holds("=", None, None));
        assert!(!holds("<", None, Some(1.0)));
        assert_eq!(hex("#abc"), Some([0xAA, 0xBB, 0xCC]));
        assert_eq!(hex("bad"), None);
        assert!(is_web_url("https://example.com/a?b=1"));
        assert!(is_web_url("HTTP://example.com"));
        assert!(!is_web_url("javascript:alert(1)"));
        assert!(!is_web_url("https://exa mple.com"));
        assert!(!is_web_url("ftp://example.com"));
    }
}
