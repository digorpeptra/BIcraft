//! Theme and formatting for the live-viewer snapshot (M8.2, `docs/view-snapshot.md`).
//!
//! - [`resolve`]: the report's theme — built-in defaults for the base theme name, overlaid with the
//!   base theme JSON found in the report resources (Desktop embeds it under
//!   `StaticResources/SharedResources/BaseThemes/`), overlaid with the custom theme JSON
//!   (`StaticResources/RegisteredResources/`).
//! - [`page_background`]: the page `background` object, else the theme's page style.
//! - [`visual_format`]: title, background, border, data labels and legend, from the theme's visual
//!   styles (`*` then the visual type) overlaid with the visual's `objects` /
//!   `visualContainerObjects`.
//!
//! The built-in palettes are the publicly documented default Power BI theme values; no theme file
//! is shipped.

use bicraft_report::{Page, Report, Resource, Visual};
use serde_json::{Map, Value, json};

#[path = "theme_format.rs"]
mod render_keys;
pub(crate) use render_keys::series_format;

/// Largest theme document parsed (themes are tens of kilobytes; Desktop's base themes ~100 KB).
const MAX_THEME_BYTES: usize = 8 << 20;
/// Most data colors carried.
const MAX_DATA_COLORS: usize = 64;

/// The resolved report theme.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Theme {
    pub name: String,
    pub data_colors: Vec<String>,
    pub background: String,
    pub foreground: String,
    pub table_accent: String,
    pub font_family: String,
    pub font_size: f64,
    /// The sentiment and divergent colours (`good`, `neutral`, `bad`, `maximum`, `center`,
    /// `minimum`) the themes set.
    named: Map<String, Value>,
    /// `textClasses` of the base then the custom theme, merged per property.
    text_classes: Map<String, Value>,
    /// `visualStyles` of the base then the custom theme (later entries win).
    styles: Vec<Value>,
    /// A custom theme without a base theme: Desktop keeps the classic look under it (light grey
    /// regular titles; Power BI Desktop 2.159.1231.0, 2026-11-24).
    classic: bool,
}

/// The named colours [`Theme`] keeps (theme keys).
const NAMED_COLORS: [&str; 6] = ["good", "neutral", "bad", "maximum", "center", "minimum"];
/// Text classes the snapshot lists (those Customize theme edits).
const SNAPSHOT_TEXT_CLASSES: [&str; 4] = ["label", "title", "callout", "header"];
/// Text class properties the snapshot lists: theme key → snapshot key.
const TEXT_CLASS_PROPS: [(&str, &str); 4] = [
    ("fontFace", "font_face"),
    ("fontSize", "font_size"),
    ("color", "color"),
    ("fontWeight", "font_weight"),
];
/// Theme style objects the snapshot lists (those Customize theme edits): (visual type, object,
/// `$id` state).
const SNAPSHOT_STYLES: [(&str, &str, Option<&str>); 9] = [
    ("*", "background", None),
    ("*", "border", None),
    ("*", "visualHeader", None),
    ("*", "visualTooltip", None),
    ("page", "outspace", None),
    ("page", "background", None),
    ("page", "outspacePane", None),
    ("page", "filterCard", Some("Available")),
    ("page", "filterCard", Some("Applied")),
];

/// The default Power BI theme since 2020 (`CY20SU…` and later base themes).
const MODERN: [&str; 20] = [
    "#118DFF", "#12239E", "#E66C37", "#6B007B", "#E044A7", "#744EC2", "#D9B300", "#D64550", "#197278",
    "#1AAB40", "#15C6F4", "#4092FF", "#FFA058", "#BE5DC9", "#F472D0", "#B5A1FF", "#C4A200", "#FF8080",
    "#00DBBC", "#5BD667",
];

/// The classic Power BI theme (2018–2019 base themes).
const CLASSIC: [&str; 10] = [
    "#01B8AA", "#374649", "#FD625E", "#F2C80F", "#5F6B6D", "#8AD4EB", "#FE9666", "#A66999", "#3599B8",
    "#DFBFBF",
];

/// Built-in values for a base theme name (the default palette when the name is unknown).
fn builtin(name: &str) -> Theme {
    let lower = name.to_ascii_lowercase();
    let classic = lower.starts_with("cy18") || lower.starts_with("cy19") || lower.contains("classic");
    let (colors, foreground): (&[&str], &str) = if classic {
        (&CLASSIC, "#333333")
    } else {
        (&MODERN, "#252423")
    };
    Theme {
        name: name.to_string(),
        data_colors: colors.iter().map(|c| (*c).to_string()).collect(),
        background: "#FFFFFF".into(),
        foreground: foreground.into(),
        table_accent: colors.first().copied().unwrap_or("#118DFF").into(),
        font_family: "Segoe UI".into(),
        font_size: 10.0,
        named: Map::new(),
        text_classes: Map::new(),
        styles: Vec::new(),
        classic: false,
    }
}

/// `themeCollection.<key>.name`.
fn theme_ref<'a>(r: &'a Report, key: &str) -> Option<&'a str> {
    r.theme_collection
        .as_ref()?
        .get(key)?
        .get("name")?
        .as_str()
        .filter(|n| !n.is_empty())
}

/// The resource holding theme `name`: a file named `name` or `name.json`, preferring one under
/// `folder` (`SharedResources` / `RegisteredResources`).
fn find_resource<'a>(r: &'a Report, name: &str, folder: &str) -> Option<&'a Resource> {
    let wanted = name.to_ascii_lowercase();
    let with_json = format!("{wanted}.json");
    let matches = |res: &&Resource| {
        let file = res
            .path
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(&res.path)
            .to_ascii_lowercase();
        file == wanted || file == with_json
    };
    let folder = folder.to_ascii_lowercase();
    r.resources
        .iter()
        .filter(matches)
        .find(|res| res.path.to_ascii_lowercase().contains(&folder))
        .or_else(|| r.resources.iter().find(matches))
}

fn parse_theme(bytes: &[u8]) -> Option<Value> {
    if bytes.len() > MAX_THEME_BYTES {
        return None;
    }
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let v: Value = serde_json::from_slice(bytes).ok()?;
    v.is_object().then_some(v)
}

fn color_text(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 64)
        .map(str::to_string)
}

/// Overlays a theme document on `t`.
fn overlay(t: &mut Theme, doc: &Value) {
    if let Some(name) = doc.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()) {
        t.name = name.to_string();
    }
    if let Some(colors) = doc.get("dataColors").and_then(Value::as_array) {
        let colors: Vec<String> = colors
            .iter()
            .filter_map(|c| color_text(Some(c)))
            .take(MAX_DATA_COLORS)
            .collect();
        if !colors.is_empty() {
            t.data_colors = colors;
        }
    }
    if let Some(c) = color_text(doc.get("background")) {
        t.background = c;
    }
    if let Some(c) = color_text(doc.get("foreground")) {
        t.foreground = c;
    }
    if let Some(c) = color_text(doc.get("tableAccent")) {
        t.table_accent = c;
    }
    for key in NAMED_COLORS {
        if let Some(c) = color_text(doc.get(key)) {
            t.named.insert(key.into(), json!(c));
        }
    }
    if let Some(classes) = doc.get("textClasses").and_then(Value::as_object) {
        for class in SNAPSHOT_TEXT_CLASSES {
            let Some(props) = classes.get(class).and_then(Value::as_object) else {
                continue;
            };
            let merged = t
                .text_classes
                .entry(class.to_string())
                .or_insert_with(|| json!({}));
            if let Value::Object(m) = merged {
                for (key, out) in TEXT_CLASS_PROPS {
                    let v = match (key, props.get(key)) {
                        ("fontFace", Some(Value::String(s))) => first_font(s).map(Value::String),
                        ("fontSize", Some(Value::Number(n))) => n
                            .as_f64()
                            .filter(|s| s.is_finite() && *s > 0.0 && *s < 1000.0)
                            .map(number),
                        ("color", v) => color_text(v).map(Value::String),
                        ("fontWeight", Some(Value::String(s))) if s.len() <= 64 => Some(json!(s)),
                        _ => None,
                    };
                    if let Some(v) = v {
                        m.insert(out.into(), v);
                    }
                }
            }
        }
        t.text_classes
            .retain(|_, v| v.as_object().is_some_and(|m| !m.is_empty()));
    }
    let label = doc.get("textClasses").and_then(|c| c.get("label"));
    if let Some(face) = label
        .and_then(|l| l.get("fontFace"))
        .and_then(Value::as_str)
        .and_then(first_font)
    {
        t.font_family = face;
    }
    if let Some(size) = label
        .and_then(|l| l.get("fontSize"))
        .and_then(Value::as_f64)
        .filter(|s| s.is_finite() && *s > 0.0 && *s < 1000.0)
    {
        t.font_size = size;
    }
    if let Some(styles) = doc.get("visualStyles").filter(|s| s.is_object()) {
        t.styles.push(styles.clone());
    }
}

/// The first family of a CSS font list (`'Segoe UI', wf_segoe-ui_normal, …` → `Segoe UI`).
fn first_font(list: &str) -> Option<String> {
    let first = list.split(',').next()?.trim().trim_matches(['\'', '"']).trim();
    (!first.is_empty() && first.len() <= 128).then(|| first.to_string())
}

/// The report's resolved theme; `None` when the report names no theme.
pub(crate) fn resolve(r: &Report) -> Option<Theme> {
    let base = theme_ref(r, "baseTheme");
    let custom = theme_ref(r, "customTheme");
    if base.is_none() && custom.is_none() {
        return None;
    }
    let mut t = builtin(base.unwrap_or("default"));
    t.classic = base.is_none();
    if let Some(doc) = base
        .and_then(|b| find_resource(r, b, "SharedResources"))
        .and_then(|res| parse_theme(&res.bytes))
    {
        overlay(&mut t, &doc);
    }
    if let Some(doc) = custom
        .and_then(|c| find_resource(r, c, "RegisteredResources"))
        .and_then(|res| parse_theme(&res.bytes))
    {
        overlay(&mut t, &doc);
    }
    Some(t)
}

pub(crate) fn theme_json(t: Option<&Theme>) -> Value {
    let Some(t) = t else {
        return Value::Null;
    };
    let mut o = Map::new();
    o.insert("name".into(), json!(t.name));
    o.insert("data_colors".into(), json!(t.data_colors));
    o.insert("background".into(), json!(t.background));
    o.insert("foreground".into(), json!(t.foreground));
    o.insert("table_accent".into(), json!(t.table_accent));
    o.insert("font_family".into(), json!(t.font_family));
    o.insert("font_size".into(), number(t.font_size));
    if t.classic {
        o.insert("classic".into(), Value::Bool(true));
    }
    for (k, v) in &t.named {
        o.insert(k.clone(), v.clone());
    }
    if !t.text_classes.is_empty() {
        o.insert("text_classes".into(), Value::Object(t.text_classes.clone()));
    }
    let styles = style_defaults(t);
    if !styles.is_empty() {
        o.insert("styles".into(), Value::Object(styles));
    }
    Value::Object(o)
}

/// The theme's values of the style objects Customize theme edits ([`SNAPSHOT_STYLES`]): visual
/// type → object (`object.State` for a `$id` entry) → property → plain value, base then custom
/// theme, later entries winning; a state's entry starts from the stateless ones.
fn style_defaults(t: &Theme) -> Map<String, Value> {
    let mut out = Map::new();
    for (vt, object, state) in SNAPSHOT_STYLES {
        // the Filters pane objects sit under `page` in the public schema, under `*` in Microsoft
        // Learn's example (as `theme_object_entries` reads them)
        let keys: &[&str] = if matches!(object, "outspacePane" | "filterCard") {
            &["*", "page"]
        } else {
            &[vt]
        };
        let mut entries: Vec<&Value> = Vec::new();
        for styles in &t.styles {
            for key in keys {
                if let Some(list) = styles
                    .get(*key)
                    .and_then(|s| s.get("*"))
                    .and_then(|s| s.get(object))
                    .and_then(Value::as_array)
                {
                    entries.extend(list.iter().take(64).filter(|e| e.is_object()));
                }
            }
        }
        let id = |e: &Value| e.get("$id").and_then(Value::as_str).map(str::to_string);
        let mut ordered: Vec<&Value> = entries.iter().copied().filter(|e| id(e).is_none()).collect();
        if let Some(st) = state {
            ordered.extend(
                entries
                    .iter()
                    .copied()
                    .filter(|e| id(e).is_some_and(|x| x.eq_ignore_ascii_case(st))),
            );
        }
        let mut props = Map::new();
        for e in ordered {
            let Some(m) = e.as_object() else { continue };
            for name in m.keys().filter(|k| *k != "$id").take(64) {
                if let Some(v) = entry_value(e, name, true, Some(t)) {
                    props.insert(name.clone(), v);
                }
            }
        }
        if props.is_empty() {
            continue;
        }
        let key = match state {
            Some(st) => format!("{object}.{st}"),
            None => object.to_string(),
        };
        if let Value::Object(by_type) = out.entry(vt.to_string()).or_insert_with(|| json!({})) {
            by_type.insert(key, Value::Object(props));
        }
    }
    out
}

/// A float as a JSON number, integral values without a fraction.
fn number(x: f64) -> Value {
    if x.is_finite() && x.fract() == 0.0 && x.abs() < 9.0e15 {
        json!(x as i64)
    } else if x.is_finite() {
        json!(x)
    } else {
        Value::Null
    }
}

// ---------------------------------------------------------------------------------------------
// property values
// ---------------------------------------------------------------------------------------------

/// A property value, from a visual (`{"expr": {"Literal": {"Value": "14D"}}}`) or a theme style
/// (plain JSON).
#[derive(Debug, Clone, PartialEq)]
enum Prop {
    Bool(bool),
    Num(f64),
    Text(String),
    Color(String),
}

/// A PBIR literal (`true`, `14D`, `12L`, `'left'`, `datetime'…'`).
fn literal(text: &str) -> Option<Prop> {
    let t = text.trim();
    match t {
        "true" => return Some(Prop::Bool(true)),
        "false" => return Some(Prop::Bool(false)),
        _ => {}
    }
    if let Some(inner) = t.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
        return Some(Prop::Text(inner.replace("''", "'")));
    }
    // a date (a continuous date axis' `start` / `end`): its ISO text
    if let Some(inner) = t.strip_prefix("datetime'").and_then(|s| s.strip_suffix('\'')) {
        return Some(Prop::Text(inner.to_string()));
    }
    let digits = t.strip_suffix(['D', 'L', 'M']).unwrap_or(t);
    digits
        .parse::<f64>()
        .ok()
        .filter(|x| x.is_finite())
        .map(Prop::Num)
}

/// Resolves a color value: a literal, a theme data color, or a theme style string.
fn color(v: &Value, theme: Option<&Theme>) -> Option<String> {
    let c = v.get("solid").and_then(|s| s.get("color")).unwrap_or(v);
    if let Some(s) = c.as_str() {
        return Some(s.to_string());
    }
    let expr = c.get("expr")?;
    if let Some(Prop::Text(s)) = expr
        .get("Literal")
        .and_then(|l| l.get("Value"))
        .and_then(Value::as_str)
        .and_then(literal)
    {
        return Some(s);
    }
    let tdc = expr.get("ThemeDataColor")?;
    let id = tdc.get("ColorId").and_then(Value::as_i64)?;
    let percent = tdc.get("Percent").and_then(Value::as_f64).unwrap_or(0.0);
    let fallback = builtin("default");
    let t = theme.unwrap_or(&fallback);
    let base = match id {
        0 => t.background.clone(),
        1 => t.foreground.clone(),
        n => usize::try_from(n)
            .ok()
            .and_then(|n| n.checked_sub(2))
            .and_then(|i| t.data_colors.get(i))
            .cloned()?,
    };
    Some(shade(&base, percent))
}

/// Lightens (`percent` > 0) or darkens (< 0) a `#RRGGBB` color; other spellings pass through.
fn shade(hex: &str, percent: f64) -> String {
    let parsed = hex
        .strip_prefix('#')
        .filter(|h| h.len() == 6)
        .and_then(|h| u32::from_str_radix(h, 16).ok());
    let Some(rgb) = parsed else {
        return hex.to_string();
    };
    if percent == 0.0 || !percent.is_finite() {
        return hex.to_uppercase();
    }
    let p = percent.clamp(-1.0, 1.0);
    let channel = |shift: u32| {
        let c = f64::from((rgb >> shift) & 0xFF);
        let v = if p > 0.0 {
            c + (255.0 - c) * p
        } else {
            c * (1.0 + p)
        };
        v.round().clamp(0.0, 255.0) as u32
    };
    format!("#{:02X}{:02X}{:02X}", channel(16), channel(8), channel(0))
}

/// One property of a visual object entry (`properties.<name>.expr.Literal.Value`) or of a theme
/// style entry (`<name>`), colors resolved.
fn prop(entry: &Value, name: &str, from_theme: bool, theme: Option<&Theme>) -> Option<Prop> {
    let v = if from_theme {
        entry.get(name)?
    } else {
        entry.get("properties")?.get(name)?
    };
    if v.get("solid").is_some() {
        return color(v, theme).map(Prop::Color);
    }
    if from_theme {
        return match v {
            Value::Bool(b) => Some(Prop::Bool(*b)),
            Value::Number(n) => n.as_f64().map(Prop::Num),
            Value::String(s) => Some(Prop::Text(s.clone())),
            _ => None,
        };
    }
    let expr = v.get("expr")?;
    if expr.get("ThemeDataColor").is_some() {
        return color(v, theme).map(Prop::Color);
    }
    expr.get("Literal")?.get("Value")?.as_str().and_then(literal)
}

fn prop_json(p: &Prop) -> Value {
    match p {
        Prop::Bool(b) => json!(b),
        Prop::Num(x) => number(*x),
        Prop::Text(s) | Prop::Color(s) => json!(s),
    }
}

/// Style sources for one visual, lowest priority first: (entry, is theme style).
fn sources<'a>(v: &'a Visual, theme: Option<&'a Theme>, object: &str) -> Vec<(&'a Value, bool)> {
    let mut out = Vec::new();
    if let Some(t) = theme {
        let ty = v.visual_type.as_deref().unwrap_or("");
        for styles in &t.styles {
            for key in ["*", ty] {
                if key.is_empty() {
                    continue;
                }
                if let Some(entry) = styles
                    .get(key)
                    .and_then(|s| s.get("*"))
                    .and_then(|s| s.get(object))
                    .and_then(|o| o.get(0))
                {
                    out.push((entry, true));
                }
            }
        }
    }
    for key in ["objects", "visualContainerObjects"] {
        if let Some(entry) = v
            .visual_extra
            .get(key)
            .and_then(|o| o.get(object))
            .and_then(|o| o.get(0))
        {
            out.push((entry, false));
        }
    }
    out
}

/// `(output key, property names)`: the first property name that is set wins.
type PropMap<'a> = (&'a str, &'a [&'a str]);

/// Collects `(output key, property names)` of one object into a JSON map (later sources win).
fn object_json(v: &Visual, theme: Option<&Theme>, object: &str, props: &[(&str, &[&str])]) -> Option<Value> {
    let mut o = Map::new();
    for (entry, from_theme) in sources(v, theme, object) {
        for (key, names) in props {
            for name in *names {
                if let Some(p) = prop(entry, name, from_theme, theme) {
                    let value = match (*key, &p) {
                        // alignment, legend position and divider style are lower-case in the snapshot
                        ("alignment" | "position" | "style", Prop::Text(s)) => {
                            json!(s.to_ascii_lowercase())
                        }
                        _ => prop_json(&p),
                    };
                    o.insert((*key).to_string(), value);
                    break;
                }
            }
        }
    }
    (!o.is_empty()).then_some(Value::Object(o))
}

/// The snapshot `format` of a visual, `null` when nothing is set.
pub(crate) fn visual_format(v: &Visual, theme: Option<&Theme>) -> Value {
    let mut o = Map::new();
    let axis: &[PropMap<'_>] = &[
        ("show", &["show"]),
        ("title", &["showAxisTitle"]),
        ("title_text", &["titleText"]),
        ("min", &["start"]),
        ("max", &["end"]),
        ("log", &["logAxisScale"]),
        ("gridlines", &["gridlineShow"]),
        ("labels_color", &["labelColor"]),
        ("title_color", &["titleColor"]),
        ("font_size", &["fontSize"]),
        ("display_units", &["labelDisplayUnits"]),
        ("precision", &["labelPrecision"]),
        // category layout (categoryAxis): bar gaps, minimum category width, label room
        ("inner_padding", &["innerPadding"]),
        ("category_width", &["preferredCategoryWidth"]),
        ("max_margin", &["maxMarginFactor"]),
        ("concatenate", &["concatenateLabels"]),
        ("axis_type", &["axisType"]),
    ];
    // the secondary value axis of combo, line and area charts (valueAxis `sec*` properties)
    let axis2: &[PropMap<'_>] = &[
        ("show", &["secShow"]),
        ("title", &["secShowAxisTitle"]),
        ("title_text", &["secTitleText"]),
        ("min", &["secStart"]),
        ("max", &["secEnd"]),
        ("log", &["secLogAxisScale"]),
        ("labels_color", &["secLabelColor"]),
        ("font_size", &["secFontSize"]),
        ("display_units", &["secLabelDisplayUnits"]),
        ("precision", &["secLabelPrecision"]),
    ];
    let objects: [(&str, &str, &[PropMap<'_>]); 10] = [
        (
            "title",
            "title",
            &[
                ("show", &["show"]),
                ("font_size", &["fontSize"]),
                ("color", &["fontColor"]),
                ("alignment", &["alignment"]),
                ("font_family", &["fontFamily"]),
                ("bold", &["bold"]),
                ("italic", &["italic"]),
                ("underline", &["underline"]),
            ],
        ),
        (
            "background",
            "background",
            &[
                ("show", &["show"]),
                ("color", &["color"]),
                ("transparency", &["transparency"]),
            ],
        ),
        (
            "border",
            "border",
            &[
                ("show", &["show"]),
                ("color", &["color"]),
                ("radius", &["radius"]),
            ],
        ),
        (
            "legend",
            "legend",
            &[
                ("show", &["show"]),
                ("position", &["position"]),
                ("title", &["showTitle"]),
                ("title_text", &["titleText"]),
                ("labels_color", &["labelColor"]),
            ],
        ),
        (
            "subtitle",
            "subTitle",
            &[
                ("show", &["show"]),
                ("text", &["text"]),
                ("font_size", &["fontSize"]),
                ("color", &["fontColor"]),
                ("alignment", &["alignment"]),
                ("font_family", &["fontFamily"]),
                ("bold", &["bold"]),
                ("italic", &["italic"]),
                ("underline", &["underline"]),
            ],
        ),
        (
            "divider",
            "divider",
            &[
                ("show", &["show"]),
                ("color", &["color"]),
                ("width", &["width"]),
                ("style", &["style"]),
            ],
        ),
        ("x_axis", "categoryAxis", axis),
        ("y_axis", "valueAxis", axis),
        ("y2_axis", "valueAxis", axis2),
        (
            "padding",
            "padding",
            &[
                ("top", &["top"]),
                ("bottom", &["bottom"]),
                ("left", &["left"]),
                ("right", &["right"]),
            ],
        ),
    ];
    for (key, object, props) in objects {
        if let Some(x) = object_json(v, theme, object, props) {
            o.insert(key.into(), x);
        }
    }
    // Axis label sizes come from the visual only: Desktop 2.158 ignores a theme's axis
    // `fontSize` (`*` and per-type `visualStyles` alike, docs/render.md).
    for (key, object, props) in objects {
        if !key.ends_with("_axis") {
            continue;
        }
        let own = object_json(v, None, object, props).and_then(|x| x.get("font_size").cloned());
        if let Some(Value::Object(ax)) = o.get_mut(key) {
            match own {
                Some(size) => {
                    ax.insert("font_size".into(), size);
                }
                None => {
                    ax.remove("font_size");
                }
            }
        }
    }
    // buttons: the fill and text style, theme included (the snapshot `button` has the visual's own)
    if v.visual_type
        .as_deref()
        .is_some_and(|t| t.eq_ignore_ascii_case("actionButton"))
    {
        let fill: &[PropMap<'_>] = &[
            ("show", &["show"]),
            ("color", &["fillColor"]),
            ("transparency", &["transparency"]),
        ];
        let text: &[PropMap<'_>] = &[
            ("color", &["fontColor"]),
            ("font_size", &["fontSize"]),
            ("font_family", &["fontFamily"]),
            ("bold", &["bold"]),
        ];
        for (key, object, props) in [("button_fill", "fill", fill), ("button_text", "text", text)] {
            if let Some(x) = object_json(v, theme, object, props) {
                o.insert(key.into(), x);
            }
        }
    }
    let labels = object_json(v, theme, "labels", &[("show", &["show"])]);
    if let Some(show) = labels
        .as_ref()
        .and_then(|l| l.get("show"))
        .and_then(Value::as_bool)
    {
        o.insert("data_labels".into(), json!(show));
    }
    render_keys::add(&mut o, v, theme);
    if o.is_empty() {
        Value::Null
    } else {
        Value::Object(o)
    }
}

/// The page background color: the page's `background` object (unless fully transparent), else
/// the theme's page style; `null` when neither sets one.
pub(crate) fn page_background(p: &Page, theme: Option<&Theme>) -> Value {
    let page_entry = p
        .extra
        .get("objects")
        .and_then(|o| o.get("background"))
        .and_then(|b| b.get(0));
    let theme_entries: Vec<&Value> = theme
        .map(|t| {
            t.styles
                .iter()
                .filter_map(|s| {
                    s.get("page")
                        .and_then(|x| x.get("*"))
                        .and_then(|x| x.get("background"))
                        .and_then(|x| x.get(0))
                })
                .collect()
        })
        .unwrap_or_default();
    // highest priority first
    let mut candidates: Vec<(&Value, bool)> = Vec::new();
    if let Some(e) = page_entry {
        candidates.push((e, false));
    }
    candidates.extend(theme_entries.into_iter().rev().map(|e| (e, true)));
    for (entry, from_theme) in candidates {
        let transparent = matches!(
            prop(entry, "transparency", from_theme, theme),
            Some(Prop::Num(x)) if x >= 100.0
        );
        if let Some(Prop::Color(c)) = prop(entry, "color", from_theme, theme) {
            return if transparent { Value::Null } else { json!(c) };
        }
    }
    Value::Null
}

/// Entries of page object `object` (`background`, `outspace`), lowest priority first: the
/// theme's `visualStyles.page.*` (base then custom), then the page's own `objects`.
pub(crate) fn page_entries<'a>(
    p: &'a Page,
    theme: Option<&'a Theme>,
    object: &str,
) -> Vec<(&'a Value, bool)> {
    let mut out: Vec<(&Value, bool)> = theme
        .map(|t| {
            t.styles
                .iter()
                .filter_map(|s| {
                    s.get("page")
                        .and_then(|x| x.get("*"))
                        .and_then(|x| x.get(object))
                        .and_then(|x| x.get(0))
                })
                .map(|e| (e, true))
                .collect()
        })
        .unwrap_or_default();
    if let Some(e) = p
        .extra
        .get("objects")
        .and_then(|o| o.get(object))
        .and_then(|b| b.get(0))
    {
        out.push((e, false));
    }
    out
}

/// The highest-priority value of property `name` among `entries` (lowest priority first).
fn last_prop(entries: &[(&Value, bool)], name: &str, theme: Option<&Theme>) -> Option<Prop> {
    entries
        .iter()
        .rev()
        .find_map(|(e, from_theme)| prop(e, name, *from_theme, theme))
}

/// The page wallpaper (`outspace`) color and transparency (0–100).
pub(crate) fn page_wallpaper(p: &Page, theme: Option<&Theme>) -> (Option<String>, Option<f64>) {
    let entries = page_entries(p, theme, "outspace");
    let color = match last_prop(&entries, "color", theme) {
        Some(Prop::Color(c)) => Some(c),
        _ => None,
    };
    let transparency = match last_prop(&entries, "transparency", theme) {
        Some(Prop::Num(x)) => Some(x.clamp(0.0, 100.0)),
        _ => None,
    };
    (color, transparency)
}

/// The page background transparency (0–100), when the page or the theme sets one.
pub(crate) fn page_background_transparency(p: &Page, theme: Option<&Theme>) -> Option<f64> {
    match last_prop(&page_entries(p, theme, "background"), "transparency", theme) {
        Some(Prop::Num(x)) => Some(x.clamp(0.0, 100.0)),
        _ => None,
    }
}

/// Every entry of the theme's `visualStyles.*.*.<object>` then `visualStyles.page.*.<object>`
/// (base then custom theme), lowest priority first: the Filters pane's `outspacePane` and
/// `filterCard` defaults (the public theme schema puts them under `page`, Microsoft Learn's
/// example under `*`).
pub(crate) fn theme_object_entries<'a>(theme: Option<&'a Theme>, object: &str) -> Vec<&'a Value> {
    let mut out = Vec::new();
    for styles in theme.map(|t| t.styles.as_slice()).unwrap_or_default() {
        for key in ["*", "page"] {
            if let Some(list) = styles
                .get(key)
                .and_then(|s| s.get("*"))
                .and_then(|s| s.get(object))
                .and_then(Value::as_array)
            {
                out.extend(list.iter().take(64).filter(|e| e.is_object()));
            }
        }
    }
    out
}

/// Property `name` of a formatting entry as plain JSON (a colour as `#RRGGBB`, a number, a
/// boolean, a text): a theme entry (`from_theme`) holds plain values, a report entry PBIR
/// literals under `properties`.
pub(crate) fn entry_value(
    entry: &Value,
    name: &str,
    from_theme: bool,
    theme: Option<&Theme>,
) -> Option<Value> {
    prop(entry, name, from_theme, theme).map(|p| prop_json(&p))
}

/// Style sources of visual object `object`, lowest priority first (see `sources`).
pub(crate) fn visual_sources<'a>(
    v: &'a Visual,
    theme: Option<&'a Theme>,
    object: &str,
) -> Vec<(&'a Value, bool)> {
    sources(v, theme, object)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals_and_shades() {
        assert_eq!(literal("14D"), Some(Prop::Num(14.0)));
        assert_eq!(literal("12L"), Some(Prop::Num(12.0)));
        assert_eq!(literal("'It''s'"), Some(Prop::Text("It's".into())));
        assert_eq!(literal("true"), Some(Prop::Bool(true)));
        assert_eq!(literal("x"), None);
        assert_eq!(
            literal("datetime'2026-01-01T00:00:00'"),
            Some(Prop::Text("2026-01-01T00:00:00".into()))
        );
        assert_eq!(shade("#000000", 0.5), "#808080");
        assert_eq!(shade("#FFFFFF", -0.5), "#808080");
        assert_eq!(shade("red", 0.5), "red");
        assert_eq!(
            first_font("'Segoe UI Semibold', wf_segoe-ui_semibold"),
            Some("Segoe UI Semibold".into())
        );
    }
}
