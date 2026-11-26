//! Anomaly detection (`objects.anomalyDetection`): the `anomaly_detection` line of
//! `visual.analytics` (docs/formats/pbir.md "Chart specifics").
//!
//! The property names and their types are the public report theme schema's (`anomalyDetection`
//! on the line chart: `show`, `displayName`, the marker's and the expected range's
//! `marker*` / `confidenceBand*` properties, `transparency`). Microsoft Learn describes the
//! feature's limits: line charts only, no legend, one value and no secondary values, no forecast
//! or min, max, average, median and percentile lines beside it. No public source gives the
//! property that holds the sensitivity, nor the entry's selector: BIcraft writes the series'
//! queryRef as `selector.metadata` (as for error bars) and the sensitivity as a `transform`
//! `{algorithm: "AnomalyDetection", parameters: [{Name: "Sensitivity", Literal}]}` (the forecast's
//! shape), its own choice, not checked against a file Power BI Desktop saved (docs/gaps.md G56).
//! What the preview computes is BIcraft's estimate (`bicraft_render::anomaly`).

use super::*;

/// The `anomalyDetection` properties modelled (others of an entry are kept).
pub(super) const ANOMALY_PROPS: &[&str] = &[
    "show",
    "displayName",
    "transform",
    "markerShow",
    "markerShape",
    "markerShapeSize",
    "markerColor",
    "markerTransparency",
    "confidenceBandShow",
    "confidenceBandStyle",
    "confidenceBandColor",
    "transparency",
];
const ANOMALY_KEYS: &[&str] = &[
    "kind",
    "measure",
    "series",
    "show",
    "label",
    "sensitivity",
    "marker",
    "marker_shape",
    "marker_size",
    "marker_color",
    "marker_transparency",
    "band",
    "band_style",
    "band_color",
    "transparency",
];
/// The theme schema's `anomalyDetection.markerShape` values.
pub(crate) const ANOMALY_SHAPES: &[&str] = &["circle", "square", "diamond", "triangle", "droplet"];
/// The theme schema's `anomalyDetection.confidenceBandStyle` values.
pub(crate) const ANOMALY_BAND_STYLES: &[&str] = &["fill", "line", "none"];
/// The transform's algorithm name and its one parameter (BIcraft's choice, see above).
const ALGORITHM: &str = "AnomalyDetection";
const SENSITIVITY: &str = "Sensitivity";

#[derive(Clone, Copy)]
enum Kind {
    Bool,
    Color,
    Num(f64, f64),
    Enum(&'static [&'static str]),
}

type Opt = (&'static str, &'static str, Kind);

const OPTS: &[Opt] = &[
    ("marker", "markerShow", Kind::Bool),
    ("marker_shape", "markerShape", Kind::Enum(ANOMALY_SHAPES)),
    ("marker_size", "markerShapeSize", Kind::Num(1.0, 50.0)),
    ("marker_color", "markerColor", Kind::Color),
    ("marker_transparency", "markerTransparency", Kind::Num(0.0, 100.0)),
    ("band", "confidenceBandShow", Kind::Bool),
    (
        "band_style",
        "confidenceBandStyle",
        Kind::Enum(ANOMALY_BAND_STYLES),
    ),
    ("band_color", "confidenceBandColor", Kind::Color),
    ("transparency", "transparency", Kind::Num(0.0, 100.0)),
];

fn transform(sensitivity: f64) -> Value {
    json!({"algorithm": ALGORITHM, "parameters": [
        {"Literal": {"Value": format!("{sensitivity}D")}, "Name": SENSITIVITY}
    ]})
}

/// The sensitivity of a `transform` BIcraft writes.
fn decode_transform(t: &Value) -> Option<f64> {
    let o = t.as_object().filter(|o| o.len() == 2)?;
    if o.get("algorithm")?.as_str()? != ALGORITHM {
        return None;
    }
    let params = o.get("parameters")?.as_array().filter(|p| p.len() == 1)?;
    let p = params.first()?.as_object().filter(|p| p.len() == 2)?;
    if p.get("Name")?.as_str()? != SENSITIVITY {
        return None;
    }
    let text = p
        .get("Literal")?
        .as_object()
        .filter(|l| l.len() == 1)?
        .get("Value")?
        .as_str()?;
    match parse_lit(text)? {
        Lit::Num(n) if (0.0..=100.0).contains(&n) => Some(n),
        _ => None,
    }
}

/// An `anomaly_detection` line (line charts): `{kind, measure?, show?, label?, sensitivity?
/// (0-100), marker?, marker_shape?, marker_size?, marker_color?, marker_transparency?, band?,
/// band_style?, band_color?, transparency?}`.
pub(super) fn parse_anomaly(
    o: &Map<String, Value>,
    what: &str,
    v: &Visual,
    ctx: &Ctx<'_>,
) -> Result<Line, String> {
    check_keys(o, ANOMALY_KEYS, what)?;
    let vt = v.visual_type.as_deref();
    if !has_object(vt, "anomalyDetection", &["lineChart"]) {
        return Err(format!(
            "{what}: a `{}` visual has no anomaly detection (Power BI finds anomalies on line charts)",
            vt.unwrap_or("?")
        ));
    }
    let role = |r: &str| v.fields.iter().filter(|b| b.role.eq_ignore_ascii_case(r)).count();
    if role("Series") > 0 {
        return Err(format!(
            "{what}: Power BI does not find anomalies on a line chart with a legend; remove the Legend field first"
        ));
    }
    if role("Y") > 1 || role("Y2") > 0 {
        return Err(format!(
            "{what}: Power BI finds anomalies on a line chart with one value and no secondary values"
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
    let show = opt("show")
        .map(|x| get_bool(x, &w("show")))
        .transpose()?
        .unwrap_or(true);
    let mut props: Vec<(&'static str, Value)> = vec![("show", bool_v(show))];
    if let Some(l) = opt("label") {
        props.push(("displayName", text_v(get_text(l, &w("label"))?)));
    }
    if let Some(x) = opt("sensitivity") {
        props.push(("transform", transform(get_num(x, &w("sensitivity"), 0.0, 100.0)?)));
    }
    for (key, prop, kind) in OPTS {
        let Some(x) = opt(key) else { continue };
        let wk = w(key);
        let value = match kind {
            Kind::Bool => bool_v(get_bool(x, &wk)?),
            Kind::Color => color_v(&get_color(x, &wk)?),
            Kind::Num(lo, hi) => num_v(
                get_num(x, &wk, *lo, *hi)?,
                integer_prop(vt, "anomalyDetection", prop, false),
            ),
            Kind::Enum(values) => text_v(get_enum(x, &wk, values)?),
        };
        props.push((prop, value));
    }
    for (name, x) in &props {
        if *name != "transform" {
            crate::visual_catalog::check_object_entry(vt, "anomalyDetection", name, x)
                .map_err(|e| format!("{what}: {e}"))?;
        }
    }
    Ok(Line {
        object: "anomalyDetection",
        metadata: Some(query_ref),
        props,
        raw: None,
    })
}

/// The friendly form of an `anomalyDetection` entry BIcraft writes, else `None` (kept raw).
pub(super) fn decode_anomaly(entry: &Value, ctx: &Ctx<'_>) -> Option<Value> {
    let e = entry.as_object()?;
    if e.keys().any(|k| k != "properties" && k != "selector") {
        return None;
    }
    let props = e.get("properties")?.as_object()?;
    if props.keys().any(|k| !ANOMALY_PROPS.contains(&k.as_str())) {
        return None;
    }
    let selector = e.get("selector")?.as_object().filter(|s| s.len() == 1)?;
    let meta = selector.get("metadata")?.as_str()?;
    let mut out = Map::new();
    out.insert("kind".into(), json!("anomaly_detection"));
    out.insert("measure".into(), json!(measure_of(ctx, meta)?));
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
    if let Some(t) = props.get("transform") {
        out.insert("sensitivity".into(), json!(decode_transform(t)?));
    }
    for (key, prop, kind) in OPTS {
        let Some(x) = props.get(*prop) else { continue };
        let value = match kind {
            Kind::Bool => lit_bool(x).map(Value::Bool),
            Kind::Color => color_of(x).filter(|c| is_color(c)).map(Value::String),
            Kind::Num(lo, hi) => lit_num(x).filter(|n| (*lo..=*hi).contains(n)).map(|n| json!(n)),
            Kind::Enum(values) => lit_text(x)
                .filter(|s| values.contains(&s.as_str()))
                .map(Value::String),
        }?;
        out.insert((*key).into(), value);
    }
    Some(Value::Object(out))
}

/// The anomaly detection the preview evaluates: the series' field index, the sensitivity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct AnomalySpec {
    pub series: usize,
    pub sensitivity: f64,
}

/// The visual's anomaly detection, when it is on and its series is bound.
pub(crate) fn anomaly_detection(v: &Visual) -> Option<AnomalySpec> {
    let ctx = Ctx::new(v, None);
    for e in entries(v, Target::Visual, "anomalyDetection")
        .iter()
        .take(MAX_LINES)
    {
        let Some(p) = e.get("properties") else { continue };
        if p.get("show").and_then(lit_bool) == Some(false) {
            continue;
        }
        let Some(meta) = e.pointer("/selector/metadata").and_then(Value::as_str) else {
            continue;
        };
        let Some(f) = ctx.field_of_ref(meta) else { continue };
        let Some(series) = v.fields.iter().position(|b| same_field(&b.field, f)) else {
            continue;
        };
        let sensitivity = p
            .get("transform")
            .and_then(decode_transform)
            .unwrap_or(bicraft_render::anomaly::DEFAULT_SENSITIVITY);
        return Some(AnomalySpec { series, sensitivity });
    }
    None
}

#[cfg(test)]
#[path = "visual_extras_anomaly_tests.rs"]
mod tests;
