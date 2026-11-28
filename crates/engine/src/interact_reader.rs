//! The reader's Spotlight, sort and filters (`view.interact` `spotlight`, `sort` and `filter`), as
//! the Power BI service's report readers have them (Microsoft Learn, "Add spotlights to Power BI
//! reports", "Change how a chart is sorted in a report", "Filters and highlighting in reports"):
//!
//! - **Spotlight** puts one visual forward: the page's other visuals fade to near transparency
//!   (the renderer draws them at [`bicraft_render`]'s spotlight opacity). Display state only: the
//!   data are not changed, a page change and `reset` leave it. A bookmark captures it as
//!   `singleVisual.display.mode: "spotlight"` (the PBIR bookmark schema) and applying one sets it.
//! - **Sort** changes how the reader sees a visual sorted, never the report: the data report's
//!   visual takes the reader's sort as its `query.sortDefinition`, so the preview and the
//!   snapshot's `sort` follow it. `reset` (the service's Reset to default) drops it; a bookmark
//!   carries it as `singleVisual.orderBy`.
//! - **Filters** (`filter` / `filter_clear`): a reader changes the condition of a filter card the
//!   author did not lock or hide, as Power BI's reading view allows; the changed `filterConfig`
//!   is an override of the derived reports, never the report ([`filter`]).

use bicraft_model::SemanticModel;
use bicraft_report::{FieldRef, Report};
use serde_json::{Value, json};

use super::bookmark::Scope;
use super::{Interactions, find_visual, focusable};

/// `spotlight {page, visual | null}`: the visual put in spotlight (null ends it).
pub(super) fn spotlight(next: &mut Interactions, r: &Report, p: &Value) -> Result<(), String> {
    crate::params::allow(p, &["action", "page", "visual"])?;
    next.spotlight = match p.get("visual").filter(|x| !x.is_null()) {
        None => None,
        Some(_) => Some(focusable(r, p)?),
    };
    Ok(())
}

/// `sort {page, visual, field | null, direction?}`: the reader's sort of a visual (`field` and
/// `direction` as `visual.set_sort` takes them; null goes back to the report's sort).
pub(super) fn sort(
    next: &mut Interactions,
    r: &Report,
    model: Option<&SemanticModel>,
    p: &Value,
) -> Result<(), String> {
    crate::params::allow(p, &["action", "page", "visual", "field", "direction"])?;
    let field = p
        .get("field")
        .ok_or("`field` is required (a field string, or null for the report's sort)")?;
    if field.is_null() && p.get("direction").is_some_and(|d| !d.is_null()) {
        return Err("`direction` needs a `field`".into());
    }
    let (page, v) = focusable(r, p)?;
    let (_, visual) = find_visual(r, p)?;
    if visual.fields.is_empty() {
        return Err(format!("visual `{v}` has no fields to sort by"));
    }
    let key = (page, v);
    match crate::visual_props::reader_sort(visual, model, field, p.get("direction"))? {
        None => {
            next.sorts.remove(&key);
        }
        Some(sorts) => {
            // the report's own sort: nothing to override
            let stored = matches!(crate::visual_props::current_sort(visual), Some(Ok(cur)) if cur == sorts);
            if stored {
                next.sorts.remove(&key);
            } else {
                next.sorts.insert(key, sorts);
            }
        }
    }
    Ok(())
}

/// The data report's visuals take the reader's sorts.
pub(super) fn apply_sorts(view: &Interactions, data: &mut Report) {
    for ((pg, vn), sorts) in &view.sorts {
        if let Some(v) = data
            .pages
            .iter_mut()
            .find(|p| &p.name == pg)
            .and_then(|p| p.visuals.iter_mut().find(|v| &v.name == vn))
        {
            crate::visual_props::put_sort(v, sorts);
        }
    }
}

/// The spotlight visual `{page, visual}` when it is still in `r` (`null` otherwise).
pub(super) fn spotlight_json(view: &Interactions, r: &Report) -> Value {
    let Some((pg, vn)) = view.spotlight.as_ref() else {
        return Value::Null;
    };
    let there = r
        .pages
        .iter()
        .find(|p| &p.name == pg)
        .is_some_and(|p| p.visuals.iter().any(|v| &v.name == vn));
    if there {
        json!({"page": pg, "visual": vn})
    } else {
        Value::Null
    }
}

/// The reader's sorts as JSON (`view.interactions`), sorted by page and visual.
pub(super) fn sorts_json(view: &Interactions) -> Value {
    let mut keys: Vec<&(String, String)> = view.sorts.keys().collect();
    keys.sort();
    Value::Array(
        keys.into_iter()
            .filter_map(|k| {
                let sorts = view.sorts.get(k)?;
                let list: Vec<Value> = sorts
                    .iter()
                    .map(|(f, d)| json!({"field": crate::refs::field_text(f), "descending": d}))
                    .collect();
                Some(json!({"page": k.0, "visual": k.1, "sort": list}))
            })
            .collect(),
    )
}

impl Interactions {
    /// The visual in spotlight, (page, visual).
    pub(crate) fn spotlight(&self) -> Option<(&str, &str)> {
        self.spotlight.as_ref().map(|(p, v)| (p.as_str(), v.as_str()))
    }

    /// The reader's sort of a visual.
    pub(crate) fn reader_sort(&self, page: &str, visual: &str) -> Option<&[(FieldRef, bool)]> {
        self.sorts
            .get(&(page.to_string(), visual.to_string()))
            .map(Vec::as_slice)
    }
}

/// `filter {page?, visual?, field, condition}` / `filter_clear {page?, visual?, field}`: the
/// reader changes an unlocked filter card of the visual (`page` and `visual`), the page (`page`)
/// or every page (neither), as Power BI's reading view allows
/// ([`crate::filter_edit::reader_change`]). The changed `filterConfig` is an override of the
/// data and display reports (the one a bookmark sets, [`super::bookmark::apply_overrides`]),
/// never the report: `reset` drops it, a bookmark applied after it replaces it.
pub(super) fn filter(
    next: &mut Interactions,
    r: &Report,
    model: Option<&SemanticModel>,
    p: &Value,
    clear: bool,
) -> Result<(), String> {
    use crate::filter_edit::Scope as Level;
    let mut keys = vec!["action", "page", "visual", "field", "hidden", "locked"];
    if !clear {
        keys.extend_from_slice(&[
            "in",
            "not_in",
            "op",
            "value",
            "conditions",
            "logic",
            "relative",
            "top",
            "by",
            "direction",
        ]);
    }
    crate::params::allow(p, &keys)?;
    let given = |k: &str| p.get(k).is_some_and(|x| !x.is_null());
    let (scope, level, stored, known) = if given("visual") {
        let (page, v) = find_visual(r, p)?;
        if v.visual_type.is_none() {
            return Err(format!("`{}` is a visual group; groups have no filters", v.name));
        }
        (
            Scope::Visual(page.name.clone(), v.name.clone()),
            Level::Visual,
            &v.filter_config,
            crate::visual_props::known_fields(v),
        )
    } else if given("page") {
        let page = super::find_page(r, p.get("page"))?;
        (
            Scope::Page(page.name.clone()),
            Level::Page,
            &page.filter_config,
            Vec::new(),
        )
    } else {
        (Scope::Report, Level::Report, &r.filter_config, Vec::new())
    };
    let mut cfg = match next.filters.get(&scope) {
        Some(c) => c.clone(),
        None => stored.clone(),
    };
    let types = crate::report_settings::allow_change_filter_types(r);
    crate::filter_edit::reader_change(&mut cfg, p, level, model, &known, clear, types)?;
    if cfg == *stored {
        next.filters.remove(&scope);
    } else {
        next.filters.insert(scope, cfg);
    }
    Ok(())
}

/// `tree_toggle {page, visual, id}`: a reader's click on an organization chart node's collapse
/// button (BIcraft Org Chart). The reader's list of toggled ids replaces the stored
/// `tree.toggled` in the data report, the report unchanged, as the visual keeps its own state
/// where the host does not persist it; back to the stored list, the override is dropped.
pub(super) fn tree_toggle(next: &mut Interactions, r: &Report, p: &Value) -> Result<(), String> {
    crate::params::allow(p, &["action", "page", "visual", "id"])?;
    let id = p
        .get("id")
        .and_then(Value::as_str)
        .ok_or("`id` (the node's id, a text) is required")?;
    let (page, visual) = find_visual(r, p)?;
    let key = (page.name.clone(), visual.name.clone());
    let list = crate::chart_pack::toggled_after(visual, next.nodes.get(&key).map(Vec::as_slice), id)?;
    if list == crate::chart_pack::stored_toggled(visual) {
        next.nodes.remove(&key);
    } else {
        next.nodes.insert(key, list);
    }
    Ok(())
}

/// The data report's organization charts take the reader's toggled nodes.
pub(super) fn apply_nodes(view: &Interactions, data: &mut Report) {
    for ((pg, vn), list) in &view.nodes {
        if let Some(v) = data
            .pages
            .iter_mut()
            .find(|p| &p.name == pg)
            .and_then(|p| p.visuals.iter_mut().find(|v| &v.name == vn))
        {
            crate::chart_pack::put_toggled(v, list);
        }
    }
}

/// The key under which the data report's Gantt carries the reader's expanded and collapsed
/// categories (`[{path, collapsed}]`), read by the snapshot after the saved `row` entries
/// (`snapshot_visual`, `gantt_rows`). Never saved: only the derived reports have it.
pub(crate) const GANTT_STATE: &str = "bicraft.gantt";
/// Most categories of one Gantt whose state the reader set.
const MAX_GANTT_PATHS: usize = 500;
/// Category levels of a Gantt (its `Category` well takes three fields).
const GANTT_LEVELS: usize = 3;

/// `gantt_toggle {page, visual, path, collapsed}`: the reader expands or collapses one category
/// of a Gantt (its expand/collapse icon): `path` names the category's members, top level first
/// (one per `Category` field down to it), `collapsed` its new state. In this view only.
pub(super) fn gantt_toggle(next: &mut Interactions, r: &Report, p: &Value) -> Result<(), String> {
    crate::params::allow(p, &["action", "page", "visual", "path", "collapsed"])?;
    let (page, visual) = find_visual(r, p)?;
    if !visual
        .visual_type
        .as_deref()
        .is_some_and(|t| t.eq_ignore_ascii_case("ganttChart"))
    {
        return Err(format!("visual `{}` is not a Gantt", visual.name));
    }
    let levels = visual
        .fields
        .iter()
        .filter(|b| b.role.eq_ignore_ascii_case("Category"))
        .count()
        .min(GANTT_LEVELS);
    let path = p
        .get("path")
        .and_then(Value::as_array)
        .ok_or("`path` (the category's members, top level first) is required")?;
    if path.is_empty() || path.len() > levels {
        return Err(format!(
            "`path` names 1 to {levels} category members (the Gantt's Category fields), not {}",
            path.len()
        ));
    }
    if path.iter().any(|m| m.is_array() || m.is_object()) {
        return Err("`path` members are values (text, numbers, dates, true/false or null)".into());
    }
    let collapsed = p
        .get("collapsed")
        .and_then(Value::as_bool)
        .ok_or("`collapsed` (true or false) is required")?;
    let list = next
        .gantt
        .entry((page.name.clone(), visual.name.clone()))
        .or_default();
    match list.iter().position(|(q, _)| q == path) {
        Some(i) => {
            if let Some(e) = list.get_mut(i) {
                e.1 = collapsed;
            }
        }
        None if list.len() < MAX_GANTT_PATHS => list.push((path.clone(), collapsed)),
        None => {
            return Err(format!(
                "at most {MAX_GANTT_PATHS} categories of one Gantt are kept"
            ));
        }
    }
    Ok(())
}

/// The data report's Gantts take the reader's expanded and collapsed categories.
pub(super) fn apply_gantt(view: &Interactions, data: &mut Report) {
    for ((pg, vn), list) in &view.gantt {
        if let Some(v) = data
            .pages
            .iter_mut()
            .find(|p| &p.name == pg)
            .and_then(|p| p.visuals.iter_mut().find(|v| &v.name == vn))
        {
            let rows: Vec<Value> = list
                .iter()
                .map(|(path, collapsed)| json!({"path": path, "collapsed": collapsed}))
                .collect();
            v.visual_extra.insert(GANTT_STATE.into(), Value::Array(rows));
        }
    }
}

/// The reader's Gantt states (`view.interactions`' `gantt`).
pub(super) fn gantt_json(view: &Interactions) -> Value {
    let mut keys: Vec<&(String, String)> = view.gantt.keys().collect();
    keys.sort();
    Value::Array(
        keys.into_iter()
            .filter_map(|k| {
                let list = view.gantt.get(k)?;
                let rows: Vec<Value> = list
                    .iter()
                    .map(|(path, collapsed)| json!({"path": path, "collapsed": collapsed}))
                    .collect();
                Some(json!({"page": k.0, "visual": k.1, "rows": rows}))
            })
            .collect(),
    )
}
