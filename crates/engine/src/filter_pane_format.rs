//! The Filters pane's formatting (`page.format_filter_pane`): Desktop's Format pane › Filter pane
//! and Filter cards cards of a page, written to the page's `objects.outspacePane` (the pane) and
//! `objects.filterCard` (the cards, one entry per state: `selector.id` `Available` or `Applied`),
//! as the public page schema 2.1.0 (`PageFormattingObjects`, `OutspacePane`, `FilterCard`) names
//! them; a theme's `outspacePane` and `filterCard` (public report theme schema 2.157, `$id` for
//! the state) are the defaults. What each property means is in `docs/formats/pbir.md` ("Filters
//! pane formatting and card order").

use serde_json::{Map, Value, json};

use crate::Session;
use crate::params::{allow, opt_bool, opt_str, str_param};
use crate::report::{page_at_mut, page_index, parts};
use crate::rich::{bool_lit, check_color, color_lit, num_lit, text_lit};
use crate::theme::{self, Theme};
use bicraft_report::Page;

/// A property's kind and, for numbers, its bounds.
#[derive(Debug, Clone, Copy)]
enum Kind {
    Color,
    Num(f64, f64),
    Bool,
    Text,
}

/// The pane's properties (`OutspacePane`): parameter, schema name, kind.
const PANE: Props = &[
    ("background_color", "backgroundColor", Kind::Color),
    ("transparency", "transparency", Kind::Num(0.0, 100.0)),
    ("foreground_color", "foregroundColor", Kind::Color),
    ("font_family", "fontFamily", Kind::Text),
    ("title_size", "titleSize", Kind::Num(6.0, 72.0)),
    ("header_size", "headerSize", Kind::Num(6.0, 72.0)),
    ("search_text_size", "searchTextSize", Kind::Num(6.0, 72.0)),
    ("border", "border", Kind::Bool),
    ("border_color", "borderColor", Kind::Color),
    ("input_box_color", "inputBoxColor", Kind::Color),
    ("checkbox_color", "checkboxAndApplyColor", Kind::Color),
    ("width", "width", Kind::Num(100.0, 1000.0)),
];

/// A card state's properties (`FilterCard`).
const CARD: Props = &[
    ("background_color", "backgroundColor", Kind::Color),
    ("transparency", "transparency", Kind::Num(0.0, 100.0)),
    ("foreground_color", "foregroundColor", Kind::Color),
    ("font_family", "fontFamily", Kind::Text),
    ("text_size", "textSize", Kind::Num(6.0, 72.0)),
    ("border", "border", Kind::Bool),
    ("border_color", "borderColor", Kind::Color),
    ("input_box_color", "inputBoxColor", Kind::Color),
];

/// The three things formatted: the pane, the available cards and the applied cards.
const TARGETS: [&str; 3] = ["pane", "available", "applied"];

/// Longest font family kept (characters).
const MAX_FAMILY: usize = 200;

/// A property table: parameter, schema name, kind.
type Props = &'static [(&'static str, &'static str, Kind)];

/// The property table and the `objects` key of `card`, with its selector id (`None`: the pane).
fn target(card: &str) -> Result<(Props, &'static str, Option<&'static str>), String> {
    match card.to_ascii_lowercase().as_str() {
        "pane" => Ok((PANE, "outspacePane", None)),
        "available" => Ok((CARD, "filterCard", Some("Available"))),
        "applied" => Ok((CARD, "filterCard", Some("Applied"))),
        _ => Err(format!("`card` must be one of {}", TARGETS.join(", "))),
    }
}

/// The literal of `v` for a property of `kind`, checked (`key` names it in errors).
fn literal(key: &str, kind: Kind, v: &Value) -> Result<Value, String> {
    match kind {
        Kind::Color => {
            let c = v
                .as_str()
                .ok_or_else(|| format!("`{key}` must be a colour like #118DFF or null"))?;
            Ok(color_lit(&check_color(key, c)?))
        }
        Kind::Num(lo, hi) => {
            let n = v
                .as_f64()
                .filter(|n| n.is_finite())
                .ok_or_else(|| format!("`{key}` must be a number or null"))?;
            if n < lo || n > hi {
                return Err(format!("`{key}` must be from {lo} to {hi}"));
            }
            Ok(num_lit(n.round()))
        }
        Kind::Bool => v
            .as_bool()
            .map(bool_lit)
            .ok_or_else(|| format!("`{key}` must be true, false or null")),
        Kind::Text => {
            let t = v
                .as_str()
                .map(str::trim)
                .ok_or_else(|| format!("`{key}` must be a string or null"))?;
            if t.is_empty() || t.chars().count() > MAX_FAMILY || t.chars().any(char::is_control) {
                return Err(format!(
                    "`{key}` must be a font family name of 1 to {MAX_FAMILY} characters"
                ));
            }
            Ok(text_lit(t))
        }
    }
}

/// Whether the page entry `e` is the one of `id` (`None`: the entry without a selector).
fn is_entry(e: &Value, id: Option<&str>) -> bool {
    let sel = e.get("selector").filter(|s| !s.is_null());
    match (id, sel) {
        (None, None) => true,
        (Some(want), Some(s)) => s
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|x| x.eq_ignore_ascii_case(want)),
        _ => false,
    }
}

/// `page.format_filter_pane {page, card?: pane|available|applied, reset?, <property>?}`.
pub(crate) fn page_format_filter_pane(s: &mut Session, p: &Value) -> Result<Value, String> {
    let card = opt_str(p, "card")?.unwrap_or("pane").to_ascii_lowercase();
    let (props, object, id) = target(&card)?;
    let mut allowed = vec!["page", "card", "reset"];
    allowed.extend(PANE.iter().chain(CARD).map(|x| x.0));
    allow(p, &allowed)?;
    // a property of the other target is refused by name
    for (k, _, _) in PANE.iter().chain(CARD) {
        if p.get(*k).is_some() && !props.iter().any(|x| x.0 == *k) {
            return Err(format!(
                "`{k}` is not a property of {}",
                if id.is_none() {
                    "the Filters pane"
                } else {
                    "filter cards"
                }
            ));
        }
    }
    let reset = opt_bool(p, "reset")?.unwrap_or(false);
    let mut edits: Vec<(&str, Option<Value>)> = Vec::new();
    for (k, name, kind) in props {
        match p.get(*k) {
            None => {}
            Some(Value::Null) => edits.push((name, None)),
            Some(v) => edits.push((name, Some(literal(k, *kind, v)?))),
        }
    }
    if edits.is_empty() && !reset {
        return Err(format!(
            "page.format_filter_pane needs `reset` or a property ({})",
            props.iter().map(|x| x.0).collect::<Vec<_>>().join(", ")
        ));
    }
    let key = str_param(p, "page")?;
    let (_, r) = parts(s)?;
    let pi = page_index(r, key)?;
    let page = page_at_mut(r, pi)?;
    let before = page.clone();
    let mut objects = match page.extra.get("objects") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(m)) => m.clone(),
        Some(_) => return Err("page.json `objects` is not an object".into()),
    };
    let mut entries = match objects.get(object) {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) => a.clone(),
        Some(_) => return Err(format!("the page's `objects.{object}` is not a list")),
    };
    let i = match entries.iter().position(|e| is_entry(e, id)) {
        Some(i) => i,
        None => {
            // Desktop's key order: properties, then selector
            let mut e = Map::new();
            e.insert("properties".into(), Value::Object(Map::new()));
            if let Some(id) = id {
                e.insert("selector".into(), json!({"id": id}));
            }
            entries.push(Value::Object(e));
            entries.len().saturating_sub(1)
        }
    };
    let entry = entries
        .get_mut(i)
        .and_then(Value::as_object_mut)
        .ok_or_else(|| format!("the page's `objects.{object}` holds a value that is not an object"))?;
    let slot = entry
        .entry("properties")
        .or_insert_with(|| Value::Object(Map::new()));
    if !slot.is_object() {
        *slot = Value::Object(Map::new());
    }
    let pmap = slot
        .as_object_mut()
        .ok_or_else(|| "internal: no properties".to_string())?;
    let mut changes = Vec::new();
    if reset {
        // only the properties this command writes go: others stay as read
        for (_, name, _) in props {
            if pmap.shift_remove(*name).is_some() {
                changes.push(format!("{name} reset"));
            }
        }
    }
    for (name, v) in edits {
        match v {
            Some(v) => {
                if pmap.get(name) != Some(&v) {
                    pmap.insert(name.to_string(), v);
                    changes.push(format!("{name} set"));
                }
            }
            None => {
                if pmap.shift_remove(name).is_some() {
                    changes.push(format!("{name} removed"));
                }
            }
        }
    }
    let empty = pmap.is_empty() && entry.keys().all(|k| k == "properties" || k == "selector");
    if empty {
        entries.remove(i);
    }
    if entries.is_empty() {
        objects.shift_remove(object);
    } else {
        objects.insert(object.to_string(), Value::Array(entries));
    }
    if objects.is_empty() {
        page.extra.shift_remove("objects");
    } else {
        page.extra.insert("objects".into(), Value::Object(objects));
    }
    let changed = *page != before;
    let name = page.name.clone();
    if changed {
        s.report_dirty = true;
    }
    let format = s
        .report
        .as_ref()
        .and_then(|r| {
            let t = theme::resolve(r);
            r.pages.get(pi).map(|pg| resolved(pg, t.as_ref()))
        })
        .unwrap_or(Value::Null);
    Ok(json!({"page": name, "card": card, "changed": changed, "changes": changes, "format": format}))
}

/// The resolved format of one target: the theme's entries then the page's, later ones winning.
fn resolve_target(page: &Page, theme: Option<&Theme>, card: &str) -> Map<String, Value> {
    let Ok((props, object, id)) = target(card) else {
        return Map::new();
    };
    // theme entries: those without `$id` first (every state), then the state's own
    let themed = theme::theme_object_entries(theme, object);
    let theme_id = |e: &Value| e.get("$id").and_then(Value::as_str).map(str::to_string);
    let mut sources: Vec<(&Value, bool)> = Vec::new();
    sources.extend(
        themed
            .iter()
            .filter(|e| theme_id(e).is_none())
            .map(|e| (*e, true)),
    );
    if let Some(id) = id {
        sources.extend(
            themed
                .iter()
                .filter(|e| theme_id(e).is_some_and(|x| x.eq_ignore_ascii_case(id)))
                .map(|e| (*e, true)),
        );
    }
    let own = page
        .extra
        .get("objects")
        .and_then(|o| o.get(object))
        .and_then(Value::as_array);
    if let Some(list) = own {
        // a card entry without a selector formats both states, under the state's own
        if id.is_some() {
            sources.extend(list.iter().filter(|e| is_entry(e, None)).map(|e| (e, false)));
        }
        sources.extend(list.iter().filter(|e| is_entry(e, id)).map(|e| (e, false)));
    }
    let mut out = Map::new();
    for (param, name, kind) in props {
        let v = sources
            .iter()
            .rev()
            .find_map(|(e, from_theme)| theme::entry_value(e, name, *from_theme, theme));
        let v = match (kind, v) {
            (Kind::Color, Some(Value::String(c))) => check_color(param, &c).ok().map(Value::String),
            (Kind::Num(lo, hi), Some(v)) => v
                .as_f64()
                .filter(|n| n.is_finite())
                .map(|n| json!(n.clamp(*lo, *hi))),
            (Kind::Bool, Some(Value::Bool(b))) => Some(json!(b)),
            (Kind::Text, Some(Value::String(t))) => {
                let first = t
                    .split(',')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .trim_matches(['\'', '"'])
                    .trim();
                (!first.is_empty()).then(|| json!(first.chars().take(MAX_FAMILY).collect::<String>()))
            }
            _ => None,
        };
        if let Some(v) = v {
            out.insert((*param).to_string(), v);
        }
    }
    out
}

/// `{pane, available, applied}`: each the resolved properties (only those the page or the theme
/// sets).
pub(crate) fn resolved(page: &Page, theme: Option<&Theme>) -> Value {
    let mut o = Map::new();
    for t in TARGETS {
        o.insert(t.to_string(), Value::Object(resolve_target(page, theme, t)));
    }
    Value::Object(o)
}

/// The page's `filter_pane_format` for the snapshot, `None` when neither the page nor the theme
/// formats the Filters pane.
pub(crate) fn snapshot(page: &Page, theme: Option<&Theme>) -> Option<Value> {
    let v = resolved(page, theme);
    let any = v
        .as_object()
        .is_some_and(|o| o.values().any(|x| x.as_object().is_some_and(|m| !m.is_empty())));
    any.then_some(v)
}
