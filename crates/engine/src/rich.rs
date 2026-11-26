//! Rich report content (ROADMAP M11.20, `docs/formats/pbir.md` "Text boxes, images, buttons and
//! page format"): the commands behind Desktop's Insert tab and the page's Format pane.
//!
//! - `textbox.add` / `textbox.set`: a text box and its paragraphs, written as Desktop writes them
//!   (`objects.general[].properties.paragraphs[] = {textRuns: [{value, textStyle?, url?}],
//!   horizontalTextAlignment?}`), dynamic values ("+ Value") as a run whose `value` is
//!   `{propertyIdentifier: {objectName: "values", propertyName: "expr"}, selector: {id}}` with
//!   the field in `objects.values[{properties: {expr}, selector: {id}}]`;
//! - `image.add` / `image.set`: the bytes become a report resource
//!   (`StaticResources/RegisteredResources/<name><16 digits>.<ext>`, an `Image` item of the
//!   `RegisteredResources` package), the image visual names it in `general.imageUrl`
//!   (`ResourcePackageItem`) and scales it with `imageScaling.imageScalingType`;
//! - `button.add`: Desktop's Insert › Buttons (an `actionButton` with its `icon.shapeType` in the
//!   `default` state and its `visualLink`, or a page / bookmark navigator);
//! - `visual.set_action`: a button's, image's or shape's action (`visualContainerObjects.visualLink`);
//! - `page.format`: canvas background and wallpaper (colour, transparency, image, scaling), page
//!   size and vertical alignment.
//!
//! Image bytes are untrusted: at most [`MAX_IMAGE_BYTES`], typed by their content (PNG, JPEG,
//! GIF, WebP, SVG; never by the name), and decoded once by the renderer's own decoder (size caps,
//! `catch_unwind`, SVG with no reference outside its bytes) before anything is stored.

use bicraft_report::{FieldRef, Page, Position, Report, Resource, Visual};
use serde_json::{Map, Value, json};

use crate::Session;
use crate::params::{allow, opt_str, str_param};
use crate::refs::field_text;
use crate::report::{
    check_coordinate, check_size, new_object_name, opt_num, outside_page_warning, page_at_mut, page_index,
    parts, position_json, report_ref, resolve_for_binding, visual_index,
};

/// Largest image stored (the snapshot leaves out bigger ones, `images::MAX_IMAGE_BYTES`).
pub const MAX_IMAGE_BYTES: usize = crate::images::MAX_IMAGE_BYTES;
/// Longest `data` parameter (base64 of [`MAX_IMAGE_BYTES`] plus a `data:` header).
const MAX_DATA_CHARS: usize = MAX_IMAGE_BYTES / 3 * 4 + 1024;
/// Most paragraphs, runs, dynamic values and characters of one text box.
const MAX_PARAGRAPHS: usize = 500;
const MAX_RUNS: usize = 2000;
const MAX_VALUES: usize = 50;
const MAX_CHARS: usize = 100_000;
/// Longest font family, URL, tooltip, bookmark name.
const MAX_NAME: usize = 200;
const MAX_URL: usize = 2000;
const MAX_TOOLTIP: usize = 1000;
/// Longest name prefix of an image resource file.
const MAX_SLUG: usize = 60;

const REGISTERED: &str = "RegisteredResources";
const IMAGE_ITEM: &str = "Image";
const INSERTED: &str = "InsertVisualButton";

// ---------------------------------------------------------------------------------------------
// PBIR literals
// ---------------------------------------------------------------------------------------------

fn lit(s: String) -> Value {
    json!({"expr": {"Literal": {"Value": s}}})
}

pub(crate) fn text_lit(s: &str) -> Value {
    lit(format!("'{}'", s.replace('\'', "''")))
}

pub(crate) fn bool_lit(b: bool) -> Value {
    lit(b.to_string())
}

pub(crate) fn num_lit(n: f64) -> Value {
    lit(format!("{n}D"))
}

pub(crate) fn color_lit(c: &str) -> Value {
    json!({"solid": {"color": text_lit(c)}})
}

/// The text of a literal (`'x'` unquoted), `None` for other values.
fn lit_text(v: Option<&Value>) -> Option<String> {
    let s = v?.get("expr")?.get("Literal")?.get("Value")?.as_str()?;
    Some(
        s.strip_prefix('\'')
            .and_then(|x| x.strip_suffix('\''))
            .map_or_else(|| s.to_string(), |x| x.replace("''", "'")),
    )
}

/// `#RGB`, `#RRGGBB` or `#RRGGBBAA`.
pub(crate) fn check_color(key: &str, c: &str) -> Result<String, String> {
    let hex = c.strip_prefix('#').unwrap_or("");
    if matches!(hex.len(), 3 | 6 | 8) && hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
        Ok(format!("#{}", hex.to_ascii_uppercase()))
    } else {
        Err(format!(
            "`{key}` must be a colour like #118DFF (got `{}`)",
            clip(c, 40)
        ))
    }
}

fn clip(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// Links a reader may follow: `http`, `https` and `mailto` only (never `javascript:`, `file:`).
fn check_url(key: &str, u: &str) -> Result<String, String> {
    let u = u.trim();
    if u.len() > MAX_URL {
        return Err(format!("`{key}` is longer than {MAX_URL} characters"));
    }
    let lower = u.to_ascii_lowercase();
    let ok = ["http://", "https://", "mailto:"]
        .iter()
        .any(|p| lower.starts_with(p) && lower.len() > p.len());
    if !ok || u.chars().any(char::is_control) {
        return Err(format!(
            "`{key}` must be an http, https or mailto link (got `{}`)",
            clip(u, 60)
        ));
    }
    Ok(u.to_string())
}

fn bounded_text<'a>(p: &'a Value, key: &str, max: usize) -> Result<Option<&'a str>, String> {
    let s = opt_str(p, key)?;
    if s.is_some_and(|s| s.chars().count() > max) {
        return Err(format!("`{key}` is longer than {max} characters"));
    }
    Ok(s)
}

fn as_obj<'a>(v: &'a Value, what: &str) -> Result<&'a Map<String, Value>, String> {
    v.as_object().ok_or_else(|| format!("{what} must be an object"))
}

fn keys(o: &Map<String, Value>, allowed: &[&str], what: &str) -> Result<(), String> {
    match o.keys().find(|k| !allowed.contains(&k.as_str())) {
        Some(k) => Err(format!(
            "{what}: unknown key `{}` (allowed: {})",
            clip(k, 60),
            allowed.join(", ")
        )),
        None => Ok(()),
    }
}

/// A visual's object entries `objects.<name>` (or `visualContainerObjects.<name>`).
pub(crate) fn object_list<'a>(
    v: &'a mut Visual,
    group: &str,
    name: &str,
) -> Result<&'a mut Vec<Value>, String> {
    let g = v
        .visual_extra
        .entry(group.to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !g.is_object() {
        return Err(format!("the visual's `{group}` is not an object"));
    }
    let g = g.as_object_mut().ok_or("internal: not an object")?;
    let list = g
        .entry(name.to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    if !list.is_array() {
        *list = Value::Array(Vec::new());
    }
    list.as_array_mut()
        .ok_or_else(|| "internal: not a list".to_string())
}

/// The properties of the entry without a selector (created when missing) of a list.
pub(crate) fn plain_props(list: &mut Vec<Value>) -> Result<&mut Map<String, Value>, String> {
    let i = match list
        .iter()
        .position(|e| e.is_object() && e.get("selector").is_none_or(Value::is_null))
    {
        Some(i) => i,
        None => {
            list.push(json!({"properties": {}}));
            list.len().saturating_sub(1)
        }
    };
    let o = list
        .get_mut(i)
        .and_then(Value::as_object_mut)
        .ok_or("internal: no object entry")?;
    let props = o.entry("properties").or_insert_with(|| Value::Object(Map::new()));
    if !props.is_object() {
        *props = Value::Object(Map::new());
    }
    props
        .as_object_mut()
        .ok_or_else(|| "internal: no properties".to_string())
}

// ---------------------------------------------------------------------------------------------
// Inserting a visual
// ---------------------------------------------------------------------------------------------

/// A position for a new visual: the given values, else `w`×`h` centred on the page.
fn place(page: &Page, p: &Value, w: f64, h: f64) -> Result<Position, String> {
    let width = opt_num(p, "width")?
        .map(|v| check_size("width", v))
        .transpose()?
        .unwrap_or(w);
    let height = opt_num(p, "height")?
        .map(|v| check_size("height", v))
        .transpose()?
        .unwrap_or(h);
    let pw = if page.width > 0.0 { page.width } else { 1280.0 };
    let ph = if page.height > 0.0 { page.height } else { 720.0 };
    let x = match opt_num(p, "x")? {
        Some(v) => check_coordinate("x", v)?,
        None => ((pw - width) / 2.0).max(0.0).round(),
    };
    let y = match opt_num(p, "y")? {
        Some(v) => check_coordinate("y", v)?,
        None => ((ph - height) / 2.0).max(0.0).round(),
    };
    let z = page
        .visuals
        .iter()
        .map(|v| v.position.z)
        .fold(None, |m: Option<f64>, z| Some(m.map_or(z, |m| m.max(z))))
        .map_or(0.0, |m| (m + 1000.0).min(crate::report::MAX_SIZE));
    let tab_order = page
        .visuals
        .iter()
        .filter_map(|v| v.position.tab_order)
        .max()
        .map_or(0, |t| t.saturating_add(1000));
    Ok(Position {
        x,
        y,
        z,
        width,
        height,
        tab_order: Some(tab_order),
    })
}

/// Adds `v` (its name chosen here) to page `pi`; returns the command's answer.
fn insert(r: &mut Report, pi: usize, mut v: Visual, mut warnings: Vec<String>) -> Result<Value, String> {
    let page = page_at_mut(r, pi)?;
    warnings.extend(outside_page_warning(page, &v.position));
    let taken = |n: &str| page.visuals.iter().any(|x| x.name.eq_ignore_ascii_case(n));
    v.name = new_object_name(taken);
    v.visual_extra
        .insert("drillFilterOtherVisuals".into(), json!(true));
    v.extra.insert("howCreated".into(), json!(INSERTED));
    let out = json!({
        "page": page.name,
        "display_name": page.display_name,
        "visual": v.name,
        "visual_type": v.visual_type,
        "position": position_json(&v.position),
        "warnings": warnings,
    });
    page.visuals.push(v);
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Text boxes
// ---------------------------------------------------------------------------------------------

/// One run as the caller gives it.
struct RunIn {
    text: String,
    value: Option<FieldRef>,
    style: Map<String, Value>,
    url: Option<String>,
}

struct ParaIn {
    align: Option<&'static str>,
    list: Option<&'static str>,
    runs: Vec<RunIn>,
}

fn parse_style(r: &Map<String, Value>, what: &str) -> Result<Map<String, Value>, String> {
    let mut st = Map::new();
    let flag = |k: &str| -> Result<bool, String> {
        match r.get(k) {
            None | Some(Value::Null) => Ok(false),
            Some(Value::Bool(b)) => Ok(*b),
            Some(_) => Err(format!("{what}: `{k}` must be true or false")),
        }
    };
    if let Some(f) = r.get("font_family").filter(|v| !v.is_null()) {
        let f = f
            .as_str()
            .filter(|f| {
                !f.trim().is_empty() && f.chars().count() <= MAX_NAME && !f.chars().any(char::is_control)
            })
            .ok_or_else(|| format!("{what}: `font_family` must be a font name"))?;
        st.insert("fontFamily".into(), json!(f.trim()));
    }
    if let Some(sz) = r.get("font_size").filter(|v| !v.is_null()) {
        let n = sz
            .as_f64()
            .filter(|n| n.is_finite() && (1.0..=400.0).contains(n))
            .ok_or_else(|| format!("{what}: `font_size` must be a number of points from 1 to 400"))?;
        st.insert("fontSize".into(), json!(format!("{n}pt")));
    }
    if flag("bold")? {
        st.insert("fontWeight".into(), json!("bold"));
    }
    if flag("italic")? {
        st.insert("fontStyle".into(), json!("italic"));
    }
    if flag("underline")? {
        st.insert("textDecoration".into(), json!("underline"));
    }
    if let Some(c) = r.get("color").filter(|v| !v.is_null()) {
        let c = c
            .as_str()
            .ok_or_else(|| format!("{what}: `color` must be a string"))?;
        st.insert("color".into(), json!(check_color("color", c)?));
    }
    Ok(st)
}

fn parse_paragraphs(s: &Session, v: &Value, warnings: &mut Vec<String>) -> Result<Vec<ParaIn>, String> {
    let list = v
        .as_array()
        .ok_or("`paragraphs` must be a list of {align?, list?, runs: [{text | value, bold?…}]}")?;
    if list.len() > MAX_PARAGRAPHS {
        return Err(format!("at most {MAX_PARAGRAPHS} paragraphs"));
    }
    let measures = s
        .report
        .as_ref()
        .map(crate::report_measures::report_measures)
        .unwrap_or_default();
    let (mut runs_n, mut chars, mut values) = (0usize, 0usize, 0usize);
    let mut out = Vec::with_capacity(list.len());
    for (pi, p) in list.iter().enumerate() {
        let what = format!("paragraph {}", pi.saturating_add(1));
        let o = as_obj(p, &what)?;
        keys(o, &["align", "list", "runs"], &what)?;
        let align = match o
            .get("align")
            .and_then(Value::as_str)
            .map(str::to_ascii_lowercase)
        {
            None => None,
            Some(a) => Some(match a.as_str() {
                "left" => "left",
                "center" | "centre" => "center",
                "right" => "right",
                _ => return Err(format!("{what}: `align` must be left, center or right")),
            }),
        };
        if o.get("align").is_some_and(|a| !a.is_null() && !a.is_string()) {
            return Err(format!("{what}: `align` must be left, center or right"));
        }
        let list_kind = match o.get("list") {
            None | Some(Value::Null) => None,
            Some(Value::String(l)) => match l.to_ascii_lowercase().as_str() {
                "" | "none" => None,
                "bullet" => Some("bullet"),
                "number" | "numbered" => Some("number"),
                _ => return Err(format!("{what}: `list` must be bullet, number or none")),
            },
            Some(_) => return Err(format!("{what}: `list` must be bullet, number or none")),
        };
        let runs = match o.get("runs") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(a)) => a.clone(),
            Some(_) => return Err(format!("{what}: `runs` must be a list")),
        };
        let mut rs = Vec::with_capacity(runs.len());
        for (ri, r) in runs.iter().enumerate() {
            let what = format!("{what}, run {}", ri.saturating_add(1));
            runs_n = runs_n.saturating_add(1);
            if runs_n > MAX_RUNS {
                return Err(format!("at most {MAX_RUNS} runs in a text box"));
            }
            let ro = as_obj(r, &what)?;
            keys(
                ro,
                &[
                    "text",
                    "value",
                    "bold",
                    "italic",
                    "underline",
                    "font_family",
                    "font_size",
                    "color",
                    "url",
                ],
                &what,
            )?;
            let text = match ro.get("text") {
                None | Some(Value::Null) => String::new(),
                Some(Value::String(t)) => t.clone(),
                Some(_) => return Err(format!("{what}: `text` must be a string")),
            };
            chars = chars.saturating_add(text.chars().count());
            if chars > MAX_CHARS {
                return Err(format!("at most {MAX_CHARS} characters in a text box"));
            }
            let value = match ro.get("value") {
                None | Some(Value::Null) => None,
                Some(Value::String(f)) => {
                    values = values.saturating_add(1);
                    if values > MAX_VALUES {
                        return Err(format!("at most {MAX_VALUES} dynamic values in a text box"));
                    }
                    let field = resolve_for_binding(s.model.as_ref(), &measures, f, false, warnings)
                        .map_err(|e| format!("{what}: {e}"))?;
                    if !matches!(field, FieldRef::Measure { .. } | FieldRef::Aggregation { .. }) {
                        return Err(format!(
                            "{what}: a dynamic value is a measure or an aggregation like Sum(Table[Column]), not `{}`",
                            clip(f, 80)
                        ));
                    }
                    Some(field)
                }
                Some(_) => return Err(format!("{what}: `value` must be a field like \"[Total Sales]\"")),
            };
            if value.is_some() && !text.is_empty() {
                return Err(format!("{what}: a run has `text` or a dynamic `value`, not both"));
            }
            let url = match ro.get("url") {
                None | Some(Value::Null) => None,
                Some(Value::String(u)) if u.trim().is_empty() => None,
                Some(Value::String(u)) => Some(check_url("url", u).map_err(|e| format!("{what}: {e}"))?),
                Some(_) => return Err(format!("{what}: `url` must be a link")),
            };
            rs.push(RunIn {
                text,
                value,
                style: parse_style(ro, &what)?,
                url,
            });
        }
        out.push(ParaIn {
            align,
            list: list_kind,
            runs: rs,
        });
    }
    Ok(out)
}

/// Plain text: one paragraph per line, one run each.
fn text_paragraphs(text: &str) -> Result<Vec<ParaIn>, String> {
    if text.chars().count() > MAX_CHARS {
        return Err(format!("at most {MAX_CHARS} characters in a text box"));
    }
    let lines: Vec<&str> = text.split('\n').collect();
    if lines.len() > MAX_PARAGRAPHS {
        return Err(format!("at most {MAX_PARAGRAPHS} paragraphs"));
    }
    Ok(lines
        .into_iter()
        .map(|l| ParaIn {
            align: None,
            list: None,
            runs: vec![RunIn {
                text: l.trim_end_matches('\r').to_string(),
                value: None,
                style: Map::new(),
                url: None,
            }],
        })
        .collect())
}

/// Desktop's `paragraphs` and the `values` entries of the dynamic values.
fn write_paragraphs(paras: &[ParaIn]) -> (Value, Vec<Value>) {
    let mut values = Vec::new();
    let mut out = Vec::with_capacity(paras.len());
    for p in paras {
        let mut runs = Vec::with_capacity(p.runs.len().max(1));
        for r in &p.runs {
            let mut run = Map::new();
            match &r.value {
                Some(f) => {
                    let id = format!("value{}", values.len().saturating_add(1));
                    run.insert(
                        "value".into(),
                        json!({"propertyIdentifier": {"objectName": "values", "propertyName": "expr"},
                               "selector": {"id": id}}),
                    );
                    values.push(json!({
                        "properties": {"expr": {"expr": bicraft_report::query::field_json(f)}},
                        "selector": {"id": id},
                    }));
                }
                None => {
                    run.insert("value".into(), json!(r.text));
                }
            }
            if !r.style.is_empty() {
                run.insert("textStyle".into(), Value::Object(r.style.clone()));
            }
            if let Some(u) = &r.url {
                run.insert("url".into(), json!(u));
            }
            runs.push(Value::Object(run));
        }
        // Desktop writes an empty paragraph as one empty run
        if runs.is_empty() {
            runs.push(json!({"value": ""}));
        }
        let mut para = Map::new();
        para.insert("textRuns".into(), Value::Array(runs));
        if let Some(a) = p.align {
            para.insert("horizontalTextAlignment".into(), json!(a));
        }
        if let Some(l) = p.list {
            para.insert("listType".into(), json!(l));
        }
        out.push(Value::Object(para));
    }
    (Value::Array(out), values)
}

/// The paragraphs of `p` (`paragraphs`, else `text`), or `None` when neither is given.
fn paragraphs_param(
    s: &Session,
    p: &Value,
    warnings: &mut Vec<String>,
) -> Result<Option<Vec<ParaIn>>, String> {
    match (
        p.get("paragraphs").filter(|v| !v.is_null()),
        p.get("text").filter(|v| !v.is_null()),
    ) {
        (Some(_), Some(_)) => Err("give `paragraphs` or `text`, not both".into()),
        (Some(v), None) => parse_paragraphs(s, v, warnings).map(Some),
        (None, Some(Value::String(t))) => text_paragraphs(t).map(Some),
        (None, Some(_)) => Err("`text` must be a string".into()),
        (None, None) => Ok(None),
    }
}

fn set_text(v: &mut Visual, paras: &[ParaIn]) -> Result<(usize, usize), String> {
    let (paragraphs, values) = write_paragraphs(paras);
    let n_values = values.len();
    let general = object_list(v, "objects", "general")?;
    plain_props(general)?.insert("paragraphs".into(), paragraphs);
    let objects = v
        .visual_extra
        .get_mut("objects")
        .and_then(Value::as_object_mut)
        .ok_or("internal: no objects")?;
    if values.is_empty() {
        objects.shift_remove("values");
    } else {
        objects.insert("values".into(), Value::Array(values));
    }
    Ok((paras.len(), n_values))
}

/// `textbox.add {page, x?, y?, width?, height?, paragraphs? | text?}`.
pub(crate) fn textbox_add(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "x", "y", "width", "height", "paragraphs", "text"])?;
    let key = str_param(p, "page")?;
    let mut warnings = Vec::new();
    let paras = paragraphs_param(s, p, &mut warnings)?;
    let (_, r) = parts(s)?;
    let pi = page_index(r, key)?;
    let page = r.pages.get(pi).ok_or("internal: page index out of range")?;
    let position = place(page, p, 280.0, 110.0)?;
    let mut v = Visual {
        position,
        visual_type: Some("textbox".into()),
        ..Default::default()
    };
    let paras = paras.unwrap_or_else(|| {
        vec![ParaIn {
            align: None,
            list: None,
            runs: Vec::new(),
        }]
    });
    set_text(&mut v, &paras)?;
    let out = insert(r, pi, v, warnings)?;
    s.report_dirty = true;
    Ok(out)
}

/// `textbox.set {page, visual, paragraphs | text}`.
pub(crate) fn textbox_set(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "visual", "paragraphs", "text"])?;
    let key = str_param(p, "page")?;
    let visual = str_param(p, "visual")?;
    let mut warnings = Vec::new();
    let paras = paragraphs_param(s, p, &mut warnings)?.ok_or("textbox.set needs `paragraphs` or `text`")?;
    let (_, r) = parts(s)?;
    let pi = page_index(r, key)?;
    let page = page_at_mut(r, pi)?;
    let vi = visual_index(page, visual)?;
    let v = page
        .visuals
        .get_mut(vi)
        .ok_or("internal: visual index out of range")?;
    if !v
        .visual_type
        .as_deref()
        .is_some_and(|t| t.eq_ignore_ascii_case("textbox"))
    {
        return Err(format!(
            "visual `{visual}` is a {}, not a text box",
            v.visual_type.as_deref().unwrap_or("group")
        ));
    }
    let before = v.clone();
    let (n, values) = set_text(v, &paras)?;
    let changed = *v != before;
    let name = v.name.clone();
    if changed {
        s.report_dirty = true;
    }
    Ok(json!({"visual": name, "paragraphs": n, "values": values, "changed": changed, "warnings": warnings}))
}

// ---------------------------------------------------------------------------------------------
// Images
// ---------------------------------------------------------------------------------------------

/// A checked image: its bytes, its type and file extension, its natural size.
struct ImageIn {
    bytes: Vec<u8>,
    mime: &'static str,
    ext: &'static str,
    width: f64,
    height: f64,
    /// The name it came with (a file name), for the resource's name.
    name: String,
}

fn ext_of(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "svg",
    }
}

/// Checks untrusted image bytes: size, type by content, a full decode by the renderer.
fn check_image(bytes: Vec<u8>, name: String, warnings: &mut Vec<String>) -> Result<ImageIn, String> {
    if bytes.is_empty() {
        return Err("the image is empty".into());
    }
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(format!(
            "the image is {} bytes; the limit is {MAX_IMAGE_BYTES} ({} MB)",
            bytes.len(),
            MAX_IMAGE_BYTES >> 20
        ));
    }
    let (mime, _) = crate::images::sniff(&bytes)
        .ok_or("not an image BIcraft reads (PNG, JPEG, GIF, WebP or SVG, by its content)")?;
    let (w, h) = bicraft_render::image_size(&bytes).ok_or(
        "the image cannot be decoded (damaged, more than 16 million pixels or 16,384 pixels a side, \
         or an SVG BIcraft cannot draw)",
    )?;
    if mime == "image/webp" {
        warnings.push("Power BI Desktop's Insert image dialog does not offer WebP files".into());
    }
    Ok(ImageIn {
        bytes,
        mime,
        ext: ext_of(mime),
        width: f64::from(w),
        height: f64::from(h),
        name,
    })
}

/// The image of `p`: `path` (a file) or `data` (base64 or a `data:` URL), with `name`.
fn image_param(
    p: &Value,
    path_key: &str,
    data_key: &str,
    warnings: &mut Vec<String>,
) -> Result<Option<ImageIn>, String> {
    let path = opt_str(p, path_key)?.filter(|s| !s.trim().is_empty());
    let data = opt_str(p, data_key)?.filter(|s| !s.trim().is_empty());
    let name = bounded_text(p, "name", MAX_NAME)?;
    let (bytes, from) = match (path, data) {
        (Some(_), Some(_)) => return Err(format!("give `{path_key}` or `{data_key}`, not both")),
        (None, None) => return Ok(None),
        (Some(path), None) => {
            let fp = std::path::Path::new(path);
            let len = std::fs::metadata(fp)
                .map_err(|e| format!("cannot read {}: {e}", fp.display()))?
                .len();
            if len > MAX_IMAGE_BYTES as u64 {
                return Err(format!(
                    "{} is {len} bytes; the limit is {MAX_IMAGE_BYTES} ({} MB)",
                    fp.display(),
                    MAX_IMAGE_BYTES >> 20
                ));
            }
            let bytes = std::fs::read(fp).map_err(|e| format!("cannot read {}: {e}", fp.display()))?;
            let file = fp
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_default();
            (bytes, file)
        }
        (None, Some(d)) => {
            if d.len() > MAX_DATA_CHARS {
                return Err(format!(
                    "`{data_key}` is longer than {MAX_DATA_CHARS} characters (images up to {} MB)",
                    MAX_IMAGE_BYTES >> 20
                ));
            }
            let body = if d.get(..5).is_some_and(|h| h.eq_ignore_ascii_case("data:")) {
                let (head, body) = d.split_once(',').ok_or("a `data:` URL needs a comma")?;
                if !head.to_ascii_lowercase().ends_with(";base64") {
                    return Err(format!(
                        "`{data_key}` must be base64 (a `data:…;base64,` URL or plain base64)"
                    ));
                }
                body
            } else {
                d
            };
            let bytes =
                crate::images::base64(body).ok_or_else(|| format!("`{data_key}` is not valid base64"))?;
            (bytes, String::new())
        }
    };
    let name = name.map(str::to_string).unwrap_or(from);
    check_image(bytes, name, warnings).map(Some)
}

fn fnv64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// `<name without its extension, letters, digits, - and _ only><16 digits>.<ext>`, the digits
/// derived from the bytes (the same image gets the same file).
fn resource_file(img: &ImageIn) -> String {
    resource_file_of(&img.name, &img.bytes, img.ext, "image")
}

/// `<name without its extension, letters, digits, - and _ only><16 digits>.<ext>` for any
/// registered resource (`fallback` when the name keeps no character).
pub(crate) fn resource_file_of(name: &str, bytes: &[u8], ext: &str, fallback: &str) -> String {
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
    let slug: String = stem
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(MAX_SLUG)
        .collect();
    let slug = if slug.is_empty() {
        fallback.to_string()
    } else {
        slug
    };
    let digits = fnv64(bytes) % 10_000_000_000_000_000;
    format!("{slug}{digits:016}.{ext}")
}

fn packages(r: &Report) -> Result<Vec<Value>, String> {
    match r.extra.get("resourcePackages") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(a)) => Ok(a.clone()),
        Some(_) => Err("report.json `resourcePackages` is not a list; the image cannot be registered".into()),
    }
}

pub(crate) fn set_packages(r: &mut Report, list: Vec<Value>) {
    if r.extra.contains_key("resourcePackages") {
        r.extra.insert("resourcePackages".into(), Value::Array(list));
        return;
    }
    // a new key goes where Desktop writes it: before these report.json properties
    const AFTER: [&str; 5] = [
        "organizationCustomVisuals",
        "annotations",
        "dataSourceVariables",
        "settings",
        "slowDataSourceSettings",
    ];
    let old = std::mem::take(&mut r.extra);
    let mut value = Some(Value::Array(list));
    for (k, v) in old {
        if AFTER.contains(&k.as_str())
            && let Some(p) = value.take()
        {
            r.extra.insert("resourcePackages".into(), p);
        }
        r.extra.insert(k, v);
    }
    if let Some(p) = value {
        r.extra.insert("resourcePackages".into(), p);
    }
}

fn norm(path: &str) -> String {
    path.replace('\\', "/")
        .trim_start_matches('/')
        .to_ascii_lowercase()
}

/// Registers `img` as an `Image` item of the `RegisteredResources` package (created when
/// missing) and stores its file; returns the item name. The same bytes reuse their file.
fn register_image(r: &mut Report, img: ImageIn) -> Result<String, String> {
    let file = resource_file(&img);
    register_item(r, file, img.bytes, IMAGE_ITEM)
}

/// Registers `bytes` as the file `file`, an item of type `item_type` (`Image`, `ShapeMap`) of the
/// `RegisteredResources` package (created when missing); returns the item name. The same file
/// name reuses its item.
pub(crate) fn register_item(
    r: &mut Report,
    file: String,
    bytes: Vec<u8>,
    item_type: &str,
) -> Result<String, String> {
    let mut list = packages(r)?;
    let idx = match list.iter().position(|p| {
        let p = p.get("resourcePackage").unwrap_or(p);
        p.get("type").and_then(Value::as_str) == Some(REGISTERED)
            && p.get("name").and_then(Value::as_str).is_some()
    }) {
        Some(i) => i,
        None => {
            list.push(json!({"name": REGISTERED, "type": REGISTERED, "items": []}));
            list.len().saturating_sub(1)
        }
    };
    let pkg = list.get_mut(idx).ok_or("internal: missing resource package")?;
    let pkg_name = pkg
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or(REGISTERED)
        .to_string();
    if pkg_name.contains("..") || pkg_name.contains('/') || pkg_name.contains('\\') {
        return Err(format!(
            "resource package `{}` has an unsafe name",
            clip(&pkg_name, 60)
        ));
    }
    if pkg.get("items").is_none() {
        pkg.as_object_mut()
            .ok_or("a resource package is not an object")?
            .insert("items".into(), json!([]));
    }
    let items = pkg
        .get_mut("items")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| format!("resource package `{pkg_name}` has no item list"))?;
    if !items.iter().any(|i| {
        i.get("name")
            .and_then(Value::as_str)
            .is_some_and(|n| n.eq_ignore_ascii_case(&file))
    }) {
        items.push(json!({"name": file, "path": file, "type": item_type}));
    }
    set_packages(r, list);
    let path = format!("StaticResources/{pkg_name}/{file}");
    let n = norm(&path);
    match r.resources.iter_mut().find(|x| norm(&x.path) == n) {
        Some(x) => x.bytes = bytes,
        None => r.resources.push(Resource { path, bytes }),
    }
    Ok(file)
}

pub(crate) fn resource_url(item: &str) -> Value {
    json!({"expr": {"ResourcePackageItem": {"PackageName": REGISTERED, "PackageType": 1, "ItemName": item}}})
}

/// `ItemName`s of every `ResourcePackageItem` of the registered package in `v`.
pub(crate) fn referenced(v: &Value, out: &mut Vec<String>, depth: usize) {
    if depth > 64 {
        return;
    }
    match v {
        Value::Object(o) => {
            if let Some(item) = o.get("ResourcePackageItem")
                && item.get("PackageName").and_then(Value::as_str) == Some(REGISTERED)
                && let Some(n) = item.get("ItemName").and_then(Value::as_str)
            {
                out.push(n.to_ascii_lowercase());
            }
            for x in o.values() {
                referenced(x, out, depth.saturating_add(1));
            }
        }
        Value::Array(a) => {
            for x in a {
                referenced(x, out, depth.saturating_add(1));
            }
        }
        _ => {}
    }
}

/// Drops the registered `Image` items (and their files) among `candidates` that nothing in the
/// report references any more (an image replaced by `image.set` or `page.format`).
fn drop_unreferenced(r: &mut Report, candidates: &[String]) {
    drop_unreferenced_items(r, candidates, IMAGE_ITEM);
}

/// Drops the registered items of type `item_type` (and their files) among `candidates` that
/// nothing in the report references any more.
pub(crate) fn drop_unreferenced_items(r: &mut Report, candidates: &[String], item_type: &str) {
    if candidates.is_empty() {
        return;
    }
    let mut used = Vec::new();
    for p in &r.pages {
        if let Some(o) = p.extra.get("objects") {
            referenced(o, &mut used, 0);
        }
        for v in &p.visuals {
            for x in v.visual_extra.values() {
                referenced(x, &mut used, 0);
            }
        }
    }
    if let Some(o) = r.extra.get("objects") {
        referenced(o, &mut used, 0);
    }
    let stale: Vec<String> = candidates
        .iter()
        .map(|c| c.to_ascii_lowercase())
        .filter(|c| !used.contains(c))
        .collect();
    if stale.is_empty() {
        return;
    }
    let Ok(mut list) = packages(r) else { return };
    let mut paths = Vec::new();
    for pkg in &mut list {
        let name = pkg.get("name").and_then(Value::as_str).unwrap_or("").to_string();
        if pkg.get("type").and_then(Value::as_str) != Some(REGISTERED) {
            continue;
        }
        if let Some(items) = pkg.get_mut("items").and_then(Value::as_array_mut) {
            items.retain(|i| {
                let n = i
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_ascii_lowercase();
                let drop = i.get("type").and_then(Value::as_str) == Some(item_type) && stale.contains(&n);
                if drop && let Some(path) = i.get("path").and_then(Value::as_str) {
                    paths.push(norm(&format!("StaticResources/{name}/{path}")));
                }
                !drop
            });
        }
    }
    set_packages(r, list);
    r.resources.retain(|res| !paths.contains(&norm(&res.path)));
}

fn scaling_param(p: &Value, key: &str) -> Result<Option<&'static str>, String> {
    match opt_str(p, key)? {
        None => Ok(None),
        Some(s) => match s.to_ascii_lowercase().as_str() {
            "normal" => Ok(Some("Normal")),
            "fit" => Ok(Some("Fit")),
            "fill" => Ok(Some("Fill")),
            _ => Err(format!("`{key}` must be normal, fit or fill")),
        },
    }
}

fn set_image_props(v: &mut Visual, item: Option<&str>, scaling: Option<&str>) -> Result<(), String> {
    if let Some(item) = item {
        let general = object_list(v, "objects", "general")?;
        plain_props(general)?.insert("imageUrl".into(), resource_url(item));
    }
    if let Some(sc) = scaling {
        let list = object_list(v, "objects", "imageScaling")?;
        plain_props(list)?.insert("imageScalingType".into(), text_lit(sc));
    }
    Ok(())
}

/// The image item a visual shows (`general.imageUrl`).
fn image_item(v: &Visual) -> Option<String> {
    let mut out = Vec::new();
    referenced(v.visual_extra.get("objects")?.get("general")?, &mut out, 0);
    out.into_iter().next()
}

/// `image.add {page, path | data, name?, x?, y?, width?, height?, scaling?}`.
pub(crate) fn image_add(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "page", "path", "data", "name", "x", "y", "width", "height", "scaling",
        ],
    )?;
    let key = str_param(p, "page")?;
    let scaling = scaling_param(p, "scaling")?;
    let mut warnings = Vec::new();
    let img = image_param(p, "path", "data", &mut warnings)?
        .ok_or("image.add needs `path` (a file) or `data` (base64)")?;
    let (_, r) = parts(s)?;
    let pi = page_index(r, key)?;
    let page = r.pages.get(pi).ok_or("internal: page index out of range")?;
    // the natural size, within half the page
    let pw = if page.width > 0.0 { page.width } else { 1280.0 };
    let ph = if page.height > 0.0 { page.height } else { 720.0 };
    let k = (pw / 2.0 / img.width).min(ph / 2.0 / img.height).min(1.0);
    let (w, h) = (
        (img.width * k).round().max(1.0),
        (img.height * k).round().max(1.0),
    );
    let position = place(page, p, w, h)?;
    let (mime, nat_w, nat_h) = (img.mime, img.width, img.height);
    let item = register_image(r, img)?;
    let mut v = Visual {
        position,
        visual_type: Some("image".into()),
        ..Default::default()
    };
    set_image_props(&mut v, Some(&item), scaling)?;
    let mut out = insert(r, pi, v, warnings)?;
    if let Some(o) = out.as_object_mut() {
        o.insert("resource".into(), json!(item));
        o.insert("mime".into(), json!(mime));
        o.insert("image_size".into(), json!({"width": nat_w, "height": nat_h}));
    }
    s.report_dirty = true;
    Ok(out)
}

/// `image.set {page, visual, path? | data?, name?, scaling?}`.
pub(crate) fn image_set(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "visual", "path", "data", "name", "scaling"])?;
    let key = str_param(p, "page")?;
    let visual = str_param(p, "visual")?;
    let scaling = scaling_param(p, "scaling")?;
    let mut warnings = Vec::new();
    let img = image_param(p, "path", "data", &mut warnings)?;
    if img.is_none() && scaling.is_none() {
        return Err("image.set needs an image (`path` or `data`) or `scaling`".into());
    }
    let (_, r) = parts(s)?;
    let pi = page_index(r, key)?;
    let page = r.pages.get(pi).ok_or("internal: page index out of range")?;
    let vi = visual_index(page, visual)?;
    let v = page
        .visuals
        .get(vi)
        .ok_or("internal: visual index out of range")?;
    if !v
        .visual_type
        .as_deref()
        .is_some_and(|t| t.eq_ignore_ascii_case("image"))
    {
        return Err(format!(
            "visual `{visual}` is a {}, not an image",
            v.visual_type.as_deref().unwrap_or("group")
        ));
    }
    let old = image_item(v);
    let before = r.clone();
    let item = img.map(|i| register_image(r, i)).transpose()?;
    let page = page_at_mut(r, pi)?;
    let v = page
        .visuals
        .get_mut(vi)
        .ok_or("internal: visual index out of range")?;
    set_image_props(v, item.as_deref(), scaling)?;
    if let Some(old) = old
        && item.as_deref().is_some_and(|n| !n.eq_ignore_ascii_case(&old))
    {
        drop_unreferenced(r, &[old]);
    }
    let changed = *r != before;
    if changed {
        s.report_dirty = true;
    }
    Ok(json!({"visual": visual, "resource": item, "changed": changed, "warnings": warnings}))
}

// ---------------------------------------------------------------------------------------------
// Buttons and actions
// ---------------------------------------------------------------------------------------------

/// Desktop's Insert › Buttons: (kind, visual type, `icon.shapeType`, action type). The icon names
/// and action types are the public report theme schema's (`shapeType`, `visualLink.type`).
const BUTTONS: &[(&str, &str, &str, Option<&str>)] = &[
    ("left_arrow", "actionButton", "leftArrow", None),
    ("right_arrow", "actionButton", "rightArrow", None),
    ("reset", "actionButton", "reset", None),
    ("back", "actionButton", "back", Some("Back")),
    ("information", "actionButton", "information", None),
    ("help", "actionButton", "help", None),
    ("qna", "actionButton", "qna", Some("Qna")),
    ("bookmark", "actionButton", "bookmarks", Some("Bookmark")),
    ("blank", "actionButton", "blank", None),
    (
        "apply_all_slicers",
        "actionButton",
        "applyAllSlicers",
        Some("ApplyAllSlicers"),
    ),
    (
        "clear_all_slicers",
        "actionButton",
        "clearAllSlicers",
        Some("ClearAllSlicers"),
    ),
    ("page_navigator", "pageNavigator", "", None),
    ("bookmark_navigator", "bookmarkNavigator", "", None),
];

/// The kinds `button.add` takes, for its form.
pub const BUTTON_KINDS: &[&str] = &[
    "left_arrow",
    "right_arrow",
    "reset",
    "back",
    "information",
    "help",
    "qna",
    "bookmark",
    "blank",
    "apply_all_slicers",
    "clear_all_slicers",
    "page_navigator",
    "bookmark_navigator",
];

/// The action types `visual.set_action` takes: (parameter value, `visualLink.type`).
const ACTIONS: &[(&str, &str)] = &[
    ("back", "Back"),
    ("bookmark", "Bookmark"),
    ("drillthrough", "Drillthrough"),
    ("page_navigation", "PageNavigation"),
    ("qna", "Qna"),
    ("web_url", "WebUrl"),
    ("apply_all_slicers", "ApplyAllSlicers"),
    ("clear_all_slicers", "ClearAllSlicers"),
];

/// The action types for the form (`none` turns the action off).
pub const ACTION_TYPES: &[&str] = &[
    "none",
    "back",
    "bookmark",
    "drillthrough",
    "page_navigation",
    "qna",
    "web_url",
    "apply_all_slicers",
    "clear_all_slicers",
];

/// Visual types that carry an action in Desktop (Format › Action).
fn takes_action(t: Option<&str>) -> bool {
    t.is_some_and(|t| {
        ["actionButton", "image", "shape", "basicShape"]
            .iter()
            .any(|x| x.eq_ignore_ascii_case(t))
    })
}

/// `button.add {page, kind, x?, y?, width?, height?}`.
pub(crate) fn button_add(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "kind", "x", "y", "width", "height"])?;
    let key = str_param(p, "page")?;
    let kind = str_param(p, "kind")?
        .to_ascii_lowercase()
        .replace([' ', '-'], "_");
    let &(_, vt, icon, action) = BUTTONS
        .iter()
        .find(|b| b.0 == kind)
        .ok_or_else(|| format!("`kind` must be one of {}", BUTTON_KINDS.join(", ")))?;
    let (_, r) = parts(s)?;
    let pi = page_index(r, key)?;
    let page = r.pages.get(pi).ok_or("internal: page index out of range")?;
    let (w, h) = if vt == "actionButton" {
        (100.0, 40.0)
    } else {
        (400.0, 60.0)
    };
    let position = place(page, p, w, h)?;
    let mut v = Visual {
        position,
        visual_type: Some(vt.into()),
        ..Default::default()
    };
    if !icon.is_empty() {
        let list = object_list(&mut v, "objects", "icon")?;
        list.push(json!({"properties": {"shapeType": text_lit(icon)}, "selector": {"id": "default"}}));
    }
    if let Some(a) = action {
        let list = object_list(&mut v, "visualContainerObjects", "visualLink")?;
        list.push(json!({"properties": {"show": bool_lit(true), "type": text_lit(a)}}));
    }
    let mut out = insert(r, pi, v, Vec::new())?;
    if let Some(o) = out.as_object_mut() {
        o.insert("kind".into(), json!(kind));
        o.insert("action".into(), json!(action));
    }
    s.report_dirty = true;
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Shapes
// ---------------------------------------------------------------------------------------------

/// The shapes `shape.add` inserts: the `tileShape` values of the `shape` visual's `shape` object
/// in Microsoft's public report theme schema (`visual-shape`), in Insert › Shapes' order.
pub const SHAPE_TYPES: &[&str] = &[
    "rectangle",
    "rectangleRounded",
    "rectangleRoundedByPixel",
    "oval",
    "line",
    "triangleIsoc",
    "triangleRight",
    "arrow",
    "arrowChevron",
    "arrowPentagon",
    "pentagon",
    "hexagon",
    "octagon",
    "parallelogram",
    "trapezoid",
    "pill",
    "heart",
    "speechbubbleRectangle",
    "tabCutCorner",
    "tabCutTopCorners",
    "tabCutTopCornersByPixel",
    "tabRoundCorner",
    "tabRoundTopCorners",
];

/// `shape.add {page, shape, x?, y?, width?, height?}`: Insert › Shapes. A `shape` visual whose
/// `objects.shape` entry without selector names the tile shape (Desktop reads that object
/// without selector, `docs/formats/visual-options.md`); fill, outline and rotation stay the
/// defaults until the Format pane sets them (`visual.format`).
pub(crate) fn shape_add(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "shape", "x", "y", "width", "height"])?;
    let key = str_param(p, "page")?;
    let given = str_param(p, "shape")?;
    let shape = SHAPE_TYPES
        .iter()
        .copied()
        .find(|t| t.eq_ignore_ascii_case(given.trim()))
        .ok_or_else(|| format!("`shape` must be one of {}", SHAPE_TYPES.join(", ")))?;
    let (_, r) = parts(s)?;
    let pi = page_index(r, key)?;
    let page = r.pages.get(pi).ok_or("internal: page index out of range")?;
    let (w, h) = if shape == "line" {
        (300.0, 40.0)
    } else {
        (300.0, 200.0)
    };
    let position = place(page, p, w, h)?;
    let mut v = Visual {
        position,
        visual_type: Some("shape".into()),
        ..Default::default()
    };
    let list = object_list(&mut v, "objects", "shape")?;
    list.push(json!({"properties": {"tileShape": text_lit(shape)}}));
    let mut out = insert(r, pi, v, Vec::new())?;
    if let Some(o) = out.as_object_mut() {
        o.insert("shape".into(), json!(shape));
    }
    s.report_dirty = true;
    Ok(out)
}

/// `visual.set_action {page, visual, type, destination?, bookmark?, url?, tooltip?}`.
pub(crate) fn visual_set_action(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "page",
            "visual",
            "type",
            "destination",
            "bookmark",
            "url",
            "tooltip",
            "destination_field",
            "tooltip_field",
            "enabled_tooltip",
            "disabled_tooltip",
        ],
    )?;
    let key = str_param(p, "page")?;
    let visual = str_param(p, "visual")?;
    let ty_in = str_param(p, "type")?
        .to_ascii_lowercase()
        .replace([' ', '-'], "_");
    let ty: Option<&str> = if ty_in == "none" || ty_in.is_empty() {
        None
    } else {
        Some(
            ACTIONS
                .iter()
                .find(|(k, v)| *k == ty_in || v.eq_ignore_ascii_case(&ty_in))
                .map(|(_, v)| *v)
                .ok_or_else(|| format!("`type` must be one of {}", ACTION_TYPES.join(", ")))?,
        )
    };
    let destination = bounded_text(p, "destination", MAX_NAME)?;
    let bookmark = bounded_text(p, "bookmark", MAX_NAME)?;
    let url = opt_str(p, "url")?;
    let tooltip = bounded_text(p, "tooltip", MAX_TOOLTIP)?;
    let enabled_tip = bounded_text(p, "enabled_tooltip", MAX_TOOLTIP)?;
    let disabled_tip = bounded_text(p, "disabled_tooltip", MAX_TOOLTIP)?;
    let mut warnings = Vec::new();
    // conditional (fx) destination and tooltip: a measure, or a column read through Min
    // (Desktop's "First"), whose value names the page, bookmark or URL
    let dest_field = action_field(s, p, "destination_field", &mut warnings)?;
    let tip_field = action_field(s, p, "tooltip_field", &mut warnings)?;
    if dest_field.is_some() {
        if !matches!(
            ty,
            Some("PageNavigation" | "Drillthrough" | "Bookmark" | "WebUrl")
        ) {
            return Err(format!(
                "`destination_field` applies to page_navigation, drillthrough, bookmark and web_url, not `{ty_in}`"
            ));
        }
        if destination.is_some() || bookmark.is_some() || url.is_some() {
            return Err("pass a fixed destination (`destination`, `bookmark`, `url`) or `destination_field`, not both".into());
        }
    }
    if tip_field.is_some() && tooltip.is_some() {
        return Err("pass `tooltip` or `tooltip_field`, not both".into());
    }
    if ty.is_none()
        && (dest_field.is_some() || tip_field.is_some() || enabled_tip.is_some() || disabled_tip.is_some())
    {
        return Err("type `none` turns the action off: it takes no destination or tooltip".into());
    }
    let r = report_ref(s)?;
    let pi = page_index(r, key)?;
    // what the type takes
    let needs = |k: &str| match ty {
        Some("PageNavigation" | "Drillthrough") => k == "destination",
        Some("Bookmark") => k == "bookmark",
        Some("WebUrl") => k == "url",
        _ => false,
    };
    for (k, given) in [
        ("destination", destination.is_some()),
        ("bookmark", bookmark.is_some()),
        ("url", url.is_some()),
    ] {
        if given && !needs(k) {
            return Err(format!("`{k}` does not apply to the action type `{}`", ty_in));
        }
    }
    let target_page = match (ty, destination) {
        (Some("PageNavigation" | "Drillthrough"), Some(d)) if !d.is_empty() => {
            let ti = page_index(r, d)?;
            let t = r.pages.get(ti).ok_or("internal: page index out of range")?;
            let kind = t.extra.get("type").and_then(Value::as_str);
            if ty == Some("Drillthrough") && kind != Some("Drillthrough") {
                return Err(format!(
                    "page `{}` is not a drillthrough page (page.set_type makes one)",
                    t.display_name
                ));
            }
            if ty == Some("PageNavigation") && kind == Some("Tooltip") {
                warnings.push(format!("page `{}` is a tooltip page", t.display_name));
            }
            Some(t.name.clone())
        }
        _ => None,
    };
    if let (Some("Bookmark"), Some(b)) = (ty, bookmark)
        && !b.is_empty()
        && !r.bookmarks.iter().any(|x| x.name == b)
    {
        warnings.push(format!("the report has no bookmark `{}`", clip(b, 80)));
    }
    let url = match (ty, url) {
        (Some("WebUrl"), Some(u)) if !u.trim().is_empty() => Some(check_url("url", u)?),
        _ => None,
    };
    let (_, r) = parts(s)?;
    let page = page_at_mut(r, pi)?;
    let vi = visual_index(page, visual)?;
    let v = page
        .visuals
        .get_mut(vi)
        .ok_or("internal: visual index out of range")?;
    if !takes_action(v.visual_type.as_deref()) {
        return Err(format!(
            "visual `{visual}` is a {}: actions are on buttons, images and shapes",
            v.visual_type.as_deref().unwrap_or("group")
        ));
    }
    let before = v.clone();
    // the action lives in `visualContainerObjects`; an older file's `objects.visualLink` goes
    if let Some(o) = v.visual_extra.get_mut("objects").and_then(Value::as_object_mut) {
        o.shift_remove("visualLink");
    }
    let list = object_list(v, "visualContainerObjects", "visualLink")?;
    let props = plain_props(list)?;
    for k in [
        "type",
        "navigationSection",
        "drillthroughSection",
        "bookmark",
        "webUrl",
        "tooltip",
        "enabledTooltip",
        "disabledTooltip",
    ] {
        props.shift_remove(k);
    }
    props.insert("show".into(), bool_lit(ty.is_some()));
    if let Some(t) = ty {
        props.insert("type".into(), text_lit(t));
    }
    match (ty, &target_page) {
        (Some("PageNavigation"), Some(t)) => {
            props.insert("navigationSection".into(), text_lit(t));
        }
        (Some("Drillthrough"), Some(t)) => {
            props.insert("drillthroughSection".into(), text_lit(t));
        }
        _ => {}
    }
    if let (Some("Bookmark"), Some(b)) = (ty, bookmark)
        && !b.is_empty()
    {
        props.insert("bookmark".into(), text_lit(b));
    }
    if let Some(u) = &url {
        props.insert("webUrl".into(), text_lit(u));
    }
    if let Some(t) = tooltip.filter(|t| !t.is_empty()) {
        props.insert("tooltip".into(), text_lit(t));
    }
    if let (Some(t), Some(f)) = (ty, &dest_field) {
        let prop = match t {
            "PageNavigation" => "navigationSection",
            "Drillthrough" => "drillthroughSection",
            "Bookmark" => "bookmark",
            _ => "webUrl",
        };
        props.insert(prop.into(), f.clone());
    }
    if let Some(f) = &tip_field {
        props.insert("tooltip".into(), f.clone());
    }
    for (k, t) in [("enabledTooltip", enabled_tip), ("disabledTooltip", disabled_tip)] {
        if let Some(t) = t.filter(|t| !t.is_empty()) {
            props.insert(k.into(), text_lit(t));
        }
    }
    let changed = *v != before;
    if changed {
        s.report_dirty = true;
    }
    Ok(json!({
        "visual": visual,
        "type": ty,
        "destination": target_page,
        "conditional": dest_field.is_some(),
        "changed": changed,
        "warnings": warnings,
    }))
}

/// The `{expr}` of an fx field parameter (`[Measure]`, `Table[Column]` or `Function(Table[Column])`):
/// a column is read through `Min`, Desktop's "First" for a text field.
fn action_field(
    s: &Session,
    p: &Value,
    key: &str,
    warnings: &mut Vec<String>,
) -> Result<Option<Value>, String> {
    let Some(text) = bounded_text(p, key, MAX_NAME)? else {
        return Ok(None);
    };
    if text.trim().is_empty() {
        return Ok(None);
    }
    let r = crate::refs::resolve_field(s.model.as_ref(), text).map_err(|e| format!("`{key}`: {e}"))?;
    warnings.extend(r.warnings);
    let f = match r.field {
        FieldRef::Column { table, column } => FieldRef::Aggregation {
            table,
            column,
            function: "Min".into(),
        },
        f @ (FieldRef::Measure { .. } | FieldRef::Aggregation { .. }) => f,
        FieldRef::Other { .. } => return Err(format!("`{key}` must be a measure or a column")),
    };
    let e = crate::visual_props::field_to_json(&f, None)
        .ok_or_else(|| format!("`{key}`: {} cannot be written", field_text(&f)))?;
    Ok(Some(json!({ "expr": e })))
}

/// The field text of an fx property (`{expr: {Measure | Aggregation}}`), else `None`.
fn expr_field_text(v: Option<&Value>) -> Option<String> {
    let e = v?.get("expr")?;
    if e.get("Literal").is_some() {
        return None;
    }
    let f = bicraft_report::query::field_ref(Some(e));
    matches!(f, FieldRef::Measure { .. } | FieldRef::Aggregation { .. }).then(|| field_text(&f))
}

/// The action of a visual for the UI's editor (`visualLink`, the entry without selector).
pub(crate) fn action_json(v: &Visual) -> Option<Value> {
    let list = v
        .visual_extra
        .get("visualContainerObjects")
        .and_then(|o| o.get("visualLink"))
        .or_else(|| v.visual_extra.get("objects").and_then(|o| o.get("visualLink")))?
        .as_array()?;
    let props = list
        .iter()
        .find(|e| e.get("selector").is_none_or(Value::is_null))?
        .get("properties")?;
    let mut o = Map::new();
    let show = lit_text(props.get("show")).is_none_or(|s| s != "false");
    let ty = lit_text(props.get("type"));
    let kind = ty.as_deref().and_then(|t| {
        ACTIONS
            .iter()
            .find(|(_, v)| v.eq_ignore_ascii_case(t))
            .map(|(k, _)| *k)
    });
    o.insert("type".into(), json!(if show { kind } else { None }));
    for (k, prop) in [
        ("destination", "navigationSection"),
        ("drillthrough", "drillthroughSection"),
        ("bookmark", "bookmark"),
        ("url", "webUrl"),
        ("tooltip", "tooltip"),
        ("enabled_tooltip", "enabledTooltip"),
        ("disabled_tooltip", "disabledTooltip"),
    ] {
        if let Some(x) = lit_text(props.get(prop)) {
            let x: String = x.chars().take(MAX_URL).collect();
            if k == "drillthrough" {
                o.insert("destination".into(), json!(x));
            } else {
                o.insert(k.into(), json!(x));
            }
        }
    }
    // the fx fields of a conditional destination and tooltip
    for prop in ["navigationSection", "drillthroughSection", "bookmark", "webUrl"] {
        if let Some(f) = expr_field_text(props.get(prop)) {
            o.insert("destination_field".into(), json!(f));
        }
    }
    if let Some(f) = expr_field_text(props.get("tooltip")) {
        o.insert("tooltip_field".into(), json!(f));
    }
    Some(Value::Object(o))
}

// ---------------------------------------------------------------------------------------------
// Page format
// ---------------------------------------------------------------------------------------------

/// Desktop's page sizes (Format › Canvas settings › Type), in report pixels.
const PAGE_SIZES: &[(&str, f64, f64)] = &[
    ("16:9", 1280.0, 720.0),
    ("4:3", 960.0, 720.0),
    ("letter", 816.0, 1056.0),
    ("tooltip", 320.0, 240.0),
];

pub const PAGE_SIZE_NAMES: &[&str] = &["16:9", "4:3", "letter", "tooltip", "custom"];

/// The page object list `objects.<name>` (created when missing).
fn page_object<'a>(page: &'a mut Page, name: &str) -> Result<&'a mut Vec<Value>, String> {
    let objects = page
        .extra
        .entry("objects".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !objects.is_object() {
        return Err("page.json `objects` is not an object".into());
    }
    let objects = objects.as_object_mut().ok_or("internal: not an object")?;
    let list = objects
        .entry(name.to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    if !list.is_array() {
        *list = Value::Array(Vec::new());
    }
    list.as_array_mut()
        .ok_or_else(|| "internal: not a list".to_string())
}

/// Removes empty object lists and an empty `objects`.
fn tidy_objects(page: &mut Page) {
    if let Some(o) = page.extra.get_mut("objects").and_then(Value::as_object_mut) {
        for list in o.values_mut() {
            if let Some(a) = list.as_array_mut() {
                a.retain(|e| {
                    e.get("properties")
                        .and_then(Value::as_object)
                        .is_none_or(|p| !p.is_empty())
                        || e.get("selector").is_some()
                });
            }
        }
        o.retain(|_, v| v.as_array().is_none_or(|a| !a.is_empty()));
        if o.is_empty() {
            page.extra.shift_remove("objects");
        }
    }
}

/// One canvas layer (`background` or `outspace`) of `page.format`.
struct Layer {
    object: &'static str,
    prefix: &'static str,
    color: Option<Option<String>>,
    transparency: Option<f64>,
    image: Option<Option<ImageIn>>,
    scaling: Option<&'static str>,
}

fn layer_params(
    p: &Value,
    object: &'static str,
    prefix: &'static str,
    warnings: &mut Vec<String>,
) -> Result<Layer, String> {
    let color = match p.get(format!("{prefix}_color").as_str()) {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(c)) if c.is_empty() => Some(None),
        Some(Value::String(c)) => Some(Some(check_color(&format!("{prefix}_color"), c)?)),
        Some(_) => return Err(format!("`{prefix}_color` must be a colour like #FFFFFF or null")),
    };
    let tk = format!("{prefix}_transparency");
    let transparency = opt_num(p, &tk)?;
    if transparency.is_some_and(|t| !(0.0..=100.0).contains(&t)) {
        return Err(format!("`{tk}` must be from 0 to 100"));
    }
    let ik = format!("{prefix}_image");
    let pk = format!("{prefix}_image_path");
    let cleared =
        matches!(p.get(ik.as_str()), Some(Value::Null)) || matches!(p.get(pk.as_str()), Some(Value::Null));
    let image = match image_param(p, &pk, &ik, warnings)? {
        Some(i) => Some(Some(i)),
        None if cleared => Some(None),
        None => None,
    };
    Ok(Layer {
        object,
        prefix,
        color,
        transparency,
        image,
        scaling: scaling_param(p, &format!("{prefix}_scaling"))?,
    })
}

/// `page.format {page, background_*?, wallpaper_*?, size?, width?, height?, vertical_alignment?}`.
pub(crate) fn page_format(s: &mut Session, p: &Value) -> Result<Value, String> {
    let mut allowed = vec!["page", "size", "width", "height", "vertical_alignment"];
    allowed.extend([
        "background_color",
        "background_transparency",
        "background_image",
        "background_image_path",
        "background_scaling",
        "wallpaper_color",
        "wallpaper_transparency",
        "wallpaper_image",
        "wallpaper_image_path",
        "wallpaper_scaling",
    ]);
    allow(p, &allowed)?;
    // `name` is not taken here: the file name of a path names the resource
    let key = str_param(p, "page")?;
    let mut warnings = Vec::new();
    let layers = [
        layer_params(p, "background", "background", &mut warnings)?,
        layer_params(p, "outspace", "wallpaper", &mut warnings)?,
    ];
    let size = opt_str(p, "size")?.map(str::to_ascii_lowercase);
    let width = opt_num(p, "width")?.map(|v| check_size("width", v)).transpose()?;
    let height = opt_num(p, "height")?
        .map(|v| check_size("height", v))
        .transpose()?;
    let dims = match size.as_deref() {
        None if width.is_some() || height.is_some() => Some((width, height)),
        None => None,
        Some("custom") => {
            if width.is_none() && height.is_none() {
                return Err("a custom page size needs `width` and/or `height`".into());
            }
            Some((width, height))
        }
        Some(sz) => {
            let &(_, w, h) = PAGE_SIZES
                .iter()
                .find(|x| x.0 == sz || (sz == "16_9" && x.0 == "16:9") || (sz == "4_3" && x.0 == "4:3"))
                .ok_or_else(|| format!("`size` must be one of {}", PAGE_SIZE_NAMES.join(", ")))?;
            if width.is_some() || height.is_some() {
                return Err("`width` and `height` go with `size: custom`".into());
            }
            Some((Some(w), Some(h)))
        }
    };
    let valign = match opt_str(p, "vertical_alignment")?
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        None => None,
        Some("top") => Some("Top"),
        Some("middle") => Some("Middle"),
        Some(_) => return Err("`vertical_alignment` must be top or middle".into()),
    };
    let nothing = dims.is_none()
        && valign.is_none()
        && layers.iter().all(|l| {
            l.color.is_none() && l.transparency.is_none() && l.image.is_none() && l.scaling.is_none()
        });
    if nothing {
        return Err("page.format needs something to change (background_*, wallpaper_*, size, width, height, vertical_alignment)".into());
    }
    let (_, r) = parts(s)?;
    let pi = page_index(r, key)?;
    let before = r.clone();
    let mut old_items = Vec::new();
    let mut registered = Vec::new();
    for l in layers {
        let item = match l.image {
            Some(Some(img)) => Some(Some(register_image(r, img)?)),
            Some(None) => Some(None),
            None => None,
        };
        let page = page_at_mut(r, pi)?;
        let list = page_object(page, l.object)?;
        let props = plain_props(list)?;
        match l.color {
            Some(Some(c)) => {
                props.insert("color".into(), color_lit(&c));
            }
            Some(None) => {
                props.shift_remove("color");
            }
            None => {}
        }
        if let Some(t) = l.transparency {
            props.insert("transparency".into(), num_lit(t));
        }
        if item.is_some() || l.scaling.is_some() {
            let mut prev = Vec::new();
            if let Some(i) = props.get("image") {
                referenced(i, &mut prev, 0);
            }
            match item {
                Some(None) => {
                    props.shift_remove("image");
                    old_items.extend(prev);
                }
                Some(Some(name)) => {
                    let mut inner = Map::new();
                    inner.insert("name".into(), text_lit(&name));
                    inner.insert("url".into(), resource_url(&name));
                    let sc = l
                        .scaling
                        .map(str::to_string)
                        .or_else(|| {
                            props
                                .get("image")
                                .and_then(|i| i.get("image"))
                                .and_then(|i| lit_text(i.get("scaling")))
                        })
                        .unwrap_or_else(|| "Normal".to_string());
                    inner.insert("scaling".into(), text_lit(&sc));
                    props.insert("image".into(), json!({"image": inner}));
                    old_items.extend(prev.into_iter().filter(|x| !x.eq_ignore_ascii_case(&name)));
                    registered.push((l.prefix, name));
                }
                None => {
                    // a new scaling for the image there
                    match props
                        .get_mut("image")
                        .and_then(|i| i.get_mut("image"))
                        .and_then(Value::as_object_mut)
                    {
                        Some(inner) => {
                            if let Some(sc) = l.scaling {
                                inner.insert("scaling".into(), text_lit(sc));
                            }
                        }
                        None => warnings.push(format!(
                            "`{}_scaling`: the {} has no image",
                            l.prefix,
                            if l.prefix == "background" {
                                "page background"
                            } else {
                                "wallpaper"
                            }
                        )),
                    }
                }
            }
        }
    }
    let page = page_at_mut(r, pi)?;
    if let Some((w, h)) = dims {
        if let Some(w) = w {
            page.width = w;
        }
        if let Some(h) = h {
            page.height = h;
        }
    }
    if let Some(a) = valign {
        let list = page_object(page, "displayArea")?;
        plain_props(list)?.insert("verticalAlignment".into(), text_lit(a));
    }
    tidy_objects(page);
    let (pw, ph, name) = (page.width, page.height, page.name.clone());
    drop_unreferenced(r, &old_items);
    let changed = *r != before;
    if changed {
        s.report_dirty = true;
    }
    let images: Map<String, Value> = registered
        .into_iter()
        .map(|(k, v)| (k.to_string(), json!(v)))
        .collect();
    Ok(json!({
        "page": name,
        "width": pw,
        "height": ph,
        "images": images,
        "changed": changed,
        "warnings": warnings,
    }))
}

/// `verticalAlignment` of the page (`objects.displayArea`), lower case.
pub(crate) fn vertical_alignment(page: &Page) -> Option<String> {
    let list = page.extra.get("objects")?.get("displayArea")?.as_array()?;
    let props = list
        .iter()
        .find(|e| e.get("selector").is_none_or(Value::is_null))?
        .get("properties")?;
    let a = lit_text(props.get("verticalAlignment"))?.to_ascii_lowercase();
    matches!(a.as_str(), "top" | "middle").then_some(a)
}

/// The size name of a page (`16:9`…), `custom` otherwise.
pub(crate) fn size_name(page: &Page) -> &'static str {
    PAGE_SIZES
        .iter()
        .find(|(_, w, h)| (page.width - w).abs() < 0.5 && (page.height - h).abs() < 0.5)
        .map_or("custom", |x| x.0)
}

/// Whether a run of command `id` with `params` reads a file (its recovery needs a full copy).
pub(crate) fn reads_file(id: &str, params: &Value) -> bool {
    let keys: &[&str] = match id {
        "image.add" | "image.set" | "visual.import_custom" | "shape_map.set_map" => &["path"],
        "page.format" => &["background_image_path", "wallpaper_image_path"],
        _ => return false,
    };
    keys.iter().any(|k| {
        params
            .get(*k)
            .and_then(Value::as_str)
            .is_some_and(|s| !s.trim().is_empty())
    })
}

/// The dynamic values of a text box: selector id → field text (`[Total]`), for the snapshot.
pub(crate) fn text_values(v: &Visual) -> Vec<(String, String)> {
    v.visual_extra
        .get("objects")
        .and_then(|o| o.get("values"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(MAX_VALUES)
        .filter_map(|e| {
            let id = e.get("selector")?.get("id")?.as_str()?;
            let f = bicraft_report::query::field_ref(e.get("properties")?.get("expr")?.get("expr"));
            matches!(f, FieldRef::Measure { .. } | FieldRef::Aggregation { .. })
                .then(|| (id.chars().take(200).collect(), field_text(&f)))
        })
        .collect()
}
