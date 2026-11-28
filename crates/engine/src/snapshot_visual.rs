//! Per-visual viewer fields beyond the M8.2 ones (`docs/view-snapshot.md`, "Visual details"):
//! text box paragraphs, button text and icon, slicer mode and selection, series and category
//! colors, table column styles, reference lines and small multiples. Every field is optional
//! and only present when the visual sets it.

use bicraft_report::{FieldRef, Visual};
use serde_json::{Map, Value, json};

use crate::preview::filters;
use crate::refs::{field_text, resolve_field, same_field};
use bicraft_model::SemanticModel;

/// Most paragraphs carried for one text box.
const MAX_PARAGRAPHS: usize = 500;
/// Most runs carried for one text box (all paragraphs).
const MAX_RUNS: usize = 2000;
/// Most characters of text carried for one text box.
const MAX_TEXT: usize = 20_000;
/// Most series / category colors carried.
const MAX_COLORS: usize = 500;
/// Most reference lines carried.
const MAX_LINES: usize = 50;

/// Adds the fields of `v` to its snapshot object `o`.
pub(super) fn add(o: &mut Map<String, Value>, v: &Visual, model: Option<&SemanticModel>) {
    let ty = v.visual_type.as_deref().unwrap_or("");
    if ty.eq_ignore_ascii_case("textbox")
        && let Some(t) = text(v)
    {
        o.insert("text".into(), t);
    }
    if ty.eq_ignore_ascii_case("actionButton")
        && let Some(b) = button(v)
    {
        o.insert("button".into(), b);
    }
    if ty.eq_ignore_ascii_case("shape")
        && let Some(sh) = shape(v)
    {
        o.insert("shape".into(), sh);
    }
    if ty.eq_ignore_ascii_case("pageNavigator") || ty.eq_ignore_ascii_case("bookmarkNavigator") {
        o.insert("navigator".into(), navigator(v));
    }
    // the action of a button, image or shape, for the app's action editor (visual.set_action)
    if let Some(a) = crate::rich::action_json(v) {
        o.insert("action".into(), a);
    }
    if filters::is_slicer(v) {
        o.insert("slicer".into(), slicer(v));
    }
    let mut extra = Map::new();
    let colors = series_colors(v);
    if !colors.is_empty() {
        extra.insert("series_colors".into(), Value::Array(colors));
    }
    let styles = column_styles(v);
    if !styles.is_empty() {
        extra.insert("column_styles".into(), Value::Array(styles));
    }
    extra.extend(crate::theme::series_format(v, &query_refs(v)));
    let table = table_layout(v);
    if !table.is_empty() {
        extra.insert("table".into(), Value::Object(table));
    }
    if ty.eq_ignore_ascii_case("ganttChart") {
        extra.extend(gantt_layout(v));
    }
    let props = crate::visual_props::extract_props(v);
    let lines = reference_lines(v, &props, model);
    if !lines.is_empty() {
        extra.insert("reference_lines".into(), Value::Array(lines));
    }
    if let Some(sm) = small_multiples(v, &props, model) {
        extra.insert("small_multiples".into(), sm);
    }
    if extra.is_empty() {
        return;
    }
    let format = o.entry("format").or_insert(Value::Null);
    if !format.is_object() {
        *format = Value::Object(Map::new());
    }
    if let Some(f) = format.as_object_mut() {
        // table keys join the theme-resolved `table` object
        let table_extra = extra.remove("table");
        f.extend(extra);
        if let Some(Value::Object(add)) = table_extra {
            let t = f.entry("table").or_insert_with(|| Value::Object(Map::new()));
            if let Some(t) = t.as_object_mut() {
                t.extend(add);
            }
        }
    }
}

/// Table/matrix layout keys of `v` (`format.table`): `column_widths` (`columnWidth` entries
/// with a `metadata` selector: `[{column, width}]`) and `sort` (the `sortDefinition` fields
/// bound to the visual: `[{column, descending}]`).
fn table_layout(v: &Visual) -> Map<String, Value> {
    let mut t = Map::new();
    let ty = v.visual_type.as_deref().unwrap_or("");
    if !matches!(ty, "tableEx" | "pivotTable") {
        return t;
    }
    let refs = query_refs(v);
    let mut widths = Vec::new();
    let list = v
        .visual_extra
        .get("objects")
        .and_then(|o| o.get("columnWidth"))
        .and_then(Value::as_array);
    for e in list.into_iter().flatten().take(MAX_COLORS) {
        let r = e
            .get("selector")
            .and_then(|s| s.get("metadata"))
            .and_then(Value::as_str);
        let (Some(c), Some(w)) = (
            r.and_then(|r| column_of_ref(&refs, r)),
            e.get("properties")
                .and_then(Value::as_object)
                .and_then(|p| lit_num(p, "value"))
                .filter(|w| *w > 0.0 && *w < 100_000.0),
        ) else {
            continue;
        };
        widths.push(json!({"column": c, "width": w}));
    }
    if !widths.is_empty() {
        t.insert("column_widths".into(), Value::Array(widths));
    }
    let sorts = v
        .visual_extra
        .get("query")
        .and_then(|q| q.get("sortDefinition"))
        .and_then(|s| s.get("sort"))
        .and_then(Value::as_array);
    let mut sort = Vec::new();
    for s in sorts.into_iter().flatten().take(MAX_LINES) {
        let f = bicraft_report::query::field_ref(s.get("field"));
        if let Some(c) = v.fields.iter().position(|b| same_field(&b.field, &f)) {
            let desc = s.get("direction").and_then(Value::as_str) == Some("Descending");
            sort.push(json!({"column": c, "descending": desc}));
        }
    }
    if !sort.is_empty() {
        t.insert("sort".into(), Value::Array(sort));
    }
    t
}

/// A Gantt's per-row and per-column settings (docs/formats/gantt.md): `gantt_rows` (`row`
/// entries with a `data` selector: `[{path: [{c, v}], collapsed}]`, the category members they
/// name and their `isCollapsed`) and `gantt_widths` (`columnWidth` entries with a `metadata`
/// selector: `[{column, width}]`).
fn gantt_layout(v: &Visual) -> Map<String, Value> {
    let mut out = Map::new();
    let objects = v.visual_extra.get("objects");
    let list = |name: &str| {
        objects
            .and_then(|o| o.get(name))
            .and_then(Value::as_array)
            .map(|a| a.iter().take(MAX_COLORS).collect::<Vec<_>>())
            .unwrap_or_default()
    };
    let mut rows = Vec::new();
    for e in list("row") {
        let (Some(props), Some(data)) = (
            e.get("properties").and_then(Value::as_object),
            e.get("selector")
                .and_then(|s| s.get("data"))
                .and_then(Value::as_array),
        ) else {
            continue;
        };
        let Some(collapsed) = lit_bool(props, "isCollapsed") else {
            continue;
        };
        let mut path = Vec::new();
        for d in data.iter().take(8) {
            let Some(cmp) = d.get("scopeId").and_then(|s| s.get("Comparison")) else {
                continue;
            };
            let (Some((t, c)), Some(text)) = (
                cmp.get("Left").and_then(column_expr),
                cmp.get("Right")
                    .and_then(|r| r.get("Literal"))
                    .and_then(|l| l.get("Value"))
                    .and_then(Value::as_str),
            ) else {
                continue;
            };
            let field = FieldRef::Column { table: t, column: c };
            if let Some(i) = v.fields.iter().position(|b| same_field(&b.field, &field)) {
                path.push(json!({"c": i, "v": literal_json(text)}));
            }
        }
        if !path.is_empty() {
            rows.push(json!({"path": path, "collapsed": collapsed}));
        }
    }
    // the reader's expanded and collapsed categories (view.interact gantt_toggle) come last,
    // so they win: each member on its Category field, top level first
    let cats: Vec<usize> = v
        .fields
        .iter()
        .enumerate()
        .filter(|(_, b)| b.role.eq_ignore_ascii_case("Category"))
        .map(|(i, _)| i)
        .collect();
    let reader = v
        .visual_extra
        .get(crate::interact::reader::GANTT_STATE)
        .and_then(Value::as_array);
    for e in reader.into_iter().flatten().take(MAX_COLORS) {
        let (Some(members), Some(collapsed)) = (
            e.get("path").and_then(Value::as_array),
            e.get("collapsed").and_then(Value::as_bool),
        ) else {
            continue;
        };
        let path: Vec<Value> = cats
            .iter()
            .zip(members)
            .map(|(c, m)| json!({"c": c, "v": m}))
            .collect();
        if !path.is_empty() && path.len() == members.len() {
            rows.push(json!({"path": path, "collapsed": collapsed}));
        }
    }
    if !rows.is_empty() {
        out.insert("gantt_rows".into(), Value::Array(rows));
    }
    let refs = query_refs(v);
    let mut widths = Vec::new();
    for e in list("columnWidth") {
        let (Some(c), Some(w)) = (
            e.get("selector")
                .and_then(|s| s.get("metadata"))
                .and_then(Value::as_str)
                .and_then(|r| column_of_ref(&refs, r)),
            e.get("properties")
                .and_then(Value::as_object)
                .and_then(|p| lit_num(p, "value"))
                .filter(|w| *w > 0.0 && *w < 100_000.0),
        ) else {
            continue;
        };
        widths.push(json!({"column": c, "width": w}));
    }
    if !widths.is_empty() {
        out.insert("gantt_widths".into(), Value::Array(widths));
    }
    out
}

/// The first entry of `objects.<object>` (`visual_extra.objects`) whose selector is
/// `{"id": state}`, else the one without a selector.
fn entry<'a>(v: &'a Visual, object: &str, state: Option<&str>) -> Option<&'a Map<String, Value>> {
    let list = v.visual_extra.get("objects")?.get(object)?.as_array()?;
    let props = |e: &'a Value| e.get("properties").and_then(Value::as_object);
    if let Some(s) = state
        && let Some(e) = list.iter().find(|e| {
            e.get("selector")
                .and_then(|x| x.get("id"))
                .and_then(Value::as_str)
                == Some(s)
        })
    {
        return props(e);
    }
    list.iter()
        .find(|e| e.get("selector").is_none_or(Value::is_null))
        .and_then(props)
}

/// A literal property: (text, number, bool) forms.
fn lit<'a>(props: &'a Map<String, Value>, name: &str) -> Option<&'a str> {
    props
        .get(name)?
        .get("expr")?
        .get("Literal")?
        .get("Value")?
        .as_str()
}

fn lit_text(props: &Map<String, Value>, name: &str) -> Option<String> {
    let t = lit(props, name)?;
    let inner = t.strip_prefix('\'')?.strip_suffix('\'')?;
    Some(inner.replace("''", "'"))
}

fn lit_num(props: &Map<String, Value>, name: &str) -> Option<f64> {
    let t = lit(props, name)?;
    t.strip_suffix(['D', 'L', 'M'])
        .unwrap_or(t)
        .parse::<f64>()
        .ok()
        .filter(|x| x.is_finite())
}

fn lit_bool(props: &Map<String, Value>, name: &str) -> Option<bool> {
    match lit(props, name)? {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// A plain color property (`{"solid": {"color": <literal>}}`).
fn lit_color(props: &Map<String, Value>, name: &str) -> Option<String> {
    let c = props.get(name)?.get("solid")?.get("color")?;
    let t = c.get("expr")?.get("Literal")?.get("Value")?.as_str()?;
    let inner = t.strip_prefix('\'')?.strip_suffix('\'')?;
    inner.starts_with('#').then(|| inner.to_string())
}

fn number(x: f64) -> Value {
    if x.fract() == 0.0 && x.abs() < 9.0e15 {
        json!(x as i64)
    } else {
        json!(x)
    }
}

/// `"14pt"` / `"14px"` / `14` → points.
fn font_size(v: &Value) -> Option<f64> {
    let x = match v {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) => {
            let t = s.trim();
            if let Some(px) = t.strip_suffix("px") {
                px.trim().parse::<f64>().ok()? * 0.75
            } else {
                t.strip_suffix("pt").unwrap_or(t).trim().parse::<f64>().ok()?
            }
        }
        _ => return None,
    };
    (x.is_finite() && x > 0.0 && x < 1000.0).then_some(x)
}

/// Text box paragraphs: `{paragraphs: [{align?, runs: [{text, bold?, italic?, underline?,
/// font_size?, color?, font_family?, url?, bound?}]}]}` (`bound`: the selector id of a dynamic
/// value, whose text `bound_values::apply` fills).
fn text(v: &Visual) -> Option<Value> {
    let paragraphs = v
        .visual_extra
        .get("objects")?
        .get("general")?
        .as_array()?
        .iter()
        .find_map(|e| e.get("properties")?.get("paragraphs")?.as_array())?;
    let (mut runs_left, mut chars_left) = (MAX_RUNS, MAX_TEXT);
    let values = crate::rich::text_values(v);
    let mut out = Vec::new();
    let mut cut = paragraphs.len() > MAX_PARAGRAPHS;
    for p in paragraphs.iter().take(MAX_PARAGRAPHS) {
        let mut runs = Vec::new();
        for r in p.get("textRuns").and_then(Value::as_array).into_iter().flatten() {
            if runs_left == 0 || chars_left == 0 {
                cut = true;
                break;
            }
            runs_left -= 1;
            let raw = r.get("value").and_then(Value::as_str).unwrap_or("");
            let take: String = raw.chars().take(chars_left).collect();
            chars_left = chars_left.saturating_sub(take.chars().count());
            let mut run = Map::new();
            run.insert("text".into(), json!(take));
            // a dynamic value: `bound_values::apply` fills its text
            if let Some(id) = r
                .get("value")
                .filter(|x| x.get("propertyIdentifier").is_some())
                .and_then(|x| x.get("selector"))
                .and_then(|s| s.get("id"))
                .and_then(Value::as_str)
            {
                run.insert("bound".into(), json!(id.chars().take(200).collect::<String>()));
                // its field, for the app's text editor (textbox.set takes it back)
                if let Some((_, f)) = values.iter().find(|(k, _)| k == id) {
                    run.insert("field".into(), json!(f));
                }
            }
            if let Some(st) = r.get("textStyle").and_then(Value::as_object) {
                let s = |k: &str| st.get(k).and_then(Value::as_str);
                if s("fontWeight").is_some_and(|w| w == "bold" || w.parse::<u32>().is_ok_and(|n| n >= 600)) {
                    run.insert("bold".into(), json!(true));
                }
                if s("fontStyle") == Some("italic") {
                    run.insert("italic".into(), json!(true));
                }
                if s("textDecoration").is_some_and(|d| d.contains("underline")) {
                    run.insert("underline".into(), json!(true));
                }
                if let Some(x) = st.get("fontSize").and_then(font_size) {
                    run.insert("font_size".into(), number(x));
                }
                if let Some(c) = s("color").filter(|c| c.starts_with('#') && c.len() <= 9) {
                    run.insert("color".into(), json!(c));
                }
                if let Some(f) = s("fontFamily") {
                    run.insert(
                        "font_family".into(),
                        json!(f.chars().take(200).collect::<String>()),
                    );
                }
            }
            if let Some(u) = r.get("url").and_then(Value::as_str) {
                run.insert("url".into(), json!(u.chars().take(2000).collect::<String>()));
            }
            runs.push(Value::Object(run));
        }
        let mut para = Map::new();
        if let Some(a) = p.get("horizontalTextAlignment").and_then(Value::as_str) {
            para.insert("align".into(), json!(a.to_ascii_lowercase()));
        }
        if let Some(l) = p
            .get("listType")
            .and_then(Value::as_str)
            .map(str::to_ascii_lowercase)
            && (l == "bullet" || l == "number")
        {
            para.insert("list".into(), json!(l));
        }
        para.insert("runs".into(), Value::Array(runs));
        out.push(Value::Object(para));
    }
    let mut o = Map::new();
    o.insert("paragraphs".into(), Value::Array(out));
    if cut {
        o.insert("truncated".into(), json!(true));
    }
    Some(Value::Object(o))
}

/// Button: `{text?, show_text?, font_size?, font_color?, align?, icon?, show_icon?,
/// icon_placement?, icon_color?, fill?, fill_transparency?, show_fill?, show_outline?, outline?,
/// outline_weight?, outline_transparency?, show_glow?, glow?, glow_blur?, glow_transparency?,
/// show_shadow?, shadow?, shadow_blur?, shadow_distance?, shadow_angle?, shadow_transparency?,
/// shape?: {tile?, adjust?, tail?}, states?: {hover?, press?, disabled?}}` from the default
/// state (the shape for every state: the schema gives it no `$id`); `states` holds what
/// each other state sets (only those properties), which the readers draw over the default one
/// (`docs/formats/buttons.md`). The fill images and custom icons per state are added by
/// `images::visual_fields` (`fill_image`, `icon_image`).
fn button(v: &Visual) -> Option<Value> {
    let mut o = button_props(|object| entry(v, object, Some("default")));
    let mut shape = Map::new();
    if let Some(s) = entry(v, "shape", Some("default")) {
        tile_shape(s, &mut shape);
    }
    if !shape.is_empty() {
        o.insert("shape".into(), Value::Object(shape));
    }
    let mut states = Map::new();
    for state in ["hover", "press", "disabled"] {
        let s = button_props(|object| exact_state(v, object, state));
        if !s.is_empty() {
            states.insert(state.into(), Value::Object(s));
        }
    }
    if !states.is_empty() {
        o.insert("states".into(), Value::Object(states));
    }
    (!o.is_empty()).then_some(Value::Object(o))
}

/// The entry of `objects.<object>` whose selector is exactly `{id: state}` (no fallback).
fn exact_state<'a>(v: &'a Visual, object: &str, state: &str) -> Option<&'a Map<String, Value>> {
    v.visual_extra
        .get("objects")?
        .get(object)?
        .as_array()?
        .iter()
        .take(64)
        .find(|e| {
            e.get("selector")
                .and_then(|x| x.get("id"))
                .and_then(Value::as_str)
                == Some(state)
        })?
        .get("properties")?
        .as_object()
}

/// A button's text, icon, fill and outline properties of one state (`get` gives the state's
/// entry of an object).
fn button_props<'a>(get: impl Fn(&str) -> Option<&'a Map<String, Value>>) -> Map<String, Value> {
    let mut o = Map::new();
    if let Some(t) = get("text") {
        if let Some(x) = lit_text(t, "text") {
            o.insert("text".into(), json!(x.chars().take(1000).collect::<String>()));
        }
        if let Some(b) = lit_bool(t, "show") {
            o.insert("show_text".into(), json!(b));
        }
        if let Some(x) = lit_num(t, "fontSize") {
            o.insert("font_size".into(), number(x));
        }
        if let Some(c) = lit_color(t, "fontColor") {
            o.insert("font_color".into(), json!(c));
        }
        if let Some(a) = lit_text(t, "horizontalAlignment") {
            o.insert("align".into(), json!(a.to_ascii_lowercase()));
        }
    }
    if let Some(i) = get("icon") {
        if let Some(x) = lit_text(i, "shapeType") {
            o.insert("icon".into(), json!(x.chars().take(100).collect::<String>()));
        }
        if let Some(b) = lit_bool(i, "show") {
            o.insert("show_icon".into(), json!(b));
        }
        if let Some(p) = lit_text(i, "placement") {
            o.insert("icon_placement".into(), json!(p.to_ascii_lowercase()));
        }
        if let Some(c) = lit_color(i, "lineColor") {
            o.insert("icon_color".into(), json!(c));
        }
    }
    if let Some(f) = get("fill") {
        if let Some(b) = lit_bool(f, "show") {
            o.insert("show_fill".into(), json!(b));
        }
        if let Some(c) = lit_color(f, "fillColor") {
            o.insert("fill".into(), json!(c));
        }
        if let Some(x) = lit_num(f, "transparency") {
            o.insert("fill_transparency".into(), number(x.clamp(0.0, 100.0)));
        }
    }
    if let Some(f) = get("outline") {
        if let Some(b) = lit_bool(f, "show") {
            o.insert("show_outline".into(), json!(b));
        }
        if let Some(c) = lit_color(f, "lineColor") {
            o.insert("outline".into(), json!(c));
        }
        if let Some(x) = lit_num(f, "weight") {
            o.insert("outline_weight".into(), number(x.clamp(0.0, 20.0)));
        }
        if let Some(x) = lit_num(f, "transparency") {
            o.insert("outline_transparency".into(), number(x.clamp(0.0, 100.0)));
        }
    }
    effects(get("glow"), get("shadow"), &mut o);
    o
}

/// A shape's `text` entry: `{text, show_text?, font_color?, font_size?, bold?, italic?, align?,
/// valign?}` (the theme schema's `visual-shape` `text` object).
fn shape_text(t: &Map<String, Value>, o: &mut Map<String, Value>) {
    if let Some(x) = lit_text(t, "text") {
        o.insert("text".into(), json!(x.chars().take(1000).collect::<String>()));
    }
    if let Some(b) = lit_bool(t, "show") {
        o.insert("show_text".into(), json!(b));
    }
    if let Some(c) = lit_color(t, "fontColor") {
        o.insert("font_color".into(), json!(c));
    }
    if let Some(x) = lit_num(t, "fontSize") {
        o.insert("font_size".into(), number(x.clamp(1.0, 200.0)));
    }
    for k in ["bold", "italic"] {
        if let Some(b) = lit_bool(t, k) {
            o.insert(k.into(), json!(b));
        }
    }
    if let Some(a) = lit_text(t, "horizontalAlignment") {
        o.insert("align".into(), json!(a.to_ascii_lowercase()));
    }
    if let Some(a) = lit_text(t, "verticalAlignment") {
        o.insert("valign".into(), json!(a.to_ascii_lowercase()));
    }
}

/// A shape state's fill, outline and text overrides (`hover`, `press`): only what it sets.
fn shape_state(v: &Visual, state: &str) -> Map<String, Value> {
    let mut o = Map::new();
    if let Some(f) = exact_state(v, "fill", state) {
        if let Some(b) = lit_bool(f, "show") {
            o.insert("show_fill".into(), json!(b));
        }
        if let Some(c) = lit_color(f, "fillColor") {
            o.insert("fill".into(), json!(c));
        }
        if let Some(x) = lit_num(f, "transparency") {
            o.insert("fill_transparency".into(), number(x.clamp(0.0, 100.0)));
        }
    }
    if let Some(f) = exact_state(v, "outline", state) {
        if let Some(b) = lit_bool(f, "show") {
            o.insert("show_line".into(), json!(b));
        }
        if let Some(c) = lit_color(f, "lineColor") {
            o.insert("line".into(), json!(c));
        }
        if let Some(x) = lit_num(f, "weight") {
            o.insert("line_weight".into(), number(x.clamp(0.0, 100.0)));
        }
        if let Some(x) = lit_num(f, "transparency") {
            o.insert("line_transparency".into(), number(x.clamp(0.0, 100.0)));
        }
    }
    if let Some(t) = exact_state(v, "text", state) {
        let mut text = Map::new();
        shape_text(t, &mut text);
        if !text.is_empty() {
            o.insert("text".into(), Value::Object(text));
        }
    }
    o
}

/// The numeric adjustments of a shape's `shape` object (public theme schema, `visual-shape`).
const SHAPE_ADJUST: &[&str] = &[
    "arrowStemWidth",
    "arrowheadSize",
    "chevronAngle",
    "hexagonSlant",
    "isocelesTriangleTipPosition",
    "octagonSnipSize",
    "parallelogramSlant",
    "rectangleRoundedCurve",
    "roundEdge",
    "speechBubbleHeight",
    "speechBubbleTailAngle",
    "tabCutCornerSnipSizeBottom",
    "tabCutCornerSnipSizeTop",
    "tabCutCornerSnipSizeTopRight",
    "tabRoundCornerBottom",
    "tabRoundCornerTop",
    "tabRoundCornerTopRight",
    "trapezoidSlant",
];

/// The entry of `objects.<object>` whose selector is exactly the `default` state.
fn default_state<'a>(v: &'a Visual, object: &str) -> Option<&'a Map<String, Value>> {
    v.visual_extra
        .get("objects")?
        .get(object)?
        .as_array()?
        .iter()
        .find(|e| {
            e.get("selector")
                .and_then(Value::as_object)
                .is_some_and(|s| s.len() == 1 && s.get("id").and_then(Value::as_str) == Some("default"))
        })?
        .get("properties")?
        .as_object()
}

/// The tile shape of `objects.shape` properties `s` (a shape's, a navigator's buttons'):
/// `tile`, `adjust`, `tail` into `o`.
fn tile_shape(s: &Map<String, Value>, o: &mut Map<String, Value>) {
    if let Some(t) = lit_text(s, "tileShape") {
        o.insert("tile".into(), json!(t.chars().take(60).collect::<String>()));
    }
    let mut adjust = Map::new();
    for k in SHAPE_ADJUST {
        if let Some(n) = lit_num(s, k) {
            adjust.insert((*k).into(), number(n.clamp(-10_000.0, 10_000.0)));
        }
    }
    if !adjust.is_empty() {
        o.insert("adjust".into(), Value::Object(adjust));
    }
    if let Some(t) = lit_text(s, "speechBubbleTailPosition") {
        o.insert("tail".into(), json!(t.chars().take(30).collect::<String>()));
    }
}

/// Shape: `{tile?, adjust?: {<schema name>: n}, tail?, cap?, angle?, fill?, fill_transparency?,
/// show_fill?, line?, line_weight?, line_transparency?, show_line?}`, read where Desktop reads
/// them (`docs/formats/visual-options.md`, "State selectors"): the tile shape and the rotation
/// from the entries without selector, fill and outline from the `default` state (their `show`
/// also from the entry without selector).
fn shape(v: &Visual) -> Option<Value> {
    let mut o = Map::new();
    if let Some(s) = entry(v, "shape", None) {
        tile_shape(s, &mut o);
        if let Some(t) = lit_text(s, "linecapType") {
            o.insert("cap".into(), json!(t.chars().take(30).collect::<String>()));
        }
    }
    if let Some(r) = entry(v, "rotation", None)
        && let Some(a) = lit_num(r, "shapeAngle").or_else(|| lit_num(r, "angle"))
    {
        o.insert("angle".into(), number(a.rem_euclid(360.0)));
    }
    let plain_show = |object: &str| entry(v, object, None).and_then(|e| lit_bool(e, "show"));
    let fill = default_state(v, "fill");
    if let Some(b) = fill
        .and_then(|f| lit_bool(f, "show"))
        .or_else(|| plain_show("fill"))
    {
        o.insert("show_fill".into(), json!(b));
    }
    if let Some(f) = fill {
        if let Some(c) = lit_color(f, "fillColor") {
            o.insert("fill".into(), json!(c));
        }
        if let Some(x) = lit_num(f, "transparency") {
            o.insert("fill_transparency".into(), number(x.clamp(0.0, 100.0)));
        }
    }
    let line = default_state(v, "outline");
    if let Some(b) = line
        .and_then(|f| lit_bool(f, "show"))
        .or_else(|| plain_show("outline"))
    {
        o.insert("show_line".into(), json!(b));
    }
    if let Some(f) = line {
        if let Some(c) = lit_color(f, "lineColor") {
            o.insert("line".into(), json!(c));
        }
        if let Some(x) = lit_num(f, "weight") {
            o.insert("line_weight".into(), number(x.clamp(0.0, 100.0)));
        }
        if let Some(x) = lit_num(f, "transparency") {
            o.insert("line_transparency".into(), number(x.clamp(0.0, 100.0)));
        }
    }
    // the text inside the shape (Format > Style > Text), from the default state, else the entry
    // without selector
    if let Some(t) = default_state(v, "text").or_else(|| entry(v, "text", None)) {
        let mut text = Map::new();
        shape_text(t, &mut text);
        if !text.is_empty() {
            o.insert("text".into(), Value::Object(text));
        }
    }
    let mut states = Map::new();
    for state in ["hover", "press"] {
        let st = shape_state(v, state);
        if !st.is_empty() {
            states.insert(state.into(), Value::Object(st));
        }
    }
    if !states.is_empty() {
        o.insert("states".into(), Value::Object(states));
    }
    (!o.is_empty()).then_some(Value::Object(o))
}

/// Slicer: `{mode, selected?: [values], inverted?: true, range?: {from?, to?}}`.
fn slicer(v: &Visual) -> Value {
    let ty = v.visual_type.as_deref().unwrap_or("").to_ascii_lowercase();
    let data = entry(v, "data", None);
    let mode = match ty.as_str() {
        "listslicer" => "list".to_string(),
        "advancedslicervisual" | "buttonslicer" => "tile".to_string(),
        "textslicer" => "text".to_string(),
        _ => {
            let m = data.and_then(|d| lit_text(d, "mode")).unwrap_or_default();
            let horizontal = entry(v, "general", None)
                .and_then(|g| lit_num(g, "orientation"))
                .is_some_and(|x| x == 1.0);
            match m.to_ascii_lowercase().as_str() {
                "dropdown" => "dropdown".to_string(),
                "between" => "between".to_string(),
                "before" => "before".to_string(),
                "after" => "after".to_string(),
                "relative" => "relative".to_string(),
                // a what-if parameter's single-value slider (docs/formats/parameters.md)
                "single" => "single".to_string(),
                "basic" | "" if horizontal => "tile".to_string(),
                _ => "list".to_string(),
            }
        }
    };
    let mut o = Map::new();
    o.insert("mode".into(), json!(mode));
    if let Some(sel) = filters::selection_json(v)
        && let Some(m) = sel.as_object()
    {
        o.extend(m.clone());
    }
    if let Some(sel) = entry(v, "selection", None) {
        if let Some(b) = lit_bool(sel, "singleSelect") {
            o.insert("single_select".into(), json!(b));
        }
        if let Some(b) = lit_bool(sel, "selectAllCheckboxEnabled") {
            o.insert("select_all".into(), json!(b));
        }
    }
    slicer_display(v, data, &mut o);
    Value::Object(o)
}

/// What Desktop displays beyond the filter: the range bounds stored in `data`
/// (`numericStart`/`numericEnd`, `startDate`/`endDate` win over the filter's), the relative-date
/// settings, the search box and the header switch.
fn slicer_display(v: &Visual, data: Option<&Map<String, Value>>, o: &mut Map<String, Value>) {
    if let Some(d) = data {
        let mut range = o
            .get("range")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let mut shown = false;
        for (key, num, date) in [
            ("from", "numericStart", "startDate"),
            ("to", "numericEnd", "endDate"),
        ] {
            let value = match lit_num(d, num) {
                Some(x) => Some(number(x)),
                None => lit(d, date)
                    .map(literal_json)
                    .filter(|x| x.is_string() || x.is_number()),
            };
            if let Some(x) = value {
                range.insert(key.into(), x);
                range.remove(&format!("{key}_exclusive"));
                shown = true;
            }
        }
        if !range.is_empty() && !shown {
            // Desktop shows the column's whole domain for a range held by the filter only
            range.insert("filter_only".into(), json!(true));
        }
        if !range.is_empty() {
            o.insert("range".into(), Value::Object(range));
        }
        let mut rel = Map::new();
        if let Some(r) = lit_text(d, "relativeRange") {
            rel.insert("range".into(), json!(r.to_ascii_lowercase()));
        }
        if let Some(x) = lit_num(d, "relativeDuration") {
            rel.insert("duration".into(), number(x));
        }
        for (key, prop) in [
            ("period", "relativePeriod"),
            ("time_period", "relativeTimePeriod"),
        ] {
            if let Some(p) = lit_text(d, prop).filter(|p| !p.eq_ignore_ascii_case("none")) {
                rel.insert(
                    key.into(),
                    json!(p.to_ascii_lowercase().chars().take(100).collect::<String>()),
                );
            }
        }
        // the days the settings cover (today included), shown under the inputs
        if let (Some(r), Some(p)) = (
            rel.get("range").and_then(Value::as_str),
            rel.get("period").and_then(Value::as_str),
        ) {
            let n = rel.get("duration").and_then(Value::as_f64).unwrap_or(1.0);
            let today = filters::today();
            if let Some((a, b)) = filters::relative_days(r, n, p, today) {
                rel.insert("from".into(), json!(filters::iso_day(a)));
                rel.insert("to".into(), json!(filters::iso_day(b)));
            }
        }
        if !rel.is_empty() {
            o.insert("relative".into(), Value::Object(rel));
        }
    }
    if let Some(b) = entry(v, "general", None).and_then(|g| lit_bool(g, "selfFilterEnabled")) {
        o.insert("search_box".into(), json!(b));
    }
    if let Some(b) = entry(v, "header", None).and_then(|h| lit_bool(h, "show")) {
        o.insert("header_show".into(), json!(b));
    }
    if let Some(t) = entry(v, "header", None).and_then(|h| lit_text(h, "text")) {
        o.insert(
            "header_text".into(),
            json!(t.chars().take(1000).collect::<String>()),
        );
    }
}

/// Adds `slicer.domain: {min, max}` to a range slicer (`between` / `before` / `after`): the
/// exact bounds of its column's stored data (every row, not the preview's capped rows).
pub(super) fn add_domain(o: &mut Map<String, Value>, v: &Visual, s: &crate::Session) {
    let Some(sl) = o.get_mut("slicer").and_then(Value::as_object_mut) else {
        return;
    };
    // a classic slicer on a date column written without a mode: Desktop draws the between
    // range (Power BI Desktop 2.159.1231.0, 2026-11-24)
    if sl.get("mode").and_then(Value::as_str) == Some("list")
        && v.visual_type
            .as_deref()
            .is_some_and(|t| t.eq_ignore_ascii_case("slicer"))
        && entry(v, "data", None).and_then(|d| lit_text(d, "mode")).is_none()
        && entry(v, "general", None).and_then(|g| lit_num(g, "orientation")) != Some(1.0)
        && let Some(FieldRef::Column { table, column }) = v.fields.first().map(|b| &b.field)
        && s.model
            .as_ref()
            .and_then(|m| m.table(table)?.column(column))
            .is_some_and(|c| c.data_type == bicraft_model::DataType::DateTime)
    {
        sl.insert("mode".into(), json!("between"));
    }
    let Some(FieldRef::Column { table, column }) = v.fields.first().map(|b| &b.field) else {
        return;
    };
    let what_if = s.model.as_ref().filter(|m| {
        m.table(table)
            .and_then(|t| t.column(column))
            .is_some_and(crate::parameters::is_what_if_column)
    });
    // a classic slicer on a what-if parameter's column written without a mode (Desktop writes
    // none for its own): Desktop draws the single-value slider (docs/formats/parameters.md)
    if let Some(m) = what_if {
        if sl.get("mode").and_then(Value::as_str) == Some("list")
            && v.visual_type
                .as_deref()
                .is_some_and(|t| t.eq_ignore_ascii_case("slicer"))
            && entry(v, "data", None).and_then(|d| lit_text(d, "mode")).is_none()
        {
            sl.insert("mode".into(), json!("single"));
        }
        if sl.get("mode").and_then(Value::as_str) == Some("single")
            && let Some(d) = crate::parameters::what_if_default(m, table, column)
        {
            sl.insert("default".into(), number(d));
        }
    }
    let ranged = matches!(
        sl.get("mode").and_then(Value::as_str),
        Some("between" | "before" | "after" | "single")
    );
    if !ranged {
        return;
    }
    if let Some(d) = crate::preview::column_domain(s, table, column) {
        sl.insert("domain".into(), d);
    }
}

/// `queryRef` of each projection, in `Visual::fields` order (the default ones when the visual
/// has no query yet).
fn query_refs(v: &Visual) -> Vec<Option<String>> {
    let mut out = Vec::new();
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
                out.push(p.get("queryRef").and_then(Value::as_str).map(str::to_string));
            }
        }
    }
    if out.len() != v.fields.len() {
        out = v
            .fields
            .iter()
            .map(|b| bicraft_report::query::default_query_ref(&b.field))
            .collect();
    }
    out
}

fn column_of_ref(refs: &[Option<String>], r: &str) -> Option<usize> {
    refs.iter().position(|x| x.as_deref() == Some(r))
}

/// A `{Column: …}` expression → `(entity, property)`.
fn column_expr(e: &Value) -> Option<(String, String)> {
    let c = e.get("Column")?;
    let entity = c.get("Expression")?.get("SourceRef")?.get("Entity")?.as_str()?;
    Some((entity.to_string(), c.get("Property")?.as_str()?.to_string()))
}

/// A literal as JSON (text unquoted, numbers, booleans, dates as ISO text).
fn literal_json(text: &str) -> Value {
    match crate::visual_props::parse_lit(text) {
        crate::visual_props::Lit::Null => Value::Null,
        crate::visual_props::Lit::Bool(b) => json!(b),
        crate::visual_props::Lit::Num(n, _) => number(n),
        crate::visual_props::Lit::Text(t) => json!(t),
        crate::visual_props::Lit::Date(d) => json!(d.strip_suffix("T00:00:00").unwrap_or(&d)),
        crate::visual_props::Lit::Other(o) => json!(o),
    }
}

/// Explicit data colors: `[{column, color}]` for a series (`selector.metadata`) and
/// `[{field, value, column?, color}]` for one category or legend value (`selector.data`).
fn series_colors(v: &Visual) -> Vec<Value> {
    let refs = query_refs(v);
    let mut out = Vec::new();
    let list = v
        .visual_extra
        .get("objects")
        .and_then(|o| o.get("dataPoint"))
        .and_then(Value::as_array);
    for e in list.into_iter().flatten() {
        if out.len() >= MAX_COLORS {
            break;
        }
        let Some(props) = e.get("properties").and_then(Value::as_object) else {
            continue;
        };
        let Some(color) = lit_color(props, "fill") else {
            continue;
        };
        let Some(sel) = e.get("selector").filter(|x| !x.is_null()) else {
            // the default color of every data point (a single-series chart's color)
            out.push(json!({"color": color}));
            continue;
        };
        if let Some(r) = sel.get("metadata").and_then(Value::as_str)
            && sel.get("data").is_none()
        {
            if let Some(c) = column_of_ref(&refs, r) {
                out.push(json!({"column": c, "color": color}));
            }
            continue;
        }
        let Some(cmp) = sel
            .get("data")
            .and_then(Value::as_array)
            .and_then(|d| d.first())
            .and_then(|d| d.get("scopeId"))
            .and_then(|s| s.get("Comparison"))
        else {
            continue;
        };
        let (Some((t, c)), Some(lit)) = (
            cmp.get("Left").and_then(column_expr),
            cmp.get("Right")
                .and_then(|r| r.get("Literal"))
                .and_then(|l| l.get("Value"))
                .and_then(Value::as_str),
        ) else {
            continue;
        };
        let field = FieldRef::Column { table: t, column: c };
        let mut item = Map::new();
        item.insert("field".into(), json!(field_text(&field)));
        if let Some(i) = v.fields.iter().position(|b| same_field(&b.field, &field)) {
            item.insert("column".into(), json!(i));
        }
        item.insert("value".into(), literal_json(lit));
        item.insert("color".into(), json!(color));
        out.push(Value::Object(item));
    }
    out
}

/// Table/matrix per-column styles (`values` entries with a `metadata` selector):
/// `[{column, bg?, fg?}]`.
fn column_styles(v: &Visual) -> Vec<Value> {
    let refs = query_refs(v);
    let mut out = Vec::new();
    let list = v
        .visual_extra
        .get("objects")
        .and_then(|o| o.get("values"))
        .and_then(Value::as_array);
    for e in list.into_iter().flatten() {
        let Some(r) = e
            .get("selector")
            .and_then(|s| s.get("metadata"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let (Some(props), Some(c)) = (
            e.get("properties").and_then(Value::as_object),
            column_of_ref(&refs, r),
        ) else {
            continue;
        };
        let mut item = Map::new();
        if let Some(x) = lit_color(props, "backColor") {
            item.insert("bg".into(), json!(x));
        }
        if let Some(x) = lit_color(props, "fontColor") {
            item.insert("fg".into(), json!(x));
        }
        if item.is_empty() {
            continue;
        }
        item.insert("column".into(), json!(c));
        out.push(Value::Object(item));
        if out.len() >= MAX_COLORS {
            break;
        }
    }
    out
}

/// The visible column a field text names (`[Total]`, `Sum(T[C])`…).
fn column_of_text(v: &Visual, text: &str, model: Option<&SemanticModel>) -> Option<usize> {
    let f = resolve_field(model, text).ok()?.field;
    v.fields.iter().position(|b| same_field(&b.field, &f))
}

/// Analytics lines (`props.analytics`): `[{kind, axis, value?, column?, percentile?, color?,
/// style?, width?, transparency?, label?, data_label?, position?}]`, error bars' options,
/// symmetry shading's colours and anomaly detection's options; hidden and unreadable lines are
/// left out.
fn reference_lines(v: &Visual, props: &Value, model: Option<&SemanticModel>) -> Vec<Value> {
    let mut out = Vec::new();
    for l in props
        .get("analytics")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(MAX_LINES)
    {
        let Some(o) = l.as_object() else {
            continue;
        };
        let Some(kind) = o.get("kind").and_then(Value::as_str) else {
            continue;
        };
        if o.get("show") == Some(&Value::Bool(false)) || kind == "forecast" {
            continue;
        }
        let mut item = Map::new();
        item.insert("kind".into(), json!(kind));
        item.insert(
            "axis".into(),
            json!(o.get("axis").and_then(Value::as_str).unwrap_or("y")),
        );
        for k in [
            "value",
            "percentile",
            "color",
            "style",
            "width",
            "label",
            "data_label",
            "position",
            "transparency",
            // error bars (their bounds' values are in `data.error_bars`)
            "relative",
            "bar",
            "match_series",
            "marker",
            "marker_shape",
            "band",
            "band_style",
            "band_color",
            // symmetry shading
            "upper_color",
            "lower_color",
            // anomaly detection (its values are in `data.anomalies`)
            "sensitivity",
            "marker_size",
            "marker_color",
            "marker_transparency",
        ] {
            if let Some(x) = o.get(k) {
                let x = match x.as_f64() {
                    Some(n) if x.is_f64() => number(n),
                    _ => x.clone(),
                };
                item.insert(k.into(), x);
            }
        }
        if let Some(m) = o.get("measure").and_then(Value::as_str) {
            match column_of_text(v, m, model) {
                Some(c) => {
                    item.insert("column".into(), json!(c));
                }
                None => continue,
            }
        }
        out.push(Value::Object(item));
    }
    out
}

/// `{column, rows?, columns?}`: the data column whose values split the chart.
fn small_multiples(v: &Visual, props: &Value, model: Option<&SemanticModel>) -> Option<Value> {
    let sm = props.get("small_multiples")?.as_object()?;
    let column = column_of_text(v, sm.get("field")?.as_str()?, model)?;
    let mut o = Map::new();
    o.insert("column".into(), json!(column));
    for k in ["rows", "columns"] {
        if let Some(x) = sm.get(k) {
            o.insert(k.into(), x.clone());
        }
    }
    Some(Value::Object(o))
}

/// A page or bookmark navigator's options (`navigator`): `orientation` (`objects.layout`:
/// `horizontal`, `vertical`, `grid`; null when not set), `rows` and `columns` of a grid,
/// `bookmark_group` and `selected_bookmark` (`objects.bookmarks`), `show_hidden_pages` and
/// `show_tooltip_pages` (`objects.pages`). Property names and values: the report theme schema
/// (`crates/visuals/schemas/`).
fn navigator(v: &Visual) -> Value {
    let layout = entry(v, "layout", None);
    let orientation = layout
        .and_then(|p| lit_num(p, "orientation"))
        .and_then(|n| match n as i64 {
            0 => Some("grid"),
            1 => Some("vertical"),
            2 => Some("horizontal"),
            _ => None,
        });
    let count = |k: &str| {
        layout
            .and_then(|p| lit_num(p, k))
            .filter(|n| *n >= 1.0)
            .map(|n| n.min(100.0) as u64)
    };
    let bm = entry(v, "bookmarks", None);
    let text = |k: &str| bm.and_then(|p| lit_text(p, k)).filter(|s| !s.is_empty());
    let pages = entry(v, "pages", None);
    let flag = |k: &str| pages.and_then(|p| lit_bool(p, k)).unwrap_or(false);
    let padding = layout
        .and_then(|p| lit_num(p, "cellPadding"))
        .filter(|n| *n >= 0.0)
        .map(|n| number(n.min(100.0)));
    let mut styles = Map::new();
    for state in ["default", "selected", "hover", "press"] {
        let s = nav_style(v, state);
        if !s.is_empty() {
            styles.insert(state.into(), Value::Object(s));
        }
    }
    // the buttons' shape (`objects.shape`, one for every state)
    let mut shape = Map::new();
    if let Some(s) = entry(v, "shape", None) {
        tile_shape(s, &mut shape);
    }
    json!({
        "orientation": orientation,
        "shape": (!shape.is_empty()).then_some(Value::Object(shape)),
        "rows": count("rowCount"),
        "columns": count("columnCount"),
        "padding": padding,
        "styles": styles,
        "bookmark_group": text("bookmarkGroup"),
        "selected_bookmark": text("selectedBookmark"),
        "allow_deselection": bm.and_then(|p| lit_bool(p, "allowDeselectionBookmark")).unwrap_or(false),
        "deselection_bookmark": text("deselectionBookmark"),
        "hide_deselection_bookmark": bm.and_then(|p| lit_bool(p, "hideDeselectedBookmark")).unwrap_or(false),
        "show_hidden_pages": flag("showHiddenPages"),
        "show_tooltip_pages": flag("showTooltipPages"),
    })
}

/// The entry of `objects.<object>` for button state `state` (`selector.id`); for `"default"`,
/// also the entry without a selector.
fn state_entry<'a>(v: &'a Visual, object: &str, state: &str) -> Option<&'a Map<String, Value>> {
    let list = v.visual_extra.get("objects")?.get(object)?.as_array()?;
    let id = |e: &Value| {
        e.get("selector")
            .and_then(|x| x.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    list.iter()
        .take(64)
        .find(|e| id(e).as_deref() == Some(state))
        .or_else(|| {
            (state == "default")
                .then(|| {
                    list.iter()
                        .take(64)
                        .find(|e| e.get("selector").is_none_or(Value::is_null))
                })
                .flatten()
        })
        .and_then(|e| e.get("properties"))
        .and_then(Value::as_object)
}

/// A navigator button style of one state (the report theme schema's `bookmarkNavigator` /
/// `pageNavigator` cards `fill`, `outline`, `text`, `glow`, `shadow`, `accentBar`, each with
/// `$id` `default`,
/// `hover`, `selected`, `disabled`): only the properties the visual sets.
fn nav_style(v: &Visual, state: &str) -> Map<String, Value> {
    let mut o = Map::new();
    if let Some(f) = state_entry(v, "fill", state) {
        if let Some(b) = lit_bool(f, "show") {
            o.insert("show_fill".into(), json!(b));
        }
        if let Some(c) = lit_color(f, "fillColor") {
            o.insert("fill".into(), json!(c));
        }
        if let Some(x) = lit_num(f, "transparency") {
            o.insert("fill_transparency".into(), number(x.clamp(0.0, 100.0)));
        }
    }
    if let Some(f) = state_entry(v, "outline", state) {
        if let Some(b) = lit_bool(f, "show") {
            o.insert("show_outline".into(), json!(b));
        }
        if let Some(c) = lit_color(f, "lineColor") {
            o.insert("outline".into(), json!(c));
        }
        if let Some(x) = lit_num(f, "weight") {
            o.insert("outline_weight".into(), number(x.clamp(0.0, 20.0)));
        }
        if let Some(x) = lit_num(f, "transparency") {
            o.insert("outline_transparency".into(), number(x.clamp(0.0, 100.0)));
        }
    }
    if let Some(t) = state_entry(v, "text", state) {
        if let Some(b) = lit_bool(t, "show") {
            o.insert("show_text".into(), json!(b));
        }
        if let Some(c) = lit_color(t, "fontColor") {
            o.insert("font_color".into(), json!(c));
        }
        if let Some(x) = lit_num(t, "fontSize") {
            o.insert("font_size".into(), number(x.clamp(1.0, 200.0)));
        }
        if let Some(b) = lit_bool(t, "bold") {
            o.insert("bold".into(), json!(b));
        }
        if let Some(b) = lit_bool(t, "italic") {
            o.insert("italic".into(), json!(b));
        }
    }
    effects(
        state_entry(v, "glow", state),
        state_entry(v, "shadow", state),
        &mut o,
    );
    if let Some(a) = state_entry(v, "accentBar", state) {
        if let Some(b) = lit_bool(a, "show") {
            o.insert("show_accent".into(), json!(b));
        }
        if let Some(c) = lit_color(a, "color") {
            o.insert("accent".into(), json!(c));
        }
        if let Some(p) = lit_text(a, "position") {
            o.insert("accent_position".into(), json!(p.to_ascii_lowercase()));
        }
        if let Some(x) = lit_num(a, "width") {
            o.insert("accent_width".into(), number(x.clamp(0.0, 100.0)));
        }
    }
    o
}

/// The `glow` and `shadow` properties `glow` / `shadow` of one state (report theme schema
/// `visual-actionButton`, `visual-pageNavigator`, `visual-bookmarkNavigator`: Shape glow, Shape
/// shadow) into `o`: `show_glow`, `glow`, `glow_blur`, `glow_transparency`, `show_shadow`,
/// `shadow`, `shadow_blur`, `shadow_distance`, `shadow_angle`, `shadow_transparency`,
/// `shadow_position`.
fn effects(
    glow: Option<&Map<String, Value>>,
    shadow: Option<&Map<String, Value>>,
    o: &mut Map<String, Value>,
) {
    if let Some(g) = glow {
        if let Some(b) = lit_bool(g, "show") {
            o.insert("show_glow".into(), json!(b));
        }
        if let Some(c) = lit_color(g, "color") {
            o.insert("glow".into(), json!(c));
        }
        if let Some(x) = lit_num(g, "shadowBlur") {
            o.insert("glow_blur".into(), number(x.clamp(0.0, 100.0)));
        }
        if let Some(x) = lit_num(g, "transparency") {
            o.insert("glow_transparency".into(), number(x.clamp(0.0, 100.0)));
        }
    }
    if let Some(s) = shadow {
        if let Some(b) = lit_bool(s, "show") {
            o.insert("show_shadow".into(), json!(b));
        }
        if let Some(c) = lit_color(s, "color") {
            o.insert("shadow".into(), json!(c));
        }
        for (key, prop, max) in [
            ("shadow_blur", "shadowBlur", 100.0),
            ("shadow_distance", "shadowDistance", 100.0),
            ("shadow_angle", "angle", 360.0),
            ("shadow_transparency", "transparency", 100.0),
        ] {
            if let Some(x) = lit_num(s, prop) {
                o.insert(key.into(), number(x.clamp(0.0, max)));
            }
        }
        if let Some(p) = lit_text(s, "shadowPositionPreset") {
            o.insert(
                "shadow_position".into(),
                json!(p.chars().take(30).collect::<String>().to_ascii_lowercase()),
            );
        }
    }
}
