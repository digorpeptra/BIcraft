//! Measure-bound visual properties (`docs/view-snapshot.md`, "Bound values"): the dynamic values
//! of text box runs, a button's text and fill, a visual's title and subtitle, the conditional
//! (fx) destination and tooltip of a button's, image's or shape's action. Desktop evaluates
//! each measure in the visual's filter context (report, page and visual filters, slicers) without
//! the visual's own grouping; the preview does the same through a field-less copy of the visual
//! that binds just those measures ([`synthetic`], evaluated by `preview::report_bound`).
//!
//! [`apply`] puts the values into the snapshot: text runs (`text.paragraphs[].runs[]` with a
//! `bound` selector id), `button.text` / `button.fill`, `title`, `format.subtitle.text`. When a
//! value cannot be evaluated the visual gets `preview_reason`, and the renderer marks it
//! `preview` instead of drawing it as if it were complete.

use bicraft_model::SemanticModel;
use bicraft_report::{FieldBinding, FieldRef, Visual};
use serde_json::{Map, Value, json};

/// Most bound values read for one visual.
const MAX_BOUND: usize = 50;
/// Longest bound text carried.
const MAX_TEXT: usize = 4000;

/// Where a bound value goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Slot {
    /// A text box run whose value is `{propertyIdentifier: {objectName: "values"}, selector: {id}}`.
    Run(String),
    ButtonText,
    ButtonFill,
    Title,
    Subtitle,
    /// A conditional (fx) action destination: `visualLink.navigationSection`,
    /// `drillthroughSection`, `bookmark` or `webUrl` (`action.dynamic_destination`).
    ActionDestination,
    /// A conditional action tooltip: `visualLink.tooltip` (`action.dynamic_tooltip`).
    ActionTooltip,
}

/// The entry of `<group>.<object>` for `state` (`selector.id`), else the one without a selector.
fn entry<'a>(group: Option<&'a Value>, object: &str, state: Option<&str>) -> Option<&'a Map<String, Value>> {
    let list = group?.get(object)?.as_array()?;
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

/// The field of `{"expr": {"Measure" | "Aggregation": …}}`; other expressions are not bound.
fn expr_field(prop: Option<&Value>) -> Option<FieldRef> {
    let e = prop?.get("expr")?;
    let f = bicraft_report::query::field_ref(Some(e));
    matches!(f, FieldRef::Measure { .. } | FieldRef::Aggregation { .. }).then_some(f)
}

/// Every measure-bound property of `v`, in a stable order.
pub(crate) fn fields(v: &Visual) -> Vec<(Slot, FieldRef)> {
    let mut out = Vec::new();
    let objects = v.visual_extra.get("objects");
    let container = v.visual_extra.get("visualContainerObjects");
    let ty = v.visual_type.as_deref().unwrap_or("");
    if ty.eq_ignore_ascii_case("textbox") {
        for e in objects
            .and_then(|o| o.get("values"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(id) = e
                .get("selector")
                .and_then(|s| s.get("id"))
                .and_then(Value::as_str)
            else {
                continue;
            };
            if let Some(f) = expr_field(e.get("properties").and_then(|p| p.get("expr"))) {
                out.push((Slot::Run(id.to_string()), f));
            }
        }
    }
    if ty.eq_ignore_ascii_case("actionButton") {
        if let Some(f) = expr_field(entry(objects, "text", Some("default")).and_then(|t| t.get("text"))) {
            out.push((Slot::ButtonText, f));
        }
        let fill = entry(objects, "fill", Some("default"))
            .and_then(|f| f.get("fillColor"))
            .and_then(|c| c.get("solid"))
            .and_then(|s| s.get("color"));
        if let Some(f) = expr_field(fill) {
            out.push((Slot::ButtonFill, f));
        }
    }
    // a button's, image's or shape's conditional action destination and tooltip
    if let Some(link) = entry(container, "visualLink", None).or_else(|| entry(objects, "visualLink", None)) {
        for prop in ["navigationSection", "drillthroughSection", "bookmark", "webUrl"] {
            if let Some(f) = expr_field(link.get(prop)) {
                out.push((Slot::ActionDestination, f));
                break;
            }
        }
        if let Some(f) = expr_field(link.get("tooltip")) {
            out.push((Slot::ActionTooltip, f));
        }
    }
    for (slot, object) in [(Slot::Title, "title"), (Slot::Subtitle, "subTitle")] {
        let text = entry(container, object, None)
            .or_else(|| entry(objects, object, None))
            .and_then(|t| t.get("text"));
        if let Some(f) = expr_field(text) {
            out.push((slot, f));
        }
    }
    out.truncate(MAX_BOUND);
    out
}

/// The distinct fields of `bound`, in order (the columns of the synthetic visual).
fn distinct(bound: &[(Slot, FieldRef)]) -> Vec<FieldRef> {
    let mut out: Vec<FieldRef> = Vec::new();
    for (_, f) in bound {
        if !out.contains(f) {
            out.push(f.clone());
        }
    }
    out
}

/// A copy of `v` that binds only its bound measures (one row, no grouping), with its filters.
pub(crate) fn synthetic(v: &Visual, bound: &[(Slot, FieldRef)]) -> Visual {
    let keep_type = crate::preview::filters::is_slicer(v);
    Visual {
        name: v.name.clone(),
        position: v.position,
        visual_type: Some(if keep_type {
            v.visual_type.clone().unwrap_or_default()
        } else {
            "card".to_string()
        }),
        fields: distinct(bound)
            .into_iter()
            .map(|field| FieldBinding {
                display_name: None,
                show_as: None,
                role: "Values".into(),
                field,
            })
            .collect(),
        parent_group: None,
        is_hidden: false,
        filter_config: v.filter_config.clone(),
        visual_extra: Default::default(),
        extra: Default::default(),
    }
}

/// The model's culture (for formatting bound numbers and dates), en-US by default.
fn locale(model: Option<&SemanticModel>) -> &'static bicraft_locale::Locale {
    model
        .and_then(|m| m.culture.as_deref())
        .and_then(bicraft_locale::Locale::find)
        .unwrap_or_else(bicraft_locale::Locale::en_us)
}

fn format_string(model: Option<&SemanticModel>, f: &FieldRef) -> Option<String> {
    let FieldRef::Measure { table, measure } = f else {
        return None;
    };
    let m = model?;
    let find = |t: Option<&str>| {
        m.tables
            .iter()
            .filter(|x| t.is_none_or(|t| x.name.eq_ignore_ascii_case(t)))
            .flat_map(|x| &x.measures)
            .find(|x| x.name.eq_ignore_ascii_case(measure))
    };
    find(Some(table)).or_else(|| find(None))?.format_string.clone()
}

/// A bound value as display text, formatted with the measure's format string.
fn display(v: &Value, fmt: Option<&str>, loc: &bicraft_locale::Locale) -> String {
    use bicraft_locale::vb;
    let f = fmt.unwrap_or("");
    let t = match v {
        Value::Null => String::new(),
        Value::Bool(b) => vb::format(vb::Value::Bool(*b), f, loc).unwrap_or_else(|| b.to_string()),
        Value::Number(n) => {
            let x = n.as_f64().unwrap_or(0.0);
            vb::format(vb::Value::Number(x), f, loc)
                .unwrap_or_else(|| bicraft_locale::number::general(x, '.'))
        }
        Value::String(s) if !f.is_empty() && vb::is_date_format(f) => {
            match crate::preview::filters::parse_as_of(s) {
                Ok(x) => vb::format(vb::Value::Date(x), f, loc).unwrap_or_else(|| s.clone()),
                Err(_) => s.clone(),
            }
        }
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    t.chars().take(MAX_TEXT).collect()
}

/// Puts the bound values of `v` (`data`: the synthetic visual's `visual.data`, `None` when it
/// was not computed) into its snapshot object `o`.
pub(crate) fn apply(
    o: &mut Map<String, Value>,
    v: &Visual,
    data: Option<&Value>,
    model: Option<&SemanticModel>,
) {
    let bound = fields(v);
    if bound.is_empty() {
        return;
    }
    let columns = distinct(&bound);
    let row = data
        .filter(|d| d.get("status").and_then(Value::as_str) == Some("ok"))
        .and_then(|d| d.get("rows"))
        .and_then(Value::as_array)
        .and_then(|r| r.first())
        .and_then(Value::as_array);
    let reason = match data {
        None => Some("the bound values were not computed within the time budget".to_string()),
        Some(d) if row.is_none() && d.get("status").and_then(Value::as_str) == Some("ok") => None,
        Some(d) if row.is_none() => Some(format!(
            "bound measure not evaluated: {}",
            d.get("reason").and_then(Value::as_str).unwrap_or("no data")
        )),
        Some(_) => None,
    };
    let loc = locale(model);
    let value_of = |f: &FieldRef| -> Value {
        let i = columns.iter().position(|c| c == f);
        row.and_then(|r| i.and_then(|i| r.get(i)))
            .cloned()
            .unwrap_or(Value::Null)
    };
    for (slot, f) in &bound {
        let raw = value_of(f);
        let text = display(&raw, format_string(model, f).as_deref(), loc);
        match slot {
            Slot::Run(id) => {
                let runs = o
                    .get_mut("text")
                    .and_then(|t| t.get_mut("paragraphs"))
                    .and_then(Value::as_array_mut)
                    .into_iter()
                    .flatten()
                    .filter_map(|p| p.get_mut("runs").and_then(Value::as_array_mut))
                    .flatten();
                for r in runs {
                    if r.get("bound").and_then(Value::as_str) == Some(id.as_str())
                        && let Some(r) = r.as_object_mut()
                    {
                        r.insert("text".into(), json!(text));
                    }
                }
            }
            Slot::ButtonText | Slot::ButtonFill => {
                let b = o.entry("button").or_insert_with(|| Value::Object(Map::new()));
                if let Some(b) = b.as_object_mut() {
                    if *slot == Slot::ButtonText {
                        b.insert("text".into(), json!(text));
                    } else if text.starts_with('#') && text.len() <= 9 {
                        b.insert("fill".into(), json!(text));
                    }
                }
            }
            Slot::ActionDestination | Slot::ActionTooltip => {
                let key = if *slot == Slot::ActionDestination {
                    "dynamic_destination"
                } else {
                    "dynamic_tooltip"
                };
                let a = o.entry("action").or_insert_with(|| Value::Object(Map::new()));
                if let Some(a) = a.as_object_mut() {
                    a.insert(key.into(), json!(text));
                }
            }
            Slot::Title => {
                o.insert("title".into(), json!(text));
            }
            Slot::Subtitle => {
                if let Some(s) = o
                    .get_mut("format")
                    .and_then(|f| f.get_mut("subtitle"))
                    .and_then(Value::as_object_mut)
                {
                    s.insert("text".into(), json!(text));
                }
            }
        }
    }
    if let Some(r) = reason {
        o.insert("preview_reason".into(), json!(r));
    }
}
