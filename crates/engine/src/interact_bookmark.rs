//! The reader's bookmarks (`view.interact {action: "bookmark"}`, a bookmark button, a bookmark
//! navigator): a bookmark applied to the viewer's state, never to the report or the files.
//!
//! What a bookmark document holds is read as `bookmark.apply` reads it (`bookmarks.rs`):
//! `explorationState.activeSection`, each section's `visualContainers[<visual>]` display mode
//! (`singleVisual.display.mode`: hidden, spotlight), its `singleVisual.orderBy` (the reader's
//! sort, under the data options) and the `filters` of the report, of each page and of each
//! visual, within the bookmark's `options` (`suppressActiveSection`, `suppressDisplay`,
//! `suppressData`, `applyOnlyToTargetVisuals` with `targetVisualNames`). Here they become
//! reader state:
//!
//! - the page: a navigation (`Back` returns to the page left);
//! - the display: a per-visual visibility override, which the snapshot shows as the visual's
//!   `hidden` (an override equal to the stored state is dropped);
//! - the data: a per-container filter override (report, page, visual), the `filterConfig` the
//!   data report evaluates; each slicer's captured selection (`singleVisual.objects`, below)
//!   becomes its applied selection, as a click would set it; the data points a container's
//!   `highlight` holds become the page's selection (`interact_highlight.rs`), else the page's
//!   selection is cleared; the drill states the containers hold (`activeProjections`, a
//!   matrix's `expansionStates`, `filters.byExpr` drill entries: `interact_drill.rs`) become the
//!   reader's drill, else each visual goes back to its stored drill state.
//!
//! **Slicer selections** (Desktop's Data option covers them, Microsoft Learn "Create report
//! bookmarks"): a slicer's container holds `singleVisual {visualType, objects}` where `objects`
//! is the bookmark schema's `DataViewObjectDefinitionUpdates`: `merge.general[0].properties.filter`
//! (the selection, the same `{filter}` a slicer stores in `objects.general`), `merge.data[0]`
//! (the range and relative-date properties a slicer shows), and `remove: [{object: "general",
//! property: "filter"}]` for a slicer with nothing selected. [`capture_slicer`] writes it,
//! [`slicer_state`] reads it; a container without them leaves the slicer alone.
//!
//! The last bookmark applied is kept (`interaction.bookmark`): a bookmark navigator shows it
//! selected.

use bicraft_model::SemanticModel;
use bicraft_report::{Report, Visual};
use serde_json::{Map, Value, json};

use super::{Interactions, RANGE_PROPS, RELATIVE_PROPS, SlicerState, put_slicer, stored_state};
use crate::preview::filters;

/// Most sections, visuals per section and target names read from one bookmark.
const MAX_ITEMS: usize = 10_000;

/// Where a filter override applies.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum Scope {
    Report,
    Page(String),
    Visual(String, String),
}

impl Scope {
    pub(crate) fn json(&self) -> Value {
        match self {
            Scope::Report => json!({"scope": "report"}),
            Scope::Page(p) => json!({"scope": "page", "page": p}),
            Scope::Visual(p, v) => json!({"scope": "visual", "page": p, "visual": v}),
        }
    }
}

/// Applies bookmark `key` (name or display name) to `view`. Returns the answer's fields:
/// `bookmark`, `page` (the page shown, when the bookmark changes it), `hidden` (the visuals whose
/// visibility changed), `filters_changed`, `warnings`.
pub(super) fn apply(
    view: &mut Interactions,
    r: &Report,
    model: Option<&SemanticModel>,
    key: &str,
) -> Result<Map<String, Value>, String> {
    let i = crate::bookmarks::bookmark_index(r, key)?;
    let b = r
        .bookmarks
        .get(i)
        .ok_or("internal: bookmark index out of range")?;
    let doc = &b.document;
    let state = doc
        .get("explorationState")
        .filter(|x| x.is_object())
        .ok_or("the bookmark holds no explorationState: nothing to apply")?;
    let opts = doc.get("options").cloned().unwrap_or(Value::Null);
    let flag = |k: &str| opts.get(k).and_then(Value::as_bool) == Some(true);
    let targets: Option<Vec<String>> = flag("applyOnlyToTargetVisuals").then(|| {
        opts.get("targetVisualNames")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .take(MAX_ITEMS)
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    });
    let in_targets = |v: &str| targets.as_ref().is_none_or(|t| t.iter().any(|x| x == v));
    let mut warnings: Vec<String> = Vec::new();
    let mut out = Map::new();
    // the page
    if !flag("suppressActiveSection")
        && let Some(want) = state.get("activeSection").and_then(Value::as_str)
    {
        match r.pages.iter().find(|p| p.name == want) {
            Some(p) => {
                view.go_to(r, &p.name);
                out.insert("page".into(), json!(p.name));
            }
            None => warnings.push(format!("the bookmark's page `{}` no longer exists", clip(want))),
        }
    }
    let empty = Map::new();
    let sections = state.get("sections").and_then(Value::as_object).unwrap_or(&empty);
    let mut hidden: Vec<Value> = Vec::new();
    let mut slicers: Vec<Value> = Vec::new();
    let filters_before = view.filters.clone();
    // the visuals the bookmark puts in spotlight (the page shown first)
    let mut spotlights: Vec<(String, String)> = Vec::new();
    for (page_name, sec) in sections.iter().take(MAX_ITEMS) {
        let Some(page) = r.pages.iter().find(|p| &p.name == page_name) else {
            warnings.push(format!(
                "the bookmark's page `{}` no longer exists",
                clip(page_name)
            ));
            continue;
        };
        let containers = sec.get("visualContainers").and_then(Value::as_object);
        if !flag("suppressDisplay")
            && let Some(containers) = containers
        {
            for v in page.visuals.iter().filter(|v| in_targets(&v.name)) {
                let mode = containers.get(&v.name).and_then(crate::bookmarks::display_mode);
                if mode == Some("spotlight") {
                    spotlights.push((page.name.clone(), v.name.clone()));
                }
                let want = mode == Some("hidden");
                let k = (page.name.clone(), v.name.clone());
                let now = view.hidden.get(&k).copied().unwrap_or(v.is_hidden);
                if want == v.is_hidden {
                    view.hidden.remove(&k);
                } else {
                    view.hidden.insert(k, want);
                }
                if now != want {
                    hidden.push(json!({"page": page.name, "visual": v.name, "hidden": want}));
                }
            }
        }
        if !flag("suppressData") {
            if let Some(f) = sec.get("filters") {
                set_filters(view, Scope::Page(page.name.clone()), &page.filter_config, f);
            }
            if let Some(containers) = containers {
                for v in page.visuals.iter().filter(|v| in_targets(&v.name)) {
                    // the sort the bookmark holds (`singleVisual.orderBy`), else the report's
                    let k = (page.name.clone(), v.name.clone());
                    match containers
                        .get(&v.name)
                        .and_then(|c| c.pointer("/singleVisual/orderBy"))
                        .and_then(crate::visual_props::sorts_from_order_by)
                    {
                        Some(sorts) if !matches!(crate::visual_props::current_sort(v), Some(Ok(cur)) if cur == sorts) =>
                        {
                            view.sorts.insert(k, sorts);
                        }
                        _ => {
                            view.sorts.remove(&k);
                        }
                    }
                    if let Some(f) = containers.get(&v.name).and_then(|c| c.get("filters")) {
                        set_filters(
                            view,
                            Scope::Visual(page.name.clone(), v.name.clone()),
                            &v.filter_config,
                            f,
                        );
                    }
                    if let Some(st) = containers.get(&v.name).and_then(|c| slicer_state(v, c)) {
                        let k = (page.name.clone(), v.name.clone());
                        let before = view.applied.get(&k).cloned();
                        let was_pending = view.pending.remove(&k).is_some();
                        if same_selection(v, &st) {
                            view.applied.remove(&k);
                        } else {
                            view.applied.insert(k.clone(), st);
                        }
                        if was_pending || view.applied.get(&k) != before.as_ref() {
                            slicers.push(json!({"page": page.name, "visual": v.name}));
                        }
                    }
                }
            }
            // the data points it selected (interact_highlight.rs), else none
            super::highlight::restore(
                view,
                page,
                containers,
                page.visuals.iter().filter(|v| in_targets(&v.name)),
            );
            // the drill states it holds (interact_drill.rs), else the stored ones
            super::drill::restore(
                view,
                model,
                page,
                containers,
                page.visuals.iter().filter(|v| in_targets(&v.name)),
            );
        }
    }
    if !flag("suppressData")
        && let Some(f) = state.get("filters")
    {
        set_filters(view, Scope::Report, &r.filter_config, f);
    }
    // the spotlight: the bookmark's (on the page shown when it has several), else none; a
    // bookmark of some visuals keeps a spotlight outside them
    if !flag("suppressDisplay") {
        let shown = view.page.clone().or_else(|| r.active_page.clone());
        let pick = spotlights
            .iter()
            .find(|(p, _)| Some(p) == shown.as_ref())
            .or_else(|| spotlights.first())
            .cloned();
        let keep = pick.is_none() && view.spotlight.as_ref().is_some_and(|(_, v)| !in_targets(v));
        if !keep {
            view.spotlight = pick;
        }
    }
    view.bookmark = Some(b.name.clone());
    out.insert("bookmark".into(), json!(b.name));
    out.insert("hidden".into(), Value::Array(hidden));
    out.insert("filters_changed".into(), json!(view.filters != filters_before));
    out.insert("slicers".into(), Value::Array(slicers));
    out.insert("warnings".into(), json!(warnings));
    Ok(out)
}

/// The filter override of one container: the stored `filterConfig` with the bookmark's filters
/// restored; none when that is the stored one.
fn set_filters(view: &mut Interactions, scope: Scope, stored: &Option<Value>, state: &Value) {
    let mut cfg = stored.clone();
    if crate::bookmarks::restore_filters(&mut cfg, state) {
        view.filters.insert(scope, cfg);
    } else {
        view.filters.remove(&scope);
    }
}

/// The range and relative-date properties of a slicer's `objects.data`.
fn data_props() -> impl Iterator<Item = &'static str> {
    RANGE_PROPS.iter().chain(RELATIVE_PROPS.iter()).copied()
}

/// The properties of the first selector-less entry of `objects[object]`.
fn plain_props<'a>(objects: Option<&'a Value>, object: &str) -> Option<&'a Map<String, Value>> {
    objects?
        .get(object)?
        .as_array()?
        .iter()
        .take(MAX_ITEMS)
        .find(|e| e.get("selector").is_none_or(Value::is_null))?
        .get("properties")?
        .as_object()
}

/// The `singleVisual` a bookmark captures for slicer `v` (its selection as shown): `None` for a
/// visual that is not a slicer. See the module docs for the shape.
pub(crate) fn capture_slicer(v: &Visual) -> Option<Value> {
    if !filters::is_slicer(v) {
        return None;
    }
    let mut merge = Map::new();
    let mut remove: Vec<Value> = Vec::new();
    match filters::slicer_filter(v) {
        Some(f) => {
            merge.insert(
                "general".into(),
                json!([{"properties": {"filter": {"filter": f}}}]),
            );
        }
        None => remove.push(json!({"object": "general", "property": "filter"})),
    }
    let mut data = Map::new();
    if let Some(props) = plain_props(v.visual_extra.get("objects"), "data") {
        for k in data_props() {
            if let Some(x) = props.get(k) {
                data.insert(k.into(), x.clone());
            }
        }
    }
    if !data.is_empty() {
        merge.insert("data".into(), json!([{"properties": data}]));
    }
    let mut objects = Map::new();
    if !merge.is_empty() {
        objects.insert("merge".into(), Value::Object(merge));
    }
    if !remove.is_empty() {
        objects.insert("remove".into(), Value::Array(remove));
    }
    let mut sv = Map::new();
    if let Some(t) = &v.visual_type {
        sv.insert("visualType".into(), json!(t));
    }
    sv.insert("objects".into(), Value::Object(objects));
    Some(Value::Object(sv))
}

/// The slicer selection bookmark container `c` holds for slicer `v`: `None` when `v` is not a
/// slicer or the container says nothing of its selection (no `general.filter` merged or
/// removed). A removed filter also clears the range and relative-date properties, as Clear
/// does; a property not merged is removed when the slicer has it.
pub(super) fn slicer_state(v: &Visual, c: &Value) -> Option<SlicerState> {
    if !filters::is_slicer(v) {
        return None;
    }
    let objects = c.pointer("/singleVisual/objects")?;
    let merged = objects.get("merge");
    let filter = merged
        .and_then(|m| m.get("general"))
        .and_then(Value::as_array)
        .and_then(|l| {
            l.iter()
                .take(MAX_ITEMS)
                .find_map(|e| e.pointer("/properties/filter/filter"))
        })
        .cloned();
    let removed = |object: &str, property: &str| {
        objects.get("remove").and_then(Value::as_array).is_some_and(|l| {
            l.iter().take(MAX_ITEMS).any(|r| {
                r.get("object").and_then(Value::as_str) == Some(object)
                    && r.get("property").and_then(Value::as_str) == Some(property)
            })
        })
    };
    if filter.is_none() && !removed("general", "filter") {
        return None;
    }
    let merged_data = plain_props(merged, "data");
    let current = plain_props(v.visual_extra.get("objects"), "data");
    let mut data = Vec::new();
    for k in data_props() {
        match merged_data.and_then(|m| m.get(k)) {
            Some(x) => data.push((k.to_string(), Some(x.clone()))),
            None if current.is_some_and(|m| m.contains_key(k)) => data.push((k.to_string(), None)),
            None => {}
        }
    }
    let mut st = SlicerState {
        filter,
        data,
        values: Vec::new(),
        inverted: false,
    };
    // the values and their sense, as a click reads them from the stored selection
    let mut shown = v.clone();
    put_slicer(&mut shown, &st);
    let read = stored_state(&shown);
    st.values = read.values;
    st.inverted = read.inverted;
    Some(st)
}

/// Whether `st` is the selection `v` already stores (filter and range properties).
fn same_selection(v: &Visual, st: &SlicerState) -> bool {
    if filters::slicer_filter(v) != st.filter.as_ref() {
        return false;
    }
    let current = plain_props(v.visual_extra.get("objects"), "data");
    st.data
        .iter()
        .all(|(k, x)| current.and_then(|m| m.get(k)) == x.as_ref())
}

/// Restores the selection bookmark container `c` holds into slicer `v` (`bookmark.apply`, which
/// writes the report). True when the visual changed.
pub(crate) fn restore_slicer(v: &mut Visual, c: &Value) -> bool {
    match slicer_state(v, c) {
        Some(st) if !same_selection(v, &st) => {
            put_slicer(v, &st);
            true
        }
        _ => false,
    }
}

/// Whether bookmark container `c` holds a selection for slicer `v`.
pub(crate) fn holds_slicer(v: &Visual, c: &Value) -> bool {
    slicer_state(v, c).is_some()
}

/// Applies the visibility and filter overrides to a derived report.
pub(super) fn apply_overrides(view: &Interactions, data: &mut Report) {
    for ((pg, vn), hide) in &view.hidden {
        if let Some(v) = data
            .pages
            .iter_mut()
            .find(|p| &p.name == pg)
            .and_then(|p| p.visuals.iter_mut().find(|v| &v.name == vn))
        {
            v.is_hidden = *hide;
        }
    }
    for (scope, cfg) in &view.filters {
        match scope {
            Scope::Report => data.filter_config = cfg.clone(),
            Scope::Page(pg) => {
                if let Some(p) = data.pages.iter_mut().find(|p| &p.name == pg) {
                    p.filter_config = cfg.clone();
                }
            }
            Scope::Visual(pg, vn) => {
                if let Some(v) = data
                    .pages
                    .iter_mut()
                    .find(|p| &p.name == pg)
                    .and_then(|p| p.visuals.iter_mut().find(|v| &v.name == vn))
                {
                    v.filter_config = cfg.clone();
                }
            }
        }
    }
}

/// The overrides as JSON (`view.interactions`): `hidden` and `filters`, sorted.
pub(super) fn state_json(view: &Interactions) -> (Value, Value) {
    let mut hidden: Vec<Value> = view
        .hidden
        .iter()
        .map(|((p, v), h)| json!({"page": p, "visual": v, "hidden": h}))
        .collect();
    hidden.sort_by_key(|x| x.to_string());
    let mut scopes: Vec<&Scope> = view.filters.keys().collect();
    scopes.sort();
    let filters: Vec<Value> = scopes
        .into_iter()
        .map(|s| {
            let mut j = s.json();
            j["filters"] = view
                .filters
                .get(s)
                .cloned()
                .flatten()
                .and_then(|c| c.get("filters").cloned())
                .unwrap_or_else(|| json!([]));
            j
        })
        .collect();
    (Value::Array(hidden), Value::Array(filters))
}

fn clip(s: &str) -> String {
    s.chars().take(80).collect()
}

#[cfg(test)]
#[path = "interact_bookmark_tests.rs"]
mod tests;
