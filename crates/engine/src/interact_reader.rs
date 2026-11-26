//! The reader's Spotlight and sort (`view.interact` `spotlight` and `sort`), as the Power BI
//! service's report readers have them from a visual's More options (Microsoft Learn, "Add
//! spotlights to Power BI reports" and "Change how a chart is sorted in a report"):
//!
//! - **Spotlight** puts one visual forward: the page's other visuals fade to near transparency
//!   (the renderer draws them at [`bicraft_render`]'s spotlight opacity). Display state only: the
//!   data are not changed, a page change and `reset` leave it. A bookmark captures it as
//!   `singleVisual.display.mode: "spotlight"` (the PBIR bookmark schema) and applying one sets it.
//! - **Sort** changes how the reader sees a visual sorted, never the report: the data report's
//!   visual takes the reader's sort as its `query.sortDefinition`, so the preview and the
//!   snapshot's `sort` follow it. `reset` (the service's Reset to default) drops it; a bookmark
//!   carries it as `singleVisual.orderBy`.

use bicraft_model::SemanticModel;
use bicraft_report::{FieldRef, Report};
use serde_json::{Value, json};

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
