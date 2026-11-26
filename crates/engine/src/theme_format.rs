//! Renderer formatting keys (M9.6, `docs/view-snapshot.md`, "Renderer formatting"): data label
//! units, precision, position, density and background; line markers, style and steps; the new
//! card's accent bar, callout, label, reference labels, layout and image; gauge axis bounds and
//! callout; pie/donut slices. Resolved like the other `format` keys: the theme's visual styles
//! (`*` then the visual type), then the visual's own `objects` entries without a selector (or
//! with the `default` state). Per-series entries (`selector.metadata`) are listed separately.

use bicraft_report::{FieldRef, Visual};
use serde_json::{Map, Value, json};

use super::{Prop, Theme, number, prop, prop_json, sources};
use crate::refs::{field_text, same_field};

/// Most per-series entries carried for one object.
const MAX_SERIES: usize = 200;
/// Most characters of a text value (image URLs, label text).
const MAX_TEXT: usize = 2000;

/// How a property value is written to the snapshot.
#[derive(Clone, Copy)]
enum K {
    /// As read (booleans, numbers, colors, text).
    Plain,
    /// Text lower-cased.
    Lower,
    /// A number; numeric text (`'1000'`) is parsed.
    Num,
    /// `on` / `off` → `true` / `false`; `auto` clears the key.
    OnOff,
}

/// `(snapshot key, PBIR property, kind)`: for one key, a later source wins; within a source, the
/// first listed property that is set wins.
type Spec = [(&'static str, &'static str, K)];

const LABELS: &Spec = &[
    ("show", "show", K::Plain),
    ("color", "color", K::Plain),
    ("font_size", "fontSize", K::Num),
    ("font_family", "fontFamily", K::Plain),
    ("bold", "bold", K::Plain),
    ("italic", "italic", K::Plain),
    ("transparency", "transparency", K::Num),
    ("display_units", "labelDisplayUnits", K::Num),
    ("precision", "labelPrecision", K::Num),
    ("position", "labelPosition", K::Lower),
    ("position", "position", K::Lower),
    ("density", "labelDensity", K::Num),
    ("background", "enableBackground", K::Plain),
    ("background", "background", K::OnOff),
    ("background_color", "backgroundColor", K::Plain),
    ("background_transparency", "backgroundTransparency", K::Num),
    ("label_style", "labelStyle", K::Plain),
    ("percent_precision", "percentageLabelPrecision", K::Num),
    ("overflow", "overflow", K::Plain),
];

const LINES: &Spec = &[
    ("show", "strokeShow", K::Plain),
    ("width", "strokeWidth", K::Num),
    ("style", "lineStyle", K::Lower),
    ("chart_type", "lineChartType", K::Lower),
    ("stepped", "stepped", K::Plain),
    ("show_marker", "showMarker", K::Plain),
    ("marker_shape", "markerShape", K::Plain),
    ("marker_size", "markerSize", K::Num),
    ("marker_color", "markerColor", K::Plain),
    ("show_area", "areaShow", K::Plain),
];

const ACCENT_BAR: &Spec = &[
    ("show", "show", K::Plain),
    ("position", "position", K::Lower),
    ("color", "color", K::Plain),
    ("width", "width", K::Num),
    ("transparency", "transparency", K::Num),
];

const CARD_VALUE: &Spec = &[
    ("show", "show", K::Plain),
    ("font_size", "fontSize", K::Num),
    ("color", "fontColor", K::Plain),
    ("font_family", "fontFamily", K::Plain),
    ("bold", "bold", K::Plain),
    ("italic", "italic", K::Plain),
    ("transparency", "transparency", K::Num),
    ("display_units", "labelDisplayUnits", K::Num),
    ("precision", "labelPrecision", K::Num),
    ("alignment", "horizontalAlignment", K::Lower),
    ("blank_as", "showBlankAs", K::Plain),
];

const CARD_LABEL: &Spec = &[
    ("show", "show", K::Plain),
    ("text", "text", K::Plain),
    ("font_size", "fontSize", K::Num),
    ("color", "fontColor", K::Plain),
    ("font_family", "fontFamily", K::Plain),
    ("bold", "bold", K::Plain),
    ("italic", "italic", K::Plain),
    ("position", "position", K::Lower),
    ("alignment", "horizontalAlignment", K::Lower),
];

const REF_TITLE: &Spec = &[
    ("show", "show", K::Plain),
    ("text", "titleText", K::Plain),
    ("font_size", "titleFontSize", K::Num),
    ("color", "titleFontColor", K::Plain),
    ("bold", "titleBold", K::Plain),
];

const REF_VALUE: &Spec = &[
    ("show", "show", K::Plain),
    ("font_size", "valueFontSize", K::Num),
    ("color", "valueFontColor", K::Plain),
    ("bold", "valueBold", K::Plain),
    ("display_units", "valueDisplayUnits", K::Num),
    ("precision", "valuePrecision", K::Num),
];

const REF_DETAIL: &Spec = &[
    ("show", "show", K::Plain),
    ("font_size", "detailFontSize", K::Num),
    ("color", "detailFontColor", K::Plain),
    ("display_units", "detailDisplayUnits", K::Num),
    ("precision", "detailPrecision", K::Num),
];

const REF_LAYOUT: &Spec = &[
    ("position", "position", K::Lower),
    ("arrangement", "arrangement", K::Lower),
    ("style", "style", K::Lower),
];

const REF_BOX: &Spec = &[
    ("background", "backgroundShow", K::Plain),
    ("background_color", "backgroundColor", K::Plain),
    ("background_transparency", "backgroundTransparency", K::Num),
];

const CARD_LAYOUT: &Spec = &[
    ("style", "style", K::Lower),
    ("orientation", "orientation", K::Num),
    ("rows", "rowCount", K::Num),
    ("columns", "columnCount", K::Num),
    ("max_tiles", "maxTiles", K::Num),
    ("auto_grid", "autoGrid", K::Plain),
    ("alignment", "alignment", K::Lower),
    ("callout_size", "calloutSize", K::Num),
    ("cell_padding", "cellPadding", K::Num),
    ("collage", "collage", K::Plain),
    ("content_order", "contentOrder", K::Plain),
];

const CARD_IMAGE: &Spec = &[
    ("show", "show", K::Plain),
    ("type", "imageType", K::Plain),
    ("url", "imageUrl", K::Plain),
    ("position", "position", K::Lower),
    ("size", "size", K::Num),
    ("fit", "fit", K::Lower),
    ("transparency", "transparency", K::Num),
];

const GAUGE_AXIS: &Spec = &[
    ("min", "min", K::Num),
    ("max", "max", K::Num),
    ("target", "target", K::Num),
];

const GAUGE_LABEL: &Spec = &[
    ("show", "show", K::Plain),
    ("color", "color", K::Plain),
    ("font_size", "fontSize", K::Num),
    ("font_family", "fontFamily", K::Plain),
    ("bold", "bold", K::Plain),
    ("display_units", "labelDisplayUnits", K::Num),
    ("precision", "labelPrecision", K::Num),
];

const SLICES: &Spec = &[
    ("inner_radius", "innerRadiusRatio", K::Num),
    ("start_angle", "startAngle", K::Num),
];

/// A KPI's indicator number (`indicator` object).
const KPI_INDICATOR: &Spec = &[
    ("display_units", "indicatorDisplayUnits", K::Num),
    ("precision", "indicatorPrecision", K::Num),
    ("font_size", "fontSize", K::Num),
    ("color", "fontColor", K::Plain),
];

/// A KPI's status colours (`status`).
const KPI_STATUS: &Spec = &[
    ("good_color", "goodColor", K::Plain),
    ("neutral_color", "neutralColor", K::Plain),
    ("bad_color", "badColor", K::Plain),
];

/// A KPI's goal and distance colours (`goals`).
const KPI_GOALS: &Spec = &[
    ("goal_color", "goalFontColor", K::Plain),
    ("distance_color", "distanceFontColor", K::Plain),
];

const TABLE_GRID: &Spec = &[("row_padding", "rowPadding", K::Num)];

const TABLE_FONT: &Spec = &[("font_size", "fontSize", K::Num)];

/// A matrix's row headers: the +/- icons (`showExpandCollapseButtons`, Desktop's "+/- icons").
const MATRIX_ROW_HEADERS: &Spec = &[("expand_buttons", "showExpandCollapseButtons", K::Plain)];

/// A ribbon chart's ribbons (`ribbonBands`, Desktop's Ribbons card: colour, Match series color,
/// Transparency, Border).
const RIBBON_BANDS: &Spec = &[
    ("show", "show", K::Plain),
    ("color", "fillColor", K::Plain),
    ("match_color", "fillMatchColor", K::Plain),
    ("transparency", "fillTransparency", K::Num),
    ("border", "borderShow", K::Plain),
    ("border_color", "borderColor", K::Plain),
    ("border_match", "borderColorMatchFill", K::Plain),
    ("border_width", "borderSize", K::Num),
    ("border_transparency", "borderTransparency", K::Num),
];

/// A ribbon chart's Spacing (`layout.ribbonGapSize`, a percentage of the tallest column).
const RIBBON_LAYOUT: &Spec = &[("gap", "ribbonGapSize", K::Num)];

/// Total labels over stacked columns (`totals`).
const TOTALS: &Spec = &[
    ("show", "show", K::Plain),
    ("color", "color", K::Plain),
    ("font_size", "fontSize", K::Num),
    ("font_family", "fontFamily", K::Plain),
    ("bold", "bold", K::Plain),
    ("display_units", "labelDisplayUnits", K::Num),
    ("precision", "labelPrecision", K::Num),
];

/// A matrix's `subTotals` switches and labels (whole-visual entries; the per-level entries are
/// the preview's `data.row_level_totals` / `column_level_totals`).
const MATRIX_TOTALS: &Spec = &[
    ("row_subtotals", "rowSubtotals", K::Plain),
    ("column_subtotals", "columnSubtotals", K::Plain),
    ("row_subtotals_position", "rowSubtotalsPosition", K::Lower),
    ("row_subtotals_label", "rowSubtotalsLabel", K::Plain),
    ("column_subtotals_label", "columnSubtotalsLabel", K::Plain),
];

/// Whether a visual object entry applies to the whole visual: no selector, or the `default`
/// state.
fn whole_visual(e: &Value) -> bool {
    match e.get("selector") {
        None | Some(Value::Null) => true,
        Some(s) => s
            .as_object()
            .is_some_and(|o| o.len() == 1 && o.get("id").and_then(Value::as_str) == Some("default")),
    }
}

/// The Gantt's shape objects (`bar`, `milestone`, `progress`): fill and border.
const GANTT_SHAPE: &Spec = &[
    ("show", "show", K::Plain),
    ("color", "color", K::Plain),
    ("customize_color", "customizeColor", K::Plain),
    ("transparency", "transparency", K::Num),
    ("height", "height", K::Num),
    ("corner_radius", "cornerRadius", K::Num),
    ("relative_height", "relativeHeight", K::Num),
    ("shape", "shape", K::Lower),
    ("size", "size", K::Num),
    ("border_show", "borderShow", K::Plain),
    ("border_color", "borderColor", K::Plain),
    ("border_match", "borderColorMatchFill", K::Plain),
    ("border_transparency", "borderTransparency", K::Num),
    ("border_width", "borderWidth", K::Num),
];

const GANTT_ROW: &Spec = &[
    ("color", "color", K::Plain),
    ("color_by_category", "colorByCategory", K::Plain),
    ("background_show", "backgroundShow", K::Plain),
    ("background_customize", "backgroundCustomizeColor", K::Plain),
    ("background_color", "backgroundColor", K::Plain),
    ("background_transparency", "backgroundTransparency", K::Num),
    ("divider_show", "dividerShow", K::Plain),
    ("divider_color", "dividerColor", K::Plain),
    ("divider_width", "dividerWidth", K::Num),
    ("divider_style", "dividerStyle", K::Lower),
    ("divider_transparency", "dividerTransparency", K::Num),
    ("icon_color", "expandCollapseIconColor", K::Plain),
    ("toggle_type", "expandCollapseToggleType", K::Num),
    ("collapsed", "isCollapsed", K::Plain),
    ("indent", "steppedLayoutIndentation", K::Num),
];

const GANTT_AXIS: &Spec = &[
    ("auto_scale", "autoScale", K::Plain),
    ("start", "start", K::Plain),
    ("end", "end", K::Plain),
    ("format_day", "formatDay", K::Plain),
    ("format_month", "formatMonth", K::Plain),
    ("format_year", "formatYear", K::Plain),
    ("primary_unit", "primaryUnit", K::Lower),
    ("secondary_unit", "secondaryUnit", K::Lower),
    ("position", "position", K::Lower),
    ("invert", "invertAxis", K::Plain),
    ("gridline_show", "gridlineShow", K::Plain),
    ("gridline_color", "gridlineColor", K::Plain),
    ("gridline_style", "gridlineStyle", K::Lower),
    ("gridline_thickness", "gridlineThickness", K::Num),
    ("gridline_transparency", "gridlineTransparency", K::Num),
    ("shading_show", "shadingShow", K::Plain),
    ("shading_color", "shadingColor", K::Plain),
    ("shading_transparency", "shadingTransparency", K::Num),
    ("zoom_show", "zoomShow", K::Plain),
];

/// Text objects (`axisTick`, `columnHeaders`, `columnValue`).
const GANTT_TEXT: &Spec = &[
    ("show", "show", K::Plain),
    ("color", "color", K::Plain),
    ("font_size", "fontSize", K::Num),
    ("font_family", "fontFamily", K::Plain),
    ("bold", "bold", K::Plain),
    ("italic", "italic", K::Plain),
    ("alignment", "horizontalAlignment", K::Lower),
    ("background_color", "backgroundColor", K::Plain),
    ("background_transparency", "backgroundTransparency", K::Num),
    ("default_width", "defaultColumnWidth", K::Num),
    ("custom_width", "customColumnWidth", K::Plain),
    ("text_wrap", "textWrap", K::Plain),
];

const GANTT_PANEL: &Spec = &[("width", "width", K::Num)];

/// Week days named by `axis.shadingDays`: a theme's list, or a report literal holding the names
/// (separated by anything but letters; BIcraft's reading, the PBIR form is not documented).
fn shading_days(v: &Visual, theme: Option<&Theme>) -> Option<Value> {
    const DAYS: [&str; 7] = [
        "sunday",
        "monday",
        "tuesday",
        "wednesday",
        "thursday",
        "friday",
        "saturday",
    ];
    let mut out = None;
    for (entry, from_theme) in entries(v, theme, "axis") {
        let words: Vec<String> = if from_theme {
            match entry.get("shadingDays").and_then(Value::as_array) {
                Some(a) => a
                    .iter()
                    .take(16)
                    .filter_map(Value::as_str)
                    .map(str::to_ascii_lowercase)
                    .collect(),
                None => continue,
            }
        } else {
            let Some(lit) = entry
                .get("properties")
                .and_then(|p| p.get("shadingDays"))
                .and_then(|x| x.get("expr"))
                .and_then(|e| e.get("Literal"))
                .and_then(|l| l.get("Value"))
                .and_then(Value::as_str)
            else {
                continue;
            };
            lit.chars()
                .take(MAX_TEXT)
                .collect::<String>()
                .split(|c: char| !c.is_ascii_alphabetic())
                .filter(|w| !w.is_empty())
                .map(str::to_ascii_lowercase)
                .collect()
        };
        let days: Vec<usize> = DAYS
            .iter()
            .enumerate()
            .filter(|(_, d)| words.iter().any(|w| w == *d))
            .map(|(i, _)| i)
            .collect();
        out = Some(json!(days));
    }
    out
}

/// `format.gantt`: the Gantt's formatting objects in snapshot form (docs/formats/gantt.md).
fn gantt(v: &Visual, theme: Option<&Theme>) -> Option<Value> {
    let parts: [(&str, &str, &Spec); 10] = [
        ("bar", "bar", GANTT_SHAPE),
        ("milestone", "milestone", GANTT_SHAPE),
        ("progress", "progress", GANTT_SHAPE),
        ("row", "row", GANTT_ROW),
        ("axis", "axis", GANTT_AXIS),
        ("axis_tick", "axisTick", GANTT_TEXT),
        ("column_headers", "columnHeaders", GANTT_TEXT),
        ("column_value", "columnValue", GANTT_TEXT),
        ("column_panel", "columnPanel", GANTT_PANEL),
        (
            "general",
            "general",
            &[("format_string", "formatString", K::Plain)],
        ),
    ];
    let mut g = Map::new();
    for (key, object, spec) in parts {
        if let Some(x) = collect(v, theme, object, spec) {
            g.insert(key.into(), Value::Object(x));
        }
    }
    if let Some(days) = shading_days(v, theme) {
        let axis = g.entry("axis").or_insert_with(|| Value::Object(Map::new()));
        if let Some(a) = axis.as_object_mut() {
            a.insert("shading_days".into(), days);
        }
    }
    (!g.is_empty()).then_some(Value::Object(g))
}

/// Sources of `object`, lowest priority first: theme styles, then every whole-visual entry.
fn entries<'a>(v: &'a Visual, theme: Option<&'a Theme>, object: &str) -> Vec<(&'a Value, bool)> {
    let mut out: Vec<(&Value, bool)> = sources(v, theme, object)
        .into_iter()
        .filter(|(_, from_theme)| *from_theme)
        .collect();
    let list = v
        .visual_extra
        .get("objects")
        .and_then(|o| o.get(object))
        .and_then(Value::as_array);
    out.extend(
        list.into_iter()
            .flatten()
            .filter(|e| whole_visual(e))
            .map(|e| (e, false)),
    );
    out
}

/// One property value in snapshot form; `Some(Null)` clears the key (`auto`).
fn convert(kind: K, p: &Prop) -> Option<Value> {
    match (kind, p) {
        (K::Lower, Prop::Text(s)) => Some(json!(s.to_ascii_lowercase())),
        (K::Num, Prop::Num(x)) => Some(number(*x)),
        (K::Num, Prop::Text(s)) => s.trim().parse::<f64>().ok().filter(|x| x.is_finite()).map(number),
        (K::Num, _) => None,
        (K::OnOff, Prop::Text(s)) => match s.to_ascii_lowercase().as_str() {
            "on" => Some(json!(true)),
            "off" => Some(json!(false)),
            _ => Some(Value::Null),
        },
        (K::OnOff, Prop::Bool(b)) => Some(json!(b)),
        (_, Prop::Text(s)) => Some(json!(s.chars().take(MAX_TEXT).collect::<String>())),
        _ => Some(prop_json(p)),
    }
}

/// Applies `spec` from one entry onto `o`.
fn apply(o: &mut Map<String, Value>, entry: &Value, from_theme: bool, theme: Option<&Theme>, spec: &Spec) {
    let mut done: Vec<&str> = Vec::new();
    for (key, name, kind) in spec {
        if done.contains(key) {
            continue;
        }
        let Some(value) = prop(entry, name, from_theme, theme).and_then(|p| convert(*kind, &p)) else {
            continue;
        };
        done.push(*key);
        if value.is_null() {
            o.remove(*key);
        } else {
            o.insert((*key).to_string(), value);
        }
    }
}

/// `spec` of `object` over all its whole-visual sources.
fn collect(v: &Visual, theme: Option<&Theme>, object: &str, spec: &Spec) -> Option<Map<String, Value>> {
    let mut o = Map::new();
    for (entry, from_theme) in entries(v, theme, object) {
        apply(&mut o, entry, from_theme, theme, spec);
    }
    (!o.is_empty()).then_some(o)
}

fn is_type(v: &Visual, names: &[&str]) -> bool {
    v.visual_type
        .as_deref()
        .is_some_and(|t| names.iter().any(|n| t.eq_ignore_ascii_case(n)))
}

/// A field-bound property (`{"expr": {"Measure": …}}`): `{field, column?}`.
fn bound_field(v: &Visual, entry: &Value, name: &str) -> Option<Value> {
    let expr = entry.get("properties")?.get(name)?.get("expr")?;
    let f = bicraft_report::query::field_ref(Some(expr));
    if matches!(f, FieldRef::Other { .. }) {
        return None;
    }
    let mut o = Map::new();
    o.insert("field".into(), json!(field_text(&f)));
    if let Some(i) = v.fields.iter().position(|b| same_field(&b.field, &f)) {
        o.insert("column".into(), json!(i));
    }
    Some(Value::Object(o))
}

/// Adds the renderer keys of `v` to its `format` map `o`.
pub(super) fn add(o: &mut Map<String, Value>, v: &Visual, theme: Option<&Theme>) {
    if let Some(l) = collect(v, theme, "labels", LABELS) {
        o.insert("labels".into(), Value::Object(l));
    }
    if let Some(mut l) = collect(v, theme, "lineStyles", LINES) {
        if l.get("chart_type").and_then(Value::as_str) == Some("step") {
            l.insert("stepped".into(), json!(true));
        }
        o.insert("lines".into(), Value::Object(l));
    }
    if is_type(v, &["cardVisual"]) {
        let mut card = Map::new();
        let parts: [(&str, &str, &Spec); 5] = [
            ("accent_bar", "accentBar", ACCENT_BAR),
            ("value", "value", CARD_VALUE),
            ("label", "label", CARD_LABEL),
            ("layout", "layout", CARD_LAYOUT),
            ("image", "image", CARD_IMAGE),
        ];
        for (key, object, spec) in parts {
            if let Some(x) = collect(v, theme, object, spec) {
                card.insert(key.into(), Value::Object(x));
            }
        }
        let mut refl = collect(v, theme, "referenceLabelLayout", REF_LAYOUT).unwrap_or_default();
        if let Some(b) = collect(v, theme, "referenceLabel", REF_BOX) {
            refl.extend(b);
        }
        let ref_parts: [(&str, &str, &Spec); 3] = [
            ("title", "referenceLabelTitle", REF_TITLE),
            ("value", "referenceLabelValue", REF_VALUE),
            ("detail", "referenceLabelDetail", REF_DETAIL),
        ];
        for (key, object, spec) in ref_parts {
            if let Some(x) = collect(v, theme, object, spec) {
                refl.insert(key.into(), Value::Object(x));
            }
        }
        if !refl.is_empty() {
            card.insert("reference_label".into(), Value::Object(refl));
        }
        if !card.is_empty() {
            o.insert("card".into(), Value::Object(card));
        }
    }
    if is_type(v, &["gauge"]) {
        let mut g = collect(v, theme, "axis", GAUGE_AXIS).unwrap_or_default();
        for (entry, from_theme) in entries(v, theme, "axis") {
            if from_theme {
                continue;
            }
            for key in ["min", "max", "target"] {
                if let Some(f) = bound_field(v, entry, key) {
                    g.insert(key.into(), f);
                }
            }
        }
        for (key, object) in [("callout", "calloutValue"), ("target_label", "target")] {
            if let Some(x) = collect(v, theme, object, GAUGE_LABEL) {
                g.insert(key.into(), Value::Object(x));
            }
        }
        if let Some(c) = collect(v, theme, "dataPoint", &[("target_color", "target", K::Plain)])
            .and_then(|m| m.get("target_color").cloned())
        {
            g.insert("target_color".into(), c);
        }
        if !g.is_empty() {
            o.insert("gauge".into(), Value::Object(g));
        }
    }
    if is_type(v, &["tableEx", "pivotTable"]) {
        let mut t = collect(v, theme, "grid", TABLE_GRID).unwrap_or_default();
        if let Some(x) = collect(v, theme, "values", TABLE_FONT).and_then(|m| m.get("font_size").cloned()) {
            t.insert("font_size".into(), x);
        }
        if let Some(x) =
            collect(v, theme, "columnHeaders", TABLE_FONT).and_then(|m| m.get("font_size").cloned())
        {
            t.insert("header_font_size".into(), x);
        }
        if is_type(v, &["pivotTable"])
            && let Some(x) = collect(v, theme, "subTotals", MATRIX_TOTALS)
        {
            t.extend(x);
        }
        if is_type(v, &["pivotTable"])
            && let Some(x) = collect(v, theme, "rowHeaders", MATRIX_ROW_HEADERS)
        {
            t.extend(x);
        }
        if !t.is_empty() {
            o.insert("table".into(), Value::Object(t));
        }
    }
    if is_type(v, &["ganttChart"])
        && let Some(g) = gantt(v, theme)
    {
        o.insert("gantt".into(), g);
    }
    if is_type(v, &["kpi"]) {
        let mut k = collect(v, theme, "indicator", KPI_INDICATOR).unwrap_or_default();
        for (object, spec) in [("status", KPI_STATUS), ("goals", KPI_GOALS)] {
            if let Some(x) = collect(v, theme, object, spec) {
                k.extend(x);
            }
        }
        if !k.is_empty() {
            o.insert("kpi".into(), Value::Object(k));
        }
    }
    if is_type(v, &["pieChart", "donutChart"])
        && let Some(s) = collect(v, theme, "slices", SLICES)
    {
        o.insert("pie".into(), Value::Object(s));
    }
    if is_type(v, &["ribbonChart"]) {
        let mut r = collect(v, theme, "ribbonBands", RIBBON_BANDS).unwrap_or_default();
        if let Some(x) = collect(v, theme, "layout", RIBBON_LAYOUT) {
            r.extend(x);
        }
        if !r.is_empty() {
            o.insert("ribbon".into(), Value::Object(r));
        }
        if let Some(t) = collect(v, theme, "totals", TOTALS) {
            o.insert("totals".into(), Value::Object(t));
        }
    }
}

/// Per-series label and line settings (`labels` / `lineStyles` entries whose selector is
/// `{"metadata": queryRef}`): `(snapshot key, [{column, …}])`. `refs` are the projections'
/// `queryRef`s in field order.
pub(crate) fn series_format(v: &Visual, refs: &[Option<String>]) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    for (key, object, spec) in [
        ("series_labels", "labels", LABELS),
        ("series_lines", "lineStyles", LINES),
    ] {
        let list = v
            .visual_extra
            .get("objects")
            .and_then(|o| o.get(object))
            .and_then(Value::as_array);
        let mut items = Vec::new();
        for e in list.into_iter().flatten() {
            if items.len() >= MAX_SERIES {
                break;
            }
            let Some(sel) = e.get("selector") else {
                continue;
            };
            let Some(r) = sel.get("metadata").and_then(Value::as_str) else {
                continue;
            };
            if sel.get("data").is_some() {
                continue;
            }
            let Some(column) = refs.iter().position(|x| x.as_deref() == Some(r)) else {
                continue;
            };
            let mut item = Map::new();
            apply(&mut item, e, false, None, spec);
            if item.get("chart_type").and_then(Value::as_str) == Some("step") {
                item.insert("stepped".into(), json!(true));
            }
            if item.is_empty() {
                continue;
            }
            item.insert("column".into(), json!(column));
            items.push(Value::Object(item));
        }
        if !items.is_empty() {
            out.push((key.to_string(), Value::Array(items)));
        }
    }
    out
}

#[cfg(test)]
#[path = "theme_format_tests.rs"]
mod tests;
