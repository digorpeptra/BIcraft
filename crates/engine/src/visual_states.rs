//! Formatting states of the stateful built-in visuals (Power BI Desktop 2.158): the new card,
//! the new slicers, buttons, navigators and shapes read most of their formatting objects from a
//! *state* entry, the object entry whose selector is `{"id": "<state>"}` (`default`, `hover`,
//! `press`, `selected`, `disabled`…). Desktop ignores a property written to such an object
//! without that selector; the on/off toggle `show` is the exception (Desktop reads it from the
//! entry without selector, and writes it to both). See `docs/formats/visual-options.md`,
//! "State selectors".
//!
//! Writers (`visual_props.rs`) put a property of a stateful object in the `default` state entry
//! unless the caller names another state; `report.validate` warns about formatting Desktop
//! ignores because of its selector in existing files ([`validation_issues`]).

use bicraft_report::{Report, Visual};
use serde_json::{Map, Value, json};

/// The state written when the caller names none.
pub(crate) const DEFAULT_STATE: &str = "default";
/// The `state` value that names the entry without selector.
pub(crate) const NO_STATE: &str = "none";

/// States of buttons, navigators and shapes.
const BUTTON_STATES: &[&str] = &["default", "hover", "press", "selected", "disabled"];
/// States of the new slicers (the theme schema's `$id` values).
const SLICER_STATES: &[&str] = &[
    "default",
    "hover",
    "press",
    "selection",
    "selection:selected",
    "selection:unselected",
    "expansion:expanded",
    "expansion:collapsed",
    "series:selectall",
    "interaction:hover",
    "interaction:press",
];
/// States of the new card.
const CARD_STATES: &[&str] = &["default"];

const BUTTON_OBJECTS: &[&str] = &["fill", "glow", "icon", "outline", "shadow", "shape", "text"];
const NAVIGATOR_OBJECTS: &[&str] = &["accentBar", "fill", "glow", "outline", "shadow", "text"];
const SHAPE_OBJECTS: &[&str] = &["fill", "outline", "text"];
const CARD_OBJECTS: &[&str] = &[
    "accentBar",
    "border",
    "cardCalloutArea",
    "divider",
    "fillCustom",
    "glowCustom",
    "image",
    "label",
    "layout",
    "outline",
    "padding",
    "referenceLabelDetail",
    "referenceLabelTitle",
    "referenceLabelValue",
    "shadowCustom",
    "spacing",
    "value",
];
const LIST_SLICER_OBJECTS: &[&str] = &[
    "accentBar",
    "background",
    "expansionIcon",
    "fillCustom",
    "icon",
    "image",
    "label",
    "outline",
    "padding",
    "selectionIcon",
    "value",
];
const BUTTON_SLICER_OBJECTS: &[&str] = &[
    "accentBar",
    "background",
    "fillCustom",
    "icon",
    "image",
    "label",
    "outline",
    "padding",
    "selectionIcon",
    "value",
];

/// (visual type, stateful objects, states). Sources: Desktop 2.158's report theme schema (the
/// objects whose theme entries take `$id`) and the render oracle (`docs/formats/visual-options.md`).
const STATEFUL: &[(&str, &[&str], &[&str])] = &[
    ("actionButton", BUTTON_OBJECTS, BUTTON_STATES),
    ("bookmarkNavigator", NAVIGATOR_OBJECTS, BUTTON_STATES),
    ("pageNavigator", NAVIGATOR_OBJECTS, BUTTON_STATES),
    ("shape", SHAPE_OBJECTS, BUTTON_STATES),
    ("cardVisual", CARD_OBJECTS, CARD_STATES),
    ("listSlicer", LIST_SLICER_OBJECTS, SLICER_STATES),
    ("advancedSlicerVisual", BUTTON_SLICER_OBJECTS, SLICER_STATES),
];

/// The states `object` of `visual_type` takes, when it is a stateful object.
pub(crate) fn object_states(visual_type: Option<&str>, object: &str) -> Option<&'static [&'static str]> {
    let vt = visual_type?;
    STATEFUL
        .iter()
        .find(|(t, objects, _)| *t == vt && objects.contains(&object))
        .map(|(_, _, states)| *states)
}

/// Whether `visual_type` has stateful objects.
pub(crate) fn is_stateful_type(visual_type: Option<&str>) -> bool {
    visual_type.is_some_and(|vt| STATEFUL.iter().any(|(t, _, _)| *t == vt))
}

/// The selector of state `state`.
pub(crate) fn state_selector(state: &str) -> Value {
    json!({ "id": state })
}

/// The state an entry selector names: `Some("none")` for no selector, `Some(id)` for a selector
/// that is exactly `{"id": <text>}`, `None` for any other selector.
pub(crate) fn entry_state(entry: &Value) -> Option<&str> {
    match entry.get("selector") {
        None | Some(Value::Null) => Some(NO_STATE),
        Some(Value::Object(o)) if o.len() == 1 => o.get("id").and_then(Value::as_str),
        Some(_) => None,
    }
}

fn entry_props(entry: &Value) -> Option<&Map<String, Value>> {
    entry.get("properties").and_then(Value::as_object)
}

/// Objects whose on/off `show` Desktop reads from the entry without selector even though the
/// rest of the object comes from the `default` state: with `show` only in the state entry the
/// fill or text is not drawn.
const TOGGLE_OBJECTS: &[(&str, &[&str])] = &[("actionButton", &["fill", "text"]), ("shape", &["text"])];

/// Objects of these visual types that Desktop reads only without selector (render captures): a
/// `default` state entry there is ignored. `None`: every object of the type.
const STATELESS_OBJECTS: &[(&str, Option<&[&str]>)] = &[
    ("cardVisual", Some(&["overFlow"])),
    ("advancedSlicerVisual", Some(&["layout"])),
    ("shape", Some(&["rotation", "shape"])),
    ("textSlicer", None),
    ("basicShape", None),
];

fn is_toggle(visual_type: Option<&str>, object: &str) -> bool {
    visual_type.is_some_and(|vt| {
        TOGGLE_OBJECTS
            .iter()
            .any(|(t, o)| *t == vt && o.contains(&object))
    })
}

fn is_stateless(visual_type: Option<&str>, object: &str) -> bool {
    visual_type.is_some_and(|vt| {
        STATELESS_OBJECTS
            .iter()
            .any(|(t, o)| *t == vt && o.is_none_or(|o| o.contains(&object)))
    })
}

/// `report.validate` warnings: formatting Desktop ignores because of its state selector (see
/// [`findings`]).
pub(crate) fn validation_issues(r: &Report) -> Vec<(bool, Value, String)> {
    let mut out = Vec::new();
    for page in &r.pages {
        for v in &page.visuals {
            for (object, message) in findings(v) {
                out.push((
                    false,
                    json!({"page": page.display_name, "visual": v.name, "part": "objects", "object": object}),
                    message,
                ));
            }
        }
    }
    out
}

fn list(props: &[String]) -> String {
    props
        .iter()
        .map(|p| format!("`{p}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// (object, message) for each object of `v` whose formatting Desktop 2.158 ignores:
/// - a stateful object with properties other than `show` only in the entry without selector;
/// - a button's or shape's fill / text whose `show` is not in the entry without selector;
/// - a new card's entry whose selector adds `id` to `metadata` (per-callout formatting is the
///   `metadata` selector alone);
/// - an object Desktop reads only without selector, with properties only in its `default` state
///   entry.
fn findings(v: &Visual) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let vt = v.visual_type.as_deref();
    let Some(objects) = v.visual_extra.get("objects").and_then(Value::as_object) else {
        return out;
    };
    let name = vt.unwrap_or_default();
    for (object, entries) in objects {
        let Some(entries) = entries.as_array() else {
            continue;
        };
        let props_in = |state: &str| -> Vec<&Map<String, Value>> {
            entries
                .iter()
                .filter(|e| entry_state(e) == Some(state))
                .filter_map(entry_props)
                .collect()
        };
        let default = props_in(DEFAULT_STATE);
        let stateless = props_in(NO_STATE);
        let keys = |maps: &[&Map<String, Value>]| -> Vec<String> {
            let mut k: Vec<String> = maps.iter().flat_map(|p| p.keys().cloned()).collect();
            k.sort();
            k.dedup();
            k
        };
        if object_states(vt, object).is_some() {
            let has_default_entry = entries.iter().any(|e| entry_state(e) == Some(DEFAULT_STATE));
            // `show` is exempt: Desktop reads the on/off toggle without selector (a lone
            // `show: true` draws a card's accent bar in its default look).
            let missing: Vec<String> = keys(&stateless)
                .into_iter()
                .filter(|k| k != "show" && !default.iter().any(|d| d.contains_key(k.as_str())))
                .collect();
            if !missing.is_empty() {
                out.push((
                    object.clone(),
                    format!(
                        "{name} `{object}`: {} written without a state selector; Power BI Desktop applies this object's formatting from its `default` state (selector {{\"id\": \"default\"}}) and ignores {}",
                        list(&missing),
                        if missing.len() == 1 { "it" } else { "them" }
                    ),
                ));
            }
            if is_toggle(vt, object) && has_default_entry && !stateless.iter().any(|p| p.contains_key("show"))
            {
                out.push((
                    object.clone(),
                    format!(
                        "{name} `{object}`: `show` is not in the entry without selector; Power BI Desktop reads this on/off toggle there and does not draw the {object} (write `show` both without selector and in the `default` state)"
                    ),
                ));
            }
            if vt == Some("cardVisual") {
                for e in entries {
                    let Some(sel) = e.get("selector").and_then(Value::as_object) else {
                        continue;
                    };
                    if sel.contains_key("id") && sel.contains_key("metadata") {
                        out.push((
                            object.clone(),
                            format!(
                                "{name} `{object}`: an entry's selector combines `id` with `metadata`; Power BI Desktop ignores it (per-callout formatting is the selector {{\"metadata\": <queryRef>}} alone)"
                            ),
                        ));
                    }
                }
            }
        } else if is_stateless(vt, object) {
            let loose = keys(&stateless);
            let missing: Vec<String> = keys(&default)
                .into_iter()
                .filter(|k| !loose.contains(k))
                .collect();
            if !missing.is_empty() {
                out.push((
                    object.clone(),
                    format!(
                        "{name} `{object}`: {} written only in the `default` state entry; Power BI Desktop reads this object without selector and ignores {}",
                        list(&missing),
                        if missing.len() == 1 { "it" } else { "them" }
                    ),
                ));
            }
        }
    }
    out
}

#[cfg(test)]
#[path = "visual_states_tests.rs"]
mod tests;
