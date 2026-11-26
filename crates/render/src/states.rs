//! Button, shape and navigator states in the readers (`docs/formats/buttons.md`): Power BI
//! draws a button in its default, on hover, on press or disabled state, a shape in its default,
//! hover or press state, and each navigator button in its default, hover, press or selected
//! state (Microsoft Learn, "Create buttons in Power BI reports", "Create page and bookmark
//! navigators"). The snapshot carries what each state sets (`button.states`, `shape.states`,
//! `navigator.styles`); the front end says where the pointer is ([`Pointer`]), and the engine
//! says when a button is disabled (`interaction.button.enabled`).

use serde_json::{Map, Value};

use crate::data::{Ctx, get, sk};

/// Where the reader's pointer is: over visual `visual` (and navigator button `item`, a page or
/// bookmark name), pressed or not. The front end passes it in `RenderOptions::pointer` and draws
/// the page again when it changes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Pointer {
    pub visual: String,
    pub item: Option<String>,
    pub press: bool,
}

/// The state a button or shape `v` is drawn in: `disabled` when the engine says its action cannot
/// run (a drillthrough button without a selected data point), else `press` or `hover` under the
/// pointer, else `None` (the default state).
pub fn visual_state(v: &Value, ctx: &Ctx<'_>) -> Option<&'static str> {
    if get(get(get(v, "interaction"), "button"), "enabled") == &Value::Bool(false) {
        return Some("disabled");
    }
    let p = ctx.pointer.as_ref()?;
    if p.visual != sk(v, "name") || p.item.is_some() {
        return None;
    }
    Some(if p.press { "press" } else { "hover" })
}

/// `v` with `v[key]` (`button` or `shape`) overlaid by its `states[state]` entry: the state's
/// properties win, the default state's stay for the rest (a shape's `text` merged one level
/// down). `None` when the state sets nothing.
pub fn with_state(v: &Value, key: &str, state: &str) -> Option<Value> {
    let over = get(get(get(v, key), "states"), state).as_object()?;
    if over.is_empty() {
        return None;
    }
    let mut out = v.clone();
    let base = out.get_mut(key)?.as_object_mut()?;
    for (k, x) in over {
        match (base.get_mut(k), x) {
            (Some(Value::Object(b)), Value::Object(o)) => {
                for (k2, x2) in o {
                    b.insert(k2.clone(), x2.clone());
                }
            }
            _ => {
                base.insert(k.clone(), x.clone());
            }
        }
    }
    Some(out)
}

/// Whether a disabled button sets its own disabled look (else the reader dims its default one,
/// BIcraft's choice: Microsoft Learn names the state, not its default formatting).
pub fn has_disabled_look(v: &Value) -> bool {
    get(get(get(v, "button"), "states"), "disabled")
        .as_object()
        .is_some_and(|o| !o.is_empty())
}

/// The navigator button under the pointer in navigator `v`: (its page or bookmark name, pressed).
pub fn nav_item<'a>(v: &Value, ctx: &'a Ctx<'_>) -> Option<(&'a str, bool)> {
    let p = ctx.pointer.as_ref()?;
    if p.visual != sk(v, "name") {
        return None;
    }
    Some((p.item.as_deref()?, p.press))
}

/// A navigator style merged in order (later layers win): default, selected, hover, press.
pub fn nav_layers(styles: &Value, selected: bool, hot: Option<bool>) -> Map<String, Value> {
    let mut out = Map::new();
    let mut add = |k: &str| {
        if let Some(o) = get(styles, k).as_object() {
            for (k, x) in o {
                out.insert(k.clone(), x.clone());
            }
        }
    };
    add("default");
    if selected {
        add("selected");
    }
    if let Some(press) = hot {
        add("hover");
        if press {
            add("press");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_state_overlays_the_default_one() {
        let v = json!({"name": "b", "button": {"text": "Go", "fill": "#111111", "font_color": "#222222",
            "states": {"hover": {"fill": "#333333"}}}});
        let h = with_state(&v, "button", "hover").unwrap();
        assert_eq!(h["button"]["fill"], json!("#333333"));
        assert_eq!(h["button"]["font_color"], json!("#222222"));
        assert_eq!(h["button"]["text"], json!("Go"));
        assert!(with_state(&v, "button", "press").is_none());
        // a shape's text merges one level down
        let s = json!({"shape": {"text": {"text": "Hi", "font_color": "#000000"},
            "states": {"press": {"text": {"font_color": "#FFFFFF"}}}}});
        let p = with_state(&s, "shape", "press").unwrap();
        assert_eq!(p["shape"]["text"], json!({"text": "Hi", "font_color": "#FFFFFF"}));
    }

    #[test]
    fn navigator_layers_follow_the_state_order() {
        let styles = json!({"default": {"fill": "#000001", "bold": false},
            "selected": {"fill": "#000002"}, "hover": {"fill": "#000003"}, "press": {"bold": true}});
        assert_eq!(nav_layers(&styles, false, None)["fill"], json!("#000001"));
        assert_eq!(nav_layers(&styles, true, None)["fill"], json!("#000002"));
        assert_eq!(nav_layers(&styles, true, Some(false))["fill"], json!("#000003"));
        let pressed = nav_layers(&styles, false, Some(true));
        assert_eq!(pressed["bold"], json!(true));
        assert_eq!(pressed["fill"], json!("#000003"));
    }
}
