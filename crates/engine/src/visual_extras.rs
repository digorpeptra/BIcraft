//! Visual-type specifics and report-level visual behaviour (M2.5 e): axes, analytics lines, small
//! multiples, report-page tooltips, tooltip and drillthrough pages, visual interactions, layer
//! and tab order, mobile layout.
//!
//! Commands: `visual.axis`, `visual.analytics`, `visual.small_multiples`, `visual.tooltip_page`,
//! `page.set_type`, `page.interactions`, `visual.order`, `visual.set_hidden`,
//! `visual.rename_group`, `visual.tab_order`, `visual.mobile` and `visual.mobile_format`
//! (`visual_extras_mobile.rs`).
//!
//! The first four are visual properties: they also exist as keys of the visual `props` object
//! (`visual_props.rs`, `report.apply`), handled by [`parse`] / [`apply`] / [`extract`]:
//!
//! - `axes`: `{x?, y?, y2?}`, each `{show?, title?: bool|"text", title_text?, min?, max?,
//!   start_at_zero?: true, scale?: linear|log, gridlines?: bool|{show?, color?, width?, style?},
//!   display_units?: auto|none|thousands|millions|billions|trillions, precision?, font_size?,
//!   color?, invert?, type?: categorical|continuous (x: `axisType`)}` or `null` (removes that
//!   axis' formatting). `x` is `categoryAxis`, `y` `valueAxis`, `y2` the secondary value axis
//!   (`valueAxis.sec*` on combo charts, `y2Axis` on line and area charts). In an axis object, a `null` value removes that one property.
//! - `analytics`: the complete list of analytics lines, or `null` (removes them all). Each line is
//!   `{kind: constant|average|min|max|median|percentile|trend|forecast|ratio_line|error_bars|
//!   anomaly_detection|symmetry_shading, ...}` or `{object, raw}` (an entry kept verbatim). Constant lines are
//!   `y1AxisReferenceLine` (or `xAxisReferenceLine` with `axis: "x"`) entries, the statistical
//!   lines `referenceLine` entries whose `value` is an aggregation of the series
//!   (`selector.metadata` = its queryRef), trend and forecast the `trend` / `forecast` objects;
//!   entries carry `selector.id` "1", "2"â€¦ as Desktop numbers them. The scatter chart's ratio
//!   line is `ratioLine`, its symmetry shading `plotAreaShading` (one entry each, no selector);
//!   error bars are `error` entries scoped to their series (`visual_extras_bars.rs`), anomaly
//!   detection one `anomalyDetection` entry scoped to the line's series
//!   (`visual_extras_anomaly.rs`).
//! - `small_multiples`: `{field?: "Table[Column]"|null, rows?, columns?, title?: bool|{show?,
//!   font_size?, color?, alignment?, position?: top|bottom}}` or `null`. The field is bound to the
//!   `Rows` role (Desktop's small-multiples role of cartesian charts); rows/columns go to
//!   `smallMultiplesLayout` (`layoutType` `custom`), the title to `subheader`.
//! - `tooltip_page`: a page name (report-page tooltip: `visualTooltip` `type` `Canvas`, `section`
//!   the page name), `"default"` / `null` (the default tooltip) or `"none"` (no tooltip).
//!
//! Mobile layout is a separate document (`visuals/<v>/mobile.json`, kept in `Report::resources`),
//! so it is not a visual prop: `visual.mobile` and `visual.mobile_format` edit it
//! (`visual_extras_mobile.rs`, which also covers the legacy layout's form).
//!
//! Everything is validated before anything changes, and re-running a command with the same
//! parameters changes nothing (values compare by meaning: `12L` = `12D`).

use bicraft_model::SemanticModel;
use bicraft_report::{FieldBinding, FieldRef, Page, Report, Resource, Visual};
use bicraft_visuals::PropType;
use serde_json::{Map, Value, json};

use crate::Session;
use crate::params::{allow, opt_bool, opt_str, str_param};
use crate::refs::{field_text, resolve_field, same_field};
use crate::report::{
    KNOWN_VISUAL_TYPES, check_coordinate, check_size, opt_num, page_at_mut, page_index, parts, visual_index,
};
use crate::report_apply::stable_name;
use crate::visual_series::Ctx;

/// Most analytics lines one visual may be given.
const MAX_LINES: usize = 50;
/// Most interaction rules in one call.
const MAX_RULES: usize = 10_000;
/// Most drillthrough / tooltip fields.
const MAX_PAGE_FIELDS: usize = 20;
/// Most visuals listed in one `visual.order`.
const MAX_ORDER: usize = 10_000;
/// Longest text value (labels, titles), in characters.
const MAX_TEXT: usize = 1000;
/// Longest group display name (the Selection pane's rename), in characters.
const MAX_GROUP_NAME: usize = 200;
/// Largest absolute axis bound or line value.
const MAX_VALUE: f64 = 1.0e15;
/// Default size of a tooltip page (Desktop's "Tooltip" canvas).
const TOOLTIP_SIZE: (f64, f64) = (320.0, 240.0);
/// The role Desktop binds small multiples to on cartesian charts.
pub(crate) const SMALL_MULTIPLES_ROLE: &str = "Rows";
/// Public schema prefix of PBIR documents.
const SCHEMA_PREFIX: &str = "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/";
/// `visualContainerMobileState` version written for new mobile layouts (newest public).
const MOBILE_VERSION: &str = bicraft_versions::VISUAL_MOBILE_STATE.version;

/// Key order of the PBIR `visual` object (Desktop's), for keys this module creates.
const VISUAL_ORDER: &[&str] = &[
    "visualType",
    "autoSelectVisualType",
    "query",
    "expansionStates",
    "objects",
    "visualContainerObjects",
    "syncGroup",
    "drillFilterOtherVisuals",
];
/// Key order of `page.json` (Desktop's).
const PAGE_ORDER: &[&str] = &[
    "$schema",
    "name",
    "displayName",
    "displayOption",
    "height",
    "width",
    "filterConfig",
    "pageBinding",
    "objects",
    "type",
    "visibility",
    "visualInteractions",
    "autoPageGenerationConfig",
    "annotations",
    "howCreated",
];

// Fallbacks used while the visual catalog does not know a built-in type (the catalog wins when
// it has the type): which built-in visuals have the object (report theme schema, Desktop 2.158).
const CARTESIAN: &[&str] = &[
    "barChart",
    "columnChart",
    "clusteredBarChart",
    "clusteredColumnChart",
    "hundredPercentStackedBarChart",
    "hundredPercentStackedColumnChart",
    "lineChart",
    "areaChart",
    "stackedAreaChart",
    "hundredPercentStackedAreaChart",
    "lineStackedColumnComboChart",
    "lineClusteredColumnComboChart",
    "ribbonChart",
];
const AXIS_EXTRA: &[&str] = &["waterfallChart", "scatterChart", "funnel"];
const COMBO: &[&str] = &["lineClusteredColumnComboChart", "lineStackedColumnComboChart"];
const Y2_AXIS: &[&str] = &[
    "lineChart",
    "areaChart",
    "stackedAreaChart",
    "hundredPercentStackedAreaChart",
];
const STAT_LINES: &[&str] = &[
    "clusteredBarChart",
    "clusteredColumnChart",
    "lineChart",
    "areaChart",
    "lineClusteredColumnComboChart",
    "scatterChart",
];
const TREND: &[&str] = &[
    "barChart",
    "columnChart",
    "clusteredBarChart",
    "clusteredColumnChart",
    "hundredPercentStackedBarChart",
    "hundredPercentStackedColumnChart",
    "lineChart",
    "areaChart",
    "stackedAreaChart",
    "hundredPercentStackedAreaChart",
    "lineClusteredColumnComboChart",
    "ribbonChart",
    "scatterChart",
];

// ---------------------------------------------------------------------------------------------
// PBIR values
// ---------------------------------------------------------------------------------------------

fn literal(text: String) -> Value {
    json!({"expr": {"Literal": {"Value": text}}})
}

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn bool_v(b: bool) -> Value {
    literal(b.to_string())
}

fn num_v(n: f64, integer: bool) -> Value {
    literal(format!("{n}{}", if integer { "L" } else { "D" }))
}

fn text_v(s: &str) -> Value {
    literal(quote(s))
}

fn color_v(c: &str) -> Value {
    json!({"solid": {"color": text_v(c)}})
}

/// A decoded PBIR literal.
#[derive(Debug, Clone, PartialEq)]
enum Lit {
    Bool(bool),
    Num(f64),
    Text(String),
}

/// The text of `{"expr": {"Literal": {"Value": s}}}` exactly.
fn literal_text(v: &Value) -> Option<&str> {
    let o = v.as_object().filter(|o| o.len() == 1)?;
    let e = o.get("expr")?.as_object().filter(|e| e.len() == 1)?;
    let l = e.get("Literal")?.as_object().filter(|l| l.len() == 1)?;
    l.get("Value")?.as_str()
}

fn parse_lit(s: &str) -> Option<Lit> {
    match s {
        "true" => return Some(Lit::Bool(true)),
        "false" => return Some(Lit::Bool(false)),
        _ => {}
    }
    if s.len() >= 2 && s.starts_with('\'') && s.ends_with('\'') {
        let inner = s.get(1..s.len().saturating_sub(1))?;
        return Some(Lit::Text(inner.replace("''", "'")));
    }
    let digits = s.strip_suffix(['D', 'L', 'M']).unwrap_or(s);
    digits.parse::<f64>().ok().filter(|n| n.is_finite()).map(Lit::Num)
}

fn lit(v: &Value) -> Option<Lit> {
    literal_text(v).and_then(parse_lit)
}

fn lit_bool(v: &Value) -> Option<bool> {
    match lit(v)? {
        Lit::Bool(b) => Some(b),
        _ => None,
    }
}

fn lit_num(v: &Value) -> Option<f64> {
    match lit(v)? {
        Lit::Num(n) => Some(n),
        _ => None,
    }
}

fn lit_text(v: &Value) -> Option<String> {
    match lit(v)? {
        Lit::Text(t) => Some(t),
        _ => None,
    }
}

/// The color of `{"solid": {"color": <text literal>}}` exactly.
fn color_of(v: &Value) -> Option<String> {
    let o = v.as_object().filter(|o| o.len() == 1)?;
    let solid = o.get("solid")?.as_object().filter(|s| s.len() == 1)?;
    lit_text(solid.get("color")?)
}

/// Whether two PBIR property values mean the same (`12L` = `12D`, colors ignore case).
fn same_value(a: &Value, b: &Value) -> bool {
    if let (Some(x), Some(y)) = (lit(a), lit(b)) {
        return x == y;
    }
    if let (Some(x), Some(y)) = (color_of(a), color_of(b)) {
        return x.eq_ignore_ascii_case(&y);
    }
    a == b
}

fn describe(v: &Value) -> String {
    if let Some(l) = lit(v) {
        return match l {
            Lit::Bool(b) => b.to_string(),
            Lit::Num(n) => n.to_string(),
            Lit::Text(t) => quote(&t),
        };
    }
    color_of(v).unwrap_or_else(|| "(expression)".into())
}

fn is_color(s: &str) -> bool {
    s.strip_prefix('#')
        .is_some_and(|h| matches!(h.len(), 3 | 6 | 8) && h.chars().all(|c| c.is_ascii_hexdigit()))
}

// ---------------------------------------------------------------------------------------------
// Friendly parameter readers
// ---------------------------------------------------------------------------------------------

fn as_object<'a>(v: &'a Value, what: &str) -> Result<&'a Map<String, Value>, String> {
    v.as_object().ok_or_else(|| format!("`{what}` must be an object"))
}

fn check_keys(o: &Map<String, Value>, allowed: &[&str], what: &str) -> Result<(), String> {
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

fn get_bool(v: &Value, what: &str) -> Result<bool, String> {
    v.as_bool().ok_or_else(|| format!("{what} must be true or false"))
}

fn get_num(v: &Value, what: &str, lo: f64, hi: f64) -> Result<f64, String> {
    match v.as_f64() {
        Some(n) if n.is_finite() && (lo..=hi).contains(&n) => Ok(n),
        _ => Err(format!("{what} must be a number from {lo} to {hi}")),
    }
}

fn get_int(v: &Value, what: &str, lo: i64, hi: i64) -> Result<i64, String> {
    match v.as_i64() {
        Some(n) if (lo..=hi).contains(&n) => Ok(n),
        _ => Err(format!("{what} must be a whole number from {lo} to {hi}")),
    }
}

fn get_text<'a>(v: &'a Value, what: &str) -> Result<&'a str, String> {
    let s = v.as_str().ok_or_else(|| format!("{what} must be a string"))?;
    if s.chars().count() > MAX_TEXT {
        return Err(format!("{what} is longer than {MAX_TEXT} characters"));
    }
    Ok(s)
}

fn get_color(v: &Value, what: &str) -> Result<String, String> {
    let s = v
        .as_str()
        .ok_or_else(|| format!("{what} must be a color like \"#118DFF\""))?;
    if !is_color(s) {
        return Err(format!(
            "{what} `{s}` is not a color (#RGB, #RRGGBB or #RRGGBBAA)"
        ));
    }
    Ok(s.to_string())
}

/// One of `allowed` (case-insensitive); returns the canonical spelling.
fn get_enum(v: &Value, what: &str, allowed: &[&'static str]) -> Result<&'static str, String> {
    let s = v.as_str().unwrap_or_default();
    allowed
        .iter()
        .find(|a| a.eq_ignore_ascii_case(s))
        .copied()
        .ok_or_else(|| format!("{what} must be one of {}", allowed.join(", ")))
}

// ---------------------------------------------------------------------------------------------
// Formatting objects of a visual
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Visual,
    Container,
}

impl Target {
    fn key(self) -> &'static str {
        match self {
            Self::Visual => "objects",
            Self::Container => "visualContainerObjects",
        }
    }
}

/// Inserts `key` into `map` (in place when present), else before the first key ranked after it
/// in `order`, else at the end.
fn insert_ordered(map: &mut Map<String, Value>, key: &str, value: Value, order: &[&str]) {
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

/// The entries of `object` in `target`.
fn entries<'a>(v: &'a Visual, target: Target, object: &str) -> &'a [Value] {
    v.visual_extra
        .get(target.key())
        .and_then(|o| o.get(object))
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

/// The properties of the selector-less entry of `object` in `target`.
fn props_of<'a>(v: &'a Visual, target: Target, object: &str) -> Option<&'a Map<String, Value>> {
    entries(v, target, object)
        .iter()
        .find(|e| e.get("selector").is_none())?
        .get("properties")?
        .as_object()
}

/// A property edit: set (`Some`) or remove (`None`).
type PropOp = (String, Option<Value>);

/// Applies property edits to the selector-less entry of `object` (created when a value is set),
/// recording what changed and dropping what the edits leave empty.
fn set_props(
    v: &mut Visual,
    target: Target,
    object: &str,
    ops: &[PropOp],
    changes: &mut Vec<String>,
) -> Result<(), String> {
    let key = target.key();
    let setting = ops.iter().any(|(_, x)| x.is_some());
    let before = changes.len();
    if !v.visual_extra.contains_key(key) {
        if !setting {
            return Ok(());
        }
        insert_ordered(&mut v.visual_extra, key, json!({}), VISUAL_ORDER);
    }
    let prefix = match target {
        Target::Visual => "objects.",
        Target::Container => "",
    };
    let objects = v
        .visual_extra
        .get_mut(key)
        .and_then(Value::as_object_mut)
        .ok_or_else(|| format!("the visual's `{key}` is not an object; it cannot be edited"))?;
    if !objects.contains_key(object) {
        if !setting {
            return Ok(());
        }
        objects.insert(object.to_string(), json!([]));
    }
    let arr = objects
        .get_mut(object)
        .and_then(Value::as_array_mut)
        .ok_or_else(|| format!("the visual's `{key}.{object}` is not a list; it cannot be edited"))?;
    let idx = match arr.iter().position(|e| e.get("selector").is_none()) {
        Some(i) => i,
        None if setting => {
            arr.push(json!({"properties": {}}));
            arr.len().saturating_sub(1)
        }
        None => return Ok(()),
    };
    let entry_empty = {
        let entry = arr
            .get_mut(idx)
            .and_then(Value::as_object_mut)
            .ok_or_else(|| format!("an entry of `{key}.{object}` is not an object"))?;
        if !entry.contains_key("properties") {
            entry.insert("properties".into(), json!({}));
        }
        let props = entry
            .get_mut("properties")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| format!("the properties of `{key}.{object}` are not an object"))?;
        for (name, value) in ops {
            match value {
                Some(x) => {
                    if props.get(name).is_some_and(|old| same_value(old, x)) {
                        continue;
                    }
                    props.insert(name.clone(), x.clone());
                    changes.push(format!("{prefix}{object}.{name} = {}", describe(x)));
                }
                None => {
                    if props.shift_remove(name).is_some() {
                        changes.push(format!("{prefix}{object}.{name} removed"));
                    }
                }
            }
        }
        props.is_empty() && entry.keys().all(|k| k == "properties")
    };
    if changes.len() == before {
        return Ok(());
    }
    if entry_empty {
        arr.remove(idx);
    }
    if arr.is_empty() {
        objects.shift_remove(object);
    }
    if objects.is_empty() {
        v.visual_extra.shift_remove(key);
    }
    Ok(())
}

/// Whether visuals of type `vt` have formatting object `object`: the catalog decides for the
/// types it knows; built-in types it does not know yet use `fallback`; custom visuals may.
fn has_object(vt: Option<&str>, object: &str, fallback: &[&str]) -> bool {
    let Some(t) = vt else { return false };
    if bicraft_visuals::visual(t).is_some() {
        return bicraft_visuals::object(t, object).is_some();
    }
    if KNOWN_VISUAL_TYPES.contains(&t) {
        return fallback.contains(&t);
    }
    true
}

/// Whether the catalog types `object.property` as an integer (`12L`); `default` otherwise.
fn integer_prop(vt: Option<&str>, object: &str, property: &str, default: bool) -> bool {
    match vt.and_then(|t| bicraft_visuals::property(t, object, property)) {
        Some(p) => matches!(p.ty, PropType::Number { integer: true, .. }),
        None => default,
    }
}

/// Checks literal values against the catalog (unknown objects, properties and bad values fail;
/// custom visuals pass).
fn check_ops(vt: Option<&str>, object: &str, ops: &[PropOp]) -> Result<(), String> {
    for (name, value) in ops {
        if let Some(x) = value {
            crate::visual_catalog::check_object_entry(vt, object, name, x)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Axes
// ---------------------------------------------------------------------------------------------

const AXES: &[&str] = &["x", "y", "y2"];
const AXIS_KEYS: &[&str] = &[
    "show",
    "title",
    "title_text",
    "min",
    "max",
    "start_at_zero",
    "scale",
    "gridlines",
    "display_units",
    "precision",
    "font_size",
    "color",
    "invert",
    "type",
    "label_angle",
    "ticks",
];
const DISPLAY_UNITS: &[(&str, f64)] = &[
    ("auto", 0.0),
    ("none", 1.0),
    ("thousands", 1.0e3),
    ("millions", 1.0e6),
    ("billions", 1.0e9),
    ("trillions", 1.0e12),
];
const LINE_STYLES: &[&str] = &["solid", "dashed", "dotted"];

/// Where an axis lives.
struct AxisAt {
    object: &'static str,
    /// Secondary-axis property names (`secStart`â€¦).
    sec: bool,
    /// The property that shows the axis.
    show: &'static str,
}

impl AxisAt {
    fn name(&self, base: &str) -> String {
        if !self.sec {
            return base.to_string();
        }
        let mut c = base.chars();
        match c.next() {
            Some(f) => format!("sec{}{}", f.to_ascii_uppercase(), c.as_str()),
            None => "sec".into(),
        }
    }
}

fn axis_at(vt: Option<&str>, axis: &str) -> Result<AxisAt, String> {
    let t = vt.unwrap_or("?");
    let missing = |what: &str| format!("a `{t}` visual has no {what} axis");
    match axis {
        "x" | "y" => {
            let object = if axis == "x" { "categoryAxis" } else { "valueAxis" };
            let fallback: Vec<&str> = CARTESIAN.iter().chain(AXIS_EXTRA).copied().collect();
            if !has_object(vt, object, &fallback) {
                return Err(missing(axis));
            }
            Ok(AxisAt {
                object,
                sec: false,
                show: "show",
            })
        }
        "y2" => {
            if COMBO.contains(&t) {
                Ok(AxisAt {
                    object: "valueAxis",
                    sec: true,
                    show: "secShow",
                })
            } else if Y2_AXIS.contains(&t)
                || (!KNOWN_VISUAL_TYPES.contains(&t) && has_object(vt, "y2Axis", &[]))
            {
                Ok(AxisAt {
                    object: "y2Axis",
                    sec: true,
                    show: "show",
                })
            } else {
                Err(missing("secondary (y2)"))
            }
        }
        _ => Err(format!("axis must be one of {}", AXES.join(", "))),
    }
}

/// The modelled PBIR properties of an axis (what `null` removes).
fn axis_props(at: &AxisAt) -> Vec<String> {
    let mut out = vec![at.show.to_string()];
    for base in [
        "showAxisTitle",
        "titleText",
        "start",
        "end",
        "logAxisScale",
        "labelDisplayUnits",
        "labelPrecision",
        "fontSize",
        "labelColor",
    ] {
        out.push(at.name(base));
    }
    if !at.sec {
        for p in [
            "gridlineShow",
            "gridlineColor",
            "gridlineThickness",
            "gridlineStyle",
            "invertAxis",
        ] {
            out.push(p.to_string());
        }
    }
    if at.object == "categoryAxis" {
        out.push("axisType".into());
    }
    out
}

fn parse_axis(vt: Option<&str>, axis: &str, spec: &Value) -> Result<(AxisAt, Vec<PropOp>), String> {
    let at = axis_at(vt, axis)?;
    let what = format!("axes.{axis}");
    if spec.is_null() {
        let ops = axis_props(&at).into_iter().map(|p| (p, None)).collect();
        return Ok((at, ops));
    }
    let o = as_object(spec, &what)?;
    check_keys(o, AXIS_KEYS, &what)?;
    let mut ops: Vec<PropOp> = Vec::new();
    let integer = |p: &str, d: bool| integer_prop(vt, at.object, p, d);
    let mut push = |name: String, v: Option<Value>| ops.push((name, v));
    for (k, x) in o {
        let w = format!("{what}.{k}");
        let null = x.is_null();
        match k.as_str() {
            "show" => push(
                at.show.into(),
                (!null).then(|| get_bool(x, &w).map(bool_v)).transpose()?,
            ),
            "title" => {
                if null {
                    push(at.name("showAxisTitle"), None);
                    push(at.name("titleText"), None);
                } else if x.is_string() {
                    let t = get_text(x, &w)?.to_string();
                    push(at.name("showAxisTitle"), Some(bool_v(true)));
                    push(at.name("titleText"), Some(text_v(&t)));
                } else {
                    let b = x
                        .as_bool()
                        .ok_or_else(|| format!("{w} must be true, false or a title text"))?;
                    push(at.name("showAxisTitle"), Some(bool_v(b)));
                }
            }
            "title_text" => push(
                at.name("titleText"),
                (!null).then(|| get_text(x, &w).map(text_v)).transpose()?,
            ),
            "min" | "max" => {
                if o.get("start_at_zero").is_some_and(|z| !z.is_null()) && k == "min" && !null {
                    return Err(format!("{what}: give `min` or `start_at_zero`, not both"));
                }
                let base = if k == "min" { "start" } else { "end" };
                let name = at.name(base);
                let v = (!null)
                    .then(|| get_num(x, &w, -MAX_VALUE, MAX_VALUE))
                    .transpose()?
                    .map(|n| num_v(n, integer(&name, false)));
                push(name, v);
            }
            "start_at_zero" => {
                if !null {
                    if x.as_bool() != Some(true) {
                        return Err(format!(
                            "{w} can only be true (pass `min: null` to let Power BI choose the start)"
                        ));
                    }
                    let name = at.name("start");
                    let int = integer(&name, false);
                    push(name, Some(num_v(0.0, int)));
                }
            }
            "scale" => push(
                at.name("logAxisScale"),
                (!null)
                    .then(|| get_enum(x, &w, &["linear", "log"]).map(|s| bool_v(s == "log")))
                    .transpose()?,
            ),
            "gridlines" => {
                if at.sec {
                    return Err(format!("{w}: the secondary axis has no gridlines of its own"));
                }
                let names = [
                    "gridlineShow",
                    "gridlineColor",
                    "gridlineThickness",
                    "gridlineStyle",
                ];
                if null {
                    for n in names {
                        push(n.into(), None);
                    }
                } else if let Some(b) = x.as_bool() {
                    push("gridlineShow".into(), Some(bool_v(b)));
                } else {
                    let g = as_object(x, &w)?;
                    check_keys(g, &["show", "color", "width", "style"], &w)?;
                    for (gk, gx) in g {
                        let gw = format!("{w}.{gk}");
                        let gnull = gx.is_null();
                        match gk.as_str() {
                            "show" => push(
                                "gridlineShow".into(),
                                (!gnull).then(|| get_bool(gx, &gw).map(bool_v)).transpose()?,
                            ),
                            "color" => push(
                                "gridlineColor".into(),
                                (!gnull)
                                    .then(|| get_color(gx, &gw).map(|c| color_v(&c)))
                                    .transpose()?,
                            ),
                            "width" => push(
                                "gridlineThickness".into(),
                                (!gnull)
                                    .then(|| get_num(gx, &gw, 0.0, 100.0).map(|n| num_v(n, false)))
                                    .transpose()?,
                            ),
                            _ => push(
                                "gridlineStyle".into(),
                                (!gnull)
                                    .then(|| get_enum(gx, &gw, LINE_STYLES).map(text_v))
                                    .transpose()?,
                            ),
                        }
                    }
                }
            }
            "display_units" => {
                let name = at.name("labelDisplayUnits");
                let v = if null {
                    None
                } else {
                    let units = match x {
                        Value::String(s) => DISPLAY_UNITS
                            .iter()
                            .find(|(n, _)| n.eq_ignore_ascii_case(s))
                            .map(|(_, u)| *u),
                        _ => x.as_f64().filter(|n| DISPLAY_UNITS.iter().any(|(_, u)| u == n)),
                    }
                    .ok_or_else(|| {
                        format!(
                            "{w} must be one of {}",
                            DISPLAY_UNITS
                                .iter()
                                .map(|(n, _)| *n)
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    })?;
                    Some(num_v(units, integer(&name, false)))
                };
                push(name, v);
            }
            "precision" => {
                let name = at.name("labelPrecision");
                let v = (!null)
                    .then(|| get_int(x, &w, 0, 15))
                    .transpose()?
                    .map(|n| num_v(n as f64, integer(&name, true)));
                push(name, v);
            }
            "font_size" => {
                let name = at.name("fontSize");
                let v = (!null)
                    .then(|| get_num(x, &w, 8.0, 60.0))
                    .transpose()?
                    .map(|n| num_v(n, integer(&name, false)));
                push(name, v);
            }
            "color" => push(
                at.name("labelColor"),
                (!null)
                    .then(|| get_color(x, &w).map(|c| color_v(&c)))
                    .transpose()?,
            ),
            "invert" => {
                if at.sec {
                    return Err(format!("{w}: the secondary axis cannot be inverted"));
                }
                push(
                    "invertAxis".into(),
                    (!null).then(|| get_bool(x, &w).map(bool_v)).transpose()?,
                );
            }
            "type" => {
                if at.object != "categoryAxis" {
                    return Err(format!("{w}: only the x axis can be categorical or continuous"));
                }
                push(
                    "axisType".into(),
                    (!null)
                        .then(|| {
                            get_enum(x, &w, &["categorical", "continuous"]).map(|t| {
                                text_v(if t == "continuous" {
                                    "Scalar"
                                } else {
                                    "Categorical"
                                })
                            })
                        })
                        .transpose()?,
                );
            }
            // `label_angle`, `ticks`
            _ => {
                return Err(format!(
                    "{w}: Power BI Desktop has no such axis setting (axis labels rotate automatically \
                     and tick marks are not configurable)"
                ));
            }
        }
    }
    check_ops(vt, at.object, &ops)?;
    Ok((at, ops))
}

fn extract_axis(v: &Visual, axis: &str) -> Option<Value> {
    let at = axis_at(v.visual_type.as_deref(), axis).ok()?;
    let props = props_of(v, Target::Visual, at.object)?;
    let get = |base: &str| props.get(&at.name(base));
    let mut out = Map::new();
    if let Some(b) = props.get(at.show).and_then(lit_bool) {
        out.insert("show".into(), json!(b));
    }
    if let Some(b) = get("showAxisTitle").and_then(lit_bool) {
        out.insert("title".into(), json!(b));
    }
    if let Some(t) = get("titleText").and_then(lit_text) {
        out.insert("title_text".into(), json!(t));
    }
    if let Some(n) = get("start").and_then(lit_num) {
        out.insert("min".into(), json!(n));
    }
    if let Some(n) = get("end").and_then(lit_num) {
        out.insert("max".into(), json!(n));
    }
    if let Some(b) = get("logAxisScale").and_then(lit_bool) {
        out.insert("scale".into(), json!(if b { "log" } else { "linear" }));
    }
    if !at.sec {
        let mut g = Map::new();
        if let Some(b) = props.get("gridlineShow").and_then(lit_bool) {
            g.insert("show".into(), json!(b));
        }
        if let Some(c) = props.get("gridlineColor").and_then(color_of) {
            g.insert("color".into(), json!(c));
        }
        if let Some(n) = props.get("gridlineThickness").and_then(lit_num) {
            g.insert("width".into(), json!(n));
        }
        if let Some(s) = props.get("gridlineStyle").and_then(lit_text)
            && LINE_STYLES.contains(&s.as_str())
        {
            g.insert("style".into(), json!(s));
        }
        match (g.len(), g.get("show")) {
            (0, _) => {}
            (1, Some(b)) => {
                out.insert("gridlines".into(), b.clone());
            }
            _ => {
                out.insert("gridlines".into(), Value::Object(g));
            }
        }
        if let Some(b) = props.get("invertAxis").and_then(lit_bool) {
            out.insert("invert".into(), json!(b));
        }
    }
    if at.object == "categoryAxis" {
        match props.get("axisType").and_then(lit_text).as_deref() {
            Some("Scalar") => {
                out.insert("type".into(), json!("continuous"));
            }
            Some("Categorical") => {
                out.insert("type".into(), json!("categorical"));
            }
            _ => {}
        }
    }
    if let Some(n) = get("labelDisplayUnits").and_then(lit_num)
        && let Some((name, _)) = DISPLAY_UNITS.iter().find(|(_, u)| *u == n)
    {
        out.insert("display_units".into(), json!(name));
    }
    if let Some(n) = get("labelPrecision").and_then(lit_num)
        && n.fract() == 0.0
        && (0.0..=15.0).contains(&n)
    {
        out.insert("precision".into(), json!(n as i64));
    }
    if let Some(n) = get("fontSize").and_then(lit_num)
        && (8.0..=60.0).contains(&n)
    {
        out.insert("font_size".into(), json!(n));
    }
    if let Some(c) = get("labelColor").and_then(color_of)
        && is_color(&c)
    {
        out.insert("color".into(), json!(c));
    }
    // Values out of the accepted ranges would not re-apply: leave them out (apply never touches
    // properties it is not given).
    if out
        .get("min")
        .and_then(Value::as_f64)
        .is_some_and(|n| n.abs() > MAX_VALUE)
    {
        out.shift_remove("min");
    }
    if out
        .get("max")
        .and_then(Value::as_f64)
        .is_some_and(|n| n.abs() > MAX_VALUE)
    {
        out.shift_remove("max");
    }
    if out
        .get("title_text")
        .and_then(Value::as_str)
        .is_some_and(|t| t.chars().count() > MAX_TEXT)
    {
        out.shift_remove("title_text");
    }
    (!out.is_empty()).then_some(Value::Object(out))
}

// ---------------------------------------------------------------------------------------------
// Analytics lines
// ---------------------------------------------------------------------------------------------

/// Objects holding analytics lines, in the order lines are listed.
const LINE_OBJECTS: &[&str] = &[
    "y1AxisReferenceLine",
    "xAxisReferenceLine",
    "referenceLine",
    "trend",
    "forecast",
    "error",
    "anomalyDetection",
    "ratioLine",
    "plotAreaShading",
];
/// Objects with one entry and no selector.
const SINGLE_OBJECTS: &[&str] = &["trend", "ratioLine", "plotAreaShading"];
/// The line properties this module models (other properties of an entry are kept).
const LINE_PROPS: &[&str] = &[
    "show",
    "displayName",
    "value",
    "transform",
    "lineColor",
    "style",
    "width",
    "position",
    "dataLabelShow",
    "transparency",
];
/// The properties modelled in `object`'s entries.
fn modelled(object: &str) -> &'static [&'static str] {
    match object {
        "error" => bars::ERROR_PROPS,
        "plotAreaShading" => bars::SHADING_PROPS,
        "anomalyDetection" => anomaly::ANOMALY_PROPS,
        _ => LINE_PROPS,
    }
}
const LINE_KEYS: &[&str] = &[
    "kind",
    "axis",
    "measure",
    "series",
    "value",
    "percentile",
    "label",
    "color",
    "style",
    "width",
    "position",
    "data_label",
    "show",
    "transparency",
    "length",
    "ignore_last",
    "confidence",
    "seasonality",
    "unit",
];
/// Statistical lines: kind name and aggregation function (`QueryAggregateFunction`).
const STAT_KINDS: &[(&str, u64)] = &[("average", 1), ("min", 3), ("max", 4), ("median", 6)];
/// Forecast transform parameters (Desktop's names) and their friendly keys.
const FORECAST_PARAMS: &[(&str, &str, bool)] = &[
    ("Unit", "unit", true),
    ("ForecastLength", "length", true),
    ("IgnoreLast", "ignore_last", true),
    ("ConfidenceLevel", "confidence", false),
    ("MaxSeasonality", "seasonality", true),
];

/// One analytics line, ready to write.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Line {
    object: &'static str,
    /// `selector.metadata` (statistical lines: the series' queryRef).
    metadata: Option<String>,
    /// Modelled properties in Desktop's order.
    props: Vec<(&'static str, Value)>,
    /// An entry kept verbatim.
    raw: Option<Value>,
}

fn line_object(name: &str) -> Option<&'static str> {
    LINE_OBJECTS.iter().find(|o| **o == name).copied()
}

/// The series a statistical line follows: `measure` (a bound field, or a queryRef), else the
/// first measure or aggregation bound to `Y` / `Values`.
fn line_series(v: &Visual, ctx: &Ctx<'_>, text: Option<&str>, what: &str) -> Result<String, String> {
    if let Some(t) = text {
        if ctx.projections.iter().any(|(_, r)| r == t) {
            return Ok(t.to_string());
        }
        return ctx.bound(t).map(|(_, r)| r).map_err(|e| format!("{what}: {e}"));
    }
    let candidate = v.fields.iter().find(|b| {
        (b.role.eq_ignore_ascii_case("Y") || b.role.eq_ignore_ascii_case("Values"))
            && matches!(b.field, FieldRef::Measure { .. } | FieldRef::Aggregation { .. })
    });
    candidate
        .and_then(|b| ctx.query_ref(&b.field))
        .map(str::to_string)
        .ok_or_else(|| format!("{what}: the visual has no measure in Y to follow; pass `measure`"))
}

fn stat_value(query_ref: &str, function: u64) -> Value {
    json!({"expr": {"Aggregation": {
        "Expression": {"SelectRef": {"ExpressionName": query_ref}},
        "Function": function,
    }}})
}

fn percentile_value(query_ref: &str, k: f64) -> Value {
    json!({"expr": {"Percentile": {
        "Expression": {"SelectRef": {"ExpressionName": query_ref}},
        "K": k,
    }}})
}

fn forecast_transform(params: &[(&'static str, f64, bool)]) -> Value {
    let list: Vec<Value> = params
        .iter()
        // Desktop writes every parameter as a double (`7D`, `10D`, `0.95D`).
        .map(|(name, n, _)| json!({"Literal": {"Value": format!("{n}D")}, "Name": name}))
        .collect();
    json!({"algorithm": "Forecast", "parameters": list})
}

fn parse_line(item: &Value, i: usize, v: &Visual, ctx: &Ctx<'_>) -> Result<Line, String> {
    let what = format!("analytics[{i}]");
    let o = as_object(item, &what)?;
    if let Some(raw) = o.get("raw") {
        check_keys(o, &["object", "raw"], &what)?;
        let name = o.get("object").and_then(Value::as_str).unwrap_or_default();
        let object = line_object(name)
            .ok_or_else(|| format!("{what}.object must be one of {}", LINE_OBJECTS.join(", ")))?;
        if !raw.get("properties").is_some_and(Value::is_object) {
            return Err(format!(
                "{what}.raw must be a PBIR entry {{properties, selector?}}"
            ));
        }
        return Ok(Line {
            object,
            metadata: None,
            props: Vec::new(),
            raw: Some(raw.clone()),
        });
    }
    match o
        .get("kind")
        .and_then(Value::as_str)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("error_bars" | "error") => return bars::parse_error(o, &what, v, ctx),
        Some("symmetry_shading") => return bars::parse_shading(o, &what, v),
        Some("anomaly_detection" | "anomalies") => return anomaly::parse_anomaly(o, &what, v, ctx),
        _ => {}
    }
    check_keys(o, LINE_KEYS, &what)?;
    let kind = o
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            format!(
                "{what}.kind is missing (constant, average, min, max, median, percentile, trend, forecast, error_bars, anomaly_detection, ratio_line or symmetry_shading)"
            )
        })?
        .to_ascii_lowercase();
    let opt = |k: &str| o.get(k).filter(|x| !x.is_null());
    let w = |k: &str| format!("{what}.{k}");
    let deny = |keys: &[&str]| -> Result<(), String> {
        for k in keys {
            if opt(k).is_some() {
                return Err(format!("{what}: `{k}` does not apply to a {kind} line"));
            }
        }
        Ok(())
    };
    let series = match (opt("measure"), opt("series")) {
        (Some(_), Some(_)) => return Err(format!("{what}: give `measure` or `series`, not both")),
        (Some(m), None) | (None, Some(m)) => Some(get_text(m, &w("measure"))?),
        (None, None) => None,
    };
    let vt = v.visual_type.as_deref();
    let (object, metadata, value, transform) = match kind.as_str() {
        "constant" => {
            deny(&[
                "measure",
                "series",
                "percentile",
                "length",
                "ignore_last",
                "confidence",
                "seasonality",
                "unit",
            ])?;
            let axis = opt("axis")
                .map(|a| get_enum(a, &w("axis"), &["x", "y"]))
                .transpose()?;
            let object = if axis == Some("x") {
                "xAxisReferenceLine"
            } else {
                "y1AxisReferenceLine"
            };
            let n = opt("value")
                .ok_or_else(|| format!("{what}: a constant line needs a `value`"))
                .and_then(|x| get_num(x, &w("value"), -MAX_VALUE, MAX_VALUE))?;
            let fallback: Vec<&str> = CARTESIAN
                .iter()
                .chain(&["waterfallChart", "scatterChart"])
                .copied()
                .collect();
            if !has_object(vt, object, &fallback) {
                return Err(format!(
                    "{what}: a `{}` visual has no constant lines",
                    vt.unwrap_or("?")
                ));
            }
            (
                object,
                None,
                Some(num_v(n, integer_prop(vt, object, "value", false))),
                None,
            )
        }
        "average" | "min" | "max" | "median" | "percentile" => {
            deny(&[
                "axis",
                "value",
                "length",
                "ignore_last",
                "confidence",
                "seasonality",
                "unit",
            ])?;
            if !has_object(vt, "referenceLine", STAT_LINES) {
                return Err(format!(
                    "{what}: a `{}` visual has no {kind} lines (Power BI offers them on clustered column and bar, line, area, combo and scatter charts)",
                    vt.unwrap_or("?")
                ));
            }
            let query_ref = line_series(v, ctx, series, &what)?;
            let value = if kind == "percentile" {
                let p = opt("percentile")
                    .ok_or_else(|| format!("{what}: a percentile line needs `percentile` (0-100)"))
                    .and_then(|x| get_num(x, &w("percentile"), 0.0, 100.0))?;
                percentile_value(&query_ref, p / 100.0)
            } else {
                deny(&["percentile"])?;
                let f = STAT_KINDS.iter().find(|(n, _)| *n == kind).map_or(1, |(_, f)| *f);
                stat_value(&query_ref, f)
            };
            ("referenceLine", Some(query_ref), Some(value), None)
        }
        "trend" => {
            deny(&[
                "axis",
                "value",
                "measure",
                "series",
                "percentile",
                "position",
                "data_label",
                "length",
                "ignore_last",
                "confidence",
                "seasonality",
                "unit",
            ])?;
            if !has_object(vt, "trend", TREND) {
                return Err(format!(
                    "{what}: a `{}` visual has no trend line",
                    vt.unwrap_or("?")
                ));
            }
            ("trend", None, None, None)
        }
        "ratio_line" => {
            deny(&[
                "axis",
                "value",
                "measure",
                "series",
                "percentile",
                "position",
                "data_label",
                "length",
                "ignore_last",
                "confidence",
                "seasonality",
                "unit",
            ])?;
            if !has_object(vt, "ratioLine", &["scatterChart"]) {
                return Err(format!(
                    "{what}: a `{}` visual has no ratio line (Power BI offers it on scatter charts)",
                    vt.unwrap_or("?")
                ));
            }
            ("ratioLine", None, None, None)
        }
        "forecast" => {
            deny(&[
                "axis",
                "value",
                "percentile",
                "position",
                "data_label",
                "transparency",
            ])?;
            if !has_object(vt, "forecast", &["lineChart"]) {
                return Err(format!(
                    "{what}: a `{}` visual has no forecast (Power BI forecasts line charts)",
                    vt.unwrap_or("?")
                ));
            }
            let mut params: Vec<(&'static str, f64, bool)> = Vec::new();
            for (name, key, int) in FORECAST_PARAMS {
                let n = match (*key, opt(key)) {
                    ("unit", None) => Some(7.0),
                    ("length", None) => Some(10.0),
                    ("ignore_last", None) => Some(0.0),
                    ("confidence", None) => Some(0.95),
                    ("seasonality", None) => None,
                    ("confidence", Some(x)) => Some(get_num(x, &w(key), 0.01, 0.999)?),
                    (_, Some(x)) => Some(get_int(x, &w(key), 0, 100_000)? as f64),
                    _ => None,
                };
                if let Some(n) = n {
                    params.push((name, n, *int));
                }
            }
            // Desktop scopes the forecast to the series it extends.
            let query_ref = line_series(v, ctx, series, &what)?;
            (
                "forecast",
                Some(query_ref),
                None,
                Some(forecast_transform(&params)),
            )
        }
        other => {
            return Err(format!(
                "{what}.kind `{other}` is not one of constant, average, min, max, median, percentile, trend, forecast, error_bars, anomaly_detection, ratio_line, symmetry_shading"
            ));
        }
    };
    let mut props: Vec<(&'static str, Value)> = Vec::new();
    let show = opt("show")
        .map(|x| get_bool(x, &w("show")))
        .transpose()?
        .unwrap_or(true);
    props.push(("show", bool_v(show)));
    if let Some(l) = opt("label") {
        props.push(("displayName", text_v(get_text(l, &w("label"))?)));
    }
    if let Some(x) = value {
        props.push(("value", x));
    }
    if let Some(t) = transform {
        props.push(("transform", t));
    }
    if let Some(c) = opt("color") {
        props.push(("lineColor", color_v(&get_color(c, &w("color"))?)));
    }
    if let Some(s) = opt("style") {
        props.push(("style", text_v(get_enum(s, &w("style"), LINE_STYLES)?)));
    }
    if let Some(x) = opt("width") {
        let n = get_num(x, &w("width"), 0.0, 100.0)?;
        props.push(("width", num_v(n, integer_prop(vt, object, "width", false))));
    }
    if let Some(x) = opt("transparency") {
        let n = get_num(x, &w("transparency"), 0.0, 100.0)?;
        props.push((
            "transparency",
            num_v(n, integer_prop(vt, object, "transparency", false)),
        ));
    }
    if let Some(p) = opt("position") {
        props.push((
            "position",
            text_v(get_enum(p, &w("position"), &["front", "back"])?),
        ));
    }
    if let Some(b) = opt("data_label") {
        props.push(("dataLabelShow", bool_v(get_bool(b, &w("data_label"))?)));
    }
    for (name, x) in &props {
        let expression = *name == "transform" || (*name == "value" && object == "referenceLine");
        if !expression {
            crate::visual_catalog::check_object_entry(vt, object, name, x)
                .map_err(|e| format!("{what}: {e}"))?;
        }
    }
    Ok(Line {
        object,
        metadata,
        props,
        raw: None,
    })
}

fn parse_lines(v: &Value, visual: &Visual, ctx: &Ctx<'_>) -> Result<Vec<Line>, String> {
    let list = match v {
        Value::Null => return Ok(Vec::new()),
        Value::Array(a) => a,
        _ => return Err("`analytics` must be a list of lines, or null to remove them all".into()),
    };
    if list.len() > MAX_LINES {
        return Err(format!("at most {MAX_LINES} analytics lines"));
    }
    let lines = list
        .iter()
        .enumerate()
        .map(|(i, item)| parse_line(item, i, visual, ctx))
        .collect::<Result<Vec<_>, _>>()?;
    for (single, name) in [
        ("trend", "trend line"),
        ("forecast", "forecast line"),
        ("ratioLine", "ratio line"),
        ("plotAreaShading", "symmetry shading"),
        ("anomalyDetection", "anomaly detection"),
    ] {
        if lines
            .iter()
            .filter(|l| l.object == single && l.raw.is_none())
            .count()
            > 1
        {
            return Err(format!("a visual has at most one {name}"));
        }
    }
    // Microsoft Learn: forecast, min, max, average, median and percentile lines do not work with
    // anomaly detection
    if lines
        .iter()
        .any(|l| l.object == "anomalyDetection" && l.raw.is_none())
        && lines
            .iter()
            .any(|l| matches!(l.object, "forecast" | "referenceLine") && l.raw.is_none())
    {
        return Err(
            "anomaly detection does not work with a forecast or min, max, average, median and percentile lines (Power BI disables them together); remove those first"
                .into(),
        );
    }
    let mut series: Vec<&str> = Vec::new();
    for l in lines.iter().filter(|l| l.object == "error" && l.raw.is_none()) {
        let m = l.metadata.as_deref().unwrap_or_default();
        if series.contains(&m) {
            return Err(format!("a series has at most one set of error bars ({m})"));
        }
        series.push(m);
    }
    Ok(lines)
}

fn selector_id(entry: &Value) -> Option<u64> {
    entry.get("selector")?.get("id")?.as_str()?.parse().ok()
}

/// The entry for line `d`, reusing `base` (the existing entry at the same place) when there is
/// one: its selector id and unmodelled properties are kept; an equivalent base is returned as is.
fn line_entry(d: &Line, base: Option<&Value>, next_id: &mut u64) -> Value {
    if let Some(raw) = &d.raw {
        return raw.clone();
    }
    let uses_id = !SINGLE_OBJECTS.contains(&d.object) && !matches!(d.object, "error" | "anomalyDetection");
    let props_of = modelled(d.object);
    let base = base.and_then(Value::as_object);
    let empty = Map::new();
    let base_props = base
        .and_then(|b| b.get("properties"))
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    // Selector: keep the base's; set the id when missing, the metadata of statistical lines.
    let mut selector = base
        .and_then(|b| b.get("selector"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if uses_id && !selector.contains_key("id") {
        selector.insert("id".into(), json!(next_id.to_string()));
        *next_id = next_id.saturating_add(1);
    }
    match &d.metadata {
        // Desktop writes `{metadata, id}`.
        Some(m) if selector.get("metadata").and_then(Value::as_str) != Some(m.as_str()) => {
            selector.shift_remove("metadata");
            selector.shift_insert(0, "metadata".into(), json!(m));
        }
        Some(_) => {}
        None => {
            selector.shift_remove("metadata");
        }
    }
    let modelled: Vec<&String> = base_props
        .keys()
        .filter(|k| props_of.contains(&k.as_str()))
        .collect();
    let same_props = modelled.len() == d.props.len()
        && d.props
            .iter()
            .all(|(k, x)| base_props.get(*k).is_some_and(|old| same_value(old, x)));
    let base_selector = base.and_then(|b| b.get("selector"));
    let same_selector = match base_selector {
        Some(s) => s.as_object() == Some(&selector),
        None => selector.is_empty(),
    };
    if let Some(b) = base
        && same_props
        && same_selector
    {
        return Value::Object(b.clone());
    }
    let mut props = base_props.clone();
    props.retain(|k, _| !props_of.contains(&k.as_str()) || d.props.iter().any(|(n, _)| n == k));
    for (k, x) in &d.props {
        if props.get(*k).is_some_and(|old| same_value(old, x)) {
            continue;
        }
        props.insert((*k).to_string(), x.clone());
    }
    let mut entry = base.cloned().unwrap_or_default();
    entry.insert("properties".into(), Value::Object(props));
    if selector.is_empty() {
        entry.shift_remove("selector");
    } else {
        entry.insert("selector".into(), Value::Object(selector));
    }
    Value::Object(entry)
}

fn apply_lines(v: &mut Visual, lines: &[Line], changes: &mut Vec<String>) -> Result<(), String> {
    for object in LINE_OBJECTS {
        let existing: Vec<Value> = entries(v, Target::Visual, object).to_vec();
        let desired: Vec<&Line> = lines.iter().filter(|l| l.object == *object).collect();
        let mut next_id = existing
            .iter()
            .filter_map(selector_id)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let out: Vec<Value> = desired
            .iter()
            .enumerate()
            .map(|(i, d)| line_entry(d, existing.get(i), &mut next_id))
            .collect();
        if out == existing {
            continue;
        }
        changes.push(format!(
            "objects.{object}: {} line(s) (was {})",
            out.len(),
            existing.len()
        ));
        if out.is_empty() {
            if let Some(objects) = v.visual_extra.get_mut("objects").and_then(Value::as_object_mut) {
                objects.shift_remove(*object);
                if objects.is_empty() {
                    v.visual_extra.shift_remove("objects");
                }
            }
            continue;
        }
        if !v.visual_extra.contains_key("objects") {
            insert_ordered(&mut v.visual_extra, "objects", json!({}), VISUAL_ORDER);
        }
        let objects = v
            .visual_extra
            .get_mut("objects")
            .and_then(Value::as_object_mut)
            .ok_or("the visual's `objects` is not an object; it cannot be edited")?;
        objects.insert((*object).to_string(), Value::Array(out));
    }
    Ok(())
}

/// `{"SelectRef": {"ExpressionName": r}}` exactly.
fn select_ref(v: &Value) -> Option<&str> {
    let o = v.as_object().filter(|o| o.len() == 1)?;
    let s = o.get("SelectRef")?.as_object().filter(|s| s.len() == 1)?;
    s.get("ExpressionName")?.as_str()
}

/// The friendly form of a statistical line's value: (kind, percentile, queryRef).
fn decode_stat(v: &Value) -> Option<(&'static str, Option<f64>, &str)> {
    let expr = v.as_object().filter(|o| o.len() == 1)?.get("expr")?;
    let expr = expr.as_object().filter(|o| o.len() == 1)?;
    if let Some(a) = expr.get("Aggregation").and_then(Value::as_object) {
        if a.len() != 2 {
            return None;
        }
        let r = select_ref(a.get("Expression")?)?;
        let f = a.get("Function")?.as_u64()?;
        let (kind, _) = STAT_KINDS.iter().find(|(_, c)| *c == f)?;
        return Some((kind, None, r));
    }
    let p = expr.get("Percentile")?.as_object()?;
    if p.keys()
        .any(|k| !matches!(k.as_str(), "Expression" | "K" | "Exclusive"))
    {
        return None;
    }
    if p.get("Exclusive").is_some_and(|e| e.as_bool() != Some(false)) {
        return None;
    }
    let r = select_ref(p.get("Expression")?)?;
    let k = p.get("K")?.as_f64()?;
    if !(0.0..=1.0).contains(&k) {
        return None;
    }
    // Percent with a clean decimal representation (0.9 â†’ 90, not 90.00000000000001).
    let pct = ((k * 100.0) * 1e9).round() / 1e9;
    Some(("percentile", Some(pct), r))
}

fn decode_forecast(t: &Value, out: &mut Map<String, Value>) -> Option<()> {
    let o = t.as_object().filter(|o| o.len() == 2)?;
    if o.get("algorithm")?.as_str()? != "Forecast" {
        return None;
    }
    let mut seen = Map::new();
    for p in o.get("parameters")?.as_array()? {
        let po = p.as_object().filter(|po| po.len() == 2)?;
        let name = po.get("Name")?.as_str()?;
        let (_, key, int) = FORECAST_PARAMS.iter().find(|(n, _, _)| *n == name)?;
        let text = po
            .get("Literal")?
            .as_object()
            .filter(|l| l.len() == 1)?
            .get("Value")?
            .as_str()?;
        let n = match parse_lit(text)? {
            Lit::Num(n) => n,
            _ => return None,
        };
        if *int && (n.fract() != 0.0 || !(0.0..=100_000.0).contains(&n)) {
            return None;
        }
        if !*int && !(0.01..=0.999).contains(&n) {
            return None;
        }
        seen.insert((*key).to_string(), if *int { json!(n as i64) } else { json!(n) });
    }
    // Defaults the parser fills in must be explicit to re-apply identically.
    for (_, key, _) in FORECAST_PARAMS {
        if *key != "seasonality" && !seen.contains_key(*key) {
            return None;
        }
    }
    for (_, key, _) in FORECAST_PARAMS {
        if let Some(x) = seen.get(*key) {
            out.insert((*key).to_string(), x.clone());
        }
    }
    Some(())
}

/// The `measure` text of a line's `selector.metadata`: the bound field, else the queryRef itself
/// (when bound); `None` when the series is not bound any more.
fn measure_of(ctx: &Ctx<'_>, meta: &str) -> Option<String> {
    match ctx.field_of_ref(meta) {
        Some(f) => Some(field_text(f)),
        None if ctx.projections.iter().any(|(_, q)| q == meta) => Some(meta.to_string()),
        None => None,
    }
}

/// The friendly form of an analytics entry, or `None` when it must be kept raw.
fn decode_line(object: &str, entry: &Value, ctx: &Ctx<'_>) -> Option<Value> {
    match object {
        "error" => return bars::decode_error(entry, ctx),
        "plotAreaShading" => return bars::decode_shading(entry),
        "anomalyDetection" => return anomaly::decode_anomaly(entry, ctx),
        _ => {}
    }
    let e = entry.as_object()?;
    if e.keys().any(|k| k != "properties" && k != "selector") {
        return None;
    }
    let props = e.get("properties")?.as_object()?;
    if props.keys().any(|k| !LINE_PROPS.contains(&k.as_str())) {
        return None;
    }
    let selector = e.get("selector").and_then(Value::as_object);
    let sel_keys: Vec<&str> = selector
        .map(|s| s.keys().map(String::as_str).collect())
        .unwrap_or_default();
    let mut out = Map::new();
    match object {
        "y1AxisReferenceLine" | "xAxisReferenceLine" => {
            if sel_keys != ["id"] || props.contains_key("transform") {
                return None;
            }
            out.insert("kind".into(), json!("constant"));
            if object == "xAxisReferenceLine" {
                out.insert("axis".into(), json!("x"));
            }
            let n = lit_num(props.get("value")?)?;
            if n.abs() > MAX_VALUE {
                return None;
            }
            out.insert("value".into(), json!(n));
        }
        "referenceLine" => {
            if sel_keys.iter().any(|k| !matches!(*k, "id" | "metadata")) || !sel_keys.contains(&"id") {
                return None;
            }
            let meta = selector?.get("metadata")?.as_str()?;
            let (kind, pct, r) = decode_stat(props.get("value")?)?;
            if r != meta || props.contains_key("transform") {
                return None;
            }
            out.insert("kind".into(), json!(kind));
            out.insert("measure".into(), json!(measure_of(ctx, meta)?));
            if let Some(p) = pct {
                out.insert("percentile".into(), json!(p));
            }
        }
        "trend" | "ratioLine" => {
            if props.contains_key("value") || props.contains_key("transform") {
                return None;
            }
            if props.contains_key("position") || props.contains_key("dataLabelShow") || selector.is_some() {
                return None;
            }
            out.insert(
                "kind".into(),
                json!(if object == "trend" { "trend" } else { "ratio_line" }),
            );
        }
        "forecast" => {
            if sel_keys.len() != 2
                || !sel_keys.contains(&"id")
                || !sel_keys.contains(&"metadata")
                || props.contains_key("value")
                || props.contains_key("position")
                || props.contains_key("dataLabelShow")
                || props.contains_key("transparency")
            {
                return None;
            }
            out.insert("kind".into(), json!("forecast"));
            let meta = selector?.get("metadata")?.as_str()?;
            out.insert("measure".into(), json!(measure_of(ctx, meta)?));
            decode_forecast(props.get("transform")?, &mut out)?;
        }
        _ => return None,
    }
    // Lines are always written with `show`.
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
    if let Some(x) = props.get("lineColor") {
        out.insert("color".into(), json!(color_of(x).filter(|c| is_color(c))?));
    }
    if let Some(x) = props.get("style") {
        let s = lit_text(x).filter(|s| LINE_STYLES.contains(&s.as_str()))?;
        out.insert("style".into(), json!(s));
    }
    if let Some(x) = props.get("width") {
        out.insert(
            "width".into(),
            json!(lit_num(x).filter(|n| (0.0..=100.0).contains(n))?),
        );
    }
    if let Some(x) = props.get("transparency") {
        out.insert(
            "transparency".into(),
            json!(lit_num(x).filter(|n| (0.0..=100.0).contains(n))?),
        );
    }
    if let Some(x) = props.get("position") {
        let s = lit_text(x).filter(|s| s == "front" || s == "back")?;
        out.insert("position".into(), json!(s));
    }
    if let Some(x) = props.get("dataLabelShow") {
        out.insert("data_label".into(), json!(lit_bool(x)?));
    }
    Some(Value::Object(out))
}

fn extract_lines(v: &Visual) -> Option<Value> {
    let ctx = Ctx::new(v, None);
    let mut out = Vec::new();
    for object in LINE_OBJECTS {
        for entry in entries(v, Target::Visual, object) {
            out.push(
                decode_line(object, entry, &ctx).unwrap_or_else(|| json!({"object": object, "raw": entry})),
            );
        }
    }
    (!out.is_empty()).then_some(Value::Array(out))
}

// ---------------------------------------------------------------------------------------------
// Small multiples
// ---------------------------------------------------------------------------------------------

const ALIGNMENTS: &[&str] = &["left", "center", "right"];

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SmallMultiples {
    /// `Some(None)` unbinds, `Some(Some(f))` binds `f`.
    field: Option<Option<FieldRef>>,
    layout: Vec<PropOp>,
    subheader: Vec<PropOp>,
}

pub(crate) fn small_multiples_types(vt: Option<&str>) -> bool {
    has_object(vt, "smallMultiplesLayout", CARTESIAN)
}

fn parse_small_multiples(spec: &Value, v: &Visual, ctx: &Ctx<'_>) -> Result<SmallMultiples, String> {
    let vt = v.visual_type.as_deref();
    let layout_names = ["layoutType", "rowCount", "columnCount"];
    let sub_names = ["show", "fontSize", "fontColor", "alignment", "position"];
    if spec.is_null() {
        return Ok(SmallMultiples {
            field: Some(None),
            layout: layout_names.iter().map(|n| ((*n).to_string(), None)).collect(),
            subheader: sub_names.iter().map(|n| ((*n).to_string(), None)).collect(),
        });
    }
    if !small_multiples_types(vt) {
        return Err(format!(
            "a `{}` visual has no small multiples (Power BI offers them on column, bar, line, area, combo and ribbon charts)",
            vt.unwrap_or("?")
        ));
    }
    let o = as_object(spec, "small_multiples")?;
    check_keys(o, &["field", "rows", "columns", "title"], "small_multiples")?;
    let field = match o.get("field") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(x) => {
            let text = get_text(x, "small_multiples.field")?;
            let f = ctx.field(text)?;
            if !matches!(f, FieldRef::Column { .. }) {
                return Err(format!(
                    "small_multiples.field: {} is not a column; small multiples split by a column's values",
                    field_text(&f)
                ));
            }
            Some(Some(f))
        }
    };
    let mut layout: Vec<PropOp> = Vec::new();
    let mut grid = false;
    for (key, prop) in [("rows", "rowCount"), ("columns", "columnCount")] {
        match o.get(key) {
            None => {}
            Some(Value::Null) => layout.push((prop.into(), None)),
            Some(x) => {
                let n = get_int(x, &format!("small_multiples.{key}"), 1, 6)?;
                let int = integer_prop(vt, "smallMultiplesLayout", prop, true);
                layout.push((prop.into(), Some(num_v(n as f64, int))));
                grid = true;
            }
        }
    }
    if grid {
        layout.insert(0, ("layoutType".into(), Some(text_v("custom"))));
    } else if o.get("rows").is_some_and(Value::is_null) && o.get("columns").is_some_and(Value::is_null) {
        layout.insert(0, ("layoutType".into(), None));
    }
    let mut subheader: Vec<PropOp> = Vec::new();
    match o.get("title") {
        None => {}
        Some(Value::Null) => {
            subheader = sub_names.iter().map(|n| ((*n).to_string(), None)).collect();
        }
        Some(Value::Bool(b)) => subheader.push(("show".into(), Some(bool_v(*b)))),
        Some(x) => {
            let t = as_object(x, "small_multiples.title")?;
            check_keys(
                t,
                &["show", "font_size", "color", "alignment", "position"],
                "small_multiples.title",
            )?;
            for (k, x) in t {
                let w = format!("small_multiples.title.{k}");
                let set = !x.is_null();
                let (name, val) = match k.as_str() {
                    "show" => ("show", set.then(|| get_bool(x, &w).map(bool_v)).transpose()?),
                    "font_size" => (
                        "fontSize",
                        set.then(|| get_num(x, &w, 8.0, 60.0).map(|n| num_v(n, false)))
                            .transpose()?,
                    ),
                    "color" => (
                        "fontColor",
                        set.then(|| get_color(x, &w).map(|c| color_v(&c))).transpose()?,
                    ),
                    "alignment" => (
                        "alignment",
                        set.then(|| get_enum(x, &w, ALIGNMENTS).map(text_v)).transpose()?,
                    ),
                    _ => (
                        "position",
                        set.then(|| get_enum(x, &w, &["top", "bottom"]).map(text_v))
                            .transpose()?,
                    ),
                };
                subheader.push((name.into(), val));
            }
        }
    }
    check_ops(vt, "smallMultiplesLayout", &layout)?;
    check_ops(vt, "subheader", &subheader)?;
    Ok(SmallMultiples {
        field,
        layout,
        subheader,
    })
}

fn is_sm_role(role: &str) -> bool {
    role.eq_ignore_ascii_case(SMALL_MULTIPLES_ROLE)
}

fn apply_small_multiples(
    v: &mut Visual,
    sm: &SmallMultiples,
    changes: &mut Vec<String>,
) -> Result<(), String> {
    match &sm.field {
        None => {}
        Some(None) => {
            let before = v.fields.len();
            v.fields.retain(|b| !is_sm_role(&b.role));
            if v.fields.len() != before {
                changes.push("small multiples field removed".into());
            }
        }
        Some(Some(f)) => {
            let current: Vec<&FieldBinding> = v.fields.iter().filter(|b| is_sm_role(&b.role)).collect();
            let same = matches!(current.as_slice(), [b] if same_field(&b.field, f));
            if !same {
                let role = current
                    .first()
                    .map_or_else(|| SMALL_MULTIPLES_ROLE.to_string(), |b| b.role.clone());
                v.fields.retain(|b| !is_sm_role(&b.role));
                // Desktop orders the roles Category, Series, Rows, Yâ€¦
                let at = v
                    .fields
                    .iter()
                    .position(|b| {
                        ["Y", "Y2", "Values", "Tooltips", "Size", "X"]
                            .iter()
                            .any(|r| b.role.eq_ignore_ascii_case(r))
                    })
                    .unwrap_or(v.fields.len());
                v.fields.insert(
                    at,
                    FieldBinding {
                        display_name: None,
                        show_as: None,
                        role,
                        field: f.clone(),
                    },
                );
                changes.push(format!("small multiples by {}", field_text(f)));
            }
        }
    }
    set_props(v, Target::Visual, "smallMultiplesLayout", &sm.layout, changes)?;
    set_props(v, Target::Visual, "subheader", &sm.subheader, changes)
}

fn extract_small_multiples(v: &Visual) -> Option<Value> {
    // `Rows` is also the row role of matrices and tables: only charts that offer small
    // multiples have any to extract.
    if !small_multiples_types(v.visual_type.as_deref()) {
        return None;
    }
    let mut out = Map::new();
    let bound: Vec<&FieldBinding> = v.fields.iter().filter(|b| is_sm_role(&b.role)).collect();
    if let [b] = bound.as_slice()
        && matches!(b.field, FieldRef::Column { .. })
    {
        out.insert("field".into(), json!(field_text(&b.field)));
    }
    if let Some(p) = props_of(v, Target::Visual, "smallMultiplesLayout") {
        let custom = p.get("layoutType").and_then(lit_text).as_deref() == Some("custom");
        for (key, prop) in [("rows", "rowCount"), ("columns", "columnCount")] {
            if let Some(n) = p.get(prop).and_then(lit_num)
                && custom
                && n.fract() == 0.0
                && (1.0..=6.0).contains(&n)
            {
                out.insert(key.into(), json!(n as i64));
            }
        }
    }
    if let Some(p) = props_of(v, Target::Visual, "subheader") {
        let mut t = Map::new();
        if let Some(b) = p.get("show").and_then(lit_bool) {
            t.insert("show".into(), json!(b));
        }
        if let Some(n) = p
            .get("fontSize")
            .and_then(lit_num)
            .filter(|n| (8.0..=60.0).contains(n))
        {
            t.insert("font_size".into(), json!(n));
        }
        if let Some(c) = p.get("fontColor").and_then(color_of).filter(|c| is_color(c)) {
            t.insert("color".into(), json!(c));
        }
        if let Some(a) = p
            .get("alignment")
            .and_then(lit_text)
            .filter(|a| ALIGNMENTS.contains(&a.as_str()))
        {
            t.insert("alignment".into(), json!(a));
        }
        if let Some(a) = p
            .get("position")
            .and_then(lit_text)
            .filter(|a| a == "top" || a == "bottom")
        {
            t.insert("position".into(), json!(a));
        }
        match (t.len(), t.get("show")) {
            (0, _) => {}
            (1, Some(b)) => {
                out.insert("title".into(), b.clone());
            }
            _ => {
                out.insert("title".into(), Value::Object(t));
            }
        }
    }
    (!out.is_empty()).then_some(Value::Object(out))
}

// ---------------------------------------------------------------------------------------------
// Report-page tooltip of a visual
// ---------------------------------------------------------------------------------------------

fn tooltip_ops(spec: &Value) -> Result<Vec<PropOp>, String> {
    let s = match spec {
        Value::Null => "default",
        Value::String(s) => s.as_str(),
        _ => return Err("`tooltip_page` must be a page name, \"default\" or \"none\"".into()),
    };
    let ops = match s {
        "default" => vec![
            ("show".into(), None),
            ("type".into(), None),
            ("section".into(), None),
        ],
        "none" => vec![
            ("show".into(), Some(bool_v(false))),
            ("type".into(), None),
            ("section".into(), None),
        ],
        page => {
            if page.is_empty() || page.chars().count() > MAX_TEXT {
                return Err("`tooltip_page` must be a page name".into());
            }
            vec![
                ("show".into(), Some(bool_v(true))),
                ("type".into(), Some(text_v("Canvas"))),
                ("section".into(), Some(text_v(page))),
            ]
        }
    };
    Ok(ops)
}

fn extract_tooltip(v: &Visual) -> Option<Value> {
    let p = props_of(v, Target::Container, "visualTooltip")?;
    let show = p.get("show").and_then(lit_bool);
    let ty = p.get("type").and_then(lit_text);
    let section = p.get("section").and_then(lit_text);
    match (show, ty.as_deref(), section) {
        (Some(true), Some("Canvas"), Some(s)) if !s.is_empty() && s != "default" && s != "none" => {
            Some(json!(s))
        }
        (Some(false), None, None) => Some(json!("none")),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// The props keys (visual_props.rs)
// ---------------------------------------------------------------------------------------------

/// The validated edits of the `axes`, `analytics`, `small_multiples` and `tooltip_page` props.
#[derive(Debug, Clone, Default)]
pub(crate) struct Extras {
    axes: Vec<(&'static str, Vec<PropOp>)>,
    lines: Option<Vec<Line>>,
    small_multiples: Option<SmallMultiples>,
    tooltip: Option<Vec<PropOp>>,
}

/// Validates the props keys of this module (everything else in `o` is ignored).
pub(crate) fn parse(
    o: &Map<String, Value>,
    v: &Visual,
    model: Option<&SemanticModel>,
) -> Result<Extras, String> {
    let mut out = Extras::default();
    let vt = v.visual_type.as_deref();
    if let Some(a) = o.get("axes") {
        let axes = as_object(a, "axes")?;
        check_keys(axes, AXES, "axes")?;
        for (k, spec) in axes {
            let (at, ops) = parse_axis(vt, k, spec)?;
            out.axes.push((at.object, ops));
        }
    }
    let needs_ctx = o.contains_key("analytics") || o.contains_key("small_multiples");
    if needs_ctx {
        let ctx = Ctx::new(v, model);
        if let Some(l) = o.get("analytics") {
            out.lines = Some(parse_lines(l, v, &ctx).map_err(|e| format!("analytics: {e}"))?);
        }
        if let Some(sm) = o.get("small_multiples") {
            out.small_multiples = Some(parse_small_multiples(sm, v, &ctx)?);
        }
    }
    if let Some(t) = o.get("tooltip_page") {
        let ops = tooltip_ops(t)?;
        check_ops(vt, "visualTooltip", &ops)?;
        out.tooltip = Some(ops);
    }
    Ok(out)
}

/// Applies validated edits to `v` (a working copy), recording each change.
pub(crate) fn apply(v: &mut Visual, e: &Extras, changes: &mut Vec<String>) -> Result<(), String> {
    for (object, ops) in &e.axes {
        set_props(v, Target::Visual, object, ops, changes)?;
    }
    if let Some(sm) = &e.small_multiples {
        apply_small_multiples(v, sm, changes)?;
    }
    if let Some(lines) = &e.lines {
        apply_lines(v, lines, changes)?;
    }
    if let Some(ops) = &e.tooltip {
        set_props(v, Target::Container, "visualTooltip", ops, changes)?;
    }
    Ok(())
}

/// Adds this module's props keys of `v` to `out` (those with a value).
pub(crate) fn extract(v: &Visual, out: &mut Map<String, Value>) {
    let mut axes = Map::new();
    for axis in AXES {
        if let Some(x) = extract_axis(v, axis) {
            axes.insert((*axis).to_string(), x);
        }
    }
    if !axes.is_empty() {
        out.insert("axes".into(), Value::Object(axes));
    }
    if let Some(l) = extract_lines(v) {
        out.insert("analytics".into(), l);
    }
    if let Some(sm) = extract_small_multiples(v) {
        out.insert("small_multiples".into(), sm);
    }
    if let Some(t) = extract_tooltip(v) {
        out.insert("tooltip_page".into(), t);
    }
}

// ---------------------------------------------------------------------------------------------
// Visual commands
// ---------------------------------------------------------------------------------------------

/// The parameters minus `page` / `visual` (and `drop`).
fn rest(p: &Value, drop: &[&str]) -> Map<String, Value> {
    let mut o = p.as_object().cloned().unwrap_or_default();
    for k in ["page", "visual"].iter().chain(drop) {
        o.shift_remove(*k);
    }
    o
}

pub(crate) fn visual_axis(s: &mut Session, p: &Value) -> Result<Value, String> {
    let mut allowed = vec!["page", "visual", "axis", "clear"];
    allowed.extend_from_slice(AXIS_KEYS);
    allow(p, &allowed)?;
    let axis = str_param(p, "axis")?.to_ascii_lowercase();
    let clear = opt_bool(p, "clear")?.unwrap_or(false);
    let body = rest(p, &["axis", "clear"]);
    let spec = if clear {
        if !body.is_empty() {
            return Err("`clear` removes the axis formatting: give no other settings with it".into());
        }
        Value::Null
    } else {
        if body.is_empty() {
            return Err(format!(
                "nothing to set: pass some of {} (or clear: true)",
                AXIS_KEYS.join(", ")
            ));
        }
        Value::Object(body)
    };
    crate::visual_props::edit_visual(s, p, |_, _| Ok(json!({"axes": {axis: spec}})))
}

pub(crate) fn visual_analytics(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "visual", "lines"])?;
    let lines = p
        .get("lines")
        .cloned()
        .ok_or("missing parameter `lines` (a list of lines, or null to remove them all)")?;
    crate::visual_props::edit_visual(s, p, |_, _| Ok(json!({"analytics": lines})))
}

pub(crate) fn visual_small_multiples(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "visual", "field", "rows", "columns", "title"])?;
    let body = rest(p, &[]);
    let spec = match body.get("field") {
        Some(Value::Null) if body.len() == 1 => Value::Null,
        _ if body.is_empty() => {
            return Err(
                "nothing to set: pass field (or null to remove small multiples), rows, columns or title"
                    .into(),
            );
        }
        _ => Value::Object(body),
    };
    crate::visual_props::edit_visual(s, p, |_, _| Ok(json!({"small_multiples": spec})))
}

pub(crate) fn visual_tooltip_page(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "visual", "tooltip_page"])?;
    let target = match p.get("tooltip_page") {
        None => return Err("missing parameter `tooltip_page` (a page, \"default\" or \"none\")".into()),
        Some(Value::Null) => None,
        Some(Value::String(t)) => Some(t.clone()),
        Some(_) => return Err("parameter `tooltip_page` must be a page, \"default\" or \"none\"".into()),
    };
    let mut warnings = Vec::new();
    let value = match target.as_deref() {
        None | Some("default") => json!("default"),
        Some("none") => json!("none"),
        Some(key) => {
            let r = crate::report::report_ref(s)?;
            let i = page_index(r, key)?;
            let page = r.pages.get(i).ok_or("internal: page index out of range")?;
            if page_kind(page) != Some("Tooltip") {
                warnings.push(format!(
                    "page `{}` is not a tooltip page; make it one with page.set_type {{type: \"tooltip\"}}",
                    page.display_name
                ));
            }
            json!(page.name)
        }
    };
    let mut out = crate::visual_props::edit_visual(s, p, |_, _| Ok(json!({"tooltip_page": value})))?;
    if let Some(o) = out.as_object_mut()
        && !warnings.is_empty()
    {
        o.insert("warnings".into(), json!(warnings));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Page type: tooltip and drillthrough pages
// ---------------------------------------------------------------------------------------------

/// `page.type` (`Tooltip` / `Drillthrough`), when set.
fn page_kind(page: &Page) -> Option<&str> {
    page.extra.get("type").and_then(Value::as_str)
}

/// A drillthrough / tooltip field: a column, or an aggregated column (`asAggregation`).
fn resolve_page_field(model: Option<&SemanticModel>, text: &str) -> Result<(FieldRef, bool), String> {
    let f = resolve_field(model, text)?.field;
    match f {
        FieldRef::Column { .. } => Ok((f, false)),
        FieldRef::Aggregation { table, column, .. } => Ok((FieldRef::Column { table, column }, true)),
        _ => Err(format!(
            "{} is not a column: drillthrough and tooltip pages filter by columns (or summarized columns)",
            field_text(&f)
        )),
    }
}

fn filter_list(page: &Page) -> Vec<Value> {
    page.filter_config
        .as_ref()
        .and_then(|f| f.get("filters"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn is_drill_filter(f: &Value) -> bool {
    f.get("howCreated").and_then(Value::as_str) == Some("Drillthrough")
}

fn filter_field(f: &Value) -> FieldRef {
    bicraft_report::query::field_ref(f.get("field"))
}

/// The page's binding fields: its drillthrough filters, as (field, asAggregation).
fn page_fields(page: &Page) -> Vec<Value> {
    let params = page
        .extra
        .get("pageBinding")
        .and_then(|b| b.get("parameters"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    filter_list(page)
        .iter()
        .filter(|f| is_drill_filter(f))
        .map(|f| {
            let name = f.get("name").and_then(Value::as_str).unwrap_or_default();
            let agg = params.iter().any(|p| {
                p.get("boundFilter").and_then(Value::as_str) == Some(name)
                    && p.get("asAggregation").and_then(Value::as_bool) == Some(true)
            });
            let field = filter_field(f);
            match (&field, agg) {
                (FieldRef::Column { table, column }, true) => json!(field_text(&FieldRef::Aggregation {
                    table: table.clone(),
                    column: column.clone(),
                    function: "Sum".into(),
                })),
                _ => json!(field_text(&field)),
            }
        })
        .collect()
}

/// A Desktop-style back button (action button, `visualLink` type Back).
fn is_back_button(v: &Visual) -> bool {
    v.visual_type.as_deref() == Some("actionButton")
        && props_of(v, Target::Container, "visualLink")
            .and_then(|p| p.get("type"))
            .and_then(lit_text)
            .as_deref()
            == Some("Back")
}

fn back_button(page: &Page) -> Visual {
    let top_z = page.visuals.iter().map(|v| v.position.z).fold(0.0_f64, f64::max);
    let top_tab = page.visuals.iter().filter_map(|v| v.position.tab_order).max();
    let mut v = Visual {
        name: stable_name(&format!("{}/back-button", page.name)),
        visual_type: Some("actionButton".into()),
        ..Default::default()
    };
    v.position.x = 0.0;
    v.position.y = 0.0;
    v.position.width = 100.0;
    v.position.height = 40.0;
    v.position.z = if page.visuals.is_empty() {
        0.0
    } else {
        top_z + 1000.0
    };
    v.position.tab_order = Some(top_tab.map_or(0, |t| t.saturating_add(1000)));
    v.visual_extra.insert(
        "objects".into(),
        json!({"icon": [{"properties": {"shapeType": text_v("back")}, "selector": {"id": "default"}}]}),
    );
    v.visual_extra.insert(
        "visualContainerObjects".into(),
        json!({"visualLink": [{"properties": {"show": bool_v(true), "type": text_v("Back")}}]}),
    );
    v.visual_extra
        .insert("drillFilterOtherVisuals".into(), json!(true));
    v.extra.insert("howCreated".into(), json!("InsertVisualButton"));
    v
}

pub(crate) fn page_set_type(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "page",
            "type",
            "fields",
            "keep_all_filters",
            "cross_report",
            "back_button",
            "width",
            "height",
            "hidden",
        ],
    )?;
    let key = str_param(p, "page")?;
    let kind = match str_param(p, "type")?.to_ascii_lowercase().as_str() {
        "report" => None,
        "tooltip" => Some("Tooltip"),
        "drillthrough" => Some("Drillthrough"),
        other => return Err(format!("type `{other}` must be report, tooltip or drillthrough")),
    };
    let fields_text: Option<Vec<String>> = match p.get("fields") {
        None | Some(Value::Null) => None,
        Some(Value::Array(a)) => {
            if a.len() > MAX_PAGE_FIELDS {
                return Err(format!("at most {MAX_PAGE_FIELDS} fields"));
            }
            Some(
                a.iter()
                    .map(|x| {
                        x.as_str()
                            .map(str::to_string)
                            .ok_or("`fields` must be a list of field strings")
                    })
                    .collect::<Result<_, _>>()?,
            )
        }
        Some(_) => return Err("`fields` must be a list of field strings like \"Product[Category]\"".into()),
    };
    let keep_all = opt_bool(p, "keep_all_filters")?;
    let cross_report = opt_bool(p, "cross_report")?;
    let back = opt_bool(p, "back_button")?;
    let width = opt_num(p, "width")?.map(|w| check_size("width", w)).transpose()?;
    let height = opt_num(p, "height")?
        .map(|h| check_size("height", h))
        .transpose()?;
    let hidden = opt_bool(p, "hidden")?;
    if kind.is_none() && (fields_text.is_some() || keep_all.is_some() || cross_report.is_some()) {
        return Err(
            "fields, keep_all_filters and cross_report apply to tooltip and drillthrough pages".into(),
        );
    }
    if kind != Some("Drillthrough") && (cross_report.is_some() || back == Some(true)) {
        return Err("cross_report and back_button apply to drillthrough pages".into());
    }
    let (model, r) = parts(s)?;
    let pi = page_index(r, key)?;
    let page = r.pages.get(pi).ok_or("internal: page index out of range")?;
    let fields = fields_text
        .as_ref()
        .map(|list| {
            list.iter()
                .map(|t| resolve_page_field(model, t))
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?;
    if let Some(f) = &fields {
        for (i, (a, _)) in f.iter().enumerate() {
            if f.iter().skip(i.saturating_add(1)).any(|(b, _)| same_field(a, b)) {
                return Err(format!("{} is listed twice", field_text(a)));
            }
        }
    }

    let mut new = page.clone();
    let mut changes: Vec<String> = Vec::new();
    let old_kind = page_kind(page).map(str::to_string);
    let binding_kind = |pg: &Page| {
        pg.extra
            .get("pageBinding")
            .and_then(|b| b.get("type"))
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    match kind {
        None => {
            if matches!(old_kind.as_deref(), Some("Tooltip" | "Drillthrough")) {
                new.extra.shift_remove("type");
            }
            if matches!(binding_kind(page).as_deref(), Some("Tooltip" | "Drillthrough")) {
                new.extra.shift_remove("pageBinding");
            }
            let filters = filter_list(&new);
            if filters.iter().any(is_drill_filter) {
                let kept: Vec<Value> = filters.into_iter().filter(|f| !is_drill_filter(f)).collect();
                set_filters(&mut new, kept);
            }
        }
        Some(k) => {
            insert_ordered(&mut new.extra, "type", json!(k), PAGE_ORDER);
            let mut binding = new
                .extra
                .get("pageBinding")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            if !binding.get("name").is_some_and(Value::is_string) {
                binding.insert(
                    "name".into(),
                    json!(stable_name(&format!("{}/pageBinding", page.name))),
                );
            }
            binding.insert("type".into(), json!(k));
            if let Some(list) = &fields {
                let filters = filter_list(&new);
                let old_params: Vec<Value> = binding
                    .get("parameters")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let mut kept: Vec<Value> = filters.iter().filter(|f| !is_drill_filter(f)).cloned().collect();
                let mut params = Vec::new();
                for (f, agg) in list {
                    let existing = filters
                        .iter()
                        .find(|x| is_drill_filter(x) && same_field(&filter_field(x), f));
                    let container = match existing {
                        Some(x) => x.clone(),
                        None => json!({
                            "name": stable_name(&format!("{}/drillthrough/{}", page.name, field_text(f))),
                            "field": bicraft_report::query::field_json(f),
                            "type": "Categorical",
                            "howCreated": "Drillthrough",
                        }),
                    };
                    let fname = container
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let mut param = old_params
                        .iter()
                        .find(|x| x.get("boundFilter").and_then(Value::as_str) == Some(fname.as_str()))
                        .and_then(Value::as_object)
                        .cloned()
                        .unwrap_or_else(|| {
                            let mut m = Map::new();
                            m.insert("name".into(), json!(format!("Param_{fname}")));
                            m.insert("boundFilter".into(), json!(fname));
                            m
                        });
                    if *agg {
                        if param.get("asAggregation").and_then(Value::as_bool) != Some(true) {
                            param.shift_remove("fieldExpr");
                            param.insert("asAggregation".into(), json!(true));
                        }
                    } else {
                        param.shift_remove("asAggregation");
                    }
                    if !param.contains_key("fieldExpr") {
                        param.insert("fieldExpr".into(), bicraft_report::query::field_json(f));
                    }
                    params.push(Value::Object(param));
                    kept.push(container);
                }
                set_filters(&mut new, kept);
                binding.insert("parameters".into(), Value::Array(params));
            }
            if !binding.contains_key("parameters") {
                binding.insert("parameters".into(), json!([]));
            }
            match cross_report {
                Some(true) => {
                    binding.insert("referenceScope".into(), json!("CrossReport"));
                }
                Some(false) => {
                    binding.shift_remove("referenceScope");
                }
                None => {}
            }
            match keep_all {
                Some(true) => {
                    binding.shift_remove("acceptsFilterContext");
                }
                Some(false) => {
                    binding.insert("acceptsFilterContext".into(), json!("None"));
                }
                None => {}
            }
            // Desktop's key order.
            let mut ordered = Map::new();
            for k in [
                "name",
                "type",
                "parameters",
                "referenceScope",
                "acceptsFilterContext",
            ] {
                if let Some(x) = binding.shift_remove(k) {
                    ordered.insert(k.into(), x);
                }
            }
            ordered.extend(binding);
            insert_ordered(&mut new.extra, "pageBinding", Value::Object(ordered), PAGE_ORDER);
            if k == "Tooltip" && old_kind.as_deref() != Some("Tooltip") && width.is_none() && height.is_none()
            {
                (new.width, new.height) = TOOLTIP_SIZE;
            }
        }
    }
    if let Some(w) = width {
        new.width = w;
    }
    if let Some(h) = height {
        new.height = h;
    }
    if let Some(h) = hidden {
        new.visibility = h.then(|| "HiddenInViewMode".to_string());
    }
    match back {
        Some(true) if !new.visuals.iter().any(is_back_button) => {
            let b = back_button(&new);
            if new.visuals.iter().any(|v| v.name.eq_ignore_ascii_case(&b.name)) {
                return Err(format!("a visual named `{}` already exists", b.name));
            }
            changes.push(format!("back button `{}` added", b.name));
            new.visuals.push(b);
        }
        Some(false) => {
            let before = new.visuals.len();
            new.visuals.retain(|v| !is_back_button(v));
            if new.visuals.len() != before {
                changes.push("back button removed".into());
            }
        }
        _ => {}
    }
    if page_kind(&new) != page_kind(page) {
        changes.push(format!("type: {}", page_kind(&new).unwrap_or("report")));
    }
    if new.extra.get("pageBinding") != page.extra.get("pageBinding") {
        changes.push("pageBinding updated".into());
    }
    if new.filter_config != page.filter_config {
        changes.push("drillthrough filters updated".into());
    }
    if (new.width, new.height) != (page.width, page.height) {
        changes.push(format!("size: {} x {}", new.width, new.height));
    }
    if new.visibility != page.visibility {
        changes.push(format!("hidden: {}", new.visibility.is_some()));
    }
    let changed = new != *page;
    let out = json!({
        "page": new.name,
        "type": page_kind(&new).map_or("report", |k| if k == "Tooltip" { "tooltip" } else { "drillthrough" }),
        "changed": changed,
        "changes": changes,
        "fields": page_fields(&new),
        "width": new.width,
        "height": new.height,
        "binding": new.extra.get("pageBinding"),
    });
    if changed {
        if let Some(slot) = r.pages.get_mut(pi) {
            *slot = new;
        }
        s.report_dirty = true;
    }
    Ok(out)
}

fn set_filters(page: &mut Page, filters: Vec<Value>) {
    let mut config = page
        .filter_config
        .as_ref()
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if filters.is_empty() {
        config.shift_remove("filters");
    } else {
        config.insert("filters".into(), Value::Array(filters));
    }
    page.filter_config = (!config.is_empty()).then_some(Value::Object(config));
}

// ---------------------------------------------------------------------------------------------
// Visual interactions
// ---------------------------------------------------------------------------------------------

const INTERACTIONS: &[(&str, &str)] = &[
    ("filter", "DataFilter"),
    ("highlight", "HighlightFilter"),
    ("none", "NoFilter"),
];

pub(crate) fn page_interactions(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "rules", "clear"])?;
    let key = str_param(p, "page")?;
    let clear = opt_bool(p, "clear")?.unwrap_or(false);
    let rules = match p.get("rules") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) => a.clone(),
        Some(_) => return Err("`rules` must be a list of {source, target, type}".into()),
    };
    if rules.len() > MAX_RULES {
        return Err(format!("at most {MAX_RULES} rules"));
    }
    let (_, r) = parts(s)?;
    let pi = page_index(r, key)?;
    let page = r.pages.get(pi).ok_or("internal: page index out of range")?;
    // a visual stands for itself; a visual group for every visual inside it (nested groups too)
    let members_of = |k: &str, what: &str| -> Result<(String, Vec<String>), String> {
        let i = visual_index(page, k).map_err(|e| format!("{what}: {e}"))?;
        let v = page.visuals.get(i).ok_or("internal: visual index out of range")?;
        if v.visual_type.is_some() {
            return Ok((v.name.clone(), vec![v.name.clone()]));
        }
        let members: Vec<String> = page
            .visuals
            .iter()
            .filter(|m| m.visual_type.is_some() && inside_group(page, m, &v.name))
            .map(|m| m.name.clone())
            .collect();
        Ok((v.name.clone(), members))
    };
    // (source, target, Some(type) | None for default), one per pair of visuals
    let mut parsed: Vec<(String, String, Option<&'static str>)> = Vec::new();
    let mut listed: Vec<(String, String)> = Vec::new();
    for (i, rule) in rules.iter().enumerate() {
        let what = format!("rules[{i}]");
        let o = as_object(rule, &what)?;
        check_keys(o, &["source", "target", "type"], &what)?;
        let (source, sources) = members_of(
            get_text(o.get("source").unwrap_or(&Value::Null), &format!("{what}.source"))?,
            &what,
        )?;
        let (target, targets) = members_of(
            get_text(o.get("target").unwrap_or(&Value::Null), &format!("{what}.target"))?,
            &what,
        )?;
        if source == target {
            return Err(format!("{what}: a visual does not interact with itself"));
        }
        let ty = o
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_ascii_lowercase();
        let pbir = if ty == "default" {
            None
        } else {
            Some(
                INTERACTIONS
                    .iter()
                    .find(|(f, _)| *f == ty)
                    .map(|(_, p)| *p)
                    .ok_or_else(|| format!("{what}.type must be filter, highlight, none or default"))?,
            )
        };
        if listed.iter().any(|(s2, t2)| *s2 == source && *t2 == target) {
            return Err(format!("{what}: {source} -> {target} is listed twice"));
        }
        let mut pairs = 0usize;
        for a in &sources {
            for b in targets.iter().filter(|b| *b != a && !sources.contains(b)) {
                if parsed.len() >= MAX_RULES {
                    return Err(format!("at most {MAX_RULES} pairs of visuals"));
                }
                // a later rule wins over an earlier one for the same pair
                parsed.retain(|(x, y, _)| !(x == a && y == b));
                parsed.push((a.clone(), b.clone(), pbir));
                pairs += 1;
            }
        }
        if sources.is_empty() || targets.is_empty() {
            let empty = if sources.is_empty() { &source } else { &target };
            return Err(format!("{what}: the group {empty} holds no visual"));
        }
        if pairs == 0 {
            return Err(format!(
                "{what}: {source} and {target} have no pair of visuals between them (one is inside the other)"
            ));
        }
        listed.push((source, target));
    }
    let mut list: Vec<Value> = if clear {
        Vec::new()
    } else {
        page.extra
            .get("visualInteractions")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    for (source, target, ty) in &parsed {
        let at = list.iter().position(|x| {
            x.get("source").and_then(Value::as_str) == Some(source.as_str())
                && x.get("target").and_then(Value::as_str) == Some(target.as_str())
        });
        match (at, ty) {
            (Some(i), Some(t)) => {
                if let Some(slot) = list.get_mut(i) {
                    *slot = json!({"source": source, "target": target, "type": t});
                }
            }
            (None, Some(t)) => list.push(json!({"source": source, "target": target, "type": t})),
            (Some(i), None) => {
                list.remove(i);
            }
            (None, None) => {}
        }
    }
    let mut new = page.clone();
    if list.is_empty() {
        new.extra.shift_remove("visualInteractions");
    } else {
        insert_ordered(
            &mut new.extra,
            "visualInteractions",
            Value::Array(list.clone()),
            PAGE_ORDER,
        );
    }
    let changed = new != *page;
    let friendly: Vec<Value> = list
        .iter()
        .map(|x| {
            let t = x.get("type").and_then(Value::as_str).unwrap_or_default();
            let ty = INTERACTIONS.iter().find(|(_, p)| *p == t).map_or(t, |(f, _)| f);
            json!({"source": x.get("source"), "target": x.get("target"), "type": ty})
        })
        .collect();
    let out = json!({"page": new.name, "changed": changed, "interactions": friendly});
    if changed {
        if let Some(slot) = r.pages.get_mut(pi) {
            *slot = new;
        }
        s.report_dirty = true;
    }
    Ok(out)
}

/// Whether `v` sits inside the group named `group` (its parent, or a parent's parent…).
fn inside_group(page: &Page, v: &Visual, group: &str) -> bool {
    let mut parent = v.parent_group.as_deref();
    // a bound on the depth: a cycle of `parentGroupName` ends here
    for _ in 0..64 {
        let Some(p) = parent else { return false };
        if p.eq_ignore_ascii_case(group) {
            return true;
        }
        parent = page
            .visuals
            .iter()
            .find(|g| g.name.eq_ignore_ascii_case(p))
            .and_then(|g| g.parent_group.as_deref());
    }
    false
}

// ---------------------------------------------------------------------------------------------
// Layer and tab order
// ---------------------------------------------------------------------------------------------

fn same_parent(a: &Visual, b: &Visual) -> bool {
    match (&a.parent_group, &b.parent_group) {
        (None, None) => true,
        (Some(x), Some(y)) => x.eq_ignore_ascii_case(y),
        _ => false,
    }
}

/// `values` sorted ascending, or `None` when some are missing or repeated.
fn distinct_sorted<T: PartialOrd + Copy>(values: &[Option<T>]) -> Option<Vec<T>> {
    let mut v: Vec<T> = values.iter().copied().collect::<Option<Vec<T>>>()?;
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    if v.windows(2).any(|w| matches!(w, [a, b] if a == b)) {
        return None;
    }
    Some(v)
}

pub(crate) fn visual_order(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "visuals", "visual", "to"])?;
    let key = str_param(p, "page")?;
    let (_, r) = parts(s)?;
    let pi = page_index(r, key)?;
    let page = r.pages.get(pi).ok_or("internal: page index out of range")?;
    // Visual indices of the listed visuals, front to back.
    let to_given = !matches!(p.get("to"), None | Some(Value::Null));
    let (anchor, listed_front, to) = match (p.get("visuals"), opt_str(p, "visual")?) {
        // a set moved together (the canvas's multi-selection): `visual` and `visuals`
        (Some(Value::Array(list)), first) if to_given => {
            if list.len() > MAX_ORDER {
                return Err(format!("`visuals` must list at most {MAX_ORDER} visual names"));
            }
            let mut names: Vec<&str> = first.into_iter().collect();
            for x in list {
                names.push(x.as_str().ok_or("`visuals` must be a list of visual names")?);
            }
            let mut idx = Vec::new();
            for name in names {
                let i = visual_index(page, name)?;
                if !idx.contains(&i) {
                    idx.push(i);
                }
            }
            let Some(&first) = idx.first() else {
                return Err("name the visuals: `visual` or `visuals`".into());
            };
            let to = str_param(p, "to")?.to_ascii_lowercase();
            if !matches!(to.as_str(), "front" | "back" | "forward" | "backward") {
                return Err("`to` must be front, back, forward or backward".into());
            }
            (first, idx, Some(to))
        }
        (Some(Value::Array(list)), None) => {
            if p.get("to").is_some() {
                return Err("`to` goes with `visual`, not with `visuals`".into());
            }
            if list.is_empty() || list.len() > MAX_ORDER {
                return Err(format!("`visuals` must list 1 to {MAX_ORDER} visual names"));
            }
            let mut idx = Vec::new();
            for x in list {
                let name = x.as_str().ok_or("`visuals` must be a list of visual names")?;
                let i = visual_index(page, name)?;
                if idx.contains(&i) {
                    return Err(format!("`{name}` is listed twice"));
                }
                idx.push(i);
            }
            (idx.first().copied().unwrap_or(0), idx, None)
        }
        (None | Some(Value::Null), Some(name)) => {
            let i = visual_index(page, name)?;
            let to = str_param(p, "to")
                .map_err(|_| "missing `to`: front, back, forward or backward".to_string())?
                .to_ascii_lowercase();
            if !matches!(to.as_str(), "front" | "back" | "forward" | "backward") {
                return Err("`to` must be front, back, forward or backward".into());
            }
            (i, vec![i], Some(to))
        }
        _ => return Err("pass `visuals` (names, front to back) or `visual` with `to`".into()),
    };
    let anchor_v = page
        .visuals
        .get(anchor)
        .ok_or("internal: visual index out of range")?;
    for &i in &listed_front {
        let v = page.visuals.get(i).ok_or("internal: visual index out of range")?;
        if !same_parent(v, anchor_v) {
            return Err(format!(
                "`{}` and `{}` are not in the same group: order visuals within one group (or the page's top level)",
                anchor_v.name, v.name
            ));
        }
    }
    // Siblings, front to back by the current z (ties: later in the file is in front).
    let mut siblings: Vec<usize> = (0..page.visuals.len())
        .filter(|&i| page.visuals.get(i).is_some_and(|v| same_parent(v, anchor_v)))
        .collect();
    let z = |i: usize| page.visuals.get(i).map_or(0.0, |v| v.position.z);
    siblings.sort_by(|&a, &b| {
        z(b).partial_cmp(&z(a))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.cmp(&a))
    });
    let current = siblings.clone();
    let order: Vec<usize> = match to.as_deref() {
        None => listed_front
            .iter()
            .copied()
            .chain(siblings.iter().copied().filter(|i| !listed_front.contains(i)))
            .collect(),
        Some(dir) => {
            // the listed visuals move together, keeping their order
            let listed = |i: &usize| listed_front.contains(i);
            let mut o = siblings.clone();
            match dir {
                "front" | "back" => {
                    let (sel, rest): (Vec<usize>, Vec<usize>) = o.iter().copied().partition(listed);
                    o = if dir == "front" {
                        sel.into_iter().chain(rest).collect()
                    } else {
                        rest.into_iter().chain(sel).collect()
                    };
                }
                "forward" => {
                    for k in 1..o.len() {
                        let (a, b) = (o.get(k - 1).copied(), o.get(k).copied());
                        if let (Some(a), Some(b)) = (a, b)
                            && listed(&b)
                            && !listed(&a)
                        {
                            o.swap(k - 1, k);
                        }
                    }
                }
                _ => {
                    for k in (1..o.len()).rev() {
                        let (a, b) = (o.get(k - 1).copied(), o.get(k).copied());
                        if let (Some(a), Some(b)) = (a, b)
                            && listed(&a)
                            && !listed(&b)
                        {
                            o.swap(k - 1, k);
                        }
                    }
                }
            }
            o
        }
    };
    // New z / tabOrder: the siblings' own values redistributed (back to front ascending) when they
    // are distinct, else fresh multiples of 1000 above the smallest.
    let n = order.len();
    let zs: Vec<Option<f64>> = siblings.iter().map(|&i| Some(z(i))).collect();
    let tabs: Vec<Option<i64>> = siblings
        .iter()
        .map(|&i| page.visuals.get(i).and_then(|v| v.position.tab_order))
        .collect();
    let base_z = zs.iter().flatten().copied().fold(f64::INFINITY, f64::min);
    let base_z = if base_z.is_finite() { base_z } else { 0.0 };
    let z_values: Vec<f64> = distinct_sorted(&zs).unwrap_or_else(|| {
        (0..n)
            .map(|k| (base_z + 1000.0 * k as f64).min(crate::report::MAX_SIZE))
            .collect()
    });
    let any_tab = tabs.iter().any(Option::is_some);
    let tab_values: Option<Vec<i64>> = if any_tab {
        Some(distinct_sorted(&tabs).unwrap_or_else(|| {
            let base = tabs.iter().flatten().copied().min().unwrap_or(0);
            (0..n)
                .map(|k| base.saturating_add(1000_i64.saturating_mul(k as i64)))
                .collect()
        }))
    } else {
        None
    };
    let mut new = page.clone();
    // Back to front: rank 0 is the back.
    for (rank, &i) in order.iter().rev().enumerate() {
        if let Some(v) = new.visuals.get_mut(i) {
            if let Some(zv) = z_values.get(rank) {
                v.position.z = *zv;
            }
            if let Some(t) = tab_values.as_ref().and_then(|t| t.get(rank)) {
                v.position.tab_order = Some(*t);
            }
        }
    }
    let changed = new != *page;
    let names: Vec<String> = order
        .iter()
        .filter_map(|&i| new.visuals.get(i).map(|v| v.name.clone()))
        .collect();
    let out = json!({
        "page": new.name,
        "changed": changed,
        "front_to_back": names,
        "previous": current.iter().filter_map(|&i| page.visuals.get(i).map(|v| v.name.clone())).collect::<Vec<_>>(),
    });
    if changed {
        if let Some(slot) = r.pages.get_mut(pi) {
            *slot = new;
        }
        s.report_dirty = true;
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Selection pane: show or hide visuals, rename a group, tab order
// ---------------------------------------------------------------------------------------------

/// The visual names of `visual` and `visuals`, without repeats, at least one.
fn listed_visuals(p: &Value) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(v) = opt_str(p, "visual")? {
        out.push(v.to_string());
    }
    match p.get("visuals") {
        None | Some(Value::Null) => {}
        Some(Value::Array(list)) => {
            if list.len() > MAX_ORDER {
                return Err(format!("`visuals` lists more than {MAX_ORDER} visual names"));
            }
            for x in list {
                let n = x.as_str().ok_or("`visuals` must be a list of visual names")?;
                if !out.iter().any(|o| o.eq_ignore_ascii_case(n)) {
                    out.push(n.to_string());
                }
            }
        }
        Some(_) => return Err("`visuals` must be a list of visual names".into()),
    }
    if out.is_empty() {
        return Err("name the visuals: `visual` or `visuals` (a list of names)".into());
    }
    Ok(out)
}

/// `visual.set_hidden` (the Selection pane's eye): whether the visuals of the page are shown.
pub(crate) fn visual_set_hidden(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "visual", "visuals", "hidden"])?;
    let key = str_param(p, "page")?;
    let hidden =
        opt_bool(p, "hidden")?.ok_or("missing `hidden`: true hides the visuals, false shows them")?;
    let names = listed_visuals(p)?;
    let (_, r) = parts(s)?;
    let pi = page_index(r, key)?;
    let page = page_at_mut(r, pi)?;
    let page_name = page.name.clone();
    let mut new = page.clone();
    let mut out = Vec::with_capacity(names.len());
    for name in &names {
        let i = visual_index(page, name)?;
        let Some(v) = new.visuals.get_mut(i) else {
            continue;
        };
        v.is_hidden = hidden;
        out.push(json!({"visual": v.name, "hidden": v.is_hidden}));
    }
    let changed = new != *page;
    if changed {
        *page = new;
        s.report_dirty = true;
    }
    Ok(json!({"page": page_name, "hidden": hidden, "changed": changed, "visuals": out}))
}

/// `visual.rename_group`: a visual group's display name (`visualGroup.displayName`).
pub(crate) fn visual_rename_group(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "group", "display_name"])?;
    let key = str_param(p, "page")?;
    let group = str_param(p, "group")?;
    let display = str_param(p, "display_name")?.trim();
    if display.is_empty() || display.chars().count() > MAX_GROUP_NAME {
        return Err(format!("`display_name` must be 1 to {MAX_GROUP_NAME} characters"));
    }
    let (_, r) = parts(s)?;
    let pi = page_index(r, key)?;
    let (page_name, group_name, changed) = {
        let page = page_at_mut(r, pi)?;
        let page_name = page.name.clone();
        let i = visual_index(page, group)?;
        let Some(v) = page.visuals.get_mut(i) else {
            return Err("internal: visual index out of range".into());
        };
        if v.visual_type.is_some() {
            return Err(format!(
                "`{}` is a visual, not a group: a group's name is its display name",
                v.name
            ));
        }
        let Some(g) = v.extra.get_mut("visualGroup").and_then(Value::as_object_mut) else {
            return Err(format!("`{}` has no visualGroup: it is not a group", v.name));
        };
        let changed = g.get("displayName").and_then(Value::as_str) != Some(display);
        if changed {
            g.insert("displayName".into(), json!(display));
        }
        (page_name, v.name.clone(), changed)
    };
    if changed {
        s.report_dirty = true;
    }
    Ok(json!({
        "page": page_name,
        "group": group_name,
        "display_name": display,
        "changed": changed,
    }))
}

/// `visual.tab_order`: the keyboard (tab) order of the visuals listed, in that order.
pub(crate) fn visual_tab_order(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "visuals"])?;
    let key = str_param(p, "page")?;
    let Some(Value::Array(list)) = p.get("visuals") else {
        return Err("missing `visuals`: the visual names in tab order".into());
    };
    if list.is_empty() || list.len() > MAX_ORDER {
        return Err(format!("`visuals` must list 1 to {MAX_ORDER} visual names"));
    }
    let mut names: Vec<&str> = Vec::with_capacity(list.len());
    for x in list {
        let n = x.as_str().ok_or("`visuals` must be a list of visual names")?;
        if names.iter().any(|o| o.eq_ignore_ascii_case(n)) {
            return Err(format!("`{n}` is listed twice"));
        }
        names.push(n);
    }
    let (_, r) = parts(s)?;
    let pi = page_index(r, key)?;
    let (page_name, changed, out) = {
        let page = page_at_mut(r, pi)?;
        let page_name = page.name.clone();
        let mut idx = Vec::with_capacity(names.len());
        for n in &names {
            idx.push(visual_index(page, n)?);
        }
        let mut new = page.clone();
        let mut out = Vec::with_capacity(idx.len());
        for (k, i) in idx.into_iter().enumerate() {
            let Some(v) = new.visuals.get_mut(i) else {
                continue;
            };
            let tab = 1000_i64.saturating_mul(k as i64);
            v.position.tab_order = Some(tab);
            out.push(json!({"visual": v.name, "tab_order": tab}));
        }
        let changed = new != *page;
        if changed {
            *page = new;
        }
        (page_name, changed, out)
    };
    if changed {
        s.report_dirty = true;
    }
    Ok(json!({"page": page_name, "changed": changed, "tab_order": out}))
}

#[path = "visual_extras_mobile.rs"]
mod mobile;
#[cfg(test)]
pub(crate) use mobile::mobile_position;
pub(crate) use mobile::{merged_visual, mobile_doc, mobile_snapshot, visual_mobile, visual_mobile_format};

#[path = "visual_extras_bars.rs"]
mod bars;
pub(crate) use bars::{ErrorBars, error_bars};

#[path = "visual_extras_anomaly.rs"]
mod anomaly;
pub(crate) use anomaly::anomaly_detection;

#[cfg(test)]
#[path = "visual_extras_tests.rs"]
mod tests;

/// `visual.drill_filter {page, visual, apply_to: "page" | "visual" | "default"}`: Desktop's
/// "Apply drill down filters to" (Format ribbon): `page` ("Entire page") writes
/// `drillFilterOtherVisuals: true`, so drilling into the visual filters the page's other visuals;
/// `visual` ("Selected visual") writes `false`; `default` removes it, so the report's
/// "Drilling filters other visuals" (`report.settings`) applies.
pub(crate) fn visual_drill_filter(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "visual", "apply_to"])?;
    let page_key = str_param(p, "page")?;
    let visual_key = str_param(p, "visual")?;
    let value = match str_param(p, "apply_to")?.to_ascii_lowercase().as_str() {
        "page" => Some(true),
        "visual" => Some(false),
        "default" => None,
        other => {
            return Err(format!(
                "`apply_to` must be \"page\", \"visual\" or \"default\", got `{other}`"
            ));
        }
    };
    let (_, r) = parts(s)?;
    let pi = crate::report::page_index(r, page_key)?;
    let page = r.pages.get_mut(pi).ok_or("internal: page index out of range")?;
    let vi = crate::report::visual_index(page, visual_key)?;
    let v = page
        .visuals
        .get_mut(vi)
        .ok_or("internal: visual index out of range")?;
    let before = v.visual_extra.get("drillFilterOtherVisuals").cloned();
    match value {
        Some(b) => {
            v.visual_extra.insert("drillFilterOtherVisuals".into(), json!(b));
        }
        None => {
            v.visual_extra.shift_remove("drillFilterOtherVisuals");
        }
    }
    let changed = v.visual_extra.get("drillFilterOtherVisuals").cloned() != before;
    let name = v.name.clone();
    if changed {
        s.report_dirty = true;
    }
    let r = s.report.as_ref().ok_or("the session has no report")?;
    let effective = value.unwrap_or_else(|| crate::interact::drill_filters_default(r));
    Ok(json!({
        "visual": name,
        "changed": changed,
        "apply_to": match value { Some(true) => "page", Some(false) => "visual", None => "default" },
        "filters_other_visuals": effective,
    }))
}
