//! Error bars (`objects.error`) and the scatter chart's symmetry shading
//! (`objects.plotAreaShading`): the `error_bars` and `symmetry_shading` lines of
//! `visual.analytics` (docs/formats/pbir.md "Chart specifics").
//!
//! The property names and their types are the public report theme schema's (`error` on bar,
//! column, line and combo charts; `plotAreaShading` on the scatter chart). The bounds are one
//! `errorRange` property whose value no public schema describes: BIcraft writes
//! `{kind: "ErrorRange", explicit: {isRelative, upperBound: {expr}, lowerBound: {expr}}}`, its own
//! choice, not checked against a file Power BI Desktop saved (docs/gaps.md G56).

use bicraft_report::FieldRef;
use bicraft_report::query::{field_json, field_ref};

use super::*;

/// Built-in types without an `error` object in the catalog's place (report theme schema 2.157).
pub(super) const ERROR_TYPES: &[&str] = &[
    "barChart",
    "columnChart",
    "clusteredBarChart",
    "clusteredColumnChart",
    "hundredPercentStackedBarChart",
    "hundredPercentStackedColumnChart",
    "lineChart",
    "lineStackedColumnComboChart",
    "lineClusteredColumnComboChart",
    "ribbonChart",
];

/// The `error` properties modelled (others of an entry are kept).
pub(super) const ERROR_PROPS: &[&str] = &[
    "enabled",
    "errorRange",
    "barShow",
    "barColor",
    "barMatchSeriesColor",
    "barWidth",
    "markerShow",
    "markerShape",
    "shadeShow",
    "shadeBandStyle",
    "shadeColor",
    "shadeTransparency",
    "labelShow",
    "tooltipShow",
];
const ERROR_KEYS: &[&str] = &[
    "kind",
    "measure",
    "series",
    "upper",
    "lower",
    "relative",
    "show",
    "bar",
    "color",
    "match_series",
    "width",
    "marker",
    "marker_shape",
    "band",
    "band_style",
    "band_color",
    "transparency",
    "data_label",
    "tooltip",
];
/// The `plotAreaShading` properties modelled.
pub(super) const SHADING_PROPS: &[&str] = &[
    "show",
    "displayName",
    "upperShadingColor",
    "lowerShadingColor",
    "transparency",
];
const SHADING_KEYS: &[&str] = &[
    "kind",
    "show",
    "label",
    "upper_color",
    "lower_color",
    "transparency",
];
/// The theme schema's `error.markerShape` values.
pub(crate) const MARKER_SHAPES: &[&str] = &[
    "circle",
    "square",
    "diamond",
    "triangle",
    "x",
    "shortDash",
    "longDash",
    "plus",
    "none",
];
/// The theme schema's `error.shadeBandStyle` values.
pub(crate) const BAND_STYLES: &[&str] = &["fill", "line", "fillLine"];

#[derive(Clone, Copy)]
enum Kind {
    Bool,
    Color,
    Num(f64, f64),
    Enum(&'static [&'static str]),
}

/// An option of a line: friendly key, PBIR property, its kind.
type Opt = (&'static str, &'static str, Kind);

const ERROR_OPTS: &[Opt] = &[
    ("bar", "barShow", Kind::Bool),
    ("color", "barColor", Kind::Color),
    ("match_series", "barMatchSeriesColor", Kind::Bool),
    ("width", "barWidth", Kind::Num(0.0, 100.0)),
    ("marker", "markerShow", Kind::Bool),
    ("marker_shape", "markerShape", Kind::Enum(MARKER_SHAPES)),
    ("band", "shadeShow", Kind::Bool),
    ("band_style", "shadeBandStyle", Kind::Enum(BAND_STYLES)),
    ("band_color", "shadeColor", Kind::Color),
    ("transparency", "shadeTransparency", Kind::Num(0.0, 100.0)),
    ("data_label", "labelShow", Kind::Bool),
    ("tooltip", "tooltipShow", Kind::Bool),
];
const SHADING_OPTS: &[Opt] = &[
    ("upper_color", "upperShadingColor", Kind::Color),
    ("lower_color", "lowerShadingColor", Kind::Color),
    ("transparency", "transparency", Kind::Num(0.0, 100.0)),
];

/// The PBIR value of option `key` (validated).
fn opt_value(
    vt: Option<&str>,
    object: &str,
    (key, prop, kind): &Opt,
    x: &Value,
    what: &str,
) -> Result<Value, String> {
    let w = format!("{what}.{key}");
    Ok(match kind {
        Kind::Bool => bool_v(get_bool(x, &w)?),
        Kind::Color => color_v(&get_color(x, &w)?),
        Kind::Num(lo, hi) => num_v(get_num(x, &w, *lo, *hi)?, integer_prop(vt, object, prop, false)),
        Kind::Enum(values) => text_v(get_enum(x, &w, values)?),
    })
}

/// The friendly value of option `kind` read from PBIR, or `None` when it is not one BIcraft
/// writes.
fn opt_decode(kind: Kind, x: &Value) -> Option<Value> {
    match kind {
        Kind::Bool => lit_bool(x).map(Value::Bool),
        Kind::Color => color_of(x).filter(|c| is_color(c)).map(Value::String),
        Kind::Num(lo, hi) => lit_num(x).filter(|n| (lo..=hi).contains(n)).map(|n| json!(n)),
        Kind::Enum(values) => lit_text(x)
            .filter(|s| values.contains(&s.as_str()))
            .map(Value::String),
    }
}

/// `{kind: "ErrorRange", explicit: {isRelative, upperBound?, lowerBound?}}`.
fn error_range(upper: Option<&FieldRef>, lower: Option<&FieldRef>, relative: bool) -> Value {
    let mut explicit = Map::new();
    explicit.insert("isRelative".into(), bool_v(relative));
    if let Some(u) = upper {
        explicit.insert("upperBound".into(), json!({"expr": field_json(u)}));
    }
    if let Some(l) = lower {
        explicit.insert("lowerBound".into(), json!({"expr": field_json(l)}));
    }
    json!({"kind": "ErrorRange", "explicit": explicit})
}

fn is_value_field(f: &FieldRef) -> bool {
    matches!(f, FieldRef::Measure { .. } | FieldRef::Aggregation { .. })
}

/// The bound of `{expr: <field>}`: a measure or an aggregation.
fn decode_bound(v: &Value) -> Option<FieldRef> {
    let o = v.as_object().filter(|o| o.len() == 1)?;
    let f = field_ref(o.get("expr"));
    is_value_field(&f).then_some(f)
}

/// (upper, lower, relative) of an `errorRange` value BIcraft writes.
pub(super) fn decode_range(v: &Value) -> Option<(Option<FieldRef>, Option<FieldRef>, bool)> {
    let o = v.as_object().filter(|o| o.len() == 2)?;
    if o.get("kind")?.as_str()? != "ErrorRange" {
        return None;
    }
    let e = o.get("explicit")?.as_object()?;
    if e.keys()
        .any(|k| !matches!(k.as_str(), "isRelative" | "upperBound" | "lowerBound"))
    {
        return None;
    }
    let relative = lit_bool(e.get("isRelative")?)?;
    let upper = match e.get("upperBound") {
        Some(b) => Some(decode_bound(b)?),
        None => None,
    };
    let lower = match e.get("lowerBound") {
        Some(b) => Some(decode_bound(b)?),
        None => None,
    };
    Some((upper, lower, relative))
}

/// The bounds the visual's error bars name now (read back without a model).
fn existing_bounds(v: &Visual) -> Vec<FieldRef> {
    let mut out = Vec::new();
    for e in entries(v, Target::Visual, "error") {
        if let Some((u, l, _)) = e.pointer("/properties/errorRange").and_then(decode_range) {
            out.extend(u);
            out.extend(l);
        }
    }
    out
}

/// A bound's field: the model's (or a bound field), else one the error bars name already.
fn bound_field(ctx: &Ctx<'_>, v: &Visual, text: &str, what: &str) -> Result<FieldRef, String> {
    let f = match ctx.field(text) {
        Ok(f) => f,
        Err(e) => existing_bounds(v)
            .into_iter()
            .find(|f| field_text(f) == text)
            .ok_or_else(|| format!("{what}: {e}"))?,
    };
    if !is_value_field(&f) {
        return Err(format!(
            "{what}: {text} is not a measure or an aggregation (a bound is one value per data point, such as [Upper] or Sum(Sales[High]))"
        ));
    }
    Ok(f)
}

/// An `error_bars` line: `{kind, measure?, upper?, lower?, relative?, show?, bar?, color?,
/// match_series?, width?, marker?, marker_shape?, band?, band_style?, band_color?, transparency?,
/// data_label?, tooltip?}`.
pub(super) fn parse_error(
    o: &Map<String, Value>,
    what: &str,
    v: &Visual,
    ctx: &Ctx<'_>,
) -> Result<Line, String> {
    check_keys(o, ERROR_KEYS, what)?;
    let vt = v.visual_type.as_deref();
    if !has_object(vt, "error", ERROR_TYPES) {
        return Err(format!(
            "{what}: a `{}` visual has no error bars (Power BI offers them on bar, column, line and combo charts)",
            vt.unwrap_or("?")
        ));
    }
    let opt = |k: &str| o.get(k).filter(|x| !x.is_null());
    let w = |k: &str| format!("{what}.{k}");
    let series = match (opt("measure"), opt("series")) {
        (Some(_), Some(_)) => return Err(format!("{what}: give `measure` or `series`, not both")),
        (Some(m), None) | (None, Some(m)) => Some(get_text(m, &w("measure"))?),
        (None, None) => None,
    };
    let query_ref = line_series(v, ctx, series, what)?;
    let bound = |k: &str| -> Result<Option<FieldRef>, String> {
        opt(k)
            .map(|x| get_text(x, &w(k)).and_then(|t| bound_field(ctx, v, t, &w(k))))
            .transpose()
    };
    let (upper, lower) = (bound("upper")?, bound("lower")?);
    if upper.is_none() && lower.is_none() {
        return Err(format!(
            "{what}: error bars need `upper` or `lower` (a measure or an aggregation, such as [Upper])"
        ));
    }
    let relative = opt("relative")
        .map(|x| get_bool(x, &w("relative")))
        .transpose()?
        .unwrap_or(false);
    let show = opt("show")
        .map(|x| get_bool(x, &w("show")))
        .transpose()?
        .unwrap_or(true);
    let mut props: Vec<(&'static str, Value)> = vec![
        ("enabled", bool_v(show)),
        (
            "errorRange",
            error_range(upper.as_ref(), lower.as_ref(), relative),
        ),
    ];
    for spec in ERROR_OPTS {
        if let Some(x) = opt(spec.0) {
            props.push((spec.1, opt_value(vt, "error", spec, x, what)?));
        }
    }
    for (name, x) in &props {
        if *name != "errorRange" {
            crate::visual_catalog::check_object_entry(vt, "error", name, x)
                .map_err(|e| format!("{what}: {e}"))?;
        }
    }
    Ok(Line {
        object: "error",
        metadata: Some(query_ref),
        props,
        raw: None,
    })
}

/// A `symmetry_shading` line (scatter charts): `{kind, show?, label?, upper_color?,
/// lower_color?, transparency?}`.
pub(super) fn parse_shading(o: &Map<String, Value>, what: &str, v: &Visual) -> Result<Line, String> {
    check_keys(o, SHADING_KEYS, what)?;
    let vt = v.visual_type.as_deref();
    if !has_object(vt, "plotAreaShading", &["scatterChart"]) {
        return Err(format!(
            "{what}: a `{}` visual has no symmetry shading (Power BI offers it on scatter charts)",
            vt.unwrap_or("?")
        ));
    }
    let opt = |k: &str| o.get(k).filter(|x| !x.is_null());
    let show = opt("show")
        .map(|x| get_bool(x, &format!("{what}.show")))
        .transpose()?
        .unwrap_or(true);
    let mut props: Vec<(&'static str, Value)> = vec![("show", bool_v(show))];
    if let Some(l) = opt("label") {
        props.push(("displayName", text_v(get_text(l, &format!("{what}.label"))?)));
    }
    for spec in SHADING_OPTS {
        if let Some(x) = opt(spec.0) {
            props.push((spec.1, opt_value(vt, "plotAreaShading", spec, x, what)?));
        }
    }
    for (name, x) in &props {
        crate::visual_catalog::check_object_entry(vt, "plotAreaShading", name, x)
            .map_err(|e| format!("{what}: {e}"))?;
    }
    Ok(Line {
        object: "plotAreaShading",
        metadata: None,
        props,
        raw: None,
    })
}

/// The friendly form of an `error` entry BIcraft writes, else `None` (kept raw).
pub(super) fn decode_error(entry: &Value, ctx: &Ctx<'_>) -> Option<Value> {
    let e = entry.as_object()?;
    if e.keys().any(|k| k != "properties" && k != "selector") {
        return None;
    }
    let props = e.get("properties")?.as_object()?;
    if props.keys().any(|k| !ERROR_PROPS.contains(&k.as_str())) {
        return None;
    }
    let selector = e.get("selector")?.as_object().filter(|s| s.len() == 1)?;
    let meta = selector.get("metadata")?.as_str()?;
    let mut out = Map::new();
    out.insert("kind".into(), json!("error_bars"));
    out.insert("measure".into(), json!(measure_of(ctx, meta)?));
    let (upper, lower, relative) = decode_range(props.get("errorRange")?)?;
    if upper.is_none() && lower.is_none() {
        return None;
    }
    if let Some(u) = &upper {
        out.insert("upper".into(), json!(field_text(u)));
    }
    if let Some(l) = &lower {
        out.insert("lower".into(), json!(field_text(l)));
    }
    if relative {
        out.insert("relative".into(), json!(true));
    }
    match props.get("enabled").and_then(lit_bool) {
        Some(true) => {}
        Some(false) => {
            out.insert("show".into(), json!(false));
        }
        None => return None,
    }
    for (key, prop, kind) in ERROR_OPTS {
        if let Some(x) = props.get(*prop) {
            out.insert((*key).into(), opt_decode(*kind, x)?);
        }
    }
    Some(Value::Object(out))
}

/// The friendly form of a `plotAreaShading` entry BIcraft writes, else `None`.
pub(super) fn decode_shading(entry: &Value) -> Option<Value> {
    let e = entry.as_object()?;
    if e.keys().any(|k| k != "properties") {
        return None;
    }
    let props = e.get("properties")?.as_object()?;
    if props.keys().any(|k| !SHADING_PROPS.contains(&k.as_str())) {
        return None;
    }
    let mut out = Map::new();
    out.insert("kind".into(), json!("symmetry_shading"));
    match props.get("show").and_then(lit_bool) {
        Some(true) => {}
        Some(false) => {
            out.insert("show".into(), json!(false));
        }
        None => return None,
    }
    if let Some(x) = props.get("displayName") {
        let t = lit_text(x).filter(|t| t.chars().count() <= MAX_TEXT)?;
        out.insert("label".into(), json!(t));
    }
    for (key, prop, kind) in SHADING_OPTS {
        if let Some(x) = props.get(*prop) {
            out.insert((*key).into(), opt_decode(*kind, x)?);
        }
    }
    Some(Value::Object(out))
}

/// The error bars the preview evaluates: the series' field index, its bounds.
#[derive(Debug, Clone)]
pub(crate) struct ErrorBars {
    pub series: usize,
    pub upper: Option<FieldRef>,
    pub lower: Option<FieldRef>,
    pub relative: bool,
}

/// The visual's error bars that are on and whose series is bound.
pub(crate) fn error_bars(v: &Visual) -> Vec<ErrorBars> {
    let ctx = Ctx::new(v, None);
    let mut out = Vec::new();
    for e in entries(v, Target::Visual, "error").iter().take(MAX_LINES) {
        let Some(p) = e.get("properties") else { continue };
        if p.get("enabled").and_then(lit_bool) != Some(true) {
            continue;
        }
        let Some((upper, lower, relative)) = p.get("errorRange").and_then(decode_range) else {
            continue;
        };
        let Some(meta) = e.pointer("/selector/metadata").and_then(Value::as_str) else {
            continue;
        };
        let Some(f) = ctx.field_of_ref(meta) else { continue };
        let Some(series) = v.fields.iter().position(|b| same_field(&b.field, f)) else {
            continue;
        };
        out.push(ErrorBars {
            series,
            upper,
            lower,
            relative,
        });
    }
    out
}

#[cfg(test)]
#[path = "visual_extras_bars_tests.rs"]
mod tests;
