//! The Filters pane's commands (docs/design.md §12, ROADMAP M11.13): filters on the page
//! (`page.filter`, `page.clear_filters`), on every page (`report.filter`,
//! `report.clear_filters`), the empty filter card a field dropped on a section adds, clearing a
//! card's condition while keeping the card, and the values a basic filter lists
//! (`column.values`). The conditions themselves are `visual.filter`'s ([`crate::visual_props`]):
//! basic (`in` / `not_in`), advanced (`op` + `value`, `conditions` + `logic`), relative date
//! or time (`relative`, [`crate::filter_relative`]) and top N (visual filters only, as in
//! Desktop). The view-mode flags `hidden` and `locked` without a condition set the card's lock
//! and eye (Desktop's icons on each card) and keep its condition.
//!
//! Page and report filters take columns only, as in Power BI Desktop: a measure or an aggregation
//! is filtered on a visual.

use bicraft_model::SemanticModel;
use bicraft_report::FieldRef;
use serde_json::{Map, Value, json};

use crate::Session;
use crate::params::{allow, opt_bool, str_param};
use crate::refs::{field_text, same_field};
use crate::report::{page_at_mut, page_index, parts, visual_index};
use crate::visual_props::{
    FILTER_PARAMS, apply_filters, card_json, field_from_json, field_to_json, filter_list, filters_without,
    fresh_name, has_condition, insert_ordered, known_fields, parse_filters, resolve,
};

/// Most filter cards of one level.
const MAX_CARDS: usize = 100;
/// Most values `column.values` returns.
const MAX_VALUES: usize = 10_000;
/// Most rows `column.values` reads.
const MAX_ROWS: usize = 50_000_000;
/// Key order of a filter container (Desktop's), as `visual_props` writes it.
const ORDER: &[&str] = &[
    "name",
    "displayName",
    "ordinal",
    "field",
    "type",
    "filter",
    "restatement",
    "howCreated",
    "isHiddenInViewMode",
    "isLockedInViewMode",
    "objects",
];

/// Which level's filters a command edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scope {
    Visual,
    Page,
    Report,
}

impl Scope {
    fn owner(self) -> &'static str {
        match self {
            Scope::Visual => "the visual's",
            Scope::Page => "the page's",
            Scope::Report => "the report's",
        }
    }

    fn keys(self) -> &'static [&'static str] {
        match self {
            Scope::Visual => &["page", "visual"],
            Scope::Page => &["page"],
            Scope::Report => &[],
        }
    }
}

/// Runs `f` on the `filterConfig` of the scope `p` names, with the model and the fields known
/// there, and reports `{page?, visual?, changed, changes, filters: [cards]}`.
fn with_cfg(
    s: &mut Session,
    p: &Value,
    scope: Scope,
    f: impl FnOnce(&mut Option<Value>, Option<&SemanticModel>, &[FieldRef]) -> Result<Vec<String>, String>,
) -> Result<Value, String> {
    let (model, r) = parts(s)?;
    let mut out = Map::new();
    let (cfg, known): (&mut Option<Value>, Vec<FieldRef>) = match scope {
        Scope::Report => (&mut r.filter_config, Vec::new()),
        Scope::Page | Scope::Visual => {
            let pi = page_index(r, str_param(p, "page")?)?;
            let page = page_at_mut(r, pi)?;
            out.insert("page".into(), json!(page.name));
            if scope == Scope::Page {
                (&mut page.filter_config, Vec::new())
            } else {
                let vi = visual_index(page, str_param(p, "visual")?)?;
                let v = page
                    .visuals
                    .get_mut(vi)
                    .ok_or("internal: visual index out of range")?;
                if v.visual_type.is_none() {
                    return Err(format!("`{}` is a visual group; groups have no filters", v.name));
                }
                out.insert("visual".into(), json!(v.name));
                let known = known_fields(v);
                (&mut v.filter_config, known)
            }
        }
    };
    let mut next = cfg.clone();
    let changes = f(&mut next, model, &known)?;
    let changed = next != *cfg;
    if changed {
        *cfg = next;
    }
    out.insert("changed".into(), json!(changed));
    out.insert("changes".into(), json!(changes));
    out.insert("filters".into(), cards(cfg.as_ref()));
    if changed {
        s.report_dirty = true;
    }
    Ok(Value::Object(out))
}

/// The cards of a `filterConfig` as the snapshot lists them ([`card_json`]).
/// They come in the order the Filters pane shows them ([`sort_mode`], [`display_order`]).
pub(crate) fn cards(cfg: Option<&Value>) -> Value {
    let list = filter_list(cfg).unwrap_or_default();
    let order = display_order(&list, sort_mode(cfg));
    Value::Array(
        order
            .into_iter()
            .filter_map(|i| list.get(i).copied())
            .take(MAX_CARDS)
            .map(card_json)
            .collect(),
    )
}

fn flag(p: &Value, k: &str) -> Result<Option<bool>, String> {
    opt_bool(p, k)
}

/// The containers of `cfg`, refusing a `filterConfig` BIcraft cannot edit.
fn containers(cfg: &Option<Value>, owner: &str) -> Result<Vec<Value>, String> {
    match cfg {
        None => Ok(Vec::new()),
        Some(Value::Object(o)) => match o.get("filters") {
            None => Ok(Vec::new()),
            Some(Value::Array(a)) => Ok(a.clone()),
            Some(_) => Err(format!(
                "{owner} filterConfig.filters is not a list; it cannot be edited"
            )),
        },
        Some(_) => Err(format!(
            "{owner} filterConfig is not an object; it cannot be edited"
        )),
    }
}

/// Writes `list` back as the containers of `cfg` (no `filterConfig` left when it is empty).
fn put(cfg: &mut Option<Value>, list: Vec<Value>) {
    let mut o = cfg
        .as_ref()
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if list.is_empty() {
        o.shift_remove("filters");
    } else {
        insert_ordered(
            &mut o,
            "filters",
            Value::Array(list),
            &["$schema", "filters", "filterSortOrder"],
        );
    }
    *cfg = if o.is_empty() {
        None
    } else {
        Some(Value::Object(o))
    };
}

fn card_field(c: &Value) -> Option<FieldRef> {
    c.get("field").and_then(|f| field_from_json(f, &Vec::new(), None))
}

/// Adds an empty filter card on `field` (Desktop's `type` for it: `Categorical` for a column,
/// `Advanced` otherwise; `howCreated` `Auto` for a field the visual binds, as Desktop makes those
/// cards, else `User`), or sets the view-mode flags of the card already there.
fn add_card(
    cfg: &mut Option<Value>,
    field: &FieldRef,
    bound: bool,
    hidden: Option<bool>,
    locked: Option<bool>,
    owner: &str,
) -> Result<Vec<String>, String> {
    let mut list = containers(cfg, owner)?;
    let mut changes = Vec::new();
    let text = field_text(field);
    if let Some(c) = list
        .iter_mut()
        .find(|c| card_field(c).is_some_and(|f| same_field(&f, field)))
    {
        let o = c
            .as_object_mut()
            .ok_or_else(|| format!("a filter of {owner} is not an object; it cannot be edited"))?;
        for (key, value) in [("isHiddenInViewMode", hidden), ("isLockedInViewMode", locked)] {
            let Some(b) = value else { continue };
            if o.get(key).and_then(Value::as_bool).unwrap_or(false) != b {
                insert_ordered(o, key, json!(b), ORDER);
                changes.push(format!("filter on {text}: {key} {b}"));
            }
        }
        put(cfg, list);
        return Ok(changes);
    }
    if list.len() >= MAX_CARDS {
        return Err(format!("{owner} filters already have {MAX_CARDS} cards"));
    }
    let taken: Vec<String> = list
        .iter()
        .filter_map(|c| c.get("name").and_then(Value::as_str).map(str::to_string))
        .collect();
    let mut c = Map::new();
    c.insert("name".into(), json!(fresh_name(&taken)));
    c.insert(
        "field".into(),
        field_to_json(field, None).ok_or("this field cannot be written in a filter")?,
    );
    let ty = if matches!(field, FieldRef::Column { .. }) {
        "Categorical"
    } else {
        "Advanced"
    };
    c.insert("type".into(), json!(ty));
    c.insert("howCreated".into(), json!(if bound { "Auto" } else { "User" }));
    if let Some(h) = hidden {
        c.insert("isHiddenInViewMode".into(), json!(h));
    }
    if let Some(l) = locked {
        c.insert("isLockedInViewMode".into(), json!(l));
    }
    list.push(Value::Object(c));
    changes.push(format!("filter card on {text} added"));
    put(cfg, list);
    Ok(changes)
}

/// Clears the condition of the cards on `field` (every card when `None`). With `keep`, every
/// card stays without condition (Desktop's Clear filter); else a card Desktop made for a bound
/// field (`howCreated: Auto`) stays without condition and the others go (Remove filter).
fn clear_cards(
    cfg: &mut Option<Value>,
    field: Option<&FieldRef>,
    keep: bool,
    owner: &str,
) -> Result<Vec<String>, String> {
    let list = containers(cfg, owner)?;
    let mut changes = Vec::new();
    let mut matched = false;
    let mut out = Vec::with_capacity(list.len());
    for mut c in list {
        let f = card_field(&c);
        let hit = match field {
            None => true,
            Some(want) => f.as_ref().is_some_and(|f| same_field(f, want)),
        };
        if !hit {
            out.push(c);
            continue;
        }
        matched = true;
        let label = f.as_ref().map_or_else(|| "a field".to_string(), field_text);
        let auto = c.get("howCreated").and_then(Value::as_str) == Some("Auto");
        if keep || auto {
            if let Some(o) = c.as_object_mut()
                && o.shift_remove("filter").is_some()
            {
                changes.push(format!("filter on {label} cleared"));
            }
            out.push(c);
        } else {
            changes.push(format!("filter on {label} removed"));
        }
    }
    if let Some(f) = field
        && !matched
    {
        return Err(format!("{owner} filters have no card on {}", field_text(f)));
    }
    put(cfg, out);
    Ok(changes)
}

/// The field of a `*.filter` / `*.clear_filters` call, resolved where it is filtered.
fn field_of(
    p: &Value,
    model: Option<&SemanticModel>,
    known: &[FieldRef],
    scope: Scope,
) -> Result<Option<FieldRef>, String> {
    let text = match p.get("field") {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::String(t)) => t,
        Some(_) => return Err("parameter `field` must be a string".into()),
    };
    let f = resolve(text, model, known)?;
    if scope != Scope::Visual && !matches!(f, FieldRef::Column { .. }) {
        return Err(format!(
            "{} filters take columns; {} is filtered on a visual (visual.filter)",
            if scope == Scope::Page { "page" } else { "report" },
            field_text(&f)
        ));
    }
    Ok(Some(f))
}

/// An empty filter card (no condition): `*.filter {field}` without `in`, `op`, `conditions` or
/// `top`.
pub(crate) fn filter_card(s: &mut Session, p: &Value, scope: Scope) -> Result<Value, String> {
    for k in ["value", "logic", "by", "direction"] {
        if p.get(k).is_some_and(|v| !v.is_null()) {
            return Err(format!(
                "`{k}` needs a condition (`in` / `not_in`, `op` or `conditions`, `top`)"
            ));
        }
    }
    let hidden = flag(p, "hidden")?;
    let locked = flag(p, "locked")?;
    with_cfg(s, p, scope, |cfg, model, known| {
        let field = field_of(p, model, known, scope)?.ok_or("missing string parameter `field`")?;
        let bound = known.iter().any(|k| same_field(k, &field));
        add_card(cfg, &field, bound, hidden, locked, scope.owner())
    })
}

/// `*.clear_filters` through the cards (see [`clear_cards`]).
pub(crate) fn clear_cards_of(s: &mut Session, p: &Value, scope: Scope, keep: bool) -> Result<Value, String> {
    with_cfg(s, p, scope, |cfg, model, known| {
        let field = match field_of(p, model, known, scope) {
            Ok(f) => f,
            // a card on a field the model no longer has (model.analyze's fix): found by its name
            Err(e) => Some(stale_card_field(cfg, p).ok_or(e)?),
        };
        clear_cards(cfg, field.as_ref(), keep, scope.owner())
    })
}

/// The field of the card `p.field` (`'T'[C]`) names, compared by table and name only: a card
/// on a column or measure removed or renamed in the model.
fn stale_card_field(cfg: &Option<Value>, p: &Value) -> Option<FieldRef> {
    let text = p.get("field").and_then(Value::as_str)?;
    let crate::refs::ParsedField::Qualified { table, name } = crate::refs::parse_field(text).ok()? else {
        return None;
    };
    let eq = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
    containers(cfg, "").ok()?.iter().find_map(|c| {
        let f = card_field(c)?;
        let hit = match &f {
            FieldRef::Column { table: t, column: n }
            | FieldRef::Aggregation {
                table: t, column: n, ..
            }
            | FieldRef::Measure { table: t, measure: n } => eq(t, &table) && eq(n, &name),
            FieldRef::Other { .. } => false,
        };
        hit.then_some(f)
    })
}

/// `page.filter` / `report.filter`.
fn level_filter(s: &mut Session, p: &Value, scope: Scope) -> Result<Value, String> {
    // Desktop offers Top N on visual filters only (docs/formats/pbir.md, "Top N cards")
    const TOP_N: [&str; 3] = ["top", "by", "direction"];
    for k in TOP_N {
        if p.get(k).is_some() {
            return Err(format!(
                "top N filters are visual-level filters (visual.filter), as in Power BI Desktop: `{k}` is not taken here"
            ));
        }
    }
    let mut keys = scope.keys().to_vec();
    keys.extend(FILTER_PARAMS.iter().filter(|k| !TOP_N.contains(k)));
    allow(p, &keys)?;
    if !has_condition(p) {
        return filter_card(s, p, scope);
    }
    let mut item = p.as_object().cloned().unwrap_or_default();
    item.shift_remove("page");
    let item = Value::Object(item);
    with_cfg(s, p, scope, |cfg, model, known| {
        let field = field_of(p, model, known, scope)?.ok_or("missing string parameter `field`")?;
        let (mut list, _) = filters_without(cfg.as_ref(), model, known, Some(&field));
        list.push(item);
        let reqs = parse_filters(&Value::Array(list), model, known)?;
        let mut changes = Vec::new();
        apply_filters(cfg, &reqs, &mut changes, scope.owner())?;
        Ok(changes)
    })
}

/// `page.clear_filters` / `report.clear_filters`.
fn level_clear(s: &mut Session, p: &Value, scope: Scope) -> Result<Value, String> {
    let mut keys = scope.keys().to_vec();
    keys.extend_from_slice(&["field", "keep_cards"]);
    allow(p, &keys)?;
    let keep = opt_bool(p, "keep_cards")?.unwrap_or(false);
    clear_cards_of(s, p, scope, keep)
}

pub(crate) fn page_filter(s: &mut Session, p: &Value) -> Result<Value, String> {
    level_filter(s, p, Scope::Page)
}

pub(crate) fn page_clear_filters(s: &mut Session, p: &Value) -> Result<Value, String> {
    level_clear(s, p, Scope::Page)
}

pub(crate) fn report_filter(s: &mut Session, p: &Value) -> Result<Value, String> {
    level_filter(s, p, Scope::Report)
}

pub(crate) fn report_clear_filters(s: &mut Session, p: &Value) -> Result<Value, String> {
    level_clear(s, p, Scope::Report)
}

/// `column.values {table, column, search?, limit?}`: the column's distinct values over its stored
/// data or literal partitions, sorted (blank first), at most `limit` (default and cap
/// [`MAX_VALUES`]), those containing `search` (case-insensitive) when given: `{values, count,
/// truncated, available}`. `available: false` when the session has no data for it.
pub(crate) fn column_values(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &["table", "column", "search", "limit"])?;
    let table = str_param(p, "table")?;
    let column = str_param(p, "column")?;
    let search = crate::params::opt_str(p, "search")?.map(str::to_lowercase);
    if search.as_ref().is_some_and(|q| q.chars().count() > 1000) {
        return Err("`search` is longer than 1000 characters".into());
    }
    let limit = match p.get("limit") {
        None | Some(Value::Null) => MAX_VALUES,
        Some(v) => v
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| (1..=MAX_VALUES).contains(n))
            .ok_or_else(|| format!("`limit` must be a whole number from 1 to {MAX_VALUES}"))?,
    };
    let model = s.model.as_ref().ok_or("no model is open")?;
    let t = model
        .table(table)
        .ok_or_else(|| format!("the model has no table `{table}`"))?;
    let c = t
        .column(column)
        .ok_or_else(|| format!("table `{}` has no column `{column}`", t.name))?;
    let (t_name, c_name) = (t.name.clone(), c.name.clone());
    let Some(values) = crate::preview::distinct_values(s, &t_name, &c_name, MAX_ROWS)? else {
        return Ok(
            json!({"table": t_name, "column": c_name, "values": [], "count": 0,
                         "truncated": false, "available": false}),
        );
    };
    let matches = |v: &Value| match &search {
        None => true,
        Some(q) => match v {
            Value::Null => "(blank)".contains(q.as_str()),
            Value::String(x) => x.to_lowercase().contains(q.as_str()),
            other => other.to_string().to_lowercase().contains(q.as_str()),
        },
    };
    let found: Vec<Value> = values.into_iter().filter(matches).collect();
    let count = found.len();
    let truncated = count > limit;
    let values: Vec<Value> = found.into_iter().take(limit).collect();
    Ok(
        json!({"table": t_name, "column": c_name, "values": values, "count": count,
              "truncated": truncated, "available": true}),
    )
}

// ----- card order (filter.order) -----

/// `filterSortOrder` of a `filterConfig` as the snapshot names it: `ascending` (also when absent:
/// Desktop lists filters alphabetically by default, Microsoft Learn), `descending` or `custom`.
pub(crate) fn sort_mode(cfg: Option<&Value>) -> &'static str {
    match cfg.and_then(|c| c.get("filterSortOrder")).and_then(Value::as_str) {
        Some(s) if s.eq_ignore_ascii_case("Descending") => "descending",
        Some(s) if s.eq_ignore_ascii_case("Custom") => "custom",
        _ => "ascending",
    }
}

/// The name a card is sorted by: its `displayName`, else its column's or measure's name, else
/// its field's text (lower case).
fn sort_key(c: &Value) -> String {
    if let Some(d) = c.get("displayName").and_then(Value::as_str) {
        return d.to_lowercase();
    }
    match card_field(c) {
        Some(FieldRef::Column { column, .. }) => column.to_lowercase(),
        Some(FieldRef::Measure { measure, .. }) => measure.to_lowercase(),
        Some(FieldRef::Aggregation { column, .. }) => column.to_lowercase(),
        Some(f) => field_text(&f).to_lowercase(),
        None => String::new(),
    }
}

/// The positions of `list` in the order the Filters pane shows them under `mode` (the public
/// `filterConfiguration` schema: by display name ascending or descending, or by `ordinal`, the
/// cards without one last, by display name).
fn display_order(list: &[&Value], mode: &str) -> Vec<usize> {
    let keys: Vec<String> = list.iter().map(|c| sort_key(c)).collect();
    let by_name = |a: &usize, b: &usize| {
        let ka = keys.get(*a).map(String::as_str).unwrap_or("");
        let kb = keys.get(*b).map(String::as_str).unwrap_or("");
        ka.cmp(kb)
    };
    let mut order: Vec<usize> = (0..list.len()).collect();
    match mode {
        "descending" => order.sort_by(|a, b| by_name(b, a)),
        "custom" => {
            let ord = |i: &usize| {
                list.get(*i)
                    .and_then(|c| c.get("ordinal"))
                    .and_then(Value::as_f64)
                    .filter(|x| x.is_finite())
            };
            order.sort_by(|a, b| match (ord(a), ord(b)) {
                (Some(x), Some(y)) => x.total_cmp(&y).then_with(|| by_name(a, b)),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => by_name(a, b),
            });
        }
        _ => order.sort_by(by_name),
    }
    order
}

/// Moves the card on `field` to position `to` of the shown order (custom order: every card gets
/// its `ordinal`), and / or sets the sort mode.
fn order_cards(
    cfg: &mut Option<Value>,
    field: Option<(&FieldRef, usize)>,
    sort: Option<&str>,
    owner: &str,
) -> Result<Vec<String>, String> {
    let mut list = containers(cfg, owner)?;
    let refs: Vec<&Value> = list.iter().collect();
    let mode = sort.unwrap_or_else(|| {
        if field.is_some() {
            "custom"
        } else {
            sort_mode(cfg.as_ref())
        }
    });
    let mut order = display_order(&refs, sort_mode(cfg.as_ref()));
    drop(refs);
    let mut changes = Vec::new();
    if let Some((f, to)) = field {
        if mode != "custom" {
            return Err("moving a card sets the custom order: `sort` must be custom or absent".into());
        }
        let at = order
            .iter()
            .position(|i| {
                list.get(*i)
                    .and_then(card_field)
                    .is_some_and(|x| same_field(&x, f))
            })
            .ok_or_else(|| format!("{owner} filters have no card on {}", field_text(f)))?;
        let i = order.remove(at);
        let to = to.min(order.len());
        order.insert(to, i);
        changes.push(format!(
            "filter on {} moved to position {}",
            field_text(f),
            to + 1
        ));
    }
    if mode == "custom" {
        for (n, i) in order.iter().enumerate() {
            let Some(o) = list.get_mut(*i).and_then(Value::as_object_mut) else {
                return Err(format!(
                    "a filter of {owner} is not an object; it cannot be edited"
                ));
            };
            let want = json!(n);
            if o.get("ordinal") != Some(&want) {
                insert_ordered(o, "ordinal", want, ORDER);
            }
        }
    }
    let name = match mode {
        "descending" => "Descending",
        "custom" => "Custom",
        _ => "Ascending",
    };
    let before = cfg
        .as_ref()
        .and_then(|c| c.get("filterSortOrder"))
        .and_then(Value::as_str);
    if before != Some(name) {
        changes.push(format!("filters sorted {mode}"));
    }
    put(cfg, list);
    let mut o = cfg
        .as_ref()
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    insert_ordered(
        &mut o,
        "filterSortOrder",
        json!(name),
        &["$schema", "filters", "filterSortOrder"],
    );
    *cfg = Some(Value::Object(o));
    Ok(changes)
}

/// `filter.order {level: visual|page|report, page?, visual?, field?, to?, sort?}`.
pub(crate) fn filter_order(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["level", "page", "visual", "field", "to", "sort"])?;
    let scope = match str_param(p, "level")?.to_ascii_lowercase().as_str() {
        "visual" => Scope::Visual,
        "page" => Scope::Page,
        "report" => Scope::Report,
        _ => return Err("`level` must be visual, page or report".into()),
    };
    for k in ["page", "visual"] {
        if p.get(k).is_some() && !scope.keys().contains(&k) {
            return Err(format!("`{k}` is not taken for {} filters", scope.owner()));
        }
    }
    let sort = match crate::params::opt_str(p, "sort")?
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        None => None,
        Some("custom") => Some("custom"),
        Some("ascending") | Some("a_z") => Some("ascending"),
        Some("descending") | Some("z_a") => Some("descending"),
        Some(_) => return Err("`sort` must be custom, ascending or descending".into()),
    };
    let to = match p.get("to") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            v.as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .filter(|n| *n < MAX_CARDS)
                .ok_or_else(|| format!("`to` must be a whole number from 0 to {}", MAX_CARDS - 1))?,
        ),
    };
    let has_field = p.get("field").is_some_and(|v| !v.is_null());
    if has_field != to.is_some() {
        return Err("`field` and `to` go together (the card and its new position)".into());
    }
    if !has_field && sort.is_none() {
        return Err("filter.order needs `field` and `to`, or `sort`".into());
    }
    with_cfg(s, p, scope, |cfg, model, known| {
        let field = field_of(p, model, known, scope)?;
        let moved = match (&field, to) {
            (Some(f), Some(t)) => Some((f, t)),
            _ => None,
        };
        if containers(cfg, scope.owner())?.is_empty() {
            return Err(format!("{} filters have no card to order", scope.owner()));
        }
        order_cards(cfg, moved, sort, scope.owner())
    })
}
